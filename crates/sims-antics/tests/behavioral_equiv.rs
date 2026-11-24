//! `verify-equiv --behavioral` (`LiveOptions::behavioral`) and the cut-point
//! bisimulation the live checker falls back to for loops and path explosions
//! (`docs/decisions/20261123-verifier-behavioral-mode.md`).
//!
//! Every tree here is synthetic. The positives check that a different layout of the same
//! program proves equivalent: renumbered nodes, renamed or dead locals, unbounded loops,
//! long chains of diamonds, and whole files. The negatives are small mutations of those
//! positives that change what the world sees, and must never prove equivalent.

use sims_antics::encode::encode_bhav;
use sims_antics::ir::BhavChunk;
use sims_antics::lift::lift_bhav;
use sims_antics::{
    EquivOptions, EquivResult, IffEquivOptions, IffVerdict, LiveContext, LiveOptions,
    check_bhav_equivalent_combined, check_bhav_live_equivalent, check_iff_equivalent, compile_air,
};
use sims_content::{ArchivePool, Index, Install};
use sims_formats::behavior::{Behavior, Instruction};
use sims_formats::iff::{ChunkData, DELETED, Iff, write as write_iff};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

const T: u8 = 0xFE;
const F: u8 = 0xFF;
const ERR: u8 = 0xFD;

// Expression scopes.
const GLOBAL: u8 = 6;
const LIT: u8 = 7;
const TEMP: u8 = 8;
const PARAM: u8 = 9;
const LOCAL: u8 = 25;

// Expression operators.
const LT: u8 = 1;
const EQ: u8 = 2;
const ADD: u8 = 3;
const SET: u8 = 5;
const DIV: u8 = 7;
const GE: u8 = 14;
const LE: u8 = 15;

// Primitives.
const SLEEP: u16 = 0;
const TEST: u16 = 4;
const IDLE: u16 = 17;
const SOUND: u16 = 23;
const SET_TO_NEXT: u16 = 31;
const ANIMATE: u16 = 44;

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

/// Animate clip `clip` from the global tables (source 3) with `flags`. Without flag
/// 0x20 an event goes to `param[0]`; the clip finishes on `true` and reports an event
/// on `false`.
fn animate(clip: i16, flags: u8, on_true: u8, on_false: u8) -> Instruction {
    let [lo, hi] = clip.to_le_bytes();
    prim(ANIMATE, [lo, hi, 0, 0, 3, flags, 0, 0], on_true, on_false)
}

fn sound(id: u8, next: u8) -> Instruction {
    prim(SOUND, [id, 0, 0, 0, 0, 0, 0, 0], next, next)
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

/// The same program laid out differently: node 0 stays the entry, and the other nodes
/// are stored in reverse order, with every branch target moved along.
fn reversed(bhav: &BhavChunk) -> BhavChunk {
    let n = bhav.instructions.len();
    let place = |i: usize| if i == 0 { 0 } else { n - i };
    let remap = |t: u8| {
        if usize::from(t) < n {
            place(usize::from(t)) as u8
        } else {
            t
        }
    };
    let mut instructions = bhav.instructions.clone();
    for (i, ins) in bhav.instructions.iter().enumerate() {
        instructions[place(i)] = Instruction {
            on_true: remap(ins.on_true),
            on_false: remap(ins.on_false),
            ..*ins
        };
    }
    BhavChunk {
        instructions,
        ..bhav.clone()
    }
}

fn strict() -> LiveOptions {
    LiveOptions::default()
}

fn behavioral() -> LiveOptions {
    LiveOptions {
        behavioral: true,
        ..LiveOptions::default()
    }
}

fn live(a: &BhavChunk, b: &BhavChunk, opts: &LiveOptions) -> EquivResult {
    let ctx = LiveContext::default();
    check_bhav_live_equivalent(a, &ctx, b, &ctx, opts)
}

fn combined(a: &BhavChunk, b: &BhavChunk, opts: &LiveOptions) -> EquivResult {
    let ctx = LiveContext::default();
    check_bhav_equivalent_combined(a, &ctx, b, &ctx, &EquivOptions::default(), opts).1
}

#[track_caller]
fn assert_equivalent(result: &EquivResult) {
    assert!(
        matches!(result, EquivResult::Equivalent { .. }),
        "expected Equivalent, got {result:?}"
    );
}

#[track_caller]
fn assert_not_equivalent(result: &EquivResult) {
    assert!(
        !matches!(result, EquivResult::Equivalent { .. }),
        "expected a verdict other than Equivalent, got {result:?}"
    );
}

#[track_caller]
fn assert_inconclusive_because(result: &EquivResult, needle: &str) {
    match result {
        EquivResult::Inconclusive { reason } => {
            assert!(reason.contains(needle), "expected {needle:?} in: {reason}");
        }
        other => panic!("expected Inconclusive, got {other:?}"),
    }
}

/// Behavioral equivalence holds both ways, and through the combined checker; strict
/// equivalence of the same pair is `strict_proves`.
#[track_caller]
fn assert_behavioral_only(a: &BhavChunk, b: &BhavChunk, strict_proves: bool) {
    for (x, y) in [(a, b), (b, a)] {
        assert_equivalent(&live(x, y, &behavioral()));
        assert_equivalent(&combined(x, y, &behavioral()));
        let s = combined(x, y, &strict());
        assert_eq!(
            matches!(s, EquivResult::Equivalent { .. }),
            strict_proves,
            "strict verdict: {s:?}"
        );
    }
}

/// Never equivalent, in either mode or direction.
#[track_caller]
fn assert_never_equivalent(a: &BhavChunk, b: &BhavChunk) {
    for opts in [strict(), behavioral()] {
        for (x, y) in [(a, b), (b, a)] {
            assert_not_equivalent(&live(x, y, &opts));
            assert_not_equivalent(&combined(x, y, &opts));
        }
    }
}

// ============================================================================
// Node numbers and frame layout
// ============================================================================

/// A primitive's node number is what a yield leaves the frame at, so strict mode keeps
/// it. The behavioral relation does not.
#[test]
fn renumbered_nodes_are_behaviorally_equivalent() {
    let a = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), EQ, (LIT, 3), 1, 2),
            sound(1, 3),
            sound(2, 3),
            expr((TEMP, 1), SET, (LIT, 7), T, T),
        ],
    );
    let b = reversed(&a);
    assert_ne!(encode_bhav(&a).unwrap(), encode_bhav(&b).unwrap());
    assert_behavioral_only(&a, &b, false);
    // Strict mode still says why.
    assert_inconclusive_because(&live(&a, &b, &strict()), "which a fault or a yield leaves");
}

