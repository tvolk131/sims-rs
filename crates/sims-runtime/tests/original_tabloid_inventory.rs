//! Authored entry into the shipped Tabloid4125 subroutine on ordinarily loaded
//! Goth residents. This proves its inventory operation and managed continuation,
//! not natural newspaper scheduling or a complete tick of gameplay parity.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Options, Session};
use sims_vm::{ObjectId, Stop, person::InventoryEntry};
use std::collections::BTreeSet;

#[test]
#[ignore = "requires private original Goth/Tabloid resources; headless authored script entry and managed cold resume"]
fn ordinary_goth_tabloid_removes_tokens_and_cold_resumes() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(tabloid)?
        .join()
        .expect("original Tabloid inventory test panicked")
}

fn resident(session: &Session, name: &str) -> Result<ObjectId> {
    session
        .house
        .household
        .as_ref()
        .context("Goth household absent")?
        .residents
        .iter()
        .map(|r| r.object as ObjectId)
        .find(|id| {
            session.simulation.vm.world.objects[id]
                .definition
                .as_ref()
                .is_some_and(|d| d.source.ends_with(name))
        })
        .with_context(|| format!("Goth resident {name} absent"))
}

fn tabloid() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut warm = Session::load(&index, "UserData/Houses/House05.iff", Options::default())
        .context("ordinary Goth load")?;
    let caller = resident(&warm, "User00009.iff")?;
    let target = resident(&warm, "User00010.iff")?;
    ensure!(caller != target);
    let source = index.resources[index.find("ExpansionPack6/ExpansionPack6.far::Tabloid.iff")?]
        .locator
        .clone();
    let loaded = sims_content::vm::load_program(&index, &source, "Global.iff")?;
    ensure!(loaded.globals == warm.program.globals);
    for (name, module) in loaded.modules {
        if let Some(old) = warm.program.modules.get(&name) {
            ensure!(
                old.content_hash == module.content_hash,
                "changed original module {name}"
            );
        } else {
            warm.program.modules.insert(name, module);
        }
    }
    // Match the existing lazy-resource link contract before any managed save;
    // adding OBJD resources also extends the architecture's static catalog.
    warm.simulation
        .architecture
        .as_mut()
        .context("Goth architecture absent")?
        .bind_footprints(&warm.program)
        .map_err(anyhow::Error::msg)?;
    let (_, routine) = warm
        .program
        .resolve(&source, 4125)
        .map_err(anyhow::Error::msg)?;
    ensure!(routine.name == "Remove All Tokens");
    ensure!(routine.body.args == 1 && routine.body.locals == 0);
    let instructions = &routine.body.instructions;
    // The complete original routine uses caller temps and argument0 only. It
    // neither dereferences callee/stack-object fields nor expects prior locals.
    // Thus a loaded person may own this explicitly authored diagnostic frame.
    ensure!(instructions.len() == 14);
    ensure!(
        instructions[0].opcode == 2 && instructions[0].operands == [0, 0, 255, 255, 0, 5, 8, 7]
    );
    ensure!(instructions[12].opcode == 2 && instructions[12].operands == [4, 0, 0, 0, 0, 5, 8, 9]);
    let mut tokens = vec![];
    for (pc, instruction) in instructions
        .iter()
        .enumerate()
        .filter(|(pc, _)| ![0, 12].contains(pc))
    {
        ensure!(
            instruction.opcode == 0x33 && instruction.operands[..4] == [1, 6, 3, 0x29],
            "unexpected Tabloid4125 pc{pc}"
        );
        tokens.push(u32::from_le_bytes(
            instruction.operands[4..].try_into().unwrap(),
        ));
    }
    ensure!(tokens.len() == 12 && tokens.iter().copied().collect::<BTreeSet<_>>().len() == 12);
    ensure!(tokens[0] == 0x52e387db);
    let hash = warm.program.modules[&source].content_hash.clone();
    let caller_guid = warm.simulation.vm.world.objects[&caller]
        .definition
        .as_ref()
        .unwrap()
        .guid;
    let target_guid = warm.simulation.vm.world.objects[&target]
        .definition
        .as_ref()
        .unwrap()
        .guid;
    let people = warm
        .simulation
        .vm
        .world
        .person
        .as_mut()
        .context("ordinary neighborhood owner absent")?;
    let caller_neighbor = people
        .neighbors
        .iter()
        .find(|(_, n)| n.guid == caller_guid)
        .map(|(&id, _)| id)
        .context("caller neighbor absent")?;
    let target_neighbor = people
        .neighbors
        .iter()
        .find(|(_, n)| n.guid == target_guid)
        .map(|(&id, _)| id)
        .context("target neighbor absent")?;
    ensure!(caller_neighbor != target_neighbor);
    let retained = vec![
        InventoryEntry {
            kind: 5,
            guid: tokens[0],
            count: 8,
        },
        InventoryEntry {
            kind: 6,
            guid: 0xdeadbeef,
            count: 9,
        },
    ];
    let mut entries = vec![retained[0].clone()];
    entries.extend(tokens.iter().enumerate().map(|(i, &guid)| InventoryEntry {
        kind: 6,
        guid,
        count: 2 + i as i16,
    }));
    entries.push(retained[1].clone());
    // These are explicit test inputs, not captured original inventory contents.
    people
        .neighbors
        .get_mut(&caller_neighbor)
        .unwrap()
        .inventory = entries.clone();
    people
        .neighbors
        .get_mut(&target_neighbor)
        .unwrap()
        .inventory = entries;
    people.inventory_dirty = false;
    let mut expected_world = warm.simulation.vm.world.clone();
    expected_world
        .person
        .as_mut()
        .unwrap()
        .neighbors
        .get_mut(&target_neighbor)
        .unwrap()
        .inventory = retained;
    expected_world.person.as_mut().unwrap().inventory_dirty = true;
    expected_world.objects.get_mut(&caller).unwrap().temps[0] = -1;
    expected_world.objects.get_mut(&caller).unwrap().temps[4] = target;
    let rng = (
        warm.simulation.vm.rng_state,
        warm.simulation.vm.lot_rng_state,
        warm.simulation.vm.crt_rng_state,
        warm.simulation.vm.lot_rng_scope.clone(),
    );
    let tick = warm.simulation.vm.tick;
    let mut immutable = index.clone();
    immutable
        .resources
        .retain(|r| !r.locator.to_ascii_lowercase().starts_with("userdata"));
    let mut cold = Session::restore(
        &immutable,
        serde_json::from_slice(&serde_json::to_vec(&warm.save_managed()?)?)?,
    )
    .context("initial authored inventory managed restore")?;
    ensure!(warm.simulation.vm == cold.simulation.vm);
    let mut ids = vec![];
    for session in [&mut warm, &mut cold] {
        ids.push(
            session
                .simulation
                .vm
                .spawn(
                    &session.program,
                    sims_vm::Start {
                        owner: &source,
                        bhav: 4125,
                        caller,
                        callee: caller,
                        stack_object: target,
                        args: vec![target],
                        check: false,
                    },
                )
                .map_err(anyhow::Error::msg)?,
        );
    }
    ensure!(ids[0] == ids[1]);
    let thread = ids[0];
    let mut removals = 0;
    let mut mid_resume = false;
    let mut done = false;
    for step in 0..32 {
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
            "Tabloid continuation differs at step{step}"
        );
        ensure!(a.trace.len() == 1);
        if a.trace[0].opcode == 0x33 {
            removals += 1;
        }
        if step == 5 {
            cold = Session::restore(
                &immutable,
                serde_json::from_slice(&serde_json::to_vec(&cold.save_managed()?)?)?,
            )
            .context("mid-Tabloid managed restore")?;
            ensure!(
                warm.simulation.vm == cold.simulation.vm,
                "mid-Tabloid managed resume changed VM/inventory"
            );
            mid_resume = true;
        }
        if a.stop == Stop::Returned(true) {
            done = true;
            break;
        }
        ensure!(
            a.stop == Stop::Budget,
            "Tabloid script did not complete: {:?}",
            a
        );
    }
    ensure!(done && mid_resume && removals == 12);
    ensure!(
        warm.simulation.vm.world == expected_world,
        "Tabloid changed state beyond selected inventory, UI dirty bit and script temps"
    );
    ensure!(warm.simulation.vm.tick == tick);
    ensure!(
        (
            warm.simulation.vm.rng_state,
            warm.simulation.vm.lot_rng_state,
            warm.simulation.vm.crt_rng_state,
            warm.simulation.vm.lot_rng_scope.clone()
        ) == rng
    );
    let resumed = Session::restore(
        &immutable,
        serde_json::from_slice(&serde_json::to_vec(&cold.save_managed()?)?)?,
    )
    .context("completed Tabloid managed restore")?;
    ensure!(
        resumed.simulation.vm == warm.simulation.vm,
        "finished Tabloid inventory was lost on managed cold resume"
    );
    println!(
        "original Tabloid4125 {hash}; caller{caller}, selected Goth person{target}; {removals} original removals, mid-routine and completed managed cold resume; RNG/tick retained"
    );
    Ok(())
}
