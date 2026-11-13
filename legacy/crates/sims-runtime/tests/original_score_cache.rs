//! Ordinary original-house loading must publish the personal score owner that
//! Find Best Action consumes. These tests never attach captured score caches.
use anyhow::{Context, Result, ensure};
use sims_content::{Index, house_loading::AnimatorEffect, saves::House};
use sims_runtime::original_session::{Options, Session};
use sims_vm::{ObjectId, motives::score_owner::Owner};
use std::collections::{BTreeMap, BTreeSet};

fn index() -> Result<Index> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    Index::scan(&root.join("game-data/cabinet-extracted"))
}

#[test]
#[ignore = "requires the private original Goth household and score resources"]
fn goth_postload_publishes_scores_between_animator_callbacks() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(postload_order)?
        .join()
        .expect("post-load score test panicked")
}

fn postload_order() -> Result<()> {
    let index = index()?;
    let house = House::load(&index, "UserData/Houses/House05.iff")?;
    let mut before = BTreeMap::new();
    let mut observed = vec![];
    let loaded = house.load_frozen_with_animators(
        &index,
        sims_vm::loading::HouseContext {
            simulator: sims_vm::loading::SimulatorContext {
                game_mode: 0,
                retained_global25: 0,
                global30: 1,
            },
            installed_expansions: 0xff,
            performance_class: 2,
        },
        &mut |effect, execution, program, _, _| {
            match effect {
                AnimatorEffect::ResetPlacementCache(id) => {
                    // The mock animator only observes this boundary. Thus the
                    // complete VM may change only by the score publication
                    // between this callback and the following graphics call.
                    ensure!(before.insert(id, execution.vm.clone()).is_none());
                    observed.push(effect);
                }
                AnimatorEffect::Rebuild(id) => {
                    let mut expected = before.remove(&id).context("graphics preceded reset")?;
                    let curves = sims_vm::person_loading::load_score_curves(
                        &execution.vm.world,
                        program,
                        id,
                    )
                    .map_err(anyhow::Error::msg)?;
                    let owner = Owner::capture(&curves).map_err(anyhow::Error::msg)?;
                    ensure!(
                        execution.vm.world.objects[&id]
                            .native_motive_scores
                            .as_ref()
                            == Some(&owner),
                        "person {id}: post-load score cache was not published before graphics"
                    );
                    expected
                        .world
                        .objects
                        .get_mut(&id)
                        .unwrap()
                        .native_motive_scores = Some(owner);
                    ensure!(
                        expected == execution.vm,
                        "score publication changed VM processing, RNG, frames or yields"
                    );
                    observed.push(effect);
                }
                _ => {}
            }
            Ok(())
        },
    )?;
    ensure!(before.is_empty());
    let mut expected_order = vec![];
    for tail in &loaded.resident_tails {
        expected_order.extend([
            AnimatorEffect::ResetPlacementCache(tail.id),
            AnimatorEffect::Rebuild(tail.id),
        ]);
        ensure!(
            loaded.execution.vm.world.objects[&tail.id]
                .native_motive_scores
                .as_ref()
                == Some(&Owner::capture(&tail.scores).map_err(anyhow::Error::msg)?),
            "diagnostic score result and live owner differ"
        );
    }
    ensure!(observed == expected_order);
    ensure!(
        loaded.resident_tails.len() == 3,
        "Goth residents were omitted"
    );
    ensure!(loaded.execution.vm.tick == u64::from(house.simulator.ticks));
    Ok(())
}

fn scores(session: &Session) -> Result<BTreeMap<ObjectId, (Owner, u32)>> {
    let mut result = BTreeMap::new();
    let mut tables = BTreeSet::new();
    for resident in &session
        .house
        .household
        .as_ref()
        .context("household absent")?
        .residents
    {
        let id = resident.object as i16;
        let object = &session.simulation.vm.world.objects[&id];
        let curves = sims_vm::motives::score_owner::current(
            &session.simulation.vm.world,
            &session.program,
            id,
        )
        .map_err(anyhow::Error::msg)?;
        let resource_curves = sims_vm::person_loading::load_score_curves(
            &session.simulation.vm.world,
            &session.program,
            id,
        )
        .map_err(anyhow::Error::msg)?;
        let owner = Owner::capture(&curves).map_err(anyhow::Error::msg)?;
        ensure!(owner == Owner::capture(&resource_curves).map_err(anyhow::Error::msg)?);
        ensure!(object.native_motive_scores.as_ref() == Some(&owner));
        ensure!(curves.len() == 9);
        let person = object.person.as_ref().context("resident person absent")?;
        let profile: [i16; 256] = (0..256)
            .map(|i| {
                person
                    .get(&(i as i16))
                    .copied()
                    .context("person field absent")
            })
            .collect::<Result<Vec<_>>>()?
            .try_into()
            .unwrap();
        // This is the same native scoring operation used when the residential
        // query starts. Reading the owner and scoring must not consume RNG.
        let score = sims_vm::motives::score(
            object.motives.as_ref().context("resident motives absent")?,
            &profile,
            &curves,
            None,
        )
        .map_err(anyhow::Error::msg)?;
        ensure!(score.is_finite());
        tables.insert(sims_vm::person_loading::score_table(profile[58], 0).1);
        result.insert(id, (owner, score.to_bits()));
    }
    ensure!(
        tables == BTreeSet::from([501, 503]),
        "adult/child selection was not covered"
    );
    Ok(result)
}

