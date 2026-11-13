use serde_json::{Value, json};
use sims_vm::{
    World,
    family::{self, Change, Host, Stage},
};
use std::collections::BTreeMap;

fn world(c: &Value) -> World {
    use sims_vm::{
        Definition, Object,
        person::{Neighbor, WorldState},
    };
    let mut people = WorldState::default();
    let mut slots = vec![];
    let mut guids = BTreeMap::new();
    for (i, n) in c["neighbors"].as_array().unwrap().iter().enumerate() {
        if n.is_null() {
            slots.push(None);
            continue;
        }
        let id = (i + 1) as i16;
        let guid = n["guid"].as_u64().unwrap() as u32;
        guids.entry(guid).or_insert(id);
        slots.push(Some(id));
        people.neighbors.insert(
            id,
            Neighbor {
                guid,
                person: [(61, n["family"].as_i64().unwrap() as i16)].into(),
                ..Default::default()
            },
        );
    }
    people.neighbor_slots = Some(slots);
    let mut order = vec![];
    for f in c["families"].as_array().unwrap() {
        let id = f["id"].as_i64().unwrap() as i16;
        order.push(id);
        people.families.insert(
            id,
            f["members"]
                .as_array()
                .unwrap()
                .iter()
                .map(|g| guids[&(g.as_u64().unwrap() as u32)])
                .collect(),
        );
        people.family_metadata.insert(
            id,
            family::Metadata {
                flags: f["flags"].as_u64().unwrap() as u32,
                dirty: f["dirty"].as_bool().unwrap(),
                friend_count: None,
                member_guids: Some(serde_json::from_value(f["members"].clone()).unwrap()),
            },
        );
    }
    people.controlled_family = order[c["selected"].as_u64().unwrap() as usize];
    people.family_order = Some(order);
    people.person_order = c["order"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as i16 + 1)
        .collect();
    let mut world = World::default();
    world.globals.insert(10, 9);
    people
        .lot_zoning
        .insert(9, if c["residential"] == true { 1 } else { 2 });
    world.person = Some(people);
    for (i, p) in c["people"].as_array().unwrap().iter().enumerate() {
        world.objects.insert(
            (i + 1) as i16,
            Object {
                definition: Some(Definition {
                    source: "family".into(),
                    object: (i + 1) as u16,
                    guid: p["guid"].as_u64().unwrap() as u32,
                    slots: 0,
                }),
                person: Some(
                    [
                        (61, p["family"].as_i64().unwrap() as i16),
                        (32, p["visitor"].as_i64().unwrap() as i16),
                    ]
                    .into(),
                ),
                ..Default::default()
            },
        );
    }
    world
}

fn state(w: &World) -> Value {
    let p = w.person.as_ref().unwrap();
    json!({
        "families": p.family_order.as_ref().unwrap().iter().map(|id| json!({
            "id":id,"members":p.families[id].iter().map(|n| p.neighbors[n].guid).collect::<Vec<_>>(),
            "flags":p.family_metadata[id].flags,"dirty":p.family_metadata[id].dirty
        })).collect::<Vec<_>>(),
        "neighbors":p.neighbor_slots.as_ref().unwrap().iter().map(|n| n.map(|n|p.neighbors[&n].person[&61])).collect::<Vec<_>>(),
        "people":w.objects.values().map(|o|json!({"family":o.person.as_ref().unwrap()[&61],
            "visitor":o.person.as_ref().unwrap()[&32]})).collect::<Vec<_>>()
    })
}

