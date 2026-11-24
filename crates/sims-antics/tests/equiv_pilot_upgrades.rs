//! Verifier upgrades from the multi-asset `.antics` pilot (Track 7): decisive operand
//! differences on a primitive both trees reach, Set To Next's explicit stack-object
//! destination, the frame-shape rule, behavioral local renaming, fault-time last
//! results, and `--assume-equiv`. Every tree here is synthetic.

use sims_antics::encode::encode_bhav;
use sims_antics::ir::BhavChunk;
use sims_antics::{
    EquivResult, IffEquivOptions, IffVerdict, LiveContext, LiveOptions, ResourceVerdict,
    check_bhav_live_equivalent, check_iff_equivalent,
};
use sims_formats::behavior::Instruction;
use sims_formats::iff::{ChunkData, write as write_iff};
use std::process::Command;

const T: u8 = 0xFE;
const F: u8 = 0xFF;
const ERR: u8 = 0xFD;

// Expression scopes and operators.
const LIT: u8 = 7;
const TEMP: u8 = 8;
const PARAM: u8 = 9;
const LOCAL: u8 = 25;
const GT: u8 = 0;
const EQ: u8 = 2;
const SET: u8 = 5;

fn expr(lhs: (u8, i16), op: u8, rhs: (u8, i16), on_true: u8, on_false: u8) -> Instruction {
    let mut operands = [0u8; 8];
    operands[0..2].copy_from_slice(&lhs.1.to_le_bytes());
    operands[2..4].copy_from_slice(&rhs.1.to_le_bytes());
    operands[5] = op;
    operands[6] = lhs.0;
    operands[7] = rhs.0;
    Instruction {
        opcode: 2,
        on_true,
        on_false,
        operands,
    }
}

fn prim(opcode: u16, operands: [u8; 8], on_true: u8, on_false: u8) -> Instruction {
    Instruction {
        opcode,
        on_true,
        on_false,
        operands,
    }
}

fn tree(id: u16, args: u8, locals: u16, instructions: Vec<Instruction>) -> BhavChunk {
    BhavChunk {
        id,
        label: format!("Tree{id}"),
        format: 0x8002,
        kind: 0,
        args,
        locals,
        revision: 0,
        instructions,
        trailing: Vec::new(),
    }
}

fn live_with(a: &BhavChunk, b: &BhavChunk, opts: &LiveOptions) -> EquivResult {
    let ctx = LiveContext::default();
    check_bhav_live_equivalent(a, &ctx, b, &ctx, opts)
}

fn live(a: &BhavChunk, b: &BhavChunk) -> EquivResult {
    live_with(a, b, &LiveOptions::default())
}

fn behavioral() -> LiveOptions {
    LiveOptions {
        behavioral: true,
        ..LiveOptions::default()
    }
}

#[track_caller]
fn assert_equivalent(result: &EquivResult) {
    assert!(
        matches!(result, EquivResult::Equivalent { .. }),
        "expected Equivalent, got {result:?}"
    );
}

#[track_caller]
fn assert_counterexample(result: &EquivResult) -> String {
    match result {
        EquivResult::Counterexample { divergence, .. } => divergence.clone(),
        other => panic!("expected Counterexample, got {other:?}"),
    }
}

#[track_caller]
fn assert_inconclusive(result: &EquivResult) {
    assert!(
        matches!(result, EquivResult::Inconclusive { .. }),
        "expected Inconclusive, got {result:?}"
    );
}

/// Go To Routing Slot (opcode 45): slot `number`, `source` (0 parameter, 1 own, 2 global).
fn route(number: i16, source: u16) -> [u8; 8] {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&number.to_le_bytes());
    ops[2..4].copy_from_slice(&source.to_le_bytes());
    ops
}

// ----------------------------------------------------------------------------
// Decisive operand differences
// ----------------------------------------------------------------------------

/// The pilot's mis-compile: an own slot where the original routes to a global one. Both
/// trees run the primitive first, so it is reached, and a stack object without (or with)
/// that slot tells the two apart.
#[test]
fn route_to_own_versus_global_slot_is_different() {
    let a = tree(4096, 0, 0, vec![prim(45, route(8, 2), T, F)]);
    let b = tree(4096, 0, 0, vec![prim(45, route(8, 1), T, F)]);
    for opts in [LiveOptions::default(), behavioral()] {
        let why = assert_counterexample(&live_with(&a, &b, &opts));
        assert!(why.contains("global routing slot 8"), "{why}");
        assert!(why.contains("own slot 8"), "{why}");
        assert!(why.contains("Not replayed"), "{why}");
    }
}

