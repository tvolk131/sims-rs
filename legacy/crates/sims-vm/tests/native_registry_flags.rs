use serde_json::Value;
use sims_vm::{
    Definition, Module, Object, Program, Vm, World,
    idle::IdleState,
    program::{AutonomyAdvertisement, MenuEntry},
};

fn entries(values: &Value) -> Option<Vec<MenuEntry>> {
    values.as_array().map(|rows| {
        rows.iter()
            .map(|e| MenuEntry {
                flags: e["flags"].as_u64().unwrap() as u32,
                autonomy: AutonomyAdvertisement {
                    threshold: e["threshold"].as_u64().unwrap() as u32,
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect()
    })
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_predicate_and_masked_registry_writes() {
    let report: Value = serde_json::from_str(include_str!("fixtures/registry-flags.json")).unwrap();
    for row in report["advertisements"].as_array().unwrap() {
        let c = &row["input"];
        let table = entries(&c["table"]);
        assert_eq!(
            sims_vm::registry_flags::advertises(
                c["self_target"].as_bool().unwrap(),
                table.as_deref()
            ),
            row["original"]["advertises"].as_bool().unwrap(),
            "{c}"
        );
    }
    for row in report["full_scan"].as_array().unwrap() {
        let c = &row["input"];
        let mut zoning = std::collections::BTreeMap::new();
        for pair in c["zoning"].as_array().unwrap() {
            zoning
                .entry(pair[0].as_i64().unwrap() as i16)
                .or_insert(pair[1].as_i64().unwrap() as i32);
        }
        assert_eq!(
            sims_vm::autonomy::full_lot_scan(c["lot"].as_i64().unwrap() as i16, &zoning),
            row["original"].as_bool().unwrap(),
            "{c}"
        );
    }
    for row in report["flags"].as_array().unwrap() {
        let c = &row["input"];
        let mut words: Vec<_> = c["words"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u32)
            .collect();
        let index = c["id"].as_u64().unwrap() as usize - 1;
        let mut state = IdleState::from_registry_word(words[index]);
        assert_eq!(state.registry_word(), words[index]);
        state.set_flags(
            c["mask"].as_u64().unwrap() as u16,
            c["enabled"].as_u64().unwrap() as u8 != 0,
        );
        words[index] = state.registry_word();
        assert_eq!(serde_json::to_value(words).unwrap(), row["original"], "{c}");
    }
}
#[test]
fn reset_preserves_flags_and_tail_observes_final_callback_fields() {
    let mut p = Program::default();
    let mut m = Module::default();
    m.object_definition_words.insert(1, vec![0; 70]);
    m.object_interactions.insert(1, vec![MenuEntry::default()]);
    p.modules.insert("object".into(), m);
    let mut object = Object::construct(
        Definition {
            source: "object".into(),
            object: 1,
            guid: 123,
            slots: 0,
        },
        &p,
        0,
    )
    .unwrap();
    object.idle = Some(IdleState::from_registry_word(0xa5f70035));
    object.reset_storage(&p, 1, Default::default()).unwrap();
    assert_eq!(object.idle.as_ref().unwrap().registry_word(), 0xa5f7ffff);
    let mut world = World::default();
    world.objects.insert(1, object);
    let mut vm = Vm::new(world, 1);
    // An Init/Main callback writes field 60 before the enclosing reset returns.
    vm.world.objects.get_mut(&1).unwrap().data.insert(60, -1);
    vm.refresh_registry_flags(&p, 1).unwrap();
    assert_eq!(
        vm.world.objects[&1].idle.as_ref().unwrap().registry_word(),
        0xa5ffffff
    );
    // Caches stay unchanged between reset/load boundaries.
    vm.world.objects.get_mut(&1).unwrap().data.insert(60, 0);
    p.modules
        .get_mut("object")
        .unwrap()
        .object_interactions
        .get_mut(&1)
        .unwrap()[0]
        .autonomy
        .threshold = 100;
    assert_eq!(
        vm.world.objects[&1].idle.as_ref().unwrap().registry_word(),
        0xa5ffffff
    );
    vm.refresh_registry_flags(&p, 1).unwrap();
    assert_eq!(
        vm.world.objects[&1].idle.as_ref().unwrap().registry_word(),
        0xa5f5ffff
    );
}

#[test]
fn multipart_registration_advertises_only_the_resolved_interaction_owner() {
    let mut p = Program::default();
    let mut m = Module::default();
    let mut world = World::default();
    for id in [1, 2] {
        let mut words = vec![0; 70];
        words[8] = if id == 1 { -1 } else { 1 };
        words[10] = 1;
        m.object_definition_words.insert(id as u16, words);
        m.object_interactions
            .insert(id as u16, vec![MenuEntry::default()]);
        world.objects.insert(
            id,
            Object {
                definition: Some(Definition {
                    source: "object".into(),
                    object: id as u16,
                    guid: id as u32,
                    slots: 0,
                }),
                data: [(60, 0)].into(),
                idle: Some(IdleState::from_registry_word(0x0009000c)),
                multitile: Some(sims_vm::multitile::Part {
                    head: (id == 2).then_some(1),
                    next: (id == 1).then_some(2),
                    origin: [0; 3],
                    offset: [0; 3],
                }),
                ..Default::default()
            },
        );
    }
    p.modules.insert("object".into(), m);
    let mut vm = Vm::new(world, 1);
    vm.refresh_registry_flags(&p, 2).unwrap();
    vm.refresh_registry_flags(&p, 1).unwrap();
    assert_eq!(
        vm.world.objects[&1].idle.as_ref().unwrap().registry_word(),
        0x0009000c
    );
    assert_eq!(
        vm.world.objects[&2].idle.as_ref().unwrap().registry_word(),
        0x0001000c
    );
}
