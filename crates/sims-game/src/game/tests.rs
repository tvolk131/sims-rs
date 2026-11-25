use super::*;

#[test]
fn a_trip_that_only_saved_more_is_the_same_trip() {
    let trip = sims_sim::TripRequest {
        house: 21,
        leader_guid: 0x1234,
        family: 0,
        caller: 7,
        saved: vec![1],
    };
    let mut merged = trip.clone();
    merged.saved.push(2);
    assert!(same_trip(&trip, &merged));
    for other in [
        sims_sim::TripRequest {
            house: 22,
            ..trip.clone()
        },
        sims_sim::TripRequest {
            leader_guid: 0x4321,
            ..trip.clone()
        },
        sims_sim::TripRequest {
            family: 3,
            ..trip.clone()
        },
        sims_sim::TripRequest {
            caller: 8,
            ..trip.clone()
        },
    ] {
        assert!(!same_trip(&trip, &other), "{other:?}");
    }
}

fn ctss(entries: &[(u8, &str)]) -> sims_formats::strings::StringTable {
    sims_formats::strings::StringTable {
        format: sims_formats::strings::Format::Commented,
        entries: entries
            .iter()
            .map(|&(language, text)| sims_formats::strings::TextEntry {
                language,
                text: text.to_owned(),
                comment: String::new(),
                raw_text: text.as_bytes().to_vec(),
                raw_comment: Vec::new(),
            })
            .collect(),
    }
}

/// `0x457d3b` / `0x4061e3`: the catalog name the Phonebook checks
/// (`0x523064`) is never empty, and its language retry stops at the
/// first language that has the slot.
#[test]
fn catalog_names_fall_back_as_native_does() {
    // `CATS`: the name is the second string.
    assert_eq!(
        definition_catalog_name(Some(b"desc\0Bella\0"), None, true, "Label"),
        "Bella"
    );
    // US English first.
    let table = ctss(&[(1, "Mortimer"), (2, "Morty")]);
    assert_eq!(
        definition_catalog_name(None, Some(&table), true, "Label"),
        "Mortimer"
    );
    // No text and no label: `*` alone (`0x457db4`), so not skipped.
    assert_eq!(definition_catalog_name(None, None, true, ""), "*");
    assert_eq!(
        definition_catalog_name(None, None, false, "Chair"),
        "*Chair"
    );
    // A person whose US English slot is empty retries; the first
    // language with the slot wins even when its text is empty
    // (`0x47145f`), so language 3's name is never reached.
    let table = ctss(&[(1, ""), (3, "Kaylynn")]);
    assert_eq!(
        definition_catalog_name(None, Some(&table), true, "Label"),
        "*Label",
        "US English has the slot, empty, so the probe stops there"
    );
    let table = ctss(&[(2, ""), (3, "Kaylynn")]);
    assert_eq!(
        definition_catalog_name(None, Some(&table), true, "Label"),
        "*Label",
        "language 2 has the slot, empty, so the probe stops there"
    );
    let table = ctss(&[(5, "Erin"), (3, "Kaylynn")]);
    assert_eq!(
        definition_catalog_name(None, Some(&table), true, "Label"),
        "Kaylynn",
        "the probe goes by language number, not file order"
    );
    // Only a person retries.
    assert_eq!(
        definition_catalog_name(None, Some(&table), false, "Label"),
        "*Label"
    );
    // A language's slot 1 is the first entry of its last run.
    let table = ctss(&[(1, "Old"), (2, "x"), (1, "New")]);
    assert_eq!(
        definition_catalog_name(None, Some(&table), false, "Label"),
        "New"
    );
    // The probe reaches code 20 (Korean, index 19).
    let table = ctss(&[(20, "Korean")]);
    assert_eq!(
        definition_catalog_name(None, Some(&table), true, "Label"),
        "Korean"
    );
}

