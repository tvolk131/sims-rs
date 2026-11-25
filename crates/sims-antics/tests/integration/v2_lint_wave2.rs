//! `.antics` v2 lint, wave 2: `set_to_next(.., dest:)` as a read and a write, BCON entries
//! read by name inside `raw { }`, PR I-3's forms in `manual_raw_has_builtin`, and the
//! `stale_lint_allow` and `unreachable_code` rules.
//!
//! Every fixture here is synthetic (GUIDs in the clean-room 0x7E00_xxxx range). Each item has
//! positive and negative cases. See `docs/decisions/20261125-antics-v2-lint-wave2.md`.

use sims_antics::v2::lint::{
    LINT_RULES, LintDiagnostic, LintInput, Severity, lint_v2_source, lint_v2_sources,
};

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

/// No finding at all.
fn clean(source: &str) {
    let found = lint(source);
    assert!(
        found.is_empty(),
        "unexpected findings in\n{source}\n{found:#?}"
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
fn the_rule_table_lists_the_wave_2_rules() {
    for name in ["stale_lint_allow", "unreachable_code"] {
        let r = LINT_RULES
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(r.severity, Severity::Warning);
        assert!(r.suppressible);
    }
}

// --- set_to_next(.., dest: x) --------------------------------------------------------------

#[test]
fn set_to_next_dest_reads_the_cursor_it_steps() {
    // Set To Next starts from the cursor in `dest:`: an unassigned local there reads 0.
    let source = tree(
        "        let c: ObjectId;\n        while set_to_next(person, dest: c) {\n            self.level += 1;\n        }",
    );
    assert!(compiles(&source));
    let d = one(&source, "uninitialized_local_read");
    assert_eq!(d.line, line_of(&source, "dest: c"));
    // Read by the step itself, so never unused, even with no other read.
    none(&source, "unused_decl");

    let source = tree(
        "        let c: ObjectId = 0;\n        while set_to_next(person, dest: c) {\n            self.level += 1;\n        }",
    );
    assert!(compiles(&source));
    clean(&source);
}

#[test]
fn set_to_next_dest_writes_the_next_cursor() {
    // Assigned first, stepped, then read: nothing to report.
    let source = tree(
        "        let c: i16;\n        c = 0;\n        set_to_next(person, dest: c);\n        self.level = c;",
    );
    assert!(compiles(&source));
    clean(&source);
    // A read in another argument of the step is still a read of the unassigned local.
    let source = tree(
        "        let c: i16;\n        let g: i16;\n        set_to_next(person, dest: c);\n        self.level = c + g;",
    );
    let found = of(&source, "uninitialized_local_read");
    let lines: Vec<usize> = found.iter().map(|d| d.line).collect();
    assert_eq!(
        lines,
        [line_of(&source, "dest: c"), line_of(&source, "c + g")],
        "{found:#?}"
    );
}

#[test]
fn set_to_next_dest_into_param_and_temp_is_no_local() {
    let source = obj(
        "    #[bhav(label = \"Probe\", args = 2)] fn probe() {\n        self.level = 0;\n        self.mode = Mode::Off;\n        if set_to_next(part, dest: param[1]) {\n            self.level = 1;\n        }\n        set_to_next(neighbor, dest: temp[2]);\n    }",
    );
    assert!(compiles(&source));
    clean(&source);
}

#[test]
fn a_local_only_written_by_a_builtin_is_unused() {
    // `random_into(x, ..)` only writes `x`: with no read after, the local is unused.
    let d = one(
        &tree("        let roll: i16 = 0;\n        random_into(roll, 5)?;"),
        "unused_decl",
    );
    assert_eq!(d.message, "local `roll` is never read");
    // `set_to_next(.., dest: x)` reads `x` before it writes it.
    none(
        &tree("        let c: ObjectId = 0;\n        set_to_next(person, dest: c);"),
        "unused_decl",
    );
    // A read in another argument counts.
    none(
        &tree("        let roll: i16 = 3;\n        random_into(roll, roll)?;"),
        "unused_decl",
    );
}

#[test]
fn a_write_the_lint_cannot_tell_from_a_read_counts_as_read() {
    // A user tree named `random_into` takes the call: its arguments are plain reads.
    let source = obj(
        "    #[bhav(label = \"Roll\")] fn random_into(x: i16, n: i16) { self.level = x + n; }\n    \
         #[bhav(label = \"Probe\")] fn probe() {\n        self.level = 0;\n        self.mode = Mode::Off;\n        \
         let roll: i16 = 2;\n        random_into(roll, 5);\n    }",
    );
    none(&source, "unused_decl");
    // `relationship` with named flags may carry flag 4, which reads `value:`.
    let source = sim_tree(
        "        let n: i16 = 3;\n        relationship(column: 0, dir: 0, flags: SOME_FLAGS, local: 0, value: n);",
    )
    .replace("object \"", "const SOME_FLAGS = 4;\nobject \"");
    none(&source, "unused_decl");
    // With literal flags, no flag 4: a pure write.
    one(
        &sim_tree(
            "        let n: i16 = 3;\n        relationship(column: 0, dir: 0, flags: 0, local: 0, value: n);",
        ),
        "unused_decl",
    );
}

#[test]
fn an_animate_statement_local_is_only_written() {
    let source = sim_tree(
        "        let event: i16 = 0;\n        animate(clip: 1, table: 0, events: 2, local: event) |_e| {\n            sim.person[1] = 1;\n        }",
    );
    let d = one(&source, "unused_decl");
    assert_eq!(d.message, "local `event` is never read");
    // Read in the event handler: used.
    let source = sim_tree(
        "        let event: i16 = 0;\n        animate(clip: 1, table: 0, events: 2, local: event) |_e| {\n            if event == 0 { sim.person[1] = 1; }\n        }",
    );
    none(&source, "unused_decl");
}

// --- BCON entries read by name in raw code ------------------------------------------------

#[test]
fn bcon_entries_read_by_name_inside_raw_are_used() {
    let source = obj(
        "    bcon 4097 \"Tuning\" { low = 1, high = 9, mid = 5, spare = 7 }\n    \
         #[bhav(label = \"Probe\")] fn probe() {\n        self.level = Tuning.low;\n        self.mode = Mode::Off;\n        \
         raw { temp[0] := Tuning.high; }\n        raw { temp[1] := Tuning::mid; }\n    }",
    );
    let found = of(&source, "unused_decl");
    let messages: Vec<&str> = found.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(
        messages,
        ["BCON entry `spare` of table 4097 is never read"],
        "{found:#?}"
    );
    // In a comment or a string of the raw code, a path is no read.
    let source = obj("    bcon 4097 \"Tuning\" { low = 1, high = 9 }\n    \
         #[bhav(label = \"Probe\")] fn probe() {\n        self.level = Tuning.low;\n        self.mode = Mode::Off;\n        \
         raw {\n            # Tuning.high\n            temp[0] := 1; // Tuning::high\n        }\n    }");
    one(&source, "unused_decl");
}

// --- manual_raw_has_builtin: PR I-3 forms --------------------------------------------------

/// The help of the one `manual_raw_has_builtin` finding of `body` (a Sim-context tree).
fn raw_help(body: &str) -> String {
    let source = sim_tree(body);
    one(&source, "manual_raw_has_builtin")
        .help
        .unwrap_or_default()
}

fn no_raw_builtin(body: &str) {
    none(&sim_tree(body), "manual_raw_has_builtin");
}

#[test]
fn raw_remove_object_has_a_builtin() {
    for (stmt, v2) in [
        ("remove_object(stack_obj);", "use remove_object(stack_obj)"),
        (
            "remove_object(1, flags: 2);",
            "use remove_object(stack_obj, flags: 2)",
        ),
        ("remove_object(0, flags: 0);", "use remove_object(me)"),
        ("remove_object(self);", "use remove_object(me)"),
        ("remove_object(0x0);", "use remove_object(me)"),
        (
            "remove_object(0x1, flags: 0x2);",
            "use remove_object(stack_obj, flags: 2)",
        ),
        (
            "remove_object(me, no_yield: true, skip_resets: false);",
            "use remove_object(me, flags: 1)",
        ),
        (
            "remove_object(1, flags: 1, skip_resets: true);",
            "use remove_object(stack_obj, flags: 3)",
        ),
    ] {
        assert_eq!(raw_help(&format!("        raw {{ {stmt} }}")), v2, "{stmt}");
    }
    // The v2 spelling compiles where the raw one did.
    let raw = sim_tree("        raw { remove_object(1, flags: 2); }");
    let v2 = sim_tree("        remove_object(stack_obj, flags: 2);");
    assert!(compiles(&raw) && compiles(&v2));
}

#[test]
fn raw_remove_object_of_a_shadowed_name_stays_raw() {
    // A local `me` would take the v2 spelling's target.
    no_raw_builtin(
        "        let me: i16 = 0;\n        raw { remove_object(0); }\n        self.level = me;",
    );
    // A user function named `remove_object` would take the call.
    let source = obj(
        "    #[bhav(label = \"Gone\")] fn remove_object(x: i16) { self.level = x; }\n    \
         #[bhav(label = \"Probe\")] fn probe(sim: Sim) {\n        self.level = 0;\n        self.mode = Mode::Off;\n        \
         raw { remove_object(1); }\n    }",
    );
    none(&source, "manual_raw_has_builtin");
}

#[test]
fn raw_remove_object_without_a_v2_spelling_stays_raw() {
    for stmt in [
        // Mode 2 has no v2 spelling.
        "remove_object(2);",
        // Maxis's pad bytes.
        "remove_object(0, flags: 2) pad 0xff;",
        // A flags byte out of range, an unknown option.
        "remove_object(1, flags: 300);",
        "remove_object(1, now: true);",
        // Its own edges.
        "remove_object(1) -> true: true, false: false;",
    ] {
        no_raw_builtin(&format!("        raw {{ {stmt} }}"));
    }
    // One statement v2 cannot spell keeps the whole block.
    no_raw_builtin(
        "        raw {\n            remove_object(1);\n            temp[0] := 3;\n        }",
    );
}

#[test]
fn raw_find_best_object_has_a_builtin() {
    for stmt in [
        "find_best_object(cook);",
        "find_best_object(clean, flags: 4);",
        "find_best_object(repair, 4);",
        // Function 14 is `repair`.
        "find_best_object(14, flags: 0);",
    ] {
        // In an object's own tree, where `self` is the object.
        let help = one(
            &tree(&format!("        raw {{ {stmt} }}")),
            "manual_raw_has_builtin",
        )
        .help
        .unwrap_or_default();
        assert_eq!(
            help, "use `find_best_object(..)` as an `if` / `while` test",
            "{stmt}"
        );
    }
    for stmt in [
        // Never searched: v2 refuses it.
        "find_best_object(stand);",
        "find_best_object(9);",
        // A custom origin has no v2 spelling.
        "find_best_object(cook, origin: temp[0]);",
        // No functional name: v2 spells `category4` only for the untyped search.
        "find_best_object(category4);",
        "find_best_object(cook) -> true: @a, false: false;",
    ] {
        none(
            &tree(&format!("        raw {{ {stmt} }}")),
            "manual_raw_has_builtin",
        );
    }
    // In a Sim's body, v2 puts the entry stack object back for a later `self`; the raw
    // search leaves the found one there.
    no_raw_builtin("        raw { find_best_object(prep); }\n        self.attr[0] = 42;");
}

/// A Sim-context tree with two parameters.
fn sim_tree_with_args(body: &str) -> String {
    obj(&format!(
        "    #[bhav(label = \"Probe\", args = 2)] fn probe(sim: Sim) {{\n        self.level = 0;\n        self.mode = Mode::Off;\n{body}\n    }}"
    ))
}

#[test]
fn raw_set_to_next_dest_has_a_builtin() {
    for stmt in [
        "set_to_next(person, dest: temp[3]);",
        "set_to_next(part, into: temp[0]);",
        "set_to_next(neighbor, dest: temp[7]);",
    ] {
        let help = raw_help(&format!(
            "        raw {{ {stmt} }}\n        self.level = 1;"
        ));
        assert_eq!(help, "use set_to_next(.., dest: ..)", "{stmt}");
    }
    // A parameter below the tree's argument count.
    for stmt in [
        "set_to_next(3, guid: 0x7E00_0101, dest: param[0]);",
        "set_to_next(all, dest: param[1]);",
    ] {
        let source = sim_tree_with_args(&format!(
            "        raw {{ {stmt} }}\n        self.level = 1;"
        ));
        let d = one(&source, "manual_raw_has_builtin");
        assert_eq!(
            d.help.as_deref(),
            Some("use set_to_next(.., dest: ..)"),
            "{stmt}"
        );
    }
}

#[test]
fn raw_set_to_next_into_a_missing_param_or_shadowed_temp_stays_raw() {
    // The tree has no arguments: `param[0]` has no v2 spelling there.
    no_raw_builtin("        raw { set_to_next(all, dest: param[0]); }\n        self.level = 1;");
    // Past the argument count.
    none(
        &sim_tree_with_args(
            "        raw { set_to_next(all, dest: param[2]); }\n        self.level = 1;",
        ),
        "manual_raw_has_builtin",
    );
    // A local named `temp` would take `temp[3]` in v2.
    no_raw_builtin(
        "        let temp: i16 = 0;\n        raw { set_to_next(person, dest: temp[3]); }\n        self.level = temp;",
    );
}

#[test]
fn raw_set_to_next_without_a_v2_spelling_stays_raw() {
    for (lets, stmt) in [
        // There are 8 temps.
        ("", "set_to_next(person, dest: temp[8]);"),
        // No `dest:`, or the stack object (out of this rule's scope).
        ("", "set_to_next(person);"),
        ("", "set_to_next(person, dest: stack_obj);"),
        // `beside` takes `local:`.
        ("", "set_to_next(beside, dest: temp[0], local: 1);"),
        ("", "set_to_next(9, dest: temp[0]);"),
        // A v2 local: raw code cannot name it.
        (
            "let c: ObjectId = 0;\n        ",
            "set_to_next(all, dest: c);",
        ),
        ("let n = 0;\n        ", "set_to_next(neighbor, dest: n);"),
        // A typed handle, a bool, a parameter, a raw block's own local.
        (
            "let o: Object = stack_obj;\n        ",
            "set_to_next(all, dest: o);",
        ),
        ("let b = true;\n        ", "set_to_next(all, dest: b);"),
        ("", "set_to_next(all, dest: sim);"),
        (
            "raw { let mine = 0; }\n        ",
            "set_to_next(all, dest: mine);",
        ),
        // A GUID by name, two destinations.
        ("", "set_to_next(guid, SOME_GUID, dest: temp[0]);"),
        ("", "set_to_next(all, dest: temp[0], dest: temp[1]);"),
    ] {
        no_raw_builtin(&format!(
            "        {lets}raw {{ {stmt} }}\n        self.level = 1;"
        ));
    }
}

#[test]
fn raw_definition_and_job_cells_have_names() {
    for (stmt, v2) in [
        ("temp[0] := scope(21, 9);", "use stack_obj.object_type"),
        ("temp[7] := scope(21,11);", "use stack_obj.sub_index"),
        ("temp[1] := scope(33, 12);", "use job_field(12)"),
        ("temp[1] := scope(0x15, 9);", "use stack_obj.object_type"),
    ] {
        let help = raw_help(&format!(
            "        raw {{ {stmt} }}\n        self.level = 1;"
        ));
        assert_eq!(help, v2, "{stmt}");
    }
    // Two primitives, two spellings.
    assert_eq!(
        raw_help(
            "        raw {\n            temp[0] := scope(21, 9);\n            remove_object(1);\n        }"
        ),
        "use stack_obj.object_type, remove_object(stack_obj)"
    );
}

#[test]
fn raw_job_and_search_forms_under_a_user_function_stay_raw() {
    // A user tree named like the builtin takes the v2 call.
    for (helper, stmt) in [
        (
            "    #[bhav(label = \"Job\")] #[ret(temp = 0)] fn job_field(n: i16) -> i16 { return n; }\n    ",
            "temp[1] := scope(33, 12);",
        ),
        (
            "    #[bhav(label = \"Find\")] fn find_best_object(n: i16) -> bool { return n == 1; }\n    ",
            "find_best_object(cook);",
        ),
        (
            "    #[bhav(label = \"Step\")] fn set_to_next(n: i16) -> bool { return n == 1; }\n    ",
            "set_to_next(person, dest: temp[3]);",
        ),
    ] {
        let source = obj(&format!(
            "{helper}#[bhav(label = \"Probe\")] fn probe(sim: Sim) {{\n        self.level = 0;\n        self.mode = Mode::Off;\n        \
             raw {{ {stmt} }}\n        self.level = 1;\n    }}"
        ));
        none(&source, "manual_raw_has_builtin");
    }
}

#[test]
fn raw_definition_cells_without_a_v2_name_stay_raw() {
    for (lets, stmt) in [
        // Definition words with no v2 name.
        ("", "temp[0] := scope(21, 65);"),
        // `has_function` (scope 35) is a bool and raw code cannot name a v2 local.
        ("let has: bool = false;\n        ", "has := scope(35, 26);"),
        ("", "temp[0] := scope(21, 9) + 1;"),
        // `has_function` is a bool: not into an `i16`.
        ("", "temp[0] := scope(35, 26);"),
        ("let t: i16 = 0;\n        ", "t := scope(35, 26);"),
        // An object-type word is no bool.
        ("let has: bool = false;\n        ", "has := scope(21, 9);"),
        // A cell v2 spells another way, a temp past 7, a local by number.
        ("", "my.attr[0] := scope(33, 12);"),
        ("", "temp[8] := scope(21, 9);"),
        ("", "local[0] := scope(21, 9);"),
        // A test, not a store.
        ("", "if scope(21, 9) == 2 { temp[0] := 1; }"),
        ("", "expr(scope(35, 26) != 0) -> true: true, false: false;"),
    ] {
        no_raw_builtin(&format!(
            "        {lets}raw {{ {stmt} }}\n        self.level = 1;"
        ));
    }
}

// --- stale_lint_allow ----------------------------------------------------------------------

#[test]
fn an_allow_that_suppresses_nothing_is_stale() {
    let source = tree(
        "        // antics-lint: allow(while_true)\n        loop {\n            sleep(1);\n        }",
    );
    let d = one(&source, "stale_lint_allow");
    assert_eq!(d.line, line_of(&source, "allow(while_true)"));
    assert_eq!(d.column, 9);
    assert_eq!(
        d.message,
        "`allow(while_true)` suppresses nothing: no `while_true` finding on the code it covers"
    );
    assert_eq!(
        d.help.as_deref(),
        Some("remove `while_true` from the directive")
    );
    // The same allow above the code it is for.
    clean(&tree(
        "        // antics-lint: allow(while_true)\n        while true {\n            sleep(1);\n        }",
    ));
}

#[test]
fn each_rule_of_an_allow_list_is_judged_alone() {
    let source = tree(
        "        // antics-lint: allow(while_true, unused_decl, negated_comparison)\n        while true {\n            sleep(1);\n        }",
    );
    let found = of(&source, "stale_lint_allow");
    let messages: Vec<&str> = found.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "`allow(unused_decl)` suppresses nothing: no `unused_decl` finding on the code it covers",
            "`allow(negated_comparison)` suppresses nothing: no `negated_comparison` finding on the code it covers",
        ]
    );
}

