//! Volume, pan and pitch formulas, from the original's code.

use sims_audio::levels::{self, Inputs, Levels, UNITY};

#[test]
fn volumes_become_directsound_attenuations() {
    assert_eq!(levels::attenuation(0, 1024), -10_000);
    assert_eq!(levels::attenuation(1024, 1024), 0);
    assert_eq!(levels::attenuation(512, 1024), -2000);
    assert_eq!(levels::attenuation(1, 1024), -3997);
    // Clamped to 0..1024 before the master volume scales it.
    assert_eq!(levels::attenuation(5000, 1024), 0);
    assert_eq!(levels::attenuation(-5, 1024), -10_000);
    assert_eq!(levels::attenuation(1024, 512), -2000);
}

#[test]
fn pans_become_directsound_pans() {
    assert_eq!(levels::pan(512), 0);
    assert_eq!(levels::pan(0), -10_000);
    assert_eq!(levels::pan(1024), 10_000);
    assert_eq!(levels::pan(256), -5000);
    assert_eq!(levels::pan(-7), -10_000);
}

#[test]
fn attenuations_become_gains() {
    assert_eq!(levels::gain(0), UNITY);
    assert_eq!(levels::gain(-10_000), 0);
    assert_eq!(levels::gain(-2000), 6553); // -20 dB: a tenth
    assert_eq!(levels::gain(-600), 32_845); // -6 dB: about a half
    // A pan attenuates only the far side.
    assert_eq!(levels::gains(0, -10_000), [UNITY, 0]);
    assert_eq!(levels::gains(0, 2000), [6553, UNITY]);
    assert_eq!(levels::gains(-2000, 0), [6553, 6553]);
}

#[test]
fn pitch_sets_an_absolute_rate() {
    assert_eq!(levels::rate(3600), 22_050);
    assert_eq!(levels::rate(2400), 11_025);
    assert_eq!(levels::rate(4800), 44_100);
    assert_eq!(levels::rate(3700), 23_361); // a semitone up
    assert_eq!(levels::rate(3500), 20_812); // a semitone down
    assert_eq!(levels::rate(-100_000), 100);
    assert_eq!(levels::rate(i32::MAX), 100_000);
}

fn inputs() -> Inputs {
    Inputs {
        vol: 1024,
        extvol: 1024,
        pan: 512,
        pitch: 3600,
        duck_priority: 10,
        three_d: false,
        group_volume: 1024,
        main_duck_priority: 0,
        master: 1024,
    }
}

#[test]
fn volume_multiplies_vol_extvol_and_group_and_ducks_by_half() {
    assert_eq!(levels::levels(&inputs()), Levels::UNITY);
    let half = Inputs {
        vol: 512,
        ..inputs()
    };
    assert_eq!(levels::volume(&half), 512);
    let group = Inputs {
        extvol: 512,
        group_volume: 512,
        ..inputs()
    };
    assert_eq!(levels::volume(&group), 256);
    // A main duck priority above the track's halves it; one at or below
    // does not.
    let ducked = Inputs {
        main_duck_priority: 30,
        ..inputs()
    };
    assert_eq!(levels::volume(&ducked), 512);
    let not = Inputs {
        main_duck_priority: 10,
        ..inputs()
    };
    assert_eq!(levels::volume(&not), 1024);
    // Truncation toward zero, as the original's divisions.
    let odd = Inputs {
        vol: 3,
        main_duck_priority: 30,
        ..inputs()
    };
    assert_eq!(levels::volume(&odd), 1);
    let negative = Inputs {
        vol: -3,
        main_duck_priority: 30,
        ..inputs()
    };
    assert_eq!(levels::volume(&negative), -1);
    // A 3D track is centred whatever its pan.
    let three_d = Inputs {
        pan: 0,
        three_d: true,
        ..inputs()
    };
    assert_eq!(levels::levels(&three_d).gains, [UNITY, UNITY]);
}
