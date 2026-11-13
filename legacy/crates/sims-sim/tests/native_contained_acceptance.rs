use serde_json::{Value, json};
use sims_sim::{
    lot_acceptance::{Floor, Input, Outcome, Reply, Request, contained},
    lot_links::Links,
};
use sims_vm::{Object, World, autonomy::Placement};
fn compare(report: Value) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let n = |key: &str| c[key].as_i64().unwrap() as i16;
        let mut input = c.clone();
        let mut flags = vec![c["parent_flags"].clone()];
        flags.extend(c["other_flags"].as_array().unwrap().iter().cloned());
        input["occupants"] = json!(flags);
        let input: Input = serde_json::from_value(input).unwrap();
        let mut world = World::default();
        for id in 1..=flags.len() as i16 + 1 {
            let f = if id == 1 {
                n("flags")
            } else {
                flags[id as usize - 2].as_i64().unwrap() as i16
            };
            let mut o = Object {
                data: [
                    (4, n("height_mask")),
                    (8, f),
                    (14, -1),
                    (26, 0),
                    (27, n("weight")),
                    (28, n("support")),
                ]
                .into(),
                ..Default::default()
            };
            if id == 1 && c["person"] == true || id == 2 && c["parent_person"] == true {
                o.person = Some(Default::default());
            }
            world.objects.insert(id, o);
            world.positions.insert(
                id,
                Placement {
                    x: 56,
                    y: 56,
                    level: 1,
                },
            );
        }
        if c["cycle"] == true {
            world
                .objects
                .get_mut(&2)
                .unwrap()
                .data
                .extend([(14, 1), (26, 1)]);
        }
        let slots: Vec<contained::Slot> = serde_json::from_value(c["slots"].clone()).unwrap();
        let links = Links {
            size: 8,
            heads: vec![vec![0; 64]; 2],
            next: world.objects.keys().map(|&id| (id, 0)).collect(),
        };
        let mut state = Floor::new_contained(input, n("initial_error") as i32).unwrap();
        let mut events = vec![];
        let mut error = n("initial_error") as i32;
        while let Some(request) = state.next() {
            state = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
            assert_eq!(state.next(), Some(request));
            let reply = match request {
                Request::WallFit => {
                    events.push(request);
                    let accepted = c["wall_fit"] == true;
                    if !accepted {
                        error = n("wall_error") as i32;
                    }
                    Reply::Geometry(Outcome { accepted, error })
                }
                Request::ActiveLevel => {
                    events.push(request);
                    Reply::ActiveLevel(n("active_level") as i32)
                }
                Request::Containment => Reply::Geometry(
                    contained::check(
                        &mut world,
                        &links,
                        1,
                        2,
                        n("slot") as i32,
                        &slots,
                        n("mode"),
                        error,
                    )
                    .unwrap(),
                ),
                Request::Collision => panic!("contained query reached floor collision"),
            };
            state.resume(request, reply).unwrap();
        }
        let result = state.outcome().unwrap();
        assert_eq!(
            json!({"accepted":result.accepted,"error":result.error,"events":events}),
            row["original"],
            "case {case}: {c}"
        );
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_contained_environment_surface_capacity_and_ancestor_checks() {
    compare(serde_json::from_str(include_str!("fixtures/contained-acceptance.json")).unwrap());
}
#[test]
#[ignore = "requires private original contained-placement corpus"]
fn all_native_contained_acceptance_cases() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/contained-acceptance.json");
    compare(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}
