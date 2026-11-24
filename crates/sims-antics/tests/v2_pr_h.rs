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
    let start = [format!("\"{name}\" {{"), format!("\"{name}\" (")]
        .iter()
        .find_map(|h| air.find(h.as_str()))
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

// ---------------------------------------------------------------------------------------------
// Tier 1: compile-time branch specialization
// ---------------------------------------------------------------------------------------------

/// The `.air` lines of a tree body, trimmed, without the header and closing brace.
fn lines(body: &str) -> Vec<&str> {
    let mut out: Vec<&str> = body
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    out.pop();
    out
}

/// A GUID chosen by a ladder feeds a primitive's GUID operand; the fallback leaves the loop.
/// Each arm runs its own `manage_inventory` with its own constant; the statement after the
/// region is shared; no local holds the choice.
#[test]
fn specialize_guid_into_a_primitive_operand() {
    let air = expand(
        "spec_guid.antics",
        &obj(r#"    const ITEM_A = 0x7F40_1001;
    const ITEM_B = 0x7F40_1002;
    tree take (sim: Sim) {
        loop {
            const ITEM = match self.n {
                1 => ITEM_A,
                2 => ITEM_B,
                3 => 0x7F40_1003,
                _ => break,
            };
            if !manage_inventory(op: 2, kind: 1, flags: 0, guid: ITEM) {
                break;
            }
            self.n = 0;
            break;
        }
    }"#),
    );
    let body = tree(&air, "take");
    let guard = |n: u32| {
        format!("if !manage_inventory(op: 2, kind: 1, flags: 0, guid: 0x7f40100{n}) {{\nbreak;\n}}")
    };
    let flat = lines(body).join("\n");
    assert!(
        flat.starts_with(&format!(
            "while true {{\nif stack.attr[0] == 1 {{\n{}\n}} else if stack.attr[0] == 2 {{\n{}\n}} else if stack.attr[0] == 3 {{\n{}\n}} else {{\nbreak;\n}}\nstack.attr[0] := 0;\nbreak;\n}}",
            guard(1),
            guard(2),
            guard(3)
        )),
        "{body}"
    );
    assert!(!body.contains("let "), "{body}");
}

/// A string index chosen by a parameter, with `stack_obj = x;` between the binding and its use:
/// the whole tail up to the last use is copied, and the fallback returns.
#[test]
fn specialize_label_with_a_statement_between() {
    let air = expand(
        "spec_label.antics",
        &obj(r#"    tree label (spell: i16) {
        const LABEL = match spell {
            1 => 13,
            2 => 14,
            _ => return false,
        };
        stack_obj = spell;
        action_label(table: 301, index: LABEL);
        return true;
    }"#),
    );
    let body = tree(&air, "label");
    assert_eq!(
        lines(body),
        [
            "if param[0] == 1 {",
            "stack_obj := param[0];",
            "action_string(table: 301, index: 13);",
            "} else if param[0] == 2 {",
            "stack_obj := param[0];",
            "action_string(table: 301, index: 14);",
            "} else {",
            "return false;",
            "}",
            "return true;",
        ],
        "{body}"
    );
}

/// A tuple from an `if` chain specializes a two-statement tail; the statement after it is
/// shared.
#[test]
fn specialize_a_tuple_into_a_two_statement_tail() {
    let air = expand(
        "spec_tuple.antics",
        &obj(r#"    tree odds (sim: Sim) {
        const (WEAR, ODDS) = if self.n == 1 {
            (10, 100)
        } else if self.n == 2 {
            (20, 200)
        } else {
            (30, 300)
        };
        if self.attr[1] <= WEAR {
            return false;
        }
        random_into(temp[0], ODDS)?;
        return temp[0] == 0;
    }"#),
    );
    let body = tree(&air, "odds");
    for (wear, odds) in [(10, 100), (20, 200), (30, 300)] {
        assert!(
            body.contains(&format!(
                "if stack.attr[1] <= {wear} {{\n                return false;"
            )) || body.contains(&format!("if stack.attr[1] <= {wear} {{")),
            "{body}"
        );
        assert!(
            body.contains(&format!("if !random(temp[0], {odds}) {{")),
            "{body}"
        );
    }
    assert_eq!(body.matches("random(").count(), 3, "{body}");
    assert_eq!(body.matches("if temp[0] == 0 {").count(), 1, "{body}");
    assert!(!body.contains("let "), "{body}");
}

/// Inline: a `match` / `if` operand of a builtin splits the statement per arm.
#[test]
fn specialize_inline_builtin_operands() {
    let air = expand(
        "spec_inline.antics",
        &obj(r#"    const CLIP_C = 3;
    tree anim (sim: Sim) {
        let roll: i16;
        let ev: i16;
        random_into(roll, 3);
        animate(clip: match roll { 0 => 1, 1 => 2, _ => CLIP_C }, table: 0, events: 0, local: ev);
        play_sound(if self.n == 1 { 0x0042 } else { 0x0043 }, stack_obj);
    }"#),
    );
    let body = tree(&air, "anim");
    for clip in 1..=3 {
        assert!(
            body.contains(&format!(
                "animate(clip: {clip}, table: 0, flags: 0x20, events: 0, local: __v_ev_1)"
            )),
            "{body}"
        );
    }
    assert!(body.contains("if __v_roll_0 == 0 {"), "{body}");
    assert!(body.contains("} else if __v_roll_0 == 1 {"), "{body}");
    assert!(body.contains("play_sound(66, stack_obj);"), "{body}");
    assert!(body.contains("play_sound(67, stack_obj);"), "{body}");
    // The roll and the event are the only locals.
    assert_eq!(body.matches("let ").count(), 2, "{body}");
}

/// Regions: a `let` the region declares and uses later pulls the later use in; a bound name
/// in a pattern takes its value; bindings nest; a binding nothing uses still runs its exits.
#[test]
fn specialize_regions_patterns_and_nesting() {
    let air = expand(
        "spec_regions.antics",
        &obj(r#"    const LOW = 1;
    tree t (k: i16) {
        const BASE = match k {
            1 => 10,
            _ => 20,
        };
        let total = BASE + 1;
        self.n = 7;
        self.attr[2] = total;
        return true;
    }
    tree u (k: i16) {
        const PICK = match k {
            0 => LOW,
            _ => 2,
        };
        match self.n {
            PICK => self.attr[1] = 5,
            _ => {}
        }
        const (A, B) = match self.attr[3] {
            4 => (40, 41),
            _ => return false,
        };
        self.attr[4] = A + PICK;
        self.attr[5] = B;
        return true;
    }
    tree w (k: i16) {
        const UNUSED = match k {
            0 => 1,
            _ => return false,
        };
        self.n = 1;
        return true;
    }"#),
    );
    // `total` is used after `self.n = 7`, so the region runs to its store. (The optimizer then
    // shares the copies' identical tails again.)
    let t = tree(&air, "t");
    assert!(t.contains("let __v_total_0 = 10;"), "{t}");
    assert!(t.contains("__v_total_0 := 20;"), "{t}");
    assert!(t.contains("my.attr[2] := __v_total_0;"), "{t}");
    let u = tree(&air, "u");
    assert!(u.contains("if my.attr[0] == 1 {"), "{u}");
    assert!(u.contains("if my.attr[0] == 2 {"), "{u}");
    assert!(u.contains("my.attr[4] := 41;") || u.contains("41"), "{u}");
    let w = tree(&air, "w");
    assert!(w.contains("if param[0] == 0 {"), "{w}");
    assert!(w.contains("return false;"), "{w}");
    assert_eq!(w.matches("my.attr[0] := 1;").count(), 1, "{w}");
}

#[test]
fn specialize_rejects_unsound_bindings() {
    for (name, body, needle) in [
        (
            "no_wild.antics",
            "const X = match k { 1 => 2, 2 => 3 }; self.n = X;",
            "must end with a '_' arm",
        ),
        (
            "no_else.antics",
            "const X = if k == 1 { 2 }; self.n = X;",
            "needs an 'else'",
        ),
        (
            "falls.antics",
            "const X = match k { 1 => 2, _ => { self.n = 1; } }; self.n = X;",
            "must end in 'break', 'continue', or 'return'",
        ),
        (
            "runtime.antics",
            "const X = match k { 1 => self.n, _ => 0 }; self.n = X;",
            "compile-time operands",
        ),
        (
            "arity.antics",
            "const (X, Y) = match k { 1 => (1, 2), _ => 3 }; self.n = X + Y;",
            "yields 1 value(s) but the binding names 2",
        ),
        (
            "shadow.antics",
            "const X = match k { 1 => 2, _ => 3 }; self.n = X; let X = 4; self.n = X;",
            "cannot be redeclared",
        ),
        (
            "written.antics",
            "let v = 1; const X = match k { 1 => v, _ => 3 }; v = 9; self.n = X;",
            "may change",
        ),
        (
            "defer.antics",
            "const X = match k { 1 => 2, _ => 3 }; defer { self.n = 0; } self.n = X;",
            "'defer' cannot be in a const binding's region",
        ),
        (
            "raw.antics",
            "const X = match k { 1 => 2, _ => 3 }; raw { my.attr[0] := X; }",
            "which 'raw { }' cannot name",
        ),
        (
            "plain.antics",
            "const X = 3; self.n = X;",
            "chooses its value with 'match' or 'if'",
        ),
        (
            "arm.antics",
            "match k { 1 => const X = match k { _ => 1 }, _ => {} }",
            "must be a statement of a '{ ... }' block",
        ),
    ] {
        expect_err(
            name,
            &obj(&format!("    tree t (k: i16) {{ {body} return true; }}")),
            needle,
        );
    }
}

/// `fmt` keeps a binding as written (it is token based), and lint accepts it.
#[test]
fn specialize_survives_fmt_and_lint() {
    let src = obj(r#"    tree label (spell: i16) {
        const LABEL = match spell {
            1 => 13,
            _ => return false,
        };
        action_label(table: 301, index: LABEL);
        return true;
    }"#);
    let formatted = sims_antics::v2::format_v2_source(&src).expect("fmt");
    assert!(
        formatted.contains("const LABEL = match spell {"),
        "{formatted}"
    );
    assert_eq!(
        sims_antics::v2::format_v2_source(&formatted).expect("fmt again"),
        formatted
    );
    sims_antics::v2::lint_v2_source(&formatted, "spec_fmt.antics").expect("lint");
    expand("spec_fmt.antics", &formatted);
}

// ---------------------------------------------------------------------------------------------
// Tier 2: `const` names inside `raw { }`
// ---------------------------------------------------------------------------------------------

/// A `const` in an operand position of raw code is its number (a GUID in hex), and a tuning
/// table entry is its `bcon[N]` operand. Labels, branch targets, fields, cells, call names,
/// argument keys, strings, comments and the raw code's own locals are left alone.
#[test]
fn raw_blocks_resolve_consts_and_tuning_entries() {
    let air = expand(
        "rawconst.antics",
        &obj(r#"    const SLOT = 3;
    const KIND = 0x7F40_1001;
    const done = 7;
    const mine = 9;
    bcon 4097 "Tuning" {
        cheap = 10,
        dear = 25,
    }
    tree t (sim: Sim) {
        raw {
            my.attr[SLOT] := SLOT;
            temp[1] := Tuning.dear;
            expr(my.attr[0] == SLOT) -> true: @done, false: false;
            temp[0] := done; // “done” stays here
          @done:
            local mine;
            mine := 1;
            set_to_next(guid, KIND, dest: temp[2]);
        }
    }
"#),
    );
    let t = tree(&air, "t");
    for want in [
        "my.attr[3] := 3;",
        "expr(my.attr[0] == 3) -> true: @done, false: false;",
        "temp[0] := 7; // “done” stays here",
        "@done:",
        "local mine;",
        "mine := 1;",
        "set_to_next(guid, 0x7f401001, dest: temp[2]);",
    ] {
        assert!(t.contains(want), "missing `{want}` in:\n{t}");
    }
    assert!(
        t.contains("temp[1] := bcon[") && !t.contains("Tuning.dear"),
        "the tuning entry is a bcon operand:\n{t}"
    );
}

/// A name one raw block declares as a local stays that local in every raw block of the tree,
/// and declaring a name an earlier raw block read as a const is an error.
#[test]
fn raw_locals_shadow_consts_tree_wide() {
    let air = expand(
        "rawshadow.antics",
        &obj(r#"    const k = 5;
    tree t {
        raw {
            local k;
            k := 1;
        }
        raw { temp[0] := k; }
    }
"#),
    );
    assert!(tree(&air, "t").contains("temp[0] := k;"), "{air}");
    expect_err(
        "rawclash.antics",
        &obj(r#"    const k = 5;
    tree t {
        raw { temp[0] := k; }
        raw {
            local k;
            k := 1;
        }
    }
"#),
        "which an earlier raw block of this tree read as a const",
    );
}

// ---------------------------------------------------------------------------------------------
// Tier 2: `@slot(N) let`
// ---------------------------------------------------------------------------------------------

/// BHAV `id` of the compiled object.
fn bhav(name: &str, src: &str, id: u16) -> sims_formats::behavior::Behavior {
    use sims_formats::behavior::Behavior;
    use sims_formats::iff::Iff;
    let obj = compile_v2_source(name, src, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{name} must compile: {e}"));
    let iff = Iff::parse(obj.iff_bytes.clone()).expect("IFF parse");
    let chunk = iff.get(b"BHAV", id).expect("BHAV present");
    Behavior::parse(iff.data(chunk)).expect("BHAV parse")
}

/// The local count of BHAV `id` in the compiled object.
fn bhav_locals(name: &str, src: &str, id: u16) -> u16 {
    bhav(name, src, id).locals
}

/// A pinned variable lives in `local[N]`, and the tree's other locals take the lowest words
/// around it (not the words after it), so the frame stays as small as the pin allows.
#[test]
fn slot_lets_pin_their_word_and_pack_the_rest() {
    let src = obj(r#"    tree t (sim: Sim) {
        @slot(3) let ev: i16;
        let a = sim.attr[2];
        let b = sim.attr[3] + a;
        while !animate(clip: 1, table: 0, events: 0, local: ev) {
            sim.attr[1] = ev;
        }
        self.n = a + b;
    }
"#);
    let air = expand("slot.antics", &src);
    let t = tree(&air, "t");
    for want in [
        "local[0] := my.attr[2];",
        "local[1] := my.attr[3];",
        "local[1] += local[0];",
        "events: 0, local: local[3])",
        "my.attr[1] := local[3];",
    ] {
        assert!(t.contains(want), "missing `{want}` in:\n{t}");
    }
    assert!(!t.contains("let "), "every local is placed:\n{t}");
    assert_eq!(bhav_locals("slot.antics", &src, 4098), 4);
}

/// A pinned word can be shared with a `raw` block on purpose (a value only some paths write),
/// and a tree that formats text keeps a pinned store its `$Local:N` string reads.
#[test]
fn slot_lets_share_words_with_raw_and_text() {
    let air = expand(
        "slotraw.antics",
        &obj(r#"    strings 301 "Notes" {
        title = "Count",
        msg = "Left $Local:2",
    }
    tree u (sim: Sim) {
        @slot(5) let pref: i16;
        if sim.attr[0] == 1 {
            pref = sim.data[9];
        }
        sim.data[9] = pref;
        raw { temp[0] := local[5]; }
    }
    tree v {
        @slot(2) let left = self.n + 4;
        dialog(kind: ok, title: title, message: msg);
    }
"#),
    );
    let u = tree(&air, "u");
    for want in [
        "local[5] := my.data[9];",
        "my.data[9] := local[5];",
        "temp[0] := local[5];",
    ] {
        assert!(u.contains(want), "missing `{want}` in:\n{u}");
    }
    let v = tree(&air, "v");
    assert!(
        v.contains("local[2] := my.attr[0];") && v.contains("local[2] += 4;"),
        "{v}"
    );
}

#[test]
fn slot_lets_reject_unsound_pins() {
    expect_err(
        "slot_twice.antics",
        &obj(r#"    tree t {
        @slot(1) let a = 1;
        @slot(1) let b = 2;
        self.n = a + b;
    }
"#),
        "local slot 1 already holds 'a'",
    );
    expect_err(
        "slot_inline.antics",
        &obj(r#"    fn helper() -> i16 {
        @slot(1) let a = 1;
        return a;
    }
    tree t { self.n = helper(); }
"#),
        "not allowed in an inlined helper",
    );
    expect_err(
        "slot_push.antics",
        &obj(r#"    tree go_run (sim: Sim) {
        @slot(1) let a = 1;
        let o = stack_obj;
        stack_obj = sim;
        push_interaction(object: o, menu: 0, priority: 2, icon: o);
        self.n = a;
    }
"#),
        "cannot share a tree with '@slot(N) let'",
    );
    expect_err(
        "slot_range.antics",
        &obj("    tree t { @slot(300) let a = 1; self.n = a; }\n"),
        "out of range",
    );
    expect_err(
        "slot_some.antics",
        &obj("    tree t { @slot(1) let Some(x) = find(PROBE) else { return; } }\n"),
        "applies to 'let x = ..;'",
    );
}

/// Disjoint scopes may reuse a slot, and `@slot` survives `fmt` and `lint`.
#[test]
fn slot_lets_reuse_across_scopes_and_survive_fmt() {
    let src = obj(r#"    tree t {
        if self.n == 1 {
            @slot(2) let a = 1;
            self.n = a;
        } else {
            @slot(2) let b: i16;
            self.n = b;
        }
    }
"#);
    // `fmt` (owned by the lint track) does not know the attribute yet and writes `@ slot(2)`,
    // which parses the same.
    let formatted = sims_antics::v2::format_v2_source(&src).expect("fmt");
    assert!(formatted.contains("slot(2) let a = 1;"), "{formatted}");
    assert_eq!(
        sims_antics::v2::format_v2_source(&formatted).expect("fmt again"),
        formatted
    );
    sims_antics::v2::lint_v2_source(&formatted, "slot_fmt.antics").expect("lint");
    let air = expand("slot_fmt.antics", &formatted);
    let t = tree(&air, "t");
    assert!(t.contains("my.attr[0] := local[2];"), "{t}");
}

// ---------------------------------------------------------------------------------------------
// Tier 2: `@frame(...)` on lifecycle blocks
// ---------------------------------------------------------------------------------------------

/// `init`, `main` and the OBJf hooks take `@frame(...)` like a standalone tree, so an object
/// whose main declares four arguments is written as `main { }` (no extra stub tree), and
/// `param[N]` below the declared count is addressable.
#[test]
fn lifecycle_blocks_take_frames() {
    let src = obj(r#"    @id(4100) @frame(args: 4, locals: 2) main {
        loop {
            sleep(10);
            self.n += param[3];
        }
    }
    @frame(locals: 1, revision: 3) init {
        self.n = 5;
    }
    @frame(locals: 0) load { self.n = 0; }
"#);
    let air = expand("lifeframe.antics", &src);
    let headers: Vec<&str> = air.lines().filter(|l| l.starts_with("tree ")).collect();
    assert_eq!(
        headers,
        [
            "tree 4096 \"Init\" (locals: 1, revision: 3) {",
            "tree 4100 \"Main\" (args: 4, locals: 2) {",
            "tree 4097 \"Load\" (locals: 0) {",
        ],
        "{air}"
    );
    let main = bhav("lifeframe.antics", &src, 4100);
    assert_eq!((main.args, main.locals), (4, 2));
    let init = bhav("lifeframe.antics", &src, 4096);
    assert_eq!((init.locals, init.revision), (1, 3));
}

#[test]
fn lifecycle_frames_are_checked() {
    expect_err(
        "lifeempty.antics",
        &obj("    @frame(empty) main { }\n"),
        "@frame(empty) is only allowed on a standalone 'tree'",
    );
    expect_err(
        "lifeparam.antics",
        &obj("    main { self.n = param[0]; }\n"),
        "param",
    );
    expect_err(
        "lifeconst.antics",
        &obj("    @frame(locals: 1) const K = 1;\n"),
        "@frame(...) is only allowed on a standalone 'tree' or on init, main",
    );
}
