//! Verifier round 2 (Track 7): last-result liveness before nodes that set it first,
//! per-string `$Local` reads of `Dialog`/`Add/Change Action String`, renaming locals a
//! primitive reads by value, Random Number's definite write, primitives that cannot see
//! the scratch pointer, pure reads of the running object's data words, and dead call
//! arguments. Each rule has a case it proves and adversarial cases it must not. Every
//! tree and string here is synthetic.

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

// Expression scopes and operators.
const MY_DATA: u8 = 3;
const LIT: u8 = 7;
const TEMP: u8 = 8;
const PARAM: u8 = 9;
const PARAM_ATTR: u8 = 22;
const LOCAL: u8 = 25;
const ATTR: u8 = 0;
const GLOBAL: u8 = 6;
const EQ: u8 = 2;
const ADD: u8 = 3;
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

fn behavioral() -> LiveOptions {
    LiveOptions {
        behavioral: true,
        ..LiveOptions::default()
    }
}

/// `--ignore-fault-last-result`.
fn fault_last_result_unread(base: LiveOptions) -> LiveOptions {
    LiveOptions {
        observe_fault_last_result: false,
        ..base
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
fn assert_not_equivalent(result: &EquivResult) {
    assert!(
        !matches!(result, EquivResult::Equivalent { .. }),
        "expected a non-Equivalent verdict, got {result:?}"
    );
}

fn bhav_chunk(bhav: &BhavChunk) -> ChunkData {
    ChunkData {
        tag: *b"BHAV",
        id: bhav.id,
        flags: 0x10,
        label: bhav.label.clone(),
        data: encode_bhav(bhav).expect("encode_bhav"),
    }
}

/// A version -3 string table of US English strings, with resource flags `flags`.
fn strings(id: u16, flags: u16, texts: &[&str]) -> ChunkData {
    let mut data = Vec::new();
    data.extend_from_slice(&(-3i16).to_le_bytes());
    data.extend_from_slice(&u16::try_from(texts.len()).unwrap().to_le_bytes());
    for text in texts {
        data.push(1);
        data.extend_from_slice(text.as_bytes());
        data.extend_from_slice(b"\0\0");
    }
    ChunkData {
        tag: *b"STR#",
        id,
        flags,
        label: String::new(),
        data,
    }
}

fn file(chunks: Vec<ChunkData>) -> Vec<u8> {
    write_iff(&chunks).expect("write")
}

fn verdict_of(report: &sims_antics::IffEquivReport, id: u16) -> &ResourceVerdict {
    &report
        .resources
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("no resource #{id}: {report:#?}"))
        .verdict
}

#[track_caller]
fn assert_tree_equivalent(report: &sims_antics::IffEquivReport, id: u16) {
    assert!(
        matches!(verdict_of(report, id), ResourceVerdict::Equivalent { .. }),
        "#{id}: {report:#?}"
    );
}

#[track_caller]
fn assert_tree_not_equivalent(report: &sims_antics::IffEquivReport, id: u16) {
    assert!(
        !matches!(
            verdict_of(report, id),
            ResourceVerdict::Equivalent { .. } | ResourceVerdict::Identical
        ),
        "#{id}: {report:#?}"
    );
}

fn check(a: &[u8], b: &[u8], live: LiveOptions) -> sims_antics::IffEquivReport {
    let opts = IffEquivOptions {
        live,
        ..IffEquivOptions::default()
    };
    check_iff_equivalent(a, b, &opts).expect("check")
}

// ----------------------------------------------------------------------------
// Last result before a node that sets it first
// ----------------------------------------------------------------------------

/// `temp[0] == lhs` with both branches to node 1, then `next`.
fn compare_then(lhs: i16, next: Instruction) -> BhavChunk {
    tree(
        4096,
        0,
        0,
        vec![expr((TEMP, 0), EQ, (LIT, lhs), 1, 1), next],
    )
}

/// Refresh (7) never yields, runs no tree and sets the last result when it branches:
/// the comparison's result before it is dead once faults are assumed not to expose it.
#[test]
fn last_result_before_a_node_that_sets_it_is_dead_when_faults_hide_it() {
    let a = compare_then(1, prim(7, [0; 8], T, F));
    let b = compare_then(2, prim(7, [0; 8], T, F));
    for base in [LiveOptions::default(), behavioral()] {
        // A fault keeps the last result, so by default it is compared.
        assert_not_equivalent(&live_with(&a, &b, &base));
        assert_equivalent(&live_with(&a, &b, &fault_last_result_unread(base)));
    }
}

/// Idle (17) and Gosub Found Action (30) may yield or run a tree that reads the last
/// result, and Push Interaction (13) may run one: none of them hides it.
#[test]
fn last_result_before_a_node_that_may_read_it_stays_observed() {
    for opcode in [13, 17, 30] {
        let a = compare_then(1, prim(opcode, [0; 8], T, F));
        let b = compare_then(2, prim(opcode, [0; 8], T, F));
        let result = live_with(&a, &b, &fault_last_result_unread(behavioral()));
        assert_not_equivalent(&result);
    }
}

/// An `Expression` writing global 3 (the selected person in co-op mode) may move the
/// selection and run a tree; one writing a temporary sets the result first.
#[test]
fn last_result_before_a_global_write_stays_observed() {
    let opts = fault_last_result_unread(behavioral());
    let temp_write = |lhs| compare_then(lhs, expr((TEMP, 1), SET, (LIT, 4), T, T));
    // The temporary write is no event, so the last result flows on to the exit, where
    // both sides hold `true` from the second `Expression`.
    assert_equivalent(&live_with(&temp_write(1), &temp_write(2), &opts));
    let global_write = |lhs| compare_then(lhs, expr((GLOBAL, 3), SET, (LIT, 4), T, T));
    assert_not_equivalent(&live_with(&global_write(1), &global_write(2), &opts));
}

/// A call to a private tree of the same file whose first node sets the last result
/// hides it; an indexed (kind 4) callee or one that starts with Idle does not.
#[test]
fn a_callee_that_sets_the_last_result_first_hides_it() {
    let callee_first = |first: Instruction, kind: u8| {
        let mut callee = tree(4097, 0, 0, vec![first]);
        callee.kind = kind;
        callee
    };
    let files = |callee: &BhavChunk| {
        let side = |lhs| {
            file(vec![
                bhav_chunk(&compare_then(lhs, prim(4097, [0; 8], T, F))),
                bhav_chunk(callee),
            ])
        };
        (side(1), side(2))
    };
    let opts = fault_last_result_unread(behavioral());
    let (a, b) = files(&callee_first(prim(7, [0; 8], T, F), 0));
    assert_tree_equivalent(&check(&a, &b, opts), 4096);
    // By default the fault inside the callee may expose it.
    assert_tree_not_equivalent(&check(&a, &b, behavioral()), 4096);
    for callee in [
        callee_first(prim(7, [0; 8], T, F), 4),
        callee_first(prim(17, [0; 8], T, F), 0),
    ] {
        let (a, b) = files(&callee);
        assert_tree_not_equivalent(&check(&a, &b, opts), 4096);
    }
}

/// Only one side's callee sets the result first: the values are equated only when both
/// sides hide them.
#[test]
fn a_last_result_hidden_on_one_side_only_stays_observed() {
    let side = |lhs, callee_first: Instruction| {
        file(vec![
            bhav_chunk(&compare_then(lhs, prim(4097, [0; 8], T, F))),
            bhav_chunk(&tree(4097, 0, 0, vec![callee_first])),
        ])
    };
    let a = side(1, prim(7, [0; 8], T, F));
    let b = side(2, prim(17, [0; 8], T, F));
    let report = check(&a, &b, fault_last_result_unread(behavioral()));
    assert_tree_not_equivalent(&report, 4096);
}

// ----------------------------------------------------------------------------
// Text locals: what a Dialog's strings may read
// ----------------------------------------------------------------------------

/// Dialog (36) of type 0 showing message string `message - 1` of STR# 301.
fn dialog(message: u8) -> [u8; 8] {
    [0, 0, message, 0, 0, 0, 0, 0]
}

/// `local[slot] := 5`, then the dialog.
fn local_then_dialog(slot: i16, message: u8) -> BhavChunk {
    tree(
        4096,
        0,
        4,
        vec![
            expr((LOCAL, slot), SET, (LIT, 5), 1, 1),
            prim(36, dialog(message), T, T),
        ],
    )
}

fn dialog_files(slot_b: i16, message: u8, table: Option<ChunkData>) -> (Vec<u8>, Vec<u8>) {
    let side = |slot| {
        let mut chunks = vec![bhav_chunk(&local_then_dialog(slot, message))];
        chunks.extend(table.clone());
        file(chunks)
    };
    (side(0), side(slot_b))
}

/// The message names a string with no token, so the dialog reads no local and the
/// `let` may sit in another slot, though another string of the table holds `$Me`.
#[test]
fn a_dialog_string_without_tokens_lets_locals_be_renamed() {
    let table = strings(301, 0x10, &["Plain words.", "Hello $Me, $Local:0."]);
    let (a, b) = dialog_files(2, 1, Some(table));
    assert_tree_equivalent(&check(&a, &b, behavioral()), 4096);
}

/// The string the dialog formats names the local (or holds `$Me`, whose name may hold
/// any token), so the slot number is observable.
#[test]
fn a_dialog_string_that_may_read_the_local_pins_it() {
    for (message, texts) in [
        (1, ["Costs $Local:0.", "x"]),
        (2, ["x", "Hello $Me."]),
        (1, ["At $TimeLocal:0", "x"]),
        (1, ["$Local:1$Local:2", "x"]),
    ] {
        let (a, b) = dialog_files(2, message, Some(strings(301, 0x10, &texts)));
        assert_tree_not_equivalent(&check(&a, &b, behavioral()), 4096);
    }
}

/// A table the loader does not read by index (no flag 0x10 here: its header is
/// byte-swapped) counts every string at every index.
#[test]
fn a_table_not_read_by_index_counts_every_string() {
    let table = strings(301, 0, &["Plain words.", "Hello $Me."]);
    let (a, b) = dialog_files(2, 1, Some(table));
    assert_tree_not_equivalent(&check(&a, &b, behavioral()), 4096);
}

/// Without a table the dialog shows a default label (or fails) and reads no local.
#[test]
fn a_dialog_without_a_table_reads_no_local() {
    let (a, b) = dialog_files(2, 1, None);
    assert_tree_equivalent(&check(&a, &b, behavioral()), 4096);
}

/// The renamed slot must still hold the same value where the text reads it: B's dialog
/// reads `local[0]`, which B never wrote.
#[test]
fn a_dialog_reading_a_renamed_away_slot_is_not_equivalent() {
    let table = strings(301, 0x10, &["Costs $Local:0."]);
    let (a, b) = dialog_files(3, 1, Some(table));
    assert_tree_not_equivalent(&check(&a, &b, behavioral()), 4096);
    // Strict mode never renames.
    let (a, b) = dialog_files(2, 1, Some(strings(301, 0x10, &["Plain words."])));
    assert_tree_not_equivalent(&check(&a, &b, LiveOptions::default()), 4096);
}

// ----------------------------------------------------------------------------
// Locals a primitive reads by value
// ----------------------------------------------------------------------------

/// Alter Budget (25) by `local[slot]`.
fn budget_from(slot: i16) -> [u8; 8] {
    let mut ops = [2, 0, 0, 0, 0, 0, 0, 0];
    ops[2..4].copy_from_slice(&slot.to_le_bytes());
    ops
}

fn store_then_budget(slot: i16, value: i16) -> BhavChunk {
    tree(
        4096,
        0,
        4,
        vec![
            expr((LOCAL, slot), SET, (LIT, value), 1, 1),
            prim(25, budget_from(slot), T, F),
        ],
    )
}

/// The primitive reads the local's value, so the same value in another slot is the same
/// behavior; strict mode still observes the slot.
#[test]
fn a_local_read_by_value_may_be_renamed() {
    let (a, b) = (store_then_budget(0, 9), store_then_budget(3, 9));
    assert_equivalent(&live_with(&a, &b, &behavioral()));
    assert_not_equivalent(&live_with(&a, &b, &LiveOptions::default()));
    assert_not_equivalent(&live_with(&a, &store_then_budget(3, 8), &behavioral()));
}

// ----------------------------------------------------------------------------
// Random Number's definite write
// ----------------------------------------------------------------------------

/// Random Number (8) into `local[slot]`, range 10.
fn random_into(slot: i16) -> [u8; 8] {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&slot.to_le_bytes());
    ops[2] = LOCAL;
    ops[4] = 10;
    ops[6] = LIT;
    ops
}

