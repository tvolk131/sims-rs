//! `lift --v2` wave 2 spellings: `f()?;`, `if a || b`, shared arm tails, `x.remove()`,
//! `set_to_next(.., dest: x)` into locals, parameters and temps, `find_best_object(kind)` as a
//! condition, the definition and job cells (`x.object_type`, `x.sub_index`,
//! `x.has_function[N]`, `job_field(F)`), and, in behavioral mode, temps calls to trees with
//! arguments. Each is lifted in strict and in behavioral mode. Every object is synthetic
//! (GUIDs in the clean-room 0x7E00_xxxx range).

use sims_antics::compile_v2_source;
use sims_antics::v2::{
    DecompileV2Options, PackResolverConfig, TreeCheck, decompile_iff_to_v2_report, lint_v2_source,
};
use sims_formats::iff::Iff;

fn compile(source: &str) -> Iff {
    let object = compile_v2_source("wave2.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{e}\n{source}"));
    Iff::parse(object.iff_bytes).expect("parse IFF")
}

/// Lifts `source`'s object in strict or behavioral mode: every tree proven, the text compiles
/// again and lints clean.
fn lift(source: &str, behavioral: bool) -> String {
    let options = DecompileV2Options {
        behavioral,
        ..DecompileV2Options::default()
    };
    let report = decompile_iff_to_v2_report(&compile(source), &options).expect("lift");
    assert!(report.compiled);
    assert!(
        report
            .trees
            .iter()
            .all(|t| matches!(t.check, TreeCheck::Identical | TreeCheck::Equivalent)),
        "{:?}\n{}",
        report.trees,
        report.source
    );
    compile(&report.source);
    let lints = lint_v2_source(&report.source, "lifted.antics").expect("lint");
    assert!(lints.is_empty(), "{lints:?}\n{}", report.source);
    report.source
}

fn has(lifted: &str, spellings: &[&str]) {
    for s in spellings {
        assert!(lifted.contains(s), "missing {s:?} in:\n{lifted}");
    }
}

fn lacks(lifted: &str, spellings: &[&str]) {
    for s in spellings {
        assert!(!lifted.contains(s), "unexpected {s:?} in:\n{lifted}");
    }
}

/// The text of tree `id`, from its attribute to the next tree's.
fn tree(lifted: &str, id: u16) -> &str {
    let at = lifted
        .find(&format!("#[bhav(id = {id}"))
        .unwrap_or_else(|| panic!("no tree {id} in:\n{lifted}"));
    let rest = &lifted[at..];
    let end = rest[1..].find("#[bhav(").map_or(rest.len(), |e| e + 1);
    &rest[..end]
}

#[test]
fn failed_calls_returning_error_lift_as_try() {
    let source = r#"object "Fable Quill" {
    guid: 0x7E00_0150;
    #[bhav(id = 4300, locals = 0)]
    fn ready() -> bool {
        if temp[0] > 3 { return false; }
        return true;
    }
    #[bhav(id = 4301, locals = 0)]
    fn work() {
        if !ready() { return error; }
        temp[1] = 2;
        if !ready() { return error; }
        temp[2] = 3;
    }
    #[bhav(id = 4302, locals = 0)]
    fn finish() -> bool {
        temp[3] = 1;
        if ready() { return true; } else { return error; }
    }
}"#;
    for behavioral in [false, true] {
        let lifted = lift(source, behavioral);
        let work = tree(&lifted, 4301);
        assert_eq!(work.matches("()?;").count(), 2, "{work}");
        lacks(work, &["return error;"]);
        // With a plain return on the other arm, the return follows the `?`.
        let finish = tree(&lifted, 4302);
        has(finish, &["()?;\n        return true;"]);
        lacks(finish, &["return error;"]);
    }
}

#[test]
fn same_arms_join_with_or_and_shared_tails_move_after_the_chain() {
    let source = r#"object "Fable Loom" {
    guid: 0x7E00_0151;
    #[bhav(id = 4300, locals = 0)]
    fn weave() {
        if temp[0] == 1 {
            temp[1] = 5;
            temp[2] = 6;
        } else if temp[0] == 7 {
            temp[1] = 5;
            temp[2] = 6;
        } else if temp[0] == 9 {
            temp[3] = 1;
            temp[4] = 8;
            temp[5] = 9;
        } else {
            temp[3] = 2;
            temp[4] = 8;
            temp[5] = 9;
        }
    }
}"#;
    // Behavioral mode does not compare node numbers, so one copy of each joined body is
    // proven the same as the original's two.
    let lifted = lift(source, true);
    let weave = tree(&lifted, 4300);
    has(weave, &["if temp[0] == 1 || temp[0] == 7 {"]);
    assert_eq!(weave.matches("temp[4] = 8;").count(), 1, "{weave}");
    assert_eq!(weave.matches("temp[1] = 5;").count(), 1, "{weave}");
    // Strict mode keeps whichever spelling it proves.
    lift(source, false);
}

