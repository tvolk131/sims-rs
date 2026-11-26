//! `lift --v2` writes the error exit as `panic!();` and a call whose result nothing tests as
//! `let _ = f();` (#475's forms), in strict and in behavioral mode. Each lifted file builds to
//! the same bytes as its older spelling (`return error;`, `if f() {}`), and `raw { }` keeps
//! `.air`'s own syntax. Every object is synthetic (GUIDs in the clean-room 0x7E00_xxxx range).

use sims_antics::compile_v2_source;
use sims_antics::v2::{
    DecompileV2Options, PackResolverConfig, TreeCheck, decompile_iff_to_v2_report, lint_v2_source,
};
use sims_formats::iff::Iff;

fn build(source: &str) -> Vec<u8> {
    compile_v2_source("discard.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{e}\n{source}"))
        .iff_bytes
}

/// Lifts `source`'s object: every tree proven, the text compiles again and lints clean.
fn lift(source: &str, behavioral: bool) -> String {
    let options = DecompileV2Options {
        behavioral,
        ..DecompileV2Options::default()
    };
    let iff = Iff::parse(build(source)).expect("parse IFF");
    let report = decompile_iff_to_v2_report(&iff, &options).expect("lift");
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
    let lints = lint_v2_source(&report.source, "lifted.antics").expect("lint");
    assert!(lints.is_empty(), "{lints:?}\n{}", report.source);
    report.source
}

fn lift_both(source: &str) -> [String; 2] {
    [lift(source, false), lift(source, true)]
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

/// `lifted` written back in the older spellings builds the same bytes.
fn same_bytes_as_older_spelling(lifted: &str) {
    let older = lifted
        .replace("panic!();", "return error;")
        .lines()
        .map(|line| match line.trim_start().strip_prefix("let _ = ") {
            Some(call) => {
                let indent = &line[..line.len() - line.trim_start().len()];
                format!("{indent}if {} {{}}", call.trim_end_matches(';'))
            }
            None => line.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(older, lifted);
    assert_eq!(build(&older), build(lifted), "{older}");
}

#[test]
fn the_error_exit_is_panic() {
    let source = r#"object "Fable Discard Exit" {
    guid: 0x7E00_0175;
    #[bhav(id = 4300)]
    fn check() {
        if temp[0] == 1 {
            return error;
        }
        temp[1] = 2;
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &["if temp[0] == 1 {\n            panic!();\n        }"],
        );
        lacks(&lifted, &["return error"]);
        same_bytes_as_older_spelling(&lifted);
    }
}

#[test]
fn an_unused_call_result_is_let_underscore() {
    let source = r#"object "Fable Discard Calls" {
    guid: 0x7E00_0176;
    #[bhav(id = 4300, label = "Serve")]
    fn serve() {
        if make_food() {}
        if !make_food() {}
        if temp[0] == 1 {}
        temp[1] = 1;
    }
    #[bhav(id = 4301, label = "Make Food")]
    fn make_food() -> bool {
        return temp[2] == 3;
    }
}"#;
    for lifted in lift_both(source) {
        has(&lifted, &["let _ = make_food();"]);
        lacks(&lifted, &["if make_food() {}", "if !make_food() {}"]);
        // A comparison is no call: it keeps its `if`.
        has(&lifted, &["if temp[0] == 1 {}"]);
        same_bytes_as_older_spelling(&lifted);
    }
}

#[test]
fn a_result_call_that_fails_out_is_still_the_question_mark() {
    let source = r#"object "Fable Discard Try" {
    guid: 0x7E00_0177;
    #[bhav(id = 4300, label = "Serve")]
    fn serve() {
        if !make_food() {
            return error;
        }
        if make_food() {
        } else {
            return error;
        }
        temp[1] = 1;
    }
    #[bhav(id = 4301, label = "Make Food")]
    fn make_food() -> Result {
        if temp[2] == 3 {
            return error;
        }
        return true;
    }
}"#;
    for lifted in lift_both(source) {
        has(&lifted, &["make_food()?;"]);
        lacks(&lifted, &["return error", "let _ = make_food();"]);
    }
}

#[test]
fn raw_code_keeps_its_own_syntax() {
    let source = r#"object "Fable Discard Raw" {
    guid: 0x7E00_0178;
    #[bhav(id = 4300, label = "Count", locals = 0)]
    fn count() {
        raw {
            @n0:
            call 4301(temps) -> true: @n1, false: @n1;
            @n1:
            scope(21, 3) := bcon[1];
            expr(temp[0] == 1) -> true: true, false: error;
        }
    }
    #[bhav(id = 4301, label = "Tick", locals = 0)]
    fn tick() -> bool {
        return temp[2] == 3;
    }
}"#;
    for lifted in lift_both(source) {
        // The raw block is `.air`, left as it is; the code around it takes the v2 forms.
        let raw = lifted.find("raw {").expect("a raw block");
        let end = raw + lifted[raw..].find('}').expect("its end");
        let body = &lifted[raw..end];
        has(body, &["scope(21, 3) :="]);
        lacks(body, &["panic!", "let _ ="]);
        has(&lifted[end..], &["panic!();"]);
        same_bytes_as_older_spelling(&lifted);
    }
}