/// Random Number always stores its draw: the slot it draws into holds the same value
/// on both sides afterwards, so a copy of it is the same, wherever it went.
#[test]
fn random_number_writes_its_destination() {
    let body = |slot: i16| {
        tree(
            4096,
            0,
            4,
            vec![
                prim(8, random_into(slot), 1, 1),
                expr((TEMP, 0), SET, (LOCAL, slot), T, T),
            ],
        )
    };
    assert_equivalent(&live_with(&body(0), &body(0), &LiveOptions::default()));
    assert_equivalent(&live_with(&body(0), &body(3), &behavioral()));
    // Another local than the draw's is another value.
    let other = tree(
        4096,
        0,
        4,
        vec![
            prim(8, random_into(3), 1, 1),
            expr((TEMP, 0), SET, (LOCAL, 2), T, T),
        ],
    );
    assert_not_equivalent(&live_with(&body(0), &other, &behavioral()));
    // Two draws are two values.
    let twice = |second: i16| {
        tree(
            4096,
            0,
            4,
            vec![
                prim(8, random_into(0), 1, 1),
                prim(8, random_into(1), 2, 2),
                expr((TEMP, 0), SET, (LOCAL, second), T, T),
            ],
        )
    };
    assert_not_equivalent(&live_with(&twice(0), &twice(1), &behavioral()));
}

