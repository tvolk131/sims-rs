use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_career_vector_order_missing_cursor_and_exhaustion() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/career-enumeration.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let e = &row["original"];
        let scope = c["scope"].as_u64().unwrap() as u8;
        let mut p = Program {
            careers: Some(program::CareerCatalog {
                text: None,
                source: "synthetic".into(),
                content_hash: "synthetic".into(),
                ids: serde_json::from_value(c["ids"].clone()).unwrap(),
                records: None,
            }),
            globals: "test".into(),
            modules: BTreeMap::from([(
                "test".into(),
                Module {
                    source: "test".into(),
                    routines: BTreeMap::from([(
                        4096,
                        Routine {
                            name: "career iterator".into(),
                            body: Behavior {
                                format: 0x8002,
                                kind: 0,
                                args: 0,
                                locals: 0,
                                instructions: vec![Instruction {
                                    opcode: 31,
                                    on_true: 254,
                                    on_false: 255,
                                    operands: [0, 0, 0, 0, 138, scope, 0, 0],
                                }],
                            },
                        },
                    )]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let current = c["current"].as_i64().unwrap() as i16;
        let mut w = World::default();
        let mut o = Object::default();
        o.temps[0] = current;
        w.objects.insert(1, o);
        let mut v = Vm::new(w, 1);
        let t = v
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: current,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        let checkpoint = Snapshot::capture(&p, &v);
        let mut resumed = checkpoint.clone().restore(&p).unwrap();
        let a = v.run_slice(&p, t, 1).unwrap();
        assert_eq!(a, resumed.run_slice(&p, t, 1).unwrap());
        assert_eq!(v, resumed);
        assert_eq!(e["error"], Value::Null, "{c}");
        assert_eq!(
            a.stop,
            Stop::Returned(e["returned"].as_bool().unwrap()),
            "{c}: {a:?}"
        );
        if scope == 8 {
            assert_eq!(json!(v.world.objects[&1].temps[0]), e["value"], "{c}");
        }
        let expected = if e["returned"] == true {
            format!("→ {}", e["value"])
        } else {
            "destination retained".into()
        };
        assert!(a.trace[0].effect.contains(&expected), "{c}: {a:?}");
        p.careers.as_mut().unwrap().ids.push(999);
        assert!(
            checkpoint.restore(&p).is_err(),
            "career catalog is part of program identity"
        );
    }
}
