//! `.antics` v2.3 Step 1: `if let Some(v) = f() { .. } else { .. }` on `#[ret(temp = N)] ->
//! Option<T>` trees, flag methods on object-loop handles, and three lowering fixes from PR #469
//! (the constant leaf of a short-circuit result, a functional calling a Sim tree, and raw `if`
//! arms that end alike). All sources are synthetic.

use sims_antics::v2::lint::lint_v2_source;
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

/// The `.air` body of tree `name`, its lines trimmed, without the header and closing brace.
fn tree_named(air: &str, name: &str) -> Vec<String> {
    let at = air
        .find(&format!("\"{name}\""))
        .unwrap_or_else(|| panic!("tree {name} in .air:\n{air}"));
    air[at..]
        .lines()
        .skip(1)
        .take_while(|l| *l != "}")
        .map(|l| l.trim().to_owned())
        .collect()
}

fn lines(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

/// A local `#[ret(temp = 2)] -> Option<i16>` tree.
const HALVE: &str = "#[bhav] #[ret(temp = 2)] fn halve(x: i16) -> Option<i16> {
        if x < 2 { return None; }
        Some(x / 2)
    }";

/// A synthetic object: `items` as object members, then one object-context tree `t` running
/// `body` (no Sim, so no entry save of the stack object).
fn obj(uses: &str, items: &str, body: &str) -> String {
    format!(
        r#"{uses}
object "V2.3 Step One" {{
    guid: 0x7F40_02C0;
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
        tree_named(&air("a.antics", &sa), "t"),
        tree_named(&air("b.antics", &sb), "t"),
    );
}

const GLOBALS: &str = "use std::globals::*;";

mod if_let {
    use super::*;

    #[test]
    fn costs_what_the_hand_written_test_costs() {
        // The call's true exit runs the then block with `k` read from the temp.
        same_iff(
            "282",
            GLOBALS,
            "extern mod legacy { #[id(282)] fn periods(period: i16) -> bool; }",
            "if let Some(k) = globals::take_elapsed_periods(30) { self.n = k; } else { self.n = 1; }",
            "if legacy::periods(30) { self.n = temp[0]; } else { self.n = 1; }",
        );
        // With a diverging else, it is `let Some(..) = .. else { .. };` node for node.
        same_iff(
            "let-else",
            "",
            HALVE,
            "if let Some(h) = halve(self.n) { self.n = h; } else { return true; }",
            "let Some(h) = halve(self.n) else { return true; }; self.n = h;",
        );
        // No else block.
        same_iff(
            "no else",
            "",
            HALVE,
            "if let Some(h) = halve(self.n) { self.n = h; } self.n += 1;",
            "if halve(self.n) { self.n = temp[2]; } self.n += 1;",
        );
    }

    #[test]
    fn else_if_chains_nest() {
        same_iff(
            "chain",
            GLOBALS,
            HALVE,
            "if let Some(k) = halve(self.n) { self.n = k; } \
             else if let Some(c: ObjectId) = globals::find_companion() { stack_obj = c; } \
             else if self.n > 3 { self.n = 0; } else { self.n = 2; }",
            "if halve(self.n) { self.n = temp[2]; } \
             else if globals::find_companion() { stack_obj = temp[0]; } \
             else if self.n > 3 { self.n = 0; } else { self.n = 2; }",
        );
    }

    #[test]
    fn the_binding_is_scoped_to_the_then_block() {
        let bad = |name: &str, body: &str| {
            expect_err(name, &obj("", HALVE, body), "unknown identifier 'k'");
        };
        bad(
            "after.antics",
            "if let Some(k) = halve(self.n) { self.n = k; } self.n = k;",
        );
        bad(
            "in_else.antics",
            "if let Some(k) = halve(self.n) { self.n = k; } else { self.n = k; }",
        );
        // An outer `k` is shadowed only inside the then block.
        let a = air(
            "shadow.antics",
            &obj(
                "",
                HALVE,
                "let k = 5; if let Some(k) = halve(self.n) { self.n = k; } self.n = k;",
            ),
        );
        assert_eq!(
            tree_named(&a, "t"),
            lines(&[
                "let __v_k_0 = 5;",
                "temp[0] := my.attr[0];",
                "if call 4098(temps) {",
                "my.attr[0] := temp[2];",
                "}",
                "my.attr[0] := __v_k_0;",
                "return true;",
            ])
        );
    }

    #[test]
    fn values_and_tails() {
        // A tail `if let` is the value of a helper or a `-> bool` tree.
        let src = format!(
            r#"object "If Let Tails" {{
    guid: 0x7F40_02C1;
    state n: i16;
    {HALVE}
    fn h(x: i16) -> i16 {{ if let Some(k) = halve(x) {{ k + 1 }} else {{ 0 }} }}
    #[bhav] fn a() -> bool {{ if let Some(k) = halve(self.n) {{ k > 2 }} else {{ false }} }}
    #[bhav] fn b() {{ self.n = h(self.n); }}
}}
"#
        );
        let a = air("tails.antics", &src);
        assert_eq!(
            tree_named(&a, "b"),
            lines(&[
                "let __arg_x_0;",
                "temp[0] := my.attr[0];",
                "if call 4098(temps) {",
                "__arg_x_0 := temp[2];",
                "__arg_x_0 += 1;",
                "} else {",
                "__arg_x_0 := 0;",
                "}",
                "my.attr[0] := __arg_x_0;",
                "return true;",
            ])
        );
        iff("tails.antics", &src);
    }

    #[test]
    fn other_scrutinees_are_refused() {
        let bad = |name: &str, items: &str, body: &str, needle: &str| {
            expect_err(name, &obj(GLOBALS, items, body), needle);
        };
        let tree_needed = "needs a call to a tree returning 'Option<T>' with '#[ret(temp = N)]'";
        bad(
            "bool.antics",
            "extern mod m { #[id(300)] fn f() -> bool; }",
            "if let Some(x) = m::f() { self.n = x; }",
            "'if let Some(x) = ..' needs a call returning 'Option<T>' with '#[ret(temp = N)]', but m::f does not",
        );
        bad(
            "value.antics",
            "",
            "if let Some(v) = self.n { self.n = v; }",
            tree_needed,
        );
        bad(
            "run_functional.antics",
            "",
            "if let Some(v) = stack_obj.run_functional(Functional::Clean) { self.n = v; }",
            tree_needed,
        );
        bad(
            "find_best_object.antics",
            "",
            "if let Some(o) = find_best_object(\"x\") { stack_obj = o; }",
            tree_needed,
        );
        bad(
            "pattern.antics",
            "",
            "if let v = globals::find_companion() { }",
            "'if let' takes only the pattern 'Some(name)'",
        );
        bad(
            "none.antics",
            "",
            "if let None = globals::find_companion() { }",
            "'if let' takes only the pattern 'Some(name)'",
        );
        bad(
            "wrong_type.antics",
            "",
            "if let Some(c: bool) = globals::find_companion() { self.n = 1; }",
            "'if let Some(c: bool)' does not match 'ObjectId' result",
        );
    }

    #[test]
    fn formats_and_lints() {
        let src = obj(
            GLOBALS,
            HALVE,
            "if let Some(k) = halve(self.n) { self.n = k; } else if let Some(c: ObjectId) = globals::find_companion() { stack_obj = c; } else { self.n = 2; }",
        );
        let formatted = format_v2_source(&src).expect("format");
        assert!(
            formatted.contains(
                "        if let Some(k) = halve(self.n) {\n            self.n = k;\n        } else if let Some(c: ObjectId) = globals::find_companion() {\n            stack_obj = c;\n        } else {\n            self.n = 2;\n        }\n"
            ),
            "{formatted}"
        );
        assert!(is_v2_formatted(&formatted).expect("check"), "{formatted}");
        assert_eq!(format_v2_source(&formatted).expect("format"), formatted);
        assert_eq!(iff("a.antics", &src), iff("b.antics", &formatted));
        let findings = lint_v2_source(&formatted, "fmt.antics").expect("lint");
        assert!(findings.is_empty(), "{findings:#?}");
    }
}