#[test]
fn catalog_strings_read_a_ctss_in_its_flags_byte_order() {
    use sims_formats::iff::{ChunkData, Iff, write};
    // A version -1 table of two strings, written big-endian (flag 0x10
    // clear) and little-endian (flag 0x10 set).
    let file = |flags: u16, data: Vec<u8>| {
        Iff::parse(
            write(&[ChunkData {
                tag: *b"CTSS",
                id: 2000,
                flags,
                label: String::new(),
                data,
            }])
            .unwrap(),
        )
        .unwrap()
    };
    let mut big = vec![0xff, 0xff, 0x00, 0x02];
    big.extend(b"Bella\0Goth\0");
    let mut little = vec![0xff, 0xff, 0x02, 0x00];
    little.extend(b"Bella\0Goth\0");
    for (flags, data) in [(0, big), (0x10, little)] {
        let iff = file(flags, data);
        let (cats, ctss) = catalog_strings(&iff, 2000);
        assert!(cats.is_none());
        assert_eq!(
            definition_catalog_name(cats, ctss.as_ref(), true, "Label"),
            "Bella",
            "flags {flags:#x}"
        );
    }
    // A table of length-prefixed strings (a count, not a version)
    // reads 0xa1 as `o` and 0xd5 as `'` (`0x470f45`).
    let iff = file(0x10, b"\x02\x00\x04R\xa1sa\x03O\xd5s".to_vec());
    let (_, ctss) = catalog_strings(&iff, 2000);
    let ctss = ctss.expect("length-prefixed CTSS");
    assert_eq!(ctss.entries[0].text, "Rosa");
    assert_eq!(ctss.entries[1].text, "O's");
    assert_eq!(
        definition_catalog_name(None, Some(&ctss), true, "Label"),
        "Rosa"
    );
    // Only that layout: a NUL-terminated table keeps the byte.
    let iff = file(0x10, b"\xff\xff\x01\x00R\xa1sa\x00".to_vec());
    let (_, ctss) = catalog_strings(&iff, 2000);
    assert_eq!(ctss.expect("terminated CTSS").entries[0].text, "R¡sa");
}

#[test]
fn a_preview_never_stands_in_for_the_vm_request() {
    let request = SpecialPanelRequest::CallNeighbor { caller_neighbor: 7 };
    let shown = ShownSpecialPanel {
        tick: 0,
        request: request.clone(),
    };
    let mut panel = ActiveSpecialPanel {
        tick: 0,
        request,
        coop_dialog_id: None,
        responses: BTreeMap::new(),
        preview_targets: BTreeMap::new(),
        preview: true,
    };
    assert!(
        !panel.shows(&shown, None),
        "a tick-0 preview of the same request is replaced by the real one"
    );
    panel.preview = false;
    assert!(panel.shows(&shown, None), "the real panel stays as it is");
}
use sims_vm::{Fault, Location, Trap};

fn fault(object: u16) -> Fault {
    Fault {
        object,
        at: Some(Location {
            tree: 0x1002,
            node: 7,
        }),
        trap: Trap::Unimplemented { opcode: 0x2a },
    }
}

#[test]
fn a_centred_person_is_raised_by_half_its_zooms_height() {
    // trunc(2 sqrt 6 * 2.0 * 2^(zoom + 1)) / 2 for far, medium, near.
    assert_eq!([1, 2, 3].map(super::person_rise), [19, 39, 78]);
}

