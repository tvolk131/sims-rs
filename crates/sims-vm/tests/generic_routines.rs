//! Which Generic Sims Call routines the install's trees call that the
//! port still refuses.
//!
//! Every BHAV in the install is read, and each call of opcode 1, Generic
//! Sims Call, gives its routine number (operand word 0). Each number is
//! then run once on the synthetic lot of the interpreter tests. A routine
//! the port does not have fails with
//! `Trap::Unsupported("Generic Sims Call routine N")`; any other result
//! means it is ported, though it may still fail on the synthetic lot for
//! its own reasons. The routines refused must be exactly [`RESIDUALS`]:
//! port one and take it off, and a new one the trees call shows up here.
//!
//! Needs an install (`SIMS_INSTALL`); skipped otherwise. Only routine
//! numbers and tree locations are printed, never the trees.

#[path = "interpreter/support.rs"]
#[allow(dead_code)]
mod support;

use sims_content::{ArchivePool, Index, Install};
use sims_formats::behavior::Behavior;
use sims_formats::iff::{DELETED, Iff};
use sims_vm::Trap;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use support::*;

/// The routines the install's trees call that the port refuses.
const RESIDUALS: &[u16] = &[
    0, 1, 4, 5, 13, 17, 19, 20, 22, 23, 24, 25, 27, 29, 30, 32, 33, 34, 36, 38, 39, 40, 41, 42,
];

/// Whether the port refuses routine `routine`.
fn refused(routine: u16) -> bool {
    let mut fx = Fixture::new();
    let [lo, hi] = routine.to_le_bytes();
    fx.add(
        SIM_FILE,
        4096,
        tree(vec![ins(
            1,
            RETURN_TRUE,
            RETURN_FALSE,
            [lo, hi, 0, 0, 0, 0, 0, 0],
        )]),
    );
    let unsupported = format!("Generic Sims Call routine {routine}");
    matches!(
        fx.run_with(SIM, 4096, 0, &[], 100),
        Err(fault) if fault.trap == Trap::Unsupported(unsupported)
    )
}

#[test]
fn the_generic_routines_the_install_calls_are_ported_or_listed() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        eprintln!("skipping: set SIMS_INSTALL to scan the install's Generic Sims Calls");
        return;
    };
    let install = Install::detect(&root).expect("detect the install");
    let index = Index::build(&install).expect("index the install");
    let pool = ArchivePool::new();
    // Each routine: how many calls, and the first tree that makes one.
    let mut calls: BTreeMap<u16, (usize, String)> = BTreeMap::new();
    for resource in &index.resources {
        let name = resource.name();
        if !name.to_ascii_lowercase().ends_with(".iff") {
            continue;
        }
        let Ok(bytes) = resource.read_with(&pool) else {
            continue;
        };
        let Ok(iff) = Iff::parse(bytes) else {
            continue;
        };
        for chunk in &iff.chunks {
            if &chunk.tag != b"BHAV" || chunk.flags & DELETED != 0 {
                continue;
            }
            let Ok(behavior) = Behavior::parse(iff.data(chunk)) else {
                continue;
            };
            for instruction in behavior.instructions.iter().filter(|i| i.opcode == 1) {
                let operands = instruction.operands;
                let routine = u16::from_le_bytes([operands[0], operands[1]]);
                let entry = calls.entry(routine).or_insert_with(|| {
                    (
                        0,
                        format!("{} tree 0x{:04x}", resource.location(&index.root), chunk.id),
                    )
                });
                entry.0 += 1;
            }
        }
    }
    assert!(!calls.is_empty(), "the install has no Generic Sims Calls");
    let refused: BTreeSet<u16> = calls.keys().copied().filter(|&r| refused(r)).collect();
    for (routine, (count, first)) in &calls {
        let state = if refused.contains(routine) {
            "refused"
        } else {
            "ported"
        };
        eprintln!("routine {routine:>3}: {state:<7} {count:>5} calls, first in {first}");
    }
    let listed: BTreeSet<u16> = RESIDUALS.iter().copied().collect();
    assert_eq!(
        refused,
        listed,
        "the refused routines the install calls differ from RESIDUALS (newly refused: {:?}; \
         ported or no longer called: {:?})",
        refused.difference(&listed).collect::<Vec<_>>(),
        listed.difference(&refused).collect::<Vec<_>>()
    );
}