mod loop_handle_flag_methods {
    use super::*;

    const FLAGS: &str = "use std::flags::*;";

    /// The method forms on an object-loop handle build the operator forms' bytes (the direct
    /// cursor lowering, no stack-object swaps).
    #[test]
    fn methods_build_the_operators_bytes() {
        let (f, h) = ("ObjectFlags2::CatchesFire", "ObjectFlags2::LightSource");
        let cases = [
            (
                format!("o.flags2.set({f});"),
                format!("o.flags2 flag_set {f};"),
            ),
            (
                format!("o.flags2.clear({f});"),
                format!("o.flags2 flag_clear {f};"),
            ),
            (
                format!("if o.flags2.contains({f}) {{ o.data[1] = 1; }}"),
                format!("if o.flags2 flag_test {f} {{ o.data[1] = 1; }}"),
            ),
            (
                format!("if !o.flags2.contains({f}) {{ break; }}"),
                format!("if !(o.flags2 flag_test {f}) {{ break; }}"),
            ),
            (
                format!("o.flags2.set_all([{f}, {h}]);"),
                format!("o.flags2.set({f} | {h});"),
            ),
            (
                format!("o.flags2.clear_all([{f}, {h}]);"),
                format!("o.flags2.clear({f} | {h});"),
            ),
            (
                format!("if o.flags2.contains_any([{f}, {h}]) {{ o.data[1] = 1; }}"),
                format!("if o.flags2.intersects({f} | {h}) {{ o.data[1] = 1; }}"),
            ),
        ];
        for (method, operator) in cases {
            let wrap = |b: &str| format!("for o in lot.objects() {{ {b} }}");
            same_iff(&method, FLAGS, "", &wrap(&method), &wrap(&operator));
        }
        let a = air(
            "direct.antics",
            &obj(
                FLAGS,
                "",
                "for o in lot.objects() { o.flags2.set(ObjectFlags2::CatchesFire); }",
            ),
        );
        let t = tree_named(&a, "t");
        assert!(!t.iter().any(|l| l.contains("saved_stack_obj")), "{t:#?}");
    }
}

