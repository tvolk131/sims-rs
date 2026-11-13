use sims_core::{CrtRng, GameRng};
use sims_formats::lot::Simulator;
use sims_formats::tatt::{Entry, Tatt};
use sims_sim::load::{Context, LoadError};
use sims_sim::{Budget, TypeAttributes, World};

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
    assert_eq!(world.select_family(3, 5, 1), Err(LoadError::NoContext));
    world.apply_context(CONTEXT).unwrap();
    world.select_family(3, 5, 123_456).unwrap();
    assert_eq!(world.globals[9], 3);
    assert_eq!(world.globals[10], 5);
    assert_eq!(world.budget.funds, 123_456);
    assert_eq!((world.globals[14], world.globals[13]), (12, 3456));

    // Negative funds split toward zero.
    world.set_funds(-123_456).unwrap();
    assert_eq!((world.globals[14], world.globals[13]), (-12, -3456));

    // An empty lot has no money, whatever the family record says.
    world.select_family(0, 5, 9999).unwrap();
    assert_eq!(world.budget.funds, 0);
    assert_eq!((world.globals[14], world.globals[13]), (0, 0));
}

#[test]
fn funds_do_not_change_outside_budget_mode_zero() {
    let mut world = lot(vec![0; 64]);
    world.apply_context(CONTEXT).unwrap();
    world.globals[32] = 1;
    world.select_family(3, 5, 500).unwrap();
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
    for version in [52, 63] {
        assert_eq!(
            world.read_arrays(&saved, version),
            Err(LoadError::ArrayVersion(version))
        );
    }
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
    assert_eq!(cell(wall, 60, 60), [0; 14]);
    // The other arrays are taken whole.
    assert_eq!(cell(&world.arrays[&3], 63, 63), [3]);
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
    assert_eq!(world.renumber_surfaces(Some(&[]), Some(&[])), Ok(5));
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
    assert_eq!(world.renumber_surfaces(None, Some(&[])), Ok(0));
    assert_eq!(world.encode(), before.encode());
}

#[test]
fn renumbering_that_needs_more_fails_and_changes_nothing() {
    use sims_formats::lot::{SurfaceEntry, SurfaceMapping};
    let listed = [SurfaceMapping {
        name: "tile.flr".into(),
        entries: vec![SurfaceEntry {
            id: 40,
            local: 1,
            fallback: 0,
        }],
    }];
    let mut world = surfaces();
    set(&mut world, 1, 0, 0, &40u16.to_le_bytes());
    let before = world.clone();
    // A map with entries needs the loaded patterns.
    assert_eq!(
        world.renumber_surfaces(Some(&listed), Some(&[])),
        Err(LoadError::SurfaceCatalog)
    );
    assert_eq!(
        world.renumber_surfaces(Some(&[]), Some(&listed)),
        Err(LoadError::SurfaceCatalog)
    );
    // A group with no entries lists nothing.
    let unlisted = [SurfaceMapping {
        name: "tile.flr".into(),
        entries: Vec::new(),
    }];
    assert_eq!(
        world.clone().renumber_surfaces(Some(&unlisted), None),
        Ok(1)
    );
    assert_eq!(world.encode(), before.encode());

    // Wall patterns: a straight segment's one word, by segment.
    for (segment, at) in [(1u8, 6), (2, 8), (4, 12), (8, 10)] {
        let mut world = surfaces();
        let mut wall = [0u8; 14];
        wall[0] = segment;
        wall[at] = 31;
        set(&mut world, 102, 2, 3, &wall);
        let stop = LoadError::SurfaceRewrite {
            level: 2,
            x: 2,
            y: 3,
        };
        assert_eq!(world.renumber_surfaces(Some(&[]), Some(&[])), Err(stop));
        // The same word under another segment is not that segment's
        // pattern, and built-in patterns stay.
        wall[0] = if segment == 1 { 2 } else { 1 };
        wall[6] = if at == 6 { 31 } else { 30 };
        wall[8] = if at == 6 { 248 } else { 0 };
        set(&mut world, 102, 2, 3, &wall);
        if at != 6 && at != 8 {
            assert_eq!(world.renumber_surfaces(Some(&[]), Some(&[])), Ok(0));
        }
        // Without a wall map the walls are not looked at.
        wall[0] = segment;
        set(&mut world, 102, 2, 3, &wall);
        assert_eq!(world.renumber_surfaces(Some(&[]), None), Ok(0));
    }
    // A diagonal segment has two pattern words, and its cell's floor is
    // two halves kept in the wall cell.
    let mut world = surfaces();
    let mut wall = [0u8; 14];
    wall[0] = 0x10;
    wall[12] = 100;
    set(&mut world, 2, 1, 1, &wall);
    let stop = LoadError::SurfaceRewrite {
        level: 1,
        x: 1,
        y: 1,
    };
    assert_eq!(
        world.renumber_surfaces(Some(&[]), Some(&[])),
        Err(stop.clone())
    );
    let mut world = surfaces();
    set(&mut world, 1, 1, 1, &255u16.to_le_bytes());
    wall[12] = 0;
    wall[2] = 250; // a half floor: 250 is renumbered as a floor ID
    set(&mut world, 2, 1, 1, &wall);
    assert_eq!(
        world.renumber_surfaces(Some(&[]), Some(&[])),
        Err(stop.clone())
    );
    // As a wall pattern 250 is built in, and a floor that is not 255 does
    // not read the halves.
    set(&mut world, 1, 1, 1, &0u16.to_le_bytes());
    assert_eq!(world.renumber_surfaces(Some(&[]), Some(&[])), Ok(0));
    // The walk stops when it reaches the last segment from a lower one:
    // with segments 1 and 0x20, only segment 1's pattern is looked at.
    let mut world = surfaces();
    let mut wall = [0u8; 14];
    wall[0] = 0x21;
    wall[10] = 40;
    set(&mut world, 2, 1, 1, &wall);
    assert_eq!(world.renumber_surfaces(Some(&[]), Some(&[])), Ok(0));
    wall[0] = 0x20;
    set(&mut world, 2, 1, 1, &wall);
    assert_eq!(world.renumber_surfaces(Some(&[]), Some(&[])), Err(stop));
    // A lot without the arrays cannot be renumbered.
    let mut world = lot(vec![0; 64]);
    assert_eq!(
        world.renumber_surfaces(Some(&[]), Some(&[])),
        Err(LoadError::MissingArray(1))
    );
}
