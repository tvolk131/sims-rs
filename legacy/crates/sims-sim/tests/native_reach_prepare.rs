use serde_json::{Value, json};
use sims_sim::{lot_links::Links, lot_reach};
use sims_vm::{Object, World, autonomy::Placement};
use std::collections::BTreeMap;

#[cfg(feature = "private-fixtures")]
#[test]
fn original_reach_preparation_uses_physical_ancestry_runtime_heights_and_native_rounding() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-reach-prepare.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut world = World::default();
        let mut heights = BTreeMap::new();
        for o in c["objects"].as_array().unwrap() {
            let id = o["id"].as_i64().unwrap() as i16;
            let slots = o["slots"].as_array().unwrap();
            world.objects.insert(
                id,
                Object {
                    data: [
                        (14, o["physical_slot"].as_i64().unwrap() as i16),
                        (26, o["predecessor"].as_i64().unwrap() as i16),
                    ]
                    .into(),
                    slots: slots
                        .iter()
                        .map(|s| match s["object"].as_i64().unwrap() as i16 {
                            0 => None,
                            id => Some(id),
                        })
                        .collect(),
                    ..Default::default()
                },
            );
            world.positions.insert(
                id,
                Placement {
                    x: 40,
                    y: 40,
                    level: 1,
                },
            );
            for (i, s) in slots.iter().enumerate() {
                heights.insert((id, i), s["z"].as_f64().unwrap() as f32);
            }
        }
        let links = Links {
            size: 8,
            heads: vec![vec![0; 64]],
            next: world.objects.keys().map(|&id| (id, 0)).collect(),
        };
        let args: Vec<i16> = serde_json::from_value(c["args"].clone()).unwrap();
        let mut operands = [0; 8];
        operands[..2].copy_from_slice(&(c["mode"].as_i64().unwrap() as i16).to_le_bytes());
        operands[2] = 1;
        operands[4..6].copy_from_slice(&(c["index"].as_i64().unwrap() as i16).to_le_bytes());
        let result = lot_reach::prepare(
            &mut world,
            &links,
            1,
            c["target"].as_i64().unwrap() as i16,
            &args,
            &operands,
            |_, id, slot| Ok(heights[&(id, slot)]),
        );
        let physical: BTreeMap<_, _> = world
            .objects
            .iter()
            .map(|(&id, o)| (id, o.data[&14]))
            .collect();
        let actual = json!({"error":result.as_ref().err().map(|e| {
            assert!(e.contains("native error 0x08"), "case {case}: {e}"); 8
        }), "success":result.as_ref().ok().map(Option::is_some),
        "prepared":result.as_ref().ok().and_then(|p| *p), "physical_slots":physical});
        assert_eq!(actual, row["original"], "case {case}: {c}");
        let saved = serde_json::to_vec(&world).unwrap();
        assert_eq!(world, serde_json::from_slice::<World>(&saved).unwrap());
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 279);
}