#[test]
fn scroll_limits_match_0x4a9fa9_across_rotations_view_widths_and_terrain() {
    use sims_render::camera::{Walls, Zoom};
    let mut world = sims_sim::World::empty();
    world.size = 52;
    let full = sims_render::lot::FULL * sims_render::lot::FULL;
    for (id, stride) in [(0, 4), (1, 1), (2, 8), (101, 1), (102, 8)] {
        world.arrays.insert(
            id,
            sims_formats::lot::Array {
                width: sims_render::lot::FULL,
                height: sims_render::lot::FULL,
                stride,
                encoding: 0,
                data: vec![0; full * stride],
            },
        );
    }
    let flat = sims_render::Lot::from_world(&world).unwrap();
    let cam = |zoom: Zoom, rotation: u8| Camera {
        zoom,
        rotation,
        level: 1,
        origin: [0, 0],
        walls: Walls::Up,
        focus: sims_render::Focus::default(),
    };

    // 52x52 flat lot at far zoom, 800x500 house view:
    // span_x = 50 * 32 = 1600, min_x = (-800 - 16) + 800 = -16,
    // max_x = 800 + 32 = 832;
    // max_y = 0 + 58 * 2 = 116;
    // min_y = (500 - 32) - 50 * 16 - 8 = -340.
    assert_eq!(
        scroll_limits(&flat, &cam(Zoom::Far, 0), [800, 500]),
        ([-16, 832], [-340, 116])
    );
    // Rotation 1 on a 52x52 lot shifts min_rx = 12, min_ry = 0:
    // shift_x = 12 * 16 = 192, shift_y = 12 * 8 = 96.
    assert_eq!(
        scroll_limits(&flat, &cam(Zoom::Far, 1), [800, 500]),
        ([-16 - 192, 832 - 192], [-340 - 96, 116 - 96])
    );
    // Rotation 2 shifts min_rx = 12, min_ry = 12:
    // shift_x = 0, shift_y = 24 * 8 = 192.
    assert_eq!(
        scroll_limits(&flat, &cam(Zoom::Far, 2), [800, 500]),
        ([-16, 832], [-340 - 192, 116 - 192])
    );
    // Viewport wider than span_x (1600) centres x at view_w / 2.
    let ([min_x, max_x], _) = scroll_limits(&flat, &cam(Zoom::Far, 0), [2000, 500]);
    assert_eq!((min_x, max_x), (1000, 1000));

    // Raising the top corner tile (1, 1) to height 10 raises max_y by
    // max(0, (10 << 2) - 8 * 1) = 32 -> 116 + 32 = 148.
    let cell = |x: usize, y: usize| (x * sims_render::lot::FULL + y) * 4;
    world.arrays.get_mut(&0).unwrap().data[cell(1, 1)..cell(1, 1) + 4]
        .copy_from_slice(&[10, 10, 10, 10]);
    let raised = sims_render::Lot::from_world(&world).unwrap();
    let (_, [min_y, max_y]) = scroll_limits(&raised, &cam(Zoom::Far, 0), [800, 500]);
    assert_eq!((min_y, max_y), (-340, 148));
}

#[test]
fn centre_position_averages_linked_multipart_tiles() {
    let mut world = sims_sim::World::empty();
    let make = |id: u16, position: [i32; 3], parent: Option<i16>| sims_sim::Object {
        id,
        guid: 0,
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
        multipart: parent.map(|p| sims_formats::saved_objects::Multipart {
            offset: [0; 3],
            group: [0; 3],
            parent: p,
        }),
        portal: None,
        person: None,
        registry_word: None,
        last_result: false,
        removal_skips_resets: false,
        selected: false,
    };
    world.objects.insert(1, make(1, [100, 200, 1], None));
    world.objects.insert(10, make(10, [384, 416, 1], Some(0)));
    world.objects.insert(11, make(11, [400, 416, 1], Some(10)));
    world.objects.insert(12, make(12, [416, 449, 2], Some(10)));
    world.next_parts.insert(10, 11);
    world.next_parts.insert(11, 12);
    assert_eq!(centre_position(&world, 1), Some([100, 200, 1]));
    // (384+400+416)/3 = 400, (416+416+449)/3 = 1281/3 = 427; level comes
    // from the queried object (0x434ced reads param_1 + 0x110).
    assert_eq!(centre_position(&world, 10), Some([400, 427, 1]));
    assert_eq!(centre_position(&world, 11), Some([400, 427, 1]));
    assert_eq!(centre_position(&world, 12), Some([400, 427, 2]));
}
#[test]
fn an_error_says_what_failed_and_what_was_done() {
    let mut error = ScriptError {
        tick: 900,
        object: 12,
        fault: fault(12),
        in_thread: true,
        then: None,
        recovery: Recovery::Reset,
    };
    let line = describe(&error);
    assert!(line.starts_with("tick 900, object 12: object 12, tree 0x1002 node 7"));
    assert!(line.ends_with("; it was reset"), "{line}");
    // A caught thread fault, then one in the rest of the tick.
    error.then = Some(fault(13));
    error.recovery = Recovery::Restarted(fault(12));
    let line = describe(&error);
    assert!(
        line.starts_with("tick 900, object 12: object 12,"),
        "{line}"
    );
    assert!(line.contains("; then, outside its thread, object 13"));
    assert!(line.ends_with("its thread restarted at Main"), "{line}");
    error.then = None;
    error.in_thread = false;
    error.recovery = Recovery::Parked(None);
    let line = describe(&error);
    assert!(line.starts_with("tick 900, object 12 outside its thread:"));
    assert!(line.ends_with("it failed again after a reset, so it stops"));
    error.recovery = Recovery::Parked(Some(fault(12)));
    let line = describe(&error);
    assert!(
        line.contains("restarting its thread failed (object 12,"),
        "{line}"
    );
    assert!(line.ends_with("so it stops"));
}

