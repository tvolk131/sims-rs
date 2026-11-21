//! The mixer plays the install's sounds, and the Goths' lot's. Needs `SIMS_INSTALL`; skipped
//! when it is unset. Nothing from the game is kept.

use sims_audio::SoundEvent;
use sims_audio::player::RATE;
use sims_game::sound::{Input, Mixer};
use sims_game::view::Move;
use sims_render::Walls;
use sims_ui::command::Speed;

#[test]
fn an_event_plays_and_the_inputs_are_taken() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL is not set; skipping the sound test");
        return;
    };
    let mut mixer = Mixer::open(std::path::Path::new(&root)).expect("the sound files");
    // Quiet before anything plays.
    assert!(mixer.render(512).iter().all(|&s| s == 0));
    mixer.input(Input::Hour(12));
    mixer.input(Input::Speed(Speed::Normal));
    mixer.input(Input::Events(vec![SoundEvent::Play {
        object: 1,
        name: "Phone_ring".into(),
    }]));
    let second = mixer.render(RATE as usize);
    assert_eq!(second.len(), 2 * RATE as usize);
    assert!(second.iter().any(|&s| s != 0), "the ring is silent");
    assert!(mixer.new_faults().is_empty());
    mixer.input(Input::Events(vec![SoundEvent::StopAll]));
}

#[test]
fn hour_change_with_still_camera_updates_the_night_loop() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL is not set; skipping the sound test");
        return;
    };
    let mut mixer = Mixer::open(std::path::Path::new(&root)).expect("the sound files");
    let snap = |hour: i32| {
        let mut s = sims_audio::sources::Snapshot::default();
        s.absent[4] = hour;
        s.lot = 5;
        s
    };
    // Enter live mode via the same helper the window sends on launch
    // (`sound::enter_live`), then stop the ambience score and
    // neighbourhood music with StopAll so every sound heard comes solely
    // from `nite_loop`.
    mixer.input(Input::Sources(snap(18)));
    mixer.input(Input::View(0));
    mixer.input(Input::Hour(18));
    let mut events = sims_game::sound::enter_live();
    events.push(SoundEvent::StopAll);
    mixer.input(Input::Events(events));
    mixer.input(Input::Camera);
    assert!(mixer.render(RATE as usize).iter().all(|&s| s == 0));

    // Clock crosses 19:00 with NO Input::Camera (stationary camera):
    // Input::Hour runs kind 33 (0x46d4d3 -> 0x4f66bc -> 0x57aac5) and
    // starts `nite_loop`.
    mixer.input(Input::Sources(snap(19)));
    mixer.input(Input::Hour(19));
    let night = mixer.render(RATE as usize);
    assert!(
        night.iter().any(|&s| s != 0),
        "nite_loop should start at 19:00 without moving the camera"
    );

    // Clock crosses 06:00 with NO Input::Camera: `nite_loop` fades out.
    mixer.input(Input::Sources(snap(6)));
    mixer.input(Input::Hour(6));
    let _ = mixer.render(RATE as usize);
    assert!(
        mixer.render(RATE as usize).iter().all(|&s| s == 0),
        "nite_loop should stop at 06:00 without moving the camera"
    );
    assert!(mixer.new_faults().is_empty());
}