/// Two divisions that may each trap: strict mode tells the faults apart by node, the
/// behavioral relation by what they leave the world.
#[test]
fn renumbered_faults_are_behaviorally_equivalent() {
    let a = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), EQ, (LIT, 0), 1, 2),
            expr((TEMP, 1), DIV, (TEMP, 2), T, ERR),
            expr((TEMP, 3), DIV, (TEMP, 4), T, ERR),
        ],
    );
    let b = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), EQ, (LIT, 0), 2, 1),
            expr((TEMP, 3), DIV, (TEMP, 4), T, ERR),
            expr((TEMP, 1), DIV, (TEMP, 2), T, ERR),
        ],
    );
    assert_behavioral_only(&a, &b, false);
}

/// Locals renamed one-to-one, a dead local write, and a different local count: none
/// of it reaches the world, so only strict mode (frame residue) sees it.
#[test]
fn renamed_dead_and_extra_locals_are_behaviorally_equivalent() {
    let a = tree(
        4096,
        0,
        1,
        vec![
            expr((LOCAL, 0), SET, (TEMP, 0), 1, 1),
            expr((LOCAL, 0), ADD, (LIT, 2), 2, 2),
            sound(4, 3),
            expr((TEMP, 1), SET, (LOCAL, 0), T, T),
        ],
    );
    let b = tree(
        4096,
        0,
        3,
        vec![
            expr((LOCAL, 2), SET, (TEMP, 0), 1, 1),
            expr((LOCAL, 1), SET, (LIT, 99), 2, 2),
            expr((LOCAL, 2), ADD, (LIT, 2), 3, 3),
            sound(4, 4),
            expr((TEMP, 1), SET, (LOCAL, 2), T, T),
        ],
    );
    assert_behavioral_only(&a, &b, false);
    // A renaming that is not one-to-one reads the wrong value.
    let merged = tree(
        4096,
        0,
        2,
        vec![
            expr((LOCAL, 0), SET, (TEMP, 0), 1, 1),
            expr((LOCAL, 1), SET, (LIT, 99), 2, 2),
            expr((LOCAL, 0), ADD, (LIT, 2), 3, 3),
            sound(4, 4),
            expr((TEMP, 1), SET, (LOCAL, 1), T, T),
        ],
    );
    assert_never_equivalent(&a, &merged);
}

/// A call's caller frame fixes where the callee's frame starts in the frame storage
/// (stale words it may read); the behavioral relation does not observe frame storage.
#[test]
fn call_frame_offsets_are_behavioral() {
    let call = || prim(4097, [0xFF; 8], T, F);
    let a = tree(4096, 0, 1, vec![call()]);
    let b = tree(4096, 0, 3, vec![call()]);
    assert_behavioral_only(&a, &b, false);
}

// ============================================================================
// Unbounded loops
// ============================================================================

