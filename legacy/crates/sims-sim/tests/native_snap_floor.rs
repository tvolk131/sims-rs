use sims_formats::execution::Slot;
use sims_sim::lot_snap::floor_plan;
use sims_vm::{Location, RouteRequest, SnapRequest, routing::Destination};
#[cfg(feature = "private-fixtures")]
#[test]
fn native_snap_floor_slot_transform_and_placement_arguments() {
    let r: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/snap-routing-slot.json")).unwrap();
    for (i, row) in r["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let offset: [f32; 2] = serde_json::from_value(c["routing_offset"].clone()).unwrap();
        let slot = Slot {
            kind: 3,
            offset: [offset[0], offset[1], 0.],
            standing: 0,
            sitting: 0,
            ground: 0,
            flags: c["slot_flags"].as_u64().unwrap() as u32,
            snap_target: -1,
            proximity: [0; 3],
            max_size: 0,
            unknown: 0,
            gradient: 0.,
            height: 0,
            facing: 0,
            resolution: 0,
        };
        let request = SnapRequest {
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
                destination: Destination::Slot {
                    source: "test".into(),
                    chunk: 128,
                    index: 0,
                    version: 10,
                    slot,
                },
            },
            flags: 2,
        };
        let p = floor_plan(
            &request,
            [80, 96, 2],
            c["rotation"].as_u64().unwrap() as u8,
            3,
        )
        .unwrap();
        assert_eq!(
            serde_json::json!([
                p.position[0],
                p.position[1],
                p.level,
                0,
                0,
                u8::from(p.bypass_adjacency),
                p.direction,
                0
            ]),
            row["original"]["placement"],
            "case {i}: {c}"
        );
    }
}

fn compare_placement(report: serde_json::Value) {
    use serde_json::json;
    use sims_sim::{
        lot_route::candidate::{Reply, Request},
        lot_snap::{Plan, adjacency},
    };
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let old = serde_json::from_value(c["actor"].clone()).unwrap();
        let to: [i32; 3] = serde_json::from_value(c["destination"].clone()).unwrap();
        let plan = Plan {
            position: [to[0], to[1]],
            level: to[2],
            direction: 6,
            bypass_adjacency: c["bypass"].as_bool().unwrap(),
            container: None,
            offsets: [0; 3],
        };
        let mut events = vec![];
        let mut error = 91;
        let mut wall = 0;
        let accept = adjacency(
            8,
            old,
            c["room"].as_i64().unwrap() as i16,
            &plan,
            &mut error,
            |r| {
                Ok(match r {
                    Request::Room(_) => Reply::Room(c["destination_room"].as_i64().unwrap() as i16),
                    Request::Walls { tile, rotation } => {
                        events.push(json!({"wall":[tile,rotation]}));
                        let mask = c["walls"][wall].as_u64().unwrap() as u8;
                        wall += 1;
                        Reply::Walls(mask)
                    }
                    _ => panic!("unexpected Snap query"),
                })
            },
        )
        .unwrap();
        let mut facing = 3;
        let mut facing_at_commit = None;
        let result = if accept {
            events.push(json!({"accept":[plan.position,plan.level,0,0]}));
            if c["accepted"].as_bool().unwrap() {
                events.push(json!({"commit":[plan.position,plan.level,0,0,1]}));
                facing_at_commit = Some(facing);
                facing = 6;
                if c["person"].as_bool().unwrap() {
                    events.push(json!({"animator_reset":1}));
                }
                1
            } else {
                0
            }
        } else {
            0
        };
        assert_eq!(
            json!({"returned":result,"events":events,"error":error,"facing":facing,"facing_at_commit":facing_at_commit}),
            row["original"],
            "case {i}: {c}"
        );
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_snap_placement_adjacency_and_order() {
    compare_placement(serde_json::from_str(include_str!("fixtures/snap-placement.json")).unwrap());
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_multipart_snap_virtual_dispatch_and_post_commit_facing() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/snap-group.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 192);
    compare_placement(report);
}
#[test]
#[ignore = "requires private native Snap placement wrapper corpus"]
fn all_original_snap_placement_wrappers() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/snap-placement.json");
    compare_placement(serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap());
}
