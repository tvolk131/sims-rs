//! Only fields read by the production room/physical routing adapters. Room
//! partitioning, map import, lighting and resize history are not executed here.
use super::*;
use crate::{
    architecture::{Architecture, Footprint},
    lot_geometry::{Floor, FloorSource, Lot},
    lot_rooms::{Partition, metrics},
};
use sims_formats::lot::WallTile;

pub(super) fn project(
    world: &mut World,
    captured: &Value,
    metadata: &Value,
    p: &Program,
) -> Architecture {
    let size = captured["floors"][0]["primary"]["width"].as_u64().unwrap() as usize;
    world.map_size = Some(sims_vm::world_map::State {
        active_size: captured["size"].as_i64().unwrap() as i32,
    });
    let pointers: BTreeMap<_, _> = captured["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            (
                o["pointer"].as_u64().unwrap() as u32,
                o["id"].as_i64().unwrap() as i16,
            )
        })
        .collect();
    let mut a = Architecture {
        width: (size * 16) as i32,
        height: (size * 16) as i32,
        floors: vec![],
        outdoors: BTreeMap::new(),
        footprints: BTreeMap::new(),
        original: None,
    };
    for raw in captured["objects"].as_array().unwrap() {
        let id = raw["id"].as_i64().unwrap() as i16;
        let o = world.objects.get_mut(&id).unwrap();
        let d = o.definition.as_ref().unwrap();
        let m = &p.modules[&d.source];
        let words = &m.object_definition_words[&d.object];
        let observed = metadata
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap();
        let function15 = m.object_functions[&d.object]
            .get(15)
            .map_or(0, |f| f.action);
        assert_eq!(json!(function15), observed["function15"]);
        if words[9] == 8 {
            assert_eq!(observed["portal_class"], true);
        }
        let footprint = Footprint {
            kind: words[9],
            portal_style: words[0x8a / 2],
            base_graphic: words[8],
            portal: function15 != 0,
            size: 0,
            insets: 0,
        };
        if let Some(old) = a.footprints.insert(d.guid, footprint.clone()) {
            assert_eq!(old, footprint);
        }
        if observed["multipart"] == true {
            let storage = bytes(&raw["storage"]);
            o.multitile = Some(sims_vm::multitile::Part {
                head: pointers.get(&word(&storage, 0x190)).copied(),
                next: pointers.get(&word(&storage, 0x18c)).copied(),
                origin: [0; 3],
                offset: [0; 3],
            });
        }
    }
    a.original = Some(Lot {
        size,
        active_size: None,
        heights: bytes(&captured["heights"]["bytes"])
            .chunks_exact(4)
            .map(|p| p.try_into().unwrap())
            .collect(),
        pool: vec![],
        water: bytes(&captured["water"]["bytes"]),
        floors: captured["floors"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let walls: Vec<_> = bytes(&f["walls"]["bytes"])
                    .chunks_exact(14)
                    .map(|b| {
                        let v = shorts(&b[2..]);
                        WallTile {
                            segments: b[0],
                            flags: b[1],
                            styles: [v[0] as u16, v[1] as u16],
                            patterns: std::array::from_fn(|n| v[2 + n] as u16),
                        }
                    })
                    .collect();
                let flags = bytes(&f["flags"]["bytes"]);
                let rooms = Partition {
                    primary: shorts(&bytes(&f["primary"]["bytes"])),
                    halves: vec![],
                    room_tiles: captured["rooms"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|r| r["id"].as_u64().unwrap() as usize >> 10 == i)
                        .map(|r| {
                            (
                                r["id"].as_i64().unwrap() as i16,
                                bytes(&r["tiles"])
                                    .chunks_exact(3)
                                    .map(|p| [p[0] as usize, p[1] as usize, p[2] as usize])
                                    .collect(),
                            )
                        })
                        .collect(),
                };
                let metrics = metrics(&rooms, &walls, &flags, size).unwrap();
                Floor {
                    source: FloorSource {
                        coverings: shorts(&bytes(&f["coverings"]["bytes"]))
                            .into_iter()
                            .map(|n| n as u16)
                            .collect(),
                        walls,
                        flags: flags.clone(),
                        object_heads: vec![],
                    },
                    rooms,
                    metrics,
                    flags: crate::lot_flags::Rebuilt {
                        flags,
                        upper_contents: false,
                        upper_support: false,
                    },
                }
            })
            .collect(),
    });
    a
}
