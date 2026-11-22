use sims_content::Install;
use sims_formats::lot::Array;
use sims_sim::World;
use sims_sim::load::{ExpandedWall, FLOORS, WALLS};
use sims_vm::boot::save::Saved;
use sims_vm::boot::{Frames, Lot};
use sims_vm::{
    ARCHITECTURE_BUDGET, BuildPrices, DEFAULT_GRASS_PRICE, DEFAULT_POOL_PRICE,
    DEFAULT_ROOF_PATTERN, DEFAULT_TERRAIN_PRICE, DEFAULT_WATER_PRICE, FLAG_LOCKED, FloorDragMode,
    GRASS_ARRAY, GRASS_RESTORED_VALUE, Interpreter, MAX_TERRAIN_SLOPE, PoolMode, ROOF_META_ARRAY,
    RoofPitch, RoofState, Stream, TARGET_GRASS_ARRAY, TERRAIN_DIRT_GRASS, TerrainMode, ToolPrices,
    WALL_WEST, WallDragMode, WallStyle, WaterMode, build_refund, is_grass_tile_editable,
    is_pool_tile, is_terrain_vertex_editable, is_water_tile, plan_floor_edit, plan_pool_edit,
    plan_roof_edit, plan_roof_edit_with_base, plan_roof_pattern, plan_roof_pattern_with_base,
    plan_roof_pitch, plan_roof_pitch_with_base, plan_terrain_edit, plan_wall_drag, plan_water_edit,
    preview_pool_on_world, preview_roof_on_world, preview_terrain_on_world, preview_water_on_world,
    roof_state, roof_state_with_base, terrain_vertex_height,
};
use std::path::PathBuf;

const FULL: usize = 64;
const WALL_CELL: usize = 14;

fn empty_array(stride: usize, fill: u8) -> Array {
    Array {
        width: FULL,
        height: FULL,
        stride,
        encoding: 0,
        data: vec![fill; FULL * FULL * stride],
    }
}

fn synthetic_world(size: i16, funds: i32, base_height: u8) -> World {
    let mut world = World::empty();
    world.size = size;
    world.budget.funds = funds;
    world.budget.days = vec![Some([0; 8])];
    world.arrays.insert(0, empty_array(4, base_height));
    world.arrays.insert(FLOORS[0], empty_array(2, 0));
    world.arrays.insert(FLOORS[1], empty_array(2, 0));
    world.arrays.insert(WALLS[0], empty_array(WALL_CELL, 0));
    world.arrays.insert(WALLS[1], empty_array(WALL_CELL, 0));
    world.arrays.insert(3, empty_array(2, 0));
    world.arrays.insert(103, empty_array(2, 0));
    world.arrays.insert(GRASS_ARRAY, empty_array(1, 50u8));
    world
        .arrays
        .insert(TARGET_GRASS_ARRAY, empty_array(1, 50u8));
    world.arrays.insert(8, empty_array(1, 0));
    world.arrays.insert(108, empty_array(1, 0));
    world.arrays.insert(9, empty_array(1, 0xff));
    world.arrays.insert(10, empty_array(1, 0xff));
    world.rebuild_rooms().expect("rebuild rooms");
    world
}

fn place_dummy_object(world: &mut World, id: u16, tile: [i32; 2], level: i32) {
    world.objects.insert(
        id,
        sims_sim::Object {
            id,
            guid: 0x1234_5678,
            footprint: [0; 4],
            position: [tile[0] * 16 + 8, tile[1] * 16 + 8, level],
            unknown_short: 0,
            attributes: Vec::new(),
            temps: Vec::new(),
            data: vec![0; 64],
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
        },
    );
}

fn cell(x: usize, y: usize) -> usize {
    x * FULL + y
}

fn empty_interpreter() -> (sims_vm::Program, sims_content::Catalog) {
    use std::collections::BTreeMap;
    let program = sims_vm::Program {
        global: 0,
        libraries: BTreeMap::new(),
        semi_globals: BTreeMap::new(),
        careers: None,
        career_texts_end: true,
        labels_hold_no_token: true,
        zoning: BTreeMap::new(),
        animations: Default::default(),
        people: Default::default(),
        icon_frames: None,
        wardrobe: None,
        ordered_wardrobe: None,
        character_files: Default::default(),
        expansions: 0,
        characters: Default::default(),
    };
    let catalog = sims_content::Catalog {
        global: 0,
        objects: Vec::new(),
        by_guid: BTreeMap::new(),
        semi_globals: BTreeMap::new(),
        diagnostics: Vec::new(),
    };
    (program, catalog)
}

fn arch_spend(world: &World) -> i32 {
    world.budget.days[0].unwrap_or([0; 8])[usize::from(ARCHITECTURE_BUDGET)]
}

