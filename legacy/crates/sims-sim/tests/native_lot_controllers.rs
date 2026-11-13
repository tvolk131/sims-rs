use serde_json::Value;
use sims_sim::{clock::LotClock, lot_controllers};
use sims_vm::{Definition, Object, Vm, World, autonomy::Placement, multitile::Part};

#[cfg(feature = "private-fixtures")]
#[test]
fn controller_queries_match_original_instructions_including_order_and_wrapping() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-lot-controllers.json")).unwrap();
    for (n, case) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &case["input"];
        let expected = &case["original"];
        let helper = Definition {
            source: "helper".into(),
            object: 1,
            guid: 17,
            slots: 0,
        };
        let mut world = World::default();
        let group: Vec<i16> = serde_json::from_value(c["group"].clone()).unwrap();
        for source in c["objects"].as_array().unwrap() {
            let id = source["id"].as_i64().unwrap() as i16;
            let mut object = Object {
                definition: Some(Definition {
                    source: if source["matches"].as_bool().unwrap() {
                        "helper"
                    } else {
                        "other"
                    }
                    .into(),
                    ..helper.clone()
                }),
                relationship_order: Some(vec![]),
                ..Default::default()
            };
            if let Some(index) = group.iter().position(|v| *v == id) {
                object.multitile = Some(Part {
                    head: (index != 0).then_some(group[0]),
                    next: group.get(index + 1).copied(),
                    origin: [0; 3],
                    offset: [0; 3],
                });
            }
            world.objects.insert(id, object);
            world.positions.insert(
                id,
                Placement {
                    x: source["position"][0].as_i64().unwrap() as i32,
                    y: source["position"][1].as_i64().unwrap() as i32,
                    level: 1,
                },
            );
        }
        let owner = c["owner"].as_i64().unwrap() as i16;
        for row in c["relationships"].as_array().unwrap() {
            world.objects.get_mut(&owner).unwrap().insert_relationship(
                row[0].as_i64().unwrap() as i16,
                serde_json::from_value(row[1].clone()).unwrap(),
            );
        }
        let born: Vec<i16> = serde_json::from_value(c["born"].clone()).unwrap();
        world
            .objects
            .get_mut(&owner)
            .unwrap()
            .data
            .extend([45, 46, 47, 24, 21].into_iter().zip(born));
        let now: Vec<i16> = serde_json::from_value(c["now"].clone()).unwrap();
        let clock = LotClock {
            year: now[0],
            month: now[1],
            day: now[2],
            hour: now[3],
            minute: now[4],
            second: 0,
            period: 0,
            elapsed_days: 0,
            month_days: 30,
            ticks: 0,
            statistics: [[0; 8]; 6],
        };
        let vm = Vm::new(world, 1);
        assert_eq!(
            serde_json::to_value(lot_controllers::existing(&vm, owner, &helper).unwrap()).unwrap(),
            expected["existing"],
            "existing case {n}"
        );
        assert_eq!(
            serde_json::to_value(lot_controllers::center(&vm, owner).unwrap()).unwrap(),
            expected["center"],
            "center case {n}"
        );
        assert_eq!(
            i64::from(lot_controllers::elapsed_minutes(&clock, &vm.world.objects[&owner]).unwrap()),
            expected["elapsed_minutes"].as_i64().unwrap(),
            "date case {n}"
        );
        let lot = c["lot"].as_i64().unwrap() as i16;
        let zone = c["zone"].as_i64().unwrap() as i32;
        assert_eq!(
            lot_controllers::excluded_lot(lot, zone),
            expected["excluded"].as_bool().unwrap(),
            "lot case {n}"
        );
        assert_eq!(
            sims_vm::zoning::is_residential(&[(lot, zone)].into(), lot),
            expected["residential"].as_bool().unwrap(),
            "zoning case {n}"
        );
    }
}
