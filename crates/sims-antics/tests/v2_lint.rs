//! `.antics` v2 linter (`sims-antics lint`).
//!
//! Every fixture here is synthetic (GUIDs in the clean-room 0x7E00_xxxx range). Each rule
//! has at least one positive and one negative case.

use sims_antics::v2::lint::{
    ENUM_WILDCARD_ALL_COVERED, LINT_RULES, LintDiagnostic, LintInput, Severity,
    diagnostics_to_json, lint_v2_source, lint_v2_sources, render_diagnostic,
};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Wraps object members in a synthetic object.
fn obj(members: &str) -> String {
    format!(
        "enum Mode {{ Off, Low, High }}\n\
         object \"Synth Probe\" {{\n    guid: 0x7E00_0100;\n    price: 1;\n    \
         sprite: Sprite::procedural(\"floor_lamp\", primary: #101010, accent: #202020, trim: #303030);\n    \
         state level @ 2: i16;\n    state mode: Mode;\n{members}\n}}\n"
    )
}

/// Wraps statements in an object-context tree that uses `level` and `mode`.
fn tree(body: &str) -> String {
    obj(&format!(
        "    tree probe \"Probe\" {{\n        self.level = 0;\n        self.mode = Mode::Off;\n{body}\n    }}"
    ))
}

