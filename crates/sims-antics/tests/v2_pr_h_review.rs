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
        &obj(r#"    #[bhav]
    fn scan(sim: Sim) {
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
    #[bhav]
    fn a() {
        temp[0] = f();
    }
    #[bhav]
    fn b() {
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
        &obj(r#"    #[bhav]
    fn a(sim: Sim) {
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
        &obj(r#"    #[bhav]
    fn a() {
        #[slot(0)] let x = 1;
        defer { temp[0] = x; }
        let x = 2;
        #[slot(0)] let z = 3;
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
    #[bhav]
    fn a() {
        raw { local[0] := 7; }
        #[slot(0)] let x = f();
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
    #[bhav]
    fn a() {
        #[slot(3)] let x = 17;
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
        &obj(r#"    #[bhav]
    fn a(sim: Sim) {
        let a: i16;
        stack_obj = sim.container;
        relationship(column: 0, dir: 0, flags: 1, value: a);
        raw { temp[0] := __v_a_0; }
    }
    #[bhav]
    fn b(sim: Sim) {
        let a = 7;
        stack_obj = sim.container;
        raw {
            random(temp[0], 2) -> true: @skip, false: @go;
            @go: temp[3] := 1;
        }
        let b = 8;
        raw { @skip: temp[1] := __v_b_1; }
    }
    #[bhav]
    fn c(sim: Sim) {
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
        &obj(r#"    #[bhav]
    fn scan(sim: Sim) {
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
        &obj(r#"    #[bhav]
    fn a() {
        let y = temp[0];
        const X = if temp[1] == 1 { y } else { y };
        temp[2] = X;
    }"#),
        "'y' is a variable",
    );
    expect_err(
        "yield_stack_obj.antics",
        &obj(r#"    #[bhav]
    fn a() {
        const X = if temp[1] == 1 { stack_obj } else { 0 };
        temp[2] = X;
    }"#),
        "cannot yield 'stack_obj'",
    );
    expect_err(
        "write_through.antics",
        &obj(r#"    #[bhav]
    fn a() {
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
        &obj(r#"    #[bhav]
    fn a() {
        const X = if temp[1] == 1 { 1 } else { 2 };
        raw { @done: temp[1] := 9; }
        temp[0] = X;
    }"#),
        "defines a label",
    );
    expect_err(
        "region_text.antics",
        &obj(r#"    #[bhav]
    fn a() {
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
        &obj(r#"    #[bhav]
    fn a() {
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
    #[bhav]
    fn a() {
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
    #[bhav(id = 4300)]
    fn a() {
        raw { set_to_next(part, dest: stack_obj); temp[1] := done; }
    }"#)),
        nodes(&obj(r#"    #[bhav(id = 4300)]
    fn a() {
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
        &obj(r#"    #[bhav]
    fn a() {
        #[slot(3)] let a = 17;
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
            "    const done = 3;\n    #[bhav(id = 4300)] fn a() {{ {raw} }}"
        ))),
        nodes(&obj(&format!("    #[bhav(id = 4300)] fn a() {{ {raw} }}"))),
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
    #[bhav]
    fn a() {
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
        &obj(r#"    #[bhav]
    fn a() {
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
            r#"    #[bhav(id = 4200)]
    fn seed() { #[slot(10)] let x = 17; temp[1] = x; }
    #[bhav(id = 4201)]
    fn read() { #[slot(10)] let x: i16; temp[0] = x; }"#,
        ),
    );
    assert!(tree(&air, "seed").contains("local[10] := 17;"), "{air}");
    expect_err(
        "pin_frame.antics",
        &obj("    #[bhav(locals = 3)] fn a() { #[slot(3)] let x: i16; temp[0] = x; }"),
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
        &obj(r#"    #[bhav]
    fn a() {
        #[slot(3)] let a = 17;
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
        &obj(r#"    #[bhav]
    fn a() {
        #[slot(3)] let a = 17;
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
        &obj(r#"    #[bhav]
    fn a(sim: Sim) {
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
        &obj(r#"    #[bhav]
    fn a(sim: Sim) {
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
    #[bhav]
    fn a(sim: Sim) {
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
        &obj(r#"    #[bhav]
    fn a() {
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
    #[bhav]
    fn a() {
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
                "    #[bhav]
    fn a() {{
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
                "    #[bhav]
    fn a() {{
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
    #[bhav]
    fn a() {
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
    #[bhav(id = 4300)]
    fn a() {
        raw { // let K is an explanatory comment
              temp[1] := K; }
    }"#)),
        nodes(&obj("    #[bhav(id = 4300)] fn a() { raw { temp[1] := 7; } }")),
    );
    assert_eq!(
        nodes(&obj(r#"    const part = 7;
    #[bhav(id = 4300)]
    fn a() { raw { set_to_next(part, dest: stack_obj); temp[0] := part; } }"#)),
        nodes(&obj(
            "    #[bhav(id = 4300)] fn a() { raw { set_to_next(part, dest: stack_obj); temp[0] := 7; } }"
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
                "    #[bhav]
    fn a() {{
        for item in lot.sims() {{}} else {{ {raw} }}
    }}"
            )),
        );
    }
}

// ---------------------------------------------------------------------------
// Review round 3
// ---------------------------------------------------------------------------

/// Raw constants resolve wherever a raw block lands: twice in one inlined helper, past a `local K`
/// in discarded code, and in a `local:` operand. A constant named like an `.air` keyword (`temp`)
/// never shadows it.
#[test]
fn raw_consts_survive_inlining_discarded_code_and_keywords() {
    let literal = nodes(&obj(
        "    #[bhav(id = 4300)] fn a() { raw { temp[0] := 7; temp[1] := 7; } }",
    ));
    assert_eq!(
        nodes(&obj(r#"    const K = 7;
    fn f() { raw { temp[0] := K; temp[1] := K; } }
    #[bhav(id = 4300)]
    fn a() { f(); }"#)),
        literal,
    );
    assert_eq!(
        nodes(&obj(r#"    const K = 7;
    #[bhav(id = 4300)]
    fn a() {
        if false { raw { local K; } }
        raw { temp[0] := K; temp[1] := K; }
    }"#)),
        literal,
    );
    assert_eq!(
        nodes(&obj(r#"    const temp = 17;
    #[bhav(id = 4300)]
    fn a() { raw { temp[0] := 7; temp[1] := 7; } }"#)),
        literal,
    );
    assert_eq!(
        nodes(&obj(r#"    const L = 1;
    #[bhav(id = 4300)]
    fn a() { raw { animate(clip: 1, table: 0, events: 0, local: L); } }"#)),
        nodes(&obj(
            "    #[bhav(id = 4300)] fn a() { raw { animate(clip: 1, table: 0, events: 0, local: 1); } }"
        )),
    );
}

/// Only live `@slot` lets pin a word: a slot in discarded code pins nothing, and a raw block's own
/// local spelled like a slot name keeps its `.air` placement. A call target spelled like a pinned
/// local stays a call target.
#[test]
fn slot_pins_come_from_live_code_and_leave_call_targets_alone() {
    let air = expand(
        "slot_dead.antics",
        &obj(r#"    #[bhav]
    fn a() {
        if false { #[slot(7)] let x: i16; }
        #[slot(3)] let q = 1;
        raw { local __slot7_x_0; __slot7_x_0 := 9; temp[0] := local[7]; }
    }"#),
    );
    let body = tree(&air, "a");
    assert!(body.contains("__slot7_x_0 := 9;"), "{body}");
    assert!(!body.contains("local[7] := 9;"), "{body}");
    let air = expand(
        "slot_call.antics",
        &obj(r#"    #[bhav]
    fn __slot3_x_0() { return true; }
    #[bhav]
    fn a() {
        #[slot(3)] let x = 7;
        raw { call __slot3_x_0() -> true: true, false: false; }
    }"#),
    );
    let body = tree(&air, "a");
    assert!(body.contains("local[3] := 7;"), "{body}");
    assert!(body.contains("call __slot3_x_0()"), "{body}");
}

/// A raw `for ... else` body that names `stack_obj` as any primitive's target (an unspaced `:=`,
/// `sqrt`, a test like `inc_lt`) gets the exhaustion restore. A subroutine call does not: the VM
/// gives each frame its own stack object.
#[test]
fn raw_stack_obj_targets_restore_but_calls_do_not() {
    for raw in [
        "stack_obj:=9;",
        "stack_obj sqrt 9;",
        "if stack_obj inc_lt 9 { temp[0] := 1; }",
    ] {
        let air = expand(
            "cursor_raw.antics",
            &obj(&format!(
                "    #[bhav]
    fn a(sim: Sim) {{
        for item in lot.objects() {{ raw {{ {raw} }} }} else {{ raw {{ temp[1] := stack_obj; }} }}
    }}"
            )),
        );
        let body = tree(&air, "a");
        let ls = lines(body);
        let exhausted = index_of(&ls, |l| l == "temp[1] := stack_obj;", body);
        assert_eq!(ls[exhausted - 1], "stack_obj := __self_obj;", "{body}");
    }
    let air = expand(
        "cursor_call.antics",
        &obj(r#"    #[bhav(id = 4300)]
    fn b() { raw { stack_obj := 9; } }
    #[bhav]
    fn a(sim: Sim) {
        for item in lot.objects() {
            raw { if call 4300() { temp[1] := 1; } }
        } else { gosub_found_action(); }
    }"#),
    );
    assert!(
        !tree(&air, "a").contains("stack_obj := __self_obj;"),
        "{air}"
    );
}

/// `.air` named constants are read-only operands: a constant naming a cell is rejected, dotted or
/// not, so it can never be a destination.
#[test]
fn air_constants_cannot_name_cells() {
    for decl in [
        "const X = temp[0]; random(X, 7);",
        "const T.X = temp[0]; T.X := 7;",
    ] {
        let src = format!("tree 4096 \"Probe\" {{ {decl} return true; }}");
        let err = sims_antics::compile_air(&src).expect_err("cell constant must fail");
        assert!(
            err.to_string()
                .contains("must be an integer, a literal or a bcon[..] entry"),
            "{err}"
        );
    }
}

/// A raw `for ... else` block the `.air` parser rejects still lowers with a correct loop shape:
/// the loop-exit fact falls back to the tokens while the other facts stay conservative.
#[test]
fn unparsed_raw_exhaustion_blocks_keep_the_loop_shape() {
    let src = obj(r#"    const K = 7;
    #[bhav]
    fn a() {
        for item in lot.objects() {}
        else { raw { refresh(target: K); } }
    }"#);
    let air = sims_antics::v2::expand_v2_source_to_objects(
        "else_lenient.antics",
        &src,
        &PackResolverConfig::default(),
    )
    .expect("expands")
    .remove(0)
    .air_source;
    let body = tree(&air, "a");
    let ls = lines(body);
    let raw = index_of(&ls, |l| l == "refresh(target: K);", body);
    assert_eq!(ls[raw + 1], "break;", "{body}");
}

// ---------------------------------------------------------------------------
// Review round 4
// ---------------------------------------------------------------------------

/// The `stack_obj := __self_obj;` line right before the exhaustion block's first line `first`
/// in tree `a` of `members`.
fn restore_before(members: &str, first: &str) -> bool {
    let air = expand("cursor.antics", &obj(members));
    let body = tree(&air, "a");
    let ls = lines(body);
    let at = index_of(&ls, |l| l == first, body);
    ls[at - 1] == "stack_obj := __self_obj;"
}

/// An `@slot` word stays in the frame even when nothing in the tree names it again (`.air`'s
/// `min_locals` header): a raw primitive or text may read it by number.
#[test]
fn uninitialized_slots_keep_their_frame_word() {
    let (locals, _) = nodes(&obj(r#"    #[bhav(id = 4300)]
    fn a() {
        #[slot(3)] let x: i16;
        raw { op 2 (0, 0, 3, 0, 0, 5, 8, 25); }
    }"#));
    assert_eq!(locals, 4);
}

/// The exhaustion restore follows the control flow: a raw `scope (10, ..)` write in any spacing
/// moves the cursor, a nested loop's `break` carries the moved cursor out past a later restore,
/// and a path that returns or only calls a subroutine leaves nothing to restore.
#[test]
fn exhaustion_restores_follow_writes_and_paths() {
    assert!(restore_before(
        r#"    #[bhav]
    fn a(sim: Sim) {
        for item in lot.objects() { raw { scope (10, 0):=9; } }
        else { raw { temp[1] := stack_obj; } }
    }"#,
        "temp[1] := stack_obj;",
    ));
    assert!(restore_before(
        r#"    #[bhav]
    fn a(sim: Sim) {
        for item in lot.objects() {
            while temp[0] != 0 {
                if temp[1] != 0 {
                    stack_obj = item;
                    break;
                }
                temp[2] = item.attr[0];
                temp[0] = 0;
            }
        } else { raw { temp[3] := stack_obj; } }
    }"#,
        "temp[3] := stack_obj;",
    ));
    for body in [
        "raw { stack_obj := 9; } return true;",
        "if item.attr[0] == 1 { temp[1] = 1; } raw { subroutine(4300); }",
    ] {
        let air = expand(
            "cursor_quiet.antics",
            &obj(&format!(
                "    #[bhav(id = 4300)]
    fn b() {{ return true; }}
    #[bhav]
    fn a(sim: Sim) {{
        for item in lot.objects() {{ {body} }} else {{ gosub_found_action(); }}
    }}"
            )),
        );
        let body = tree(&air, "a");
        let ls = lines(body);
        let found = index_of(&ls, |l| l == "op 0x001e (0, 0, 0, 0, 0, 0, 0, 0);", body);
        assert_ne!(ls[found - 1], "stack_obj := __self_obj;", "{body}");
    }
}

/// A raw `break` or `continue` cannot leave a loop whose exits do work, such as restoring the
/// cursor a `for ... else` moved: the raw jump would skip it.
#[test]
fn raw_loop_exits_cannot_skip_loop_work() {
    expect_err(
        "raw_break.antics",
        &obj(r#"    #[bhav]
    fn a(sim: Sim) {
        for item in lot.objects() { raw { stack_obj := 9; break; } } else { temp[2] = 1; }
        temp[1] = self.attr[0];
    }"#),
        "cannot break/continue out of a loop whose exits do work",
    );
}

/// A bool helper whose `with lock` unlocks after its `return` is tested on a saved flag, like a
/// `defer`: the unlock would otherwise replace the result the caller's arm reads.
#[test]
fn lock_helpers_branch_on_a_saved_flag() {
    let air = expand(
        "lock_result.antics",
        &obj(r#"    state occupied: bool;
    fn f() -> bool {
        with lock(self.occupied) { return temp[0] == 0; }
    }
    #[bhav]
    fn a(sim: Sim) {
        if f() {} else { gosub_found_action(); }
    }"#),
    );
    let body = tree(&air, "a");
    let ls = lines(body);
    let unlock = index_of(&ls, |l| l.ends_with(":= 0;") && l.contains("attr"), body);
    let test = index_of(&ls, |l| l.starts_with("if __cond_"), body);
    assert!(unlock < test, "{body}");
}

/// `subroutine(x)` names a tree, so slot placement leaves `x` alone.
#[test]
fn subroutine_targets_keep_slot_names() {
    let air = expand(
        "slot_subroutine.antics",
        &obj(r#"    #[bhav]
    fn __slot3_x_0() { return true; }
    #[bhav]
    fn a() {
        #[slot(3)] let x = 1;
        raw { subroutine(__slot3_x_0); }
    }"#),
    );
    let body = tree(&air, "a");
    assert!(body.contains("subroutine(__slot3_x_0);"), "{body}");
    assert!(body.contains("local[3] := 1;"), "{body}");
}

/// `.air` constants: an integer constant is a string index (`dialog(message: K)`), a raw block's
/// own `const` is its own (no second declaration from v2), and every write position rejects a
/// constant.
#[test]
fn air_constants_index_strings_and_stay_read_only() {
    let strings = r#"    strings 302 "More" { other = "Message" }
"#;
    assert_eq!(
        nodes(&obj(&format!(
            "{strings}    const MSG = 1;\n    #[bhav(id = 4300)] fn a() {{ raw {{ dialog(message: MSG); }} }}"
        ))),
        nodes(&obj(&format!(
            "{strings}    #[bhav(id = 4300)] fn a() {{ raw {{ dialog(message: 1); }} }}"
        ))),
    );
    assert_eq!(
        nodes(&obj(r#"    const K = 7;
    fn f() { raw { const K = 9; temp[0] := K; } }
    #[bhav(id = 4300)]
    fn a() { f(); }"#)),
        nodes(&obj("    #[bhav(id = 4300)] fn a() { raw { temp[0] := 9; } }")),
    );
    for stmt in [
        "expr(X := 17);",
        "if X inc_lt 5 { return true; }",
        "random(X, 7);",
        "set_to_next(all, dest: X);",
    ] {
        let src = format!("tree 4096 \"Probe\" {{ const X = 2; {stmt} return true; }}");
        let err = sims_antics::compile_air(&src).expect_err("a constant target must fail");
        assert!(
            err.to_string()
                .contains("is a constant, so it cannot be assigned"),
            "{stmt}: {err}"
        );
    }
}

// ---------------------------------------------------------------------------
// Review round 5
// ---------------------------------------------------------------------------

/// The exhaustion restore skips what cannot leave the cursor moved: `flag_test` reads the stack
/// object, code after a `return` is unreachable, and a quoted subroutine call runs in its own
/// frame.
#[test]
fn exhaustion_restores_skip_reads_unreachable_code_and_calls() {
    for body in [
        "temp[0] = item.attr[0];\n            raw {\n                if scope (10, 0) flag_test 1 {\n                    temp[1] := 1;\n                }\n            }",
        "raw {\n                return true;\n                stack_obj := 9;\n            }",
        "if item.attr[0] == 1 { temp[1] = 1; } raw { call \"b\"(); }",
    ] {
        let air = expand(
            "cursor_quiet.antics",
            &obj(&format!(
                "    #[bhav(id = 4300)]
    fn b() {{ return true; }}
    #[bhav]
    fn a(sim: Sim) {{
        for item in lot.objects() {{
            {body}
        }} else {{ gosub_found_action(); }}
    }}"
            )),
        );
        let body = tree(&air, "a");
        let ls = lines(body);
        let found = index_of(&ls, |l| l == "op 0x001e (0, 0, 0, 0, 0, 0, 0, 0);", body);
        assert_ne!(ls[found - 1], "stack_obj := __self_obj;", "{body}");
    }
}

/// Raw loop exits are judged per edge: a raw `break` out of `repeat` (whose work is the
/// `continue` step) and a raw `continue` in a `for ... else` (whose restore is on `break` and
/// exhaustion) skip nothing; a raw `continue` out of `repeat` would skip the counter step. A raw
/// block that names a constant an earlier raw block declared is still read by its shape.
#[test]
fn raw_loop_exits_are_judged_per_edge() {
    expand(
        "repeat_break.antics",
        &obj("    #[bhav] fn a() { repeat 2 { raw { break; } } temp[0] = 1; }"),
    );
    assert!(restore_before(
        r#"    #[bhav]
    fn a(sim: Sim) {
        for item in lot.objects() { raw { stack_obj := 9; continue; } }
        else { raw { temp[2] := stack_obj; } }
    }"#,
        "temp[2] := stack_obj;",
    ));
    expect_err(
        "repeat_continue.antics",
        &obj("    #[bhav] fn a() { repeat 2 { raw { temp[0] := 1; continue; } } }"),
        "cannot break/continue out of a loop whose exits do work",
    );
    expand(
        "repeat_inner.antics",
        &obj(r#"    #[bhav]
    fn a() {
        repeat 2 {
            raw { const K = 0; while temp[K] != 0 { break; } }
        }
    }"#),
    );
}

/// A raw block's own constant is owned by its full name: it moves no `@slot` binding, hides no
/// tuning entry (`Tuning.dear` with an own `Tuning`), and a helper inlined twice declares it twice
/// with the same value, which `.air` accepts.
#[test]
fn raw_own_constants_are_owned_by_full_name() {
    let (locals, _) = nodes(&obj(r#"    #[bhav(id = 4300)]
    fn a() {
        #[slot(3)] let x = 1;
        raw { const __slot3_x_0 = 2; }
        temp[0] = x;
    }"#));
    assert_eq!(locals, 4);
    let air = expand(
        "own_tuning.antics",
        &obj(r#"    bcon 4097 "Tuning" { dear = 25 }
    #[bhav]
    fn a() { raw { const Tuning = 9; temp[0] := Tuning.dear; } }"#),
    );
    assert!(tree(&air, "a").contains("const Tuning.dear = "), "{air}");
    assert_eq!(
        nodes(&obj(r#"    fn f() { raw { const K = 2; temp[0] := K; } }
    #[bhav(id = 4300)]
    fn a() { f(); f(); }"#)),
        nodes(&obj(
            "    #[bhav(id = 4300)] fn a() { raw { temp[0] := 2; temp[0] := 2; } }"
        )),
    );
}

/// `.air` constants: a comparison in `expr` reads its left side, `relationship(value:)` writes
/// it unless flag 4 is set, and a dotted integer constant is a string index.
#[test]
fn air_constants_follow_each_operands_direction() {
    let air = |body: &str| {
        sims_antics::compile_air(&format!("tree 4096 \"Probe\" {{ {body} return true; }}"))
            .map(|m| m.behaviors[&4096].instructions[0].operands)
    };
    assert_eq!(
        air("const K = 2; expr(K == temp[0]) -> true: true, false: false;").expect("reads"),
        air("expr(2 == temp[0]) -> true: true, false: false;").expect("reads"),
    );
    let err = air("const K = 2; relationship(value: K);").expect_err("output");
    assert!(err.to_string().contains("is a constant"), "{err}");
    air("const K = 2; relationship(flags: 4, value: K);").expect("input");
    assert_eq!(
        air("const Labels.Message = 1; dialog(message: Labels.Message);").expect("index"),
        air("dialog(message: 1);").expect("index"),
    );
}

/// A raw `.air` constant belongs to the raw block that declares it, and its declaration ends
/// there: its name appearing in another raw block, in any role, is an error, so each block is
/// analysed on its own. Copies of one block (a helper inlined twice) are that block, and a block
/// that names no raw constant of another changes nothing. A word before `:` is a name too
/// (`bcon[K:0]` reads a constant), and `const:` is a label, not a declaration.
#[test]
fn raw_constants_belong_to_one_block() {
    const SHARED: &str = "a raw constant's name appears only in its own block";
    expect_err(
        "label_shared.antics",
        &obj("    #[bhav] fn a() { raw { const K = 2; temp[0] := K; } raw { K: temp[1] := 3; } }"),
        SHARED,
    );
    expect_err(
        "pad_shared.antics",
        &obj("    #[bhav] fn a() { raw { const K = 0; } raw { pad(K: 0xA0); } }"),
        SHARED,
    );
    assert_eq!(
        nodes(&obj(
            "    const K = 0;\n    #[bhav(id = 4300)] fn a() { raw { temp[0] := bcon[K:0]; } }"
        )),
        nodes(&obj(
            "    #[bhav(id = 4300)] fn a() { raw { temp[0] := bcon[0:0]; } }"
        )),
    );
    nodes(&obj(
        "    #[bhav(id = 4300)] fn a() { raw { const: temp[0] := 1; } }",
    ));
    expect_err(
        "dotted_shared.antics",
        &obj(r#"    bcon 4097 "Tuning" { valid = 25 }
    #[bhav]
    fn a() {
        raw { const Tuning.extra = 2; }
        raw { temp[0] := Tuning.extra; }
    }"#),
        SHARED,
    );
    expect_err(
        "text_op_shared.antics",
        &obj(r#"    #[bhav]
    fn a() {
        raw { const TextOp = 50; }
        raw { op TextOp (45, 1, 0, 0, 1, 0, 0, 0); }
    }"#),
        SHARED,
    );
    expect_err(
        "redeclared.antics",
        &obj("    #[bhav] fn a() { raw { const K = 1; } raw { const K = 1; temp[0] := K; } }"),
        SHARED,
    );
    expect_err(
        "discarded_redeclared.antics",
        &obj("    #[bhav] fn a() { raw { const K = 1; } if false { raw { const K = 2; } } }"),
        SHARED,
    );
    expect_err(
        "split.antics",
        &obj("    #[bhav] fn a() { raw { const K = 9 } raw { ; temp[0] := 1; } }"),
        "must end with ';' in its own raw block",
    );
    let seeded = |body: &str| {
        nodes(&format!(
            "object \"Review\" {{\n    guid: 0x7F40_0800;\n    const SEED = 7;\n{body}\n}}\n"
        ))
    };
    seeded(
        r#"    fn f() { raw { const K = literal(SEED); temp[0] := K; } }
    #[bhav(id = 4300)]
    fn a() { f(); f(); }"#,
    );
    assert_eq!(
        seeded(
            r#"    #[bhav(id = 4300)]
    fn a(sim: Sim) {
        raw { const K = literal(SEED); }
        raw { if gosub_found_action() { temp[1] := 3; } else { temp[1] := 4; } }
        stack_obj = sim.container;
    }"#
        ),
        seeded(
            r#"    #[bhav(id = 4300)]
    fn a(sim: Sim) {
        raw { if gosub_found_action() { temp[1] := 3; } else { temp[1] := 4; } }
        stack_obj = sim.container;
    }"#
        ),
    );
    seeded(
        r#"    #[bhav(id = 4300)]
    fn a() {
        raw { const K = literal(SEED); }
        const C = if temp[0] == 0 { 7 } else { 9 };
        let x = C;
        temp[1] = x;
        raw { temp[2] := 3; }
    }"#,
    );
}
