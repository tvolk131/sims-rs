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

// --- v2.3 step 2: `if let Some(v) = f()` in the binding and flow lints ---------------------
// See `docs/decisions/20261126-antics-v2-3-step2-lint.md`.

/// A tree running `body`, with `params`, beside three `Option` trees: `halve` (`i16`), and
/// `pick` and `pick_for(sim)` (`ObjectId`, in the object's and a Sim's context).
fn if_let_tree(params: &str, body: &str) -> String {
    obj(
        FLAGS,
        &format!(
            "    #[bhav] #[ret(temp = 2)] fn halve(x: i16) -> Option<i16> {{\n        if x < 2 {{ return None; }}\n        Some(x / 2)\n    }}\n    #[bhav] #[ret(temp = 2)] fn pick() -> Option<ObjectId> {{\n        if self.level < 2 {{ return None; }}\n        Some(stack_obj)\n    }}\n    #[bhav] #[ret(temp = 2)] fn pick_for(s: Sim) -> Option<ObjectId> {{\n        if self.level < 2 {{ return None; }}\n        Some(stack_obj)\n    }}\n    #[bhav(label = \"Probe\")] fn probe({params}) {{\n{body}\n    }}"
        ),
    )
}

#[test]
fn an_if_let_binding_nothing_reads_is_unused() {
    let source = if_let_tree(
        "",
        "        if let Some(k) = halve(self.level) { self.level = 1; }",
    );
    iff(&source);
    let d = one(&source, "unused_decl");
    assert_eq!(d.message, "local `k` is never read");
    assert_eq!((d.line, d.column), position(&source, "if let"), "{d:#?}");
    assert!(
        d.help
            .as_deref()
            .is_some_and(|h| h.contains("`_k`") && !h.contains("if f(..)")),
        "{d:#?}"
    );
    // Read in its `then` block, or named `_`: nothing to report.
    for body in [
        "        if let Some(k) = halve(self.level) { self.level = k; }",
        "        if let Some(_k) = halve(self.level) { self.level = 1; }",
    ] {
        let source = if_let_tree("", body);
        iff(&source);
        none(&source, "unused_decl");
    }
    // The help never names a word the block already uses: `_k` there would read the outer one.
    let source = if_let_tree(
        "",
        "        let _k = 7;\n        if let Some(k) = halve(self.level) { self.level = _k; }",
    );
    iff(&source);
    let d = one(&source, "unused_decl");
    assert_eq!(d.help.as_deref(), Some("name it `_k2`"), "{d:#?}");
    let renamed = source.replace("Some(k)", "Some(_k2)");
    none(&renamed, "unused_decl");
    assert_eq!(iff(&source), iff(&renamed));
    // A raw block reading the binding's frame word reads it.
    let source = if_let_tree(
        "",
        "        if let Some(k) = halve(self.level) {\n            raw {\n                my.attr[2] := __v_k_0;\n            }\n        }",
    );
    iff(&source);
    none(&source, "unused_decl");
    // An `else if let` binding is judged by its own `then` block.
    let source = if_let_tree(
        "",
        "        if let Some(k) = halve(self.level) { self.level = k; } else if let Some(j) = halve(3) { self.level = 2; }",
    );
    iff(&source);
    assert_eq!(
        one(&source, "unused_decl").message,
        "local `j` is never read"
    );
}

#[test]
fn an_if_let_binding_is_scoped_to_its_then_block() {
    // The outer `k` is never assigned: the `then` block's `k` is the `if let`'s, and the
    // read after the `if let` is the outer one's.
    let source = if_let_tree(
        "",
        "        let k: i16;\n        if let Some(k) = halve(self.level) { self.level = k; }\n        self.level = k;",
    );
    iff(&source);
    let d = one(&source, "uninitialized_local_read");
    let (line, _) = position(&source, "if let");
    assert_eq!((d.line, d.column), (line + 1, 22), "{d:#?}");
    // Its test and its `else` block read the outer one.
    for (body, needle) in [
        (
            "        let k: i16;\n        if let Some(k) = halve(k) { self.level = k; }\n        k = 1;\n        self.level = k;",
            "halve(k)",
        ),
        (
            "        let k: i16;\n        if let Some(k) = halve(self.level) { self.level = k; } else { self.level = k + 1; }\n        k = 1;\n        self.level = k;",
            "k + 1",
        ),
    ] {
        let source = if_let_tree("", body);
        iff(&source);
        let d = one(&source, "uninitialized_local_read");
        let (line, column) = position(&source, needle);
        let column = column + needle.find('k').unwrap();
        assert_eq!((d.line, d.column), (line, column), "{d:#?}");
    }
    // Assigned after the `if let`, before the read: nothing to report.
    let source = if_let_tree(
        "",
        "        let k: i16;\n        if let Some(k) = halve(self.level) { self.level = k; }\n        k = 2;\n        self.level = k;",
    );
    iff(&source);
    none(&source, "uninitialized_local_read");
}

