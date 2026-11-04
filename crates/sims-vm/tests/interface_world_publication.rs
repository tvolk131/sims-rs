use sims_vm::interface::{
    state::State,
    world_publication::{Inputs, Music, Request as R, Update, music},
};

#[test]
fn selected_person_request_preserves_historical_wire_tag_without_treating_it_as_audio() {
    let old = serde_json::json!({"SoundObject": 7});
    let r: R = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(r, R::SelectPersonIndex(7));
    assert_eq!(serde_json::to_value(r).unwrap(), old);
}

fn input() -> Inputs {
    Inputs {
        menu_present: false,
        menu_active: 0,
        closing: 0,
        include_visible: 1,
        zoom: 2,
        rotation: 3,
        floor: 2,
        walls: 4,
        tool: Some(20),
        tool_argument: 0xabcd1234,
        tool_gate: 0,
        current_tool: Some(10),
        selected_person_index: 77,
        lot_speed: 100,
        lot_kind: 0,
        lot_number: 1,
        zoning: 0,
    }
}

#[test]
fn full_world_publication_orders_all_dirty_branches_and_restores_each_child() {
    let mut s = State::constructed(760);
    s.mode = 2;
    let mut lot = 3;
    let mut f = Update::new(u32::MAX);
    let expected = [
        R::RoomMask(1),
        R::DrawingMode(0),
        R::ResumeAudio,
        R::Zoom(2),
        R::RefreshAudio,
        R::Rotation(3),
        R::Floor(2),
        R::SetSpeed(760),
        R::Resume,
        R::ResumeAudio,
        R::WallsDown(0),
        R::Cutaway(1),
        R::Roof(0),
        R::SelectTool(Some(20)),
        R::ActivateTool {
            tool: Some(20),
            argument_low: 0x1234,
        },
        R::SelectPersonIndex(77),
        R::StopMusic,
        R::PlayMusic(Music::Live),
    ];
    for (n, r) in expected.into_iter().enumerate() {
        assert_eq!(f.advance(&s, &mut lot, input()).unwrap(), Some(r));
        assert_eq!(lot, if n < 7 { 3 } else { 0 });
        let saved = serde_json::to_vec(&f).unwrap();
        f = serde_json::from_slice(&saved).unwrap();
        let before = f.clone();
        assert_eq!(f.advance(&s, &mut lot, input()).unwrap(), Some(r));
        assert_eq!(f, before);
        assert!(f.acknowledge(R::WindowReset).is_err());
        f.acknowledge(r).unwrap();
    }
    assert_eq!(f.advance(&s, &mut lot, input()).unwrap(), None);
    assert!(f.complete());
}

#[test]
fn dirty_caller_rereads_shared_fields_but_retains_wall_and_music_decisions() {
    let mut s = State::constructed(760);
    s.mode = 2;
    let mut i = input();
    let mut lot = 3;
    let mut f = Update::new(u32::MAX);
    let mut got = vec![];
    for _ in 0..30 {
        let Some(r) = f.advance(&s, &mut lot, i).unwrap() else {
            break;
        };
        got.push(r);
        match r {
            R::RoomMask(_) => {
                s.mode = 3;
                i.closing = 1;
            } // Skip drawing, but do not retest closing.
            R::PauseAudio => s.mode = 2,
            R::Zoom(_) => i.rotation = 1,
            R::Rotation(_) => i.floor = 1,
            R::Floor(_) => s.speed = 924,
            R::SetSpeed(_) => s.paused = 128,
            R::Pause => {
                s.paused = 0;
                s.mode = 1;
            } // Keep the captured audio branch.
            R::WallsDown(_) => i.walls = 3, // Keep captured cutaway/roof values.
            R::SelectTool(_) => {
                i.tool = Some(30);
                i.tool_argument = 0xffff8001;
                i.tool_gate = 1;
            }
            R::SelectPersonIndex(_) => s.mode = 4,
            R::StopMusic => s.mode = 3,
            R::PlayMusic(_) => s.mode = 2, // Still execute the captured Options tail.
            _ => (),
        }
        f.acknowledge(r).unwrap();
    }
    assert!(f.complete());
    assert_eq!(
        got,
        vec![
            R::RoomMask(1),
            R::PauseAudio,
            R::Zoom(2),
            R::RefreshAudio,
            R::Rotation(1),
            R::Floor(1),
            R::SetSpeed(924),
            R::Pause,
            R::PauseAudio,
            R::WallsDown(0),
            R::Cutaway(1),
            R::Roof(0),
            R::SelectTool(Some(20)),
            R::ActivateTool {
                tool: Some(30),
                argument_low: 0x8001
            },
            R::SelectPersonIndex(77),
            R::StopMusic,
            R::PlayMusic(Music::Options),
            R::WindowReset
        ]
    );
    assert_eq!(lot, 0);
}