#[test]
fn an_updates_errors_are_logged_in_the_order_they_happened() {
    let error = ScriptError {
        tick: 1,
        object: 12,
        fault: fault(12),
        in_thread: true,
        then: None,
        recovery: Recovery::Reset,
    };
    let frames = |skipped: &[(u32, &str)]| -> Vec<SkippedFrame> {
        skipped
            .iter()
            .map(|&(after, error)| SkippedFrame {
                after,
                error: error.to_string(),
            })
            .collect()
    };
    let starts = |lines: Vec<String>| -> Vec<String> {
        lines.into_iter().map(|l| l[..13].to_string()).collect()
    };
    let errors = std::slice::from_ref(&error);
    // The first step: the frame before the tick, the tick, the frame
    // after it.
    let first = in_order(1, errors, &frames(&[(1, "a"), (0, "b")]));
    assert_eq!(
        starts(first.clone()),
        ["the frame bef", "tick 1, objec", "the frame aft"]
    );
    assert_eq!(first[0], "the frame before the first tick was skipped: b");
    assert_eq!(first[2], "the frame after the tick was skipped: a");
    // A single frame on the first step says which one it was.
    assert_eq!(
        starts(in_order(1, errors, &frames(&[(1, "a")]))),
        ["tick 1, objec", "the frame aft"]
    );
    assert_eq!(
        starts(in_order(1, errors, &frames(&[(0, "b")]))),
        ["the frame bef", "tick 1, objec"]
    );
    // A step that failed hands back the frame before its tick only.
    assert_eq!(
        in_order(1, &[], &frames(&[(0, "b")])),
        ["the frame before the first tick was skipped: b"]
    );
    // Later steps draw only after the tick.
    assert_eq!(
        in_order(34, &[], &frames(&[(34, "a")])),
        ["the frame after the tick was skipped: a"]
    );
    assert!(in_order(2, &[], &[]).is_empty());
}

#[test]
fn errors_show_for_a_while_and_the_log_is_bounded() {
    let mut errors = Errors::default();
    assert_eq!(errors.recent(0), None);
    errors.push(100, "first".into());
    assert_eq!(errors.recent(100).as_deref(), Some("Error caught: first"));
    errors.push(110, "second".into());
    assert_eq!(
        errors.recent(110 + SHOWN_TICKS - 1).as_deref(),
        Some("Error caught (2 so far): second")
    );
    assert_eq!(errors.recent(110 + SHOWN_TICKS), None);
    // The same error at another tick is counted, not listed or shown
    // again.
    errors.push(300, "tick 7, object 2: x".into());
    errors.push(310 + SHOWN_TICKS, "tick 8, object 2: x".into());
    assert_eq!(errors.lines.len(), 3);
    assert_eq!(errors.lines[2], "tick 7, object 2: x");
    assert_eq!(errors.count, 4);
    assert_eq!(errors.recent(310 + SHOWN_TICKS), None);
    for n in 0..LOGGED * 2 {
        errors.push(200, n.to_string());
    }
    assert_eq!(errors.lines.len(), LOGGED + 1);
    assert_eq!(errors.count, LOGGED as u64 * 2 + 4);
    assert_eq!(
        errors.lines.last().unwrap(),
        "further errors are counted, not listed"
    );
    // Once SEEN different errors are remembered, a new one is only
    // counted, so one recurring every tick does not hold the line open.
    for n in errors.seen.len()..SEEN {
        errors.push(400, format!("tick 9, filler {n}"));
    }
    let count = errors.count;
    errors.push(400 + SHOWN_TICKS, "tick 10, object 3: new".into());
    assert_eq!(errors.count, count + 1);
    assert_eq!(errors.recent(400 + SHOWN_TICKS), None);
    assert_eq!(errors.lines.len(), LOGGED + 1);
}