#[test]
fn code_after_an_if_let_whose_branches_all_leave_is_unreachable() {
    let source = if_let_tree(
        "",
        "        loop {\n            if let Some(k) = halve(self.level) { self.level = k; break; } else { break; }\n            self.level = 1;\n        }",
    );
    let d = one(&source, "unreachable_code");
    assert_eq!(
        (d.line, d.column),
        position(&source, "self.level = 1;"),
        "{d:#?}"
    );
    let (line, _) = position(&source, "if let");
    assert_eq!(
        d.note.as_deref(),
        Some(
            format!("every branch of the `if let` on line {line} always leaves this block")
                .as_str()
        ),
    );
    // The build drops it, as the help says.
    let dropped = source.replace("\n            self.level = 1;", "");
    assert_eq!(iff(&source), iff(&dropped));
    // Without an `else`, or with one that goes on, control reaches it.
    for body in [
        "        loop {\n            if let Some(k) = halve(self.level) { self.level = k; break; }\n            self.level = 1;\n        }",
        "        loop {\n            if let Some(k) = halve(self.level) { self.level = k; break; } else { self.level = 2; }\n            self.level = 1;\n            sleep(1);\n        }",
    ] {
        let source = if_let_tree("", body);
        iff(&source);
        none(&source, "unreachable_code");
    }
}

#[test]
fn an_if_let_test_is_a_call_that_may_yield() {
    // The test runs a tree, which may yield: no `unbounded_loop_without_yield`.
    let source = if_let_tree(
        "",
        "        loop {\n            if let Some(k) = halve(self.level) { self.level = k; }\n        }",
    );
    iff(&source);
    none(&source, "unbounded_loop_without_yield");
}

#[test]
fn an_else_holding_only_an_if_let_collapses() {
    for (body, collapsed) in [
        (
            "        if self.level > 1 {\n            self.level = 1;\n        } else {\n            if let Some(k) = halve(self.level) {\n                self.level = k;\n            }\n        }",
            "        if self.level > 1 {\n            self.level = 1;\n        } else if let Some(k) = halve(self.level) {\n            self.level = k;\n        }",
        ),
        (
            "        if let Some(k) = halve(self.level) {\n            self.level = k;\n        } else {\n            if self.level > 0 {\n                self.level = 0;\n            }\n        }",
            "        if let Some(k) = halve(self.level) {\n            self.level = k;\n        } else if self.level > 0 {\n            self.level = 0;\n        }",
        ),
        (
            "        if let Some(k) = halve(self.level) {\n            self.level = k;\n        } else {\n            if let Some(j) = halve(4) {\n                self.level = j;\n            } else {\n                self.level = 0;\n            }\n        }",
            "        if let Some(k) = halve(self.level) {\n            self.level = k;\n        } else if let Some(j) = halve(4) {\n            self.level = j;\n        } else {\n            self.level = 0;\n        }",
        ),
    ] {
        let source = if_let_tree("", body);
        let d = one(&source, "collapsible_else_if");
        assert_eq!((d.line, d.column), position(&source, "else {"), "{d:#?}");
        let fixed = if_let_tree("", collapsed);
        none(&fixed, "collapsible_else_if");
        assert_eq!(iff(&source), iff(&fixed), "{source}\nvs\n{fixed}");
    }
}