struct Services<'a> {
    input: &'a Value,
    events: Vec<Value>,
}
impl Host for Services<'_> {
    fn close_panel(&mut self, _world: &mut World) -> Result<(), String> {
        self.events.push(json!(["provider"]));
        if self.input["panel"] == true {
            self.events.push(json!(["panel_valid"]));
            if self.input["panel_active"] == true {
                self.events.push(json!(["panel_event", [3, 16, 27, 0]]));
            }
        }
        Ok(())
    }
    fn residential(&mut self, _: &World) -> Result<bool, String> {
        self.events.push(json!(["residential"]));
        Ok(self.input["residential"].as_bool().unwrap())
    }
    fn observe(&mut self, stage: Stage, w: &World) {
        let name = match stage {
            Stage::Member => "member",
            Stage::Live => "live",
            Stage::Erase => "erase",
            Stage::Visitors => "visitors",
        };
        self.events.push(json!([name, state(w)]));
    }
    fn broadcast(&mut self, w: &mut World, change: Change) -> Result<(), String> {
        self.events.push(json!(["notify", 232, 0, state(w)]));
        w.person.as_mut().unwrap().family_changes.push(change);
        Ok(())
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_family_removal_preserves_guid_order_and_each_helper_boundary() {
    let r: Value = serde_json::from_str(include_str!("fixtures/family-removal.json")).unwrap();
    assert_eq!(r["cases"].as_array().unwrap().len(), 256);
    for (i, row) in r["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut w = world(c);
        let mut host = Services {
            input: c,
            events: vec![],
        };
        for (target, expected) in c["targets"]
            .as_array()
            .unwrap()
            .iter()
            .zip(row["original"].as_array().unwrap())
        {
            let result = family::remove(&mut w, target.as_i64().unwrap() as i16, &mut host);
            let (error, returned) = match result {
                Ok(result) => (None, Some(u8::from(result))),
                Err(e) => {
                    assert!(e.contains("native error 0x37"), "case {i}: {e}");
                    (Some(55), None)
                }
            };
            assert_eq!(
                json!({"error":error,"returned":returned,"state":state(&w),"events":host.events}),
                *expected,
                "case {i}"
            );
            assert!(
                w.person
                    .as_ref()
                    .unwrap()
                    .neighbors
                    .values()
                    .all(|n| !n.dirty),
                "native direct family word writes do not dirty neighbors"
            );
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn loaded_family_metadata_matches_original_constructor_and_fami_reader() {
    let r: Value =
        serde_json::from_str(include_str!("fixtures/family-loaded-metadata.json")).unwrap();
    assert_eq!(r["cases"].as_array().unwrap().len(), 45);
    for row in r["cases"].as_array().unwrap() {
        let c = &row["input"];
        let original = &row["original"];
        let mut bytes = vec![0; 4];
        bytes.extend((c["version"].as_u64().unwrap() as u32).to_le_bytes());
        bytes.extend(b"IMAF");
        bytes.extend(
            c["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| b.as_u64().unwrap() as u8),
        );
        let f = sims_formats::neighborhood::Family::parse(&bytes).unwrap();
        let mut p = sims_vm::person::WorldState::default();
        p.neighbor_slots = Some(vec![Some(1), Some(2)]);
        for (id, guid) in [(1, 11), (2, 22)] {
            p.neighbors.insert(
                id,
                sims_vm::person::Neighbor {
                    guid,
                    ..Default::default()
                },
            );
        }
        p.prepare_family_reload(&[sims_vm::neighborhood::Family {
            id: 7,
            number: f.number,
            house: f.house as i16,
            flags: f.flags,
            friends: f.friends,
            members: f.members.clone(),
        }])
        .unwrap();
        assert_eq!(
            json!({"flags":p.family_metadata[&0].flags,"dirty":p.family_metadata[&0].dirty}),
            original["constructed"]
        );
        assert_eq!(p.family_metadata[&7].flags, original["flags"]);
        assert_eq!(p.family_metadata[&7].dirty, original["dirty"]);
        assert_eq!(p.family_metadata[&7].friend_count, Some(f.friends));
        assert_eq!(p.family_metadata[&7].member_guids, Some(f.members.clone()));
        assert_eq!(p.family_metadata[&0].friend_count, Some(0));
        assert_eq!(p.family_metadata[&0].member_guids, Some(vec![]));
        assert_eq!(json!(f.members), original["members"]);
        assert_eq!(original["consumed"], c["data"].as_array().unwrap().len());
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn generic_family_call_uses_neighbor_identity_and_restores_complete_vm_state() {
    use sims_formats::behavior::{Behavior, Instruction};
    use sims_vm::*;
    let report: Value = serde_json::from_str(include_str!("fixtures/family-removal.json")).unwrap();
    let c = report["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["input"])
        .find(|c| c["targets"] == json!([3, 3]) && c["residential"] == true)
        .unwrap();
    let mut module = Module {
        source: "family".into(),
        ..Default::default()
    };
    module.routines.insert(
        4096,
        Routine {
            name: "remove family".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args: 0,
                locals: 0,
                instructions: vec![Instruction {
                    opcode: 1,
                    on_true: 254,
                    on_false: 255,
                    operands: [6, 0, 0, 0, 0, 0, 0, 0],
                }],
            },
        },
    );
    let p = Program {
        modules: [("family".into(), module)].into(),
        ..Default::default()
    };
    let mut vm = Vm::new(world(c), 1);
    for expected in [true, false] {
        let id = vm
            .spawn(
                &p,
                Start {
                    owner: "family",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: 3,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        let mut restored = Snapshot::capture(&p, &vm).restore(&p).unwrap();
        assert_eq!(
            vm.run_slice(&p, id, 10).unwrap(),
            restored.run_slice(&p, id, 10).unwrap()
        );
        assert_eq!(vm, restored);
        assert_eq!(vm.threads[&id].status, Status::Returned(expected));
        assert_eq!(
            vm,
            serde_json::from_slice::<Snapshot>(
                &serde_json::to_vec(&Snapshot::capture(&p, &vm)).unwrap()
            )
            .unwrap()
            .restore(&p)
            .unwrap()
        );
    }
    let people = vm.world.person.as_ref().unwrap();
    assert_eq!(people.neighbors[&3].person[&61], 0);
    assert_eq!(vm.world.objects[&2].person.as_ref().unwrap()[&61], 0);
    assert_eq!(
        vm.world.objects[&3].person.as_ref().unwrap()[&61],
        7,
        "physical object3 is not neighbor3"
    );
    assert_eq!(
        people.families[&9].len(),
        2,
        "only the first duplicate is erased"
    );
    assert_eq!(
        people.family_changes,
        vec![family::Change {
            family: 9,
            neighbor: 3,
            guid: 22
        }]
    );
}
