//! `.antics` v2.2 polish: the bitflags list methods (`set_all`, `clear_all`, `contains_all`,
//! `contains_any`), zero-copy argument staging for calls that pass arguments in temps, and the
//! `#[ret(temp = N)] -> Option<T>` return ABI (a true/false answer that carries a value).
//! All sources are synthetic.

use sims_antics::v2::{format_v2_source, is_v2_formatted};
use sims_antics::{PackResolverConfig, compile_v2_source};

fn iff(name: &str, src: &str) -> Vec<u8> {
    compile_v2_source(name, src, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{name} must compile: {e}\n{src}"))
        .iff_bytes
}

fn air(name: &str, src: &str) -> String {
    sims_antics::v2::expand_v2_source_to_objects(name, src, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{name} must expand: {e}\n{src}"))
        .into_iter()
        .next()
        .expect("one object")
        .air_source
}

fn expect_err(name: &str, src: &str, needle: &str) {
    let err = compile_v2_source(name, src, &PackResolverConfig::default())
        .expect_err(&format!("{name} must fail:\n{src}"));
    assert!(
        err.message.contains(needle),
        "{name}: expected '{needle}' in: {err}"
    );
}

/// The `.air` body of tree `t`, its lines trimmed, without the header and closing brace.
fn tree_t(air: &str) -> Vec<String> {
    let at = air.find("\"t\"").expect("tree t in .air");
    air[at..]
        .lines()
        .skip(1)
        .take_while(|l| *l != "}")
        .map(|l| l.trim().to_owned())
        .collect()
}

/// Tree `t`'s node count and frame locals in the compiled `.iff` (`t` is the object's first
/// `#[bhav] fn`, so it gets the first id after Init and Main).
fn t_frame(src: &str) -> (usize, u16) {
    let a = air("frame.antics", src);
    assert!(a.contains("tree 4098 \"t\""), "t must be tree 4098:\n{a}");
    let compiled = compile_v2_source("frame.antics", src, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("must compile: {e}\n{src}"));
    let bhav = &compiled.behaviors[&4098];
    (bhav.instructions.len(), bhav.locals)
}

/// A synthetic object: `items` as object members, then one object-context tree `t` running
/// `body` (no Sim, so no entry save of the stack object).
fn obj(uses: &str, items: &str, body: &str) -> String {
    format!(
        r#"{uses}
object "Temps And Flags" {{
    guid: 0x7F40_02A0;
    state n: i16;
    {items}
    #[bhav] fn t() {{ {body} }}
}}
"#
    )
}

/// Asserts that two bodies of `t` compile to the same `.iff` bytes.
fn same_iff(case: &str, uses: &str, items: &str, a: &str, b: &str) {
    let (sa, sb) = (obj(uses, items, a), obj(uses, items, b));
    let (ia, ib) = (iff("a.antics", &sa), iff("b.antics", &sb));
    assert!(
        ia == ib,
        "{case}: .iff bytes differ\n--- a\n{:#?}\n--- b\n{:#?}",
        tree_t(&air("a.antics", &sa)),
        tree_t(&air("b.antics", &sb)),
    );
}

fn lines(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

const FLAGS: &str = "use std::flags::*;";

// ---------------------------------------------------------------------------------------
// Bitflags list methods
// ---------------------------------------------------------------------------------------

#[test]
fn flag_lists_lower_exactly_as_the_union_forms() {
    let (h, u, s) = (
        "ObjectFlags::Hidden",
        "ObjectFlags::InUse",
        "ObjectFlags::NotSolid",
    );
    let cases = [
        (
            format!("self.flags.set_all([{h}, {u}]);"),
            format!("self.flags.set({h} | {u});"),
        ),
        (
            format!("self.flags.set_all([{h}, {u}, {s}]);"),
            format!("self.flags.set({h} | {u} | {s});"),
        ),
        (
            format!("self.flags.set_all([{h}]);"),
            format!("self.flags.set({h});"),
        ),
        (
            format!("self.flags.clear_all([{h}, {u}]);"),
            format!("self.flags.clear({h} | {u});"),
        ),
        (
            format!("self.flags.clear_all([{u}]);"),
            format!("self.flags.clear({u});"),
        ),
        (
            format!("if self.flags.contains_all([{h}, {u}]) {{ self.n = 1; }}"),
            format!("if self.flags.contains({h} | {u}) {{ self.n = 1; }}"),
        ),
        (
            format!("if !self.flags.contains_all([{h}, {u}]) {{ self.n = 1; }}"),
            format!("if !self.flags.contains({h} | {u}) {{ self.n = 1; }}"),
        ),
        (
            format!("if self.flags.contains_any([{h}, {u}]) {{ self.n = 1; }}"),
            format!("if self.flags.intersects({h} | {u}) {{ self.n = 1; }}"),
        ),
        (
            format!("if !self.flags.contains_any([{h}, {u}]) {{ self.n = 1; }}"),
            format!("if !self.flags.intersects({h} | {u}) {{ self.n = 1; }}"),
        ),
        (
            format!("if self.flags.contains_any([{h}]) {{ self.n = 1; }}"),
            format!("if self.flags.intersects({h}) {{ self.n = 1; }}"),
        ),
        (
            format!("let b = self.flags.contains_all([{h}, {u}]); if b {{ self.n = 2; }}"),
            format!("let b = self.flags.contains({h} | {u}); if b {{ self.n = 2; }}"),
        ),
        (
            format!(
                "while self.flags.contains_any([{h}, {u}]) {{ self.flags.clear_all([{h}, {u}]); }}"
            ),
            format!("while self.flags.intersects({h} | {u}) {{ self.flags.clear({h} | {u}); }}"),
        ),
        // A runtime element takes the runtime path of the union form.
        (
            format!("let f = ObjectFlags::from_raw(self.n); self.flags.set_all([f, {u}]);"),
            format!("let f = ObjectFlags::from_raw(self.n); self.flags.set(f | {u});"),
        ),
        (
            format!("let f = ObjectFlags::from_raw(self.n); self.flags.clear_all([f, {u}]);"),
            format!("let f = ObjectFlags::from_raw(self.n); self.flags.clear(f | {u});"),
        ),
        (
            format!(
                "let f = ObjectFlags::from_raw(self.n); if self.flags.contains_all([{u}, f]) {{ self.n = 1; }}"
            ),
            format!(
                "let f = ObjectFlags::from_raw(self.n); if self.flags.contains({u} | f) {{ self.n = 1; }}"
            ),
        ),
    ];
    for (i, (list, union)) in cases.iter().enumerate() {
        same_iff(&format!("case {i}: {list}"), FLAGS, "", list, union);
    }
    // One flag is one bit operation; several are one mask operation.
    let a = tree_t(&air(
        "nodes.antics",
        &obj(
            FLAGS,
            "",
            &format!(
                "self.flags.set_all([{h}]); self.flags.set_all([{h}, {u}]); \
                 self.flags.clear_all([{u}]); self.flags.clear_all([{h}, {u}]);"
            ),
        ),
    ));
    assert_eq!(
        a,
        lines(&[
            "my.data[8] flag_set 15;",
            "my.data[8] |= 16416;",
            "my.data[8] flag_clear 6;",
            "my.data[8] &= -16417;",
            "return true;",
        ])
    );
}

#[test]
fn flag_lists_work_on_mut_refs_and_declared_bitflags() {
    let items = "bitflags Moods { Calm = 1, Bright = 3, Keen = 5 }
    state m: Moods;
    fn mark(m: &mut Moods) { m.set_all([Moods::Calm, Moods::Keen]); }
    fn unmark(m: &mut Moods) { m.clear(Moods::Calm | Moods::Keen); }";
    same_iff(
        "mut ref",
        "",
        items,
        "mark(&mut self.m); if self.m.contains_any([Moods::Bright, Moods::Keen]) { unmark(&mut self.m); }",
        "self.m.set(Moods::Calm | Moods::Keen); if self.m.intersects(Moods::Bright | Moods::Keen) { self.m.clear_all([Moods::Calm, Moods::Keen]); }",
    );
}

#[test]
fn flag_list_misuse_is_refused() {
    let bad = |name: &str, body: &str, needle: &str| {
        expect_err(name, &obj(FLAGS, "", body), needle);
    };
    bad(
        "empty_set",
        "self.flags.set_all([]);",
        "'.set_all([])' names no flags",
    );
    bad(
        "empty_clear",
        "self.flags.clear_all([]);",
        "'.clear_all([])' names no flags",
    );
    bad(
        "empty_all",
        "if self.flags.contains_all([]) { self.n = 1; }",
        "'.contains_all([])' names no flags",
    );
    bad(
        "empty_any",
        "if self.flags.contains_any([]) { self.n = 1; }",
        "'.contains_any([])' names no flags",
    );
    bad(
        "no_brackets",
        "self.flags.set_all(ObjectFlags::Hidden);",
        "takes a bracketed list of flags",
    );
    bad(
        "two_lists",
        "self.flags.set_all([ObjectFlags::Hidden], [ObjectFlags::InUse]);",
        "takes exactly one argument",
    );
    bad(
        "other_flags",
        "self.flags.set_all([ObjectFlags::Hidden, Placement::Floor]);",
        "type mismatch",
    );
    bad(
        "i16_element",
        "let x = 4; self.flags.set_all([x]);",
        "type mismatch in '.set_all(...)'",
    );
    bad(
        "not_bitflags",
        "self.n.set_all([ObjectFlags::Hidden]);",
        "'.set_all(...)' requires a bitflags target",
    );
    bad(
        "not_bitflags_test",
        "if self.n.contains_any([ObjectFlags::Hidden]) { self.n = 1; }",
        "'.contains_any(...)' requires a bitflags receiver",
    );
    bad(
        "type_args",
        "self.flags.clear_all::<i16>([ObjectFlags::Hidden]);",
        "does not accept type arguments",
    );
    // `[..]` is only the list of these methods.
    bad(
        "stray_list",
        "self.flags.set([ObjectFlags::Hidden]);",
        "expected expression",
    );
}

#[test]
fn flag_lists_format_and_stay_formatted() {
    let src = obj(
        FLAGS,
        "",
        "self.flags.set_all([ ObjectFlags::Hidden,ObjectFlags::InUse ]); \
         if self.flags.contains_any([ObjectFlags::Hidden]) { self.flags.clear_all([ObjectFlags::InUse]); }",
    );
    let formatted = format_v2_source(&src).expect("format");
    for want in [
        "self.flags.set_all([ObjectFlags::Hidden, ObjectFlags::InUse]);",
        "if self.flags.contains_any([ObjectFlags::Hidden]) {",
        "self.flags.clear_all([ObjectFlags::InUse]);",
    ] {
        assert!(
            formatted.contains(want),
            "missing `{want}` in:\n{formatted}"
        );
    }
    assert!(is_v2_formatted(&formatted).expect("check"));
    assert_eq!(iff("raw.antics", &src), iff("fmt.antics", &formatted));
}

// ---------------------------------------------------------------------------------------
// Zero-copy temps staging
// ---------------------------------------------------------------------------------------

const EXTERNS: &str = "extern mod g {
    #[id(290)] fn two(a: i16, b: i16) -> bool;
    #[id(291)] fn three(a: i16, b: i16, c: i16) -> bool;
    #[id(292)] fn pinned(#[temp(0)] a: i16, #[temp(1)] b: i16) -> bool;
    #[id(293)] fn high(#[temp(3)] x: i16) -> bool;
    #[id(294)] #[ret(temp = 0)] fn value(x: i16) -> i16;
}";

/// `t`'s `.air` body, node count and frame locals for `body` with the test externs in scope.
fn staged(body: &str) -> (Vec<String>, usize, u16) {
    // `t` takes an argument: an argless object tree may be an object's bottom frame, where a
    // call staged in place that would leave no frame word at all takes the copying form.
    let src = obj(EXTERNS, "", body).replace("fn t()", "fn t(x: i16)");
    let (nodes, locals) = t_frame(&src);
    (tree_t(&air("staged.antics", &src)), nodes, locals)
}

#[test]
fn arguments_already_in_their_temps_need_no_copy() {
    // Positional arguments in order, `#[temp(N)]` parameters fed from their own temp, and a
    // temp parameter above the positional window: no local, and of the stores only the last,
    // the staging's final Expression (its result and scratch pointer reach the callee).
    for (body, last) in [
        ("if g::two(temp[0], temp[1]) { self.n = 1; }", 1),
        ("if g::pinned(temp[0], temp[1]) { self.n = 1; }", 1),
        ("if g::pinned(b: temp[1], a: temp[0]) { self.n = 1; }", 1),
        ("if g::high(temp[3]) { self.n = 1; }", 3),
    ] {
        let (text, nodes, locals) = staged(body);
        assert_eq!(text[0], format!("temp[{last}] := temp[{last}];"), "{body}");
        assert_eq!(
            text[1].split(" {").next().map(|c| c.ends_with("(temps)")),
            Some(true)
        );
        assert_eq!((nodes, locals), (3, 0), "{body}: {text:#?}");
    }
    // The same in a tree that declares no locals at all.
    let src = format!(
        r#"{EXTERNS}
object "Temps And Flags" {{
    guid: 0x7F40_02A1;
    state n: i16;
    #[bhav(locals = 0)]
    fn t() {{ if g::three(temp[0], temp[1], temp[2]) {{ self.n = 1; }} }}
}}
"#
    );
    assert_eq!(
        tree_t(&air("locals0.antics", &src)),
        lines(&[
            "temp[2] := temp[2];",
            "if call 291(temps) {",
            "my.attr[0] := 1;",
            "}",
            "return true;"
        ])
    );
}

#[test]
fn staging_orders_stores_so_no_temp_is_read_after_it_is_overwritten() {
    // A swap is a cycle: one copy breaks it.
    let (text, nodes, locals) = staged("if g::two(temp[1], temp[0]) { self.n = 1; }");
    assert_eq!(
        text,
        lines(&[
            "let __arg_0 = temp[0];",
            "temp[0] := temp[1];",
            "temp[1] := __arg_0;",
            "if call 290(temps) {",
            "my.attr[0] := 1;",
            "}",
            "return true;",
        ])
    );
    assert_eq!((nodes, locals), (5, 1));
    // A rotation of three is one cycle too.
    let (text, _, locals) = staged("if g::three(temp[1], temp[2], temp[0]) { self.n = 1; }");
    assert_eq!(
        &text[..4],
        &lines(&[
            "let __arg_0 = temp[0];",
            "temp[0] := temp[1];",
            "temp[1] := temp[2];",
            "temp[2] := __arg_0;",
        ])[..]
    );
    assert_eq!(locals, 1);
    // A call with any argument but a literal or a bare temp stages in the copying form (its
    // read may fault, and the temps and last result it leaves must be the copying form's).
    let (text, _, locals) = staged("if g::two(self.n, temp[0]) { self.n = 1; }");
    assert_eq!(
        &text[..3],
        &lines(&[
            "let __arg_0 = temp[0];",
            "temp[0] := my.attr[0];",
            "temp[1] := __arg_0;",
        ])[..]
    );
    assert_eq!(locals, 1);
    // In place, the stores keep parameter order: reading forward needs no save.
    let (text, _, locals) = staged("if g::two(temp[1], temp[2]) { self.n = 1; }");
    assert_eq!(
        &text[..2],
        &lines(&["temp[0] := temp[1];", "temp[1] := temp[2];"])[..]
    );
    assert_eq!(locals, 0);
    let (text, _, locals) = staged("if g::two(5, temp[0]) { self.n = 1; }");
    assert_eq!(
        &text[..3],
        &lines(&[
            "let __arg_0 = temp[0];",
            "temp[0] := 5;",
            "temp[1] := __arg_0;"
        ])[..]
    );
    assert_eq!(locals, 1);
    // An operand that may fault: `temp[1]` is copied before the stack object's attribute is
    // read, as in the copying form.
    let (text, _, _) = staged("if g::three(stack_obj.attr[0], self.n, temp[1]) { self.n = 1; }");
    let save = text.iter().position(|l| l == "let __arg_0 = temp[1];");
    let read = text.iter().position(|l| l.contains("stack.attr[0]"));
    assert!(
        matches!((save, read), (Some(s), Some(r)) if s < r),
        "{text:#?}"
    );
    // One temp feeding two parameters.
    let (text, _, locals) = staged("if g::two(temp[0], temp[0]) { self.n = 1; }");
    assert_eq!(
        &text[..2],
        &lines(&["temp[1] := temp[0];", "if call 290(temps) {"])[..]
    );
    assert_eq!(locals, 0);
    // A computed argument reading a temp: the copying form.
    let (text, _, _) = staged("if g::two(temp[0] + 1, temp[0]) { self.n = 1; }");
    assert_eq!(
        &text[..5],
        &lines(&[
            "let __arith_0 = temp[0];",
            "__arith_0 += 1;",
            "let __arg_1 = temp[0];",
            "temp[0] := __arith_0;",
            "temp[1] := __arg_1;",
        ])[..]
    );
}

#[test]
fn a_temp_a_later_argument_may_change_is_still_copied_first() {
    // `g::value(5)` runs a tree, which may write any temp: `temp[1]` is read before it.
    let (text, _, _) = staged("if g::two(temp[1], g::value(5)) { self.n = 1; }");
    let read = text.iter().position(|l| l == "let __arg_0 = temp[1];");
    let call = text.iter().position(|l| l.starts_with("if call 294(5)"));
    assert!(
        matches!((read, call), (Some(r), Some(c)) if r < c),
        "{text:#?}"
    );
}

#[test]
fn constant_arguments_keep_the_literal_call() {
    let (text, nodes, locals) =
        staged("if g::two(1, 2) { self.n = 1; } if g::pinned(4, temp[1]) { self.n = 2; }");
    assert_eq!(
        text,
        lines(&[
            "if call 290(1, 2) {",
            "my.attr[0] := 1;",
            "}",
            "temp[0] := 4;",
            "temp[1] := temp[1];",
            "if call 292(temps) {",
            "my.attr[0] := 2;",
            "}",
            "return true;",
        ])
    );
    assert_eq!((nodes, locals), (6, 0));
}

#[test]
fn std_globals_called_with_staged_temps_pass_the_temp_as_it_is() {
    // The trees the originals call in temps mode: the argument already in `temp[0]` is passed
    // as it is, with no copy; its store stays as the staging's final Expression.
    let t = tree_t(&air(
        "wants_fun.antics",
        &obj(
            "use std::globals::*;",
            "",
            "temp[0] = 40; if globals::wants_fun(temp[0]) { self.n = 1; }",
        ),
    ));
    assert_eq!(
        &t[..3],
        &lines(&[
            "temp[0] := 40;",
            "temp[0] := temp[0];",
            "if call 285(temps) {"
        ])[..],
        "{t:#?}"
    );
    assert!(!t.iter().any(|l| l.contains("__arg")), "{t:#?}");
}

// ---------------------------------------------------------------------------------------
// `#[ret(temp = N)] -> Option<T>`
// ---------------------------------------------------------------------------------------

const GLOBALS: &str = "use std::globals::*;";

#[test]
fn option_answers_cost_what_the_hand_written_temp_read_costs() {
    // 282: `let Some(n) = .. else { .. };` is the bool test and the temp read, nothing more.
    same_iff(
        "282 let-else",
        GLOBALS,
        "extern mod legacy { #[id(282)] fn periods(period: i16) -> bool; }",
        "let Some(k) = globals::take_elapsed_periods(30) else { return true; }; self.n = k;",
        "if !legacy::periods(30) { return true; } self.n = temp[0];",
    );
    // 333: a companion id into the stack object.
    same_iff(
        "333 let-else",
        GLOBALS,
        "extern mod legacy { #[id(333)] fn companion() -> bool; }",
        "let Some(c) = globals::find_companion() else { return false; }; stack_obj = c; self.n = stack_obj.data[1];",
        "if !legacy::companion() { return false; } stack_obj = temp[0]; self.n = stack_obj.data[1];",
    );
    let (nodes, locals) = t_frame(&obj(
        GLOBALS,
        "",
        "let Some(k) = globals::take_elapsed_periods(30) else { return true; }; self.n = k;",
    ));
    assert_eq!((nodes, locals), (2, 0));
    let (nodes, locals) = t_frame(&obj(
        GLOBALS,
        "",
        "let Some(c) = globals::find_companion() else { return false; }; stack_obj = c; self.n = stack_obj.data[1];",
    ));
    assert_eq!((nodes, locals), (3, 0));
}

#[test]
fn option_calls_test_like_bool_and_unwrap_with_try() {
    let legacy = "extern mod legacy { #[id(282)] fn periods(period: i16) -> bool; }";
    // A bare test is the `Some` exit, and a statement ignores the answer.
    same_iff(
        "test and statement",
        GLOBALS,
        legacy,
        "if globals::take_elapsed_periods(12) { self.n = temp[0]; } globals::find_companion(); \
         if !globals::find_companion() { self.n = 3; }",
        "if legacy::periods(12) { self.n = temp[0]; } if globals::find_companion() {} \
         if !globals::find_companion() { self.n = 3; }",
    );
    // `?` turns `None` into the error exit, as on a `Result<T>`.
    same_iff(
        "try",
        GLOBALS,
        legacy,
        "let k = globals::take_elapsed_periods(12)?; self.n = k; globals::find_companion()?;",
        "if !legacy::periods(12) { panic!(); } self.n = temp[0]; globals::find_companion()?;",
    );
    // `let _ =` discards it like the statement.
    same_iff(
        "discard",
        GLOBALS,
        "",
        "let _ = globals::find_companion();",
        "globals::find_companion();",
    );
    // The binding keeps its type.
    same_iff(
        "annotated",
        GLOBALS,
        "",
        "let Some(c: ObjectId) = globals::find_companion() else { return false; }; stack_obj = c; self.n = stack_obj.data[1];",
        "let Some(c) = globals::find_companion() else { return false; }; stack_obj = c; self.n = stack_obj.data[1];",
    );
}

#[test]
fn option_trees_return_some_and_none() {
    // A local tree declares the ABI too: `Some(v)` stores the value and exits true, `None`
    // exits false.
    let items = "#[bhav] #[ret(temp = 2)] fn halve(x: i16) -> Option<i16> {
        if x < 2 { return None; }
        Some(x / 2)
    }";
    let a = air(
        "tree.antics",
        &obj(
            "",
            items,
            "let Some(h) = halve(self.n) else { return false; }; self.n = h;",
        ),
    );
    let halve_at = a.find("\"halve\"").expect("halve");
    let halve: Vec<&str> = a[halve_at..]
        .lines()
        .skip(1)
        .take_while(|l| *l != "}")
        .map(str::trim)
        .collect();
    assert_eq!(
        halve,
        [
            "if param[0] < 2 {",
            "return false;",
            "}",
            "temp[2] := param[0];",
            "temp[2] /= 2;",
            "return true;",
        ]
    );
    assert_eq!(
        tree_t(&a),
        lines(&[
            "temp[0] := my.attr[0];",
            "if !call 4098(temps) {",
            "return false;",
            "}",
            "my.attr[0] := temp[2];",
            "return true;",
        ])
    );
}

#[test]
fn a_bool_declaration_of_an_option_tree_still_merges() {
    // Existing `extern mod globals` blocks that declare 282 / 333 as `-> bool` keep compiling
    // next to the typed `std::globals`, and call the tree the same way.
    same_iff(
        "merge",
        GLOBALS,
        "extern mod globals { #[id(333)] fn has_companion() -> bool; #[id(282)] fn periods(period: i16) -> bool; }",
        "if globals::has_companion() { self.n = temp[0]; } if globals::periods(5) { self.n = 1; }",
        "if globals::find_companion() { self.n = temp[0]; } if globals::take_elapsed_periods(5) { self.n = 1; }",
    );
    // One name keeps one meaning.
    expect_err(
        "same_name.antics",
        &obj(
            GLOBALS,
            "extern mod globals { #[id(333)] fn find_companion() -> bool; }",
            "if globals::find_companion() { self.n = 1; }",
        ),
        "declares fn 'find_companion' twice, differently",
    );
}

#[test]
fn option_misuse_is_refused() {
    let bad = |name: &str, items: &str, body: &str, needle: &str| {
        expect_err(name, &obj(GLOBALS, items, body), needle);
    };
    bad(
        "value.antics",
        "",
        "let c = globals::find_companion(); stack_obj = c;",
        "returns an Option: bind its value with 'let Some(v) = .. else",
    );
    bad(
        "no_temp.antics",
        "extern mod m { #[id(300)] fn f() -> Option<i16>; }",
        "m::f();",
        "'Option<i16>' return requires the temporary that carries it",
    );
    bad(
        "bool_let_some.antics",
        "extern mod m { #[id(300)] fn f() -> bool; }",
        "let Some(x) = m::f() else { return false; }; self.n = x;",
        "needs a call returning 'Option<T>' with '#[ret(temp = N)]'",
    );
    bad(
        "wrong_type.antics",
        "",
        "let Some(c: bool) = globals::find_companion() else { return false; }; self.n = 1;",
        "does not match 'ObjectId' result",
    );
    bad(
        "else_falls_through.antics",
        "",
        "let Some(c) = globals::find_companion() else { self.n = 1; }; stack_obj = c;",
        "else-block must diverge",
    );
    bad(
        "missing_return.antics",
        "#[bhav] #[ret(temp = 0)] fn f(x: i16) -> Option<i16> { if x > 1 { return Some(x); } }",
        "",
        "must return on all control-flow paths",
    );
}

/// The lines of tree `name` in `src`'s `.air`, trimmed.
fn tree_named(src: &str, name: &str) -> Vec<String> {
    let text = air("tree.antics", src);
    let head = format!("\"{name}\" ");
    text.lines()
        .skip_while(|l| !(l.starts_with("tree ") && l.contains(&head)))
        .skip(1)
        .take_while(|l| *l != "}")
        .map(|l| l.trim().to_owned())
        .collect()
}

#[test]
fn objects_that_read_global_34_stage_in_the_copying_form() {
    // Every store is an Expression, which sets the scratch pointer a `global[34]` read sees:
    // the store of `temp[0]` into itself stays.
    let src = r#"object "Scratch" {
    guid: 0x7F40_02A1;
    #[bhav] fn sink(a: i16, b: i16) {}
    #[bhav] fn t() { sink(temp[0], global[34]); }
}"#;
    let t = tree_named(src, "t");
    assert_eq!(
        &t[..2],
        &lines(&["temp[0] := temp[0];", "temp[1] := global[34];"])[..],
        "{t:#?}"
    );
    // So does a tree of an object whose other tree reads it: a call carries the pointer.
    let src = r#"object "Scratch" {
    guid: 0x7F40_02A1;
    state n: i16;
    #[bhav] fn sink(a: i16) { self.n = global[34]; }
    #[bhav] fn t() { sink(temp[0]); }
}"#;
    let t = tree_named(src, "t");
    assert_eq!(t[0], "temp[0] := temp[0];", "{t:#?}");
}