/// The difference is still found behind a loop the checker cannot close.
#[test]
fn route_to_difference_is_found_behind_an_unbounded_loop() {
    // n0: route; n1: temp[0] += 1 forever (temp[0] != 99 loops back).
    let body = |source| {
        vec![
            prim(45, route(3, source), 1, 1),
            expr((TEMP, 0), 3, (LIT, 1), 2, 2),
            expr((TEMP, 0), EQ, (LIT, 99), T, 0),
        ]
    };
    let a = tree(4096, 0, 0, body(1));
    let b = tree(4096, 0, 0, body(2));
    assert_counterexample(&live_with(&a, &b, &behavioral()));
}

/// Source 0 names the slot by a parameter: `param[0]` against the literal 0 differ for
/// any witness where `param[0] != 0`.
#[test]
fn route_to_parameter_versus_literal_slot_is_different() {
    let a = tree(4096, 1, 0, vec![prim(45, route(0, 0), T, F)]);
    let b = tree(4096, 1, 0, vec![prim(45, route(0, 1), T, F)]);
    let why = assert_counterexample(&live(&a, &b));
    assert!(why.contains("witness"), "{why}");
}

/// Under `param[0] == 5`, naming slot `param[0]` is naming slot 5: not different.
#[test]
fn route_to_parameter_slot_pinned_by_the_path_is_not_different() {
    let a = tree(
        4096,
        1,
        0,
        vec![
            expr((PARAM, 0), EQ, (LIT, 5), 1, F),
            prim(45, route(0, 0), T, F),
        ],
    );
    let b = tree(
        4096,
        1,
        0,
        vec![
            expr((PARAM, 0), EQ, (LIT, 5), 1, F),
            prim(45, route(5, 1), T, F),
        ],
    );
    let result = live(&a, &b);
    assert!(
        !matches!(result, EquivResult::Counterexample { .. }),
        "{result:?}"
    );
}

/// Two global slots depend on the game's fixed table, and a call before the primitive
/// may never return: neither is reported as different.
#[test]
fn route_to_differences_the_checker_cannot_decide_stay_inconclusive() {
    let a = tree(4096, 0, 0, vec![prim(45, route(8, 2), T, F)]);
    let b = tree(4096, 0, 0, vec![prim(45, route(9, 2), T, F)]);
    assert_inconclusive(&live(&a, &b));

    let call = prim(0x1001, [0xFF; 8], 1, F);
    let a = tree(4096, 0, 0, vec![call.clone(), prim(45, route(8, 2), T, F)]);
    let b = tree(4096, 0, 0, vec![call, prim(45, route(8, 1), T, F)]);
    assert_inconclusive(&live(&a, &b));
}

/// Both trees set the stack object to an ID no object has before routing, so Go To
/// Routing Slot fails on both sides before it looks at a slot: no world tells the
/// operands apart. The same holds when the path tests the stack object, which may pin it
/// to such an ID.
#[test]
fn route_to_with_a_stack_object_the_path_constrains_is_not_different() {
    const STACK_OBJ: u8 = 10;
    for prefix in [
        expr((STACK_OBJ, 0), SET, (LIT, -1), 1, ERR),
        expr((STACK_OBJ, 0), EQ, (LIT, 0), 1, F),
    ] {
        let a = tree(
            4096,
            0,
            0,
            vec![prefix.clone(), prim(45, route(8, 2), T, F)],
        );
        let b = tree(4096, 0, 0, vec![prefix, prim(45, route(8, 1), T, F)]);
        for opts in [LiveOptions::default(), behavioral()] {
            let result = live_with(&a, &b, &opts);
            assert!(
                !matches!(result, EquivResult::Counterexample { .. }),
                "{result:?}"
            );
        }
    }
}

/// An opaque read before the primitive may fault in every world: `global[-1]` is out of
/// the VM's 0..64 range, so neither tree ever reaches Go To Routing Slot.
#[test]
fn route_to_after_a_read_that_may_always_fault_is_not_different() {
    const GLOBAL: u8 = 6;
    let body = |number| {
        vec![
            expr((LOCAL, 0), SET, (GLOBAL, -1), 1, ERR),
            prim(45, route(number, 1), T, F),
        ]
    };
    let (a, b) = (tree(4096, 0, 1, body(0)), tree(4096, 0, 1, body(1)));
    for opts in [LiveOptions::default(), behavioral()] {
        let result = live_with(&a, &b, &opts);
        assert!(
            !matches!(result, EquivResult::Counterexample { .. }),
            "{result:?}"
        );
    }
}

