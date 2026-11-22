//! In-memory `SIMS_INSTALL`-gated structured high-level lifting & formal
//! trace-equivalence verification suite.
//!
//! Legal boundary: reads original `.iff` files from `$SIMS_INSTALL` strictly in
//! memory, lifts structured BHAVs into high-level `.antics` (zero raw `op`
//! instructions), compiles back through `sims_antics::compile_antics`, proves
//! behavioral equivalence via `check_bhav_equivalent`, prints metadata-only
//! summary counts (never writing or logging commercial bytes/strings/scripts),
//! and drops all buffers immediately.
//!
//! Skips cleanly when `SIMS_INSTALL` is unset (CI-safe).

use sims_antics::{
    EquivOptions, EquivResult, StructuredLiftOutcome, check_bhav_equivalent_with_arities,
    compile_antics, lift::lift_bhav, lift_bhav_structured,
};
use sims_content::{ArchivePool, Index, Install};
use sims_formats::behavior::Behavior;
use sims_formats::iff::{DELETED, Iff};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn bhav_arity(b: &Behavior) -> usize {
    match b.format {
        0x8000 => 4,
        0x8001 => usize::from(b.args).min(4),
        _ => usize::from(b.args),
    }
}

#[test]
fn complete_collection_in_memory_structured_lift_and_equiv_proof() {
    let Ok(install_root) = std::env::var("SIMS_INSTALL") else {
        eprintln!("structured_equiv: skipped (SIMS_INSTALL unset)");
        return;
    };
    if install_root.trim().is_empty() {
        eprintln!("structured_equiv: skipped (SIMS_INSTALL empty)");
        return;
    }

    let install = Install::detect(&PathBuf::from(&install_root))
        .expect("SIMS_INSTALL did not resolve to a valid Sims installation");
    let index = Index::build(&install).expect("failed to index SIMS_INSTALL");
    let pool = ArchivePool::new();

    // Pre-pass (strictly in memory): collect global (256..4096) BHAV arities across the
    // install so inline subroutine calls (`opcode >= 256`) are verified against real arities.
    let mut shared_arities: BTreeMap<u16, usize> = BTreeMap::new();
    for resource in &index.resources {
        let name = resource.name();
        if name.len() < 4 || !name[name.len() - 4..].eq_ignore_ascii_case(".iff") {
            continue;
        }
        let Ok(bytes) = resource.read_with(&pool) else {
            continue;
        };
        let Ok(iff) = Iff::parse(bytes) else {
            continue;
        };
        for chunk in &iff.chunks {
            if chunk.flags & DELETED != 0 || &chunk.tag != b"BHAV" || chunk.id >= 4096 {
                continue;
            }
            if let Ok(bhav) = Behavior::parse(iff.data(chunk)) {
                let arity = bhav_arity(&bhav);
                shared_arities
                    .entry(chunk.id)
                    .and_modify(|prev| *prev = (*prev).max(arity))
                    .or_insert(arity);
            }
        }
    }

    let mut total_bhavs = 0usize;
    let mut lifted_high_level = 0usize;
    let mut verified_equivalent = 0usize;
    let mut inconclusive = 0usize;
    let mut counterexamples = 0usize;
    let mut unsupported_opcode = 0usize;
    let mut unsupported_header_or_target = 0usize;
    let mut unstructured = 0usize;
    let mut compile_failures = 0usize;
    let mut first_counterexample_meta: Option<String> = None;
    let mut first_compile_failure_meta: Option<String> = None;

    let opts = EquivOptions::default();

    for resource in &index.resources {
        let name = resource.name();
        if name.len() < 4 || !name[name.len() - 4..].eq_ignore_ascii_case(".iff") {
            continue;
        }
        let Ok(bytes) = resource.read_with(&pool) else {
            continue;
        };
        let Ok(iff) = Iff::parse(bytes) else {
            continue;
        };

        let mut iff_arities = shared_arities.clone();
        for chunk in &iff.chunks {
            if chunk.flags & DELETED != 0 || &chunk.tag != b"BHAV" {
                continue;
            }
            if let Ok(bhav) = Behavior::parse(iff.data(chunk)) {
                iff_arities.insert(chunk.id, bhav_arity(&bhav));
            }
        }

        for chunk in &iff.chunks {
            if chunk.flags & DELETED != 0 || &chunk.tag != b"BHAV" {
                continue;
            }
            let data = iff.data(chunk);
            if Behavior::parse(data).is_err() {
                continue;
            }
            let Ok(bhav) = lift_bhav(chunk.id, "", data) else {
                continue;
            };
            total_bhavs += 1;

            // Skip trees containing layout-dependent >4-arg inline calls before lifting.
            let has_wide_inline_call = bhav.instructions.iter().any(|ins| {
                ins.opcode >= 256
                    && ins.operands != [0xFF; 8]
                    && iff_arities.get(&ins.opcode).copied().unwrap_or(5) > 4
            });
            if has_wide_inline_call {
                unsupported_header_or_target += 1;
                continue;
            }

            match lift_bhav_structured(&bhav) {
                StructuredLiftOutcome::Lifted(source_text) => {
                    assert!(
                        !source_text.contains("op 0x"),
                        "structured lifter emitted raw op on resource_index={total_bhavs} BHAV #{}",
                        chunk.id
                    );
                    lifted_high_level += 1;
                    let compiled = match compile_antics(&source_text) {
                        Ok(m) => m,
                        Err(_) => {
                            compile_failures += 1;
                            if first_compile_failure_meta.is_none() {
                                first_compile_failure_meta = Some(format!(
                                    "resource_index={} BHAV #{} (compile_failure)",
                                    total_bhavs, chunk.id
                                ));
                            }
                            continue;
                        }
                    };
                    let recompiled = &compiled.behaviors[&chunk.id];
                    match check_bhav_equivalent_with_arities(
                        &bhav,
                        recompiled,
                        &opts,
                        Some(&iff_arities),
                    ) {
                        EquivResult::Equivalent { .. } => {
                            verified_equivalent += 1;
                        }
                        EquivResult::Inconclusive { .. } => {
                            inconclusive += 1;
                        }
                        EquivResult::Counterexample { path, .. } => {
                            counterexamples += 1;
                            if first_counterexample_meta.is_none() {
                                let last_pcs =
                                    path.last().map(|s| (s.pc_a, s.pc_b)).unwrap_or((0, 0));
                                first_counterexample_meta = Some(format!(
                                    "resource_index={} BHAV #{} (steps={}, last_pc_a={}, last_pc_b={})",
                                    total_bhavs,
                                    chunk.id,
                                    path.len(),
                                    last_pcs.0,
                                    last_pcs.1
                                ));
                            }
                        }
                    }
                }
                StructuredLiftOutcome::UnsupportedOpcode(_) => unsupported_opcode += 1,
                StructuredLiftOutcome::UnsupportedHeaderOrTarget => {
                    unsupported_header_or_target += 1;
                }
                StructuredLiftOutcome::Unstructured => unstructured += 1,
            }
        }
    }

    eprintln!(
        "structured_equiv summary: total_bhavs={total_bhavs}, lifted_high_level={lifted_high_level}, \
         verified_equivalent={verified_equivalent}, inconclusive={inconclusive}, \
         counterexamples={counterexamples}, compile_failures={compile_failures}, \
         unsupported_opcode={unsupported_opcode}, unsupported_header_or_target={unsupported_header_or_target}, \
         unstructured={unstructured}"
    );

    assert!(
        total_bhavs >= 28_000,
        "expected Complete Collection corpus (>=28k BHAVs), got {total_bhavs}"
    );
    assert!(
        verified_equivalent >= 10_000,
        "expected >=10,000 real BHAVs formally verified Equivalent from high-level .antics, got {verified_equivalent}"
    );
    assert_eq!(
        inconclusive, 0,
        "expected 0 inconclusive proofs among lifted BHAVs, got {inconclusive}"
    );
    assert_eq!(
        verified_equivalent, lifted_high_level,
        "every cleanly lifted high-level BHAV must formally verify Equivalent"
    );
    assert_eq!(
        compile_failures, 0,
        "high-level structured lifter produced uncompilable .antics: {:?}",
        first_compile_failure_meta
    );
    assert_eq!(
        counterexamples, 0,
        "formal equivalence checker found a divergence between original and structured-recompiled BHAV: {:?}",
        first_counterexample_meta
    );
}
