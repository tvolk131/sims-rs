//!Actual421986 composition in a stock house; no synthetic lifecycle replies.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::Session;
use std::collections::BTreeSet;

#[test]
#[ignore = "private original Buy checkpoint and installed assets; release/headless"]
fn original_purchase_creates_furniture_then_cursor_and_cleans_up_failure_across_restore()
-> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("purchase test panicked")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = root.join(std::env::var("SIMS_PURCHASE_CURSOR_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let source: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-buy-product-frames-1/session.json"),
    )?)?;
    let base = Session::restore(&index, serde_json::from_value(source["session"].clone())?)?;
    let tool = base
        .host
        .presentation
        .move_tool
        .as_ref()
        .context("selected tool absent")?;
    let chair = tool.history.definitions[&tool
        .tool
        .tool
        .definition
        .context("chair definition absent")?]
        .clone();
    let cursor = tool
        .history
        .definitions
        .values()
        .find(|d| d.source.ends_with("Cursors.iff") && d.object == 128)
        .context("actual Tile Cursor not registered")?
        .clone();
    let products = base.buy_products()?;
    let sofa = products
        .iter()
        .find(|p| p.source.source.ends_with("Sofas.iff") && p.fields.subindex == -1)
        .context("original sofa absent")?
        .source
        .clone();
    let original_ids: BTreeSet<_> = base.simulation.vm.world.objects.keys().copied().collect();
    let original_budget = base.simulation.vm.world.budget.clone();
    let original_tick = base.simulation.vm.tick;
    let initial_pixels = software::render(
        Device::restore(
            &base,
            serde_json::from_value(source["camera_device"].clone())?,
        )?
        .scene(),
    );
    initial_pixels.save(out.join("before.png"))?;
    let mut reports = vec![];
    for (name, definition, cursor_definition) in [
        ("chair", Some(chair.clone()), Some(cursor.clone())),
        ("sofa", Some(sofa), Some(cursor.clone())),
        ("failed-chair", Some(chair), None),
        ("null", None, None),
    ] {
        let mut finals = vec![];
        let mut runs = vec![];
        for resumed in [false, true] {
            eprintln!("purchase {name}, restored={resumed}");
            let mut s =
                Session::restore(&index, serde_json::from_value(source["session"].clone())?)?;
            let mut d =
                Device::restore(&s, serde_json::from_value(source["camera_device"].clone())?)?;
            s.begin_purchase_cursor(definition.clone(), cursor_definition.clone(), 1, false)?;
            let mut restored = BTreeSet::new();
            let mut slices = 0;
            let mut created_furniture = None;
            let mut creation_order = vec![];
            for n in 0..10000 {
                slices += 1;
                ensure!(s.begin_clock_frame().is_err());
                let step = if s.house_notification_pending() {
                    d.advance_notification(&mut s, if resumed { 1 } else { 256 })
                        .map(|_| false)
                } else {
                    s.advance_purchase_cursor(if resumed { 1 } else { 256 })
                };
                if let Err(e) = step {
                    std::fs::write(
                        out.join(format!("{name}-error.json")),
                        serde_json::to_vec(
                            &json!({"session":s.save(),"camera_device":d.save(&s)?}),
                        )?,
                    )?;
                    return Err(e).with_context(|| format!("{name} slice {n}"));
                }
                let owner = s.host.presentation.purchase_cursor.as_ref().unwrap();
                created_furniture = created_furniture.or(owner.furniture());
                let phase = serde_json::to_value(owner)?["phase"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                if s.house_notification_pending() {
                    let label = if phase == "Furniture" {
                        "furniture"
                    } else {
                        "cursor"
                    };
                    if creation_order.last() != Some(&label) {
                        creation_order.push(label);
                    }
                    if phase == "Cursor" {
                        ensure!(
                            s.simulation
                                .vm
                                .world
                                .objects
                                .iter()
                                .filter(|(id, _)| !original_ids.contains(id))
                                .filter_map(|(_, o)| o.cursor.as_ref())
                                .all(|c| c.purchase == 0),
                            "purchase mark preceded cursor creation notification"
                        );
                    }
                }
                let boundary = if s.house_notification_pending() {
                    format!("{phase}-notice")
                } else if phase == "Furniture"
                    && s.simulation
                        .vm
                        .world
                        .objects
                        .keys()
                        .any(|id| !original_ids.contains(id))
                {
                    "Furniture-build".into()
                } else {
                    phase.clone()
                };
                let cold_boundary = match name {
                    "chair" => matches!(
                        boundary.as_str(),
                        "Furniture-build" | "Cursor-notice" | "Mark"
                    ),
                    "sofa" => matches!(boundary.as_str(), "BeginCursor" | "Cursor-notice"),
                    "failed-chair" => boundary == "Remove",
                    _ => false,
                };
                if resumed && cold_boundary && restored.insert(boundary.clone()) {
                    let saved = json!({"session":s.save(),"camera_device":d.save(&s)?});
                    std::fs::write(
                        out.join(format!("{name}-{boundary}.json")),
                        serde_json::to_vec(&saved)?,
                    )?;
                    s = Session::restore(
                        &index,
                        serde_json::from_value(saved["session"].clone())?,
                    )?;
                    d = Device::restore(
                        &s,
                        serde_json::from_value(saved["camera_device"].clone())?,
                    )?;
                    ensure!(
                        json!({"session":s.save(),"camera_device":d.save(&s)?}) == saved,
                        "cold restore changed {boundary}"
                    );
                }
                if s.host
                    .presentation
                    .purchase_cursor
                    .as_ref()
                    .unwrap()
                    .complete()
                {
                    break;
                }
                ensure!(n < 9999, "purchase wrapper stalled");
            }
            let result = s.finish_purchase_cursor()?;
            ensure!(
                creation_order
                    == match name {
                        "chair" | "sofa" => vec!["furniture", "cursor"],
                        "failed-chair" => vec!["furniture"],
                        _ => vec![],
                    },
                "creation notification order differs: {creation_order:?}"
            );
            ensure!(!s.purchase_cursor_pending() && !s.house_notification_pending());
            ensure!(
                s.simulation.vm.tick == original_tick
                    && s.simulation.vm.world.budget == original_budget
            );
            let world = &s.simulation.vm.world;
            let added: Vec<_> = world
                .objects
                .keys()
                .filter(|id| !original_ids.contains(id))
                .copied()
                .collect();
            let mut marked = vec![];
            if let Some(id) = result {
                ensure!(cursor_definition.is_some());
                let mut next = Some(id);
                while let Some(id) = next {
                    let o = &world.objects[&id];
                    let c = o.cursor.as_ref().context("returned chain is not cursor")?;
                    ensure!(c.purchase == 1 && c.postconstructed);
                    ensure!(
                        world
                            .scheduler_storage
                            .as_ref()
                            .unwrap()
                            .object_for_handle(c.underlying.context("new cursor has no furniture")?)
                            .is_some()
                    );
                    marked.push(id);
                    next = o.multitile.as_ref().context("cursor links absent")?.next;
                }
                //This installed catalog entry constructs parts16813/16814.
                ensure!(marked.len() == if name == "sofa" { 2 } else { 1 });
                ensure!(added.len() >= 2 * marked.len());
                ensure!(
                    s.host.presentation.object_focus
                        == world.scheduler_storage.as_ref().unwrap().handle(id)
                );
                if resumed {
                    ensure!(
                        restored.contains("Cursor-notice")
                            && if name == "chair" {
                                restored.contains("Furniture-build") && restored.contains("Mark")
                            } else {
                                restored.contains("BeginCursor")
                            }
                    );
                }
            } else {
                ensure!(cursor_definition.is_none() && added.is_empty());
                if name == "failed-chair" {
                    ensure!(
                        created_furniture.is_some()
                            && !world.objects.contains_key(&created_furniture.unwrap())
                    );
                    if resumed {
                        ensure!(restored.contains("Remove"));
                    }
                }
            }
            d.refresh(&s)?;
            let pixels = software::render(d.scene());
            let mut saved = source.clone();
            saved["session"] = serde_json::to_value(s.save())?;
            saved["camera_device"] = serde_json::to_value(d.save(&s)?)?;
            if resumed {
                pixels.save(out.join(format!("{name}.png")))?;
                std::fs::write(
                    out.join(format!("{name}-session.json")),
                    serde_json::to_vec(&saved)?,
                )?;
            }
            finals.push((saved, pixels));
            runs.push(json!({"resumed":resumed,"cold_boundaries":restored,"slices":slices,"creation_order":creation_order,"created_furniture":created_furniture,"returned":result,"marked":marked,"added":added}));
        }
        ensure!(
            finals[0] == finals[1],
            "{name} differs across budgets/cold restore"
        );
        reports.push(json!({"name":name,"runs":runs,"full_state_and_pixels_equal":true}));
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"tick":original_tick,"budget_unchanged":true,"original_bytecode":true,"native_execution":false,"actual_iced_input":false,"placement_commit":false,"cases":reports}),
        )?,
    )?;
    Ok(())
}