#[test]
fn an_if_let_binding_hides_a_saved_self_handle() {
    // Inside the `then` block `saved` is the picked object, not the handle saved before
    // the swap: assigning it to `stack_obj` restores nothing.
    let source = if_let_tree(
        "sim: Sim",
        "        let saved = stack_obj;\n        stack_obj = sim;\n        if let Some(saved) = pick_for(sim) {\n            stack_obj = saved;\n            self.level = 1;\n        }",
    );
    iff(&source);
    let d = one(&source, "stack_obj_clobber_after_self");
    assert_eq!(
        (d.line, d.column),
        position(&source, "self.level = 1;"),
        "{d:#?}"
    );
    // Under another name the saved handle still restores it.
    let source = if_let_tree(
        "sim: Sim",
        "        let saved = stack_obj;\n        stack_obj = sim;\n        if let Some(other) = pick_for(sim) {\n            stack_obj = saved;\n            self.level = 1;\n            stack_obj = other;\n        }",
    );
    iff(&source);
    none(&source, "stack_obj_clobber_after_self");
    // (`if let Some(self)` no longer parses: `self` cannot be bound, v2.4.)
}

// --- v2.3 step 2: flag methods and names on `lot.objects()` loop handles -------------------

#[test]
fn a_lot_objects_handle_takes_flag_methods() {
    for (stmt, original, want) in [
        (
            "o.wall_placement flag_set WallPlacement::NoCorner;",
            "o.wall_placement flag_set WallPlacement::NoCorner",
            "o.wall_placement.set(WallPlacement::NoCorner)",
        ),
        (
            "o.flags flag_clear ObjectFlags::InUse;",
            "o.flags flag_clear ObjectFlags::InUse",
            "o.flags.clear(ObjectFlags::InUse)",
        ),
        (
            "if o.flags2 flag_test ObjectFlags2::CatchesFire { self.level = 1; }",
            "o.flags2 flag_test ObjectFlags2::CatchesFire",
            "o.flags2.contains(ObjectFlags2::CatchesFire)",
        ),
    ] {
        let source = tree(
            FLAGS,
            "",
            &format!("        for o in lot.objects() {{\n            {stmt}\n        }}"),
        );
        rewrites(&source, "prefer_method_syntax", original, want);
    }
}

#[test]
fn a_lot_objects_handle_takes_flag_names() {
    let source = tree(
        FLAGS,
        "",
        "        for o in lot.objects() {\n            o.wall_placement flag_set 1;\n        }",
    );
    let d = one(&source, "untyped_engine_constant");
    assert_eq!(
        d.suggestion.as_deref(),
        Some("WallPlacement::NeedsWallFront"),
        "{d:#?}"
    );
    let (line, column) = position(&source, "flag_set 1;");
    assert_eq!(
        (d.line, d.column),
        (line, column + "flag_set ".len()),
        "{d:#?}"
    );
    let fixed = source.replace("flag_set 1;", "flag_set WallPlacement::NeedsWallFront;");
    assert_eq!(iff(&source), iff(&fixed));
}

#[test]
fn other_handles_take_no_flag_method() {
    for body in [
        // A local of the handle's name hides it.
        "        for o in lot.objects() {\n            let o: ObjectId = stack_obj;\n            o.wall_placement flag_set WallPlacement::NoCorner;\n        }",
        // So does an `if let` binding.
        "        for o in lot.objects() {\n            if let Some(o) = pick() {\n                o.wall_placement flag_set WallPlacement::NoCorner;\n            }\n        }",
        // A plain object handle, which may be a typed object's.
        "        let o: ObjectId = stack_obj;\n        o.wall_placement flag_set WallPlacement::NoCorner;",
    ] {
        let source = if_let_tree("", body);
        iff(&source);
        none(&source, "prefer_method_syntax");
    }
    // After the loop the name is not the handle.
    let source = if_let_tree(
        "",
        "        let o: ObjectId = stack_obj;\n        for o in lot.objects() {\n            self.level += 1;\n        }\n        o.wall_placement flag_set WallPlacement::NoCorner;",
    );
    iff(&source);
    none(&source, "prefer_method_syntax");
    // A BCON table of the handle's name is what `o.wall_placement` reads.
    let source = format!(
        "extern bcon o {{\n    #[addr(16896)] wall_placement;\n}}\n{}",
        tree(
            FLAGS,
            "",
            "        for o in lot.objects() {\n            if o.wall_placement flag_test 1 { self.level = 1; }\n        }"
        )
    );
    none(&source, "prefer_method_syntax");
    none(&source, "untyped_engine_constant");
}

