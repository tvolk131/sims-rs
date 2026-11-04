//! Stock furniture pickup, rollback and relocation through the real button caller.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{
    Session,
    move_tool::{
        button::{Input, Kind},
        motion,
    },
};
use std::{collections::BTreeSet, path::Path};

#[test]
#[ignore = "private stock house; release/headless"]
fn stock_furniture_pickup_cancel_move_and_cold_resume() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("pickup test panicked")
}
fn saved(s: &Session, d: &Device) -> Result<Value> {
    Ok(json!({"session":s.save(),"camera_device":d.save(s)?}))
}
fn finish_move(s: &mut Session, d: &mut Device) -> Result<()> {
    for _ in 0..30000 {
        if d.advance_move_tool(s, 256)? {
            return Ok(());
        }
    }
    anyhow::bail!("motion stalled")
}
fn button(
    s: &mut Session,
    d: &mut Device,
    index: &Index,
    out: &Path,
    name: &str,
    resumed: bool,
    cold: &mut BTreeSet<String>,
    input: Input,
) -> Result<()> {
    s.begin_move_button(input)?;
    for n in 0..50000 {
        ensure!(s.begin_clock_frame().is_err());
        let result = d.advance_move_button(s, if resumed { 1 } else { 256 });
        if let Err(e) = result {
            std::fs::write(
                out.join(format!("{name}-error.json")),
                serde_json::to_vec(&saved(s, d)?)?,
            )?;
            return Err(e).with_context(|| format!("{name} {:?} slice{n}", input.kind));
        }
        if result.unwrap() {
            return Ok(());
        }
        let b = serde_json::to_value(&s.host.presentation.move_button)?;
        let boundary = if s.simulation.cursor_factory.is_some() {
            Some("factory")
        } else if input.kind == Kind::Cancel
            && s.host
                .presentation
                .move_tool
                .as_ref()
                .is_some_and(|m| m.tool.payload.is_some())
        {
            Some("rollback")
        } else if s.simulation.cursor_release.is_some() {
            Some("release")
        } else if b["child"].get("Selection").is_some() {
            Some("selection")
        } else {
            None
        };
        if resumed
            && let Some(boundary) = boundary
            && cold.insert(boundary.to_owned())
        {
            let before = saved(s, d)?;
            std::fs::write(
                out.join(format!("{name}-{boundary}.json")),
                serde_json::to_vec(&before)?,
            )?;
            *s = Session::restore(index, serde_json::from_value(before["session"].clone())?)?;
            *d = Device::restore(s, serde_json::from_value(before["camera_device"].clone())?)?;
            ensure!(
                before == saved(s, d)?,
                "{name} {boundary} restore changed state"
            );
        }
    }
    anyhow::bail!("button stalled")
}
fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = root.join(std::env::var("SIMS_MOVE_PICKUP_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let source: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-buy-product-frames-1/session.json"),
    )?)?;
    let mut reports = vec![];
    for (name, target, parts, price, destination) in [
        ("chair", 198, vec![198], 150, [376, 488]),
        ("sofa", 174, vec![173, 174], 250, [424, 520]),
    ] {
        let mut finals = vec![];
        for resumed in [false, true] {
            eprintln!("stock pickup {name} resumed={resumed}");
            let mut s =
                Session::restore(&index, serde_json::from_value(source["session"].clone())?)?;
            let mut d =
                Device::restore(&s, serde_json::from_value(source["camera_device"].clone())?)?;
            let mut cold = BTreeSet::new();
            let empty = motion::Input {
                position: [-16, -16],
                screen: [0, 0],
                picked: 0,
            };
            button(
                &mut s,
                &mut d,
                &index,
                &out,
                name,
                resumed,
                &mut cold,
                Input {
                    kind: Kind::Cancel,
                    pointer: empty,
                    shift: false,
                },
            )?;
            let tick = s.simulation.vm.tick;
            let budget = s.simulation.vm.world.budget.clone();
            let original: Vec<_> = parts
                .iter()
                .map(|id| (*id, s.simulation.vm.world.positions[id].clone()))
                .collect();
            let identities: Vec<_> = parts
                .iter()
                .map(|id| {
                    (
                        *id,
                        s.simulation
                            .vm
                            .world
                            .scheduler_storage
                            .as_ref()
                            .unwrap()
                            .handle(*id)
                            .unwrap(),
                    )
                })
                .collect();
            let p = &s.simulation.vm.world.positions[&target];
            let pickup = motion::Input {
                position: [p.x, p.y],
                screen: [200, 300],
                picked: target,
            };
            let moved = motion::Input {
                position: destination,
                screen: [300, 400],
                picked: 0,
            };
            let mut stages = vec![];
            for cancel in [true, false] {
                for kind in [Kind::Down, Kind::Up] {
                    button(
                        &mut s,
                        &mut d,
                        &index,
                        &out,
                        name,
                        resumed,
                        &mut cold,
                        Input {
                            kind,
                            pointer: pickup,
                            shift: false,
                        },
                    )?;
                }
                ensure!(
                    s.host
                        .presentation
                        .move_tool
                        .as_ref()
                        .unwrap()
                        .tool
                        .held
                        .is_some(),
                    "pickup click unexpectedly dropped furniture"
                );
                let selected = d
                    .furniture_selection()
                    .context("actual selected-furniture controls absent")?;
                ensure!(
                    selected.price == price && selected.move_allowed && selected.sell_allowed,
                    "unexpected toolbar: {selected:?}"
                );
                ensure!(
                    s.host
                        .presentation
                        .move_tool
                        .as_ref()
                        .unwrap()
                        .tool
                        .tool
                        .mode
                        == 1
                );
                ensure!(
                    identities.iter().all(|(id, h)| s
                        .simulation
                        .vm
                        .world
                        .scheduler_storage
                        .as_ref()
                        .unwrap()
                        .handle(*id)
                        == Some(*h)),
                    "pickup replaced furniture"
                );
                s.begin_move_tool_motion(moved)?;
                finish_move(&mut s, &mut d)?;
                stages.push(json!({"cancel":cancel,"tool":s.host.presentation.move_tool,"selection":d.furniture_selection(),"positions":parts.iter().map(|id|(*id,&s.simulation.vm.world.positions[id])).collect::<Vec<_>>()}));
                if cancel {
                    button(
                        &mut s,
                        &mut d,
                        &index,
                        &out,
                        name,
                        resumed,
                        &mut cold,
                        Input {
                            kind: Kind::Cancel,
                            pointer: moved,
                            shift: false,
                        },
                    )?;
                    ensure!(
                        original
                            .iter()
                            .all(|(id, p)| s.simulation.vm.world.positions.get(id) == Some(p)),
                        "cancel failed to restore positions"
                    );
                } else {
                    for kind in [Kind::Down, Kind::Up] {
                        button(
                            &mut s,
                            &mut d,
                            &index,
                            &out,
                            name,
                            resumed,
                            &mut cold,
                            Input {
                                kind,
                                pointer: moved,
                                shift: false,
                            },
                        )?;
                    }
                    ensure!(
                        original.iter().all(|(id, p)| s
                            .simulation
                            .vm
                            .world
                            .positions
                            .get(id)
                            .is_some_and(|new| new != p && new.x > 0)),
                        "relocation did not commit"
                    );
                }
                ensure!(
                    s.host
                        .presentation
                        .move_tool
                        .as_ref()
                        .unwrap()
                        .tool
                        .held
                        .is_none()
                );
                ensure!(
                    d.furniture_selection().is_none(),
                    "selected controls survived release"
                );
                ensure!(
                    s.simulation
                        .vm
                        .world
                        .objects
                        .values()
                        .all(|o| o.cursor.is_none())
                );
                ensure!(
                    s.simulation.vm.world.budget == budget && s.simulation.vm.tick == tick,
                    "move charged funds or advanced gameplay"
                );
            }
            let final_state = saved(&s, &d)?;
            let pixels = software::render(d.scene());
            if resumed {
                ensure!(
                    cold == BTreeSet::from_iter(
                        ["factory", "rollback", "release", "selection"].map(str::to_owned)
                    ),
                    "missing cold boundaries {cold:?}"
                );
                std::fs::write(
                    out.join(format!("{name}-session.json")),
                    serde_json::to_vec(&final_state)?,
                )?;
                image::save_buffer(
                    out.join(format!("{name}.png")),
                    &pixels,
                    d.scene().width,
                    d.scene().height,
                    image::ColorType::Rgba8,
                )?;
            }
            reports.push(json!({"name":name,"resumed":resumed,"tick":tick,"budget":budget,"selected_original_part":target,"parts":parts,"price":price,"cold_restores":cold,"cancel_restores_positions":true,"pickup_retains_identity":true,"stages":stages}));
            finals.push((final_state, pixels));
        }
        ensure!(
            finals[0] == finals[1],
            "{name} full state or pixels differ across budgets/restores"
        );
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"cases":reports,"full_state_and_pixels_equal":true,"actual_iced_input":false}),
        )?,
    )?;
    Ok(())
}
