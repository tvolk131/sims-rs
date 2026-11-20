use sims_core::{CrtRng, GameRng};
use sims_formats::lot::Simulator;
use sims_formats::tatt::{Entry, Tatt};
use sims_sim::load::{Context, Household, LoadError};
use sims_sim::{Budget, TypeAttributes, World};
use std::collections::{BTreeMap, BTreeSet};

fn lot(globals: Vec<i16>) -> World {
    World {
        globals,
        lot_rng: GameRng::from_state(1),
        global_rng: GameRng::from_state(2),
        budget: Budget {
            funds: 777,
            pending_funds: 0,
            lot_value: 0,
            objects_value: 0,
            architecture_value: 0,
            days: Vec::new(),
        },
        ..World::empty()
    }
}

const CONTEXT: Context = Context {
    mode: -2,
    global25: 41,
    free_will: 1,
    expansions: 0xff,
    performance_class: 2,
};

#[test]
fn the_context_overlays_the_saved_globals() {
    // 32 saved words, as SIMI before version 0x41 holds.
    let saved: Vec<i16> = (100..132).collect();
    let mut world = lot(saved);
    world.apply_context(CONTEXT).unwrap();
    let g = &world.globals;
    assert_eq!(g.len(), 64);
    // Saved words survive unless the load sets them.
    assert_eq!(g[0], 100);
    assert_eq!(g[31], 131);
    // Words past the saved ones start at zero.
    assert_eq!(&g[34..], &[0; 30]);
    assert_eq!(g[15], -2, "mode is sign-extended");
    assert_eq!(g[25], 41);
    assert_eq!(g[29], 30);
    assert_eq!(g[30], 1);
    assert_eq!(g[16], 116, "speed in range is kept");
    assert_eq!(g[18], 116, "global 18 follows the speed");
    assert_eq!(g[20], 0xff);
    assert_eq!(g[33], 2);
    assert_eq!(g[32], 0);
    // The saved family stays until it is cleared, after the lot is resized.
    assert_eq!(g[9], 109);
    world.clear_family().unwrap();
    assert_eq!(world.globals[9], 0);
    assert_eq!(world.globals[10], 110);
}

#[test]
fn speed_is_clamped_and_a_negative_global_18_is_kept() {
    let mut saved = vec![0i16; 64];
    saved[16] = 5000;
    saved[18] = -1;
    saved[32] = 1;
    saved[40] = 9;
    let mut world = lot(saved);
    world.apply_context(CONTEXT).unwrap();
    assert_eq!(world.globals[16], 1000);
    assert_eq!(world.globals[18], -1);
    assert_eq!(world.globals[32], 0);
    assert_eq!(world.globals[40], 9);

    let mut saved = vec![0i16; 64];
    saved[16] = -7;
    saved[18] = 3;
    let mut world = lot(saved);
    world.apply_context(CONTEXT).unwrap();
    assert_eq!(world.globals[16], 0);
    assert_eq!(world.globals[18], 0);
}

#[test]
fn other_global_counts_are_rejected() {
    let mut world = lot(vec![0; 31]);
    assert_eq!(
        world.apply_context(CONTEXT),
        Err(LoadError::GlobalCount(31))
    );
}

#[test]
fn selecting_the_family_sets_its_globals_and_funds() {
    // A 32-word save has no budget mode global until the context is applied.
    let mut world = lot(vec![0; 32]);
    assert_eq!(
        world.select_family(3, 5, 1, None),
        Err(LoadError::NoContext)
    );
    world.apply_context(CONTEXT).unwrap();
    world.select_family(3, 5, 123_456, None).unwrap();
    assert_eq!(world.globals[9], 3);
    assert_eq!(world.globals[10], 5);
    assert_eq!(world.budget.funds, 123_456);
    assert_eq!((world.globals[14], world.globals[13]), (12, 3456));
    // A family record with no magic history leaves none selected.
    assert_eq!(world.magic_history, None);
    world
        .select_family(3, 5, 123_456, Some([1, 2, 3, 4, 5, 6]))
        .unwrap();
    assert_eq!(
        world.magic_history,
        Some(sims_sim::MagicHistory {
            family: 3,
            values: [1, 2, 3, 4, 5, 6]
        })
    );

    // Negative funds split toward zero.
    world.set_funds(-123_456).unwrap();
    assert_eq!((world.globals[14], world.globals[13]), (-12, -3456));

    // An empty lot has no money, whatever the family record says.
    world.select_family(0, 5, 9999, Some([9; 6])).unwrap();
    assert_eq!(world.budget.funds, 0);
    assert_eq!((world.globals[14], world.globals[13]), (0, 0));
    assert_eq!(world.magic_history, None);
}

#[test]
fn funds_do_not_change_outside_budget_mode_zero() {
    let mut world = lot(vec![0; 64]);
    world.apply_context(CONTEXT).unwrap();
    world.globals[32] = 1;
    world.select_family(3, 5, 500, None).unwrap();
    assert_eq!(world.globals[9], 3);
    assert_eq!(world.budget.funds, 777);
    assert_eq!(world.globals[13], 0);
}

fn simulator() -> Simulator {
    Simulator {
        version: 0x3e,
        globals: (0..32).collect(),
        ticks: 88444,
        scheduler_cursor: 3,
        funds: 2515,
        lot_rng: 11,
        global_rng: 22,
        pending_funds: 1,
        lot_value: 2,
        objects_value: 3,
        architecture_value: 4,
        budget_days: vec![Some([1; 8]), None],
        tail: Vec::new(),
    }
}

