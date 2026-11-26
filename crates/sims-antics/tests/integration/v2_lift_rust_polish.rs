//! `lift --v2` in plainer Rust: `else if` for an `else` holding one `if` chain, trees called by
//! their own short name (the `_{id}` suffix only on a clash), `Motive::` indices, `Table::entry`
//! constants, braced `use` lines, `!call()` without parentheses, and, in behavioral mode only,
//! `!(a op b)` written as the flipped comparison where the tree stays proven. Each is lifted in
//! strict and in behavioral mode. Every object is synthetic (GUIDs in the clean-room 0x7E00_xxxx
//! range).

use sims_antics::compile_v2_source;
use sims_antics::v2::{
    DecompileV2Options, DecompileV2Report, PackResolverConfig, TreeCheck,
    decompile_iff_to_v2_report, lint_v2_source,
};
use sims_formats::iff::Iff;

fn compile(source: &str) -> Iff {
    let object = compile_v2_source("polish.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{e}\n{source}"));
    Iff::parse(object.iff_bytes).expect("parse IFF")
}

fn report(source: &str, behavioral: bool) -> DecompileV2Report {
    report_allowing(source, behavioral, &[])
}

/// [`report`], with lint findings of the `allowed` rules accepted.
fn report_allowing(source: &str, behavioral: bool, allowed: &[&str]) -> DecompileV2Report {
    let options = DecompileV2Options {
        behavioral,
        ..DecompileV2Options::default()
    };
    let report = decompile_iff_to_v2_report(&compile(source), &options).expect("lift");
    assert!(report.compiled);
    compile(&report.source);
    let lints: Vec<_> = lint_v2_source(&report.source, "lifted.antics")
        .expect("lint")
        .into_iter()
        .filter(|d| !allowed.contains(&d.rule))
        .collect();
    assert!(lints.is_empty(), "{lints:?}\n{}", report.source);
    report
}

/// Lifts `source`'s object: every tree proven, the text compiles again and lints clean.
fn lift(source: &str, behavioral: bool) -> String {
    let report = report(source, behavioral);
    assert!(
        report
            .trees
            .iter()
            .all(|t| matches!(t.check, TreeCheck::Identical | TreeCheck::Equivalent)),
        "{:?}\n{}",
        report.trees,
        report.source
    );
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

#[test]
fn an_else_holding_only_an_if_is_else_if() {
    let source = r#"object "Fable Polish Gate" {
    guid: 0x7E00_0170;
    #[bhav(id = 4300)]
    fn gate() {
        if temp[0] == 1 {
            temp[1] = 1;
        } else {
            if temp[2] > 3 {
                temp[1] = 2;
            }
        }
        temp[3] = 4;
        if temp[0] == 5 {
            temp[1] = 5;
        } else {
            if temp[2] < 6 {
                temp[1] = 6;
            }
            temp[3] = 7;
        }
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &[
                "} else if temp[2] > 3 {",
                "} else {\n            if temp[2] < 6 {",
            ],
        );
        lacks(&lifted, &["} else {\n            if temp[2] > 3 {"]);
    }
}

#[test]
fn trees_are_called_by_their_own_name_and_keep_the_id_on_a_clash() {
    let source = r#"object "Fable Polish Names" {
    guid: 0x7E00_0171;
    sound chime = fwav(0, wav: "fable_chime");
    #[bhav(id = 4300, label = "Have A Meal")]
    fn first() {
        if check_for_food() {
            temp[0] = 1;
        }
        open_a();
        open_b();
        sleep_tree();
        chime_tree();
    }
    #[bhav(id = 4301, label = "Check For Food")]
    fn check_for_food() -> bool {
        return temp[1] == 2;
    }
    #[bhav(id = 4302, label = "Open")]
    fn open_a() {
        temp[2] = 1;
    }
    #[bhav(id = 4303, label = "Open")]
    fn open_b() {
        temp[2] = 2;
    }
    #[bhav(id = 4304, label = "Sleep")]
    fn sleep_tree() {
        temp[3] = 1;
    }
    #[bhav(id = 4305, label = "Fable Chime")]
    fn chime_tree() {
        play_sound(chime, stack_obj);
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &[
                "fn have_a_meal() {",
                "if check_for_food() {",
                "fn check_for_food() {",
                // Two trees share the label: both keep their IDs.
                "fn open_4302() {",
                "fn open_4303() {",
                "if open_4302() {}",
                // A builtin's name, and a name the file already uses (the sound), keep theirs.
                "fn sleep_4304() {",
                "fn fable_chime_4305() {",
                "stack_obj.play_sound(fable_chime);",
            ],
        );
        lacks(&lifted, &["have_a_meal_4300", "check_for_food_4301"]);
    }
}

