//! `.antics` v2.1 Rust-idiom polish: `panic!()` (the SimAntics error exit, its one spelling
//! since v2.2 step 2) and `let _ = expr;` (evaluate and discard). All sources are synthetic.

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

/// The `.air` of the object's tree `t` (from its header to the end).
fn tree_t(air: &str) -> &str {
    let at = air.find("\"t\" {").expect("tree t in .air");
    &air[at..]
}

/// A synthetic object: `items` as object members, then one Sim tree `t` running `body`.
fn sim_obj(uses: &str, items: &str, body: &str) -> String {
    format!(
        r#"{uses}
object "Polish" {{
    guid: 0x7F40_0280;
    state n: i16;
    {items}
    #[bhav] fn t(sim: Sim) {{ {body} }}
}}
"#
    )
}

/// Asserts that two bodies of `t` compile to the same `.iff` bytes.
fn same_iff(case: &str, uses: &str, items: &str, a: &str, b: &str) {
    let (sa, sb) = (sim_obj(uses, items, a), sim_obj(uses, items, b));
    let (ia, ib) = (iff("a.antics", &sa), iff("b.antics", &sb));
    assert!(
        ia == ib,
        "{case}: .iff bytes differ\n--- a\n{}\n--- b\n{}",
        tree_t(&air("a.antics", &sa)),
        tree_t(&air("b.antics", &sb)),
    );
}

mod panic_macro {
    use super::*;

    /// `body` builds, and tree `t` takes the error exit (`.air` `return error;`).
    fn exits(case: &str, body: &str) {
        let src = sim_obj("", "", body);
        iff("exit.antics", &src);
        let air = air("exit.antics", &src);
        assert!(tree_t(&air).contains("return error;"), "{case}\n{air}");
    }

    #[test]
    fn statement_and_tail_forms_are_the_error_exit() {
        exits("statement", "if self.n == 1 { panic!(); } self.n = 2;");
        same_iff(
            "block tail without ';'",
            "",
            "",
            "if self.n == 1 { panic!() } self.n = 2;",
            "if self.n == 1 { panic!(); } self.n = 2;",
        );
        exits("last statement of the tree", "self.n = 2; panic!();");
        exits(
            "after cleanups",
            "defer { self.n = 0; } if self.n == 1 { panic!(); } self.n = 2;",
        );
    }

    #[test]
    fn match_arms_are_the_error_exit() {
        same_iff(
            "value match arm",
            "",
            "",
            "let z = match self.n { 1 => 4, 2 => panic!(), _ => 5, }; self.n = z;",
            "let z = match self.n { 1 => 4, 2 => { panic!(); } _ => 5, }; self.n = z;",
        );
        exits(
            "value match wildcard arm",
            "let z = match self.n { 1 => 4, _ => panic!() }; self.n = z;",
        );
        same_iff(
            "statement match arm",
            "",
            "",
            "match self.n { 1 => panic!(), _ => self.n = 3, }",
            "match self.n { 1 => { panic!(); } _ => { self.n = 3; } }",
        );
        exits(
            "statement match block arm",
            "match self.n { 1 => { panic!() } _ => { self.n = 3; } }",
        );
    }

    #[test]
    fn diverging_value_branches_are_the_error_exit() {
        same_iff(
            "let-if else",
            "",
            "",
            "let y = if self.n == 1 { 2 } else { panic!() }; self.n = y;",
            "let y = if self.n == 1 { 2 } else { panic!(); }; self.n = y;",
        );
        exits(
            "let-if then",
            "let y = if self.n == 1 { panic!() } else { 3 }; self.n = y;",
        );
        exits(
            "nested diverging if",
            "let y = if self.n == 1 { 2 } else if self.n == 2 { 3 } else { panic!() }; self.n = y;",
        );
    }

