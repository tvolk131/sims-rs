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

/// `iff`'s chunks (without its resource map), edited by `edit`, plus `extra` at the end.
fn rebuild(iff: &Iff, mut edit: impl FnMut(&mut ChunkData), extra: Vec<ChunkData>) -> Iff {
    let mut chunks: Vec<ChunkData> = iff
        .chunks
        .iter()
        .filter(|c| &c.tag != b"rsmp")
        .map(|c| ChunkData {
            tag: c.tag,
            id: c.id,
            flags: c.flags,
            label: c.label.clone(),
            data: iff.data(c).to_vec(),
        })
        .collect();
    chunks.iter_mut().for_each(&mut edit);
    chunks.extend(extra);
    Iff::parse(sims_formats::iff::write(&chunks).expect("write IFF")).expect("parse IFF")
}

fn proven(check: &TreeCheck) -> bool {
    matches!(check, TreeCheck::Identical | TreeCheck::Equivalent)
}

/// The rebuilt object's file.
fn rebuilt(report: &sims_antics::v2::DecompileV2Report) -> Iff {
    compile(&report.source)
}

#[test]
fn a_tree_run_by_name_is_proven_only_with_the_tree_it_runs() {
    // `Pump` reads a string table the lift cannot reproduce (its chunk is marked
    // deleted), so it is not proven; neither is a caller that runs it by name, nor
    // primitives that look up a fixed name (Go To Relative Position, Go To Routing
    // Slot, Reach) finding it.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_0130;
    strings 300 "Lines" {
        a = "A",
    }
    strings 303 "Tree names" {
        pump = "Pump",
    }
    @id(4300) @frame(locals: 0) tree pump "Pump" {
        raw { op 0x0015 (0x2c, 0x01, 0, 0, 0, 0, 0, 0); }
    }
    @id(4301) @frame(locals: 0) tree caller "Caller" {
        raw { run_tree_by_name(table: 303, index: 1, mode: 0); }
    }
    @id(4302) @frame(locals: 0) tree ask "route ask to move animation" {
        raw { op 0x0015 (0x2c, 0x01, 0, 0, 0, 0, 0, 0); }
    }
    @id(4303) @frame(locals: 0) tree route "Route" {
        raw { op 0x002d (0, 0, 0, 0, 0, 0, 0, 0); }
    }
    @id(4304) @frame(locals: 0) tree grab "CT - Grab" {
        raw { op 0x0015 (0x2c, 0x01, 0, 0, 0, 0, 0, 0); }
    }
    @id(4305) @frame(locals: 0) tree reach "Reach" {
        raw { op 0x002f (0, 0, 0, 0, 0, 0, 0, 0); }
    }
    @id(4306) @frame(locals: 0) tree relative "Relative" {
        raw { op 0x001b (0, 0, 0, 0, 0, 0, 0, 0); }
    }
}"#;
    let original = rebuild(
        &compile(source),
        |c| {
            if &c.tag == b"STR#" && c.id == 300 {
                c.flags |= sims_formats::iff::DELETED;
            }
        },
        Vec::new(),
    );
    let report = report(&original);
    for id in [4300, 4302, 4304] {
        assert!(!proven(check_of(&report, id)), "{:?}", report.trees);
    }
    for id in [4301, 4303, 4305, 4306] {
        assert!(
            matches!(check_of(&report, id), TreeCheck::Inconclusive(why) if why.contains("not proven")),
            "{id}: {:?}",
            report.trees
        );
    }
}

#[test]
fn a_lookup_tries_exact_names_before_ignoring_case() {
    // Labelled `mAiN`, the original's `main` is found for `Main` only after every file's
    // exact pass (a global `Main` would win); rebuilt as `Main` it wins at once.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_0131;
    strings 303 "Tree names" {
        main = "Main",
    }
    @id(4200) main { temp[0] = 1; }
    @id(4301) @frame(locals: 0) tree caller "Caller" {
        raw { run_tree_by_name(table: 303, index: 1, mode: 0); }
    }
}"#;
    let report = report(&relabel(&compile(source), 4200, "mAiN"));
    assert!(
        matches!(check_of(&report, 4301), TreeCheck::Inconclusive(why) if why.contains("by name")),
        "{:?}",
        report.trees
    );
}

