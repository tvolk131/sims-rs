//! Which Generic Sims Call routines the install's trees call that the
//! port still refuses.
//!
//! Every BHAV in the install is read, the neighborhoods' character
//! files included; a file or tree that cannot be read fails the test.
//! Each call of opcode 1, Generic Sims Call, gives its routine number
//! (operand word 0). Each number is
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

/// The paths in `dir`, sorted, so the scan never depends on the order the
/// filesystem lists them in.
#[allow(clippy::disallowed_methods)]
fn sorted_paths(dir: &std::path::Path) -> std::io::Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    paths.sort();
    Ok(paths)
}

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
    // The files to read: the index's IFF files, and the neighborhoods'
    // character files, which a lot's load adds to its index
    // (`Index::add_characters`) but `Index::build` does not.
    let mut files: Vec<(String, Result<Vec<u8>, String>)> = index
        .resources
        .iter()
        .filter(|r| r.name().to_ascii_lowercase().ends_with(".iff"))
        .map(|r| {
            let bytes = r.read_with(&pool).map_err(|e| e.to_string());
            (r.location(&index.root), bytes)
        })
        .collect();
    let neighborhoods: Vec<PathBuf> = sorted_paths(&root)
        .expect("list the install")
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.to_ascii_lowercase().starts_with("userdata"))
        })
        .collect();
    let mut characters = 0;
    for neighborhood in neighborhoods {
        let Ok(entries) = sorted_paths(&neighborhood.join("Characters")) else {
            continue;
        };
        let paths = entries.into_iter().filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("iff"))
        });
        for path in paths {
            characters += 1;
            let shown = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .display()
                .to_string();
            files.push((shown, std::fs::read(&path).map_err(|e| e.to_string())));
        }
    }
    assert!(
        characters > 0,
        "the install has no neighborhood character files"
    );
    // Each routine: how many calls, and the first tree that makes one.
    let mut calls: BTreeMap<u16, (usize, String)> = BTreeMap::new();
    // Files or trees that could not be read: they would hide calls.
    let mut unread = Vec::new();
    for (location, bytes) in files {
        let iff = match bytes.map(Iff::parse) {
            Ok(Ok(iff)) => iff,
            Ok(Err(e)) => {
                unread.push(format!("{location}: {e}"));
                continue;
            }
            Err(e) => {
                unread.push(format!("{location}: {e}"));
                continue;
            }
        };
        for chunk in &iff.chunks {
            if &chunk.tag != b"BHAV" || chunk.flags & DELETED != 0 {
                continue;
            }
            let behavior = match Behavior::parse(iff.data(chunk)) {
                Ok(behavior) => behavior,
                Err(e) => {
                    unread.push(format!("{location} tree 0x{:04x}: {e}", chunk.id));
                    continue;
                }
            };
            for instruction in behavior.instructions.iter().filter(|i| i.opcode == 1) {
                let operands = instruction.operands;
                let routine = u16::from_le_bytes([operands[0], operands[1]]);
                let entry = calls
                    .entry(routine)
                    .or_insert_with(|| (0, format!("{location} tree 0x{:04x}", chunk.id)));
                entry.0 += 1;
            }
        }
    }
    for line in &unread {
        eprintln!("unread: {line}");
    }
    assert!(
        unread.is_empty(),
        "{} files or trees could not be read",
        unread.len()
    );
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