// ----------------------------------------------------------------------------
// Primitives that cannot see the scratch pointer
// ----------------------------------------------------------------------------

/// `temp[0] == 1` (an `Expression`, which sets the scratch pointer, and here only that
/// and the last result) before or after `middle`, then Random Number into `temp[1]`,
/// which sets it again.
fn staged(expression_first: bool, middle: Instruction) -> BhavChunk {
    let assign = |next| expr((TEMP, 0), EQ, (LIT, 1), next, next);
    let mut random = [0u8; 8];
    random[2] = TEMP;
    random[0] = 1;
    random[4] = 10;
    random[6] = LIT;
    let mut middle = middle;
    let nodes = if expression_first {
        middle.on_true = 2;
        middle.on_false = 2;
        vec![assign(1), middle, prim(8, random, T, T)]
    } else {
        middle.on_true = 1;
        middle.on_false = 1;
        vec![middle, assign(2), prim(8, random, T, T)]
    };
    tree(4096, 0, 0, nodes)
}

/// Refresh (7) cannot read the scratch pointer, so the `Expression` may move across it:
/// the state differs only at Refresh's fault, which is compared unless assumed unread.
#[test]
fn a_scratch_blind_primitive_does_not_observe_the_scratch_pointer() {
    let refresh = prim(7, [0; 8], 0, 0);
    let (a, b) = (staged(true, refresh.clone()), staged(false, refresh));
    let opts = LiveOptions {
        observe_last_result: false,
        observe_fault_last_result: false,
        ..behavioral()
    };
    assert_not_equivalent(&live_with(&a, &b, &opts));
    let fault_unread = LiveOptions {
        observe_fault_scratch_pointer: false,
        ..opts
    };
    assert_equivalent(&live_with(&a, &b, &fault_unread));
}