/// No decisive difference where the witness does not say which nodes run: a tree ID with
/// bit 15 set runs another tree's nodes (the VM looks the frame up without it), and a
/// kind-4 tree's entry picks its first node.
#[test]
fn route_to_in_a_tree_that_may_not_run_its_own_nodes_is_not_different() {
    let flagged = |number| tree(0x9000, 0, 0, vec![prim(45, route(number, 1), T, F)]);
    let kind4 = |number| BhavChunk {
        kind: 4,
        ..tree(
            4096,
            0,
            0,
            vec![
                expr((TEMP, 0), SET, (LIT, 0), T, T),
                prim(45, route(number, 1), T, F),
            ],
        )
    };
    for (a, b) in [(flagged(0), flagged(1)), (kind4(0), kind4(1))] {
        for opts in [LiveOptions::default(), behavioral()] {
            let result = live_with(&a, &b, &opts);
            assert!(
                !matches!(result, EquivResult::Counterexample { .. }),
                "{result:?}"
            );
        }
    }
}

/// A difference the path condition rules out is no difference.
#[test]
fn route_to_difference_on_an_infeasible_path_is_not_reported() {
    // Both trees route only when temp[0] == 1 and temp[0] == 2, which never holds.
    let guard = |source| {
        vec![
            expr((TEMP, 0), EQ, (LIT, 1), 1, F),
            expr((TEMP, 0), EQ, (LIT, 2), 2, F),
            prim(45, route(8, source), T, F),
        ]
    };
    assert_equivalent(&live(
        &tree(4096, 0, 0, guard(1)),
        &tree(4096, 0, 0, guard(2)),
    ));
}

// ----------------------------------------------------------------------------
// Behavioral local renaming
// ----------------------------------------------------------------------------

/// Animate Sim (opcode 44) with flag 0x20: the event goes to `local[slot]`.
fn animate(slot: u8) -> [u8; 8] {
    [1, 0, slot, 0, 0, 0x20, 0, 0]
}

/// Animate into `local[slot]`; on an event copy it to `temp[0]`.
fn animate_event_tree(slot: u8, locals: u16) -> BhavChunk {
    tree(
        4096,
        0,
        locals,
        vec![
            prim(44, animate(slot), T, 1),
            expr((TEMP, 0), SET, (LOCAL, i16::from(slot)), T, T),
        ],
    )
}

/// The pilot's `animate(local:)` case: the same event local under another slot number.
/// The behavioral relation renames it; the strict one observes the slot.
#[test]
fn animate_event_local_in_another_slot_is_renamed_behaviorally() {
    let a = animate_event_tree(0, 4);
    let b = animate_event_tree(3, 4);
    assert_equivalent(&live_with(&a, &b, &behavioral()));
    assert!(!matches!(live(&a, &b), EquivResult::Equivalent { .. }));
}

/// `let`s numbered in another order, each written before it is read, with the event
/// local among them.
#[test]
fn write_before_read_locals_are_renamed_behaviorally() {
    // local[x] := 7; animate(local: y); on event temp[0] := local[y] + local[x].
    let body = |x: i16, y: u8| {
        tree(
            4096,
            0,
            3,
            vec![
                expr((LOCAL, x), SET, (LIT, 7), 1, 1),
                prim(44, animate(y), T, 2),
                expr((TEMP, 0), SET, (LOCAL, i16::from(y)), 3, 3),
                expr((TEMP, 0), 3, (LOCAL, x), T, T),
            ],
        )
    };
    assert_equivalent(&live_with(&body(0, 1), &body(2, 0), &behavioral()));
}

/// A local read before anything writes it holds what an earlier frame (or Create New
/// Object Instance) left: its number is observable, so it is never renamed.
#[test]
fn a_local_read_before_written_is_not_renamed() {
    let body = |slot: i16| {
        tree(
            4096,
            0,
            4,
            vec![
                expr((TEMP, 1), SET, (LOCAL, slot), 1, 1),
                prim(44, animate(slot as u8), T, 2),
                expr((TEMP, 0), SET, (LOCAL, slot), T, T),
            ],
        )
    };
    let result = live_with(&body(0), &body(3), &behavioral());
    assert!(
        !matches!(result, EquivResult::Equivalent { .. }),
        "{result:?}"
    );
}

