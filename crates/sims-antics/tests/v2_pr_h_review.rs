//! `.antics` v2 PR H: regressions found by review. Each test pins one soundness rule. All sources
//! are synthetic.

use sims_antics::{PackResolverConfig, compile_v2_source};

fn expand(name: &str, src: &str) -> String {
    let cfg = PackResolverConfig::default();
    compile_v2_source(name, src, &cfg).unwrap_or_else(|e| panic!("{name} must compile: {e}"));
    sims_antics::v2::expand_v2_source_to_objects(name, src, &cfg)
        .unwrap_or_else(|e| panic!("{name} must expand: {e}"))
        .into_iter()
        .next()
        .expect("one object")
        .air_source
}

fn expect_err(name: &str, src: &str, needle: &str) {
    let cfg = PackResolverConfig::default();
    let err = compile_v2_source(name, src, &cfg).expect_err(&format!("{name} must fail"));
    assert!(
        err.message.contains(needle),
        "{name}: expected '{needle}' in: {err}"
    );
}

/// A synthetic object around `members`.
fn obj(members: &str) -> String {
    format!(
        r#"object "Review" {{
    guid: 0x7F40_0800;
    state value: i16;
    strings 301 "Notes" {{ title = "T", msg = "Was $Local:0" }}
{members}
}}
"#
    )
}

/// The `.air` body of tree `name` (from its header to the next tree).
fn tree<'a>(air: &'a str, name: &str) -> &'a str {
    let start = [format!("\"{name}\" {{"), format!("\"{name}\" (")]
        .iter()
        .find_map(|h| air.find(h.as_str()))
        .unwrap_or_else(|| panic!("tree {name} in:\n{air}"));
    let rest = &air[start..];
    let end = rest[1..].find("\ntree ").map_or(rest.len(), |i| i + 1);
    &rest[..end]
}

/// The trimmed, non-empty lines of `body`.
fn lines(body: &str) -> Vec<&str> {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect()
}

fn index_of(lines: &[&str], pred: impl Fn(&str) -> bool, body: &str) -> usize {
    lines
        .iter()
        .position(|l| pred(l))
        .unwrap_or_else(|| panic!("line not found in:\n{body}"))
}

/// A loop whose condition reads an indexed `self` cell after the cursor moved re-reads it at the
/// head of every iteration; it is not hoisted into a value computed once.
#[test]
fn cursor_loops_reread_indexed_self_cells() {
    let air = expand(
        "cursor_head.antics",
        &obj(r#"    tree scan (sim: Sim) {
        while self.attr[0] != 0 {
            self.attr[0] = 0;
            set_to_next(all, dest: stack_obj);
        }
    }"#),
    );
    let body = tree(&air, "scan");
    let ls = lines(body);
    let head = index_of(&ls, |l| l == "while true {", body);
    let read = index_of(&ls, |l| l.ends_with("= stack.attr[0];"), body);
    assert!(
        read > head,
        "the condition is read inside the loop:\n{body}"
    );
    assert!(
        !ls.iter().any(|l| l.starts_with("while __")),
        "no hoisted condition:\n{body}"
    );
}

/// A `return` in a `for … else` exhaustion block of an inline helper leaves the helper: the code
/// after the loop does not run.
#[test]
fn inline_returns_from_for_else_leave_the_helper() {
    let air = expand(
        "for_else_return.antics",
        &obj(r#"    fn f() -> i16 {
        for o in lot.objects() { if temp[1] == 1 { break; } } else { return 9; }
        temp[2] = 44;
        return 8;
    }
    fn g() -> bool {
        for o in lot.objects() { if temp[1] == 1 { break; } } else { return true; }
        temp[2] = 44;
        return false;
    }
    tree a {
        temp[0] = f();
    }
    tree b {
        if g() { temp[3] = 1; } else { temp[3] = 2; }
    }"#),
    );
    for name in ["a", "b"] {
        let body = tree(&air, name);
        let ls = lines(body);
        let store = index_of(&ls, |l| l == "temp[2] := 44;", body);
        assert!(
            ls[store - 1] == "}" && ls[store - 2] == "break;" && ls[store - 3].starts_with("if __"),
            "{name}: the exhaustion return is tested before the rest of the helper:\n{body}"
        );
    }
}

