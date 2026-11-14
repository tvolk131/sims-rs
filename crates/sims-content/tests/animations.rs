//! Looking a clip up by name. Synthetic; no game data.

use sims_content::{Animations, Clip, Error};

fn clip(name: &str, duration: u32) -> Clip {
    Clip {
        name: name.into(),
        skill: "Skill".into(),
        duration,
        frames: 4,
        motions: 1,
        events: false,
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
