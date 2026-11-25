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
    // `@slot(N)` pins the frame word for code that addresses it by number, which may
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
fn a_const_written_as_its_value_in_raw_is_used() {
    // The value written in place of the name is a use too.
    none(
        &obj(
            "    const LEVEL_SLOT = 7;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.mode = Mode::Off;\n        self.level = 0;\n        raw { attr[7] := 1; }\n    }",
        ),
        "unused_decl",
    );
    // A frame word (`temp[7]`) is not one.
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
    assert_eq!(d.line, line_of(&source, "tree big"));

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
    assert_eq!(d.line, line_of(&source, "#[id(4200)]"), "{d:?}");
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