/// A raw branch whose numeric targets are on the lines after its `->` keeps the entry save, since
/// dropping it would shift node numbers.
#[test]
fn multiline_numeric_targets_keep_the_entry_save() {
    let air = expand(
        "multiline_targets.antics",
        &obj(r#"    tree a (sim: Sim) {
        stack_obj = sim.container;
        raw {
            random(temp[0], 2) ->
                true: 4, false: 4;
            temp[1] := 11;
            temp[1] := 22;
            temp[1] += 1;
        }
    }"#),
    );
    assert!(
        tree(&air, "a").contains("let __self_obj = stack_obj;"),
        "{air}"
    );
}

/// A pinned variable shadowed in its scope is still live while a `defer` captured it, so another
/// variable cannot take its slot.
#[test]
fn shadowed_slots_captured_by_a_defer_stay_occupied() {
    expect_err(
        "slot_defer.antics",
        &obj(r#"    tree a {
        @slot(0) let x = 1;
        defer { temp[0] = x; }
        let x = 2;
        @slot(0) let z = 3;
        raw { temp[2] := local[0]; }
    }"#),
        "already holds 'x'",
    );
}

/// A helper stored into a pinned word stages its result: the word is not written before the
/// helper's cleanup, which can read it as `$Local:N`. (A fault ends the frame, so a partial store
/// before one is unobservable, as in ordinary code.)
#[test]
fn inline_helpers_do_not_publish_into_pinned_words_early() {
    let air = expand(
        "slot_inline.antics",
        &obj(r#"    fn f() -> i16 {
        defer { dialog(kind: ok, title: title, message: msg); }
        return 9;
    }
    tree a {
        raw { local[0] := 7; }
        @slot(0) let x = f();
        temp[0] = x;
    }
"#),
    );
    let a = tree(&air, "a");
    assert!(!a.contains("local[0] := 9;"), "staged result:\n{a}");
    let dialog = a.find("dialog(").expect("dialog");
    let commit = a.rfind("local[0] := local[").expect("commit into the slot");
    assert!(commit > dialog, "committed after the cleanup:\n{a}");
}

/// A pinned word passed by value is copied, so a helper that writes that word by number still
/// returns the argument's value.
#[test]
fn pinned_arguments_are_copied() {
    let air = expand(
        "slot_arg.antics",
        &obj(r#"    fn f(a: i16) -> i16 {
        raw { local[3] := 44; }
        return a;
    }
    tree a {
        @slot(3) let x = 17;
        temp[0] = f(x);
    }"#),
    );
    let body = tree(&air, "a");
    let ls = lines(body);
    let write = index_of(&ls, |l| l == "local[3] := 44;", body);
    let copy = index_of(&ls, |l| l.ends_with(":= local[3];"), body);
    assert!(copy < write, "the argument is copied first:\n{body}");
    assert!(
        !ls[write + 1..].iter().any(|l| l.contains(":= local[3]")),
        "nothing reads the word after the helper wrote it:\n{body}"
    );
}

/// The entry save stays when it is needed or observable: a relationship read with flag 1 writes
/// its local only on the true exit; a raw label branch can skip a `let`; in a tree whose raw code
/// keeps the `.air` layout, a dialog can name the save's word as `$Local:0`.
#[test]
fn entry_save_stays_when_locals_can_be_unwritten_or_observed() {
    let air = expand(
        "entry_save.antics",
        &obj(r#"    tree a (sim: Sim) {
        let a: i16;
        stack_obj = sim.container;
        relationship(column: 0, dir: 0, flags: 1, value: a);
        raw { temp[0] := __v_a_0; }
    }
    tree b (sim: Sim) {
        let a = 7;
        stack_obj = sim.container;
        raw {
            random(temp[0], 2) -> true: @skip, false: @go;
            @go: temp[3] := 1;
        }
        let b = 8;
        raw { @skip: temp[1] := __v_b_1; }
    }
    tree c (sim: Sim) {
        let a = 1;
        stack_obj = sim.container;
        raw { temp[0] := __v_a_0; }
        dialog(kind: ok, title: title, message: msg);
    }"#),
    );
    for name in ["a", "b", "c"] {
        assert!(
            tree(&air, name).contains("let __self_obj = stack_obj;"),
            "{name} keeps the save:\n{air}"
        );
    }
}

