//! Portable numeric results from the composed original factory; no game bytes.
use serde_json::{Value, json};
use sims_vm::{
    person_factory::{
        PostConstructRequest as Post,
        registered::{Factory, Request},
    },
    *,
};
use std::collections::BTreeMap;

#[cfg(feature = "private-fixtures")]
#[test]
fn original_registration_order_and_initial_scores_survive_snapshots() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/person-factory-registration.json")).unwrap();
    let guid = 0x44a86a6c;
    let mut words = vec![0; 108];
    words[9] = 2;
    words[14] = guid as u16 as i16;
    words[15] = (guid >> 16) as i16;
    let p = Program {
        globals: "person".into(),
        modules: BTreeMap::from([(
            "person".into(),
            Module {
                source: "person".into(),
                object_definition_words: BTreeMap::from([(128, words)]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let definition = Definition {
        source: "person".into(),
        object: 128,
        guid,
        slots: 0,
    };
    assert_eq!(report["cases"].as_array().unwrap().len(), 6);
    for row in report["cases"].as_array().unwrap() {
        let mut people = person::WorldState {
            object_order: vec![2],
            ..Default::default()
        };
        if row["input"]["cached"] == true {
            people.resource_instances.insert(guid, vec![]);
        }
        let mut vm = Vm::new(
            World {
                objects: BTreeMap::from([(2, Object::default())]),
                person: Some(people),
                actions: Some(Default::default()),
                refresh: Some(Default::default()),
                ..Default::default()
            },
            1,
        );
        let mut f = Factory::begin(
            &mut vm,
            &p,
            definition.clone(),
            construction::ObjectDate {
                year: 1997,
                month: 1,
                day: 1,
            },
            [12, 34],
        )
        .unwrap()
        .unwrap();
        assert_eq!(f.object(), 1);
        let r = &row["registry"];
        let registry = vm.world.person.as_ref().unwrap();
        assert_eq!(json!(registry.object_order), r["order"]);
        assert_eq!(json!(registry.person_order), r["people"]);
        assert_eq!(
            json!(
                registry
                    .resource_instances
                    .get(&guid)
                    .cloned()
                    .unwrap_or_default()
            ),
            r["instances"]
        );
        assert_eq!(
            json!(person_loading::initial_score_curves()),
            row["initial_scores"]
        );
        assert!(
            vm.world.objects[&1].data.values().all(|v| *v == 0),
            "registration must not prematurely stamp/reset the object"
        );
        assert!(
            vm.crt_rng_state.is_none(),
            "registration must not invent a CRT state"
        );
        assert_eq!(f.next(&mut vm, &p).unwrap(), Some(Request::Registered(1)));
        let before = f.clone();
        assert!(
            f.complete(&mut vm, &p, Request::Registered(2), None)
                .is_err()
        );
        assert_eq!(f, before);
        f.complete(&mut vm, &p, Request::Registered(1), None)
            .unwrap();
        let request = f.next(&mut vm, &p).unwrap().unwrap();
        assert_eq!(request, Request::Post(Post::Base));
        let bytes = serde_json::to_vec(&(Snapshot::capture(&p, &vm), &f)).unwrap();
        let (snapshot, mut restored): (Snapshot, Factory) = serde_json::from_slice(&bytes).unwrap();
        vm = snapshot.restore(&p).unwrap();
        restored.validate(&vm, &p).unwrap();
        assert_eq!(restored.next(&mut vm, &p).unwrap(), Some(request));
        vm.prepare_person_base_postconstruction(&p, 1).unwrap();
        restored.complete(&mut vm, &p, request, None).unwrap();
        assert_eq!(
            restored.next(&mut vm, &p).unwrap(),
            Some(Request::Post(Post::BindAnimator))
        );
        assert_eq!(
            vm.world.objects[&1].data[&45], 0,
            "date comes after post-construction"
        );
        assert_eq!(vm.world.objects[&1].slots.len(), 5);
        assert_eq!(
            vm.world.positions[&1],
            autonomy::Placement {
                x: -16,
                y: -16,
                level: 1
            }
        );
        assert!(!restored.is_complete() && !restored.activated());
    }
}