#[test]
fn the_simulator_state_is_read_into_the_lot_the_game_holds() {
    let mut world = World::empty();
    assert_eq!((world.size, world.globals.len()), (64, 64));
    // What the game holds that no house file does.
    world.size = 40;
    world.crt_rng = CrtRng::from_state(99);
    world.order = vec![7];
    world.removals = vec![7];
    world.event_owner = 7;
    world.read_simulator(&simulator());
    assert_eq!(world.ticks, 88444);
    assert_eq!(world.globals, (0..32).collect::<Vec<i16>>());
    assert_eq!(world.scheduler_cursor, 3);
    assert_eq!((world.lot_rng.state(), world.global_rng.state()), (11, 22));
    assert_eq!((world.budget.funds, world.budget.pending_funds), (2515, 1));
    assert_eq!(world.budget.days, [Some([1; 8]), None]);
    assert_eq!((world.size, world.crt_rng.state()), (40, 99));
    assert_eq!(
        (&world.order[..], &world.removals[..]),
        (&[7][..], &[7][..])
    );
    assert_eq!(world.event_owner, 7);
}

fn row(guid: u32, values: &[i16]) -> Entry {
    Entry {
        guid,
        values: values.to_vec(),
        count_end: 0,
        end: 0,
    }
}

fn attributes(guid: u32, values: &[i16]) -> TypeAttributes {
    TypeAttributes {
        guid,
        values: values.to_vec(),
    }
}

fn read(records: &[TypeAttributes], rows: Vec<Entry>) -> Vec<TypeAttributes> {
    let mut world = World::empty();
    world.type_attributes = records.to_vec();
    world.read_type_attributes(&Tatt {
        version: 0,
        packed: true,
        entries: rows,
        count_end: 0,
        consumed: 0,
    });
    world.type_attributes
}

#[test]
fn saved_type_attributes_fill_the_first_record_of_their_type() {
    let records = [
        attributes(1, &[9, 9, 9]),
        attributes(2, &[9]),
        attributes(1, &[9, 9]),
    ];
    // A short row zeroes the rest; a row for no record is skipped.
    let rows = vec![row(1, &[5, 6]), row(77, &[1; 128]), row(2, &[4])];
    assert_eq!(
        read(&records, rows),
        [
            attributes(1, &[5, 6, 0]),
            attributes(2, &[4]),
            attributes(1, &[9, 9]),
        ]
    );
}

#[test]
fn a_row_too_long_for_its_record_ends_the_read() {
    let records = [
        attributes(1, &[9]),
        attributes(2, &[9]),
        attributes(3, &[9]),
    ];
    let rows = vec![row(1, &[5]), row(2, &[4, 4]), row(3, &[3])];
    assert_eq!(
        read(&records, rows),
        [
            attributes(1, &[5]),
            attributes(2, &[9]),
            attributes(3, &[9])
        ]
    );
    // So does a row for no record that is longer than the scratch buffer.
    let rows = vec![row(77, &[1; 129]), row(1, &[5])];
    assert_eq!(read(&records, rows), records);
}

/// An object with nothing but an ID and a position.
fn object_at(id: u16, position: [i32; 3]) -> sims_sim::Object {
    sims_sim::Object {
        id,
        guid: 1,
        footprint: [0; 4],
        position,
        unknown_short: 0,
        attributes: Vec::new(),
        temps: Vec::new(),
        data: Vec::new(),
        obsolete_word: 0,
        stack_capacity_bytes: 0,
        stack: Vec::new(),
        relationship_flag: 0,
        relationships: Vec::new(),
        slots: Vec::new(),
        dynamic_sprites: Vec::new(),
        multipart: None,
        portal: None,
        person: None,
        registry_word: None,
        last_result: false,
        removal_skips_resets: false,
        selected: false,
    }
}

#[test]
fn the_scheduler_word_keeps_a_countdown_and_flags() {
    let mut object = sims_sim::Object {
        id: 1,
        guid: 1,
        footprint: [0; 4],
        position: [0; 3],
        unknown_short: 0,
        attributes: Vec::new(),
        temps: Vec::new(),
        data: Vec::new(),
        obsolete_word: 0,
        stack_capacity_bytes: 0,
        stack: Vec::new(),
        relationship_flag: 0,
        relationships: Vec::new(),
        slots: Vec::new(),
        dynamic_sprites: Vec::new(),
        multipart: None,
        portal: None,
        person: None,
        registry_word: None,
        last_result: false,
        removal_skips_resets: false,
        selected: false,
    };
    // Without a word there is nothing to set.
    object.set_countdown(5);
    object.set_flags(1, true);
    assert_eq!((object.countdown(), object.flags()), (None, 0));
    assert_eq!(object.registry_word, None);
    object.registry_word = Some(0x0009_1fd3);
    assert_eq!((object.countdown(), object.flags()), (Some(8147), 9));
    object.set_countdown(-2);
    assert_eq!(object.registry_word, Some(0x0009_fffe));
    object.set_flags(8, false);
    object.set_flags(2, true);
    assert_eq!((object.countdown(), object.flags()), (Some(-2), 3));
    // The top flag bit makes the word negative; nothing else changes.
    object.set_flags(0x8000, true);
    assert_eq!((object.countdown(), object.flags()), (Some(-2), 0x8003));
    object.set_countdown(19999);
    assert_eq!((object.countdown(), object.flags()), (Some(19999), 0x8003));
}

fn array(side: usize, stride: usize, fill: u8) -> sims_formats::lot::Array {
    sims_formats::lot::Array {
        width: side,
        height: side,
        stride,
        encoding: 0,
        data: vec![fill; side * side * stride],
    }
}

/// A house's arrays as versions 53 to 62 save them: one byte per floor
/// cell, 8 per wall cell, and every array filled with its own ID.
fn saved_arrays() -> std::collections::BTreeMap<u16, sims_formats::lot::Array> {
    use sims_sim::load::{SAVED_ARRAYS, WALLS};
    SAVED_ARRAYS
        .iter()
        .map(|&id| {
            let stride = if WALLS.contains(&id) { 8 } else { 1 };
            (id, array(64, stride, id as u8))
        })
        .collect()
}