#[test]
fn the_sound_queue_keeps_the_newest_and_hands_them_over_once() {
    let play = |object: i16| SoundEvent::Play {
        object,
        name: format!("sound {object}"),
    };
    let mut queue = VecDeque::new();
    keep(&mut queue, (0..3).map(play).collect(), 4);
    keep(&mut queue, (3..6).map(play).collect(), 4);
    // The two oldest are dropped; the rest stay in order.
    assert_eq!(
        Vec::from(queue.clone()),
        (2..6).map(play).collect::<Vec<_>>()
    );
    keep(&mut queue, Vec::new(), 4);
    assert_eq!(queue.len(), 4);
    // Taking them empties the queue, as `Game::take_sounds` does.
    let taken: Vec<_> = queue.drain(..).collect();
    assert_eq!(taken.len(), 4);
    assert!(queue.is_empty());
}

#[test]
fn a_mid_tick_speed_reset_updates_the_panel_and_drops_owed_time() {
    use sims_sim::speed::{FAST, IDLE, NORMAL};

    let mut world = sims_sim::World::empty();
    world.globals[18] = -1;
    world.set_speed(i32::from(FAST));

    let mut pace = Pace::default();
    pace.set_speed(Speed::Fast);
    // Unchanged speed keeps both the panel selection and accrued credit.
    assert_eq!(pace.due(13_000, FAST), 0);
    assert!(!sync_speed_after_step(&mut pace, (FAST, -1), &world));
    assert_eq!(pace.speed, Speed::Fast);
    assert_eq!(pace.due(1_000, FAST), 1);

    // Accrue 13 ms at Fast, then drop speed to Normal inside a step:
    // the panel switches to Normal and the 13 ms credit is dropped so
    // the next Normal tick waits a full 42 ms.
    assert_eq!(pace.due(13_000, FAST), 0);
    world.set_speed(i32::from(NORMAL));
    assert!(sync_speed_after_step(&mut pace, (FAST, -1), &world));
    assert_eq!(pace.speed, Speed::Normal);
    assert_eq!(pace.due(29_000, NORMAL), 0);
    assert_eq!(pace.due(12_999, NORMAL), 0);
    assert_eq!(pace.due(1, NORMAL), 1);

    // Idle speed (1000, returning to Fast) dropping to Normal (760, 760)
    // also switches the panel to Normal and drops accrued credit.
    pace.set_speed(Speed::Fast);
    world.globals[16] = IDLE;
    world.globals[18] = FAST;
    assert_eq!(pace.due(500, IDLE), 0);
    world.set_speed(i32::from(NORMAL));
    assert_eq!((world.speed(), world.return_speed()), (NORMAL, NORMAL));
    assert!(sync_speed_after_step(&mut pace, (IDLE, FAST), &world));
    assert_eq!(pace.speed, Speed::Normal);
    assert_eq!(pace.due(41_999, NORMAL), 0);
    assert_eq!(pace.due(1, NORMAL), 1);

    // Wake-up clearing global 18 from (760, 760) to (760, -1) inside a
    // manual step still calls 0x46d697 and must drop accrued credit.
    assert_eq!(pace.due(41_000, NORMAL), 0);
    world.globals[18] = -1;
    assert!(sync_speed_after_step(&mut pace, (NORMAL, NORMAL), &world));
    assert_eq!(pace.speed, Speed::Normal);
    assert_eq!(pace.due(1_000, NORMAL), 0);
    assert_eq!(pace.due(40_999, NORMAL), 0);
    assert_eq!(pace.due(1, NORMAL), 1);
}