// --- v2.3 step 2: unread `extern bcon` entries -----------------------------------------------

fn unused_extern(source: &str) -> Vec<String> {
    of(source, "unused_decl")
        .into_iter()
        .map(|d| d.message)
        .filter(|m| m.contains("extern bcon"))
        .collect()
}

#[test]
fn an_extern_bcon_entry_nothing_reads_is_unused() {
    let source = format!(
        "extern bcon library_consts {{\n    #[addr(16896)] g260_0;\n    #[addr(16901)] g260_5;\n}}\n{}",
        tree("", "", "        self.level = library_consts::g260_5;")
    );
    iff(&source);
    let found = of(&source, "unused_decl");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(
        found[0].message,
        "`extern bcon` entry `g260_0` of `library_consts` is never read"
    );
    assert_eq!(
        (found[0].line, found[0].column),
        position(&source, "g260_0;"),
        "{:#?}",
        found[0]
    );
    // Removing it builds the same bytes.
    let removed = source.replace("    #[addr(16896)] g260_0;\n", "");
    assert_eq!(iff(&source), iff(&removed));
    // On the object too.
    let source = tree(
        "",
        "",
        "        self.level = library_consts.g260_5;\n    }\n    extern bcon library_consts {\n        #[addr(16896)] g260_0;\n        #[addr(16901)] g260_5;",
    );
    iff(&source);
    assert_eq!(
        unused_extern(&source),
        ["`extern bcon` entry `g260_0` of `library_consts` is never read"]
    );
}

#[test]
fn extern_bcon_entries_read_by_name_or_number_are_used() {
    let decl = "extern bcon library_consts {\n    #[addr(16896)] g260_0;\n    #[addr(16901)] g260_5;\n    #[addr(0x4202)] g260_2;\n    #[addr(16903)] _g260_7;\n}\n";
    for body in [
        // Paths and member reads.
        "        self.level = library_consts::g260_0 + library_consts.g260_5 + library_consts::g260_2;",
        // `.air` reads them by name, or by number at their address (the lift names every
        // entry it reads either way).
        "        raw {\n            my.attr[2] := library_consts.g260_0;\n            my.attr[2] := bcon[16901];\n            my.attr[2] := bcon[0x4202];\n        }",
        // A raw read at an index that is not a literal may read any of them.
        "        raw {\n            const SLOT = 16896;\n            my.attr[2] := bcon[SLOT];\n        }",
    ] {
        let source = format!("{decl}{}", tree("", "", body));
        iff(&source);
        assert!(unused_extern(&source).is_empty(), "{source}");
    }
    // A `pub` block may be read outside the file set.
    let source = format!(
        "pub {decl}{}",
        tree("", "", "        self.level = library_consts::g260_5;")
    );
    iff(&source);
    assert!(unused_extern(&source).is_empty(), "{source}");
}

#[test]
fn raw_bcon_reads_by_table_and_entry_or_scope_26_count() {
    // 16896 = table 132, entry 0; 16897 = table 132, entry 1.
    let decl =
        "extern bcon library_consts {\n    #[addr(16896)] g260_0;\n    #[addr(16897)] g260_1;\n}\n";
    for (read, unread) in [
        ("my.attr[2] := bcon[132:0];", "g260_1"),
        ("my.attr[2] := const[132,1];", "g260_0"),
        ("my.attr[2] := scope(26, 16897);", "g260_0"),
        (
            "const K = scope(26, 0x4200);\n            my.attr[2] := K;",
            "g260_1",
        ),
    ] {
        let source = format!(
            "{decl}{}",
            tree(
                "",
                "",
                &format!("        raw {{\n            {read}\n        }}")
            )
        );
        iff(&source);
        assert_eq!(
            unused_extern(&source),
            [format!(
                "`extern bcon` entry `{unread}` of `library_consts` is never read"
            )],
            "{source}"
        );
    }
}