#[test]
fn an_end_of_line_allow_covers_its_line_and_block() {
    // At the end of a block-opening line it covers the block.
    clean(&tree(
        "        if self.level == 0 { // antics-lint: allow(while_true)\n            while true {\n                sleep(1);\n            }\n        }",
    ));
    // At the end of a plain line it covers that line only.
    let source = tree(
        "        self.level = 2; // antics-lint: allow(while_true)\n        while true {\n            sleep(1);\n        }",
    );
    one(&source, "stale_lint_allow");
    one(&source, "while_true");
}

#[test]
fn a_file_allow_is_stale_only_with_no_finding_in_the_file() {
    let used = format!(
        "//! antics-lint: allow(unused_decl)\n{}",
        tree("        let spare = random(5);")
    );
    clean(&used);
    let stale = format!(
        "//! antics-lint: allow(unused_decl)\n{}",
        tree("        sleep(1);")
    );
    let d = one(&stale, "stale_lint_allow");
    assert_eq!(
        d.message,
        "`allow(unused_decl)` suppresses nothing: no `unused_decl` finding in this file"
    );
    assert_eq!(d.line, 1);
}

#[test]
fn an_allow_with_no_code_after_it_is_stale() {
    let source = format!(
        "{}// antics-lint: allow(while_true)\n",
        tree("        sleep(1);")
    );
    let d = one(&source, "stale_lint_allow");
    assert!(d.message.ends_with("(no code follows it)"), "{d:#?}");
}