#[test]
fn house_viewport_size_follows_the_control_panel_strip() {
    use sims_ui::command::Mode;
    use sims_ui::hud::Controls;

    let mut controls = Controls::default();
    assert!(!controls.expanded);
    assert_eq!(viewport_for_controls([800, 600], &controls), (800, 600));

    controls.expanded = true;
    controls.mode = Mode::Live;
    assert_eq!(viewport_for_controls([800, 600], &controls), (800, 500));

    controls.mode = Mode::Build;
    assert_eq!(viewport_for_controls([800, 600], &controls), (800, 500));

    controls.mode = Mode::Options;
    assert_eq!(viewport_for_controls([800, 600], &controls), (800, 500));

    controls.mode = Mode::Buy;
    assert_eq!(viewport_for_controls([800, 600], &controls), (800, 500));

    // Short windows clamp to at least 1 pixel rather than wrapping.
    assert_eq!(viewport_for_controls([800, 80], &controls), (800, 1));
}

#[test]
fn screen_centre_pick_uses_active_viewport_and_native_origin_column() {
    use sims_render::camera::{Walls, Zoom};
    use sims_ui::hud::Controls;

    let mut world = sims_sim::World::empty();
    world.size = 64;
    let full = sims_render::lot::FULL * sims_render::lot::FULL;
    for (id, stride) in [(0, 4), (1, 1), (2, 8), (101, 1), (102, 8)] {
        world.arrays.insert(
            id,
            sims_formats::lot::Array {
                width: sims_render::lot::FULL,
                height: sims_render::lot::FULL,
                stride,
                encoding: 0,
                data: vec![0; full * stride],
            },
        );
    }
    let lot = sims_render::Lot::from_world(&world).unwrap();
    let pick_tile = |game_origin: [i32; 2], expanded: bool| {
        let controls = Controls {
            expanded,
            ..Controls::default()
        };
        let (w, h) = viewport_for_controls([800, 600], &controls);
        let camera = Camera {
            origin: [
                game_origin[0].saturating_add(pick::ORIGIN_COLUMN),
                game_origin[1],
            ],
            zoom: Zoom::Far,
            rotation: 0,
            level: 1,
            walls: Walls::Cutaway,
            focus: sims_render::Focus::default(),
        };
        sims_render::screen_centre_pick(&lot, &camera, [w as i32, h as i32])
            .map(|[x, y]| [(x >> 4) as i8, (y >> 4) as i8])
    };
    // Native origin [400, 0] (game origin [399, 0]): expanded 800x500 picks
    // [15, 15], collapsed 800x600 picks [18, 18].
    assert_eq!(pick_tile([399, 0], true), Some([15, 15]));
    assert_eq!(pick_tile([399, 0], false), Some([18, 18]));
    // Expanded 800x500: game origin [398, -6] (native [399, -6]) picks
    // [16, 15], while game origin [399, -6] (native [400, -6]) picks [16, 16].
    assert_eq!(pick_tile([398, -6], true), Some([16, 15]));
    assert_eq!(pick_tile([399, -6], true), Some([16, 16]));

    // Zooming from Far to Medium with the panel expanded (800x500 active
    // viewport) uses native 0x4a7b58 + 0x434cb7 + 0x4a7996 anchoring:
    // - from native [400, -6] (inside lot): stays on [16, 16];
    // - from native [400, 240] (tile [0, 0], outside interior): falls back
    //   to quarter-lot tile [16, 16] (0x4a7c56..0x4a7c96) instead of [32, 32].
    let controls = Controls {
        expanded: true,
        ..Controls::default()
    };
    let (w, h) = viewport_for_controls([800, 600], &controls);
    let viewport = [w as i32, h as i32];
    let pick_unrotated = |camera: Camera| {
        let mut native = camera;
        native.origin[0] = native.origin[0].saturating_add(pick::ORIGIN_COLUMN);
        let tile = sims_render::screen_centre_pick(&lot, &native, viewport)
            .map(|[x, y]| [i32::from((x >> 4) as i8), i32::from((y >> 4) as i8)])?;
        Some(native.unrotate_tile(tile))
    };

    for start_origin in [[399, -6], [399, 240]] {
        let mut cam = Camera {
            origin: start_origin,
            zoom: Zoom::Far,
            rotation: 0,
            level: 1,
            walls: Walls::Cutaway,
            focus: sims_render::Focus::default(),
        };
        assert!(native_zoom_or_rotate(
            &lot,
            &mut cam,
            viewport,
            Move::ZoomIn
        ));
        assert_eq!(
            pick_unrotated(cam),
            Some([16, 16]),
            "zoom from game origin {start_origin:?}"
        );
    }

    // On floor 2 at native origin [400, -6] (game [399, -6]), clockwise
    // rotation anchors on the displayed floor's pick ([19, 19]) and keeps
    // the unrotated search start at [19, 19].
    let mut floor2 = Camera {
        origin: [399, -6],
        zoom: Zoom::Far,
        rotation: 0,
        level: 2,
        walls: Walls::Cutaway,
        focus: sims_render::Focus::default(),
    };
    assert_eq!(pick_unrotated(floor2), Some([19, 19]));
    assert!(native_zoom_or_rotate(
        &lot,
        &mut floor2,
        viewport,
        Move::Rotate(true)
    ));
    assert_eq!(pick_unrotated(floor2), Some([19, 19]));

    // Off-lot wrapped pick: on a flat 64x64 lot at 800x600 (collapsed),
    // Far -> Medium from native origin [381, -3997] (game [380, -3997])
    // wraps tile [269, 268] to signed bytes [13, 12] at 0x4a7af0 and
    // recenters to native origin [392, -117] (game [391, -117]).
    let mut wrapped = Camera {
        origin: [380, -3997],
        zoom: Zoom::Far,
        rotation: 0,
        level: 1,
        walls: Walls::Cutaway,
        focus: sims_render::Focus::default(),
    };
    assert!(native_zoom_or_rotate(
        &lot,
        &mut wrapped,
        [800, 600],
        Move::ZoomIn
    ));
    assert_eq!(
        [wrapped.origin[0] + pick::ORIGIN_COLUMN, wrapped.origin[1]],
        [392, -117]
    );

    // 16-bit screen wrap during zoom recentring (0x4a79fd..0x4a7a47): from
    // game origin [380, 61539] (whose low 16 bits wrap to -3997), Far ->
    // Medium projects with the current origin and clamps to [391, 232].
    let mut wrapped_high = Camera {
        origin: [380, 61539],
        zoom: Zoom::Far,
        rotation: 0,
        level: 1,
        walls: Walls::Cutaway,
        focus: sims_render::Focus::default(),
    };
    assert!(native_zoom_or_rotate(
        &lot,
        &mut wrapped_high,
        [800, 600],
        Move::ZoomIn
    ));
    assert_eq!(wrapped_high.origin, [391, 232]);

    // Wrapped lot position projection in `lot_point`: [424, 392, 1] and
    // [4520, 392, 1] (differing by 256 tiles = 4096 fine units) project
    // identically after signed-byte tile narrowing (0x4a9c5c..0x4a9c71).
    let near = Camera {
        origin: [0, 0],
        zoom: Zoom::Near,
        rotation: 0,
        level: 1,
        walls: Walls::Cutaway,
        focus: sims_render::Focus::default(),
    };
    assert_eq!(
        lot_point(&near, &lot, [424, 392, 1]),
        lot_point(&near, &lot, [4520, 392, 1])
    );
}

