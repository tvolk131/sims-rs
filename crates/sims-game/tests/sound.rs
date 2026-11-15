//! The mixer plays the install's sounds, and the Goths' lot's. Needs `SIMS_INSTALL`; skipped
//! when it is unset. Nothing from the game is kept.

use sims_audio::SoundEvent;
use sims_audio::player::RATE;
use sims_game::sound::{Input, Mixer};
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
    };
    let mut game = sims_game::Game::open(&options).expect("the Goth house opens");
    let mut mixer = Mixer::open(&install).expect("the sound files");
    mixer.input(Input::Speed(Speed::Normal));
    let (mut played, mut loud, mut faults) = (Vec::new(), 0, Vec::new());
    // A second of the lot, then a second of its sound, as the window does.
    while game.run(20) > 0 {
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
        loud += mixer
            .render(RATE as usize)
            .iter()
            .filter(|&&s| s != 0)
            .count();
        faults.extend(mixer.new_faults());
    }
    assert!(played.iter().any(|n| n == "door_open"), "{played:?}");
    assert!(loud > 0, "the Goths' lot is silent");
    // The route sounds have no event in the install's sound files.
    assert!(
        faults
            .iter()
            .all(|f| f == "no event begin_route" || f == "no event end_route"),
        "{faults:?}"
    );
}