#[test]
fn unknown_rules_and_plain_comments_are_not_stale() {
    let source = tree("        // antics-lint: allow(no_such_rule)\n        sleep(1);");
    one(&source, "unknown_lint_rule");
    none(&source, "stale_lint_allow");
    clean(&tree(
        "        // antics-lint is quiet here; allow(while_true) is just prose\n        sleep(1);",
    ));
}

#[test]
fn an_allow_of_an_unsuppressible_rule_is_judged_too() {
    // With the error there, the allow is not stale: the error stands with its note.
    let source = tree(
        "        match self.mode {\n            Mode::Off => sleep(1),\n            Mode::Low => sleep(2),\n            Mode::High => sleep(3),\n            // antics-lint: allow(enum_wildcard_all_covered)\n            _ => sleep(4),\n        }",
    );
    let d = one(&source, "enum_wildcard_all_covered");
    assert!(
        d.note
            .as_deref()
            .unwrap_or("")
            .contains("cannot be allowed"),
        "{d:#?}"
    );
    none(&source, "stale_lint_allow");
    // With no error, it is stale and says it could never have been allowed.
    let source =
        tree("        // antics-lint: allow(enum_wildcard_all_covered)\n        sleep(1);");
    let d = one(&source, "stale_lint_allow");
    assert!(
        d.note
            .as_deref()
            .unwrap_or("")
            .starts_with("`enum_wildcard_all_covered` cannot be allowed"),
        "{d:#?}"
    );
}