/// Animate until the clip finishes, counting its events in `temp[0]`: the loop has no
/// literal bound, so path enumeration gives up and the cut-point bisimulation proves it.
fn animate_loop(clip: i16, flags: u8, exit: u8, step: i16) -> BhavChunk {
    tree(
        4096,
        1,
        0,
        vec![
            animate(clip, flags, exit, 1),
            expr((TEMP, 0), ADD, (LIT, step), 2, 2),
            sound(9, 0),
        ],
    )
}

#[test]
fn unbounded_animation_loops_prove_by_bisimulation() {
    let a = animate_loop(5, 0, T, 1);
    // Every event the clip reports adds a pass through the body, so path enumeration
    // never finishes; the same tree proves in strict mode, through the bisimulation.
    assert_equivalent(&live(&a, &a, &strict()));
    assert_behavioral_only(&a, &reversed(&a), false);
}

/// A counter in a local that no literal bounds: the bisimulation widens it, and renamed
/// locals keep pairing up.
#[test]
fn unbounded_local_counters_are_widened() {
    let counting = |local: i16, locals: u16| {
        tree(
            4096,
            1,
            locals,
            vec![
                expr((LOCAL, local), SET, (LIT, 0), 1, 1),
                animate(5, 0, 3, 2),
                expr((LOCAL, local), ADD, (LIT, 1), 1, 1),
                expr((TEMP, 0), SET, (LOCAL, local), T, T),
            ],
        )
    };
    let a = counting(0, 1);
    let b = reversed(&counting(1, 2));
    assert_equivalent(&live(&a, &a, &strict()));
    assert_behavioral_only(&a, &b, false);
    // Counting by two, or starting at one, leaves a different count.
    let by_two = tree(
        4096,
        1,
        1,
        vec![
            expr((LOCAL, 0), SET, (LIT, 0), 1, 1),
            animate(5, 0, 3, 2),
            expr((LOCAL, 0), ADD, (LIT, 2), 1, 1),
            expr((TEMP, 0), SET, (LOCAL, 0), T, T),
        ],
    );
    assert_never_equivalent(&a, &by_two);
    let from_one = tree(
        4096,
        1,
        1,
        vec![
            expr((LOCAL, 0), SET, (LIT, 1), 1, 1),
            animate(5, 0, 3, 2),
            expr((LOCAL, 0), ADD, (LIT, 1), 1, 1),
            expr((TEMP, 0), SET, (LOCAL, 0), T, T),
        ],
    );
    assert_never_equivalent(&a, &from_one);
}

/// `while (true)`: idle, then play a sound, forever. Only a fault ends it.
#[test]
fn endless_idle_loops_prove() {
    let a = tree(
        4096,
        1,
        0,
        vec![
            expr((PARAM, 0), SET, (LIT, 30), 1, 1),
            prim(IDLE, [0; 8], 2, 1),
            sound(3, 1),
        ],
    );
    assert_equivalent(&live(&a, &a, &strict()));
    assert_behavioral_only(&a, &reversed(&a), false);
    // Dropping the sound, or leaving the loop after it, is a different program.
    let silent = tree(
        4096,
        1,
        0,
        vec![
            expr((PARAM, 0), SET, (LIT, 30), 1, 1),
            prim(IDLE, [0; 8], 2, 1),
            expr((TEMP, 0), SET, (TEMP, 0), 1, 1),
        ],
    );
    assert_never_equivalent(&a, &silent);
    let terminating = tree(
        4096,
        1,
        0,
        vec![
            expr((PARAM, 0), SET, (LIT, 30), 1, 1),
            prim(IDLE, [0; 8], 2, 1),
            sound(3, T),
        ],
    );
    assert_never_equivalent(&a, &terminating);
}

/// Scan objects until none is left, counting them in `temp[1]`.
#[test]
fn set_to_next_scans_prove() {
    let scan = |step: i16, done: u8| {
        tree(
            4096,
            0,
            0,
            vec![
                expr((TEMP, 1), SET, (LIT, 0), 1, 1),
                prim(SET_TO_NEXT, [0; 8], 2, done),
                expr((TEMP, 1), ADD, (LIT, step), 1, 1),
            ],
        )
    };
    let a = scan(1, T);
    assert_equivalent(&live(&a, &a, &strict()));
    assert_behavioral_only(&a, &reversed(&a), false);
    assert_never_equivalent(&a, &scan(2, T));
    assert_never_equivalent(&a, &scan(1, F));
}

