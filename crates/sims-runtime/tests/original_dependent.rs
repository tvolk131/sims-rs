//! Saved-house construction must retain the same HEAD attachment owner as
//! native45962b, before a selected person's ordinary refresh consumes it.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Options, Session};
use sims_sim::{lot_route::path::Policy, pose::portrait::view::Input};
use sims_vm::slot_geometry::Anchor;
#[path = "support/registry_startup.rs"]
mod registry_startup;
#[test]
#[ignore = "requires private original Goth house and character/indicator assets"]
fn saved_goth_head_indicator_survives_live_updates_and_restore() -> Result<()> {
    // Whole-session restore has large typed locals, as in the other private
    // household tests; this changes only this test worker's stack.
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .unwrap()
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut session = Session::load(
        &index,
        "UserData/Houses/House05.iff",
        Options {
            routing: Policy::InferredRectangles,
            ..Default::default()
        },
    )?;
    let actor = session
        .house
        .household
        .as_ref()
        .context("household absent")?
        .residents[0]
        .object as i16;
    let object = &session.simulation.vm.world.objects[&actor];
    eprintln!(
        "saved Goth actor{actor} slots{} geometry{:?}",
        object.slots.len(),
        object.native_slot_geometry
    );
    ensure!(
        object
            .native_slot_geometry
            .as_ref()
            .and_then(|s| s.get(1))
            .and_then(|s| s.anchor.as_ref())
            == Some(&Anchor::Named(b"HEAD".to_vec())),
        "saved-house constructor omitted the native HEAD anchor"
    );
    let camera = Input {
        viewport: [0, 0, 800, 600],
        center: [0, 0],
        zoom: 3,
        rotation: 0,
    };
    session.begin_selection(Some(actor))?;
    let mut selected = false;
    for _ in 0..2000 {
        if session.advance_selection(camera, 10000)? {
            selected = true;
            break;
        }
    }
    ensure!(selected, "selection failed to finish");
    let child =
        session.simulation.vm.world.objects[&actor].slots[1].context("HEAD indicator absent")?;
    session
        .simulation
        .characters
        .get_mut(&actor)
        .context("resident absent")?
        .character
        .visible = true;
    session.simulation.vm.world.globals.insert(17, 0);
    session.simulation.vm.world.globals.insert(19, 0);
    let before = session.simulation.vm.tick;
    for _ in 0..4 {
        registry_startup::step_startup_inputs(&mut session)?;
    }
    ensure!(
        session.simulation.vm.tick > before,
        "selected household did not advance"
    );
    ensure!(
        session.host.ordinary.graphics[&child]
            .marker_value
            .is_some(),
        "dependent HEAD graphic was not updated"
    );
    ensure!(
        session.host.ordinary.graphics[&child]
            .ready
            .frame
            .placement
            .explicit,
        "HEAD indicator did not retain its explicit transform"
    );
    let saved = session.save();
    // Negative control: a snapshot with an unknown current anchor must still
    // stop at the original owner guard. Do not silently substitute HEAD here.
    let mut unknown = Session::restore(&index, saved.clone())?;
    unknown
        .simulation
        .vm
        .world
        .objects
        .get_mut(&actor)
        .unwrap()
        .native_slot_geometry
        .as_mut()
        .unwrap()[1]
        .anchor = None;
    let error = registry_startup::step_startup_inputs(&mut unknown).unwrap_err();
    ensure!(
        format!("{error:#}").contains("dependent slot anchor owner is unobserved"),
        "unexpected missing-owner failure: {error:#}"
    );
    eprintln!("Unknown HEAD anchor negative control rejected: {error:#}");
    drop(unknown);
    let mut cold = Session::restore(
        &index,
        serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
    )?;
    registry_startup::step_startup_inputs(&mut session)?;
    registry_startup::step_startup_inputs(&mut cold)?;
    ensure!(
        session.simulation == cold.simulation && session.host.snapshot() == cold.host.snapshot(),
        "restored dependent update differs"
    );
    Ok(())
}