#[test]
fn stale_lint_allow_can_itself_be_allowed() {
    let source = format!(
        "//! antics-lint: allow(stale_lint_allow)\n{}",
        tree("        // antics-lint: allow(while_true)\n        sleep(1);")
    );
    clean(&source);
}

#[test]
fn no_stale_allow_in_a_file_that_does_not_build() {
    // Rules that read the build (`frame_headroom`, the `$Local:N` slots) are silent when a
    // file does not build, so an allow of them cannot be judged.
    let source = obj(
        "    #[bhav(label = \"Probe\")] fn probe(o: ObjectId) {\n        self.level = 0;\n        self.mode = Mode::Off;\n        // antics-lint: allow(frame_headroom)\n        remove_object(o);\n    }",
    );
    assert!(!compiles(&source));
    none(&source, "stale_lint_allow");
}

#[test]
fn an_allow_inside_raw_code_is_judged() {
    let source = tree(
        "        raw {\n            // antics-lint: allow(raw_bare_call_false_exit)\n            call 4096(temps);\n        }\n        raw {\n            // antics-lint: allow(raw_bare_call_false_exit)\n            temp[0] := 1;\n        }\n        sleep(1);",
    );
    let d = one(&source, "stale_lint_allow");
    assert_eq!(d.line, line_of(&source, "temp[0] := 1") - 1);
}

