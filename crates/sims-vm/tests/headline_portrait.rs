use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{
    headline::{Catalog, State},
    *,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Case {
    selector: u8,
    flags: u16,
    target: i16,
    temp: i16,
    duration: i16,
    options: u16,
    active: i32,
    priority: i32,
    slot: bool,
    group: u8,
    neighbor_resource: bool,
    args: Vec<i16>,
    locals: Vec<i16>,
    continuation: u32,
}
impl Default for Case {
    fn default() -> Self {
        Self {
            selector: 0,
            flags: 0,
            target: 3,
            temp: -2,
            duration: 20,
            options: 0,
            active: 0,
            priority: 0,
            slot: true,
            group: 0,
            neighbor_resource: true,
            args: vec![3, 1],
            locals: vec![1, 3, 0],
            continuation: 3,
        }
    }
}
fn setup(c: &Case) -> (Program, Vm, ThreadId) {
    let mut m = Module {
        source: "test".into(),
        headline_catalog: Some(Catalog {
            frames: BTreeMap::from([(123, 9)]),
            ..Default::default()
        }),
        ..Default::default()
    };
    for (id, guid) in [(1, 101), (2, 103), (3, 109)] {
        let mut w = vec![0; 108];
        w[10] = if c.group == 0 { 0 } else { 5 };
        w[11] = if id == 3 { -1 } else { 0 };
        w[14] = guid;
        m.object_definition_words.insert(id, w);
    }
    // Same-file enumeration excludes the native probe's different-file decoy.
    m.object_definition_order = if c.group == 2 {
        vec![2, 1]
    } else {
        vec![2, 3, 1]
    };
    let mut b = [0; 8];
    b[..2].copy_from_slice(&c.flags.to_le_bytes());
    b[2] = c.selector;
    b[3] = 7;
    b[4..6].copy_from_slice(&c.duration.to_le_bytes());
    b[6..].copy_from_slice(&c.options.to_le_bytes());
    m.routines.insert(
        4096,
        Routine {
            name: "headline".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args: c.args.len() as u8,
                locals: c.locals.len() as u16,
                instructions: vec![Instruction {
                    opcode: 0x29,
                    on_true: 254,
                    on_false: 253,
                    operands: b,
                }],
            },
        },
    );
    let p = Program {
        globals: "test".into(),
        modules: BTreeMap::from([("test".into(), m)]),
        neighbor_resource_flags: Some(if c.neighbor_resource {
            BTreeMap::from([(203, 4)])
        } else {
            BTreeMap::new()
        }),
        ..Default::default()
    };
    let mut world = World::default();
    for (id, object, guid) in [(1, 1, 101), (3, 2, 103)] {
        let mut o = Object::construct(
            Definition {
                source: "test".into(),
                object,
                guid,
                slots: 0,
            },
            &p,
            0,
        )
        .unwrap();
        o.headline = c.slot.then_some(State {
            duration: c.active,
            sprite: 123,
            frames: 3,
            frame: 2,
            step: 1,
            ticks: 8,
            background: 101,
            overlay: 103,
            priority: c.priority,
            ..Default::default()
        });
        o.temps[0] = c.temp;
        world.objects.insert(id, o);
    }
    world.person = Some(person::WorldState {
        neighbors: BTreeMap::from([
            (
                1,
                person::Neighbor {
                    guid: 203,
                    ..Default::default()
                },
            ),
            (
                3,
                person::Neighbor {
                    guid: 203,
                    ..Default::default()
                },
            ),
        ]),
        neighbor_slots: Some(vec![Some(1), None, Some(3)]),
        ..Default::default()
    });
    let mut vm = Vm::new(world, 1);
    let t = vm
        .spawn(
            &p,
            Start {
                owner: "test",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: c.target,
                args: c.args.clone(),
                check: false,
            },
        )
        .unwrap();
    let f = vm.threads.get_mut(&t).unwrap().stack.last_mut().unwrap();
    f.locals = c.locals.clone();
    f.continuation = c.continuation;
    (p, vm, t)
}
fn execute(c: &Case) -> (Program, Vm, ThreadId) {
    let (p, mut v, t) = setup(c);
    v.run_slice(&p, t, 1).unwrap();
    (p, v, t)
}
fn h(v: &Vm) -> &State {
    v.world.objects[&1].headline.as_ref().unwrap()
}
fn succeeded(v: &Vm, t: ThreadId) {
    assert!(
        matches!(v.threads[&t].status, Status::Returned(true)),
        "{:?}",
        v.threads[&t].status
    );
}

