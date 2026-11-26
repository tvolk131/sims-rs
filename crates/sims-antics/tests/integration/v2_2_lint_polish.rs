//! `.antics` v2.2 lint polish: dot names for a Sim's numbered skills, personality traits
//! and motives (`untyped_engine_field`), flag methods on typed flag words
//! (`prefer_method_syntax`), and why `negated_comparison` stays quiet before an exit.
//!
//! The lint keeps a finding of the first two only when its rewrite builds the file to the
//! same bytes. The tests apply each suggestion as a user would and compare the builds.
//!
//! Every fixture here is synthetic (GUIDs in the clean-room 0x7E00_xxxx range). See
//! `docs/decisions/20261125-antics-v2-2-lint-polish.md`.

use sims_antics::v2::PackResolverConfig;
use sims_antics::v2::lint::{LintDiagnostic, Severity, lint_v2_source};
use sims_antics::{IffEquivOptions, IffVerdict, LiveOptions, check_iff_equivalent};

/// A synthetic object with `uses` before it and `members` in it.
fn obj(uses: &str, members: &str) -> String {
    format!(
        "{uses}object \"Synth Probe\" {{\n    guid: 0x7E00_0720;\n    price: 1;\n    #[slot(2)] state level: i16;\n{members}\n}}\n"
    )
}

/// An object-context tree running `body`.
fn tree(uses: &str, body: &str) -> String {
    obj(
        uses,
        &format!("    #[bhav(label = \"Probe\")] fn probe() {{\n{body}\n    }}"),
    )
}

/// A Sim-context tree running `body`.
fn sim_tree(uses: &str, body: &str) -> String {
    obj(
        uses,
        &format!("    #[bhav(label = \"Probe\")] fn probe(sim: Sim) {{\n{body}\n    }}"),
    )
}

fn lint(source: &str) -> Vec<LintDiagnostic> {
    lint_v2_source(source, "probe.antics").unwrap_or_else(|e| panic!("lint failed: {e}\n{source}"))
}

fn of(source: &str, rule: &str) -> Vec<LintDiagnostic> {
    lint(source)
        .into_iter()
        .filter(|d| d.rule == rule)
        .collect()
}

fn one(source: &str, rule: &str) -> LintDiagnostic {
    let found = of(source, rule);
    assert_eq!(
        found.len(),
        1,
        "expected one {rule} in\n{source}\ngot {found:#?}"
    );
    found.into_iter().next().unwrap()
}

fn none(source: &str, rule: &str) {
    let found = of(source, rule);
    assert!(
        found.is_empty(),
        "unexpected {rule} in\n{source}\n{found:#?}"
    );
}