// --- unreachable_code ----------------------------------------------------------------------

#[test]
fn a_statement_after_return_break_or_continue_is_unreachable() {
    let source = tree("        return;\n        self.level = 1;");
    let d = one(&source, "unreachable_code");
    assert_eq!(d.line, line_of(&source, "self.level = 1"));
    assert_eq!(d.column, 9);
    assert_eq!(
        d.note.as_deref(),
        Some(
            format!(
                "`return` on line {} always leaves this block",
                line_of(&source, "return;")
            )
            .as_str()
        )
    );
    let source = tree("        return error;\n        sleep(1);");
    one(&source, "unreachable_code");
    let source = tree(
        "        for i in 0..3 {\n            if i == 1 {\n                continue;\n                self.level = i;\n            }\n            self.level = 2;\n        }",
    );
    let d = one(&source, "unreachable_code");
    assert_eq!(d.line, line_of(&source, "self.level = i"));
    let source = tree(
        "        loop {\n            sleep(1);\n            break;\n            self.level = 3;\n        }",
    );
    one(&source, "unreachable_code");
    // Only the first unreachable statement of a block.
    one(
        &tree("        return;\n        self.level = 1;\n        self.level = 2;"),
        "unreachable_code",
    );
}

#[test]
fn a_statement_after_an_if_or_match_that_always_leaves_is_unreachable() {
    let source = tree(
        "        if self.level == 0 {\n            return;\n        } else if self.level == 1 {\n            sleep(1);\n            return;\n        } else {\n            return error;\n        }\n        self.level = 4;",
    );
    let d = one(&source, "unreachable_code");
    assert_eq!(d.line, line_of(&source, "self.level = 4"));
    assert!(d.note.unwrap().starts_with("every branch of the `if`"));
    let source = tree(
        "        match self.level {\n            0 => return,\n            1 => {\n                sleep(1);\n                return;\n            }\n            _ => return,\n        }\n        sleep(1);",
    );
    one(&source, "unreachable_code");
}

