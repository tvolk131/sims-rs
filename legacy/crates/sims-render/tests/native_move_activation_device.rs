//!Original catalog activation through the live cleanup/history owner.
use anyhow::{Context, Result, ensure};
use sims_content::{Index, object_catalog::Catalog};
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session, move_tool};
use sims_sim::{
    edit_history::{object, snapshot},
    object_catalog::move_tool::{Groups, cleanup},
};
use std::{collections::BTreeSet, sync::Arc};

#[test]
#[ignore = "private original-house checkpoint; release/headless"]
fn original_catalog_activation_runs_live_cleanup_before_classification_and_cold_resumes()
-> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("move activation panicked")
}
fn cold(index: &Index, s: &mut Session) -> Result<()> {
    let saved = s.save();
    *s = Session::restore(index, serde_json::from_slice(&serde_json::to_vec(&saved)?)?)?;
    ensure!(serde_json::to_value(saved)? == serde_json::to_value(s.save())?);
    Ok(())
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_MOVE_ACTIVATION_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Arc::new(Index::scan(&root.join("game-data/cabinet-extracted"))?);
    let mut catalog = Catalog::household(
        index.clone(),
        "UserData/Neighborhood.iff",
        Default::default(),
    )?;
    let mut groups = Groups::default();
    catalog
        .populate_move_tool_groups(&mut groups)
        .map_err(anyhow::Error::msg)?;
    let mut template = move_tool::History::new(1);
    template.retain_catalog(&mut catalog)?;
    template
        .store
        .construct_manager(5001)
        .map_err(anyhow::Error::msg)?;
    let seed: Saved = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-cursor-destruction-frames-1/goth-false-session.json"),
    )?)?;
    let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-ui-timer-dispatch-iced/goth/resumed.json"),
    )?)?;
    let camera: sims_render::camera_device::Saved =
        serde_json::from_value(wrapper["camera_device"].clone())?;
    let mut choices = vec![];
    for (n, g) in groups.0.iter().enumerate() {
        let id = groups
            .lookup(n, g.first, &catalog.definitions)
            .map_err(anyhow::Error::msg)?
            .context("original group empty")?;
        choices.push((g.first as u32, Some(id), n as i32 + 3));
    }
    //The original tests OBJD+c0 against simulator+44 (global26). Use an
    //actual catalog row satisfying that predicate; upper argument bits drop.
    let row = catalog
        .definitions
        .entries
        .iter()
        .position(|e| i16::from_le_bytes(e.data[0xc0..0xc2].try_into().unwrap()) == 1)
        .context("catalog lacks selection-gated object")?;
    ensure!(row <= i16::MAX as usize);
    choices.push((0x12340000 | row as u32, Some(row as u32 + 1), 2));
    choices.push((u32::MAX, None, 1));
    let mut finals = vec![];
    let mut runs = vec![];
    for resumed in [false, true] {
        eprintln!("live move activation resumed={resumed}");
        let mut s = Session::restore(&index, seed.clone())?;
        let mut history = template.clone();
        //An explicit empty ObjectUndoable has real Stop/Release/destructor
        //work, without pretending it was imported from a Windows UI stack.
        object::construction::construct(
            object::construction::Input {
                command: 7002,
                script_id: 0,
                kind: 1,
                cost: 0,
                budget: 8001,
            },
            &mut history.store,
            &mut history.objects,
            &mut history.references,
            &mut snapshot::live::View {
                world: &mut s.simulation.vm.world,
                program: &s.program,
                links: &s.simulation.native_lot.as_ref().unwrap().placement.links,
                definition: &|_| Err("null snapshot cannot read definition".into()),
            },
            &mut || Err("null snapshot cannot allocate children".into()),
        )
        .map_err(anyhow::Error::msg)?;
        history.store.acquire(7002).map_err(anyhow::Error::msg)?;
        let mut tool = cleanup::State::constructor_fields(5001);
        tool.payload = Some(7002);
        let state = move_tool::State::new(
            tool,
            cleanup::Inputs {
                registry: 5002,
                window: 0,
                key_function: 0,
                control_state: 0,
            },
            history,
        );
        s.bind_move_tool(state)?;
        let unbound = serde_json::to_value(s.save())?;
        ensure!(s.begin_move_tool_activation(choices[0].0).is_err());
        ensure!(serde_json::to_value(s.save())? == unbound);
        s.host
            .presentation
            .move_tool
            .as_mut()
            .unwrap()
            .retain_activation_groups(groups.clone())?;
        let initial = s.simulation.clone();
        let mut device = Device::restore(&s, camera.clone())?;
        let image = software::render(device.scene());
        let mut restored = BTreeSet::new();
        let mut fields = vec![];
        for (choice, &(argument, expected, mode)) in choices.iter().enumerate() {
            s.begin_move_tool_activation(argument)?;
            ensure!(s.begin_move_tool_activation(argument).is_err());
            ensure!(s.step().is_err());
            if resumed && choice == 0 {
                let mut broken = serde_json::to_value(s.save())?;
                broken["view"]["move_tool"]["groups"] = serde_json::Value::Null;
                ensure!(Session::restore(&index, serde_json::from_value(broken)?).is_err());
                cold(&index, &mut s)?;
                restored.insert("entry");
            }
            for iteration in 0..1000 {
                let step = s.advance_move_tool(if resumed { 1 } else { 1000 })?;
                ensure!(step.instructions == 0);
                let state = s.host.presentation.move_tool.as_ref().unwrap();
                if resumed && choice == 0 {
                    let label = if state.tool.payload == Some(7002)
                        && !state.history.store.commands.contains_key(&7002)
                    {
                        Some("retired payload")
                    } else {
                        None
                    };
                    if let Some(label) = label.filter(|label| restored.insert(label)) {
                        eprintln!("restoring {label}");
                        cold(&index, &mut s)?;
                    }
                }
                match step.progress {
                    move_tool::Progress::Complete => break,
                    move_tool::Progress::Pending => (),
                    other => anyhow::bail!("unbound activation child: {other:?}"),
                }
                ensure!(iteration < 999);
            }
            let state = s.host.presentation.move_tool.as_ref().unwrap();
            ensure!(state.tool.payload.is_none() && state.tool.held.is_none());
            ensure!(!state.history.store.commands.contains_key(&7002));
            ensure!(!state.history.objects.0.contains_key(&7002));
            ensure!(state.tool.tool.definition == expected && state.tool.tool.mode == mode);
            //Deactivation stores-1 at+38; classification uses the caller's
            //retained argument, not that overwritten tool field.
            ensure!(state.tool.tool.argument == -1);
            if choice == 7 {
                ensure!(
                    state.tool.tool.flags[1] == u8::from(s.simulation.vm.world.globals[&26] == 0)
                );
            }
            fields.push(serde_json::to_value(&state.tool)?);
            s.finish_move_tool_cleanup()?;
            ensure!(
                s.simulation == initial,
                "activation changed VM/physical state"
            );
        }
        if resumed {
            ensure!(restored.contains("retired payload"));
            cold(&index, &mut s)?;
            restored.insert("complete");
        }
        device.refresh(&s)?;
        ensure!(software::render(device.scene()) == image);
        if resumed {
            image.save(out.join("goth.png"))?;
            std::fs::write(out.join("session.json"), serde_json::to_vec(&s.save())?)?;
        }
        runs.push(
            serde_json::json!({"resumed":resumed,"choices":choices,"fields":fields,
            "restored":restored,"simulation_unchanged":true,"pixels_unchanged":true}),
        );
        finals.push(serde_json::to_value(s.save())?);
    }
    ensure!(finals[0] == finals[1]);
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
        "runs":runs,"equal":true,"scope":"Actual original household/catalog and live activation/empty command cleanup. Explicit tool fixture; no full Buy UI or normal tick advancement."}))?,
    )?;
    Ok(())
}
