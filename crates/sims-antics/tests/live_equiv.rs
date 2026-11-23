//! Live-output equivalence (`check_bhav_live_equivalent`), the combined lockstep→live
//! checker, and the whole-file `verify-equiv` checker.

use sims_antics::encode::encode_bhav;
use sims_antics::ir::BhavChunk;
use sims_antics::{
    EquivMethod, EquivOptions, EquivResult, IffEquivOptions, IffVerdict, LiveContext, LiveOptions,
    ResourceVerdict, check_bhav_equivalent, check_bhav_equivalent_combined,
    check_bhav_live_equivalent, check_iff_equivalent, compile_air,
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

/// Maxis' "CT - Is Fridge" shape: a single `my.attr[0] > my.attr[0]` node whose both
/// branches return true. Reading `my.attr[0]` cannot trap (every object has 8
/// attributes), so the compare is a dead pure read and the tree is `return true;`.
#[test]
fn dead_pure_read_dummy_node_equals_return_true() {
    let fridge = tree(4143, 0, 0, vec![expr((ATTR, 0), GT, (ATTR, 0), T, T)]);
    let high_level = air(r#"tree 4143 "CT - Is Fridge" { return true; }"#, 4143);

    // The lockstep checker treats the attribute read as an observable event.
    assert!(!matches!(
        check_bhav_equivalent(&fridge, &high_level, &EquivOptions::default()),
        EquivResult::Equivalent { .. }
    ));
    assert_equivalent(&live(&fridge, &high_level));
    let ctx = LiveContext::default();
    let (method, result) = check_bhav_equivalent_combined(
        &fridge,
        &ctx,
        &high_level,
        &ctx,
        &EquivOptions::default(),
        &LiveOptions::default(),
    );
    assert_eq!(method, EquivMethod::LiveOutput);
    assert_equivalent(&result);

    // Different exits are still told apart.
    let returns_false = air(r#"tree 4143 "X" { return false; }"#, 4143);
    assert_counterexample(&live(&fridge, &returns_false));
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
    let dummy = tree(4096, 0, 0, vec![expr((ATTR, 8), GT, (ATTR, 8), T, T)]);
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
    let writes_local = tree(4096, 0, 1, vec![expr((LOCAL, 0), SET, (LIT, 3), T, ERR)]);
    let nothing = tree(4096, 0, 1, vec![expr((LIT, 0), EQ, (LIT, 0), T, T)]);
    assert_equivalent(&live(&writes_local, &nothing));

    let strict = LiveOptions {
        observe_frame_residue: true,
        ..LiveOptions::default()
    };
    assert_counterexample(&live_with(
        &writes_local,
        &nothing,
        &LiveContext::default(),
        &strict,
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
    assert_equivalent(&live(&looped, &direct));
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

    // Guarding the zero case changes a VM trap into `return false`.
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
        EquivResult::Counterexample { divergence, .. } => {
            assert!(divergence.contains("trap"), "{divergence}");
        }
        other => panic!("expected Counterexample, got {other:?}"),
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
    assert_counterexample(&live(&bare, &after_expression));
    let lenient = LiveOptions {
        observe_scratch_pointer: false,
        ..LiveOptions::default()
    };
    assert_equivalent(&live_with(
        &bare,
        &after_expression,
        &LiveContext::default(),
        &lenient,
    ));

    // Different primitive operands are a counterexample.
    let other_sound = tree(4096, 0, 0, vec![prim(23, [2, 0, 0, 0, 0, 0, 0, 0], T, T)]);
    assert_counterexample(&live(&bare, &other_sound));
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
    assert_counterexample(&live(&a, &c));
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
        bhav_chunk(&fridge, "original label"),
        str_chunk(128, &[("alpha", "first"), ("beta", "")]),
    ])
    .expect("write A");
    let b = write_iff(&[
        bhav_chunk(&high_level, "new label"),
        str_chunk(128, &[("alpha", "changed comment"), ("beta", "")]),
    ])
    .expect("write B");
    (a, b)
}

#[test]
fn whole_file_equivalence_with_notes() {
    let (a, b) = fridge_files();
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
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
    assert!(bhav.notes.iter().any(|n| n.contains("label")), "{bhav:?}");
    let strings = report
        .resources
        .iter()
        .find(|r| &r.tag == b"STR#")
        .expect("STR# report");
    assert!(matches!(
        strings.verdict,
        ResourceVerdict::Equivalent { .. }
    ));
    assert!(strings.notes.iter().any(|n| n.contains("comment")));
}

#[test]
fn whole_file_differences_name_indices_only() {
    let (a, _) = fridge_files();
    let fridge = tree(4143, 0, 0, vec![expr((ATTR, 0), GT, (ATTR, 0), T, T)]);
    let b = write_iff(&[
        bhav_chunk(&fridge, "original label"),
        str_chunk(128, &[("alpha", "first"), ("secret text", "")]),
        str_chunk(129, &[("extra", "")]),
    ])
    .expect("write B");
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Different);
    let text = format!("{report:?}");
    assert!(
        !text.contains("secret"),
        "report leaks string contents: {text}"
    );
    assert!(text.contains("indices 1"), "{text}");
    assert!(text.contains("only in file B"), "{text}");
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
            bhav_chunk(&returns_false, "x"),
            str_chunk(128, &[("alpha", "first"), ("beta", "")]),
        ])
        .unwrap(),
    )
    .unwrap();
    let out = cli()
        .arg("verify-equiv")
        .arg(&path_a)
        .arg(&path_c)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("BHAV #4143: DIFFERENT"), "{stdout}");

    // `my.attr[8]` without `--my-attributes` is inconclusive (exit 2), with it equivalent.
    let attr8 = tree(4143, 0, 0, vec![expr((ATTR, 8), GT, (ATTR, 8), T, T)]);
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
        .args(["verify-equiv", "--tree", "4143"])
        .arg(&path_a)
        .arg(&path_b)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("EQUIVALENT: 1 resources"), "{stdout}");
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

    // Without calls the frame size is unobservable (residue is opt-in).
    let no_call_more = tree(4096, 0, 4, vec![expr((TEMP, 0), SET, (LIT, 1), T, ERR)]);
    let no_call_fewer = tree(4096, 0, 0, vec![expr((TEMP, 0), SET, (LIT, 1), T, ERR)]);
    assert_equivalent(&live(&no_call_more, &no_call_fewer));
}
