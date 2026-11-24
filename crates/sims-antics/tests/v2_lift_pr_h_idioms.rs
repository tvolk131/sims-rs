//! `lift --v2` writes v2 PR H's spellings: lifecycle `@frame`, in-place `set_to_next`,
//! `return error`, `@slot` lets, `const` branch specialization and `@context(inherit)`.
//! Every object is synthetic (GUIDs in the clean-room 0x7E00_xxxx range).

use sims_antics::compile_v2_source;
use sims_antics::v2::{
    DecompileV2Options, PackResolverConfig, TreeCheck, decompile_iff_to_v2_report, lint_v2_source,
};
use sims_formats::iff::{ChunkData, Iff};

fn compile(source: &str) -> Iff {
    let object = compile_v2_source("pr_h.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{e}\n{source}"));
    Iff::parse(object.iff_bytes).expect("parse IFF")
}

/// Lifts `source`'s object: every tree proven, the text compiles again and lints clean.
fn lift(source: &str) -> (String, Vec<sims_antics::v2::DecompiledTree>) {
    lift_iff(&compile(source))
}

fn lift_iff(iff: &Iff) -> (String, Vec<sims_antics::v2::DecompiledTree>) {
    let report = decompile_iff_to_v2_report(iff, &DecompileV2Options::default()).expect("lift");
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

/// `iff` with BHAV `id` relabelled `label`.
fn relabel(iff: &Iff, id: u16, label: &str) -> Iff {
    let chunks: Vec<ChunkData> = iff
        .chunks
        .iter()
        .filter(|c| &c.tag != b"rsmp")
        .map(|c| ChunkData {
            tag: c.tag,
            id: c.id,
            flags: c.flags,
            label: if &c.tag == b"BHAV" && c.id == id {
                label.to_owned()
            } else {
                c.label.clone()
            },
            data: iff.data(c).to_vec(),
        })
        .collect();
    Iff::parse(sims_formats::iff::write(&chunks).expect("write IFF")).expect("parse IFF")
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

/// `iff`'s lift report, every tree checked.
fn report(iff: &Iff) -> sims_antics::v2::DecompileV2Report {
    let report = decompile_iff_to_v2_report(iff, &DecompileV2Options::default()).expect("lift");
    assert!(report.compiled);
    compile(&report.source);
    report
}

fn check_of(report: &sims_antics::v2::DecompileV2Report, id: u16) -> &TreeCheck {
    &report
        .trees
        .iter()
        .find(|t| t.id == id)
        .expect("tree")
        .check
}

#[test]
fn a_lifecycle_tree_the_compiler_would_rename_is_not_proven() {
    // Run Tree By Name finds a tree by its label, and v2 names a block after its kind;
    // written as a plain tree instead, the object's `main` entry would run a compiler-made
    // tree. The block keeps the entry and is left unproven.
    let iff = relabel(
        &compile(
            r#"object "Fable Clock" {
    guid: 0x7E00_0127;
    @id(4200) main { temp[0] = 1; }
    @id(4201) init { temp[1] = 2; }
}"#,
        ),
        4200,
        "Tick",
    );
    let report = report(&iff);
    assert!(
        report.source.contains("@id(4200) main {"),
        "{}",
        report.source
    );
    assert!(
        matches!(check_of(&report, 4200), TreeCheck::Inconclusive(why) if why.contains("labelled")),
        "{:?}",
        report.trees
    );
    assert!(
        matches!(check_of(&report, 4201), TreeCheck::Identical),
        "{:?}",
        report.trees
    );
}

#[test]
fn labels_keep_quotes_spaces_and_western_letters() {
    let source = r#"object "Fable Clock" {
    guid: 0x7E00_012D;
    @id(4300) @frame(locals: 0) tree a "A" { temp[0] = 1; }
}"#;
    for label in ["Caf\u{e9} \"Test\"", "Grill ", "   "] {
        let (lifted, _) = lift_iff(&relabel(&compile(source), 4300, label));
        let written = label.replace('"', "\\\"");
        assert!(
            lifted.contains(&format!("\"{written}\" {{")),
            "{label:?}:\n{lifted}"
        );
    }
    // A label the compiler cannot store (64 bytes) is cut to fit; the lift goes on.
    let long = format!("T{}", " ".repeat(62)) + "X";
    let report = report(&relabel(&compile(source), 4300, &long));
    assert!(
        matches!(check_of(&report, 4300), TreeCheck::Inconclusive(why) if why.contains("labelled")),
        "{:?}",
        report.trees
    );
}

