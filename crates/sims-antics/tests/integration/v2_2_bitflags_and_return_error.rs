//! `.antics` v2.2 step 2: `wall_placement` (`std::flags::WallPlacement`) and `world.expansions`
//! (`std::flags::Expansions`) are lenient engine `bitflags` words, and `panic!()` is the one
//! spelling of the error exit (`return error;` returns a value named `error`).
//! All sources are synthetic.

use sims_antics::v2::lint::lint_v2_source;
use sims_antics::v2::{DecompileV2Options, TreeCheck, decompile_iff_to_v2_report};
use sims_antics::{PackResolverConfig, compile_v2_source};
use sims_formats::iff::Iff;

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

const USES: &str = "use std::flags::*;";

/// A synthetic object: `items` as object members, then one object-context tree `t`.
fn obj(items: &str, body: &str) -> String {
    format!(
        r#"{USES}
object "Step Two" {{
    guid: 0x7F40_02B0;
    state n: i16;
    {items}
    #[bhav] fn t() {{ {body} }}
}}
"#
    )
}

/// The same object with a Sim tree `t(sim: Sim)`.
fn sim_obj(body: &str) -> String {
    format!(
        r#"{USES}
object "Step Two" {{
    guid: 0x7F40_02B0;
    state n: i16;
    #[bhav] fn t(sim: Sim) {{ {body} }}
}}
"#
    )
}

/// Asserts that two sources compile to the same `.iff` bytes.
fn same_bytes(case: &str, a: &str, b: &str) {
    assert!(
        iff("a.antics", a) == iff("b.antics", b),
        "{case}: .iff bytes differ\n--- a\n{:#?}\n--- b\n{:#?}",
        tree_t(&air("a.antics", a)),
        tree_t(&air("b.antics", b)),
    );
}

fn same_iff(case: &str, a: &str, b: &str) {
    same_bytes(case, &obj("", a), &obj("", b));
}

mod wall_placement {
    use super::*;

    #[test]
    fn methods_build_the_raw_operators_bytes() {
        for (method, raw) in [
            (
                "self.wall_placement.set(WallPlacement::NoWallLeft);",
                "self.wall_placement flag_set 12;",
            ),
            (
                "self.wall_placement.clear(WallPlacement::NoCorner);",
                "self.wall_placement flag_clear 5;",
            ),
            (
                "if self.wall_placement.contains(WallPlacement::NeedsCorner) { self.n = 1; }",
                "if self.wall_placement flag_test 6 { self.n = 1; }",
            ),
            (
                "if !self.wall_placement.contains(WallPlacement::NoWallFront) { self.n = 1; }",
                "if !(self.wall_placement flag_test 9) { self.n = 1; }",
            ),
            (
                "stack_obj.wall_placement.set(WallPlacement::NoWallRight);",
                "stack_obj.wall_placement flag_set 10;",
            ),
            // A typed constant on the operator is the same bit.
            (
                "self.wall_placement.set(WallPlacement::NoWallBehind);",
                "self.wall_placement flag_set WallPlacement::NoWallBehind;",
            ),
        ] {
            same_iff(method, method, raw);
        }
    }

    #[test]
    fn flag_lists_build_the_union_bytes() {
        same_iff(
            "set_all",
            "self.wall_placement.set_all([WallPlacement::NoWallLeft, WallPlacement::NoWallRight]);",
            "self.wall_placement.set(WallPlacement::NoWallLeft | WallPlacement::NoWallRight);",
        );
        same_iff(
            "clear_all",
            "self.wall_placement.clear_all([WallPlacement::NoCorner, WallPlacement::NeedsCorner]);",
            "self.wall_placement.clear(WallPlacement::NoCorner | WallPlacement::NeedsCorner);",
        );
        same_iff(
            "contains_any",
            "if self.wall_placement.contains_any([WallPlacement::NoWallLeft, WallPlacement::NoWallRight]) { self.n = 1; }",
            "if self.wall_placement.intersects(WallPlacement::NoWallLeft | WallPlacement::NoWallRight) { self.n = 1; }",
        );
    }