/// The bytes of one cell.
fn cell(array: &sims_formats::lot::Array, x: usize, y: usize) -> &[u8] {
    array.cell(x, y).unwrap()
}

#[test]
fn the_saved_lot_arrays_replace_the_lots() {
    use sims_sim::load::SAVED_ARRAYS;
    let mut saved = saved_arrays();
    // An array the load does not ask for.
    saved.insert(55, array(64, 1, 55));
    let mut world = lot(vec![0; 64]);
    world.arrays.insert(4, array(64, 1, 200));
    world.arrays.insert(77, array(64, 1, 77));
    world.size = 52;
    let before = world.clone();
    world.read_arrays(&saved, 62).unwrap();
    let mut ids: Vec<u16> = SAVED_ARRAYS.to_vec();
    ids.push(77);
    ids.sort_unstable();
    assert_eq!(world.arrays.keys().copied().collect::<Vec<_>>(), ids);
    assert_eq!(world.arrays[&4].data[0], 4);
    assert_eq!(world.arrays[&77].data[0], 77, "an array of the lot's own");
    // Nothing else changes: the lot's size is not in the arrays.
    world.arrays = before.arrays.clone();
    assert_eq!(world.encode(), before.encode());

    // Arrays 9 and 10 may be absent; the lot keeps its own.
    let mut world = before.clone();
    world.arrays.insert(9, array(64, 1, 99));
    saved.remove(&9);
    saved.remove(&10);
    world.read_arrays(&saved, 62).unwrap();
    assert_eq!(world.arrays[&9].data[0], 99);
    assert!(!world.arrays.contains_key(&10));

    // Any other missing array, or one of another size, changes nothing.
    let mut world = before.clone();
    saved.remove(&103);
    assert_eq!(
        world.read_arrays(&saved, 62),
        Err(LoadError::MissingArray(103))
    );
    saved.insert(103, array(32, 2, 0));
    assert_eq!(
        world.read_arrays(&saved, 62),
        Err(LoadError::ArraySize {
            array: 103,
            width: 32,
            height: 32
        })
    );
    // Nor does a house version with another array layout.
    let saved = saved_arrays();
    assert_eq!(
        world.read_arrays(&saved, 52),
        Err(LoadError::ArrayVersion(52))
    );
    // Or a floor saved wide, which only later versions do.
    let mut wide = saved.clone();
    wide.insert(1, array(64, 2, 1));
    assert_eq!(
        world.read_arrays(&wide, 62),
        Err(LoadError::ArrayStride {
            array: 1,
            stride: 2
        })
    );
    assert_eq!(world.encode(), before.encode());
}

#[test]
fn saved_floors_and_walls_are_widened_inside_the_lot_size() {
    let mut saved = saved_arrays();
    // One wall cell with distinct bytes.
    let wall = saved.get_mut(&2).unwrap();
    let at = (3 * 64 + 5) * 8;
    wall.data[at..at + 8].copy_from_slice(&[0x11, 0x22, 3, 4, 5, 6, 250, 8]);
    let mut world = lot(vec![0; 64]);
    world.size = 52;
    // The lot's own upper floor, wide already, which keeps its cells
    // outside the size.
    world.arrays.insert(101, array(64, 2, 0xee));
    world.read_arrays(&saved, 62).unwrap();
    let floor = &world.arrays[&1];
    assert_eq!((floor.stride, floor.data.len()), (2, 64 * 64 * 2));
    assert_eq!(cell(floor, 0, 0), [1, 0]);
    assert_eq!(cell(floor, 51, 51), [1, 0]);
    // Outside the size a lot without the array has zeros.
    assert_eq!(cell(floor, 52, 0), [0, 0]);
    assert_eq!(cell(floor, 0, 52), [0, 0]);
    let upper = &world.arrays[&101];
    assert_eq!(cell(upper, 51, 0), [101, 0]);
    assert_eq!(cell(upper, 52, 0), [0xee, 0xee]);
    assert_eq!(cell(upper, 63, 63), [0xee, 0xee]);
    let wall = &world.arrays[&2];
    assert_eq!(wall.stride, 14);
    assert_eq!(
        cell(wall, 3, 5),
        [0x11, 0x22, 3, 0, 4, 0, 5, 0, 6, 0, 250, 0, 8, 0]
    );
    assert_eq!(
        cell(wall, 51, 0),
        [2, 2, 2, 0, 2, 0, 2, 0, 2, 0, 2, 0, 2, 0]
    );
    // Outside the size a lot without the array has a bare lot's cells:
    // on the ground level's walls, flags 0x0f and both styles 1.
    assert_eq!(cell(wall, 60, 60), sims_sim::load::BARE_GROUND_WALL);
    assert_eq!(cell(&world.arrays[&102], 60, 60), [0; 14]);
    // The other arrays are taken whole.
    assert_eq!(cell(&world.arrays[&3], 63, 63), [3]);
}

/// A house's arrays as versions from 63 save them: floors and walls
/// wide under their own IDs, every array filled with its own ID, and the
/// narrow floors and walls of an older version left in the file.
fn wide_arrays() -> std::collections::BTreeMap<u16, sims_formats::lot::Array> {
    use sims_sim::load::WIDE_SURFACES;
    let mut saved = saved_arrays();
    for (lot, chunk) in WIDE_SURFACES {
        let stride = if sims_sim::load::FLOORS.contains(&lot) {
            2
        } else {
            14
        };
        saved.insert(chunk, array(64, stride, chunk as u8));
    }
    saved
}