#[test]
fn a_tree_label_in_quotes_reads_no_bcon() {
    let source = format!(
        "extern bcon library_consts {{\n    #[addr(16896)] g260_0;\n    #[addr(16897)] g260_1;\n}}\n{}",
        tree(
            "",
            "",
            "        raw {\n            my.attr[2] := bcon[132:0];\n            call \"bcon[132:1]\"();\n        }\n    }\n    #[bhav(label = \"bcon[132:1]\")] fn named() {"
        )
    );
    iff(&source);
    assert_eq!(
        unused_extern(&source),
        ["`extern bcon` entry `g260_1` of `library_consts` is never read"]
    );
}

#[test]
fn a_raw_bcon_index_named_by_a_constant_may_read_any_entry() {
    // `_16896` is a name, not the literal 16896; `_25` may be 26.
    let decl =
        "extern bcon library_consts {\n    #[addr(16896)] g260_0;\n    #[addr(16897)] g260_1;\n}\n";
    for read in [
        "const _16896 = 16897;\n            my.attr[2] := bcon[_16896];",
        "const _25 = 26;\n            my.attr[2] := scope(_25, 16897);",
    ] {
        let source = format!(
            "{decl}{}",
            tree(
                "",
                "",
                &format!("        raw {{\n            {read}\n        }}")
            )
        );
        iff(&source);
        assert!(unused_extern(&source).is_empty(), "{source}");
    }
}

#[test]
fn an_object_from_a_template_of_its_own_file_is_judged() {
    let source = format!(
        "template Base {{ }}\n{}",
        tree(
            "",
            "",
            "        self.level = 1;\n    }\n    extern bcon library_consts {\n        #[addr(16896)] g260_0;"
        )
        .replace("object \"Synth Probe\" {", "object \"Synth Probe\" from Base {")
    );
    iff(&source);
    assert_eq!(
        unused_extern(&source),
        ["`extern bcon` entry `g260_0` of `library_consts` is never read"]
    );
}

// --- v2.3 step 3: `light` (object data word 51) ----------------------------------------------

#[test]
fn data_word_51_is_named_light() {
    for cell in ["self", "stack_obj"] {
        let untyped = format!("{cell}.data[51]");
        let typed = format!("{cell}.light");
        let read = tree("", "", &format!("        temp[0] = {untyped};"));
        rewrites(&read, "untyped_engine_field", &untyped, &typed);
        let write = tree("", "", &format!("        {untyped} = 50;"));
        rewrites(&write, "untyped_engine_field", &untyped, &typed);
    }
    // A Sim handle names it too: it is an object word every object has.
    let source = tree("", "sim: Sim", "        temp[0] = sim.data[51];");
    rewrites(&source, "untyped_engine_field", "sim.data[51]", "sim.light");
}

#[test]
fn a_state_named_light_keeps_data_word_51_unnamed() {
    let source = tree("", "", "        self.data[51] = 50;").replace(
        "#[slot(2)] state level: i16;",
        "#[slot(2)] state level: i16;\n    #[slot(3)] state light: i16;",
    );
    assert!(source.contains("state light"), "{source}");
    iff(&source);
    none(&source, "untyped_engine_field");
}

#[test]
fn data_word_16_is_named_light_mode() {
    for cell in ["self", "stack_obj"] {
        let untyped = format!("{cell}.data[16]");
        let typed = format!("{cell}.light_mode");
        let read = tree("", "", &format!("        temp[0] = {untyped};"));
        rewrites(&read, "untyped_engine_field", &untyped, &typed);
        let write = tree("", "", &format!("        {untyped} = 1;"));
        rewrites(&write, "untyped_engine_field", &untyped, &typed);
    }
    // A Sim handle and a `lot.objects()` handle name it too: every object has the word.
    let source = tree("", "sim: Sim", "        temp[0] = sim.data[16];");
    rewrites(
        &source,
        "untyped_engine_field",
        "sim.data[16]",
        "sim.light_mode",
    );
    let source = tree(
        "",
        "",
        "        for o in lot.objects() {\n            if o.data[16] == 1 {\n                self.level += 1;\n            }\n        }",
    );
    rewrites(
        &source,
        "untyped_engine_field",
        "o.data[16]",
        "o.light_mode",
    );
}

