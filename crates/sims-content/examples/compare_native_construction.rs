//! Differential base construction/reset and every executable chair Init frame.
use serde_json::{Value, json};
use sims_content::{Index, vm::load_program};
use sims_vm::{construction::ObjectDate, refresh::DrawState, *};
use std::{collections::BTreeMap, path::Path};
fn frames(t: &Thread) -> Value {
    let mut frames = vec![];
    for (i, f) in t.stack.iter().enumerate() {
        if t.callback.as_ref().is_some_and(|b| b.base_depth == i) {
            frames.push(json!({"bhav":65535,"pc":0,"stack_object":0,"args":[],"locals":[],"offset":f.storage_offset-8}));
        }
        frames.push(json!({"bhav":f.location.bhav,"pc":f.location.instruction,"stack_object":f.stack_object,"args":f.args,"locals":f.locals,"offset":f.storage_offset}));
    }
    json!(frames)
}
fn storage(v: &Vm) -> Value {
    let o = &v.world.objects[&1];
    let d = &v.world.refresh.as_ref().unwrap().draw[&1];
    json!({"data":(0..72).map(|i|o.data[&i]).collect::<Vec<_>>(),"attributes":o.attributes,"temps":o.temps,"draw":[u32::from(d.dynamic),d.expiry as u32,d.refreshed_at]})
}
fn synthetic_callbacks(report: &Value) -> usize {
    use sims_formats::{
        behavior::{Behavior, Instruction},
        execution::ObjectFunction,
    };
    let expr = |scope: u8, index: i16, value: i16, yes: u8| {
        let mut operands = [0; 8];
        operands[..2].copy_from_slice(&index.to_le_bytes());
        operands[2..4].copy_from_slice(&value.to_le_bytes());
        operands[5..].copy_from_slice(&[5, scope, 7]);
        Instruction {
            opcode: 2,
            on_true: yes,
            on_false: 253,
            operands,
        }
    };
    let routine = |args, locals, instructions| Routine {
        name: "callback probe".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args,
            locals,
            instructions,
        },
    };
    let mut count = 0;
    for row in report["callbacks"].as_array().unwrap() {
        let c = &row["input"];
        let expected = &row["original"];
        let mut last = expr(0, 1, 1, 254);
        last.operands[7] = 25;
        let m = Module {
            source: "fixture".into(),
            routines: BTreeMap::from([
                (
                    4096,
                    routine(
                        4,
                        4,
                        vec![Instruction {
                            opcode: 4109,
                            on_true: 254,
                            on_false: 255,
                            operands: [0; 8],
                        }],
                    ),
                ),
                (
                    4108,
                    routine(
                        4,
                        4,
                        vec![
                            expr(8, 0, 55, 1),
                            expr(25, 0, 77, 2),
                            expr(0, 0, 42, c["return"].as_u64().unwrap() as u8),
                        ],
                    ),
                ),
                (
                    4109,
                    routine(0, 8, vec![expr(25, 6, 123, 1), expr(25, 7, 456, 2), last]),
                ),
            ]),
            object_functions: BTreeMap::from([(
                1,
                vec![
                    ObjectFunction {
                        action: 4108,
                        condition: 0,
                    },
                    ObjectFunction {
                        action: 4096,
                        condition: 0,
                    },
                ],
            )]),
            ..Default::default()
        };
        let p = Program {
            globals: "fixture".into(),
            modules: BTreeMap::from([("fixture".into(), m)]),
            ..Default::default()
        };
        let mut w = World::default();
        w.objects.insert(
            1,
            Object {
                definition: Some(Definition {
                    source: "fixture".into(),
                    object: 1,
                    guid: 0,
                    slots: 0,
                }),
                temps: [1, 2, 3, 0, 7, -1, 8, 255],
                attributes: vec![100, 101, 102, 103],
                ..Default::default()
            },
        );
        let mut v = Vm::new(w, 1);
        let id = v.start_object_lifecycle(&p, 1).unwrap();
        v.threads.get_mut(&id).unwrap().stack[0].locals = vec![101, 102, 103, 104];
        for key in ["callback_steps", "main_steps"] {
            for step in expected[key]
                .as_array()
                .unwrap()
                .iter()
                .filter(|s| s["pc"].as_u64().unwrap() < 254)
            {
                let mut native = step["frames"].clone();
                for (i, f) in v.threads[&id].stack.iter().enumerate() {
                    let native_index = i + usize::from(
                        v.threads[&id]
                            .callback
                            .as_ref()
                            .is_some_and(|b| i >= b.base_depth),
                    );
                    for &local in &f.opaque_locals {
                        native[native_index]["locals"][local] = json!(0);
                    }
                }
                assert_eq!(frames(&v.threads[&id]), native, "{key} {step}");
                let mut replay = Snapshot::capture(&p, &v).restore(&p).unwrap();
                assert_eq!(
                    v.run_slice(&p, id, 1).unwrap(),
                    replay.run_slice(&p, id, 1).unwrap()
                );
                assert_eq!(v, replay);
                count += 1;
            }
            if key == "callback_steps" {
                assert_eq!(frames(&v.threads[&id]), expected["after_callback"]);
                assert_eq!(
                    u32::from(v.threads[&id].callback_result.unwrap()),
                    expected["returned"].as_u64().unwrap() as u32
                );
            }
        }
        assert_eq!(
            json!(v.world.objects[&1].attributes),
            expected["attributes"]
        );
        assert_eq!(v.threads[&id].status, Status::Returned(true));
    }
    count
}
fn main() -> anyhow::Result<()> {
    let report: Value = serde_json::from_slice(&std::fs::read(
        std::env::args().nth(1).expect("private report"),
    )?)?;
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let callbacks = synthetic_callbacks(&report);
    println!(
        "{callbacks} native callback return/reused-arena checkpoints match, including opaque locals overwritten before use"
    );
    let index = Index::scan(Path::new("game-data/cabinet-extracted"))?;
    let p = load_program(&index, "DiningChairs.iff", "Global.iff")?;
    let owner = p
        .modules
        .keys()
        .find(|s| s.ends_with("DiningChairs.iff"))
        .unwrap()
        .clone();
    let def = Definition {
        source: owner.clone(),
        object: 16807,
        guid: 0,
        slots: 0,
    };
    for row in report["construct"].as_array().unwrap() {
        let mut p = p.clone();
        p.modules
            .get_mut(&owner)
            .unwrap()
            .object_definition_words
            .get_mut(&16807)
            .unwrap()[0x74 / 2] = row["input"].as_i64().unwrap() as i16;
        let o = Object::construct(def.clone(), &p, 0).unwrap();
        let mut v = Vm::new(World::default(), 1);
        v.world.objects.insert(1, o);
        v.world.refresh = Some(refresh::RefreshState {
            draw: BTreeMap::from([(1, DrawState::constructed(0))]),
            ..Default::default()
        });
        assert_eq!(storage(&v), row["original"], "constructor {}", row["input"]);
    }
    let mut comparisons = 0;
    for row in report["reset"].as_array().unwrap() {
        let c = &row["input"];
        let expected = &row["original"];
        let mut p = p.clone();
        let words = p
            .modules
            .get_mut(&owner)
            .unwrap()
            .object_definition_words
            .get_mut(&16807)
            .unwrap();
        words[0x14 / 2] = c["master"].as_i64().unwrap() as i16;
        words[0x62 / 2] = c["definition_value"].as_i64().unwrap() as i16;
        let mut o = Object::construct(def.clone(), &p, 1).unwrap();
        o.data = (0..72).map(|i| (i, 100 + i)).collect();
        o.temps = [1, 2, 3, 0, 7, -1, 8, 255];
        o.attributes.fill(99);
        for (i, value) in [
            (1, c["direction"].as_i64().unwrap() as i16),
            (26, 0),
            (14, -1),
            (29, 1),
        ] {
            o.data.insert(i, value);
        }
        for (i, x) in c["created"].as_array().unwrap().iter().enumerate() {
            o.data.insert(45 + i as i16, x.as_i64().unwrap() as i16);
        }
        let mut w = World::default();
        if let Some(slot) = c["container"].as_u64() {
            o.container = Some((2, slot as u16));
            let mut parent = Object {
                slots: vec![None; slot as usize + 1],
                ..Default::default()
            };
            parent.slots[slot as usize] = Some(1);
            w.objects.insert(2, parent);
        }
        w.objects.insert(1, o);
        w.refresh = Some(refresh::RefreshState {
            visible_level: 1,
            render_clock: 1234,
            draw: BTreeMap::from([(1, DrawState::constructed(1))]),
            rooms: BTreeMap::from([(1, Default::default())]),
            ..Default::default()
        });
        if w.objects.contains_key(&2) {
            w.refresh
                .as_mut()
                .unwrap()
                .draw
                .insert(2, DrawState::constructed(1));
        }
        let mut v = Vm::new(w, 1);
        let now = ObjectDate {
            year: c["now"][0].as_i64().unwrap() as i16,
            month: c["now"][1].as_i64().unwrap() as i16,
            day: c["now"][2].as_i64().unwrap() as i16,
        };
        let id = v.reset_object_lifecycle(&p, 1, now).unwrap();
        for step in expected["steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["pc"].as_u64().unwrap() < 254)
        {
            assert_eq!(frames(&v.threads[&id]), step["frames"], "{c}; {step}");
            let mut resumed = Snapshot::capture(&p, &v).restore(&p).unwrap();
            let a = v.run_slice(&p, id, 1).unwrap();
            let b = resumed.run_slice(&p, id, 1).unwrap();
            assert_eq!(a, b);
            assert_eq!(v, resumed);
            assert_ne!(a.stop, Stop::Faulted, "{a:?}");
            comparisons += 1;
        }
        assert_eq!(v.lifecycles[&1].phase, lifecycle::Phase::Main);
        assert_eq!(storage(&v), expected["storage"], "reset {c}");
        assert_eq!(frames(&v.threads[&id]), expected["frames"]);
        assert_eq!(
            v.world.objects[&1].idle.as_ref().unwrap().countdown as u16,
            u16::MAX
        );
        assert_eq!(
            v.world.refresh.as_ref().unwrap().invalidated.len(),
            expected["invalidations"].as_u64().unwrap() as usize
        );
        // Verify retained Main and sentinel arena survive the completed callback.
        assert_eq!(Snapshot::capture(&p, &v).restore(&p).unwrap(), v);
    }
    println!(
        "{} native constructor cases, {} reset cases and {comparisons} original Init instruction/frame checkpoints match (including save/resume)",
        report["construct"].as_array().unwrap().len(),
        report["reset"].as_array().unwrap().len()
    );
    Ok(())
}
