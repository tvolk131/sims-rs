//! `lift --v2` writes v2 PR H's spellings: lifecycle `@frame`, in-place `set_to_next`,
//! `return error`, `@slot` lets, `const` branch specialization and `@context(inherit)`.
//! Every object is synthetic (GUIDs in the clean-room 0x7E00_xxxx range).

use sims_antics::compile_v2_source;
use sims_antics::v2::{
    DecompileV2Options, PackResolverConfig, TreeCheck, decompile_iff_to_v2_report, lint_v2_source,
};
use sims_formats::iff::Iff;

fn compile(source: &str) -> Iff {
    let object = compile_v2_source("pr_h.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{e}\n{source}"));
    Iff::parse(object.iff_bytes).expect("parse IFF")
}

/// Lifts `source`'s object: every tree proven, the text compiles again and lints clean.
fn lift(source: &str) -> (String, Vec<sims_antics::v2::DecompiledTree>) {
    let report =
        decompile_iff_to_v2_report(&compile(source), &DecompileV2Options::default()).expect("lift");
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
    (report.source, report.trees)
}

fn has(lifted: &str, spellings: &[&str]) {
    for s in spellings {
        assert!(lifted.contains(s), "missing {s:?} in:\n{lifted}");
    }
}

#[test]
fn lifecycle_trees_stay_blocks_with_their_header() {
    let (lifted, trees) = lift(
        r#"object "Fable Kiln" {
    guid: 0x7E00_0120;
    @id(4200) @frame(args: 4, locals: 1) main { temp[0] = param[1]; }
    @id(4201) init { temp[1] = 2; }
}"#,
    );
    has(
        &lifted,
        &[
            "@id(4200) @frame(args: 4, locals: 1, format: 0x8002, revision: ",
            ") main {",
            // The default header reproduces `init`, so it takes no `@frame`.
            "@id(4201) init {",
            "temp[0] = param[1];",
        ],
    );
    assert!(!lifted.contains("written as a plain tree"), "{lifted}");
    assert!(trees.iter().all(|t| !t.standalone_lifecycle), "{trees:?}");
}

#[test]
fn cursor_steps_and_error_returns_lift_as_builtins() {
    let (lifted, _) = lift(
        r#"object "Fable Sieve" {
    guid: 0x7E00_0121;
    const PROBE = 0x7E00_0121;
    @id(4300) @frame(locals: 0) tree step -> bool {
        if !set_to_next(part, dest: stack_obj) { return false; }
        while set_to_next(guid, PROBE, dest: stack_obj) {
            if temp[0] > 3 { return error; }
            temp[0] += 1;
        }
        return true;
    }
}"#,
    );
    has(
        &lifted,
        &[
            "if !set_to_next(part, dest: stack_obj) {",
            "while set_to_next(guid, ",
            ", dest: stack_obj) {",
            "return error;",
        ],
    );
    assert!(!lifted.contains("raw"), "{lifted}");
}

#[test]
fn builtin_locals_are_pinned_with_slot() {
    let (lifted, _) = lift(
        r#"object "Fable Plumb" {
    guid: 0x7E00_0122;
    @id(4300) @frame(locals: 3) tree pinned {
        raw {
            local[2] := 14;
            find_location(mode: 1, flags: 1, local: 2);
        }
        temp[4] = 2;
    }
}"#,
    );
    has(
        &lifted,
        &[
            "@slot(0) let slot_0: i16;",
            "@slot(2) let slot_2: i16;",
            "slot_2 = 14;",
            "find_location(mode: 1, flags: 1, local: slot_2);",
        ],
    );
    assert!(!lifted.contains("raw"), "{lifted}");
}

#[test]
fn a_tail_if_with_literal_only_differences_is_a_const_binding() {
    let (lifted, _) = lift(
        r#"object "Fable Scale" {
    guid: 0x7E00_0123;
    @id(4300) @frame(locals: 0) tree weigh {
        temp[2] = 1;
        const (LOW, HIGH) = if temp[0] > 3 { (5, 50) } else { (6, 60) };
        temp[1] = LOW;
        temp[3] = HIGH;
        temp[4] = 7;
    }
}"#,
    );
    has(
        &lifted,
        &[
            "const (ARM_",
            " = if temp[0] > 3 { (5, 50) } else { (6, 60) };",
            "temp[4] = 7;",
        ],
    );
    // An `if` whose arms differ in more than literals stays an `if`.
    let (lifted, _) = lift(
        r#"object "Fable Scale" {
    guid: 0x7E00_0124;
    @id(4300) @frame(locals: 0) tree weigh {
        if temp[0] > 3 { temp[1] = 5; } else { temp[2] = 6; }
    }
}"#,
    );
    assert!(!lifted.contains("const"), "{lifted}");
}

#[test]
fn a_tree_both_contexts_call_is_written_inherit() {
    let (lifted, _) = lift(
        r#"object "Fable Bell" {
    guid: 0x7E00_0125;
    @id(4300) @frame(locals: 0) @context(inherit) tree ring "Ring" {
        me.person[3] = 1;
        me.motive[2] = 4;
        temp[0] = stack_obj.attr[1];
    }
    @id(4301) @frame(locals: 0) tree from_object "From Object" { ring(); }
    @id(4302) @frame(locals: 0) tree from_sim "From Sim" (sim: Sim) {
        sim.motive[1] = 2;
        ring();
    }
}"#,
    );
    has(
        &lifted,
        &[
            "@context(inherit) tree ring_4300 \"Ring\" {",
            "me.person[3] = 1;",
            "me.motive[2] = 4;",
            "temp[0] = stack_obj.attr[1];",
        ],
    );
}

#[test]
fn argless_calls_between_sim_trees_name_the_sim() {
    let (lifted, _) = lift(
        r#"object "Fable Lute" {
    guid: 0x7E00_0126;
    @id(4300) @frame(locals: 0) tree tune "Tune" (sim: Sim) {
        sim.motive[1] = 2;
        sim.motive[2] = 3;
    }
    @id(4301) @frame(locals: 0) tree play "Play" (sim: Sim) {
        sim.motive[3] = 1;
        tune(sim);
    }
}"#,
    );
    has(&lifted, &["tune_4300(sim)"]);
    assert!(!lifted.contains("raw"), "{lifted}");
}
