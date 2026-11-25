//! `.antics` v2 lint, wave 4: in step with PR I-4 (#469). `std` glob imports bring in their
//! modules' `pub fn`s, `untyped_engine_field` knows person word 81 (`fame`), and
//! `manual_raw_has_builtin` knows #469's spellings: `padN:` bytes on `play_sound` /
//! `clear_motive_changes`, `reach(.., slot:)`, and the scope 12 / 24 / 27 / 30 / 31 / 32 cells.
//!
//! Each raw form the rule names is checked against its v2 spelling: the two builds come out
//! byte for byte the same.
//!
//! Every fixture here is synthetic (GUIDs in the clean-room 0x7E00_xxxx range). See
//! `docs/decisions/20261125-antics-v2-lint-wave4.md`.

use sims_antics::v2::PackResolverConfig;
use sims_antics::v2::lint::{LintDiagnostic, Severity, lint_v2_source};

/// A synthetic object with `uses` before it and `members` in it.
fn obj(uses: &str, members: &str) -> String {
    format!(
        "{uses}object \"Synth Probe\" {{\n    guid: 0x7E00_0400;\n    price: 1;\n{members}\n}}\n"
    )
}

/// An object-context tree running `body`.
fn tree(body: &str) -> String {
    obj(
        "",
        &format!("    #[bhav(label = \"Probe\")] fn probe(a: i16) {{\n{body}\n    }}"),
    )
}