/// A Dialog may format any local (`$Local:N`, `$TimeLocal:N`): a tree with one keeps
/// every slot number.
#[test]
fn a_dialog_pins_every_local() {
    let body = |slot: u8| {
        tree(
            4096,
            0,
            4,
            vec![prim(44, animate(slot), T, 1), prim(36, [0; 8], T, T)],
        )
    };
    let result = live_with(&body(0), &body(3), &behavioral());
    assert!(
        !matches!(result, EquivResult::Equivalent { .. }),
        "{result:?}"
    );
}

/// Renaming must not hide a different value: B copies another local than the event.
#[test]
fn a_renamed_local_holding_another_value_is_not_equivalent() {
    let a = animate_event_tree(0, 4);
    let b = tree(
        4096,
        0,
        4,
        vec![
            expr((LOCAL, 2), SET, (LIT, 9), 1, 1),
            prim(44, animate(3), T, 2),
            expr((TEMP, 0), SET, (LOCAL, 2), T, T),
        ],
    );
    let result = live_with(&a, &b, &behavioral());
    assert!(
        !matches!(result, EquivResult::Equivalent { .. }),
        "{result:?}"
    );
}

/// Slot numbers another primitive names where the renaming does not rewrite them (Set
/// Balloon/Headline's local, `ops[0] >> 1`) stay put. (Random Number's destination is
/// renamed: see `tests/equiv_round2.rs`.)
#[test]
fn a_local_another_primitive_names_is_not_renamed() {
    let headline_from = |slot: i16| {
        let mut ops = [0u8; 8];
        ops[0..2].copy_from_slice(&(slot << 1).to_le_bytes());
        ops[2] = 2;
        ops[3] = 7;
        ops
    };
    let body = |slot: i16| {
        tree(
            4096,
            0,
            4,
            vec![
                expr((LOCAL, slot), SET, (LIT, 7), 1, 1),
                prim(41, headline_from(slot), T, T),
            ],
        )
    };
    let result = live_with(&body(0), &body(3), &behavioral());
    assert!(
        !matches!(result, EquivResult::Equivalent { .. }),
        "{result:?}"
    );
}

// ----------------------------------------------------------------------------
// Set To Next, frame shape, fault-time last result
// ----------------------------------------------------------------------------

/// Set To Next naming the stack object as its destination (bit 7, scope 10, index 0)
/// resolves the same cell as the default form (`set_to_next.rs:119`). Strict mode
/// still compares a faulting node's raw bytes, so this holds behaviorally.
#[test]
fn set_to_next_explicit_stack_object_destination_is_the_default() {
    let next = |ops: [u8; 8]| tree(4096, 0, 0, vec![prim(31, ops, T, F)]);
    let default = next([0, 0, 0, 0, 0x01, 0, 0, 0]);
    let explicit = next([0, 0, 0, 0, 0x81, 10, 0, 0]);
    assert_equivalent(&live_with(&default, &explicit, &behavioral()));
    // Index 1 of scope 10, or another scope, is another destination.
    for ops in [[0, 0, 0, 0, 0x81, 10, 0, 1], [0, 0, 0, 0, 0x81, 8, 0, 0]] {
        let other = next(ops);
        let result = live_with(&default, &other, &behavioral());
        assert!(
            !matches!(result, EquivResult::Equivalent { .. }),
            "{result:?}"
        );
    }
}

/// With an argument, Create New Object Instance writes the argument and never looks at
/// the locals (`create.rs:537`): a local more is no frame-shape difference. Without one
/// it writes the first local, so whether there is one still matters.
#[test]
fn frame_shape_counts_locals_only_for_trees_without_arguments() {
    let copy = |args: u8, locals: u16| {
        tree(
            4096,
            args,
            locals,
            vec![expr((TEMP, 0), SET, (LIT, 1), T, T)],
        )
    };
    assert_equivalent(&live_with(&copy(1, 0), &copy(1, 1), &behavioral()));
    assert_equivalent(&live_with(&copy(1, 2), &copy(1, 14), &behavioral()));
    match live_with(&copy(0, 0), &copy(0, 1), &behavioral()) {
        EquivResult::Inconclusive { reason } => assert!(reason.contains("shape"), "{reason}"),
        other => panic!("expected Inconclusive, got {other:?}"),
    }
}