/// Negatives on the animation loop: every one changes what the world sees.
#[test]
fn mutated_animation_loops_are_never_equivalent() {
    let a = animate_loop(5, 0, T, 1);
    // The loop's exit returns false instead of true.
    assert_never_equivalent(&a, &animate_loop(5, 0, F, 1));
    // Another clip, or the same clip played backward (flag 0x02).
    assert_never_equivalent(&a, &animate_loop(6, 0, T, 1));
    assert_never_equivalent(&a, &animate_loop(5, 0x02, T, 1));
    // A different count left in temp[0].
    assert_never_equivalent(&a, &animate_loop(5, 0, T, 2));
    // The sound in the body is missing.
    let silent = tree(
        4096,
        1,
        0,
        vec![animate(5, 0, T, 1), expr((TEMP, 0), ADD, (LIT, 1), 0, 0)],
    );
    assert_never_equivalent(&a, &silent);
    // The body may now fault (division by a temp that may be zero).
    let faulting = tree(
        4096,
        1,
        0,
        vec![
            animate(5, 0, T, 1),
            expr((TEMP, 0), ADD, (LIT, 1), 2, 2),
            expr((TEMP, 2), DIV, (TEMP, 3), 3, 3),
            sound(9, 0),
        ],
    );
    assert_never_equivalent(&a, &faulting);
    // The body ends in the error exit instead of looping.
    let erroring = tree(
        4096,
        1,
        0,
        vec![
            animate(5, 0, T, 1),
            expr((TEMP, 0), ADD, (LIT, 1), 2, 2),
            sound(9, ERR),
        ],
    );
    assert_never_equivalent(&a, &erroring);
}

/// A loop that runs while `local[0] < param[0]`, against one that runs while
/// `local[0] <= param[0]`: one more iteration, so one more sound.
#[test]
fn off_by_one_loop_bounds_are_never_equivalent() {
    let bounded = |op: u8| {
        tree(
            4096,
            1,
            1,
            vec![
                expr((LOCAL, 0), SET, (LIT, 0), 1, 1),
                expr((LOCAL, 0), op, (PARAM, 0), 2, T),
                sound(1, 3),
                expr((LOCAL, 0), ADD, (LIT, 1), 1, 1),
            ],
        )
    };
    let (lt, le) = (bounded(LT), bounded(LE));
    assert_equivalent(&live(&lt, &lt, &strict()));
    // The sound stays at node 2, so the lockstep checker proves this layout strictly.
    assert_behavioral_only(&lt, &reversed(&lt), true);
    assert_never_equivalent(&lt, &le);
    // The same off-by-one in an animation loop, whose count no literal bounds either.
    let animated = |op: u8| {
        tree(
            4096,
            1,
            1,
            vec![
                expr((LOCAL, 0), SET, (LIT, 0), 1, 1),
                animate(5, 0, T, 2),
                expr((LOCAL, 0), ADD, (LIT, 1), 3, 3),
                expr((LOCAL, 0), op, (LIT, 3), 1, T),
            ],
        )
    };
    assert_behavioral_only(&animated(LT), &reversed(&animated(LT)), false);
    assert_never_equivalent(&animated(LT), &animated(LE));
}

// ============================================================================
// Diamonds
// ============================================================================

/// `diamonds` sequential `if (test) sound 1; else sound 2;` blocks. Each test and sound
/// may also fault, so the paths multiply far past the enumeration bound.
fn primitive_diamonds(diamonds: usize, swap_last: bool) -> BhavChunk {
    let mut nodes = Vec::new();
    for d in 0..diamonds {
        let base = (3 * d) as u8;
        let next = if d + 1 == diamonds { T } else { base + 3 };
        let (yes, no) = if swap_last && d + 1 == diamonds {
            (2, 1)
        } else {
            (1, 2)
        };
        nodes.push(prim(
            TEST,
            [d as u8, 0, 0, 0, 0, 0, 0, 0],
            base + 1,
            base + 2,
        ));
        nodes.push(sound(yes, next));
        nodes.push(sound(no, next));
    }
    tree(4096, 0, 0, nodes)
}

#[test]
fn primitive_diamonds_merge_at_cuts() {
    let a = primitive_diamonds(12, false);
    // Without the bisimulation the path bound is the limit.
    assert_equivalent(&live(&a, &a, &strict()));
    assert_behavioral_only(&a, &reversed(&a), false);
    assert_never_equivalent(&a, &primitive_diamonds(12, true));
}

/// `diamonds` sequential `if (global[1] > 0) temp[0] = 1; else temp[0] = 2;` blocks: no
/// primitive at all, only reads of untracked memory. The bisimulation cuts at every
/// event instead.
fn read_diamonds(diamonds: usize, last_value: i16) -> BhavChunk {
    let mut nodes = Vec::new();
    for d in 0..diamonds {
        let base = (3 * d) as u8;
        let next = if d + 1 == diamonds { T } else { base + 3 };
        let second = if d + 1 == diamonds { last_value } else { 2 };
        nodes.push(expr((GLOBAL, 1), EQ, (LIT, 0), base + 1, base + 2));
        nodes.push(expr((TEMP, 0), SET, (LIT, 1), next, next));
        nodes.push(expr((TEMP, 0), SET, (LIT, second), next, next));
    }
    tree(4096, 0, 0, nodes)
}