#[test]
fn a_state_named_light_mode_keeps_data_word_16_unnamed() {
    let source = tree("", "", "        self.data[16] = 1;").replace(
        "#[slot(2)] state level: i16;",
        "#[slot(2)] state level: i16;\n    #[slot(3)] state light_mode: i16;",
    );
    assert!(source.contains("state light_mode"), "{source}");
    iff(&source);
    none(&source, "untyped_engine_field");
    // `stack_obj` has no states: its `light_mode` is the engine word whatever this object's
    // states are called.
    let source = tree("", "", "        temp[0] = stack_obj.data[16];").replace(
        "#[slot(2)] state level: i16;",
        "#[slot(2)] state level: i16;\n    #[slot(3)] state light_mode: i16;",
    );
    rewrites(
        &source,
        "untyped_engine_field",
        "stack_obj.data[16]",
        "stack_obj.light_mode",
    );
    // A typed handle names its object's states: `h.light_mode` would be the state. In code the
    // build folds away, the trial cannot tell, so the name check must.
    let source = tree(
        "",
        "h: Self",
        "        if false {\n            temp[0] = h.data[16];\n        }",
    )
    .replace(
        "#[slot(2)] state level: i16;",
        "#[slot(2)] state level: i16;\n    #[slot(3)] state light_mode: i16;",
    );
    iff(&source);
    none(&source, "untyped_engine_field");
}

// --- v2.4: `prefer_if_let` ----------------------------------------------------------------
// See `docs/decisions/20261126-antics-v2-4-runner-grid-and-if-let-lint.md`.

/// Asserts that `source` has one `prefer_if_let` finding, at `at`, suggesting `want`, and that
/// `fixed` (the rewrite written out) builds the same bytes and has no finding.
fn if_let_rewrite(source: &str, at: &str, want: &str, fixed: &str) {
    iff(source);
    let d = one(source, "prefer_if_let");
    assert_eq!(d.suggestion.as_deref(), Some(want), "{d:#?}");
    assert_eq!((d.line, d.column), position(source, at), "{d:#?}");
    assert_eq!(
        iff(source),
        iff(fixed),
        "rewrite builds other bytes:\n{fixed}"
    );
    none(fixed, "prefer_if_let");
}

#[test]
fn an_option_value_read_by_its_temp_takes_if_let() {
    // The `then` block reads the value `pick` leaves in `temp[2]`.
    let source = if_let_tree(
        "",
        "        if pick() {\n            stack_obj = temp[2];\n            self.level = 1;\n        }",
    );
    let fixed = if_let_tree(
        "",
        "        if let Some(value) = pick() {\n            stack_obj = value;\n            self.level = 1;\n        }",
    );
    if_let_rewrite(
        &source,
        "if pick()",
        "if let Some(value) = pick() {",
        &fixed,
    );
    let d = one(&source, "prefer_if_let");
    assert_eq!(
        d.message,
        "`pick` returns `Option`, and this code reads its value as `temp[2]`"
    );
    // An `if !f() { return ..; }` reads it in the rest of the block: `let .. else`.
    let source = if_let_tree(
        "",
        "        if !pick() {\n            return;\n        }\n        stack_obj = temp[2];\n        self.level = 1;",
    );
    let fixed = if_let_tree(
        "",
        "        let Some(value) = pick() else {\n            return;\n        };\n        stack_obj = value;\n        self.level = 1;",
    );
    if_let_rewrite(
        &source,
        "if !pick()",
        "let Some(value) = pick() else { .. };",
        &fixed,
    );
}

#[test]
fn a_std_globals_option_names_its_value_after_the_callee() {
    let source = tree(
        "use std::globals::*;\n",
        "",
        "        if globals::find_companion() {\n            stack_obj = temp[0];\n        }",
    );
    let fixed = tree(
        "use std::globals::*;\n",
        "",
        "        if let Some(companion) = globals::find_companion() {\n            stack_obj = companion;\n        }",
    );
    if_let_rewrite(
        &source,
        "if globals::",
        "if let Some(companion) = globals::find_companion() {",
        &fixed,
    );
    // A name the block already uses is not taken.
    let source = tree(
        "use std::globals::*;\n",
        "",
        "        let companion = 3;\n        if globals::find_companion() {\n            stack_obj = temp[0];\n            self.level = companion;\n        }",
    );
    let d = one(&source, "prefer_if_let");
    assert_eq!(
        d.suggestion.as_deref(),
        Some("if let Some(companion_2) = globals::find_companion() {"),
        "{d:#?}"
    );
}

