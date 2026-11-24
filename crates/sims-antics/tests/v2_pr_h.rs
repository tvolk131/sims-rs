//! `.antics` v2 PR H: pilot regressions, compile-time branch specialization, and raw-eliminating
//! features. All sources are synthetic.

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

/// A synthetic object with one routing slot (`front`, routing ordinal 0) around `members`.
fn obj(members: &str) -> String {
    format!(
        r#"object "Pr H" {{
    guid: 0x7F40_0250;
    state n: i16;

    slot front {{
        kind: routing,
        offset: [0.0, -0.75, 0.0],
        standing: true,
        sitting: false,
        ground: true,
        facing: north,
    }}
{members}
}}
"#
    )
}

/// The `.air` body of tree `name` (from its header to the next tree).
fn tree<'a>(air: &'a str, name: &str) -> &'a str {
    let start = air
        .find(&format!("\"{name}\" {{"))
        .unwrap_or_else(|| panic!("tree {name} in:\n{air}"));
    let rest = &air[start..];
    let end = rest[1..].find("\ntree ").map_or(rest.len(), |i| i + 1);
    &rest[..end]
}

// ---------------------------------------------------------------------------------------------
// Tier 1: the entry save (`let __self_obj = stack_obj;`) after PR G
// ---------------------------------------------------------------------------------------------

/// Out-operands write their local: `random_into` on both exits, `animate(local:)` on the false
/// exit (an event), relationship reads on the true exit. A tree whose uninitialized lets are all
/// written that way before any read needs no save, even next to `route_to`, a `stack_obj`
/// clobber and a one-line raw `if`.
#[test]
fn out_operand_writes_do_not_keep_the_entry_save() {
    let air = expand(
        "outs.antics",
        &obj(r#"    tree t (sim: Sim) {
        let a: i16;
        let ev: i16;
        random_into(a, 10);
        stack_obj = sim.container;
        stack_obj.attr[0] = a;
        while !animate(clip: 1, table: 0, events: 0, local: ev) {
            sim.attr[1] = ev;
        }
        route_to(front);
        raw { if random(temp[0], 3) { temp[1] := 2; } }
    }
    tree u (sim: Sim) {
        let a: i16;
        relationship(column: 0, dir: 0, flags: 0, value: a);
        stack_obj = sim.container;
        sim.attr[1] = a;
    }
    tree v (sim: Sim) {
        let ev: i16;
        stack_obj = sim.container;
        animate(clip: 1, table: 0, events: 1, local: ev) |e| {
            if e == 3 { break; }
        }
        route_to(front);
    }"#),
    );
    for name in ["t", "u", "v"] {
        let body = tree(&air, name);
        assert!(!body.contains("__self_obj"), "{name}:\n{body}");
    }
}

/// A local read on a path where nothing wrote it reads the frame word the save would occupy, so
/// the save stays: animate takes its true exit (finished) without writing `local:`.
#[test]
fn a_read_of_an_unwritten_out_operand_keeps_the_entry_save() {
    let air = expand(
        "unwritten.antics",
        &obj(r#"    tree t (sim: Sim) {
        let a: i16;
        let ev: i16;
        random_into(a, 10);
        stack_obj = sim.container;
        stack_obj.attr[0] = a;
        while !animate(clip: 1, table: 0, events: 0, local: ev) {}
        sim.attr[1] = ev;
    }
    tree u (sim: Sim) {
        let a: i16;
        let ev: i16;
        random_into(a, 10);
        stack_obj = sim.container;
        stack_obj.attr[0] = a;
        if relationship(column: 0, dir: 0, flags: 1, value: ev) {}
        sim.attr[1] = ev;
    }
    tree v (sim: Sim) {
        let a: i16;
        let ev: i16;
        random_into(a, 10);
        stack_obj = sim.container;
        stack_obj.attr[0] = a;
        while true {
            if animate(clip: 1, table: 0, events: 0, local: ev) { break; }
            if self.n == 2 { break; }
        }
        sim.attr[1] = ev;
    }"#),
    );
    for name in ["t", "u", "v"] {
        let body = tree(&air, name);
        assert!(
            body.contains("let __self_obj = stack_obj;"),
            "{name}:\n{body}"
        );
    }
}

/// `sim.attr[i]` / `sim.data[i]` in a Sim tree are the running frame's own cells: they never
/// move the stack object, so they need no save and no restore.
#[test]
fn sim_cells_do_not_clobber_the_stack_object() {
    let air = expand(
        "sim_cells.antics",
        &obj(r#"    tree t (sim: Sim) {
        sim.attr[2] = sim.data[3];
        self.n = 1;
    }"#),
    );
    let body = tree(&air, "t");
    assert!(!body.contains("__self_obj"), "{body}");
    assert!(body.contains("my.attr[2] := my.data[3];"), "{body}");
    assert!(body.contains("stack.attr[0] := 1;"), "{body}");
}