    #[test]
    fn inline_helper_panic_exits_the_bhav() {
        same_iff(
            "helper",
            "",
            "fn check(v: i16) { if v == 1 { panic!(); } }",
            "check(self.n); self.n = 2;",
            "check(self.n); self.n = 2;",
        );
        let a = sim_obj(
            "",
            "fn check(v: i16) { if v == 1 { panic!(); } }",
            "check(self.n); self.n = 2;",
        );
        let air = air("a.antics", &a);
        assert!(tree_t(&air).contains("return error;"), "{air}");
    }

    /// `return error;` returns a binding named `error`; `panic!()` always errors.
    #[test]
    fn panic_ignores_a_binding_named_error() {
        let tree = |body: &str| {
            format!(
                r#"object "Polish" {{
    guid: 0x7F40_0281;
    #[bhav] #[ret(temp = 0)] fn t(sim: Sim) -> i16 {{ {body} }}
}}
"#
            )
        };
        let panics = air("p.antics", &tree("let error = 3; panic!();"));
        assert!(tree_t(&panics).contains("return error;"), "{panics}");
        let returns = air("r.antics", &tree("let error = 3; return error;"));
        assert!(!tree_t(&returns).contains("return error;"), "{returns}");
        assert_eq!(
            iff("p.antics", &tree("let error = 3; panic!();")),
            iff("e.antics", &tree("panic!();")),
        );
    }

    #[test]
    fn rejects_arguments_defer_and_value_position() {
        for args in ["1", "\"oops\"", "self.n"] {
            expect_err(
                "args.antics",
                &sim_obj("", "", &format!("panic!({args});")),
                "'panic!()' takes no arguments: the SimAntics error exit (253) carries no message",
            );
        }
        expect_err(
            "defer.antics",
            &sim_obj("", "", "defer { panic!(); } self.n = 1;"),
            "'panic!()' cannot exit out of a 'defer' block",
        );
        expect_err(
            "value.antics",
            &sim_obj("", "", "let x = panic!(); self.n = x;"),
            "'panic!()' is not a value",
        );
        expect_err(
            "missing semicolon.antics",
            &sim_obj("", "", "panic!() self.n = 1;"),
            "expected ';'",
        );
        // `return error;` is no error exit any more: with no `error` in scope it names
        // `panic!()`.
        expect_err(
            "return error.antics",
            &sim_obj("", "", "if self.n == 1 { return error; }"),
            "'return error;' is no longer the error exit; write 'panic!();'",
        );
    }

    #[test]
    fn fmt_prints_panic_tight() {
        let src = sim_obj(
            "",
            "",
            "if self.n == 1 { panic ! ( ) ; } let z = match self.n { 1 => 4, _ => panic !() }; let y = if z == 1 { 2 } else { panic!( ) }; self.n = y; let a = !(self.n == 1); self.n = if a { 1 } else { 0 };",
        );
        let out = format_v2_source(&src).expect("formats");
        assert!(out.contains("            panic!();\n"), "{out}");
        assert!(out.contains("            _ => panic!()\n"), "{out}");
        assert!(out.contains("            panic!()\n        };"), "{out}");
        assert_eq!(out.matches("panic!()").count(), 3, "{out}");
        assert!(out.contains("let a = !(self.n == 1);"), "{out}");
        assert!(!out.contains("panic !"), "{out}");
        assert!(is_v2_formatted(&out).expect("checks"), "{out}");
        assert_eq!(format_v2_source(&out).expect("formats"), out);
        assert_eq!(iff("src.antics", &src), iff("out.antics", &out));
    }
}

mod discard_let {
    use super::*;