#[test]
fn read_diamonds_merge_at_cuts() {
    let a = read_diamonds(13, 2);
    assert_equivalent(&live(&a, &a, &strict()));
    assert_behavioral_only(&a, &reversed(&a), false);
    assert_never_equivalent(&a, &read_diamonds(13, 3));
}

// ============================================================================
// What the behavioral relation still observes
// ============================================================================

/// An extra `Expression` before a primitive sets the scratch pointer the primitive may
/// read; an extra branch changes the last result. Both stay observable.
#[test]
fn scratch_pointer_and_last_result_stay_observed() {
    let a = tree(4096, 0, 0, vec![sound(1, T)]);
    let b = tree(
        4096,
        0,
        0,
        vec![expr((TEMP, 0), SET, (TEMP, 0), 1, 1), sound(1, T)],
    );
    assert_inconclusive_because(&live(&a, &b, &behavioral()), "scratch-pointer");
    let relaxed = LiveOptions {
        observe_scratch_pointer: false,
        ..behavioral()
    };
    assert_inconclusive_because(&live(&a, &b, &relaxed), "last branch result");
    // Ignoring both still leaves the sound's fault: a failing primitive leaves the
    // scratch pointer and the last result behind.
    let relaxed = LiveOptions {
        observe_last_result: false,
        ..relaxed
    };
    assert_inconclusive_because(&live(&a, &b, &relaxed), "scratch-pointer state at exit");
}

/// A fault keeps the thread's last result and the scratch pointer in the object
/// (`interpreter.rs:809`), whatever `--ignore-last-result` says, so a fault reached with
/// a different last result is still a difference.
#[test]
fn faults_keep_the_last_result() {
    let a = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), LT, (LIT, 3), 1, 1),
            expr((TEMP, 1), DIV, (TEMP, 2), T, T),
        ],
    );
    let b = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), GE, (LIT, 3), 1, 1),
            expr((TEMP, 1), DIV, (TEMP, 2), T, T),
        ],
    );
    let ignoring = LiveOptions {
        observe_last_result: false,
        ..behavioral()
    };
    assert_not_equivalent(&live(&a, &b, &ignoring));
    // A fault of another kind is another fault.
    let trapping = tree(4096, 0, 0, vec![expr((TEMP, 1), DIV, (TEMP, 2), T, T)]);
    let erroring = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 2), EQ, (LIT, 0), 1, 2),
            expr((TEMP, 2), EQ, (LIT, 0), ERR, ERR),
            expr((TEMP, 1), DIV, (TEMP, 2), T, T),
        ],
    );
    assert_never_equivalent(&trapping, &erroring);
}

/// Mutating a final temp or the return value is never hidden by the relation.
#[test]
fn returned_values_and_temps_are_observed() {
    let a = tree(
        4096,
        0,
        0,
        vec![sound(1, 1), expr((TEMP, 0), SET, (LIT, 4), T, T)],
    );
    let other_temp = tree(
        4096,
        0,
        0,
        vec![sound(1, 1), expr((TEMP, 0), SET, (LIT, 5), T, T)],
    );
    let other_return = tree(
        4096,
        0,
        0,
        vec![sound(1, 1), expr((TEMP, 0), SET, (LIT, 4), F, F)],
    );
    assert_never_equivalent(&a, &other_temp);
    assert_never_equivalent(&a, &other_return);
    // A sleep's duration is the parameter it reads.
    let sleeping = |ticks: i16| {
        tree(
            4096,
            1,
            0,
            vec![
                expr((PARAM, 0), SET, (LIT, ticks), 1, 1),
                prim(SLEEP, [0; 8], T, T),
            ],
        )
    };
    assert_never_equivalent(&sleeping(5), &sleeping(6));
}

// ============================================================================
// Whole files and the CLI
// ============================================================================

fn bhav_chunk(bhav: &BhavChunk, label: &str) -> ChunkData {
    ChunkData {
        tag: *b"BHAV",
        id: bhav.id,
        flags: 0x10,
        label: label.to_owned(),
        data: encode_bhav(bhav).expect("encode_bhav"),
    }
}

