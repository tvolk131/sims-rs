//! Looking a clip up by name. Synthetic; no game data.

use sims_content::{Animations, Clip, Error, Event, Track};
use sims_formats::animation::TimedProperty;

fn clip(name: &str, duration: u32) -> Clip {
    Clip {
        name: name.into(),
        skill: "Skill".into(),
        duration,
        distance: 0,
        moving: false,
        frames: 4,
        motions: 1,
        tracks: Vec::new(),
    }
}

fn refused(animations: &Animations, name: &str) -> String {
    match animations.clip(name) {
        Err(Error::Unsupported(message)) => message,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_clip_is_found_by_its_exact_name_with_its_samples() {
    let mut animations = Animations::default();
    assert!(animations.is_empty());
    animations.insert("a.far", clip("walk", 1));
    // The same definition in a second file is no conflict.
    animations.insert("b.far", clip("walk", 1));
    animations.insert_samples("skill");
    assert_eq!(animations.clip("walk").unwrap().duration, 1);
}

#[test]
fn a_clip_that_cannot_be_told_is_refused() {
    let mut animations = Animations::default();
    animations.insert("a.far", clip("walk", 1));
    assert!(refused(&animations, "walk").contains("samples"));
    animations.insert_samples("Skill");
    assert!(refused(&animations, "run").contains("no animation file"));
    // A name that differs only in case finds the clip ...
    assert_eq!(animations.clip("Walk").unwrap().name, "walk");
    // ... unless two names do.
    animations.insert("a.far", clip("WALK", 1));
    assert!(refused(&animations, "Walk").contains("only in case"));
    assert_eq!(animations.clip("WALK").unwrap().name, "WALK");
    animations.insert("b.far", clip("walk", 2));
    assert!(refused(&animations, "walk").contains("defined differently"));
}

fn timed(time_ms: i32, pairs: &[(&str, &str)]) -> TimedProperty {
    TimedProperty {
        time_ms,
        bone: "ROOT".into(),
        pairs: pairs.iter().map(|&(k, v)| (k.into(), v.into())).collect(),
    }
}

#[test]
fn a_motions_events_are_ordered_by_time_with_lowercase_properties() {
    // 0x59431a and 0x594390: a key is looked up as spelled and stored in
    // lowercase, so "XEVT" then "XEVT" adds two pairs, while "xevt" after
    // a stored "xevt" replaces its value. 0x5946fc: a second event at
    // the same time replaces the first's list.
    let track = Track::build(
        "ROOT",
        &[
            timed(30, &[("Sound", "Foo")]),
            timed(10, &[("XEVT", "1"), ("XEVT", "2")]),
            timed(20, &[("xevt", "1"), ("xevt", "3")]),
            timed(30, &[("LeftHand", "4")]),
        ],
    );
    let event = |time: i32, pairs: &[(&str, &str)]| Event {
        time,
        properties: pairs.iter().map(|&(k, v)| (k.into(), v.into())).collect(),
    };
    assert_eq!(track.bone, "ROOT");
    assert_eq!(
        track.events,
        [
            event(10, &[("xevt", "1"), ("xevt", "2")]),
            event(20, &[("xevt", "3")]),
            event(30, &[("lefthand", "4")]),
        ]
    );
}
