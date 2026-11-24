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

/// Tree 4300's nodes as `(opcode, operands, true exit, false exit)`, and its local count.
type Node = (u16, [u8; 8], u8, u8);

fn nodes(src: &str) -> (u16, Vec<Node>) {
    let cfg = PackResolverConfig::default();
    let tree = compile_v2_source("nodes.antics", src, &cfg)
        .unwrap_or_else(|e| panic!("must compile: {e}\n{src}"))
        .behaviors
        .remove(&4300)
        .expect("tree 4300");
    let nodes = tree
        .instructions
        .into_iter()
        .map(|n| (n.opcode, n.operands, n.on_true, n.on_false))
        .collect();
    (tree.locals, nodes)
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
    // The constant is a tree-level `.air` constant, which only operand positions resolve.
    assert!(body.contains("const done = 0;"), "{body}");
    assert!(body.contains("temp[1] := done;"), "{body}");
    assert_eq!(
        nodes(&obj(r#"    const part = 0;
    const done = 0;
    @id(4300) tree a {
        raw { set_to_next(part, dest: stack_obj); temp[1] := done; }
    }"#)),
        nodes(&obj(r#"    @id(4300) tree a {
        raw { set_to_next(part, dest: stack_obj); temp[1] := 0; }
    }"#)),
        "operands still resolve"
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

// ---------------------------------------------------------------------------
// Review round 2
// ---------------------------------------------------------------------------

/// A raw constant spelled like a branch target stays the target, even with a comment (holding a
/// `;`) between the arrow and the name: the `.air` parser resolves constants only as operands.
#[test]
fn raw_targets_stay_labels_past_comments() {
    let raw = r#"raw { temp[1] := 0; expr(temp[0] := 7) -> true: /* ; */ done, false: done;
            done: temp[1] := 17;
            temp[1] += 1; }"#;
    assert_eq!(
        nodes(&obj(&format!(
            "    const done = 3;\n    @id(4300) tree a {{ {raw} }}"
        ))),
        nodes(&obj(&format!("    @id(4300) tree a {{ {raw} }}"))),
    );
}

/// A helper running raw code may write any frame word, so a caller local passed by value is
/// copied before the helper runs.
#[test]
fn helpers_with_raw_code_get_copied_arguments() {
    let air = expand(
        "raw_alias.antics",
        &obj(r#"    fn f(a: i16) -> i16 {
        raw { __v_x_0 := 44; }
        return a;
    }
    tree a {
        let x = 17;
        temp[0] = f(x);
    }"#),
    );
    let body = tree(&air, "a");
    assert!(body.contains("let __arg_a_1 = __v_x_0;"), "{body}");
    assert!(body.contains("__ret_f_2 := __arg_a_1;"), "{body}");
}

/// Splitting an inline `if` out of an argument list keeps operand order: an earlier operand is
/// read before a selector that writes it (`inc_lt(&mut temp[0], ..)`).
#[test]
fn inline_operands_are_read_before_a_writing_selector() {
    let air = expand(
        "motive_order.antics",
        &obj(r#"    tree a {
        set_motive_change(hunger, temp[0], if inc_lt(&mut temp[0], 2) { 10 } else { 20 });
    }"#),
    );
    let body = tree(&air, "a");
    let ls = lines(body);
    let read = index_of(&ls, |l| l.ends_with("= temp[0];"), body);
    let select = index_of(&ls, |l| l.contains("inc_lt"), body);
    assert!(read < select, "{body}");
}

/// An `@slot` store is kept even when the tree itself never reads it again (another tree using
/// the same frame word may), and a slot must lie inside an explicit `@frame(locals: L)`.
#[test]
fn pinned_stores_survive_and_fit_the_frame() {
    let air = expand(
        "pin_reuse.antics",
        &obj(
            r#"    @id(4200) tree seed { @slot(10) let x = 17; temp[1] = x; }
    @id(4201) tree read { @slot(10) let x: i16; temp[0] = x; }"#,
        ),
    );
    assert!(tree(&air, "seed").contains("local[10] := 17;"), "{air}");
    expect_err(
        "pin_frame.antics",
        &obj("    @frame(locals: 3) tree a { @slot(3) let x: i16; temp[0] = x; }"),
        "gives the tree 3 local word(s)",
    );
}

/// Slot placement pins only the locals lowering pinned, and rewrites operands only: a raw
/// block's own `__slot..` name keeps its `.air` placement, and a bare branch target spelled like
/// a pinned local stays a target.
#[test]
fn slot_placement_follows_provenance_and_tokens() {
    let air = expand(
        "slot_target.antics",
        &obj(r#"    tree a {
        @slot(3) let a = 17;
        raw { random(temp[0], 2) -> true: __slot3_a_0, false: __slot3_a_0;
              __slot3_a_0: temp[0] := __slot3_a_0; }
    }"#),
    );
    let body = tree(&air, "a");
    assert!(
        body.contains("-> true: __slot3_a_0, false: __slot3_a_0;"),
        "{body}"
    );
    assert!(body.contains("__slot3_a_0: temp[0] := local[3];"), "{body}");
    let air = expand(
        "slot_raw.antics",
        &obj(r#"    tree a {
        @slot(3) let a = 17;
        raw { let __slot3_raw_99 = 44; temp[1] := __slot3_raw_99; }
        temp[0] = a;
    }"#),
    );
    let body = tree(&air, "a");
    assert!(body.contains("let __slot3_raw_99 = 44;"), "{body}");
    assert!(body.contains("temp[0] := local[3];"), "{body}");
}

/// `random` whose range resolves to a word pointer (global 34) draws into the range cell, so its
/// destination is not definitely written and the entry save stays.
#[test]
fn random_with_a_pointer_range_is_no_definite_write() {
    let air = expand(
        "entry_random.antics",
        &obj(r#"    tree a (sim: Sim) {
        let a: i16;
        stack_obj = sim.container;
        raw { random(__v_a_0, global[34]); temp[1] := __v_a_0; }
    }"#),
    );
    assert!(
        tree(&air, "a").contains("let __self_obj = stack_obj;"),
        "{air}"
    );
}

/// A `for ... else` body that reads a field through the item moves the cursor and back, so the
/// exhaustion block still sees the iterator's false result: no restore store before it.
#[test]
fn for_else_field_reads_leave_no_exhaustion_restore() {
    let air = expand(
        "cursor_foreign.antics",
        &obj(r#"    tree a (sim: Sim) {
        for item in lot.objects() {
            if item.attr[0] == 1 { temp[1] = 1; }
        } else { raw { gosub_found_action(); } }
    }"#),
    );
    let body = tree(&air, "a");
    let ls = lines(body);
    let found = index_of(&ls, |l| l == "gosub_found_action();", body);
    assert_eq!(ls[found - 1], "}", "{body}");
}

/// A bool helper with a `defer` is not branched on directly: its cleanup would run between the
/// condition and the caller's arm, which may read the last result.
#[test]
fn bool_helpers_with_defers_branch_on_a_saved_flag() {
    let air = expand(
        "bool_cleanup.antics",
        &obj(r#"    fn f() -> bool {
        defer { temp[1] = 99; }
        return false;
    }
    tree a (sim: Sim) {
        if f() {} else { gosub_found_action(); }
    }"#),
    );
    let body = tree(&air, "a");
    let ls = lines(body);
    let cleanup = index_of(&ls, |l| l == "temp[1] := 99;", body);
    let test = index_of(&ls, |l| l.starts_with("if __cond_"), body);
    assert!(cleanup < test, "{body}");
}

/// Const regions: a field read through a capitalized local is a variable read, a yielded name
/// passed to a call is a read (not a write), and raw blocks that declare locals or branch to
/// numbered nodes cannot be copied into each arm.
#[test]
fn const_regions_tell_reads_from_writes_and_reject_uncopyable_raw() {
    expect_err(
        "region_field.antics",
        &obj(r#"    tree a {
        let X = self.id;
        const C = if temp[0] == 0 { X.graphic } else { 0 };
        self.graphic = 42;
        temp[1] = C;
    }"#),
        "'X' is a variable",
    );
    let air = expand(
        "region_call.antics",
        &obj(r#"    const K = 7;
    fn f(a: i16) -> i16 { return a; }
    tree a {
        const C = if temp[0] == 0 { K } else { 9 };
        temp[1] = f(K);
        temp[2] = C;
    }"#),
    );
    assert_eq!(tree(&air, "a").matches("temp[1] := 7;").count(), 2, "{air}");
    for (name, raw, why) in [
        (
            "region_decl",
            "raw { let x = 4; temp[1] := x; }",
            "declares a local",
        ),
        (
            "region_branch",
            "raw { random(temp[2], 2) -> true: 2, false: 2; }",
            "numbered node",
        ),
    ] {
        expect_err(
            name,
            &obj(&format!(
                "    tree a {{
        const C = if temp[0] == 0 {{ 7 }} else {{ 9 }};
        {raw}
        temp[3] = C;
    }}"
            )),
            why,
        );
    }
}

/// A tree formats text when a raw block runs a text primitive in any spelling (`action_string
/// (..)` with a space, raw `op 50`) or an inlined helper's copy does; then a local declared in a
/// const region (one frame word per arm) is an error.
#[test]
fn text_formatting_is_found_by_parse_not_spelling() {
    for (name, extra) in [
        ("text_spaced", "raw { action_string (301, 1); }"),
        ("text_op", "raw { op 50 (45, 1, 0, 0, 1, 0, 0, 0); }"),
    ] {
        expect_err(
            name,
            &obj(&format!(
                "    tree a {{
        const C = if temp[0] == 0 {{ 7 }} else {{ 9 }};
        let a = C;
        temp[1] = a;
        {extra}
    }}"
            )),
            "formats text",
        );
    }
    expect_err(
        "text_inline.antics",
        &obj(r#"    fn f() -> i16 { let x = 5; return x; }
    tree a {
        const C = if temp[0] == 0 { 7 } else { 9 };
        temp[1] = f() + C;
        action_label(table: 301, index: 1);
    }"#),
        "'x' is declared in a const binding's region",
    );
}

/// Raw constants follow the tokens: a commented `let K` declares nothing, and an `.air` mode word
/// spelled like a constant keeps its meaning while the operand resolves.
#[test]
fn raw_consts_follow_tokens_not_text() {
    assert_eq!(
        nodes(&obj(r#"    const K = 7;
    @id(4300) tree a {
        raw { // let K is an explanatory comment
              temp[1] := K; }
    }"#)),
        nodes(&obj("    @id(4300) tree a { raw { temp[1] := 7; } }")),
    );
    assert_eq!(
        nodes(&obj(r#"    const part = 7;
    @id(4300) tree a { raw { set_to_next(part, dest: stack_obj); temp[0] := part; } }"#)),
        nodes(&obj(
            "    @id(4300) tree a { raw { set_to_next(part, dest: stack_obj); temp[0] := 7; } }"
        )),
    );
}

/// A raw loop nested in a `for ... else` exhaustion block, or a commented `continue`, does not
/// exit the `for`: only parsed loop exits outside a raw loop count.
#[test]
fn raw_nested_loops_do_not_exit_a_for_else() {
    for raw in [
        "raw { while true { temp[0] := 1; break; } }",
        "raw { temp[0] := 1; // continue\n }",
    ] {
        expand(
            "for_raw.antics",
            &obj(&format!(
                "    tree a {{
        for item in lot.sims() {{}} else {{ {raw} }}
    }}"
            )),
        );
    }
}
