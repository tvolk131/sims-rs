#![cfg(feature = "private-fixtures")]

use serde_json::{Value, json};
use sims_sim::{
    lot_links::Links,
    lot_presenter::{Inputs, View},
};
use sims_vm::{
    Definition, Module, Object, ObjectId, Program, World,
    autonomy::Placement,
    presenter::{Classification, Lists, Order},
    refresh::{DrawState, RefreshState},
};

fn ids(v: &Value) -> Vec<Option<ObjectId>> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().filter(|v| *v != 0).map(|v| v as i16))
        .collect()
}
fn draw(world: &World) -> Value {
    json!(
        (1..=4)
            .map(|id| {
                let d = &world.refresh.as_ref().unwrap().draw[&id];
                [u32::from(d.dynamic), d.expiry as u32, d.refreshed_at]
            })
            .collect::<Vec<_>>()
    )
}
fn links(size: usize) -> Links {
    Links {
        size,
        heads: vec![],
        next: Default::default(),
    }
}

#[test]
fn unknown_or_conflicting_presentation_byte_is_not_guessed() {
    use sims_vm::presenter::DurationView;
    let mut world = World {
        person: Some(Default::default()),
        refresh: Some(Default::default()),
        ..Default::default()
    };
    world.objects.insert(1, Object::default());
    let program = Program::default();
    let links = links(64);
    let mut inputs = Inputs {
        force_dynamic: false,
        graphic_ready: Default::default(),
        keep_dynamic: Default::default(),
    };
    assert!(
        View::new(&mut world, &program, &links, &inputs, &[])
            .unwrap()
            .keep_dynamic(1)
            .is_err()
    );
    inputs.keep_dynamic.insert(1, 128);
    assert!(
        View::new(&mut world, &program, &links, &inputs, &[])
            .unwrap()
            .keep_dynamic(1)
            .unwrap()
    );
    world.objects.get_mut(&1).unwrap().native_keep_dynamic = Some(0);
    assert!(
        View::new(&mut world, &program, &links, &inputs, &[])
            .unwrap()
            .keep_dynamic(1)
            .is_err()
    );
    inputs.keep_dynamic.clear();
    assert!(
        !View::new(&mut world, &program, &links, &inputs, &[])
            .unwrap()
            .keep_dynamic(1)
            .unwrap()
    );
    world.objects.get_mut(&1).unwrap().native_keep_dynamic = Some(255);
    assert!(
        View::new(&mut world, &program, &links, &inputs, &[])
            .unwrap()
            .keep_dynamic(1)
            .unwrap()
    );
}