#[test]
fn ui_effect_highlights_outline_buttons_portraits_and_first_relationship_control() {
    let relation = |guid: u32, id: i16| sims_ui::household::Relation {
        guid,
        id,
        daily: 10,
        lifetime: 5,
        friends: false,
        pink_heart: false,
        red_heart: false,
        star: false,
        family: false,
    };
    let items = vec![
        sims_ui::hud::Item {
            rect: sims_ui::frame::Rect::new(2, 2, 10, 10),
            kind: sims_ui::hud::Kind::Button {
                id: sims_ui::hud::id::LIVE,
                cells: 4,
                selected: false,
            },
            command: None,
        },
        sims_ui::hud::Item {
            rect: sims_ui::frame::Rect::new(16, 2, 10, 10),
            kind: sims_ui::hud::Kind::Portrait {
                guid: 0x1111_2222,
                selected: false,
            },
            command: None,
        },
        sims_ui::hud::Item {
            rect: sims_ui::frame::Rect::new(30, 2, 10, 10),
            kind: sims_ui::hud::Kind::Relation {
                relation: relation(0xAAAA_0001, 1),
                lit: false,
            },
            command: None,
        },
        sims_ui::hud::Item {
            rect: sims_ui::frame::Rect::new(44, 2, 10, 10),
            kind: sims_ui::hud::Kind::Relation {
                relation: relation(0xAAAA_0002, 2),
                lit: false,
            },
            command: None,
        },
    ];

    // Empty highlight sets leave the canvas untouched.
    let mut canvas = Canvas::new(64, 20, [0, 0, 0]);
    draw_ui_effect_highlights(
        &items,
        &BTreeSet::new(),
        &BTreeSet::new(),
        false,
        &mut canvas,
    );
    assert!(canvas.color.iter().all(|&px| px == 0));

    // Active highlights draw the outline on the matching button, portrait,
    // and only the first relationship control (matching 0x4769ca).
    let buttons = BTreeSet::from([sims_ui::hud::id::LIVE as i16]);
    let portraits = BTreeSet::from([0x1111_2222u32]);
    draw_ui_effect_highlights(&items, &buttons, &portraits, true, &mut canvas);
    let pixel = |c: &Canvas, x: usize, y: usize| c.pixel_rgb(x, y);
    assert_eq!(pixel(&canvas, 2, 2), UI_EFFECT_HIGHLIGHT_RGB);
    assert_eq!(pixel(&canvas, 16, 2), UI_EFFECT_HIGHLIGHT_RGB);
    assert_eq!(pixel(&canvas, 30, 2), UI_EFFECT_HIGHLIGHT_RGB);
    assert_eq!(pixel(&canvas, 44, 2), [0, 0, 0]);
    // Interior pixels stay untouched.
    assert_eq!(pixel(&canvas, 6, 6), [0, 0, 0]);
}

