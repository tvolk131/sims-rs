#![cfg(feature = "private-fixtures")]

use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{
    Module, Object, Program, Routine, Snapshot, Start, Status, Stop, Vm, World,
    ui_effect::{Kind, widgets},
};
use std::collections::BTreeMap;
fn fixture() -> Value {
    let r: Value = serde_json::from_str(include_str!("fixtures/ui-effect.json")).unwrap();
    assert_eq!(
        r["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        r["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    r
}
fn setup(b: [u8; 8], value: i16) -> (Program, Vm) {
    let mut p = Program::default();
    p.globals = "local".into();
    p.modules.insert(
        "local".into(),
        Module {
            source: "local".into(),
            routines: BTreeMap::from([(
                4096,
                Routine {
                    name: "UI effect".into(),
                    body: Behavior {
                        format: 0x8002,
                        kind: 0,
                        args: 4,
                        locals: 4,
                        instructions: vec![Instruction {
                            opcode: 0x22,
                            on_true: 254,
                            on_false: 253,
                            operands: b,
                        }],
                    },
                },
            )]),
            ..Default::default()
        },
    );
    let mut w = World::default();
    w.objects.insert(
        1,
        Object {
            attributes: vec![value; 4],
            temps: [value; 8],
            ..Default::default()
        },
    );
    let mut vm = Vm::new(w, 1);
    vm.spawn(
        &p,
        Start {
            owner: "local",
            bhav: 4096,
            caller: 1,
            callee: 1,
            stack_object: 1,
            args: vec![1, 2, 3, 4],
            check: false,
        },
    )
    .unwrap();
    local(&mut vm, value);
    (p, vm)
}
fn local(vm: &mut Vm, value: i16) {
    let t = vm.threads.get_mut(&1).unwrap();
    let f = t.stack.last_mut().unwrap();
    f.locals[0] = value;
    t.frame_memory[f.storage_offset + 8 + f.args.len()] = Some(value);
}

#[test]
fn round_stops_at_the_first_synchronous_ui_call() {
    let (p, mut vm) = setup([0, 7, 0xdd, 7, 1, 0, 0, 0], 0);
    vm.world.objects.insert(2, vm.world.objects[&1].clone());
    let second = vm
        .spawn(
            &p,
            Start {
                owner: "local",
                bhav: 4096,
                caller: 2,
                callee: 2,
                stack_object: 2,
                args: vec![0; 4],
                check: false,
            },
        )
        .unwrap();
    let before = vm.threads[&second].clone();
    let runs = vm.run_round(&p, 100);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].1.stop, Stop::AwaitingUiEffect);
    assert_eq!(vm.threads[&second], before);
}

#[test]
fn native_ui_effect_reads_again_after_control_and_restores_each_call() {
    let r = fixture();
    let rows = r["primitives"].as_array().unwrap();
    assert_eq!(rows.len(), 500);
    for row in rows {
        let c = &row["input"];
        let original = &row["original"];
        let scope = c["scope"].as_u64().unwrap() as u8;
        let value = c["value"].as_i64().unwrap() as i16;
        let index = if scope == 7 { value } else { 0 };
        let [lo, hi] = index.to_le_bytes();
        let b = [
            c["mode"].as_u64().unwrap() as u8,
            scope,
            lo,
            hi,
            c["flags"].as_u64().unwrap() as u8,
            255,
            177,
            91,
        ];
        let (p, mut vm) = setup(b, value);
        let stop = vm.run_slice(&p, 1, 100).unwrap().stop;
        if let Some(code) = original["error"].as_u64() {
            assert_eq!(stop, Stop::Faulted);
            let Status::Faulted(f) = &vm.threads[&1].status else {
                unreachable!()
            };
            assert!(
                f.message.contains(&format!("native error {code:#04x}")),
                "{f:?}"
            );
            continue;
        }
        let mut events = vec![];
        while let Status::AwaitingUiEffect(call) = &vm.threads[&1].status {
            let request = call.request();
            vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
            let before = vm.clone();
            assert!(vm.advance_tick().is_err());
            assert_eq!(vm, before);
            assert_eq!(
                vm.run_slice(&p, 1, 100).unwrap().stop,
                Stop::AwaitingUiEffect
            );
            assert_eq!(vm, before);
            events.push(json!([
                match request.kind {
                    Kind::Control => "control",
                    Kind::Portrait => "portrait",
                    Kind::Relationship => "relationship",
                },
                request.value,
                u8::from(request.enabled)
            ]));
            let replacement = c["replacement"].as_i64().unwrap() as i16;
            match scope {
                0 => vm.world.objects.get_mut(&1).unwrap().attributes[0] = replacement,
                8 => vm.world.objects.get_mut(&1).unwrap().temps[0] = replacement,
                25 => local(&mut vm, replacement),
                _ => (),
            }
            vm.complete_ui_effect(&p, 1, request).unwrap();
            let after = vm.clone();
            assert!(vm.complete_ui_effect(&p, 1, request).is_err());
            assert_eq!(vm, after);
            vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
        }
        assert_eq!(json!(events), original["events"], "{c}");
        assert_eq!(vm.threads[&1].status, Status::Returned(true));
        assert_eq!(original["returned"], 1);
    }
}

