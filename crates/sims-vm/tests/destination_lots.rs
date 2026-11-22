//! Live Complete Collection stepping across all five travel zones plus unit
//! coverage for destination-lot autonomy, `Generic Sims Call` routine 26, and
//! operand scope 38 (neighbor's definition).

use sims_vm::boot::{Frames, Lot};
use std::path::PathBuf;

fn sims_install() -> Option<PathBuf> {
    std::env::var_os("SIMS_INSTALL").map(PathBuf::from)
}

fn step_destination_lot(rel_house: &str, ticks: u32, select_first_person: bool) {
    let Some(root) = sims_install() else {
        return;
    };
    let house = root.join(rel_house);
    let mut lot = Lot::load(&root, &house, &[3; 32], Frames::default())
        .unwrap_or_else(|e| panic!("failed to load {rel_house}: {e}"));
    let lot_num = lot.world().globals.get(10).copied().unwrap_or(0);
    let fam_num = lot.world().globals.get(9).copied().unwrap_or(0);
    let zoning = lot.program().zoning.get(&lot_num).copied().unwrap_or(0);
    eprintln!(
        "{rel_house}: loaded lot={lot_num} family={fam_num} zoning={zoning} objects={} people={}",
        lot.world().objects.len(),
        lot.world().people.len(),
    );
    let mut native_errors = 0usize;
    let mut selected = false;
    for tick in 1..=ticks {
        if select_first_person
            && !selected
            && let Some(&person) = lot.world().people.first()
        {
            lot.select(person).unwrap_or_else(|e| {
                panic!("{rel_house} failed to select person {person} at tick {tick}: {e}")
            });
            selected = true;
        }
        let update = lot
            .step_play()
            .unwrap_or_else(|err| panic!("{rel_house} halted at tick {tick}: {err}"));
        for error in &update.errors {
            assert!(
                !error.fault.trap.unported(),
                "{rel_house} hit unported trap at tick {tick}: object {}: {}",
                error.object,
                error.fault
            );
            if let Some(then) = &error.then {
                assert!(
                    !then.trap.unported(),
                    "{rel_house} hit unported follow-up trap at tick {tick}: object {}: {then}",
                    error.object,
                );
            }
            native_errors += 1;
        }
        assert!(
            update.frame_errors.is_empty(),
            "{rel_house} had frame errors at tick {tick}: {:?}",
            update.frame_errors
        );
    }
    eprintln!(
        "{rel_house}: stepped {ticks} ticks cleanly (objects={} people={} native_errors={native_errors} hash={})",
        lot.world().objects.len(),
        lot.world().people.len(),
        lot.world().hash_hex(),
    );
}

#[test]
fn downtown_house21_steps_cleanly() {
    step_destination_lot("UserData/Houses/House21.iff", 300, false);
}

#[test]
fn vacation_house40_steps_cleanly() {
    // In Sims.exe, Vacation Island lots only enter Live Mode with a traveling
    // household (`global[3] != 0`). Stepping `House40` with `global[3] == 0`
    // hits a Maxis bug in `NPC_Vacation_Controller.iff` BHAV `0x1006`
    // (`Control Vacation Director`): node 44 branches past node 20 when
    // `global[3] == 0` and node 22 reads uninitialized `local 6`, which lands
    // on half of a popped frame header's `cXBHAV*` pointer in frame storage.
    step_destination_lot("UserData/Houses/House40.iff", 300, true);
}

#[test]
fn old_town_community_house58_steps_cleanly() {
    step_destination_lot("UserData/Houses/House58.iff", 300, false);
}

#[test]
fn studio_town_house81_steps_cleanly() {
    step_destination_lot("UserData/Houses/House81.iff", 300, false);
}

#[test]
fn magic_town_house90_and_house93_step_cleanly() {
    step_destination_lot("UserData/Houses/House90.iff", 300, false);
    step_destination_lot("UserData/Houses/House93.iff", 300, false);
}
