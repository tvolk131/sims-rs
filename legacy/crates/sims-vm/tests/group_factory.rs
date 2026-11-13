use serde_json::{Value, json};
use sims_formats::{
    behavior::{Behavior, Instruction},
    execution::ObjectFunction,
};
use sims_vm::{group_factory::Factory, *};
use std::collections::BTreeMap;

fn expression(scope: u8, index: i16, op: u8, value: i16, yes: u8) -> Instruction {
    let mut operands = [0; 8];
    operands[..2].copy_from_slice(&index.to_le_bytes());
    operands[2..4].copy_from_slice(&value.to_le_bytes());
    operands[5..].copy_from_slice(&[op, scope, 7]);
    Instruction {
        opcode: 2,
        on_true: yes,
        on_false: 254,
        operands,
    }
}
fn routine(code: Vec<Instruction>) -> Routine {
    Routine {
        name: "synthetic".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args: 0,
            locals: 0,
            instructions: code,
        },
    }
}
fn fixture(c: &Value) -> (Program, Vm) {
    let mut m = Module {
        source: "group".into(),
        ..Default::default()
    };
    let init = if c["mutate_price"].as_bool().unwrap() {
        vec![expression(3, 11, 2, 1, 1), expression(3, 41, 5, -777, 254)]
    } else {
        vec![expression(7, 0, 2, 0, 254)]
    };
    m.routines = BTreeMap::from([
        (4096, routine(init)),
        (4097, routine(vec![expression(7, 0, 2, 0, 254)])),
    ]);
    for i in 0..=c["count"].as_u64().unwrap() as u16 {
        let mut w = vec![0; 108];
        w[9] = 4;
        w[10] = 7;
        w[11] = if i == 0 {
            -1
        } else {
            c["order"][i as usize - 1].as_i64().unwrap() as i16
        };
        w[14] = 100 + i as i16;
        w[18] = if i == 0 {
            c["price"].as_i64().unwrap() as i16
        } else {
            50 + i as i16
        };
        w[63] = i16::from(i > 0 && i64::from(w[11]) == c["primary"].as_i64().unwrap());
        m.object_definition_order.push(i);
        m.object_definition_words.insert(i, w);
        m.object_functions.insert(
            i,
            vec![
                ObjectFunction {
                    action: 4096,
                    condition: 0,
                },
                ObjectFunction {
                    action: 4097,
                    condition: 0,
                },
            ],
        );
    }
    let p = Program {
        globals: "group".into(),
        modules: BTreeMap::from([("group".into(), m)]),
        ..Default::default()
    };
    let w = World {
        scheduler_storage: Some(Default::default()),
        person: Some(Default::default()),
        refresh: Some(Default::default()),
        ..Default::default()
    };
    (p, Vm::new(w, 1))
}
fn state(v: &Vm, ids: &[ObjectId]) -> Value {
    json!(ids.iter().map(|id| {
        let o=&v.world.objects[id];let part=o.multitile.as_ref().unwrap();
        json!({"id":id,"resource":o.definition.as_ref().unwrap().object,"head":part.head.unwrap_or(0),
            "next":part.next.unwrap_or(0),"price":o.data[&41],"direction":o.data[&1],
            "origin":part.origin,"offset":part.offset})
    }).collect::<Vec<_>>())
}
#[cfg(feature = "private-fixtures")]
#[test]
fn multipart_factory_matches_native_visibility_price_and_order() {
    let report: Value = serde_json::from_str(include_str!("fixtures/group-factory.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let (p, mut v) = fixture(&row["input"]);
        let mut f = Factory::begin(&mut v, &p, "group", 0, Default::default(), [0, 0]).unwrap();
        assert_eq!(
            f.head(),
            row["original"]["returned"].as_i64().unwrap() as i16
        );
        let expected: Vec<_> = row["original"]["order"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["event"] == "init" || e["event"] == "main")
            .cloned()
            .collect();
        let mut observed = vec![];
        while let Some(id) = f.current(&mut v, &p).unwrap() {
            observed.push(json!({"event":"init","object":id,"parts":state(&v,&f.parts)}));
            let thread = v.object_threads[&id];
            let mut main_seen = false;
            loop {
                if v.threads[&thread]
                    .stack
                    .last()
                    .is_some_and(|frame| frame.location.bhav == 4097)
                    && !main_seen
                {
                    observed.push(json!({"event":"main","object":id,"parts":state(&v,&f.parts)}));
                    main_seen = true;
                }
                f.validate(&v, &p).unwrap();
                let snapshot = Snapshot::capture(&p, &v);
                let mut resumed = snapshot.restore(&p).unwrap();
                let saved_f: Factory =
                    serde_json::from_slice(&serde_json::to_vec(&f).unwrap()).unwrap();
                saved_f.validate(&resumed, &p).unwrap();
                assert_eq!(
                    v.run_slice(&p, thread, 1).unwrap(),
                    resumed.run_slice(&p, thread, 1).unwrap()
                );
                assert_eq!(v, resumed);
                if matches!(
                    v.threads[&thread].status,
                    Status::Returned(_) | Status::Sleeping { .. }
                ) {
                    break;
                }
            }
            f.finish_current(&mut v, &p).unwrap();
        }
        assert_eq!(observed, expected, "{}", row["input"]);
        assert_eq!(state(&v, &f.parts), row["original"]["parts"]);
        f.validate(&v, &p).unwrap();
        let mut bad = f.clone();
        bad.parts.reverse();
        if bad.parts.len() > 1 {
            assert!(bad.validate(&v, &p).is_err());
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn staged_group_postconstruction_matches_native_registration_visibility_and_shared_reset() {
    use sims_vm::group_factory::Construction;
    let report: Value = serde_json::from_str(include_str!("fixtures/group-factory.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let (p, mut vm) = fixture(&row["input"]);
        let mut reference = vm.clone();
        let now = construction::ObjectDate {
            year: 1997,
            month: 3,
            day: 12,
        };
        let time = [13, 25];
        let expected = Factory::begin(&mut reference, &p, "group", 0, now, time).unwrap();
        let mut c = Construction::new(&p, "group", 0, now, time).unwrap();
        let mut observed = vec![];
        while let Some(id) = c.register_next(&mut vm, &p).unwrap() {
            for event in ["register", "postconstruct"] {
                observed.push(json!({"event":event,"object":id,"parts":state(&vm,c.parts())}));
            }
            assert!(vm.object_threads.is_empty() && vm.lifecycles.is_empty());
            assert_eq!(vm.world.objects[&id].data[&45], 0);
            assert!(c.register_next(&mut vm, &p).is_err());
            assert!(c.finish_part(&mut vm, &p, id + 100).is_err());
            assert!(c.finish(&mut vm, &p).is_err());
            let before = vm.clone();
            c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
            let mut bad = Snapshot::capture(&p, &vm);
            bad.state
                .world
                .objects
                .get_mut(&id)
                .unwrap()
                .multitile
                .as_mut()
                .unwrap()
                .origin = [99; 3];
            assert!(
                bad.restore_with_group_construction(&p, &[], &[], &[], &[], &[&c])
                    .is_err()
            );
            assert!(
                Snapshot::capture(&p, &vm)
                    .restore_with_group_construction(&p, &[], &[], &[], &[], &[&c, &c])
                    .is_err()
            );
            let words = construction::definition_words(
                &p,
                vm.world.objects[&id].definition.as_ref().unwrap(),
            )
            .unwrap();
            if multitile::origin(words).unwrap() != [0; 3] {
                assert!(Snapshot::capture(&p, &vm).restore(&p).is_err());
            }
            vm = Snapshot::capture(&p, &vm)
                .restore_with_group_construction(&p, &[], &[], &[], &[], &[&c])
                .unwrap();
            c.validate(&vm, &p).unwrap();
            assert_eq!(vm, before);
            c.finish_part(&mut vm, &p, id).unwrap();
            assert!(c.finish_part(&mut vm, &p, id).is_err());
            observed.push(json!({"event":"date","object":id,"parts":state(&vm,c.parts())}));
            assert_eq!(vm.world.objects[&id].data[&45], 1997);
        }
        let original: Vec<_> = row["original"]["order"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| {
                matches!(
                    r["event"].as_str(),
                    Some("register" | "postconstruct" | "date")
                )
            })
            .cloned()
            .collect();
        assert_eq!(observed, original, "{}", row["input"]);
        let f = c.finish(&mut vm, &p).unwrap();
        assert_eq!(f, expected);
        assert_eq!(vm, reference);
    }
}