#[test]
fn pausing_the_lot_pauses_live_ambience_and_night_loop() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL is not set; skipping the sound test");
        return;
    };
    let install = std::path::PathBuf::from(root);
    let options = sims_game::Options {
        house: sims_game::game::goth_house(&install),
        install: install.clone(),
        answers: vec![3],
        play: false,
        frames: sims_game::Frames::default(),
    };
    let mut game = sims_game::Game::open(&options).expect("the Goth house opens");
    let mut mixer = Mixer::open(&install).expect("the sound files");
    let mut feed = sims_game::sound::Feed::default();

    // First feed enters live mode (bkground_nhood1, setambience_backyard,
    // bkground_live) and sends the initial sources, view, hour and camera.
    let mut sent = Vec::new();
    feed.update(&mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.contains(&Input::Events(sims_game::sound::enter_live())));
    assert!(!game.paused());

    // Simulate night hour on the live lot so `nite_loop` joins backyard
    // ambience, and confirm audio is playing before pausing.
    let mut night = game.sound_sources();
    night.absent[4] = 22;
    mixer.input(Input::Sources(night));
    mixer.input(Input::Hour(22));
    assert!(
        mixer.render(RATE as usize).iter().any(|&s| s != 0),
        "live ambience and nite_loop should play before pausing"
    );

    // Pausing the lot queues SoundEvent::Pause (0x4ce5c5), stopping
    // `nite_loop` and pausing backyard ambience.
    game.toggle_pause();
    assert!(game.paused());
    sent.clear();
    feed.update(&mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.contains(&Input::Events(vec![SoundEvent::Pause])));
    let _ = mixer.render(RATE as usize);
    assert!(
        mixer.render(RATE as usize).iter().all(|&s| s == 0),
        "pausing the lot should silence live ambience and nite_loop"
    );

    // Unpausing via toggle_pause queues SoundEvent::Unpause (0x4ce5d9),
    // resuming live ambience immediately. (`nite_loop` was faded and then
    // held by Pause, so Unpause lets its fade finish as 0x574a00 still
    // sees it running; a subsequent camera move then restarts it.)
    game.toggle_pause();
    assert!(!game.paused());
    sent.clear();
    feed.update(&mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.contains(&Input::Events(vec![SoundEvent::Unpause])));
    assert!(
        mixer.render(RATE as usize).iter().any(|&s| s != 0),
        "unpausing should resume live ambience"
    );

    // After the held pause fade finishes, stopping ambience isolates
    // `nite_loop` so we can verify that kind 33 (Camera) restarts it.
    mixer.input(Input::Events(vec![SoundEvent::StopAll]));
    assert!(mixer.render(RATE as usize).iter().all(|&s| s == 0));
    mixer.input(Input::Camera);
    assert!(
        mixer.render(RATE as usize).iter().any(|&s| s != 0),
        "kind 33 after unpause should restart nite_loop"
    );

    // Pausing and then picking a speed (which unpauses the lot) also
    // forwards Pause then Unpause.
    game.toggle_pause();
    feed.update(&mut game, |input| mixer.input(input));
    game.set_speed(Speed::Fast);
    assert!(!game.paused());
    sent.clear();
    feed.update(&mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.contains(&Input::Events(vec![SoundEvent::Unpause])));
    assert!(mixer.new_faults().is_empty());
}