#[test]
#[ignore = "requires private original Goth house/assets; headless ordinary load and managed cold resume"]
fn ordinary_goth_scores_survive_cold_resume_and_complete_original_query() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(live_resume)?
        .join()
        .expect("ordinary Goth score test panicked")
}

fn live_resume() -> Result<()> {
    let index = index()?;
    let mut warm = Session::load(&index, "UserData/Houses/House05.iff", Options::default())?;
    let before = warm.save();
    let expected_scores = scores(&warm)?;
    ensure!(expected_scores.len() == 3);
    ensure!(
        warm.simulation == before.simulation.state,
        "passive scoring mutated simulation"
    );

    // Restore from a managed save with no original UserData entries available
    // through the resource index. No starting-house import or score injection.
    let mut immutable = index.clone();
    immutable
        .resources
        .retain(|r| !r.locator.to_ascii_lowercase().starts_with("userdata"));
    let encoded = serde_json::to_vec(&warm.save_managed()?)?;
    let mut cold = Session::restore(&immutable, serde_json::from_slice(&encoded)?)?;
    ensure!(scores(&cold)? == expected_scores);
    ensure!(serde_json::to_vec(&warm.save())? == serde_json::to_vec(&cold.save())?);

    // Explicit authored diagnostic entry at the original opcode. Natural Main
    // scheduling is a separate concern: the original Find Best Action body uses
    // caller, check/dispatch mode and the live world, not pre-call locals/args.
    // Spawn supplies correctly sized original frames; the caller/stack object
    // are the loaded person. No score cache, curves or motives are injected.
    let actor = warm
        .house
        .household
        .as_ref()
        .unwrap()
        .residents
        .iter()
        .map(|r| r.object as i16)
        .find(|id| {
            warm.simulation.vm.world.objects[id]
                .definition
                .as_ref()
                .is_some_and(|d| d.source.ends_with("User00009.iff"))
        })
        .context("original Goth User00009 actor absent")?;
    let owner = warm.simulation.vm.world.objects[&actor]
        .definition
        .as_ref()
        .unwrap()
        .source
        .clone();
    let (_, routine) = warm
        .program
        .resolve(&owner, 8233)
        .map_err(anyhow::Error::msg)?;
    let pc = routine
        .body
        .instructions
        .iter()
        .position(|i| i.opcode == 3)
        .context("original PersonGlobals8233 has no Find Best Action")?;
    let mut ids = vec![];
    for session in [&mut warm, &mut cold] {
        let people = session.simulation.vm.world.person.as_mut().unwrap();
        people.free_will = true; // explicit authored process preference
        ensure!(people.controlled_family != 0);
        let context = people
            .autonomy
            .as_ref()
            .context("ordinary residential context absent")?;
        ensure!(context.residential && !context.suppressed);
        ensure!(session.simulation.vm.scheduled_object_visit.is_none());
        let thread = session
            .simulation
            .vm
            .spawn(
                &session.program,
                sims_vm::Start {
                    owner: &owner,
                    bhav: 8233,
                    caller: actor,
                    callee: actor,
                    stack_object: actor,
                    args: vec![],
                    check: false,
                },
            )
            .map_err(anyhow::Error::msg)?;
        let call = session.simulation.vm.threads.get_mut(&thread).unwrap();
        ensure!(!call.check && call.dispatch.is_none() && call.autonomy.is_none());
        call.stack.last_mut().unwrap().location.instruction = pc;
        ids.push(thread);
    }
    ensure!(ids[0] == ids[1]);
    let thread = ids[0];
    let mut resumed_query = false;
    let mut completed = None;
    let mut steps = 0;
    for _ in 0..100_000 {
        let a = warm
            .simulation
            .vm
            .run_slice(&warm.program, thread, 1)
            .map_err(anyhow::Error::msg)?;
        let b = cold
            .simulation
            .vm
            .run_slice(&cold.program, thread, 1)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            a == b && warm.simulation.vm == cold.simulation.vm,
            "original query instruction/state/RNG/cold continuation differs"
        );
        ensure!(
            a.stop != sims_vm::Stop::Faulted,
            "original eligibility/query fault: {:?}",
            a.trace
        );
        steps += a.trace.len();
        if let Some(done) = a
            .trace
            .iter()
            .find(|r| r.opcode == 3 && r.effect.starts_with("Find Best Action evaluated "))
        {
            completed = Some(done.clone());
            break;
        }
        ensure!(
            a.stop == sims_vm::Stop::Budget,
            "original eligibility needs an unsupported host service: {:?}, {:?}",
            a.stop,
            a.trace
        );
        if !resumed_query && cold.simulation.vm.threads[&thread].autonomy.is_some() {
            let saved = sims_vm::Snapshot::capture(&cold.program, &cold.simulation.vm);
            let restored =
                sims_vm::Snapshot::from_json(&serde_json::to_vec(&saved)?, &cold.program)
                    .map_err(anyhow::Error::msg)?;
            ensure!(
                restored == warm.simulation.vm,
                "suspended query cold state differs"
            );
            cold.simulation.vm = restored;
            resumed_query = true;
        }
    }
    let completed = completed.context("original query diagnostic did not finish")?;
    ensure!(
        resumed_query,
        "suspended original query was not cold-restored"
    );
    ensure!(warm.simulation.vm.threads[&thread].autonomy.is_none());
    ensure!(scores(&warm)? == scores(&cold)?);
    println!(
        "Goth ordinary import/managed restore; authored original-query entry actor{actor}, BHAV8233 pc{pc}; {steps} instructions; suspended cold restore; {}",
        completed.effect
    );
    Ok(())
}
