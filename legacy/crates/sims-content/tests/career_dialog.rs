//! A bounded primitive regression using recorded household inputs. This does not
//! resume an old full-program checkpoint or establish a new household frontier.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_content::{Index, saves::House};
use sims_vm::{Frame, Start, Status, Vm, World};

#[test]
#[ignore = "requires original assets and the private v64 Mashuga dialog-boundary input"]
fn recorded_mashuga_promotion_formats_through_the_original_dialog_primitive() -> Result<()> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = "UserData2/Houses/House09.iff";
    let input = root.join("cache/neighborhood-input-experiments/mashuga-empty-career-text-v64/original-live-character-final.json");
    let report: Value = serde_json::from_slice(&std::fs::read(&input)?)?;
    ensure!(report["source"] == source);
    ensure!(
        report["boundary"]
            .as_str()
            .unwrap()
            .contains("$TimeLocal:2")
    );
    let state = &report["simulation"]["state"]["vm"];
    let recorded: Frame = serde_json::from_value(
        state["threads"]["12"]["stack"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone(),
    )?;
    ensure!(recorded.location.bhav == 4103 && recorded.location.instruction == 5);
    ensure!(recorded.caller == 195 && recorded.opaque_locals.is_empty());
    ensure!(recorded.locals == [130, 0, 23, 5, 14, 1, 12, 70, 260]);
    let world: World = serde_json::from_value(state["world"].clone())?;
    ensure!(world.dialogs.records.is_empty() && world.dialogs.modal.is_none());
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let house = House::load(&index, source)?;
    let program = house.link_program(&index)?;
    // Reconstruct only this primitive's explicit inputs in a fresh VM/frame.
    // Preserve fresh frame-storage ownership, rather than relabel an old save.
    let mut vm = Vm::new(world, 1);
    let id = vm
        .spawn(
            &program,
            Start {
                owner: &recorded.code_owner,
                bhav: recorded.location.bhav,
                caller: recorded.caller,
                callee: recorded.callee,
                stack_object: recorded.stack_object,
                args: recorded.args.clone(),
                check: false,
            },
        )
        .map_err(anyhow::Error::msg)?;
    let frame = vm.threads.get_mut(&id).unwrap().stack.last_mut().unwrap();
    ensure!(frame.locals.len() == recorded.locals.len());
    frame.locals.clone_from(&recorded.locals);
    frame.location = recorded.location.clone();
    vm.run_slice(&program, id, 1).map_err(anyhow::Error::msg)?;
    ensure!(
        !matches!(vm.threads[&id].status, Status::Faulted(_)),
        "{:?}",
        vm.threads[&id].status
    );
    ensure!(vm.world.dialogs.records.len() == 1);
    let record = &vm.world.dialogs.records[0];
    ensure!(record.caller == 195);
    ensure!(record.body.contains("Conspiracy Theorist"));
    ensure!(record.body.contains("11:00 pm to  5:00 am"));
    ensure!(record.body.contains("$130") && record.body.contains("$260"));
    ensure!(!record.body.contains("$Job") && !record.body.contains("$TimeLocal"));
    std::fs::write(
        root.join("cache/native-mashuga-promotion-dialog-v65.json"),
        serde_json::to_vec_pretty(&json!([{
            "source":source,
            "scope":"isolated original Dialog primitive with recorded v64 inputs; not a continued household replay",
            "input_report":input,
            "input_neighborhood_experiment":report["neighborhood_input_experiment"],
            "input_frame":recorded,
            "dialogs":[{"record":record,"response":null}],
        }]))?,
    )?;
    println!(
        "{}: original promotion dialog formatted; {} bytes; icon {:?}; no response supplied",
        record.title,
        record.body.len(),
        record.job_icon
    );
    Ok(())
}
