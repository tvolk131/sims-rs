use serde::Deserialize;
use sims_formats::lot::WallTile;
use sims_sim::{
    architecture::{Architecture, Footprint},
    lot_geometry::{FloorSource, Lot},
    lot_loading,
};
use sims_vm::{Definition, Object, World, autonomy::Placement};
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Input {
    size: usize,
    level: i32,
    primary: Vec<i16>,
    walls: Vec<u8>,
    floors: Vec<u16>,
    root: i16,
    position: [i32; 3],
    objects: Vec<Item>,
}
#[derive(Deserialize)]
struct Item {
    id: i16,
    kind: i16,
    size: i16,
    insets: u16,
    facing: i16,
    person68: i16,
    person84: i16,
    person73: u16,
    room: i16,
    light: i16,
    position: [i32; 3],
    children: Vec<Option<i16>>,
}
fn world(c: &Input) -> (World, Architecture) {
    let floor = FloorSource {
        coverings: c.floors.clone(),
        walls: c
            .walls
            .iter()
            .map(|&segments| WallTile {
                segments,
                styles: [1, 1],
                ..Default::default()
            })
            .collect(),
        flags: vec![0; c.size * c.size],
        object_heads: vec![0; c.size * c.size],
    };
    let mut lot = Lot::rebuild(
        c.size,
        vec![[0; 4]; c.size * c.size],
        vec![255; c.size * c.size],
        vec![255; c.size * c.size],
        vec![floor; c.level as usize],
    )
    .unwrap();
    let mut a = Architecture::from_original(lot.clone(), &Default::default()).unwrap();
    // This oracle supplies the room grid; it is not a partition test. Replace
    // only the query boundary after constructing valid architectural storage.
    lot.floors[(c.level - 1) as usize].rooms.primary = c.primary.clone();
    a.original = Some(lot);
    let mut w = World::default();
    for o in &c.objects {
        w.objects.insert(
            o.id,
            Object {
                definition: Some(Definition {
                    source: "oracle".into(),
                    object: o.id as u16,
                    guid: o.id as u32,
                    slots: 0,
                }),
                data: BTreeMap::from([(1, o.facing), (7, o.light), (29, o.room)]),
                person: (o.kind == 2).then(|| {
                    BTreeMap::from([(68, o.person68), (73, o.person73 as i16), (84, o.person84)])
                }),
                slots: o.children.clone(),
                ..Default::default()
            },
        );
        w.positions.insert(
            o.id,
            Placement {
                x: o.position[0],
                y: o.position[1],
                level: o.position[2],
            },
        );
        a.footprints.insert(
            o.id as u32,
            Footprint {
                kind: o.kind,
                size: o.size,
                insets: o.insets,
                portal: false,
                portal_style: 0,
                base_graphic: 0,
            },
        );
    }
    (w, a)
}

#[test]
fn restoring_one_object_updates_children_but_not_unvisited_neighbors() {
    let c:Input=serde_json::from_value(serde_json::json!({
        "size":4,"level":1,"primary":[0,0,0,0,0,7,7,0,0,7,7,0,0,0,0,0],"walls":vec![0;16],"floors":vec![0;16],"root":1,"position":[24,24,1],
        "objects":[
            {"id":1,"kind":4,"size":16,"insets":0,"facing":0,"person68":0,"person73":0,"person84":0,"room":2,"light":1,"position":[40,40,1],"children":[2]},
            {"id":2,"kind":4,"size":4,"insets":0,"facing":0,"person68":0,"person73":0,"person84":0,"room":2,"light":0,"position":[40,40,1],"children":[]},
            {"id":3,"kind":4,"size":16,"insets":0,"facing":0,"person68":0,"person73":0,"person84":0,"room":2,"light":1,"position":[40,40,1],"children":[]}
        ]
    })).unwrap();
    let (mut w, a) = world(&c);
    let before = w.objects[&3].clone();
    let result = lot_loading::restore_position(
        &mut w,
        &a,
        1,
        Placement {
            x: 24,
            y: 24,
            level: 1,
        },
    )
    .unwrap();
    assert_eq!(
        result.objects.iter().map(|u| u.object).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(result.objects[0].bounds, [32, 32, 16, 16]);
    assert_eq!(
        result.effects,
        [
            lot_loading::Effect::Invalidate(1),
            lot_loading::Effect::Room(2),
            lot_loading::Effect::Room(7),
            lot_loading::Effect::Invalidate(2)
        ]
    );
    assert_eq!(w.objects[&3], before);
    assert_eq!(w.positions[&2].x, 24);
    assert_eq!(w.positions[&3].x, 40);
    w.objects.get_mut(&2).unwrap().slots.push(Some(1));
    let before = w.clone();
    assert!(
        lot_loading::restore_position(
            &mut w,
            &a,
            1,
            Placement {
                x: 48,
                y: 48,
                level: 1
            }
        )
        .is_err()
    );
    assert_eq!(w, before);
}

#[test]
#[ignore = "requires private original-code position restoration oracle"]
fn original_loaded_position_footprint_room_and_containment() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/loaded-position.json");
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c: Input = serde_json::from_value(row["input"].clone()).unwrap();
        let (mut w, a) = world(&c);
        let actual = lot_loading::restore_position(
            &mut w,
            &a,
            c.root,
            Placement {
                x: c.position[0],
                y: c.position[1],
                level: c.position[2],
            },
        )
        .unwrap();
        let expected: lot_loading::PlacementUpdate =
            serde_json::from_value(row["original"].clone()).unwrap();
        assert_eq!(actual, expected, "case {i}");
        for u in actual.objects {
            assert_eq!(w.positions[&u.object].level, c.position[2]);
            assert_eq!(w.objects[&u.object].data[&29], u.room);
        }
    }
}