#[test]
fn from_version_63_floors_and_walls_are_read_whole_from_their_own_ids() {
    let mut saved = wide_arrays();
    let floor = saved.get_mut(&11).unwrap();
    floor.data[(3 * 64 + 5) * 2..][..2].copy_from_slice(&300u16.to_le_bytes());
    let mut world = lot(vec![0; 64]);
    world.size = 52;
    world.arrays.insert(101, array(64, 2, 0xee));
    let before = world.clone();
    for version in [63, 73] {
        let mut world = before.clone();
        world.read_arrays(&saved, version).unwrap();
        let floor = &world.arrays[&1];
        assert_eq!(cell(floor, 3, 5), 300u16.to_le_bytes());
        // Whole: outside the lot's size too.
        assert_eq!(cell(floor, 63, 63), [11, 11]);
        assert_eq!(cell(&world.arrays[&101], 60, 0), [111, 111]);
        assert_eq!(cell(&world.arrays[&2], 60, 60), [12; 14]);
        assert_eq!(cell(&world.arrays[&102], 0, 0), [112; 14]);
        assert_eq!(cell(&world.arrays[&3], 63, 63), [3]);
        // The chunks are not arrays of the lot.
        assert!(!world.arrays.contains_key(&11));
    }
    // A missing wide array, or one with narrow cells, changes nothing,
    // and the narrow chunks do not stand in for it.
    let mut world = before.clone();
    let mut narrow = saved.clone();
    narrow.remove(&112);
    assert_eq!(
        world.read_arrays(&narrow, 63),
        Err(LoadError::MissingArray(112))
    );
    narrow.insert(112, array(64, 8, 0));
    assert_eq!(
        world.read_arrays(&narrow, 63),
        Err(LoadError::ArrayStride {
            array: 112,
            stride: 8
        })
    );
    narrow.insert(112, array(64, 14, 0));
    narrow.insert(11, array(32, 2, 0));
    assert_eq!(
        world.read_arrays(&narrow, 63),
        Err(LoadError::ArraySize {
            array: 11,
            width: 32,
            height: 32
        })
    );
    assert_eq!(world.encode(), before.encode());
}

/// A lot of size 8 with empty floors and walls, already wide.
fn surfaces() -> World {
    let mut world = lot(vec![0; 64]);
    world.size = 8;
    for id in sims_sim::load::FLOORS {
        world.arrays.insert(id, array(64, 2, 0));
    }
    for id in sims_sim::load::WALLS {
        world.arrays.insert(id, array(64, 14, 0));
    }
    world
}

fn set(world: &mut World, id: u16, x: usize, y: usize, bytes: &[u8]) {
    let array = world.arrays.get_mut(&id).unwrap();
    let at = (x * 64 + y) * array.stride;
    array.data[at..at + bytes.len()].copy_from_slice(bytes);
}

#[test]
fn an_empty_map_gives_unlisted_floor_ids_65535() {
    let mut world = surfaces();
    // Built-in floors, the diagonal marker and 65535 keep their IDs.
    for (x, id) in [0u16, 29, 255, 65535].into_iter().enumerate() {
        set(&mut world, 1, x, 0, &id.to_le_bytes());
    }
    // IDs the map should have listed.
    for (x, id) in [30u16, 254, 256, 65534].into_iter().enumerate() {
        set(&mut world, 1, x, 1, &id.to_le_bytes());
    }
    set(&mut world, 101, 7, 7, &300u16.to_le_bytes());
    // Outside the lot's size nothing is looked at.
    set(&mut world, 1, 8, 0, &300u16.to_le_bytes());
    let before = world.clone();
    assert_eq!(
        world.renumber_surfaces(Some(&[]), Some(&[]), &Default::default()),
        Ok(5)
    );
    let floor = &world.arrays[&1];
    for (x, id) in [0u16, 29, 255, 65535].into_iter().enumerate() {
        assert_eq!(cell(floor, x, 0), id.to_le_bytes());
    }
    for x in 0..4 {
        assert_eq!(cell(floor, x, 1), [0xff, 0xff]);
    }
    assert_eq!(cell(&world.arrays[&101], 7, 7), [0xff, 0xff]);
    assert_eq!(cell(floor, 8, 0), 300u16.to_le_bytes());
    // A house without a floor map leaves the floors alone.
    let mut world = before.clone();
    assert_eq!(
        world.renumber_surfaces(None, Some(&[]), &Default::default()),
        Ok(0)
    );
    assert_eq!(world.encode(), before.encode());
}

/// A floor catalog with one pattern file after the built-in one: `tile.flr`,
/// whose pattern 1 has key 300 and whose pattern 2 is missing.
fn catalog() -> sims_content::Surfaces {
    use sims_content::{SurfaceCatalog, SurfaceFile, Surfaces};
    let file = |name: &str, keys: &[(u16, u16)]| SurfaceFile {
        name: name.into(),
        location: name.into(),
        keys: keys.iter().copied().collect(),
        price: None,
    };
    Surfaces {
        floors: SurfaceCatalog {
            files: vec![file("floors.iff", &[]), file("tile.flr", &[(1, 300)])],
        },
        walls: SurfaceCatalog {
            files: vec![file("walls.iff", &[]), file("brick.wll", &[(1, 400)])],
        },
    }
}

fn group(name: &str, entries: &[(u16, u16)]) -> sims_formats::lot::SurfaceMapping {
    sims_formats::lot::SurfaceMapping {
        name: name.into(),
        entries: entries
            .iter()
            .map(|&(id, local)| sims_formats::lot::SurfaceEntry {
                id,
                local,
                fallback: 0,
            })
            .collect(),
    }
}