    #[test]
    fn the_sim_and_handles_have_it_too() {
        same_bytes(
            "sim",
            &sim_obj("sim.wall_placement.set(WallPlacement::NoWallLeft);"),
            &sim_obj("sim.wall_placement flag_set 12;"),
        );
        same_iff(
            "handle local",
            "let h = stack_obj; if h.wall_placement.contains(WallPlacement::NoCorner) { self.n = 1; }",
            "let h = stack_obj; if h.wall_placement flag_test 5 { self.n = 1; }",
        );
    }

    /// Every numeric use it had as an `i16` still builds, with the bytes of the `i16` cell
    /// written by number (`self.data[13]`, which stays `i16`).
    #[test]
    fn keeps_every_i16_use() {
        let body = |w: &str| {
            format!(
                "{w} flag_set 12; if {w} == 0 {{ self.n = 1; }} {w} = 5; {w} += 1; \
                 self.n = {w}; self.n = {w} * 2 + 1; let x: i16 = {w}; self.n = x; \
                 random_into({w}, 6); if {w} > 3 {{ self.n = 2; }} \
                 match {w} {{ 1 | 2 => {{ self.n = 9; }} _ => {{}} }} \
                 for i in 0..{w} {{ self.n += i; }} temp[0] = {w}; \
                 {w} = self.n & 4;"
            )
        };
        same_iff(
            "i16 uses",
            &body("self.wall_placement"),
            &body("self.data[13]"),
        );
        // A local inferred from it is the lenient type, and still takes numbers.
        same_iff(
            "local",
            "let w = self.wall_placement; let v: i16 = w; w.set(WallPlacement::NoCorner); self.n = w + v;",
            "let w = self.data[13]; let v: i16 = w; w flag_set 5; self.n = w + v;",
        );
    }

    /// Like every typed engine word, it refuses a flag of another type.
    #[test]
    fn refuses_another_flag_type() {
        expect_err(
            "other type.antics",
            &obj("", "self.wall_placement flag_set WallAdjacency::WallFront;"),
            "type mismatch: expected bitflags 'WallPlacement', found 'WallAdjacency'",
        );
        expect_err(
            "other method.antics",
            &obj("", "self.wall_placement.set(ObjectFlags::InTheWay);"),
            "WallPlacement",
        );
        // Written by number, the word is an `i16`: no methods.
        expect_err(
            "by number.antics",
            &obj("", "self.data[13].set(WallPlacement::NoCorner);"),
            "requires a bitflags target, found 'i16'",
        );
    }
}

mod expansions {
    use super::*;

    #[test]
    fn methods_build_the_raw_operators_bytes() {
        for (method, raw) in [
            (
                "if world.expansions.contains(Expansions::Pets) { self.n = 1; }",
                "if world.expansions flag_test 6 { self.n = 1; }",
            ),
            (
                "if world.expansions.contains(Expansions::Magic) { self.n = 1; }",
                "if world.expansions flag_test Expansions::Magic { self.n = 1; }",
            ),
            (
                "if !world.expansions.contains(Expansions::Dating) { self.n = 1; }",
                "if !(world.expansions flag_test 3) { self.n = 1; }",
            ),
            // `global[20]` written out is the same word.
            (
                "if global[20].contains(Expansions::Fame) { self.n = 1; }",
                "if world.expansions flag_test 7 { self.n = 1; }",
            ),
            // Writable, as it was.
            (
                "world.expansions.set(Expansions::Vacations);",
                "world.expansions flag_set 4;",
            ),
            (
                "world.expansions.clear(Expansions::Vacations);",
                "world.expansions flag_clear 4;",
            ),
        ] {
            same_iff(method, method, raw);
        }
        same_iff(
            "contains_any",
            "if world.expansions.contains_any([Expansions::Pets, Expansions::Magic]) { self.n = 1; }",
            "if world.expansions.intersects(Expansions::Pets | Expansions::Magic) { self.n = 1; }",
        );
        same_iff(
            "contains_all",
            "if world.expansions.contains_all([Expansions::Pets, Expansions::Magic]) { self.n = 1; }",
            "if world.expansions.contains(Expansions::Pets | Expansions::Magic) { self.n = 1; }",
        );
    }