#[test]
fn live_catalog_neighbor_and_raw_local_selectors() {
    for (c, guid) in [
        (Case::default(), Some(103)),
        (
            Case {
                group: 1,
                ..Case::default()
            },
            Some(109),
        ),
        (
            Case {
                group: 2,
                ..Case::default()
            },
            None,
        ),
        (
            Case {
                selector: 1,
                group: 1,
                ..Case::default()
            },
            Some(203),
        ),
        (
            Case {
                selector: 2,
                ..Case::default()
            },
            Some(101),
        ),
        (
            Case {
                selector: 2,
                flags: 2,
                ..Case::default()
            },
            Some(103),
        ),
        (
            Case {
                selector: 2,
                flags: 65534,
                ..Case::default()
            },
            Some(101),
        ),
        (
            Case {
                selector: 2,
                flags: 65532,
                ..Case::default()
            },
            Some(103),
        ),
        (
            Case {
                selector: 2,
                flags: 65524,
                ..Case::default()
            },
            Some(103),
        ),
        (
            Case {
                selector: 2,
                flags: 65520,
                ..Case::default()
            },
            Some(103),
        ),
    ] {
        let (_, v, t) = execute(&c);
        succeeded(&v, t);
        assert_eq!(
            (
                h(&v).sprite,
                h(&v).portrait,
                h(&v).frames,
                h(&v).frame,
                h(&v).step
            ),
            (65535, guid, 1, 0, 1),
            "{c:?}"
        );
    }
    // Reverse animation and temp-adjusted indices do not change portrait selection.
    let (_, v, t) = execute(&Case {
        selector: 2,
        options: 0x1f02,
        duration: 4,
        ..Case::default()
    });
    succeeded(&v, t);
    assert_eq!(
        (
            h(&v).portrait,
            h(&v).duration,
            h(&v).background,
            h(&v).overlay,
            h(&v).flag
        ),
        (Some(101), 48, 102, 103, true)
    );
}

#[test]
fn no_op_clear_and_error_ordering_preserve_prior_state() {
    for c in [
        Case {
            target: 2,
            ..Case::default()
        },
        Case {
            selector: 255,
            ..Case::default()
        },
        Case {
            selector: 1,
            neighbor_resource: false,
            ..Case::default()
        },
    ] {
        let (_, v, t) = execute(&c);
        succeeded(&v, t);
        assert_eq!(
            (h(&v).sprite, h(&v).frame, h(&v).ticks, h(&v).priority),
            (123, 2, 8, 1)
        );
    }
    for (active, duration, expected_priority, expected_ticks) in [(1, 20, 2, 8), (0, 0, 1, 0)] {
        let (_, v, t) = execute(&Case {
            selector: 2,
            flags: 6,
            active,
            duration,
            priority: 2,
            ..Case::default()
        });
        succeeded(&v, t);
        assert_eq!(
            (h(&v).priority, h(&v).ticks, h(&v).sprite),
            (expected_priority, expected_ticks, 123)
        );
    }
    let (_, v, t) = execute(&Case {
        selector: 2,
        flags: 6,
        ..Case::default()
    });
    assert!(matches!(&v.threads[&t].status, Status::Faulted(f) if f.message.contains("0x33")));
    assert_eq!(h(&v).priority, 1); // priority write precedes the native error
    let (_, v, t) = execute(&Case {
        selector: 2,
        flags: 65530,
        ..Case::default()
    });
    assert!(matches!(&v.threads[&t].status, Status::Faulted(f) if f.message.contains("opaque")));
    let (_, v, t) = execute(&Case {
        target: 2,
        flags: 1,
        slot: false,
        ..Case::default()
    });
    assert!(matches!(&v.threads[&t].status, Status::Faulted(f) if f.message.contains("0x15")));
    let (_, v, t) = execute(&Case {
        selector: 2,
        flags: 6,
        slot: false,
        ..Case::default()
    });
    succeeded(&v, t);
}