#[test]
fn terrain_raise_lower_level_grass_guards_and_undo_redo() {
    let (program, catalog) = empty_interpreter();
    let interp = Interpreter::new(&program, &catalog, Stream::Global);
    let prices = ToolPrices::default();
    assert_eq!(prices.terrain_raise, DEFAULT_TERRAIN_PRICE);
    assert_eq!(prices.grass, DEFAULT_GRASS_PRICE);

    let mut world = synthetic_world(20, 1000, 0);

    // Raise vertex (10, 10) step by step up to MAX_TERRAIN_SLOPE (5).
    for step in 1..=MAX_TERRAIN_SLOPE {
        let plan = plan_terrain_edit(&world, [10, 10], [10, 10], TerrainMode::Raise, &prices);
        assert!(plan.valid && plan.affordable, "step {step} must be valid");
        assert_eq!(plan.net_cost, 1);
        assert_eq!(plan.height_changes.len(), 4);

        let mut preview = world.clone();
        preview_terrain_on_world(&mut preview, &plan);
        assert_eq!(terrain_vertex_height(&preview, 10, 10), Some(step));

        let hist = interp
            .build_commit_terrain(&mut world, &plan)
            .unwrap()
            .expect("commit raise");
        assert_eq!(hist.net_cost(), 1);
        assert_eq!(terrain_vertex_height(&world, 10, 10), Some(step));
    }
    assert_eq!(world.budget.funds, 1000 - i32::from(MAX_TERRAIN_SLOPE));
    assert_eq!(arch_spend(&world), i32::from(MAX_TERRAIN_SLOPE));

    // All four incident tiles lost grass to dirt (-100), while target grass stayed 50.
    for (tx, ty) in [(9, 9), (10, 9), (9, 10), (10, 10)] {
        assert_eq!(
            world.arrays[&GRASS_ARRAY].data[cell(tx, ty)] as i8,
            TERRAIN_DIRT_GRASS
        );
        assert_eq!(
            world.arrays[&TARGET_GRASS_ARRAY].data[cell(tx, ty)] as i8,
            50
        );
    }

    // 6th raise must be refused by MAX_TERRAIN_SLOPE against flat neighbors (0).
    let too_steep = plan_terrain_edit(&world, [10, 10], [10, 10], TerrainMode::Raise, &prices);
    assert!(
        !too_steep.valid,
        "exceeding MAX_TERRAIN_SLOPE must be refused"
    );

    // Raising the 4 orthogonal neighbors to 1 unlocks raising (10, 10) to 6.
    for v in [[9, 10], [11, 10], [10, 9], [10, 11]] {
        let p = plan_terrain_edit(&world, v, v, TerrainMode::Raise, &prices);
        assert!(p.valid);
        interp
            .build_commit_terrain(&mut world, &p)
            .unwrap()
            .unwrap();
    }
    let step6 = plan_terrain_edit(&world, [10, 10], [10, 10], TerrainMode::Raise, &prices);
    assert!(step6.valid, "raising neighbors unlocks step 6");
    let mut step6_hist = interp
        .build_commit_terrain(&mut world, &step6)
        .unwrap()
        .unwrap();
    assert_eq!(terrain_vertex_height(&world, 10, 10), Some(6));

    // Undo and Redo step 6.
    assert!(
        interp
            .build_undo_entry(&mut world, &mut step6_hist)
            .unwrap()
    );
    assert_eq!(terrain_vertex_height(&world, 10, 10), Some(5));
    assert!(
        interp
            .build_redo_entry(&mut world, &mut step6_hist)
            .unwrap()
    );
    assert_eq!(terrain_vertex_height(&world, 10, 10), Some(6));

    // Lower (10, 10) back to 5.
    let lower = plan_terrain_edit(&world, [10, 10], [10, 10], TerrainMode::Lower, &prices);
    assert!(lower.valid && lower.net_cost == 1);
    interp
        .build_commit_terrain(&mut world, &lower)
        .unwrap()
        .unwrap();
    assert_eq!(terrain_vertex_height(&world, 10, 10), Some(5));

    // LevelAverage across 3x3 vertex box (9..=11, 9..=11).
    let avg_plan = plan_terrain_edit(&world, [9, 9], [11, 11], TerrainMode::LevelAverage, &prices);
    assert!(avg_plan.valid);
    let mut avg_hist = interp
        .build_commit_terrain(&mut world, &avg_plan)
        .unwrap()
        .unwrap();
    assert_eq!(terrain_vertex_height(&world, 10, 10), Some(1));
    assert!(interp.build_undo_entry(&mut world, &mut avg_hist).unwrap());
    assert_eq!(terrain_vertex_height(&world, 10, 10), Some(5));

    // Level drag anchored at (8, 8) (height 0) flattens (8..=11, 8..=11) back to 0.
    let level_plan = plan_terrain_edit(&world, [8, 8], [11, 11], TerrainMode::Level, &prices);
    assert!(level_plan.valid && level_plan.net_cost == 5);
    let mut level_hist = interp
        .build_commit_terrain(&mut world, &level_plan)
        .unwrap()
        .unwrap();
    for vx in 8..=11 {
        for vy in 8..=11 {
            assert_eq!(terrain_vertex_height(&world, vx, vy), Some(0));
        }
    }

    // Restore grass over (9..=10, 9..=10) (+$4), then Ctrl remove grass ($0).
    let funds_before_grass = world.budget.funds;
    let grass_plan =
        plan_terrain_edit(&world, [9, 9], [10, 10], TerrainMode::RestoreGrass, &prices);
    assert!(grass_plan.valid && grass_plan.net_cost == 4);
    let mut grass_hist = interp
        .build_commit_terrain(&mut world, &grass_plan)
        .unwrap()
        .unwrap();
    assert_eq!(world.budget.funds, funds_before_grass - 4);
    for (tx, ty) in [(9, 9), (10, 9), (9, 10), (10, 10)] {
        assert_eq!(
            world.arrays[&GRASS_ARRAY].data[cell(tx, ty)] as i8,
            GRASS_RESTORED_VALUE
        );
    }
    assert!(
        interp
            .build_undo_entry(&mut world, &mut grass_hist)
            .unwrap()
    );
    assert_eq!(world.budget.funds, funds_before_grass);
    assert_eq!(
        world.arrays[&GRASS_ARRAY].data[cell(9, 9)] as i8,
        TERRAIN_DIRT_GRASS
    );
    assert!(
        interp
            .build_redo_entry(&mut world, &mut grass_hist)
            .unwrap()
    );
    assert_eq!(
        world.arrays[&GRASS_ARRAY].data[cell(9, 9)] as i8,
        GRASS_RESTORED_VALUE
    );

    let strip_grass = plan_terrain_edit(&world, [9, 9], [9, 9], TerrainMode::RemoveGrass, &prices);
    assert!(strip_grass.valid && strip_grass.net_cost == 0);
    let mut strip_hist = interp
        .build_commit_terrain(&mut world, &strip_grass)
        .unwrap()
        .unwrap();
    assert_eq!(
        world.arrays[&GRASS_ARRAY].data[cell(9, 9)] as i8,
        TERRAIN_DIRT_GRASS
    );
    assert!(
        interp
            .build_undo_entry(&mut world, &mut strip_hist)
            .unwrap()
    );
    assert_eq!(
        world.arrays[&GRASS_ARRAY].data[cell(9, 9)] as i8,
        GRASS_RESTORED_VALUE
    );

    // Undo the level operation to verify terrain + buildable flags round-trip.
    assert!(
        interp
            .build_undo_entry(&mut world, &mut grass_hist)
            .unwrap()
    );
    assert!(
        interp
            .build_undo_entry(&mut world, &mut level_hist)
            .unwrap()
    );
    assert_eq!(terrain_vertex_height(&world, 10, 10), Some(5));
}

#[test]
fn terrain_enforces_road_lock_wall_floor_object_and_stale_guards() {
    let (program, catalog) = empty_interpreter();
    let interp = Interpreter::new(&program, &catalog, Stream::Global);
    let prices = ToolPrices::default();
    let mut world = synthetic_world(20, 1000, 0);

    // Border vertices (x < 2 or x >= size - 1) are never editable.
    assert!(!is_terrain_vertex_editable(&world, 0, 10));
    assert!(!is_terrain_vertex_editable(&world, 1, 10));
    assert!(!is_terrain_vertex_editable(&world, 19, 10));
    assert!(is_terrain_vertex_editable(&world, 5, 5));

    // Road tile at (4, 4) locks all 4 of its corners (4,4), (5,4), (4,5), (5,5).
    let off = cell(4, 4) * 2;
    world.arrays.get_mut(&FLOORS[0]).unwrap().data[off..off + 2]
        .copy_from_slice(&10u16.to_le_bytes());
    assert!(!is_terrain_vertex_editable(&world, 5, 5));
    assert!(!is_grass_tile_editable(&world, 4, 4));

    // FLAG_LOCKED (0x20) at (7, 7) blocks vertex (8, 8) and grass at (7, 7).
    world.arrays.get_mut(&8).unwrap().data[cell(7, 7)] |= FLAG_LOCKED;
    assert!(!is_terrain_vertex_editable(&world, 8, 8));
    assert!(!is_grass_tile_editable(&world, 7, 7));

    // Floor tile at (11, 11) blocks vertex (12, 12) and grass at (11, 11).
    let foff = cell(11, 11) * 2;
    world.arrays.get_mut(&FLOORS[0]).unwrap().data[foff..foff + 2]
        .copy_from_slice(&1u16.to_le_bytes());
    assert!(!is_terrain_vertex_editable(&world, 12, 12));
    assert!(!is_grass_tile_editable(&world, 11, 11));

    // Wall on (14, 14) blocks vertex (15, 15).
    let mut w = ExpandedWall::get(&world.arrays[&WALLS[0]].data, 20, 14, 14);
    w.segments |= WALL_WEST;
    w.styles[0] = Some(1);
    w.put(
        &mut world.arrays.get_mut(&WALLS[0]).unwrap().data,
        20,
        14,
        14,
    )
    .unwrap();
    assert!(!is_terrain_vertex_editable(&world, 15, 15));

    // Plan raise at (16, 16) while clear, then place an object on (16, 16) -> stale commit refused!
    let stale_plan = plan_terrain_edit(&world, [16, 16], [16, 16], TerrainMode::Raise, &prices);
    assert!(stale_plan.valid);
    place_dummy_object(&mut world, 42, [16, 16], 1);
    assert!(!is_terrain_vertex_editable(&world, 16, 16));
    assert!(
        interp
            .build_commit_terrain(&mut world, &stale_plan)
            .unwrap()
            .is_none(),
        "stale terrain plan must be rejected after object placement"
    );
    assert_eq!(world.budget.funds, 1000);
}

