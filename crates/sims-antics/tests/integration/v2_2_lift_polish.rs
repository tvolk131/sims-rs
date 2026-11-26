//! `lift --v2` (v2.2 polish) writes motives and skills by name (`sim.motive.hunger`,
//! `sim.skill.cooking`), typed flags by method (`.set(..)`, `.clear(..)`, `.contains(..)`),
//! and in behavioral mode locals without `#[slot(N)]` and arguments by name, wherever the
//! object builds the same bytes. Every object is synthetic (GUIDs in the clean-room
//! 0x7E00_xxxx range).

use sims_antics::compile_v2_source;
use sims_antics::v2::{
    DecompileV2Options, PackResolverConfig, TreeCheck, decompile_iff_to_v2_report, lint_v2_source,
};
use sims_formats::iff::Iff;

fn build(source: &str) -> Vec<u8> {
    compile_v2_source("polish.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{e}\n{source}"))
        .iff_bytes
}

/// Lifts `source`'s object: every tree proven, the text builds the same bytes as the
/// original and lints clean.
fn lift(source: &str, behavioral: bool) -> String {
    let options = DecompileV2Options {
        behavioral,
        ..DecompileV2Options::default()
    };
    let original = build(source);
    let iff = Iff::parse(original).expect("parse IFF");
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

/// `lifted` with each `(new, old)` pair written back in the older spelling builds the same
/// bytes.
fn same_bytes_as(lifted: &str, older: &[(&str, &str)]) {
    let mut text = lifted.to_owned();
    for (new, old) in older {
        assert!(text.contains(new), "missing {new:?} in:\n{lifted}");
        text = text.replace(new, old);
    }
    assert_eq!(build(&text), build(lifted), "{text}");
}

#[test]
fn motives_and_skills_are_written_by_name() {
    let source = r#"object "Fable Polish Cells" {
    guid: 0x7E00_0179;
    #[bhav(id = 4300, label = "Read", locals = 0)]
    fn read(sim: Sim) {
        raw {
            temp[0] := my.motive[7];
            temp[1] := my.person[10];
            my.motive[5] += 2;
            temp[2] := my.motive[1];
            temp[3] := my.person[2];
        }
    }
}"#;
    for behavioral in [false, true] {
        let lifted = lift(source, behavioral);
        has(
            &lifted,
            &[
                "temp[0] = sim.motive.hunger;",
                "temp[1] = sim.skill.cooking;",
                "sim.motive.energy += 2;",
                // An index no motive name covers stays a number.
                "temp[2] = sim.motive[1];",
                "temp[3] = sim.personality.nice;",
            ],
        );
        lacks(&lifted, &["Motive::", "person[10]"]);
        same_bytes_as(
            &lifted,
            &[
                ("sim.motive.hunger", "sim.motive[Motive::Hunger]"),
                ("sim.motive.energy", "sim.motive[Motive::Energy]"),
                ("sim.skill.cooking", "sim.person[10]"),
            ],
        );
    }
}

#[test]
fn typed_flags_are_written_by_method() {
    let source = r#"use std::flags::ObjectFlags;

object "Fable Polish Flags" {
    guid: 0x7E00_017A;
    #[bhav(id = 4300, label = "Mark", locals = 0)]
    fn mark() {
        self.flags.set(ObjectFlags::InTheWay);
        self.flags.clear(ObjectFlags::NotSolid);
        if self.flags.contains(ObjectFlags::InUse) {
            temp[0] = 1;
        }
        if !self.flags.contains(ObjectFlags::Passable) {
            temp[1] = 1;
        }
        temp[2] flag_set 3;
        if temp[3] flag_test 2 {
            temp[4] = 1;
        }
    }
}"#;
    for behavioral in [false, true] {
        let lifted = lift(source, behavioral);
        has(
            &lifted,
            &[
                "self.flags.set(ObjectFlags::InTheWay);",
                "self.flags.clear(ObjectFlags::NotSolid);",
                "if self.flags.contains(ObjectFlags::InUse) {",
                "if !self.flags.contains(ObjectFlags::Passable) {",
                // A bit number on an untyped cell keeps the operator.
                "temp[2] flag_set 3;",
                "temp[3] flag_test 2",
            ],
        );
        lacks(
            &lifted,
            &[
                "flag_set ObjectFlags",
                "flag_clear",
                "flag_test ObjectFlags",
            ],
        );
        same_bytes_as(
            &lifted,
            &[
                (
                    "self.flags.set(ObjectFlags::InTheWay);",
                    "self.flags flag_set ObjectFlags::InTheWay;",
                ),
                (
                    "self.flags.clear(ObjectFlags::NotSolid);",
                    "self.flags flag_clear ObjectFlags::NotSolid;",
                ),
                (
                    "self.flags.contains(ObjectFlags::InUse)",
                    "self.flags flag_test ObjectFlags::InUse",
                ),
                (
                    "!self.flags.contains(ObjectFlags::Passable)",
                    "!(self.flags flag_test ObjectFlags::Passable)",
                ),
            ],
        );
    }
}

