use serde_json::Value;
use sims_vm::{Definition, Module, Program, Vm, World};

#[cfg(feature = "private-fixtures")]
#[test]
fn absent_local_slot_chunk_keeps_the_constructed_empty_slots() {
    let report: Value = serde_json::from_str(include_str!("fixtures/missing-slots.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        assert_eq!(row["original"]["returned"], 1);
        assert_eq!(row["original"]["unchanged"], true);
        let slot = row["input"]["slot"].as_u64().unwrap() as u16;
        let mut words = vec![0; 108];
        words[9] = 4;
        words[14] = 123;
        words[20] = slot as i16;
        words[102] = 1;
        let mut module = Module {
            source: "local".into(),
            ..Default::default()
        };
        module.object_definition_words.insert(128, words);
        let program = Program {
            modules: [("local".into(), module)].into(),
            ..Default::default()
        };
        let mut vm = Vm::new(
            World {
                person: Some(Default::default()),
                refresh: Some(Default::default()),
                ..Default::default()
            },
            1,
        );
        vm.begin_object_factory_graphics(
            &program,
            1,
            Definition {
                source: "local".into(),
                object: 128,
                guid: 123,
                slots: slot,
            },
        )
        .unwrap();
        assert!(vm.world.objects[&1].slots.is_empty());
        assert_eq!(vm.world.person.as_ref().unwrap().object_order, [1]);
        assert!(vm.object_threads.is_empty() && vm.lifecycles.is_empty());
    }
}