#[test]
fn a_listed_id_takes_its_patterns_key_and_one_without_fails() {
    let patterns = catalog();
    let mut world = surfaces();
    set(&mut world, 1, 0, 0, &40u16.to_le_bytes());
    set(&mut world, 101, 3, 4, &40u16.to_le_bytes());
    set(&mut world, 1, 1, 0, &41u16.to_le_bytes());
    let before = world.clone();
    // The name matches ignoring case.
    let map = [group("TILE.flr", &[(40, 1)])];
    assert_eq!(world.renumber_surfaces(Some(&map), None, &patterns), Ok(3));
    assert_eq!(cell(&world.arrays[&1], 0, 0), 300u16.to_le_bytes());
    assert_eq!(cell(&world.arrays[&101], 3, 4), 300u16.to_le_bytes());
    assert_eq!(cell(&world.arrays[&1], 1, 0), [0xff, 0xff]);
    // A pattern the file lacks, a file the catalog lacks, the built-in
    // file and an ID past the table all need the colour match.
    for map in [
        [group("tile.flr", &[(40, 2)])],
        [group("stone.flr", &[(40, 1)])],
        [group("floors.iff", &[(40, 1)])],
        [group("tile.flr", &[(65535, 1)])],
    ] {
        let mut world = before.clone();
        assert!(matches!(
            world.renumber_surfaces(Some(&map), None, &patterns),
            Err(LoadError::SurfaceColour { .. })
        ));
        assert_eq!(world.encode(), before.encode());
    }
    // A later group's entry replaces an earlier one's for the same ID,
    // before the colour match is needed.
    let mut world = before.clone();
    let map = [group("tile.flr", &[(40, 1)]), group("tile.flr", &[(40, 2)])];
    assert!(matches!(
        world.renumber_surfaces(Some(&map), None, &patterns),
        Err(LoadError::SurfaceColour { id: 40, .. })
    ));
}

#[test]
fn a_wall_pattern_is_written_back_with_its_neighbours() {
    let patterns = catalog();
    let map = [group("brick.wll", &[(31, 1)])];
    // Each straight segment's pattern word, and the side of the wall the
    // service marks in the neighbour.
    for (segment, at, neighbour, side) in [(1u8, 6, (1, 3), 4u8), (2, 8, (2, 2), 8)] {
        let mut world = surfaces();
        let mut wall = [0u8; 14];
        wall[0] = segment;
        wall[at] = 31;
        set(&mut world, 102, 2, 3, &wall);
        assert_eq!(world.renumber_surfaces(None, Some(&map), &patterns), Ok(1));
        let cells = &world.arrays[&102];
        assert_eq!(cell(cells, 2, 3)[at..at + 2], 400u16.to_le_bytes());
        assert_eq!(cell(cells, neighbour.0, neighbour.1)[0], side);
        // Unlisted, the pattern becomes 65535.
        set(&mut world, 102, 2, 3, &wall);
        assert_eq!(world.renumber_surfaces(None, Some(&[]), &patterns), Ok(1));
        assert_eq!(cell(&world.arrays[&102], 2, 3)[at..at + 2], [0xff, 0xff]);
        // Without a wall map the walls are not looked at.
        set(&mut world, 102, 2, 3, &wall);
        assert_eq!(world.renumber_surfaces(Some(&[]), None, &patterns), Ok(0));
    }
    // An east or south wall is the neighbour's west or north one: written
    // back, it rewrites the neighbour's style, which the service reads from
    // the neighbour only when the neighbour owns the wall.
    for (segment, at, neighbour, owns) in [(4u8, 12, (3, 3), 1u8), (8, 10, (2, 4), 2)] {
        let mut world = surfaces();
        let mut wall = [0u8; 14];
        wall[0] = segment;
        wall[at] = 31;
        set(&mut world, 102, 2, 3, &wall);
        let before = world.clone();
        assert_eq!(
            world.renumber_surfaces(None, Some(&map), &patterns),
            Err(LoadError::SurfaceRewrite {
                level: 2,
                x: 2,
                y: 3
            })
        );
        assert_eq!(world.encode(), before.encode());
        let mut other = [0u8; 14];
        other[0] = owns;
        other[1] = 0x0f;
        other[2 * usize::from(owns)] = 7;
        set(&mut world, 102, neighbour.0, neighbour.1, &other);
        assert_eq!(world.renumber_surfaces(None, Some(&map), &patterns), Ok(1));
        let cells = &world.arrays[&102];
        assert_eq!(cell(cells, 2, 3)[at..at + 2], 400u16.to_le_bytes());
        assert_eq!(cell(cells, neighbour.0, neighbour.1), other);
    }
    // A segment the cell's byte has that the walk does not reach stays.
    let mut world = surfaces();
    let mut wall = [0u8; 14];
    wall[0] = 0x21;
    wall[6] = 31;
    wall[10] = 40;
    set(&mut world, 2, 1, 1, &wall);
    assert_eq!(world.renumber_surfaces(None, Some(&map), &patterns), Ok(1));
    let cells = &world.arrays[&2];
    assert_eq!(cell(cells, 1, 1)[6..8], 400u16.to_le_bytes());
    assert_eq!(cell(cells, 1, 1)[10..12], 40u16.to_le_bytes());
    // A lowest segment above 0x20 is not ported.
    let mut world = surfaces();
    wall[0] = 0x40;
    set(&mut world, 2, 1, 1, &wall);
    assert_eq!(
        world.renumber_surfaces(None, Some(&map), &patterns),
        Err(LoadError::SurfaceRewrite {
            level: 1,
            x: 1,
            y: 1
        })
    );
    // A lot without the arrays cannot be renumbered.
    let mut world = lot(vec![0; 64]);
    assert_eq!(
        world.renumber_surfaces(Some(&[]), Some(&[]), &patterns),
        Err(LoadError::MissingArray(1))
    );
}