#[test]
fn a_lifecycle_revision_is_kept() {
    let (lifted, _) = lift(
        r#"object "Fable Ledger" {
    guid: 0x7E00_0128;
    @id(4200) @frame(revision: 7) main { temp[0] = 1; }
}"#,
    );
    has(&lifted, &["revision: 7) main {"]);
}

#[test]
fn signed_literals_lift_as_const_bindings_of_one_sign() {
    let (lifted, _) = lift(
        r#"object "Fable Gauge" {
    guid: 0x7E00_0129;
    @id(4300) @frame(locals: 0) tree dip {
        if temp[0] > 3 { temp[1] = -5; } else { temp[1] = -6; }
        temp[4] = 7;
    }
}"#,
    );
    has(&lifted, &["= if temp[0] > 3 { -5 } else { -6 };"]);
    // Opposite signs differ by a token: the `if` stays.
    let (lifted, _) = lift(
        r#"object "Fable Gauge" {
    guid: 0x7E00_012A;
    @id(4300) @frame(locals: 0) tree dip {
        if temp[0] > 3 { temp[1] = -5; } else { temp[1] = 6; }
        temp[4] = 7;
    }
}"#,
    );
    assert!(!lifted.contains("const"), "{lifted}");
}

#[test]
fn an_inherit_tree_reads_the_stack_objects_person_kind() {
    let (lifted, _) = lift(
        r#"object "Fable Gong" {
    guid: 0x7E00_012B;
    @id(4300) @frame(locals: 0) @context(inherit) tree ring "Ring" {
        me.person[3] = 1;
        temp[0] = stack_obj.person[65];
    }
    @id(4301) @frame(locals: 0) tree from_object "From Object" { ring(); }
    @id(4302) @frame(locals: 0) tree from_sim "From Sim" (sim: Sim) {
        sim.motive[1] = 2;
        ring();
    }
}"#,
    );
    has(&lifted, &["temp[0] = stack_obj.person[65];"]);
    assert!(!lifted.contains("raw {"), "{lifted}");
}

#[test]
fn a_beside_step_names_its_local() {
    let (lifted, _) = lift(
        r#"object "Fable Hinge" {
    guid: 0x7E00_012C;
    @id(4300) @frame(locals: 3) tree near -> bool {
        @slot(2) let side: i16 = 1;
        if !set_to_next(beside, dest: stack_obj, local: side) { return false; }
        return true;
    }
}"#,
    );
    has(
        &lifted,
        &[
            "@slot(2) let slot_2: i16;",
            "set_to_next(beside, dest: stack_obj, local: slot_2)",
        ],
    );
    assert!(!lifted.contains("raw {"), "{lifted}");
}

#[test]
fn a_tree_run_by_name_keeps_its_name_table() {
    // Run Tree By Name reads the name from this file's `STR#` 303: the lift writes the
    // table, and a renamed tree the name finds leaves the caller unproven.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_012E;
    strings 303 "Tree names" {
        pump = "Pump",
    }
    @id(4300) @frame(locals: 0) tree pump "Pump" { temp[0] = 1; }
    @id(4301) @frame(locals: 0) tree caller "Caller" {
        raw { run_tree_by_name(table: 303, index: 1, mode: 0); }
    }
}"#;
    let (lifted, _) = lift(source);
    has(&lifted, &["strings 303", "= \"Pump\","]);
    // `main` labelled `Tick` is rebuilt as `Main`: a name `Tick` no longer finds it.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_012F;
    strings 303 "Tree names" {
        tick = "Tick",
    }
    @id(4200) main { temp[0] = 1; }
    @id(4301) @frame(locals: 0) tree caller "Caller" {
        raw { run_tree_by_name(table: 303, index: 1, mode: 0); }
    }
}"#;
    let report = report(&relabel(&compile(source), 4200, "Tick"));
    assert!(
        matches!(check_of(&report, 4301), TreeCheck::Inconclusive(why) if why.contains("by name")),
        "{:?}",
        report.trees
    );
}
