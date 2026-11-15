//! The mixer plays the install's sounds. Needs `SIMS_INSTALL`; skipped
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