/// `for … else` with a local cursor and a body that leaves the stack object alone does not
/// restore it before the exhaustion block, which may read the iterator's false result.
#[test]
fn for_else_keeps_the_last_result_for_its_exhaustion_block() {
    let air = expand(
        "for_else_result.antics",
        &obj(r#"    tree scan (sim: Sim) {
        for item in lot.objects() {} else { raw { gosub_found_action(); } }
    }"#),
    );
    let body = tree(&air, "scan");
    assert!(!body.contains("stack_obj :="), "no restore:\n{body}");
    let ls = lines(body);
    let found = index_of(&ls, |l| l == "gosub_found_action();", body);
    assert_eq!(ls[found - 1], "}", "directly after the iterator:\n{body}");
}

/// A const binding yields compile-time operands only: no variables, no `stack_obj`, and its names
/// cannot be assigned or borrowed.
#[test]
fn const_bindings_yield_only_stable_operands() {
    expect_err(
        "yield_var.antics",
        &obj(r#"    tree a {
        let y = temp[0];
        const X = if temp[1] == 1 { y } else { y };
        temp[2] = X;
    }"#),
        "'y' is a variable",
    );
    expect_err(
        "yield_stack_obj.antics",
        &obj(r#"    tree a {
        const X = if temp[1] == 1 { stack_obj } else { 0 };
        temp[2] = X;
    }"#),
        "cannot yield 'stack_obj'",
    );
    expect_err(
        "write_through.antics",
        &obj(r#"    tree a {
        const X = if temp[1] == 1 { 1 } else { 2 };
        X = 9;
        temp[2] = X;
    }"#),
        "'X' is bound by a const binding",
    );
}

/// A const binding's region is copied into each arm, so it cannot define a raw label, and in a
/// tree that formats text it cannot declare a variable (each copy would take its own word).
#[test]
fn const_regions_reject_layout_sensitive_code() {
    expect_err(
        "region_label.antics",
        &obj(r#"    tree a {
        const X = if temp[1] == 1 { 1 } else { 2 };
        raw { @done: temp[1] := 9; }
        temp[0] = X;
    }"#),
        "defines a label",
    );
    expect_err(
        "region_text.antics",
        &obj(r#"    tree a {
        const X = if temp[1] == 1 { 10 } else { 20 };
        let y = X;
        temp[2] = y;
        dialog(kind: ok, title: title, message: msg);
    }"#),
        "'y' is declared in a const binding's region",
    );
    // Without text the copies are independent locals, as before.
    expand(
        "region_plain.antics",
        &obj(r#"    tree a {
        const X = if temp[1] == 1 { 10 } else { 20 };
        let y = X;
        temp[2] = y;
    }"#),
    );
}

/// Raw const substitution leaves `.air` words (such as the `part` mode) and branch targets on a
/// later line alone, while the same name in an operand position is its value.
#[test]
fn raw_consts_leave_air_words_and_targets_alone() {
    let air = expand(
        "raw_words.antics",
        &obj(r#"    const part = 0;
    const done = 0;
    tree a {
        raw {
            set_to_next(part, dest: stack_obj);
            random(temp[0], 2) ->
                true: done, false: done;
            temp[1] := done;
            done: temp[2] := 1;
        }
    }"#),
    );
    let body = tree(&air, "a");
    assert!(
        body.contains("set_to_next(part, dest: stack_obj);"),
        "{body}"
    );
    assert!(body.contains("true: done, false: done;"), "{body}");
    assert!(
        body.contains("temp[1] := 0;"),
        "operands still resolve:\n{body}"
    );
}

/// Slot rewriting replaces local operands only: a raw label spelled like the generated name stays a
/// label.
#[test]
fn slot_rewriting_leaves_labels_alone() {
    let air = expand(
        "slot_label.antics",
        &obj(r#"    tree a {
        @slot(3) let a = 17;
        raw { @__slot3_a_0: temp[0] := __slot3_a_0; }
    }"#),
    );
    let body = tree(&air, "a");
    assert!(
        body.contains("@__slot3_a_0: temp[0] := local[3];"),
        "{body}"
    );
}