#[test]
fn a_deleted_last_chunk_names_its_tree() {
    // The VM keeps the last BHAV chunk of an ID, deleted or not, so this file's tree
    // 4300 is labelled `Gone`: the caller is proven only if the rebuilt file agrees.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_0132;
    strings 303 "Tree names" {
        pump = "Pump",
    }
    @id(4300) @frame(locals: 0) tree pump "Pump" { temp[0] = 1; }
    @id(4301) @frame(locals: 0) tree caller "Caller" {
        raw { run_tree_by_name(table: 303, index: 1, mode: 0); }
    }
}"#;
    let iff = compile(source);
    let pump = iff.all(b"BHAV").find(|c| c.id == 4300).expect("pump");
    let gone = ChunkData {
        tag: *b"BHAV",
        id: 4300,
        flags: sims_formats::iff::DELETED,
        label: "Gone".to_owned(),
        data: iff.data(pump).to_vec(),
    };
    let original = rebuild(&iff, |_| {}, vec![gone]);
    let report = report(&original);
    let rebuilt = rebuilt(&report);
    let label = rebuilt
        .chunks
        .iter()
        .rev()
        .find(|c| &c.tag == b"BHAV" && c.id == 4300)
        .map(|c| c.label.clone());
    // The lift keeps the VM's chunk, so the name finds nothing in either file.
    assert_eq!(label.as_deref(), Some("Gone"));
    assert!(proven(check_of(&report, 4301)), "{:?}", report.trees);
}

/// The trees an object's `init` and `main` callbacks run, the VM's way: its OBJf
/// table when it has one, else the OBJD's own words.
fn init_and_main(iff: &Iff) -> (u16, u16) {
    let objd = iff.chunks.iter().find(|c| &c.tag == b"OBJD").expect("OBJD");
    if let Some(table) = iff.get(b"OBJf", objd.id) {
        let entries = sims_formats::execution::ObjectFunctions::parse(iff.data(table))
            .expect("OBJf")
            .entries;
        let action = |i: usize| entries.get(i).map_or(0, |f| f.action);
        return (action(0), action(1));
    }
    let data = iff.data(objd);
    let word = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]);
    (word(0x56), word(0x0a))
}

/// An OBJf table with `entries` (condition, action).
fn objf_table(entries: &[(u16, u16)]) -> Vec<u8> {
    let mut table = Vec::new();
    table.extend(0u32.to_le_bytes());
    table.extend(0u32.to_le_bytes());
    table.extend(b"fJBO");
    table.extend((entries.len() as u32).to_le_bytes());
    for &(condition, action) in entries {
        table.extend(condition.to_le_bytes());
        table.extend(action.to_le_bytes());
    }
    table
}

/// `iff` with its OBJD's callback table replaced by `entries` (OBJf and OBJD words).
fn with_callbacks(iff: &Iff, entries: &[(u16, u16)]) -> Iff {
    let objd = iff
        .chunks
        .iter()
        .find(|c| &c.tag == b"OBJD")
        .expect("OBJD")
        .id;
    let had_table = iff.get(b"OBJf", objd).is_some();
    let extra = if had_table {
        Vec::new()
    } else {
        vec![ChunkData {
            tag: *b"OBJf",
            id: objd,
            flags: 0,
            label: String::new(),
            data: objf_table(entries),
        }]
    };
    rebuild(
        iff,
        |c| {
            if &c.tag == b"OBJf" && c.id == objd {
                c.data = objf_table(entries);
            }
            if &c.tag == b"OBJD" && c.id == objd {
                let word = |n: usize| entries.get(n).map_or(0, |&(_, a)| a).to_le_bytes();
                c.data[0x56..0x58].copy_from_slice(&word(0));
                c.data[0x0a..0x0c].copy_from_slice(&word(1));
            }
        },
        extra,
    )
}