#[test]
fn trees_that_keep_their_layout_stage_in_the_copying_form() {
    // A raw block's numbered branch targets count nodes, so the call's staging keeps the
    // copying form's node count: target 3 is still the call.
    let staged = |raw: &str| {
        let src = format!(
            r#"object "Layout" {{
    guid: 0x7F40_02A2;
    state n: i16;
    #[bhav] fn sink(x: i16, y: i16) {{}}
    #[bhav] fn t(x: i16) {{
        {raw}
        sink(temp[0], temp[1]);
        self.n = 10;
    }}
}}"#
        );
        tree_named(&src, "t")
    };
    let kept = staged("raw { random(temp[2], 1) -> true: 3, false: 3; }");
    assert_eq!(
        &kept[1..3],
        &lines(&["let __arg_0 = temp[0];", "let __arg_1 = temp[1];"])[..],
        "{kept:#?}"
    );
    // Without the raw block the first store into itself goes.
    let free = staged("");
    assert_eq!(
        &free[..2],
        &lines(&["temp[1] := temp[1];", "if call 4098(temps) {"])[..],
        "{free:#?}"
    );
}

#[test]
fn a_temp_loop_counter_staged_into_itself_is_still_written() {
    let src = r#"object "Loop" {
    guid: 0x7F40_02A3;
    state n: i16;
    #[bhav] fn rewind(#[temp(0)] counter: i16) { temp[0] = -1; }
    #[bhav] fn t() {
        #[temp(0)]
        for i in 0..3 {
            self.n = i;
            rewind(i);
        }
    }
}"#;
    expect_err("loop.antics", src, "writes its counter 'temp[0]'");
    // The same call in an inline helper's captured branch.
    let src = r#"object "Loop" {
    guid: 0x7F40_02A3;
    state n: i16;
    fn test(n: i16) -> bool { n < 2 }
    #[bhav] fn rewind(#[temp(0)] counter: i16) { temp[0] = -1; }
    #[bhav] fn t() {
        #[temp(0)]
        for i in 0..3 {
            if test(i) { rewind(i); }
        }
    }
}"#;
    expect_err("loop-inline.antics", src, "writes its counter 'temp[0]'");
}

