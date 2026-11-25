//! `.antics` v2 linter (`sims-antics lint`).
//!
//! Every fixture here is synthetic (GUIDs in the clean-room 0x7E00_xxxx range). Each rule
//! has at least one positive and one negative case.

use sims_antics::v2::lint::{
    ENUM_WILDCARD_ALL_COVERED, LINT_RULES, LintDiagnostic, LintInput, Severity,
    diagnostics_to_json, lint_v2_paths, lint_v2_source, lint_v2_sources, render_diagnostic,
};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Wraps object members in a synthetic object.
fn obj(members: &str) -> String {
    format!(
        "enum Mode {{ Off, Low, High }}\n\
         object \"Synth Probe\" {{\n    guid: 0x7E00_0100;\n    price: 1;\n    \
         sprite: Sprite::procedural(\"floor_lamp\", primary: #101010, accent: #202020, trim: #303030);\n    \
         #[slot(2)] state level: i16;\n    state mode: Mode;\n{members}\n}}\n"
    )
}

/// Wraps statements in an object-context tree that uses `level` and `mode`.
fn tree(body: &str) -> String {
    obj(&format!(
        "    #[bhav(label = \"Probe\")] fn probe() {{\n        self.level = 0;\n        self.mode = Mode::Off;\n{body}\n    }}"
    ))
}

