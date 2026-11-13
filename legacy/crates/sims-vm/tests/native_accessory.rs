use serde_json::Value;
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;
fn program(source: u8, flags: u8, index: u8, name: &str, available: bool) -> Program {
    let mut strings = vec![String::new(); usize::from(index) + 1];
    strings[usize::from(index)] = name.into();
    let mut p = Program {
        globals: "global".into(),
        ..Default::default()
    };
    p.accessories.extend(
        ["suit-fork", "mixed-case name", "indexed", "other"].map(|s| (s.into(), BTreeMap::new())),
    );
    p.modules.insert(
        "code".into(),
        Module {
            source: "code".into(),
            routines: BTreeMap::from([(
                8192,
                Routine {
                    name: "accessory".into(),
                    body: Behavior {
                        format: 0x8002,
                        kind: 0,
                        args: 0,
                        locals: 0,
                        instructions: vec![Instruction {
                            opcode: 6,
                            on_true: 254,
                            on_false: 255,
                            operands: [index, source, flags, 0, 0, 0, 0, 0],
                        }],
                    },
                },
            )]),
            ..Default::default()
        },
    );
    p.modules.insert(
        "private".into(),
        Module {
            source: "private".into(),
            semiglobal: Some("code".into()),
            ..Default::default()
        },
    );
    if available {
        p.modules.insert(
            "global".into(),
            Module {
                source: "global".into(),
                strings: BTreeMap::from([(304, strings.clone())]),
                ..Default::default()
            },
        );
        p.modules.insert(
            "private".into(),
            Module {
                source: "private".into(),
                semiglobal: Some("code".into()),
                strings: BTreeMap::from([(304, strings)]),
                ..Default::default()
            },
        );
    }
    p
}
fn run(p: &Program, accessories: Vec<String>) -> (Vm, Run) {
    run_with_host(p, accessories, false)
}
fn run_with_host(p: &Program, accessories: Vec<String>, native: bool) -> (Vm, Run) {
    let mut w = World::default();
    w.objects.insert(
        1,
        Object {
            person: Some(BTreeMap::new()),
            accessories,
            ..Default::default()
        },
    );
    let mut vm = Vm::new(w, 1);
    vm.native_accessory_host = native;
    let id = vm
        .spawn(
            p,
            Start {
                owner: "private",
                bhav: 8192,
                caller: 1,
                callee: 1,
                stack_object: 0,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    // Private resource identity is independent of the executing BHAV location.
    vm.threads
        .get_mut(&id)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .code_owner = "private".into();
    let result = vm.run_slice(p, id, 1).unwrap();
    (vm, result)
}

#[test]
fn native_host_owns_add_and_remove_even_with_metadata_loaded() {
    for source in [0, 2] {
        for flags in [0, 1] {
            let p = program(source, flags, 0, "SUIT-FORK", true);
            let (mut vm, run) = run_with_host(&p, vec!["other".into()], true);
            assert_eq!(run.stop, Stop::AwaitingAccessory);
            let (thread, command) = vm.native_accessory_commands(&p).unwrap().pop().unwrap();
            assert_eq!(command.name, "suit-fork");
            assert_eq!(command.remove, flags == 1);
            assert!(command.request > 0);
            assert!(vm.advance_tick().is_err());
            let saved = Snapshot::capture(&p, &vm);
            vm = serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&saved).unwrap())
                .unwrap()
                .restore(&p)
                .unwrap();
            let before = vm.clone();
            vm.resume_accessory_assets(&p);
            assert_eq!(
                vm, before,
                "metadata alone cannot complete a native binding"
            );
            let mut wrong = command.clone();
            wrong.request += 1;
            assert!(vm.finish_native_accessory(&p, thread, &wrong).is_err());
            assert_eq!(vm, before);
            vm.finish_native_accessory(&p, thread, &command).unwrap();
            assert_eq!(vm.threads[&thread].status, Status::Returned(true));
            assert!(vm.finish_native_accessory(&p, thread, &command).is_err());
            assert_eq!(
                vm.world.objects[&1].accessories,
                ["other"],
                "legacy descriptor is not native mesh ownership"
            );
            assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_accessory_primitive_resource_index_case_and_flags() {
    let r: Value = serde_json::from_str(include_str!("fixtures/change-accessory.json")).unwrap();
    for row in r["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        // Valid Rust frames always have a loaded private module. Native null
        // private-resource cases are recorded but have no valid Rust counterpart.
        if c["source"] == 2 && c["available"] == false {
            continue;
        }
        let p = program(
            c["source"].as_u64().unwrap() as u8,
            c["flags"].as_u64().unwrap() as u8,
            c["index"].as_u64().unwrap() as u8,
            c["name"].as_str().unwrap(),
            c["available"] == true,
        );
        let before = vec!["suit-fork".to_string(), "other".to_string()];
        let (vm, run) = run(&p, before.clone());
        let mut expected = before;
        if let Some(error) = n["error"].as_u64() {
            assert_eq!(run.stop, Stop::Faulted, "{c}");
            assert!(
                run.trace[0].effect.contains(&format!("0x{error:02x}")),
                "{c}: {:?}",
                run.trace
            );
        } else {
            assert_eq!(run.stop, Stop::Returned(true), "{c}");
            let call = &n["calls"][0];
            let name = call["name"].as_str().unwrap();
            expected.retain(|v| v != name);
            if call["operation"] == "add" {
                expected.push(name.into());
            }
        }
        assert_eq!(vm.world.objects[&1].accessories, expected, "{c}");
    }
}
#[test]
fn accessory_snapshot_preserves_order_and_rejects_unavailable_appearance() {
    let p = program(2, 0, 0, "SUIT-FORK", true);
    let (vm, _) = run(&p, vec!["suit-fork".into(), "other".into()]);
    assert_eq!(vm.world.objects[&1].accessories, ["other", "suit-fork"]);
    // Completed synthetic frames retain their private owner, which is loaded here.
    let snapshot = Snapshot::capture(&p, &vm);
    assert_eq!(snapshot.clone().restore(&p).unwrap(), vm);
    let mut bad = snapshot;
    bad.state
        .world
        .objects
        .get_mut(&1)
        .unwrap()
        .accessories
        .push("missing".into());
    assert!(bad.restore(&p).unwrap_err().contains("accessory"));
    let missing = program(2, 0, 0, "NOT-LOADED", true);
    let (vm, result) = run(&missing, vec!["other".into()]);
    assert_eq!(result.stop, Stop::AwaitingAccessory);
    assert_eq!(vm.world.objects[&1].accessories, ["other"]);
}

#[test]
fn accessory_resource_pause_roundtrips_without_consuming_the_instruction() {
    for source in [0, 2] {
        let mut p = program(source, 0, 0, "NOT-LOADED", true);
        let (mut vm, first) = run(&p, vec!["other".into()]);
        assert_eq!(first.stop, Stop::AwaitingAccessory);
        assert_eq!(
            vm.pending_accessory_assets(&p),
            ["not-loaded".to_string()].into()
        );
        let id = *vm.threads.keys().next().unwrap();
        assert!(vm.advance_tick().is_err());
        let checkpoint = Snapshot::capture(&p, &vm);
        vm = serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&checkpoint).unwrap())
            .unwrap()
            .restore(&p)
            .unwrap();
        for budget in [1, 1000] {
            let before = vm.clone();
            let run = vm.run_slice(&p, id, budget).unwrap();
            assert_eq!(run.stop, Stop::AwaitingAccessory);
            assert!(run.trace.is_empty());
            assert_eq!(vm, before);
        }
        let mut bad = checkpoint.clone();
        if let Status::AwaitingAccessory { name, .. } =
            &mut bad.state.threads.get_mut(&id).unwrap().status
        {
            *name = "other".into();
        }
        assert!(bad.restore(&p).is_err());
        p.accessories
            .insert("not-loaded".into(), Default::default());
        assert!(
            checkpoint.restore(&p).is_err(),
            "content fingerprint must change with resolved resources"
        );
        vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
        assert_eq!(
            vm.run_slice(&p, id, 1000).unwrap().stop,
            Stop::Returned(true)
        );
        let (baseline, _) = run(&p, vec!["other".into()]);
        assert_eq!(
            vm, baseline,
            "pause must not change tick/RNG/PC/accessory ordering"
        );
    }
    // Removing a missing appearance never needs a resource load.
    let p = program(2, 1, 0, "NOT-LOADED", true);
    let (vm, r) = run(&p, vec!["other".into()]);
    assert_eq!(r.stop, Stop::Returned(true));
    assert!(vm.pending_accessory_assets(&p).is_empty());
}