#[test]
fn a_diagonal_walls_patterns_and_half_floors_are_renumbered() {
    let patterns = catalog();
    let floors = [group("tile.flr", &[(250, 1)])];
    let walls = [group("brick.wll", &[(100, 1)])];
    let mut world = surfaces();
    let mut wall = [0u8; 14];
    wall[0] = 0x10;
    wall[2] = 250; // the first half floor
    wall[4] = 9;
    wall[6] = 250; // the second
    wall[10] = 100;
    wall[12] = 100;
    set(&mut world, 2, 1, 1, &wall);
    // A floor that is not 255 does not read the halves.
    assert_eq!(
        world
            .clone()
            .renumber_surfaces(Some(&floors), None, &patterns),
        Ok(0)
    );
    set(&mut world, 1, 1, 1, &255u16.to_le_bytes());
    assert_eq!(
        world.renumber_surfaces(Some(&floors), Some(&walls), &patterns),
        Ok(2)
    );
    let mut expected = wall;
    expected[2..4].copy_from_slice(&300u16.to_le_bytes());
    expected[6..8].copy_from_slice(&300u16.to_le_bytes());
    expected[10..12].copy_from_slice(&400u16.to_le_bytes());
    expected[12..14].copy_from_slice(&400u16.to_le_bytes());
    assert_eq!(cell(&world.arrays[&2], 1, 1), expected);
    assert_eq!(cell(&world.arrays[&1], 1, 1), 255u16.to_le_bytes());
}

/// A lot of size 8 with empty object grids on both levels, and objects at
/// the given positions.
fn grid(objects: &[(u16, [i32; 3])]) -> World {
    let mut world = lot(vec![0; 64]);
    world.size = 8;
    world.arrays.insert(3, array(64, 2, 0));
    world.arrays.insert(103, array(64, 2, 0));
    for &(id, position) in objects {
        world.objects.insert(id, object_at(id, position));
        world.order.push(id);
    }
    world
}

fn head(world: &World, level: u16, x: usize, y: usize) -> i16 {
    let bytes = cell(&world.arrays[&level], x, y);
    i16::from_le_bytes([bytes[0], bytes[1]])
}

#[test]
fn an_objects_tile_is_its_position_in_sixteenths_rounded_down() {
    assert_eq!(World::tile([0, 0, 1]), [0, 0, 1]);
    assert_eq!(World::tile([47, 48, 2]), [2, 3, 2]);
    // Off the lot, where the factory puts a new object.
    assert_eq!(World::tile([-16, -16, 1]), [-1, -1, 1]);
    assert_eq!(World::tile([-1, -17, 0]), [-1, -2, 0]);
    // Each part is cut to a signed byte.
    assert_eq!(World::tile([128 * 16, 255 * 16, 257]), [-128, -1, 1]);
}

#[test]
fn grid_cells_that_name_an_object_which_is_elsewhere_are_cleared() {
    use sims_sim::load::GridRepair;
    let mut world = grid(&[
        (1, [2 * 16 + 5, 3 * 16 + 15, 1]), // on tile 2, 3
        (2, [2 * 16, 3 * 16, 1]),          // on the same tile, behind 1
        (3, [5 * 16, 16, 1]),              // on tile 5, 1
        (4, [-16, -16, 1]),                // off the lot
        (5, [6 * 16, 6 * 16, 2]),          // upstairs
        (6, [4 * 16, 4 * 16, 0]),          // on level 0
    ]);
    for (x, y, id) in [
        (2, 3, 1i16), // right
        (3, 2, 1),    // the same object, coordinates swapped
        (5, 1, 3),    // right
        (5, 2, 3),    // the next tile
        (0, 0, 4),    // an object that is off the lot
        (6, 6, 5),    // an object that is upstairs
        (4, 4, 6),    // an object whose level is 0, not 1
        (1, 1, 7),    // no such object
        (7, 7, -3),   // a negative ID
        (7, 0, 2),    // an object that is on another tile, not at its head
    ] {
        set(&mut world, 3, x, y, &id.to_le_bytes());
    }
    // Outside the lot's size, and upstairs: not looked at.
    set(&mut world, 3, 8, 0, &7i16.to_le_bytes());
    set(&mut world, 3, 0, 8, &7i16.to_le_bytes());
    set(&mut world, 3, 63, 63, &1i16.to_le_bytes());
    set(&mut world, 103, 1, 1, &7i16.to_le_bytes());
    set(&mut world, 103, 6, 6, &5i16.to_le_bytes());
    let before = world.clone();
    let repair = |tile, object, found| GridRepair {
        tile,
        object,
        found,
    };
    // Column by column, as the original walks the grid.
    assert_eq!(
        world.repair_grid(),
        Ok(vec![
            repair([0, 0], 4, Some([-1, -1, 1])),
            repair([1, 1], 7, None),
            repair([3, 2], 1, Some([2, 3, 1])),
            repair([4, 4], 6, Some([4, 4, 0])),
            repair([5, 2], 3, Some([5, 1, 1])),
            repair([6, 6], 5, Some([6, 6, 2])),
            repair([7, 0], 2, Some([2, 3, 1])),
            repair([7, 7], -3, None),
        ])
    );
    for (x, y, id) in [(2, 3, 1), (5, 1, 3), (8, 0, 7), (0, 8, 7), (63, 63, 1)] {
        assert_eq!(head(&world, 3, x, y), id, "cell {x}, {y}");
    }
    for (x, y) in [
        (0, 0),
        (1, 1),
        (3, 2),
        (4, 4),
        (5, 2),
        (6, 6),
        (7, 0),
        (7, 7),
    ] {
        assert_eq!(head(&world, 3, x, y), 0, "cell {x}, {y}");
    }
    // Nothing else changes: the upper grid, the objects, the list.
    assert_eq!(world.arrays[&103], before.arrays[&103]);
    let mut same = before.clone();
    same.arrays.insert(3, world.arrays[&3].clone());
    assert_eq!(world.encode(), same.encode());
    // A second pass finds nothing.
    assert_eq!(world.repair_grid(), Ok(vec![]));
    assert_eq!(world.encode(), same.encode());
}