/// Wraps statements in a Sim-context tree.
fn sim_tree(body: &str) -> String {
    obj(&format!(
        "    #[bhav(label = \"Probe\")] fn probe(sim: Sim) {{\n        self.level = 0;\n        self.mode = Mode::Off;\n{body}\n    }}"
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
fn the_rule_table_lists_every_rule() {
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
        "uninitialized_local_read",
        "dialog_local_slot_mismatch",
        "raw_bare_call_false_exit",
        "missing_sim_frame_args",
        "shadowed_const",
        "duplicate_std_global",
        "untyped_engine_field",
        "raw_syntax",
        "object_tree_me_field",
    ] {
        assert!(names.contains(&expected), "{expected} missing");
    }
    let unsuppressible: Vec<&str> = LINT_RULES
        .iter()
        .filter(|r| !r.suppressible)
        .map(|r| r.name)
        .collect();
    assert_eq!(
        unsuppressible,
        [
            ENUM_WILDCARD_ALL_COVERED,
            "raw_syntax",
            "object_tree_me_field"
        ]
    );
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
fn wildcard_rules_count_every_alternative_of_an_or_pattern() {
    let single = tree(
        "        match self.mode {\n            Mode::Off | Mode::Low => { self.level = 1; }\n            _ => { self.level = 3; }\n        }",
    );
    assert_eq!(
        one(&single, "enum_wildcard_single_variant")
            .suggestion
            .as_deref(),
        Some("Mode::High")
    );
    let all = tree(
        "        self.level = match self.mode { Mode::Off | Mode::Low => 1, Mode::High => 2, _ => 4 };",
    );
    one(&all, "enum_wildcard_all_covered");
    none(&all, "enum_wildcard_single_variant");
    // Alternatives from another enum, or ints, leave the arm alone.
    for arm in ["Mode::Off | Other::Low", "Mode::Off | 2"] {
        let source = tree(&format!(
            "        match self.mode {{\n            {arm} => {{ self.level = 1; }}\n            Mode::Low => {{ self.level = 2; }}\n            _ => {{ self.level = 3; }}\n        }}"
        ));
        let found = lint(&source);
        assert!(
            !found.iter().any(|d| d.rule.starts_with("enum_wildcard")),
            "{source}\n{found:#?}"
        );
    }
}

#[test]
fn if_expressions_are_walked_like_other_expressions() {
    // A local read only inside an if-expression is used.
    none(
        &tree("        let x = random(5);\n        self.level = if x > 2 { 1 } else { 2 };"),
        "unused_decl",
    );
    none(
        &tree(
            "        let x = random(5);\n        self.level = if self.level > 2 { 1 } else if self.level > 1 { x } else { 3 };",
        ),
        "unused_decl",
    );
    // A match nested in an if-expression is still checked.
    one(
        &tree(
            "        self.level = if self.level > 0 { match self.mode { Mode::Off => 1, Mode::Low => 2, _ => 3 } } else { 0 };",
        ),
        "enum_wildcard_single_variant",
    );
}

#[test]
fn wildcard_rules_resolve_the_enum_through_imports() {
    // `Mode` here is `Wider` imported under another name; an unrelated two-variant `Mode`
    // elsewhere in the set must not make the `_` arm look redundant.
    let user = obj("    #[bhav(label = \"Probe\")] fn probe() {\n        match self.mode {\n            Mode::Off => { self.level = 1; }\n            Mode::Low => { self.level = 2; }\n            _ => { self.level = 3; }\n        }\n    }")
        .replace(
            "enum Mode { Off, Low, High }\n",
            "use crate::shared::wide::Wider as Mode;\n",
        );
    // A pack on disk: the import resolves to `shared/wide.antics`, which is not among the
    // linted files; an unrelated `Wider` and `Mode` are.
    let dir = scratch("enum-imports");
    std::fs::create_dir_all(dir.join("shared")).unwrap();
    std::fs::create_dir_all(dir.join("objects")).unwrap();
    std::fs::write(dir.join("pack.toml"), "[pack]\nname = \"Synth Pack\"\n").unwrap();
    std::fs::write(
        dir.join("shared").join("wide.antics"),
        "pub enum Wider { Off, Low, High }\n",
    )
    .unwrap();
    let narrow = dir.join("narrow.antics");
    std::fs::write(
        &narrow,
        "pub enum Wider { Off, Low }\npub enum Mode { Off, Low }\n",
    )
    .unwrap();
    let files = |user: &str| {
        let path = dir.join("objects").join("user.antics");
        std::fs::write(&path, user).unwrap();
        lint_v2_paths(&[path, narrow.clone()]).unwrap().0
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
    // Without a pack the import cannot be resolved, so it is not judged either.
    let found = lint_v2_sources(&[
        LintInput {
            name: "narrow.antics".to_owned(),
            source: "pub enum Wider { Off, Low }\n".to_owned(),
            ..LintInput::default()
        },
        LintInput {
            name: "user.antics".to_owned(),
            source: user.clone(),
            ..LintInput::default()
        },
    ])
    .unwrap();
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
        "extern mod synth_globals {{\n    #[id(256)] fn try_light(power: i16) -> Result;\n}}\n{}",
        obj(
            "    fn is_bright() -> bool { return self.level > 5; }\n    \
             #[bhav(label = \"Check\")]
             fn check() -> bool { return self.level > 1; }\n    \
             #[bhav(label = \"Probe\")]
             fn probe() {\n        self.level = 0;\n        self.mode = Mode::Off;\n        \
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
         #[bhav(label = \"Probe\")]
         fn probe() {\n        self.level = 0;\n        self.mode = Mode::Off;\n        \
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
    // A handle saved before the swap restores it too.
    none(
        &sim_tree(
            "        let saved = stack_obj;\n        stack_obj = sim;\n        stack_obj = saved;\n        self.level = 1;",
        ),
        "stack_obj_clobber_after_self",
    );
    // A local saved after the swap holds the other object: assigning it back is no restore.
    let late = sim_tree(
        "        stack_obj = sim;\n        let other = stack_obj;\n        stack_obj = other;\n        self.level = 1;",
    );
    let d = one(&late, "stack_obj_clobber_after_self");
    assert!(
        d.help.as_deref().unwrap().contains("stack_obj.<field>"),
        "{d:?}"
    );
    // A handle saved in a nested block does not outlive it: the outer `saved` is the Sim.
    one(
        &sim_tree(
            "        let saved = sim;\n        if self.level > 0 {\n            let saved = stack_obj;\n            sleep(1);\n        }\n        stack_obj = sim;\n        stack_obj = saved;\n        self.level = 1;",
        ),
        "stack_obj_clobber_after_self",
    );
    // A store to the outer binding in a nested block is kept.
    one(
        &sim_tree(
            "        let saved = stack_obj;\n        if temp[0] != 0 {\n            saved = sim;\n        } else {\n            saved = sim;\n        }\n        stack_obj = sim;\n        stack_obj = saved;\n        self.level = 1;",
        ),
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
         #[bhav(label = \"Probe\")]
         fn probe() {\n        self.level = Tuning.low + USED;\n        self.mode = Mode::Off;\n        \
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
    none(
        &tree("        let x = 7;\n        raw { temp[1] := local /* gap */ [1]; }"),
        "unused_decl",
    );
    // A mention in an `.air` comment is not an address.
    one(
        &tree(
            "        let x = 7;\n        raw {\n            temp[1] := 2; # not local[1]\n        }",
        ),
        "unused_decl",
    );
}

#[test]
fn bcon_entries_are_not_judged_when_something_reads_them_by_number() {
    let members = "    bcon 4097 \"Tuning\" { low = 1, high = 9 }\n    \
         #[bhav(label = \"Probe\")]
         fn probe() {\n        self.level = Tuning.low;\n        self.mode = Mode::Off;\n";
    let flagged = obj(&format!("{members}    }}"));
    assert_eq!(of(&flagged, "unused_decl").len(), 1, "{flagged}");
    for read in [
        "bcon[129]",
        "bcon [129]",
        "bcon /* gap */ [129]",
        "const[129]",
    ] {
        let raw = obj(&format!(
            "{members}        raw {{ temp[0] := {read}; }}\n    }}"
        ));
        none(&raw, "unused_decl");
    }
    let external = format!(
        "extern mod synth_globals {{\n    #[id(256)] fn tune() -> bool;\n}}\n{}",
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
    let d = one(&source, "uninitialized_local_read");
    assert_eq!(d.line, line_of(&source, "self.level = x;"));
    assert!(d.message.contains("before anything assigns it"), "{d:?}");
    none(&source, "unused_decl");
    // Assigned on some path first, or through `&mut`: not flagged.
    none(
        &tree("        let x: i16;\n        if self.level > 1 { x = 1; }\n        self.level = x;"),
        "uninitialized_local_read",
    );
    none(
        &obj(
            "    fn put(v: &mut i16) { *v = 2; }\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        let x: i16;\n        put(&mut x);\n        self.level = x;\n    }",
        ),
        "uninitialized_local_read",
    );
}

#[test]
fn a_slot_pinned_local_is_neither_unread_nor_uninitialized() {
    // `#[slot(N)]` pins the frame word for code that addresses it by number, which may
    // fill or read it: no `uninitialized_local_read`, no `unused_decl`.
    for body in [
        "        #[slot(2)] let x: i16;\n        self.level = x;\n        x = 3;\n        self.level += x;",
        "        #[slot(2)] let y: i16 = 1;\n        self.level = 2;",
    ] {
        let source = tree(body);
        none(&source, "uninitialized_local_read");
        none(&source, "unused_decl");
        assert!(lint(&source).is_empty(), "{body}: {:?}", lint(&source));
    }
    // An unpinned one still is.
    one(
        &tree(
            "        let x: i16;\n        self.level = x;\n        x = 3;\n        self.level += x;",
        ),
        "uninitialized_local_read",
    );
}

#[test]
fn builtins_that_write_a_local_assign_it() {
    // `animate(local:)` and `random_into(x, n)` write their target: no read-before-assign,
    // and the target is not unused.
    let source = sim_tree(
        "        let event: i16;\n        animate(clip: 1, table: 0, events: 2, local: event) |_e| {\n            if event == 0 { sim.person[1] = 1; }\n        }\n        let roll: i16;\n        random_into(roll, 5)?;\n        self.level = roll;",
    );
    none(&source, "uninitialized_local_read");
    none(&source, "unused_decl");
    // `set_headline(local:)` reads its local.
    one(
        &sim_tree(
            "        let x: i16;\n        set_headline(kind: 1, index: 0, duration: 5, options: 0, local: x);\n        self.level = x;",
        ),
        "uninitialized_local_read",
    );
    // A read in another argument of the same call still counts as a read.
    let source = tree(
        "        let roll: i16;\n        random_into(roll, roll)?;\n        self.level = roll;",
    );
    one(&source, "uninitialized_local_read");
}

#[test]
fn relationship_reads_into_its_value_local() {
    // Without flag 4 `relationship(value:)` reads the number into the local.
    for flags in ["0", "1", "SOME_FLAGS"] {
        let source = sim_tree(&format!(
            "        let n: i16;\n        relationship(column: 0, dir: 0, flags: {flags}, local: 0, value: n);\n        self.level = n;"
        ));
        let source = if flags == "SOME_FLAGS" {
            source.replace("object \"", "const SOME_FLAGS = 0;\nobject \"")
        } else {
            source
        };
        // A call statement goes on when the row is missing (false, `n` unwritten).
        one(&source, "uninitialized_local_read");
        none(&source, "unused_decl");
        // As `?` or a test, the read follows a true result, which wrote `n`.
        for form in [
            format!(
                "relationship(column: 0, dir: 0, flags: {flags}, local: 0, value: n)?;\n        self.level = n;"
            ),
            format!(
                "if relationship(column: 0, dir: 0, flags: {flags}, local: 0, value: n) {{\n            self.level = n;\n        }}"
            ),
        ] {
            let source = sim_tree(&format!("        let n: i16;\n        {form}"));
            let source = if flags == "SOME_FLAGS" {
                source.replace("object \"", "const SOME_FLAGS = 0;\nobject \"")
            } else {
                source
            };
            none(&source, "uninitialized_local_read");
        }
    }
    // With flag 4 it writes the relationship from the local: a read.
    one(
        &sim_tree(
            "        let n: i16;\n        relationship(column: 0, dir: 0, flags: 4, local: 0, value: n);\n        self.level = n;",
        ),
        "uninitialized_local_read",
    );
}

// --- $Local:N string tokens -------------------------------------------------------------

/// A Sim tree whose dialog shows the string `hi`.
fn dialog_tree(text: &str, body: &str) -> String {
    obj(&format!(
        "    strings 301 \"Dialog prim string set\" {{ hi = \"{text}\" }}\n    #[bhav(label = \"Probe\")] fn probe(sim: Sim) {{\n        self.level = 0;\n        self.mode = Mode::Off;\n{body}\n        dialog(kind: 0, message: hi, title: hi)?;\n    }}"
    ))
}

#[test]
fn locals_of_trees_showing_strings_the_file_does_not_hold_may_be_read() {
    // A dialog's `STR#` 301 number past the file's own entries (the object's spliced
    // table), or `show_string` by number: the text, and its `$Local:N` tokens, are
    // unknown, so no local is reported unread.
    let unread = "        let shown = random(7);";
    for (members, call) in [
        ("", "dialog(kind: 0, message: 3, title: 3)?;"),
        (
            "    strings 301 \"Dialog prim string set\" { hi = \"Hi\" }\n",
            "dialog(kind: 0, message: 2, title: hi)?;",
        ),
        ("", "action_label(table: 431, index: 2);"),
    ] {
        let source = obj(&format!(
            "{members}    #[bhav(label = \"Probe\")] fn probe(sim: Sim) {{\n        self.level = 0;\n        self.mode = Mode::Off;\n{unread}\n        {call}\n    }}"
        ));
        none(&source, "unused_decl");
    }
    // The file's own entry without a token leaves it unread.
    let source = dialog_tree("Hi", unread);
    one(&source, "unused_decl");
}

#[test]
fn a_local_read_by_an_action_label_token_is_used() {
    let source = obj(
        "    strings 431 \"Labels\" { a = \"$Local:1\" }\n    #[bhav(label = \"Probe\")] fn probe(sim: Sim) {\n        self.level = 0;\n        self.mode = Mode::Off;\n        let shown = random(7);\n        action_label(table: 431, index: 1);\n    }",
    );
    none(&source, "unused_decl");
    none(&source, "dialog_local_slot_mismatch");
    // Show String formats no token: its text reads nothing.
    let source = obj(
        "    strings 431 \"Labels\" { a = \"$Local:1\" }\n    #[bhav(label = \"Probe\")] fn probe(sim: Sim) {\n        self.level = 0;\n        self.mode = Mode::Off;\n        let shown = random(7);\n        show_string(table: 431, index: 1);\n    }",
    );
    one(&source, "unused_decl");
}

#[test]
fn a_local_read_only_by_a_string_token_is_used() {
    let body = "        let first = random(5);\n        self.level = first;\n        let shown = random(7);";
    // Lowered: the `random` temporary is slot 0, `first` slot 1 and `shown` slot 2 (a tree that
    // formats text keeps every `let` in its own slot).
    let source = dialog_tree("you got $Local:2", body);
    none(&source, "unused_decl");
    none(&source, "dialog_local_slot_mismatch");
    let source = dialog_tree("you got $TimeLocal:2", body);
    none(&source, "unused_decl");
    // A token reading another slot leaves `shown` unread, and says where it is.
    let source = dialog_tree("you got $Local:0", body);
    let d = one(&source, "unused_decl");
    assert!(d.message.contains("`shown`"), "{d:?}");
    assert!(
        d.note.as_deref().unwrap().contains("`shown` is on slot 2"),
        "{d:?}"
    );
}

#[test]
fn a_string_token_past_the_frame_is_flagged() {
    let source = dialog_tree("you got $Local:4", "        let shown = random(7);");
    let d = one(&source, "dialog_local_slot_mismatch");
    assert_eq!(d.line, line_of(&source, "dialog("));
    assert!(
        d.message
            .contains("`$Local:4` in string `hi` reads local slot 4"),
        "{d:?}"
    );
    assert!(d.note.as_deref().unwrap().contains("1 `shown`"), "{d:?}");
    // Slot 0 holds the `random` temporary, not a `let`.
    let source = dialog_tree("you got $Local:0", "        let shown = random(7);");
    let d = one(&source, "dialog_local_slot_mismatch");
    assert!(d.message.contains("compiler temporary"), "{d:?}");
    // A raw `local[N]` slot is the raw code's business.
    none(
        &dialog_tree(
            "you got $Local:0",
            "        raw { local[0] := 3; }\n        self.level = 1;",
        ),
        "dialog_local_slot_mismatch",
    );
}

#[test]
fn string_tokens_follow_hex_raw_locals_and_numeric_dialog_strings() {
    // `local[0x2]` reserves slots 0..=2; the `random` temporary is 3, `shown` 4.
    let hex = |token: &str| {
        dialog_tree(
            &format!("you got $Local:{token}"),
            "        raw { local[0x2] := 3; }\n        let shown = random(7);",
        )
    };
    none(&hex("4"), "unused_decl");
    none(&hex("4"), "dialog_local_slot_mismatch");
    let d = one(&hex("3"), "dialog_local_slot_mismatch");
    assert!(d.message.contains("compiler temporary"), "{d:?}");
    // A dialog's numeric string is 1-based into `STR#` 301.
    let numeric = obj(
        "    strings 301 \"Dialog prim string set\" { hi = \"you got $Local:4\" }\n    #[bhav(label = \"Probe\")] fn probe(sim: Sim) {\n        self.level = 0;\n        self.mode = Mode::Off;\n        let shown = random(7);\n        dialog(kind: 0, message: 1, title: 1)?;\n    }",
    );
    let d = one(&numeric, "dialog_local_slot_mismatch");
    assert!(d.message.contains("dialog string 1"), "{d:?}");
    // `kind: 2` is a number, not dialog string 2; a `buttons` tuple names strings.
    let two = |call: &str| {
        obj(&format!(
            "    strings 301 \"Dialog prim string set\" {{ msg = \"Go on?\", a = \"$Local:4\", b = \"No\" }}\n    #[bhav(label = \"Probe\")] fn probe(sim: Sim) {{\n        self.level = 0;\n        self.mode = Mode::Off;\n        {call}\n    }}"
        ))
    };
    none(
        &two("dialog(kind: 2, message: msg, title: msg)?;"),
        "dialog_local_slot_mismatch",
    );
    one(
        &two("dialog(kind: yes_no, message: msg, buttons: (a, b))?;"),
        "dialog_local_slot_mismatch",
    );
    // A state with a string's name is not the string.
    none(
        &obj(
            "    strings 301 \"Dialog prim string set\" { hi = \"you got $Local:4\" }\n    state hi: i16;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.level = 0;\n        self.mode = Mode::Off;\n        self.hi = 1;\n    }",
        ),
        "dialog_local_slot_mismatch",
    );
}

// --- raw_bare_call_false_exit -------------------------------------------------------------

#[test]
fn a_raw_block_ending_in_a_bare_call_before_more_code_is_flagged() {
    let source = obj(
        "    #[bhav(id = 4300, label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        raw {\n            temp[0] := 1;\n            call 4301(temps); // why\n        }\n        self.level = 1;\n    }\n    #[bhav(id = 4301, label = \"Other\")] fn other() { return true; }",
    );
    let d = one(&source, "raw_bare_call_false_exit");
    assert_eq!(d.line, line_of(&source, "call 4301(temps);"));
    assert!(
        d.help
            .as_deref()
            .unwrap()
            .contains("if call 4301(temps) {}"),
        "{d:?}"
    );
}

#[test]
fn allow_directives_work_inside_raw() {
    let wrap = |raw: &str| {
        obj(&format!(
            "    #[bhav(id = 4300, label = \"Probe\")] fn probe() {{\n        self.mode = Mode::Off;\n        raw {{\n            temp[0] := 1;\n{raw}\n        }}\n        self.level = 1;\n    }}\n    #[bhav(id = 4301, label = \"Other\")] fn other() {{ return true; }}"
        ))
    };
    // Trailing the call's line, in `.air`'s `//` or `#` comment form.
    for raw in [
        "            call 4301(temps); // antics-lint: allow(raw_bare_call_false_exit)",
        "            call 4301(temps); # antics-lint: allow(raw_bare_call_false_exit)",
        // On its own line: covers the next line of code.
        "            # antics-lint: allow(raw_bare_call_false_exit)\n\n            call 4301(temps);",
    ] {
        let source = wrap(raw);
        none(&source, "raw_bare_call_false_exit");
        none(&source, "unknown_lint_rule");
    }
    // On another line, for another rule, or as `//!`: the call is still reported.
    for raw in [
        "            call 4301(temps);\n            # antics-lint: allow(raw_bare_call_false_exit)",
        "            call 4301(temps); # antics-lint: allow(while_true)",
        "            //! antics-lint: allow(raw_bare_call_false_exit)\n            temp[1] := 2;\n            call 4301(temps);",
    ] {
        one(&wrap(raw), "raw_bare_call_false_exit");
    }
    // A `#` inside an `.air` string is not a comment.
    one(
        &wrap(
            "            temp[1] := \"# antics-lint: allow(raw_bare_call_false_exit)\";\n            call 4301(temps);",
        ),
        "raw_bare_call_false_exit",
    );
}

#[test]
fn guarded_or_final_raw_calls_are_fine() {
    let other = "\n    #[bhav(id = 4301, label = \"Other\")] fn other() { return true; }";
    for body in [
        // Both results go on.
        "        raw { if call 4301(temps) {} }\n        self.level = 1;",
        // Nothing follows the block in its list: the false exit is the tree's own.
        "        self.level = 1;\n        raw { call 4301(temps); }",
        // The call is not the block's last statement.
        "        raw { call 4301(temps); temp[0] := 1; }\n        self.level = 1;",
    ] {
        none(
            &obj(&format!(
                "    #[bhav(id = 4300, label = \"Probe\")] fn probe() {{\n        self.mode = Mode::Off;\n{body}\n    }}{other}"
            )),
            "raw_bare_call_false_exit",
        );
    }
}

#[test]
fn raw_calls_with_their_own_edges_or_nested_in_an_arm() {
    let wrap = |body: &str| {
        obj(&format!(
            "    #[bhav(id = 4300, label = \"Probe\")] fn probe() {{\n        self.mode = Mode::Off;\n{body}\n    }}\n    #[bhav(id = 4301, label = \"Other\")] fn other() {{ return true; }}"
        ))
    };
    // The call names both edges.
    none(
        &wrap(
            "        raw { call 4301(temps) -> true: true, false: true; }\n        self.level = 1;",
        ),
        "raw_bare_call_false_exit",
    );
    // Last in an `if` arm, but the code after the `if` follows it.
    let source = wrap(
        "        if self.level > 0 {\n            raw { call 4301(temps); }\n        }\n        self.level = 1;",
    );
    one(&source, "raw_bare_call_false_exit");
    // A label does not hide the call.
    one(
        &wrap("        raw { @last: call 4301(temps); }\n        self.level = 1;"),
        "raw_bare_call_false_exit",
    );
    // Last in a loop body: the loop's test follows it.
    one(
        &wrap("        while self.level > 0 {\n            raw { call 4301(temps); }\n        }"),
        "raw_bare_call_false_exit",
    );
    // Last in an arm that is last in the tree: the false exit is the tree's own.
    none(
        &wrap("        if self.level > 0 {\n            raw { call 4301(temps); }\n        }"),
        "raw_bare_call_false_exit",
    );
}

// --- missing_sim_frame_args ---------------------------------------------------------------

#[test]
fn a_sim_tree_reading_param_in_raw_without_frame_args_is_flagged() {
    let source = sim_tree("        raw { temp[0] := param[1]; }\n        sim.motive[1] = 1;");
    let d = one(&source, "missing_sim_frame_args");
    assert!(d.message.contains("`param[1]`"), "{d:?}");
    assert!(d.message.contains("declares no argument words"), "{d:?}");
    assert!(
        d.help.as_deref().unwrap().contains("#[frame(args = 2)]"),
        "{d:?}"
    );
    // Pinned arguments (any count: the frame is declared), an object-context tree, or
    // declared parameters: fine.
    none(
        &obj(
            "    #[bhav(label = \"Probe\", args = 1)] fn probe(sim: Sim) {\n        self.level = 0;\n        self.mode = Mode::Off;\n        raw { temp[0] := param[1]; }\n        sim.motive[1] = 1;\n    }",
        ),
        "missing_sim_frame_args",
    );
    none(
        &obj(
            "    #[bhav(label = \"Probe\", args = 2)] fn probe(sim: Sim) {\n        self.level = 0;\n        self.mode = Mode::Off;\n        raw { temp[0] := param[1]; }\n        sim.motive[1] = 1;\n    }",
        ),
        "missing_sim_frame_args",
    );
    none(
        &tree("        raw { temp[0] := param[1]; }"),
        "missing_sim_frame_args",
    );
}

#[test]
fn a_const_written_as_its_value_in_raw_is_not_a_use() {
    // Raw code resolves v2 constants by name, so the bare value is no use of the name
    // (this was a use before docs/decisions/20261125-antics-v2-new-lints.md).
    one(
        &obj(
            "    const LEVEL_SLOT = 7;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        self.level = 0;\n        raw { attr[7] := 1; }\n    }",
        ),
        "unused_decl",
    );
    one(
        &obj(
            "    const LEVEL_SLOT = 7;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        self.level = 0;\n        raw { temp[7] := 1; }\n    }",
        ),
        "unused_decl",
    );
}

#[test]
fn a_const_named_in_raw_is_used() {
    // Lowering declares a v2 `const` a raw block names as an `.air` constant.
    let source = obj(
        "    const LEVEL_SLOT = 7;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        self.level = 0;\n        raw { temp[0] := LEVEL_SLOT; }\n    }",
    );
    none(&source, "unused_decl");
    assert!(lint(&source).is_empty(), "{:?}", lint(&source));
    // A block's own `.air` constant of the same name is not the v2 one.
    one(
        &obj(
            "    const LEVEL_SLOT = 7;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        self.level = 0;\n        raw { const LEVEL_SLOT = 3; temp[0] := LEVEL_SLOT; }\n    }",
        ),
        "unused_decl",
    );
    // `const[K]` reads `K`, and `const K.e = ..` declares no `K`: a bare `K` there is a use.
    for raw in [
        "raw { temp[0] := const[LEVEL_SLOT]; }",
        "raw { const LEVEL_SLOT.e = 2; temp[0] := LEVEL_SLOT; }",
    ] {
        none(
            &obj(&format!(
                "    const LEVEL_SLOT = 7;\n    #[bhav(label = \"Probe\")] fn probe() {{\n        self.mode = Mode::Off;\n        self.level = 0;\n        {raw}\n    }}"
            )),
            "unused_decl",
        );
    }
    // `K.e` after the block's own `const K.e` is not the v2 `K`; a `.const` field declares
    // nothing.
    one(
        &obj(
            "    const LEVEL_SLOT = 7;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        self.level = 0;\n        raw { const LEVEL_SLOT.e = 2; temp[0] := LEVEL_SLOT.e; }\n    }",
        ),
        "unused_decl",
    );
    none(
        &obj(
            "    const LEVEL_SLOT = 7;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        self.level = 0;\n        raw { const Tune.const = LEVEL_SLOT; temp[0] := Tune.const; }\n    }",
        ),
        "unused_decl",
    );
    // A field after `.` is not the constant.
    one(
        &obj(
            "    const room = 7;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        self.level = 0;\n        raw { temp[0] := my.room; }\n    }",
        ),
        "unused_decl",
    );
}

#[test]
fn a_const_named_in_a_raw_comment_or_string_is_just_unused() {
    for raw in [
        "raw { /* ROOM */ temp[0] := 1; }",
        "raw { temp[0] := 1; # ROOM\n }",
        "raw { temp[0] := \"ROOM \\\" ROOM\"; }",
    ] {
        let d = one(
            &obj(&format!(
                "    const ROOM = 2;\n    #[bhav(label = \"Probe\")] fn probe() {{\n        self.mode = Mode::Off;\n        self.level = 0;\n        {raw}\n    }}"
            )),
            "unused_decl",
        );
        assert!(d.message.contains("never used"), "{raw}: {d:?}");
    }
}

#[test]
fn negated_comparisons_storing_a_local_are_fine() {
    // An unwritten local may fault before the store replaces the last result.
    none(
        &tree(
            "        let x: i16;\n        if !(temp[0] > 0) { temp[1] = x; } else { temp[1] = 0; }\n        temp[2] = 3;",
        ),
        "negated_comparison",
    );
    // A `const` cannot.
    one(
        &obj(
            "    const K = 2;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.level = 0;\n        if !(temp[0] > 0) { temp[1] = K; } else { temp[1] = 0; }\n        temp[2] = 3;\n    }",
        ),
        "negated_comparison",
    );
}

#[test]
fn a_raw_action_string_may_read_any_local() {
    let source = obj(
        "    strings 431 \"Labels\" { a = \"$Local:0\" }\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.level = 0;\n        self.mode = Mode::Off;\n        let shown = 7;\n        raw { action_string(table: 431, index: 1); }\n    }",
    );
    none(&source, "unused_decl");
}

#[test]
fn a_const_matching_an_index_in_a_raw_label_is_unused() {
    let d = one(
        &obj(
            "    const ROOM = 7;\n    #[bhav(id = 4300, label = \"Probe\")] fn probe() {\n        self.level = 0;\n        self.mode = Mode::Off;\n        raw { call \"ROOM attr[7]\"(temps); }\n    }\n    #[bhav(id = 4301, label = \"ROOM attr[7]\")] fn other() { return true; }",
        ),
        "unused_decl",
    );
    assert!(d.message.contains("never used"), "{d:?}");
}

#[test]
fn raw_bare_calls_are_reported_on_their_own_line() {
    let wrap = |pre: &str, raw: &str| {
        obj(&format!(
            "    #[bhav(id = 4300, label = \"Probe\")] fn probe() {{\n        self.mode = Mode::Off;\n{pre}{raw}\n        self.level = 1;\n    }}\n    #[bhav(id = 4301, label = \"Other\")] fn other() {{ return true; }}"
        ))
    };
    let line_of = |source: &str, needle: &str| {
        source
            .lines()
            .position(|l| l.contains(needle))
            .map(|n| n + 1)
            .unwrap()
    };
    // `raw`, `{`, a directive for another rule and the call on successive lines.
    let source = wrap(
        "",
        "        raw\n        {\n            # antics-lint: allow(while_true)\n            call 4301(temps);\n        }",
    );
    let d = one(&source, "raw_bare_call_false_exit");
    assert_eq!(d.line, line_of(&source, "call 4301"), "{d:?}");
    // ... and with that directive for this rule, the call is not reported.
    none(
        &wrap(
            "",
            "        raw\n        {\n            # antics-lint: allow(raw_bare_call_false_exit)\n            call 4301(temps);\n        }",
        ),
        "raw_bare_call_false_exit",
    );
    // A commented-out copy of the call earlier in the block does not take its place.
    let source = wrap(
        "",
        "        raw {\n            /* call 4301(temps); */\n            temp[0] := 1;\n            call 4301(temps);\n        }",
    );
    let d = one(&source, "raw_bare_call_false_exit");
    assert_eq!(d.line, line_of(&source, "            call 4301"), "{d:?}");
    // Multi-byte text before a `raw` with a commented `raw` ahead of it.
    let source = wrap(
        "",
        "        dialog(\"\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\"); /*raw*/ raw {\n            temp[0] := 1;\n            call 4301(temps);\n        }",
    );
    let d = one(&source, "raw_bare_call_false_exit");
    assert_eq!(d.line, line_of(&source, "call 4301"), "{d:?}");
    assert_eq!(d.column, 13, "{d:?}");
}

#[test]
fn raw_directives_after_labels_and_before_the_brace() {
    let wrap = |raw: &str| {
        obj(&format!(
            "    #[bhav(id = 4300, label = \"Probe\")] fn probe() {{\n        self.mode = Mode::Off;\n{raw}\n        self.level = 1;\n    }}\n    #[bhav(id = 4301, label = \"Other\")] fn other() {{ return true; }}"
        ))
    };
    // A label on its own line: the call's trailing directive covers it.
    none(
        &wrap(
            "        raw {\n            temp[0] := 1;\n            @where:\n            call 4301(temps); # antics-lint: allow(raw_bare_call_false_exit)\n        }",
        ),
        "raw_bare_call_false_exit",
    );
    // `//!` between `raw` and `{` is a v2 comment: file level.
    none(
        &wrap(
            "        raw //! antics-lint: allow(raw_bare_call_false_exit)\n        {\n            temp[0] := 1;\n            call 4301(temps);\n        }",
        ),
        "raw_bare_call_false_exit",
    );
}

#[test]
fn odd_raw_text_does_not_crash_the_linter() {
    // A non-ASCII comment before `[`, and an index no frame can hold.
    for raw in [
        "        raw { # caf\u{e9}[2]\n            temp[0] := 1; }",
        "        raw { local[9223372036854775807] := 1; }",
        "        raw { local[0x7FFF_FFFF_FFFF_FFFF] := 1; }",
    ] {
        let _ = lint_v2_source(&tree(raw), "probe.antics");
    }
    // An argument index no frame can hold is the compiler's to reject.
    let _ = lint_v2_source(
        &sim_tree("        raw { temp[0] := param[9223372036854775807]; }"),
        "probe.antics",
    );
}

#[test]
fn used_underscored_raw_and_shared_declarations_are_fine() {
    let source = obj("    #[slot(6)] state slot_state: i16;\n    \
         bcon 4097 \"Tuning\" { low = 1, high = 9 }\n    \
         #[bhav(label = \"Probe\")]
         fn probe() {\n        self.mode = Mode::Off;\n        \
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
        "    #[bhav(label = \"Big Tree\")] fn big() {{\n        self.mode = Mode::Off;\n{many_nodes}    }}"
    ));
    let d = one(&source, "frame_headroom");
    assert!(d.message.contains("nodes"), "{d:?}");
    assert_eq!(d.line, line_of(&source, "fn big()"));

    let lets: String = (0..14)
        .map(|i| format!("        let v{i} = random(9);\n"))
        .collect();
    let sum: Vec<String> = (0..14).map(|i| format!("v{i}")).collect();
    let source = obj(&format!(
        "    #[bhav(label = \"Wide Tree\")] fn wide() {{\n        self.mode = Mode::Off;\n{lets}        self.level = {};\n    }}",
        sum.join(" + ")
    ));
    let d = one(&source, "frame_headroom");
    assert!(d.message.contains("locals"), "{d:?}");

    // Raw slots push the named locals up: the message says so.
    let source = obj(
        "    #[bhav(label = \"Wide Tree\")] fn wide() {\n        self.mode = Mode::Off;\n        raw { local[11] := temp[0]; temp[1] := local[11]; }\n        let a = random(9);\n        self.level = a;\n    }",
    );
    let d = one(&source, "frame_headroom");
    assert!(
        d.message.contains("12 raw slot(s) up to `local[11]`"),
        "{d:?}"
    );
}

#[test]
fn frame_headroom_points_at_the_tree_in_its_own_object() {
    // Two objects in one file, each with a tree labeled "Shared"; only the first is big.
    let many_nodes: String = (0..205).map(|_| "        self.level += 1;\n").collect();
    let big = obj(&format!(
        "    #[bhav(id = 4200, label = \"Shared\")] fn shared() {{\n        self.mode = Mode::Off;\n{many_nodes}    }}"
    ));
    let small = obj("    #[bhav(id = 4201, label = \"Shared\")] fn shared() {\n        self.mode = Mode::Off;\n        self.level += 1;\n    }")
        .replace("enum Mode { Off, Low, High }\n", "")
        .replace("Synth Probe", "Synth Probe Two")
        .replace("0x7E00_0100", "0x7E00_0101");
    let source = format!("{big}{small}");
    let d = one(&source, "frame_headroom");
    assert!(d.message.contains("tree 4200"), "{d:?}");
    assert_eq!(d.line, line_of(&source, "#[bhav(id = 4200"), "{d:?}");
    // Two unpinned trees sharing a label in one object: the object, not the wrong tree.
    let source = obj(&format!(
        "    #[bhav(label = \"Shared\")] fn a() {{\n        self.mode = Mode::Off;\n    }}\n    #[bhav(label = \"Shared\")] fn b() {{\n        self.mode = Mode::Off;\n{many_nodes}    }}"
    ));
    let d = one(&source, "frame_headroom");
    assert_eq!(d.line, line_of(&source, "object \"Synth Probe\""), "{d:?}");
}

#[test]
fn small_trees_have_headroom() {
    none(&tree("        self.level += 1;"), "frame_headroom");
}

// --- redundant_bool_return ----------------------------------------------------------------

#[test]
fn if_returning_bool_literals_is_flagged() {
    let source = obj(
        "    state on: bool;\n    fn high() -> bool {\n        if self.on { return true; }\n        return false;\n    }",
    );
    let d = one(&source, "redundant_bool_return");
    assert_eq!(d.suggestion.as_deref(), Some("return self.on;"));
    let with_else = obj(
        "    state on: bool;\n    fn high() -> bool {\n        if self.on { return true; } else { return false; }\n    }",
    );
    assert_eq!(
        one(&with_else, "redundant_bool_return")
            .suggestion
            .as_deref(),
        Some("return self.on;")
    );
}

#[test]
fn if_returning_a_comparison_is_flagged_in_the_same_polarity() {
    // `return c;` ends on the node testing `c` with the same exits: the same last result.
    for (cond, returned) in [
        ("self.level > 3", "self.level > 3"),
        ("!(self.level > 3)", "!(self.level > 3)"),
        ("self.on && self.level == 2", "self.on && (self.level == 2)"),
    ] {
        let d = one(
            &obj(&format!(
                "    state on: bool;\n    fn high() -> bool {{\n        if {cond} {{ return true; }}\n        return false;\n    }}"
            )),
            "redundant_bool_return",
        );
        assert_eq!(d.suggestion, Some(format!("return {returned};")));
    }
}

#[test]
fn if_returning_a_test_that_may_fold_or_a_deferred_block_is_fine() {
    for body in [
        // `1 < 2` folds away: the `if` leaves the earlier compare's result, where
        // `return 1 < 2;` would run a compare.
        "        if self.level > 0 { return false; }\n        if 1 < 2 { return true; }\n        return false;",
        "        if self.level > 0 && true { return true; }\n        return false;",
        // A `defer` runs before each `return`: the two forms lower differently.
        "        defer { self.level = 0; }\n        if self.level > 3 { return true; }\n        return false;",
    ] {
        none(
            &obj(&format!("    fn high() -> bool {{\n{body}\n    }}")),
            "redundant_bool_return",
        );
    }
}

#[test]
fn if_returning_a_call_with_a_literal_argument_is_flagged() {
    // A `bool` literal among a call's arguments does not make its test fold.
    let d = one(
        &obj(
            "    fn ran() -> bool {\n        if stack_obj.run_functional(4, change_icon: true) { return true; }\n        return false;\n    }",
        ),
        "redundant_bool_return",
    );
    assert_eq!(
        d.suggestion.as_deref(),
        Some("return stack_obj.run_functional(4, change_icon: true);")
    );
}

#[test]
fn if_returning_a_long_chain_or_inside_a_lock_is_fine() {
    for body in [
        // Three tests: `return` lowers the chain through a local.
        "        if temp[0] > 3 && temp[1] == 2 && temp[2] < 1 { return true; }\n        return false;",
        "        if temp[0] > 3 || temp[1] == 2 { return true; }\n        return false;",
        // `return` inside `with lock` leaves through the unlock, after the test.
        "        with lock(self.on) {\n            if temp[0] > 0 { return true; }\n            return false;\n        }\n        return false;",
    ] {
        none(
            &obj(&format!(
                "    state on: bool;\n    fn high() -> bool {{\n{body}\n    }}"
            )),
            "redundant_bool_return",
        );
    }
}

#[test]
fn suggestions_spell_valid_v2() {
    // An operand that is itself a comparison keeps its parentheses.
    let d = one(
        &obj(
            "    state on: bool;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.level = 0;\n        if !((temp[0] > 0) == self.on) { temp[1] = 1; } else { temp[1] = 2; }\n        temp[2] = 3;\n    }",
        ),
        "negated_comparison",
    );
    assert_eq!(d.suggestion.as_deref(), Some("(temp[0] > 0) != self.on"));
    // A non-ASCII space stays itself; v2 has no `\u{..}` escape.
    let d = one(
        &obj(
            "    fn ask() -> bool {\n        if dialog(kind: 0, message: \"Hi\u{a0}there\") { return true; }\n        return false;\n    }",
        ),
        "redundant_bool_return",
    );
    assert_eq!(
        d.suggestion.as_deref(),
        Some("return dialog(kind: 0, message: \"Hi\u{a0}there\");")
    );
}

#[test]
fn if_returning_false_then_true_keeps_its_last_result() {
    // `return !c;` would end on the opposite last result: never reported.
    for cond in ["self.on", "!self.on", "self.level > 3", "!(self.level > 3)"] {
        for shape in [
            format!("if {cond} {{ return false; }}\n        return true;"),
            format!("if {cond} {{ return false; }} else {{ return true; }}"),
        ] {
            none(
                &obj(&format!(
                    "    state on: bool;\n    fn low() -> bool {{\n        {shape}\n    }}"
                )),
                "redundant_bool_return",
            );
        }
    }
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
fn enum_chains_on_an_enum_state_are_flagged() {
    let source = tree(
        "        if self.mode == Mode::Off {\n            sleep(1);\n        } else if self.mode == Mode::Low {\n            sleep(2);\n        } else if self.mode == Mode::High {\n            sleep(3);\n        }",
    );
    one(&source, "collapsible_if_chain");
}

#[test]
fn short_or_mixed_chains_are_fine() {
    for body in [
        "        if self.level == 1 {\n            sleep(1);\n        } else if self.level == 2 {\n            sleep(2);\n        }",
        "        if self.level == 1 {\n            sleep(1);\n        } else if temp[0] == 2 {\n            sleep(2);\n        } else if self.level == 3 {\n            sleep(3);\n        }",
        "        if self.level == 1 {\n            sleep(1);\n        } else if self.level > 2 {\n            sleep(2);\n        } else if self.level == 3 {\n            sleep(3);\n        }",
        "        if self.level == 1 {\n            sleep(1);\n        } else if self.level == 1 {\n            sleep(2);\n        } else if self.level == 3 {\n            sleep(3);\n        }",
        // A `match` on an `ObjectId` does not compile.
        "        if stack_obj == 1 {\n            sleep(1);\n        } else if stack_obj == 2 {\n            sleep(2);\n        } else if stack_obj == 3 {\n            sleep(3);\n        }",
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
        "        raw { refresh(target: 2, mode: 0); }",
        "        raw { go_to_relative(location: 200); }",
        "        raw { special_effect(duration: --9223372036854775808); }",
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
            "    const OTHER = 0x7E00_0200;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        if stack_obj.is_type(OTHER) { self.level = 1; }\n    }",
        ),
        "magic_guid_literal",
    );
}

// --- negated_comparison -------------------------------------------------------------------

#[test]
fn negated_comparisons_are_flagged_with_a_polarity_note() {
    let source = tree(
        "        if !(self.level == 3) { temp[0] = 1; }\n        temp[2] = 0;\n        if !(self.level < temp[0]) { self.level = temp[1] + 1; }\n        self.mode = Mode::Low;\n        sleep(2);",
    );
    let found = of(&source, "negated_comparison");
    let suggestions: Vec<&str> = found
        .iter()
        .filter_map(|d| d.suggestion.as_deref())
        .collect();
    assert_eq!(suggestions, ["self.level != 3", "self.level >= temp[0]"]);
    assert!(found[0].note.as_deref().unwrap().contains("polarity"));
    // Both branches go on to a store that cannot fault.
    one(
        &tree("        while !(self.level > 3) { self.level += 1; }\n        temp[0] = 2;"),
        "negated_comparison",
    );
}

#[test]
fn negated_comparisons_a_branch_may_end_on_are_fine() {
    // A branch that can end the tree right after the compare leaves its result as the
    // tree's last result: flipping it would change that.
    for body in [
        "        if !(self.level == 3) { temp[0] = 1; }",
        "        if !(self.level == 3) { return true; }\n        temp[0] = 1;",
        "        if !(self.level == 3) { temp[0] = 1; }\n        return false;",
        "        while !(self.level > 3) { self.level += 1; }",
        // A test of constants folds away: the `return` right after is the next node.
        "        if !(self.level == 3) {\n            if true { return true; }\n        }\n        temp[0] = 1;",
        "        if !(self.level == 3) {\n            if true || self.level == 0 { return true; }\n        }\n        temp[0] = 1;",
    ] {
        none(&tree(body), "negated_comparison");
    }
}

#[test]
fn negated_comparisons_before_a_call_or_a_fault_are_fine() {
    // The last result is observable at a call (the callee and the verifier see it) and
    // at a fault, so the compare's own result may be the one that shows.
    for body in [
        "        if !(self.level == 3) { sleep(1); }\n        temp[0] = 1;",
        "        if !(self.level == 3) { temp[0] = 1; }\n        sleep(1);",
        "        while !(self.level > 3) { self.level += 1; }\n        sleep(1);",
        "        if !(self.level == 3) { temp[0] = 1; }\n        temp[1] = temp[0] / 2;",
        "        if !(self.level == 3) { temp[0] = 1; }\n        temp[1] = temp[temp[0]];",
        // A store to a `let` may be folded away.
        "        let n = 0;\n        if !(self.level == 3) { n = 1; }\n        temp[0] = n;",
        // `temp[8]` is past the temps: the store faults.
        "        if !(temp[0] > 0) { temp[8] = 1; } else { temp[1] = 2; }\n        temp[3] = 1;",
        // A flag operation faults on a bit out of range.
        "        if !(temp[0] > 0) { temp[1] = 1; } else { temp[1] flag_set temp[2]; }\n        temp[3] = 1;",
        // A test may fold away (`-1 < 0`), and then the tree ends on the compare.
        "        if !(temp[0] > 0) {\n            if -1 < 0 {}\n        } else {\n            temp[1] = 2;\n        }\n        return true;",
        // A test of places is not counted either.
        "        if !(temp[0] > 0) { temp[1] = 1; }\n        if temp[2] == 2 { sleep(1); }",
        // A field of `self` that is not a `state` may lower to a primitive.
        "        if !(temp[0] > 0) { self.price = 1; }\n        temp[3] = 1;",
    ] {
        none(&tree(body), "negated_comparison");
    }
    // In a Sim's body `self` follows `stack_obj`, which may be missing: a store to it
    // can fault.
    none(
        &sim_tree("        if !(self.level == 3) { self.level = 1; }\n        temp[0] = 1;"),
        "negated_comparison",
    );
    one(
        &sim_tree("        if !(temp[0] == 3) { temp[1] = 1; }\n        temp[2] = 1;"),
        "negated_comparison",
    );
}

#[test]
fn plain_negation_is_fine() {
    none(
        &obj(
            "    state on: bool;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        self.level = 0;\n        if !self.on { sleep(1); }\n        if !(self.on && self.level > 0) { sleep(1); }\n    }",
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
    // Trailing a block's opening line: covers the whole block (what `fmt` leaves of an
    // end-of-line directive on a one-line `if`).
    none(
        &obj(
            "    state on: bool;\n    fn on() -> bool {\n        if !self.on { // antics-lint: allow(redundant_bool_return)\n            return false;\n        }\n        return true;\n    }",
        ),
        "redundant_bool_return",
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
    // However the block is introduced.
    one(
        &tree(&format!(
            "        raw /* {{ */ {{\n            //! antics-lint: allow(while_true)\n            notify_idle() pad 0xff;\n        }}\n{flagged}"
        )),
        "while_true",
    );
    one(
        &tree(&format!(
            "        dialog(\"\u{e9}\u{e9}\u{e9}\u{e9}\"); raw {{\n            //! antics-lint: allow(while_true)\n            notify_idle() pad 0xff;\n        }}\n{flagged}"
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

// --- source detection -------------------------------------------------------------------------

#[test]
fn lint_reads_v2_files_that_start_with_consts_comments_and_attributes() {
    let dir = scratch("detect");
    let v2 = dir.join("consts_first.antics");
    let air = dir.join("legacy.antics");
    let mut head = String::from("// header comment\n/* block */\n");
    for i in 0..40 {
        head.push_str(&format!("const C{i} = {i};\n"));
    }
    std::fs::write(
        &v2,
        format!(
            "{head}{}",
            tree("        while true {\n            sleep(1);\n        }")
        ),
    )
    .unwrap();
    std::fs::write(
        &air,
        "# an .air file\n// another comment\ntree 4096 \"Main\" {\n    temp[0] := 1;\n}\n",
    )
    .unwrap();
    let (diags, sources) = lint_v2_paths(std::slice::from_ref(&dir)).expect("lint dir");
    assert!(
        sources.keys().any(|k| k.ends_with("consts_first.antics")),
        "{sources:?}"
    );
    assert!(
        !sources.keys().any(|k| k.ends_with("legacy.antics")),
        "{sources:?}"
    );
    assert!(diags.iter().any(|d| d.rule == "while_true"), "{diags:#?}");
    // The 40 leading constants are never used.
    assert_eq!(diags.iter().filter(|d| d.rule == "unused_decl").count(), 40);
    let out = cli(&[v2.as_os_str()]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("warning[while_true]"), "{text}");
}

#[test]
fn every_command_detects_a_v2_file_that_starts_with_uses_and_externs() {
    for head in [
        "use std::globals::*;\n",
        "extern mod synth_globals {\n    #[id(256)] fn tune() -> bool;\n}\n",
        "#[inline]\nfn helper() {\n    sleep(1);\n}\n",
        "/* tree 4096 \"NotAir\" { } */\n// bcon\n",
    ] {
        let dir = scratch("detect_cmds");
        let path = dir.join("probe.antics");
        std::fs::write(&path, format!("{head}{}", tree("        sleep(1);"))).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_sims-antics"))
            .arg("check")
            .arg(&path)
            .output()
            .expect("run check");
        assert!(
            out.status.success(),
            "{head}\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

// --- shadowed_const ------------------------------------------------------------------------------

#[test]
fn a_block_const_shadowing_an_outer_const_is_flagged() {
    let source = format!(
        "const STYLE = 13;\n{}",
        obj(
            "    const PICK = 2;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        \
             const STYLE = if self.level == 1 { 4 } else { 5 };\n        \
             const (PICK, OTHER) = match self.mode { Mode::Off => (1, 2), _ => (3, 4) };\n        \
             self.level = STYLE + PICK + OTHER;\n    }"
        )
    );
    let found = of(&source, "shadowed_const");
    let mut messages: Vec<&str> = found.iter().map(|d| d.message.as_str()).collect();
    messages.sort_unstable();
    assert_eq!(
        messages,
        [
            "block constant `PICK` shadows the constant `PICK` declared at line 9",
            "block constant `STYLE` shadows the constant `STYLE` declared at line 1",
        ],
        "{found:#?}"
    );
    assert!(found.iter().all(|d| d.severity == Severity::Warning));
    let style = found.iter().find(|d| d.message.contains("STYLE")).unwrap();
    assert_eq!(style.line, line_of(&source, "const STYLE = if"));
}

#[test]
fn object_consts_and_const_params_shadowing_file_consts_are_flagged() {
    let source = format!(
        "const LIMIT = 3;\nconst STEP = 1;\n#[inline]\nfn bump(const STEP: i16) -> i16 {{\n    return STEP + LIMIT;\n}}\n{}",
        obj(
            "    const LIMIT = 4;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        self.level = bump(2) + LIMIT;\n    }"
        )
    );
    let found = of(&source, "shadowed_const");
    assert_eq!(found.len(), 2, "{found:#?}");
    assert!(
        found
            .iter()
            .any(|d| d.message.starts_with("`const` parameter `STEP`"))
    );
    assert!(
        found
            .iter()
            .any(|d| d.message.starts_with("object constant `LIMIT`"))
    );
}

#[test]
fn distinct_names_do_not_shadow() {
    let source = format!(
        "const STYLE = 13;\n{}",
        obj(
            "    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        \
             const PICK = if self.level == 1 { STYLE } else { 5 };\n        self.level = PICK;\n    }"
        )
    );
    none(&source, "shadowed_const");
    none(&source, "unused_decl");
}

// --- unused_decl for constants -------------------------------------------------------------------

#[test]
fn unused_file_object_and_block_consts_are_flagged() {
    let source = format!(
        "const HOUR = 0;\nconst USED_TOP = 1;\n{}",
        obj(
            "    const SPARE = 4;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        \
             const UNREAD = if self.level == 1 { 4 } else { 5 };\n        \
             self.level = USED_TOP;\n        raw { temp[0] := global[0]; }\n    }"
        )
    );
    let found = of(&source, "unused_decl");
    let mut messages: Vec<&str> = found.iter().map(|d| d.message.as_str()).collect();
    messages.sort_unstable();
    assert_eq!(
        messages,
        [
            "block constant `UNREAD` is never used",
            "constant `HOUR` is never used",
            "constant `SPARE` is never used",
        ],
        "{found:#?}"
    );
}

#[test]
fn a_const_declared_in_two_files_is_judged_by_its_uses() {
    // Each file declares `HOUR`; neither uses it. Before, the second declaration counted
    // as a use of the first.
    let a = LintInput {
        name: "a.antics".to_owned(),
        source: format!("const HOUR = 0;\n{}", tree("        sleep(1);")),
        ..LintInput::default()
    };
    let b = LintInput {
        name: "b.antics".to_owned(),
        source: format!("const HOUR = 0;\n{}", tree("        sleep(2);")),
        ..LintInput::default()
    };
    let diags = lint_v2_sources(&[a, b]).expect("lint");
    let unused: Vec<_> = diags.iter().filter(|d| d.rule == "unused_decl").collect();
    assert_eq!(unused.len(), 2, "{unused:#?}");
}

#[test]
fn consts_used_by_guid_index_or_raw_name_are_used() {
    let source = String::from(
        "const PROBE_GUID = 0x7E00_0100;\nconst SLOT = 2;\nconst LOW = 1;\nconst RAW_NAMED = 9;\n\
         enum Mode { Off, Low, High }\n\
         object \"Synth Probe\" {\n    guid: PROBE_GUID;\n    price: 1;\n    \
         sprite: Sprite::procedural(\"floor_lamp\", primary: #101010, accent: #202020, trim: #303030);\n    \
         state mode: Mode;\n    \
         #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        \
         self.attr[SLOT] = LOW;\n        raw { temp[0] := RAW_NAMED; }\n    }\n}\n",
    );
    none(&source, "unused_decl");
}

// --- duplicate_std_global ---------------------------------------------------------------------------

#[test]
fn an_extern_matching_std_globals_is_flagged() {
    let source = format!(
        "extern mod globals {{\n    #[id(280)] fn wait_ticks(n: i16) -> Result;\n    #[id(288)] fn busy() -> bool;\n    #[id(300)] fn other() -> bool;\n}}\n{}",
        tree(
            "        globals::wait_ticks(1)?;\n        if globals::busy() && globals::other() { sleep(1); }"
        )
    );
    let found = of(&source, "duplicate_std_global");
    assert_eq!(found.len(), 2, "{found:#?}");
    assert!(found[0].message.contains("global 280"), "{found:#?}");
    assert!(
        found[0].message.contains("globals::idle_ticks"),
        "{found:#?}"
    );
    assert!(found[1].message.contains("globals::in_use"), "{found:#?}");
    assert_eq!(found[0].line, 2);
}

#[test]
fn an_extern_with_another_abi_is_not_a_duplicate() {
    // 280 returning `bool`, 288 taking an argument: other ABIs, so other declarations.
    let source = format!(
        "extern mod more_globals {{\n    #[id(280)] fn wait_ticks(n: i16) -> bool;\n    #[id(288)] fn busy(x: i16) -> bool;\n}}\n{}",
        tree("        if more_globals::wait_ticks(1) && more_globals::busy(2) { sleep(1); }")
    );
    none(&source, "duplicate_std_global");
}

// --- untyped_engine_field ---------------------------------------------------------------------------

fn untyped(source: &str) -> Vec<String> {
    of(source, "untyped_engine_field")
        .into_iter()
        .map(|d| d.suggestion.unwrap_or_default())
        .collect()
}

#[test]
fn indexed_cells_with_typed_names_are_flagged() {
    assert_eq!(
        untyped(&tree(
            "        self.level = global[0] + global[5] + self.data[29] + self.data[7];\n        self.data[0] = 2;"
        )),
        [
            "world.hour",
            "world.minute",
            "self.room",
            "self.room_impact",
            "self.graphic"
        ]
    );
    assert_eq!(
        untyped(&sim_tree(
            "        self.level = sim.person[58] + sim.person[33] + me.person[57];\n        sim.person[57] = 1;\n        if sim.data[11] == stack_obj { sleep(1); }"
        )),
        [
            "sim.age",
            "sim.priority",
            "me.job_level",
            "sim.job_level",
            "sim.id"
        ]
    );
}

#[test]
fn indexed_cells_without_a_drop_in_name_are_not_flagged() {
    for body in [
        // No typed name.
        "        self.level = global[2] + self.data[3] + self.attr[0];",
        // Already typed.
        "        self.level = world.hour + self.room;",
        // A read-only name is not assigned.
        "        self.data[29] = 1;",
        // An `ObjectId` name is not arithmetic.
        "        self.level = self.data[11] + 1;",
        // Raw code is left alone.
        "        raw { temp[0] := global[0]; }",
        // `me` is the object here: its cells have no Sim names.
        "        self.level = me.person[58] + me.data[11];",
    ] {
        none(&tree(body), "untyped_engine_field");
    }
    none(
        &sim_tree("        self.level = sim.data[11] + 1;\n        sim.data[29] = 3;"),
        "untyped_engine_field",
    );
}

#[test]
fn every_suggested_name_compiles_to_the_same_trees() {
    let config = sims_antics::v2::PackResolverConfig::default();
    let mut cases: Vec<(String, bool)> = Vec::new();
    for g in [0, 1, 3, 4, 5, 6, 7, 8, 10, 17, 28, 30] {
        cases.push((format!("self.level = global[{g}];"), false));
        cases.push((format!("global[{g}] = 1;"), false));
    }
    for d in [0, 7, 13, 15, 25, 31, 32] {
        cases.push((format!("self.level = self.data[{d}];"), false));
        cases.push((format!("self.data[{d}] = 1;"), false));
    }
    cases.push(("self.level = self.data[29];".to_owned(), false));
    cases.push((
        "if self.data[11] == stack_obj { sleep(1); }".to_owned(),
        false,
    ));
    cases.push(("self.level = self.data[11];".to_owned(), false));
    for p in [0, 33, 34, 56, 57, 58, 61] {
        cases.push((format!("self.level = sim.person[{p}];"), true));
        cases.push((format!("sim.person[{p}] = 1;"), true));
    }
    cases.push(("self.level = sim.person[32];".to_owned(), true));
    cases.push(("self.level = sim.data[29];".to_owned(), true));
    cases.push((
        "if sim.data[11] != stack_obj { sleep(1); }".to_owned(),
        true,
    ));
    cases.push(("if sim.data[2] == stack_obj { sleep(1); }".to_owned(), true));
    // Read back: a store to `stack_obj` nothing reads may be dropped.
    for cell in ["data[11]", "slot[0]"] {
        cases.push((
            format!("stack_obj = sim.{cell};\n        self.level = stack_obj.attr[1];"),
            true,
        ));
    }
    cases.push(("self.level = me.person[58];".to_owned(), true));
    for (stmt, sim) in cases {
        let body = format!("        {stmt}");
        let source = if sim { sim_tree(&body) } else { tree(&body) };
        let d = one(&source, "untyped_engine_field");
        let line = source.lines().nth(d.line - 1).unwrap();
        let start = d.column - 1;
        let end = start + line[start..].find(']').unwrap() + 1;
        let fixed = format!(
            "{}{}{}",
            &line[..start],
            d.suggestion.as_deref().unwrap(),
            &line[end..]
        );
        let rewritten = source.replacen(line, &fixed, 1);
        none(&rewritten, "untyped_engine_field");
        let a = sims_antics::compile_v2_source("a.antics", &source, &config)
            .unwrap_or_else(|e| panic!("{e}\n{source}"));
        let b = sims_antics::compile_v2_source("b.antics", &rewritten, &config)
            .unwrap_or_else(|e| panic!("{e}\n{rewritten}"));
        assert!(
            a.iff_bytes == b.iff_bytes,
            "`{stmt}` and `{fixed}` compile apart"
        );
    }
}

// --- raw_syntax -------------------------------------------------------------------------------------

#[test]
fn raw_code_that_does_not_parse_is_an_error() {
    let source =
        tree("        raw {\n            temp[0] := 1;\n            temp[1] := ;\n        }");
    let d = one(&source, "raw_syntax");
    assert_eq!(d.severity, Severity::Error);
    assert_eq!(d.line, line_of(&source, "temp[1] := ;"));
    // It cannot be allowed: the file does not build.
    let allowed = format!("//! antics-lint: allow(raw_syntax)\n{source}");
    let d = one(&allowed, "raw_syntax");
    assert!(
        d.note
            .as_deref()
            .unwrap_or("")
            .contains("cannot be allowed"),
        "{d:?}"
    );
}

#[test]
fn raw_code_naming_v2_consts_and_bcon_entries_parses() {
    let source = format!(
        "const SLOT = 2;\nconst OTHER_GUID = 0x7E00_0200;\n{}",
        obj(
            "    bcon 4097 \"Tuning\" { low = 1, high = 9 }\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        \
             self.level = Tuning.high;\n        \
             raw {\n            my.attr[SLOT] := Tuning.low;\n            temp[0] := SLOT;\n            create_object(OTHER_GUID, 0, 0);\n        }\n    }"
        )
    );
    none(&source, "raw_syntax");
}

// --- object_tree_me_field ------------------------------------------------------------------------

#[test]
fn me_fields_in_an_objects_own_tree_are_an_error() {
    let source = tree("        self.level = me.age;\n        self.level = me.motive.fun;");
    let found = of(&source, "object_tree_me_field");
    assert_eq!(found.len(), 2, "{found:#?}");
    assert!(found.iter().all(|d| d.severity == Severity::Error));
    assert!(
        found[0].help.as_deref().unwrap_or("").contains("sim.age"),
        "{found:#?}"
    );
    // The same in `main`.
    one(
        &obj("    main {\n        self.level = me.room;\n        self.mode = Mode::Off;\n    }"),
        "object_tree_me_field",
    );
}

#[test]
fn me_cells_and_sim_context_me_fields_are_fine() {
    none(
        &tree("        self.level = me.attr[0] + me.data[3];"),
        "object_tree_me_field",
    );
    none(
        &sim_tree("        self.level = me.age;"),
        "object_tree_me_field",
    );
    none(
        &obj(
            "    #[bhav(label = \"Probe\", context = inherit)] fn probe() {\n        self.level = me.age;\n        self.mode = Mode::Off;\n    }",
        ),
        "object_tree_me_field",
    );
}

// --- review round 1 regressions ------------------------------------------------------------------

#[test]
fn local_bindings_named_me_or_self_are_not_the_engine_handles() {
    // `me` bound by a local is that local, not the object.
    none(
        &tree("        let me: ObjectId = stack_obj;\n        self.level = me.room;"),
        "object_tree_me_field",
    );
    // A Sim parameter named `self`: `self.data[N]` is the Sim's cell, not the object's.
    none(
        &obj(
            "    #[bhav(label = \"Probe\")] fn probe(self: Sim) {\n        self.mode = Mode::Off;\n        self.level = self.data[29] + self.person[58];\n    }",
        ),
        "untyped_engine_field",
    );
    // A local `me` in a Sim tree is no Sim handle.
    none(
        &sim_tree("        let me = 3;\n        self.level = me + 1;"),
        "untyped_engine_field",
    );
}

#[test]
fn a_bcon_table_named_me_is_not_the_object() {
    // Declared on the object, or by a template the object uses: `me.entry` is the table.
    none(
        &obj(
            "    extern bcon me { #[addr(16521)] probe; }\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        self.level = me.probe;\n    }",
        ),
        "object_tree_me_field",
    );
}

#[test]
fn a_const_parameter_index_is_not_flagged() {
    // `pick(29)` and `pick(0)` specialize `self.data[N]` to two cells: one name cannot fit.
    let source = format!(
        "#[inline]\nfn pick(const N: i16) -> i16 {{\n    return self.data[N];\n}}\n{}",
        tree("        self.level = pick(29) + pick(0);")
    );
    none(&source, "untyped_engine_field");
}

#[test]
fn raw_code_naming_extern_bcon_entries_and_its_own_entries_parses() {
    let source = format!(
        "extern bcon Tuning {{ #[addr(16521)] hour; }}\n{}",
        tree(
            "        raw {\n            temp[0] := Tuning.hour;\n            const Local.low = 3;\n            temp[1] := Local.low;\n        }"
        )
    );
    none(&source, "raw_syntax");
}

#[test]
fn raw_locals_are_tree_wide_and_not_constants() {
    let source = format!(
        "const COUNT = 4;\n{}",
        tree("        raw { let COUNT = temp[0]; }\n        raw { COUNT := 1; }")
    );
    none(&source, "raw_syntax");
}

#[test]
fn raw_names_from_other_modules_are_no_syntax_error_but_bad_syntax_is() {
    // Constants and tables a `use` may bring in are not resolved here.
    none(
        &tree("        raw { my.attr[ELSEWHERE] := Other.entry; }"),
        "raw_syntax",
    );
    // A stray identifier is still a syntax error.
    let source = tree("        raw { temp[0] := 1 nonsense; }");
    let d = one(&source, "raw_syntax");
    assert!(d.message.contains("nonsense"), "{d:?}");
}

#[test]
fn a_block_const_that_returns_is_control_flow_not_an_unused_constant() {
    none(
        &tree(
            "        const GATE = if self.level == 1 { return; } else { 1 };\n        self.level = 2;",
        ),
        "unused_decl",
    );
}

#[test]
fn duplicate_std_global_help_names_the_std_parameters() {
    let source = format!(
        "extern mod globals {{\n    #[id(280)] fn wait_ticks(count: i16) -> Result;\n}}\n{}",
        tree("        globals::wait_ticks(1)?;")
    );
    let d = one(&source, "duplicate_std_global");
    assert!(
        d.help.as_deref().unwrap_or("").contains("named `ticks`"),
        "{d:?}"
    );
}

// --- review round 2 regressions ------------------------------------------------------------------

/// A one-tree object around `init` statements.
fn init(body: &str) -> String {
    format!("object \"Probe\" {{ guid: 1; init {{ {body} }} }}\n")
}

/// Whether `check` compiles `source`.
fn compiles(source: &str) -> bool {
    sims_antics::compile_v2_source(
        "probe.antics",
        source,
        &sims_antics::v2::PackResolverConfig::default(),
    )
    .is_ok()
}

#[test]
fn a_functional_guard_sees_the_functional_parameters() {
    let source = "object \"Probe\" { guid: 1; functional prep(me: ObjectId) { guard(sim) => me.data[2] == 0, run(sim) {} } }\n";
    assert!(compiles(source));
    // `me` is the parameter, a handle: its cell 2 is the handle's `container`.
    let d = one(source, "untyped_engine_field");
    assert_eq!(d.suggestion.as_deref(), Some("me.container"), "{d:?}");
}

#[test]
fn me_may_be_an_imported_bcon_table() {
    for source in [
        format!("use crate::imported::me;\n{}", init("temp[0] = me.room;")),
        "use crate::imported::Base;\nobject \"Probe\" from Base { guid: 1; init { temp[0] = me.room; } }\n"
            .to_owned(),
    ] {
        none(&source, "object_tree_me_field");
    }
}

#[test]
fn a_local_me_is_in_scope_only_after_it_and_in_its_block() {
    // The initializer still reads the object's `me`.
    one(
        &init("let me: ObjectId = me.id; temp[0] = me.room;"),
        "object_tree_me_field",
    );
    // After the `if`, `me` is the object again.
    let source = init(
        "if temp[0] == 1 { let me: ObjectId = self.id; temp[1] = me.room; } temp[2] = me.room;",
    );
    let d = one(&source, "object_tree_me_field");
    assert_eq!(d.column, source.rfind("me.room").unwrap() + 1, "{d:?}");
}

#[test]
fn only_cell_groups_of_me_may_be_indexed() {
    let source = init("temp[0] = me.room[0];");
    assert!(!compiles(&source));
    one(&source, "object_tree_me_field");
}

#[test]
fn raw_code_in_a_for_else_is_checked() {
    let bad = init("for p in objects_by_guid(1) {} else { raw { temp[0] := ; } }");
    assert!(!compiles(&bad));
    one(&bad, "raw_syntax");
    let good = format!(
        "const COUNT = 4;\n{}",
        init("for p in objects_by_guid(1) {} else { raw { let COUNT = 0; } } raw { COUNT := 1; }")
    );
    assert!(compiles(&good));
    none(&good, "raw_syntax");
}

#[test]
fn raw_syntax_reports_nothing_check_accepts() {
    for source in [
        // A local in discarded code declares nothing.
        format!(
            "const COUNT = 4;\n{}",
            init("if false { raw { let COUNT = 0; } } raw { temp[COUNT] := 1; }")
        ),
        // One block finishes another's statement.
        init("raw { temp[0] := 1 } raw { ; }"),
    ] {
        assert!(compiles(&source), "{source}");
        none(&source, "raw_syntax");
    }
    // A constant `use` brings in, whatever the error points at, and many of them.
    let names: Vec<String> = (0..40).map(|i| format!("K{i}")).collect();
    let many = names
        .iter()
        .map(|n| format!("temp[{n}] := 1;"))
        .collect::<Vec<_>>()
        .join(" ");
    for body in [
        "raw { sleep(TICKS); }".to_owned(),
        format!("raw {{ {many} }}"),
    ] {
        none(
            &format!("use crate::tuning::*;\n{}", init(&body)),
            "raw_syntax",
        );
    }
}

#[test]
fn raw_syntax_reports_what_check_rejects() {
    for source in [
        // BCON entries are `bcon[..]` operands, which `sleep` does not take.
        format!(
            "extern bcon Tune {{ #[addr(16521)] value; }}\n{}",
            init("raw { sleep(Tune.value); }")
        ),
        "object \"Probe\" { guid: 1; bcon 4096 \"Tune\" { value = 1 } init { raw { sleep(Tune.value); } } }\n"
            .to_owned(),
        // A forward reference to the block's own constant.
        init("raw { temp[VALUE] := 1; const VALUE = 1; }"),
        // A known constant is not declared twice: its own error stays.
        format!("const COUNT = 4;\n{}", init("raw { COUNT := 1; }")),
    ] {
        assert!(!compiles(&source), "{source}");
        let d = one(&source, "raw_syntax");
        assert!(!d.message.contains("duplicate"), "{d:?}");
    }
}

#[test]
fn an_entry_named_like_a_keyword_is_no_control_flow() {
    let source = format!(
        "extern bcon Tune {{ #[addr(16521)] break; }}\n{}",
        init("const UNUSED = if temp[0] == 1 { Tune.break } else { 1 }; temp[1] = 2;")
    );
    one(&source, "unused_decl");
}

#[test]
fn an_unused_binding_with_calls_keeps_them() {
    let source = format!(
        "#[inline]\nfn noisy() -> bool {{\n    self.data[0] = 1;\n    return true;\n}}\n{}",
        init("const UNUSED = if noisy() { 1 } else { 2 }; temp[0] = 1;")
    );
    let d = one(&source, "unused_decl");
    assert!(d.help.as_deref().unwrap_or("").contains("keeping"), "{d:?}");
}

// --- review round 3 regressions ------------------------------------------------------------------

#[test]
fn the_unallowable_errors_need_check_to_reject_the_file() {
    // Each reads like an error to the lint's approximation of lowering, and `check`
    // accepts each: the lowered tree or scope differs from the source body.
    for source in [
        // A template's constant wins over the file's in its objects.
        "const N = 65536;\ntemplate Base { const N = 1; }\nobject \"Probe\" from Base { guid: 1; init { raw { temp[N] := 1; } } }\n"
            .to_owned(),
        // The template's `init` and the object's join in one tree.
        "template Base { init { raw { temp[0] := 1 } } }\nobject \"Probe\" from Base { guid: 1; init { raw { ; } } }\n"
            .to_owned(),
        // A template no object uses is not lowered.
        "template Unused { init { temp[0] = me.room; raw { temp[0] := ; } } }\nobject \"Probe\" { guid: 1; init { temp[0] = 1; } }\n"
            .to_owned(),
    ] {
        assert!(compiles(&source), "{source}");
        none(&source, "raw_syntax");
        none(&source, "object_tree_me_field");
    }
}

#[test]
fn a_raw_constant_after_a_closed_air_comment_is_used() {
    // `.air` comments do not nest: `temp[N]` follows the first `*/`.
    let source = "const N = 1;\nobject \"Probe\" { guid: 1; init { raw { /* a /* b */ temp[N] := 1; # */\n } } }\n";
    assert!(compiles(source));
    none(source, "unused_decl");
}

#[test]
fn unused_binding_help_keeps_tuples_and_method_calls() {
    let tuple =
        init("const (USED, UNUSED) = if temp[0] == 1 { (1, 2) } else { (3, 4) }; temp[1] = USED;");
    let d = one(&tuple, "unused_decl");
    assert_eq!(d.help.as_deref(), Some("write `_` in its place"));
    let method = "object \"Probe\" { guid: 1; #[bhav(label = \"Probe\")] fn probe(sim: Sim) { const UNUSED = if sim.debit_funds(1) { 1 } else { 2 }; temp[0] = 1; } }\n";
    let d = one(method, "unused_decl");
    assert!(d.help.as_deref().unwrap_or("").contains("keeping"), "{d:?}");
}

#[test]
fn duplicate_std_global_help_offers_an_alias() {
    let source = "const globals = 4;\nextern mod timing { #[id(280)] fn wait(ticks: i16) -> Result; }\nobject \"Probe\" { guid: 1; init { timing::wait(globals)?; } }\n";
    let d = one(source, "duplicate_std_global");
    assert!(
        d.help
            .as_deref()
            .unwrap_or("")
            .contains("globals as std_globals"),
        "{d:?}"
    );
}