/// Push Interaction (13) may run an at-once interaction's trees as this object, which
/// may read the scratch pointer: it observes it.
#[test]
fn push_interaction_observes_the_scratch_pointer() {
    let push = prim(13, [0; 8], 0, 0);
    let (a, b) = (staged(true, push.clone()), staged(false, push));
    let opts = LiveOptions {
        observe_last_result: false,
        observe_fault_last_result: false,
        observe_fault_scratch_pointer: false,
        ..behavioral()
    };
    assert_not_equivalent(&live_with(&a, &b, &opts));
}

/// Random Number sets the scratch pointer before anything else, so before its fault too:
/// a different state before it is never seen. Refresh keeps it, so there it is.
#[test]
fn a_primitive_that_sets_the_scratch_pointer_first_hides_it_at_its_fault() {
    let mut random = [0u8; 8];
    random[2] = TEMP;
    random[0] = 1;
    random[4] = 10;
    random[6] = LIT;
    let with = |extra: bool, last: Instruction| {
        let mut nodes = Vec::new();
        if extra {
            nodes.push(expr((TEMP, 2), EQ, (TEMP, 2), 1, 1));
        }
        nodes.push(last);
        tree(4096, 0, 0, nodes)
    };
    let opts = LiveOptions {
        observe_last_result: false,
        observe_fault_last_result: false,
        ..behavioral()
    };
    let random_last = prim(8, random, T, T);
    assert_equivalent(&live_with(
        &with(true, random_last.clone()),
        &with(false, random_last),
        &opts,
    ));
    let refresh_last = prim(7, [0; 8], T, T);
    assert_not_equivalent(&live_with(
        &with(true, refresh_last.clone()),
        &with(false, refresh_last),
        &opts,
    ));
}

/// Get Direction To checks its destination scope word before it sets the scratch
/// pointer (`direction.rs:45`): with a word above 255 it faults first and leaves the
/// state as it was, so there it does not hide a different state before it.
#[test]
fn get_direction_to_with_a_bad_scope_keeps_the_scratch_pointer() {
    let direction = |scope: u16| {
        let mut ops = [0u8; 8];
        ops[2..4].copy_from_slice(&scope.to_le_bytes());
        prim(12, ops, T, T)
    };
    let with = |extra: bool, last: Instruction| {
        let mut nodes = Vec::new();
        if extra {
            nodes.push(expr((TEMP, 2), EQ, (TEMP, 2), 1, 1));
        }
        nodes.push(last);
        tree(4096, 0, 0, nodes)
    };
    let opts = LiveOptions {
        observe_last_result: false,
        observe_fault_last_result: false,
        ..behavioral()
    };
    assert_equivalent(&live_with(
        &with(true, direction(u16::from(TEMP))),
        &with(false, direction(u16::from(TEMP))),
        &opts,
    ));
    assert_not_equivalent(&live_with(
        &with(true, direction(0x100)),
        &with(false, direction(0x100)),
        &opts,
    ));
}