#[test]
fn prefer_if_let_needs_a_read_the_binding_builds_the_same() {
    for body in [
        // Nothing reads the value.
        "        if pick() {\n            self.level = 1;\n        }",
        // Another temp.
        "        if pick() {\n            stack_obj = temp[1];\n        }",
        // A call may change the temp before the read.
        "        if pick() {\n            self.level = 1;\n            halve(3);\n            stack_obj = temp[2];\n        }",
        // The `if !` block does not always leave.
        "        if !pick() {\n            self.level = 1;\n        }\n        stack_obj = temp[2];",
        // `if !` with an `else`.
        "        if !pick() {\n            return;\n        } else {\n            self.level = 1;\n        }\n        stack_obj = temp[2];",
        // The binding takes a frame word the temp read did not: other bytes.
        "        if halve(self.level) {\n            self.level += temp[2];\n            self.level += temp[2];\n        }",
    ] {
        let source = if_let_tree("", body);
        iff(&source);
        none(&source, "prefer_if_let");
    }
    // A `-> bool` tree leaves no value.
    let source = obj(
        "",
        "    #[bhav] fn test_it() -> bool {\n        self.level > 2\n    }\n    #[bhav(label = \"Probe\")] fn probe() {\n        if test_it() {\n            stack_obj = temp[2];\n        }\n    }",
    );
    iff(&source);
    none(&source, "prefer_if_let");
}

#[test]
fn prefer_if_let_names_the_reads_it_renames() {
    // A write to the temp ends the renamed reads: the help says which read takes the name.
    let source = if_let_tree(
        "",
        "        if !pick() {\n            return;\n        }\n        self.level = temp[2];\n        temp[2] = 3;\n        self.level = temp[2];",
    );
    let d = one(&source, "prefer_if_let");
    // The first read, on the first `self.level = temp[2];` line.
    let (line, column) = position(&source, "self.level = temp[2];");
    let column = column + "self.level = ".len();
    assert_eq!(
        d.help.as_deref(),
        Some(
            format!(
                "write `let Some(value) = pick() else {{ .. }};`; `value` is the value for the rest of the block; rename `temp[2]` to `value` at {line}:{column} only (a later `temp[2]` may hold another value)"
            )
            .as_str()
        ),
        "{d:#?}"
    );
    let fixed = if_let_tree(
        "",
        "        let Some(value) = pick() else {\n            return;\n        };\n        self.level = value;\n        temp[2] = 3;\n        self.level = temp[2];",
    );
    assert_eq!(iff(&source), iff(&fixed));
    none(&fixed, "prefer_if_let");
}

#[test]
fn prefer_if_let_skips_names_the_region_declares_and_keeps_comments() {
    // An inner `let value` would hide the binding: the next free name is taken.
    let source = if_let_tree(
        "",
        "        if pick() {\n            stack_obj = temp[2];\n            let value = 3;\n            self.level = value;\n        }",
    );
    let fixed = if_let_tree(
        "",
        "        if let Some(value_2) = pick() {\n            stack_obj = value_2;\n            let value = 3;\n            self.level = value;\n        }",
    );
    if_let_rewrite(
        &source,
        "if pick()",
        "if let Some(value_2) = pick() {",
        &fixed,
    );
    // A comment in the leaving block stays where it is, a nested one holding a brace too.
    let source = if_let_tree(
        "",
        "        if !pick() {\n            /* outer /* inner */ { */\n            return;\n        }\n        stack_obj = temp[2];",
    );
    let fixed = if_let_tree(
        "",
        "        let Some(value) = pick() else {\n            /* outer /* inner */ { */\n            return;\n        };\n        stack_obj = value;",
    );
    if_let_rewrite(
        &source,
        "if !pick()",
        "let Some(value) = pick() else { .. };",
        &fixed,
    );
    let source = if_let_tree(
        "",
        "        if !pick() {\n            // nothing to pick\n            return;\n        }\n        stack_obj = temp[2];",
    );
    let fixed = if_let_tree(
        "",
        "        let Some(value) = pick() else {\n            // nothing to pick\n            return;\n        };\n        stack_obj = value;",
    );
    if_let_rewrite(
        &source,
        "if !pick()",
        "let Some(value) = pick() else { .. };",
        &fixed,
    );
}
