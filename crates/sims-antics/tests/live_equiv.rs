//! Live-output equivalence (`check_bhav_live_equivalent`), the combined lockstep→live
//! checker, and the whole-file `verify-equiv` checker.

use sims_antics::encode::encode_bhav;
use sims_antics::ir::BhavChunk;
use sims_antics::{
    EquivMethod, EquivOptions, EquivResult, IffEquivOptions, IffEquivReport, IffVerdict,
    LiveContext, LiveOptions, ResourceReport, ResourceVerdict, check_bhav_equivalent,
    check_bhav_equivalent_combined, check_bhav_live_equivalent, check_iff_equivalent, compile_air,
};
use sims_formats::behavior::Instruction;
use sims_formats::iff::{ChunkData, write as write_iff};
use std::process::Command;

const T: u8 = 0xFE;
const F: u8 = 0xFF;
const ERR: u8 = 0xFD;

// Expression scopes.
const ATTR: u8 = 0;
const GLOBAL: u8 = 6;
const LIT: u8 = 7;
const TEMP: u8 = 8;
const PARAM: u8 = 9;
const LOCAL: u8 = 25;

// Expression operators.
const GT: u8 = 0;
const LT: u8 = 1;
const EQ: u8 = 2;
const ADD: u8 = 3;
const SET: u8 = 5;
const DIV: u8 = 7;

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

fn air(src: &str, id: u16) -> BhavChunk {
    compile_air(src).expect("compile .air").behaviors[&id].clone()
}

fn live(a: &BhavChunk, b: &BhavChunk) -> EquivResult {
    live_with(a, b, &LiveContext::default(), &LiveOptions::default())
}

fn live_with(a: &BhavChunk, b: &BhavChunk, ctx: &LiveContext, opts: &LiveOptions) -> EquivResult {
    check_bhav_live_equivalent(a, ctx, b, ctx, opts)
}

#[track_caller]
fn assert_equivalent(result: &EquivResult) {
    assert!(
        matches!(result, EquivResult::Equivalent { .. }),
        "expected Equivalent, got {result:?}"
    );
}

#[track_caller]
fn assert_counterexample(result: &EquivResult) {
    assert!(
        matches!(result, EquivResult::Counterexample { .. }),
        "expected Counterexample, got {result:?}"
    );
}

#[track_caller]
fn assert_inconclusive(result: &EquivResult) {
    assert!(
        matches!(result, EquivResult::Inconclusive { .. }),
        "expected Inconclusive, got {result:?}"
    );
}

fn ignoring_last_result() -> LiveOptions {
    LiveOptions {
        observe_last_result: false,
        ..LiveOptions::default()
    }
}

/// Maxis' "CT - Is Fridge" shape: a single `my.attr[0] > my.attr[0]` node whose both
/// branches return true. Reading `my.attr[0]` cannot trap (every object has 8
/// attributes), so the compare is a dead pure read and the tree returns true like
/// `return true;`. It differs in one thing only: its final test is false, and the VM
/// records the last branch result when the frame of a tree an action started is popped
/// (`object_uses.rs:246`). The strict default cannot rule that out, and the replay
/// world (no person, no action) cannot show it, so the result is undecided; ignoring
/// the last result proves the trees equivalent.
#[test]
fn dead_pure_read_dummy_node_equals_return_true() {
    let fridge = tree(4143, 0, 0, vec![expr((ATTR, 0), GT, (ATTR, 0), T, T)]);
    let high_level = air(r#"tree 4143 "CT - Is Fridge" { return true; }"#, 4143);

    // The lockstep checker treats the attribute read as an observable event.
    assert!(!matches!(
        check_bhav_equivalent(&fridge, &high_level, &EquivOptions::default()),
        EquivResult::Equivalent { .. }
    ));
    match live(&fridge, &high_level) {
        EquivResult::Inconclusive { reason } => {
            assert!(reason.contains("last branch result"), "{reason}");
        }
        other => panic!("expected Inconclusive, got {other:?}"),
    }
    let ctx = LiveContext::default();
    assert_equivalent(&live_with(
        &fridge,
        &high_level,
        &ctx,
        &ignoring_last_result(),
    ));
    let (method, result) = check_bhav_equivalent_combined(
        &fridge,
        &ctx,
        &high_level,
        &ctx,
        &EquivOptions::default(),
        &ignoring_last_result(),
    );
    assert_eq!(method, EquivMethod::LiveOutput);
    assert_equivalent(&result);

    // A dummy test that is true leaves the same last result as `return true;`.
    let true_dummy = tree(4143, 0, 0, vec![expr((ATTR, 0), EQ, (ATTR, 0), T, T)]);
    assert_equivalent(&live(&true_dummy, &high_level));

    // Different exits are still told apart: the replay runs both and they return
    // different values.
    let returns_false = air(r#"tree 4143 "X" { return false; }"#, 4143);
    match live_with(&fridge, &returns_false, &ctx, &ignoring_last_result()) {
        EquivResult::Counterexample { divergence, .. } => {
            assert!(
                divergence.contains("sims-vm runs both") && divergence.contains("A returns true"),
                "{divergence}"
            );
        }
        other => panic!("expected Counterexample, got {other:?}"),
    }
}

/// `p < 5 -> true : false` and `p >= 5 -> false : true` return the same, but their own
/// results (the thread's last result) are opposite.
#[test]
fn last_branch_result_is_observed() {
    let lt = tree(4096, 1, 0, vec![expr((PARAM, 0), LT, (LIT, 5), T, F)]);
    let ge = tree(4096, 1, 0, vec![expr((PARAM, 0), 14, (LIT, 5), F, T)]);
    // Observed, but no replay shows it (the replay world has no action to record it).
    assert_inconclusive(&live(&lt, &ge));
    assert_equivalent(&live_with(
        &lt,
        &ge,
        &LiveContext::default(),
        &ignoring_last_result(),
    ));
    // Lockstep matches them unless asked to compare last results.
    assert_equivalent(&check_bhav_equivalent(&lt, &ge, &EquivOptions::default()));
    let strict_lockstep = EquivOptions {
        max_silent_steps: 0,
        observe_last_result: true,
        ..EquivOptions::default()
    };
    assert_counterexample(&check_bhav_equivalent(&lt, &ge, &strict_lockstep));
    let ctx = LiveContext::default();
    let (method, result) = check_bhav_equivalent_combined(
        &lt,
        &ctx,
        &ge,
        &ctx,
        &EquivOptions::default(),
        &LiveOptions::default(),
    );
    assert_eq!(method, EquivMethod::LiveOutput);
    assert_inconclusive(&result);

    // Before a primitive or a call it is observed too, but only as a possible
    // difference: the primitive (Idle, `idle.rs:187`) or the called tree may fail
    // before reading it, and Play Sound never reads it (`sound.rs:64`). Different
    // results after them are still compared at exit.
    let then = |test: Instruction, next: Instruction| tree(4096, 1, 0, vec![test, next]);
    let lt_test = expr((PARAM, 0), LT, (LIT, 5), 1, 1);
    let ge_test = expr((PARAM, 0), 14, (LIT, 5), 1, 1);
    for next in [
        prim(17, [1, 0, 0, 0, 0, 0, 0, 0], T, T),
        prim(0x1000, [0; 8], T, T),
        prim(23, [1, 0, 0, 0, 0, 0, 0, 0], T, T),
    ] {
        assert_inconclusive(&live(
            &then(lt_test.clone(), next.clone()),
            &then(ge_test.clone(), next),
        ));
    }
}

/// A self assignment `temp[0] := temp[0]` returning false leaves nothing behind.
#[test]
fn dummy_self_assignment_equals_return_false() {
    let dummy = tree(4096, 0, 0, vec![expr((TEMP, 0), SET, (TEMP, 0), F, F)]);
    let high_level = air(r#"tree 4096 "X" { return false; }"#, 4096);
    assert_equivalent(&live(&dummy, &high_level));

    let returns_true = air(r#"tree 4096 "X" { return true; }"#, 4096);
    assert_counterexample(&live(&dummy, &returns_true));
}

/// `my.attr[8]` may not exist, so reading it may trap: the dummy node is not provably
/// dead unless the caller says the object has more attributes.
#[test]
fn attribute_read_beyond_guaranteed_count_is_not_dead() {
    let dummy = tree(4096, 0, 0, vec![expr((ATTR, 8), EQ, (ATTR, 8), T, T)]);
    let high_level = air(r#"tree 4096 "X" { return true; }"#, 4096);
    let result = live(&dummy, &high_level);
    assert!(
        !matches!(result, EquivResult::Equivalent { .. }),
        "{result:?}"
    );

    let ctx = LiveContext {
        my_attribute_count: 9,
        ..LiveContext::default()
    };
    assert_equivalent(&live_with(
        &dummy,
        &high_level,
        &ctx,
        &LiveOptions::default(),
    ));
}

#[test]
fn overwritten_and_restored_temps_are_dead() {
    // temp[0] := 5; temp[0] := 7  ==  temp[0] := 7
    let twice = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), SET, (LIT, 5), 1, ERR),
            expr((TEMP, 0), SET, (LIT, 7), T, ERR),
        ],
    );
    let once = tree(4096, 0, 0, vec![expr((TEMP, 0), SET, (LIT, 7), T, ERR)]);
    assert_equivalent(&live(&twice, &once));

    // A different final value is a counterexample.
    let other = tree(4096, 0, 0, vec![expr((TEMP, 0), SET, (LIT, 5), T, ERR)]);
    assert_counterexample(&live(&twice, &other));

    // Saving and restoring a temp around a scratch computation leaves it unchanged.
    let save_restore = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 1), SET, (TEMP, 0), 1, ERR),
            expr((TEMP, 0), ADD, (LIT, 3), 2, ERR),
            expr((TEMP, 0), SET, (TEMP, 1), 3, ERR),
            expr((TEMP, 1), SET, (LIT, 0), T, ERR),
        ],
    );
    let clear_temp1 = tree(4096, 0, 0, vec![expr((TEMP, 1), SET, (LIT, 0), T, ERR)]);
    assert_equivalent(&live(&save_restore, &clear_temp1));
}

#[test]
fn dead_locals_and_strict_frame_residue() {
    // A local left in the frame storage can be read by a later frame at the same
    // depth before it writes it (`interpreter.rs:1216`): observed by default, but only
    // ever a possible difference.
    let writes_local = tree(4096, 0, 1, vec![expr((LOCAL, 0), SET, (LIT, 3), T, ERR)]);
    let nothing = tree(4096, 0, 1, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    assert_inconclusive(&live(&writes_local, &nothing));

    let ignoring = LiveOptions {
        observe_frame_residue: false,
        ..LiveOptions::default()
    };
    assert_equivalent(&live_with(
        &writes_local,
        &nothing,
        &LiveContext::default(),
        &ignoring,
    ));
}

#[test]
fn branches_that_merge_are_equivalent_to_straight_line_code() {
    let branchy = tree(
        4096,
        1,
        0,
        vec![
            expr((PARAM, 0), LT, (LIT, 5), 1, 2),
            expr((ATTR, 1), SET, (LIT, 1), T, ERR),
            expr((ATTR, 1), SET, (LIT, 1), T, ERR),
        ],
    );
    let straight = tree(4096, 1, 0, vec![expr((ATTR, 1), SET, (LIT, 1), T, ERR)]);
    assert_equivalent(&live(&branchy, &straight));

    // Different writes on one branch are found, with a concrete condition.
    let diverging = tree(
        4096,
        1,
        0,
        vec![
            expr((PARAM, 0), LT, (LIT, 5), 1, 2),
            expr((ATTR, 1), SET, (LIT, 1), T, ERR),
            expr((ATTR, 1), SET, (LIT, 2), T, ERR),
        ],
    );
    match live(&diverging, &straight) {
        EquivResult::Counterexample { divergence, .. } => {
            assert!(divergence.contains("my.attr[1]"), "{divergence}");
        }
        other => panic!("expected Counterexample, got {other:?}"),
    }
}

#[test]
fn literal_bounded_loops_fold() {
    let src = r#"
tree 4096 "Loop" {
    temp[0] := 0;
    while temp[0] < 3 {
        temp[0] += 1;
    }
    return true;
}
tree 4097 "Direct" {
    temp[0] := 3;
    return true;
}
"#;
    let module = compile_air(src).expect("compile");
    let looped = module.behaviors[&4096].clone();
    let mut direct = module.behaviors[&4097].clone();
    direct.id = 4096;
    // The loop returns from its failed exit test, so its last result is false: a
    // possible difference no replay shows.
    assert_inconclusive(&live(&looped, &direct));
    assert_equivalent(&live_with(
        &looped,
        &direct,
        &LiveContext::default(),
        &ignoring_last_result(),
    ));
}

#[test]
fn division_by_zero_traps_are_observable() {
    let divide = tree(4096, 0, 0, vec![expr((TEMP, 0), DIV, (TEMP, 1), T, ERR)]);
    let divide_then_dummy = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), DIV, (TEMP, 1), 1, ERR),
            expr((TEMP, 2), SET, (TEMP, 2), T, T),
        ],
    );
    assert_equivalent(&live(&divide, &divide_then_dummy));

    // Guarding the zero case changes a VM trap into `return false`. A path that faults
    // is undecided: the replay counts only runs that return.
    let guarded = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 1), EQ, (LIT, 0), F, 1),
            expr((TEMP, 0), DIV, (TEMP, 1), T, ERR),
        ],
    );
    match live(&divide, &guarded) {
        EquivResult::Inconclusive { reason } => {
            assert!(reason.contains("trap"), "{reason}");
        }
        other => panic!("expected Inconclusive, got {other:?}"),
    }
}

#[test]
fn scratch_pointer_state_before_primitives_is_observed() {
    // PlaySound (opcode 23) always returns true.
    let sound = [1, 0, 0, 0, 0, 0, 0, 0];
    let bare = tree(4096, 0, 0, vec![prim(23, sound, T, T)]);
    let after_expression = tree(
        4096,
        0,
        0,
        vec![expr((LIT, 0), EQ, (LIT, 0), 1, 1), prim(23, sound, T, T)],
    );
    // Play Sound does not read it, and a tree a call runs may or may not (Global 34):
    // observed, but only a possible difference.
    assert_inconclusive(&live(&bare, &after_expression));
    let call = prim(0x1000, [0; 8], T, T);
    assert_inconclusive(&live(
        &tree(4096, 0, 0, vec![call.clone()]),
        &tree(4096, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 0), 1, 1), call]),
    ));
    // The test before the sound also sets the last result, and moves the sound to node
    // 1, where a failing (or yielding) sound leaves the frame: still undecided with
    // neither the scratch pointer nor the last result observed.
    let lenient = LiveOptions {
        observe_scratch_pointer: false,
        observe_last_result: false,
        ..LiveOptions::default()
    };
    match live_with(&bare, &after_expression, &LiveContext::default(), &lenient) {
        EquivResult::Inconclusive { reason } => assert!(reason.contains("node"), "{reason}"),
        other => panic!("expected Inconclusive, got {other:?}"),
    }
    // After the sound, the same test is only a scratch-pointer and last-result change.
    let expression_after = tree(
        4096,
        0,
        0,
        vec![prim(23, sound, 1, 1), expr((LIT, 0), EQ, (LIT, 0), T, T)],
    );
    assert_inconclusive(&live(&bare, &expression_after));
    assert_equivalent(&live_with(
        &bare,
        &expression_after,
        &LiveContext::default(),
        &lenient,
    ));

    // Different primitive operands differ only possibly: both may fail alike.
    let other_sound = tree(4096, 0, 0, vec![prim(23, [2, 0, 0, 0, 0, 0, 0, 0], T, T)]);
    assert_inconclusive(&live(&bare, &other_sound));
}

#[test]
fn temps_passed_to_calls_are_compared_but_dead_ones_after_are_not() {
    // A call reads the temps (`[0xFF; 8]` passes them as arguments), so a temp written
    // before it is observable; one written after it and then overwritten is not.
    let call = |on_true| prim(0x1000, [0xFF; 8], on_true, F);
    let a = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), SET, (LIT, 1), 1, ERR),
            call(2),
            expr((TEMP, 3), SET, (LIT, 9), 3, ERR),
            expr((TEMP, 3), SET, (LIT, 4), T, ERR),
        ],
    );
    let b = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), SET, (LIT, 1), 1, ERR),
            call(2),
            expr((TEMP, 3), SET, (LIT, 4), T, ERR),
        ],
    );
    assert_equivalent(&live(&a, &b));

    let c = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), SET, (LIT, 2), 1, ERR),
            call(2),
            expr((TEMP, 3), SET, (LIT, 4), T, ERR),
        ],
    );
    // Compared, but the called tree may not read them: only a possible difference.
    assert_inconclusive(&live(&a, &c));
}