/// The exit still observes the scratch pointer.
#[test]
fn the_scratch_pointer_at_the_exit_stays_observed() {
    let body = |extra: bool| {
        let mut nodes = vec![prim(7, [0; 8], 1, 1)];
        if extra {
            nodes.push(expr((TEMP, 2), EQ, (TEMP, 2), T, T));
        } else {
            nodes[0].on_true = T;
            nodes[0].on_false = T;
        }
        tree(4096, 0, 0, nodes)
    };
    let opts = LiveOptions {
        observe_last_result: false,
        observe_fault_last_result: false,
        observe_fault_scratch_pointer: false,
        ..behavioral()
    };
    assert_not_equivalent(&live_with(&body(true), &body(false), &opts));
}

/// The CLI flag prints its assumption.
#[test]
fn ignore_fault_scratch_pointer_is_an_assume_line() {
    let refresh = prim(7, [0; 8], 0, 0);
    let side = |first| file(vec![bhav_chunk(&staged(first, refresh.clone()))]);
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("equiv-round2-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let (path_a, path_b) = (dir.join("a.iff"), dir.join("b.iff"));
    std::fs::write(&path_a, side(true)).unwrap();
    std::fs::write(&path_b, side(false)).unwrap();
    let run = |extra: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_sims-antics"))
            .arg("verify-equiv")
            .args([
                "--behavioral",
                "--ignore-last-result",
                "--ignore-fault-last-result",
            ])
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
    assert_ne!(code, Some(0), "{stdout}");
    let (code, stdout) = run(&["--ignore-fault-scratch-pointer"]);
    assert_eq!(code, Some(0), "{stdout}");
    assert!(
        stdout.contains("ASSUME: a path that faults is compared without the scratch-pointer"),
        "{stdout}"
    );
}

// ----------------------------------------------------------------------------
// The running object's data words
// ----------------------------------------------------------------------------

/// `temp[0] := my.data[idx]`, `middle` (if any), then `temp[1] :=` a second read or a
/// copy of `temp[0]`.
fn data_twice(idx: i16, middle: Option<Instruction>, reread: bool) -> BhavChunk {
    let mut nodes = vec![expr((TEMP, 0), SET, (MY_DATA, idx), 1, 1)];
    if let Some(mut m) = middle {
        m.on_true = 2;
        m.on_false = 2;
        nodes.push(m);
    }
    let rhs = if reread { (MY_DATA, idx) } else { (TEMP, 0) };
    nodes.push(expr((TEMP, 1), SET, rhs, T, T));
    tree(4096, 0, 0, nodes)
}

/// Every object has 72 data words, and only an event changes them, so two reads with no
/// event between them read one value, and a read neither faults nor runs anything.
#[test]
fn a_data_word_read_twice_reads_one_value() {
    let opts = behavioral();
    assert_equivalent(&live_with(
        &data_twice(11, None, true),
        &data_twice(11, None, false),
        &opts,
    ));
    // The last word, 71, too.
    assert_equivalent(&live_with(
        &data_twice(71, None, true),
        &data_twice(71, None, false),
        &opts,
    ));
}

/// The lockout word (25) wakes the object when read, and words from 72 on may fault:
/// those reads stay events.
#[test]
fn lockout_and_out_of_range_data_reads_stay_events() {
    for idx in [25, 72, -1] {
        let result = live_with(
            &data_twice(idx, None, true),
            &data_twice(idx, None, false),
            &behavioral(),
        );
        assert_not_equivalent(&result);
    }
}

/// An event between the reads may change the word: no stale value.
#[test]
fn a_data_word_read_after_an_event_is_a_new_value() {
    let refresh = prim(7, [0; 8], 0, 0);
    let result = live_with(
        &data_twice(11, Some(refresh.clone()), true),
        &data_twice(11, Some(refresh), false),
        &behavioral(),
    );
    assert_not_equivalent(&result);
}

// ----------------------------------------------------------------------------
// Dead call arguments
// ----------------------------------------------------------------------------

/// Call (to `callee`, with operand words `args`).
fn call_args(args: [i16; 4]) -> [u8; 8] {
    let mut ops = [0u8; 8];
    for (i, a) in args.iter().enumerate() {
        ops[2 * i..2 * i + 2].copy_from_slice(&a.to_le_bytes());
    }
    ops
}

/// Tree 4096 calls 4097 with `args`; 4097 is `callee`.
fn dead_arg_file(args: [i16; 4], callee: &BhavChunk) -> Vec<u8> {
    file(vec![
        bhav_chunk(&tree(4096, 0, 0, vec![prim(4097, call_args(args), T, F)])),
        bhav_chunk(callee),
    ])
}

/// 4097 with two arguments: `first`, then `temp[0] := param[0] + param[1]`.
fn callee_with(first: Instruction) -> BhavChunk {
    tree(
        4097,
        2,
        0,
        vec![
            first,
            expr((TEMP, 0), SET, (PARAM, 0), 2, 2),
            expr((TEMP, 0), ADD, (PARAM, 1), T, T),
        ],
    )
}

/// The callee overwrites `param[0]` with a literal before reading it: what the caller
/// passes there is dead. Strict mode still compares a faulting node's bytes, and the
/// call may fault, so this holds behaviorally.
#[test]
fn an_argument_the_callee_overwrites_first_is_dead() {
    let callee = callee_with(expr((PARAM, 0), SET, (LIT, 5), 1, 1));
    let a = dead_arg_file([1, 7, 0, 0], &callee);
    let b = dead_arg_file([2, 7, 0, 0], &callee);
    assert_tree_equivalent(&check(&a, &b, behavioral()), 4096);
    assert_tree_not_equivalent(&check(&a, &b, LiveOptions::default()), 4096);
    // The live argument still counts.
    let c = dead_arg_file([1, 8, 0, 0], &callee);
    let report = check(&a, &c, behavioral());
    assert_tree_not_equivalent(&report, 4096);
    assert_ne!(report.verdict(), IffVerdict::Equivalent);
}

/// Reading the parameter before writing it makes the argument live.
#[test]
fn an_argument_read_before_it_is_overwritten_is_live() {
    let callee = tree(
        4097,
        2,
        0,
        vec![
            expr((TEMP, 1), SET, (PARAM, 0), 1, 1),
            expr((PARAM, 0), SET, (LIT, 5), 2, 2),
            expr((TEMP, 0), ADD, (PARAM, 1), T, T),
        ],
    );
    let a = dead_arg_file([1, 7, 0, 0], &callee);
    let b = dead_arg_file([2, 7, 0, 0], &callee);
    assert_tree_not_equivalent(&check(&a, &b, behavioral()), 4096);
    // So does reading it through scope 22 (an attribute of the stack object, indexed by
    // the parameter) on the right.
    let by_attribute = callee_with(expr((TEMP, 1), SET, (PARAM_ATTR, 0), 1, 1));
    let a = dead_arg_file([1, 7, 0, 0], &by_attribute);
    let b = dead_arg_file([2, 7, 0, 0], &by_attribute);
    assert_tree_not_equivalent(&check(&a, &b, behavioral()), 4096);
}

/// Overwriting from a read that may fault (an attribute out of range) leaves the
/// caller's value in the frame at the fault: strict mode observes it, the behavioral
/// relation (no frame at a fault) does not.
#[test]
fn an_argument_overwritten_by_a_faulting_read_is_dead_only_behaviorally() {
    let callee = callee_with(expr((PARAM, 0), SET, (ATTR, 3), 1, 1));
    let a = dead_arg_file([1, 7, 0, 0], &callee);
    let b = dead_arg_file([2, 7, 0, 0], &callee);
    assert_tree_equivalent(&check(&a, &b, behavioral()), 4096);
    // The same chain from a literal is dead behaviorally too, and strict mode keeps
    // both (the faulting call's bytes).
    let literal = callee_with(expr((PARAM, 0), SET, (LIT, 3), 1, 1));
    let a = dead_arg_file([1, 7, 0, 0], &literal);
    let b = dead_arg_file([2, 7, 0, 0], &literal);
    assert_tree_equivalent(&check(&a, &b, behavioral()), 4096);
    assert_tree_not_equivalent(&check(&a, &b, LiveOptions::default()), 4096);
}

/// A parameter no node reads is dead behaviorally (strict mode sees it in the frame).
#[test]
fn a_parameter_nothing_reads_is_dead_only_behaviorally() {
    let callee = tree(4097, 2, 0, vec![expr((TEMP, 0), SET, (PARAM, 0), T, T)]);
    let a = dead_arg_file([1, 7, 0, 0], &callee);
    let b = dead_arg_file([1, 8, 0, 0], &callee);
    assert_tree_equivalent(&check(&a, &b, behavioral()), 4096);
    assert_tree_not_equivalent(&check(&a, &b, LiveOptions::default()), 4096);
    // A primitive that reads it (Sleep for param[1] ticks), or one whose frame use is not
    // modeled (opcode 40), keeps it live.
    for reader in [
        prim(0, [1, 0, 0, 0, 0, 0, 0, 0], T, T),
        prim(40, [0; 8], T, T),
    ] {
        let callee = tree(
            4097,
            2,
            0,
            vec![expr((TEMP, 0), SET, (PARAM, 0), 1, 1), reader],
        );
        let a = dead_arg_file([1, 7, 0, 0], &callee);
        let b = dead_arg_file([1, 8, 0, 0], &callee);
        assert_tree_not_equivalent(&check(&a, &b, behavioral()), 4096);
    }
}

/// A dropped argument stays in frame storage after the callee returns, and a later
/// frame at the same depth starts with it as an uninitialized local. Strict mode keeps
/// the argument. The behavioral relation assumes stale frame words agree
/// (`20261123-verifier-behavioral-mode.md`), so it does not cover this.
#[test]
fn a_dropped_argument_a_later_frame_may_read_stays_strict() {
    let file_passing = |arg: i16| {
        file(vec![
            bhav_chunk(&tree(
                4096,
                0,
                0,
                vec![
                    prim(4097, call_args([arg, 0, 0, 0]), 1, 1),
                    prim(4098, call_args([0; 4]), T, T),
                ],
            )),
            bhav_chunk(&tree(4097, 1, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)])),
            bhav_chunk(&tree(
                4098,
                0,
                1,
                vec![expr((TEMP, 0), SET, (LOCAL, 0), T, T)],
            )),
        ])
    };
    let (a, b) = (file_passing(7), file_passing(8));
    assert_tree_not_equivalent(&check(&a, &b, LiveOptions::default()), 4096);
}