mod short_circuit_constant_leaf {
    use super::*;

    /// A constant that decides a returned `&&`/`||` leaves its result through a test node, so
    /// the tree's last result is that constant, not the previous test's.
    #[test]
    fn a_constant_leaf_sets_the_result() {
        let src = r#"object "Constant Leaf" {
    guid: 0x7F40_02C2;
    fn h1() -> bool { return temp[0] == 1 && false; }
    #[bhav] fn a() -> bool { return temp[0] == 1 && false; }
    #[bhav] fn b() -> bool { return temp[0] == 1 || true; }
    #[bhav] fn e() -> bool { return temp[0] == 1 && true; }
    #[bhav] fn f() -> bool { return h1(); }
    #[bhav] fn n() -> bool { if h1() { temp[2] = 1; } return true; }
}
"#;
        let a = air("leaf.antics", src);
        assert_eq!(
            tree_named(&a, "a"),
            lines(&[
                "if !temp[0] == 1 {",
                "return false;",
                "}",
                "if 1 == 0 {",
                "}",
                "return false;",
            ])
        );
        assert_eq!(
            tree_named(&a, "b"),
            lines(&[
                "if temp[0] == 1 {",
                "return true;",
                "}",
                "if 1 == 1 {",
                "}",
                "return true;",
            ])
        );
        // A constant that matches the test before it needs no node.
        assert_eq!(
            tree_named(&a, "e"),
            lines(&["if !temp[0] == 1 {", "return false;", "}", "return true;"])
        );
        assert_eq!(
            tree_named(&a, "f"),
            lines(&[
                "if temp[0] == 1 {",
                "if 1 == 0 {",
                "}",
                "return false;",
                "}",
                "return false;",
            ])
        );
        // A test whose arm does not exit with the result keeps no extra node.
        assert_eq!(
            tree_named(&a, "n"),
            lines(&["if temp[0] == 1 {", "}", "return true;"])
        );
        iff("leaf.antics", src);
    }
}

mod functional_calls_a_sim_tree {
    use super::*;

    fn object(members: &str) -> String {
        format!("object \"Functional Sim\" {{\n    guid: 0x7F40_02C3;\n{members}\n}}\n")
    }

    /// `run_functional` and Find Best Object run a functional's action and guard on whichever
    /// object asks, so neither may call a Sim tree.
    #[test]
    fn is_refused() {
        let sim_tree = "    #[bhav] fn needs_sim(sim: Sim) -> bool { sim.fame > 0 }\n";
        for functional in [
            "functional cook { run(sim) { needs_sim(sim); } }",
            "functional cook { guard(sim) { needs_sim(sim) } run(sim) { } }",
        ] {
            expect_err(
                "functional.antics",
                &object(&format!("{sim_tree}    {functional}")),
                "a functional's action or guard may run on an object",
            );
        }
        // A helper is fine.
        iff(
            "helper.antics",
            &object(
                "    state n: i16;\n    fn bump() { self.n += 1; }\n    \
                 functional cook { run(sim) { bump(); } }",
            ),
        );
    }
}

mod raw_arms_that_end_alike {
    use super::*;

    /// A raw `if` whose arms end with the same instruction keeps both: the optimizer's tail
    /// merge would otherwise move it after the `if`, changing the layout the raw block wrote.
    #[test]
    fn keep_their_layout() {
        let src = r#"object "Raw Tails" {
    guid: 0x7F40_02C4;
    #[bhav] fn t() {
        temp[3] = 4;
        raw {
            if temp[0] == 1 {
                temp[1] := 1;
                temp[2] := 3;
            } else {
                temp[1] := 2;
                temp[2] := 3;
            }
        }
    }
}
"#;
        let t = tree_named(&air("raw.antics", src), "t");
        assert_eq!(
            t.iter().filter(|l| *l == "temp[2] := 3;").count(),
            2,
            "{t:#?}"
        );
        iff("raw.antics", src);
    }
}

/// Regressions from the first review round.
mod review_round_1 {
    use super::*;

    /// A raw arm ending in a loop the backend unwraps would end like the other arm after all.
    #[test]
    fn a_raw_arm_ending_in_a_loop_keeps_its_layout() {
        let src = r#"object "Raw Loop Tail" {
    guid: 0x7F40_02C5;
    #[bhav] fn t() {
        raw {
            if temp[0] == 1 {
                temp[1] := 1;
                while true {
                    temp[2] := 3;
                    break;
                }
            } else {
                temp[1] := 2;
                temp[2] := 3;
            }
        }
    }
}
"#;
        let t = tree_named(&air("raw_loop.antics", src), "t");
        assert_eq!(
            t.iter().filter(|l| *l == "temp[2] := 3;").count(),
            2,
            "{t:#?}"
        );
        iff("raw_loop.antics", src);
    }

    /// A variable named `let` is still a condition.
    #[test]
    fn if_on_a_variable_named_let() {
        same_iff(
            "let variable",
            "",
            "",
            "let let = temp[0] == 1; if let { temp[1] = 2; }",
            "let x = temp[0] == 1; if x { temp[1] = 2; }",
        );
    }
}
