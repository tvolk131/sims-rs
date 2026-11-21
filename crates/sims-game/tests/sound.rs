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

    // Pausing the lot queues SoundEvent::Pause (0x4ce5c5), holding
    // `nite_loop` and pausing backyard ambience. `MainMusicLoop` is spared
    // by Pause (`PAUSE_SPARES`), so drain the remainder of `bkground_live`'s
    // slow fade-out (`fade_speed = 3` per 20 ms tick ≈ 6.84 s) before
    // asserting silence.
    game.toggle_pause();
    assert!(game.paused());
    sent.clear();
    feed.update(&mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.contains(&Input::Events(vec![SoundEvent::Pause])));
    let _ = mixer.render(7 * RATE as usize);
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
    let changed = feed.set_focused(true, &mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(!changed);
    assert!(sent.is_empty(), "initial Focused(true) must be a no-op");

    // First feed enters live mode and starts backyard ambience.
    feed.update(&mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(!sent.contains(&Input::Active(true)));
    assert!(!sent.contains(&Input::Active(false)));
    assert!(!game.paused());
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

    // Queue a camera move right before losing focus: focus loss must flush
    // the prior camera move BEFORE sending Input::Active(false), then
    // auto-pause the lot (0x4f7635..0x4f765a, byte [0x6c9fa1] == 1) without
    // sending SoundEvent::Pause yet (deferred to next update at 0x4ce5c5).
    game.apply(Move::Scroll([8, 0]));
    sent.clear();
    let changed = feed.set_focused(false, &mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(changed, "unfocusing an unpaused live lot should pause it");
    assert!(game.paused());
    assert!(!feed.focused());
    assert_eq!(sent.last(), Some(&Input::Active(false)));
    assert!(
        sent.iter().position(|i| *i == Input::Camera) < Some(sent.len() - 1),
        "prior camera move must flush before Input::Active(false): {sent:?}"
    );
    assert!(
        !sent.contains(&Input::Events(vec![SoundEvent::Pause])),
        "SoundEvent::Pause must wait for the next update (0x4ce5c5): {sent:?}"
    );

    // Duplicate Focused(false) is ignored.
    sent.clear();
    assert!(!feed.set_focused(false, &mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    }));
    assert!(sent.is_empty(), "duplicate Focused(false) must be ignored");

    // Next update forwards the pending SoundEvent::Pause (0x4ce5c5).
    sent.clear();
    feed.update(&mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.contains(&Input::Events(vec![SoundEvent::Pause])));

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

    // Regaining focus restores the saved unpaused state (0x4f765c..0x4f7666)
    // and sends Input::Active(true) immediately (0x4f76d5); duplicate true
    // is ignored. SoundEvent::Unpause then arrives on the next update
    // (0x4ce5d9), resuming live ambience.
    sent.clear();
    let changed = feed.set_focused(true, &mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(changed, "refocusing should restore unpaused state");
    assert!(!game.paused());
    assert!(feed.focused());
    assert_eq!(sent.last(), Some(&Input::Active(true)));
    assert!(
        !sent.contains(&Input::Events(vec![SoundEvent::Unpause])),
        "SoundEvent::Unpause must wait for the next update (0x4ce5d9): {sent:?}"
    );
    sent.clear();
    assert!(!feed.set_focused(true, &mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    }));
    assert!(sent.is_empty(), "duplicate Focused(true) must be ignored");

    sent.clear();
    feed.update(&mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.contains(&Input::Events(vec![SoundEvent::Unpause])));
    assert!(
        mixer.render(4 * RATE as usize).iter().any(|&s| s != 0),
        "regaining focus should resume live ambience"
    );

    // Focus loss followed immediately by focus regain (with no tick in
    // between) restores `paused = false` and still marks pause dirty
    // (`0x4cee4a` sets bit `0x800000` unconditionally), so the next update
    // dispatches `SoundEvent::Unpause` (`0x4ce5d9`).
    assert!(feed.set_focused(false, &mut game, |input| mixer.input(input)));
    assert!(game.paused());
    assert!(feed.set_focused(true, &mut game, |input| mixer.input(input)));
    assert!(!game.paused());
    sent.clear();
    feed.update(&mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.contains(&Input::Events(vec![SoundEvent::Unpause])));
    assert!(!sent.contains(&Input::Events(vec![SoundEvent::Pause])));

    // Settled focus-loss pause followed by rapid refocus + blur (with no
    // update in between): must send Active(true) then Active(false) WITHOUT
    // prematurely emitting SoundEvent::Unpause on the second blur, and the
    // next update dispatches only SoundEvent::Pause.
    assert!(feed.set_focused(false, &mut game, |input| mixer.input(input)));
    feed.update(&mut game, |input| mixer.input(input));
    sent.clear();
    assert!(feed.set_focused(true, &mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    }));
    assert!(feed.set_focused(false, &mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    }));
    assert_eq!(sent, [Input::Active(true), Input::Active(false)]);
    sent.clear();
    feed.update(&mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.contains(&Input::Events(vec![SoundEvent::Pause])));
    assert!(!sent.contains(&Input::Events(vec![SoundEvent::Unpause])));
    // Restore focus for the next check.
    assert!(feed.set_focused(true, &mut game, |input| mixer.input(input)));
    feed.update(&mut game, |input| mixer.input(input));

    // Manual unpause followed immediately by focus loss before `feed.update`:
    // both write the same deferred pause flag (`0x4cee52`..`0x4cee5d`), so
    // `set_focused(false)` must NOT emit `Unpause` before `Active(false)` and
    // the next update emits only `Pause`; later refocus restores unpaused.
    game.toggle_pause();
    feed.update(&mut game, |input| mixer.input(input));
    assert!(game.paused());
    game.toggle_pause(); // manual unpause, not yet flushed to feed
    assert!(!game.paused());
    sent.clear();
    assert!(feed.set_focused(false, &mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    }));
    assert!(game.paused());
    assert_eq!(sent, [Input::Active(false)]);
    sent.clear();
    feed.update(&mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.contains(&Input::Events(vec![SoundEvent::Pause])));
    assert!(!sent.contains(&Input::Events(vec![SoundEvent::Unpause])));
    assert!(feed.set_focused(true, &mut game, |input| mixer.input(input)));
    assert!(!game.paused());
    feed.update(&mut game, |input| mixer.input(input));

    // Focus cycling while the lot is ALREADY paused saves saved_paused=true
    // and keeps the lot and live ambience paused after refocusing (still
    // dispatching `SoundEvent::Pause` for the dirty pause bit at `0x4ce5c5`).
    game.toggle_pause();
    feed.update(&mut game, |input| mixer.input(input));
    let _ = mixer.render(6 * RATE as usize);
    assert!(!feed.set_focused(false, &mut game, |input| mixer.input(input)));
    assert!(game.paused());
    assert!(!feed.set_focused(true, &mut game, |input| mixer.input(input)));
    assert!(game.paused());
    sent.clear();
    feed.update(&mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.contains(&Input::Events(vec![SoundEvent::Pause])));
    assert!(
        mixer.render(4 * RATE as usize).iter().all(|&s| s == 0),
        "regaining focus while paused must keep live ambience paused"
    );
    assert!(mixer.new_faults().is_empty());

    // Focus loss while an unanswered modal dialog refuses `game.toggle_pause()`
    // still records requested pause = true (`0x4cee52`..`0x4cee5d`) and
    // dispatches `SoundEvent::Pause` (`0x4ce5b0`..`0x4ce5c5`) rather than
    // `Unpause`, keeps audio paused across further updates while unfocused,
    // and dispatches `SoundEvent::Unpause` on focus regain.
    let modal_options = sims_game::Options {
        house: sims_game::game::goth_house(&install),
        install: install.clone(),
        answers: Vec::new(),
        play: false,
        frames: sims_game::Frames::default(),
    };
    let mut modal_game = sims_game::Game::open(&modal_options).expect("the Goth house opens");
    let _ = modal_game.run(2); // opens the initial Help dialog and waits for an answer
    assert!(!modal_game.paused());
    let mut modal_feed = sims_game::sound::Feed::default();
    modal_feed.update(&mut modal_game, |_| {});
    let mut modal_sent = Vec::new();
    assert!(!modal_feed.set_focused(false, &mut modal_game, |input| modal_sent.push(input)));
    assert!(!modal_game.paused()); // refused while modal dialog waits
    assert_eq!(modal_sent, [Input::Active(false)]);
    modal_sent.clear();
    modal_feed.update(&mut modal_game, |input| modal_sent.push(input));
    assert!(modal_sent.contains(&Input::Events(vec![SoundEvent::Pause])));
    assert!(!modal_sent.contains(&Input::Events(vec![SoundEvent::Unpause])));
    // Subsequent updates while still unfocused must not spuriously unpause.
    modal_sent.clear();
    modal_feed.update(&mut modal_game, |input| modal_sent.push(input));
    assert!(!modal_sent.contains(&Input::Events(vec![SoundEvent::Unpause])));
    // Regaining focus dispatches Active(true), then Unpause on the next update.
    modal_sent.clear();
    assert!(!modal_feed.set_focused(true, &mut modal_game, |input| modal_sent.push(input)));
    assert_eq!(modal_sent, [Input::Active(true)]);
    modal_sent.clear();
    modal_feed.update(&mut modal_game, |input| modal_sent.push(input));
    assert!(modal_sent.contains(&Input::Events(vec![SoundEvent::Unpause])));
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
