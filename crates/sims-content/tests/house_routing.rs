//! Original architecture and saved-world boundary supplied to the two native
//! obstacle routines. This is pre-activation evidence, not a playable load.
#[path = "support/catalog.rs"]
mod historical_catalog;
use serde_json::{Value, json};
use sims_sim::{Point, architecture::Architecture, lot_route, portals};
use std::{collections::BTreeSet, fs, path::PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn corpus() -> Vec<Value> {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let catalog = historical_catalog::global_controllers(&index).unwrap();
    let mut rows = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        let mut p = house.link_program(&index).unwrap();
        sims_content::catalog::link_controllers(&index, &mut p, &catalog).unwrap();
        let mut e = house.prepare_execution(&p).unwrap();
        e.load_simulator_context(
            &house,
            sims_vm::loading::HouseContext {
                simulator: sims_vm::loading::SimulatorContext {
                    game_mode: 0,
                    retained_global25: 0,
                    global30: 1,
                },
                installed_expansions: 0xff,
                performance_class: 2,
            },
        )
        .unwrap();
        e.reconcile_global_controllers(&p, &catalog).unwrap();
        let heads = e.repair_object_grid(&house).unwrap().floors;
        let lot = house.lot_geometry(&heads).unwrap();
        assert_eq!(lot.water, house.water_cells().unwrap());
        let a = Architecture::from_original(lot.clone(), &p).unwrap();
        let parts: Vec<_> =
            e.vm.world
                .objects
                .iter()
                .filter_map(|(&id, o)| {
                    let part = o.multitile.as_ref()?;
                    let f = &a.footprints[&o.definition.as_ref()?.guid];
                    Some(portals::PairPart {
                        id,
                        head: part.head,
                        next: part.next,
                        style: f.portal_style,
                        base_graphic: f.base_graphic,
                        portal: f.portal,
                    })
                })
                .collect();
        for (floor_index, f) in lot.floors.iter().enumerate() {
            let level = floor_index as i32 + 1;
            let mut tile_occupants = vec![vec![]; lot.size * lot.size];
            for (cell, &head) in heads[floor_index].iter().enumerate() {
                let mut pending = vec![head as i16];
                let mut seen = BTreeSet::new();
                while let Some(id) = pending.pop() {
                    if id == 0 {
                        continue;
                    }
                    // Registry lookup returns null for an absent saved ID.
                    let Some(o) = e.vm.world.objects.get(&id) else {
                        continue;
                    };
                    assert!(
                        seen.insert(id),
                        "cyclic tile/container traversal in {source}"
                    );
                    let definition = o.definition.as_ref().unwrap();
                    let shape = &a.footprints[&definition.guid];
                    let partner = portals::partner(&parts, id);
                    let partner_room = partner.map(|other| {
                        let saved = &house.objects.instances[&(other as u16)];
                        a.room_at(
                            saved.position[2].max(1),
                            Point::new(saved.position[0], saved.position[1]),
                        )
                    });
                    if shape.kind == 8 && shape.portal {
                        assert!(
                            partner_room.is_some(),
                            "{source} portal #{id} has no partner"
                        );
                    }
                    tile_occupants[cell].push(lot_route::TileOccupant {
                        kind: shape.kind,
                        flags: o.data[&8] as u16,
                        function15: shape.portal,
                        portal_class: shape.kind == 8,
                        partner_room,
                    });
                    let next = house
                        .objects
                        .instances
                        .get(&(id as u16))
                        .filter(|s| s.guid == definition.guid)
                        .and_then(|s| s.slots.first())
                        .map_or(0, |s| s.object);
                    // Native physical slot zero precedes containment slots.
                    pending.extend(o.slots.iter().rev().filter_map(|s| *s));
                    pending.push(next);
                }
            }
            for (&room, metric) in &f.metrics {
                if !f.rooms.primary.contains(&room) {
                    assert!(
                        lot.route_obstacles(room, &tile_occupants, true, false)
                            .is_err()
                    );
                    println!("{source} room {room}: split-only bounds explicitly unsupported");
                    continue;
                }
                let prepared = lot
                    .route_obstacles(room, &tile_occupants, true, false)
                    .unwrap();
                let available = tile_occupants
                    .iter()
                    .enumerate()
                    .map(|(i, objects)| {
                        lot_route::tile_available(
                            level,
                            f.rooms.primary[i],
                            room,
                            lot.water[i],
                            objects,
                        )
                        .unwrap()
                    })
                    .collect::<Vec<_>>();
                rows.push(json!({"source":source,"input":{
                    "size":lot.size,"level":level,"room":room,"primary":f.rooms.primary,
                    "walls":f.source.walls.iter().map(|w|w.segments).collect::<Vec<_>>(),
                    "styles":f.source.walls.iter().map(|w|w.styles).collect::<Vec<_>>(),
                    "floors":f.source.coverings,"flags":f.flags.flags,"water":lot.water,
                    "available":available,"tile_occupants":tile_occupants,
                    "outside":metric.outdoors,"terrain":true,"ignore_marker":false,
                },"rust":prepared}));
                println!(
                    "{source} room {room}: {} pre-tail rectangles",
                    prepared.rectangles.len()
                );
            }
        }
    }
    rows
}

#[test]
#[ignore = "exports private original house obstacle inputs"]
fn export_original_house_routing_corpus() {
    fs::write(
        repo().join("cache/original-house-route-obstacles.json"),
        serde_json::to_vec_pretty(&corpus()).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "requires original houses and combined native obstacle oracle"]
fn fresh_original_house_obstacles_match_native() {
    let expected: Value = serde_json::from_slice(
        &fs::read(repo().join("cache/research/sims-exe/oracle/house-route-obstacles.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        expected["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let fresh = corpus();
    assert_eq!(fresh.len(), expected["cases"].as_array().unwrap().len());
    for (actual, native) in fresh.iter().zip(expected["cases"].as_array().unwrap()) {
        assert_eq!(actual["source"], native["source"]);
        assert_eq!(actual["input"], native["input"]);
        assert_eq!(
            actual["rust"], native["original"],
            "{} room {}",
            actual["source"], actual["input"]["room"]
        );
    }
}
