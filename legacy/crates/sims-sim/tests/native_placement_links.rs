use serde::Deserialize;
use serde_json::{Value, json};
use sims_sim::lot_links::{Descriptor, Links};
use sims_vm::{Object, World, autonomy::Placement};
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Input {
    size: usize,
    heads: Vec<Vec<i16>>,
    objects: Vec<Item>,
    #[serde(default)]
    operations: Vec<Value>,
}
#[derive(Deserialize)]
struct Item {
    id: i16,
    position: [i32; 3],
    data: BTreeMap<i16, i16>,
    slots: Vec<i16>,
    #[serde(default)]
    functions: Vec<usize>,
}
fn fixture(c: &Input) -> (Links, World) {
    let mut world = World::default();
    let mut links = Links {
        size: c.size,
        heads: c.heads.clone(),
        next: BTreeMap::new(),
    };
    for o in &c.objects {
        world.objects.insert(
            o.id,
            Object {
                data: o.data.clone(),
                slots: o
                    .slots
                    .iter()
                    .skip(1)
                    .map(|&id| (id != 0).then_some(id))
                    .collect(),
                ..Default::default()
            },
        );
        world.positions.insert(
            o.id,
            Placement {
                x: o.position[0],
                y: o.position[1],
                level: o.position[2],
            },
        );
        links.next.insert(o.id, o.slots[0]);
    }
    (links, world)
}
fn compare(report: Value) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c: Input = serde_json::from_value(row["input"].clone()).unwrap();
        let (mut links, mut world) = fixture(&c);
        let mut saved = None;
        for (step, op) in c.operations.iter().enumerate() {
            let id = op["id"].as_i64().unwrap() as i16;
            let mut captured = Value::Null;
            match op["kind"].as_str().unwrap() {
                "capture" => {
                    let d = links.capture(&mut world, id).unwrap();
                    if saved.is_none() || op["remember"].as_bool() == Some(true) {
                        saved = Some(d);
                    }
                    captured = json!(d);
                }
                "raw" => captured = json!(links.capture_raw(&world, id).unwrap()),
                "walk" => captured = json!(links.walk(&world, id).unwrap()),
                "position" => {
                    let p: [i32; 3] = serde_json::from_value(op["position"].clone()).unwrap();
                    world.positions.insert(
                        id,
                        Placement {
                            x: p[0],
                            y: p[1],
                            level: p[2],
                        },
                    );
                }
                "apply" => links
                    .apply(
                        &mut world,
                        id,
                        serde_json::from_value::<Descriptor>(op["descriptor"].clone()).unwrap(),
                    )
                    .unwrap(),
                "restore" => links.apply(&mut world, id, saved.unwrap()).unwrap(),
                _ => panic!("unknown operation"),
            }
            let objects: Vec<_> = c
                .objects
                .iter()
                .filter_map(|o| {
                    let object = &world.objects[&o.id];
                    let p = world.positions[&o.id];
                    let data: BTreeMap<_, _> = [2, 3, 14, 26]
                        .into_iter()
                        .map(|i| (i, object.data[&i]))
                        .collect();
                    let slots: Vec<_> = std::iter::once(links.next[&o.id])
                        .chain(object.slots.iter().map(|s| s.unwrap_or(0)))
                        .collect();
                    let initial_data:BTreeMap<_,_>=[2,3,14,26].into_iter().map(|i|(i,o.data[&i])).collect();
                    ([p.x,p.y,p.level]!=o.position || data!=initial_data || slots!=o.slots)
                        .then(|| json!({"id":o.id,"position":[p.x,p.y,p.level],"data":data,"slots":slots}))
                })
                .collect();
            let heads: Vec<BTreeMap<usize, i16>> = links
                .heads
                .iter()
                .zip(&c.heads)
                .map(|(actual, initial)| {
                    actual
                        .iter()
                        .zip(initial)
                        .enumerate()
                        .filter_map(|(i, (&a, &b))| (a != b).then_some((i, a)))
                        .collect()
                })
                .collect();
            let actual = json!({"captured":captured,"state":{"heads":heads,"objects":objects}});
            assert_eq!(
                actual, row["original"][step],
                "case {case} step {step}: {op}"
            );
        }
    }
}

fn compare_adjacency(report: Value) {
    use sims_formats::execution::ObjectFunction;
    use sims_vm::{Definition, Module, Program};
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c: Input = serde_json::from_value(row["input"].clone()).unwrap();
        let (links, mut world) = fixture(&c);
        let mut module = Module {
            source: "objects".into(),
            ..Default::default()
        };
        for o in &c.objects {
            world.objects.get_mut(&o.id).unwrap().definition = Some(Definition {
                source: "objects".into(),
                object: o.id as u16,
                guid: o.id as u32,
                slots: 0,
            });
            module.object_functions.insert(
                o.id as u16,
                (0..27)
                    .map(|i| ObjectFunction {
                        action: if o.functions.contains(&i) { 4096 } else { 0 },
                        condition: 0,
                    })
                    .collect(),
            );
        }
        let p = Program {
            globals: "objects".into(),
            modules: BTreeMap::from([("objects".into(), module)]),
            ..Default::default()
        };
        let tile = serde_json::from_value(row["input"]["tile"].clone()).unwrap();
        let mut callbacks = vec![];
        if let Some(mut scan) =
            sims_sim::lot_adjacency::Adjacency::begin(&links, &world, &p, tile).unwrap()
        {
            while let Some(update) = scan.next(&links, &mut world, &p).unwrap() {
                if update.callback {
                    callbacks.push(update.object);
                    if update.object == 1 && row["input"]["inject"] == true {
                        world.objects.get_mut(&1).unwrap().slots[1] = Some(7);
                        world
                            .objects
                            .get_mut(&7)
                            .unwrap()
                            .data
                            .extend([(14, 2), (26, 1)]);
                    }
                }
            }
        }
        let flags: BTreeMap<_, _> = world
            .objects
            .iter()
            .map(|(&id, o)| (id, o.data[&8] as u16))
            .collect();
        assert_eq!(
            json!({"flags":flags,"callbacks":callbacks}),
            row["original"],
            "adjacency case {i}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_linked_adjacency_observations() {
    compare_adjacency(serde_json::from_str(include_str!("fixtures/link-adjacency.json")).unwrap());
}
#[test]
#[ignore = "requires the private native adjacency corpus"]
fn full_native_linked_adjacency() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/link-adjacency.json");
    compare_adjacency(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_link_observations() {
    compare(serde_json::from_str(include_str!("fixtures/placement-links.json")).unwrap());
}

#[test]
#[ignore = "requires private native placement-link corpus"]
fn full_native_link_corpus() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle");
    compare(
        serde_json::from_slice(&std::fs::read(root.join("placement-links.json")).unwrap()).unwrap(),
    );
}

#[test]
#[ignore = "requires private original-house placement-link corpus"]
fn original_house_link_corpus() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle");
    let report: Value =
        serde_json::from_slice(&std::fs::read(root.join("house-placement-links.json")).unwrap())
            .unwrap();
    let inputs: Value = serde_json::from_slice(
        &std::fs::read(root.join("house-placement-link-inputs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["cases"].as_array().unwrap().len(),
        inputs.as_array().unwrap().len()
    );
    for (native, fresh) in report["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(inputs.as_array().unwrap())
    {
        assert_eq!(native["source"], fresh["source"]);
        assert_eq!(
            native["input"], fresh["input"],
            "stale private placement oracle input"
        );
    }
    compare(report);
}