#[test]
fn water_pond_paint_depression_masks_guards_refund_and_undo_redo() {
    let (program, catalog) = empty_interpreter();
    let interp = Interpreter::new(&program, &catalog, Stream::Global);
    let prices = ToolPrices::default();
    assert_eq!(prices.water, DEFAULT_WATER_PRICE);
    assert_eq!(build_refund(prices.water), 12);

    // Start at elevation 3 so pond depression is visible.
    let mut world = synthetic_world(20, 500, 3);
    // Test optional Arry 10 creation & Undo removal.
    world.arrays.remove(&10);

    let paint = plan_water_edit(&world, [5, 5], [6, 6], WaterMode::Paint, &prices);
    assert!(paint.valid && paint.affordable);
    assert!(paint.created_water_array);
    assert_eq!(paint.net_cost, 4 * 15);
    assert!(
        !paint.height_changes.is_empty(),
        "pond should depress elevated basin"
    );

    let mut preview = world.clone();
    preview_water_on_world(&mut preview, &paint);
    assert!(is_water_tile(&preview, 5, 5));

    let mut paint_hist = interp
        .build_commit_water(&mut world, &paint)
        .unwrap()
        .expect("commit pond paint");
    assert_eq!(world.budget.funds, 500 - 60);
    assert_eq!(arch_spend(&world), 60);

    // Verify exact Complete Collection 4-bit connectivity masks (N=1, W=2, S=4, E=8).
    let wdata = &world.arrays[&10].data;
    assert_eq!(wdata[cell(5, 5)], 0x0c); // E + S
    assert_eq!(wdata[cell(6, 5)], 0x06); // W + S
    assert_eq!(wdata[cell(5, 6)], 0x09); // E + N
    assert_eq!(wdata[cell(6, 6)], 0x03); // W + N
    assert_eq!(terrain_vertex_height(&world, 6, 6), Some(2));

    // Ctrl-remove half the pond (x=6, y=5..=6) -> 80% refund (-$24) and masks update!
    let rem = plan_water_edit(&world, [6, 5], [6, 6], WaterMode::Remove, &prices);
    assert!(rem.valid && rem.net_cost == -24);
    let mut rem_hist = interp
        .build_commit_water(&mut world, &rem)
        .unwrap()
        .expect("commit pond remove");
    assert_eq!(world.budget.funds, 500 - 60 + 24);
    assert_eq!(world.arrays[&10].data[cell(5, 5)], 0x04); // S only
    assert_eq!(world.arrays[&10].data[cell(5, 6)], 0x01); // N only
    assert_eq!(world.arrays[&10].data[cell(6, 5)], 0xff);
    assert_eq!(world.arrays[&10].data[cell(6, 6)], 0xff);

    // Locked river tile refuses removal.
    world.arrays.get_mut(&8).unwrap().data[cell(5, 5)] |= FLAG_LOCKED;
    let locked_rem = plan_water_edit(&world, [5, 5], [5, 5], WaterMode::Remove, &prices);
    assert!(!locked_rem.valid, "locked river tile must refuse removal");
    world.arrays.get_mut(&8).unwrap().data[cell(5, 5)] &= !FLAG_LOCKED;

    // Undo remove -> Undo paint -> Arry 10 removed & elevation + funds restored!
    assert!(interp.build_undo_entry(&mut world, &mut rem_hist).unwrap());
    assert_eq!(world.arrays[&10].data[cell(5, 5)], 0x0c);
    assert!(
        interp
            .build_undo_entry(&mut world, &mut paint_hist)
            .unwrap()
    );
    assert!(!world.arrays.contains_key(&10));
    assert_eq!(terrain_vertex_height(&world, 6, 6), Some(3));
    assert_eq!(world.budget.funds, 500);

    // Redo paint -> Redo remove.
    assert!(
        interp
            .build_redo_entry(&mut world, &mut paint_hist)
            .unwrap()
    );
    assert!(is_water_tile(&world, 5, 5));
    assert!(interp.build_redo_entry(&mut world, &mut rem_hist).unwrap());
    assert!(!is_water_tile(&world, 6, 5));
}

#[test]
fn swimming_pool_dig_expand_demolish_rooms_guards_and_undo_redo() {
    let (program, catalog) = empty_interpreter();
    let interp = Interpreter::new(&program, &catalog, Stream::Global);
    let prices = ToolPrices::default();
    assert_eq!(prices.pool, DEFAULT_POOL_PRICE);
    assert_eq!(build_refund(prices.pool), 60);

    let mut world = synthetic_world(20, 1000, 0);

    // Dig a 2x2 swimming pool ($300).
    let dig = plan_pool_edit(&world, [8, 8], [9, 9], PoolMode::Dig, &prices);
    assert!(dig.valid && dig.affordable && dig.net_cost == 300);

    let mut preview = world.clone();
    preview_pool_on_world(&mut preview, &dig);
    assert!(is_pool_tile(&preview, 8, 8));

    let mut dig_hist = interp
        .build_commit_pool(&mut world, &dig)
        .unwrap()
        .expect("commit pool dig");
    assert_eq!(world.budget.funds, 700);
    assert_eq!(arch_spend(&world), 300);
    let pdata = &world.arrays[&9].data;
    assert_eq!(pdata[cell(8, 8)], 0x0c);
    assert_eq!(pdata[cell(9, 8)], 0x06);
    assert_eq!(pdata[cell(8, 9)], 0x09);
    assert_eq!(pdata[cell(9, 9)], 0x03);

    // Rebuild rooms must mark the pool tiles as a pool room (`room.pool == true`).
    let pool_room_id = world.rooms.map[0][cell(8, 8)];
    assert!(
        world.rooms.rooms.get(&pool_room_id).is_some_and(|r| r.pool),
        "pool tiles must belong to a rebuilt pool room"
    );

    // Expanding over (8..=10, 8..=9) charges only the 2 new tiles ($150).
    let expand = plan_pool_edit(&world, [8, 8], [10, 9], PoolMode::Dig, &prices);
    assert!(expand.valid && expand.net_cost == 150);
    let mut expand_hist = interp
        .build_commit_pool(&mut world, &expand)
        .unwrap()
        .expect("commit pool expand");
    assert_eq!(world.arrays[&9].data[cell(9, 8)], 0x0e);
    assert_eq!(world.arrays[&9].data[cell(10, 8)], 0x06);
    assert_eq!(world.budget.funds, 550);

    // Guards: sloped tile refuses pool dig!
    let raise = plan_terrain_edit(&world, [14, 14], [14, 14], TerrainMode::Raise, &prices);
    interp
        .build_commit_terrain(&mut world, &raise)
        .unwrap()
        .unwrap();
    let sloped_pool = plan_pool_edit(&world, [13, 13], [14, 14], PoolMode::Dig, &prices);
    assert!(!sloped_pool.valid, "sloped tile must refuse pool digging");

    // Guards: wall, floor, water, and object refuse pool dig!
    let build_prices = BuildPrices::default();
    let wplan = plan_wall_drag(
        &world,
        [3, 3],
        [3, 5],
        1,
        WallStyle::Standard,
        WallDragMode::Build,
        &build_prices,
    );
    interp
        .build_commit_wall(&mut world, &wplan)
        .unwrap()
        .unwrap();
    assert!(!plan_pool_edit(&world, [3, 3], [4, 4], PoolMode::Dig, &prices).valid);

    let fplan = plan_floor_edit(
        &world,
        [5, 3],
        [5, 3],
        1,
        1,
        FloorDragMode::Lay,
        &build_prices,
        None,
    );
    interp
        .build_commit_floor(&mut world, &fplan)
        .unwrap()
        .unwrap();
    assert!(!plan_pool_edit(&world, [5, 3], [5, 3], PoolMode::Dig, &prices).valid);

    place_dummy_object(&mut world, 77, [5, 14], 1);
    assert!(!plan_pool_edit(&world, [5, 14], [5, 14], PoolMode::Dig, &prices).valid);

    // Demolish guard: object standing inside pool tile blocks demolishing that tile.
    place_dummy_object(&mut world, 88, [10, 9], 1);
    assert!(
        !plan_pool_edit(&world, [10, 8], [10, 9], PoolMode::Demolish, &prices).valid,
        "object/Sim inside pool tile must block demolition"
    );
    world.objects.remove(&88);

    // Ctrl-demolish the 2 expanded tiles -> 80% refund (-$120).
    let funds_before_demolish = world.budget.funds;
    let demo = plan_pool_edit(&world, [10, 8], [10, 9], PoolMode::Demolish, &prices);
    assert!(demo.valid && demo.net_cost == -120);
    let mut demo_hist = interp
        .build_commit_pool(&mut world, &demo)
        .unwrap()
        .expect("commit pool demolish");
    assert_eq!(world.budget.funds, funds_before_demolish + 120);
    assert!(!is_pool_tile(&world, 10, 8));
    assert_eq!(world.arrays[&9].data[cell(9, 8)], 0x06);

    // Undo demolish -> Undo expand -> Undo dig -> pool completely gone & no pool room!
    assert!(interp.build_undo_entry(&mut world, &mut demo_hist).unwrap());
    assert!(is_pool_tile(&world, 10, 8));
    assert!(
        interp
            .build_undo_entry(&mut world, &mut expand_hist)
            .unwrap()
    );
    assert!(!is_pool_tile(&world, 10, 8));
    assert!(interp.build_undo_entry(&mut world, &mut dig_hist).unwrap());
    assert!(!is_pool_tile(&world, 8, 8));
    assert!(
        !world.rooms.rooms.values().any(|r| r.pool),
        "undoing pool dig must remove pool room"
    );

    // Redo dig restores pool room.
    assert!(interp.build_redo_entry(&mut world, &mut dig_hist).unwrap());
    assert!(is_pool_tile(&world, 8, 8));
    assert!(world.rooms.rooms.values().any(|r| r.pool));
}

