//! Native wrapper decisions on synthetic placements; full placement acceptance
//! is a deliberately supplied boundary in these original-code vectors.
use serde_json::{Value, json};
use sims_vm::{Object, World};
#[cfg(feature = "private-fixtures")]
#[test]
fn original_drop_onto_operand_slots_errors_and_placement_order() {
    let report: Value = serde_json::from_str(include_str!("fixtures/domestic-drop.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let words: Vec<i16> = serde_json::from_value(c["operands"].clone()).unwrap();
        let operands: [u8; 8] = words
            .iter()
            .flat_map(|n| n.to_le_bytes())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let args: Vec<i16> = serde_json::from_value(c["args"].clone()).unwrap();
        let slots: Vec<i16> = serde_json::from_value(c["slots"].clone()).unwrap();
        let target = c["target"].as_i64().unwrap() as i16;
        let mut w = World::default();
        w.objects.insert(
            1,
            Object {
                slots: slots.iter().map(|&n| (n != 0).then_some(n)).collect(),
                ..Default::default()
            },
        );
        w.objects.insert(2, Object::default());
        let mut error = None;
        let mut calls = vec![];
        match sims_vm::transfer::onto_slots(&operands, &args) {
            Err(_) => error = Some(8),
            Ok([source, destination]) => {
                match sims_sim::lot_transfer::destination(
                    &w,
                    &request(target, false, Some([source, destination])),
                ) {
                    Err(e) => error = Some(if e.contains("0x1b") { 27 } else { 21 }),
                    Ok(Some((_, to))) => {
                        calls.push(json!([
                            "check",
                            to.position,
                            to.level,
                            to.container.unwrap_or(0),
                            to.slot as u32
                        ]));
                        if c["success"] == true {
                            calls.push(json!([
                                "place",
                                to.position,
                                to.level,
                                to.container.unwrap_or(0),
                                to.slot as u32
                            ]));
                        }
                    }
                    Ok(None) => {}
                }
            }
        }
        assert_eq!(json!(error), n["error"], "{c}");
        assert_eq!(json!(calls), n["calls"], "{c}");
        if error.is_none() {
            assert_eq!(
                json!(u8::from(!calls.is_empty() && c["success"] == true)),
                n["returned"],
                "{c}"
            );
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_grab_drop_select_slot_zero_and_preserve_native_call_order() {
    let report: Value = serde_json::from_str(include_str!("fixtures/grab-drop.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let grab = c["grab"].as_bool().unwrap();
        let carried = c["carried"].as_i64().unwrap() as i16;
        let mut w = World::default();
        w.objects.insert(2, Object::default());
        w.objects.insert(
            1,
            Object {
                slots: if c["has_slot"] == true {
                    vec![(carried != 0).then_some(carried)]
                } else {
                    vec![]
                },
                ..Default::default()
            },
        );
        w.objects
            .get_mut(&1)
            .unwrap()
            .data
            .insert(1, c["direction"].as_i64().unwrap() as i16);
        w.positions.insert(
            1,
            sims_vm::autonomy::Placement {
                x: 32,
                y: 48,
                level: 2,
            },
        );
        let target = sims_sim::lot_transfer::destination(
            &w,
            &request(c["target"].as_i64().unwrap() as i16, grab, None),
        )
        .unwrap();
        let mut calls = vec![];
        if let Some((_, to)) = target {
            calls.push(json!([
                "check",
                to.position,
                to.level,
                to.container.unwrap_or(0),
                to.slot
            ]));
            if c["success"] == true {
                calls.push(json!([
                    "place",
                    to.position,
                    to.level,
                    to.container.unwrap_or(0),
                    to.slot
                ]));
            }
        }
        assert_eq!(json!(calls), n["calls"], "{c}");
        assert_eq!(
            json!(u8::from(target.is_some() && c["success"] == true)),
            n["returned"],
            "{c}"
        );
    }
}

fn request(target: i16, grab: bool, onto: Option<[i16; 2]>) -> sims_vm::transfer::Request {
    sims_vm::transfer::Request {
        id: 1,
        origin: sims_vm::Location {
            source: "probe".into(),
            bhav: 4096,
            instruction: 0,
        },
        caller: 1,
        target,
        grab,
        onto,
    }
}
