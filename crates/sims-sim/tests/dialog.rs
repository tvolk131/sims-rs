use sims_formats::behavior::{Behavior, Instruction};
use sims_sim::{Room, Simulation, Snapshot, StepStop};
use sims_vm::*;
use std::collections::BTreeMap;

fn fixture(mode: u8, flags: u8) -> (Program, Simulation) {
    let mut routine = Routine {
        name: "synthetic prompt".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args: 0,
            locals: 0,
            instructions: vec![Instruction {
                opcode: 0x24,
                on_true: 254,
                on_false: 255,
                operands: [0, 0, 2, 0, 0, mode, 1, flags],
            }],
        },
    };
    if flags & 8 != 0 {
        routine.body.locals = 4;
        routine.body.instructions[0].operands[1] = 3;
    }
    let p = Program {
        globals: "test".into(),
        modules: BTreeMap::from([
            (
                "test".into(),
                Module {
                    source: "test".into(),
                    routines: BTreeMap::from([(4096, routine)]),
                    strings: BTreeMap::from([(
                        301,
                        vec![
                            "Notice".into(),
                            "$Object and $Me".into(),
                            "job $Local:3".into(),
                        ],
                    )]),
                    catalog_names: BTreeMap::from([(1, "Bob".into()), (2, "Betty".into())]),
                    ..Default::default()
                },
            ),
            (
                "UIText.iff".into(),
                Module {
                    source: "UIText.iff".into(),
                    strings: BTreeMap::from([(
                        142,
                        vec!["OK".into(), "Cancel".into(), "Yes".into(), "No".into()],
                    )]),
                    ..Default::default()
                },
            ),
        ]),
        ..Default::default()
    };
    let mut w = World::default();
    for id in [1, 2] {
        w.objects.insert(
            id,
            Object {
                data: BTreeMap::from([(1, 0)]),
                definition: Some(Definition {
                    source: "test".into(),
                    object: id as u16,
                    guid: id as u32,
                    slots: 0,
                }),
                ..Default::default()
            },
        );
    }
    let mut v = Vm::new(w, 7);
    for id in [1, 2] {
        v.spawn(
            &p,
            Start {
                owner: "test",
                bhav: 4096,
                caller: id,
                callee: id,
                stack_object: 2,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    }
    let bodies = [1, 2]
        .into_iter()
        .map(|id| {
            (
                id,
                sims_sim::Body {
                    position: sims_sim::Point::new(16 * id as i32, 16),
                    facing: 1,
                    level: 1,
                    radius: 0,
                    elevation: 0,
                },
            )
        })
        .collect();
    let s = Simulation::from_placements(
        v,
        Room {
            width: 64,
            height: 64,
            obstacles: vec![],
        },
        bodies,
        Default::default(),
        None,
    )
    .unwrap();
    (p, s)
}
fn save(p: &Program, s: &Simulation) -> Simulation {
    serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&Snapshot::capture(p, s)).unwrap())
        .unwrap()
        .restore(p)
        .unwrap()
}
fn drive(p: &Program, s: &mut Simulation, budget: usize) -> StepStop {
    for _ in 0..20 {
        let stop = s.run_fixture_tick(p, budget).unwrap().stop;
        let pending =
            s.vm.threads
                .iter()
                .find_map(|(&id, t)| matches!(t.status, Status::AwaitingDialog(_)).then_some(id));
        if let Some(thread) = pending {
            let description =
                s.vm.dialog_construction(thread)
                    .unwrap()
                    .describe(p, &s.vm.world, &mut NeighborName("Neighbor"))
                    .unwrap();
            // These scheduler tests supply an absent image, as a native cache
            // miss does. Pixel/cache ownership is covered separately.
            s.vm.finish_dialog_construction(p, thread, description.record)
                .unwrap();
        }
        if stop != StepStop::Budget {
            return stop;
        }
    }
    panic!("dialog scheduler never yielded")
}