#[test]
fn a_callback_tree_is_proven_only_where_the_rebuilt_object_runs_it() {
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_0133;
    @id(4200) main { temp[0] = 1; }
    @id(4300) @frame(locals: 0) tree pump "Pump" { temp[0] = 2; }
}"#;
    let iff = compile(source);
    // One tree as both `init` and `main`.
    let shared = with_callbacks(&iff, &[(0, 4200), (0, 4200)]);
    assert_eq!(init_and_main(&shared), (4200, 4200));
    let lifted = report(&shared);
    if proven(check_of(&lifted, 4200)) {
        assert_eq!(init_and_main(&rebuilt(&lifted)), (4200, 4200));
    }
    // `Pump` as `main`.
    let overridden = with_callbacks(&iff, &[(0, 4096), (0, 4300)]);
    assert_eq!(init_and_main(&overridden).1, 4300);
    let lifted = report(&overridden);
    for id in [4200, 4300] {
        if proven(check_of(&lifted, id)) {
            assert_eq!(
                init_and_main(&rebuilt(&lifted)).1,
                4300,
                "{id}: {:?}",
                lifted.trees
            );
        }
    }
}

#[test]
fn a_functional_call_is_proven_only_with_the_callback_it_runs() {
    // Function 0 runs the stack object's callback 18 (`functional prep`'s action). The
    // lift writes the callback trees as plain trees, so the rebuilt object registers
    // none: the action and the call are not proven.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_0134;
    functional prep -> i16 {
        score: 10,
        guard(sim) => true,
        run(sim) { return 3; }
    }
    @id(4301) @frame(locals: 0) tree caller "Caller" {
        raw { op 0x0014 (0, 0, 0, 0, 0, 0, 0, 0); }
    }
}"#;
    let report = report(&compile(source));
    assert!(
        matches!(check_of(&report, 4301), TreeCheck::Inconclusive(why) if why.contains("callback")),
        "{:?}\n{}",
        report.trees,
        report.source
    );
}

#[test]
fn a_semi_global_link_is_kept_or_no_tree_is_proven() {
    // Semi-global calls and constants resolve through `GLOB`, and so do the global trees
    // a tree calls (they run with this object as their owner): with another link, no
    // tree of the object is proven, whatever it calls.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_0135;
    @id(4301) @frame(locals: 0) tree caller "Caller" {
        raw { op 0x2000 (0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff); }
    }
    @id(4302) @frame(locals: 0) tree plain "Plain" { temp[0] = 1; }
}"#;
    let glob = |data: &[u8]| ChunkData {
        tag: *b"GLOB",
        id: 128,
        flags: 0,
        label: String::new(),
        data: data.to_vec(),
    };
    let original = rebuild(&compile(source), |_| {}, vec![glob(b"FableGlobals\0")]);
    let linked = report(&original);
    // The lift writes no `GLOB`, so 8192 resolves nowhere in the rebuilt object.
    let link = |iff: &Iff| iff.all(b"GLOB").next().map(|c| iff.data(c).to_vec());
    assert_ne!(link(&rebuilt(&linked)), link(&original));
    for id in [4301, 4302] {
        assert!(
            matches!(check_of(&linked, id), TreeCheck::Inconclusive(why) if why.contains("semi-global")),
            "{id}: {:?}",
            linked.trees
        );
    }
    // An empty name links nothing, like no `GLOB` at all.
    let empty = report(&rebuild(&compile(source), |_| {}, vec![glob(b"\0")]));
    assert!(proven(check_of(&empty, 4302)), "{:?}", empty.trees);
}

#[test]
fn definitions_sharing_a_chunk_id_share_its_callbacks() {
    // The VM keeps one callback table per OBJD chunk ID (the last one's), so neither
    // definition runs 4200 as `main` here.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_0136;
    @id(4200) main { temp[0] = 1; }
    @id(4300) @frame(locals: 0) tree pump "Pump" { temp[0] = 2; }
}"#;
    let iff = compile(source);
    let objd = iff.chunks.iter().find(|c| &c.tag == b"OBJD").expect("OBJD");
    let mut second = iff.data(objd).to_vec();
    second[0x0a..0x0c].copy_from_slice(&4300u16.to_le_bytes());
    second[0x1c..0x20].copy_from_slice(&0x7E00_0137u32.to_le_bytes());
    // The flags' high byte is part of a resource's key, so both definitions are live.
    let second = ChunkData {
        tag: *b"OBJD",
        id: objd.id,
        flags: objd.flags | 0x0100,
        label: "second".to_owned(),
        data: second,
    };
    let original = rebuild(
        &iff,
        |c| {
            if &c.tag == b"OBJf" {
                c.tag = *b"XOBf";
            }
        },
        vec![second],
    );
    assert_eq!(
        original.resource_order(b"OBJD").expect("resources").len(),
        2
    );
    // The rebuilt object gives each definition its own ID, so the first runs 4200.
    let report = report(&original);
    assert!(
        matches!(check_of(&report, 4200), TreeCheck::Inconclusive(why) if why.contains("callback")),
        "{:?}",
        report.trees
    );
}

