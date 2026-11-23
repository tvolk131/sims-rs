//! `SIMS_INSTALL`-gated soundness sweep for the live-output checker.
//!
//! Reads original `.iff` files strictly in memory and prints counts only (never names,
//! strings or script bytes):
//!
//! 1. every `BHAV` is checked against itself: the live-output checker must never report
//!    a counterexample for identical trees;
//! 2. every tree the structured lifter recompiles to a lockstep-equivalent tree is
//!    re-checked with the live-output checker, which must not contradict lockstep;
//! 3. "CT - Is Fridge" style dummy trees (one `Expression` whose both branches return the
//!    same value) are proven equal to the matching `return true;` / `return false;`
//!    whenever their reads cannot trap, ignoring the last branch result; with it
//!    observed, every refutation must be that last result.
//!
//! Skips cleanly when `SIMS_INSTALL` is unset (CI-safe).

use sims_antics::{
    EquivOptions, EquivResult, LiftOptions, LiveContext, LiveOptions, StructuredLiftOutcome,
    check_bhav_equivalent_with_arities, check_bhav_live_equivalent, compile_air, compile_antics,
    lift::lift_bhav, lift_bhav_structured_with_arities,
};
use sims_content::{ArchivePool, Catalog, Index, Install};
use sims_formats::behavior::Behavior;
use sims_formats::execution::constants;
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

fn arities_in_range(iff: &Iff, lo: u16, hi: u16) -> BTreeMap<u16, usize> {
    let mut out = BTreeMap::new();
    for chunk in &iff.chunks {
        if chunk.flags & DELETED != 0 || &chunk.tag != b"BHAV" || chunk.id < lo || chunk.id > hi {
            continue;
        }
        if let Ok(bhav) = Behavior::parse(iff.data(chunk)) {
            out.insert(chunk.id, bhav_arity(&bhav));
        }
    }
    out
}