#[test]
fn original_classification_through_loaded_world_fields() {
    classification(false);
    classification(true);
}
fn classification(retained: bool) {
    let report: Value = serde_json::from_str(include_str!(
        "../../sims-vm/tests/fixtures/presenter-classification.json"
    ))
    .unwrap();
    let mut count = 0;
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        if c["mode"] != "classify" {
            continue;
        }
        count += 1;
        let mut world = World::default();
        world.person = Some(Default::default());
        world.refresh = Some(RefreshState {
            visible_level: c["visible"].as_i64().unwrap() as i32,
            render_clock: c["clock"].as_u64().unwrap() as u32,
            ..Default::default()
        });
        let mut program = Program::default();
        let mut module = Module::default();
        let mut inputs = Inputs {
            force_dynamic: c["force"] != 0,
            graphic_ready: Default::default(),
            keep_dynamic: Default::default(),
        };
        for (n, o) in c["objects"].as_array().unwrap().iter().enumerate() {
            let id = n as i16 + 1;
            let mut words = vec![0; 128];
            words[4] = o["graphics"].as_i64().unwrap() as i16;
            words[10] = o["group"].as_i64().unwrap() as i16;
            words[101] = o["flags"].as_i64().unwrap() as i16;
            module.object_definition_words.insert(id as u16, words);
            world.objects.insert(
                id,
                Object {
                    native_keep_dynamic: retained.then(|| o["keep"].as_u64().unwrap() as u8),
                    definition: Some(Definition {
                        source: "oracle".into(),
                        object: id as u16,
                        guid: id as u32,
                        slots: 0,
                    }),
                    person: (o["person"] == true).then(Default::default),
                    data: [
                        (14, -1),
                        (26, o["parent"].as_i64().unwrap() as i16),
                        (34, o["word34"].as_i64().unwrap() as i16),
                    ]
                    .into(),
                    slots: ids(&o["slots"]),
                    ..Default::default()
                },
            );
            world.positions.insert(
                id,
                Placement {
                    x: o["tile"][1].as_i64().unwrap() as i32 * 16,
                    y: o["tile"][0].as_i64().unwrap() as i32 * 16,
                    level: o["level"].as_i64().unwrap() as i32,
                },
            );
            world.refresh.as_mut().unwrap().draw.insert(
                id,
                DrawState {
                    level: o["level"].as_i64().unwrap() as i32,
                    dynamic: o["draw"][0] != 0,
                    expiry: o["draw"][1].as_u64().unwrap() as i32,
                    refreshed_at: o["draw"][2].as_u64().unwrap() as u32,
                },
            );
            inputs
                .graphic_ready
                .insert(id, o["ready"].as_u64().unwrap() as u8);
            if !retained {
                inputs
                    .keep_dynamic
                    .insert(id, o["keep"].as_u64().unwrap() as u8);
            }
        }
        program.modules.insert("oracle".into(), module);
        let order: Vec<_> = ids(&c["order"]).into_iter().flatten().collect();
        let links = links(64);
        let mut owner = Classification::default();
        let mut lists = Lists::default();
        let mut events = vec![];
        let mut applied = vec![];
        loop {
            let request = owner
                .advance(
                    &mut View::new(&mut world, &program, &links, &inputs, &order).unwrap(),
                    &mut lists,
                )
                .unwrap();
            let Some(id) = request else {
                break;
            };
            let restored: (World, Classification, Inputs, Lists) = serde_json::from_slice(
                &serde_json::to_vec(&(&world, &owner, &inputs, &lists)).unwrap(),
            )
            .unwrap();
            assert_eq!(
                restored,
                (world.clone(), owner.clone(), inputs.clone(), lists.clone())
            );
            (world, owner, inputs, lists) = restored;
            owner.validate().unwrap();
            events.push(json!({"object":id,"draw":draw(&world)}));
            for (i, ch) in c["changes"].as_array().into_iter().flatten().enumerate() {
                if applied.contains(&i) || ch["at"] != json!(events.len()) {
                    continue;
                }
                applied.push(i);
                if let Some(clock) = ch["clock"].as_u64() {
                    world.refresh.as_mut().unwrap().render_clock = clock as u32;
                }
                if let Some(id) = ch["object"].as_i64() {
                    let id = id as i16;
                    if !ch["slots"].is_null() {
                        world.objects.get_mut(&id).unwrap().slots = ids(&ch["slots"]);
                    }
                    if let Some(value) = ch["word34"].as_i64() {
                        world
                            .objects
                            .get_mut(&id)
                            .unwrap()
                            .data
                            .insert(34, value as i16);
                    }
                    if let Some(value) = ch["flags"].as_i64() {
                        program
                            .modules
                            .get_mut("oracle")
                            .unwrap()
                            .object_definition_words
                            .get_mut(&(id as u16))
                            .unwrap()[101] = value as i16;
                    }
                }
            }
            View::new(&mut world, &program, &links, &inputs, &order)
                .unwrap()
                .invalidate(id)
                .unwrap();
            owner.acknowledge(id).unwrap();
        }
        assert!(owner.complete());
        assert_eq!(
            json!({"draw":draw(&world),"lists":[lists.dynamic,lists.ordinary,lists.special],"events":events,"applied":applied}),
            row["original"],
            "{c}"
        );
    }
    assert_eq!(count, 188);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_registry_order_uses_native_cached_column_row() {
    let report: Value = serde_json::from_str(include_str!(
        "../../sims-vm/tests/fixtures/presenter-order.json"
    ))
    .unwrap();
    let mut count = 0;
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        if c["heap"] == true {
            continue;
        }
        count += 1;
        let mut world = World::default();
        world.camera_rotation = c["rotation"].as_u64().unwrap() as u8;
        world.person = Some(Default::default());
        world.person.as_mut().unwrap().object_order =
            ids(&c["order"]).into_iter().flatten().collect();
        world.refresh = Some(Default::default());
        for (n, tile) in c["tiles"].as_array().unwrap().iter().enumerate() {
            world.positions.insert(
                n as i16 + 1,
                Placement {
                    x: tile[1].as_i64().unwrap() as i32 * 16,
                    y: tile[0].as_i64().unwrap() as i32 * 16,
                    level: tile[2].as_i64().unwrap() as i32,
                },
            );
        }
        let mut cache = Order {
            dirty: c["dirty"] != 0,
            rotation: c["cached_rotation"].as_u64().unwrap() as u32,
            objects: ids(&c["cached"]).into_iter().flatten().collect(),
        };
        let inputs = Inputs {
            force_dynamic: false,
            graphic_ready: Default::default(),
            keep_dynamic: Default::default(),
        };
        cache
            .get(
                &View::new(
                    &mut world,
                    &Program::default(),
                    &links(c["size"].as_u64().unwrap() as usize),
                    &inputs,
                    &[],
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            json!({"order":cache.objects,"dirty":cache.dirty,"rotation":cache.rotation}),
            row["original"]
        );
    }
    assert_eq!(count, 240);
}