#[test]
fn global_34_is_inconclusive() {
    let reads_g34 = tree(4096, 0, 0, vec![expr((TEMP, 0), SET, (GLOBAL, 34), T, ERR)]);
    let other = tree(4096, 0, 0, vec![expr((TEMP, 0), SET, (LIT, 0), T, ERR)]);
    assert_inconclusive(&live(&reads_g34, &other));
}

// ----------------------------------------------------------------------------
// Whole-file checker
// ----------------------------------------------------------------------------

/// A version -2 (commented) string table in little-endian layout (chunk flag 0x10).
fn str_table(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut data = (-2i16).to_le_bytes().to_vec();
    data.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (text, comment) in entries {
        data.extend_from_slice(text.as_bytes());
        data.push(0);
        data.extend_from_slice(comment.as_bytes());
        data.push(0);
    }
    data
}

fn bhav_chunk(bhav: &BhavChunk, label: &str) -> ChunkData {
    ChunkData {
        tag: *b"BHAV",
        id: bhav.id,
        flags: 0x10,
        label: label.to_owned(),
        data: encode_bhav(bhav).expect("encode_bhav"),
    }
}

fn str_chunk(id: u16, entries: &[(&str, &str)]) -> ChunkData {
    ChunkData {
        tag: *b"STR#",
        id,
        flags: 0x10,
        label: String::new(),
        data: str_table(entries),
    }
}