#[test]
fn live_output_checker_is_sound_on_the_install() {
    let Ok(install_root) = std::env::var("SIMS_INSTALL") else {
        eprintln!("live_equiv_corpus: skipped (SIMS_INSTALL unset)");
        return;
    };
    if install_root.trim().is_empty() {
        eprintln!("live_equiv_corpus: skipped (SIMS_INSTALL empty)");
        return;
    }
    let install = Install::detect(&PathBuf::from(&install_root))
        .expect("SIMS_INSTALL did not resolve to a valid Sims installation");
    let index = Index::build(&install).expect("failed to index SIMS_INSTALL");
    let pool = ArchivePool::new();
    let catalog = Catalog::build_with_pool(&index, &pool).expect("failed to build catalog");
    let global_iff = Iff::parse(
        index.resources[catalog.global]
            .read_with(&pool)
            .expect("failed to read Global.iff"),
    )
    .expect("failed to parse Global.iff");
    let global_arities = arities_in_range(&global_iff, 256, 4095);
    let mut semi_cache: BTreeMap<usize, BTreeMap<u16, usize>> = BTreeMap::new();

    let opts = LiveOptions::default();
    let ignoring_last_result = LiveOptions {
        observe_last_result: false,
        ..LiveOptions::default()
    };
    let lockstep = EquivOptions::default();
    let lift_opts = LiftOptions {
        strict_padding: true,
        annotate_comments: false,
    };
    let return_true = compile_air(r#"tree 4096 "T" { return true; }"#)
        .unwrap()
        .behaviors[&4096]
        .clone();
    let return_false = compile_air(r#"tree 4096 "F" { return false; }"#)
        .unwrap()
        .behaviors[&4096]
        .clone();

    let mut total = 0usize;
    let mut self_equivalent = 0usize;
    let mut self_inconclusive = 0usize;
    let mut self_counterexamples = 0usize;
    let mut inconclusive_reasons: BTreeMap<String, usize> = BTreeMap::new();
    let mut lockstep_equivalent = 0usize;
    let mut live_agrees = 0usize;
    let mut live_inconclusive_on_lockstep_equivalent = 0usize;
    let mut live_contradicts_lockstep = 0usize;
    let mut dummy_trees = 0usize;
    let mut dummy_proven = 0usize;
    let mut dummy_refuted = 0usize;
    let mut dummy_strict_proven = 0usize;
    let mut dummy_strict_last_result = 0usize;
    let mut dummy_strict_other = 0usize;
    let mut fridges_dummy_proven = 0usize;
    let mut first_problem: Option<String> = None;

    for (resource_index, resource) in index.resources.iter().enumerate() {
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
        let is_fridges = name
            .rsplit(['/', '\\'])
            .next()
            .is_some_and(|base| base.eq_ignore_ascii_case("Fridges.iff"));
        let mut arities = global_arities.clone();
        arities.extend(arities_in_range(&iff, 4096, 8191));
        if let Some(Some(semi)) = catalog.semi_globals.get(&resource_index).copied() {
            let map = semi_cache.entry(semi).or_insert_with(|| {
                index.resources[semi]
                    .read_with(&pool)
                    .ok()
                    .and_then(|b| Iff::parse(b).ok())
                    .map(|s| arities_in_range(&s, 8192, u16::MAX))
                    .unwrap_or_default()
            });
            arities.extend(map.iter().map(|(&k, &v)| (k, v)));
        } else {
            arities.extend(arities_in_range(&iff, 8192, u16::MAX));
        }
        let bcons: BTreeMap<u16, Vec<i16>> = iff
            .all(b"BCON")
            .filter(|c| c.flags & DELETED == 0)
            .filter_map(|c| constants(iff.data(c)).ok().map(|v| (c.id, v)))
            .collect();
        let ctx = LiveContext {
            callee_arities: Some(arities.clone()),
            local_bcons: Some(bcons),
            ..LiveContext::default()
        };

        for chunk in &iff.chunks {
            if chunk.flags & DELETED != 0 || &chunk.tag != b"BHAV" {
                continue;
            }
            let data = iff.data(chunk);
            let Ok(bhav) = lift_bhav(chunk.id, "", data) else {
                continue;
            };
            total += 1;

            // 1. Self-equivalence.
            match check_bhav_live_equivalent(&bhav, &ctx, &bhav, &ctx, &opts) {
                EquivResult::Equivalent { .. } => self_equivalent += 1,
                EquivResult::Inconclusive { reason } => {
                    self_inconclusive += 1;
                    let key: String = reason
                        .trim_start_matches("A: ")
                        .trim_start_matches("B: ")
                        .split(|c: char| c.is_ascii_digit() || c == '(')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_owned();
                    *inconclusive_reasons.entry(key).or_default() += 1;
                }
                EquivResult::Counterexample { divergence, .. } => {
                    self_counterexamples += 1;
                    first_problem.get_or_insert_with(|| {
                        format!(
                            "self-check counterexample: resource_index={resource_index} BHAV #{}: {divergence}",
                            chunk.id
                        )
                    });
                }
            }

            // 2. Structured lift + recompile: live must agree with lockstep.
            if let StructuredLiftOutcome::Lifted(source) =
                lift_bhav_structured_with_arities(&bhav, lift_opts, Some(&arities))
                && let Ok(module) = compile_antics(&source)
                && let Some(recompiled) = module.behaviors.get(&chunk.id)
                && matches!(
                    check_bhav_equivalent_with_arities(
                        &bhav,
                        recompiled,
                        &lockstep,
                        Some(&arities)
                    ),
                    EquivResult::Equivalent { .. }
                )
            {
                lockstep_equivalent += 1;
                match check_bhav_live_equivalent(&bhav, &ctx, recompiled, &ctx, &opts) {
                    EquivResult::Equivalent { .. } => live_agrees += 1,
                    EquivResult::Inconclusive { .. } => {
                        live_inconclusive_on_lockstep_equivalent += 1;
                    }
                    EquivResult::Counterexample { divergence, .. } => {
                        live_contradicts_lockstep += 1;
                        first_problem.get_or_insert_with(|| {
                            format!(
                                "live contradicts lockstep: resource_index={resource_index} BHAV #{}: {divergence}",
                                chunk.id
                            )
                        });
                    }
                }
            }

            // 3. One-node dummy trees: an Expression compare whose both branches agree.
            if bhav.instructions.len() == 1 {
                let ins = &bhav.instructions[0];
                let compare =
                    ins.opcode == 2 && matches!(ins.operands[5], 0 | 1 | 2 | 14 | 15 | 16);
                if compare && ins.on_true == ins.on_false && matches!(ins.on_true, 0xFE | 0xFF) {
                    dummy_trees += 1;
                    let mut target = if ins.on_true == 0xFE {
                        return_true.clone()
                    } else {
                        return_false.clone()
                    };
                    target.id = bhav.id;
                    target.kind = bhav.kind;
                    target.format = bhav.format;
                    target.args = bhav.args;
                    target.locals = bhav.locals;
                    match check_bhav_live_equivalent(&bhav, &ctx, &target, &ctx, &opts) {
                        EquivResult::Equivalent { .. } => dummy_strict_proven += 1,
                        EquivResult::Counterexample { divergence, .. } => {
                            if divergence.contains("last branch result") {
                                dummy_strict_last_result += 1;
                            } else {
                                dummy_strict_other += 1;
                            }
                        }
                        EquivResult::Inconclusive { .. } => {}
                    }
                    match check_bhav_live_equivalent(
                        &bhav,
                        &ctx,
                        &target,
                        &ctx,
                        &ignoring_last_result,
                    ) {
                        EquivResult::Equivalent { .. } => {
                            dummy_proven += 1;
                            if is_fridges && chunk.id == 4143 {
                                fridges_dummy_proven += 1;
                            }
                        }
                        EquivResult::Counterexample { .. } => dummy_refuted += 1,
                        EquivResult::Inconclusive { .. } => {}
                    }
                }
            }
        }
    }

    eprintln!(
        "live_equiv_corpus: total={total} self_equivalent={self_equivalent} \
         self_inconclusive={self_inconclusive} self_counterexamples={self_counterexamples} \
         lockstep_equivalent={lockstep_equivalent} live_agrees={live_agrees} \
         live_inconclusive={live_inconclusive_on_lockstep_equivalent} \
         live_contradicts={live_contradicts_lockstep} dummy_trees={dummy_trees} \
         dummy_proven={dummy_proven} dummy_refuted={dummy_refuted} \
         fridges_4143_proven={fridges_dummy_proven} dummy_strict_proven={dummy_strict_proven} \
         dummy_strict_last_result={dummy_strict_last_result} \
         dummy_strict_other={dummy_strict_other}"
    );
    for (reason, count) in &inconclusive_reasons {
        eprintln!("live_equiv_corpus: inconclusive {count}x: {reason}");
    }
    if let Some(problem) = &first_problem {
        eprintln!("live_equiv_corpus: first problem: {problem}");
    }
    assert!(total > 0, "no BHAVs found under SIMS_INSTALL");
    assert_eq!(
        self_counterexamples, 0,
        "identical trees reported different"
    );
    assert_eq!(
        live_contradicts_lockstep, 0,
        "live-output contradicts lockstep"
    );
    assert_eq!(
        dummy_refuted, 0,
        "a dummy tree was refuted against its return value"
    );
    assert_eq!(
        fridges_dummy_proven, 1,
        "Fridges.iff tree 4143 must equal `return true;` (ignoring the last result)"
    );
    assert_eq!(
        dummy_strict_other, 0,
        "a dummy tree was refuted for something other than its last result"
    );
}