    #[test]
    fn keeps_every_i16_use() {
        let a = air(
            "compat.antics",
            &format!(
                r#"{USES}
object "Step Two" {{
    guid: 0x7F40_02B0;
    state n: i16;
    #[bhav] #[ret(temp = 0)] fn g(v: i16) -> i16 {{ return v; }}
    #[bhav] #[ret(temp = 0)] fn h() -> i16 {{ return world.expansions; }}
    #[bhav] fn t() {{
        if world.expansions == 0 {{ self.n = 1; }}
        if world.expansions > 3 {{ self.n = 2; }}
        self.n = world.expansions + 1;
        self.n = g(world.expansions);
        random_into(self.n, world.expansions);
        world.expansions = temp[0];
        global[20] = 7;
        temp[1] = world.expansions;
        match world.expansions {{ 0 => {{ self.n = 5; }} _ => {{}} }}
        self.n = WallPlacement::from_raw(world.expansions) as i16;
    }}
}}
"#
            ),
        );
        let t = tree_t(&a);
        for line in [
            "if global[20] == 0 {",
            "if global[20] > 3 {",
            "my.attr[0] += 1;",
            "temp[0] := global[20];",
            "global[20] := temp[0];",
            "global[20] := 7;",
            "temp[1] := global[20];",
        ] {
            assert!(t.iter().any(|l| l == line), "missing {line:?} in {t:#?}");
        }
    }

    #[test]
    fn refuses_another_flag_type() {
        expect_err(
            "other type.antics",
            &obj(
                "",
                "if world.expansions flag_test WallPlacement::NoCorner { self.n = 1; }",
            ),
            "type mismatch: expected bitflags 'Expansions', found 'WallPlacement'",
        );
        expect_err(
            "other method.antics",
            &obj("", "world.expansions.set(WallPlacement::NoCorner);"),
            "Expansions",
        );
    }
}

/// Former `i16` uses (Sol review round 1): each builds as it did with an untyped cell in the
/// word's place. `self.data[13]` stands for `wall_placement` (same bytes); `global[19]` for
/// `world.expansions` (the same `.air` once renumbered).
mod former_i16_uses {
    use super::*;

    const ITEMS: &str = "fn bump(x: &mut i16) { *x += 1; }\n    \
                         fn pass(v: i16) -> i16 { v }\n    \
                         fn pick(error: bool) -> bool { return error; }";

    fn same_as_untyped(case: &str, body: &str) {
        let wall = |w: &str| obj(ITEMS, &body.replace('W', w));
        same_bytes(case, &wall("self.wall_placement"), &wall("self.data[13]"));
        let typed = air("typed.antics", &wall("world.expansions"));
        let untyped =
            air("untyped.antics", &wall("global[19]")).replace("global[19]", "global[20]");
        assert_eq!(typed, untyped, "{case}");
    }

    #[test]
    fn compound_operators() {
        same_as_untyped("compound", "let w = W; w += 3; w -= self.n; self.n = w;");
        same_as_untyped("compound on the word", "W += W; W *= 2; W -= self.n;");
    }

    /// A runtime value of a typed flag word is a mask, never a bit number, and so is refused
    /// on `flag_set` / `flag_clear` (as for `flags2` and the other typed engine words).
    #[test]
    fn a_runtime_bit_number_from_the_words_is_refused() {
        for w in ["self.wall_placement", "world.expansions"] {
            expect_err(
                "index.antics",
                &obj("", &format!("temp[0] flag_set {w};")),
                "takes a single flag constant or an i16 bit number",
            );
            expect_err(
                "local.antics",
                &obj("", &format!("let w = {w}; temp[0] flag_clear w;")),
                "takes a single flag constant or an i16 bit number",
            );
            expect_err(
                "test.antics",
                &obj("", &format!("if temp[0] flag_test {w} {{ self.n = 1; }}")),
                "'flag_test' requires matching bitflags or i16 operands",
            );
        }
    }

