//! Numeric native vectors; no original assets or behavior bytecode are included.
use serde_json::{Value, json};
use sims_vm::{
    person_factory::{PostConstructRequest as R, PostConstruction},
    *,
};
use std::collections::BTreeMap;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_postconstruction_fields_order_and_resumable_services() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/person-postconstruct.json")).unwrap();
    let mut words = vec![0; 108];
    words[9] = 2;
    let p = Program {
        globals: "synthetic".into(),
        modules: BTreeMap::from([(
            "synthetic".into(),
            Module {
                source: "synthetic".into(),
                object_definition_words: BTreeMap::from([(128, words)]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    assert_eq!(report["cases"].as_array().unwrap().len(), 9);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let native = &row["original"];
        let guid = 0x44a86a6c;
        let object = Object::construct_person(
            Definition {
                source: "synthetic".into(),
                object: 128,
                guid,
                slots: 0,
            },
            &p,
            0,
        )
        .unwrap();
        let revision = c["revision"].as_i64().unwrap() as i32;
        let mut neighbor: BTreeMap<_, _> = (0..256)
            .map(|i| (i, if revision > 0 { i * 2 } else { 0 }))
            .collect();
        neighbor.insert(61, 0);
        let mut w = World::default();
        let mut constants = autonomy_constants::Owner::default();
        constants.process.loaded = Some(1); // the fixture oracle explicitly preloads this gate
        w.autonomy_constants = Some(constants);
        w.objects.insert(1, object);
        w.person = Some(person::WorldState {
            neighbors: BTreeMap::from([(
                1,
                person::Neighbor {
                    guid: if c["existing"] == true { guid } else { 123 },
                    person_revision: revision,
                    person: neighbor,
                    ..Default::default()
                },
            )]),
            neighbor_slots: Some(vec![Some(1), None, None]),
            object_order: vec![1],
            person_order: vec![1],
            ..Default::default()
        });
        let mut vm = Vm::new(w, c["seed"].as_u64().unwrap());
        vm.crt_rng_state = Some(c["crt"].as_u64().unwrap() as u32);
        let mut operation = PostConstruction::begin(&vm, &p, 1).unwrap();
        let mut requests = vec![];
        while let Some(request) = operation.next(&mut vm).unwrap() {
            let bytes = serde_json::to_vec(&(Snapshot::capture(&p, &vm), &operation)).unwrap();
            let (saved, continuation): (Snapshot, PostConstruction) =
                serde_json::from_slice(&bytes).unwrap();
            vm = saved.restore(&p).unwrap();
            operation = continuation;
            assert_eq!(operation.next(&mut vm).unwrap(), Some(request));
            if request == R::BindAnimator {
                let person = vm
                    .world
                    .objects
                    .get_mut(&1)
                    .unwrap()
                    .person
                    .as_mut()
                    .unwrap();
                for (field, value) in row["profile_fields"].as_object().unwrap() {
                    person.insert(field.parse().unwrap(), value.as_i64().unwrap() as i16);
                }
            }
            let before = operation.clone();
            let wrong = if request == R::Base {
                R::InitialScores
            } else {
                R::Base
            };
            assert!(operation.complete(wrong).is_err());
            assert_eq!(operation, before);
            operation.complete(request).unwrap();
            requests.push(request);
        }
        assert_eq!(
            requests,
            [
                R::Base,
                R::BindAnimator,
                R::InitialScores,
                R::RefreshResourceProfile
            ]
        );
        let o = &vm.world.objects[&1];
        assert_eq!(
            json!(
                o.person
                    .as_ref()
                    .unwrap()
                    .values()
                    .copied()
                    .collect::<Vec<_>>()
            ),
            native["person"]
        );
        assert_eq!(json!(vm.crt_rng_state), native["crt"]);
        assert_eq!(json!(vm.rng_state), native["global_rng"]);
        assert_eq!(json!(o.slots.len()), native["slot_count"]);
        assert_eq!(
            json!(person::BUILTIN_SLOT_FLAGS),
            json!(
                native["slots"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .skip(1)
                    .map(|slot| slot[7].as_u64().unwrap())
                    .collect::<Vec<_>>()
            ),
        );
        assert_eq!(o.data[&67] as usize, o.slots.len());
        assert_eq!(o.headline, Some(Default::default()));
        assert!(
            o.routing_slots.is_none(),
            "headline storage is not a route descriptor"
        );
        for (i, slot) in native["slots"]
            .as_array()
            .unwrap()
            .iter()
            .skip(1)
            .enumerate()
        {
            assert_eq!(slot[5], person::BUILTIN_SLOT_HEIGHTS[i]);
            let geometry = &o.native_slot_geometry.as_ref().unwrap()[i];
            assert_eq!(geometry.offset, [0; 3]);
            assert_eq!(geometry.height, slot[5].as_i64().unwrap() as i32);
            if native["slots"][i + 1][7].as_u64().unwrap() & 2 != 0 {
                assert_eq!(
                    geometry.anchor,
                    Some(slot_geometry::Anchor::Named(b"HEAD".to_vec()))
                );
                assert_ne!(
                    slot[8],
                    row["input"]["poison"].as_u64().unwrap() * 0x01010101
                );
            } else {
                // The same heap-poison word survives original45962b. Never
                // turn an untouched word into an observed null anchor.
                assert_eq!(geometry.anchor, None);
                assert_eq!(
                    slot[8],
                    row["input"]["poison"].as_u64().unwrap() * 0x01010101
                );
            }

            assert_eq!(
                slot[7].as_u64().unwrap() & 2 != 0,
                person::BUILTIN_SLOT_BONES[i].is_some()
            );
        }
        assert_eq!(native["headline_count"], 1);
        assert_eq!(native["headline"][2], 2f32.to_bits());
        assert_eq!(native["headline"][4], 0);
        assert_eq!(native["headline"][13], 0);
        assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    }
}

#[test]
fn empty_headline_snapshot_needs_no_sprite_catalog_but_active_sprites_do() {
    let p = Program::default();
    let mut w = World::default();
    w.objects.insert(
        1,
        Object {
            headline: Some(Default::default()),
            ..Default::default()
        },
    );
    let mut vm = Vm::new(w, 1);
    assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    vm.world
        .objects
        .get_mut(&1)
        .unwrap()
        .headline
        .as_mut()
        .unwrap()
        .sprite = 200;
    assert!(
        Snapshot::capture(&p, &vm)
            .restore(&p)
            .unwrap_err()
            .contains("sprite catalog")
    );
    let h = vm
        .world
        .objects
        .get_mut(&1)
        .unwrap()
        .headline
        .as_mut()
        .unwrap();
    h.sprite = 0;
    h.duration = 1;
    assert!(Snapshot::capture(&p, &vm).restore(&p).is_err());
}
