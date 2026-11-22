//! Bit-for-bit equivalence tests for [`sims_render::LotCache`] against
//! uncached [`sims_render::draw_lot_with_preview`].

use sims_formats::lot::Array;
use sims_render::lot::{FULL, Lamps, NORTH, WEST};
use sims_render::{
    CacheHit, Camera, Canvas, Focus, Lot, LotCache, RenderOptions, Selected, Walls, Zoom,
    draw_lot_with_preview, draw_lot_with_preview_and_options, open_house,
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

        // 10. Toggling `Object::person` on an already-tracked preview
        //     participant and on a wall-shaping object must invalidate the
        //     cache while keeping `holds_person` fixed and genuinely changing
        //     the rendered pixels and `Stats`.
        let sample_person = world
            .objects
            .get(&person_id)
            .and_then(|o| o.person.clone())
            .expect("person data");
        // Undo the synthetic `holds_person` link from step 9 so `target_id`
        // and `person_id` stay clean.
        if let Some(holder) = world.objects.get_mut(&target_id)
            && holder.slots.len() >= 2
        {
            holder.slots[1].object = 0;
        }
        if let Some(person) = world.objects.get_mut(&person_id)
            && person.data.len() > 26
        {
            person.data[14] = 0;
            person.data[26] = 0;
        }

        // Find a visible on-screen non-wall-shaping object and a visible
        // on-screen wall-shaping object that are uncontained (so toggling
        // `person` keeps `holds_person` unchanged) and verify both change the
        // rendered output and `Stats.objects`.
        let mut baseline = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
        let baseline_stats = draw_lot_with_preview(
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            &mut baseline,
        );
        let uncontained = |o: &sims_sim::Object| o.data.get(14).copied().unwrap_or(0) == 0;
        let visible_preview_id = world
            .objects
            .iter()
            .find(|&(&id, o)| {
                o.person.is_none()
                    && uncontained(o)
                    && baseline.drawn(id).is_some()
                    && assets.definition(o.guid).is_none_or(|d| !d.shapes_wall())
            })
            .map(|(&id, _)| id)
            .expect("expected a visible uncontained non-wall-shaping object on House05");
        let visible_wall_shaper_id = world
            .objects
            .iter()
            .find(|&(&id, o)| {
                o.person.is_none()
                    && uncontained(o)
                    && baseline.drawn(id).is_some()
                    && assets.definition(o.guid).is_some_and(|d| d.shapes_wall())
            })
            .map(|(&id, _)| id)
            .expect("expected a visible uncontained wall-shaping object on House05");

        // Preview participant person-flag toggle.
        let mut preview_before = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
        let preview_before_stats = cache.draw_lot_with_preview(
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            Some((visible_preview_id, second_id)),
            &mut preview_before,
        );
        assert_parity(
            &mut cache,
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            Some((visible_preview_id, second_id)),
            size,
            "preview participant before person flag toggle",
        );
        world.objects.get_mut(&visible_preview_id).unwrap().person = Some(sample_person.clone());
        let mut preview_after_uncached = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
        let preview_after_stats = draw_lot_with_preview(
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            Some((visible_preview_id, second_id)),
            &mut preview_after_uncached,
        );
        assert_ne!(
            preview_before, preview_after_uncached,
            "preview person toggle must change rendered canvas"
        );
        assert_ne!(
            preview_before_stats.objects, preview_after_stats.objects,
            "preview person toggle must change Stats.objects"
        );
        assert_parity(
            &mut cache,
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            Some((visible_preview_id, second_id)),
            size,
            "preview participant gained person flag",
        );
        world.objects.get_mut(&visible_preview_id).unwrap().person = None;
        assert_parity(
            &mut cache,
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            Some((visible_preview_id, second_id)),
            size,
            "preview participant cleared person flag",
        );

        // Wall-shaping object person-flag toggle.
        assert_parity(
            &mut cache,
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            size,
            "wall shaper before person flag toggle",
        );
        world
            .objects
            .get_mut(&visible_wall_shaper_id)
            .unwrap()
            .person = Some(sample_person);
        let mut wall_after_uncached = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
        let wall_after_stats = draw_lot_with_preview(
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            &mut wall_after_uncached,
        );
        assert_ne!(
            baseline, wall_after_uncached,
            "wall-shaper person toggle must change rendered canvas"
        );
        assert_ne!(
            baseline_stats.objects, wall_after_stats.objects,
            "wall-shaper person toggle must change Stats.objects"
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
            "wall shaper gained person flag",
        );
        world
            .objects
            .get_mut(&visible_wall_shaper_id)
            .unwrap()
            .person = None;
        assert_parity(
            &mut cache,
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            size,
            "wall shaper cleared person flag",
        );
    }

    // 11. Padded canvas storage (`color`/`depth` longer than `width * height`)
    //     must be cleared and preserved on a cache hit identically to uncached
    //     `draw_lot_with_preview`, and must not leak padding to subsequent
    //     standard-sized canvases.
    assert_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        "prime cache before padded canvas hit",
    );
    {
        let mut padded_cached = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
        padded_cached
            .color
            .push(sims_render::pack_rgb([0x11, 0x22, 0x33]));
        padded_cached.depth.push(42);
        let mut padded_uncached = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
        padded_uncached
            .color
            .push(sims_render::pack_rgb([0x11, 0x22, 0x33]));
        padded_uncached.depth.push(42);
        let s_cached = cache.draw_lot_with_preview(
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            &mut padded_cached,
        );
        let s_uncached = draw_lot_with_preview(
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            &mut padded_uncached,
        );
        assert_eq!(s_cached, s_uncached, "padded canvas hit stats mismatch");
        assert_eq!(
            padded_cached, padded_uncached,
            "padded canvas hit storage mismatch"
        );
        assert_eq!(padded_cached.color.len(), size[0] * size[1] + 1);
        assert_eq!(padded_cached.depth.len(), size[0] * size[1] + 1);
    }
    // Populate from a padded canvas on a miss, then verify a normal-sized hit
    // keeps its exact length.
    cache.clear();
    {
        let mut padded_miss = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
        padded_miss
            .color
            .push(sims_render::pack_rgb([0x44, 0x55, 0x66]));
        padded_miss.depth.push(99);
        let mut uncached_miss = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
        uncached_miss
            .color
            .push(sims_render::pack_rgb([0x44, 0x55, 0x66]));
        uncached_miss.depth.push(99);
        let s_cached = cache.draw_lot_with_preview(
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            &mut padded_miss,
        );
        let s_uncached = draw_lot_with_preview(
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            &mut uncached_miss,
        );
        assert_eq!(s_cached, s_uncached);
        assert_eq!(padded_miss, uncached_miss);
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
        "standard canvas hit after padded miss",
    );

    // 12. Swapping to a different `Assets` instance / catalog from the same
    //     install must not share a cache entry.
    let install = sims_content::Install::detect(&root).expect("detect install");
    let mut trimmed_catalog = assets.catalog().clone();
    trimmed_catalog.by_guid.clear();
    let mut other_assets =
        sims_render::Assets::new(&install, assets.index().clone(), trimmed_catalog)
            .expect("construct alternate Assets");
    assert_parity(
        &mut cache,
        &world,
        &lot,
        &mut other_assets,
        &camera,
        (400, 300),
        None,
        size,
        "alternate Assets catalog from same install",
    );

    // 13. Zero-area canvases (`0×0`, `400×0`, `0×300`) with empty vs. padded
    //     `color` buffers must never share stale object rectangles in either
    //     order.
    for [zw, zh] in [[0usize, 0], [400, 0], [0, 300]] {
        for pad_first in [false, true] {
            let mut zcache = LotCache::new();
            for padded in [pad_first, !pad_first] {
                let mut cached = Canvas::new(zw, zh, [0xaa, 0xbb, 0xcc]);
                let mut uncached = Canvas::new(zw, zh, [0xaa, 0xbb, 0xcc]);
                if padded {
                    cached.color.push(sims_render::pack_rgb([1, 2, 3]));
                    uncached.color.push(sims_render::pack_rgb([1, 2, 3]));
                }
                let sc = zcache.draw_lot_with_preview(
                    &world,
                    &lot,
                    assets,
                    &camera,
                    (400, 300),
                    None,
                    &mut cached,
                );
                let su = draw_lot_with_preview(
                    &world,
                    &lot,
                    assets,
                    &camera,
                    (400, 300),
                    None,
                    &mut uncached,
                );
                assert_eq!(sc, su, "zero-area stats mismatch {zw}x{zh} padded={padded}");
                assert_eq!(
                    cached, uncached,
                    "zero-area canvas mismatch {zw}x{zh} padded={padded}"
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn assert_parity_with_hit(
    cache: &mut LotCache,
    world: &World,
    lot: &Lot,
    assets: &mut sims_render::Assets,
    camera: &Camera,
    viewport: (u32, u32),
    preview: Option<(u16, u16)>,
    size: [usize; 2],
    expected_hit: CacheHit,
    label: &str,
) -> Canvas {
    let mut cached = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
    let mut uncached = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
    let stats_cached =
        cache.draw_lot_with_preview(world, lot, assets, camera, viewport, preview, &mut cached);
    let actual_hit = cache.last_hit();
    let stats_uncached =
        draw_lot_with_preview(world, lot, assets, camera, viewport, preview, &mut uncached);
    assert_eq!(
        actual_hit, expected_hit,
        "cache hit tier mismatch at {label}"
    );
    assert_eq!(stats_cached, stats_uncached, "stats mismatch at {label}");
    assert_eq!(cached, uncached, "canvas mismatch at {label}");
    cached
}

#[test]
fn lot_cache_two_tier_hits_environment_and_matches_uncached() {
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

    let size = [400, 300];
    let origin = {
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
    let mut camera = Camera {
        zoom: Zoom::Medium,
        rotation: 0,
        level: 1,
        origin,
        walls: Walls::Cutaway,
        focus: Focus::default(),
    };

    // 1. Cold draw populates both Tier 1 (environment) and Tier 2 (full lot).
    let baseline = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Miss,
        "tier2 cold draw",
    );
    assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Full,
        "tier2 steady hit",
    );

    // Find visible level-1 objects whose mutations visibly change pixels:
    // - `anim_id`: non-wall object with multiple graphics frames (or any visible
    //   non-wall object whose `data[0] + 1` changes rendered pixels),
    // - `door_id`: visible wall-shaping object (door/window) whose `data[0]`
    //   changes pixels and whose wall cutout changes pixels when moved off-lot.
    let anim_id = world
        .objects
        .iter()
        .find(|&(&id, o)| {
            if o.person.is_none()
                && o.position[2] == 1
                && baseline.drawn(id).is_some()
                && assets
                    .definition(o.guid)
                    .is_some_and(|d| !d.shapes_wall() && d.graphics > 1)
            {
                let mut probe_world = world.clone();
                if let Some(obj) = probe_world.objects.get_mut(&id) {
                    if obj.data.is_empty() {
                        obj.data.push(0);
                    }
                    obj.data[0] = obj.data[0].wrapping_add(1);
                }
                let mut probe = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
                draw_lot_with_preview(
                    &probe_world,
                    &lot,
                    assets,
                    &camera,
                    (400, 300),
                    None,
                    &mut probe,
                );
                probe.color != baseline.color
            } else {
                false
            }
        })
        .map(|(&id, _)| id)
        .expect("visible non-wall level-1 object with visibly distinct graphic frame");
    let door_id = world
        .objects
        .iter()
        .find(|&(&id, o)| {
            if o.person.is_none()
                && o.position[2] == 1
                && baseline.drawn(id).is_some()
                && matches!(o.data.get(13).copied().unwrap_or(0) as u16 & 5, 1 | 4)
                && assets.definition(o.guid).is_some_and(|d| d.shapes_wall())
            {
                let mut probe_world = world.clone();
                if let Some(door) = probe_world.objects.get_mut(&id) {
                    if door.data.is_empty() {
                        door.data.push(0);
                    }
                    door.data[0] = door.data[0].wrapping_add(1);
                }
                let mut probe = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
                draw_lot_with_preview(
                    &probe_world,
                    &lot,
                    assets,
                    &camera,
                    (400, 300),
                    None,
                    &mut probe,
                );
                probe.color != baseline.color
            } else {
                false
            }
        })
        .map(|(&id, _)| id)
        .expect("visible wall-shaping level-1 object with distinct graphic frame");

    // 2. Non-wall object graphic animation (`data[0]`) hits Tier 1 (`Environment`),
    //    genuinely changing rendered pixels, then next steady frame hits Tier 2 (`Full`).
    {
        let obj = world.objects.get_mut(&anim_id).unwrap();
        if obj.data.len() <= 51 {
            obj.data.resize(52, 0);
        }
        obj.data[0] = obj.data[0].wrapping_add(1);
    }
    let after_anim = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Environment,
        "animated non-wall object graphic frame",
    );
    assert_ne!(
        after_anim.color, baseline.color,
        "non-wall graphic animation must visibly change pixels"
    );
    assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Full,
        "steady after animated object",
    );

    // 3. Door opening/closing (`data[0]` on a wall-shaping object) leaves its
    //    wall aperture untouched -> Tier 1 hit (`Environment`) + exact parity!
    {
        let door = world.objects.get_mut(&door_id).unwrap();
        if door.data.is_empty() {
            door.data.resize(2, 0);
        }
        door.data[0] = door.data[0].wrapping_add(1);
    }
    let after_door = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Environment,
        "door graphic frame open/close",
    );
    assert_ne!(
        after_door.color, after_anim.color,
        "door graphic animation must visibly change pixels"
    );

    // 4. Moving all parts of a wall-shaping object off-lot removes its wall
    //    aperture -> Tier 1 MUST miss (`Miss`) so walls repaint without the cutout!
    let door_parent = world
        .objects
        .get(&door_id)
        .and_then(|o| o.multipart.as_ref().map(|m| m.parent))
        .filter(|&p| p > 0)
        .unwrap_or(door_id as i16);
    let door_group: Vec<(u16, [i32; 3])> = world
        .objects
        .iter()
        .filter(|&(&id, o)| {
            id == door_id
                || id as i16 == door_parent
                || o.multipart
                    .as_ref()
                    .is_some_and(|m| m.parent == door_parent)
        })
        .map(|(&id, o)| (id, o.position))
        .collect();
    for &(id, _) in &door_group {
        world.objects.get_mut(&id).unwrap().position = [-16, -16, 1];
    }
    let after_door_removed = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Miss,
        "wall-shaper removed off-lot invalidates tier 1",
    );
    assert_ne!(
        after_door_removed.color, after_door.color,
        "removing wall-shaper must visibly change pixels"
    );
    for &(id, pos) in &door_group {
        world.objects.get_mut(&id).unwrap().position = pos;
    }
    let after_door_restored = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Miss,
        "wall-shaper restored",
    );
    assert_eq!(after_door_restored.color, after_door.color);

    // 5. At night with dimmed room light, toggling object glow (`data[51]`)
    //    visibly brightens a non-window object via Tier 1 (`Environment`),
    //    while adding a `lot.lamps` pool visibly illuminates a floored tile
    //    via Tier 1 miss (`Miss`).
    if world.globals.len() < 20 {
        world.globals.resize(20, 0);
    }
    world.globals[4] = 2;
    let _ = world.rebuild_rooms();
    lot = Lot::from_world(&world)
        .expect("Lot::from_world after rebuild_rooms")
        .with_floor_rotation(opened.rotation);
    lot.time_of_day = 2;
    for v in lot.room_lights.values_mut() {
        *v = (*v).min(60);
    }
    for v in lot.room_fractions.values_mut() {
        *v = (*v).min(0.25);
    }
    let night_base = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Miss,
        "nighttime dim room baseline",
    );
    let glow_id = world
        .objects
        .iter()
        .find(|&(&id, o)| {
            if o.person.is_none()
                && o.position[2] == 1
                && night_base.drawn(id).is_some()
                && assets.definition(o.guid).is_none_or(|d| !d.shapes_wall())
            {
                let mut probe_world = world.clone();
                if let Some(obj) = probe_world.objects.get_mut(&id) {
                    if obj.data.len() <= 51 {
                        obj.data.resize(52, 0);
                    }
                    obj.data[51] = if obj.data[51] == 0 { 100 } else { 0 };
                }
                let mut probe = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
                draw_lot_with_preview(
                    &probe_world,
                    &lot,
                    assets,
                    &camera,
                    (400, 300),
                    None,
                    &mut probe,
                );
                probe.color != night_base.color
            } else {
                false
            }
        })
        .map(|(&id, _)| id)
        .expect("visible non-wall object whose glow word visibly changes night pixels");
    {
        let obj = world.objects.get_mut(&glow_id).unwrap();
        if obj.data.len() <= 51 {
            obj.data.resize(52, 0);
        }
        obj.data[51] = if obj.data[51] == 0 { 100 } else { 0 };
    }
    let after_glow = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Environment,
        "lamp glow word toggle at night",
    );
    assert_ne!(
        after_glow.color, night_base.color,
        "glow word toggle at night must visibly change object pixels"
    );
    // Pick a floored tile under `glow_id` (known on-screen and dimmed at night)
    // so the lamp pool visibly illuminates its floor tile.
    let glow_tile = world.objects.get(&glow_id).unwrap().position;
    let lamp_cell = Lot::cell(
        (glow_tile[0] / 16).clamp(0, lot.size as i32 - 1) as usize,
        (glow_tile[1] / 16).clamp(0, lot.size as i32 - 1) as usize,
    );
    lot.levels[0].floors[lamp_cell] = lot.levels[0].floors[lamp_cell].max(1);
    lot.lamps[0][lamp_cell] = Lamps([Some([0, 0]), None]);
    let after_lamp_pool = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Miss,
        "lot lamp pool change invalidates tier 1",
    );
    assert_ne!(
        after_lamp_pool.color, after_glow.color,
        "adding lamp pool on visible night floor must visibly change pixels"
    );

    // 6. TV / appliance dynamic sprite animation (`dynamic_sprites`) hits Tier 1
    //    and visibly changes pixels on a real dynamic-sprite object on House05.
    let (dyn_id, dyn_cam, dyn_target) = world
        .objects
        .iter()
        .find_map(|(&id, o)| {
            if o.person.is_some()
                || o.position[2] != 1
                || o.position[0] < 0
                || o.position[1] < 0
                || !assets
                    .definition(o.guid)
                    .is_some_and(|d| !d.shapes_wall() && d.dynamic_base > 0)
            {
                return None;
            }
            let mut cam = camera;
            cam.origin = [0, 0];
            let p = cam.project_fine(cam.rotate_fine([o.position[0], o.position[1]]), 1, 0);
            cam.origin = [size[0] as i32 / 2 - p[0], size[1] as i32 / 2 - p[1]];
            let mut before = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
            draw_lot_with_preview(&world, &lot, assets, &cam, (400, 300), None, &mut before);
            for candidate in [vec![0i16; 16], vec![1i16; 16]] {
                if o.dynamic_sprites == candidate {
                    continue;
                }
                let mut probe_world = world.clone();
                probe_world.objects.get_mut(&id).unwrap().dynamic_sprites = candidate.clone();
                let mut probe = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
                draw_lot_with_preview(
                    &probe_world,
                    &lot,
                    assets,
                    &cam,
                    (400, 300),
                    None,
                    &mut probe,
                );
                if probe.color != before.color {
                    return Some((id, cam, candidate));
                }
            }
            None
        })
        .expect("placed House05 dynamic-sprite object whose dynamic_sprites visibly change pixels");
    let before_dyn = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &dyn_cam,
        (400, 300),
        None,
        size,
        CacheHit::Miss,
        "dynamic-sprite object framed cold",
    );
    world.objects.get_mut(&dyn_id).unwrap().dynamic_sprites = dyn_target;
    let after_dyn_framed = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &dyn_cam,
        (400, 300),
        None,
        size,
        CacheHit::Environment,
        "appliance dynamic_sprites toggle",
    );
    assert_ne!(
        after_dyn_framed.color, before_dyn.color,
        "dynamic_sprites toggle must visibly change pixels"
    );
    let after_dyn = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Miss,
        "restore main camera after dynamic-sprite check",
    );

    // 7. Buy Mode preview moving (cursor + visible off-lot held object, no
    //    shadow/wall change) hits Tier 1 (`Environment`) and visibly moves the
    //    rendered preview pixels!
    let preview_held_id = world
        .objects
        .iter()
        .find(|&(&id, o)| {
            id != anim_id
                && id != door_id
                && o.person.is_none()
                && o.position[2] == 1
                && night_base.drawn(id).is_some()
                && assets.definition(o.guid).is_none_or(|d| !d.shapes_wall())
        })
        .map(|(&id, _)| id)
        .expect("visible non-wall object for buy preview");
    let saved_held_pos = world.objects.get(&preview_held_id).unwrap().position;
    let saved_anim_pos = world.objects.get(&anim_id).unwrap().position;
    world.objects.get_mut(&preview_held_id).unwrap().position = [-16, -16, 1];
    let after_preview = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        Some((anim_id, preview_held_id)),
        size,
        CacheHit::Environment,
        "buy preview active",
    );
    assert_ne!(
        after_preview.color, after_dyn.color,
        "activating buy preview must visibly change pixels"
    );
    world.objects.get_mut(&anim_id).unwrap().position[0] += 16;
    world.objects.get_mut(&anim_id).unwrap().position[1] += 16;
    let after_preview_move = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        Some((anim_id, preview_held_id)),
        size,
        CacheHit::Environment,
        "buy preview moved",
    );
    assert_ne!(
        after_preview_move.color, after_preview.color,
        "moving buy preview must visibly change pixels"
    );
    world.objects.get_mut(&preview_held_id).unwrap().position = saved_held_pos;
    world.objects.get_mut(&anim_id).unwrap().position = saved_anim_pos;

    // 8. Padded canvas on a Tier-1 hit must match uncached bit-for-bit including
    //    trailing padded elements.
    world.objects.get_mut(&anim_id).unwrap().data[0] =
        world.objects.get(&anim_id).unwrap().data[0].wrapping_add(1);
    {
        let mut padded_cached = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
        padded_cached
            .color
            .push(sims_render::pack_rgb([0x11, 0x22, 0x33]));
        padded_cached.depth.push(42);
        let mut padded_uncached = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
        padded_uncached
            .color
            .push(sims_render::pack_rgb([0x11, 0x22, 0x33]));
        padded_uncached.depth.push(42);
        let sc = cache.draw_lot_with_preview(
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            &mut padded_cached,
        );
        assert_eq!(cache.last_hit(), CacheHit::Environment);
        let su = draw_lot_with_preview(
            &world,
            &lot,
            assets,
            &camera,
            (400, 300),
            None,
            &mut padded_uncached,
        );
        assert_eq!(sc, su);
        assert_eq!(padded_cached, padded_uncached);
    }

    // 9. Level 2 (`camera.level == 2`): place visible level-2 walls on the
    //    centered tile so toggling `camera.walls` from `Down` to `Up` visibly
    //    raises level-2 walls while still hitting Tier 1 (`Environment`) for
    //    level 1 (which stands full height under both wall modes).
    lot.levels[1].walls[lamp_cell].segments |= WEST | NORTH;
    lot.levels[1].walls[lamp_cell].styles = [1, 1];
    lot.levels[1].walls[lamp_cell].patterns = [2, 2, 2, 2];
    camera.level = 2;
    camera.walls = Walls::Down;
    let level2_cold = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Miss,
        "level 2 cold",
    );
    assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Full,
        "level 2 steady",
    );
    let mut level2_visible = std::collections::BTreeSet::new();
    for y in 0..size[1] as i32 {
        for x in 0..size[0] as i32 {
            if let Some(id) = level2_cold.id_at(x, y) {
                level2_visible.insert(id);
            }
        }
    }
    let level2_vis_id = world
        .objects
        .iter()
        .find(|&(&id, o)| {
            o.person.is_none()
                && o.position[2] == 1
                && level2_visible.contains(&id)
                && assets.definition(o.guid).is_none_or(|d| !d.shapes_wall())
        })
        .map(|(&id, _)| id)
        .expect("level-1 object with visible pixels in level-2 view");
    {
        let obj = world.objects.get_mut(&level2_vis_id).unwrap();
        if obj.data.len() <= 34 {
            obj.data.resize(35, 0);
        }
        obj.data[34] = 1;
    }
    let level2_anim = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Environment,
        "level 2 with level-1 object hidden toggle",
    );
    assert_ne!(
        level2_anim.color, level2_cold.color,
        "level-1 object mutation on level 2 must visibly change pixels"
    );
    camera.walls = Walls::Up;
    let level2_walls_up = assert_parity_with_hit(
        &mut cache,
        &world,
        &lot,
        assets,
        &camera,
        (400, 300),
        None,
        size,
        CacheHit::Environment,
        "level 2 walls mode switch keeps level-1 tier 1",
    );
    assert_ne!(
        level2_walls_up.color, level2_anim.color,
        "raising level-2 walls must visibly change pixels while reusing level-1 tier 1"
    );
    world.objects.get_mut(&level2_vis_id).unwrap().data[34] = 0;

    // 10. `RenderOptions` (`shadows` and `lighting`) must invalidate BOTH Tier 2
    //     and Tier 1 (`CacheHit::Miss`), visibly change pixels, and preserve
    //     Tier-1 hits and bit-for-bit parity when objects subsequently animate.
    camera.level = 1;
    camera.walls = Walls::Cutaway;
    let assert_options_parity = |cache: &mut LotCache,
                                 world: &World,
                                 lot: &Lot,
                                 assets: &mut sims_render::Assets,
                                 options: RenderOptions,
                                 expected_hit: CacheHit,
                                 label: &str|
     -> Canvas {
        let mut cached = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
        let mut uncached = Canvas::new(size[0], size[1], [0xaa, 0xbb, 0xcc]);
        let sc = cache.draw_lot_with_preview_and_options(
            world,
            lot,
            assets,
            &camera,
            (400, 300),
            None,
            options,
            &mut cached,
        );
        let actual_hit = cache.last_hit();
        let su = draw_lot_with_preview_and_options(
            world,
            lot,
            assets,
            &camera,
            (400, 300),
            None,
            options,
            &mut uncached,
        );
        assert_eq!(
            actual_hit, expected_hit,
            "cache hit tier mismatch at {label}"
        );
        assert_eq!(sc, su, "stats mismatch at {label}");
        assert_eq!(cached, uncached, "canvas mismatch at {label}");
        cached
    };

    let opt_both = RenderOptions {
        shadows: true,
        lighting: true,
    };
    let both_canvas = assert_options_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        opt_both,
        CacheHit::Miss,
        "render options shadows=true lighting=true cold",
    );
    assert_options_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        opt_both,
        CacheHit::Full,
        "render options shadows=true lighting=true steady",
    );

    // Turning off lighting hides the night lamp pool added in step 5.
    let opt_no_lighting = RenderOptions {
        shadows: true,
        lighting: false,
    };
    let no_lighting_canvas = assert_options_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        opt_no_lighting,
        CacheHit::Miss,
        "render options lighting=false invalidates tier 1 and tier 2",
    );
    assert_ne!(
        no_lighting_canvas.color, both_canvas.color,
        "toggling lighting off must visibly remove floor lamp pool"
    );
    assert_options_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        opt_no_lighting,
        CacheHit::Full,
        "render options lighting=false steady",
    );

    // Turning off shadows removes object floor shadows.
    let opt_neither = RenderOptions {
        shadows: false,
        lighting: false,
    };
    let neither_canvas = assert_options_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        opt_neither,
        CacheHit::Miss,
        "render options shadows=false invalidates tier 1 and tier 2",
    );
    assert_ne!(
        neither_canvas.color, no_lighting_canvas.color,
        "toggling shadows off must visibly remove floor shadows"
    );

    // Animating an object while shadows/lighting are off still hits Tier 1 (`Environment`)
    // and matches uncached rendering bit-for-bit.
    {
        let obj = world.objects.get_mut(&glow_id).unwrap();
        obj.data[51] = if obj.data[51] == 0 { 100 } else { 0 };
    }
    let anim_neither_canvas = assert_options_parity(
        &mut cache,
        &world,
        &lot,
        assets,
        opt_neither,
        CacheHit::Environment,
        "object glow toggle with shadows=false lighting=false hits tier 1",
    );
    assert_ne!(
        anim_neither_canvas.color, neither_canvas.color,
        "object glow toggle must visibly change pixels under shadows=false lighting=false"
    );
}
