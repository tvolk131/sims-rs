//! Bit-for-bit equivalence tests for [`sims_render::LotCache`] against
//! uncached [`sims_render::draw_lot_with_preview`].

use sims_formats::lot::Array;
use sims_render::lot::{FULL, Lamps, NORTH, WEST};
use sims_render::{
    Camera, Canvas, Focus, Lot, LotCache, Selected, Walls, Zoom, draw_lot_with_preview, open_house,
};
use sims_sim::World;
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!("skipping: set SIMS_INSTALL to verify LotCache against House05 art");
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn assert_parity(
    cache: &mut LotCache,
    world: &World,
    lot: &Lot,
    assets: &mut sims_render::Assets,
    camera: &Camera,
    viewport: (u32, u32),
    preview: Option<(u16, u16)>,
    size: [usize; 2],
    label: &str,
) {
    let mut cached = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
    let mut uncached = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
    let stats_cached =
        cache.draw_lot_with_preview(world, lot, assets, camera, viewport, preview, &mut cached);
    let stats_uncached =
        draw_lot_with_preview(world, lot, assets, camera, viewport, preview, &mut uncached);
    assert_eq!(stats_cached, stats_uncached, "stats mismatch at {label}");
    assert_eq!(cached, uncached, "canvas mismatch at {label}");
}