#[test]
fn roof_pattern_pitch_switching_house_view_parsing_and_undo_redo() {
    let (program, catalog) = empty_interpreter();
    let interp = Interpreter::new(&program, &catalog, Stream::Global);
    let mut world = synthetic_world(20, 500, 0);

    // Default roof state without touching world.arrays.
    assert!(!world.arrays.contains_key(&ROOF_META_ARRAY));
    assert_eq!(
        roof_state(&world),
        RoofState {
            pattern: DEFAULT_ROOF_PATTERN.to_owned(),
            pitch: RoofPitch::Medium,
        }
    );

    // No-op, empty stem, and non-ASCII UTF-8 stem safety.
    assert!(!plan_roof_pattern(&world, "r1_").valid);
    assert!(!plan_roof_pattern(&world, "   ").valid);
    assert!(!plan_roof_pitch(&world, RoofPitch::Medium).valid);
    assert_eq!(plan_roof_pattern(&world, "café").after.pattern, "café");
    assert_eq!(plan_roof_pattern(&world, "café.BMP").after.pattern, "café");

    // Switch pattern (with .bmp suffix normalized) -> $0 cost.
    let pat_plan = plan_roof_pattern(&world, "MMSuburban2Roof.bmp");
    assert!(pat_plan.valid);
    let mut preview = world.clone();
    preview_roof_on_world(&mut preview, &pat_plan);
    assert_eq!(roof_state(&preview).pattern, "MMSuburban2Roof");

    let mut pat_hist = interp
        .build_commit_roof(&mut world, &pat_plan)
        .unwrap()
        .expect("commit roof pattern");
    assert_eq!(pat_hist.net_cost(), 0);
    assert_eq!(world.budget.funds, 500);
    assert_eq!(roof_state(&world).pattern, "MMSuburban2Roof");
    assert_eq!(roof_state(&world).pitch, RoofPitch::Medium);

    // Switch pitch -> Steep (1.0).
    let pitch_plan = plan_roof_pitch(&world, RoofPitch::Steep);
    assert!(pitch_plan.valid);
    let mut pitch_hist = interp
        .build_commit_roof(&mut world, &pitch_plan)
        .unwrap()
        .expect("commit roof pitch");
    assert_eq!(roof_state(&world).pitch, RoofPitch::Steep);
    assert_eq!(roof_state(&world).pitch.as_f32(), 1.0);

    // Combined edit -> Flat + gravel1.
    let both = plan_roof_edit(&world, Some("gravel1"), Some(RoofPitch::Flat));
    assert!(both.valid);
    let mut both_hist = interp
        .build_commit_roof(&mut world, &both)
        .unwrap()
        .expect("commit combined roof edit");
    assert_eq!(roof_state(&world).pattern, "gravel1");
    assert_eq!(roof_state(&world).pitch, RoofPitch::Flat);

    // Undo all 3 -> ROOF_META_ARRAY removed completely!
    assert!(interp.build_undo_entry(&mut world, &mut both_hist).unwrap());
    assert_eq!(roof_state(&world).pattern, "MMSuburban2Roof");
    assert_eq!(roof_state(&world).pitch, RoofPitch::Steep);
    assert!(
        interp
            .build_undo_entry(&mut world, &mut pitch_hist)
            .unwrap()
    );
    assert_eq!(roof_state(&world).pitch, RoofPitch::Medium);
    assert!(interp.build_undo_entry(&mut world, &mut pat_hist).unwrap());
    assert!(!world.arrays.contains_key(&ROOF_META_ARRAY));
    assert_eq!(roof_state(&world).pattern, DEFAULT_ROOF_PATTERN);

    // Redo restores pattern + pitch.
    assert!(interp.build_redo_entry(&mut world, &mut pat_hist).unwrap());
    assert!(
        interp
            .build_redo_entry(&mut world, &mut pitch_hist)
            .unwrap()
    );
    assert_eq!(roof_state(&world).pattern, "MMSuburban2Roof");
    assert_eq!(roof_state(&world).pitch, RoofPitch::Steep);

    // Test HouseView native pitch (`unknown_word` f32 bits) + stem parsing.
    let view_steep = sims_formats::lot::HouseView {
        version: 62,
        fields: [0; 4],
        camera_direction: 0,
        unknown_shorts: [0; 2],
        unknown_word: 1.0f32.to_bits(),
        roof: "r4_".to_owned(),
        tail: Vec::new(),
    };
    assert_eq!(
        RoofState::from_house_view(&view_steep),
        RoofState {
            pattern: "r4_".to_owned(),
            pitch: RoofPitch::Steep,
        }
    );
}

