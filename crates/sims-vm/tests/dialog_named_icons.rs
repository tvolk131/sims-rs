use sims_vm::dialog::{GlobalBitmap, NamedIcon, named_icon};

// Expectations recovered from 436552..436933 and 46f891. These are static
// instruction-derived checks, not newly executed original-engine cases.
#[test]
fn named_global_icons_preserve_identity_crop_and_crt_number_rules() {
    for (text, id, button_cell) in [
        ("gz 4500", 4500, true),
        ("GZ 4 5 0 0", 4500, true),
        ("gzi 4500", 4500, false),
        ("GzI \t+4500tail", 4500, false),
        ("gz -1", -1, true),
        ("gz 4294967297", 1, true),
    ] {
        assert_eq!(
            named_icon(text).unwrap(),
            Some(NamedIcon::Global(GlobalBitmap { id, button_cell })),
            "{text}"
        );
    }
    for text in ["", "gz", "gzi zero", "gz 0", "gz 4294967296"] {
        assert_eq!(named_icon(text).unwrap(), None, "{text}");
    }
    for (text, job) in [("JOB -1", -1), ("job", 0), ("Job 2 1", 21)] {
        assert_eq!(named_icon(text).unwrap(), Some(NamedIcon::Job(job)));
    }
    assert_eq!(named_icon("job 22").unwrap(), None);
    for text in ["rel", "REL 0", "Rel ignored", "rel -1"] {
        assert_eq!(named_icon(text).unwrap(), Some(NamedIcon::Relationship));
    }
    for (text, guid) in [
        ("guid -2032092235", 2262875061),
        ("GUID 1 2 3", 123),
        ("guid", 0),
    ] {
        assert_eq!(named_icon(text).unwrap(), Some(NamedIcon::Guid(guid)));
    }
    for text in ["gz\t4500", "my 5000"] {
        assert!(
            named_icon(text).is_err(),
            "unsupported categories must stay explicit: {text}"
        );
    }
}