#[test]
fn lot_cache_matches_uncached_across_house05_states_and_mutations() {
    let Some(root) = install() else {
        return;
    };
    let house = root.join("UserData/Houses/House05.iff");
    let mut opened = open_house(&root, &house).expect("open House05");
    let mut world = opened.world;
    let mut lot = Lot::from_world(&world)
        .expect("Lot::from_world")
        .with_floor_rotation(opened.rotation);
    let assets = &mut opened.assets;
    let mut cache = LotCache::new();

    let base_origin = {
        let c = lot.built_center();
        let cam = Camera {
            zoom: Zoom::Medium,
            rotation: 0,
            level: 1,
            origin: [0, 0],
            walls: Walls::Cutaway,
            focus: Focus::default(),
        };
        let p = cam.project_fine(cam.rotate_fine(c), 1, 0);
        [200 - p[0], 150 - p[1]]
    };

    let size = [400, 300];
    let mut camera = Camera {
        zoom: Zoom::Medium,
        rotation: 0,
        level: 1,
        origin: base_origin,
        walls: Walls::Cutaway,
        focus: Focus::default(),
    };

    // 1. Cold draw + repeated steady-camera hit.
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "initial cold",
    );
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "steady hit",
    );

    // 2. Viewport clip_bottom change (expanded live strip) & canvas resize.
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 200),
        None,
        size,
        "strip viewport",
    );
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (320, 240),
        None,
        [320, 240],
        "canvas resize",
    );

    // 3. Camera pan (reusing cached Grass), zooms, rotations, wall modes, levels.
    camera.origin[0] += 24;
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "pan right",
    );
    for zoom in Zoom::ALL {
        for rotation in 0..4 {
            for walls in [Walls::Up, Walls::Cutaway, Walls::Down] {
                camera.zoom = zoom;
                camera.rotation = rotation;
                camera.walls = walls;
                assert_parity(
                    &mut cache,
                    &world,
                    &lot,
                    assets,
                    &camera,
                    (400, 300),
                    None,
                    size,
                    &format!("{zoom:?} r{rotation} {walls:?}"),
                );
            }
        }
    }
    camera.zoom = Zoom::Medium;
    camera.rotation = 0;
    camera.walls = Walls::Cutaway;
    camera.level = 2;
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "level 2",
    );
    camera.level = 1;

    // 4. Cutaway focus changes (pointer moves within same room, different rooms, selected Sim).
    let snap = camera;
    camera.focus.point(&lot, &snap, [200, 150]);
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "cutaway hover 1",
    );
    let snap = camera;
    camera.focus.point(&lot, &snap, [201, 150]);
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "cutaway hover same room",
    );
    let snap = camera;
    camera.focus.point(&lot, &snap, [120, 110]);
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "cutaway hover other room",
    );
    camera.focus.selected = Some(Selected {
        tile: [25, 25],
        level: 1,
        room: 1,
    });
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "cutaway selected sim",
    );

    // 5. Day / dusk / night / dawn transitions and room lighting + lamp pools.
    if world.globals.len() < 20 {
        world.globals.resize(20, 0);
    }
    for tod in [0, 1, 2, 3] {
        world.globals[4] = tod;
        lot.time_of_day = tod;
        assert_parity(
            &mut cache,
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            size,
            &format!("time_of_day {tod}"),
        );
    }
    world.globals[4] = 2;
    lot.time_of_day = 2;
    for v in lot.room_lights.values_mut() {
        *v = (*v).saturating_sub(80);
    }
    for v in lot.room_fractions.values_mut() {
        *v *= 0.6;
    }
    let lamp_cell = Lot::cell(25, 25);
    lot.lamps[0][lamp_cell] = Lamps([Some([0, 0]), None]);
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "room lighting + lamp pool change",
    );

    // 6. Floor, wall, terrain height, and grass edits.
    let edit_cell = Lot::cell(26, 26);
    lot.levels[0].floors[edit_cell] = 1;
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "floor edit",
    );
    lot.levels[0].walls[edit_cell].segments |= WEST | NORTH;
    lot.levels[0].walls[edit_cell].styles = [1, 1];
    lot.levels[0].walls[edit_cell].patterns = [2, 2, 2, 2];
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "wall edit",
    );
    lot.heights[edit_cell] = [2, 2, 2, 2];
    lot.grass[edit_cell] = 80;
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "terrain + grass edit",
    );

    // 7. Buy Mode terrain grid toggle and buildable-flags array 8 edit.
    world.globals[19] = 2;
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "buy mode grid on",
    );
    world
        .arrays
        .entry(8)
        .or_insert_with(|| Array {
            width: FULL,
            height: FULL,
            stride: 1,
            encoding: 0,
            data: vec![0; FULL * FULL],
        })
        .data[Lot::cell(24, 24)] ^= 0x20;
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "buy mode grid array 8 edit",
    );
    world.globals[19] = 0;

    // 8. Object mutations: graphic frame, facing, hidden, glow light, dynamic sprite,
    //    position, shadow-caster toggle, and Buy Mode preview override.
    let target_id = *world
        .objects
        .iter()
        .find(|(_, o)| o.person.is_none() && o.position[2] == 1 && o.position[0] > 0)
        .expect("placed non-person object")
        .0;
    let obj = world.objects.get_mut(&target_id).unwrap();
    if obj.data.len() <= 51 {
        obj.data.resize(52, 0);
    }
    obj.data[0] = obj.data[0].wrapping_add(1);
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "object graphic change",
    );
    let obj = world.objects.get_mut(&target_id).unwrap();
    obj.data[1] = (obj.data[1] + 2) & 7;
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "object facing change",
    );
    let obj = world.objects.get_mut(&target_id).unwrap();
    obj.data[51] = if obj.data[51] == 0 { 100 } else { 0 };
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "object glow light change",
    );
    let obj = world.objects.get_mut(&target_id).unwrap();
    obj.data[34] = 1;
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "object hidden toggle",
    );
    let obj = world.objects.get_mut(&target_id).unwrap();
    obj.data[34] = 0;
    obj.position[0] += 16;
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "object moved",
    );

    // Simulate a Buy Mode preview pairing two objects.
    let second_id = *world
        .objects
        .iter()
        .find(|(id, o)| {
            **id != target_id && o.person.is_none() && o.position[2] == 1 && o.position[0] > 0
        })
        .expect("second object")
        .0;
    world.objects.get_mut(&second_id).unwrap().position = [-16, -16, 1];
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        Some((target_id, second_id)),
        size,
        "buy mode preview active",
    );
    world.objects.get_mut(&target_id).unwrap().position[1] += 16;
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        Some((target_id, second_id)),
        size,
        "buy mode preview moved",
    );

    // 9. Shadow-chain `slots[0].object` mutation (same array-3 head, changed
    //    linked-list successor) and `holds_person` tree change.
    let mut shadow_head_id = None;
    if let Some(grid) = world.arrays.get(&3) {
        for x in 0..lot.size {
            for y in 0..lot.size {
                let cell = Lot::cell(x, y);
                let head = i16::from_le_bytes([grid.data[cell * 2], grid.data[cell * 2 + 1]]);
                if head > 0
                    && let Some(o) = world.objects.get(&(head as u16))
                    && !o.slots.is_empty()
                {
                    shadow_head_id = Some(head as u16);
                    break;
                }
            }
            if shadow_head_id.is_some() {
                break;
            }
        }
    }
    if let Some(head_id) = shadow_head_id {
        assert_parity(
            &mut cache,
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            size,
            "before shadow chain edit",
        );
        // Hide the head object so `shadow_of_chain` follows `slots[0].object`,
        // then mutate `slots[0].object` while keeping array 3 untouched.
        if let Some(o) = world.objects.get_mut(&head_id) {
            if o.data.len() <= 34 {
                o.data.resize(35, 0);
            }
            o.data[34] = 1;
        }
        assert_parity(
            &mut cache,
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            size,
            "shadow head hidden",
        );
        if let Some(o) = world.objects.get_mut(&head_id) {
            o.slots[0].object = target_id as i16;
        }
        assert_parity(
            &mut cache,
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            size,
            "shadow chain successor changed with same array-3 head",
        );
    }

    // Mutate `holds_person` by linking a non-person object to hold a person.
    if let Some(&person_id) = world
        .objects
        .iter()
        .find(|(_, o)| o.person.is_some())
        .map(|(id, _)| id)
    {
        if let Some(holder) = world.objects.get_mut(&target_id) {
            if holder.slots.len() < 2 {
                holder.slots.resize(
                    2,
                    sims_formats::saved_objects::Slot {
                        unknown: 0,
                        object: 0,
                    },
                );
            }
            holder.slots[1].object = person_id as i16;
        }
        if let Some(person) = world.objects.get_mut(&person_id) {
            if person.data.len() <= 26 {
                person.data.resize(27, 0);
            }
            person.data[14] = 1;
            person.data[26] = target_id as i16;
        }
        assert_parity(
            &mut cache,
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            size,
            "holds_person tree added",
        );
    }
}