    /// Value branches must agree on one type, as for `flags2`: a typed word beside an `i16`
    /// is refused (write `self.data[13]` / `global[20]` for the number).
    #[test]
    fn value_branches_with_an_i16_are_refused() {
        for w in ["self.wall_placement", "world.expansions"] {
            for body in [
                format!("let w = if self.n > 0 {{ {w} }} else {{ 0 }}; self.n = w;"),
                format!("let w = match self.n {{ 1 => {w}, _ => 0, }}; self.n = w;"),
            ] {
                expect_err("branch.antics", &obj("", &body), "have different types");
            }
        }
    }

    #[test]
    fn mut_i16_parameters_and_builtins() {
        same_as_untyped("&mut field", "bump(&mut W);");
        same_as_untyped("&mut local", "let w = W; bump(&mut w); self.n = w;");
        same_as_untyped("sprite value", "set_dynamic_sprite(index: 2, value: W);");
        same_as_untyped("cursor local", "let w = W; set_to_next(person, dest: w);");
        same_as_untyped("helper argument", "self.n = pass(W);");
        same_as_untyped(
            "relationship cell",
            "relationship(column: 0, dir: 0, flags: 0, value: W);",
        );
        same_as_untyped(
            "relationship local",
            "let w = W; relationship(column: 0, dir: 0, flags: 0, value: w); self.n = w;",
        );
    }

    #[test]
    fn a_numeric_receiver() {
        same_as_untyped("receiver", "let w = W; w.run_functional(4);");
    }

    /// The words keep the `ObjectId` and cross-type refusals every typed engine word has.
    #[test]
    fn object_ids_and_other_engine_words_do_not_mix() {
        for (w, other) in [
            ("self.wall_placement", "world.expansions"),
            ("world.expansions", "self.flags2"),
        ] {
            for (body, needle) in [
                (
                    format!("let o: ObjectId = {w}; stack_obj = o;"),
                    "cannot initialize 'ObjectId' variable",
                ),
                (
                    format!("{w} = stack_obj;"),
                    "cannot assign 'ObjectId' expression",
                ),
                (
                    format!("self.n = {w} & {other};"),
                    "mismatched operand types",
                ),
                (
                    format!("if {w} < {other} {{ self.n = 2; }}"),
                    "ordering comparison is not supported",
                ),
                (format!("{w} = {other};"), "cannot assign"),
            ] {
                expect_err("mix.antics", &obj("", &body), needle);
            }
        }
    }

    /// Arithmetic between two engine words builds as numbers, as with `flags2`.
    #[test]
    fn two_engine_words_in_arithmetic() {
        let body = "self.n = A + B; if A flag_test 3 { self.n = 3; }";
        let src = |a: &str, b: &str| obj("", &body.replace('A', a).replace('B', b));
        same_bytes(
            "two words",
            &src("self.wall_placement", "world.expansions"),
            &src("self.data[13]", "world.expansions"),
        );
    }

    /// `pick(error)` returns its parameter `error` with the lowering it had when
    /// `return error;` parsed as the error exit: no tail-condition shortcut, which the same
    /// helper takes under another parameter name.
    #[test]
    fn a_bound_return_error_keeps_its_lowering() {
        let body = "while pick(self.n == 0) { self.n = 2; }";
        let bound = obj(ITEMS, body);
        let renamed = obj(&ITEMS.replace("error", "e"), body);
        assert_ne!(
            iff("bound.antics", &bound),
            iff("renamed.antics", &renamed),
            "{}",
            air("bound.antics", &bound)
        );
    }
}

mod lint_and_lift {
    use super::*;

    fn method_suggestions(source: &str) -> Vec<String> {
        lint_v2_source(source, "probe.antics")
            .unwrap_or_else(|e| panic!("lint failed: {e}\n{source}"))
            .into_iter()
            .filter(|d| d.rule == "prefer_method_syntax")
            .filter_map(|d| d.suggestion)
            .collect()
    }