fn iff(source: &str) -> Vec<u8> {
    sims_antics::compile_v2_source("probe.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("must compile: {e}\n{source}"))
        .iff_bytes
}

/// The 1-based line of the first line of `source` containing `needle`.
fn line_of(source: &str, needle: &str) -> usize {
    source
        .lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not in source"))
        + 1
}

/// Asserts that `source` has exactly one `rule` finding, on the line of `original`, whose
/// suggestion is `want`, and that writing it in place of the unique text `original`
/// builds the same bytes. Returns the rewritten source.
fn rewrites(source: &str, rule: &str, original: &str, want: &str) -> String {
    let d = one(source, rule);
    assert_eq!(d.suggestion.as_deref(), Some(want), "{d:#?}");
    assert_eq!(d.line, line_of(source, original), "{d:#?}");
    assert_eq!(d.severity, Severity::Warning);
    assert_eq!(
        source.matches(original).count(),
        1,
        "{original:?} must be unique"
    );
    let fixed = source.replacen(original, want, 1);
    assert_eq!(
        iff(source),
        iff(&fixed),
        "rewrite builds other bytes:\n{fixed}"
    );
    fixed
}

// --- untyped_engine_field: skills, personality, motives -------------------------------------

const SKILLS: &[(i64, &str)] = &[
    (10, "cooking"),
    (11, "charisma"),
    (12, "mechanical"),
    (15, "creativity"),
    (17, "body"),
    (18, "logic"),
];

const TRAITS: &[(i64, &str)] = &[
    (2, "nice"),
    (3, "active"),
    (5, "playful"),
    (6, "outgoing"),
    (7, "neat"),
];

const MOTIVES: &[(i64, &str)] = &[
    (3, "mood"),
    (5, "energy"),
    (6, "comfort"),
    (7, "hunger"),
    (8, "hygiene"),
    (9, "bladder"),
    (11, "alertness"),
    (13, "room"),
    (14, "social"),
    (15, "fun"),
];

#[test]
fn a_sims_numbered_skills_are_skill_names() {
    for &(n, name) in SKILLS {
        let cell = format!("sim.person[{n}]");
        let source = sim_tree("", &format!("        temp[0] = {cell};"));
        rewrites(
            &source,
            "untyped_engine_field",
            &cell,
            &format!("sim.skill.{name}"),
        );
    }
}

#[test]
fn a_sims_numbered_traits_are_personality_names() {
    for &(n, name) in TRAITS {
        let cell = format!("sim.person[{n}]");
        let source = sim_tree("", &format!("        temp[0] = {cell};"));
        rewrites(
            &source,
            "untyped_engine_field",
            &cell,
            &format!("sim.personality.{name}"),
        );
    }
}

#[test]
fn a_sims_numbered_motives_are_motive_names() {
    for &(n, name) in MOTIVES {
        let cell = format!("sim.motive[{n}]");
        let source = sim_tree("", &format!("        temp[0] = {cell};"));
        rewrites(
            &source,
            "untyped_engine_field",
            &cell,
            &format!("sim.motive.{name}"),
        );
    }
}

#[test]
fn writes_to_skills_traits_and_motives_take_the_names() {
    for (stmt, cell, want) in [
        ("sim.person[10] = 5;", "sim.person[10]", "sim.skill.cooking"),
        (
            "sim.person[18] += 100;",
            "sim.person[18]",
            "sim.skill.logic",
        ),
        (
            "sim.person[7] = 3;",
            "sim.person[7]",
            "sim.personality.neat",
        ),
        ("sim.motive[7] = 50;", "sim.motive[7]", "sim.motive.hunger"),
        ("sim.motive[15] -= 10;", "sim.motive[15]", "sim.motive.fun"),
    ] {
        let source = sim_tree("", &format!("        {stmt}"));
        rewrites(&source, "untyped_engine_field", cell, want);
    }
}

#[test]
fn stack_obj_and_handle_locals_name_skills_and_traits() {
    for (cell, want) in [
        ("stack_obj.person[11]", "stack_obj.skill.charisma"),
        ("stack_obj.person[15]", "stack_obj.skill.creativity"),
        ("stack_obj.person[2]", "stack_obj.personality.nice"),
        ("stack_obj.person[6]", "stack_obj.personality.outgoing"),
        ("stack_obj.motive[14]", "stack_obj.motive.social"),
    ] {
        let source = tree("", &format!("        temp[0] = {cell};"));
        rewrites(&source, "untyped_engine_field", cell, want);
    }
    let source = tree(
        "",
        "        let h = stack_obj;\n        temp[0] = h.person[12];",
    );
    rewrites(
        &source,
        "untyped_engine_field",
        "h.person[12]",
        "h.skill.mechanical",
    );
}

#[test]
fn every_cell_of_one_statement_takes_its_name() {
    let source = sim_tree(
        "",
        "        temp[0] = sim.person[10] + sim.person[7] + stack_obj.person[3];",
    );
    let found = of(&source, "untyped_engine_field");
    let wants: Vec<_> = found.iter().filter_map(|d| d.suggestion.clone()).collect();
    assert_eq!(
        wants,
        [
            "sim.skill.cooking",
            "sim.personality.neat",
            "stack_obj.personality.active"
        ],
        "{found:#?}"
    );
    let fixed = source
        .replacen("sim.person[10]", "sim.skill.cooking", 1)
        .replacen("sim.person[7]", "sim.personality.neat", 1)
        .replacen("stack_obj.person[3]", "stack_obj.personality.active", 1);
    assert_eq!(iff(&source), iff(&fixed));
}

#[test]
fn unnamed_person_words_and_constant_indices_stay() {
    // Words 4, 8 and 9 have no dot name; motive 4 is no motive of a Sim.
    for body in [
        "        temp[0] = sim.person[4];",
        "        temp[0] = sim.person[8];",
        "        temp[0] = sim.person[9];",
        "        temp[0] = sim.motive[4];",
        "        temp[0] = stack_obj.person[13];",
    ] {
        none(&sim_tree("", body), "untyped_engine_field");
    }
    // A constant may stand for several cells.
    let source = obj(
        "",
        "    const COOKING = 10;\n    #[bhav(label = \"Probe\")] fn probe(sim: Sim) {\n        temp[0] = sim.person[COOKING];\n    }",
    );
    none(&source, "untyped_engine_field");
    // A comment inside the cell would be lost.
    let source = sim_tree("", "        temp[0] = sim.person[/* cooking */ 10];");
    none(&source, "untyped_engine_field");
}

#[test]
fn the_dot_names_of_an_object_tree_are_not_proposed_on_self() {
    // `self` is the object: its `person` words are no Sim's.
    let source = tree("", "        temp[0] = self.data[41];");
    let d = one(&source, "untyped_engine_field");
    assert_eq!(d.suggestion.as_deref(), Some("self.value"));
}

// --- prefer_method_syntax: flag methods on typed flag words ---------------------------------

const OBJECT_FLAGS: &str = "use std::flags::ObjectFlags;\n";

#[test]
fn flag_set_and_flag_clear_statements_are_set_and_clear() {
    let source = tree(
        OBJECT_FLAGS,
        "        self.flags flag_set ObjectFlags::InUse;",
    );
    rewrites(
        &source,
        "prefer_method_syntax",
        "self.flags flag_set ObjectFlags::InUse",
        "self.flags.set(ObjectFlags::InUse)",
    );
    let source = tree(
        OBJECT_FLAGS,
        "        stack_obj.flags flag_clear ObjectFlags::Hidden;",
    );
    rewrites(
        &source,
        "prefer_method_syntax",
        "stack_obj.flags flag_clear ObjectFlags::Hidden",
        "stack_obj.flags.clear(ObjectFlags::Hidden)",
    );
    let d = one(&source, "prefer_method_syntax");
    assert_eq!(
        d.message,
        "`flag_clear` on a typed flag word has a method form"
    );
    assert_eq!(
        d.help.as_deref(),
        Some("write `stack_obj.flags.clear(ObjectFlags::Hidden)`; it builds the same nodes")
    );
}

#[test]
fn flag_test_is_contains() {
    let source = tree(
        OBJECT_FLAGS,
        "        if self.flags flag_test ObjectFlags::InUse {\n            temp[0] = 1;\n        }",
    );
    rewrites(
        &source,
        "prefer_method_syntax",
        "self.flags flag_test ObjectFlags::InUse",
        "self.flags.contains(ObjectFlags::InUse)",
    );
    // Negated, and in a Sim's tree on `stack_obj`.
    let source = sim_tree(
        OBJECT_FLAGS,
        "        if !(stack_obj.flags flag_test ObjectFlags::NoInteractions) {\n            temp[0] = 1;\n        }",
    );
    rewrites(
        &source,
        "prefer_method_syntax",
        "stack_obj.flags flag_test ObjectFlags::NoInteractions",
        "stack_obj.flags.contains(ObjectFlags::NoInteractions)",
    );
}

#[test]
fn every_typed_object_flag_word_has_the_methods() {
    for (uses, stmt, want) in [
        (
            "use std::flags::ObjectFlags2;\n",
            "self.flags2 flag_set ObjectFlags2::CatchesFire",
            "self.flags2.set(ObjectFlags2::CatchesFire)",
        ),
        (
            "use std::flags::BuyFlags;\n",
            "self.buy_flags flag_clear BuyFlags::CanSell",
            "self.buy_flags.clear(BuyFlags::CanSell)",
        ),
        (
            "use std::flags::WallAdjacency;\n",
            "self.wall_adjacency flag_clear WallAdjacency::WallLeft",
            "self.wall_adjacency.clear(WallAdjacency::WallLeft)",
        ),
        // Prelude types need no `use`.
        (
            "",
            "self.placement flag_set Placement::Water",
            "self.placement.set(Placement::Water)",
        ),
        (
            "",
            "self.allowed_heights flag_clear Height::Counter",
            "self.allowed_heights.clear(Height::Counter)",
        ),
    ] {
        let source = tree(uses, &format!("        {stmt};"));
        rewrites(&source, "prefer_method_syntax", stmt, want);
    }
}

#[test]
fn the_operand_keeps_the_files_spelling_of_the_type() {
    let source = tree(
        "use std::flags::ObjectFlags as Of;\n",
        "        self.flags flag_set Of::InUse;",
    );
    rewrites(
        &source,
        "prefer_method_syntax",
        "self.flags flag_set Of::InUse",
        "self.flags.set(Of::InUse)",
    );
    let source = tree(
        "use std::flags::*;\n",
        "        self.flags flag_set ObjectFlags::InUse;",
    );
    rewrites(
        &source,
        "prefer_method_syntax",
        "self.flags flag_set ObjectFlags::InUse",
        "self.flags.set(ObjectFlags::InUse)",
    );
}

#[test]
fn raw_bits_and_cells_by_number_have_no_flag_method() {
    // A raw bit: `untyped_engine_constant` names it first.
    let source = tree("", "        self.flags flag_set 6;");
    none(&source, "prefer_method_syntax");
    one(&source, "untyped_engine_constant");
    // A cell by number: `untyped_engine_field` names it first.
    let source = tree(
        OBJECT_FLAGS,
        "        self.data[8] flag_set ObjectFlags::InUse;",
    );
    none(&source, "prefer_method_syntax");
    one(&source, "untyped_engine_field");
    let source = tree(
        OBJECT_FLAGS,
        "        if stack_obj.data[8] flag_test ObjectFlags::InUse {\n            temp[0] = 1;\n        }",
    );
    none(&source, "prefer_method_syntax");
}

#[test]
fn untyped_receivers_and_other_operands_have_no_flag_method() {
    // `global[20]` written out: `untyped_engine_field` names it `world.expansions` first
    // (`world.expansions` and `wall_placement` take methods since v2.2 step 2).
    let source = tree(
        "use std::flags::Expansions;\n",
        "        if global[20] flag_test Expansions::Pets {\n            temp[0] = 1;\n        }",
    );
    none(&source, "prefer_method_syntax");
    // A state, not the engine word.
    let source = obj(
        "",
        "    bitflags Mine {\n        A = 1,\n    }\n    state mine: Mine;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mine flag_set Mine::A;\n    }",
    );
    none(&source, "prefer_method_syntax");
    // An operand of another type than the word's.
    let source = tree(
        "use std::flags::ObjectFlags2;\n",
        "        self.flags flag_set ObjectFlags2::CatchesFire;",
    );
    none(&source, "prefer_method_syntax");
    // A file type that shadows `std`'s.
    let source = obj(
        "",
        "    #[bhav(label = \"Probe\")] fn probe() {\n        self.flags flag_set ObjectFlags::Mine;\n    }\n}\nbitflags ObjectFlags {\n    Mine = 6,",
    );
    none(&source, "prefer_method_syntax");
    // A comment the rewrite would drop.
    let source = tree(
        OBJECT_FLAGS,
        "        self.flags /* busy */ flag_set ObjectFlags::InUse;",
    );
    none(&source, "prefer_method_syntax");
}

#[test]
fn a_flag_method_and_its_constant_are_one_rewrite_each() {
    // The constant is named first; the method form follows once the operand is typed.
    let source = tree(OBJECT_FLAGS, "        self.flags flag_set 6;");
    let d = one(&source, "untyped_engine_constant");
    assert_eq!(d.suggestion.as_deref(), Some("ObjectFlags::InUse"));
    let typed = source.replacen("flag_set 6", "flag_set ObjectFlags::InUse", 1);
    assert_eq!(iff(&source), iff(&typed));
    rewrites(
        &typed,
        "prefer_method_syntax",
        "self.flags flag_set ObjectFlags::InUse",
        "self.flags.set(ObjectFlags::InUse)",
    );
}

#[test]
fn a_prelude_alias_keeps_its_spelling() {
    let uses = "use std::flags::{Placement as P, Height as H};\n";
    for (stmt, want) in [
        (
            "self.placement flag_set P::Water",
            "self.placement.set(P::Water)",
        ),
        (
            "self.allowed_heights flag_clear H::Counter",
            "self.allowed_heights.clear(H::Counter)",
        ),
    ] {
        let source = tree(uses, &format!("        {stmt};"));
        rewrites(&source, "prefer_method_syntax", stmt, want);
    }
}

#[test]
fn an_object_type_that_shadows_the_imported_one_has_no_flag_method() {
    // The object's own `ObjectFlags` hides the imported `std` type inside it.
    let source = obj(
        OBJECT_FLAGS,
        "    bitflags ObjectFlags {\n        Custom = 6,\n    }\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.flags flag_set ObjectFlags::Custom;\n    }",
    );
    none(&source, "prefer_method_syntax");
}

#[test]
fn a_parenthesised_receiver_has_no_dot_name() {
    // The rewrite replaces `sim.person[10]`; `(sim).person[10]` is other text.
    let source = sim_tree("", "        temp[0] = (sim).person[10];");
    none(&source, "untyped_engine_field");
}

#[test]
fn spaces_inside_a_cell_keep_its_name() {
    for (cell, want) in [
        ("stack_obj.data [41]", "stack_obj.value"),
        ("stack_obj . data[41]", "stack_obj.value"),
        ("stack_obj.data\t[41]", "stack_obj.value"),
        ("stack_obj.data\r[41]", "stack_obj.value"),
        ("sim.person [10]", "sim.skill.cooking"),
    ] {
        let source = sim_tree("", &format!("        temp[0] = {cell};"));
        rewrites(&source, "untyped_engine_field", cell, want);
    }
}

#[test]
fn every_import_of_the_flag_type_is_a_spelling() {
    let stmts =
        "        self.flags flag_set ObjectFlags::InUse;\n        self.flags flag_set Of::InUse;";
    for uses in [
        "use std::flags::ObjectFlags;\nuse std::flags::ObjectFlags as Of;\n",
        "use std::flags::*;\nuse std::flags::{ObjectFlags as Of};\n",
    ] {
        let source = tree(uses, stmts);
        assert_eq!(
            suggestions(&source, "prefer_method_syntax"),
            [
                "self.flags.set(ObjectFlags::InUse)",
                "self.flags.set(Of::InUse)"
            ],
            "{source}"
        );
    }
    // A file type named `ObjectFlags` leaves the alias `Of` its meaning.
    let source = tree(
        "use std::flags::{ObjectFlags as Of};\nbitflags ObjectFlags {\n    Custom = 6,\n}\n",
        "        self.flags flag_set Of::InUse;",
    );
    rewrites(
        &source,
        "prefer_method_syntax",
        "self.flags flag_set Of::InUse",
        "self.flags.set(Of::InUse)",
    );
}

#[test]
fn only_the_objects_own_template_shadows_its_types_and_states() {
    let stmt = "        self.flags flag_set ObjectFlags::InUse;";
    // Another template's type or state is not this object's.
    for template in [
        "template Other {\n    bitflags ObjectFlags {\n        Custom = 6,\n    }\n}\n",
        "template Other {\n    state flags: i16;\n}\n",
    ] {
        let source = tree(&format!("{OBJECT_FLAGS}{template}"), stmt);
        rewrites(
            &source,
            "prefer_method_syntax",
            "self.flags flag_set ObjectFlags::InUse",
            "self.flags.set(ObjectFlags::InUse)",
        );
    }
    // Another object's type or state neither.
    let other = "object \"Other Probe\" {\n    guid: 0x7E00_0721;\n    price: 1;\n    state flags: i16;\n    bitflags ObjectFlags {\n        Custom = 6,\n    }\n}\n";
    let source = tree(&format!("{OBJECT_FLAGS}{other}"), stmt);
    one(&source, "prefer_method_syntax");
    // The object's own template's type or state is.
    for template in [
        "template Base {\n    bitflags ObjectFlags {\n        Custom = 6,\n    }\n}\n",
        "template Base {\n    state flags: ObjectFlags;\n}\n",
    ] {
        let source = format!(
            "{OBJECT_FLAGS}{template}object \"Synth Probe\" from Base {{\n    guid: 0x7E00_0720;\n    price: 1;\n    #[bhav(label = \"Probe\")] fn probe() {{\n        self.flags flag_set ObjectFlags::InUse;\n    }}\n}}\n"
        );
        none(&source, "prefer_method_syntax");
    }
}

#[test]
fn an_objects_state_shadows_me_only_where_me_is_the_object() {
    let members = |body: &str, sim: &str| {
        format!(
            "    state flags: i16;\n    #[bhav(label = \"Probe\")] fn probe({sim}) {{\n{body}\n    }}"
        )
    };
    // In a Sim's body `me` is the Sim, whose flags no state of the object hides.
    let source = obj(
        OBJECT_FLAGS,
        &members("        me.flags flag_set ObjectFlags::InUse;", "sim: Sim"),
    );
    rewrites(
        &source,
        "prefer_method_syntax",
        "me.flags flag_set ObjectFlags::InUse",
        "me.flags.set(ObjectFlags::InUse)",
    );
    // In the object's own body `self.flags` is the state.
    let source = obj(
        OBJECT_FLAGS,
        &members("        self.flags flag_set ObjectFlags::InUse;", ""),
    );
    none(&source, "prefer_method_syntax");
}

// --- trial budgets: the new rewrites never crowd out an older finding -----------------------

fn suggestions(source: &str, rule: &str) -> Vec<String> {
    of(source, rule)
        .into_iter()
        .filter_map(|d| d.suggestion)
        .collect()
}

#[test]
fn flag_methods_do_not_use_up_the_trial_budget_of_a_raw_bit() {
    // 48 flag methods are tried after the raw bit's constant, not before it.
    let params: Vec<_> = (0..49).map(|i| format!("b{i}: bool")).collect();
    let mut args = vec!["            self.flags flag_test ObjectFlags::InUse"; 48];
    args.push("            self.flags flag_test 6");
    let source = obj(
        OBJECT_FLAGS,
        &format!(
            "    fn consume({}) {{\n        temp[0] = 2;\n    }}\n    #[bhav(label = \"Probe\")] fn probe() {{\n        consume(\n{}\n        );\n    }}",
            params.join(", "),
            args.join(",\n"),
        ),
    );
    assert_eq!(
        suggestions(&source, "untyped_engine_constant"),
        ["ObjectFlags::InUse"]
    );
}

#[test]
fn dot_names_do_not_use_up_the_trial_budget_of_an_object_word() {
    // 48 skill names are tried after `stack_obj.value`, not before it.
    let body = "        let v = sim.person[10];\n        temp[0] = v;\n".repeat(48)
        + "        temp[1] = stack_obj.data[41];";
    let source = sim_tree("", &body);
    let found = suggestions(&source, "untyped_engine_field");
    assert!(found.iter().any(|s| s == "stack_obj.value"), "{found:?}");
}

#[test]
fn dot_names_do_not_use_up_the_joint_budget_of_object_words() {
    // Dot names and object words interleave; the object words are checked together first.
    let block =
        "        if self.data[11] == stack_obj.data[40] {\n            temp[0] = 1;\n        }\n"
            .to_owned()
            + &"        temp[0] = sim.person[10];\n".repeat(3);
    let source = sim_tree(
        "",
        &(block.repeat(3) + "        temp[1] = stack_obj.data[41];"),
    );
    let found = suggestions(&source, "untyped_engine_field");
    assert_eq!(
        found.iter().filter(|s| *s == "self.id").count(),
        3,
        "{found:?}"
    );
    assert!(found.iter().any(|s| s == "stack_obj.value"), "{found:?}");
}

// --- negated_comparison: an exit observes the compare's own result --------------------------

/// The verdict of comparing the builds of `a` and `b` on the probe tree (4098, after
/// `Init` and `Main`), with or without the thread's last branch result observed (only
/// a named tree is relaxed, as with `verify-equiv --tree 4098 --ignore-last-result`).
fn verdict(a: &str, b: &str, observe_last_result: bool) -> IffVerdict {
    let opts = IffEquivOptions {
        live: LiveOptions {
            observe_last_result,
            ..LiveOptions::default()
        },
        tree: Some(4098),
        ..IffEquivOptions::default()
    };
    check_iff_equivalent(&iff(a), &iff(b), &opts)
        .expect("check")
        .verdict()
}

#[test]
fn a_negated_compare_before_a_quiet_store_is_reported_and_proves_equivalent() {
    let source = tree(
        "",
        "        if !(self.level > 0) {\n            temp[1] = 1;\n        } else {\n            temp[1] = 2;\n        }\n        temp[2] = 3;",
    );
    let d = one(&source, "negated_comparison");
    assert_eq!(d.suggestion.as_deref(), Some("self.level <= 0"));
    let flipped = source.replacen("!(self.level > 0)", "self.level <= 0", 1);
    assert_eq!(verdict(&source, &flipped, true), IffVerdict::Equivalent);
}

#[test]
fn a_negated_compare_before_an_exit_is_not_reported() {
    // The VM pops the frame on `return true` / `return false` before it writes the exit
    // value, and the pop passes the compare's own result on (`release_use` and
    // `end_action`, `sims-vm/src/interpreter.rs` `branch`). The flipped compare has the
    // opposite result. On an error exit the result stays for play-mode recovery and a
    // person's reset.
    for exit in ["return false;", "return true;", "panic!();", "return;"] {
        let (ret, tail) = if exit.starts_with("return true") || exit.starts_with("return false") {
            (" -> bool", "\n        return true;")
        } else {
            ("", "")
        };
        let body = format!(
            "        if !(self.level > 0) {{\n            {exit}\n        }}\n        temp[2] = 3;{tail}"
        );
        let source = obj(
            "",
            &format!("    #[bhav(label = \"Probe\")] fn probe(){ret} {{\n{body}\n    }}"),
        );
        none(&source, "negated_comparison");
        let flipped = source.replacen("!(self.level > 0)", "self.level <= 0", 1);
        assert_ne!(
            verdict(&source, &flipped, true),
            IffVerdict::Equivalent,
            "{exit}: the flip must not prove equivalent\n{source}"
        );
    }
}

#[test]
fn only_the_last_result_tells_the_flip_before_a_return_apart() {
    for exit in ["return false;", "return true;"] {
        let source = obj(
            "",
            &format!(
                "    #[bhav(label = \"Probe\")] fn probe() -> bool {{\n        if !(self.level > 0) {{\n            {exit}\n        }}\n        temp[2] = 3;\n        return true;\n    }}"
            ),
        );
        let flipped = source.replacen("!(self.level > 0)", "self.level <= 0", 1);
        assert_ne!(verdict(&source, &flipped, true), IffVerdict::Equivalent);
        assert_eq!(
            verdict(&source, &flipped, false),
            IffVerdict::Equivalent,
            "{exit}"
        );
    }
}

#[test]
fn an_exit_on_either_branch_keeps_the_rule_quiet() {
    for (then_body, else_body) in [
        ("temp[1] = 1;", "return false;"),
        ("panic!();", "temp[1] = 2;"),
        ("temp[1] = 1;", "panic!();"),
    ] {
        let source = obj(
            "",
            &format!(
                "    #[bhav(label = \"Probe\")] fn probe() -> bool {{\n        if !(self.level > 0) {{\n            {then_body}\n        }} else {{\n            {else_body}\n        }}\n        temp[2] = 3;\n        return true;\n    }}"
            ),
        );
        none(&source, "negated_comparison");
    }
}