#[test]
fn live_goth_lot_terrain_water_pool_roof_undo_redo_and_save_restore() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        return;
    };
    let house_path = root.join("UserData/Houses/House05.iff");
    let frames = Frames {
        after: Vec::new(),
        every: Vec::new(),
    };
    let mut lot = Lot::load(&root, &house_path, &[], frames.clone()).expect("load Goth house");
    let prices = ToolPrices::default();
    let initial_funds = lot.world().budget.funds;
    let initial_arch = arch_spend(lot.world());

    // Find a clean 4x4 open flat backyard block (away from walls/objects/roads).
    let mut anchor = None;
    let size = usize::try_from(lot.world().size).unwrap();
    'search: for x in 4..size.saturating_sub(8) {
        for y in 4..size.saturating_sub(8) {
            let ok_pool = plan_pool_edit(
                lot.world(),
                [x as i32, y as i32],
                [x as i32 + 1, y as i32 + 1],
                PoolMode::Dig,
                &prices,
            )
            .valid;
            let ok_water = plan_water_edit(
                lot.world(),
                [x as i32 + 3, y as i32],
                [x as i32 + 4, y as i32 + 1],
                WaterMode::Paint,
                &prices,
            )
            .valid;
            let ok_terr = is_terrain_vertex_editable(lot.world(), x + 4, y + 4);
            if ok_pool && ok_water && ok_terr {
                anchor = Some((x, y));
                break 'search;
            }
        }
    }
    let (ax, ay) = anchor.expect("Goth backyard must have an open flat area");
    let tx = (ax + 4) as i32;
    let ty = (ay + 4) as i32;
    let initial_h = terrain_vertex_height(lot.world(), ax + 4, ay + 4).unwrap();

    // 1. Raise terrain (+$1)
    let raise_plan =
        plan_terrain_edit(lot.world(), [tx, ty], [tx, ty], TerrainMode::Raise, &prices);
    assert!(lot.build_commit_terrain(&raise_plan).unwrap());
    assert_eq!(
        terrain_vertex_height(lot.world(), ax + 4, ay + 4),
        Some(initial_h + 1)
    );

    // 2. Restore grass on disturbed tile (+$1)
    let grass_plan = plan_terrain_edit(
        lot.world(),
        [tx, ty],
        [tx, ty],
        TerrainMode::RestoreGrass,
        &prices,
    );
    assert!(lot.build_commit_terrain(&grass_plan).unwrap());
    assert_eq!(
        lot.world().arrays[&GRASS_ARRAY].data[cell(ax + 4, ay + 4)] as i8,
        GRASS_RESTORED_VALUE
    );

    // 3. Paint 2x2 pond (+$60)
    let pond_plan = plan_water_edit(
        lot.world(),
        [ax as i32 + 3, ay as i32],
        [ax as i32 + 4, ay as i32 + 1],
        WaterMode::Paint,
        &prices,
    );
    assert!(lot.build_commit_water(&pond_plan).unwrap());
    assert!(is_water_tile(lot.world(), ax + 3, ay));

    // 4. Dig 2x2 swimming pool (+$300)
    let pool_plan = plan_pool_edit(
        lot.world(),
        [ax as i32, ay as i32],
        [ax as i32 + 1, ay as i32 + 1],
        PoolMode::Dig,
        &prices,
    );
    assert!(lot.build_commit_pool(&pool_plan).unwrap());
    assert!(is_pool_tile(lot.world(), ax, ay));
    assert!(lot.world().rooms.rooms.values().any(|r| r.pool));

    // 5. Switch roof pitch preserving House05's HOUS 0 baseline (`r3_`, `Steep`),
    // then switch pattern to `r1_` (must not be treated as a no-op!).
    let goth_house = sims_content::House::load(&house_path, lot.catalog()).expect("parse House05");
    let goth_base = RoofState::from_house_view(goth_house.view.as_ref().expect("House05 HOUS 0"));
    assert_eq!(
        goth_base,
        RoofState {
            pattern: "r3_".to_owned(),
            pitch: RoofPitch::Steep,
        }
    );
    assert_eq!(lot.roof_state_with_base(&goth_base), goth_base);
    let roof_pitch_plan = plan_roof_pitch_with_base(lot.world(), &goth_base, RoofPitch::Shallow);
    assert!(roof_pitch_plan.valid);
    assert_eq!(roof_pitch_plan.after.pattern, "r3_");
    assert!(lot.build_commit_roof(&roof_pitch_plan).unwrap());
    assert_eq!(
        lot.roof_state_with_base(&goth_base),
        RoofState {
            pattern: "r3_".to_owned(),
            pitch: RoofPitch::Shallow,
        }
    );
    let roof_pat_plan = plan_roof_pattern_with_base(lot.world(), &goth_base, "r1_");
    assert!(roof_pat_plan.valid);
    assert!(lot.build_commit_roof(&roof_pat_plan).unwrap());
    assert_eq!(
        lot.roof_state_with_base(&goth_base),
        RoofState {
            pattern: "r1_".to_owned(),
            pitch: RoofPitch::Shallow,
        }
    );

    // Check exact Architecture budget accounting (+1 + 1 + 60 + 300 = +362).
    assert_eq!(lot.world().budget.funds, initial_funds - 362);
    assert_eq!(arch_spend(lot.world()), initial_arch + 362);

    // Undo all 6 steps in chronological reverse order!
    for _ in 0..6 {
        assert!(lot.undo().unwrap());
    }
    assert!(!lot.can_undo());
    assert!(lot.can_redo());
    assert_eq!(lot.world().budget.funds, initial_funds);
    assert_eq!(arch_spend(lot.world()), initial_arch);
    assert_eq!(
        terrain_vertex_height(lot.world(), ax + 4, ay + 4),
        Some(initial_h)
    );
    assert!(!is_water_tile(lot.world(), ax + 3, ay));
    assert!(!is_pool_tile(lot.world(), ax, ay));
    assert!(!lot.world().arrays.contains_key(&ROOF_META_ARRAY));
    assert_eq!(lot.roof_state_with_base(&goth_base), goth_base);

    // Redo all 6 steps!
    for _ in 0..6 {
        assert!(lot.redo().unwrap());
    }
    assert!(!lot.can_redo());
    assert_eq!(lot.world().budget.funds, initial_funds - 362);
    assert_eq!(lot.roof_state_with_base(&goth_base).pattern, "r1_");
    assert_eq!(
        lot.roof_state_with_base(&goth_base).pitch,
        RoofPitch::Shallow
    );
    assert!(is_pool_tile(lot.world(), ax, ay));
    assert!(is_water_tile(lot.world(), ax + 3, ay));

    // Save -> encode -> decode -> restore into a fresh Goth Lot!
    let encoded = lot.save().expect("save lot").encode();
    let decoded = Saved::decode(&encoded).expect("decode saved lot");
    let mut restored = Lot::load(&root, &house_path, &[], frames).expect("reload Goth");
    restored.restore(decoded).expect("restore saved lot");
    assert_eq!(restored.world().budget.funds, initial_funds - 362);
    assert_eq!(
        terrain_vertex_height(restored.world(), ax + 4, ay + 4),
        Some(initial_h + 1)
    );
    assert!(is_water_tile(restored.world(), ax + 3, ay));
    assert!(is_pool_tile(restored.world(), ax, ay));
    assert!(restored.world().rooms.rooms.values().any(|r| r.pool));
    assert_eq!(
        restored.roof_state_with_base(&goth_base),
        RoofState {
            pattern: "r1_".to_owned(),
            pitch: RoofPitch::Shallow,
        }
    );

    // Verify native HOUS 0 roof parsing across real Complete Collection houses.
    let install = Install::detect(&root).expect("detect install");
    for (rel, expected_pitch, expected_pat) in [
        ("UserData/Houses/House01.iff", RoofPitch::Medium, "r1_"),
        ("UserData/Houses/House02.iff", RoofPitch::Steep, "r4_"),
        ("UserData/Houses/House05.iff", RoofPitch::Steep, "r3_"),
        ("UserData/Houses/House07.iff", RoofPitch::Shallow, "r1_"),
    ] {
        let p = install.file(rel).expect("house file");
        let h = sims_content::House::load(&p, lot.catalog()).expect("parse house");
        let view = h.view.as_ref().expect("HOUS 0");
        assert_eq!(
            RoofState::from_house_view(view),
            RoofState {
                pattern: expected_pat.to_owned(),
                pitch: expected_pitch,
            },
            "{rel} HOUS 0 roof mismatch"
        );
    }
}