/// Two trees that call each other, each laid out in reverse in file B; tree 4098 loops
/// on an animation and calls 4096 from its body.
fn mutual_files(break_pong: bool) -> (Vec<u8>, Vec<u8>) {
    let module =
        compile_air(r#"tree 4096 "Ping" { if call 4097(temps) { return true; } return false; }"#)
            .expect("compile .air");
    // The compiled call, with the bytes the compiler gives it.
    let call = |callee: u16, on_true, on_false| Instruction {
        opcode: callee,
        on_true,
        on_false,
        ..module.behaviors[&4096].instructions[0]
    };
    let ping = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 1), SET, (LIT, 0), 1, 1),
            sound(1, 2),
            call(4097, 3, F),
            expr((TEMP, 1), SET, (LIT, 5), T, T),
        ],
    );
    let pong = tree(
        4097,
        0,
        0,
        vec![
            expr((TEMP, 0), SET, (LIT, 1), 1, 1),
            sound(2, 2),
            call(4096, F, 3),
            expr((TEMP, 2), SET, (LIT, 1), T, T),
        ],
    );
    let looper = tree(
        4098,
        1,
        0,
        vec![
            animate(5, 0, T, 1),
            expr((TEMP, 0), ADD, (LIT, 1), 2, 2),
            call(4096, 0, 0),
        ],
    );
    let mut pong_b = reversed(&pong);
    if break_pong {
        pong_b.instructions[0] = expr((TEMP, 0), SET, (LIT, 2), 3, 3);
    }
    let a = write_iff(&[
        bhav_chunk(&ping, "Ping"),
        bhav_chunk(&pong, "Pong"),
        bhav_chunk(&looper, "Looper"),
    ])
    .expect("write");
    let b = write_iff(&[
        bhav_chunk(&reversed(&ping), "Ping"),
        bhav_chunk(&pong_b, "Pong"),
        bhav_chunk(&reversed(&looper), "Looper"),
    ])
    .expect("write");
    (a, b)
}

// ============================================================================
// What the frame's shape and the VM's side effects still decide
// ============================================================================

/// Push Interaction checks its frame operand itself and fails differently one slot past
/// the end of the frame than further out, so a primitive's out-of-frame operand keeps
/// its distance from the end even when frame sizes are abstracted.
#[test]
fn out_of_frame_primitive_operands_keep_their_distance() {
    const PUSH_INTERACTION: u16 = 13;
    // Interaction 0 of the object named by local 2 (flag 0x02).
    let push = || prim(PUSH_INTERACTION, [0, 2, 0, 2, 0, 0, 0, 0], T, F);
    let one_local = tree(4096, 0, 1, vec![push()]);
    let two_locals = tree(4096, 0, 2, vec![push()]);
    assert_never_equivalent(&one_local, &two_locals);
    assert_inconclusive_because(
        &live(&one_local, &two_locals, &behavioral()),
        "outside the frame by different amounts",
    );
}

/// The argument count and whether a frame has any word decide how a tree can be entered
/// (a caller supplies the arguments; Create New Object Instance writes the bottom frame's
/// first word), in every mode.
#[test]
fn frame_shape_is_observed() {
    let body = || vec![expr((TEMP, 0), SET, (LIT, 1), T, F)];
    for (a, b) in [
        (tree(4096, 4, 1, body()), tree(4096, 5, 1, body())),
        (tree(4096, 0, 0, body()), tree(4096, 0, 1, body())),
    ] {
        for opts in [strict(), behavioral()] {
            assert_inconclusive_because(&live(&a, &b, &opts), "frames differ in shape");
            assert_inconclusive_because(&live(&b, &a, &opts), "frames differ in shape");
        }
    }
    // The shape is the one the VM uses: the local count is a byte, so 256 locals are
    // none, and format 0x8000 always has four arguments.
    let wide = tree(4096, 0, 256, body());
    let mut fixed_four = tree(4096, 5, 1, body());
    fixed_four.format = 0x8000;
    for (a, b) in [
        (tree(4096, 0, 1, body()), wide),
        (tree(4096, 5, 1, body()), fixed_four),
    ] {
        assert_inconclusive_because(&live(&a, &b, &behavioral()), "frames differ in shape");
    }
    // Any number of locals past the first is still abstracted.
    assert_equivalent(&live(
        &tree(4096, 0, 1, body()),
        &tree(4096, 0, 7, body()),
        &behavioral(),
    ));
}

/// A store to memory the checker does not track may run code (selecting a Sim runs its
/// placement callback) that leaves the scratch pointer in another state, so it is not
/// assumed to be the one the `Expression` set before the store.
#[test]
fn untracked_writes_leave_the_scratch_pointer_unknown() {
    let write = |next| expr((GLOBAL, 3), SET, (LIT, 1), next, next);
    let a = tree(4096, 0, 1, vec![write(T)]);
    let b = tree(
        4096,
        0,
        1,
        vec![write(1), expr((TEMP, 0), SET, (TEMP, 0), T, T)],
    );
    assert_never_equivalent(&a, &b);
    assert_inconclusive_because(
        &live(&a, &b, &behavioral()),
        "scratch-pointer state at exit",
    );
}

