//! `.antics` v2 lint, wave 6: the rules in step with #475's `panic!()` and `let _ = expr;`,
//! and `unused_decl` reading `Table::entry` BCON paths (#476's lift spelling).
//!
//! Every fixture here is synthetic (GUIDs in the clean-room 0x7E00_xxxx range). See
//! `docs/decisions/20261125-antics-v2-lint-panic-and-bcon-paths.md`.

use sims_antics::v2::PackResolverConfig;
use sims_antics::v2::lint::{LintDiagnostic, Severity, lint_v2_source};

const PREAMBLE: &str = "extern mod lib {\n    #[id(300)] fn must() -> Result;\n    #[id(310)] fn probe_bool() -> bool;\n}\n";

/// A synthetic object with `uses` before it and `members` in it.
fn obj(uses: &str, members: &str) -> String {
    format!(
        "{uses}{PREAMBLE}object \"Synth Probe\" {{\n    guid: 0x7E00_0600;\n    price: 1;\n    #[slot(2)] state level: i16;\n{members}\n}}\n"
    )
}

/// An object-context tree running `body`.
fn tree(uses: &str, body: &str) -> String {
    obj(
        uses,
        &format!("    #[bhav(label = \"Probe\")] fn probe() {{\n{body}\n    }}"),
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

/// The `use` line a finding's help asks for, if any.
fn import_of(d: &LintDiagnostic) -> Option<String> {
    let help = d.help.as_deref()?;
    let at = help.find("add `use ")?;
    let rest = &help[at + 5..];
    Some(rest[..rest.find('`')?].to_owned())
}

/// Asserts that `source` has exactly one `rule` finding, on the first line of `context`,
/// whose suggestion is `want`, and that writing it in place of `original` within the unique
/// text `context` (with the `use` line its help names) builds the same bytes.
fn rewrites_in(source: &str, rule: &str, context: &str, original: &str, want: &str) -> String {
    let d = one(source, rule);
    assert_eq!(d.suggestion.as_deref(), Some(want), "{d:#?}");
    let first = context.lines().next().unwrap_or(context);
    assert_eq!(d.line, line_of(source, first), "{d:#?}");
    assert_eq!(d.severity, Severity::Warning);
    assert_eq!(
        source.matches(context).count(),
        1,
        "{context:?} must be unique"
    );
    assert_eq!(
        context.matches(original).count(),
        1,
        "{original:?} in {context:?}"
    );
    let mut fixed = source.replacen(context, &context.replacen(original, want, 1), 1);
    if let Some(line) = import_of(&d) {
        fixed = format!("{line}\n{fixed}");
    }
    assert_eq!(
        iff(source),
        iff(&fixed),
        "rewrite builds other bytes:\n{fixed}"
    );
    fixed
}

fn rewrites(source: &str, rule: &str, original: &str, want: &str) -> String {
    rewrites_in(source, rule, original, original, want)
}

// --- panic!(): unreachable_code -------------------------------------------------------------

#[test]
fn code_after_panic_is_unreachable_and_says_panic() {
    let source = tree("", "        panic!();\n        self.level = 1;");
    let d = one(&source, "unreachable_code");
    assert_eq!(d.line, line_of(&source, "self.level = 1"));
    assert_eq!(
        d.note.as_deref(),
        Some(
            format!(
                "`panic!()` on line {} always leaves this block",
                line_of(&source, "panic!();")
            )
            .as_str()
        )
    );
    // `return error;` is a `return` (of a value named `error`), and says so.
    let source = tree("", "        return error;\n        self.level = 1;");
    let d = one(&source, "unreachable_code");
    assert_eq!(
        d.note.as_deref(),
        Some(
            format!(
                "`return` on line {} always leaves this block",
                line_of(&source, "return error;")
            )
            .as_str()
        )
    );
}

#[test]
fn an_if_whose_branches_all_panic_or_return_leaves() {
    let source = tree(
        "",
        "        if self.level == 0 {\n            panic!();\n        } else {\n            return;\n        }\n        self.level = 4;",
    );
    let d = one(&source, "unreachable_code");
    assert_eq!(d.line, line_of(&source, "self.level = 4"));
    // One branch that falls through: reachable.
    none(
        &tree(
            "",
            "        if self.level == 0 {\n            panic!();\n        }\n        self.level = 4;",
        ),
        "unreachable_code",
    );
    // A `panic!()` in a loop body ends the loop like `return`: nothing after it in
    // the body is reachable, but the loop itself may still be left by `break`.
    let source = tree(
        "",
        "        loop {\n            if self.level == 0 {\n                break;\n            }\n            panic!();\n            self.level = 1;\n        }\n        self.level = 2;",
    );
    let d = one(&source, "unreachable_code");
    assert_eq!(d.line, line_of(&source, "self.level = 1"));
}

// --- panic!(): prefer_question_mark ---------------------------------------------------------

#[test]
fn a_result_call_that_only_panics_is_a_question_mark() {
    let source = tree(
        "",
        "        if !lib::must() {\n            panic!();\n        }\n        self.level = 1;",
    );
    rewrites(
        &source,
        "prefer_question_mark",
        "if !lib::must() {\n            panic!();\n        }",
        "lib::must()?;",
    );
    let source = tree(
        "",
        "        if lib::must() {\n        } else {\n            panic!();\n        }",
    );
    rewrites(
        &source,
        "prefer_question_mark",
        "if lib::must() {\n        } else {\n            panic!();\n        }",
        "lib::must()?;",
    );
    // `return error;` returns a value named `error`: it is no error exit, so no `?`.
    let source = tree("", "        if !lib::must() { return error; }");
    none(&source, "prefer_question_mark");
}

#[test]
fn panic_is_the_error_exit_even_beside_a_binding_named_error() {
    // `return error;` returns the local; `panic!()` and `?` never do.
    let source = tree(
        "",
        "        let error = random(3);\n        self.level = error;\n        if !lib::must() {\n            panic!();\n        }",
    );
    rewrites(
        &source,
        "prefer_question_mark",
        "if !lib::must() {\n            panic!();\n        }",
        "lib::must()?;",
    );
    let source = tree(
        "",
        "        let error = random(3);\n        self.level = error;\n        if !lib::must() {\n            return error;\n        }",
    );
    none(&source, "prefer_question_mark");
}

#[test]
fn no_question_mark_for_a_panicking_bool_call_or_a_busier_branch() {
    none(
        &tree("", "        if !lib::probe_bool() { panic!(); }"),
        "prefer_question_mark",
    );
    none(
        &tree(
            "",
            "        if !lib::must() {\n            self.level = 1;\n            panic!();\n        }",
        ),
        "prefer_question_mark",
    );
    none(
        &tree(
            "",
            "        if !lib::must() {\n            // why\n            panic!();\n        }",
        ),
        "prefer_question_mark",
    );
    none(
        &tree(
            "",
            "        if self.level == 1 {\n            self.level = 2;\n        } else if !lib::must() {\n            panic!();\n        }",
        ),
        "prefer_question_mark",
    );
    // `if f() { panic!(); }` panics on success: not `?`.
    none(
        &tree("", "        if lib::must() { panic!(); }"),
        "prefer_question_mark",
    );
}

#[test]
fn the_rule_summary_names_panic() {
    let r = sims_antics::v2::lint::LINT_RULES
        .iter()
        .find(|r| r.name == "prefer_question_mark")
        .unwrap();
    assert!(r.summary.contains("panic!()"), "{}", r.summary);
}

// --- let _ = expr ---------------------------------------------------------------------------

#[test]
fn the_discard_is_never_an_unused_local() {
    for body in [
        "        let _ = random(3);",
        "        let _ = lib::must();\n        let _ = lib::probe_bool();",
        "        let _ = 5;",
    ] {
        let source = tree("", &format!("        self.level = 1;\n{body}"));
        none(&source, "unused_decl");
    }
}

#[test]
fn reads_inside_a_discard_count() {
    let source = tree(
        "",
        "        self.level = 1;\n        let y = random(3);\n        let _ = random(y);",
    );
    none(&source, "unused_decl");
    // The same local unread is still flagged.
    let d = one(
        &tree(
            "",
            "        self.level = 1;\n        let y = random(3);\n        let _ = random(4);",
        ),
        "unused_decl",
    );
    assert_eq!(d.message, "local `y` is never read");
    // A read of a local nothing assigned yet is caught inside the discard too.
    let source = tree(
        "",
        "        let x: i16;\n        let _ = random(x);\n        x = 1;\n        self.level = x;",
    );
    let d = one(&source, "uninitialized_local_read");
    assert_eq!(d.line, line_of(&source, "let _ = random(x);"));
    none(
        &tree(
            "",
            "        let x: i16;\n        x = 1;\n        let _ = random(x);",
        ),
        "uninitialized_local_read",
    );
}

#[test]
fn a_discarded_call_is_no_dropped_bool_call() {
    none(
        &tree("", "        let _ = lib::must();"),
        "dropped_bool_call",
    );
    none(
        &tree("", "        let _ = can_afford(5);"),
        "dropped_bool_call",
    );
    // The bare statement still is, and its help names the discard.
    let d = one(&tree("", "        lib::must();"), "dropped_bool_call");
    assert!(
        d.help
            .as_deref()
            .unwrap()
            .contains("`let _ = lib::must(..);`"),
        "{d:#?}"
    );
    let d = one(&tree("", "        can_afford(5);"), "dropped_bool_call");
    assert!(
        d.help
            .as_deref()
            .unwrap()
            .contains("`let _ = can_afford(..);`"),
        "{d:#?}"
    );
}

#[test]
fn a_discarded_call_still_takes_method_syntax() {
    let source = tree("", "        let _ = play_sound(5, stack_obj);");
    rewrites(
        &source,
        "prefer_method_syntax",
        "play_sound(5, stack_obj)",
        "stack_obj.play_sound(5)",
    );
    // As before, only a call standing for the whole statement.
    none(
        &tree("", "        let _ = random(5) + 1;"),
        "prefer_method_syntax",
    );
}

#[test]
fn engine_words_inside_a_discard_get_their_names() {
    let source = tree("", "        let _ = random(self.data[41]);");
    rewrites(
        &source,
        "untyped_engine_field",
        "self.data[41]",
        "self.value",
    );
    let source = tree("", "        let _ = self.flags2 flag_test 6;");
    rewrites_in(
        &source,
        "untyped_engine_constant",
        "flag_test 6",
        "6",
        "ObjectFlags2::CatchesFire",
    );
}

// --- Table::entry BCON reads ----------------------------------------------------------------

const TUNING: &str = "    bcon 4097 \"Tuning\" { low = 1, high = 9 }\n";

/// An object with the `Tuning` table and `members`, before which `uses` stand. No `extern
/// mod`: one may read any BCON by number, which turns the entry check off.
fn bcon_obj(uses: &str, members: &str) -> String {
    format!(
        "{uses}object \"Synth Probe\" {{\n    guid: 0x7E00_0601;\n    price: 1;\n    #[slot(2)] state level: i16;\n{TUNING}{members}\n}}\n"
    )
}

fn bcon_probe(body: &str) -> String {
    bcon_obj(
        "",
        &format!("    #[bhav(label = \"Probe\")] fn probe() {{\n{body}\n    }}"),
    )
}

fn unused_bcon(source: &str) -> Vec<String> {
    let mut found: Vec<String> = of(source, "unused_decl")
        .into_iter()
        .map(|d| d.message)
        .filter(|m| m.starts_with("BCON entry"))
        .collect();
    found.sort_unstable();
    found
}

#[test]
fn a_path_read_of_a_bcon_entry_counts() {
    let source = bcon_probe("        self.level = Tuning::low + Tuning::high;");
    assert!(unused_bcon(&source).is_empty(), "{:#?}", lint(&source));
    // Both spellings build the same tree.
    assert_eq!(
        iff(&source),
        iff(&bcon_probe(
            "        self.level = Tuning.low + Tuning.high;"
        ))
    );
    // Mixed: one by path, one by field.
    let source = bcon_probe("        self.level = Tuning::low + Tuning.high;");
    assert!(unused_bcon(&source).is_empty(), "{:#?}", lint(&source));
    // The other entry unread is still flagged.
    let source = bcon_probe("        self.level = Tuning::low;");
    assert_eq!(
        unused_bcon(&source),
        ["BCON entry `high` of table 4097 is never read"]
    );
}

#[test]
fn a_path_through_another_qualifier_does_not_count() {
    // `Motive::high`-shaped paths name some other table or type, not this entry.
    let source = bcon_obj(
        "enum Other { high, low }\n",
        "    #[bhav(label = \"Probe\")] fn probe() {\n        self.level = Tuning::low;\n        self.level = Other::high as i16;\n    }",
    );
    assert_eq!(
        unused_bcon(&source),
        ["BCON entry `high` of table 4097 is never read"]
    );
}

#[test]
fn an_enum_or_bitflags_of_the_table_name_wins_the_path() {
    // The type, not the BCON table, owns `Tuning::high`: the entry stays unread.
    let source = bcon_obj(
        "enum Tuning { low, high }\n",
        "    #[bhav(label = \"Probe\")] fn probe() {\n        self.level = Tuning.low;\n        self.level = Tuning::high as i16;\n    }",
    );
    assert_eq!(
        unused_bcon(&source),
        ["BCON entry `high` of table 4097 is never read"]
    );
    let source = bcon_obj(
        "",
        "    bitflags Tuning { low = 1, high = 2 }\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.level = Tuning.low;\n        self.level = Tuning::high as i16;\n    }",
    );
    assert_eq!(
        unused_bcon(&source),
        ["BCON entry `high` of table 4097 is never read"]
    );
}

#[test]
fn a_called_path_does_not_count() {
    // `globals::init_object()` calls a global tree; it reads no entry of a table whose
    // snake-case name is `globals`.
    let source = format!(
        "use std::globals::*;\n{}",
        bcon_obj(
            "",
            "    bcon 4098 \"Globals\" { init_object = 1, spare = 2 }\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.level = Tuning::low + Tuning::high + Globals::spare;\n        globals::init_object()?;\n    }",
        )
    );
    assert_eq!(
        unused_bcon(&source),
        ["BCON entry `init_object` of table 4098 is never read"]
    );
    // Read by path, it counts.
    let read = source.replace("Globals::spare", "globals::init_object + Globals::spare");
    assert!(unused_bcon(&read).is_empty(), "{:#?}", lint(&read));
    iff(&read);
}

#[test]
fn an_imported_name_wins_the_path() {
    // A table labeled like an imported type: `ObjectFlags2::CatchesFire` names the flag.
    let source = format!(
        "use std::flags::ObjectFlags2;\n{}",
        bcon_obj(
            "",
            "    bcon 4098 \"ObjectFlags2\" { CatchesFire = 1, spare = 2 }\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.level = Tuning::low + Tuning::high + ObjectFlags2.spare;\n        self.flags2 flag_set ObjectFlags2::CatchesFire;\n    }",
        )
    );
    assert_eq!(
        unused_bcon(&source),
        ["BCON entry `CatchesFire` of table 4098 is never read"]
    );
}

#[test]
fn a_grouped_callee_is_a_call_too() {
    let source = format!(
        "use std::globals::*;\n{}",
        bcon_obj(
            "",
            "    bcon 4098 \"Globals\" { init_object = 1, spare = 2 }\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.level = Tuning::low + Tuning::high + Globals::spare;\n        (globals::init_object)()?;\n    }",
        )
    );
    assert_eq!(
        unused_bcon(&source),
        ["BCON entry `init_object` of table 4098 is never read"]
    );
    // A grouped read is still a read.
    let read = bcon_probe("        self.level = (Tuning::low) + Tuning::high;");
    assert!(unused_bcon(&read).is_empty(), "{:#?}", lint(&read));
}

#[test]
fn glob_imported_and_prelude_types_win_the_path() {
    // `use std::flags::*` brings `ObjectFlags2`; the path is the flag.
    let source = format!(
        "use std::flags::*;\n{}",
        bcon_obj(
            "",
            "    bcon 4098 \"ObjectFlags2\" { CatchesFire = 1, spare = 2 }\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.level = Tuning::low + Tuning::high + ObjectFlags2.spare;\n        self.flags2 flag_set ObjectFlags2::CatchesFire;\n    }",
        )
    );
    assert_eq!(
        unused_bcon(&source),
        ["BCON entry `CatchesFire` of table 4098 is never read"]
    );
    // `Height` is a prelude type, named with no `use`.
    let source = bcon_obj(
        "",
        "    bcon 4098 \"Height\" { Floor = 41, spare = 42 }\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.level = Tuning::low + Tuning::high + Height.spare;\n        self.allowed_heights flag_set Height::Floor;\n    }",
    );
    assert_eq!(
        unused_bcon(&source),
        ["BCON entry `Floor` of table 4098 is never read"]
    );
}

#[test]
fn a_type_declared_in_another_body_does_not_take_the_path() {
    // `Unrelated`'s enum is not in scope in the object: `Tuning::high` reads the table.
    let source = format!(
        "pub template Unrelated {{\n    enum Tuning {{ high, low }}\n}}\n{}",
        bcon_probe("        self.level = Tuning.low + Tuning::high;")
    );
    assert!(unused_bcon(&source).is_empty(), "{:#?}", lint(&source));
    // The object's own template's enum is.
    let source = format!(
        "pub template Base {{\n    enum Tuning {{ high, low }}\n}}\n{}",
        bcon_probe("        self.level = Tuning.low;\n        self.level = Tuning::high as i16;")
            .replacen(
                "object \"Synth Probe\" {",
                "object \"Synth Probe\" from Base {",
                1
            )
    );
    assert_eq!(
        unused_bcon(&source),
        ["BCON entry `high` of table 4097 is never read"]
    );
}

#[test]
fn a_discarded_relationship_write_is_not_sure() {
    // `relationship` writes `value:` only when the row exists; `let _ =` builds the same as
    // the call statement.
    for call in [
        "relationship(column: 0, dir: 0, flags: 0, local: who, value: n);",
        "let _ = relationship(column: 0, dir: 0, flags: 0, local: who, value: n);",
    ] {
        let source = tree(
            "",
            &format!(
                "        let who: i16 = 0;\n        let n: i16;\n        {call}\n        self.level = n;"
            ),
        );
        let d = one(&source, "uninitialized_local_read");
        assert_eq!(d.line, line_of(&source, "self.level = n;"));
    }
    let bare = tree(
        "",
        "        let who: i16 = 0;\n        let n: i16;\n        relationship(column: 0, dir: 0, flags: 0, local: who, value: n);\n        self.level = n;",
    );
    let discard = bare.replace("relationship(", "let _ = relationship(");
    assert_eq!(iff(&bare), iff(&discard));
}

#[test]
fn a_dropped_is_type_names_its_receiver() {
    let d = one(
        &tree("", "        stack_obj.is_type(0x7E00_0200);"),
        "dropped_bool_call",
    );
    let help = d.help.as_deref().unwrap();
    assert!(
        help.contains("`if stack_obj.is_type(..) { ... }`"),
        "{help}"
    );
    assert!(help.contains("`let _ = stack_obj.is_type(..);`"), "{help}");
}

#[test]
fn a_nested_grouped_callee_is_a_call_too() {
    let source = format!(
        "use std::globals::*;\n{}",
        bcon_obj(
            "",
            "    bcon 4098 \"Globals\" { init_object = 1, spare = 2 }\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.level = Tuning::low + Tuning::high + Globals::spare;\n        ((globals::init_object))()?;\n    }",
        )
    );
    assert_eq!(
        unused_bcon(&source),
        ["BCON entry `init_object` of table 4098 is never read"]
    );
}

#[test]
fn an_objects_own_type_wins_the_path_in_its_template() {
    // Lowering merges the template's members with the object's: the object's enum takes
    // `Tuning::high` in the template's tree too.
    let source = format!(
        "pub template Base {{\n    #[bhav(label = \"Read\")] fn read() {{\n        self.level = Tuning::high as i16;\n    }}\n}}\n{}",
        bcon_probe("        self.level = Tuning.low;")
            .replacen(
                "object \"Synth Probe\" {",
                "object \"Synth Probe\" from Base {",
                1
            )
            .replacen(
                "    #[slot(2)]",
                "    enum Tuning { high }\n    #[slot(2)]",
                1
            )
    );
    iff(&source);
    assert_eq!(
        unused_bcon(&source),
        ["BCON entry `high` of table 4097 is never read"]
    );
    // Without the enum, the template's path reads the object's table.
    let source = format!(
        "pub template Base {{\n    #[bhav(label = \"Read\")] fn read() {{\n        self.level = Tuning::high;\n    }}\n}}\n{}",
        bcon_probe("        self.level = Tuning.low;").replacen(
            "object \"Synth Probe\" {",
            "object \"Synth Probe\" from Base {",
            1
        )
    );
    iff(&source);
    assert!(unused_bcon(&source).is_empty(), "{:#?}", lint(&source));
}

#[test]
fn a_path_reads_only_its_own_objects_table() {
    // Two objects with a "Tuning" table: one reads `Tuning::high` of its own table, which
    // says nothing of the other's.
    let reader = "object \"Reader\" {\n    guid: 0x7E00_0602;\n    #[slot(2)] state level: i16;\n    bcon 4097 \"Tuning\" { high = 27 }\n    init { self.level = Tuning::high; }\n}\n";
    let other = "object \"Other\" {\n    guid: 0x7E00_0603;\n    #[slot(2)] state level: i16;\n    bcon 4097 \"Tuning\" { base = 15, high = 72 }\n    init { self.level = Tuning::base; }\n}\n";
    let untouched = "object \"Untouched\" {\n    guid: 0x7E00_0604;\n    #[slot(2)] state level: i16;\n    enum Tuning { high }\n    bcon 4097 \"Tuning\" { high = 41, spare = 42 }\n    init { self.level = Tuning::high as i16; }\n}\n";
    let input = |name: &str, source: &str| sims_antics::v2::lint::LintInput {
        name: name.to_owned(),
        source: source.to_owned(),
        path: None,
        pack_dir: None,
    };
    let found = sims_antics::v2::lint::lint_v2_sources(&[
        input("reader.antics", reader),
        input("other.antics", other),
        input("untouched.antics", untouched),
    ])
    .unwrap();
    let mut bcon: Vec<(String, String)> = found
        .into_iter()
        .filter(|d| d.message.starts_with("BCON entry"))
        .map(|d| (d.file, d.message))
        .collect();
    bcon.sort_unstable();
    assert_eq!(
        bcon,
        [(
            "other.antics".to_owned(),
            "BCON entry `high` of table 4097 is never read".to_owned()
        )]
    );
}

#[test]
fn a_dropped_is_type_on_a_field_names_the_field() {
    let d = one(
        &tree("", "        self.container.is_type(0x7E00_0200);"),
        "dropped_bool_call",
    );
    let help = d.help.as_deref().unwrap();
    assert!(
        help.contains("`let _ = self.container.is_type(..);`"),
        "{help}"
    );
}

#[test]
fn same_named_objects_in_two_files_keep_their_own_reads() {
    let object = |guid: &str, body: &str| {
        format!(
            "object \"Shared Name\" {{\n    guid: {guid};\n    #[slot(2)] state level: i16;\n    bcon 4097 \"Tuning\" {{ base = 15, high = 72 }}\n    init {{ {body} }}\n}}\n"
        )
    };
    let input = |name: &str, source: String| sims_antics::v2::lint::LintInput {
        name: name.to_owned(),
        source,
        path: None,
        pack_dir: None,
    };
    let found = sims_antics::v2::lint::lint_v2_sources(&[
        input(
            "reader.antics",
            object("0x7E00_0605", "self.level = Tuning::base + Tuning::high;"),
        ),
        input(
            "other.antics",
            object("0x7E00_0606", "self.level = Tuning::base;"),
        ),
    ])
    .unwrap();
    let bcon: Vec<(String, String)> = found
        .into_iter()
        .filter(|d| d.message.starts_with("BCON entry"))
        .map(|d| (d.file, d.message))
        .collect();
    assert_eq!(
        bcon,
        [(
            "other.antics".to_owned(),
            "BCON entry `high` of table 4097 is never read".to_owned()
        )]
    );
}

#[test]
fn a_module_level_helper_read_counts_for_the_objects() {
    let source = "extern bcon Tuning {\n    #[addr(129)] high;\n}\nfn read_high() -> i16 { return Tuning::high; }\nobject \"Extern Probe\" {\n    guid: 0x7E00_0607;\n    #[slot(2)] state level: i16;\n    bcon 4097 \"Tuning\" { base = 41, high = 42, spare = 43 }\n    init { self.level = Tuning::base + read_high(); }\n}\n";
    iff(source);
    assert_eq!(
        unused_bcon(source),
        ["BCON entry `spare` of table 4097 is never read"]
    );
}