/// A raw block that moves the stack object and restores it itself (labels included) does not
/// keep an unread save.
#[test]
fn raw_blocks_that_restore_the_stack_object_drop_the_save() {
    let air = expand(
        "raw_restore.antics",
        &obj(r#"    tree t (sim: Sim) {
        raw {
            local[0] := stack_obj;
            stack_obj := my.data[2];
            stack.attr[0] := 1;
          done:
            stack_obj := local[0];
        }
    }"#),
    );
    let body = tree(&air, "t");
    assert!(!body.contains("__self_obj"), "{body}");
}

// ---------------------------------------------------------------------------------------------
// Tier 1: dialog / action-string text reads frame locals
// ---------------------------------------------------------------------------------------------

/// `$Local:N` in dialog text reads frame words by position, so a tree that formats text keeps
/// every user local's store and slot: none is dropped as dead or merged with another.
#[test]
fn text_formatting_keeps_user_locals_in_place() {
    let air = expand(
        "text.antics",
        &obj(r#"    strings 301 "Notes" {
        title = "Count",
        msg = "From $Local:0 to $Local:1",
    }
    tree t {
        let start = self.n;
        let stop = self.n + 4;
        dialog(kind: ok, title: title, message: msg);
    }"#),
    );
    let body = tree(&air, "t");
    assert!(body.contains("let __v_start_0 = my.attr[0];"), "{body}");
    assert!(body.contains("let __v_stop_1 = my.attr[0];"), "{body}");
    assert!(body.contains("__v_stop_1 += 4;"), "{body}");
}

// ---------------------------------------------------------------------------------------------
// Tier 1: relationship(value:) and `return a != b`
// ---------------------------------------------------------------------------------------------

/// With flag 4 the VM reads `value:`, so any i16 value works (computed into a temporary when it
/// needs lines); otherwise the VM writes it, so any writable direct i16 cell works.
#[test]
fn relationship_value_takes_cells_and_constants() {
    let air = expand(
        "rel.antics",
        &obj(r#"    tree t (sim: Sim) {
        let o = 0;
        relationship(column: 0, dir: 2, flags: 4, local: o, value: 25);
        relationship(column: 1, dir: 0, flags: 4, value: self.n);
        relationship(column: 1, dir: 0, flags: 0, value: self.n);
        relationship(column: 1, dir: 0, flags: 1, value: temp[0]);
        relationship(column: 2, dir: 0, flags: 4, value: self.n + 3);
    }"#),
    );
    let body = tree(&air, "t");
    for needle in [
        "relationship(column: 0, dir: 2, flags: 4, local: __v_o_0, value: 25);",
        "relationship(column: 1, dir: 0, flags: 4, value: stack.attr[0]);",
        "relationship(column: 1, dir: 0, flags: 0, value: stack.attr[0]);",
        "relationship(column: 1, dir: 0, flags: 1, value: temp[0]);",
        "__arith_1 += 3;\n    relationship(column: 2, dir: 0, flags: 4, value: __arith_1);",
    ] {
        assert!(body.contains(needle), "missing '{needle}':\n{body}");
    }
    for (name, body, needle) in [
        (
            "rel_const.antics",
            "relationship(column: 1, dir: 0, flags: 0, value: 25);",
            "without flag 4 stores the number into a cell, so a constant is not allowed",
        ),
        (
            "rel_expr.antics",
            "relationship(column: 1, dir: 0, flags: 0, value: self.n + 1);",
            "not a valid assignment target",
        ),
        (
            "rel_slot.antics",
            "relationship(column: 1, dir: 0, value: sim.hand);",
            "is read-only",
        ),
    ] {
        expect_err(
            name,
            &obj(&format!("    tree t (sim: Sim) {{ {body} }}")),
            needle,
        );
    }
}

/// A relationship read stores into its `value:` local, so it is a write for the analyses that
/// track locals (an inline helper may not substitute a param that the call writes, ...).
#[test]
fn relationship_reads_store_into_their_local() {
    let air = expand(
        "rel_write.antics",
        &obj(r#"    tree t (sim: Sim) {
        let score = 3;
        relationship(column: 0, dir: 0, flags: 0, value: score);
        self.n = score;
    }"#),
    );
    let body = tree(&air, "t");
    assert!(body.contains("value: __v_score_0);"), "{body}");
    assert!(body.contains("stack.attr[0] := __v_score_0;"), "{body}");
}

/// The node you write is the node you get: `return a != b` is one `!=` node returning on its
/// exits, the same as `if a != b { return true; } return false;`; `return !(a == b)` is the `==`
/// node with its exits swapped, which is what a tree that tests `==` and returns false on a match
/// compiles to.
#[test]
fn return_compare_keeps_its_operator() {
    let air = expand(
        "ret.antics",
        &obj(r#"    tree a { return self.n == 1; }
    tree b { return self.n != 1; }
    tree c { return !(self.n == 1); }
    tree d { if self.n != 1 { return true; } return false; }"#),
    );
    assert!(tree(&air, "a").contains("if my.attr[0] == 1 {"));
    assert!(tree(&air, "b").contains("if my.attr[0] != 1 {"));
    assert!(tree(&air, "c").contains("if !my.attr[0] == 1 {"));
    assert_eq!(
        tree(&air, "b").replace("\"b\"", "").trim_end(),
        tree(&air, "d").replace("\"d\"", "").trim_end()
    );
}