#[test]
fn portrait_lifetime_and_save_resume_retain_resource_after_object_removal() {
    for group in [0, 1, 2] {
        let (p, mut v, t) = execute(&Case {
            group,
            duration: 30,
            ..Case::default()
        });
        succeeded(&v, t);
        v.world.objects.remove(&3);
        // Resource identity belongs to the resource catalog, not the removed live object.
        let json = serde_json::to_string(&Snapshot::capture(&p, &v)).unwrap();
        let mut restored: Vm = serde_json::from_str::<Snapshot>(&json)
            .unwrap()
            .restore(&p)
            .unwrap();
        for tick in 1..40 {
            v.tick = tick;
            restored.tick = tick;
            v.update_headline(1).unwrap();
            restored.update_headline(1).unwrap();
            assert_eq!(v, restored);
        }
        let mut invalid = Snapshot::capture(&p, &v);
        invalid
            .state
            .world
            .objects
            .get_mut(&1)
            .unwrap()
            .headline
            .as_mut()
            .unwrap()
            .portrait = Some(999999);
        assert!(invalid.restore(&p).is_err());
        let mut invalid = Snapshot::capture(&p, &v);
        invalid
            .state
            .world
            .objects
            .get_mut(&1)
            .unwrap()
            .headline
            .as_mut()
            .unwrap()
            .step = -1;
        assert!(invalid.restore(&p).is_err());
    }
}

#[test]
#[ignore = "private original-code report supplied by SIMS_HEADLINE_PORTRAIT_REPORT"]
fn compare_original_portrait_primitive() {
    let path = std::env::var("SIMS_HEADLINE_PORTRAIT_REPORT").unwrap();
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    let mut compared = 0;
    let mut opaque = 0;
    for row in report["cases"].as_array().unwrap() {
        let c: Case = serde_json::from_value(row["input"].clone()).unwrap();
        let (_, v, t) = execute(&c);
        let n = &row["original"];
        if matches!(&v.threads[&t].status, Status::Faulted(f) if f.message.contains("opaque")) {
            assert_eq!(c.flags, 65530);
            opaque += 1;
            continue;
        }
        if let Status::Faulted(f) = &v.threads[&t].status {
            assert!(
                f.message
                    .contains(&format!("0x{:x}", n["error"].as_u64().unwrap())),
                "{c:?}: {f:?}"
            );
        } else {
            succeeded(&v, t);
            assert_eq!(n["returned"], 1);
            assert!(n["error"].is_null());
        }
        for (id, key) in [(1, "caller"), (3, "target")] {
            let fallback = State {
                duration: c.active,
                sprite: 123,
                frames: 3,
                frame: 2,
                step: 1,
                ticks: 8,
                background: 101,
                overlay: 103,
                priority: c.priority,
                ..Default::default()
            };
            let s = v.world.objects[&id].headline.as_ref().unwrap_or(&fallback);
            let words = json!([
                s.duration,
                s.sprite as i16 as i32,
                s.portrait.unwrap_or(0),
                s.frames,
                s.frame,
                s.step,
                s.ticks,
                s.background,
                s.overlay,
                s.priority,
                i32::from(s.flag)
            ]);
            assert_eq!(words, n[key], "{c:?} {key}");
        }
        compared += 1;
    }
    eprintln!(
        "{compared} original portrait cases match; {opaque} explicit opaque-pointer boundaries"
    );
}