#[test]
fn borrowed_widget_search_matches_native_rtti_depth_first_order() {
    let r = fixture();
    let rows = r["lookups"].as_array().unwrap();
    assert_eq!(rows.len(), 1296);
    for row in rows {
        let c = &row["input"];
        let nodes: Vec<_> = c["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| widgets::Node {
                kind: match n["kind"].as_u64().unwrap() {
                    0 => widgets::Kind::Window,
                    1 => widgets::Kind::Button,
                    _ => widgets::Kind::Relationship,
                },
                bitmap: n["image"].as_u64().map(|i| widgets::Bitmap {
                    handle: i as u32,
                    resource: c["image_ids"][i as usize].as_i64().unwrap() as i32,
                }),
                children: serde_json::from_value(n["children"].clone()).unwrap(),
            })
            .collect();
        let query = match c["kind"].as_str().unwrap() {
            "control" => widgets::Query::Control(c["wanted"].as_i64().unwrap() as i32),
            "portrait" => widgets::Query::Portrait(c["wanted"].as_u64().unwrap() as u32),
            _ => widgets::Query::Relationship,
        };
        assert_eq!(
            json!(widgets::find(&nodes, c["root"].as_u64().map(|n| n as usize), query).unwrap()),
            row["original"],
            "{c}"
        );
    }
    let bad = vec![widgets::Node {
        kind: widgets::Kind::Window,
        bitmap: None,
        children: vec![0],
    }];
    assert!(widgets::find(&bad, Some(0), widgets::Query::Relationship).is_err());
}

#[test]
fn highlight_geometry_and_eligibility_match_original_widget_calls() {
    let r = fixture();
    let rows = r["highlights"].as_array().unwrap();
    assert_eq!(rows.len(), 98);
    for row in rows {
        let c = &row["input"];
        let target = widgets::Geometry {
            position: serde_json::from_value(c["position"].clone()).unwrap(),
            rectangle: serde_json::from_value(c["target_rect"].clone()).unwrap(),
            flags: c["flags"].as_u64().unwrap() as u32,
        };
        let rect = widgets::highlight_rectangle(
            target,
            serde_json::from_value(c["highlight_rect"].clone()).unwrap(),
            c["enabled"].as_u64().unwrap() != 0,
        );
        let events = row["original"]["events"].as_array().unwrap();
        if let Some(rect) = rect {
            assert_eq!(
                json!(["move", rect[0], rect[1], rect[2], rect[3]]),
                events[0]
            );
            assert_eq!(
                &events[1..4],
                &[json!(["show"]), json!(["raise"]), json!(["palette", 0])]
            );
            assert_eq!(
                events.len(),
                if c["old_color"] == c["color"] { 4 } else { 5 }
            );
            if events.len() == 5 {
                assert_eq!(events[4], json!(["repaint"]));
            }
            assert_eq!(row["original"]["color"], c["color"]);
        } else {
            assert_eq!(events, &[json!(["hide"])]);
            assert_eq!(row["original"]["color"], c["old_color"]);
        }
    }
}