#[test]
fn window_focus_changes_forward_kind_25_and_deduplicate_initial_focus() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL is not set; skipping the sound test");
        return;
    };
    let install = std::path::PathBuf::from(root);
    let options = sims_game::Options {
        house: sims_game::game::goth_house(&install),
        install: install.clone(),
        answers: vec![3],
        play: false,
        frames: sims_game::Frames::default(),
    };
    let mut game = sims_game::Game::open(&options).expect("the Goth house opens");
    let mut mixer = Mixer::open(&install).expect("the sound files");
    let mut feed = sims_game::sound::Feed::default();

    // Startup matches 0x4eee14 + 0x4f7610: `focused` begins `true`, so an
    // initial `Focused(true)` from the windowing system sends no kind 25.
    assert!(feed.focused());
    let mut sent = Vec::new();
    feed.set_focused(true, &mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.is_empty(), "initial Focused(true) must be a no-op");

    // First feed enters live mode and starts backyard ambience.
    feed.update(&mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(!sent.contains(&Input::Active(true)));
    assert!(!sent.contains(&Input::Active(false)));
    // Set outdoor view and give the ambience score a few beats to start
    // playing cells at hour 12 (daytime, so audio comes purely from
    // backyard ambience, not `nite_loop`).
    let mut noon = game.sound_sources();
    noon.absent[4] = 12;
    mixer.input(Input::Sources(noon));
    mixer.input(Input::View(100));
    mixer.input(Input::Hour(12));
    assert!(
        mixer.render(4 * RATE as usize).iter().any(|&s| s != 0),
        "live ambience should play while focused"
    );

    // Losing window focus sends Input::Active(false) (kind 25 off), pausing
    // live ambience and skipping new kind 1 plays; a duplicate false is
    // deduplicated.
    sent.clear();
    feed.set_focused(false, &mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(!feed.focused());
    assert_eq!(sent, [Input::Active(false)]);
    sent.clear();
    feed.set_focused(false, &mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.is_empty(), "duplicate Focused(false) must be ignored");

    // Let any already-started ambience cell samples finish, then confirm
    // no new ambience cells or kind 1 plays sound while unfocused.
    let _ = mixer.render(6 * RATE as usize);
    mixer.input(Input::Events(vec![SoundEvent::Play {
        object: 1,
        name: "Phone_ring".into(),
    }]));
    assert!(
        mixer.render(2 * RATE as usize).iter().all(|&s| s == 0),
        "unfocused sound should keep ambience paused and skip kind 1 plays"
    );

    // Regaining focus sends Input::Active(true) (kind 25 on), resuming live
    // ambience and allowing kind 1 plays again; duplicate true is ignored.
    sent.clear();
    feed.set_focused(true, &mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(feed.focused());
    assert_eq!(sent, [Input::Active(true)]);
    sent.clear();
    feed.set_focused(true, &mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.is_empty(), "duplicate Focused(true) must be ignored");
    assert!(
        mixer.render(4 * RATE as usize).iter().any(|&s| s != 0),
        "regaining focus should resume live ambience"
    );

    // Focus cycling while the lot is paused does not unpause live ambience.
    game.toggle_pause();
    feed.update(&mut game, |input| mixer.input(input));
    let _ = mixer.render(6 * RATE as usize);
    feed.set_focused(false, &mut game, |input| mixer.input(input));
    feed.set_focused(true, &mut game, |input| mixer.input(input));
    assert!(
        mixer.render(4 * RATE as usize).iter().all(|&s| s == 0),
        "regaining focus while paused must keep live ambience paused"
    );
    assert!(mixer.new_faults().is_empty());
}

#[test]
fn the_goths_sounds_play() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL is not set; skipping the Goth house sound test");
        return;
    };
    let install = std::path::PathBuf::from(root);
    let options = sims_game::Options {
        house: sims_game::game::goth_house(&install),
        install: install.clone(),
        answers: vec![3],
        play: false,
        frames: sims_game::Frames::default(),
    };
    let mut game = sims_game::Game::open(&options).expect("the Goth house opens");
    // The walls leave every object where it is on the screen, so they
    // send no kind 33; a scroll does.
    let shot = game.shot();
    game.apply(Move::Walls(Walls::Down));
    assert_eq!(game.shot(), shot);
    let mut mixer = Mixer::open(&install).expect("the sound files");
    mixer.input(Input::Speed(Speed::Normal));
    let (mut played, mut loud, mut faults) = (Vec::new(), 0, Vec::new());
    // A second of the lot, then a second of its sound, as the window does;
    // the camera scrolls a little each second, so every track is placed
    // again (kind 33).
    // The fountain (object 197) plays "fountain_tinkle" every 300
    // updates; its track 1346 runs for ever, so each play after the first
    // starts nothing, as in the original.
    while game.run(20) > 0 {
        game.apply(Move::Scroll([8, 0]));
        assert_ne!(game.shot(), shot);
        mixer.input(Input::Sources(game.sound_sources()));
        let sounds = game.take_sounds();
        for event in &sounds {
            if let sims_vm::SoundEvent::Play { name, .. } = event {
                played.push(name.clone());
            }
        }
        mixer.input(Input::Hour(game.hour()));
        mixer.input(Input::Events(
            sounds.into_iter().map(sims_game::sound::event).collect(),
        ));
        mixer.input(Input::View(game.ambience_view()));
        mixer.input(Input::Camera);
        loud += mixer
            .render(RATE as usize)
            .iter()
            .filter(|&&s| s != 0)
            .count();
        faults.extend(mixer.new_faults());
    }
    assert!(played.iter().any(|n| n == "door_open"), "{played:?}");
    assert!(loud > 0, "the Goths' lot is silent");
    // The route sounds have no event in the install's sound files, and
    // SimsGeneratedHitSource.hot spells the rejected-tickler event
    // `vox_ticklee_rejected` while the interaction's FWAV asks for
    // `vox_tickler_rejected`.
    assert!(
        faults.iter().all(|f| {
            matches!(
                f.as_str(),
                "no event begin_route" | "no event end_route" | "no event vox_tickler_rejected"
            )
        }),
        "{faults:?}"
    );
}