#[test]
fn remove_object_lifts_as_remove() {
    let source = r#"object "Fable Broom" {
    guid: 0x7E00_0152;
    #[bhav(id = 4300, locals = 0)]
    fn sweep() {
        stack_obj = temp[0];
        stack_obj.remove(skip_resets: true);
        stack_obj = temp[1];
        if !stack_obj.remove() { return false; }
        temp[2] = 1;
    }
}"#;
    for behavioral in [false, true] {
        let lifted = lift(source, behavioral);
        has(
            &lifted,
            &[
                "stack_obj.remove(skip_resets: true);",
                "if !stack_obj.remove() {",
            ],
        );
        lacks(&lifted, &["remove_object("]);
    }
}

#[test]
fn set_to_next_writes_locals_params_and_temps() {
    let source = r#"object "Fable Abacus" {
    guid: 0x7E00_0153;
    #[bhav(id = 4300, args = 4, locals = 1)]
    fn count() {
        while set_to_next(person, dest: temp[2]) {
            temp[3] += 1;
        }
        if set_to_next(non_person, dest: param[1]) {
            temp[4] = 1;
        }
    }
}"#;
    for behavioral in [false, true] {
        let lifted = lift(source, behavioral);
        has(
            &lifted,
            &[
                "while set_to_next(person, dest: temp[2]) {",
                "set_to_next(non_person, dest: param[1])",
            ],
        );
        lacks(&lifted, &["raw"]);
    }
}

#[test]
fn find_best_object_lifts_as_a_condition() {
    let source = r#"object "Fable Ladle" {
    guid: 0x7E00_0154;
    #[bhav(id = 4300, locals = 0)]
    fn seek() {
        if find_best_object(cook) {
            temp[1] = 1;
        }
        temp[2] = 2;
    }
}"#;
    for behavioral in [false, true] {
        let lifted = lift(source, behavioral);
        has(&lifted, &["if find_best_object(cook) {"]);
        lacks(&lifted, &["raw"]);
    }
}

#[test]
fn definition_and_job_cells_lift_by_name() {
    let source = r#"object "Fable Ledger" {
    guid: 0x7E00_0155;
    #[bhav(id = 4300, locals = 0)]
    fn audit() {
        stack_obj = temp[0];
        if stack_obj.object_type == 2 { temp[1] = 1; }
        temp[2] = stack_obj.sub_index;
        if stack_obj.has_function[3] { temp[3] = 1; }
        if job_field(5) > 2 { temp[4] = 1; }
    }
}"#;
    for behavioral in [false, true] {
        let lifted = lift(source, behavioral);
        has(
            &lifted,
            &[
                "if stack_obj.object_type == 2 {",
                "temp[2] = stack_obj.sub_index;",
                "if stack_obj.has_function[3] {",
                "job_field(5) > 2",
            ],
        );
        lacks(&lifted, &["scope("]);
    }
}

#[test]
fn temps_calls_to_trees_with_arguments_lift_in_behavioral_mode() {
    let source = r#"object "Fable Kettle" {
    guid: 0x7E00_0156;
    #[bhav(id = 4300, locals = 0)]
    fn boil(level: i16, heat: i16) -> bool {
        if level > heat { return false; }
        return true;
    }
    #[bhav(id = 4301, args = 4, locals = 0)]
    fn heat() {
        if !boil(2, 4) { return false; }
        temp[0] = 5;
        raw {
            call 4300(temps) -> true: @next, false: false;
            @next:
            temp[1] := 1;
        }
    }
}"#;
    // Strict mode keeps the temps call raw: v2's copies of the temps are other nodes.
    let strict = lift(source, false);
    has(tree(&strict, 4301), &["call 4300(temps)"]);
    // Behavioral mode names the callee; the copies need a frame local, which the header
    // gains (frame storage is not compared there).
    let lifted = lift(source, true);
    let heat = tree(&lifted, 4301);
    has(heat, &["(temp[0], temp[1])", "args = 4, locals = 2,"]);
    lacks(heat, &["call 4300(temps)", "raw"]);
}