#[test]
fn terrain_level_clamps_asymmetric_fixed_neighbors() {
    let (program, catalog) = empty_interpreter();
    let interp = Interpreter::new(&program, &catalog, Stream::Global);
    let prices = ToolPrices::default();
    let mut clamp_world = synthetic_world(16, 500, 10);
    for vx in 0..=16usize {
        let h = if vx <= 4 {
            15u8
        } else if vx >= 8 {
            5u8
        } else {
            10u8
        };
        for vy in 0..=16usize {
            if vx > 0 && vy > 0 {
                clamp_world.arrays.get_mut(&0).unwrap().data[cell(vx - 1, vy - 1) * 4 + 3] = h;
            }
            if vx < 16 && vy > 0 {
                clamp_world.arrays.get_mut(&0).unwrap().data[cell(vx, vy - 1) * 4] = h;
            }
            if vx > 0 && vy < 16 {
                clamp_world.arrays.get_mut(&0).unwrap().data[cell(vx - 1, vy) * 4 + 2] = h;
            }
            if vx < 16 && vy < 16 {
                clamp_world.arrays.get_mut(&0).unwrap().data[cell(vx, vy) * 4 + 1] = h;
            }
        }
    }
    let clamped = plan_terrain_edit(&clamp_world, [4, 6], [7, 6], TerrainMode::Level, &prices);
    assert!(clamped.valid);
    interp
        .build_commit_terrain(&mut clamp_world, &clamped)
        .unwrap()
        .unwrap();
    let h5 = terrain_vertex_height(&clamp_world, 5, 6).unwrap();
    let h6 = terrain_vertex_height(&clamp_world, 6, 6).unwrap();
    let h7 = terrain_vertex_height(&clamp_world, 7, 6).unwrap();
    assert!(
        h7 <= 10,
        "vertex bordering fixed height 5 must clamp to <= 10, got {h7}"
    );
    assert!(h5.abs_diff(h6) <= MAX_TERRAIN_SLOPE);
    assert!(h6.abs_diff(h7) <= MAX_TERRAIN_SLOPE);
}

#[test]
fn undo_and_redo_enforce_live_terrain_pond_and_pool_guards() {
    let (program, catalog) = empty_interpreter();
    let interp = Interpreter::new(&program, &catalog, Stream::Global);
    let prices = ToolPrices::default();
    let mut world = synthetic_world(20, 1000, 3);

    // 1. Pool demolish -> Sim moves onto dry tile -> Undo MUST refuse without changing world/funds!
    let dig = plan_pool_edit(&world, [6, 6], [7, 7], PoolMode::Dig, &prices);
    let mut dig_hist = interp.build_commit_pool(&mut world, &dig).unwrap().unwrap();
    let demo = plan_pool_edit(&world, [6, 6], [7, 7], PoolMode::Demolish, &prices);
    let mut demo_hist = interp
        .build_commit_pool(&mut world, &demo)
        .unwrap()
        .unwrap();
    let funds_after_demo = world.budget.funds;
    place_dummy_object(&mut world, 50, [6, 6], 1);
    let snapshot_with_sim = world.clone();
    assert!(
        !interp.build_undo_entry(&mut world, &mut demo_hist).unwrap(),
        "undoing pool demolition under an occupant must be refused"
    );
    assert_eq!(world.budget.funds, funds_after_demo);
    assert!(!is_pool_tile(&world, 6, 6));
    assert_eq!(world.arrays[&9].data, snapshot_with_sim.arrays[&9].data);

    // Removing the occupant allows Undo to succeed.
    world.objects.remove(&50);
    assert!(interp.build_undo_entry(&mut world, &mut demo_hist).unwrap());
    assert!(is_pool_tile(&world, 6, 6));

    // Undo pool dig, then place a floor / lock / object -> Redo pool dig MUST refuse!
    assert!(interp.build_undo_entry(&mut world, &mut dig_hist).unwrap());
    let foff = cell(6, 6) * 2;
    world.arrays.get_mut(&FLOORS[0]).unwrap().data[foff..foff + 2]
        .copy_from_slice(&1u16.to_le_bytes());
    let funds_before_redo = world.budget.funds;
    assert!(
        !interp.build_redo_entry(&mut world, &mut dig_hist).unwrap(),
        "redoing pool dig over a newly laid floor must be refused"
    );
    assert_eq!(world.budget.funds, funds_before_redo);
    assert!(!is_pool_tile(&world, 6, 6));
    world.arrays.get_mut(&FLOORS[0]).unwrap().data[foff..foff + 2].copy_from_slice(&[0, 0]);
    world.arrays.get_mut(&8).unwrap().data[cell(6, 6)] |= FLAG_LOCKED;
    assert!(
        !interp.build_redo_entry(&mut world, &mut dig_hist).unwrap(),
        "redoing pool dig over a locked tile must be refused"
    );
    world.arrays.get_mut(&8).unwrap().data[cell(6, 6)] &= !FLAG_LOCKED;
    assert!(interp.build_redo_entry(&mut world, &mut dig_hist).unwrap());
    assert!(is_pool_tile(&world, 6, 6));

    // 2. Terrain Undo/Redo refused if an incident tile gains an object.
    let raise = plan_terrain_edit(&world, [12, 12], [12, 12], TerrainMode::Raise, &prices);
    let mut raise_hist = interp
        .build_commit_terrain(&mut world, &raise)
        .unwrap()
        .unwrap();
    place_dummy_object(&mut world, 51, [12, 12], 1);
    assert!(
        !interp
            .build_undo_entry(&mut world, &mut raise_hist)
            .unwrap(),
        "undoing terrain raise under an object must be refused"
    );
    assert_eq!(terrain_vertex_height(&world, 12, 12), Some(4));
    world.objects.remove(&51);
    assert!(
        interp
            .build_undo_entry(&mut world, &mut raise_hist)
            .unwrap()
    );
    assert_eq!(terrain_vertex_height(&world, 12, 12), Some(3));

    // 3. Pond Paint Undo refused if a tile becomes locked or occupied.
    let pond = plan_water_edit(&world, [15, 15], [15, 15], WaterMode::Paint, &prices);
    let mut pond_hist = interp
        .build_commit_water(&mut world, &pond)
        .unwrap()
        .unwrap();
    world.arrays.get_mut(&8).unwrap().data[cell(15, 15)] |= FLAG_LOCKED;
    assert!(
        !interp.build_undo_entry(&mut world, &mut pond_hist).unwrap(),
        "undoing pond paint on a locked tile must be refused"
    );
    assert!(is_water_tile(&world, 15, 15));
    world.arrays.get_mut(&8).unwrap().data[cell(15, 15)] &= !FLAG_LOCKED;
    assert!(interp.build_undo_entry(&mut world, &mut pond_hist).unwrap());
    assert!(!is_water_tile(&world, 15, 15));
}