/// Animate's event local is a dead byte without flag 0x20 for the live checker only.
/// `pad` clauses keep the canonical operands' fill (bytes 3 and 7), so existing `.air`
/// compiles to the same bytes; a call before the node may read byte 2 as an argument.
#[test]
fn animate_event_local_is_dead_only_in_live_events() {
    let src = "tree 4096 \"Padded\" (format: 0x8002, kind: 0, args: 1, locals: 1, revision: 0) {\n    @n0:\n    animate(clip: 1, table: 3, flags: 0, events: 1, local: 0) pad 0xff -> true: true, false: false;\n}\n";
    let file = compile_air(src).expect("compiles");
    assert_eq!(
        file.behaviors[&4096].instructions[0].operands,
        [1, 0, 0, 0xFF, 3, 0, 1, 0xFF]
    );
    // Two trees that differ only in that byte prove equivalent behaviorally.
    let a = tree(4096, 1, 1, vec![animate(1, 0, T, F)]);
    let mut b = a.clone();
    b.instructions[0].operands[2] = 9;
    assert_equivalent(&live(&a, &b, &behavioral()));
}

#[test]
fn whole_files_prove_when_every_tree_does() {
    let (a, b) = mutual_files(false);
    let opts = |live| IffEquivOptions {
        live,
        ..IffEquivOptions::default()
    };
    let report = check_iff_equivalent(&a, &b, &opts(behavioral())).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
    let report = check_iff_equivalent(&a, &b, &opts(strict())).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
    // One tree of the cycle now leaves a different temp: the file is not proven, and
    // neither is any tree that may run it.
    let (a, b) = mutual_files(true);
    let report = check_iff_equivalent(&a, &b, &opts(behavioral())).expect("check");
    assert_ne!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
    for id in [4096, 4097, 4098] {
        let r = report.resources.iter().find(|r| r.id == id).unwrap();
        assert!(
            !format!("{:?}", r.verdict).starts_with("Equivalent"),
            "{report:#?}"
        );
    }
}