/// A Sim-context tree running `body`, with an `i16` argument `a` after the Sim.
fn sim_tree(body: &str) -> String {
    obj(
        "",
        &format!("    #[bhav(label = \"Probe\")] fn probe(sim: Sim, a: i16) {{\n{body}\n    }}"),
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

const RAW_RULE: &str = "manual_raw_has_builtin";

/// `raw { <raw> }` in a tree built by `wrap` gets one `manual_raw_has_builtin` hint whose help
/// is `use <help>`, and the v2 statement `v2` builds the same bytes as the raw block.
fn same_as_raw(wrap: fn(&str) -> String, raw: &str, help: &str, v2: &str) {
    let raw_src = wrap(&format!("        raw {{ {raw} }}"));
    let d = one(&raw_src, RAW_RULE);
    assert_eq!(d.severity, Severity::Warning, "{d:#?}");
    assert_eq!(
        d.help.as_deref(),
        Some(format!("use {help}").as_str()),
        "{raw}"
    );
    assert_eq!(d.suggestion, None);
    let v2_src = wrap(&format!("        {v2}"));
    assert_eq!(iff(&raw_src), iff(&v2_src), "{raw} vs {v2}");
    none(&v2_src, RAW_RULE);
}

/// `raw { <raw> }` in a tree built by `wrap` gets no `manual_raw_has_builtin` hint.
fn stays_raw(wrap: fn(&str) -> String, raw: &str) {
    none(&wrap(&format!("        raw {{ {raw} }}")), RAW_RULE);
}

// --- std glob imports ---------------------------------------------------------------------

#[test]
fn std_glob_helpers_are_the_files_functions() {
    // Every `pub fn` of the std modules, called through a glob import: the lint reads them as
    // the file's functions and reports nothing about them.
    let source = obj(
        "use std::math::*;\nuse std::motives::*;\nuse std::budget::*;\nuse std::engine::BudgetCategory;\n",
        "    #[bhav(label = \"Probe\")] fn probe(sim: Sim) {\n        temp[0] = clamp(max(min(temp[1], 4), 0), 0, abs(temp[2]));\n        if chance_percent(50) && try_charge(sim, 5, BudgetCategory::Buy) {\n            temp[3] = 1;\n        }\n    }",
    );
    iff(&source);
    let found = lint(&source);
    assert!(found.is_empty(), "{found:#?}");
}

// --- untyped_engine_field: fame -------------------------------------------------------------

#[test]
fn person_word_81_is_fame() {
    for (body, original, want) in [
        (
            "        temp[0] = sim.person[81];",
            "sim.person[81]",
            "sim.fame",
        ),
        (
            "        temp[0] = stack_obj.person[81];",
            "stack_obj.person[81]",
            "stack_obj.fame",
        ),
    ] {
        let source = sim_tree(body);
        let d = one(&source, "untyped_engine_field");
        assert_eq!(d.suggestion.as_deref(), Some(want), "{d:#?}");
        let fixed = source.replacen(original, want, 1);
        assert_eq!(iff(&source), iff(&fixed), "{fixed}");
    }
}

// --- manual_raw_has_builtin: pads ---------------------------------------------------------

#[test]
fn padded_sounds_have_a_builtin() {
    for (raw, help, v2) in [
        (
            "play_sound(0, stack_obj) pad(3: 0x01);",
            "play_sound(.., stack_obj, padN: 0xNN)",
            "play_sound(0, stack_obj, pad3: 0x01);",
        ),
        (
            "play_sound(7, true) pad(6: 0xff, 7: 0xff);",
            "play_sound(.., stack_obj, padN: 0xNN)",
            "play_sound(7, stack_obj, pad7: 0xff, pad6: 0xff);",
        ),
        (
            "play_sound(5) pad(2: 0x10);",
            "play_sound(.., false, padN: 0xNN)",
            "play_sound(5, false, pad2: 0x10);",
        ),
        (
            "play_sound(5, stack_obj: false) pad(5: 0x01);",
            "play_sound(.., false, padN: 0xNN)",
            "play_sound(5, false, pad5: 0x01);",
        ),
        (
            "clear_motive_changes() pad(2: 0x05);",
            "clear_motive_changes(padN: 0xNN)",
            "clear_motive_changes(pad2: 0x05);",
        ),
        (
            "clear_motive_changes();",
            "clear_motive_changes()",
            "clear_motive_changes();",
        ),
    ] {
        same_as_raw(sim_tree, raw, help, v2);
        same_as_raw(tree, raw, help, v2);
    }
}

#[test]
fn sounds_without_a_v2_spelling_stay_raw() {
    for raw in [
        // Bytes the call spells.
        "play_sound(0, stack_obj) pad(0: 0x01);",
        "play_sound(0) pad(4: 0x02);",
        // A fill pad, a byte past 7, and one named twice.
        "play_sound(0) pad 0xff;",
        "play_sound(0) pad(8: 0x01);",
        "play_sound(0) pad(3: 0x01, 3: 0x02);",
        // `flags:`, a non-literal sound, and no pad (wave 1 leaves that raw).
        "play_sound(0, flags: 4) pad(3: 0x01);",
        "play_sound(temp[0]) pad(3: 0x01);",
        "play_sound(0, stack_obj);",
        // Clear Motive Changes takes no arguments.
        "clear_motive_changes(1);",
        // A pad on any other primitive.
        "refresh(target: 0, mode: 1) pad(3: 0x01);",
    ] {
        stays_raw(sim_tree, raw);
    }
}

// --- manual_raw_has_builtin: reach --------------------------------------------------------

#[test]
fn reach_has_a_builtin() {
    for (raw, help, v2) in [
        (
            "reach(mode: 1, moves: true, param: 0);",
            "reach(mode: 1, moves: .., slot: param[..])",
            "reach(mode: 1, moves: true, slot: a);",
        ),
        (
            "reach(mode: 1, moves: false, param: param[0]);",
            "reach(mode: 1, moves: .., slot: param[..])",
            "reach(mode: 1, moves: false, slot: param[0]);",
        ),
        (
            "reach(mode: 0, moves: false);",
            "reach(mode: .., moves: ..)",
            "reach(mode: 0, moves: false);",
        ),
        (
            "reach(mode: 2, moves: true, param: 0);",
            "reach(mode: .., moves: ..)",
            "reach(mode: 2, moves: true);",
        ),
    ] {
        same_as_raw(sim_tree, raw, help, v2);
    }
}

#[test]
fn reach_without_a_v2_spelling_stays_raw() {
    for raw in [
        // Mode 1 defaults to parameter 0, which the tree has, but parameter 1 it lacks.
        "reach(mode: 1, moves: true, param: 1);",
        // Another mode with a slot parameter.
        "reach(mode: 0, moves: true, param: 1);",
        // A named key twice, and an unknown key.
        "reach(mode: 0, mode: 1);",
        "reach(mode: 0, bogus: 1);",
    ] {
        stays_raw(sim_tree, raw);
    }
    // No arguments at all: mode 1's parameter has nothing to name.
    stays_raw(
        |b| {
            obj(
                "",
                &format!("    #[bhav(label = \"Probe\")] fn probe() {{\n{b}\n    }}"),
            )
        },
        "reach(mode: 1, moves: true);",
    );
}

// --- manual_raw_has_builtin: scopes -------------------------------------------------------

#[test]
fn scope_cells_have_a_builtin() {
    for (raw, help, v2) in [
        (
            "scope(12, 6) := 100;",
            "advert.delta[6]",
            "advert.delta[6] = 100;",
        ),
        (
            "scope(12, 7) := temp[1];",
            "advert.delta[7]",
            "advert.delta[7] = temp[1];",
        ),
        (
            "temp[2] := scope(12, 3);",
            "advert.delta[3]",
            "temp[2] = advert.delta[3];",
        ),
        (
            "scope(27, 0) := 1;",
            "stack_obj.sprite_flags[temp[..]]",
            "stack_obj.sprite_flags[temp[0]] = 1;",
        ),
        (
            "scope(27, 2) := param[0];",
            "stack_obj.sprite_flags[temp[..]]",
            "stack_obj.sprite_flags[temp[2]] = param[0];",
        ),
        (
            "temp[1] := scope(27, 3);",
            "stack_obj.sprite_flags[temp[..]]",
            "temp[1] = stack_obj.sprite_flags[temp[3]];",
        ),
        (
            "scope(30, 0) := param[0];",
            "me.person[temp[..]]",
            "me.person[temp[0]] = param[0];",
        ),
        (
            "temp[4] := scope(30, 5);",
            "me.person[temp[..]]",
            "temp[4] = me.person[temp[5]];",
        ),
        (
            "temp[7] := scope(31, 1);",
            "stack_obj.person[temp[..]]",
            "temp[7] = stack_obj.person[temp[1]];",
        ),
        (
            "scope(31, 0) := -2;",
            "stack_obj.person[temp[..]]",
            "stack_obj.person[temp[0]] = -2;",
        ),
        (
            "temp[0] := scope(24, 9);",
            "stack_obj.neighbor.family_friend_count",
            "temp[0] = stack_obj.neighbor.family_friend_count;",
        ),
        (
            "temp[0] := scope(24, 2);",
            "stack_obj.neighbor.age",
            "temp[0] = stack_obj.neighbor.age;",
        ),
        (
            "temp[1] := scope(24, 7);",
            "stack_obj.neighbor.field[7]",
            "temp[1] = stack_obj.neighbor.field[7];",
        ),
        (
            "temp[2] := scope(32, 81);",
            "stack_obj.neighbor.person[81]",
            "temp[2] = stack_obj.neighbor.person[81];",
        ),
    ] {
        same_as_raw(sim_tree, raw, help, v2);
        same_as_raw(tree, raw, help, v2);
    }
}

#[test]
fn scope_cells_without_a_v2_spelling_stay_raw() {
    for raw in [
        // A local source, a frame word v2 cannot name.
        "scope(12, 7) := local[1];",
        // Out of range: a motive past 15, a temp past 7, a neighbor field past 9.
        "scope(12, 16) := 1;",
        "scope(27, 8) := 1;",
        "temp[0] := scope(24, 10);",
        // The neighbor cells are read-only.
        "scope(24, 2) := 1;",
        "scope(32, 81) := 1;",
        // A parameter the tree does not have.
        "scope(27, 0) := param[1];",
        // Not a temp destination.
        "param[0] := scope(27, 0);",
        // Another compound operator.
        "scope(12, 6) += 1;",
    ] {
        stays_raw(sim_tree, raw);
    }
}

#[test]
fn rebound_names_keep_the_raw_block() {
    // A local named like the spelling's root takes the name over.
    for (decl, raw) in [
        ("let advert = 1; temp[0] = advert;", "scope(12, 6) := 100;"),
        ("let me = 1; temp[0] = me;", "scope(30, 0) := 1;"),
        ("let temp = 1; a = temp;", "scope(27, 0) := 1;"),
    ] {
        none(
            &sim_tree(&format!("        {decl}\n        raw {{ {raw} }}")),
            RAW_RULE,
        );
    }
    // A user function named like the builtin.
    let source = obj(
        "",
        "    fn reach() { }\n    #[bhav(label = \"Probe\")] fn probe(sim: Sim) {\n        reach();\n        raw { reach(mode: 0, moves: false); }\n    }",
    );
    none(&source, RAW_RULE);
    let source = obj(
        "",
        "    fn clear_motive_changes() { }\n    #[bhav(label = \"Probe\")] fn probe(sim: Sim) {\n        clear_motive_changes();\n        raw { clear_motive_changes() pad(2: 0x05); }\n    }",
    );
    none(&source, RAW_RULE);
}

#[test]
fn make_new_character_stays_raw() {
    // v2 reads skin, age and sex from named locals; a raw block names frame words.
    stays_raw(
        sim_tree,
        "make_new_character(skin: local[0], age: local[1], sex: local[2], species: 0);",
    );
}

#[test]
fn a_block_mixing_forms_lists_each_spelling_once() {
    let source = sim_tree(
        "        raw {\n            scope(27, 0) := 1;\n            scope(27, 1) := 0;\n            play_sound(0, stack_obj) pad(3: 0x01);\n        }",
    );
    let d = one(&source, RAW_RULE);
    assert_eq!(
        d.help.as_deref(),
        Some("use stack_obj.sprite_flags[temp[..]], play_sound(.., stack_obj, padN: 0xNN)")
    );
    // One statement without a spelling keeps the whole block.
    stays_raw(sim_tree, "scope(27, 0) := 1; scope(12, 7) := local[1];");
}

// --- review round 1 -------------------------------------------------------------------------

#[test]
fn a_template_member_names_no_builtin_call() {
    // An object in another file that extends the template may declare its own `reach`, which
    // the call would then mean. Cells name no call, so they are still spelled.
    let template = |raw: &str| {
        format!(
            "template Base {{\n    #[bhav(label = \"Probe\")] fn probe(a: i16) {{\n        raw {{ {raw} }}\n    }}\n}}\nobject \"Synth Probe\" from Base {{\n    guid: 0x7E00_0401;\n    price: 1;\n}}\n"
        )
    };
    for raw in [
        "reach(mode: 0, moves: false);",
        "clear_motive_changes() pad(2: 0x05);",
        "play_sound(0, stack_obj) pad(3: 0x01);",
        "remove_object(stack_obj);",
        "temp[0] := scope(33, 1);",
        "refresh(target: 0, mode: 1);",
    ] {
        none(&template(raw), RAW_RULE);
    }
    let d = one(&template("scope(27, 0) := 1;"), RAW_RULE);
    assert_eq!(
        d.help.as_deref(),
        Some("use stack_obj.sprite_flags[temp[..]]")
    );
}

#[test]
fn a_constant_named_like_the_spelling_keeps_the_raw_block() {
    // Parsing substitutes a block constant for its name, so `advert.delta[0]` would index the
    // constant.
    for decl in [
        "const advert = if a > 0 { 1 } else { 2 }; temp[0] = advert;",
        "const me = if a > 0 { 1 } else { 2 }; temp[0] = me;",
    ] {
        let source = sim_tree(&format!(
            "        {decl}\n        raw {{ scope(12, 0) := 1; scope(30, 0) := 1; }}"
        ));
        none(&source, RAW_RULE);
    }
}

#[test]
fn a_block_must_end_its_statement() {
    // `.air` reads adjacent raw blocks as one stream: this is one padded primitive.
    none(
        &sim_tree("        raw { clear_motive_changes() } raw { pad(3: 0x05); }"),
        RAW_RULE,
    );
    stays_raw(sim_tree, "clear_motive_changes()");
    stays_raw(sim_tree, "remove_object(stack_obj)");
    // Two tokens are not read as one.
    for raw in [
        "play_sound(1 2) pad(3: 0x01);",
        "temp[0] := scope(1 2, 0);",
        "scope(27, 0) := 1 2;",
    ] {
        stays_raw(sim_tree, raw);
    }
}

#[test]
fn pads_the_encoder_refuses_stay_raw() {
    for raw in [
        // Byte 3 of Clear Motive Changes already holds 1.
        "clear_motive_changes() pad(3: 1);",
        // Out of order.
        "play_sound(5, stack_obj) pad(7: 0xff, 6: 0xff);",
        // A byte that already holds the value.
        "play_sound(5) pad(3: 0x00);",
    ] {
        stays_raw(sim_tree, raw);
    }
    // Byte 3 set to another value is still a pad of Clear Motive Changes' own byte.
    same_as_raw(
        sim_tree,
        "clear_motive_changes() pad(5: 0x01);",
        "clear_motive_changes(padN: 0xNN)",
        "clear_motive_changes(pad5: 0x01);",
    );
}

// --- review round 2 -------------------------------------------------------------------------

#[test]
fn only_a_semicolon_ends_a_statement() {
    // The leading `;` ends the first block's call, so the second block is not a statement of
    // its own.
    none(
        &sim_tree("        raw { clear_motive_changes() } raw { ; clear_motive_changes(); }"),
        RAW_RULE,
    );
    // A line break ends nothing: these two lines are one statement.
    stays_raw(sim_tree, "clear_motive_changes()\n clear_motive_changes();");
    stays_raw(sim_tree, "clear_motive_changes();; clear_motive_changes();");
    // A sign apart from its digits is not a literal.
    stays_raw(sim_tree, "scope(31, 0) := - 2;");
    // Several statements on several lines still get the hint.
    let d = one(
        &sim_tree(
            "        raw {\n            scope(27, 0) := 1;\n            scope(27, 1) := 0;\n        }",
        ),
        RAW_RULE,
    );
    assert_eq!(
        d.help.as_deref(),
        Some("use stack_obj.sprite_flags[temp[..]]")
    );
}

#[test]
fn reach_defaults_its_mode_to_0() {
    same_as_raw(
        sim_tree,
        "reach(moves: true);",
        "reach(mode: .., moves: ..)",
        "reach(mode: 0, moves: true);",
    );
}

// --- review round 3 -------------------------------------------------------------------------

#[test]
fn literals_read_as_the_air_lexer_reads_them() {
    for raw in [
        "clear_motive_changes() pad(5: 0x+2);",
        "clear_motive_changes() pad(+5: 0x02);",
        "scope(12, +6) := 1;",
        "scope(12, 6) := 0x-1;",
        "refresh(target: 0, mode: 0x+1);",
    ] {
        stays_raw(sim_tree, raw);
    }
    same_as_raw(
        sim_tree,
        "clear_motive_changes() pad(5: 0x0_2);",
        "clear_motive_changes(padN: 0xNN)",
        "clear_motive_changes(pad5: 0x02);",
    );
}