fn fridge_files() -> (Vec<u8>, Vec<u8>) {
    let fridge = tree(4143, 0, 0, vec![expr((ATTR, 0), GT, (ATTR, 0), T, T)]);
    let high_level = air(r#"tree 4143 "CT - Is Fridge" { return true; }"#, 4143);
    let a = write_iff(&[
        bhav_chunk(&fridge, "CT - Is Fridge"),
        str_chunk(128, &[("alpha", "first"), ("beta", "")]),
    ])
    .expect("write A");
    let b = write_iff(&[
        bhav_chunk(&high_level, "CT - Is Fridge"),
        str_chunk(128, &[("alpha", "first"), ("beta", "")]),
    ])
    .expect("write B");
    (a, b)
}

#[test]
fn whole_file_mode_compares_bhav_bodies() {
    let (a, b) = fridge_files();
    // Strictly, the Fridge tree's final test leaves a different last result, which no
    // replay shows: undecided.
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
    let opts = IffEquivOptions {
        live: ignoring_last_result(),
        ..IffEquivOptions::default()
    };
    let report = check_iff_equivalent(&a, &b, &opts).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
    let bhav = report
        .resources
        .iter()
        .find(|r| &r.tag == b"BHAV")
        .expect("BHAV report");
    assert!(
        matches!(&bhav.verdict, ResourceVerdict::Equivalent { how } if how.starts_with("live-output")),
        "{bhav:?}"
    );
    let strings = report
        .resources
        .iter()
        .find(|r| &r.tag == b"STR#")
        .expect("STR# report");
    assert_eq!(strings.verdict, ResourceVerdict::Identical);
}

#[test]
fn whole_file_reports_never_show_contents() {
    let (a, _) = fridge_files();
    let fridge = tree(4143, 0, 0, vec![expr((ATTR, 0), GT, (ATTR, 0), T, T)]);
    let b = write_iff(&[
        bhav_chunk(&fridge, "CT - Is Fridge"),
        str_chunk(128, &[("alpha", "first"), ("secret text", "")]),
    ])
    .expect("write B");
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive);
    let text = format!("{report:?}");
    assert!(
        !text.contains("secret"),
        "report leaks string contents: {text}"
    );
    assert!(text.contains("STR# #128 has different contents"), "{text}");
}

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sims-antics"))
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("live-equiv-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn verify_equiv_cli_exit_codes() {
    let dir = scratch_dir("cli");
    let (a, b) = fridge_files();
    let path_a = dir.join("a.iff");
    let path_b = dir.join("b.iff");
    std::fs::write(&path_a, &a).unwrap();
    std::fs::write(&path_b, &b).unwrap();

    let out = cli()
        .arg("verify-equiv")
        .arg(&path_a)
        .arg(&path_b)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(2), "{stdout}");
    assert!(stdout.contains("last branch result"), "{stdout}");
    let out = cli()
        .args(["verify-equiv", "--ignore-last-result"])
        .arg(&path_a)
        .arg(&path_b)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("EQUIVALENT: 2 resources"), "{stdout}");
    assert!(
        stdout.contains("BHAV #4143: equivalent (live-output"),
        "{stdout}"
    );

    // A different exit is exit code 1.
    let returns_false = air(r#"tree 4143 "X" { return false; }"#, 4143);
    let path_c = dir.join("c.iff");
    std::fs::write(
        &path_c,
        write_iff(&[
            bhav_chunk(&returns_false, "CT - Is Fridge"),
            str_chunk(128, &[("alpha", "first"), ("beta", "")]),
        ])
        .unwrap(),
    )
    .unwrap();
    let out = cli()
        .args(["verify-equiv", "--ignore-last-result"])
        .arg(&path_a)
        .arg(&path_c)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("BHAV #4143: DIFFERENT"), "{stdout}");

    // `my.attr[8]` without `--my-attributes` is inconclusive (exit 2), with it equivalent.
    let attr8 = tree(4143, 0, 0, vec![expr((ATTR, 8), EQ, (ATTR, 8), T, T)]);
    let path_d = dir.join("d.iff");
    std::fs::write(&path_d, write_iff(&[bhav_chunk(&attr8, "x")]).unwrap()).unwrap();
    let path_e = dir.join("e.iff");
    let high_level = air(r#"tree 4143 "X" { return true; }"#, 4143);
    std::fs::write(&path_e, write_iff(&[bhav_chunk(&high_level, "x")]).unwrap()).unwrap();
    let out = cli()
        .arg("verify-equiv")
        .arg(&path_d)
        .arg(&path_e)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let out = cli()
        .args(["verify-equiv", "--my-attributes", "9"])
        .arg(&path_d)
        .arg(&path_e)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    // `--tree` limits the comparison; usage errors are exit code 2.
    let out = cli()
        .args(["verify-equiv", "--ignore-last-result", "--tree", "4143"])
        .arg(&path_a)
        .arg(&path_b)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    // The other resources stay in: the tree may read them.
    assert!(stdout.contains("EQUIVALENT: 2 resources"), "{stdout}");
    let out = cli().arg("verify-equiv").arg(&path_a).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

/// Frame storage is one word stack: a callee's frame starts after the caller's, so its
/// uninitialized locals come from wherever an earlier frame left words there. Trees
/// whose frames differ in size can hand their callees different locals, so a dead local
/// is only dead when nothing runs another tree.
#[test]
fn caller_frame_size_matters_when_calling_other_trees() {
    let call = prim(0x1000, [0; 8], T, F);
    let more_locals = tree(4096, 0, 4, vec![call.clone()]);
    let fewer_locals = tree(4096, 0, 0, vec![call]);
    let ctx = LiveContext {
        callee_arities: Some([(0x1000, 0)].into_iter().collect()),
        ..LiveContext::default()
    };
    assert_inconclusive(&live_with(
        &more_locals,
        &fewer_locals,
        &ctx,
        &LiveOptions::default(),
    ));
    let (_, combined) = check_bhav_equivalent_combined(
        &more_locals,
        &ctx,
        &fewer_locals,
        &ctx,
        &EquivOptions::default(),
        &LiveOptions::default(),
    );
    assert_inconclusive(&combined);

    // Without calls the frame size shows only in what the frame leaves behind.
    let no_call_more = tree(4096, 0, 4, vec![expr((TEMP, 0), SET, (LIT, 1), T, ERR)]);
    let no_call_fewer = tree(4096, 0, 0, vec![expr((TEMP, 0), SET, (LIT, 1), T, ERR)]);
    assert_inconclusive(&live(&no_call_more, &no_call_fewer));
    let ignoring = LiveOptions {
        observe_frame_residue: false,
        ..LiveOptions::default()
    };
    // A frame with no word at all still shows: Create New Object Instance writes into
    // the first word of the new object's bottom frame and fails when there is none.
    assert_inconclusive(&live_with(
        &no_call_more,
        &no_call_fewer,
        &LiveContext::default(),
        &ignoring,
    ));
    let no_call_one = tree(4096, 0, 1, vec![expr((TEMP, 0), SET, (LIT, 1), T, ERR)]);
    assert_equivalent(&live_with(
        &no_call_more,
        &no_call_one,
        &LiveContext::default(),
        &ignoring,
    ));
}

/// Branch targets 251 and 252 are invalid whatever the tree's length (native 0x44c).
#[test]
fn invalid_branch_targets_trap() {
    let mut nodes = vec![expr((LIT, 0), EQ, (LIT, 0), 251, 251)];
    nodes.extend((1..=251).map(|_| expr((LIT, 0), EQ, (LIT, 0), T, T)));
    let jumps_far = tree(4096, 0, 0, nodes);
    let returns = tree(4096, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    // A trap makes the pair undecided (the replay counts only runs that return).
    match live(&jumps_far, &returns) {
        EquivResult::Inconclusive { reason } => {
            assert!(reason.contains("0x44c"), "{reason}");
        }
        other => panic!("expected Inconclusive, got {other:?}"),
    }
    let ctx = LiveContext::default();
    let (_, combined) = check_bhav_equivalent_combined(
        &jumps_far,
        &ctx,
        &returns,
        &ctx,
        &EquivOptions::default(),
        &LiveOptions::default(),
    );
    assert_inconclusive(&combined);
}

/// A local the tree has not written may hold half of a pointer an earlier frame left,
/// and reading it can fail (`interpreter.rs:1277`), so it is not a dead read.
#[test]
fn reading_an_unwritten_local_is_not_dead() {
    let reads = tree(4096, 0, 1, vec![expr((LOCAL, 0), EQ, (LOCAL, 0), T, T)]);
    let returns = tree(4096, 0, 1, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    assert_inconclusive(&live(&reads, &returns));

    let write_first = |test: Instruction| {
        tree(
            4096,
            0,
            1,
            vec![expr((LOCAL, 0), SET, (LIT, 1), 1, ERR), test],
        )
    };
    assert_equivalent(&live(
        &write_first(expr((LOCAL, 0), EQ, (LOCAL, 0), T, T)),
        &write_first(expr((LIT, 0), EQ, (LIT, 0), T, T)),
    ));
}

/// Random succeeds or traps; it never takes its false branch.
#[test]
fn random_never_takes_its_false_branch() {
    let ops = [0, 0, 8, 0, 10, 0, 7, 0];
    let a = tree(4096, 0, 0, vec![prim(8, ops, T, F)]);
    let b = tree(4096, 0, 0, vec![prim(8, ops, T, T)]);
    assert_equivalent(&live(&a, &b));
}

/// A call whose target has different argument counts in the two files runs different
/// trees, so it cannot be matched.
#[test]
fn calls_to_trees_with_different_arities_are_inconclusive() {
    let call = tree(4096, 0, 0, vec![prim(0x1000, [0xFF; 8], T, F)]);
    let ctx = |arity| LiveContext {
        callee_arities: Some([(0x1000, arity)].into_iter().collect()),
        ..LiveContext::default()
    };
    let opts = LiveOptions::default();
    assert_inconclusive(&check_bhav_live_equivalent(
        &call,
        &ctx(1),
        &call,
        &ctx(2),
        &opts,
    ));
    let (_, combined) = check_bhav_equivalent_combined(
        &call,
        &ctx(1),
        &call,
        &ctx(2),
        &EquivOptions::default(),
        &opts,
    );
    assert_inconclusive(&combined);
}

/// Two trees reading tuning constants cannot be matched when their files' constants
/// differ: an unfolded read or a primitive would see different values.
#[test]
fn differing_constant_tables_are_inconclusive() {
    let reads_constant = tree(4096, 0, 0, vec![prim(23, [0, 0, 0, 0, 0, 0, 0, 0], T, T)]);
    let ctx = |value| LiveContext {
        local_bcons: Some([(4096, vec![value])].into_iter().collect()),
        ..LiveContext::default()
    };
    let (_, combined) = check_bhav_equivalent_combined(
        &reads_constant,
        &ctx(1),
        &reads_constant,
        &ctx(2),
        &EquivOptions::default(),
        &LiveOptions::default(),
    );
    assert_inconclusive(&combined);

    // Whole files: identical tree bytes against different constants, with `--tree`.
    let bcon = |value: i16| ChunkData {
        tag: *b"BCON",
        id: 4096,
        flags: 0x10,
        label: String::new(),
        data: [vec![1, 0], value.to_le_bytes().to_vec()].concat(),
    };
    let file = |value| write_iff(&[bhav_chunk(&reads_constant, "T"), bcon(value)]).expect("write");
    let opts = IffEquivOptions {
        tree: Some(4096),
        ..IffEquivOptions::default()
    };
    // Only BHAV bodies may differ, with `--tree` too: a changed constant table is
    // outside the precondition.
    for opts in [opts, IffEquivOptions::default()] {
        let report = check_iff_equivalent(&file(1), &file(2), &opts).expect("check");
        assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
        assert!(
            report
                .file_problems
                .iter()
                .any(|p| p.contains("BCON #4096 has different contents")),
            "{report:#?}"
        );
    }
}

/// Run Tree By Name finds trees by chunk label, so a renamed tree is outside the
/// precondition (only BHAV bodies may differ).
#[test]
fn bhav_labels_are_lookup_keys() {
    let t = tree(4096, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    let a = write_iff(&[bhav_chunk(&t, "Old")]).expect("write");
    let b = write_iff(&[bhav_chunk(&t, "New")]).expect("write");
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
    assert!(format!("{report:?}").contains("label"));
}

/// Every chunk that is not a `BHAV` must be byte-identical, in the same place, with the
/// same label and flags: string tables that load the same (a comment-only change, a
/// different layout) are still outside the precondition.
#[test]
fn non_bhav_chunks_must_be_identical() {
    let t = tree(4096, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    let check = |a: &[ChunkData], b: &[ChunkData]| {
        let report = check_iff_equivalent(
            &write_iff(a).expect("write"),
            &write_iff(b).expect("write"),
            &IffEquivOptions::default(),
        )
        .expect("check");
        (report.verdict(), report.file_problems.join("; "))
    };
    let strings = str_chunk(128, &[("fo", "")]);
    let commented = str_chunk(128, &[("fo", "note")]);
    let mac = ChunkData {
        data: vec![1, 0, 2, b'f', b'o'],
        ..strings.clone()
    };
    let base = [bhav_chunk(&t, "T"), strings.clone()];
    assert_eq!(check(&base, &base).0, IffVerdict::Equivalent);
    for (other, expect) in [
        (
            vec![bhav_chunk(&t, "T"), commented],
            "STR# #128 has different contents",
        ),
        (
            vec![bhav_chunk(&t, "T"), mac],
            "STR# #128 has different contents",
        ),
        (
            vec![strings.clone(), bhav_chunk(&t, "T")],
            "chunk 0 is BHAV #4096 in A but STR# #128 in B",
        ),
        (
            vec![
                bhav_chunk(&t, "T"),
                ChunkData {
                    label: "renamed".to_owned(),
                    ..strings.clone()
                },
            ],
            "STR# #128 has different labels",
        ),
        (
            vec![
                bhav_chunk(&t, "T"),
                ChunkData {
                    flags: 0,
                    ..strings.clone()
                },
            ],
            "STR# #128 has different header flags",
        ),
        (
            vec![bhav_chunk(&t, "T"), strings.clone(), str_chunk(129, &[])],
            "different chunk layouts",
        ),
    ] {
        let (verdict, problems) = check(&base, &other);
        assert_eq!(verdict, IffVerdict::Inconclusive, "{problems}");
        assert!(problems.contains(expect), "{problems}");
    }
}

/// An unusable resource map means the compared chunks may not be the ones the game
/// loads, so even a difference is only inconclusive.
#[test]
fn file_problems_take_precedence() {
    let report = IffEquivReport {
        resources: vec![ResourceReport {
            tag: *b"BHAV",
            id: 4096,
            verdict: ResourceVerdict::Different {
                why: "x".to_owned(),
            },
            notes: Vec::new(),
        }],
        file_problems: vec!["map".to_owned()],
        assumptions: Vec::new(),
        equivalent_modulo: None,
    };
    assert_eq!(report.verdict(), IffVerdict::Inconclusive);
}

/// Counterexample text stays short when terms share subterms (doubling 40 times).
#[test]
fn deep_terms_are_described_briefly() {
    let doubling = |times: usize| {
        let mut nodes = vec![expr((TEMP, 0), SET, (PARAM, 0), 1, ERR)];
        for i in 0..times {
            nodes.push(expr((TEMP, 0), ADD, (TEMP, 0), (i + 2) as u8, ERR));
        }
        let last = nodes.len() - 1;
        nodes[last].on_true = T;
        tree(4096, 1, 0, nodes)
    };
    let result = live(&doubling(40), &doubling(41));
    let text = format!("{result:?}");
    assert!(text.len() < 4000, "{} bytes", text.len());
}

// ---------------------------------------------------------------------------
// Review round 2 regressions
// ---------------------------------------------------------------------------

/// `local[0] == local[0]` reads a local the tree never wrote, which can fail in the VM
/// (`interpreter.rs:1277`); the combined checker must not take lockstep's folded proof.
#[test]
fn combined_checker_keeps_unwritten_local_reads() {
    let reads = tree(4096, 0, 1, vec![expr((LOCAL, 0), EQ, (LOCAL, 0), T, T)]);
    let literal = tree(4096, 0, 1, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    let ctx = LiveContext::default();
    let (method, result) = check_bhav_equivalent_combined(
        &reads,
        &ctx,
        &literal,
        &ctx,
        &EquivOptions::default(),
        &LiveOptions::default(),
    );
    assert_eq!(method, EquivMethod::LiveOutput);
    assert_inconclusive(&result);

    // A local written before it is read keeps the lockstep proof.
    let written = tree(
        4096,
        0,
        1,
        vec![
            expr((LOCAL, 0), SET, (LIT, 3), 1, ERR),
            expr((TEMP, 0), SET, (LOCAL, 0), T, ERR),
        ],
    );
    let (method, result) = check_bhav_equivalent_combined(
        &written,
        &ctx,
        &written,
        &ctx,
        &EquivOptions::default(),
        &LiveOptions::default(),
    );
    assert_eq!(method, EquivMethod::Lockstep);
    assert_equivalent(&result);
}

/// The live checker alone also refuses trees whose constant tables differ.
#[test]
fn live_checker_checks_constant_tables() {
    // Random Number with its range in BCON 4096 entry 0 (scope 26).
    let random = tree(
        4096,
        0,
        0,
        vec![prim(8, [0, 0, TEMP, 0, 0, 0, 26, 0], T, T)],
    );
    let ctx = |value| LiveContext {
        local_bcons: Some([(4096, vec![value])].into_iter().collect()),
        ..LiveContext::default()
    };
    assert_inconclusive(&check_bhav_live_equivalent(
        &random,
        &ctx(0),
        &random,
        &ctx(1),
        &LiveOptions::default(),
    ));
}

/// Writes `chunks` with a version-0 resource map listing `(chunk index)` for each
/// `(tag, id)` in `map`, appended as the last chunk.
fn write_with_map(chunks: &[ChunkData], map: &[usize]) -> Vec<u8> {
    let mut offsets = Vec::new();
    let mut at = 64usize;
    for c in chunks {
        offsets.push(at);
        at += 76 + c.data.len();
    }
    let mut by_tag: std::collections::BTreeMap<[u8; 4], Vec<usize>> = Default::default();
    for &i in map {
        by_tag.entry(chunks[i].tag).or_default().push(i);
    }
    let mut data = vec![0u8; 8];
    data.extend(b"pmsr");
    data.extend(0u32.to_le_bytes());
    data.extend((by_tag.len() as u32).to_le_bytes());
    for (tag, list) in &by_tag {
        data.extend(u32::from_be_bytes(*tag).to_le_bytes());
        data.extend((list.len() as u32).to_le_bytes());
        for &i in list {
            data.extend((offsets[i] as u32).to_le_bytes());
            data.extend(chunks[i].id.to_le_bytes());
            data.extend(chunks[i].flags.to_le_bytes());
            data.push(0);
            if !data.len().is_multiple_of(2) {
                data.push(0);
            }
        }
    }
    let mut all = chunks.to_vec();
    all.push(ChunkData {
        tag: *b"rsmp",
        id: 0,
        flags: 0x10,
        label: String::new(),
        data,
    });
    let mut bytes = write_iff(&all).expect("write");
    bytes[60..64].copy_from_slice(&(at as u32).to_be_bytes());
    bytes
}

/// A resource map that picks the first of two same-id chunks, while `sims-vm` keeps
/// the last (`program.rs:601`): undecided.
#[test]
fn resource_map_and_file_order_disagree() {
    let t = tree(4096, 0, 0, vec![prim(23, [0; 8], T, T)]);
    let bcon = |value: i16| ChunkData {
        tag: *b"BCON",
        id: 4096,
        flags: 0x10,
        label: String::new(),
        data: [vec![1, 0], value.to_le_bytes().to_vec()].concat(),
    };
    let a = write_with_map(&[bhav_chunk(&t, "T"), bcon(1), bcon(1)], &[0, 1]);
    let b = write_with_map(&[bhav_chunk(&t, "T"), bcon(1), bcon(2)], &[0, 1]);
    let parsed = sims_formats::iff::Iff::parse(b.clone()).expect("parse");
    assert!(parsed.map_error().is_none());
    assert_eq!(
        parsed.data(parsed.get(b"BCON", 4096).expect("mapped")),
        bcon(1).data.as_slice(),
        "the map selects the first copy"
    );
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
}

/// `sims-vm` parses every copy of a chunk, so a malformed copy before the one kept can
/// stop the load: duplicates that differ between the files are undecided.
#[test]
fn malformed_duplicates_are_inconclusive() {
    let t = tree(4096, 0, 0, vec![prim(23, [0; 8], T, T)]);
    let bcon = |data: Vec<u8>| ChunkData {
        tag: *b"BCON",
        id: 4096,
        flags: 0x10,
        label: String::new(),
        data,
    };
    let valid = vec![1, 0, 1, 0];
    let a = write_iff(&[bhav_chunk(&t, "T"), bcon(vec![9]), bcon(valid.clone())]).expect("w");
    let b = write_iff(&[bhav_chunk(&t, "T"), bcon(valid.clone())]).expect("w");
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
    // Even the same copies in both files: a file with two chunks of one type and ID is
    // outside the precondition.
    let report = check_iff_equivalent(&a, &a, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
    assert!(
        report
            .file_problems
            .iter()
            .any(|p| p.contains("more than one chunk of the same type and ID")),
        "{report:#?}"
    );
}

/// `sims-vm` reads a version-0 `STR#` #303 for Run Tree By Name, which the original
/// loader reads as empty: the two readings disagree, so the result is undecided.
#[test]
fn tree_name_tables_follow_both_readers() {
    let names = |name: &[u8]| ChunkData {
        tag: *b"STR#",
        id: 303,
        flags: 0x10,
        label: String::new(),
        data: [vec![0, 0, 1, 0, name.len() as u8], name.to_vec()].concat(),
    };
    let a = write_iff(&[names(b"Alpha")]).expect("write");
    let b = write_iff(&[names(b"Beta")]).expect("write");
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
    // So is any other table: primitives read them too (Change Suit, `change_suit.rs:137`).
    let other = |name: &[u8]| ChunkData {
        id: 304,
        ..names(name)
    };
    let a = write_iff(&[other(b"Alpha")]).expect("write");
    let b = write_iff(&[other(b"Beta")]).expect("write");
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");

    // The other direction: a localized table that changes only a non-English entry
    // differs for the original's loader but not for `sims-vm`, which keeps language 1.
    let localized = |other: &[u8]| ChunkData {
        tag: *b"STR#",
        id: 303,
        flags: 0x10,
        label: String::new(),
        data: [
            vec![0xFD, 0xFF, 2, 0, 1, b'a', 0, 0, 2],
            other.to_vec(),
            vec![0, 0],
        ]
        .concat(),
    };
    let a = write_iff(&[localized(b"x")]).expect("write");
    let b = write_iff(&[localized(b"y")]).expect("write");
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
}

/// Identical bytes load the same way in both files, even when they do not parse as a
/// string table: only the trees are compared.
#[test]
fn identical_malformed_string_tables_are_identical() {
    let bad = ChunkData {
        tag: *b"STR#",
        id: 128,
        flags: 0x10,
        label: String::new(),
        data: vec![0xFF],
    };
    let a = write_iff(&[bad]).expect("write");
    let report = check_iff_equivalent(&a, &a, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
}

/// A primitive both trees run may never succeed (a `Random` with a zero range always
/// fails), so different exits after it are not a proven difference.
#[test]
fn differences_after_a_shared_primitive_are_undecided() {
    let random = |exit| {
        tree(
            4096,
            0,
            0,
            vec![prim(8, [0, 0, TEMP, 0, 0, 0, LIT, 0], exit, exit)],
        )
    };
    assert_inconclusive(&live(&random(T), &random(F)));
}

/// Calls to trees with different argument counts that read their arguments inline
/// (more than four) are undecided, not different.
#[test]
fn inline_calls_with_different_arities_are_inconclusive() {
    let filler = expr((LIT, 0), EQ, (LIT, 0), T, T);
    let caller = tree(
        4096,
        0,
        0,
        // Not all-0xFF, so the arguments past the fourth are read inline from the
        // instructions that follow (10 or 12 bytes).
        vec![prim(0x1000, [0; 8], T, T), filler.clone(), filler.clone()],
    );
    let ctx = |arity| LiveContext {
        callee_arities: Some([(0x1000, arity)].into_iter().collect()),
        ..LiveContext::default()
    };
    assert_inconclusive(&check_bhav_live_equivalent(
        &caller,
        &ctx(5),
        &caller,
        &ctx(6),
        &LiveOptions::default(),
    ));
}

/// Kind-4 trees are entered at the clamped node, and a shorter tree's last node answers
/// the longer one's extra entries. An entry past node 250 faults at the clamped node, so
/// trees of different lengths past it fault at different nodes (undecided).
#[test]
fn kind4_entries_follow_the_clamp() {
    let node = |exit| expr((LIT, 0), EQ, (LIT, 0), exit, exit);
    let indexed = |nodes: Vec<Instruction>| BhavChunk {
        kind: 4,
        ..tree(4096, 0, 0, nodes)
    };
    let long_a = indexed(vec![node(T); 252]);
    let long_b = indexed(vec![node(T); 253]);
    assert_inconclusive(&live(&long_a, &long_b));
    let long_c = indexed([vec![node(T); 251], vec![node(F)]].concat());
    assert_equivalent(&live(&long_a, &long_c));

    let short = indexed(vec![node(T), node(T), node(F)]);
    let same = indexed(vec![node(T), node(T), node(F), node(F), node(F)]);
    assert_equivalent(&live(&short, &same));
    let differs = indexed(vec![node(T), node(T), node(F), node(T), node(F)]);
    match live(&short, &differs) {
        EquivResult::Counterexample { divergence, .. } => {
            assert!(divergence.contains("params [3]"), "{divergence}");
        }
        other => panic!("expected Counterexample, got {other:?}"),
    }
}

/// Scope 35 reads only ever give 0 or 1 (`memory.rs:542`), so a test against 2 that
/// would separate the trees has no witness.
#[test]
fn opaque_reads_are_not_free_inputs() {
    let a = tree(4096, 0, 0, vec![expr((35, 0), EQ, (LIT, 2), T, F)]);
    let b = tree(4096, 0, 0, vec![expr((35, 0), EQ, (LIT, 3), F, F)]);
    assert_inconclusive(&live(&a, &b));
}

/// A kind-4 entry is also `param[0]` (`interpreter.rs:964`): the second node of a
/// two-node tree runs only for entries of 1 and up.
#[test]
fn kind4_entry_is_the_first_parameter() {
    let indexed = |nodes: Vec<Instruction>| BhavChunk {
        kind: 4,
        ..tree(4096, 1, 0, nodes)
    };
    let one = indexed(vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    let two = indexed(vec![
        expr((LIT, 0), EQ, (LIT, 0), T, T),
        expr((PARAM, 0), 14, (LIT, 1), T, F),
    ]);
    assert_equivalent(&live(&one, &two));
    let three = indexed(vec![
        expr((LIT, 0), EQ, (LIT, 0), T, T),
        expr((PARAM, 0), 14, (LIT, 2), T, F),
    ]);
    assert_counterexample(&live(&one, &three));
}

/// Calls to the same tree with different operands and argument counts: the argument
/// count decides first, and only possibly.
#[test]
fn call_arity_is_checked_before_operands() {
    let filler = expr((LIT, 0), EQ, (LIT, 0), T, T);
    let caller = |ops: [u8; 8]| {
        tree(
            4096,
            0,
            0,
            vec![prim(0x1000, ops, T, T), filler.clone(), filler.clone()],
        )
    };
    let ctx = |arity| LiveContext {
        callee_arities: Some([(0x1000, arity)].into_iter().collect()),
        ..LiveContext::default()
    };
    match check_bhav_live_equivalent(
        &caller([0; 8]),
        &ctx(5),
        &caller([1, 0, 0, 0, 0, 0, 0, 0]),
        &ctx(6),
        &LiveOptions::default(),
    ) {
        EquivResult::Inconclusive { reason } => {
            assert!(reason.contains("argument counts"), "{reason}");
        }
        other => panic!("expected Inconclusive, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Narrowed contract: DIFFERENT from a VM replay, faults undecided, strict files
// ---------------------------------------------------------------------------

/// A counterexample carries the concrete run that shows it; a symbolic difference the
/// replay cannot run (a primitive, a different kind) is undecided.
#[test]
fn counterexamples_come_from_a_vm_replay() {
    let copy = |src: i16| tree(4096, 2, 0, vec![expr((TEMP, 0), SET, (PARAM, src), T, ERR)]);
    match live(&copy(0), &copy(1)) {
        EquivResult::Counterexample { divergence, .. } => {
            assert!(
                divergence.contains("sims-vm runs both from params")
                    && divergence.contains("temp[0] ends as"),
                "{divergence}"
            );
        }
        other => panic!("expected Counterexample, got {other:?}"),
    }
    // The same definite difference, but the trees also hold a primitive (behind a
    // branch no input takes) that the replay world does not run.
    let then_sound = |src: i16| {
        tree(
            4096,
            2,
            0,
            vec![
                expr((TEMP, 0), SET, (PARAM, src), 1, ERR),
                expr((LIT, 0), EQ, (LIT, 0), T, 2),
                prim(23, [1, 0, 0, 0, 0, 0, 0, 0], T, T),
            ],
        )
    };
    match live(&then_sound(0), &then_sound(1)) {
        EquivResult::Inconclusive { reason } => {
            assert!(
                reason.contains("no sims-vm replay confirmed it"),
                "{reason}"
            );
        }
        other => panic!("expected Inconclusive, got {other:?}"),
    }
    // Kinds 0 and 1 both enter at node 0 (`interpreter.rs:1792`): not a difference.
    let returns = tree(4096, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    let kind1 = BhavChunk {
        kind: 1,
        ..returns.clone()
    };
    assert_inconclusive(&live(&returns, &kind1));
}

/// A fault leaves the faulting frame and the last result behind (`interpreter.rs:809`):
/// two faults are the same only at the same node with the same state.
#[test]
fn faults_keep_their_state() {
    // Different last results before the same division by zero.
    let before = |test: Instruction| {
        tree(
            4096,
            0,
            0,
            vec![test, expr((TEMP, 0), DIV, (LIT, 0), T, ERR)],
        )
    };
    let true_test = before(expr((LIT, 0), EQ, (LIT, 0), 1, 1));
    let false_test = before(expr((LIT, 0), EQ, (LIT, 1), 1, 1));
    assert_inconclusive(&live(&true_test, &false_test));
    let ctx = LiveContext::default();
    // The fault keeps the last result even when the caller does not observe it at exit
    // (`--ignore-last-result`).
    for opts in [LiveOptions::default(), ignoring_last_result()] {
        match live_with(&true_test, &false_test, &ctx, &opts) {
            EquivResult::Inconclusive { reason } => {
                assert!(reason.contains("from different states"), "{reason}");
            }
            other => panic!("expected Inconclusive, got {other:?}"),
        }
        let (_, combined) = check_bhav_equivalent_combined(
            &true_test,
            &ctx,
            &false_test,
            &ctx,
            &EquivOptions::default(),
            &opts,
        );
        assert_inconclusive(&combined);
    }

    // A square root of -1 into an unwritten local or into a temp: both fault at node 0
    // with the same state before it, but the VM clears the local's unknown marker
    // before it faults (`expression.rs:69`), so the instructions must match too.
    let sqrt_into = |scope: u8| tree(4096, 0, 1, vec![expr((scope, 0), 20, (LIT, -1), T, ERR)]);
    match live(&sqrt_into(LOCAL), &sqrt_into(TEMP)) {
        EquivResult::Inconclusive { reason } => {
            assert!(
                reason.contains("from different states or instructions"),
                "{reason}"
            );
        }
        other => panic!("expected Inconclusive, got {other:?}"),
    }
    assert_equivalent(&live(&sqrt_into(LOCAL), &sqrt_into(LOCAL)));

    // The same fault after a dummy node is at a different node, even with nothing else
    // observed.
    let relaxed = LiveOptions {
        observe_last_result: false,
        observe_frame_residue: false,
        observe_scratch_pointer: false,
        ..LiveOptions::default()
    };
    let direct = tree(4096, 0, 0, vec![expr((TEMP, 0), DIV, (LIT, 0), T, ERR)]);
    let after_dummy = tree(
        4096,
        0,
        0,
        vec![
            expr((LIT, 0), EQ, (LIT, 0), 1, 1),
            expr((TEMP, 0), DIV, (LIT, 0), T, ERR),
        ],
    );
    match live_with(&direct, &after_dummy, &ctx, &relaxed) {
        EquivResult::Inconclusive { reason } => {
            assert!(reason.contains("at different nodes"), "{reason}");
        }
        other => panic!("expected Inconclusive, got {other:?}"),
    }
}

/// Lockstep matches renumbered nodes, but a fault there would leave the frame at a
/// different node. Both trees run the same test at node 0; its targets lead to their
/// own divisions, which B numbers the other way round. Every path computes the same,
/// so only the node a division faults at tells them apart.
#[test]
fn renumbered_faulting_nodes_are_undecided() {
    let test = |t, f| expr((TEMP, 0), EQ, (LIT, 0), t, f);
    let first_divide = expr((TEMP, 1), DIV, (TEMP, 2), T, ERR);
    let second_divide = expr((TEMP, 3), DIV, (TEMP, 4), T, ERR);
    let a = tree(
        4096,
        0,
        0,
        vec![test(1, 2), first_divide.clone(), second_divide.clone()],
    );
    let b = tree(
        4096,
        0,
        0,
        vec![test(2, 1), second_divide.clone(), first_divide.clone()],
    );
    let ctx = LiveContext::default();
    let (_, result) = check_bhav_equivalent_combined(
        &a,
        &ctx,
        &b,
        &ctx,
        &EquivOptions::default(),
        &LiveOptions::default(),
    );
    match result {
        EquivResult::Inconclusive { reason } => {
            assert!(reason.contains("at different nodes"), "{reason}");
        }
        other => panic!("expected Inconclusive, got {other:?}"),
    }
    // Lockstep alone pairs the renumbered nodes.
    assert_equivalent(&check_bhav_equivalent(&a, &b, &EquivOptions::default()));
    // The same trees with the same numbering are proven node for node.
    let (method, result) = check_bhav_equivalent_combined(
        &a,
        &ctx,
        &a.clone(),
        &ctx,
        &EquivOptions::default(),
        &LiveOptions::default(),
    );
    assert_eq!(method, EquivMethod::Lockstep);
    assert_equivalent(&result);
}

/// Equal outputs from different instruction counts are equivalent under the stated
/// assumption that neither run exhausts the VM's instruction budget.
#[test]
fn instruction_budget_is_a_model_assumption() {
    let twice = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), SET, (LIT, 5), 1, ERR),
            expr((TEMP, 0), SET, (LIT, 7), T, ERR),
        ],
    );
    let once = tree(4096, 0, 0, vec![expr((TEMP, 0), SET, (LIT, 7), T, ERR)]);
    assert_equivalent(&live(&twice, &once));
}

/// A resource map that selects the same chunks compares equal even when a differing
/// BHAV body moves the offsets; a map in one file only is outside the precondition.
#[test]
fn resource_maps_compare_by_the_chunks_they_select() {
    let short = tree(4096, 0, 0, vec![expr((TEMP, 0), SET, (LIT, 7), T, ERR)]);
    let long = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), SET, (LIT, 5), 1, ERR),
            expr((TEMP, 0), SET, (LIT, 7), T, ERR),
        ],
    );
    let strings = str_chunk(128, &[("a", "")]);
    let a = write_with_map(&[bhav_chunk(&short, "T"), strings.clone()], &[0, 1]);
    let b = write_with_map(&[bhav_chunk(&long, "T"), strings.clone()], &[0, 1]);
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");

    // Keep the map in B and change only its contents: its unused word, then the pad
    // byte after an entry's label, which the map parser skips but is still a byte of
    // the file.
    let map_at = u32::from_be_bytes(b[60..64].try_into().unwrap()) as usize + 76;
    for at in [12, 37] {
        let mut changed = b.clone();
        changed[map_at + at] ^= 0x5A;
        let report =
            check_iff_equivalent(&a, &changed, &IffEquivOptions::default()).expect("check");
        assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
        assert!(
            report.file_problems[0].contains("rsmp #0 (the resource map) differs"),
            "{report:#?}"
        );
    }

    let unmapped = write_iff(&[bhav_chunk(&long, "T"), strings]).expect("write");
    let report = check_iff_equivalent(&a, &unmapped, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
}

/// The file header and every chunk header are compared byte for byte: a resource map
/// that only one file's header points at (bytes 60..64 zeroed in B, so B is read in
/// physical order), and label bytes after the terminator.
#[test]
fn file_and_chunk_headers_compare_byte_for_byte() {
    let short = tree(4096, 0, 0, vec![expr((TEMP, 0), SET, (LIT, 7), T, ERR)]);
    let long = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), SET, (LIT, 5), 1, ERR),
            expr((TEMP, 0), SET, (LIT, 7), T, ERR),
        ],
    );
    // Two string tables, which the map lists in reverse physical order.
    let chunks = |bhav: &BhavChunk| {
        [
            bhav_chunk(bhav, "T"),
            str_chunk(128, &[("a", "")]),
            str_chunk(129, &[("b", "")]),
        ]
    };
    let a = write_with_map(&chunks(&short), &[0, 2, 1]);
    let b = write_with_map(&chunks(&long), &[0, 2, 1]);
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
    let mut unmapped = b.clone();
    unmapped[60..64].fill(0);
    let report = check_iff_equivalent(&a, &unmapped, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
    assert!(
        report.file_problems[0].contains("the file header"),
        "{report:#?}"
    );

    // A byte after the NUL that ends a string table's (empty) label.
    let plain = |bhav: &BhavChunk| {
        write_iff(&[bhav_chunk(bhav, "T"), str_chunk(128, &[("a", "")])]).expect("write")
    };
    let (a, mut b) = (plain(&short), plain(&long));
    let strings_at = 64 + 76 + encode_bhav(&long).unwrap().len();
    assert_eq!(&b[strings_at..strings_at + 4], b"STR#");
    b[strings_at + 12 + 5] = b'x';
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
    assert!(
        report.file_problems[0].contains("STR# #128 has different labels"),
        "{report:#?}"
    );
}

/// `equiv` refuses an IFF whose target tree is not the only chunk of its type and ID.
#[test]
fn equiv_requires_a_single_copy() {
    let dir = scratch_dir("equiv-dup");
    let source = dir.join("a.antics");
    std::fs::write(&source, "tree 4096 \"X\" { return true; }\n").unwrap();
    let t = tree(4096, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    let f = tree(4096, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 0), F, F)]);
    let iff = dir.join("b.iff");
    std::fs::write(
        &iff,
        write_iff(&[bhav_chunk(&t, "X"), bhav_chunk(&f, "X")]).unwrap(),
    )
    .unwrap();
    let out = cli()
        .arg("equiv")
        .arg(&source)
        .arg(&iff)
        .args(["--tree", "4096"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    assert!(
        stderr.contains("INCONCLUSIVE") && stderr.contains("more than one chunk"),
        "{stderr}"
    );
}

/// Any primitive may fault (Random with a zero range raises native error 4,
/// `random.rs`), and the fault leaves the frame at the primitive's node: the same
/// Random at node 1 in A and node 2 in B is undecided.
#[test]
fn primitives_may_fault_at_their_node() {
    let random = prim(8, [0, 0, 8, 0, 0, 0, 7, 0], T, T);
    let filler = |next| expr((LIT, 0), EQ, (LIT, 0), next, next);
    let a = tree(4096, 0, 0, vec![filler(1), random.clone()]);
    let b = tree(4096, 0, 0, vec![filler(1), filler(2), random.clone()]);
    let ctx = LiveContext::default();
    let relaxed = LiveOptions {
        observe_last_result: false,
        observe_frame_residue: false,
        observe_scratch_pointer: false,
        ..LiveOptions::default()
    };
    for opts in [LiveOptions::default(), relaxed] {
        match live_with(&a, &b, &ctx, &opts) {
            EquivResult::Inconclusive { reason } => {
                assert!(reason.contains("node"), "{reason}");
            }
            other => panic!("expected Inconclusive, got {other:?}"),
        }
        let (_, combined) =
            check_bhav_equivalent_combined(&a, &ctx, &b, &ctx, &EquivOptions::default(), &opts);
        assert_inconclusive(&combined);
    }
    // At the same node but after different last results, with nothing but the fault
    // observing them.
    let after = |test: Instruction| tree(4096, 0, 0, vec![test, random.clone()]);
    match live_with(
        &after(expr((LIT, 0), EQ, (LIT, 0), 1, 1)),
        &after(expr((LIT, 0), EQ, (LIT, 1), 1, 1)),
        &ctx,
        &relaxed,
    ) {
        EquivResult::Inconclusive { reason } => {
            assert!(reason.contains("from different states"), "{reason}");
        }
        other => panic!("expected Inconclusive, got {other:?}"),
    }
    // At the same node, from the same state, the two faults are the same.
    let c = tree(
        4096,
        0,
        0,
        vec![
            filler(1),
            Instruction {
                on_false: F,
                ..random
            },
        ],
    );
    assert_equivalent(&live(&a, &c));
}

/// A counterexample from locals and temps: the replay must run from the frame and the
/// temps the witness names (the frame sits after `run_tree`'s call marker).
#[test]
fn replays_seed_locals_and_temps() {
    let set_if = |lhs: (u8, i16), v: i16| {
        tree(
            4096,
            0,
            1,
            vec![
                expr(lhs, EQ, (LIT, v), 1, T),
                expr((TEMP, 0), SET, (LIT, 1), T, ERR),
            ],
        )
    };
    for lhs in [(LOCAL, 0), (TEMP, 1)] {
        match live(&set_if(lhs, 1), &set_if(lhs, 2)) {
            EquivResult::Counterexample { divergence, .. } => {
                assert!(divergence.contains("temp[0] ends as"), "{divergence}");
            }
            other => panic!("expected Counterexample for {lhs:?}, got {other:?}"),
        }
    }
}

/// The replay compares the words it defines, field by field. Writing a zero
/// advertisement delta turns the world's "no advertisements copied yet" into sixteen
/// zeroed ones: a different representation of what the VM reads back as the same
/// zeros, so it is no counterexample.
#[test]
fn replay_ignores_representation_changes() {
    let write_delta = tree(
        4096,
        0,
        0,
        vec![
            expr((12, 0), SET, (LIT, 0), 1, ERR),
            expr((LIT, 0), EQ, (LIT, 0), T, T),
        ],
    );
    let nothing = tree(4096, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    match live(&write_delta, &nothing) {
        EquivResult::Inconclusive { reason } => {
            assert!(
                reason.contains("no sims-vm replay confirmed it"),
                "{reason}"
            );
        }
        other => panic!("expected Inconclusive, got {other:?}"),
    }
}

/// `equiv` trusts a call only as far as the callee: the same tree calling a 4097 that
/// returns true in the source but false in the IFF is undecided, while a callee that
/// is the same bytes, or proven equivalent, keeps the proof.
#[test]
fn equiv_checks_the_trees_a_tree_calls() {
    let dir = scratch_dir("equiv-callee");
    let source = dir.join("a.antics");
    let main = r#"tree 4096 "Main" { if call 4097(temps) { return true; } return false; }"#;
    std::fs::write(
        &source,
        format!("{main}\ntree 4097 \"Callee\" {{ return true; }}\n"),
    )
    .unwrap();
    let root = air(main, 4096);
    let callee = |instructions| tree(4097, 0, 0, instructions);
    let run = |callee: &BhavChunk| {
        let iff = dir.join("b.iff");
        std::fs::write(
            &iff,
            write_iff(&[bhav_chunk(&root, "Main"), bhav_chunk(callee, "Callee")]).unwrap(),
        )
        .unwrap();
        let out = cli()
            .arg("equiv")
            .arg(&source)
            .arg(&iff)
            .args(["--tree", "4096"])
            .output()
            .unwrap();
        (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    };
    let returns = |target| expr((LIT, 0), EQ, (LIT, 0), target, target);
    let (ok, text) = run(&callee(vec![returns(F)]));
    assert!(
        !ok && text.contains("INCONCLUSIVE") && text.contains("tree #4097"),
        "{text}"
    );
    // The same callee, and one that returns true after a dummy node.
    let same = air(r#"tree 4097 "Callee" { return true; }"#, 4097);
    let (ok, text) = run(&same);
    assert!(ok && text.contains("EQUIVALENT"), "{text}");
    let dummy = callee(vec![expr((ATTR, 0), GT, (ATTR, 0), 1, 1), returns(T)]);
    let (ok, text) = run(&dummy);
    assert!(ok && text.contains("EQUIVALENT"), "{text}");
}

/// The return path (`interpreter.rs:1640`, `pop_frame`, `release_use`): a tree a
/// primitive runs as an object use releases it when its frame pops, may end the
/// running action with its returning node's result, and can fault once its frame is
/// written to the frame storage. So `--ignore-last-result` and
/// `--ignore-frame-residue` relax only the checked tree (run by `run_tree`, which
/// records no use); every tree in its reach is proven with both observed.
#[test]
fn reach_trees_observe_their_returns() {
    let root = air(
        r#"tree 4096 "Main" { if call 4097(temps) { return true; } return false; }"#,
        4096,
    );
    let file = |callee: BhavChunk| {
        write_iff(&[bhav_chunk(&root, "Main"), bhav_chunk(&callee, "Callee")]).unwrap()
    };
    let verdict = |report: &IffEquivReport, id: u16| {
        report
            .resources
            .iter()
            .find(|r| r.id == id)
            .unwrap()
            .verdict
            .clone()
    };
    // Callees that return true with different last results, and callees that differ
    // only in a dead local write.
    let cases = [
        (
            tree(4097, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]),
            tree(4097, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 1), T, T)]),
            LiveOptions {
                observe_last_result: false,
                ..LiveOptions::default()
            },
        ),
        (
            tree(4097, 0, 1, vec![expr((LOCAL, 0), SET, (LIT, 3), T, T)]),
            tree(4097, 0, 1, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]),
            LiveOptions {
                observe_frame_residue: false,
                ..LiveOptions::default()
            },
        ),
    ];
    for (callee_a, callee_b, live) in cases {
        let (a, b) = (file(callee_a), file(callee_b));
        reach_verdicts(&a, &b, live, 4096, 4097);
    }
    // Review P2-3: callee 4097 leaves local 0 as 1 or 2, and 4098, the same on both
    // sides, reads it before writing it.
    let calls = tree(
        4096,
        0,
        0,
        vec![prim(4097, [0; 8], 1, 1), prim(4098, [0; 8], T, T)],
    );
    let leaves = |value| tree(4097, 0, 1, vec![expr((LOCAL, 0), SET, (LIT, value), T, T)]);
    let reads = tree(4098, 0, 1, vec![expr((TEMP, 0), SET, (LOCAL, 0), T, T)]);
    let two_calls = |callee: BhavChunk| {
        write_iff(&[
            bhav_chunk(&calls, "Main"),
            bhav_chunk(&callee, "Leaves"),
            bhav_chunk(&reads, "Reads"),
        ])
        .unwrap()
    };
    reach_verdicts(
        &two_calls(leaves(1)),
        &two_calls(leaves(2)),
        LiveOptions {
            observe_frame_residue: false,
            ..LiveOptions::default()
        },
        4096,
        4097,
    );
    // Review P2-2: a tree with a primitive may be run by it as an object use, so its
    // own last result counts even with `--ignore-last-result`.
    let own = |test: Instruction| {
        write_iff(&[bhav_chunk(
            &tree(4096, 0, 0, vec![prim(3, [0; 8], 1, 1), test]),
            "Main",
        )])
        .unwrap()
    };
    let (a, b) = (
        own(expr((LIT, 0), EQ, (LIT, 0), T, T)),
        own(expr((LIT, 0), EQ, (LIT, 1), T, T)),
    );
    let relaxed = LiveOptions {
        observe_last_result: false,
        ..LiveOptions::default()
    };
    for tree in [None, Some(4096)] {
        let opts = IffEquivOptions {
            tree,
            live: relaxed,
            ..IffEquivOptions::default()
        };
        let report = check_iff_equivalent(&a, &b, &opts).expect("check");
        match verdict(&report, 4096) {
            ResourceVerdict::Inconclusive { why } => {
                assert!(why.contains("object use"), "{why}");
            }
            other => panic!("expected Inconclusive, got {other:?}"),
        }
    }
    // The same through `check_tree_with_callees` (`equiv`'s check), with two files.
    let trees = |test: Instruction| {
        std::collections::BTreeMap::from([(
            4096,
            tree(4096, 0, 0, vec![prim(3, [0; 8], 1, 1), test]),
        )])
    };
    let (ta, tb) = (
        trees(expr((LIT, 0), EQ, (LIT, 0), T, T)),
        trees(expr((LIT, 0), EQ, (LIT, 1), T, T)),
    );
    let arities = std::collections::BTreeMap::new();
    let (_, result) = sims_antics::verify::check_tree_with_callees(
        4096,
        &ta,
        Some(&a),
        &tb,
        Some(&b),
        (&arities, &arities),
        &EquivOptions::default(),
        &relaxed,
    );
    match result {
        EquivResult::Inconclusive { reason } => assert!(reason.contains("object use"), "{reason}"),
        other => panic!("expected Inconclusive, got {other:?}"),
    }
    // Without the primitive, the flag holds.
    let plain = |test: Instruction| {
        write_iff(&[bhav_chunk(&tree(4096, 0, 0, vec![test]), "Main")]).unwrap()
    };
    let report = check_iff_equivalent(
        &plain(expr((LIT, 0), EQ, (LIT, 0), T, T)),
        &plain(expr((LIT, 0), EQ, (LIT, 1), T, T)),
        &IffEquivOptions {
            live: relaxed,
            ..IffEquivOptions::default()
        },
    )
    .expect("check");
    assert!(
        matches!(verdict(&report, 4096), ResourceVerdict::Equivalent { .. }),
        "{report:#?}"
    );
    reach_cli(file);
}

/// Whole-file verdicts for `root` and `callee` under `live`: the callee is proven on its
/// own, its caller is undecided.
fn reach_verdicts(a: &[u8], b: &[u8], live: LiveOptions, root: u16, callee: u16) {
    let verdict = |report: &IffEquivReport, id: u16| {
        report
            .resources
            .iter()
            .find(|r| r.id == id)
            .unwrap()
            .verdict
            .clone()
    };
    {
        for tree in [None, Some(root)] {
            let opts = IffEquivOptions {
                tree,
                live,
                ..IffEquivOptions::default()
            };
            let report = check_iff_equivalent(a, b, &opts).expect("check");
            match verdict(&report, root) {
                ResourceVerdict::Inconclusive { why } => {
                    assert!(why.contains(&format!("may run tree #{callee}")), "{why}");
                }
                other => panic!("expected Inconclusive, got {other:?}"),
            }
            if tree.is_none() {
                // The flag still holds for the callee checked on its own.
                assert!(
                    matches!(verdict(&report, callee), ResourceVerdict::Equivalent { .. }),
                    "{report:#?}"
                );
            }
        }
    }
}

/// `equiv` (`check_tree_with_callees`) for [`reach_trees_observe_their_returns`].
fn reach_cli(file: impl Fn(BhavChunk) -> Vec<u8>) {
    // `equiv` (`check_tree_with_callees`): a source whose callee returns true from a
    // true test, against a file whose callee returns true from a false one.
    let dir = scratch_dir("equiv-reach-returns");
    let source = dir.join("a.antics");
    std::fs::write(
        &source,
        r#"tree 4096 "Main" { if call 4097(temps) { return true; } return false; }
tree 4097 "Callee" { return true; }
"#,
    )
    .unwrap();
    let iff = dir.join("b.iff");
    std::fs::write(
        &iff,
        file(tree(4097, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 1), T, T)])),
    )
    .unwrap();
    let equiv = |tree: &str| {
        let out = cli()
            .arg("equiv")
            .arg(&source)
            .arg(&iff)
            .args(["--tree", tree, "--ignore-last-result"])
            .output()
            .unwrap();
        (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    };
    let (ok, text) = equiv("4097");
    assert!(ok && text.contains("EQUIVALENT"), "{text}");
    let (ok, text) = equiv("4096");
    assert!(
        !ok && text.contains("INCONCLUSIVE") && text.contains("tree #4097"),
        "{text}"
    );
}