#[test]
fn roof_baseline_preserves_untouched_hous0_fields_and_avoids_double_strip() {
    let (program, catalog) = empty_interpreter();
    let interp = Interpreter::new(&program, &catalog, Stream::Global);
    let mut world = synthetic_world(16, 500, 0);
    let base = RoofState {
        pattern: "r3_".to_owned(),
        pitch: RoofPitch::Steep,
    };

    assert_eq!(roof_state_with_base(&world, &base), base);
    assert!(!world.arrays.contains_key(&ROOF_META_ARRAY));

    // Pitch-only edit preserves `r3_` from `HOUS 0` baseline.
    let pitch_only = plan_roof_pitch_with_base(&world, &base, RoofPitch::Shallow);
    assert!(pitch_only.valid);
    assert_eq!(pitch_only.before, base);
    assert_eq!(pitch_only.after.pattern, "r3_");
    assert_eq!(pitch_only.after.pitch, RoofPitch::Shallow);
    let mut pitch_hist = interp
        .build_commit_roof(&mut world, &pitch_only)
        .unwrap()
        .unwrap();
    assert_eq!(roof_state_with_base(&world, &base).pattern, "r3_");
    assert_eq!(
        roof_state_with_base(&world, &base).pitch,
        RoofPitch::Shallow
    );

    // Undo removes `ROOF_META_ARRAY` and resolves back to `HOUS 0` baseline.
    assert!(
        interp
            .build_undo_entry(&mut world, &mut pitch_hist)
            .unwrap()
    );
    assert!(!world.arrays.contains_key(&ROOF_META_ARRAY));
    assert_eq!(roof_state_with_base(&world, &base), base);

    // Selecting `r1_` on a house whose baseline is `r3_` is valid (not a no-op).
    let select_r1 = plan_roof_pattern_with_base(&world, &base, "r1_");
    assert!(select_r1.valid);
    assert_eq!(select_r1.after.pattern, "r1_");
    assert_eq!(select_r1.after.pitch, RoofPitch::Steep);

    // Normalization strips `.bmp` once: `tile.bmp.bmp` -> `tile.bmp` and reads back identically.
    let dbl = plan_roof_edit_with_base(&world, &base, Some("tile.bmp.bmp"), None);
    assert!(dbl.valid);
    assert_eq!(dbl.after.pattern, "tile.bmp");
    interp.build_commit_roof(&mut world, &dbl).unwrap().unwrap();
    assert_eq!(roof_state_with_base(&world, &base).pattern, "tile.bmp");
}

#[test]
fn leveling_preexisting_cliff_never_creates_new_slope_above_five() {
    let prices = ToolPrices::default();
    let set_v = |world: &mut World, vx: usize, vy: usize, h: u8| {
        if vx > 0 && vy > 0 {
            world.arrays.get_mut(&0).unwrap().data[cell(vx - 1, vy - 1) * 4 + 3] = h;
        }
        if vx < 64 && vy > 0 {
            world.arrays.get_mut(&0).unwrap().data[cell(vx, vy - 1) * 4] = h;
        }
        if vx > 0 && vy < 64 {
            world.arrays.get_mut(&0).unwrap().data[cell(vx - 1, vy) * 4 + 2] = h;
        }
        if vx < 64 && vy < 64 {
            world.arrays.get_mut(&0).unwrap().data[cell(vx, vy) * 4 + 1] = h;
        }
    };
    let check_mode = |mode: TerrainMode| {
        let (program, catalog) = empty_interpreter();
        let interp = Interpreter::new(&program, &catalog, Stream::Global);
        let mut world = synthetic_world(16, 500, 15);
        // Set up Sol's fixture: three editable vertices along y=6 at x=5,6,7
        // with heights [0, 25, 25], and fixed neighbors at y=5/y=7 holding 15
        // and x=4 holding 0, x=8 holding 20 so old internal slope on (5,6)-(6,6)
        // is 25 and (6,6)-(7,6) is 0.
        for vx in 0..=16usize {
            for vy in 0..=16usize {
                let h = if vy == 6 && vx == 5 {
                    0
                } else if vy == 6 && (vx == 6 || vx == 7) {
                    25
                } else if vy == 6 && vx == 4 {
                    0
                } else if vy == 6 && vx == 8 {
                    20
                } else {
                    15
                };
                set_v(&mut world, vx, vy, h);
            }
        }
        // Snapshot every original orthogonal edge difference in the area.
        let mut orig = [[0u8; 17]; 17];
        for (vx, col) in orig.iter_mut().enumerate() {
            for (vy, slot) in col.iter_mut().enumerate() {
                *slot = terrain_vertex_height(&world, vx, vy).unwrap();
            }
        }
        let plan = plan_terrain_edit(&world, [5, 6], [7, 6], mode, &prices);
        if plan.valid {
            interp
                .build_commit_terrain(&mut world, &plan)
                .unwrap()
                .unwrap();
        }
        for vx in 4..=8usize {
            for vy in 5..=7usize {
                let h = terrain_vertex_height(&world, vx, vy).unwrap();
                for (nx, ny) in [(vx + 1, vy), (vx, vy + 1)] {
                    if nx <= 8 && ny <= 7 {
                        let nh = terrain_vertex_height(&world, nx, ny).unwrap();
                        let old_d = orig[vx][vy].abs_diff(orig[nx][ny]);
                        let new_d = h.abs_diff(nh);
                        assert!(
                            new_d <= MAX_TERRAIN_SLOPE.max(old_d),
                            "{mode:?} steepened edge ({vx},{vy})-({nx},{ny}) from {old_d} to {new_d}"
                        );
                    }
                }
            }
        }
    };
    check_mode(TerrainMode::Level);
    check_mode(TerrainMode::LevelAverage);
}

#[test]
fn stale_adjacent_pool_pond_plans_rejected_and_undo_preserves_distant_edits() {
    let (program, catalog) = empty_interpreter();
    let interp = Interpreter::new(&program, &catalog, Stream::Global);
    let prices = ToolPrices::default();
    let mut world = synthetic_world(20, 1000, 2);

    // Preserve a distant `0x10` sentinel in Arry 9 across edits.
    world.arrays.get_mut(&9).unwrap().data[cell(14, 14)] = 0x10;

    // Plan two adjacent 1-tile pool digs on an existing dry array.
    let p1 = plan_pool_edit(&world, [5, 5], [5, 5], PoolMode::Dig, &prices);
    let p2 = plan_pool_edit(&world, [6, 5], [6, 5], PoolMode::Dig, &prices);
    assert!(p1.valid && p2.valid);
    assert!(interp.build_commit_pool(&mut world, &p1).unwrap().is_some());
    assert_eq!(world.arrays[&9].data[cell(14, 14)], 0x10);
    // Committing stale p2 must be rejected so disconnected `0x00` masks cannot form!
    assert!(
        interp.build_commit_pool(&mut world, &p2).unwrap().is_none(),
        "stale adjacent pool plan must be rejected to prevent disconnected masks"
    );
    assert!(!is_pool_tile(&world, 6, 5));

    // Same stale adjacent check for Pond/Water.
    let w1 = plan_water_edit(&world, [10, 5], [10, 5], WaterMode::Paint, &prices);
    let w2 = plan_water_edit(&world, [11, 5], [11, 5], WaterMode::Paint, &prices);
    assert!(w1.valid && w2.valid);
    assert!(
        interp
            .build_commit_water(&mut world, &w1)
            .unwrap()
            .is_some()
    );
    assert!(
        interp
            .build_commit_water(&mut world, &w2)
            .unwrap()
            .is_none(),
        "stale adjacent pond plan must be rejected to prevent disconnected masks"
    );
    assert!(!is_water_tile(&world, 11, 5));

    // Array creation Undo refuses if a distant tile or `0x10` sentinel was added later.
    world.arrays.remove(&9);
    let create_pool = plan_pool_edit(&world, [5, 10], [5, 10], PoolMode::Dig, &prices);
    assert!(create_pool.valid && create_pool.created_pool_array);
    let mut create_hist = interp
        .build_commit_pool(&mut world, &create_pool)
        .unwrap()
        .unwrap();
    world.arrays.get_mut(&9).unwrap().data[cell(14, 14)] = 0x10;
    assert!(
        !interp
            .build_undo_entry(&mut world, &mut create_hist)
            .unwrap(),
        "undoing array-creation entry must refuse when a distant 0x10 sentinel exists"
    );
    assert_eq!(world.arrays[&9].data[cell(14, 14)], 0x10);
    world.arrays.get_mut(&9).unwrap().data[cell(14, 14)] = 0xff;
    world.arrays.get_mut(&9).unwrap().data[cell(12, 12)] = 0x00;
    assert!(
        !interp
            .build_undo_entry(&mut world, &mut create_hist)
            .unwrap(),
        "undoing array-creation entry must refuse when a distant pool tile exists"
    );
    assert!(is_pool_tile(&world, 12, 12));
    world.arrays.get_mut(&9).unwrap().data[cell(12, 12)] = 0xff;
    assert!(
        interp
            .build_undo_entry(&mut world, &mut create_hist)
            .unwrap()
    );
    assert!(!world.arrays.contains_key(&9));
}