    #[test]
    fn lint_suggests_the_methods() {
        let source = obj(
            "",
            "self.wall_placement flag_set WallPlacement::NoWallLeft;\n        \
             if world.expansions flag_test Expansions::Pets { self.n = 1; }\n        \
             stack_obj.wall_placement flag_clear WallPlacement::NoCorner;",
        );
        let got = method_suggestions(&source);
        assert_eq!(
            got,
            [
                "self.wall_placement.set(WallPlacement::NoWallLeft)",
                "world.expansions.contains(Expansions::Pets)",
                "stack_obj.wall_placement.clear(WallPlacement::NoCorner)",
            ],
            "{source}"
        );
        // Each suggestion builds the same bytes.
        let fixed = source
            .replace(
                "self.wall_placement flag_set WallPlacement::NoWallLeft",
                &got[0],
            )
            .replace("world.expansions flag_test Expansions::Pets", &got[1])
            .replace(
                "stack_obj.wall_placement flag_clear WallPlacement::NoCorner",
                &got[2],
            );
        same_bytes("lint fixes", &source, &fixed);
        // `global[20]` written out gets `untyped_engine_field` first, no method form.
        let source = obj(
            "",
            "if global[20] flag_test Expansions::Pets { self.n = 1; }",
        );
        assert!(method_suggestions(&source).is_empty(), "{source}");
        // Nor on an object-loop handle: the method form there leaves the direct cursor
        // lowering (a pre-existing gap for every typed word), so the bytes would differ.
        let source = obj(
            "",
            "for o in lot.objects() { o.wall_placement flag_set WallPlacement::NoCorner; }",
        );
        assert!(method_suggestions(&source).is_empty(), "{source}");
    }

    #[test]
    fn lift_writes_the_methods() {
        let source = r#"use std::flags::*;
object "Fable Step Two" {
    guid: 0x7E00_0190;
    #[bhav(id = 4300, locals = 0)]
    fn t() {
        self.wall_placement flag_set 12;
        if world.expansions flag_test 6 {
            temp[0] = 1;
        }
        stack_obj.wall_placement flag_clear 5;
    }
}"#;
        for behavioral in [false, true] {
            let options = DecompileV2Options {
                behavioral,
                ..DecompileV2Options::default()
            };
            let parsed = Iff::parse(iff("lift.antics", source)).expect("parse IFF");
            let report = decompile_iff_to_v2_report(&parsed, &options).expect("lift");
            assert!(report.compiled, "{}", report.source);
            assert!(
                report
                    .trees
                    .iter()
                    .all(|t| matches!(t.check, TreeCheck::Identical | TreeCheck::Equivalent)),
                "{:?}\n{}",
                report.trees,
                report.source
            );
            for want in [
                "self.wall_placement.set(WallPlacement::NoWallLeft);",
                "world.expansions.contains(Expansions::Pets)",
                "stack_obj.wall_placement.clear(WallPlacement::NoCorner);",
            ] {
                assert!(
                    report.source.contains(want),
                    "missing {want:?} in:\n{}",
                    report.source
                );
            }
        }
    }
}

mod return_error {
    use super::*;

    const HINT: &str = "'return error;' is no longer the error exit; write 'panic!();'";