#[test]
fn behavioral_arguments_are_named_where_only_read() {
    let source = r#"object "Fable Polish Args" {
    guid: 0x7E00_017B;
    #[bhav(id = 4300, label = "Pick", args = 2, locals = 0)]
    fn pick() {
        raw {
            temp[0] := param[1];
            temp[1] := param[1];
            temp[1] += 2;
        }
    }
    #[bhav(id = 4301, label = "Bump", args = 1, locals = 0)]
    fn bump() {
        raw {
            param[0] += 1;
            temp[0] := param[0];
        }
    }
}"#;
    let strict = lift(source, false);
    has(
        &strict,
        &["fn pick() {", "temp[0] = param[1];", "param[0] += 1;"],
    );
    let behavioral = lift(source, true);
    has(
        &behavioral,
        &[
            "fn pick(_p0: i16, p1: i16) {",
            "temp[0] = p1;",
            // An argument the tree writes keeps its number.
            "fn bump() {",
            "param[0] += 1;",
            "temp[0] = param[0];",
        ],
    );
    lacks(&behavioral, &["param[1]"]);
    same_bytes_as(
        &behavioral,
        &[
            ("fn pick(_p0: i16, p1: i16) {", "fn pick() {"),
            ("p1", "param[1]"),
        ],
    );
}

#[test]
fn behavioral_locals_lose_their_slot_where_the_bytes_stay() {
    let source = r#"object "Fable Polish Locals" {
    guid: 0x7E00_017C;
    #[bhav(id = 4300, label = "Count", locals = 1)]
    fn count() {
        #[slot(0)] let slot_0: i16;
        if temp[1] == 2 {
            slot_0 = 3;
        } else {
            slot_0 = 4;
        }
        if temp[2] > slot_0 {
            temp[0] = 1;
        }
    }
}"#;
    let strict = lift(source, false);
    let behavioral = lift(source, true);
    for lifted in [&strict, &behavioral] {
        has(lifted, &["let slot_0: i16;"]);
    }
    has(&strict, &["#[slot(0)] let slot_0: i16;"]);
    lacks(&behavioral, &["#[slot("]);
    same_bytes_as(
        &behavioral,
        &[("let slot_0: i16;", "#[slot(0)] let slot_0: i16;")],
    );
}

#[test]
fn raw_code_keeps_its_own_syntax() {
    let source = r#"object "Fable Polish Raw" {
    guid: 0x7E00_017D;
    #[bhav(id = 4300, label = "Keep", args = 1, locals = 1)]
    fn keep(sim: Sim) {
        raw {
            @n0:
            call 4301(temps) -> true: @n1, false: @n1;
            @n1:
            scope(21, 3) := my.motive[7];
            local[0] := param[0];
            scope(21, 4) := local[0];
            temp[0] := param[0];
            expr(temp[0] == 1) -> true: true, false: error;
        }
    }
    #[bhav(id = 4301, label = "Tick", locals = 0)]
    fn tick() -> bool {
        return temp[2] == 3;
    }
}"#;
    for (lifted, p) in lift_both(source).iter().zip(["param[0]", "p0"]) {
        // The raw blocks are `.air`, left as they are.
        has(
            lifted,
            &[
                "raw {\n            scope(21, 3) := my.motive[7];\n        }",
                "raw {\n            scope(21, 4) := local[0];\n        }",
                // Raw code reads the local's word by number: its pin stays.
                "#[slot(0)] let slot_0: i16;",
                &format!("slot_0 = {p};"),
                &format!("temp[0] = {p};"),
            ],
        );
    }
}

#[test]
fn behavioral_arguments_a_primitive_writes_keep_their_numbers() {
    // Old Relationship stores the value in its `param:` argument; `random_into` writes its
    // first argument; animation events land in argument 0.
    let source = r#"object "Fable Polish Writers" {
    guid: 0x7E00_017E;
    #[bhav(id = 4300, label = "Review", args = 1, locals = 0)]
    fn review(sim: Sim) {
        raw {
            if test_sim_interacting_with() {
                old_relationship(write: false, column: 0, param: 0, dir: 0, flags: 0);
            }
            temp[0] := param[0];
        }
    }
    #[bhav(id = 4301, label = "Roll", args = 1, locals = 0)]
    fn roll() {
        random_into(param[0], 10);
        temp[0] = param[0];
    }
    #[bhav(id = 4302, label = "Wave", args = 1, locals = 0)]
    fn wave(sim: Sim) {
        animate(clip: 3, table: 1, flags: 0, events: 2);
        temp[0] = param[0];
    }
}"#;
    let lifted = lift(source, true);
    lacks(&lifted, &["p0: i16", "= p0;"]);
    has(&lifted, &["temp[0] = param[0];"]);
}

#[test]
fn behavioral_sleeping_trees_keep_their_argument_numbers() {
    // Sleep counts down in argument 0. The build restores it only for a tree that reads it,
    // and that tree lifts with the restore written out (`param[0] = slot_0;`).
    let source = r#"object "Fable Polish Sleepers" {
    guid: 0x7E00_017F;
    #[bhav(id = 4300, label = "Probe", args = 2, locals = 1)]
    fn probe(sim: Sim) {
        sleep(3);
        temp[0] = param[1];
    }
    #[bhav(id = 4301, label = "Nap", args = 2, locals = 1)]
    fn nap(sim: Sim) {
        sleep(3);
        temp[0] = param[0];
        temp[1] = param[1];
    }
}"#;
    let lifted = lift(source, true);
    has(
        &lifted,
        &[
            "temp[0] = param[1];",
            "param[0] = slot_0;",
            "temp[1] = param[1];",
        ],
    );
    lacks(&lifted, &["p0: i16", "p1: i16"]);
}