/// In whole-file mode a tree is the same only if every tree it may run is: an
/// identical tree that calls an undecided one is undecided too, alone (`--tree`) or
/// with the whole file.
#[test]
fn whole_file_trees_depend_on_their_callees() {
    let root = air(
        r#"tree 4096 "Main" { if call 4097(temps) { return true; } return false; }"#,
        4096,
    );
    let file = |callee: Instruction| {
        write_iff(&[
            bhav_chunk(&root, "Main"),
            bhav_chunk(&tree(4097, 0, 0, vec![callee]), "Callee"),
        ])
        .unwrap()
    };
    // Both callees return true, but leave different last results: undecided.
    let a = file(expr((LIT, 0), EQ, (LIT, 0), T, T));
    let b = file(expr((LIT, 0), EQ, (LIT, 1), T, T));
    for tree in [None, Some(4096)] {
        let opts = IffEquivOptions {
            tree,
            ..IffEquivOptions::default()
        };
        let report = check_iff_equivalent(&a, &b, &opts).expect("check");
        assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
        let main = report.resources.iter().find(|r| r.id == 4096).unwrap();
        match &main.verdict {
            ResourceVerdict::Inconclusive { why } => {
                assert!(why.contains("may run tree #4097"), "{why}");
            }
            other => panic!("expected Inconclusive, got {other:?}"),
        }
    }
    // A callee proven equivalent keeps the caller identical.
    let dummy = write_iff(&[
        bhav_chunk(&root, "Main"),
        bhav_chunk(
            &tree(
                4097,
                0,
                0,
                vec![
                    expr((ATTR, 0), GT, (ATTR, 0), 1, 1),
                    expr((LIT, 0), EQ, (LIT, 0), T, T),
                ],
            ),
            "Callee",
        ),
    ])
    .unwrap();
    let opts = IffEquivOptions {
        tree: Some(4096),
        ..IffEquivOptions::default()
    };
    let report = check_iff_equivalent(&a, &dummy, &opts).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
}