#[test]
fn phone_book_eligibility_is_family_kind_and_caller_only() {
    // `cWinPhoneBook` never asks where a neighbor is (`0x522f17`), so
    // there is no presence input at all.
    use sims_sim::neighbors::{FAMILY, Family, KIND, Neighbor, PERSON_WORDS, Row};
    let record = |id: i16, family: i16, kind: i16| {
        let mut data = vec![0i16; PERSON_WORDS];
        data[FAMILY] = family;
        data[KIND] = kind;
        Neighbor {
            id,
            guid: 0x1000 + id as u32,
            revision: 1,
            data,
            relationships: Vec::new(),
            friends: 0,
            stale: false,
        }
    };
    let family = |id: i32, house: i32, members: &[i16]| Family {
        id,
        house,
        members: members.iter().map(|&m| 0x1000 + m as u32).collect(),
        friends: 0,
        stale: false,
    };
    let families = [
        family(2, 1, &[1]),
        family(3, 4, &[5, 7]),
        family(4, 0, &[6]),
        family(2001, 0, &[8]),
        family(0, 0, &[9]),
    ];
    let met = Row {
        target: 0,
        values: vec![10, 20],
    };
    let eligible = |r: &Neighbor, row: &Row| phonebook_eligible(r, 1, 2, &families, row);
    assert!(eligible(&record(5, 3, 0), &met), "a neighbor");
    assert!(!eligible(&record(5, 2, 0), &met), "the caller's family");
    assert!(!eligible(&record(9, 0, 0), &met), "familyless");
    assert!(!eligible(&record(5, 3, 2), &met), "a pet");
    assert!(!eligible(&record(1, 3, 0), &met), "the caller");
    assert!(
        !eligible(&record(6, 4, 0), &met),
        "a family below 2000 without a house"
    );
    assert!(
        eligible(&record(8, 2001, 0), &met),
        "a family from 2000 needs no house"
    );
    assert!(
        !eligible(&record(10, 3, 0), &met),
        "not a member of the family it names"
    );
    assert!(
        !eligible(
            &record(7, 3, 0),
            &Row {
                target: 0,
                values: Vec::new()
            }
        ),
        "a relationship row without values"
    );
}
