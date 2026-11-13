//! Shared cache ownership across actual resumable normal-frame classification.
//! Private fresh-load input; explicit test interleavings are not native ticks.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Session, presenter::Progress};

#[test]
#[ignore = "requires the private freshly imported registry-owner checkpoint"]
fn shared_registry_frame_retains_writers_and_cold_continuations() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("registry frame test panicked"))?
}

fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index =
        Index::scan(&root.join("cache/native-parity-guest-20261023-second/assets-immutable"))?;
    ensure!(
        index
            .resources
            .iter()
            .all(|r| !r.locator.to_ascii_lowercase().starts_with("userdata"))
    );
    let input = root.join(
        "cache/native-registry-order-20261024-first/zero-replay-first/opened-managed-session.json",
    );
    let mut warm = Session::restore(
        &index,
        serde_json::from_reader(std::fs::File::open(input)?)?,
    )?;
    ensure!(warm.simulation.vm.world.registry_order.is_some());
    ensure!(
        warm.simulation
            .native_lot
            .as_ref()
            .unwrap()
            .placement
            .spatial_dirty
            .is_none()
    );
    ensure!(
        warm.host
            .presentation
            .presenter
            .as_ref()
            .unwrap()
            .order
            .is_none()
    );
    let camera = warm.retained_camera_input()?;
    let tick_rng = clocks(&warm);

    //Reject a second supplied owner before changing any retained state.
    let p = warm.host.presentation.presenter.as_ref().unwrap().clone();
    let before = serde_json::to_vec(&warm.save())?;
    ensure!(
        warm.begin_view_refresh(
            p.view,
            p.inputs,
            sims_vm::presenter::Order {
                dirty: true,
                rotation: 0,
                objects: vec![],
            }
        )
        .is_err()
    );
    ensure!(serde_json::to_vec(&warm.save())? == before);

    //Force an existing drawing root through the actual classification child.
    //This controlled test input is not claimed as an original gameplay event.
    let target = warm
        .simulation
        .vm
        .world
        .registry_order
        .as_ref()
        .unwrap()
        .objects
        .iter()
        .copied()
        .find(|id| {
            let w = &warm.simulation.vm.world;
            let o = &w.objects[id];
            o.container.is_none()
                && o.data.get(&34) == Some(&0)
                && o.native_keep_dynamic == Some(0)
                && w.refresh.as_ref().unwrap().draw[id].dynamic
                && w.refresh.as_ref().unwrap().draw[id].level == 1
                && w.positions[id].level == 1
        })
        .context("classification drawing root absent")?;
    warm.simulation
        .vm
        .world
        .refresh
        .as_mut()
        .unwrap()
        .draw
        .get_mut(&target)
        .unwrap()
        .dynamic = false;
    warm.host
        .presentation
        .presenter
        .as_mut()
        .unwrap()
        .inputs
        .force_dynamic = true;
    warm.simulation.vm.world.invalidate_registry_order();
    warm.begin_normal_view_frame()?;
    let mut cold = restore(&index, &warm)?;
    let mut phases = std::collections::BTreeSet::new();
    let mut restored = std::collections::BTreeSet::new();
    let mut interleaved = false;
    let mut complete = false;
    for _ in 0..10000 {
        let owner = serde_json::to_value(warm.host.presentation.presenter.as_ref().unwrap())?;
        let phase = owner["normal_frame"]["phase"]
            .as_str()
            .context("normal frame ended early")?;
        phases.insert(phase.to_owned());
        let classification: Option<sims_vm::presenter::Classification> =
            serde_json::from_value(owner["classification"].clone())?;
        let pending = classification.as_ref().and_then(|c| c.pending()) == Some(target);
        if !interleaved && pending {
            //A registry writer after440349 must survive the rest of4aacb9.
            //It must not be cleared by repeatedly requesting the getter.
            warm.simulation.vm.world.invalidate_registry_order();
            cold.simulation.vm.world.invalidate_registry_order();
            interleaved = true;
        }
        let checkpoint = if pending {
            "pending_invalidation"
        } else {
            phase
        };
        if (phase == "Environment" || phase == "Classification" || pending)
            && restored.insert(checkpoint.to_owned())
        {
            cold = restore(&index, &warm)?;
        }
        let a = warm.advance_view_refresh(camera, 1)?;
        let b = cold.advance_view_refresh(camera, 1)?;
        ensure!(a == b, "restored normal-frame outcome differs");
        ensure!(
            serde_json::to_vec(&warm.save())? == serde_json::to_vec(&cold.save())?,
            "restored normal-frame state differs"
        );
        ensure!(clocks(&warm) == tick_rng);
        if a == Progress::Complete {
            complete = true;
            break;
        }
        ensure!(a == Progress::Pending);
    }
    ensure!(
        complete && interleaved,
        "did not cover the real suspended invalidation"
    );
    ensure!(phases.len() == 3 && restored.contains("pending_invalidation"));
    ensure!(
        warm.simulation
            .vm
            .world
            .registry_order
            .as_ref()
            .unwrap()
            .dirty
    );
    warm.prepare_view_frame()?;
    cold.prepare_view_frame()?;
    ensure!(
        !warm
            .simulation
            .vm
            .world
            .registry_order
            .as_ref()
            .unwrap()
            .dirty
    );
    ensure!(serde_json::to_vec(&warm.save())? == serde_json::to_vec(&cold.save())?);
    ensure!(clocks(&warm) == tick_rng);
    //Passive rendering and managed restoration cannot consume the next frame.
    let saved = serde_json::to_vec(&warm.save())?;
    let mut house = sims_render::original_house::OriginalHouse::default();
    let camera = sims_render::original_house::OriginalHouse::camera(&warm, 800, 600, 0, 0.5, 1);
    let _pixels = sims_render::software::render(&house.scene(&warm, camera)?);
    ensure!(serde_json::to_vec(&warm.save())? == saved);
    let restored_final = restore(&index, &warm)?;
    ensure!(serde_json::to_vec(&restored_final.save())? == saved);
    println!(
        "shared registry: three phases, {} cold checkpoints, retained post-get writer, next-frame rebuild, passive draw, zero ticks/RNG changes",
        restored.len() + 2
    );
    Ok(())
}

fn restore(index: &Index, session: &Session) -> Result<Session> {
    Session::restore(
        index,
        serde_json::from_slice(&serde_json::to_vec(&session.save_managed()?)?)?,
    )
}
fn clocks(s: &Session) -> (u64, u32, Option<u32>, Option<u32>) {
    let vm = &s.simulation.vm;
    (vm.tick, vm.rng_state, vm.lot_rng_state, vm.crt_rng_state)
}
