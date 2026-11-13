//! Compare Rust semantic state to the private original-code oracle report.
use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{
    idle::IdleState,
    refresh::{DrawState, RefreshState, RoomState},
    *,
};
use std::collections::BTreeMap;
fn n(v: &Value, k: &str) -> i16 {
    v[k].as_i64().unwrap() as i16
}
fn routine(op: u16, index: i16) -> Routine {
    let mut operands = [0; 8];
    operands[..2].copy_from_slice(&index.to_le_bytes());
    Routine {
        name: "probe".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args: 4,
            locals: 0,
            instructions: vec![Instruction {
                opcode: op,
                on_true: 0,
                on_false: 0,
                operands,
            }],
        },
    }
}
fn program() -> Program {
    Program {
        globals: "p".into(),
        modules: BTreeMap::from([(
            "p".into(),
            Module {
                source: "p".into(),
                routines: BTreeMap::from([(4096, routine(0, 0)), (4097, routine(0x31, 0))]),
                ..Module::default()
            },
        )]),
        ..Program::default()
    }
}
fn world() -> World {
    World {
        objects: (1..=2)
            .map(|id| {
                (
                    id,
                    Object {
                        slots: vec![None],
                        idle: Some(IdleState::default()),
                        data: BTreeMap::from([(25, 0), (29, 1)]),
                        ..Object::default()
                    },
                )
            })
            .collect(),
        ..World::default()
    }
}
fn spawn(v: &mut Vm, p: &Program, caller: i16, bhav: u16, args: Vec<i16>, stack: i16) -> u64 {
    v.spawn(
        p,
        Start {
            owner: "p",
            bhav,
            caller,
            callee: caller,
            stack_object: stack,
            args,
            check: false,
        },
    )
    .unwrap()
}
fn error(e: Option<&str>) -> Value {
    e.map(|e| {
        json!(
            u32::from_str_radix(
                e.split("native error 0x")
                    .nth(1)
                    .unwrap()
                    .split(':')
                    .next()
                    .unwrap(),
                16
            )
            .unwrap()
        )
    })
    .unwrap_or(Value::Null)
}
fn main() {
    let input = std::env::args().nth(1).expect("private report path");
    let report: Value = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let mut count = 0;
    let mut boundaries = 0;
    for row in report["refresh"].as_array().unwrap() {
        let c = &row["input"];
        let mut v = Vm::new(world(), 1);
        let state = RefreshState {
            visible_level: n(c, "visible") as i32,
            render_clock: 1234,
            draw: (1..=2)
                .map(|id| {
                    (
                        id,
                        DrawState {
                            level: n(c, "level") as i32,
                            dynamic: n(c, "dynamic") != 0,
                            expiry: 88,
                            refreshed_at: 77,
                        },
                    )
                })
                .collect(),
            rooms: BTreeMap::from([(
                1,
                RoomState {
                    category: n(c, "category") as u8,
                    dirty: false,
                },
            )]),
            dirty_categories: 8,
            ..Default::default()
        };
        v.world.refresh = Some(state);
        for o in v.world.objects.values_mut() {
            o.data.insert(29, n(c, "room"));
        }
        if c["contained"] == true {
            v.world.objects.get_mut(&2).unwrap().container = Some((1, 0));
            v.world.objects.get_mut(&1).unwrap().slots[0] = Some(2);
        }
        let result = v.refresh_object(1, n(c, "stack"), n(c, "selector"), n(c, "mode"));
        // This historical corpus supplied an empty native Commander list.
        // Actual F2 consumers are covered by native_room_signals.rs instead.
        if n(c, "mode") == 1 && result.is_ok() {
            assert_eq!(row["original"]["returned"], 1);
            boundaries += 1;
            continue;
        }
        let s = v.world.refresh.as_ref().unwrap();
        let draw: Vec<_> = (1..=2)
            .map(|i| {
                let d = &s.draw[&i];
                json!([i32::from(d.dynamic), d.expiry, d.refreshed_at])
            })
            .collect();
        let actual = json!({"error":error(result.as_ref().err().map(String::as_str)),"returned":if result.is_ok(){json!(1)}else{Value::Null},"draw":draw,"invalidations":s.invalidated.len(),"room_dirty":u8::from(s.rooms[&1].dirty),"categories":s.dirty_categories,"events":[]});
        assert_eq!(actual, row["original"], "Refresh {c}");
        count += 1;
    }
    for row in report["sleep"].as_array().unwrap() {
        let c = &row["input"];
        let mut p = program();
        p.modules
            .get_mut("p")
            .unwrap()
            .routines
            .insert(4096, routine(0, n(c, "index")));
        let mut v = Vm::new(world(), 1);
        let o = v.world.objects.get_mut(&1).unwrap();
        o.person = (n(c, "kind") == 2).then(BTreeMap::new);
        o.data.insert(25, n(c, "busy"));
        o.headline = (n(c, "queued") != 0).then(|| headline::State {
            duration: i32::from(n(c, "queued")),
            ..Default::default()
        });
        o.idle = Some(IdleState {
            countdown: n(c, "scheduled"),
            flags: c["flags"].as_u64().unwrap_or(0) as u16,
            last_run: Some(0),
        });
        let id = spawn(&mut v, &p, 1, 4096, vec![n(c, "counter"), 2, 3, 4], 2);
        v.run_slice(&p, id, 1).unwrap();
        let t = &v.threads[&id];
        let err = if let Status::Faulted(f) = &t.status {
            Some(f.message.as_str())
        } else {
            None
        };
        let actual = json!({"error":error(err),"returned":if err.is_some(){Value::Null}else{json!(if matches!(t.status,Status::Sleeping{..}){2}else{1})},"counter":t.stack[0].args[0],"scheduled":v.world.objects[&1].idle.as_ref().unwrap().countdown});
        assert_eq!(actual, row["original"], "Sleep {c}");
        count += 1;
    }
    for row in report["notify"].as_array().unwrap() {
        let c = &row["input"];
        let mut p = program();
        p.modules
            .get_mut("p")
            .unwrap()
            .routines
            .insert(4096, routine(n(c, "opcode") as u16, n(c, "index")));
        let mut v = Vm::new(world(), 1);
        v.world
            .objects
            .get_mut(&2)
            .unwrap()
            .idle
            .as_mut()
            .unwrap()
            .countdown = 90;
        let target = spawn(&mut v, &p, 2, 4096, vec![50, 60, 70, 80], 1);
        if c["frames"] == true {
            v.register_object_thread(2, target).unwrap();
        }
        let id = spawn(&mut v, &p, 1, 4097, vec![], n(c, "stack"));
        v.run_slice(&p, id, 1).unwrap();
        let err = if let Status::Faulted(f) = &v.threads[&id].status {
            Some(f.message.as_str())
        } else {
            None
        };
        let actual = json!({"error":error(err),"returned":if err.is_some(){Value::Null}else{json!(1)},"args":v.threads[&target].stack[0].args,"scheduled":v.world.objects[&2].idle.as_ref().unwrap().countdown});
        assert_eq!(actual, row["original"], "Notify {c}");
        count += 1;
    }
    for row in report["scheduler"].as_array().unwrap() {
        let counts = row["input"]["countdowns"].as_array().unwrap();
        let mut states: Vec<_> = counts
            .iter()
            .map(|c| IdleState {
                countdown: c.as_i64().unwrap() as i16,
                ..Default::default()
            })
            .collect();
        let updated: Vec<_> = states
            .iter_mut()
            .enumerate()
            .filter_map(|(i, s)| s.visit(1).then_some(i + 1))
            .collect();
        let actual = json!({"error":null,"updated":updated,"countdowns":states.iter().map(|s|s.countdown).collect::<Vec<_>>()});
        assert_eq!(actual, row["original"]);
        count += 1;
    }
    println!(
        "{count} native comparisons passed; {boundaries} historical subscriber-free F2 cases covered by the separate room-signal corpus"
    );
}