#[test]
fn motives_constants_and_imports_take_their_paths() {
    let source = r#"use std::flags::Expansions;
use std::flags::ObjectFlags;

object "Fable Polish Paths" {
    guid: 0x7E00_0172;
    bcon 4096 "Tuning" {
        e0 = 5,
        e1 = 10,
    }
    #[bhav(id = 4300)]
    fn feed(sim: Sim) {
        sim.motive[Motive::Hunger] = Tuning::e1;
        sim.motive[2] = 1;
        stack_obj.motive[Motive::Comfort] = 3;
        if inc_lt(&mut sim.motive[Motive::Fun], Tuning::e0) {
            temp[0] = 1;
        }
        raw {
            temp[1] := Tuning.e0;
        }
        if stack_obj.flags flag_test ObjectFlags::InUse {
            temp[2] = 1;
        }
        if world.expansions flag_test Expansions::Magic {
            temp[3] = 1;
        }
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &[
                "sim.motive[Motive::Hunger] = Tuning::e1;",
                "sim.motive[2] = 1;",
                "stack_obj.motive[Motive::Comfort] = 3;",
                "inc_lt(&mut sim.motive[Motive::Fun], Tuning::e0)",
                "use std::flags::{Expansions, ObjectFlags};",
            ],
        );
        lacks(
            &lifted,
            &["sim.motive[7]", "Tuning.e1", "use std::flags::ObjectFlags;"],
        );
    }
}

#[test]
fn a_negated_call_drops_its_parentheses() {
    let source = r#"object "Fable Polish Count" {
    guid: 0x7E00_0173;
    #[bhav(id = 4300)]
    fn count() {
        temp[0] = 0;
        while !inc_lt(&mut temp[0], 6) {
            temp[1] = 1;
        }
    }
}"#;
    for lifted in lift_both(source) {
        lacks(&lifted, &["!(inc_lt("]);
        has(&lifted, &["!inc_lt(&mut temp[0], 6)"]);
    }
}

/// A tree the lift renumbers (its nodes are out of order), so it is proven equivalent rather
/// than identical; behavioral mode writes its negated test flipped.
const RENUMBERED: &str = r#"object "Fable Polish Flip" {
    guid: 0x7E00_0174;
    #[bhav(id = 4300, label = "Pick", locals = 0)]
    fn pick() {
        raw {
            @n0:
            expr(temp[0] > 1) -> true: @n1, false: @n2;
            @n1:
            expr(temp[2] := 3) -> true: true, false: error;
            @n2:
            expr(temp[1] := 2) -> true: @n1, false: error;
        }
    }
}"#;

#[test]
fn behavioral_mode_flips_a_negation_only_where_the_tree_stays_proven() {
    // Strict mode keeps the node: the lint's own finding stands there.
    let strict = report_allowing(RENUMBERED, false, &["negated_comparison"]);
    has(&strict.source, &["if !(temp[0] > 1) {"]);
    let behavioral = report(RENUMBERED, true);
    let check = |r: &DecompileV2Report| {
        r.trees
            .iter()
            .find(|t| t.id == 4300)
            .map(|t| t.check.clone())
    };
    assert!(
        matches!(check(&behavioral), Some(TreeCheck::Equivalent)),
        "{:?}\n{}",
        behavioral.trees,
        behavioral.source
    );
    has(&behavioral.source, &["if temp[0] <= 1 {"]);
    lacks(&behavioral.source, &["!(temp[0] > 1)"]);
}
