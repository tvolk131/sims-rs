//! `.antics` v2 lint, wave 3: `untyped_engine_field` in step with the typed engine words of
//! the typed `std` foundation (`stack_obj`, handles, the new object and Sim words), and the
//! `untyped_engine_constant`, `prefer_question_mark` and `prefer_method_syntax` rules.
//!
//! The lint keeps a finding of these rules only when its rewrite builds the file to the same
//! bytes. The tests apply each suggestion as a user would and compare the builds.
//!
//! Every fixture here is synthetic (GUIDs in the clean-room 0x7E00_xxxx range). See
//! `docs/decisions/20261125-antics-v2-lint-wave3.md`.

use sims_antics::v2::PackResolverConfig;
use sims_antics::v2::lint::{LINT_RULES, LintDiagnostic, Severity, lint_v2_source};

const PREAMBLE: &str = "extern mod lib {\n    #[id(300)] fn must() -> Result;\n    #[id(310)] fn probe_bool() -> bool;\n}\n";

/// A synthetic object with `uses` before it and `members` in it.
fn obj(uses: &str, members: &str) -> String {
    format!(
        "{uses}{PREAMBLE}object \"Synth Probe\" {{\n    guid: 0x7E00_0300;\n    price: 1;\n{members}\n}}\n"
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

/// The `use` line a finding's help asks for, if any.
fn import_of(d: &LintDiagnostic) -> Option<String> {
    let help = d.help.as_deref()?;
    let at = help.find("add `use ")?;
    let rest = &help[at + 5..];
    Some(rest[..rest.find('`')?].to_owned())
}

/// Asserts that `source` has exactly one `rule` finding, on the first line of `original`,
/// whose suggestion is `want`, and that writing it in place of `original` (with the `use`
/// line its help names) builds the same bytes. Returns the rewritten source.
fn rewrites(source: &str, rule: &str, original: &str, want: &str) -> String {
    rewrites_in(source, rule, original, original, want)
}

/// [`rewrites`] where the finding replaces `original` within the unique text `context`.
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

#[test]
fn the_rule_table_lists_the_wave_3_rules() {
    for name in [
        "untyped_engine_constant",
        "prefer_question_mark",
        "prefer_method_syntax",
    ] {
        let r = LINT_RULES
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(r.severity, Severity::Warning);
        assert!(r.suppressible);
    }
}

// --- untyped_engine_field ----------------------------------------------------------------

#[test]
fn self_names_the_new_object_words() {
    for (cell, typed) in [
        ("self.data[27]", "self.weight"),
        ("self.data[28]", "self.support_strength"),
        ("self.data[41]", "self.value"),
        ("self.data[59]", "self.category"),
    ] {
        let read = tree("", &format!("        temp[0] = {cell};"));
        rewrites(&read, "untyped_engine_field", cell, typed);
        let write = tree("", &format!("        {cell} = 5;"));
        rewrites(&write, "untyped_engine_field", cell, typed);
    }
}

#[test]
fn flag_words_are_named_by_their_typed_field() {
    for (cell, typed) in [
        ("self.data[4]", "self.allowed_heights"),
        ("self.data[5]", "self.wall_adjacency"),
        ("self.data[8]", "self.flags"),
        ("self.data[40]", "self.flags2"),
        ("self.data[42]", "self.placement"),
        ("self.data[43]", "self.buy_flags"),
    ] {
        // A bit with no name: only the word is reported.
        let source = tree("", &format!("        {cell} flag_set 16;"));
        rewrites(&source, "untyped_engine_field", cell, typed);
        none(&source, "untyped_engine_constant");
    }
}

#[test]
fn container_is_read_only_and_an_object_id() {
    let source = tree(
        "",
        "        if self.data[2] == stack_obj {\n            temp[0] = 1;\n        }",
    );
    rewrites(
        &source,
        "untyped_engine_field",
        "self.data[2]",
        "self.container",
    );
    // An `i16` use of the cell: `container` is an `ObjectId`.
    none(
        &tree("", "        temp[0] = self.data[2] + 1;"),
        "untyped_engine_field",
    );
}

#[test]
fn an_engine_enum_word_stands_in_only_cast_back() {
    // `facing` is a `Direction`: as an `i16` it does not type-check.
    none(
        &tree("", "        temp[0] = self.data[1];"),
        "untyped_engine_field",
    );
    none(
        &tree("", "        self.data[1] = 3;"),
        "untyped_engine_field",
    );
    let source = tree("", "        temp[0] = self.data[1] as i16;");
    rewrites(
        &source,
        "untyped_engine_field",
        "self.data[1]",
        "self.facing",
    );
    let source = sim_tree("", "        temp[0] = sim.person[65] as i16;");
    rewrites(
        &source,
        "untyped_engine_field",
        "sim.person[65]",
        "sim.gender_kind",
    );
    none(
        &sim_tree("", "        temp[0] = sim.person[65];"),
        "untyped_engine_field",
    );
}

#[test]
fn stack_obj_and_handles_name_their_cells() {
    for (cell, typed) in [
        ("stack_obj.data[41]", "stack_obj.value"),
        ("stack_obj.data[28]", "stack_obj.support_strength"),
        ("stack_obj.person[58]", "stack_obj.age"),
        ("stack_obj.person[65]", "stack_obj.kind"),
    ] {
        let source = tree("", &format!("        temp[0] = {cell};"));
        rewrites(&source, "untyped_engine_field", cell, typed);
    }
    let source = tree(
        "",
        "        if stack_obj.slot[0] == self {\n            temp[0] = 1;\n        }",
    );
    rewrites(
        &source,
        "untyped_engine_field",
        "stack_obj.slot[0]",
        "stack_obj.hand",
    );
    // A local holding a handle.
    let source = tree(
        "",
        "        let h = stack_obj;\n        temp[0] = h.data[41];",
    );
    rewrites(&source, "untyped_engine_field", "h.data[41]", "h.value");
}

#[test]
fn a_sim_names_the_new_person_and_object_words() {
    let source = sim_tree("", "        sim.person[77] = 0;");
    rewrites(
        &source,
        "untyped_engine_field",
        "sim.person[77]",
        "sim.date_partner",
    );
    let source = sim_tree("", "        temp[0] = sim.data[41];");
    rewrites(&source, "untyped_engine_field", "sim.data[41]", "sim.value");
}

#[test]
fn a_state_of_the_same_name_keeps_the_cell() {
    // `self.value` names the state, another cell.
    let source = obj(
        "",
        "    state value: i16;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.value = 1;\n        temp[0] = self.data[41];\n    }",
    );
    none(&source, "untyped_engine_field");
}

#[test]
fn the_expansions_global_is_world_expansions() {
    let source = tree("", "        temp[0] = global[20];");
    rewrites(
        &source,
        "untyped_engine_field",
        "global[20]",
        "world.expansions",
    );
}

#[test]
fn the_earlier_typed_names_still_report() {
    let source = tree("", "        self.data[15] = 1;");
    rewrites(
        &source,
        "untyped_engine_field",
        "self.data[15]",
        "self.broken",
    );
}

// --- duplicate_std_global covers every std::globals tree --------------------------------

#[test]
fn every_std_global_declared_locally_is_a_duplicate() {
    // Each of the 62 `std::globals` lines, declared again with the same ID and ABI.
    let std = include_str!("../../src/v2/std/globals.antics");
    let decls: Vec<&str> = std
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("#[id("))
        .collect();
    assert_eq!(decls.len(), 62);
    let source = format!(
        "extern mod mine {{\n{}\n}}\nobject \"Synth Probe\" {{\n    guid: 0x7E00_0301;\n    price: 1;\n}}\n",
        decls.join("\n")
    );
    let found = of(&source, "duplicate_std_global");
    assert_eq!(found.len(), 62, "{found:#?}");
    // Another ABI is another way to call the tree: no finding.
    let source = "extern mod mine {\n    #[id(271)] fn init_object() -> bool;\n}\nobject \"Synth Probe\" {\n    guid: 0x7E00_0301;\n    price: 1;\n}\n";
    none(source, "duplicate_std_global");
}

// --- untyped_engine_constant --------------------------------------------------------------

#[test]
fn a_flag_bit_gets_its_name_and_the_use_it_needs() {
    let source = tree("", "        self.flags2 flag_set 6;");
    let fixed = rewrites_in(
        &source,
        "untyped_engine_constant",
        "flag_set 6",
        "6",
        "ObjectFlags2::CatchesFire",
    );
    assert!(fixed.starts_with("use std::flags::ObjectFlags2;\n"));
    let d = one(&source, "untyped_engine_constant");
    assert_eq!(
        d.column,
        source.lines().nth(d.line - 1).unwrap().find('6').unwrap() + 1
    );
}

#[test]
fn flag_bits_by_word_number_and_by_name() {
    for (stmt, original, want) in [
        ("self.data[8] flag_clear 6;", "6;", "ObjectFlags::InUse"),
        (
            "stack_obj.flags flag_clear 3;",
            "3;",
            "ObjectFlags::NotSolid",
        ),
        (
            "self.wall_placement flag_set 5;",
            "5;",
            "WallPlacement::NoCorner",
        ),
        ("self.buy_flags flag_set 2;", "2;", "BuyFlags::CanPickUp"),
        ("self.placement flag_set 10;", "10;", "Placement::Air"),
    ] {
        let source = tree("use std::flags::*;\n", &format!("        {stmt}"));
        let d = one(&source, "untyped_engine_constant");
        assert_eq!(import_of(&d), None, "glob import: {d:#?}");
        // The statement's own word may also be untyped; rewrite the bit alone.
        let fixed = source.replacen(stmt, &stmt.replacen(original, &format!("{want};"), 1), 1);
        assert_eq!(d.suggestion.as_deref(), Some(want));
        assert_eq!(iff(&source), iff(&fixed));
    }
}

#[test]
fn the_expansions_mask_and_flag_tests() {
    let source = tree(
        "",
        "        if world.expansions flag_test 6 {\n            temp[0] = 1;\n        }",
    );
    rewrites_in(
        &source,
        "untyped_engine_constant",
        "flag_test 6",
        "6",
        "Expansions::Pets",
    );
}

#[test]
fn prelude_flags_need_no_use() {
    let source = tree("", "        self.allowed_heights flag_set 3;");
    let d = one(&source, "untyped_engine_constant");
    assert_eq!(d.suggestion.as_deref(), Some("Height::Table"));
    assert_eq!(import_of(&d), None);
    rewrites_in(
        &source,
        "untyped_engine_constant",
        "flag_set 3",
        "3",
        "Height::Table",
    );
}

#[test]
fn no_constant_for_an_unnamed_bit_a_named_constant_or_a_plain_word() {
    // Bit 3 of data 40 has no name.
    none(
        &tree("", "        self.flags2 flag_set 3;"),
        "untyped_engine_constant",
    );
    // Already a constant.
    let source = obj(
        "",
        "    const BIT = 6;\n    #[bhav(label = \"Probe\")] fn probe() {\n        self.flags2 flag_set BIT;\n    }",
    );
    none(&source, "untyped_engine_constant");
    // A temp is no engine flag word.
    none(
        &tree("", "        temp[0] flag_set 6;"),
        "untyped_engine_constant",
    );
}

#[test]
fn an_aliased_import_spells_the_alias_and_a_clashing_name_is_left_alone() {
    let source = tree(
        "use std::flags::ObjectFlags2 as Flags2;\n",
        "        self.flags2 flag_set 6;",
    );
    let d = one(&source, "untyped_engine_constant");
    assert_eq!(d.suggestion.as_deref(), Some("Flags2::CatchesFire"));
    assert_eq!(import_of(&d), None);
    // The file's own `ObjectFlags2` is another type.
    let source = tree(
        "bitflags ObjectFlags2 { Mine = 6 }\n",
        "        self.flags2 flag_set 6;",
    );
    none(&source, "untyped_engine_constant");
}

#[test]
fn budget_categories_and_refresh_operands() {
    let source = tree(
        "",
        "        if !can_afford(10, category: 6) {\n            return;\n        }",
    );
    rewrites_in(
        &source,
        "untyped_engine_constant",
        "category: 6",
        "6",
        "BudgetCategory::Buy",
    );
    let source = tree(
        "use std::engine::*;\n",
        "        if debit_funds(10, category: 7) {\n            temp[0] = 1;\n        }",
    );
    rewrites_in(
        &source,
        "untyped_engine_constant",
        "category: 7",
        "7",
        "BudgetCategory::Build",
    );
    // `refresh(target:, mode:)`: both operands.
    let source = tree(
        "use std::engine::*;\n",
        "        refresh(target: 1, mode: 2);",
    );
    let found = of(&source, "untyped_engine_constant");
    let wants: Vec<_> = found
        .iter()
        .map(|d| d.suggestion.clone().unwrap())
        .collect();
    assert_eq!(
        wants,
        ["RefreshTarget::StackObject", "RefreshMode::RecountRoom"]
    );
    let fixed = source.replacen(
        "refresh(target: 1, mode: 2)",
        "refresh(target: RefreshTarget::StackObject, mode: RefreshMode::RecountRoom)",
        1,
    );
    assert_eq!(iff(&source), iff(&fixed));
    // The method form's mode.
    let source = tree("use std::engine::*;\n", "        self.refresh(0);");
    rewrites_in(
        &source,
        "untyped_engine_constant",
        "self.refresh(0)",
        "0",
        "RefreshMode::Redraw",
    );
}

// --- prefer_question_mark -----------------------------------------------------------------

#[test]
fn a_result_call_that_only_returns_its_error() {
    let source = tree(
        "",
        "        if !lib::must() {\n            return error;\n        }\n        temp[0] = 1;",
    );
    rewrites(
        &source,
        "prefer_question_mark",
        "if !lib::must() {\n            return error;\n        }",
        "lib::must()?;",
    );
    let source = tree(
        "",
        "        if lib::must() {\n        } else {\n            return error;\n        }",
    );
    rewrites(
        &source,
        "prefer_question_mark",
        "if lib::must() {\n        } else {\n            return error;\n        }",
        "lib::must()?;",
    );
}

#[test]
fn std_globals_and_local_trees_return_result_too() {
    let source = tree(
        "use std::globals::*;\n",
        "        if !globals::init_object() { return error; }",
    );
    rewrites(
        &source,
        "prefer_question_mark",
        "if !globals::init_object() { return error; }",
        "globals::init_object()?;",
    );
    let source = obj(
        "",
        "    #[bhav(label = \"Helper\")] fn helper(n: i16) -> Result {\n        temp[0] = n;\n        return true;\n    }\n    #[bhav(label = \"Probe\")] fn probe() {\n        if !helper(3) { return error; }\n    }",
    );
    rewrites(
        &source,
        "prefer_question_mark",
        "if !helper(3) { return error; }",
        "helper(3)?;",
    );
}

#[test]
fn no_question_mark_for_bool_calls_other_exits_comments_or_else_if() {
    // A `bool` answer is not an error.
    none(
        &tree("", "        if !lib::probe_bool() { return error; }"),
        "prefer_question_mark",
    );
    // Another exit.
    none(
        &tree("", "        if !lib::must() { return; }"),
        "prefer_question_mark",
    );
    // More than the error in the branch.
    none(
        &tree(
            "",
            "        if !lib::must() {\n            temp[0] = 1;\n            return error;\n        }",
        ),
        "prefer_question_mark",
    );
    // A comment the rewrite would drop.
    none(
        &tree(
            "",
            "        if !lib::must() {\n            // why\n            return error;\n        }",
        ),
        "prefer_question_mark",
    );
    // `else if`: `else f()?;` is no statement.
    none(
        &tree(
            "",
            "        if temp[0] == 1 {\n            temp[1] = 1;\n        } else if !lib::must() {\n            return error;\n        }",
        ),
        "prefer_question_mark",
    );
    // A local `extern mod globals` hides `std::globals`, here with a `bool` tree.
    let source = tree(
        "use std::globals::*;\nextern mod globals {\n    #[id(271)] fn init_object() -> bool;\n}\n",
        "        if !globals::init_object() { return error; }",
    );
    none(&source, "prefer_question_mark");
    // Inside `raw { }`: not v2.
    let source = tree(
        "",
        "        raw {\n            if !call 300() {\n                return error;\n            }\n        }",
    );
    none(&source, "prefer_question_mark");
}

// --- prefer_method_syntax -----------------------------------------------------------------

#[test]
fn sound_calls_take_their_target_as_receiver() {
    let source = tree("", "        play_sound(5, stack_obj);");
    rewrites(
        &source,
        "prefer_method_syntax",
        "play_sound(5, stack_obj)",
        "stack_obj.play_sound(5)",
    );
    let source = tree("", "        play_sound(5, target: stack_obj);");
    rewrites(
        &source,
        "prefer_method_syntax",
        "play_sound(5, target: stack_obj)",
        "stack_obj.play_sound(5)",
    );
    let source = tree("", "        stop_all_sounds(stack_obj);");
    rewrites(
        &source,
        "prefer_method_syntax",
        "stop_all_sounds(stack_obj)",
        "stack_obj.stop_all_sounds()",
    );
}

#[test]
fn refresh_takes_me_or_the_stack_object_as_receiver() {
    let source = tree("", "        refresh(target: 0, mode: 2);");
    let d = of(&source, "prefer_method_syntax");
    assert_eq!(d.len(), 1);
    rewrites(
        &source,
        "prefer_method_syntax",
        "refresh(target: 0, mode: 2)",
        "self.refresh(2)",
    );
    let source = sim_tree("", "        refresh(target: 0, mode: 2);");
    rewrites(
        &source,
        "prefer_method_syntax",
        "refresh(target: 0, mode: 2)",
        "sim.refresh(2)",
    );
    let source = tree("", "        refresh(target: 1, mode: 0);");
    rewrites(
        &source,
        "prefer_method_syntax",
        "refresh(target: 1, mode: 0)",
        "stack_obj.refresh(0)",
    );
    // An inline helper runs in its caller's frame: `me` is the target.
    let source = obj(
        "",
        "    fn helper() {\n        refresh(target: 0, mode: 2);\n    }\n    #[bhav(label = \"Probe\")] fn probe() {\n        helper();\n    }",
    );
    rewrites(
        &source,
        "prefer_method_syntax",
        "refresh(target: 0, mode: 2)",
        "me.refresh(2)",
    );
}

#[test]
fn push_interaction_and_remove_object_take_their_object_as_receiver() {
    let source = tree(
        "",
        "        push_interaction(object: stack_obj, menu: 0, priority: 1);",
    );
    rewrites(
        &source,
        "prefer_method_syntax",
        "push_interaction(object: stack_obj, menu: 0, priority: 1)",
        "stack_obj.push_interaction(menu: 0, priority: 1)",
    );
    let source = tree("", "        remove_object(stack_obj);");
    rewrites(
        &source,
        "prefer_method_syntax",
        "remove_object(stack_obj)",
        "stack_obj.remove()",
    );
}

#[test]
fn no_method_form_for_a_user_function_or_a_call_used_as_a_value() {
    // A user function named like the builtin takes the call.
    let source = obj(
        "",
        "    fn play_sound(a: i16, b: i16) {\n        temp[0] = a + b;\n    }\n    #[bhav(label = \"Probe\")] fn probe() {\n        play_sound(5, 6);\n    }",
    );
    none(&source, "prefer_method_syntax");
    // Only statements are rewritten.
    none(
        &tree(
            "",
            "        if remove_object(stack_obj) {\n            temp[0] = 1;\n        }",
        ),
        "prefer_method_syntax",
    );
    // A comment inside the call would be lost.
    none(
        &tree("", "        play_sound(5, /* here */ stack_obj);"),
        "prefer_method_syntax",
    );
}

// --- suppression and the open pack --------------------------------------------------------

#[test]
fn the_new_rules_can_be_allowed() {
    let source = tree(
        "",
        "        // antics-lint: allow(prefer_method_syntax)\n        play_sound(5, stack_obj);",
    );
    none(&source, "prefer_method_syntax");
    none(&source, "stale_lint_allow");
}

#[test]
fn the_open_pack_has_no_wave_3_finding() {
    let pack = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../open-pack");
    let (diags, _) = sims_antics::v2::lint::lint_v2_paths(&[pack]).expect("lint");
    assert!(diags.is_empty(), "{diags:#?}");
}