#[test]
fn code_that_may_run_is_not_unreachable() {
    for body in [
        // No `else`: the false test goes on.
        "        if self.level == 0 {\n            return;\n        }\n        self.level = 1;",
        // One branch goes on.
        "        if self.level == 0 {\n            return;\n        } else {\n            sleep(1);\n        }\n        self.level = 1;",
        // No `_` arm: a value no arm names goes on.
        "        match self.mode {\n            Mode::Off => return,\n            Mode::Low => return,\n            Mode::High => return,\n        }\n        self.level = 1;",
        // `?` returns only on failure.
        "        random_into(self.level, 5)?;\n        self.level += 1;",
        // A loop's `break` leaves the loop, not the block after it.
        "        loop {\n            sleep(1);\n            break;\n        }\n        self.level = 1;",
        // A `return` that is the last statement.
        "        sleep(1);\n        return;",
    ] {
        none(&tree(body), "unreachable_code");
    }
}

#[test]
fn a_trailing_return_after_a_leaving_if_is_not_unreachable() {
    let source = obj(
        "    #[bhav(label = \"Probe\")] fn probe() -> bool {\n        self.level = 0;\n        self.mode = Mode::Off;\n        if self.level == 0 {\n            return true;\n        } else {\n            return false;\n        }\n        return false;\n    }",
    );
    none(&source, "unreachable_code");
    // After a plain `return`, it is.
    let source = obj(
        "    #[bhav(label = \"Probe\")] fn probe() -> bool {\n        self.level = 0;\n        self.mode = Mode::Off;\n        return true;\n        return false;\n    }",
    );
    one(&source, "unreachable_code");
}