#[test]
fn a_global_table_name_keeps_this_files_entry() {
    // A name from the global file's table is not known here, but this file may be the
    // global file: its own entry must not change.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_0138;
    strings 303 "Tree names" {
        pump = "Pump",
    }
    @id(4300) @frame(locals: 0) tree pump "Pump" { temp[0] = 7; }
    @id(4301) @frame(locals: 0) tree caller "Caller" {
        raw { run_tree_by_name(table: 303, index: 1, mode: 0, global: true); }
    }
}"#;
    // The lift writes `STR#` 303 only for this file's own names, so it drops it here.
    let original = compile(source);
    let report = report(&original);
    let table = |iff: &Iff| iff.get(b"STR#", 303).map(|c| iff.data(c).to_vec());
    assert_ne!(table(&rebuilt(&report)), table(&original));
    assert!(
        matches!(check_of(&report, 4301), TreeCheck::Inconclusive(why) if why.contains("names table")),
        "{:?}",
        report.trees
    );
}

#[test]
fn reads_of_definitions_and_callbacks_need_the_same_tables() {
    // Expression scope 21 reads the stack object's definition words, scope 35 whether
    // its callback table names a tree; the stack object may be this object.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_0139;
    @id(4200) main { temp[0] = 1; }
    @id(4300) @frame(locals: 0) tree pump "Pump" { temp[0] = 2; }
    @id(4301) @frame(locals: 0) tree words "Words" {
        raw { op 0x0002 (0, 0, 2, 0, 0, 5, 8, 21); }
    }
    @id(4302) @frame(locals: 0) tree slots "Slots" {
        raw { op 0x0002 (0, 0, 18, 0, 0, 5, 8, 35); }
    }
}"#;
    let iff = compile(source);
    let (init, main) = init_and_main(&iff);
    // Callback 18 runs `Pump`, which the lift writes as a plain tree.
    let mut entries = vec![(0, 0); 19];
    entries[0] = (0, init);
    entries[1] = (0, main);
    entries[18] = (0, 4300);
    let slots = report(&with_callbacks(&iff, &entries));
    assert!(
        matches!(check_of(&slots, 4302), TreeCheck::Inconclusive(why) if why.contains("callback table")),
        "{:?}",
        slots.trees
    );
    // A definition word the lift does not keep (a callback word an OBJf overrides).
    let objd = iff
        .chunks
        .iter()
        .find(|c| &c.tag == b"OBJD")
        .expect("OBJD")
        .id;
    let original = rebuild(
        &with_callbacks(&iff, &[(0, init), (0, main)]),
        |c| {
            if &c.tag == b"OBJD" && c.id == objd {
                c.data[0x5e..0x60].copy_from_slice(&4300u16.to_le_bytes());
            }
        },
        Vec::new(),
    );
    let report = report(&original);
    assert!(
        report
            .resources_not_reproduced
            .iter()
            .any(|r| r.starts_with("OBJD")),
        "{:?}",
        report.resources_not_reproduced
    );
    assert!(
        matches!(check_of(&report, 4301), TreeCheck::Inconclusive(why) if why.contains("definition's words")),
        "{:?}",
        report.trees
    );
}

#[test]
fn creating_an_object_runs_its_callbacks() {
    // Create Object Instance runs the new object's `init` (and more), whether or not it
    // creates from a record: it may create this object.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_013A;
    @id(4200) main { temp[0] = 1; }
    @id(4300) @frame(locals: 0) tree pump "Pump" { temp[0] = 2; }
    @id(4301) @frame(locals: 0) tree spawn "Spawn" {
        raw { op 0x002a (0x3a, 0x01, 0x00, 0x7e, 0, 0, 0, 0); }
    }
}"#;
    let iff = compile(source);
    let (init, main) = init_and_main(&iff);
    // Callback 18 runs `Pump`, which the lift writes as a plain tree.
    let mut entries = vec![(0, 0); 19];
    entries[0] = (0, init);
    entries[1] = (0, main);
    entries[18] = (0, 4300);
    let report = report(&with_callbacks(&iff, &entries));
    assert!(
        matches!(check_of(&report, 4301), TreeCheck::Inconclusive(why) if why.contains("creates an object")),
        "{:?}",
        report.trees
    );
}