#[test]
fn a_grid_that_is_missing_or_misshapen_is_refused() {
    let mut world = grid(&[]);
    world.arrays.remove(&3);
    assert_eq!(world.repair_grid(), Err(LoadError::MissingArray(3)));
    world.arrays.insert(3, array(64, 1, 0));
    assert_eq!(
        world.repair_grid(),
        Err(LoadError::ArrayStride {
            array: 3,
            stride: 1
        })
    );
    world.arrays.insert(3, array(32, 2, 0));
    assert_eq!(
        world.repair_grid(),
        Err(LoadError::ArraySize {
            array: 3,
            width: 32,
            height: 32
        })
    );
}

#[test]
fn discarding_the_removals_empties_the_queue_and_touches_no_object() {
    let mut world = grid(&[(4, [-16, -16, 1]), (9, [-16, -16, 1])]);
    world.objects.get_mut(&9).unwrap().removal_skips_resets = true;
    // An ID may outlive its object, and may name an object read later.
    world.removals = vec![4, 77, 9];
    let before = world.clone();
    assert_eq!(world.discard_removals(), [4, 77, 9]);
    assert!(world.removals.is_empty());
    let mut same = before;
    same.removals.clear();
    assert_eq!(world.encode(), same.encode());
    assert_eq!(world.discard_removals(), []);
}

/// A person whose person data word 61 (its family) and 32 (whether it is
/// a visitor) are given.
/// Person `id` of type `guid`, whose words 61 (family) and 32 (visitor)
/// hold `family` and `visitor`.
fn person_of(id: u16, guid: u32, family: i16, visitor: i16) -> sims_sim::Object {
    use sims_formats::saved_objects::{Interaction, Person};
    let idle = Interaction {
        uid: 0,
        caller: 0,
        target: 0,
        icon: 0,
        table_index: 0,
        args: [0; 4],
        priority: 0,
        action: 0,
        attenuation: 0.0,
        flags: 0,
    };
    let mut data = vec![0; 256];
    (data[61], data[32]) = (family, visitor);
    let mut object = object_at(id, [8, 8, 1]);
    object.guid = guid;
    object.person = Some(Person {
        animation_events: 0,
        engaged: 0,
        appearance: Default::default(),
        accessories: Vec::new(),
        animations: Default::default(),
        routing_state: 0,
        floats: [0.0; 9],
        previous_motives: [0.0; 16],
        motives: [0.0; 16],
        data,
        routing_frames: 0,
        active: idle.clone(),
        last: idle,
        queue: Vec::new(),
        object_uses: Vec::new(),
        motive_deltas: Vec::new(),
    });
    object
}

/// The family's members (types 10 and 11), the neighborhood's characters
/// (types 20 and 21), and a selection for house 5. In the neighborhood's
/// zoning house 5 is residential and house 7 is a community lot.
const MEMBERS: [u32; 2] = [10, 11];

fn zoning() -> BTreeMap<i16, i32> {
    BTreeMap::from([(5, 1), (7, 2)])
}

fn household<'a>(
    characters: &'a BTreeSet<u32>,
    residents: &'a BTreeSet<u16>,
    flags: u32,
    zoning: &'a BTreeMap<i16, i32>,
) -> Household<'a> {
    Household {
        house: 5,
        flags,
        members: &MEMBERS,
        characters,
        residents,
        zoning,
    }
}

fn visitor_word(world: &World, id: u16) -> i16 {
    world.objects[&id].person.as_ref().unwrap().data[32]
}

/// Adds person `id` to the lot.
fn add_person(world: &mut World, person: sims_sim::Object) {
    world.people.push(person.id);
    world.objects.insert(person.id, person);
}

#[test]
fn the_selection_ends_quietly_on_a_lot_with_only_the_household() {
    let characters = BTreeSet::from([20, 21]);
    let zones = zoning();
    let mut world = lot(vec![0; 64]);
    let none = BTreeSet::new();
    assert_eq!(
        lot(vec![0; 32]).finish_selection(&household(&characters, &none, 0, &zones)),
        Err(LoadError::NoContext)
    );
    world.apply_context(CONTEXT).unwrap();
    world.select_family(3, 5, 500, None).unwrap();
    world.objects.insert(1, object_at(1, [8, 8, 1]));
    add_person(&mut world, person_of(2, 10, 3, 0));
    add_person(&mut world, person_of(3, 11, 3, 0));
    let residents = world.residents();
    assert_eq!(residents, BTreeSet::from([2, 3]));
    let before = world.encode();
    let quiet = household(&characters, &residents, 0, &zones);
    assert_eq!(world.finish_selection(&quiet), Ok(()));
    // Other flags of the family's record do not matter.
    let other_flags = household(&characters, &residents, !4, &zones);
    assert_eq!(world.finish_selection(&other_flags), Ok(()));
    assert_eq!(world.encode(), before);

    // Each thing the original would act on is refused before anything is
    // written, and names itself.
    let refused = |world: &World, household: &Household<'_>| {
        let mut after = world.clone();
        let what = match after.finish_selection(household) {
            Err(LoadError::Selection(what)) => what,
            other => panic!("{other:?}"),
        };
        assert_eq!(after.encode(), world.encode());
        what
    };
    assert!(
        refused(&world, &household(&characters, &residents, 4, &zones))
            .contains("asks for the lot to be reset")
    );
    let mut tutorial = world.clone();
    tutorial.globals[26] = 1;
    assert!(refused(&tutorial, &quiet).contains("global 26"));
    // A neighborhood character of no family who lives in another house.
    let mut stray = world.clone();
    let mut person = person_of(4, 20, 0, 1);
    person.person.as_mut().unwrap().data[67] = 6;
    add_person(&mut stray, person);
    assert!(refused(&stray, &quiet).contains("person 4, a neighborhood character of no family"));
    // A neighborhood character who lived here and is not of the family.
    let mut moved_out = world.clone();
    add_person(&mut moved_out, person_of(4, 21, 7, 0));
    let residents = moved_out.residents();
    assert!(
        refused(&moved_out, &household(&characters, &residents, 0, &zones))
            .contains("person 4, a neighborhood character who lived here")
    );
    // Who was a resident is noted before the family is selected: a word 32
    // that changes after that does not save the character.
    let mut changed = moved_out.clone();
    changed
        .objects
        .get_mut(&4)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[32] = 1;
    assert!(
        refused(&changed, &household(&characters, &residents, 0, &zones))
            .contains("person 4, a neighborhood character who lived here")
    );
    assert_eq!(
        LoadError::Selection("x".into()).to_string(),
        "x, which is not ported"
    );
}