#[test]
fn raw_labels_after_a_return_may_be_reached() {
    // A jump in an earlier raw block may land on a label after the `return`.
    let source = tree(
        "        raw { expr(temp[0] == 1) -> true: @there, false: true; }\n        return;\n        raw {\n            @there: temp[1] := 2;\n        }\n        self.level = 1;",
    );
    none(&source, "unreachable_code");
    // A raw block with no label is as unreachable as any other statement.
    let source = tree("        return;\n        raw { temp[1] := 2; }");
    one(&source, "unreachable_code");
    // A label in a comment or a string is no label.
    let source = tree(
        "        return;\n        raw {\n            # @there: not a label\n            temp[1] := 2;\n        }",
    );
    one(&source, "unreachable_code");
}

#[test]
fn raw_edges_may_reach_code_after_a_return() {
    // A raw edge may name a node by label or by number anywhere later in the tree.
    for edge in ["true: there", "true: 2", "-> true: @there, false: true"] {
        let jump = if edge.starts_with("->") {
            format!("expr(temp[0] == 1) {edge};")
        } else {
            format!("expr(temp[0] == 1) -> {edge}, false: true;")
        };
        let source = tree(&format!(
            "        raw {{ {jump} }}\n        return;\n        self.level = 1;"
        ));
        none(&source, "unreachable_code");
    }
}

// --- review round 2 ------------------------------------------------------------------------

#[test]
fn sim_parameters_and_event_bindings_shadow_builtin_names() {
    // A Sim parameter named `param` owns `param[0]`.
    let source = obj(
        "    #[bhav(label = \"Probe\", args = 2)] fn probe(param: Sim) {\n        self.level = 0;\n        self.mode = Mode::Off;\n        raw { set_to_next(person, dest: param[0]); }\n    }",
    );
    none(&source, "manual_raw_has_builtin");
    // An animation's `|me|` / `|param|` event binding owns the name in its handler.
    no_raw_builtin(
        "        animate(clip: 1, table: 0, events: 2) |me| {\n            raw { remove_object(0); }\n        }",
    );
    let source = sim_tree_with_args(
        "        animate(clip: 1, table: 0, events: 2) |param| {\n            raw { set_to_next(person, dest: param[0]); }\n        }",
    );
    none(&source, "manual_raw_has_builtin");
    // Outside the handler, the builtin name is free again.
    let source = sim_tree(
        "        animate(clip: 1, table: 0, events: 2) |me| {\n            sim.person[1] = 1;\n        }\n        raw { remove_object(0); }",
    );
    one(&source, "manual_raw_has_builtin");
}

#[test]
fn a_tree_label_cannot_supply_the_argument_count() {
    let source = obj(
        "    #[bhav(id = 4150, label = \"Before \\\"args: 2\\\"\")] fn probe() {\n        self.level = 0;\n        self.mode = Mode::Off;\n        raw { set_to_next(person, dest: param[0]); }\n    }",
    );
    none(&source, "manual_raw_has_builtin");
}

#[test]
fn imported_helpers_take_a_builtin_name() {
    let source = format!(
        "use crate::shared::helpers::{{wipe as remove_object, draw as random_into}};\n{}",
        obj(
            "    init {\n        raw { remove_object(0); }\n        let x = 3;\n        random_into(x, 10);\n    }"
        )
    );
    none(&source, "manual_raw_has_builtin");
    assert!(
        !of(&source, "unused_decl")
            .iter()
            .any(|d| d.message == "local `x` is never read")
    );
}

