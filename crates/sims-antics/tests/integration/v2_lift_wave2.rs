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
    // Behavioral mode names the callee. Its arguments are already in their temps, so the
    // call stages nothing and the frame gains no local.
    let lifted = lift(source, true);
    let heat = tree(&lifted, 4301);
    has(heat, &["(temp[0], temp[1])", "args = 4, locals = 0,"]);
    lacks(heat, &["call 4300(temps)", "raw"]);
}

#[test]
fn set_to_next_into_a_local_declares_the_local() {
    // The destination is the only reference to the local.
    let source = r#"object "Fable Compass" {
    guid: 0x7E00_0157;
    #[bhav(id = 4300, locals = 1)]
    fn scan() {
        #[slot(0)] let cursor: i16;
        if set_to_next(person, dest: cursor) {
            temp[0] = 1;
        }
    }
}"#;
    for behavioral in [false, true] {
        let lifted = lift(source, behavioral);
        has(
            &lifted,
            &[
                "#[slot(0)] let slot_0: i16;",
                "set_to_next(person, dest: slot_0)",
            ],
        );
        lacks(&lifted, &["raw"]);
    }
}

#[test]
fn function_cells_v2_cannot_name_stay_scoped() {
    // v2 names functions 0..=255; cell 256 keeps its scope spelling.
    let source = r#"object "Fable Sextant" {
    guid: 0x7E00_0158;
    #[bhav(id = 4300, locals = 0)]
    fn probe() {
        stack_obj = temp[0];
        if stack_obj.has_function[255] { temp[2] = 1; }
        raw {
            if scope(35, 256) != 0 { temp[1] := 1; }
        }
        temp[3] = 1;
    }
}"#;
    for behavioral in [false, true] {
        let lifted = lift(source, behavioral);
        has(&lifted, &["stack_obj.has_function[255]"]);
        lacks(&lifted, &["has_function[256]"]);
    }
}

#[test]
fn shared_tails_keep_the_comments_before_them() {
    let source = r#"object "Fable Lantern" {
    guid: 0x7E00_0159;
    #[bhav(id = 4300, locals = 0)]
    fn trap() {
        if temp[0] == 1 {
            temp[1] = 1;
            loop { breakpoint(1); }
        } else {
            temp[1] = 2;
            loop { breakpoint(1); }
        }
    }
    #[bhav(id = 4301, locals = 0)]
    fn halt() {
        if temp[0] == 1 {
            loop { breakpoint(1); }
        } else {
            loop { breakpoint(1); }
        }
    }
}"#;
    for behavioral in [false, true] {
        // `lift` checks the text lints clean: each loop keeps its `allow`.
        let lifted = lift(source, behavioral);
        let halt = tree(&lifted, 4301);
        assert_eq!(halt.matches("breakpoint(1);").count(), 2, "{halt}");
    }
    let lifted = lift(source, true);
    let trap = tree(&lifted, 4300);
    assert_eq!(trap.matches("breakpoint(1);").count(), 1, "{trap}");
}

#[test]
fn function_cells_tested_above_zero_stay_scoped_and_cost_nothing_else() {
    // `x.has_function[N]` is the test `!= 0`; `> 0` differs for a negative cell, so it
    // keeps its scope spelling, and the rest of the tree lifts as before.
    let source = r#"object "Fable Hearth" {
    guid: 0x7E00_015A;
    #[bhav(id = 4300, args = 1, locals = 3)]
    fn go() {
        raw {
            local[0] := 0;
            local[1] := 1000;
            stack_obj := 0;
            param[0] := 5;
            while set_to_next(6, guid: 0x00000000, dest: stack_obj) {
                if scope(35, 19) > 0 {
                    get_distance_to(temp[0], from: my.data[11]);
                    if temp[0] < local[1] {
                        local[0] := stack_obj;
                        local[1] := temp[0];
                    }
                }
            }
            if local[0] == 0 {
                return true;
            }
            stack_obj := local[0];
            temp[3] := 1;
            return true;
        }
    }
}"#;
    for behavioral in [false, true] {
        let lifted = lift(source, behavioral);
        let go = tree(&lifted, 4300);
        lacks(go, &["has_function"]);
        assert!(go.matches("raw {").count() <= 1, "{go}");
    }
}

#[test]
fn for_loop_cursors_are_not_builtin_locals() {
    let source = r#"object "Fable Sieve" {
    guid: 0x7E00_015B;
    #[bhav(id = 4300, locals = 2)]
    fn scan() {
        for p in lot.objects() {
            raw {
                temp[2] := scope(21, 30);
            }
            temp[3] = 7;
        }
        temp[1] = self.slot[3];
    }
}"#;
    // Strict mode keeps the header; behavioral mode may give the cursor its own words
    // instead of dropping the pins on the saved object's.
    let strict = lift(source, false);
    has(
        tree(&strict, 4300),
        &["temp[1] = self.slot[3];", "locals = 2,"],
    );
    let behavioral = lift(source, true);
    has(tree(&behavioral, 4300), &["temp[1] = self.slot[3];"]);
}

#[test]
fn a_raw_self_loop_keeps_its_comment_without_an_allow() {
    // Lint does not look inside a raw body, so an allow over one would be stale.
    let source = r#"object "Fable Kiln" {
    guid: 0x7E00_015C;
    #[bhav(id = 4300, locals = 0)]
    fn trap() {
        if temp[0] == 1 {
            temp[1] = 1;
            return true;
        }
        loop {
            raw {
                breakpoint(invalid_scope: 0xffff, index: -1) pad 0xff;
            }
        }
    }
}"#;
    // `lift` checks the text lints clean. Strict mode keeps the whole tree raw.
    let lifted = lift(source, true);
    let trap = tree(&lifted, 4300);
    assert!(trap.contains("loops on itself\n"), "{trap}");
    assert!(trap.contains("loop {\n"), "{trap}");
    lacks(trap, &["allow(unbounded_loop_without_yield)"]);
}