#[test]
fn the_selection_works_out_again_who_is_a_visitor() {
    let characters = BTreeSet::from([20, 21]);
    let mut world = lot(vec![0; 64]);
    world.apply_context(CONTEXT).unwrap();
    world.select_family(3, 5, 500, None).unwrap();
    // A member saved as a visitor; a stranger saved as a resident; a
    // stranger (such as the traveling salesman) whose word 32 is 2; a
    // neighborhood character of no family whose home is this house, and
    // one of another house who was already a visitor.
    add_person(&mut world, person_of(2, 10, 3, 1));
    add_person(&mut world, person_of(3, 30, 0, 0));
    add_person(&mut world, person_of(4, 31, 0, 2));
    let mut home = person_of(5, 20, 0, 1);
    home.person.as_mut().unwrap().data[67] = 5;
    add_person(&mut world, home);
    add_person(&mut world, person_of(6, 21, 9, 3));
    let residents = world.residents();
    assert_eq!(residents, BTreeSet::from([3]));
    let zones = zoning();
    let mut residential = world.clone();
    residential
        .finish_selection(&household(&characters, &residents, 0, &zones))
        .unwrap();
    let words = |world: &World| [2, 3, 4, 5, 6].map(|id| visitor_word(world, id));
    assert_eq!(words(&residential), [0, 1, 2, 1, 3]);
    // House 0 is residential whatever the zoning says.
    let mut house_0 = world.clone();
    house_0.globals[10] = 0;
    house_0
        .finish_selection(&household(&characters, &residents, 0, &BTreeMap::new()))
        .unwrap();
    assert_eq!(words(&house_0), [0, 1, 2, 1, 3]);
    // The zoning is of global 10 as it stands when the step runs: an
    // object's Load or Init tree that moved it to a community lot makes
    // even a member a visitor. So does a house the zoning does not list.
    for lot in [7, 9] {
        let mut moved = world.clone();
        moved.globals[10] = lot;
        moved
            .finish_selection(&household(&characters, &residents, 0, &zones))
            .unwrap();
        assert_eq!(words(&moved), [1, 1, 2, 1, 3]);
    }
}

/// A lot of 40 tiles whose objects have these categories (data word 59),
/// in registration order.
fn categorized(categories: &[i16]) -> World {
    let mut world = lot(vec![0; 64]);
    world.size = 40;
    for (id, &category) in (1u16..).zip(categories) {
        let mut object = object_at(id, [0; 3]);
        object.data = vec![0; 72];
        object.data[59] = category;
        world.objects.insert(id, object);
        world.order.push(id);
    }
    world
}

#[test]
fn the_statistics_say_what_kinds_of_object_the_lot_has() {
    let features = |categories: &[i16]| {
        let mut world = categorized(categories);
        world.globals[37] = -1;
        world.update_statistics().unwrap();
        assert_eq!(world.globals[23], 40);
        world.globals[37]
    };
    assert_eq!(features(&[]), 0);
    // One bit for each kind, whichever category of the kind it is.
    for (bit, kinds) in [
        (1, &[4, 46, 47, 48, 71][..]),
        (2, &[25, 26, 50, 72]),
        (4, &[73]),
        (0x40, &[43, 49, 51, 77]),
        (0x80, &[80]),
        (0x100, &[84]),
        (0x200, &[85]),
        (0x400, &[83]),
        (0x800, &[95]),
        (0x1000, &[93, 94]),
        (0x20, &[97]),
    ] {
        for &category in kinds {
            assert_eq!(features(&[0, category, 3]), bit, "category {category}");
            assert_eq!(sims_sim::load::feature(category), bit & !0x20);
        }
    }
    assert_eq!(features(&[4, 73, 80, 93]), 0x1085);
    // Three bits need category 10 and something else.
    for (bit, kinds) in [
        (8, &[17, 21, 22, 23, 24, 30, 39, 40, 58, 59, 74][..]),
        (0x10, &[75]),
        (0x20, &[76]),
    ] {
        for &category in kinds {
            assert_eq!(features(&[category]), 0, "category {category}");
            assert_eq!(features(&[category, 10]), bit, "category {category}");
        }
    }
    assert_eq!(features(&[10]), 0);
    // Categories next to the known ones set nothing.
    assert_eq!(features(&[3, 5, 9, 20, 27, 45, 52, 70, 78, 92, 96]), 0);
    // An object the registration list does not name is not looked at.
    let mut world = categorized(&[73]);
    world.order.clear();
    world.update_statistics().unwrap();
    assert_eq!(world.globals[37], 0);
    // The statistics need all 64 globals.
    let mut world = categorized(&[]);
    world.globals.truncate(32);
    assert_eq!(world.update_statistics(), Err(LoadError::NoContext));
}