#[test]
fn an_objf_table_overrides_the_definition_words() {
    // The VM reads callbacks from OBJf when there is one: `Pump` is `main` here though
    // the OBJD words still name 4200.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_013B;
    @id(4200) main { temp[0] = 1; }
    @id(4300) @frame(locals: 0) tree pump "Pump" { temp[0] = 2; }
}"#;
    let iff = compile(source);
    let (init, main) = init_and_main(&iff);
    assert_eq!(main, 4200);
    let objd = iff
        .chunks
        .iter()
        .find(|c| &c.tag == b"OBJD")
        .expect("OBJD")
        .id;
    let table = objf_table(&[(0, init), (0, 4300)]);
    let original = if iff.get(b"OBJf", objd).is_some() {
        rebuild(
            &iff,
            |c| {
                if &c.tag == b"OBJf" && c.id == objd {
                    c.data = table.clone();
                }
            },
            Vec::new(),
        )
    } else {
        let objf = ChunkData {
            tag: *b"OBJf",
            id: objd,
            flags: 0,
            label: String::new(),
            data: table,
        };
        rebuild(&iff, |_| {}, vec![objf])
    };
    let objd_data = |f: &Iff| {
        f.data(f.chunks.iter().find(|c| &c.tag == b"OBJD").unwrap())
            .to_vec()
    };
    assert_eq!(objd_data(&original), objd_data(&iff));
    let report = report(&original);
    for id in [4200, 4300] {
        if proven(check_of(&report, id)) {
            assert_eq!(
                init_and_main(&rebuilt(&report)).1,
                4300,
                "{id}: {:?}",
                report.trees
            );
        }
    }
    assert!(!proven(check_of(&report, 4300)), "{:?}", report.trees);
}

#[test]
fn non_private_code_the_lift_drops_leaves_no_tree_proven() {
    // Lifting the global file itself: tree 4301 calls this file's global tree 256,
    // which the lift does not write. Any tree may also reach it by a name another file
    // supplies, so none is proven.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_013C;
    @id(4300) @frame(locals: 0) tree pump "Pump" { temp[0] = 2; }
    @id(4301) @frame(locals: 0) tree caller "Caller" {
        raw { op 0x0100 (0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff); }
    }
}"#;
    let iff = compile(source);
    let pump = iff.all(b"BHAV").find(|c| c.id == 4300).expect("pump");
    let global = ChunkData {
        tag: *b"BHAV",
        id: 256,
        flags: 0,
        label: "Global pump".to_owned(),
        data: iff.data(pump).to_vec(),
    };
    let trees = report(&rebuild(&iff, |_| {}, vec![global]));
    assert!(
        trees
            .resources_not_reproduced
            .contains(&"BHAV 256".to_owned()),
        "{:?}",
        trees.resources_not_reproduced
    );
    for id in [4300, 4301] {
        assert!(
            matches!(check_of(&trees, id), TreeCheck::Inconclusive(why) if why.contains("BHAV 256")),
            "{id}: {:?}",
            trees.trees
        );
    }
    // A global constant table (256) this file holds as the global file.
    let table = ChunkData {
        tag: *b"BCON",
        id: 256,
        flags: 0,
        label: "Global constants".to_owned(),
        data: vec![1, 0, 77, 0],
    };
    let constants = report(&rebuild(&iff, |_| {}, vec![table]));
    assert!(
        constants
            .resources_not_reproduced
            .contains(&"BCON 256".to_owned()),
        "{:?}",
        constants.resources_not_reproduced
    );
    assert!(
        matches!(check_of(&constants, 4300), TreeCheck::Inconclusive(why) if why.contains("BCON 256")),
        "{:?}",
        constants.trees
    );
    // The VM loads the last chunk of each constant table ID, deleted or not.
    let deleted = ChunkData {
        tag: *b"BCON",
        id: 256,
        flags: sims_formats::iff::DELETED,
        label: "Global constants".to_owned(),
        data: vec![1, 0, 77, 0],
    };
    let deleted = report(&rebuild(&iff, |_| {}, vec![deleted]));
    assert!(
        matches!(check_of(&deleted, 4300), TreeCheck::Inconclusive(why) if why.contains("BCON 256")),
        "{:?}",
        deleted.trees
    );
}

