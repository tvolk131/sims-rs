use sims_formats::{
    behavior::{Behavior, Instruction},
    career::{Career, Level},
};
use sims_vm::*;
#[cfg(feature = "private-fixtures")]
#[test]
fn native_career_scope_school_override_bounds_and_truncation() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/career-fields.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let records = report["records"].as_array().unwrap();
    let mut m = Module {
        source: "test".into(),
        ..Default::default()
    };
    m.routines.insert(
        4096,
        Routine {
            name: "probe".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args: 0,
                locals: 0,
                instructions: vec![Instruction {
                    opcode: 2,
                    on_true: 254,
                    on_false: 255,
                    operands: [0; 8],
                }],
            },
        },
    );
    let mut p = Program {
        modules: [("test".into(), m)].into(),
        careers: Some(program::CareerCatalog {
            text: None,
            source: "synthetic".into(),
            content_hash: "synthetic".into(),
            ids: records
                .iter()
                .map(|r| r["id"].as_i64().unwrap() as i32)
                .collect(),
            records: Some(
                records
                    .iter()
                    .map(|r| Career {
                        version: 0,
                        packed: true,
                        name: String::new(),
                        consumed: 0,
                        levels: r["levels"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|l| Level {
                                values: serde_json::from_value(l["values"].clone()).unwrap(),
                                strings: [
                                    String::new(),
                                    l["vehicle"].as_str().unwrap().into(),
                                    String::new(),
                                    String::new(),
                                    String::new(),
                                ],
                            })
                            .collect(),
                    })
                    .collect(),
            ),
        }),
        ..Default::default()
    };
    let mut v = Vm::new(
        World {
            objects: [(1, Object::default())].into(),
            ..Default::default()
        },
        1,
    );
    let thread = v
        .spawn(
            &p,
            Start {
                owner: "test",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 0,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let e = &row["original"];
        v.world.objects.get_mut(&1).unwrap().temps[..2].copy_from_slice(&[
            c["career"].as_i64().unwrap() as i16,
            c["level"].as_i64().unwrap() as i16,
        ]);
        let result = v.inspect_memory(
            &p,
            thread,
            33,
            c["field"].as_i64().unwrap() as i16,
            c["write"].as_bool().unwrap(),
        );
        if let Some(code) = e["error"].as_u64() {
            assert!(
                result
                    .unwrap_err()
                    .contains(&format!("native error 0x{code:02x}")),
                "{c}"
            );
        } else {
            let r = result.unwrap();
            assert_eq!(r.value, e["value"].as_i64().unwrap() as i16, "{c}");
            assert!(!r.writable && !r.float_pointer);
        }
    }
    p.careers.as_mut().unwrap().records = None;
    v.world.objects.get_mut(&1).unwrap().temps[0] = 1;
    assert!(
        v.inspect_memory(&p, thread, 33, 0, false)
            .unwrap_err()
            .contains("data unavailable")
    );
}