/// Round 6, finding 1: an operand the checker does not track may fail to read or write
/// (`global[64]` is out of range, `memory.rs:426`), and a local the tree may not have
/// written may fail to read (`interpreter.rs:1277`). Each is a fault at its node, so
/// the same read one node later is a different fault.
#[test]
fn failing_operands_fault_at_their_node() {
    let pass = |next| expr((LIT, 0), EQ, (LIT, 0), next, next);
    let shifted = |op: Instruction| {
        let one = tree(4096, 0, 1, vec![pass(1), op.clone()]);
        let two = tree(4096, 0, 1, vec![pass(1), pass(2), op]);
        (one, two)
    };
    for op in [
        // A read the VM fails: global 64 is past the 64 globals.
        expr((TEMP, 0), SET, (GLOBAL, 64), T, T),
        // A read of a local nothing wrote yet.
        expr((TEMP, 0), SET, (LOCAL, 0), T, T),
        // A write the VM fails: global 5 is not writable.
        expr((GLOBAL, 5), SET, (LIT, 1), T, T),
    ] {
        let (one, two) = shifted(op);
        let result = live(&one, &two);
        assert_inconclusive(&result);
        let EquivResult::Inconclusive { reason } = &result else {
            unreachable!()
        };
        assert!(reason.contains("failing operand"), "{reason}");
        // The same tree on both sides is still equivalent.
        assert_equivalent(&live(&two, &two));
    }
    // A fault names the tree it happened in.
    let divide = |id| tree(id, 0, 0, vec![expr((TEMP, 0), DIV, (TEMP, 1), T, T)]);
    assert_equivalent(&live(&divide(4096), &divide(4096)));
    assert_inconclusive(&live(&divide(4096), &divide(4097)));
}