struct NeighborName(&'static str);
impl dialog::NeighborHost for NeighborName {
    fn live(&mut self, definition: &Definition) -> Result<Option<u32>, String> {
        Ok(Some(u32::from(definition.object)))
    }
    fn neighbor(&mut self, id: i16) -> Result<Option<u32>, String> {
        Ok((id == 2).then_some(739))
    }
    fn name(&mut self, resource: u32) -> Result<String, String> {
        assert_eq!(resource, 739);
        Ok(self.0.into())
    }
}

#[test]
fn dialog_resource_selection_retains_formatter_order_and_distinct_namespaces() {
    #[derive(Default)]
    struct Host {
        calls: Vec<String>,
        missing: bool,
    }
    impl dialog::NeighborHost for Host {
        fn neighbor(&mut self, id: i16) -> Result<Option<u32>, String> {
            self.calls.push(format!("neighbor:{id}"));
            Ok((!self.missing && id == 2).then_some(739))
        }
        fn name(&mut self, resource: u32) -> Result<String, String> {
            self.calls.push(format!("name:{resource}"));
            assert_eq!(resource, 739);
            Ok("Neighbor".into())
        }
        fn live(&mut self, definition: &Definition) -> Result<Option<u32>, String> {
            assert_eq!(definition.object, 2);
            self.calls.push("live:2".into());
            Ok(Some(17))
        }
    }
    // Native formatting order: first button, title, body; within each string
    // $Object precedes $Neighbor regardless of their positions in that string.
    for (button, title, body, selected, neighbor_count) in [
        ("OK", "Notice", "Plain", 17, 0),
        ("$Neighbor", "Notice", "Plain", 739, 1),
        ("$Neighbor", "$Object", "Plain", 17, 1),
        ("$Object", "Notice", "$Neighbor", 739, 1),
        ("OK", "Notice", "$Neighbor $Object", 739, 1),
        ("$Neighbor", "$Neighbor", "$Object", 17, 2),
    ] {
        for mode in [0, 1, 2, 3, 5, 6, 7] {
            for missing in [false, true] {
                let (mut p, s) = fixture(0, mode * 2);
                p.modules
                    .get_mut("test")
                    .unwrap()
                    .strings
                    .insert(301, vec![title.into(), body.into()]);
                p.modules
                    .get_mut("UIText.iff")
                    .unwrap()
                    .strings
                    .get_mut(&142)
                    .unwrap()[0] = button.into();
                let f = s.vm.threads.values().next().unwrap().stack.last().unwrap();
                let operands = p.modules["test"].routines[&4096].body.instructions[0].operands;
                let mut host = Host {
                    missing,
                    ..Default::default()
                };
                let before = s.clone();
                let description =
                    dialog::describe_resources(&p, &s.vm.world, f, operands, 99, &mut host)
                        .unwrap();
                let expected = match mode {
                    0 => Some(if missing { 17 } else { selected }),
                    2 => (!missing).then_some(739),
                    _ => None,
                };
                assert_eq!(
                    description.resource, expected,
                    "{button}/{title}/{body}, mode {mode}, missing {missing}"
                );
                assert_eq!(
                    host.calls
                        .iter()
                        .filter(|c| c.starts_with("neighbor:"))
                        .count(),
                    neighbor_count + usize::from(mode == 2)
                );
                assert_eq!(
                    host.calls.iter().filter(|c| c.starts_with("name:")).count(),
                    if missing { 0 } else { neighbor_count }
                );
                assert_eq!(
                    host.calls.iter().filter(|c| c.starts_with("live:")).count(),
                    usize::from(mode == 0 && expected == Some(17))
                );
                assert_eq!(s, before, "description must not mutate simulation state");
            }
        }
    }
}

#[test]
fn live_dialog_image_ownership_and_constructor_survive_resume() {
    for flags in [0, 1, 128] {
        let (p, mut s) = fixture(0, flags);
        let thread = *s.vm.threads.keys().next().unwrap();
        assert_eq!(
            s.vm.run_slice(&p, thread, 1).unwrap().stop,
            Stop::AwaitingDialog
        );
        let request = s.vm.dialog_construction(thread).unwrap().clone();
        assert_eq!(save(&p, &s), s);
        let description = request
            .describe(&p, &s.vm.world, &mut NeighborName("Neighbor"))
            .unwrap();
        assert_eq!(description.resource, Some(2));
        assert_eq!(description.record.icon_object, Some(2));
        let mut record = description.record;
        let mut entry = image_cache::Entry::new(2);
        entry.images[1] = Some(1);
        s.vm.world.image_cache.entries.push(entry);
        s.vm.world.image_cache.refs.push(2);
        record.portrait = Some(dialog::Image {
            handle: 1,
            rect: [0, 0, 45, 45],
        });
        let mut restored = save(&p, &s);
        s.vm.finish_dialog_construction(&p, thread, record.clone())
            .unwrap();
        restored
            .vm
            .finish_dialog_construction(&p, thread, record.clone())
            .unwrap();
        assert_eq!(s, restored);
        assert_eq!(save(&p, &s), s);
        let before = s.clone();
        assert!(s.vm.finish_dialog_construction(&p, thread, record).is_err());
        assert_eq!(s, before);
        s.vm.world.dialogs.records[0].status = 5;
        s.vm.pump_dialogs().unwrap();
        assert_eq!(s.vm.world.image_cache.refs, [1]);
        assert_eq!(save(&p, &s), s);
    }
}

#[test]
fn semiglobal_dialog_keeps_private_code_owner_through_construction_and_save() {
    for flags in [4, 5] {
        let (mut p, mut s) = fixture(0, flags);
        let private = p.modules.get_mut("test").unwrap();
        let shared = private.routines[&4096].clone();
        private.semiglobal = Some("shared".into());
        private.routines.get_mut(&4096).unwrap().body.instructions = vec![Instruction {
            opcode: 8312,
            on_true: 254,
            on_false: 255,
            operands: [0; 8],
        }];
        p.modules.insert(
            "shared".into(),
            Module {
                source: "shared".into(),
                routines: [(8312, shared)].into(),
                ..Default::default()
            },
        );
        assert!(p.resolve("shared", 8312).is_err());
        let thread = *s.vm.threads.keys().next().unwrap();
        assert_eq!(
            s.vm.run_slice(&p, thread, 2).unwrap().stop,
            Stop::AwaitingDialog
        );
        let request = s.vm.dialog_construction(thread).unwrap().clone();
        assert_eq!(request.frame().code_owner, "test");
        assert_eq!(request.frame().location.source, "shared");
        let mut restored = save(&p, &s);
        assert_eq!(restored, s);
        let description = request
            .describe(&p, &s.vm.world, &mut NeighborName("Neighbor"))
            .unwrap();
        assert_eq!(description.record.source, "test");
        assert_eq!(description.record.origin.source, "shared");
        assert_eq!(description.record.title, "Notice");
        s.vm.finish_dialog_construction(&p, thread, description.record.clone())
            .unwrap();
        restored
            .vm
            .finish_dialog_construction(&p, thread, description.record)
            .unwrap();
        assert_eq!(restored, s);
        assert_eq!(save(&p, &s), s);
        assert_eq!(
            s.vm.threads[&thread].status,
            if flags & 1 == 0 {
                Status::Sleeping { wake_tick: 1 }
            } else {
                Status::Returned(true)
            }
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn construction_preserves_native_reentrant_changes_and_close_order() {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dialog-reentrant.json")).unwrap();
    assert_eq!(oracle["cases"].as_array().unwrap().len(), 18);
    for case in oracle["cases"].as_array().unwrap() {
        let input = &case["input"];
        let native = &case["original"];
        let operands: [u8; 8] = serde_json::from_value(input["operands"].clone()).unwrap();
        let (p, mut s) = fixture(operands[5], operands[7]);
        let thread = *s.vm.threads.keys().next().unwrap();
        // Construct an earlier owner record through the same public path.
        let old_status = input["status"].as_u64();
        if let Some(status) = old_status {
            let original = s.vm.threads[&thread].clone();
            assert_eq!(
                s.vm.run_slice(&p, thread, 1).unwrap().stop,
                Stop::AwaitingDialog
            );
            let record =
                s.vm.dialog_construction(thread)
                    .unwrap()
                    .describe(&p, &s.vm.world, &mut NeighborName("Neighbor"))
                    .unwrap()
                    .record;
            s.vm.finish_dialog_construction(&p, thread, record).unwrap();
            s.vm.threads.insert(thread, original);
            s.vm.world.dialogs.records[0].status = status as u32;
        }
        assert_eq!(
            s.vm.run_slice(&p, thread, 1).unwrap().stop,
            Stop::AwaitingDialog
        );
        let request = s.vm.dialog_construction(thread).unwrap().clone();
        if old_status.is_some() {
            assert_eq!(
                serde_json::json!(s.vm.world.dialogs.records[0].status),
                native["status_during_create"]
            );
        }
        let pending = save(&p, &s);
        assert_eq!(pending, s);
        assert!(s.advance_fixture_tick().is_err());
        let step = s.run_fixture_tick(&p, 1000).unwrap();
        assert_eq!(step.stop, StepStop::Budget);
        assert!(step.trace.is_empty() && step.events.is_empty());
        assert_eq!(s, pending, "ordinary lot work must remain frozen");
        let description = request
            .describe(&p, &s.vm.world, &mut NeighborName("Neighbor"))
            .unwrap();
        assert_eq!(description.resource, Some(739));
        s.vm.world.objects.get_mut(&1).unwrap().temps =
            serde_json::from_value(native["temps"].clone()).unwrap();
        s.vm.world.dialogs.protocol.words =
            serde_json::from_value(native["words"].clone()).unwrap();
        let callback_state = save(&p, &s);
        let mut resumed = callback_state.clone();
        s.vm.finish_dialog_construction(&p, thread, description.record.clone())
            .unwrap();
        resumed
            .vm
            .finish_dialog_construction(&p, thread, description.record.clone())
            .unwrap();
        assert_eq!(s, resumed);
        assert_eq!(
            s.vm.world.objects[&1].temps,
            callback_state.vm.world.objects[&1].temps
        );
        assert_eq!(
            s.vm.world.dialogs.protocol,
            callback_state.vm.world.dialogs.protocol
        );
        assert_eq!(s.vm.world.dialogs.records.last().unwrap().id, request.id());
        assert_eq!(
            s.vm.threads[&thread].status,
            if native["returned"] == 1 {
                Status::Returned(true)
            } else {
                Status::Sleeping { wake_tick: 1 }
            }
        );
        let completed = s.clone();
        assert!(
            s.vm.finish_dialog_construction(&p, thread, description.record)
                .is_err()
        );
        assert_eq!(s, completed);
        assert_eq!(save(&p, &s), s);
    }
}

#[test]
fn neighbor_text_pass_order_and_portrait_ownership_survive_resume() {
    let (mut p, mut s) = fixture(0, 4);
    p.modules
        .get_mut("test")
        .unwrap()
        .strings
        .get_mut(&301)
        .unwrap()[1] = "$Neighbor".into();
    let thread = *s.vm.threads.keys().next().unwrap();
    assert_eq!(
        s.vm.run_slice(&p, thread, 1).unwrap().stop,
        Stop::AwaitingDialog
    );
    let request = s.vm.dialog_construction(thread).unwrap().clone();
    let mut record = request
        .describe(&p, &s.vm.world, &mut NeighborName("Neighbor $Me"))
        .unwrap()
        .record;
    // Names precede Neighbor, so the newly introduced earlier token survives.
    assert_eq!(record.body, "Neighbor $Me");
    for field in ["id", "tick"] {
        let mut value = serde_json::to_value(&request).unwrap();
        value[field] = serde_json::json!(u64::MAX);
        let mut bad = Snapshot::capture(&p, &s);
        bad.state.vm.threads.get_mut(&thread).unwrap().status =
            Status::AwaitingDialog(Box::new(serde_json::from_value(value).unwrap()));
        assert!(bad.restore(&p).is_err());
    }
    let mut bad = Snapshot::capture(&p, &s);
    bad.state
        .vm
        .threads
        .get_mut(&thread)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .stack_object = 1;
    assert!(bad.restore(&p).is_err());
    // One cache owner plus the constructor's output wrapper.
    let mut entry = image_cache::Entry::new(1);
    entry.images[0] = Some(1);
    s.vm.world.image_cache.entries.push(entry);
    s.vm.world.image_cache.refs.push(2);
    record.portrait = Some(dialog::Image {
        handle: 1,
        rect: [0, 0, 45, 45],
    });
    s.vm.finish_dialog_construction(&p, thread, record).unwrap();
    let cache = &mut s.vm.world.image_cache;
    let mut owner = cache.entries[0].images[0].take();
    cache
        .assign_image(&mut owner, None, &mut |_, _| {})
        .unwrap();
    assert_eq!(s.vm.world.image_cache.refs, [1]);
    assert_eq!(save(&p, &s), s);
    let id = s.vm.pump_dialogs().unwrap().unwrap();
    s.vm.answer_dialog(id, 2).unwrap();
    s.vm.world.dialogs.records[0].status = 5;
    s.vm.pump_dialogs().unwrap();
    assert_eq!(s.vm.world.image_cache.refs, [0]);
}

#[test]
fn modal_dialogs_hold_end_of_tick_and_resume_once_in_registry_order() {
    for budget in [1, 1000] {
        for (mode, response, expected, temp) in [
            (0, 2, true, 0),
            (1, 3, false, 0),
            (2, 3, false, 0),
            (2, 4, false, 1),
        ] {
            let (p, mut s) = fixture(mode, 0x70);
            let mut restored = save(&p, &s);
            assert_eq!(drive(&p, &mut s, budget), StepStop::Dialog);
            assert_eq!(drive(&p, &mut restored, budget), StepStop::Dialog);
            assert_eq!(s, restored);
            assert_eq!(
                s.vm.world.dialogs.records.len(),
                2,
                "all objects run before modal UI pass"
            );
            assert_eq!(s.vm.world.dialogs.records[0].body, "Betty and Bob");
            assert_eq!(s.vm.world.dialogs.records[1].body, "Betty and Betty");
            for caller in [1, 2] {
                restored = save(&p, &s);
                let id = s.vm.world.dialogs.modal.unwrap();
                assert_eq!(
                    s.vm.world
                        .dialogs
                        .records
                        .iter()
                        .find(|r| r.id == id)
                        .unwrap()
                        .caller,
                    caller
                );
                let before = s.clone();
                assert_eq!(drive(&p, &mut s, budget), StepStop::Dialog);
                assert_eq!(s, before);
                assert!(s.advance_fixture_tick().is_err());
                assert_eq!(s, before);
                assert!(s.vm.answer_dialog(id, 77).is_err());
                assert_eq!(s, before);
                s.vm.answer_dialog(id, response).unwrap();
                restored.vm.answer_dialog(id, response).unwrap();
                assert!(s.vm.answer_dialog(id, response).is_err());
                assert_eq!(
                    drive(&p, &mut s, budget),
                    if caller == 1 {
                        StepStop::Dialog
                    } else {
                        StepStop::Waiting
                    }
                );
                drive(&p, &mut restored, budget);
                assert_eq!(s, restored);
            }
            assert_eq!(s.vm.tick, 0);
            restored = save(&p, &s);
            s.advance_fixture_tick().unwrap();
            restored.advance_fixture_tick().unwrap();
            assert_eq!(drive(&p, &mut s, budget), StepStop::Complete);
            drive(&p, &mut restored, budget);
            assert_eq!(s, restored);
            assert!(
                s.vm.threads
                    .values()
                    .all(|t| t.status == Status::Returned(expected))
            );
            assert!(
                s.vm.world.dialogs.records.is_empty()
                    && s.vm.world.ui_owner.dialog_owners.is_empty()
            );
            assert_eq!(s.vm.world.objects[&1].temps[7], temp);
            assert_eq!(save(&p, &s), s);
        }
    }
}

#[test]
fn native_no_resource_icon_modes_preserve_text_and_modal_save_resume() {
    for icon_mode in [1, 5, 6, 7] {
        for extra_flags in [0, 1, 0x70, 0x71] {
            for budget in [1, 1000] {
                let (p, mut s) = fixture(0, icon_mode * 2 | extra_flags);
                assert_eq!(drive(&p, &mut s, budget), StepStop::Dialog);
                assert_eq!(s.vm.world.dialogs.records[0].body, "Betty and Bob");
                assert_eq!(s.vm.world.dialogs.records[1].body, "Betty and Betty");
                for record in &s.vm.world.dialogs.records {
                    assert!(
                        record.icon_object.is_none()
                            && record.job_icon.is_none()
                            && record.bitmap_icon.is_none()
                            && record.portrait.is_none()
                    );
                }
                assert_eq!(save(&p, &s), s);
                let mut corrupt = Snapshot::capture(&p, &s);
                corrupt.state.vm.world.dialogs.records[0].icon_object = Some(2);
                assert!(corrupt.restore(&p).is_err());
                for caller in [1, 2] {
                    let mut restored = save(&p, &s);
                    let before = s.clone();
                    assert_eq!(drive(&p, &mut s, budget), StepStop::Dialog);
                    assert_eq!(s, before);
                    let id = s.vm.world.dialogs.modal.unwrap();
                    assert_eq!(
                        s.vm.world
                            .dialogs
                            .records
                            .iter()
                            .find(|r| r.id == id)
                            .unwrap()
                            .caller,
                        caller
                    );
                    s.vm.answer_dialog(id, 2).unwrap();
                    restored.vm.answer_dialog(id, 2).unwrap();
                    assert_eq!(drive(&p, &mut s, budget), drive(&p, &mut restored, budget));
                    assert_eq!(s, restored);
                }
                if extra_flags & 1 == 0 {
                    s.advance_fixture_tick().unwrap();
                    assert_eq!(drive(&p, &mut s, budget), StepStop::Complete);
                }
                assert!(
                    s.vm.threads
                        .values()
                        .all(|t| t.status == Status::Returned(true))
                );
                assert_eq!(save(&p, &s), s);
            }
        }
    }
}

#[test]
fn pending_dialog_snapshots_reject_broken_ownership_and_request_identity() {
    let (p, mut s) = fixture(0, 0);
    drive(&p, &mut s, 1);
    for n in 0..5 {
        let mut bad = Snapshot::capture(&p, &s);
        match n {
            0 => bad.state.vm.world.dialogs.records[0].id = 0,
            1 => bad.state.vm.world.dialogs.records[0].status = 5,
            2 => bad.state.vm.world.ui_owner.dialog_owners.swap(0, 1),
            3 => bad.state.vm.world.dialogs.records[0].operands[5] = 1,
            _ => bad.state.vm.world.dialogs.records[0].buttons[0].result = 7,
        }
        assert!(bad.restore(&p).is_err(), "mutant {n}");
    }
}

#[test]
fn named_school_icon_and_local_text_survive_modal_save_resume() {
    let (mut p, mut s) = fixture(0, 8);
    p.modules
        .get_mut("test")
        .unwrap()
        .strings
        .get_mut(&301)
        .unwrap()[1] = "A bus in $Local:0 minutes.".into();
    for t in s.vm.threads.values_mut() {
        let frame = t.stack.last_mut().unwrap();
        frame.locals[0] = 60;
        frame.locals[3] = -1;
    }
    assert_eq!(drive(&p, &mut s, 1), StepStop::Dialog);
    let id = s.vm.world.dialogs.modal.unwrap();
    let record =
        s.vm.world
            .dialogs
            .records
            .iter()
            .find(|r| r.id == id)
            .unwrap();
    assert_eq!(record.body, "A bus in 60 minutes.");
    assert_eq!(record.job_icon, Some(-1));
    assert_eq!(record.icon_object, None);
    let mut restored = save(&p, &s);
    s.vm.answer_dialog(id, 2).unwrap();
    restored.vm.answer_dialog(id, 2).unwrap();
    assert_eq!(drive(&p, &mut s, 1000), drive(&p, &mut restored, 1));
    assert_eq!(s, restored);
}

#[test]
fn fire_and_forget_dialogs_close_on_response_and_nonmodal_prompts_recreate_on_poll() {
    let (p, mut s) = fixture(0, 1);
    assert_eq!(drive(&p, &mut s, 1), StepStop::Dialog);
    assert!(
        s.vm.threads
            .values()
            .all(|t| t.status == Status::Returned(true))
    );
    for expected in [StepStop::Dialog, StepStop::Complete] {
        s = save(&p, &s);
        let id = s.vm.world.dialogs.modal.unwrap();
        s.vm.answer_dialog(id, 2).unwrap();
        assert_eq!(
            s.vm.world
                .dialogs
                .records
                .iter()
                .find(|r| r.id == id)
                .unwrap()
                .status,
            5
        );
        assert_eq!(drive(&p, &mut s, 1), expected);
    }
    assert!(s.vm.world.dialogs.records.is_empty());

    let (p, mut s) = fixture(0, 128);
    assert_eq!(drive(&p, &mut s, 1000), StepStop::Waiting);
    assert!(s.vm.world.dialogs.modal.is_none());
    let old =
        s.vm.world
            .dialogs
            .records
            .iter()
            .map(|r| r.id)
            .collect::<Vec<_>>();
    s = save(&p, &s);
    s.advance_fixture_tick().unwrap();
    assert_eq!(drive(&p, &mut s, 1000), StepStop::Waiting);
    assert_eq!(s.vm.world.dialogs.records.len(), 2);
    assert!(
        s.vm.world
            .dialogs
            .records
            .iter()
            .all(|r| r.status == 1 && !old.contains(&r.id))
    );
    for record in s.vm.world.dialogs.records.clone() {
        s.vm.answer_dialog(record.id, 2).unwrap();
    }
    s = save(&p, &s);
    s.advance_fixture_tick().unwrap();
    assert_eq!(drive(&p, &mut s, 1000), StepStop::Complete);
    assert!(s.vm.world.dialogs.records.is_empty());
}

#[test]
fn career_dialog_uses_frame_locals_and_caller_gender_through_save_resume() {
    use sims_vm::career_text::{Level, Text};
    for budget in [1, 1000] {
        let (mut p, mut s) = fixture(0, 8);
        p.text_language = Some(1);
        p.careers = Some(program::CareerCatalog {
            ids: vec![7, 8],
            text: Some(vec![
                Text {
                    name: "Local career".into(),
                    offers: ["Offer M".into(), "Offer F".into()],
                    levels: vec![
                        Level {
                            titles: ["Entry".into(), String::new()],
                            secondary: Default::default(),
                            description: String::new(),
                        },
                        Level {
                            titles: ["Title M".into(), "Title F".into()],
                            secondary: Default::default(),
                            description: "Description".into(),
                        },
                    ],
                },
                Text {
                    name: "Current career".into(),
                    offers: Default::default(),
                    levels: vec![Level {
                        titles: ["Current".into(), String::new()],
                        secondary: Default::default(),
                        description: String::new(),
                    }],
                },
            ]),
            ..Default::default()
        });
        p.modules
            .get_mut("test")
            .unwrap()
            .strings
            .get_mut(&301)
            .unwrap()[1] =
            "$Object | $Job:0:1 | $Job | $JobDesc:0:1 | $JobOffer:0:1 | $Local:3 | $TimeLocal:1"
                .into();
        for (&id, object) in &mut s.vm.world.objects {
            object.person = Some(BTreeMap::from([(56, 8), (57, 0), (65, id - 1)]));
        }
        for thread in s.vm.threads.values_mut() {
            let frame = thread.stack.last_mut().unwrap();
            frame.locals = vec![7, 1, 0, 7];
            // An unrelated opaque word must not prevent the selected reads.
            frame.opaque_locals.insert(2);
        }
        let mut opaque = s.clone();
        for thread in opaque.vm.threads.values_mut() {
            thread.stack.last_mut().unwrap().opaque_locals.insert(0);
        }
        assert_eq!(drive(&p, &mut opaque, budget), StepStop::Faulted);
        assert!(opaque.vm.threads.values().any(|t| {
            matches!(&t.status, Status::Faulted(f) if f.message.contains("career formatting reads opaque local"))
        }));
        let mut opaque_time = s.clone();
        for thread in opaque_time.vm.threads.values_mut() {
            thread.stack.last_mut().unwrap().opaque_locals.insert(1);
        }
        assert_eq!(drive(&p, &mut opaque_time, budget), StepStop::Faulted);
        assert!(opaque_time.vm.threads.values().any(|t| {
            matches!(&t.status, Status::Faulted(f) if f.message.contains("time formatting reads opaque local"))
        }));
        let mut restored = save(&p, &s);
        assert_eq!(drive(&p, &mut s, budget), StepStop::Dialog);
        assert_eq!(drive(&p, &mut restored, budget), StepStop::Dialog);
        assert_eq!(s, restored);
        let records = &s.vm.world.dialogs.records;
        assert_eq!(
            records[0].body,
            "Betty | Title M | Current | Description | Offer M | 7 |  1:00 am"
        );
        assert_eq!(
            records[1].body,
            "Betty | Title F | Current | Description | Offer F | 7 |  1:00 am"
        );
        assert!(records.iter().all(|r| r.job_icon == Some(7)));
        for expected in [StepStop::Dialog, StepStop::Waiting] {
            restored = save(&p, &s);
            let id = s.vm.world.dialogs.modal.unwrap();
            s.vm.answer_dialog(id, 2).unwrap();
            restored.vm.answer_dialog(id, 2).unwrap();
            assert_eq!(drive(&p, &mut s, budget), expected);
            assert_eq!(drive(&p, &mut restored, budget), expected);
            assert_eq!(s, restored);
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn private_bitmap_uses_code_resource_and_survives_choice_snapshot() {
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../sims-vm/tests/fixtures/dialog-bitmap.json"
    ))
    .unwrap();
    for row in report["cases"].as_array().unwrap() {
        assert_eq!(
            u64::from(dialog::private_bitmap_id(
                row["input"]["index"].as_u64().unwrap() as u8
            )),
            row["original"]["chunk"].as_u64().unwrap()
        );
    }
    for result in [2, 3] {
        let (mut p, mut s) = fixture(1, 6);
        p.modules
            .get_mut("test")
            .unwrap()
            .routines
            .get_mut(&4096)
            .unwrap()
            .body
            .instructions[0]
            .operands[1] = 68;
        p.modules.insert(
            "objects".into(),
            Module {
                source: "objects".into(),
                catalog_names: BTreeMap::from([(1, "Bob".into()), (2, "Betty".into())]),
                ..Default::default()
            },
        );
        for object in s.vm.world.objects.values_mut() {
            object.definition.as_mut().unwrap().source = "objects".into();
        }
        assert_eq!(drive(&p, &mut s, 1), StepStop::Dialog);
        let id = s.vm.world.dialogs.modal.unwrap();
        let record =
            s.vm.world
                .dialogs
                .records
                .iter()
                .find(|r| r.id == id)
                .unwrap();
        assert_eq!(record.source, "test");
        assert_eq!(record.bitmap_icon, Some(5068));
        assert_eq!(record.icon_object, None);
        assert_eq!(record.job_icon, None);
        assert_eq!(
            record
                .buttons
                .iter()
                .map(|b| b.label.as_str())
                .collect::<Vec<_>>(),
            ["Yes", "No"]
        );
        let mut bad = Snapshot::capture(&p, &s);
        bad.state.vm.world.dialogs.records[0].bitmap_icon = Some(5069);
        assert!(bad.restore(&p).is_err());
        let mut restored = save(&p, &s);
        let before = s.clone();
        assert_eq!(drive(&p, &mut s, 1000), StepStop::Dialog);
        assert_eq!(before, s);
        s.vm.answer_dialog(id, result).unwrap();
        restored.vm.answer_dialog(id, result).unwrap();
        assert_eq!(drive(&p, &mut s, 1), drive(&p, &mut restored, 1000));
        assert_eq!(s, restored);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn constructor_formatter_selection_matches_original_call_order() {
    struct Host {
        missing: bool,
    }
    impl dialog::NeighborHost for Host {
        fn neighbor(&mut self, id: i16) -> Result<Option<u32>, String> {
            assert_eq!(id, 3);
            Ok((!self.missing).then_some(4))
        }
        fn name(&mut self, resource: u32) -> Result<String, String> {
            assert_eq!(resource, 4);
            Ok("Neighbor".into())
        }
        fn live(&mut self, definition: &Definition) -> Result<Option<u32>, String> {
            assert_eq!(definition.object, 2);
            Ok(Some(2))
        }
    }
    let fixture_data: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dialog-format-selection.json")).unwrap();
    let rows = fixture_data["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 192);
    for row in rows {
        let c = &row["input"];
        let flags = c["flags"].as_u64().unwrap() as u8;
        let text = c["text"].as_array().unwrap();
        let (mut p, mut s) = fixture(2, flags);
        let operands = [5, 0, 2, 3, 4, 2, 1, flags];
        let module = p.modules.get_mut("test").unwrap();
        module.strings.insert(
            301,
            [3, 4, 0, 1, 2]
                .map(|i| text[i].as_str().unwrap().to_owned())
                .to_vec(),
        );
        module.routines.get_mut(&4096).unwrap().body.instructions[0].operands = operands;
        s.vm.world.objects.insert(3, s.vm.world.objects[&2].clone());
        let mut frame =
            s.vm.threads
                .values()
                .next()
                .unwrap()
                .stack
                .last()
                .unwrap()
                .clone();
        frame.stack_object = 3;
        let description = dialog::describe_resources(
            &p,
            &s.vm.world,
            &frame,
            operands,
            99,
            &mut Host {
                missing: c["missing"].as_bool().unwrap(),
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(description.resource).unwrap(),
            row["original"]["resource"],
            "{c}"
        );
        assert_eq!(row["original"]["calls"], serde_json::json!([0, 1, 2, 3, 4]));
    }
}