/// Two trees that differ only in the last branch result they leave when they fault.
fn fault_polarity_pair() -> (BhavChunk, BhavChunk) {
    // A: temp[0] == 1 -> error, else true.
    let a = tree(4096, 0, 0, vec![expr((TEMP, 0), EQ, (LIT, 1), ERR, T)]);
    // B: the same, but the test before the error is `temp[0] > 5`, false there.
    let b = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), EQ, (LIT, 1), 1, T),
            expr((TEMP, 0), GT, (LIT, 5), ERR, ERR),
        ],
    );
    (a, b)
}

/// A fault keeps the last result (play-mode recovery and person resets read it), so by
/// default a difference there is never proven equivalent; the assumption that nothing
/// reads it does.
#[test]
fn fault_last_result_is_compared_unless_assumed_unread() {
    let (a, b) = fault_polarity_pair();
    for base in [LiveOptions::default(), behavioral()] {
        let result = live_with(&a, &b, &base);
        assert!(
            !matches!(result, EquivResult::Equivalent { .. }),
            "{result:?}"
        );
    }
    // Strict mode also observes the faulting node, which differs here.
    let ignoring = LiveOptions {
        observe_fault_last_result: false,
        ..behavioral()
    };
    assert_equivalent(&live_with(&a, &b, &ignoring));
}

// ----------------------------------------------------------------------------
// Whole file: --assume-equiv and EQUIVALENT modulo
// ----------------------------------------------------------------------------

fn bhav_chunk(bhav: &BhavChunk) -> ChunkData {
    ChunkData {
        tag: *b"BHAV",
        id: bhav.id,
        flags: 0x10,
        label: bhav.label.clone(),
        data: encode_bhav(bhav).expect("encode_bhav"),
    }
}

/// Tree 4096 calls 4097 and is the same bytes in both files; 4097 is `callee` in A
/// and `callee_b` in B.
fn caller_files(callee_a: Vec<Instruction>, callee_b: Vec<Instruction>) -> (Vec<u8>, Vec<u8>) {
    let caller = tree(4096, 0, 0, vec![prim(4097, [0; 8], T, F)]);
    let file = |body| {
        write_iff(&[bhav_chunk(&caller), bhav_chunk(&tree(4097, 0, 0, body))]).expect("write")
    };
    (file(callee_a), file(callee_b))
}

/// Two global slots the checker cannot tell apart: undecidable, not different.
fn undecided_files() -> (Vec<u8>, Vec<u8>) {
    caller_files(
        vec![prim(45, route(8, 2), T, F)],
        vec![prim(45, route(9, 2), T, F)],
    )
}

#[test]
fn inconclusive_files_name_the_trees_they_are_equivalent_modulo() {
    let (a, b) = undecided_files();
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
    assert_eq!(
        report.equivalent_modulo,
        Some([4097].into_iter().collect()),
        "{report:#?}"
    );
    let caller = report.resources.iter().find(|r| r.id == 4096).unwrap();
    assert!(
        matches!(&caller.verdict, ResourceVerdict::Inconclusive { why } if why.contains("may run tree #4097")),
        "{caller:?}"
    );
}

#[test]
fn assume_equiv_makes_dependent_verdicts_conditional_and_says_so() {
    let (a, b) = undecided_files();
    let opts = IffEquivOptions {
        assume_equiv: [4097].into_iter().collect(),
        ..IffEquivOptions::default()
    };
    let report = check_iff_equivalent(&a, &b, &opts).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
    assert!(
        report
            .assumptions
            .iter()
            .any(|l| l.contains("BHAV #4097") && l.contains("NOT PROVEN")),
        "{:?}",
        report.assumptions
    );
    let callee = report.resources.iter().find(|r| r.id == 4097).unwrap();
    assert!(
        matches!(&callee.verdict, ResourceVerdict::Equivalent { how } if how.contains("ASSUMED")),
        "{callee:?}"
    );
    // An ID neither file defines is reported, not applied.
    let opts = IffEquivOptions {
        assume_equiv: [4097, 4200].into_iter().collect(),
        ..IffEquivOptions::default()
    };
    let report = check_iff_equivalent(&a, &b, &opts).expect("check");
    assert!(
        report
            .assumptions
            .iter()
            .any(|l| l.contains("BHAV #4200") && l.contains("NOT APPLIED")),
        "{:?}",
        report.assumptions
    );
}