/// Wraps statements in a Sim-context tree.
fn sim_tree(body: &str) -> String {
    obj(&format!(
        "    tree probe \"Probe\" (sim: Sim) {{\n        self.level = 0;\n        self.mode = Mode::Off;\n{body}\n    }}"
    ))
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

/// The 1-based line of the first line of `source` containing `needle`.
fn line_of(source: &str, needle: &str) -> usize {
    source
        .lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not in source"))
        + 1
}

#[test]
fn the_rule_table_lists_all_thirteen_rules() {
    let names: Vec<&str> = LINT_RULES.iter().map(|r| r.name).collect();
    for expected in [
        "enum_wildcard_single_variant",
        "enum_wildcard_all_covered",
        "dropped_bool_call",
        "unbounded_loop_without_yield",
        "stack_obj_clobber_after_self",
        "unused_decl",
        "frame_headroom",
        "while_true",
        "redundant_bool_return",
        "collapsible_if_chain",
        "manual_raw_has_builtin",
        "magic_guid_literal",
        "negated_comparison",
    ] {
        assert!(names.contains(&expected), "{expected} missing");
    }
    let unsuppressible: Vec<&str> = LINT_RULES
        .iter()
        .filter(|r| !r.suppressible)
        .map(|r| r.name)
        .collect();
    assert_eq!(unsuppressible, [ENUM_WILDCARD_ALL_COVERED]);
}

#[test]
fn a_clean_object_has_no_findings() {
    let source = tree(
        "        loop {\n            sleep(10);\n            if self.level > 3 { break; }\n            self.level += 1;\n        }",
    );
    assert_eq!(lint(&source), Vec::new());
}

// --- enum_wildcard_single_variant / enum_wildcard_all_covered ---------------------------

#[test]
fn wildcard_standing_for_one_variant_is_flagged() {
    let source = tree(
        "        match self.mode {\n            Mode::Off => { self.level = 1; }\n            Mode::Low => { self.level = 2; }\n            _ => { self.level = 3; }\n        }",
    );
    let d = one(&source, "enum_wildcard_single_variant");
    assert_eq!(d.severity, Severity::Warning);
    assert_eq!(d.line, line_of(&source, "_ =>"));
    assert_eq!(d.suggestion.as_deref(), Some("Mode::High"));
    none(&source, "enum_wildcard_all_covered");
}

#[test]
fn wildcard_standing_for_two_variants_or_ints_is_fine() {
    none(
        &tree(
            "        match self.mode {\n            Mode::Off => { self.level = 1; }\n            _ => { self.level = 3; }\n        }",
        ),
        "enum_wildcard_single_variant",
    );
    none(
        &tree(
            "        match self.level {\n            0 => { self.level = 1; }\n            1 => { self.level = 2; }\n            _ => { self.level = 3; }\n        }",
        ),
        "enum_wildcard_single_variant",
    );
    // An enum the file set does not declare is skipped.
    none(
        &tree(
            "        match self.level {\n            Other::A => { self.level = 1; }\n            _ => { self.level = 3; }\n        }",
        ),
        "enum_wildcard_single_variant",
    );
}

#[test]
fn wildcard_after_every_variant_is_an_error() {
    let source = tree(
        "        match self.mode {\n            Mode::Off => { self.level = 1; }\n            Mode::Low => { self.level = 2; }\n            Mode::High => { self.level = 3; }\n            _ => { self.level = 4; }\n        }",
    );
    let d = one(&source, "enum_wildcard_all_covered");
    assert_eq!(d.severity, Severity::Error);
    assert!(d.note.as_deref().unwrap().contains("`i16`"), "{d:?}");
    none(&source, "enum_wildcard_single_variant");
}

#[test]
fn wildcard_rules_cover_match_expressions() {
    let single =
        tree("        self.level = match self.mode { Mode::Off => 1, Mode::High => 2, _ => 3 };");
    assert_eq!(
        one(&single, "enum_wildcard_single_variant")
            .suggestion
            .as_deref(),
        Some("Mode::Low")
    );
    let all = tree(
        "        self.level = match self.mode { Mode::Off => 1, Mode::Low => 2, Mode::High => 3, _ => 4 };",
    );
    one(&all, "enum_wildcard_all_covered");
}

#[test]
fn wildcard_rules_resolve_the_enum_through_imports() {
    // `Mode` here is `Wider` imported under another name; an unrelated two-variant `Mode`
    // elsewhere in the set must not make the `_` arm look redundant.
    let user = obj("    tree probe \"Probe\" {\n        match self.mode {\n            Mode::Off => { self.level = 1; }\n            Mode::Low => { self.level = 2; }\n            _ => { self.level = 3; }\n        }\n    }")
        .replace(
            "enum Mode { Off, Low, High }\n",
            "use crate::shared::wide::Wider as Mode;\n",
        );
    let files = |user: &str| {
        lint_v2_sources(&[
            LintInput {
                name: "wide.antics".to_owned(),
                source: "pub enum Wider { Off, Low, High }\n".to_owned(),
                ..LintInput::default()
            },
            LintInput {
                name: "narrow.antics".to_owned(),
                source: "pub enum Mode { Off, Low }\n".to_owned(),
                ..LintInput::default()
            },
            LintInput {
                name: "user.antics".to_owned(),
                source: user.to_owned(),
                ..LintInput::default()
            },
        ])
        .unwrap()
    };
    let found = files(&user);
    assert!(
        !found.iter().any(|d| d.rule == "enum_wildcard_all_covered"),
        "{found:#?}"
    );
    let single: Vec<_> = found
        .iter()
        .filter(|d| d.rule == "enum_wildcard_single_variant")
        .collect();
    assert_eq!(single.len(), 1, "{found:#?}");
    assert_eq!(single[0].suggestion.as_deref(), Some("Mode::High"));
    // A name neither declared nor imported is not judged.
    let unimported = user.replace("use crate::shared::wide::Wider as Mode;\n", "");
    let found = files(&unimported);
    assert!(
        !found.iter().any(|d| d.rule.starts_with("enum_wildcard")),
        "{found:#?}"
    );
}

#[test]
fn enum_wildcard_all_covered_cannot_be_allowed() {
    let match_stmt = "        match self.mode {\n            Mode::Off => { self.level = 1; }\n            Mode::Low => { self.level = 2; }\n            Mode::High => { self.level = 3; }\n";
    for allowed in [
        format!(
            "{match_stmt}            // antics-lint: allow(enum_wildcard_all_covered)\n            _ => {{ self.level = 4; }}\n        }}"
        ),
        format!(
            "{match_stmt}            _ => {{ self.level = 4; }} // antics-lint: allow(enum_wildcard_all_covered)\n        }}"
        ),
    ] {
        let source = tree(&allowed);
        let d = one(&source, "enum_wildcard_all_covered");
        assert_eq!(d.severity, Severity::Error);
        let note = d.note.unwrap();
        assert!(note.contains("cannot be allowed"), "{note}");
        assert!(note.contains("`i16`"), "{note}");
    }
    let file_level = format!(
        "//! antics-lint: allow(enum_wildcard_all_covered)\n{}",
        tree(&format!(
            "{match_stmt}            _ => {{ self.level = 4; }}\n        }}"
        ))
    );
    one(&file_level, "enum_wildcard_all_covered");
}

// --- dropped_bool_call ------------------------------------------------------------------

#[test]
fn dropped_bool_and_result_calls_are_flagged() {
    let source = format!(
        "extern mod synth_globals {{\n    @id(256) fn try_light(power: i16) -> Result;\n}}\n{}",
        obj(
            "    fn is_bright() -> bool { return self.level > 5; }\n    \
             tree check \"Check\" -> bool { return self.level > 1; }\n    \
             tree probe \"Probe\" {\n        self.level = 0;\n        self.mode = Mode::Off;\n        \
             is_bright();\n        check();\n        synth_globals::try_light(3);\n        \
             can_afford(5);\n        stack_obj.is_type(0x7E00_0200);\n    }"
        )
    );
    let found = of(&source, "dropped_bool_call");
    let lines: Vec<usize> = found.iter().map(|d| d.line).collect();
    assert_eq!(
        lines,
        [
            line_of(&source, "is_bright();"),
            line_of(&source, "try_light(3);"),
            line_of(&source, "can_afford(5);"),
            line_of(&source, "is_type(0x7E00_0200);"),
        ],
        "{found:#?}"
    );
    assert!(found[1].message.contains("`Result`"), "{:?}", found[1]);
    assert!(found[1].help.as_deref().unwrap().contains('?'));
}

#[test]
fn used_bool_calls_and_out_param_helpers_are_fine() {
    let source = obj(
        "    fn is_bright() -> bool { return self.level > 5; }\n    \
         fn bump(n: &mut i16) -> bool { *n += 1; return *n > 3; }\n    \
         fn touch() { self.level += 1; }\n    \
         tree probe \"Probe\" {\n        self.level = 0;\n        self.mode = Mode::Off;\n        \
         if is_bright() { self.level = 1; }\n        is_bright()?;\n        touch();\n        \
         bump(&mut self.level);\n        debit_funds(5);\n        return is_bright();\n    }",
    );
    none(&source, "dropped_bool_call");
}

// --- unbounded_loop_without_yield / while_true ------------------------------------------

#[test]
fn loop_that_never_yields_or_exits_is_flagged() {
    let source =
        tree("        loop {\n            self.level += 1;\n            play_sound(0);\n        }");
    let d = one(&source, "unbounded_loop_without_yield");
    assert_eq!(d.line, line_of(&source, "loop {"));
}

#[test]
fn loops_that_yield_exit_or_call_unknown_code_are_fine() {
    for body in [
        "        loop {\n            sleep(1);\n        }",
        "        loop {\n            idle();\n        }",
        "        loop {\n            if self.level > 3 { break; }\n            self.level += 1;\n        }",
        "        loop {\n            if self.level > 3 { return true; }\n            self.level += 1;\n        }",
        "        loop {\n            (self.level < 3)?;\n            self.level += 1;\n        }",
        "        loop {\n            raw { idle(1); }\n        }",
        "        loop {\n            other_tree();\n        }",
        "        while self.level < 3 {\n            self.level += 1;\n        }",
    ] {
        none(&tree(body), "unbounded_loop_without_yield");
    }
    // A `break` of an inner loop does not end the outer one.
    let nested = tree(
        "        loop {\n            loop {\n                break;\n            }\n            self.level += 1;\n        }",
    );
    one(&nested, "unbounded_loop_without_yield");
}

#[test]
fn while_true_is_flagged_and_loop_is_not() {
    let source = tree("        while true {\n            sleep(1);\n        }");
    let d = one(&source, "while_true");
    assert_eq!(d.suggestion.as_deref(), Some("loop"));
    none(
        &tree("        loop {\n            sleep(1);\n        }"),
        "while_true",
    );
}

// --- stack_obj_clobber_after_self --------------------------------------------------------

#[test]
fn self_after_stack_obj_reassignment_is_flagged_once() {
    let source = sim_tree(
        "        stack_obj = sim;\n        self.level = 1;\n        self.level = 2;\n        stack_obj = self;\n        self.level = 3;",
    );
    let d = one(&source, "stack_obj_clobber_after_self");
    assert_eq!(d.line, line_of(&source, "self.level = 1;"));
    assert!(
        d.message
            .contains(&format!("line {}", line_of(&source, "stack_obj = sim;")))
    );
}

#[test]
fn self_in_a_nested_block_and_after_find_best_interaction_is_flagged() {
    let nested = sim_tree("        stack_obj = sim;\n        if self.level > 2 { sleep(1); }");
    one(&nested, "stack_obj_clobber_after_self");
    let inner = sim_tree(
        "        stack_obj = sim;\n        loop {\n            self.level += 1;\n            sleep(1);\n        }",
    );
    one(&inner, "stack_obj_clobber_after_self");
    let found =
        sim_tree("        if find_best_interaction() {\n            self.level = 1;\n        }");
    one(&found, "stack_obj_clobber_after_self");
}

#[test]
fn restored_or_object_context_self_is_fine() {
    none(
        &sim_tree("        stack_obj = sim;\n        stack_obj = self;\n        self.level = 1;"),
        "stack_obj_clobber_after_self",
    );
    none(
        &sim_tree("        stack_obj = stack_obj;\n        self.level = 1;"),
        "stack_obj_clobber_after_self",
    );
    // An object-context tree: `self` is always this object.
    none(
        &tree("        stack_obj = 0;\n        self.level = 1;"),
        "stack_obj_clobber_after_self",
    );
    // A restore inside a nested block ends the clobber.
    none(
        &sim_tree(
            "        stack_obj = sim;\n        if sim.motive.energy > 0 {\n            stack_obj = self;\n        }\n        self.level = 1;",
        ),
        "stack_obj_clobber_after_self",
    );
}

// --- unused_decl --------------------------------------------------------------------------

#[test]
fn unused_locals_states_consts_and_bcon_entries_are_flagged() {
    let source = obj(
        "    const SPARE = 4;\n    const USED = 2;\n    state spare_state: i16;\n    \
         bcon 4097 \"Tuning\" { low = 1, high = 9 }\n    \
         tree probe \"Probe\" {\n        self.level = Tuning.low + USED;\n        self.mode = Mode::Off;\n        \
         let unused = random(5);\n        let used = random(5);\n        self.level += used;\n    }",
    );
    let found = of(&source, "unused_decl");
    let mut messages: Vec<&str> = found.iter().map(|d| d.message.as_str()).collect();
    messages.sort_unstable();
    assert_eq!(
        messages,
        [
            "BCON entry `high` of table 4097 is never read",
            "constant `SPARE` is never used",
            "local `unused` is never read",
            "state `spare_state` is never used",
        ],
        "{found:#?}"
    );
}

#[test]
fn locals_are_not_judged_when_raw_code_addresses_locals_by_number() {
    one(
        &tree("        let x = 7;\n        raw { temp[1] := temp[0]; }"),
        "unused_decl",
    );
    none(
        &tree("        let x = 7;\n        raw { temp[1] := local[1]; }"),
        "unused_decl",
    );
    none(
        &tree("        let x: i16;\n        raw { local [0] := 2; }\n        self.level = x;"),
        "unused_decl",
    );
}

#[test]
fn bcon_entries_are_not_judged_when_something_reads_them_by_number() {
    let members = "    bcon 4097 \"Tuning\" { low = 1, high = 9 }\n    \
         tree probe \"Probe\" {\n        self.level = Tuning.low;\n        self.mode = Mode::Off;\n";
    let flagged = obj(&format!("{members}    }}"));
    assert_eq!(of(&flagged, "unused_decl").len(), 1, "{flagged}");
    for read in ["bcon[129]", "bcon [129]"] {
        let raw = obj(&format!(
            "{members}        raw {{ temp[0] := {read}; }}\n    }}"
        ));
        none(&raw, "unused_decl");
    }
    let external = format!(
        "extern mod synth_globals {{\n    @id(256) fn tune() -> bool;\n}}\n{}",
        obj(&format!("{members}        synth_globals::tune();\n    }}"))
    );
    none(&external, "unused_decl");
    // A `-> bool` extern called for what it does is not a dropped result either.
    none(&external, "dropped_bool_call");
}

#[test]
fn uninitialized_local_read_before_assignment_is_flagged() {
    let source = tree(
        "        let x: i16;\n        self.level = x;\n        x = 3;\n        self.level += x;",
    );
    let d = one(&source, "unused_decl");
    assert_eq!(d.line, line_of(&source, "self.level = x;"));
    assert!(d.message.contains("before anything assigns it"), "{d:?}");
    // Assigned on some path first, or through `&mut`: not flagged.
    none(
        &tree("        let x: i16;\n        if self.level > 1 { x = 1; }\n        self.level = x;"),
        "unused_decl",
    );
    none(
        &obj(
            "    fn put(v: &mut i16) { *v = 2; }\n    tree probe \"Probe\" {\n        self.mode = Mode::Off;\n        let x: i16;\n        put(&mut x);\n        self.level = x;\n    }",
        ),
        "unused_decl",
    );
}

#[test]
fn used_underscored_raw_and_shared_declarations_are_fine() {
    let source = obj("    state slot_state @ 6: i16;\n    \
         bcon 4097 \"Tuning\" { low = 1, high = 9 }\n    \
         tree probe \"Probe\" {\n        self.mode = Mode::Off;\n        \
         let _ignored = random(5);\n        let in_raw = random(5);\n        \
         raw { my.attr[6] := in_raw; }\n        self.level = self.attr[6];\n    }");
    none(&format!("pub const SHARED = 4;\n{source}"), "unused_decl");
    // A state another file of the set reads is used.
    let other = "fn reads_spare() -> i16 { return self.spare_state; }\n";
    let inputs = [
        LintInput {
            name: "a.antics".to_owned(),
            source: tree("").replace(
                "state mode: Mode;",
                "state mode: Mode;\n    state spare_state: i16;",
            ),
            ..LintInput::default()
        },
        LintInput {
            name: "b.antics".to_owned(),
            source: other.to_owned(),
            ..LintInput::default()
        },
    ];
    let found = lint_v2_sources(&inputs).expect("lint");
    assert!(
        !found.iter().any(|d| d.message.contains("spare_state")),
        "{found:#?}"
    );
}

// --- frame_headroom ----------------------------------------------------------------------

#[test]
fn big_trees_and_many_locals_are_flagged() {
    let many_nodes: String = (0..205).map(|_| "        self.level += 1;\n").collect();
    let source = obj(&format!(
        "    tree big \"Big Tree\" {{\n        self.mode = Mode::Off;\n{many_nodes}    }}"
    ));
    let d = one(&source, "frame_headroom");
    assert!(d.message.contains("nodes"), "{d:?}");
    assert_eq!(d.line, line_of(&source, "tree big"));

    let lets: String = (0..14)
        .map(|i| format!("        let v{i} = random(9);\n"))
        .collect();
    let sum: Vec<String> = (0..14).map(|i| format!("v{i}")).collect();
    let source = obj(&format!(
        "    tree wide \"Wide Tree\" {{\n        self.mode = Mode::Off;\n{lets}        self.level = {};\n    }}",
        sum.join(" + ")
    ));
    let d = one(&source, "frame_headroom");
    assert!(d.message.contains("locals"), "{d:?}");
}

#[test]
fn frame_headroom_points_at_the_tree_in_its_own_object() {
    // Two objects in one file, each with a tree labeled "Shared"; only the first is big.
    let many_nodes: String = (0..205).map(|_| "        self.level += 1;\n").collect();
    let big = obj(&format!(
        "    @id(4200) tree shared \"Shared\" {{\n        self.mode = Mode::Off;\n{many_nodes}    }}"
    ));
    let small = obj("    @id(4201) tree shared \"Shared\" {\n        self.mode = Mode::Off;\n        self.level += 1;\n    }")
        .replace("enum Mode { Off, Low, High }\n", "")
        .replace("Synth Probe", "Synth Probe Two")
        .replace("0x7E00_0100", "0x7E00_0101");
    let source = format!("{big}{small}");
    let d = one(&source, "frame_headroom");
    assert!(d.message.contains("tree 4200"), "{d:?}");
    assert_eq!(d.line, line_of(&source, "@id(4200)"), "{d:?}");
}

#[test]
fn small_trees_have_headroom() {
    none(&tree("        self.level += 1;"), "frame_headroom");
}

// --- redundant_bool_return ----------------------------------------------------------------

#[test]
fn if_returning_bool_literals_is_flagged() {
    let source = obj(
        "    fn high() -> bool {\n        if self.level > 3 { return true; }\n        return false;\n    }",
    );
    let d = one(&source, "redundant_bool_return");
    assert_eq!(d.suggestion.as_deref(), Some("return self.level > 3;"));
    let inverted = obj(
        "    fn low() -> bool {\n        if self.level > 3 { return false; } else { return true; }\n    }",
    );
    assert_eq!(
        one(&inverted, "redundant_bool_return")
            .suggestion
            .as_deref(),
        Some("return !(self.level > 3);")
    );
}

#[test]
fn if_returning_the_same_bool_twice_is_fine() {
    none(
        &obj(
            "    fn high() -> bool {\n        if self.level > 3 { return true; }\n        return true;\n    }",
        ),
        "redundant_bool_return",
    );
    none(
        &obj(
            "    fn high() -> bool {\n        if self.level > 3 { self.level = 0; return true; }\n        return false;\n    }",
        ),
        "redundant_bool_return",
    );
}

// --- collapsible_if_chain -----------------------------------------------------------------

#[test]
fn three_equality_tests_of_one_place_are_flagged_once() {
    let source = tree(
        "        if self.level == 1 {\n            sleep(1);\n        } else if self.level == 2 {\n            sleep(2);\n        } else if self.level == 3 {\n            sleep(3);\n        } else {\n            sleep(4);\n        }",
    );
    let d = one(&source, "collapsible_if_chain");
    assert_eq!(d.line, line_of(&source, "if self.level == 1"));
    assert!(
        d.help.as_deref().unwrap().contains("match self.level"),
        "{d:?}"
    );
}

#[test]
fn short_or_mixed_chains_are_fine() {
    for body in [
        "        if self.level == 1 {\n            sleep(1);\n        } else if self.level == 2 {\n            sleep(2);\n        }",
        "        if self.level == 1 {\n            sleep(1);\n        } else if temp[0] == 2 {\n            sleep(2);\n        } else if self.level == 3 {\n            sleep(3);\n        }",
        "        if self.level == 1 {\n            sleep(1);\n        } else if self.level > 2 {\n            sleep(2);\n        } else if self.level == 3 {\n            sleep(3);\n        }",
        "        if self.level == 1 {\n            sleep(1);\n        } else if self.level == 1 {\n            sleep(2);\n        } else if self.level == 3 {\n            sleep(3);\n        }",
        // `match` arms take only literals and enum variants.
        "        let a = random(10);\n        let b = random(10);\n        let c = random(10);\n        if self.level == a {\n            sleep(1);\n        } else if self.level == b {\n            sleep(2);\n        } else if self.level == c {\n            sleep(3);\n        }",
    ] {
        none(&tree(body), "collapsible_if_chain");
    }
}

// --- manual_raw_has_builtin ---------------------------------------------------------------

#[test]
fn raw_blocks_of_native_primitives_are_flagged() {
    let source =
        tree("        raw {\n            notify_idle();\n            breakpoint();\n        }");
    let d = one(&source, "manual_raw_has_builtin");
    let help = d.help.unwrap();
    assert!(help.contains("notify_out_of_idle()"), "{help}");
    assert!(help.contains("breakpoint(...)"), "{help}");
    one(
        &tree(
            "        raw { special_effect(duration: 10, kind: 1, size: 2, flags: 0x01, str_index: 0); # sparkle\n }",
        ),
        "manual_raw_has_builtin",
    );
}

#[test]
fn raw_blocks_v2_cannot_spell_are_fine() {
    for raw in [
        "        raw { op 0x002c (0x03, 0x00, 0x15, 0x00); }",
        "        raw { notify_idle() pad 0xff; }",
        "        raw { random(temp[0], 1) -> true: true, false: true; }",
        "        raw { notify_idle(); my.attr[0] := 1; }",
        "        raw { play_sound(1) pad(3: 0x01, 4: 0x01); }",
        // v2 spells these with different operands (a declared sound, a context target).
        "        raw { play_sound(1, stack_obj); }",
        "        raw { stop_all_sounds(true); }",
        "        raw { push_interaction(menu: 0, object: 0, priority: 0, flags: 7); }",
        "        raw { test_object_type(0x7e000100, stack_obj); }",
        "        raw { stack_obj := stack.slot[0]; }",
        // Operand forms v2's builtin rejects: a local-slot flag, a non-literal operand.
        "        raw { special_effect(duration: 0, flags: 32); }",
        "        raw { show_string(table: 300, index: local[0]); }",
        "        raw { breakpoint(temp[0]); }",
    ] {
        none(&tree(raw), "manual_raw_has_builtin");
    }
}

// --- magic_guid_literal -------------------------------------------------------------------

#[test]
fn inline_guid_literals_in_bodies_are_flagged() {
    let source = tree("        if stack_obj.is_type(0x7E00_0200) { sleep(1); }");
    let d = one(&source, "magic_guid_literal");
    assert!(d.message.contains("0x7E00_0200"), "{d:?}");
    // The object's own `guid:` and small or decimal literals are not.
    none(
        &tree("        self.level = 0x7FFF;\n        self.level = 100000;"),
        "magic_guid_literal",
    );
    none(
        &obj(
            "    const OTHER = 0x7E00_0200;\n    tree probe \"Probe\" {\n        self.mode = Mode::Off;\n        if stack_obj.is_type(OTHER) { self.level = 1; }\n    }",
        ),
        "magic_guid_literal",
    );
}

// --- negated_comparison -------------------------------------------------------------------

#[test]
fn negated_comparisons_are_flagged_with_a_polarity_note() {
    let source = tree(
        "        if !(self.level == 3) { sleep(1); }\n        if !(self.level < temp[0]) { sleep(1); }",
    );
    let found = of(&source, "negated_comparison");
    let suggestions: Vec<&str> = found
        .iter()
        .filter_map(|d| d.suggestion.as_deref())
        .collect();
    assert_eq!(suggestions, ["self.level != 3", "self.level >= temp[0]"]);
    assert!(found[0].note.as_deref().unwrap().contains("polarity"));
}

#[test]
fn plain_negation_is_fine() {
    none(
        &obj(
            "    state on: bool;\n    tree probe \"Probe\" {\n        self.mode = Mode::Off;\n        self.level = 0;\n        if !self.on { sleep(1); }\n        if !(self.on && self.level > 0) { sleep(1); }\n    }",
        ),
        "negated_comparison",
    );
}

// --- suppression --------------------------------------------------------------------------

#[test]
fn allow_directives_suppress_by_line_and_file() {
    let flagged = "        while true {\n            sleep(1);\n        }";
    // Own line: covers the next line of code (blank lines and comments in between are skipped).
    none(
        &tree(&format!(
            "        // antics-lint: allow(while_true)\n\n        // why\n{flagged}"
        )),
        "while_true",
    );
    // Trailing: covers its own line.
    none(
        &tree(
            "        while true { // antics-lint: allow(while_true, negated_comparison)\n            sleep(1);\n        }",
        ),
        "while_true",
    );
    // File level.
    none(
        &format!("//! antics-lint: allow(while_true)\n{}", tree(flagged)),
        "while_true",
    );
    // A directive for another rule, or another line, does not.
    one(
        &tree(&format!(
            "        // antics-lint: allow(negated_comparison)\n{flagged}"
        )),
        "while_true",
    );
    one(
        &tree(&format!(
            "        sleep(1); // antics-lint: allow(while_true)\n{flagged}"
        )),
        "while_true",
    );
    // Not a directive inside a string.
    one(
        &tree(&format!(
            "        dialog(\"// antics-lint: allow(while_true)\");\n{flagged}"
        )),
        "while_true",
    );
    // Nor inside a `raw { }` body, which follows `.air` comment rules.
    one(
        &tree(&format!(
            "        raw {{\n            //! antics-lint: allow(while_true)\n            notify_idle() pad 0xff;\n        }}\n{flagged}"
        )),
        "while_true",
    );
    // An `.air` `#` comment holding a quote does not hide a directive after the block.
    none(
        &tree(&format!(
            "        raw {{\n            notify_idle() pad 0xff; # \"\n        }}\n        // antics-lint: allow(while_true)\n{flagged}"
        )),
        "while_true",
    );
}

#[test]
fn unknown_rule_names_in_directives_are_reported() {
    let source = tree("        // antics-lint: allow(no_such_rule)\n        sleep(1);");
    let d = one(&source, "unknown_lint_rule");
    assert_eq!(d.line, line_of(&source, "no_such_rule"));
    assert!(d.message.contains("no_such_rule"));
}

// --- output ---------------------------------------------------------------------------------

#[test]
fn diagnostics_render_like_rustc_and_as_json() {
    let source = tree("        while true {\n            sleep(1);\n        }");
    let d = one(&source, "while_true");
    let text = render_diagnostic(&d, Some(&source));
    let line = line_of(&source, "while true");
    assert!(text.starts_with("warning[while_true]: "), "{text}");
    assert!(
        text.contains(&format!("--> probe.antics:{line}:9\n")),
        "{text}"
    );
    assert!(
        text.contains(&format!("{line} |         while true {{\n")),
        "{text}"
    );
    assert!(text.contains("|         ^\n"), "{text}");
    assert!(text.contains("= help: "), "{text}");
    let json = diagnostics_to_json(&[d]);
    assert!(
        json.starts_with(
            "[{\"rule\":\"while_true\",\"severity\":\"warning\",\"file\":\"probe.antics\""
        ),
        "{json}"
    );
    assert!(
        json.contains(&format!("\"line\":{line},\"column\":9")),
        "{json}"
    );
    assert!(json.contains("\"suggestion\":\"loop\""), "{json}");
    assert!(json.contains("\"note\":null"), "{json}");
    assert_eq!(diagnostics_to_json(&[]), "[]");
}

#[test]
fn a_file_that_does_not_parse_is_an_error() {
    assert!(lint_v2_source("object \"Broken\" {", "broken.antics").is_err());
}

#[test]
fn open_pack_lints_without_errors() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../open-pack");
    let (found, sources) = sims_antics::v2::lint::lint_v2_paths(&[root]).expect("open-pack lints");
    assert!(sources.len() >= 20, "{} files", sources.len());
    assert!(
        found.iter().all(|d| d.severity == Severity::Warning),
        "{found:#?}"
    );
}

