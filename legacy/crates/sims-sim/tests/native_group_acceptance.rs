use serde_json::{Value, json};
use sims_sim::{
    lot_acceptance::{Outcome, group::Check},
    lot_links::Links,
};
use sims_vm::{Object, World, multitile::Part};
#[cfg(feature = "private-fixtures")]
#[test]
fn group_checks_preserve_original_order_errors_heights_and_live_offsets() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/group-acceptance.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 162);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let root = c["root"].as_i64().unwrap() as i16;
        let mut w = World::default();
        for id in 1..=3 {
            w.objects.insert(
                id,
                Object {
                    multitile: Some(Part {
                        head: (id != 1).then_some(1),
                        next: (id < 3).then_some(id + 1),
                        offset: match id {
                            1 => [0, 0, 0],
                            2 => [0, 1, 0],
                            _ => [1, 0, 0],
                        },
                        origin: [0; 3],
                    }),
                    data: [(42, c["flags"].as_i64().unwrap() as i16)].into(),
                    ..Default::default()
                },
            );
        }
        let links = Links {
            size: 8,
            heads: vec![vec![0; 64]; 2],
            next: [(1, 0), (2, 0), (3, 0)].into(),
        };
        let heights: Vec<_> = (0..64)
            .map(|i| {
                [
                    (c["slope"].as_u64().unwrap() as usize * (i / 8 + i % 8)) as u8,
                    0xcc,
                    0xbb,
                    0xaa,
                ]
            })
            .collect();
        let mut check = Check::begin(
            &w,
            &links,
            root,
            serde_json::from_value(c["position"].clone()).unwrap(),
            c["level"].as_i64().unwrap() as i32,
            c["contained"].as_bool().unwrap(),
            7,
            91,
        )
        .unwrap();
        let mut events = vec![];
        while let Some(q) = check.next(&w).unwrap() {
            check = serde_json::from_value(json!(&check)).unwrap();
            check.validate(&w).unwrap();
            let i = events.len();
            events.push(
                json!({"object":q.object,"position":q.position,"level":q.level,"slot":q.slot}),
            );
            let result = Outcome {
                accepted: c["replies"][i][0].as_bool().unwrap(),
                error: c["replies"][i][1].as_i64().unwrap() as i32,
            };
            if c["mutate"] == true && i == 0 {
                w.objects
                    .get_mut(&q.object)
                    .unwrap()
                    .multitile
                    .as_mut()
                    .unwrap()
                    .next = Some(3);
                w.objects
                    .get_mut(&3)
                    .unwrap()
                    .multitile
                    .as_mut()
                    .unwrap()
                    .offset = [1, 2, 1];
                w.objects.get_mut(&root).unwrap().data.insert(42, 0x100);
            }
            check.resume(&w, 8, &heights, q, result).unwrap();
        }
        let out = check.outcome().unwrap();
        assert_eq!(
            json!({"accepted":out.accepted,"error":out.error,"events":events}),
            row["original"],
            "{c}"
        );
    }
}