/// What "EQUIVALENT modulo" advertises is what `--assume-equiv` gives: tree 4096 calls
/// 4097 in A and 4098 in B, and 4097 differs, so assuming 4096 alone proves nothing.
#[test]
fn the_modulo_set_is_exactly_what_assume_equiv_proves() {
    let file = |call: u16, callee_temp: i16| {
        let chunks = [
            tree(4096, 0, 0, vec![prim(call, [0; 8], T, F)]),
            tree(
                4097,
                0,
                0,
                vec![expr((TEMP, 0), SET, (LIT, callee_temp), T, T)],
            ),
            tree(4098, 0, 0, vec![expr((TEMP, 0), SET, (LIT, 1), T, T)]),
        ];
        write_iff(&chunks.iter().map(bhav_chunk).collect::<Vec<_>>()).expect("write")
    };
    let (a, b) = (file(4097, 1), file(4098, 2));
    for tree_opt in [None, Some(4096)] {
        let opts = IffEquivOptions {
            tree: tree_opt,
            ..IffEquivOptions::default()
        };
        let report = check_iff_equivalent(&a, &b, &opts).expect("check");
        let Some(modulo) = report.equivalent_modulo.clone() else {
            continue;
        };
        // Only reachable without the VM replay, where 4097 is not shown to differ.
        assert!(modulo.contains(&4097), "{modulo:?}: {report:#?}");
        let assumed = IffEquivOptions {
            assume_equiv: modulo,
            ..opts
        };
        let report = check_iff_equivalent(&a, &b, &assumed).expect("check");
        assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
    }
}

/// After a partial `--assume-equiv`, the modulo set still names every tree to assume,
/// the ones already assumed included, so it works as the whole list.
#[test]
fn the_modulo_set_after_a_partial_assumption_is_the_whole_list() {
    let file = |own: i16, callee: i16| {
        let chunks = [
            tree(
                4096,
                0,
                0,
                vec![prim(4097, [0; 8], 1, F), prim(45, route(own, 2), T, F)],
            ),
            tree(4097, 0, 0, vec![prim(45, route(callee, 2), T, F)]),
        ];
        write_iff(&chunks.iter().map(bhav_chunk).collect::<Vec<_>>()).expect("write")
    };
    let (a, b) = (file(8, 10), file(9, 11));
    let opts = IffEquivOptions {
        assume_equiv: [4096].into_iter().collect(),
        ..IffEquivOptions::default()
    };
    let report = check_iff_equivalent(&a, &b, &opts).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
    let modulo = report.equivalent_modulo.clone().expect("a modulo set");
    assert_eq!(modulo, [4096, 4097].into_iter().collect(), "{report:#?}");
    let opts = IffEquivOptions {
        assume_equiv: modulo,
        ..IffEquivOptions::default()
    };
    let report = check_iff_equivalent(&a, &b, &opts).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
}

/// A tree shown to differ is never assumed equivalent.
#[cfg(feature = "vm-replay")]
#[test]
fn assume_equiv_never_overrides_a_difference() {
    let (a, b) = caller_files(
        vec![expr((TEMP, 0), SET, (LIT, 1), T, T)],
        vec![expr((TEMP, 0), SET, (LIT, 2), T, T)],
    );
    let opts = IffEquivOptions {
        assume_equiv: [4097].into_iter().collect(),
        ..IffEquivOptions::default()
    };
    let report = check_iff_equivalent(&a, &b, &opts).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Different, "{report:#?}");
    assert!(
        report
            .assumptions
            .iter()
            .any(|l| l.contains("BHAV #4097") && l.contains("NOT APPLIED")),
        "{:?}",
        report.assumptions
    );
}

#[test]
fn verify_equiv_cli_prints_assumptions_and_the_modulo_set() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("equiv-pilot-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let (a, b) = undecided_files();
    let (path_a, path_b) = (dir.join("a.iff"), dir.join("b.iff"));
    std::fs::write(&path_a, &a).unwrap();
    std::fs::write(&path_b, &b).unwrap();
    let run = |extra: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_sims-antics"))
            .arg("verify-equiv")
            .args(extra)
            .arg(&path_a)
            .arg(&path_b)
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    let (code, stdout) = run(&[]);
    assert_eq!(code, Some(2), "{stdout}");
    assert!(
        stdout.contains("EQUIVALENT modulo unproven {#4097}")
            && stdout.contains("--assume-equiv 4097"),
        "{stdout}"
    );
    let (code, stdout) = run(&["--assume-equiv", "4097", "--ignore-fault-last-result"]);
    assert_eq!(code, Some(0), "{stdout}");
    assert!(stdout.contains("ASSUME: BHAV #4097"), "{stdout}");
    assert!(stdout.contains("ASSUME: a path that faults"), "{stdout}");
    let (code, _) = run(&["--assume-equiv", "x"]);
    assert_ne!(code, Some(0));
}