/// In co-op, storing to global 3 selects a Sim, which can run object callbacks. A tree
/// that stores it may run any tree of its file, so verifying it alone is not enough.
#[test]
fn selecting_a_sim_may_run_any_tree() {
    let selects = tree(4096, 0, 1, vec![expr((GLOBAL, 3), SET, (LIT, 1), T, T)]);
    let callback = |value| tree(4097, 0, 1, vec![expr((TEMP, 0), SET, (LIT, value), T, T)]);
    let file = |value| {
        write_iff(&[
            bhav_chunk(&selects, "Select"),
            bhav_chunk(&callback(value), "Callback"),
        ])
        .expect("write")
    };
    let opts = IffEquivOptions {
        live: behavioral(),
        tree: Some(4096),
        ..IffEquivOptions::default()
    };
    let report = check_iff_equivalent(&file(1), &file(2), &opts).expect("check");
    assert_ne!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
    // A comparison with global 3 only reads it.
    let reads = tree(4096, 0, 1, vec![expr((GLOBAL, 3), EQ, (LIT, 1), T, F)]);
    let a = write_iff(&[
        bhav_chunk(&reads, "Select"),
        bhav_chunk(&callback(1), "Callback"),
    ])
    .expect("write");
    let b = write_iff(&[
        bhav_chunk(&reads, "Select"),
        bhav_chunk(&callback(2), "Callback"),
    ])
    .expect("write");
    let report = check_iff_equivalent(&a, &b, &opts).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("behavioral-equiv-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn verify_equiv_cli_behavioral_flag() {
    let dir = scratch_dir("cli");
    let run = |break_pong: bool, flags: &[&str]| {
        let (a, b) = mutual_files(break_pong);
        let (path_a, path_b) = (dir.join("a.iff"), dir.join("b.iff"));
        std::fs::write(&path_a, &a).unwrap();
        std::fs::write(&path_b, &b).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_sims-antics"))
            .arg("verify-equiv")
            .arg(&path_a)
            .arg(&path_b)
            .args(flags)
            .output()
            .unwrap();
        (
            out.status.code(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    };
    let (code, text) = run(false, &["--behavioral"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("EQUIVALENT"), "{text}");
    let (code, text) = run(false, &[]);
    assert_eq!(code, Some(2), "{text}");
    assert!(text.contains("INCONCLUSIVE"), "{text}");
    let (code, text) = run(true, &["--behavioral"]);
    assert_ne!(code, Some(0), "{text}");
}

// ============================================================================
// Install sweep
// ============================================================================

/// `SIMS_INSTALL`-gated: every tree of the install against the same tree laid out in
/// reverse ([`reversed`]), under the behavioral relation. The two runs do the same
/// thing, so the checker must never report a counterexample. Prints counts only, never
/// names or bytes. Skips when `SIMS_INSTALL` is unset.
#[test]
fn install_trees_match_their_reversed_layout() {
    let Some(root) = std::env::var("SIMS_INSTALL")
        .ok()
        .filter(|r| !r.trim().is_empty())
    else {
        eprintln!("behavioral_equiv install sweep: skipped (SIMS_INSTALL unset)");
        return;
    };
    let install = Install::detect(&PathBuf::from(root)).expect("SIMS_INSTALL is not an install");
    let index = Index::build(&install).expect("index SIMS_INSTALL");
    let pool = ArchivePool::new();
    let opts = behavioral();
    // The global library's argument counts, as `verify-equiv --global` supplies them:
    // without them a call into it with spilled arguments is undecidable.
    let global: BTreeMap<u16, usize> = index
        .resources
        .iter()
        .filter(|r| {
            let name = r.name().to_ascii_lowercase();
            name == "global.iff" || name.ends_with("/global.iff")
        })
        .filter_map(|r| r.read_with(&pool).ok())
        .filter_map(|bytes| sims_antics::verify::library_arities_from_iff(&bytes).ok())
        .flat_map(|arities| arities.into_iter())
        .filter_map(|(id, n)| Some((id, n?)))
        .filter(|(id, _)| (256..4096).contains(id))
        .collect();
    eprintln!(
        "behavioral_equiv install sweep: {} global library arities",
        global.len()
    );
    let (mut total, mut equivalent, mut inconclusive, mut counterexamples) = (0, 0, 0, 0);
    let mut first_problem = None;
    // Why trees stay inconclusive, with numbers blanked so like reasons group together.
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for resource in &index.resources {
        let name = resource.name();
        if !name.to_ascii_lowercase().ends_with(".iff") {
            continue;
        }
        let Some(iff) = resource
            .read_with(&pool)
            .ok()
            .and_then(|b| Iff::parse(b).ok())
        else {
            continue;
        };
        let mut arities: BTreeMap<u16, usize> = global.clone();
        let own: BTreeMap<u16, usize> = iff
            .chunks
            .iter()
            .filter(|c| c.flags & DELETED == 0 && &c.tag == b"BHAV")
            .filter_map(|c| {
                let b = Behavior::parse(iff.data(c)).ok()?;
                let args = match b.format {
                    0x8000 => 4,
                    0x8001 => usize::from(b.args).min(4),
                    _ => usize::from(b.args),
                };
                Some((c.id, args))
            })
            .collect();
        arities.extend(own.into_iter().filter(|(id, _)| (4096..8192).contains(id)));
        let ctx = LiveContext {
            callee_arities: Some(arities),
            ..LiveContext::default()
        };
        for chunk in &iff.chunks {
            if chunk.flags & DELETED != 0 || &chunk.tag != b"BHAV" {
                continue;
            }
            let Ok(bhav) = lift_bhav(chunk.id, "", iff.data(chunk)) else {
                continue;
            };
            // A kind-4 tree is entered at any node, so its layout is its interface.
            if bhav.kind == 4 || bhav.instructions.len() < 3 {
                continue;
            }
            total += 1;
            match check_bhav_live_equivalent(&bhav, &ctx, &reversed(&bhav), &ctx, &opts) {
                EquivResult::Equivalent { .. } => equivalent += 1,
                EquivResult::Inconclusive { reason } => {
                    inconclusive += 1;
                    // The bisimulation's failure says why a loop stays unproven.
                    let reason = match reason.split_once("cut-point bisimulation failed: ") {
                        Some((_, why)) => format!("bisimulation: {why}"),
                        None => reason,
                    };
                    let blanked: String = reason
                        .chars()
                        .map(|c| if c.is_ascii_digit() { '#' } else { c })
                        .take(220)
                        .collect();
                    *reasons.entry(blanked).or_default() += 1;
                }
                EquivResult::Counterexample { divergence, .. } => {
                    counterexamples += 1;
                    first_problem
                        .get_or_insert_with(|| format!("BHAV #{}: {divergence}", chunk.id));
                }
            }
        }
    }
    eprintln!(
        "behavioral_equiv install sweep: total={total} equivalent={equivalent} \
         inconclusive={inconclusive} counterexamples={counterexamples}"
    );
    let mut common: Vec<_> = reasons.into_iter().collect();
    common.sort_by(|x, y| y.1.cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
    for (reason, count) in common.iter().take(25) {
        eprintln!("behavioral_equiv install sweep: inconclusive {count}x: {reason}");
    }
    assert!(total > 0, "no trees found under SIMS_INSTALL");
    assert_eq!(counterexamples, 0, "{first_problem:?}");
}