#[test]
fn removing_entire_pond_fills_center_back_to_dry_bank() {
    let (program, catalog) = empty_interpreter();
    let interp = Interpreter::new(&program, &catalog, Stream::Global);
    let prices = ToolPrices::default();
    let mut world = synthetic_world(20, 500, 3);

    // Paint a 2x2 pond at (5..=6, 5..=6) on flat height 3 -> all 9 vertices drop to 2.
    let paint = plan_water_edit(&world, [5, 5], [6, 6], WaterMode::Paint, &prices);
    assert!(paint.valid);
    interp
        .build_commit_water(&mut world, &paint)
        .unwrap()
        .unwrap();
    for vx in 5..=7 {
        for vy in 5..=7 {
            assert_eq!(terrain_vertex_height(&world, vx, vy), Some(2));
        }
    }

    // Ordinary Remove of all 4 tiles restores EVERY vertex including center (6, 6) to 3!
    let remove_all = plan_water_edit(&world, [5, 5], [6, 6], WaterMode::Remove, &prices);
    assert!(remove_all.valid);
    interp
        .build_commit_water(&mut world, &remove_all)
        .unwrap()
        .unwrap();
    for vx in 5..=7 {
        for vy in 5..=7 {
            assert_eq!(
                terrain_vertex_height(&world, vx, vy),
                Some(3),
                "vertex ({vx}, {vy}) should refill back to surrounding dry bank height 3"
            );
        }
    }

    // Partial removal pins vertices bordering surviving water at 2 while raising dry outer rim.
    let paint2 = plan_water_edit(&world, [5, 5], [6, 6], WaterMode::Paint, &prices);
    interp
        .build_commit_water(&mut world, &paint2)
        .unwrap()
        .unwrap();
    let remove_half = plan_water_edit(&world, [6, 5], [6, 6], WaterMode::Remove, &prices);
    interp
        .build_commit_water(&mut world, &remove_half)
        .unwrap()
        .unwrap();
    assert_eq!(terrain_vertex_height(&world, 6, 6), Some(2));
    assert_eq!(terrain_vertex_height(&world, 7, 6), Some(3));
}

#[test]
fn grow_grass_allows_undo_commits_are_transactional_and_extreme_coords_never_panic() {
    let (program, catalog) = empty_interpreter();
    let interp = Interpreter::new(&program, &catalog, Stream::Global);
    let prices = ToolPrices::default();
    let mut world = synthetic_world(20, 500, 3);

    // Terrain raise followed by natural `grow_grass` still undoes cleanly.
    let raise = plan_terrain_edit(&world, [10, 10], [10, 10], TerrainMode::Raise, &prices);
    let mut raise_hist = interp
        .build_commit_terrain(&mut world, &raise)
        .unwrap()
        .unwrap();
    world.grow_grass(5).expect("grow grass");
    assert!(
        (world.arrays[&GRASS_ARRAY].data[cell(10, 10)] as i8) > TERRAIN_DIRT_GRASS,
        "grow_grass should advance Arry 6 toward target"
    );
    assert!(
        interp
            .build_undo_entry(&mut world, &mut raise_hist)
            .unwrap(),
        "normal lawn regrowth must not strand terrain undo"
    );
    assert_eq!(terrain_vertex_height(&world, 10, 10), Some(3));
    assert_eq!(world.budget.funds, 500);

    // Pond paint followed by `grow_grass` still undoes cleanly.
    let pond = plan_water_edit(&world, [6, 6], [6, 6], WaterMode::Paint, &prices);
    let mut pond_hist = interp
        .build_commit_water(&mut world, &pond)
        .unwrap()
        .unwrap();
    world.grow_grass(5).expect("grow grass");
    assert!(
        interp.build_undo_entry(&mut world, &mut pond_hist).unwrap(),
        "normal lawn regrowth must not strand pond undo"
    );
    assert!(!is_water_tile(&world, 6, 6));
    assert_eq!(world.budget.funds, 500);

    // Direct interpreter commits stage transactionally: missing budget day faults without mutating world!
    let mut no_budget = synthetic_world(20, 500, 3);
    no_budget.budget.days.clear();
    let before_fault = no_budget.clone();
    let tp = plan_terrain_edit(&no_budget, [8, 8], [8, 8], TerrainMode::Raise, &prices);
    assert!(interp.build_commit_terrain(&mut no_budget, &tp).is_err());
    assert_eq!(no_budget.arrays[&0].data, before_fault.arrays[&0].data);
    let wp = plan_water_edit(&no_budget, [8, 8], [8, 8], WaterMode::Paint, &prices);
    assert!(interp.build_commit_water(&mut no_budget, &wp).is_err());
    assert_eq!(no_budget.arrays[&10].data, before_fault.arrays[&10].data);
    let pp = plan_pool_edit(&no_budget, [8, 8], [8, 8], PoolMode::Dig, &prices);
    assert!(interp.build_commit_pool(&mut no_budget, &pp).is_err());
    assert_eq!(no_budget.arrays[&9].data, before_fault.arrays[&9].data);

    // Extreme i32::MAX / i32::MIN coordinates never panic with overflow checks enabled.
    assert!(
        !plan_terrain_edit(
            &world,
            [i32::MAX, i32::MAX],
            [i32::MAX, i32::MAX],
            TerrainMode::Raise,
            &prices
        )
        .valid
    );
    assert!(
        !plan_water_edit(
            &world,
            [i32::MAX, 5],
            [i32::MAX, 5],
            WaterMode::Paint,
            &prices
        )
        .valid
    );
    assert!(
        !plan_water_edit(
            &world,
            [i32::MIN, i32::MIN],
            [5, 5],
            WaterMode::Paint,
            &prices
        )
        .valid
    );
    assert!(!plan_pool_edit(&world, [5, i32::MAX], [5, i32::MAX], PoolMode::Dig, &prices).valid);
    assert!(!plan_pool_edit(&world, [i32::MIN, 5], [5, 5], PoolMode::Dig, &prices).valid);
}