/// Round 6, finding 2: a jump to a missing node is `InvalidBranch { target, count }`
/// (`interpreter.rs:1484`), whose count is the tree's length.
#[test]
fn missing_node_faults_carry_the_tree_length() {
    let jump = expr((LIT, 0), EQ, (LIT, 0), 2, 2);
    let short = tree(4096, 0, 0, vec![jump.clone()]);
    let long = tree(4096, 0, 0, vec![jump, expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    let result = live(&short, &long);
    assert_inconclusive(&result);
    assert_equivalent(&live(&long, &long));
    // Lockstep pairs node 0 with node 0, but its proof does not cover the length.
    let ctx = LiveContext::default();
    let (_, combined) = check_bhav_equivalent_combined(
        &short,
        &ctx,
        &long,
        &ctx,
        &EquivOptions::default(),
        &LiveOptions::default(),
    );
    assert_inconclusive(&combined);
    // The VM enters formats 0x8000 to 0x8002 only.
    let mut odd = short.clone();
    odd.format = 0x8003;
    assert_inconclusive(&live(&short, &odd));
}

/// Review P1-1 and P2-4: `--global` and `--semi-global` supply argument counts as the
/// VM's loader keeps the trees (`program.rs:588`: physical order, deleted chunks
/// included, the last copy of an ID winning), only for their own range, and two
/// libraries that disagree on a tree are refused.
#[test]
fn library_arities_follow_the_loader() {
    let dir = scratch_dir("library-arities");
    // Tree 4096 calls `callee` with inline arguments; the node after the call is never
    // run and differs between the files. With six arguments the call reads into it.
    let object = |callee: u16, next: Instruction| {
        write_iff(&[bhav_chunk(
            &tree(4096, 0, 0, vec![prim(callee, [0; 8], T, T), next]),
            "Main",
        )])
        .unwrap()
    };
    let library = |chunks: &[(u16, u8, u16)]| {
        let chunks: Vec<ChunkData> = chunks
            .iter()
            .map(|&(id, args, flags)| ChunkData {
                flags,
                ..bhav_chunk(
                    &tree(id, args, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]),
                    "Lib",
                )
            })
            .collect();
        write_iff(&chunks).unwrap()
    };
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    };
    let run = |callee: u16, libraries: &[(&str, &std::path::Path)]| {
        let pa = write("a.iff", &object(callee, expr((LIT, 0), EQ, (LIT, 0), T, T)));
        let pb = write("b.iff", &object(callee, prim(3, [0; 8], T, T)));
        let mut cmd = cli();
        cmd.arg("verify-equiv").arg(&pa).arg(&pb);
        for (flag, path) in libraries {
            cmd.arg(flag).arg(path);
        }
        let out = cmd.output().unwrap();
        (
            out.status.code(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    };
    const LIVE: u16 = 0x10;
    const DELETED: u16 = 0x10 | 0x04;
    // A live copy with two arguments, then a deleted one with six: the VM keeps the
    // second. So do two live copies.
    for later in [DELETED, LIVE] {
        let lib = write("global.iff", &library(&[(300, 2, LIVE), (300, 6, later)]));
        let (code, text) = run(300, &[("--global", &lib)]);
        assert_eq!(code, Some(2), "{text}");
    }
    // The other way round, the call reads two arguments, inside its own operands.
    let lib = write("global.iff", &library(&[(300, 6, DELETED), (300, 2, LIVE)]));
    let (code, text) = run(300, &[("--global", &lib)]);
    assert_eq!(code, Some(0), "{text}");
    // A library supplies only its own range: a semi-global file's #300, or a global
    // file's #8192, says nothing, so the arity stays unknown.
    let none = write("none.iff", &library(&[(300, 0, LIVE), (8192, 0, LIVE)]));
    let (code, text) = run(300, &[("--global", &none)]);
    assert_eq!(code, Some(0), "{text}");
    let (code, text) = run(300, &[("--semi-global", &none)]);
    assert_eq!(code, Some(2), "{text}");
    let (code, text) = run(8192, &[("--semi-global", &none)]);
    assert_eq!(code, Some(0), "{text}");
    let (code, text) = run(8192, &[("--global", &none)]);
    assert_eq!(code, Some(2), "{text}");
    // Two libraries that disagree on a tree cannot both be the game's.
    let other = write("other.iff", &library(&[(300, 6, LIVE)]));
    let (code, text) = run(300, &[("--global", &none), ("--global", &other)]);
    assert_eq!(code, Some(2), "{text}");
    assert!(text.contains("library tree #300"), "{text}");
}

/// Round 6, fault inventory: a fault names its tree, and any tree can fault on entry
/// (more than 255 arguments, a full stack), so trees with different IDs are never
/// equivalent, even when their bodies are the same bytes and lockstep pairs every node.
#[test]
fn different_tree_ids_never_fault_alike() {
    let ctx = LiveContext::default();
    let combined = |a: &BhavChunk, b: &BhavChunk| {
        check_bhav_equivalent_combined(
            a,
            &ctx,
            b,
            &ctx,
            &EquivOptions::default(),
            &LiveOptions::default(),
        )
        .1
    };
    for body in [
        vec![expr((LIT, 0), EQ, (LIT, 0), T, T)],
        vec![expr((TEMP, 0), DIV, (TEMP, 1), T, T)],
    ] {
        let a = tree(4096, 0, 0, body.clone());
        let b = tree(4097, 0, 0, body);
        assert_inconclusive(&live(&a, &b));
        assert_inconclusive(&combined(&a, &b));
        assert_equivalent(&live(&a, &a));
        assert_equivalent(&combined(&a, &a));
    }
}

/// Round 6, finding 3: a call to 256..=4095 runs the global library's tree, whatever
/// the object's file holds under that ID (`interpreter.rs:695`). The file's own copy
/// must not supply the callee's argument count, which decides whether the call reads
/// the next node's bytes as arguments.
#[test]
fn library_trees_in_an_object_file_supply_nothing() {
    let embedded = tree(256, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    let file = |next: Instruction| {
        let main = tree(4096, 0, 0, vec![prim(256, [0; 8], T, F), next]);
        write_iff(&[bhav_chunk(&embedded, "Lib"), bhav_chunk(&main, "Main")]).unwrap()
    };
    // The node after the call is never run; only its opcode word differs.
    let a = file(expr((LIT, 0), EQ, (LIT, 0), T, T));
    let b = file(prim(3, [0; 8], T, T));
    let main_verdict = |extra: &[(u16, usize)]| {
        let opts = IffEquivOptions {
            extra_arities: extra.iter().copied().collect(),
            ..IffEquivOptions::default()
        };
        let report = check_iff_equivalent(&a, &b, &opts).expect("check");
        report
            .resources
            .iter()
            .find(|r| r.id == 4096)
            .unwrap()
            .verdict
            .clone()
    };
    // Unknown: the global tree may take more than four arguments.
    assert!(
        matches!(main_verdict(&[]), ResourceVerdict::Inconclusive { .. }),
        "{:?}",
        main_verdict(&[])
    );
    // Five arguments read the next node's opcode word.
    assert!(
        matches!(
            main_verdict(&[(256, 5)]),
            ResourceVerdict::Inconclusive { .. }
        ),
        "{:?}",
        main_verdict(&[(256, 5)])
    );
    // Four fit in the call's own operands.
    assert!(
        matches!(
            main_verdict(&[(256, 4)]),
            ResourceVerdict::Equivalent { .. }
        ),
        "{:?}",
        main_verdict(&[(256, 4)])
    );
    // The call runs the global tree, which may run any private tree of the object, not
    // the embedded copy: a private tree that differs leaves the caller undecided.
    let other = |target| tree(4097, 0, 0, vec![prim(5, [0; 8], target, target)]);
    let with_other = |target| {
        let main = tree(4096, 0, 0, vec![prim(256, [0; 8], T, F)]);
        write_iff(&[
            bhav_chunk(&embedded, "Lib"),
            bhav_chunk(&main, "Main"),
            bhav_chunk(&other(target), "Other"),
        ])
        .unwrap()
    };
    let opts = IffEquivOptions {
        extra_arities: [(256, 0)].into_iter().collect(),
        ..IffEquivOptions::default()
    };
    let report = check_iff_equivalent(&with_other(T), &with_other(F), &opts).expect("check");
    let main = report.resources.iter().find(|r| r.id == 4096).unwrap();
    match &main.verdict {
        ResourceVerdict::Inconclusive { why } => assert!(why.contains("#4097"), "{why}"),
        other => panic!("expected Inconclusive, got {other:?}"),
    }
}

/// Round 6, finding 4: the VM never runs an object file's copy of a library tree, so
/// it is neither compared nor replayed, and `--tree` refuses it.
#[test]
fn library_range_trees_are_never_replayed() {
    let returns = |target| {
        tree(
            256,
            0,
            0,
            vec![expr((LIT, 0), EQ, (LIT, 0), target, target)],
        )
    };
    let a = write_iff(&[bhav_chunk(&returns(T), "Lib")]).unwrap();
    let b = write_iff(&[bhav_chunk(&returns(F), "Lib")]).unwrap();
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    match &report.resources[0].verdict {
        ResourceVerdict::Inconclusive { why } => assert!(why.contains("never runs"), "{why}"),
        other => panic!("expected Inconclusive, got {other:?}"),
    }
    let opts = IffEquivOptions {
        tree: Some(256),
        ..IffEquivOptions::default()
    };
    let error = check_iff_equivalent(&a, &b, &opts).unwrap_err().to_string();
    assert!(error.contains("never runs BHAV #256"), "{error}");
    let dir = scratch_dir("library-tree");
    let (pa, pb) = (dir.join("a.iff"), dir.join("b.iff"));
    std::fs::write(&pa, &a).unwrap();
    std::fs::write(&pb, &b).unwrap();
    let out = cli()
        .arg("verify-equiv")
        .arg(&pa)
        .arg(&pb)
        .args(["--tree", "256"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    // The replay builds the program the VM would: a library tree has none, so its
    // candidate difference stays undecided; the same trees as private ones differ.
    assert_inconclusive(&live(&returns(T), &returns(F)));
    let private = |target| {
        tree(
            4096,
            0,
            0,
            vec![expr((LIT, 0), EQ, (LIT, 0), target, target)],
        )
    };
    assert_counterexample(&live(&private(T), &private(F)));
}

/// Round 6: trees that call each other. A cycle is proven by induction when every
/// tree in it is the same or proven equivalent, and a tree in the cycle that is not
/// leaves every caller in it undecided.
#[test]
fn recursive_call_graphs() {
    let calls = |callee| prim(callee, [0; 8], T, F);
    let main_a = tree(4096, 0, 0, vec![calls(4097)]);
    // The same call, then a dummy node on its true branch.
    let main_b = tree(
        4096,
        0,
        0,
        vec![prim(4097, [0; 8], 1, F), expr((LIT, 0), EQ, (LIT, 0), T, T)],
    );
    let back = tree(4097, 0, 0, vec![calls(4096)]);
    let file = |main: &BhavChunk, back: &BhavChunk| {
        write_iff(&[bhav_chunk(main, "Main"), bhav_chunk(back, "Back")]).unwrap()
    };
    let verdict = |report: &IffEquivReport, id: u16| {
        report
            .resources
            .iter()
            .find(|r| r.id == id)
            .unwrap()
            .verdict
            .clone()
    };
    // The dummy node leaves the scratch pointer set; this test is about the calls.
    let live = LiveOptions {
        observe_scratch_pointer: false,
        ..LiveOptions::default()
    };
    let (a, b) = (file(&main_a, &back), file(&main_b, &back));
    for tree in [None, Some(4096), Some(4097)] {
        let opts = IffEquivOptions {
            tree,
            live,
            ..IffEquivOptions::default()
        };
        let report = check_iff_equivalent(&a, &b, &opts).expect("check");
        assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
    }
    // Break the cycle's other tree: it now returns false whatever its callee does.
    let back_b = tree(4097, 0, 0, vec![prim(4096, [0; 8], F, F)]);
    let b = file(&main_b, &back_b);
    let opts = IffEquivOptions {
        live,
        ..IffEquivOptions::default()
    };
    let report = check_iff_equivalent(&a, &b, &opts).expect("check");
    for id in [4096, 4097] {
        assert!(
            matches!(verdict(&report, id), ResourceVerdict::Inconclusive { .. }),
            "{report:#?}"
        );
    }
}

// ---- Argument-free calls, stack object reads, node numbering, last result ----

const STACK_OBJ: u8 = 10;

/// Private tree 4096 (the callee, with the given header and two entry nodes) and a
/// caller 4097 whose nodes are `caller`; tree 4098 takes seven arguments.
fn call_file(callee_args: u8, callee_kind: u8, caller: Vec<Instruction>) -> Vec<u8> {
    let callee = BhavChunk {
        kind: callee_kind,
        ..tree(
            4096,
            callee_args,
            0,
            vec![
                expr((LIT, 0), EQ, (LIT, 0), T, T),
                expr((LIT, 0), EQ, (LIT, 0), F, F),
            ],
        )
    };
    let seven = tree(4098, 7, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    write_iff(&[
        bhav_chunk(&callee, "Callee"),
        bhav_chunk(&tree(4097, 0, 0, caller), "Caller"),
        bhav_chunk(&seven, "Seven"),
    ])
    .expect("write")
}

fn call_verdict(
    callee_args: u8,
    callee_kind: u8,
    a: Vec<Instruction>,
    b: Vec<Instruction>,
    opts: &IffEquivOptions,
) -> IffVerdict {
    let a = call_file(callee_args, callee_kind, a);
    let b = call_file(callee_args, callee_kind, b);
    check_iff_equivalent(&a, &b, opts).expect("check").verdict()
}

/// A call to a tree that takes no arguments and is not indexed reads none of its
/// operand bytes (`interpreter.rs`, `call`: no argument words, and the entry word only
/// picks the node of a kind-4 tree), so every spelling of it is the same call.
#[test]
fn argless_calls_ignore_their_operand_bytes() {
    let opts = IffEquivOptions::default();
    let call = |ops| vec![prim(4096, ops, T, F)];
    for ops in [
        [0; 8],
        [1, 2, 3, 4, 5, 6, 7, 8],
        [0xFF, 0xFF, 0, 0, 0, 0, 0, 0],
    ] {
        assert_eq!(
            call_verdict(0, 0, call([0xFF; 8]), call(ops), &opts),
            IffVerdict::Equivalent,
            "{ops:?}"
        );
    }
}

/// The operand bytes are read when the callee takes arguments, when it is indexed
/// (the entry word picks the node), and when the callee is unknown.
#[test]
fn calls_that_read_operands_keep_them() {
    let opts = IffEquivOptions::default();
    let call = |ops| vec![prim(4096, ops, T, F)];
    // One argument: word 0.
    assert_ne!(
        call_verdict(1, 0, call([0; 8]), call([1, 0, 0, 0, 0, 0, 0, 0]), &opts),
        IffVerdict::Equivalent
    );
    // Indexed: word 0 is the entry, and the two entries return differently.
    assert_ne!(
        call_verdict(0, 4, call([0; 8]), call([1, 0, 0, 0, 0, 0, 0, 0]), &opts),
        IffVerdict::Equivalent
    );
    // A tree the file does not hold (here global 256): its arity is unknown.
    let global = |ops| vec![prim(256, ops, T, F)];
    assert_ne!(
        call_verdict(0, 0, global([0xFF; 8]), global([0; 8]), &opts),
        IffVerdict::Equivalent
    );
    // ... unless the caller states the library tree takes no arguments, in both the
    // argument counts and the argument-free set.
    let library = |argless: &[u16], arity: Option<usize>| IffEquivOptions {
        extra_argless: argless.iter().copied().collect(),
        extra_arities: arity.map(|n| (256, n)).into_iter().collect(),
        ..IffEquivOptions::default()
    };
    assert_eq!(
        call_verdict(
            0,
            0,
            global([0xFF; 8]),
            global([0; 8]),
            &library(&[256], Some(0))
        ),
        IffVerdict::Equivalent
    );
    // A tree listed as argument-free but with no stated count, or a count that
    // contradicts it, still reads its operands.
    for arity in [None, Some(1)] {
        assert_ne!(
            call_verdict(
                0,
                0,
                global([0xFF; 8]),
                global([0; 8]),
                &library(&[256], arity)
            ),
            IffVerdict::Equivalent,
            "{arity:?}"
        );
    }
}

/// A library supplies only its own range of tree IDs: the global file 256 to 4095, a
/// semi-global file 8192 and up. A tree in the other range is not the one a call runs.
#[test]
fn library_argless_trees_come_from_their_own_range() {
    let none = |id| tree(id, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    let file = write_iff(&[
        bhav_chunk(&none(256), "Low"),
        bhav_chunk(&none(8192), "High"),
    ])
    .expect("write");
    let global = sims_antics::verify::library_argless_from_iff(&file, false).expect("global");
    assert_eq!(global, [256].into_iter().collect());
    let semi = sims_antics::verify::library_argless_from_iff(&file, true).expect("semi");
    assert_eq!(semi, [8192].into_iter().collect());
}

/// A local named like the `invalid_scope:` keyword stays an ordinary operand.
#[test]
fn breakpoint_on_a_local_named_invalid_scope() {
    let named = air(
        "tree 4096 \"Named\" {\n    let invalid_scope = 0;\n    breakpoint(invalid_scope);\n    return true;\n}\n",
        4096,
    );
    let breakpoint = named
        .instructions
        .iter()
        .find(|i| i.opcode == 15)
        .expect("a breakpoint node");
    // Local 0 (scope 25), not a scope word.
    assert_eq!(breakpoint.operands, [0, 0, 25, 0, 0, 0, 0, 0]);
}

/// A call with more than four arguments reads the instructions after it as
/// arguments, so the bytes of an argument-free call there still count.
#[test]
fn spilled_arguments_keep_argless_call_bytes() {
    let opts = IffEquivOptions::default();
    // Seven arguments are 14 bytes: 8 operands, then the next node's opcode, branches
    // and first operand word.
    let caller = |ops| vec![prim(4098, [0; 8], 1, F), prim(4096, ops, T, F)];
    assert_ne!(
        call_verdict(
            0,
            0,
            caller([0; 8]),
            caller([9, 0, 0, 0, 0, 0, 0, 0]),
            &opts
        ),
        IffVerdict::Equivalent
    );
    // The bytes past the spilled word are not arguments of either call.
    assert_eq!(
        call_verdict(
            0,
            0,
            caller([0; 8]),
            caller([0, 0, 9, 9, 9, 9, 9, 9]),
            &opts
        ),
        IffVerdict::Equivalent
    );
}

/// The stack object is a field of the running frame (`memory.rs`): reading it cannot
/// fail and changes nothing, so an unused read is dead.
#[test]
fn stack_object_reads_are_pure() {
    let reads = tree(
        4096,
        0,
        0,
        vec![expr((STACK_OBJ, 0), EQ, (STACK_OBJ, 0), T, T)],
    );
    let plain = tree(4096, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    assert_equivalent(&live(&reads, &plain));
    let copy = tree(
        4096,
        0,
        0,
        vec![
            expr((TEMP, 0), SET, (STACK_OBJ, 0), 1, F),
            expr((TEMP, 0), SET, (LIT, 0), T, T),
        ],
    );
    let zero = tree(4096, 0, 0, vec![expr((TEMP, 0), SET, (LIT, 0), T, T)]);
    assert_equivalent(&live(&copy, &zero));
}

/// Scope 10 has one entry: index 1 faults (native error 0xf).
#[test]
fn stack_object_reads_past_index_0_are_not_dead() {
    let reads = tree(
        4096,
        0,
        0,
        vec![expr((STACK_OBJ, 1), EQ, (STACK_OBJ, 1), T, T)],
    );
    let plain = tree(4096, 0, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    assert!(!matches!(
        live(&reads, &plain),
        EquivResult::Equivalent { .. }
    ));
}

/// Two reads of the stack object agree only while nothing can change it: a write
/// and a primitive in between both separate them.
#[test]
fn stack_object_reads_follow_writes_and_primitives() {
    let saved_equals_now = |between: Instruction| {
        tree(
            4096,
            0,
            0,
            vec![
                expr((TEMP, 0), SET, (STACK_OBJ, 0), 1, F),
                between,
                expr((TEMP, 0), EQ, (STACK_OBJ, 0), T, F),
            ],
        )
    };
    let always_true = |between: Instruction| {
        tree(
            4096,
            0,
            0,
            vec![
                expr((TEMP, 0), SET, (STACK_OBJ, 0), 1, F),
                between,
                expr((TEMP, 0), EQ, (TEMP, 0), T, F),
            ],
        )
    };
    let write = expr((STACK_OBJ, 0), SET, (LIT, 7), 2, F);
    assert!(!matches!(
        live(&saved_equals_now(write.clone()), &always_true(write)),
        EquivResult::Equivalent { .. }
    ));
    // Set To Next (opcode 31) may move the stack object.
    let next = prim(31, [0; 8], 2, 2);
    assert!(!matches!(
        live(&saved_equals_now(next.clone()), &always_true(next)),
        EquivResult::Equivalent { .. }
    ));
    // With nothing in between, the two reads agree.
    let nothing = expr((TEMP, 1), SET, (LIT, 0), 2, F);
    assert_equivalent(&live(
        &saved_equals_now(nothing.clone()),
        &always_true(nothing),
    ));
}

/// Constant branches and unreachable nodes need no node-for-node pairing when no
/// node can fault or yield.
#[test]
fn constant_branches_and_unreachable_nodes_are_equivalent() {
    let a = tree(
        4096,
        0,
        0,
        vec![
            expr((LIT, 1), EQ, (LIT, 1), 1, 2),
            expr((TEMP, 0), SET, (LIT, 1), T, F),
            expr((TEMP, 0), SET, (LIT, 2), T, F),
        ],
    );
    let b = tree(4096, 0, 0, vec![expr((TEMP, 0), SET, (LIT, 1), T, F)]);
    assert_equivalent(&live(&a, &b));
}

/// The frame records the node it is at, and a reset or a save while a tree sleeps
/// keeps it (`interpreter.rs`, `write_frame_to_storage`): moving a sleep to another
/// node number is not equivalent, however the rest lines up.
#[test]
fn renumbered_yields_are_not_equivalent() {
    let sleep = |next| prim(0, [0; 8], next, next);
    let a = tree(
        4096,
        1,
        0,
        vec![expr((LIT, 0), EQ, (LIT, 0), 1, 1), sleep(T)],
    );
    let b = tree(4096, 1, 0, vec![sleep(T)]);
    assert!(!matches!(live(&a, &b), EquivResult::Equivalent { .. }));
    let (_, combined) = check_bhav_equivalent_combined(
        &a,
        &LiveContext::default(),
        &b,
        &LiveContext::default(),
        &EquivOptions::default(),
        &LiveOptions::default(),
    );
    assert!(!matches!(combined, EquivResult::Equivalent { .. }));
}

/// Any tree's last branch result can be read when its frame is popped, not only an
/// action's: trees run by name, functional trees, and routing trees enter through
/// the same counted use. So a whole file with no interaction table still keeps it.
#[test]
fn last_result_is_kept_for_trees_outside_any_interaction_table() {
    let file = |test| {
        write_iff(&[bhav_chunk(
            &tree(4096, 0, 0, vec![expr((LIT, 0), EQ, (LIT, test), F, F)]),
            "Returns false",
        )])
        .expect("write")
    };
    let report =
        check_iff_equivalent(&file(0), &file(1), &IffEquivOptions::default()).expect("check");
    assert_ne!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
    let opts = IffEquivOptions {
        live: ignoring_last_result(),
        ..IffEquivOptions::default()
    };
    let report = check_iff_equivalent(&file(0), &file(1), &opts).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
}

/// A context that lists a tree as argument-free while giving it arguments is not
/// trusted: the operands are compared.
#[test]
fn argless_listing_needs_a_zero_argument_count() {
    let call = |ops| tree(4096, 0, 0, vec![prim(0x1000, ops, T, F)]);
    let (a, b) = (call([0; 8]), call([1, 0, 0, 0, 0, 0, 0, 0]));
    let ctx = |arity: Option<usize>| LiveContext {
        callee_arities: arity.map(|n| [(0x1000, n)].into_iter().collect()),
        argless_callees: Some([0x1000].into_iter().collect()),
        ..LiveContext::default()
    };
    for arity in [None, Some(1)] {
        let ctx = ctx(arity);
        assert!(
            !matches!(
                check_bhav_live_equivalent(&a, &ctx, &b, &ctx, &LiveOptions::default()),
                EquivResult::Equivalent { .. }
            ),
            "{arity:?}"
        );
        let (_, combined) = check_bhav_equivalent_combined(
            &a,
            &ctx,
            &b,
            &ctx,
            &EquivOptions::default(),
            &LiveOptions::default(),
        );
        assert!(
            !matches!(combined, EquivResult::Equivalent { .. }),
            "{arity:?}"
        );
    }
    let ctx = ctx(Some(0));
    assert_equivalent(&check_bhav_live_equivalent(
        &a,
        &ctx,
        &b,
        &ctx,
        &LiveOptions::default(),
    ));
}

/// `verify-equiv --global` / `--semi-global` mark a library tree argument-free only
/// when the file supplying its range gives it 0 arguments and it is not indexed, and
/// only when every file that supplies it agrees. Then `[0; 8]` and `[0xff; 8]` are
/// the same call.
#[test]
fn verify_equiv_cli_takes_argless_library_trees() {
    let dir = scratch_dir("cli-argless");
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    };
    const LIVE: u16 = 0x10;
    const DELETED: u16 = 0x10 | 0x04;
    // (id, args, kind, chunk flags), in physical order.
    let library_with_flags = |trees: &[(u16, u8, u8, u16)]| {
        let chunks: Vec<ChunkData> = trees
            .iter()
            .map(|&(id, args, kind, flags)| {
                let lib = BhavChunk {
                    kind,
                    ..tree(id, args, 0, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)])
                };
                ChunkData {
                    flags,
                    ..bhav_chunk(&lib, "Lib")
                }
            })
            .collect();
        write_iff(&chunks).unwrap()
    };
    let library = |trees: &[(u16, u8, u8)]| {
        let live: Vec<_> = trees
            .iter()
            .map(|&(id, args, kind)| (id, args, kind, LIVE))
            .collect();
        library_with_flags(&live)
    };
    let run = |callee: u16, libraries: &[(&str, &std::path::Path)]| {
        let object = |ops| {
            write_iff(&[bhav_chunk(
                &tree(4096, 0, 0, vec![prim(callee, ops, T, F)]),
                "Main",
            )])
        };
        let pa = write("a.iff", &object([0; 8]).unwrap());
        let pb = write("b.iff", &object([0xFF; 8]).unwrap());
        let mut cmd = cli();
        cmd.arg("verify-equiv").arg(&pa).arg(&pb);
        for (flag, path) in libraries {
            cmd.arg(flag).arg(path);
        }
        let out = cmd.output().unwrap();
        (
            out.status.code(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    };
    // Not argument-free: the call compares its operands, which differ.
    let assert_reads_operands = |(code, text): (Option<i32>, String)| {
        assert_eq!(code, Some(2), "{text}");
        assert!(text.contains("INCONCLUSIVE"), "{text}");
    };
    let plain = write("plain.iff", &library(&[(300, 0, 0), (8192, 0, 0)]));
    // Each library's own range, argument-free and not indexed: equivalent.
    let (code, text) = run(300, &[("--global", &plain)]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("EQUIVALENT"), "{text}");
    let (code, text) = run(8192, &[("--semi-global", &plain)]);
    assert_eq!(code, Some(0), "{text}");
    // No library, or a library whose range does not hold the callee: unknown.
    for libraries in [&[][..], &[("--semi-global", plain.as_path())][..]] {
        assert_reads_operands(run(300, libraries));
    }
    assert_reads_operands(run(8192, &[("--global", &plain)]));
    // One argument, or an indexed tree: the operands are read.
    for (args, kind) in [(1, 0), (0, 4)] {
        let lib = write("reads.iff", &library(&[(300, args, kind)]));
        assert_reads_operands(run(300, &[("--global", &lib)]));
    }
    // Two files that both supply the tree but disagree on its kind: not argument-free,
    // whichever comes first.
    let indexed = write("indexed.iff", &library(&[(300, 0, 4)]));
    for order in [[&plain, &indexed], [&indexed, &plain]] {
        assert_reads_operands(run(300, &[("--global", order[0]), ("--global", order[1])]));
    }
    // Inside one file the VM keeps the last copy of an ID, deleted or not, and so do
    // both readers: only an argument-free last copy counts.
    for last in [LIVE, DELETED] {
        let lib = write(
            "copies.iff",
            &library_with_flags(&[(300, 0, 0, LIVE), (300, 0, 4, last)]),
        );
        assert_reads_operands(run(300, &[("--global", &lib)]));
        let lib = write(
            "copies.iff",
            &library_with_flags(&[(300, 0, 4, LIVE), (300, 0, 0, last)]),
        );
        let (code, text) = run(300, &[("--global", &lib)]);
        assert_eq!(code, Some(0), "{text}");
    }
}

// ============================================================================
// Parameter domains (`docs/decisions/20261124-verifier-domains-and-speed.md`)
// ============================================================================

fn domains(items: &[(u16, u16, &str)]) -> sims_antics::verify::ParamDomains {
    let mut out = sims_antics::verify::ParamDomains::new();
    for &(tree, index, set) in items {
        out.entry(tree).or_default().insert(
            index,
            sims_antics::verify::ParamDomain::parse(set).expect("domain"),
        );
    }
    out
}

fn with_domains(items: &[(u16, u16, &str)]) -> LiveContext {
    LiveContext {
        param_domains: domains(items),
        ..LiveContext::default()
    }
}

/// `return param[0] < 4` against `return param[0] < 9`: they differ for 4 to 8 only.
fn defensive_pair() -> (BhavChunk, BhavChunk) {
    (
        tree(4200, 1, 0, vec![expr((PARAM, 0), LT, (LIT, 4), T, F)]),
        tree(4200, 1, 0, vec![expr((PARAM, 0), LT, (LIT, 9), T, F)]),
    )
}

/// A domain proves the pair equivalent only where it excludes every differing value,
/// and a differing value inside the domain is still a replayed counterexample.
#[test]
fn param_domains_prove_only_inside_the_domain() {
    let (a, b) = defensive_pair();
    let opts = LiveOptions::default();
    assert_counterexample(&live(&a, &b));
    for set in ["0..=3", "0..=3,20", "-100..=3", "9..=30000", "-32768..=3"] {
        let ctx = with_domains(&[(4200, 0, set)]);
        assert_equivalent(&live_with(&a, &b, &ctx, &opts));
    }
    // Adversarial: one differing value (5, or the edges 4 and 8) inside the domain.
    for set in ["0..=3,5", "0..=4", "8,9", "-32768..=32767", "6"] {
        let ctx = with_domains(&[(4200, 0, set)]);
        assert_counterexample(&live_with(&a, &b, &ctx, &opts));
    }
    // A domain for another tree, or a parameter the tree does not have, assumes nothing.
    for ctx in [
        with_domains(&[(4201, 0, "0..=3")]),
        with_domains(&[(4200, 1, "0..=3")]),
    ] {
        assert_counterexample(&live_with(&a, &b, &ctx, &opts));
    }
    // The sides must assume the same domain.
    let ctx = with_domains(&[(4200, 0, "0..=3")]);
    assert_inconclusive(&check_bhav_live_equivalent(
        &a,
        &ctx,
        &b,
        &LiveContext::default(),
        &opts,
    ));
}

/// A rewrite that is wrong on exactly one value: `temp[0] := param[0]` against one that
/// stores 7 when `param[0] == 2`.
#[test]
fn param_domain_bug_on_a_valid_value_is_a_counterexample() {
    let a = tree(4200, 1, 0, vec![expr((TEMP, 0), SET, (PARAM, 0), T, T)]);
    let b = tree(
        4200,
        1,
        0,
        vec![
            expr((PARAM, 0), EQ, (LIT, 2), 1, 2),
            expr((TEMP, 0), SET, (LIT, 7), T, T),
            expr((TEMP, 0), SET, (PARAM, 0), T, T),
        ],
    );
    let opts = LiveOptions::default();
    assert_counterexample(&live(&a, &b));
    for set in ["0,1,3", "-5..=-1,4..=9", "7"] {
        let ctx = with_domains(&[(4200, 0, set)]);
        assert_equivalent(&live_with(&a, &b, &ctx, &opts));
    }
    for set in ["0..=3", "2", "-1000..=1000", "1,2,3"] {
        let ctx = with_domains(&[(4200, 0, set)]);
        let result = live_with(&a, &b, &ctx, &opts);
        assert_counterexample(&result);
    }
}

/// Two constrained parameters split the entry into the product of their ranges.
#[test]
fn param_domains_combine_across_parameters() {
    // `temp[0] := param[0]` against a version wrong when both parameters are 5.
    let a = tree(4200, 2, 0, vec![expr((TEMP, 0), SET, (PARAM, 0), T, T)]);
    let b = tree(
        4200,
        2,
        0,
        vec![
            expr((PARAM, 0), EQ, (LIT, 5), 1, 3),
            expr((PARAM, 1), EQ, (LIT, 5), 2, 3),
            expr((TEMP, 0), SET, (LIT, 0), T, T),
            expr((TEMP, 0), SET, (PARAM, 0), T, T),
        ],
    );
    let opts = LiveOptions::default();
    for sets in [("0..=2,7", "0,1,5"), ("5", "0,1"), ("0..=4", "-9..=9")] {
        let ctx = with_domains(&[(4200, 0, sets.0), (4200, 1, sets.1)]);
        assert_equivalent(&live_with(&a, &b, &ctx, &opts));
    }
    for sets in [("0..=2,5", "0,1,5"), ("5", "5"), ("-9..=9", "4..=6")] {
        let ctx = with_domains(&[(4200, 0, sets.0), (4200, 1, sets.1)]);
        assert_counterexample(&live_with(&a, &b, &ctx, &opts));
    }
}

/// A tree whose loop the explorer cannot bound goes through the bisimulation, which
/// starts from the constrained entry.
#[test]
fn param_domains_reach_the_bisimulation() {
    // `loop { if param[0] < 4 { break } ; sleep }` style: an idle primitive inside a
    // loop on a parameter, against the same loop testing `< 9`.
    let looped = |bound: i16| {
        tree(
            4200,
            1,
            0,
            vec![
                expr((PARAM, 0), LT, (LIT, bound), T, 1),
                prim(0x0000, [1, 0, 0, 0, 0, 0, 0, 0], 0, 0),
            ],
        )
    };
    let (a, b) = (looped(4), looped(9));
    let opts = LiveOptions::default();
    let ctx = with_domains(&[(4200, 0, "0..=3")]);
    assert_equivalent(&live_with(&a, &b, &ctx, &opts));
    let ctx = with_domains(&[(4200, 0, "0..=5")]);
    assert!(
        !matches!(
            live_with(&a, &b, &ctx, &opts),
            EquivResult::Equivalent { .. }
        ),
        "a differing value inside the domain must not prove equivalent"
    );
}

/// `4096` calls `4097` (one argument) with `call_operands`; `4097` is the defensive pair.
fn domain_files(call_operands: [u8; 8]) -> (Vec<u8>, Vec<u8>) {
    let (callee_a, callee_b) = defensive_pair();
    let caller = tree(4096, 0, 0, vec![prim(4097, call_operands, T, T)]);
    let callee = |t: &BhavChunk| BhavChunk {
        id: 4097,
        ..t.clone()
    };
    let file = |callee: &BhavChunk| {
        write_iff(&[bhav_chunk(&caller, "caller"), bhav_chunk(callee, "callee")])
            .expect("write IFF")
    };
    (file(&callee(&callee_a)), file(&callee(&callee_b)))
}

fn literal_call(value: i16) -> [u8; 8] {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&value.to_le_bytes());
    ops
}

#[test]
fn whole_file_domains_check_the_files_own_callers() {
    let verdict = |report: &IffEquivReport, id: u16| {
        report
            .resources
            .iter()
            .find(|r| &r.tag == b"BHAV" && r.id == id)
            .map(|r| r.verdict.clone())
            .expect("tree report")
    };
    let opts = |set: Option<&str>| IffEquivOptions {
        param_domains: set.map_or_else(Default::default, |s| domains(&[(4097, 0, s)])),
        ..IffEquivOptions::default()
    };

    // A literal call inside the domain is discharged.
    let (a, b) = domain_files(literal_call(2));
    let report = check_iff_equivalent(&a, &b, &opts(None)).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Different, "{report:#?}");
    assert!(report.assumptions.is_empty());
    let report = check_iff_equivalent(&a, &b, &opts(Some("0..=3"))).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
    let lines = report.assumptions.join("\n");
    assert!(
        lines.contains("BHAV #4097 param[0] in {0..=3}: applied"),
        "{lines}"
    );
    // One call in each file.
    assert!(
        lines.contains("2 discharged, 0 not discharged, 0 violating"),
        "{lines}"
    );
    assert!(
        matches!(verdict(&report, 4097), ResourceVerdict::Equivalent { how } if how.contains("assuming param[0] in {0..=3}")),
        "{report:#?}"
    );

    // A literal call outside it voids the callee's proof, and its caller's with it,
    // even though the callee differs only outside the domain... and it is never
    // reported as a difference.
    let (a, b) = domain_files(literal_call(7));
    let report = check_iff_equivalent(&a, &b, &opts(Some("0..=3"))).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
    assert!(
        matches!(verdict(&report, 4097), ResourceVerdict::Inconclusive { why } if why.contains("violates") && why.contains("A #4096 node 0 passes param[0] = 7")),
        "{report:#?}"
    );
    assert!(
        matches!(verdict(&report, 4096), ResourceVerdict::Inconclusive { .. }),
        "{report:#?}"
    );
    // A caller that passes temps is not discharged: the assumption stands, and says so.
    let (a, b) = domain_files([0xFF; 8]);
    let report = check_iff_equivalent(&a, &b, &opts(Some("0..=3"))).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
    let lines = report.assumptions.join("\n");
    assert!(
        lines.contains("0 discharged, 2 not discharged (assumed: A #4096 node 0; B #4096 node 0)"),
        "{lines}"
    );
    // A domain for a parameter the tree does not have is printed as not applied.
    let opts = IffEquivOptions {
        param_domains: domains(&[(4097, 2, "1"), (4100, 0, "1"), (300, 0, "1")]),
        ..IffEquivOptions::default()
    };
    let report = check_iff_equivalent(&a, &b, &opts).expect("check");
    let lines = report.assumptions.join("\n");
    assert!(
        lines.contains("BHAV #4097 param[2] in {1}: NOT APPLIED (the tree takes 1 argument(s))"),
        "{lines}"
    );
    assert!(
        lines.contains("BHAV #4100 param[0] in {1}: NOT APPLIED (not both files define it)"),
        "{lines}"
    );
    assert!(
        lines.contains("BHAV #300 param[0] in {1}: NOT APPLIED (the VM never runs"),
        "{lines}"
    );
}

#[test]
fn verify_equiv_cli_param_domains() {
    let dir = scratch_dir("domains");
    let (a, b) = domain_files(literal_call(2));
    let (path_a, path_b) = (dir.join("a.iff"), dir.join("b.iff"));
    std::fs::write(&path_a, a).unwrap();
    std::fs::write(&path_b, b).unwrap();
    let run = |flags: &[&str]| {
        let out = cli()
            .arg("verify-equiv")
            .args(flags)
            .arg(&path_a)
            .arg(&path_b)
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
    let (code, text) = run(&[]);
    assert_eq!(code, Some(1), "{text}");
    assert!(!text.contains("ASSUM"), "{text}");
    let (code, text) = run(&["--param-domain", "4097:0=0..=3"]);
    assert_eq!(code, Some(0), "{text}");
    for line in [
        "ASSUMING parameter domains",
        "ASSUME: BHAV #4097 param[0] in {0..=3} (--param-domain 4097:0=0..=3)",
        "ASSUME: BHAV #4097 param[0] in {0..=3}: applied",
        "ASSUME: BHAV #4097 callers in the files: 2 discharged",
    ] {
        assert!(text.contains(line), "missing {line:?}:\n{text}");
    }
    // Behavioral mode takes domains too; a value that differs inside one does not hide.
    let (code, text) = run(&["--behavioral", "--param-domain", "0x1001:0=0..=3"]);
    assert_eq!(code, Some(0), "{text}");
    let (code, text) = run(&["--behavioral", "--param-domain", "4097:0=0..=3,6"]);
    assert_eq!(code, Some(1), "{text}");
    // Conflicting or malformed domains are errors.
    let (code, text) = run(&["--param-domain", "4097:0=1", "--param-domain", "4097:0=2"]);
    assert_eq!(code, Some(2), "{text}");
    assert!(text.contains("is assumed both in"), "{text}");
    for bad in [
        "4097=1",
        "4097:0=",
        "4097:0=3..=1",
        "4097:x=1",
        "4097:0=0..3",
    ] {
        let (code, text) = run(&["--param-domain", bad]);
        assert_eq!(code, Some(2), "{bad}: {text}");
    }
}

const ENUM_OBJECT: &str = r#"
object "Domain Probe" {
    guid: 0x7F53_0001;

    enum Meal {
        Soup = 0,
        Stew = 1,
        Pie = 3,
    }

    state served: i16;

    tree serve "Serve" (sim: Sim, meal: Meal, count: i16) {
        BODY
    }

    interaction "Serve" {
        run(sim) {
            serve(sim, Meal::Stew, 2);
        }
    }
}
"#;

/// The original keeps a defensive fallback for values no `Meal` has.
const DEFENSIVE_BODY: &str = "
        self.served = count;
        if meal == Meal::Soup {
        } else if meal == Meal::Stew {
        } else if meal == Meal::Pie {
        } else {
            self.served = 99;
        }";
/// The rewrite drops it.
const PLAIN_BODY: &str = "
        self.served = count;";
/// A rewrite with a bug on a valid variant, `Meal::Pie`.
const BUGGY_BODY: &str = "
        self.served = count;
        if meal == Meal::Pie {
            self.served = 98;
        }";

fn enum_object(
    dir: &std::path::Path,
    name: &str,
    body: &str,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let source = dir.join(format!("{name}.antics"));
    std::fs::write(&source, ENUM_OBJECT.replace("BODY", body.trim_start())).unwrap();
    let compiled = sims_antics::compile_v2_object_file(&source, None).expect("compile v2");
    let iff = dir.join(format!("{name}.iff"));
    std::fs::write(&iff, compiled.iff_bytes).unwrap();
    (source, iff)
}

#[test]
fn v2_enum_domains_are_extracted_by_positional_index() {
    let dir = scratch_dir("enum-extract");
    let (source, _) = enum_object(&dir, "a", DEFENSIVE_BODY);
    let found = sims_antics::verify::v2_enum_param_domains(&source, None).expect("extract");
    assert_eq!(found.len(), 1, "{found:#?}");
    let d = &found[0];
    // `sim: Sim` is not a BHAV parameter; `count: i16` is not an enum.
    assert_eq!((d.tree, d.param_index), (4098, 0), "{d:#?}");
    assert_eq!(
        (
            d.tree_name.as_str(),
            d.param_name.as_str(),
            d.enum_name.as_str()
        ),
        ("serve", "meal", "Meal")
    );
    // Adjacent variants print as a range.
    assert_eq!(d.domain.to_string(), "0..=1,3");
    // Two objects in one file are refused: trees are matched by label in one object.
    let two = dir.join("two.antics");
    std::fs::write(
        &two,
        format!(
            "{}\n{}",
            ENUM_OBJECT.replace("BODY", PLAIN_BODY.trim_start()),
            ENUM_OBJECT
                .replace("BODY", PLAIN_BODY.trim_start())
                .replace("Domain Probe", "Other Probe")
                .replace("0x7F53_0001", "0x7F53_0002")
        ),
    )
    .unwrap();
    let err = sims_antics::verify::v2_enum_param_domains(&two, None).expect_err("two objects");
    assert!(err.to_string().contains("exactly one"), "{err}");
}

#[test]
fn verify_equiv_cli_v2_enum_domains() {
    let dir = scratch_dir("enum-cli");
    let (source, original) = enum_object(&dir, "original", DEFENSIVE_BODY);
    let (_, plain) = enum_object(&dir, "plain", PLAIN_BODY);
    let (_, buggy) = enum_object(&dir, "buggy", BUGGY_BODY);
    let run = |b: &std::path::Path, flags: &[&std::ffi::OsStr]| {
        let out = cli()
            .arg("verify-equiv")
            .args(flags)
            .arg(&original)
            .arg(b)
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
    // The rewrite runs fewer `Expression` nodes, so the scratch pointer is not observed.
    let flag = [
        std::ffi::OsStr::new("--ignore-scratch-pointer"),
        std::ffi::OsStr::new("--v2-enum-domains"),
        source.as_os_str(),
    ];
    // Unconditionally the dropped fallback is a difference (`meal = 2`), which the
    // checker does not prove equivalent.
    let (code, text) = run(&plain, &flag[..1]);
    assert_ne!(code, Some(0), "{text}");
    assert!(!text.contains("ASSUM"), "{text}");
    // Assuming `meal` is a `Meal`, the rewrite is equivalent, and the run says so.
    let (code, text) = run(&plain, &flag);
    assert_eq!(code, Some(0), "{text}");
    for line in [
        "ASSUMING parameter domains",
        "ASSUME: BHAV #4098 param[0] in {0..=1,3} (",
        ": tree serve parameter meal: Meal)",
        "ASSUME: BHAV #4098 callers in the files: 2 discharged, 0 not discharged, 0 violating",
        "BHAV #4098: equivalent (live-output",
        "assuming param[0] in {0..=1,3}",
    ] {
        assert!(text.contains(line), "missing {line:?}:\n{text}");
    }
    // A bug on a valid variant is still a counterexample, in both modes.
    for behavioral in [false, true] {
        let mut flags = flag.to_vec();
        if behavioral {
            flags.push(std::ffi::OsStr::new("--behavioral"));
        }
        let (code, text) = run(&buggy, &flags);
        assert_eq!(code, Some(1), "{text}");
        assert!(text.contains("BHAV #4098: DIFFERENT"), "{text}");
    }
}

/// A difference that shows only at a domain's bound still gets a replayed witness:
/// `temp[0] = param[0]` and `temp[0] = 0` differ at -1 only, inside `-1..=0`.
#[test]
fn param_domain_bound_gets_a_witness() {
    let a = tree(4200, 1, 0, vec![expr((TEMP, 0), SET, (PARAM, 0), T, T)]);
    let b = tree(4200, 1, 0, vec![expr((TEMP, 0), SET, (LIT, 0), T, T)]);
    let opts = LiveOptions::default();
    assert_counterexample(&live_with(
        &a,
        &b,
        &with_domains(&[(4200, 0, "-1..=0")]),
        &opts,
    ));
    assert_counterexample(&live_with(
        &a,
        &b,
        &with_domains(&[(4200, 0, "0..=5")]),
        &opts,
    ));
    // Two inputs that differ only when they sit at different ends of their ranges.
    let a = tree(4200, 2, 0, vec![expr((TEMP, 0), SET, (PARAM, 0), T, T)]);
    let b = tree(4200, 2, 0, vec![expr((TEMP, 0), SET, (PARAM, 1), T, T)]);
    let both = with_domains(&[(4200, 0, "100..=101"), (4200, 1, "100..=101")]);
    assert_counterexample(&live_with(&a, &b, &both, &opts));
}

/// Callers are read from the nodes the VM can run: a row no entry reaches is not a call,
/// even when its opcode field names the constrained tree.
#[test]
fn domain_callers_are_read_from_runnable_nodes_only() {
    let file = |caller: &BhavChunk, callee: &BhavChunk| {
        write_iff(&[bhav_chunk(caller, "caller"), bhav_chunk(callee, "callee")]).expect("write IFF")
    };
    let (callee_a, callee_b) = defensive_pair();
    let callee = |t: &BhavChunk| BhavChunk {
        id: 4097,
        ..t.clone()
    };
    let opts = IffEquivOptions {
        param_domains: domains(&[(4097, 0, "0..=3")]),
        ..IffEquivOptions::default()
    };
    // Node 1 holds a call passing 7. Node 0 reaches it only when it branches there on
    // an outcome it can have: not when it never does, and not on the false edge of an
    // assignment, which always succeeds. (Each count covers both files.)
    let call_first = |to_node_1: u8| prim(4097, literal_call(2), T, to_node_1);
    let assign_first = expr((TEMP, 0), SET, (LIT, 1), T, 1);
    for (first, violating) in [(call_first(T), 0), (assign_first, 0), (call_first(1), 2)] {
        let caller = tree(4096, 0, 0, vec![first, prim(4097, literal_call(7), T, T)]);
        let (a, b) = (
            file(&caller, &callee(&callee_a)),
            file(&caller, &callee(&callee_b)),
        );
        let report = check_iff_equivalent(&a, &b, &opts).expect("check");
        let lines = report.assumptions.join("\n");
        assert!(
            lines.contains(&format!("{violating} violating")),
            "{lines}\n{report:#?}"
        );
        let expected = if violating == 0 {
            IffVerdict::Equivalent
        } else {
            IffVerdict::Inconclusive
        };
        assert_eq!(report.verdict(), expected, "{report:#?}");
    }
}

/// A violating fifth argument, spilled into the row after the call, voids the callee's
/// proof and the proofs of every tree that runs it, two callers up.
#[test]
fn spilled_argument_violation_reaches_every_caller() {
    let callee = |bound: i16| tree(4097, 5, 0, vec![expr((PARAM, 4), LT, (LIT, bound), T, F)]);
    let file = |bound: i16, fifth: u16| {
        let middle = tree(
            4096,
            0,
            0,
            vec![
                prim(4097, [0, 0, 1, 0, 2, 0, 3, 0], T, T),
                // Data: the fifth argument, in the opcode field.
                prim(fifth, [0; 8], T, T),
            ],
        );
        let top = tree(4098, 0, 0, vec![prim(4096, [0; 8], T, T)]);
        write_iff(&[
            bhav_chunk(&top, "top"),
            bhav_chunk(&middle, "middle"),
            bhav_chunk(&callee(bound), "callee"),
        ])
        .expect("write IFF")
    };
    let opts = IffEquivOptions {
        param_domains: domains(&[(4097, 4, "0..=3")]),
        ..IffEquivOptions::default()
    };
    let verdict = |report: &IffEquivReport, id: u16| {
        report
            .resources
            .iter()
            .find(|r| &r.tag == b"BHAV" && r.id == id)
            .map(|r| r.verdict.clone())
            .expect("tree report")
    };
    // Passing 2: discharged, and the pair differs only outside the domain.
    let report = check_iff_equivalent(&file(4, 2), &file(9, 2), &opts).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
    // Passing 7: a violation, so no tree on the way up proves.
    let report = check_iff_equivalent(&file(4, 7), &file(9, 7), &opts).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Inconclusive, "{report:#?}");
    for id in [4097, 4096, 4098] {
        assert!(
            matches!(verdict(&report, id), ResourceVerdict::Inconclusive { .. }),
            "#{id}: {report:#?}"
        );
    }
    assert!(
        report
            .assumptions
            .join("\n")
            .contains("passes param[4] = 7"),
        "{report:#?}"
    );
}

/// A pack whose template declares `Mode = {0, 7}` and whose object's own module
/// declares another `Mode = {0}`.
fn mode_pack(dir: &std::path::Path, object_trees: &str) -> std::path::PathBuf {
    std::fs::create_dir_all(dir.join("objects")).unwrap();
    std::fs::write(
        dir.join("pack.toml"),
        r#"
[pack]
id = "domain_pack"
name = "Domain Pack"
version = "0.1.0"
archive = "domain-pack.far"
objects = ["probe"]
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("templates.antics"),
        r#"
pub enum Mode {
    Off = 0,
    Turbo = 7,
}

pub template Gadget {
    state level: i16;

    tree run_mode "Run Mode" (mode: Mode) {
        self.level = 1;
        if mode == Mode::Turbo {
            self.level = 2;
        }
    }
}
"#,
    )
    .unwrap();
    let object = dir.join("objects/probe.antics");
    std::fs::write(
        &object,
        format!(
            r#"
use domain_pack::templates::Gadget;

enum Mode {{
    Off = 0,
}}

object "Mode Probe" from Gadget {{
    guid: 0x7F53_0101;
{object_trees}
}}
"#
        ),
    )
    .unwrap();
    object
}

/// An inherited tree's enum resolves where the tree was declared, an overriding tree's
/// where it was, and the domain follows the label the override gives.
#[test]
fn v2_enum_domains_follow_the_declaring_module() {
    let dir = scratch_dir("enum-modules");
    let inherited = mode_pack(&dir.join("inherited"), "");
    let found =
        sims_antics::verify::v2_enum_param_domains(&inherited, Some(&dir.join("inherited")))
            .expect("extract");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(
        (found[0].label.as_str(), found[0].domain.to_string()),
        ("Run Mode", "0,7".to_owned()),
        "{found:#?}"
    );

    let overridden = mode_pack(
        &dir.join("overridden"),
        r#"
    tree run_mode "Run Mode Two" (mode: Mode) {
        self.level = 3;
    }
"#,
    );
    let found =
        sims_antics::verify::v2_enum_param_domains(&overridden, Some(&dir.join("overridden")))
            .expect("extract");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(
        (found[0].label.as_str(), found[0].domain.to_string()),
        ("Run Mode Two", "0".to_owned()),
        "{found:#?}"
    );
}

/// `@frame(args: 2)` gives a tree more arguments than positional parameters; the enum
/// parameter is still `param[0]`.
#[test]
fn v2_enum_domains_allow_extra_frame_arguments() {
    let dir = scratch_dir("enum-frame");
    let source = dir.join("framed.antics");
    std::fs::write(
        &source,
        ENUM_OBJECT
            .replace("BODY", PLAIN_BODY.trim_start())
            .replace(
                "    tree serve \"Serve\"",
                "    @frame(args: 3)\n    tree serve \"Serve\"",
            ),
    )
    .unwrap();
    let found = sims_antics::verify::v2_enum_param_domains(&source, None).expect("extract");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(
        (found[0].tree, found[0].param_index),
        (4098, 0),
        "{found:#?}"
    );
}

/// A v2 domain applies to a BHAV only when both files give it the source's label.
#[test]
fn verify_equiv_cli_v2_enum_domains_check_labels() {
    let dir = scratch_dir("enum-labels");
    let (source, original) = enum_object(&dir, "original", DEFENSIVE_BODY);
    let relabeled_source = dir.join("relabeled.antics");
    std::fs::write(
        &relabeled_source,
        ENUM_OBJECT
            .replace("BODY", PLAIN_BODY.trim_start())
            .replace("tree serve \"Serve\"", "tree serve \"Reset\""),
    )
    .unwrap();
    let relabeled = dir.join("relabeled.iff");
    std::fs::write(
        &relabeled,
        sims_antics::compile_v2_object_file(&relabeled_source, None)
            .expect("compile v2")
            .iff_bytes,
    )
    .unwrap();
    let out = cli()
        .arg("verify-equiv")
        .arg("--v2-enum-domains")
        .arg(&source)
        .arg("--ignore-scratch-pointer")
        .arg(&original)
        .arg(&relabeled)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(2), "{text}");
    assert!(
        text.contains("lowers to BHAV #4098") && text.contains("labeled \"Reset\""),
        "{text}"
    );
    // Another object that gives the same BHAV the same label is not the source's object.
    let other_source = dir.join("other.antics");
    std::fs::write(
        &other_source,
        ENUM_OBJECT
            .replace("BODY", PLAIN_BODY.trim_start())
            .replace("0x7F53_0001", "0x7F53_0077"),
    )
    .unwrap();
    let other = dir.join("other.iff");
    std::fs::write(
        &other,
        sims_antics::compile_v2_object_file(&other_source, None)
            .expect("compile v2")
            .iff_bytes,
    )
    .unwrap();
    let out = cli()
        .arg("verify-equiv")
        .arg("--v2-enum-domains")
        .arg(&source)
        .arg(&other)
        .arg(&other)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(2), "{text}");
    assert!(text.contains("(GUID 0x7F530001) is not in"), "{text}");
}

/// A kind-4 tree's first parameter is its entry node, so a domain on it is never
/// applied: restricting it to node 0 would hide a difference at node 1.
#[test]
fn param_domains_never_restrict_a_kind_4_entry() {
    let kind4 = |second: i16| BhavChunk {
        kind: 4,
        ..tree(
            4200,
            1,
            0,
            vec![
                expr((TEMP, 0), SET, (LIT, 1), T, T),
                expr((TEMP, 0), SET, (LIT, second), T, T),
            ],
        )
    };
    let (a, b) = (kind4(1), kind4(2));
    let opts = LiveOptions::default();
    assert_counterexample(&live(&a, &b));
    assert_counterexample(&live_with(&a, &b, &with_domains(&[(4200, 0, "0")]), &opts));
}

/// `@id(N)` names a tree's BHAV even when another tree shares its label.
#[test]
fn v2_enum_domains_follow_pinned_ids() {
    let dir = scratch_dir("enum-pinned");
    let source = dir.join("pinned.antics");
    std::fs::write(
        &source,
        ENUM_OBJECT
            .replace("BODY", PLAIN_BODY.trim_start())
            .replace(
                "    tree serve \"Serve\"",
                "    @id(4201) tree tidy \"Serve\" () {\n        self.served = 0;\n    }\n\n    @id(4200) tree serve \"Serve\"",
            ),
    )
    .unwrap();
    let found = sims_antics::verify::v2_enum_param_domains(&source, None).expect("extract");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(
        (found[0].tree, found[0].param_index),
        (4200, 0),
        "{found:#?}"
    );
}