#[test]
fn another_files_methods_do_not_take_a_builtin_name() {
    let input = |name: &str, source: String| LintInput {
        name: name.to_owned(),
        source,
        path: None,
        pack_dir: None,
    };
    let definer = "object \"Definer\" {\n    guid: 0x7E00_0121;\n    #[bhav(label = \"Gone\")] fn remove_object(which: i16) { temp[0] = which; }\n    #[bhav(label = \"Roll\")] fn random_into(a: i16, b: i16) { temp[2] = a + b; }\n    init { remove_object(7); random_into(3, 4); }\n}\n";
    let caller = obj(
        "    init {\n        raw { remove_object(0); }\n        let x = 3;\n        random_into(x, 10);\n    }",
    );
    let found = lint_v2_sources(&[
        input("caller.antics", caller),
        input("definer.antics", definer.to_owned()),
    ])
    .unwrap();
    let rules: Vec<(&str, &str)> = found
        .iter()
        .filter(|d| d.file == "caller.antics")
        .map(|d| (d.rule, d.message.as_str()))
        .collect();
    assert!(
        rules.iter().any(|(r, _)| *r == "manual_raw_has_builtin")
            && rules.contains(&("unused_decl", "local `x` is never read")),
        "{found:#?}"
    );
}

#[test]
fn a_user_tree_named_animate_reads_its_arguments() {
    let source = obj(
        "    #[bhav(label = \"Anim\")] fn animate(local: i16) { self.level = local; }\n    init {\n        self.mode = Mode::Off;\n        let x = 2;\n        animate(local: x);\n    }",
    );
    none(&source, "unused_decl");
    let source = obj(
        "    #[bhav(label = \"Anim\")] fn animate(local: i16) { self.level = local; }\n    init {\n        self.mode = Mode::Off;\n        let x: i16;\n        animate(local: x);\n        self.level = x;\n    }",
    );
    one(&source, "uninitialized_local_read");
}

#[test]
fn an_animation_event_read_reads_its_local() {
    // `e` is the frame word `local: x` names.
    let source = sim_tree(
        "        let x = 0;\n        animate(clip: 1, table: 0, events: 2, local: x) |e| {\n            self.level = e;\n        }",
    );
    none(&source, "unused_decl");
}

// --- review round 3 ------------------------------------------------------------------------

#[test]
fn a_template_outside_the_linted_files_may_take_any_builtin_name() {
    // `Base` is not among the linted files: its members are unknown.
    let source = "use crate::shared::base::Base;\nobject \"Probe\" from Base {\n    guid: 0x7E00_1003;\n    init {\n        raw { remove_object(1); }\n        let x = 12;\n        animate(local: x);\n    }\n}\n";
    none(source, "manual_raw_has_builtin");
    none(source, "unused_decl");
}

#[test]
fn a_glob_import_does_not_turn_a_builtin_write_into_a_read() {
    let source = "use crate::shared::base::*;\nobject \"Glob Probe\" {\n    guid: 0x7E00_1104;\n    init {\n        let x: i16;\n        random_into(x, 10);\n        temp[1] = x;\n    }\n}\n";
    none(source, "uninitialized_local_read");
}

#[test]
fn escaped_labels_find_their_own_frame() {
    let source = obj(
        "    #[bhav(label = \"Before \\\"X\\\"\", args = 2)] fn first() {\n        temp[0] = 42;\n    }\n    \
         #[bhav(label = \"Before \\\\\\\"X\\\\\\\"\")] fn second() {\n        self.level = 0;\n        self.mode = Mode::Off;\n        raw { set_to_next(person, dest: param[0]); }\n    }",
    );
    none(&source, "manual_raw_has_builtin");
    // The first tree's own frame does have two arguments.
    let source = obj(
        "    #[bhav(label = \"Before \\\"X\\\"\", args = 2)] fn first() {\n        self.level = 0;\n        self.mode = Mode::Off;\n        raw { set_to_next(person, dest: param[1]); }\n    }",
    );
    one(&source, "manual_raw_has_builtin");
}

#[test]
fn a_defer_body_runs_after_the_assignments_that_follow_it() {
    let source = tree(
        "        let x: ObjectId;\n        defer { set_to_next(person, dest: x); }\n        x = 0;",
    );
    none(&source, "uninitialized_local_read");
}

#[test]
fn the_initialization_help_keeps_the_declared_type() {
    let source = tree(
        "        let x: ObjectId;\n        set_to_next(person, dest: x);\n        self.level = x.room;",
    );
    let d = one(&source, "uninitialized_local_read");
    assert_eq!(
        d.help.as_deref(),
        Some(
            "assign it before this read, or initialize it: `let x: ObjectId = 0;` (one more node)"
        )
    );
}
