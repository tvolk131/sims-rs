//!Compare the4226bc caller against recorded unchanged original instructions.
//!The corpus supplies virtual child effects; live children have separate tests.
use serde_json::{Value, json};
use sims_vm::cursor::orientation::{Call, Reply, Request, View, camera_facing};

struct State {
    furniture: bool,
    target: usize,
    facings: [i16; 3],
    position: [i32; 2],
    level: i32,
}
impl View for State {
    fn has_furniture(&self) -> Result<bool, String> {
        Ok(self.furniture)
    }
    fn facing(&self, furniture: bool) -> Result<i16, String> {
        Ok(self.facings[if furniture { self.target } else { 0 }])
    }
    fn position(&self) -> Result<[i32; 2], String> {
        Ok(self.position)
    }
    fn level(&self) -> Result<i32, String> {
        Ok(self.level)
    }
}
#[test]
#[ignore = "private original orientation corpus; generate only while Cargo/rustc idle"]
fn original_cursor_orientation_matches_callback_order_live_reads_and_attempt_bound() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/cursor-orientation-1.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["native_execution"], true);
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 1176);
    for (n, row) in cases.iter().enumerate() {
        let c = &row["input"];
        for resumed in [false, true] {
            let furniture = c["furniture"].as_bool().unwrap();
            let facing = c["facing"].as_i64().unwrap() as i16;
            let mut s = State {
                furniture,
                target: 1,
                facings: [facing, facing, 6],
                position: [56, 88],
                level: 2,
            };
            let mut call = Call::new(c["direction"].as_i64().unwrap() as i32);
            let mut events = vec![];
            for k in 0..20 {
                let Some(r) = call.advance(&s).unwrap() else {
                    break;
                };
                if resumed {
                    let bytes = serde_json::to_vec(&call).unwrap();
                    let restored: Call = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(restored, call);
                    call = restored;
                    assert_eq!(call.advance(&s).unwrap(), Some(r));
                }
                let mutation = c["mutation"] == true;
                let reply = match r {
                    Request::Detach => {
                        events.push(json!(["Detach"]));
                        if mutation {
                            s.position = [701, 702];
                            s.level = 3;
                            s.target = 2;
                        }
                        Reply::Done
                    }
                    Request::Rotate => {
                        events.push(json!(["Rotate"]));
                        let target = if s.furniture { s.target } else { 0 };
                        s.facings[target] = (i32::from(s.facings[target])
                            + c["step"].as_i64().unwrap() as i32)
                            as i16
                            & 7;
                        if mutation {
                            s.position = [801, 802];
                            s.level = 4;
                        }
                        Reply::Done
                    }
                    Request::Check { position, level } => {
                        events.push(json!(["Check", position, level]));
                        if mutation {
                            s.level = 5;
                        }
                        Reply::Checked {
                            accepted: c["accepted"] == true,
                            position: if mutation { [901, 902] } else { position },
                        }
                    }
                    Request::Place { position, level } => {
                        events.push(json!(["Place", position, level]));
                        Reply::Done
                    }
                };
                call.acknowledge(r, reply).unwrap();
                assert!(k < 19);
            }
            assert!(call.complete());
            assert_eq!(
                json!({"events":events,"facings":s.facings,"position":s.position,"level":s.level}),
                row["original"],
                "case{n}: {c}, restored={resumed}"
            );
        }
    }
    let cameras = report["cameras"].as_array().unwrap();
    assert_eq!(cameras.len(), 8);
    for c in cameras {
        assert_eq!(
            i64::from(camera_facing(c["quarter"].as_i64().unwrap() as i32)),
            c["original"].as_i64().unwrap()
        );
    }
}

#[test]
fn cursor_orientation_wrong_replies_leave_the_pending_call_unchanged() {
    let s = State {
        furniture: true,
        target: 1,
        facings: [0; 3],
        position: [56, 88],
        level: 2,
    };
    let mut c = Call::new(2);
    assert_eq!(c.advance(&s).unwrap(), Some(Request::Detach));
    let before = c.clone();
    assert!(c.acknowledge(Request::Rotate, Reply::Done).is_err());
    assert_eq!(c, before);
    assert!(
        c.acknowledge(
            Request::Detach,
            Reply::Checked {
                accepted: true,
                position: [0; 2]
            }
        )
        .is_err()
    );
    assert_eq!(c, before);
}