#[test]
fn a_global_tree_runs_with_this_object_as_owner() {
    // Caller 4301 runs global tree 256, which this file does not hold. It runs with
    // this object as owner, so it may run any private tree by name: the caller is
    // proven only while every private tree is, in the same environment.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_013F;
    strings 300 "Lines" {
        a = "A",
    }
    @id(4300) @frame(locals: 0) tree pump "Pump" {
        raw { op 0x0015 (0x2c, 0x01, 0, 0, 0, 0, 0, 0); }
    }
    @id(4301) @frame(locals: 0) tree caller "Caller" {
        raw { op 0x0100 (0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff); }
    }
}"#;
    let iff = compile(source);
    let same = report(&iff);
    assert!(proven(check_of(&same, 4300)), "{:?}", same.trees);
    assert!(proven(check_of(&same, 4301)), "{:?}", same.trees);
    // Pump reads a string table the lift cannot reproduce (its chunk is deleted).
    let gap = report(&rebuild(
        &iff,
        |c| {
            if &c.tag == b"STR#" && c.id == 300 {
                c.flags |= sims_formats::iff::DELETED;
            }
        },
        Vec::new(),
    ));
    assert!(!proven(check_of(&gap, 4300)), "{:?}", gap.trees);
    assert!(!proven(check_of(&gap, 4301)), "{:?}", gap.trees);
    // A private tree whose label the rebuilt file cannot store: the global tree may
    // look it up by that label.
    let long = format!("P{}", " ".repeat(62)) + "X";
    let renamed = report(&relabel(&iff, 4300, &long));
    assert!(!proven(check_of(&renamed, 4300)), "{:?}", renamed.trees);
    assert!(
        matches!(check_of(&renamed, 4301), TreeCheck::Inconclusive(why) if why.contains("global or semi-global")),
        "{:?}",
        renamed.trees
    );
}

#[test]
fn a_name_entry_zero_looks_nothing_up() {
    // Entry 0 names no tree in either table, so a changed label is not seen.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_013D;
    @id(4200) main { temp[0] = 1; }
    @id(4301) @frame(locals: 0) tree caller "Caller" {
        raw { run_tree_by_name(table: 303, index: 0, mode: 0, global: true); }
    }
}"#;
    let report = report(&relabel(&compile(source), 4200, "Tick"));
    assert!(proven(check_of(&report, 4301)), "{:?}", report.trees);
}

#[test]
fn a_deleted_definition_does_not_stand_in_for_the_live_one() {
    // The VM reads the live OBJD: a deleted copy with the rebuilt bytes after it does not
    // make a changed definition word look reproduced.
    let source = r#"object "Fable Bellows" {
    guid: 0x7E00_013E;
    @id(4200) main { temp[0] = 1; }
    @id(4301) @frame(locals: 0) tree words "Words" {
        raw { op 0x0002 (0, 0, 47, 0, 0, 5, 8, 21); }
    }
}"#;
    let iff = compile(source);
    let objd = iff.chunks.iter().find(|c| &c.tag == b"OBJD").expect("OBJD");
    // The VM's words start at the chunk's first word (the version), so word 47 is at
    // byte 94.
    let at = 47 * 2;
    let stale = ChunkData {
        tag: *b"OBJD",
        id: objd.id,
        flags: objd.flags | sims_formats::iff::DELETED,
        label: objd.label.clone(),
        data: iff.data(objd).to_vec(),
    };
    let original = rebuild(
        &iff,
        |c| {
            if &c.tag == b"OBJD" {
                c.data[at..at + 2].copy_from_slice(&123u16.to_le_bytes());
            }
        },
        vec![stale],
    );
    let report = report(&original);
    assert!(
        matches!(check_of(&report, 4301), TreeCheck::Inconclusive(why) if why.contains("definition's words")),
        "{:?}",
        report.trees
    );
}
