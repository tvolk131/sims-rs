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

use sims_antics::arity_catalog::{InstallArityCatalog, ResourceMatch, vm_kept_bhavs};
use sims_antics::{
    EquivOptions, EquivResult, LiftOptions, StructuredLiftOutcome,
    check_bhav_equivalent_with_arities, compile_antics, extract_lift_symbols_from_iff,
    lift::{format_bhav_labeled_air_with_context, lift_bhav},
    lift_bhav_structured_with_arities, lift_bhav_structured_with_context,
};
use sims_formats::behavior::Behavior;
use sims_formats::iff::Iff;
use std::path::PathBuf;

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

    // Strictly in memory: the same install catalog `sims-antics lift` uses resolves global
    // (`Global.iff`), private (this IFF), and semi-global (`GLOB` link) callee arities,
    // matching `sims_vm::Interpreter::behavior_from_owner`.
    let mut arity_catalog = InstallArityCatalog::load(&PathBuf::from(&install_root))
        .expect("SIMS_INSTALL did not resolve to an indexable Sims installation");

    let mut total_bhavs = 0usize;
    let mut lifted_structured = 0usize;
    let mut lifted_pure_high_level = 0usize;
    let mut lifted_with_inline_raw_op = 0usize;
    // The raw `op 0x` lines strict lifting still spells, by opcode and operand bytes.
    let mut raw_op_lines: std::collections::BTreeMap<String, usize> = Default::default();
    let mut lifted_relaxed_pure_high_level = 0usize;
    let mut verified_equivalent = 0usize;
    let mut verified_annotated_equivalent = 0usize;
    let mut verified_relaxed_equivalent = 0usize;
    let mut labeled_cfg_verified = 0usize;
    let mut labeled_cfg_relaxed_verified = 0usize;
    let mut fridges_total = 0usize;
    let mut fridges_structured = 0usize;
    let mut fridges_pure_relaxed = 0usize;
    let mut inconclusive = 0usize;
    let mut counterexamples = 0usize;
    let mut unsupported_opcode = 0usize;
    let mut unsupported_header_or_target = 0usize;
    let mut unstructured = 0usize;
    let mut compile_failures = 0usize;
    let mut first_counterexample_meta: Option<String> = None;
    let mut first_compile_failure_meta: Option<String> = None;

    let opts = EquivOptions::default();
    let strict_lift = LiftOptions {
        strict_padding: true,
        annotate_comments: false,
    };
    let annotated_lift = LiftOptions {
        strict_padding: true,
        annotate_comments: true,
    };
    let relaxed_lift = LiftOptions {
        strict_padding: false,
        annotate_comments: false,
    };

    for resource_index in 0..arity_catalog.index.resources.len() {
        let resource = &arity_catalog.index.resources[resource_index];
        let name = resource.name();
        if name.len() < 4 || !name[name.len() - 4..].eq_ignore_ascii_case(".iff") {
            continue;
        }
        let Ok(bytes) = resource.read_with(&arity_catalog.pool) else {
            continue;
        };
        let Ok(iff) = Iff::parse(bytes) else {
            continue;
        };

        let file_arities = arity_catalog.resolve(ResourceMatch::Unique(resource_index));

        let is_fridges = name
            .rsplit(['/', '\\'])
            .next()
            .is_some_and(|base| base.eq_ignore_ascii_case("Fridges.iff"));
        let lift_symbols = extract_lift_symbols_from_iff(&iff);

        // Only the chunk the VM keeps for each ID runs, so only it is checked.
        for chunk in vm_kept_bhavs(&iff).chunks {
            let data = iff.data(chunk);
            if Behavior::parse(data).is_err() {
                continue;
            }
            let Ok(bhav) = lift_bhav(chunk.id, "", data) else {
                continue;
            };
            total_bhavs += 1;
            if is_fridges {
                fridges_total += 1;
            }
            // Callee arities depend on the scope the calling tree runs in.
            let iff_arities = &file_arities.for_tree(bhav.id).arities;

            match lift_bhav_structured_with_arities(&bhav, strict_lift, Some(iff_arities)) {
                StructuredLiftOutcome::Lifted(source_text) => {
                    lifted_structured += 1;
                    if is_fridges {
                        fridges_structured += 1;
                    }
                    if source_text.contains("op 0x") {
                        lifted_with_inline_raw_op += 1;
                        for line in source_text.lines().filter(|l| l.contains("op 0x")) {
                            let line = line.trim();
                            let shown = line.split(" -> ").next().unwrap_or(line);
                            let shown = format!(
                                "{shown} [args {}, locals {}, format {:#06x}]",
                                bhav.args, bhav.locals, bhav.format
                            );
                            *raw_op_lines.entry(shown).or_default() += 1;
                        }
                    } else {
                        lifted_pure_high_level += 1;
                    }
                    let compiled = match compile_antics(&source_text) {
                        Ok(m) => m,
                        Err(e) => {
                            compile_failures += 1;
                            if first_compile_failure_meta.is_none() {
                                first_compile_failure_meta = Some(format!(
                                    "resource_index={resource_index} BHAV #{} (strict compile_failure: {e})",
                                    chunk.id
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
                        Some(iff_arities),
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
                                    "resource_index={resource_index} BHAV #{} (strict, steps={}, last_pc_a={}, last_pc_b={})",
                                    chunk.id,
                                    path.len(),
                                    last_pcs.0,
                                    last_pcs.1
                                ));
                            }
                        }
                    }

                    // Verify comment-annotated `.air` (`annotate_comments: true`) parses & proves Equivalent!
                    if let StructuredLiftOutcome::Lifted(ann_src) =
                        lift_bhav_structured_with_context(
                            &bhav,
                            annotated_lift,
                            Some(iff_arities),
                            Some(&lift_symbols),
                        )
                    {
                        match compile_antics(&ann_src) {
                            Ok(m) => {
                                let re_ann = &m.behaviors[&chunk.id];
                                if matches!(
                                    check_bhav_equivalent_with_arities(
                                        &bhav,
                                        re_ann,
                                        &opts,
                                        Some(iff_arities),
                                    ),
                                    EquivResult::Equivalent { .. }
                                ) {
                                    verified_annotated_equivalent += 1;
                                } else {
                                    counterexamples += 1;
                                }
                            }
                            Err(e) => {
                                compile_failures += 1;
                                if first_compile_failure_meta.is_none() {
                                    first_compile_failure_meta = Some(format!(
                                        "resource_index={resource_index} BHAV #{} (annotated compile_failure: {e})",
                                        chunk.id
                                    ));
                                }
                            }
                        }
                    }

                    // Also verify `--relax-padding` (`strict_padding: false`) compiles & proves Equivalent.
                    if let StructuredLiftOutcome::Lifted(relaxed_src) =
                        lift_bhav_structured_with_arities(&bhav, relaxed_lift, Some(iff_arities))
                    {
                        if !relaxed_src.contains("op 0x") {
                            lifted_relaxed_pure_high_level += 1;
                            if is_fridges {
                                fridges_pure_relaxed += 1;
                            }
                        }
                        match compile_antics(&relaxed_src) {
                            Ok(m) => {
                                let re_relaxed = &m.behaviors[&chunk.id];
                                match check_bhav_equivalent_with_arities(
                                    &bhav,
                                    re_relaxed,
                                    &opts,
                                    Some(iff_arities),
                                ) {
                                    EquivResult::Equivalent { .. } => {
                                        verified_relaxed_equivalent += 1;
                                    }
                                    EquivResult::Inconclusive { .. } => {
                                        inconclusive += 1;
                                    }
                                    EquivResult::Counterexample { path, .. } => {
                                        counterexamples += 1;
                                        if first_counterexample_meta.is_none() {
                                            let last_pcs = path
                                                .last()
                                                .map(|s| (s.pc_a, s.pc_b))
                                                .unwrap_or((0, 0));
                                            first_counterexample_meta = Some(format!(
                                                "resource_index={resource_index} BHAV #{} (relaxed, steps={}, last_pc_a={}, last_pc_b={})",
                                                chunk.id,
                                                path.len(),
                                                last_pcs.0,
                                                last_pcs.1
                                            ));
                                        }
                                    }
                                }
                            }
                            Err(_) => {
                                compile_failures += 1;
                                if first_compile_failure_meta.is_none() {
                                    first_compile_failure_meta = Some(format!(
                                        "resource_index={resource_index} BHAV #{} (relaxed compile_failure)",
                                        chunk.id
                                    ));
                                }
                            }
                        }
                    }
                }
                StructuredLiftOutcome::UnsupportedOpcode(_) => unsupported_opcode += 1,
                outcome @ (StructuredLiftOutcome::UnsupportedHeaderOrTarget
                | StructuredLiftOutcome::Unstructured) => {
                    if matches!(outcome, StructuredLiftOutcome::UnsupportedHeaderOrTarget) {
                        unsupported_header_or_target += 1;
                    } else {
                        unstructured += 1;
                    }
                    // Verify BOTH strict and relaxed decoded labeled-CFG `.air` compile cleanly
                    // and prove Equivalent on ALL fallback trees (`Unstructured` AND
                    // `UnsupportedHeaderOrTarget`, protecting extended inline-call arguments!).
                    for (mode_label, l_opts, counter) in [
                        (
                            "labeled-cfg-strict",
                            annotated_lift,
                            &mut labeled_cfg_verified,
                        ),
                        (
                            "labeled-cfg-relaxed",
                            relaxed_lift,
                            &mut labeled_cfg_relaxed_verified,
                        ),
                    ] {
                        let labeled_src = format_bhav_labeled_air_with_context(
                            &bhav,
                            l_opts,
                            Some(iff_arities),
                            Some(&lift_symbols),
                        );
                        match compile_antics(&labeled_src) {
                            Ok(m) => {
                                let re_labeled = &m.behaviors[&chunk.id];
                                match check_bhav_equivalent_with_arities(
                                    &bhav,
                                    re_labeled,
                                    &opts,
                                    Some(iff_arities),
                                ) {
                                    EquivResult::Equivalent { .. } => {
                                        *counter += 1;
                                    }
                                    EquivResult::Inconclusive { .. } => {
                                        inconclusive += 1;
                                    }
                                    EquivResult::Counterexample { path, .. } => {
                                        counterexamples += 1;
                                        if first_counterexample_meta.is_none() {
                                            let last_pcs = path
                                                .last()
                                                .map(|s| (s.pc_a, s.pc_b))
                                                .unwrap_or((0, 0));
                                            first_counterexample_meta = Some(format!(
                                                "resource_index={resource_index} BHAV #{} ({mode_label}, steps={}, last_pc_a={}, last_pc_b={})",
                                                chunk.id,
                                                path.len(),
                                                last_pcs.0,
                                                last_pcs.1
                                            ));
                                        }
                                    }
                                }
                            }
                            Err(_) => {
                                compile_failures += 1;
                                if first_compile_failure_meta.is_none() {
                                    first_compile_failure_meta = Some(format!(
                                        "resource_index={resource_index} BHAV #{} ({mode_label} compile_failure)",
                                        chunk.id
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut raw_ops: Vec<_> = raw_op_lines.into_iter().collect();
    raw_ops.sort_by(|x, y| y.1.cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
    for (line, count) in raw_ops.iter().take(40) {
        eprintln!("structured_equiv raw op {count}x: {line}");
    }
    let total_fallbacks = unstructured + unsupported_header_or_target;
    eprintln!(
        "structured_equiv summary: total_bhavs={total_bhavs}, lifted_structured={lifted_structured} \
         (pure_zero_raw_op_strict={lifted_pure_high_level}, with_inline_raw_op_strict={lifted_with_inline_raw_op}, \
         pure_zero_raw_op_relaxed={lifted_relaxed_pure_high_level}), \
         verified_equivalent_strict={verified_equivalent}, verified_annotated_equivalent={verified_annotated_equivalent}, \
         verified_equivalent_relaxed={verified_relaxed_equivalent}, \
         labeled_cfg_verified_strict={labeled_cfg_verified}/{total_fallbacks}, \
         labeled_cfg_verified_relaxed={labeled_cfg_relaxed_verified}/{total_fallbacks}, \
         fridges_catalog={fridges_structured}/{fridges_total} (pure_relaxed={fridges_pure_relaxed}), \
         inconclusive={inconclusive}, counterexamples={counterexamples}, compile_failures={compile_failures}, \
         unsupported_opcode={unsupported_opcode}, unsupported_header_or_target={unsupported_header_or_target}, \
         unstructured={unstructured}"
    );

    assert!(
        total_bhavs >= 28_000,
        "expected Complete Collection corpus (>=28k BHAVs), got {total_bhavs}"
    );
    assert!(
        verified_equivalent >= 27_000,
        "expected >=27,000 real BHAVs formally verified Equivalent from structured .air, got {verified_equivalent}"
    );
    assert_eq!(
        inconclusive, 0,
        "expected 0 inconclusive proofs among lifted BHAVs, got {inconclusive}"
    );
    assert_eq!(
        compile_failures, 0,
        "high-level structured lifter produced uncompilable .antics: {:?}",
        first_compile_failure_meta
    );
    assert_eq!(
        verified_equivalent, lifted_structured,
        "every cleanly lifted structured BHAV must formally verify Equivalent"
    );
    assert_eq!(
        verified_annotated_equivalent, lifted_structured,
        "every comment-annotated structured BHAV must compile and formally verify Equivalent"
    );
    assert_eq!(
        verified_relaxed_equivalent, lifted_structured,
        "every --relax-padding lifted structured BHAV must formally verify Equivalent"
    );
    assert_eq!(
        labeled_cfg_verified, total_fallbacks,
        "every strict decoded labeled-CFG fallback BHAV must formally verify Equivalent"
    );
    assert_eq!(
        labeled_cfg_relaxed_verified, total_fallbacks,
        "every relaxed decoded labeled-CFG fallback BHAV must formally verify Equivalent"
    );
    assert_eq!(
        counterexamples, 0,
        "formal equivalence checker found a divergence between original and structured-recompiled BHAV: {:?}",
        first_counterexample_meta
    );
}