#[test]
fn full_publication_matches_existing_live_and_transport_scopes() {
    use sims_vm::interface::{self, transport};
    for menu in [false, true] {
        for active in [0, 1] {
            for closing in [0, 1] {
                for paused in [0, 128] {
                    let mut s = State::constructed(760);
                    s.mode = 2;
                    s.paused = paused;
                    let mut i = input();
                    i.menu_present = menu;
                    i.menu_active = active;
                    i.closing = closing;
                    let mut full = Update::new(2);
                    let mut lot = 9;
                    let mut actual = vec![];
                    while let Some(r) = full.advance(&s, &mut lot, i).unwrap() {
                        actual.push(r);
                        full.acknowledge(r).unwrap();
                    }
                    let mut legacy = interface::LiveModeUpdate::default();
                    let mut expected = vec![];
                    while let Some(r) = legacy
                        .advance(interface::Inputs {
                            menu_present: menu,
                            menu_active: active,
                            closing,
                            mode: 2,
                            paused,
                            include_visible: 1,
                        })
                        .unwrap()
                    {
                        use interface::Request as L;
                        match r {
                            L::RoomMask(v) => expected.push(R::RoomMask(v)),
                            L::DrawingMode => expected.push(R::DrawingMode(0)),
                            L::ResumeAudio => expected.push(R::ResumeAudio),
                            L::PublishLiveMode => assert_eq!(lot, 0),
                            L::StopAudio => expected.push(R::StopMusic),
                            L::PlayLiveBackground => expected.push(R::PlayMusic(Music::Live)),
                        }
                        legacy.acknowledge(&r).unwrap();
                    }
                    assert_eq!(actual, expected);
                    for mode in [-1, 0, 1, 2, 3, 4, 5] {
                        for mask in [0, 0x40, 0x800000, 0x800040] {
                            s.mode = mode;
                            let mut full = Update::new(mask);
                            let mut legacy = transport::Update::new(mask).unwrap();
                            let (mut lot_a, mut lot_b) = (9, 9);
                            loop {
                                let a = full.advance(&s, &mut lot_a, i).unwrap();
                                let b = legacy
                                    .advance(
                                        &s,
                                        &mut lot_b,
                                        transport::Inputs {
                                            menu_present: menu,
                                            menu_active: active,
                                            closing,
                                            lot_speed: 100,
                                        },
                                    )
                                    .unwrap();
                                use transport::UpdateRequest as L;
                                assert_eq!(
                                    a,
                                    b.map(|r| match r {
                                        L::SetSpeed(v) => R::SetSpeed(v),
                                        L::Pause => R::Pause,
                                        L::Resume => R::Resume,
                                        L::PauseAudio => R::PauseAudio,
                                        L::ResumeAudio => R::ResumeAudio,
                                    })
                                );
                                assert_eq!(lot_a, lot_b);
                                match (a, b) {
                                    (Some(a), Some(b)) => {
                                        full.acknowledge(a).unwrap();
                                        legacy.acknowledge(b).unwrap();
                                    }
                                    (None, None) => break,
                                    _ => unreachable!(),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn tool_null_same_owner_lock_and_music_gate_follow_distinct_native_paths() {
    let mut s = State::constructed(760);
    s.mode = 2;
    for (tool, current, gate, expected) in [
        (None, None, 0, vec![R::SelectTool(None)]),
        (
            Some(20),
            Some(20),
            0,
            vec![R::ActivateTool {
                tool: Some(20),
                argument_low: 0x1234,
            }],
        ),
        (Some(20), Some(10), 1, vec![]),
    ] {
        let mut i = input();
        i.tool = tool;
        i.current_tool = current;
        i.tool_gate = gate;
        let mut f = Update::new(0x630);
        let mut lot = 3;
        let mut actual = vec![];
        while let Some(r) = f.advance(&s, &mut lot, i).unwrap() {
            actual.push(r);
            f.acknowledge(r).unwrap();
        }
        assert_eq!(actual, expected);
    }
    let mut i = input();
    let mut f = Update::new(0x630);
    let mut lot = 3;
    let r = f.advance(&s, &mut lot, i).unwrap().unwrap();
    f.acknowledge(r).unwrap();
    i.tool = None;
    assert_eq!(
        f.advance(&s, &mut lot, i).unwrap(),
        Some(R::ActivateTool {
            tool: None,
            argument_low: 0x1234
        })
    );
    let mut f = Update::new(2 | 8);
    let mut i = input();
    let mut seen = vec![];
    while let Some(r) = f.advance(&s, &mut lot, i).unwrap() {
        seen.push(r);
        if matches!(r, R::SelectPersonIndex(_)) {
            i.menu_present = true;
            i.menu_active = 0;
        }
        f.acknowledge(r).unwrap();
    }
    assert!(!seen.contains(&R::StopMusic));
}

#[test]
fn expansion_music_uses_original_precedence_and_signed_ranges() {
    for (lot, zoning, want) in [
        (39, 0, Music::Downtown),
        (40, 2, Music::Vacation),
        (49, 0, Music::Vacation),
        (50, 2, Music::Unleashed),
        (81, 2, Music::Unleashed),
        (81, 0, Music::Superstar),
        (89, 0, Music::Superstar),
        (90, 0, Music::Downtown),
        (93, 2, Music::MagicTown),
        (98, 0, Music::MagicTown),
        (99, 2, Music::Unleashed),
        (i16::MIN, 0, Music::Downtown),
        (i16::MAX, 0, Music::Downtown),
    ] {
        assert_eq!(music(1, 1, lot, zoning), Some(want));
    }
    for lot in [89, 90, 92, 93] {
        assert_eq!(
            music(1, 0, lot, 2),
            Some(if (90..=92).contains(&lot) {
                Music::MagicBuy
            } else {
                Music::Buy
            })
        );
        assert_eq!(
            music(0, 0, lot, 2),
            Some(if (90..=92).contains(&lot) {
                Music::MagicBuild
            } else {
                Music::Build
            })
        );
    }
    assert_eq!(music(0, 1, 1, 0), None);
    assert_eq!(music(-1, 0, 1, 0), None);
    assert_eq!(music(4, 1, 1, 0), Some(Music::Camera));
}
