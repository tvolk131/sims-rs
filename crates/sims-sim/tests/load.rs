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
    // Outside the size a lot without the array has a bare lot's cells:
    // on the ground level's walls, flags 0x0f and both styles 1.
    assert_eq!(cell(wall, 60, 60), sims_sim::load::BARE_GROUND_WALL);
    assert_eq!(cell(&world.arrays[&102], 60, 60), [0; 14]);
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
fn person_of(id: u16, family: i16, visitor: i16) -> sims_sim::Object {
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

#[test]
fn the_selection_ends_quietly_on_a_lot_with_only_the_household() {
    let mut world = lot(vec![0; 64]);
    assert_eq!(
        lot(vec![0; 32]).finish_selection(0),
        Err(LoadError::NoContext)
    );
    world.apply_context(CONTEXT).unwrap();
    world.select_family(3, 5, 500, None).unwrap();
    world.objects.insert(1, object_at(1, [8, 8, 1]));
    world.objects.insert(2, person_of(2, 3, 0));
    let before = world.encode();
    assert_eq!(world.finish_selection(0), Ok(()));
    // Other flags of the family's record do not matter.
    assert_eq!(world.finish_selection(!4), Ok(()));
    assert_eq!(world.encode(), before);

    // Each thing the original would act on is refused, and names itself.
    let refused = |world: &World, flags: u32| match world.finish_selection(flags) {
        Err(LoadError::Selection(what)) => what,
        other => panic!("{other:?}"),
    };
    assert!(refused(&world, 4).contains("asks for the lot to be reset"));
    let mut tutorial = world.clone();
    tutorial.globals[26] = 1;
    assert!(refused(&tutorial, 0).contains("global 26"));
    let mut guest = world.clone();
    guest.objects.insert(3, person_of(3, 7, 0));
    assert!(refused(&guest, 0).contains("person 3 is of family 7"));
    let mut homeless = world.clone();
    homeless.objects.insert(3, person_of(3, 0, 0));
    assert!(refused(&homeless, 0).contains("person 3 is of family 0"));
    let mut visiting = world.clone();
    visiting.objects.insert(3, person_of(3, 3, 1));
    assert!(refused(&visiting, 0).contains("person 3 is a visitor"));
    assert_eq!(
        LoadError::Selection("x".into()).to_string(),
        "x, which is not ported"
    );
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