/// An indexed (kind 4) callee takes its first argument as its entry node; a caller
/// outside the private range runs another file's 4097.
#[test]
fn dead_arguments_need_a_private_caller_and_a_plain_callee() {
    let mut indexed = callee_with(expr((PARAM, 0), SET, (LIT, 5), 1, 1));
    indexed.kind = 4;
    let a = dead_arg_file([1, 7, 0, 0], &indexed);
    let b = dead_arg_file([2, 7, 0, 0], &indexed);
    assert_tree_not_equivalent(&check(&a, &b, behavioral()), 4096);

    let callee = callee_with(expr((PARAM, 0), SET, (LIT, 5), 1, 1));
    let semi_global_caller = |arg: i16| {
        file(vec![
            bhav_chunk(&tree(
                8200,
                0,
                0,
                vec![prim(4097, call_args([arg, 7, 0, 0]), T, F)],
            )),
            bhav_chunk(&callee),
        ])
    };
    let report = check(&semi_global_caller(1), &semi_global_caller(2), behavioral());
    assert_tree_not_equivalent(&report, 8200);
}

/// A context's dead arguments apply only with the callee's arity known (at most 4, the
/// operand words), and only below it.
#[test]
fn dead_arguments_need_the_callee_arity() {
    let caller = |arg: i16| {
        tree(
            4096,
            0,
            0,
            vec![prim(4097, call_args([arg, 7, 0, 0]), T, F)],
        )
    };
    let (a, b) = (caller(1), caller(2));
    let ctx = |arity: Option<usize>, dead: &[usize]| LiveContext {
        dead_args: Some(
            [(4097, dead.iter().copied().collect())]
                .into_iter()
                .collect(),
        ),
        callee_arities: arity.map(|n| [(4097, n)].into_iter().collect()),
        ..LiveContext::default()
    };
    let run = |c: &LiveContext| check_bhav_live_equivalent(&a, c, &b, c, &behavioral());
    assert_equivalent(&run(&ctx(Some(2), &[0])));
    for c in [
        ctx(None, &[0]),
        ctx(Some(5), &[0]),
        ctx(Some(2), &[0, 2]),
        ctx(Some(2), &[1]),
    ] {
        assert_not_equivalent(&run(&c));
    }
}

