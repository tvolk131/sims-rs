//! `.antics` v2.2 step 2b lint polish: the bitflags list methods never yield, and the
//! `wall_placement` / `world.expansions` names and flag methods (v2.2 step 2a, #482).
//!
//! Every fixture here is synthetic (GUIDs in the clean-room 0x7E00_xxxx range). See
//! `docs/decisions/20261126-antics-v2-2-step2-lint.md`.

use sims_antics::v2::PackResolverConfig;
use sims_antics::v2::lint::{LintDiagnostic, lint_v2_source};

/// A synthetic object with `uses` before it and `members` in it.
fn obj(uses: &str, members: &str) -> String {
    format!(
        "{uses}object \"Synth Probe\" {{\n    guid: 0x7E00_0740;\n    price: 1;\n    #[slot(2)] state level: i16;\n{members}\n}}\n"
    )
}

/// A tree running `body`, with `params`.
fn tree(uses: &str, params: &str, body: &str) -> String {
    obj(
        uses,
        &format!("    #[bhav(label = \"Probe\")] fn probe({params}) {{\n{body}\n    }}"),
    )
}

fn of(source: &str, rule: &str) -> Vec<LintDiagnostic> {
    lint_v2_source(source, "probe.antics")
        .unwrap_or_else(|e| panic!("lint failed: {e}\n{source}"))
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

/// The 1-based line and column of the first occurrence of `needle` in `source`.
fn position(source: &str, needle: &str) -> (usize, usize) {
    source
        .lines()
        .enumerate()
        .find_map(|(i, l)| l.find(needle).map(|c| (i + 1, c + 1)))
        .unwrap_or_else(|| panic!("{needle:?} not in source"))
}

/// Asserts that `source` has exactly one `rule` finding, at the start of the unique text
/// `original`, whose suggestion is `want`, and that writing it there builds the same bytes.
/// Returns the rewritten source.
fn rewrites(source: &str, rule: &str, original: &str, want: &str) -> String {
    let d = one(source, rule);
    assert_eq!(d.suggestion.as_deref(), Some(want), "{d:#?}");
    assert_eq!((d.line, d.column), position(source, original), "{d:#?}");
    assert_eq!(source.matches(original).count(), 1, "{original:?}");
    let fixed = source.replacen(original, want, 1);
    assert_eq!(
        iff(source),
        iff(&fixed),
        "rewrite builds other bytes:\n{fixed}"
    );
    fixed
}

const FLAGS: &str = "use std::flags::*;\n";

// --- unbounded_loop_without_yield: the bitflags methods never yield --------------------------

#[test]
fn a_loop_of_bitflags_methods_never_yields() {
    for stmt in [
        "self.flags.set(ObjectFlags::InUse);",
        "self.flags.clear(ObjectFlags::InUse);",
        "self.flags.set_all([ObjectFlags::InUse, ObjectFlags::Hidden]);",
        "self.flags.clear_all([ObjectFlags::InUse, ObjectFlags::Hidden]);",
        "if self.flags.contains(ObjectFlags::InUse) { self.level += 1; }",
        "if self.flags.intersects(ObjectFlags::InUse | ObjectFlags::Hidden) { self.level += 1; }",
        "if self.flags.contains_all([ObjectFlags::InUse, ObjectFlags::Hidden]) { self.level += 1; }",
        "if self.flags.contains_any([ObjectFlags::InUse, ObjectFlags::Hidden]) { self.level += 1; }",
        "if world.expansions.contains_any([Expansions::Pets, Expansions::Magic]) { self.level += 1; }",
        "self.wall_placement.set_all([WallPlacement::NoCorner, WallPlacement::NoWallLeft]);",
    ] {
        let source = tree(
            FLAGS,
            "",
            &format!("        loop {{\n            {stmt}\n        }}"),
        );
        // The loop body builds: the finding is about the loop, not a broken body.
        iff(&source);
        let d = one(&source, "unbounded_loop_without_yield");
        assert_eq!((d.line, d.column), position(&source, "loop {"), "{d:#?}");
    }
}

#[test]
fn the_same_loop_that_yields_or_exits_is_fine() {
    for tail in ["idle(1);", "sleep(1);", "break;"] {
        let source = tree(
            FLAGS,
            "",
            &format!(
                "        loop {{\n            self.flags.set_all([ObjectFlags::InUse, ObjectFlags::Hidden]);\n            {tail}\n        }}"
            ),
        );
        iff(&source);
        none(&source, "unbounded_loop_without_yield");
    }
}

#[test]
fn the_loop_help_names_calls_that_build() {
    let source = tree(
        FLAGS,
        "",
        "        loop {\n            self.flags.set(ObjectFlags::InUse);\n        }",
    );
    let d = one(&source, "unbounded_loop_without_yield");
    let help = d.help.as_deref().unwrap_or_default();
    assert!(help.contains("`idle(n);` / `sleep(n);`"), "{help}");
    // The calls it names build.
    for tail in ["idle(1);", "sleep(1);"] {
        iff(&tree(
            FLAGS,
            "",
            &format!("        loop {{\n            {tail}\n        }}"),
        ));
    }
}

// --- untyped_engine_field: `wall_placement` and `world.expansions` ---------------------------

#[test]
fn wall_placement_and_expansions_cells_take_their_names() {
    for (cell, want) in [
        ("self.data[13]", "self.wall_placement"),
        ("stack_obj.data[13]", "stack_obj.wall_placement"),
        ("global[20]", "world.expansions"),
    ] {
        let source = tree(FLAGS, "", &format!("        temp[0] = {cell};"));
        rewrites(&source, "untyped_engine_field", cell, want);
    }
}

#[test]
fn a_flag_operator_on_the_cell_is_named_then_takes_the_method() {
    // The cell is named first; the method form follows on the named receiver.
    for (stmt, cell, named, op, method) in [
        (
            "self.data[13] flag_set WallPlacement::NoCorner;",
            "self.data[13]",
            "self.wall_placement",
            "self.wall_placement flag_set WallPlacement::NoCorner",
            "self.wall_placement.set(WallPlacement::NoCorner)",
        ),
        (
            "if global[20] flag_test Expansions::Pets {\n            temp[0] = 1;\n        }",
            "global[20]",
            "world.expansions",
            "world.expansions flag_test Expansions::Pets",
            "world.expansions.contains(Expansions::Pets)",
        ),
    ] {
        let source = tree(FLAGS, "", &format!("        {stmt}"));
        none(&source, "prefer_method_syntax");
        let named_source = rewrites(&source, "untyped_engine_field", cell, named);
        rewrites(&named_source, "prefer_method_syntax", op, method);
    }
}

#[test]
fn a_cell_is_reported_only_as_text_the_lift_can_replace() {
    for body in [
        // The finding would start at `global` and run past the `)`.
        "        temp[0] = (global)[20];",
        "        temp[0] = (self.data)[13];",
        // A `]` or a number in a comment between the brackets.
        "        temp[0] = global[/* ] */20];",
        "        temp[0] = global[20 /* x */];",
        "        temp[0] = self.data[/* ] */13];",
        "        temp[0] = global[\n            20\n        ];",
        // The name would run into the word after the `]`.
        "        global[20]flag_set Expansions::Pets;",
        "        self.data[13]flag_set WallPlacement::NoCorner;",
    ] {
        let source = tree(FLAGS, "", body);
        // Every form builds: only the rewrite would not.
        iff(&source);
        none(&source, "untyped_engine_field");
    }
    // Blanks and parentheses are fine: the cell is still the whole text.
    for cell in ["global [ 20 ]", "global[(20)]", "self.data[ ( 13 ) ]"] {
        let want = if cell.starts_with("global") {
            "world.expansions"
        } else {
            "self.wall_placement"
        };
        let source = tree(FLAGS, "", &format!("        temp[0] = {cell};"));
        rewrites(&source, "untyped_engine_field", cell, want);
    }
}

#[test]
fn a_wide_string_before_a_flag_operator_has_no_method_finding() {
    // The lift places the fix by a byte column; after `"ééé"` the column is not one.
    let wide = "        dialog(kind: ok, title: \"ééé\", message: \"M\"); ";
    for stmt in [
        "self.wall_placement flag_set WallPlacement::NoCorner;",
        "world.expansions flag_set Expansions::Pets;",
        "temp[0] = global[20];",
    ] {
        let source = tree(FLAGS, "", &format!("{wide}{stmt}"));
        iff(&source);
        none(&source, "prefer_method_syntax");
        none(&source, "untyped_engine_field");
        // On a line of its own, it is reported.
        let source = tree(FLAGS, "", &format!("{}\n        {stmt}", wide.trim_end()));
        let rule = if stmt.starts_with("temp") {
            "untyped_engine_field"
        } else {
            "prefer_method_syntax"
        };
        one(&source, rule);
    }
}

// --- prefer_method_syntax: every receiver of the step 2a words -------------------------------

#[test]
fn flag_methods_point_at_the_receiver_for_every_root() {
    for (params, stmt, want) in [
        (
            "",
            "self.wall_placement flag_set WallPlacement::NoCorner",
            "self.wall_placement.set(WallPlacement::NoCorner)",
        ),
        (
            "",
            "stack_obj.wall_placement flag_clear WallPlacement::NoWallLeft",
            "stack_obj.wall_placement.clear(WallPlacement::NoWallLeft)",
        ),
        (
            "sim: Sim",
            "sim.wall_placement flag_set WallPlacement::NoCorner",
            "sim.wall_placement.set(WallPlacement::NoCorner)",
        ),
        (
            "sim: Sim",
            "me.wall_placement flag_clear WallPlacement::NoCorner",
            "me.wall_placement.clear(WallPlacement::NoCorner)",
        ),
        (
            "",
            "world.expansions flag_set Expansions::Pets",
            "world.expansions.set(Expansions::Pets)",
        ),
        (
            "sim: Sim",
            "world.expansions flag_clear Expansions::Magic",
            "world.expansions.clear(Expansions::Magic)",
        ),
    ] {
        let source = tree(FLAGS, params, &format!("        {stmt};"));
        rewrites(&source, "prefer_method_syntax", stmt, want);
    }
    // In a condition, the finding starts at `world` too.
    let source = tree(
        FLAGS,
        "",
        "        if !(world.expansions flag_test Expansions::Pets) {\n            temp[0] = 1;\n        }",
    );
    rewrites(
        &source,
        "prefer_method_syntax",
        "world.expansions flag_test Expansions::Pets",
        "world.expansions.contains(Expansions::Pets)",
    );
}

#[test]
fn a_shadowing_type_or_an_unknown_scope_has_no_flag_method() {
    // The object's own `Expansions` hides `std`'s.
    let source = obj(
        FLAGS,
        "    bitflags Expansions {\n        Mine = 6,\n    }\n    #[bhav(label = \"Probe\")] fn probe() {\n        world.expansions flag_set Expansions::Mine;\n    }",
    );
    none(&source, "prefer_method_syntax");
    // A file type `WallPlacement` hides it everywhere.
    let source = tree(
        "bitflags WallPlacement {\n    Mine = 5,\n}\n",
        "",
        "        self.wall_placement flag_set WallPlacement::Mine;",
    );
    none(&source, "prefer_method_syntax");
    // A template's body: its objects may be in other files.
    let source = format!(
        "{FLAGS}template Base {{\n    #[bhav(label = \"Probe\")] fn probe() {{\n        world.expansions flag_set Expansions::Pets;\n        self.wall_placement flag_set WallPlacement::NoCorner;\n    }}\n}}\n"
    );
    none(&source, "prefer_method_syntax");
    // `world . expansions` is other text than the rewrite's receiver.
    let source = tree(
        FLAGS,
        "",
        "        world . expansions flag_set Expansions::Pets;",
    );
    none(&source, "prefer_method_syntax");
}
