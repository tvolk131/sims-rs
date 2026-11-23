//! Which Generic Sims Call routines the install's trees call that the
//! port still refuses.
//!
//! Every BHAV in the install is read, the neighborhoods' character
//! files included; a file, tree, archive or folder that cannot be read
//! fails the test.
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
use std::path::{Path, PathBuf};
use support::*;

/// The routines the install's trees call that the port refuses.
const RESIDUALS: &[u16] = &[
    0, 1, 4, 5, 13, 17, 20, 22, 23, 24, 25, 27, 29, 30, 32, 33, 34, 36, 38, 39, 40, 41, 42,
];

/// The paths in `dir`, sorted, so the scan never depends on the order the
/// filesystem lists them in. An entry that cannot be read is an error:
/// it could hide a file.
#[allow(clippy::disallowed_methods)]
fn sorted_paths(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut paths = std::fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<PathBuf>>>()?;
    paths.sort();
    Ok(paths)
}

/// Each routine: how many calls, and the first tree that makes one.
type Calls = BTreeMap<u16, (usize, String)>;

/// Adds the Generic Sims Calls in the IFF file `bytes` (shown as
/// `location`) to `calls`; a file or tree that cannot be read goes in
/// `unread`.
fn scan(
    location: &str,
    bytes: Result<Vec<u8>, String>,
    calls: &mut Calls,
    unread: &mut Vec<String>,
) {
    let iff = match bytes.and_then(|bytes| Iff::parse(bytes).map_err(|e| e.to_string())) {
        Ok(iff) => iff,
        Err(e) => {
            unread.push(format!("{location}: {e}"));
            return;
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
    let mut calls = Calls::new();
    // Files, trees and archives that could not be read: they would hide
    // calls. Each file is read and scanned on its own, so only one is held
    // at a time.
    let mut unread: Vec<String> = index.diagnostics.clone();
    // The index's IFF files.
    for resource in &index.resources {
        if !resource.name().to_ascii_lowercase().ends_with(".iff") {
            continue;
        }
        let bytes = resource.read_with(&pool).map_err(|e| e.to_string());
        scan(
            &resource.location(&index.root),
            bytes,
            &mut calls,
            &mut unread,
        );
    }
    // The neighborhoods' character files, which a lot's load adds to its
    // index (`Index::add_characters`) but `Index::build` does not. A
    // neighborhood without a Characters folder has none; any other folder
    // that cannot be listed fails the test.
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
        let folder = neighborhood.join("Characters");
        let shown = |path: &Path| {
            path.strip_prefix(&root)
                .unwrap_or(path)
                .display()
                .to_string()
        };
        let entries = match sorted_paths(&folder) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                unread.push(format!("{}: {e}", shown(&folder)));
                continue;
            }
        };
        let paths = entries.into_iter().filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("iff"))
        });
        for path in paths {
            characters += 1;
            let bytes = std::fs::read(&path).map_err(|e| e.to_string());
            scan(&shown(&path), bytes, &mut calls, &mut unread);
        }
    }
    assert!(
        characters > 0,
        "the install has no neighborhood character files"
    );
    for line in &unread {
        eprintln!("unread: {line}");
    }
    assert!(
        unread.is_empty(),
        "{} files, trees, archives or folders could not be read",
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
