use serde_json::{Value, json};
use sims_sim::lot_snap::{Plan, adjacency, contained_wall_gate, plan};
use sims_vm::{Location, RouteRequest, SnapRequest, routing::Destination};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_contained_snap_offsets_and_placement_arguments() {
    let report: Value = serde_json::from_str(include_str!("fixtures/contained-snap.json")).unwrap();
    for row in report["plans"].as_array().unwrap() {
        let c = &row["input"];
        let destination = if c["mode"] == 1 {
            Destination::FirstSlot
        } else {
            Destination::Slot {
                source: "test".into(),
                chunk: 128,
                index: 0,
                version: 10,
                slot: sims_formats::execution::Slot {
                    kind: 3,
                    offset: [0.; 3],
                    standing: 0,
                    sitting: 0,
                    ground: 0,
                    flags: c["slot_flags"].as_u64().unwrap() as u32,
                    snap_target: 0,
                    proximity: [0; 3],
                    max_size: 0,
                    unknown: 0,
                    gradient: 0.,
                    height: 0,
                    facing: 0,
                    resolution: 16,
                },
            }
        };
        let r = SnapRequest {
            route: RouteRequest {
                id: 1,
                origin: Location {
                    source: "test".into(),
                    bhav: 4096,
                    instruction: 0,
                },
                caller: 1,
                target: 2,
                no_failure_trees: false,
                destination,
            },
            flags: c["flags"].as_u64().unwrap() as u8,
        };
        let p = plan(
            &r,
            [80, 96, 2],
            c["rotation"].as_u64().unwrap() as u8,
            3,
            Some(serde_json::from_value(c["slot_offset"].clone()).unwrap()),
        )
        .unwrap();
        assert_eq!(
            json!([
                p.position[0],
                p.position[1],
                p.level,
                p.container.unwrap().0,
                p.container.unwrap().1,
                u8::from(p.bypass_adjacency),
                p.direction,
                (r.flags >> 2) & 1
            ]),
            row["original"]["placement"],
            "{c}"
        );
        if c["success"] == 1 {
            assert_eq!(
                json!(p.offsets.map(f32::from_bits)),
                row["original"]["offset"],
                "{c}"
            );
        } else {
            assert!(row["original"]["offset"].is_null());
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_contained_snap_wall_gate_precedes_commit_and_preserves_facing() {
    use sims_sim::lot_route::candidate::{Reply, Request};
    let report: Value = serde_json::from_str(include_str!("fixtures/contained-snap.json")).unwrap();
    for row in report["wrappers"].as_array().unwrap() {
        let c = &row["input"];
        let p = Plan {
            position: [72, 56],
            level: 1,
            direction: c["facing"].as_u64().unwrap() as u8,
            bypass_adjacency: c["bypass"].as_bool().unwrap(),
            container: Some((2, 2)),
            offsets: [0; 3],
        };
        let mut events = vec![];
        let mut error = 91;
        let passed = adjacency(8, [56, 56, 1], 7, &p, &mut error, |q| {
            Ok(match q {
                Request::Room(_) => Reply::Room(7),
                Request::Walls { tile, rotation } => {
                    events.push(json!({"wall":[tile,rotation]}));
                    Reply::Walls(0)
                }
                _ => panic!("unexpected query"),
            })
        })
        .unwrap();
        let mut facing = 3;
        let mut facing_at_commit = None;
        let result = if passed {
            events.push(json!({"accept":[p.position,p.level,2,2]}));
            c["accepted"].as_bool().unwrap()
                && contained_wall_gate(
                    &p,
                    if c["wall_check"] == true { 4 } else { 0 },
                    |parent, direction| {
                        assert_eq!(parent, 2);
                        events.push(json!({"footprint":[p.position,direction]}));
                        events.push(json!({"wall_crossing":[[75,59,69,53],1]}));
                        Ok(c["wall_blocked"].as_bool().unwrap())
                    },
                )
                .unwrap()
        } else {
            false
        };
        if result {
            events.push(json!({"commit":[p.position,p.level,2,2,1]}));
            facing_at_commit = Some(facing);
            facing = p.direction;
            events.push(json!({"animator_reset":1}));
        }
        assert_eq!(
            json!({"returned":u8::from(result),"events":events,"error":error,
            "facing":facing,"facing_at_commit":facing_at_commit}),
            row["original"],
            "{c}"
        );
    }
}