    #[test]
    fn without_a_binding_names_panic() {
        expect_err(
            "statement.antics",
            &obj("", "if self.n == 1 { return error; } self.n = 2;"),
            HINT,
        );
        expect_err(
            "match arm.antics",
            &obj("", "match self.n { 1 => return error, _ => self.n = 3, }"),
            HINT,
        );
        expect_err(
            "bool tree.antics",
            r#"object "Step Two" {
    guid: 0x7F40_02B0;
    #[bhav] fn t() -> bool { if temp[0] == 1 { return error; } return true; }
}
"#,
            HINT,
        );
        expect_err(
            "inline helper.antics",
            &obj(
                "fn check(v: i16) { if v == 1 { return error; } }",
                "check(self.n);",
            ),
            HINT,
        );
    }

    /// `let error = true; return error;` returns the bool, through the true exit.
    #[test]
    fn a_binding_named_error_is_returned() {
        let tree = |body: &str| {
            format!(
                r#"object "Step Two" {{
    guid: 0x7F40_02B0;
    #[bhav] fn t() -> bool {{ {body} }}
}}
"#
            )
        };
        let a = air("bound.antics", &tree("let error = true; return error;"));
        let t = tree_t(&a);
        assert!(!t.iter().any(|l| l == "return error;"), "{t:#?}");
        same_bytes(
            "bound bool",
            &tree("let error = true; return error;"),
            &tree("let e = true; return e;"),
        );
        // A parameter named `error`, and an `i16` value.
        let a = air(
            "param.antics",
            r#"object "Step Two" {
    guid: 0x7F40_02B0;
    #[bhav] fn t(error: bool) -> bool { return error; }
    #[bhav] #[ret(temp = 0)] fn u() -> i16 { let error = 7; return error; }
}
"#,
        );
        assert!(!a.contains("return error;"), "{a}");
    }

    /// A `const error` alone never made `return error;` a return: it was the error exit.
    #[test]
    fn a_const_named_error_is_refused() {
        expect_err(
            "const.antics",
            r#"object "Step Two" {
    guid: 0x7F40_02B0;
    const error = 7;
    #[bhav] #[ret(temp = 0)] fn t() -> i16 { return error; }
}
"#,
            "'return error;' names only the const 'error', and was the error exit before v2.2",
        );
        // Not through an inline helper's tail either.
        expect_err(
            "inline tail.antics",
            &obj(
                "fn check() -> bool { return error; }",
                "while check() { self.n = 2; }",
            ),
            HINT,
        );
        expect_err(
            "short circuit.antics",
            &obj(
                "fn check() -> bool { return error; }",
                "if self.n == 1 && check() { self.n = 2; }",
            ),
            HINT,
        );
    }

    /// Only the bare spelling was the error exit: `return (error);` and a tail `error` were
    /// always values, and still read a `const error` or a parameter with their old bytes.
    #[test]
    fn other_spellings_were_always_values() {
        for ret in ["return (error);", "error"] {
            let consts = |name: &str| {
                format!(
                    r#"object "Step Two" {{
    guid: 0x7F40_02B0;
    const {name} = 7;
    #[bhav] #[ret(temp = 0)] fn t() -> i16 {{ {} }}
}}
"#,
                    ret.replace("error", name)
                )
            };
            same_bytes(ret, &consts("error"), &consts("e"));
            let helper = |name: &str| {
                obj(
                    &format!(
                        "fn pick({name}: bool) -> bool {{ {} }}",
                        ret.replace("error", name)
                    ),
                    "while pick(self.n == 0) { self.n = 2; }",
                )
            };
            same_bytes(ret, &helper("error"), &helper("e"));
        }
    }

    #[test]
    fn raw_air_keeps_its_own_return_error() {
        let a = air(
            "raw.antics",
            &obj("", "raw { if temp[0] == 1 { return error; } } self.n = 2;"),
        );
        assert!(tree_t(&a).iter().any(|l| l == "return error;"), "{a}");
    }

    #[test]
    fn panic_and_question_mark_are_the_exit() {
        let a = air("panic.antics", &obj("", "if self.n == 1 { panic!(); }"));
        assert!(tree_t(&a).iter().any(|l| l == "return error;"), "{a}");
        // A binding named `error` changes neither.
        same_iff(
            "panic beside error",
            "let error = 3; self.n = error; if self.n == 1 { panic!(); }",
            "let e = 3; self.n = e; if self.n == 1 { panic!(); }",
        );
        expect_err(
            "try in defer.antics",
            &obj("", "defer { (self.n > 1)?; } self.n = 2;"),
            "'?' cannot exit out of a 'defer' block",
        );
    }
}