/// `check_tree_with_callees` (`equiv`) takes each side's trees and arities next to its
/// file. They drive dead arguments and last-result overwriters, so a description that
/// is not what the file holds must not prove anything.
#[test]
fn trees_given_with_a_file_must_be_the_files() {
    use sims_antics::EquivOptions;
    use sims_antics::verify::check_tree_with_callees;
    use std::collections::BTreeMap;
    let caller = |arg: i16| {
        tree(
            4096,
            0,
            0,
            vec![prim(4097, call_args([arg, 0, 0, 0]), T, F)],
        )
    };
    let reads = tree(4097, 1, 0, vec![expr((PARAM, 0), EQ, (LIT, 1), T, F)]);
    let ignores = tree(4097, 1, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    let with = |arg: i16, callee: &BhavChunk| {
        let trees = BTreeMap::from([(4096, caller(arg)), (4097, callee.clone())]);
        let bytes = file(vec![bhav_chunk(&trees[&4096]), bhav_chunk(callee)]);
        (trees, bytes)
    };
    let arities = BTreeMap::from([(4096, 0), (4097, 1)]);
    let run = |a: &(BTreeMap<u16, BhavChunk>, Vec<u8>), b: &(BTreeMap<u16, BhavChunk>, Vec<u8>)| {
        check_tree_with_callees(
            4096,
            &a.0,
            Some(&a.1),
            &b.0,
            Some(&b.1),
            (&arities, &arities),
            &EquivOptions::default(),
            &behavioral(),
        )
        .1
    };
    // Described as ignoring its argument, but the files' callee reads it.
    let (mut a, mut b) = (with(1, &reads), with(2, &reads));
    a.0.insert(4097, ignores.clone());
    b.0.insert(4097, ignores.clone());
    match run(&a, &b) {
        EquivResult::Inconclusive { reason } => assert!(reason.contains("#4097"), "{reason}"),
        other => panic!("expected Inconclusive, got {other:?}"),
    }
    // An arity other than the file's.
    let wrong = BTreeMap::from([(4096, 0), (4097, 2)]);
    let (a, b) = (with(1, &ignores), with(2, &ignores));
    let result = check_tree_with_callees(
        4096,
        &a.0,
        Some(&a.1),
        &b.0,
        Some(&b.1),
        (&wrong, &wrong),
        &EquivOptions::default(),
        &behavioral(),
    )
    .1;
    assert_not_equivalent(&result);
    // Described truthfully, the argument the callee ignores is dead, and the one it
    // reads is not.
    assert_equivalent(&run(&a, &b));
    assert_not_equivalent(&run(&with(1, &reads), &with(2, &reads)));
}

// ----------------------------------------------------------------------------
// Temporaries staged again
// ----------------------------------------------------------------------------

/// The same temporaries staged again before each read (as an unoptimized build does)
/// hold the same values at every event: nothing to tolerate.
#[test]
fn temporaries_staged_again_are_the_same_values() {
    // A: temp[0] := param[0]; local[0] := my.data[1]; local[1] := my.data[2]; sound.
    // B: stages temp[0] again between the reads.
    let a = tree(
        4096,
        1,
        2,
        vec![
            expr((TEMP, 0), SET, (PARAM, 0), 1, 1),
            expr((LOCAL, 0), SET, (MY_DATA, 1), 2, 2),
            expr((LOCAL, 1), SET, (MY_DATA, 2), 3, 3),
            prim(25, budget_from(1), T, F),
        ],
    );
    let b = tree(
        4096,
        1,
        2,
        vec![
            expr((TEMP, 0), SET, (PARAM, 0), 1, 1),
            expr((LOCAL, 0), SET, (MY_DATA, 1), 2, 2),
            expr((TEMP, 0), SET, (PARAM, 0), 3, 3),
            expr((LOCAL, 1), SET, (MY_DATA, 2), 4, 4),
            prim(25, budget_from(1), T, F),
        ],
    );
    assert_equivalent(&live_with(&a, &b, &behavioral()));
    // Staging another value is a difference.
    let mut c = b.clone();
    c.instructions[2] = expr((TEMP, 0), SET, (LIT, 3), 3, 3);
    assert_not_equivalent(&live_with(&a, &c, &behavioral()));
}

/// Only one side's callee overwrites the argument unseen: both sides then compare it,
/// and the same argument on both sides is no difference.
#[test]
fn an_argument_dead_on_one_side_only_is_compared() {
    let a_callee = callee_with(expr((PARAM, 0), SET, (LIT, 5), 1, 1));
    // B reads param[0] into temp[1] and overwrites that at once: the same behavior, but
    // the argument is not dead on B's side.
    let b_callee = tree(
        4097,
        2,
        0,
        vec![
            expr((TEMP, 1), SET, (PARAM, 0), 1, 1),
            expr((TEMP, 1), SET, (LIT, 0), 2, 2),
            expr((TEMP, 0), SET, (LIT, 5), 3, 3),
            expr((TEMP, 0), ADD, (PARAM, 1), T, T),
        ],
    );
    let mut a_first = a_callee.clone();
    // Keep temp[1] alike on both sides.
    a_first.instructions[2].on_true = 3;
    a_first.instructions[2].on_false = 3;
    a_first
        .instructions
        .push(expr((TEMP, 1), SET, (LIT, 0), T, T));
    // B's caller also holds a node it never reaches, so the callers are not the same
    // bytes and their call events are compared.
    let b_file = |arg: i16| {
        file(vec![
            bhav_chunk(&tree(
                4096,
                0,
                0,
                vec![
                    prim(4097, call_args([arg, 7, 0, 0]), T, F),
                    expr((TEMP, 3), SET, (LIT, 1), T, T),
                ],
            )),
            bhav_chunk(&b_callee),
        ])
    };
    let a = dead_arg_file([1, 7, 0, 0], &a_first);
    let report = check(&a, &b_file(1), behavioral());
    assert_tree_equivalent(&report, 4097);
    assert_tree_equivalent(&report, 4096);
    assert_tree_not_equivalent(&check(&a, &b_file(2), behavioral()), 4096);
}