#[test]
fn relationship_named_icon_suspends_restores_and_requires_its_real_strip_crop() {
    use sims_formats::behavior::{Behavior, Instruction};
    use sims_vm::{
        Definition, Module, Object, Program, Routine, Snapshot, Start, Status, Vm, World,
    };
    let module = Module {
        source: "fixture".into(),
        strings: [(
            301,
            vec!["rel 0".into(), "Relationship".into(), "Tutorial".into()],
        )]
        .into(),
        routines: [(
            4096,
            Routine {
                name: "relationship dialog fixture".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 0,
                    locals: 0,
                    instructions: vec![Instruction {
                        opcode: 36,
                        on_true: 254,
                        on_false: 255,
                        operands: [0, 1, 2, 0, 0, 4, 3, 137],
                    }],
                },
            },
        )]
        .into(),
        ..Module::default()
    };
    let labels = Module {
        source: "UIText.iff".into(),
        strings: [(142, vec!["OK".into()])].into(),
        ..Module::default()
    };
    let program = Program {
        globals: "fixture".into(),
        modules: [("fixture".into(), module), ("UIText.iff".into(), labels)].into(),
        ..Program::default()
    };
    let mut world = World::default();
    world.objects.insert(
        1,
        Object {
            definition: Some(Definition {
                source: "fixture".into(),
                object: 1,
                guid: 7,
                slots: 0,
            }),
            ..Object::default()
        },
    );
    let mut vm = Vm::new(world, 1);
    let thread = vm
        .spawn(
            &program,
            Start {
                owner: "fixture",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 1,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    let initial_tick = vm.tick;
    vm.run_slice(&program, thread, 1).unwrap();
    assert!(matches!(
        vm.threads[&thread].status,
        Status::AwaitingDialog(_)
    ));
    assert!(vm.world.dialogs.records.is_empty());
    assert_eq!(vm.tick, initial_tick);
    let request = vm.dialog_construction(thread).unwrap().clone();
    assert!(request.relationship_icon(&program, &vm.world).unwrap());
    let bytes = serde_json::to_vec(&Snapshot::capture(&program, &vm)).unwrap();
    let mut restored = Snapshot::from_json(&bytes, &program).unwrap();
    assert_eq!(restored.dialog_construction(thread).unwrap(), &request);
    struct NoResource;
    impl sims_vm::dialog::NeighborHost for NoResource {
        fn neighbor(&mut self, _: i16) -> Result<Option<u32>, String> {
            panic!("unexpected resource lookup")
        }
        fn name(&mut self, _: u32) -> Result<String, String> {
            panic!("unexpected name lookup")
        }
    }
    for state in [&mut vm, &mut restored] {
        let description = request
            .describe(&program, &state.world, &mut NoResource)
            .unwrap();
        assert!(description.relationship && description.resource.is_none());
        state.world.image_cache.refs.push(1);
        let mut record = description.record;
        record.portrait = Some(sims_vm::dialog::Image {
            handle: 1,
            rect: [0, 0, 45, 45],
        });
        let before = Snapshot::capture(&program, state);
        assert!(
            state
                .finish_dialog_construction(&program, thread, record.clone())
                .is_err()
        );
        assert_eq!(Snapshot::capture(&program, state), before);
        record.portrait.as_mut().unwrap().rect = [80, 0, 120, 40];
        state
            .finish_dialog_construction(&program, thread, record)
            .unwrap();
        assert_eq!(state.tick, initial_tick);
        assert_eq!(state.world.dialogs.records.len(), 1);
        Snapshot::capture(&program, state)
            .restore(&program)
            .unwrap();
    }
    assert_eq!(
        Snapshot::capture(&program, &vm),
        Snapshot::capture(&program, &restored)
    );
}

#[test]
fn guid_named_icon_restores_catalog_identity_and_rejects_wrong_crop_or_owner() {
    use sims_formats::behavior::{Behavior, Instruction};
    use sims_vm::{
        Definition, Module, Object, Program, Routine, Snapshot, Start, Status, Vm, World,
    };
    let module = Module {
        source: "fixture".into(),
        strings: [(
            301,
            vec!["guid -2032092235".into(), "Phone".into(), "Tutorial".into()],
        )]
        .into(),
        routines: [(
            4096,
            Routine {
                name: "GUID dialog fixture".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 0,
                    locals: 0,
                    instructions: vec![Instruction {
                        opcode: 36,
                        on_true: 254,
                        on_false: 255,
                        operands: [0, 1, 2, 0, 0, 4, 3, 137],
                    }],
                },
            },
        )]
        .into(),
        ..Module::default()
    };
    let labels = Module {
        source: "UIText.iff".into(),
        strings: [(142, vec!["OK".into()])].into(),
        ..Module::default()
    };
    let program = Program {
        globals: "fixture".into(),
        modules: [("fixture".into(), module), ("UIText.iff".into(), labels)].into(),
        ..Program::default()
    };
    let mut world = World::default();
    world.objects.insert(
        1,
        Object {
            definition: Some(Definition {
                source: "fixture".into(),
                object: 1,
                guid: 7,
                slots: 0,
            }),
            ..Object::default()
        },
    );
    let mut vm = Vm::new(world, 1);
    let thread = vm
        .spawn(
            &program,
            Start {
                owner: "fixture",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 1,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    vm.run_slice(&program, thread, 1).unwrap();
    assert!(
        matches!(vm.threads[&thread].status, Status::AwaitingDialog(_)),
        "{:?}",
        vm.threads[&thread].status
    );
    let pending = vm.dialog_construction(thread).unwrap().clone();
    assert_eq!(
        pending.guid_icon(&program, &vm.world).unwrap(),
        Some(2262875061)
    );
    let bytes = serde_json::to_vec(&Snapshot::capture(&program, &vm)).unwrap();
    let mut restored = Snapshot::from_json(&bytes, &program).unwrap();
    struct Resources {
        result: Option<u32>,
        calls: Vec<u32>,
    }
    impl sims_vm::dialog::NeighborHost for Resources {
        fn neighbor(&mut self, _: i16) -> Result<Option<u32>, String> {
            panic!("neighbor identity is not a GUID resource")
        }
        fn name(&mut self, _: u32) -> Result<String, String> {
            panic!("GUID lookup must not format a neighbor name")
        }
        fn guid(&mut self, guid: u32) -> Result<Option<u32>, String> {
            self.calls.push(guid);
            Ok(self.result)
        }
    }
    for state in [&mut vm, &mut restored] {
        for resource in [None, Some(0), Some(17)] {
            let mut resources = Resources {
                result: resource,
                calls: vec![],
            };
            let described = pending.describe(&program, &state.world, &mut resources);
            assert_eq!(resources.calls, vec![2262875061]);
            if resource == Some(0) {
                assert!(described.is_err());
            } else {
                assert_eq!(described.unwrap().resource, resource);
            }
        }
        let description = pending
            .describe(
                &program,
                &state.world,
                &mut Resources {
                    result: Some(17),
                    calls: vec![],
                },
            )
            .unwrap();
        assert_eq!(description.record.guid_icon, Some(2262875061));
        assert!(!description.relationship);
        state.world.image_cache.refs.push(1);
        let mut record = description.record;
        record.portrait = Some(sims_vm::dialog::Image {
            handle: 1,
            rect: [0, 0, 45, 45],
        });
        for (guid, crop) in [
            (Some(7), [0, 0, 45, 45]),
            (None, [80, 0, 120, 40]),
            (Some(2262875061), [0, 0, 120, 120]),
            (Some(2262875061), [80, 0, 120, 40]),
        ] {
            let mut wrong = record.clone();
            wrong.guid_icon = guid;
            wrong.portrait.as_mut().unwrap().rect = crop;
            let before = Snapshot::capture(&program, state);
            assert!(
                state
                    .finish_dialog_construction(&program, thread, wrong)
                    .is_err()
            );
            assert_eq!(Snapshot::capture(&program, state), before);
        }
        state
            .finish_dialog_construction(&program, thread, record)
            .unwrap();
        Snapshot::capture(&program, state)
            .restore(&program)
            .unwrap();
    }
    assert_eq!(
        Snapshot::capture(&program, &vm),
        Snapshot::capture(&program, &restored)
    );
}