#[test]
fn option_declarations_of_one_tree_must_agree_on_the_temp() {
    let src = r#"use std::globals::*;
extern mod globals {
    #[id(282)] #[ret(temp = 7)] fn elapsed(period: i16) -> Option<i16>;
}
object "Merge" {
    guid: 0x7F40_02A4;
    state n: i16;
    #[bhav] fn t() {
        let Some(n) = globals::elapsed(30) else { return false; };
        self.n = n;
    }
}"#;
    expect_err("merge.antics", src, "calls tree 282 two ways");
}

#[test]
fn flag_lists_share_the_nesting_limit() {
    let list = |n: usize| {
        let flags = vec!["ObjectFlags::Hidden"; n].join(", ");
        obj(
            "use std::flags::*;",
            "",
            &format!("self.flags.set_all([{flags}]);"),
        )
    };
    let _ = iff("ok.antics", &list(64));
    expect_err("long.antics", &list(65), "lists more than 64 flags");
}

#[test]
fn frames_whose_words_are_observable_stage_in_the_copying_form() {
    // Each object stages `sink(temp[0], temp[1])` in tree `t`; the copying form snapshots both.
    let copied = |name: &str, src: &str| {
        let t = tree_named(src, "t");
        assert!(
            t.iter().any(|l| l == "let __arg_0 = temp[0];"),
            "{name}: {t:#?}"
        );
    };
    let object = |items: &str, t: &str| {
        format!(
            r#"object "Frames" {{
    guid: 0x7F40_02A5;
    state n: i16;
    #[bhav] fn sink(a: i16, b: i16) {{}}
    {items}
    {t}
}}"#
        )
    };
    // An argless object tree may be the object's bottom frame, which Create Object writes word
    // 0 of: staged in place it would have no word.
    copied(
        "bottom frame",
        &object("", "#[bhav] fn t() { sink(temp[0], temp[1]); }"),
    );
    // An indexed tree is entered at a numbered node.
    copied(
        "kind",
        &object(
            "",
            "#[bhav(kind = 4)] fn t(entry: i16) { sink(temp[0], temp[1]); self.n = 1; }",
        ),
    );
    // Text that names frame words reads whichever local lives there.
    copied(
        "text",
        &object(
            "",
            r#"#[bhav] fn t(x: i16) {
        sink(temp[0], temp[1]);
        let shown = 7;
        dialog(kind: ok, title: "T", message: "Value: $Local:0");
        temp[2] = shown;
    }"#,
        ),
    );
    // Another tree reads a frame word a call may have left there.
    copied(
        "stale frame word",
        &object(
            "#[bhav(locals = 1)] fn read() { temp[7] = local[0]; }",
            "#[bhav] fn t(x: i16) { sink(temp[0], temp[1]); read(); }",
        ),
    );
    // None of these: in place.
    let src = object("", "#[bhav] fn t(x: i16) { sink(temp[0], temp[1]); }");
    let t = tree_named(&src, "t");
    assert!(!t.iter().any(|l| l.contains("__arg")), "{t:#?}");
}
