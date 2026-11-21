//! `sims_vm::boot::save` on a real install, when `SIMS_INSTALL` names one:
//! a lot restored from a save steps exactly as the saved lot went on to.

use sims_vm::boot::save::{SaveError, Saved};
use sims_vm::boot::{Frames, Lot, OrderError, StepError};
use std::path::PathBuf;

fn goth_house() -> Option<(PathBuf, PathBuf)> {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return None;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    Some((root, house))
}

#[test]
fn a_restored_goth_house_steps_as_the_saved_one_went_on() {
    let Some((root, house)) = goth_house() else {
        return;
    };
    // Frames follow the ticks run since the load (every 30th from 33): the
    // save falls between two frames, so the restored lot must go on from
    // the saved count.
    let mut saved_lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    for _ in 0..500 {
        saved_lot.step().unwrap();
    }
    let saved = saved_lot.save().unwrap();
    let bytes = saved.encode();
    let mut hashes = Vec::new();
    for _ in 500..1000 {
        saved_lot.step().unwrap();
        hashes.push(saved_lot.world().hash());
    }

    // Loaded with no answers: the save's are used.
    let mut restored = Lot::load(&root, &house, &[], Frames::default()).unwrap();
    restored.restore(Saved::decode(&bytes).unwrap()).unwrap();
    assert_eq!(restored.ticks(), 500);
    for (tick, hash) in (501..=1000).zip(hashes) {
        restored.step().unwrap();
        assert_eq!(restored.ticks(), tick);
        assert_eq!(restored.world().hash(), hash, "tick {tick}");
    }
}

#[test]
fn a_lot_saved_while_asking_asks_again_when_restored() {
    let Some((root, house)) = goth_house() else {
        return;
    };
    let mut listed = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    let mut asked = Lot::load(&root, &house, &[], Frames::default()).unwrap();
    asked.wait_for_answers();
    listed.step().unwrap();
    asked.step().unwrap();
    // Tick 2 shows the Help System dialog.
    assert_eq!(asked.step().unwrap_err(), StepError::Asking { tick: 2 });
    let saved = Saved::decode(&asked.save().unwrap().encode()).unwrap();
    assert_eq!(saved.world().hash(), asked.world().hash());

    let mut restored = Lot::load(&root, &house, &[], Frames::default()).unwrap();
    restored.wait_for_answers();
    restored.restore(saved).unwrap();
    assert_eq!(restored.ticks(), 1);
    assert_eq!(restored.step().unwrap_err(), StepError::Asking { tick: 2 });
    restored.answer(3).unwrap();
    for tick in 2..=100 {
        listed.step().unwrap();
        restored.step().unwrap();
        assert_eq!(
            restored.world().hash(),
            listed.world().hash(),
            "tick {tick}"
        );
    }
}

#[test]
fn a_lot_saved_once_answered_takes_no_input_until_its_step() {
    let Some((root, house)) = goth_house() else {
        return;
    };
    let mut asked = Lot::load(&root, &house, &[], Frames::default()).unwrap();
    asked.wait_for_answers();
    asked.step().unwrap();
    assert_eq!(asked.step().unwrap_err(), StepError::Asking { tick: 2 });
    asked.answer(3).unwrap();
    let saved = Saved::decode(&asked.save().unwrap().encode()).unwrap();

    let mut restored = Lot::load(&root, &house, &[], Frames::default()).unwrap();
    restored.wait_for_answers();
    restored.restore(saved).unwrap();
    // As the saved lot: the answer is given and the tick has not run
    // again, so no order is taken.
    assert_eq!(restored.pause().unwrap_err(), OrderError::Asking);
    assert_eq!(asked.pause().unwrap_err(), OrderError::Asking);
    for tick in 2..=100 {
        asked.step().unwrap();
        restored.step().unwrap();
        assert_eq!(restored.world().hash(), asked.world().hash(), "tick {tick}");
    }
}

#[test]
fn an_answered_save_takes_input_after_its_step_in_a_lot_that_does_not_wait() {
    let Some((root, house)) = goth_house() else {
        return;
    };
    let mut asked = Lot::load(&root, &house, &[], Frames::default()).unwrap();
    asked.wait_for_answers();
    asked.step().unwrap();
    assert_eq!(asked.step().unwrap_err(), StepError::Asking { tick: 2 });
    asked
        .answer(sims_vm::Answer {
            status: 3,
            bitmap_id: Some(17),
        })
        .unwrap();
    let saved = Saved::decode(&asked.save().unwrap().encode()).unwrap();

    // Restored into a lot that does not wait: the saved answer is given at
    // the tick, and the lot then takes input.
    let mut restored = Lot::load(&root, &house, &[], Frames::default()).unwrap();
    restored.restore(saved).unwrap();
    assert_eq!(restored.pause().unwrap_err(), OrderError::Asking);
    restored.step().unwrap();
    asked.step().unwrap();
    assert_eq!(restored.world().globals[12], 17);
    assert_eq!(restored.world().hash(), asked.world().hash());
    restored.pause().unwrap();
}