    const EXTERN: &str = r#"extern mod g {
    #[id(258)] fn try_light(power: i16) -> Result;
}"#;

    const HELPERS: &str = "fn ok() -> bool { self.n == 1 }
    fn fallible() -> Result { if self.n == 2 { panic!(); } true }
    fn val() -> i16 { self.n = 3; 7 }
    fn noop() { self.n = 4; }
    #[bhav] fn b(sim: Sim) -> bool { self.n == 5 }";

    /// A bool, `Result` or predicate expression: the same nodes as `if expr {}`.
    #[test]
    fn bool_and_result_discards_match_empty_if() {
        for expr in [
            "ok()",
            "fallible()",
            "b(sim)",
            "g::try_light(3)",
            "globals::init_object()",
            "stack_obj.run_functional(4)",
            "self.n == 6",
            "!ok()",
        ] {
            same_iff(
                expr,
                &format!("use std::globals::*;\n{EXTERN}"),
                HELPERS,
                &format!("let _ = {expr}; self.n = 9;"),
                &format!("if {expr} {{}} self.n = 9;"),
            );
        }
    }

    /// A void or i16 call: the same nodes as the call statement.
    #[test]
    fn value_calls_match_call_statement() {
        for expr in [
            "noop()",
            "val()",
            "sleep(1)",
            "stack_obj.play_sound(3)",
            // Builtins whose statement differs from their condition: the statement wins.
            "animate(clip: 3)",
            "create_object(0x7F40_0280, mode: 0)",
            "find_best_interaction()",
        ] {
            same_iff(
                expr,
                "",
                HELPERS,
                &format!("let _ = {expr}; self.n = 9;"),
                &format!("{expr}; self.n = 9;"),
            );
        }
    }

    /// `let _ = expr?;`: the same nodes as the statement `expr?;` (the error exit still runs).
    #[test]
    fn try_discards_match_try_statement() {
        let ext = r#"extern mod g {
    #[id(258)] fn try_light(power: i16) -> Result;
    #[id(8192)] #[ret(temp = 1)] fn doubled(x: i16) -> Result<i16>;
}"#;
        for expr in [
            "g::try_light(3)?",
            "g::doubled(3)?",
            "(self.n == 1)?",
            "fallible()?",
        ] {
            same_iff(
                expr,
                ext,
                HELPERS,
                &format!("let _ = {expr}; self.n = 9;"),
                &format!("{expr}; self.n = 9;"),
            );
        }
    }

    /// A literal has nothing to evaluate: no node, no local.
    #[test]
    fn literals_emit_nothing() {
        for expr in ["1", "-3", "true"] {
            same_iff(
                expr,
                "",
                "",
                &format!("let _ = {expr}; self.n = 9;"),
                "self.n = 9;",
            );
        }
    }

    /// Any other value is exactly an unused `let y = expr;`: the optimizer keeps what it keeps
    /// for one, since even a node over locals moves the scratch pointer `global[34]` reads.
    #[test]
    fn values_match_an_unused_let() {
        for expr in ["x", "x + 1", "-x", "x * 3 - 1", "x / 2"] {
            same_iff(
                expr,
                "",
                "",
                &format!("let x = temp[0]; let _ = {expr}; self.n = x;"),
                &format!("let x = temp[0]; let y = {expr}; self.n = x;"),
            );
        }
    }

    /// A cell read (it may fault, or wake a VM wait on the word) or a division (it may trap)
    /// is evaluated exactly as `let x = expr;` with `x` unused.
    #[test]
    fn observable_values_are_evaluated() {
        for (expr, want) in [
            ("self.in_use + 1", "in_use"),
            ("stack_obj.attr[0]", "stack.attr[0]"),
            ("7 / self.n", "/="),
            ("temp[2] * 3", "temp[2]"),
            // A tunable divisor is read at runtime, whatever its default.
            ("10 / divisor", "/="),
            ("10 % divisor", "%="),
            ("divisor + 1", "bcon.divisor"),
        ] {
            let items = "param divisor: i16 = 2;";
            let a = air(
                "eval.antics",
                &sim_obj("", items, &format!("let _ = {expr};")),
            );
            let t = tree_t(&a);
            assert!(t.contains(want), "{expr}: missing `{want}` in:\n{t}");
            let named = air(
                "named.antics",
                &sim_obj("", items, &format!("let y = {expr};")),
            );
            assert_eq!(
                t.replace("__v___", "__v_y_"),
                tree_t(&named),
                "{expr}: differs from a named let"
            );
        }
    }

    /// A call argument the statement form specializes (a value `if`) works the same way.
    #[test]
    fn call_discards_are_specialized_like_statements() {
        same_iff(
            "create_object(if ..)",
            "",
            "",
            "let _ = create_object(if temp[0] == 0 { 0x7F40_0280 } else { 0x7F40_0281 }, mode: 0);",
            "create_object(if temp[0] == 0 { 0x7F40_0280 } else { 0x7F40_0281 }, mode: 0);",
        );
    }

    /// `let _` declares no local, so a region a `const` copies may hold one even where the
    /// tree formats `$Local:N`.
    #[test]
    fn discard_in_a_copied_const_region() {
        let src = r#"object "Polish" {
    guid: 0x7F40_0282;
    #[bhav] fn t() {
        let shown = 7;
        const K = if temp[0] == 0 { 1 } else { 2 };
        let _ = 0;
        temp[1] = K;
        dialog(kind: ok, title: "T", message: "Value: $Local:0");
    }
}
"#;
        iff("const.antics", src);
    }

    /// A trial lowering keeps the diagnostic positions of the statements before it.
    #[test]
    fn diagnostics_point_at_the_right_statement() {
        let src = |tail: &str| {
            format!(
                r#"object "Polish" {{
    guid: 0x7F40_0283;
    sound ding = preset("ui_click");
    #[bhav(context = inherit)]
    fn t() {{
        play_sound(ding);
        {tail}
    }}
}}
"#
            )
        };
        let at = |tail: &str| {
            compile_v2_source("inherit.antics", &src(tail), &PackResolverConfig::default())
                .expect_err("an inherit tree using the caller's context")
                .to_string()
        };
        assert_eq!(at("let _ = true;"), at("if true {}"));
        assert!(
            at("let _ = true;").starts_with("6:9:"),
            "{}",
            at("let _ = true;")
        );
    }

    #[test]
    fn discards_allocate_no_local() {
        let src = sim_obj(
            &format!("use std::globals::*;\n{EXTERN}"),
            HELPERS,
            "let _ = ok(); let _ = fallible(); let _ = b(sim); let _ = g::try_light(3); \
             let _ = stack_obj.run_functional(4); let _ = noop(); let _ = 1; \
             let _ = 2; let _ = sim;",
        );
        let air = air("noloc.antics", &src);
        let t = tree_t(&air);
        assert!(!t.contains("__v__"), "{t}");
        assert!(!t.contains("let "), "{t}");
    }

    /// Operands that make calls still run.
    #[test]
    fn value_with_calls_runs_them() {
        let src = sim_obj(
            "",
            "fn bump() -> i16 { temp[3] = 11; 7 }",
            "let _ = bump() + bump(); self.n = 9;",
        );
        let a = air("calls.antics", &src);
        assert_eq!(tree_t(&a).matches("temp[3] := 11;").count(), 2, "{a}");
        let src = sim_obj(
            "",
            "fn bump() -> i16 { temp[3] = 11; 7 }",
            "let _ = if self.n == 1 { bump() } else { 2 }; self.n = 9;",
        );
        let a = air("if.antics", &src);
        assert_eq!(tree_t(&a).matches("temp[3] := 11;").count(), 1, "{a}");
    }

    /// `let _ = f()?;` on a `Result<bool>` does not test the payload: it is `f()?;`.
    #[test]
    fn try_on_result_bool_does_not_test_the_payload() {
        let items = "#[bhav] #[ret(temp = 1)] fn flag(sim: Sim) -> Result<bool> { temp[1] == 2 }";
        same_iff(
            "flag(sim)?",
            "",
            items,
            "let _ = flag(sim)?; self.n = 9;",
            "flag(sim)?; self.n = 9;",
        );
    }

    /// A helper named like a statement builtin: `let _` runs what the statement runs.
    #[test]
    fn helper_named_like_a_builtin_follows_the_statement() {
        same_iff(
            "sleep helper",
            "",
            "fn sleep(n: i16) -> bool { self.n = n; true }",
            "let _ = sleep(1); temp[0] = 9;",
            "sleep(1); temp[0] = 9;",
        );
    }

    /// `let _ = animate(..);` is the statement (it waits for the clip); a parenthesized
    /// `(animate(..))` is the one-node value, as the statement `(animate(..));` is.
    #[test]
    fn animate_discards_follow_the_statement() {
        same_iff(
            "animate",
            "",
            "",
            "let _ = animate(clip: 3); temp[0] = 9;",
            "animate(clip: 3); temp[0] = 9;",
        );
        same_iff(
            "(animate)",
            "",
            "",
            "let _ = (animate(clip: 3)); temp[0] = 9;",
            "(animate(clip: 3)); temp[0] = 9;",
        );
        let waits = air("w.antics", &sim_obj("", "", "let _ = animate(clip: 3);"));
        assert!(tree_t(&waits).contains("while !animate("), "{waits}");
    }

    /// Nested helpers that discard each other are lowered once per level.
    #[test]
    fn nested_helper_discards_lower_once() {
        let mut items = String::new();
        for i in 0..15 {
            items.push_str(&format!("fn f{i}() {{ let _ = f{}(); }}\n    ", i + 1));
        }
        items.push_str("fn f15() { temp[0] = 1; }");
        // Lowered once per level (a trial per level would double the work at each one).
        same_iff("chain", "", &items, "let _ = f0();", "f0();");
    }

    #[test]
    fn underscore_binds_nothing() {
        expect_err(
            "mut.antics",
            &sim_obj("", "", "let mut _ = 1;"),
            "'let mut _' has nothing to mutate",
        );
        expect_err(
            "typed.antics",
            &sim_obj("", "", "let _: i16 = 1;"),
            "'let _' takes no type",
        );
        expect_err(
            "uninit.antics",
            &sim_obj("", "", "let _;"),
            "'let _' needs a value to discard",
        );
        expect_err(
            "slot.antics",
            &sim_obj("", "", "#[slot(0)] let _ = 1;"),
            "'let _' binds no variable, so it has no slot to place",
        );
        // `for _ in` still binds nothing a body can read.
        iff(
            "for.antics",
            &sim_obj("", "", "for _ in 0..2 { self.n += 1; }"),
        );
        for read in [
            "for _ in 0..2 { self.n = _; }",
            "let _ = 1; self.n = _;",
            "let _ = ok(); if _ { self.n = 1; }",
            "let _ = val() + 1; self.n = _ + 1;",
        ] {
            expect_err(
                "read.antics",
                &sim_obj("", HELPERS, read),
                "'_' is not a value",
            );
        }
    }

    #[test]
    fn errors_match_the_plain_forms() {
        expect_err(
            "unknown fn.antics",
            &sim_obj("", "", "let _ = no_such_fn();"),
            "unknown function 'no_such_fn'",
        );
        expect_err(
            "unknown name.antics",
            &sim_obj("", "", "let _ = no_such_name;"),
            "unknown identifier 'no_such_name'",
        );
        expect_err(
            "unknown name in call.antics",
            &sim_obj(EXTERN, "", "let _ = g::try_light(no_such_name);"),
            "no_such_name",
        );
    }

    #[test]
    fn named_lets_are_unchanged() {
        same_iff(
            "named",
            "",
            HELPERS,
            "let x = val(); let _ = ok(); self.n = x;",
            "let x = val(); if ok() {} self.n = x;",
        );
        let src = sim_obj("", "", "let mut x = 1; x += 1; self.n = x;");
        iff("mut.antics", &src);
    }
}