// --- CLI ------------------------------------------------------------------------------------

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("v2_lint")
        .join(name);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn cli(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sims-antics"))
        .arg("lint")
        .args(args)
        .output()
        .expect("run sims-antics")
}

#[test]
fn cli_exit_codes_and_json() {
    let dir = scratch("cli");
    let clean = dir.join("clean.antics");
    let warn = dir.join("nested").join("warn.antics");
    let error = dir.join("error.antics");
    std::fs::create_dir_all(warn.parent().unwrap()).unwrap();
    std::fs::write(&clean, tree("        sleep(1);")).unwrap();
    std::fs::write(
        &warn,
        tree("        while true {\n            sleep(1);\n        }"),
    )
    .unwrap();
    std::fs::write(
        &error,
        tree("        self.level = match self.mode { Mode::Off => 1, Mode::Low => 2, Mode::High => 3, _ => 4 };"),
    )
    .unwrap();

    let out = cli(&[clean.as_os_str()]);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    let out = cli(&[warn.as_os_str()]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("warning[while_true]"), "{text}");
    assert!(text.contains("warn.antics:"), "{text}");

    let out = cli(&["--deny-warnings".as_ref(), warn.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));

    let out = cli(&[error.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("error[enum_wildcard_all_covered]"));

    // A directory is walked; `--json` prints one array.
    let out = cli(&["--json".as_ref(), dir.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    let json = String::from_utf8(out.stdout).unwrap();
    assert!(
        json.trim_end().starts_with('[') && json.trim_end().ends_with(']'),
        "{json}"
    );
    assert!(json.contains("\"rule\":\"while_true\""), "{json}");
    assert!(
        json.contains("\"rule\":\"enum_wildcard_all_covered\",\"severity\":\"error\""),
        "{json}"
    );

    let out = cli(&["--bogus".as_ref()]);
    assert_ne!(out.status.code(), Some(0));
    let out = cli(&[]);
    assert_ne!(out.status.code(), Some(0));
}

#[test]
fn cli_lints_files_that_start_with_const() {
    let dir = scratch("cli-const");
    let good = dir.join("good.antics");
    let bad = dir.join("bad.antics");
    std::fs::write(
        &good,
        format!(
            "const SPARE = 3;\n{}",
            tree("        while true {\n            sleep(1);\n        }")
        ),
    )
    .unwrap();
    std::fs::write(&bad, "const BAD = ;\n").unwrap();

    let out = cli(&["--json".as_ref(), good.as_os_str()]);
    let json = String::from_utf8(out.stdout).unwrap();
    assert!(json.contains("\"while_true\""), "{json}");
    assert!(json.contains("SPARE"), "{json}");

    let out = cli(&["--json".as_ref(), bad.as_os_str()]);
    assert_ne!(out.status.code(), Some(0));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("bad.antics"), "{err}");
}