#[test]
fn a_save_goes_back_only_into_the_house_it_came_from() {
    let Some((root, house)) = goth_house() else {
        return;
    };
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    lot.step().unwrap();
    let saved = lot.save().unwrap();
    let other = root.join("UserData/Houses/House07.iff");
    let mut other_lot = Lot::load(&root, &other, &[3], Frames::default()).unwrap();
    let before = other_lot.world().hash();
    let refused = other_lot.restore(saved).unwrap_err();
    assert!(
        matches!(refused, SaveError::OtherHouse { saved: (5, _), .. }),
        "{refused}"
    );
    assert_eq!(other_lot.world().hash(), before);
}

#[test]
fn a_halted_lot_is_not_saved() {
    let Some((root, house)) = goth_house() else {
        return;
    };
    // With no answers and not waiting for them, tick 2's dialog halts it.
    let mut lot = Lot::load(&root, &house, &[], Frames::default()).unwrap();
    lot.step().unwrap();
    lot.step().unwrap_err();
    assert_eq!(lot.save().unwrap_err(), SaveError::Halted);
}

#[test]
fn bytes_of_another_layout_are_refused() {
    assert!(matches!(Saved::decode(&[1, 0]), Err(SaveError::Format(_))));
    // A save of the layout before this one (format 2 had no view centre),
    // and one from a future format.
    let older = sims_vm::boot::save::FORMAT - 1;
    let mut header = older.to_le_bytes().to_vec();
    header.extend(sims_sim::SNAPSHOT_FORMAT.to_le_bytes());
    let refused = Saved::decode(&header).unwrap_err().to_string();
    assert!(
        refused.contains(&format!("save format {older}")),
        "{refused}"
    );
    let future = sims_vm::boot::save::FORMAT + 1;
    let mut header = future.to_le_bytes().to_vec();
    header.extend(sims_sim::SNAPSHOT_FORMAT.to_le_bytes());
    let refused = Saved::decode(&header).unwrap_err().to_string();
    assert!(
        refused.contains(&format!("save format {future}")),
        "{refused}"
    );
    // The world's encoding is checked as well.
    let mut header = sims_vm::boot::save::FORMAT.to_le_bytes().to_vec();
    header.extend((sims_sim::SNAPSHOT_FORMAT - 1).to_le_bytes());
    let refused = Saved::decode(&header).unwrap_err().to_string();
    assert!(refused.contains("world format"), "{refused}");
}

#[test]
fn moving_the_camera_while_a_dialog_waits_replays_with_the_original_view() {
    let Some((root, house)) = goth_house() else {
        return;
    };
    let mut baseline = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    baseline.set_view_rotation(1);
    baseline.set_view_centre(Some([10, 12]));
    baseline.step().unwrap();
    baseline.step().unwrap();

    let mut asked = Lot::load(&root, &house, &[], Frames::default()).unwrap();
    asked.wait_for_answers();
    asked.set_view_rotation(1);
    asked.set_view_centre(Some([10, 12]));
    asked.step().unwrap();
    // Tick 2 pauses on the Help System dialog and freezes (1, Some([10, 12])).
    assert_eq!(asked.step().unwrap_err(), StepError::Asking { tick: 2 });
    // Moving the camera while the dialog shows updates the live camera state,
    // but must not change the dialog tick's replay — even across save/restore.
    asked.set_view_rotation(3);
    asked.set_view_centre(Some([40, 42]));
    asked.answer(3).unwrap();
    let bytes = asked.save().unwrap().encode();

    let mut restored = Lot::load(&root, &house, &[], Frames::default()).unwrap();
    restored.wait_for_answers();
    restored.restore(Saved::decode(&bytes).unwrap()).unwrap();
    assert_eq!(restored.view_rotation(), 3);
    assert_eq!(restored.view_centre(), Some([40, 42]));
    restored.step().unwrap();
    assert_eq!(restored.world().hash(), baseline.world().hash());
    // After the replayed tick completes, the moved camera applies to later ticks.
    assert_eq!(restored.view_rotation(), 3);
    assert_eq!(restored.view_centre(), Some([40, 42]));
}
