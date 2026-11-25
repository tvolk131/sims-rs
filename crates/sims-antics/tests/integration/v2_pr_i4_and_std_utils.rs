//! `.antics` v2 Wave 2: PR I-4 and the `std::math`, `std::motives` and `std::budget` modules.
//! All sources are synthetic.

/// The built-in helper modules: `std::math`, `std::motives` and `std::budget`.
mod std_utils {
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

    fn iff(name: &str, src: &str) -> Vec<u8> {
        compile_v2_source(name, src, &PackResolverConfig::default())
            .unwrap_or_else(|e| panic!("{name} must compile: {e}"))
            .iff_bytes
    }

    fn expect_err(name: &str, src: &str, needle: &str) {
        let cfg = PackResolverConfig::default();
        let err = compile_v2_source(name, src, &cfg).expect_err(&format!("{name} must fail"));
        assert!(
            err.message.contains(needle),
            "{name}: expected '{needle}' in: {err}"
        );
    }

    fn expect_lines(name: &str, air: &str, lines: &[&str]) {
        for want in lines {
            assert!(air.contains(want), "{name}: missing `{want}` in:\n{air}");
        }
    }

    /// A synthetic object with `uses` before it and one tree 4300 running `body`.
    fn node_tree(uses: &str, body: &str) -> String {
        format!(
            r#"{uses}
object "Std Nodes" {{
    guid: 0x7F40_0261;
    #[bhav(id = 4300)] fn t() {{ {body} }}
}}
"#
        )
    }

    /// Tree 4300's node count and its `.air`.
    fn nodes(uses: &str, body: &str) -> (usize, String) {
        let src = node_tree(uses, body);
        let n = compile_v2_source("nodes.antics", &src, &PackResolverConfig::default())
            .unwrap_or_else(|e| panic!("must compile: {e}\n{src}"))
            .behaviors
            .remove(&4300)
            .expect("tree 4300")
            .instructions
            .len();
        let air = expand("nodes.antics", &src);
        let at = air.find("tree 4300").expect("tree 4300 in .air");
        (n, air[at..].to_owned())
    }

    /// A synthetic object with `uses` before it and one Sim tree `t` running `body`.
    fn sim_tree(uses: &str, body: &str) -> String {
        format!(
            r#"{uses}
object "Std Utils" {{
    guid: 0x7F40_0260;
    state n: i16;
    state m: i16;
    #[bhav] fn t(sim: Sim) {{ {body} }}
}}
"#
        )
    }

    #[test]
    fn std_math_helpers_cost_their_documented_nodes() {
        let math = "use std::math::*;";
        // The baseline: one store.
        assert_eq!(nodes(math, "temp[0] = temp[1];").0, 1);
        for (body, want) in [
            ("temp[0] = min(temp[1], temp[2]);", 4),
            ("temp[0] = max(temp[1], temp[2]);", 4),
            ("temp[0] = clamp(temp[1], temp[2], temp[3]);", 6),
            ("temp[0] = clamp(temp[1], 0, 100);", 6),
            ("temp[0] = abs(temp[1]);", 4),
            // The draw and the compare, plus the guarded store.
            ("if chance_percent(25) { temp[0] = 1; }", 3),
        ] {
            let (n, air) = nodes(math, body);
            assert_eq!(n, want, "{body}:\n{air}");
        }

        let (_, min) = nodes(math, "temp[0] = min(temp[1], temp[2]);");
        expect_lines(
            "min",
            &min,
            &["if temp[1] < temp[2] {", ":= temp[1];", ":= temp[2];"],
        );
        let (_, max) = nodes(math, "temp[0] = max(temp[1], 7);");
        expect_lines("max", &max, &["if temp[1] > 7 {", ":= 7;"]);
        let (_, clamp) = nodes(math, "temp[0] = clamp(temp[1], 0, 100);");
        expect_lines(
            "clamp",
            &clamp,
            &["if temp[1] < 0 {", "if temp[1] > 100 {", ":= 100;"],
        );
        let (_, abs) = nodes(math, "temp[0] = abs(temp[1]);");
        expect_lines("abs", &abs, &[" = temp[1];", "< 0 {", "*= -1;"]);
        let (_, chance) = nodes(math, "if chance_percent(25) { temp[0] = 1; }");
        expect_lines("chance", &chance, &[", 100);", "< 25 {", "temp[0] := 1;"]);
        assert!(!chance.contains(" = 0;"), "no initial store:\n{chance}");
    }

    #[test]
    fn std_math_imports_by_name_and_in_sim_trees() {
        let air = expand(
            "named.antics",
            &sim_tree(
                "use std::math::{abs, chance_percent, clamp, max, min};",
                "self.n = min(self.n, self.m); \
                 self.n = max(self.n, 7); \
                 self.n = clamp(self.n, 0, 100); \
                 self.n = abs(self.m); \
                 if chance_percent(self.m) { self.n = 1; }",
            ),
        );
        expect_lines("named", &air, &["*= -1;", ", 100);", "stack.attr[0] := 1;"]);
        // A glob and a list name the same helpers.
        let body = "self.n = clamp(min(self.n, self.m), 0, 100);";
        assert_eq!(
            iff("glob.antics", &sim_tree("use std::math::*;", body)),
            iff(
                "list.antics",
                &sim_tree("use std::math::{clamp, min};", body)
            )
        );
        // Helpers are imported, not ambient.
        expect_err(
            "no_import.antics",
            &sim_tree("", "self.n = clamp(self.n, 0, 100);"),
            "unsupported value call",
        );
    }

    #[test]
    fn a_user_fn_min_needs_no_std_math() {
        // Without `use std::math`, a module's own `min` is just its own function.
        let air = expand(
            "own_min.antics",
            &format!(
                "fn min(a: i16, b: i16) -> i16 {{ a + b }}\n{}",
                sim_tree("", "self.n = min(self.n, self.m);")
            ),
        );
        assert!(air.contains("+="), "own min adds:\n{air}");
        assert!(!air.contains(" < "), "not std's min:\n{air}");
        // And a helper of its own named like another std module's helper.
        expand(
            "own_try_charge.antics",
            &format!(
                "fn try_charge(a: i16) -> i16 {{ a }}\n{}",
                sim_tree(
                    "use std::math::min;",
                    "self.n = min(try_charge(self.n), 3);"
                )
            ),
        );
    }

    /// A synthetic object whose interactions call every `std::motives` helper.
    const MOTIVE_OBJECT: &str = r#"
object "Std Motives" {
    guid: 0x7F40_0262;
    state done: i16 = 0;
    state fixed: i16 = 0;

    interaction "Rest" {
        run(sim) {
            self.done = restore_motive_loop(sim, Motive::Energy, cycles: 10, step: 30);
        }
    }

    interaction "Rest Tracked" {
        run(sim) {
            restore_motive_tracked(
                sim,
                Motive::Comfort,
                cycles: 4,
                step: 10,
                completed: &mut self.done,
                ticks: 3,
                cap: 90,
            );
        }
    }

    interaction "Soak" {
        run(sim) {
            self.done = restore_two_motives_loop(sim, Motive::Hygiene, 20, Motive::Comfort, 5, cycles: 6);
        }
    }

    interaction "Soak Tracked" {
        run(sim) {
            restore_two_motives_tracked(
                sim,
                Motive::Hygiene,
                20,
                Motive::Comfort,
                5,
                cycles: 6,
                completed: &mut self.done,
                primary_cap: 80,
            );
        }
    }

    interaction "Fix" {
        run(sim) {
            if repair_by_progress(sim, goal: 100, base_step: 20, mechanic_divisor: 100, cycles_out: &mut self.done) {
                self.fixed = 1;
            }
        }
    }
}
"#;

    const MOTIVES_USE: &str = "use std::motives::{\n    repair_by_progress, restore_motive_loop, restore_motive_tracked, restore_two_motives_loop,\n    restore_two_motives_tracked,\n};\n";

    #[test]
    fn std_motives_build_the_bytes_of_the_open_pack_helpers() {
        let from_std = iff("std.antics", &format!("{MOTIVES_USE}{MOTIVE_OBJECT}"));
        // The std source as a module's own functions.
        let source = include_str!("../../src/v2/std/motives.antics");
        let local = iff("local.antics", &format!("{source}\n{MOTIVE_OBJECT}"));
        assert_eq!(from_std, local);
        // The open pack's helpers (which delegate to `std::motives`), in the same file.
        let helpers = include_str!("../../../../open-pack/common/helpers.antics");
        let pack = iff("pack.antics", &format!("{helpers}\n{MOTIVE_OBJECT}"));
        assert_eq!(from_std, pack);
    }

    #[test]
    fn std_motives_loops_are_cancel_safe() {
        let air = expand("motives.antics", &format!("{MOTIVES_USE}{MOTIVE_OBJECT}"));
        expect_lines(
            "motives",
            &air,
            &[
                // The cancel check, before and after each sleep.
                "if my.person.priority == 0 {",
                "idle(2);",
                "idle(3);",
                "my.motive[5] += 30;",
                "my.motive[8] += 20;",
                "my.motive[6] += 5;",
                "if my.motive[5] >= 100 {",
                "if my.motive[6] >= 90 {",
                "if my.motive[8] >= 80 {",
            ],
        );
    }

    /// `try_charge` in a Sim tree with `uses`, as a condition.
    fn charge(uses: &str, call: &str) -> String {
        sim_tree(
            uses,
            &format!("if {call} {{ self.n = 1; }} else {{ self.n = 2; }}"),
        )
    }

    const BUDGET_USE: &str = "use std::budget::try_charge;\nuse std::engine::BudgetCategory;";

    #[test]
    fn try_charge_is_one_alter_budget_node() {
        let typed = expand(
            "typed.antics",
            &charge(BUDGET_USE, "try_charge(sim, 50, BudgetCategory::Buy)"),
        );
        expect_lines("typed", &typed, &["if alter_budget(50, category: 6) {"]);
        assert!(!typed.contains("test_only"), "no separate test:\n{typed}");
        // The same bytes as the primitive written out.
        assert_eq!(
            iff(
                "typed.antics",
                &charge(BUDGET_USE, "try_charge(sim, 25, BudgetCategory::Build)")
            ),
            iff(
                "prim.antics",
                &charge("", "sim.debit_funds(25, category: 7)")
            )
        );
    }

    #[test]
    fn try_charge_takes_an_i16_category_through_from_raw() {
        let raw = expand(
            "raw.antics",
            &charge(
                BUDGET_USE,
                "try_charge(sim, 50, BudgetCategory::from_raw(6))",
            ),
        );
        expect_lines("raw", &raw, &["if alter_budget(50, category: 6) {"]);
        // A const and a category with no variant.
        let unnamed = expand(
            "unnamed.antics",
            &format!(
                "const LEDGER = 3;\n{}",
                charge(
                    BUDGET_USE,
                    "try_charge(sim, 50, BudgetCategory::from_raw(LEDGER))"
                )
            ),
        );
        expect_lines("unnamed", &unnamed, &["alter_budget(50, category: 3)"]);
        assert_eq!(
            iff(
                "raw.antics",
                &charge(
                    BUDGET_USE,
                    "try_charge(sim, 50, BudgetCategory::from_raw(7))"
                )
            ),
            iff(
                "typed.antics",
                &charge(BUDGET_USE, "try_charge(sim, 50, BudgetCategory::Build)")
            )
        );
        // A bare number is not a `BudgetCategory`; a runtime word is not a constant.
        expect_err(
            "bare.antics",
            &charge(BUDGET_USE, "try_charge(sim, 50, 6)"),
            "BudgetCategory",
        );
        expect_err(
            "runtime.antics",
            &charge(
                BUDGET_USE,
                "try_charge(sim, 50, BudgetCategory::from_raw(self.m))",
            ),
            "must be a compile-time constant",
        );
        expect_err(
            "range.antics",
            &charge(
                BUDGET_USE,
                "try_charge(sim, 50, BudgetCategory::from_raw(300))",
            ),
            "out of range",
        );
    }

    #[test]
    fn unknown_std_module_lists_every_built_in_module() {
        let err = compile_v2_source(
            "unknown.antics",
            &sim_tree("use std::maths::*;", "self.n = 1;"),
            &PackResolverConfig::default(),
        )
        .expect_err("no std::maths");
        for m in ["globals", "flags", "engine", "math", "motives", "budget"] {
            assert!(
                err.message.contains(&format!("'std::{m}'")),
                "{m} in: {err}"
            );
        }
    }

    /// The helpers run in `sims-vm` through `sims-antics test`.
    #[cfg(feature = "vm-replay")]
    mod vm {
        use std::path::{Path, PathBuf};
        use std::process::Command;

        const OBJECT: &str = r#"use std::budget::try_charge;
use std::engine::BudgetCategory;
use std::math::*;
use std::motives::{repair_by_progress, restore_motive_loop};

object "Std Utils Probe" {
    guid: 0x7E00_7E20;
    price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);

    state a: i16 = 0;
    state b: i16 = 0;
    state out: i16 = 0;
    state done: i16 = 0;

    main {
        loop {
            sleep(1);
        }
    }

    #[bhav(id = 4400, label = "Min")]
    fn do_min() {
        self.out = min(self.a, self.b);
    }

    #[bhav(id = 4401, label = "Max")]
    fn do_max() {
        self.out = max(self.a, self.b);
    }

    #[bhav(id = 4402, label = "Clamp")]
    fn do_clamp() {
        self.out = clamp(self.a, -10, self.b);
    }

    #[bhav(id = 4403, label = "Abs")]
    fn do_abs() {
        self.out = abs(self.a);
    }

    #[bhav(id = 4404, label = "Chance")]
    fn do_chance() {
        return chance_percent(self.a);
    }

    interaction "Rest" {
        run(sim) {
            self.done = restore_motive_loop(sim, Motive::Energy, cycles: 10, step: 30);
        }
    }

    interaction "Fix" {
        run(sim) {
            if repair_by_progress(sim, goal: 100, base_step: 20, mechanic_divisor: 100, cycles_out: &mut self.done) {
                self.out = 1;
            }
        }
    }

    interaction "Buy" {
        run(sim) {
            if try_charge(sim, 50, BudgetCategory::Buy) {
                self.out = 1;
            } else {
                self.out = 2;
            }
        }
    }

    interaction "Buy Raw" {
        run(sim) {
            if try_charge(sim, 70, BudgetCategory::from_raw(7)) {
                self.out = 1;
            } else {
                self.out = 2;
            }
        }
    }
}
"#;

        const SPEC: &str = r#"[subject]
antics = "std-utils-probe.antics"

[sim]
motives = { energy = 0 }

[[test]]
name = "min"
tree = "Min"
given = { attrs = { a = 3, b = -7 } }
expect = { attrs = { out = -7 } }

[[test]]
name = "min of equals"
tree = "Min"
given = { attrs = { a = 4, b = 4 } }
expect = { attrs = { out = 4 } }

[[test]]
name = "max"
tree = "Max"
given = { attrs = { a = 3, b = -7 } }
expect = { attrs = { out = 3 } }

[[test]]
name = "clamp above"
tree = "Clamp"
given = { attrs = { a = 150, b = 100 } }
expect = { attrs = { out = 100 } }

[[test]]
name = "clamp below"
tree = "Clamp"
given = { attrs = { a = -32768, b = 100 } }
expect = { attrs = { out = -10 } }

[[test]]
name = "clamp inside"
tree = "Clamp"
given = { attrs = { a = 42, b = 100 } }
expect = { attrs = { out = 42 } }

[[test]]
name = "clamp with lo above hi: at or above lo gives hi"
tree = "Clamp"
given = { attrs = { a = 0, b = -20 } }
expect = { attrs = { out = -20 } }

[[test]]
name = "clamp with lo above hi: below lo gives lo"
tree = "Clamp"
given = { attrs = { a = -15, b = -20 } }
expect = { attrs = { out = -10 } }

[[test]]
name = "abs of a negative"
tree = "Abs"
given = { attrs = { a = -9 } }
expect = { attrs = { out = 9 } }

[[test]]
name = "abs of a positive"
tree = "Abs"
given = { attrs = { a = 32767 } }
expect = { attrs = { out = 32767 } }

[[test]]
name = "abs of -32768 wraps to itself"
tree = "Abs"
given = { attrs = { a = -32768 } }
expect = { attrs = { out = -32768 } }

[[test]]
name = "a 0% chance is never true"
tree = "Chance"
given = { attrs = { a = 0 } }
expect = { result = false }

[[test]]
name = "a 100% chance is always true"
tree = "Chance"
given = { attrs = { a = 100 } }
expect = { result = true }

[[test]]
name = "restoring stops at the cap"
interaction = "Rest"
expect = { attrs = { done = 4 }, sim = { motives = { energy = 100 } } }

[[test]]
name = "an unskilled repair takes five cycles"
interaction = "Fix"
expect = { attrs = { done = 5, out = 1 } }

[[test]]
name = "an affordable charge is spent"
interaction = "Buy"
given = { funds = 1000 }
expect = { attrs = { out = 1 }, funds = 950 }

[[test]]
name = "an unaffordable charge spends nothing"
interaction = "Buy"
given = { funds = 49 }
expect = { attrs = { out = 2 }, funds = 49 }

[[test]]
name = "a numbered category charges too"
interaction = "Buy Raw"
given = { funds = 100 }
expect = { attrs = { out = 1 }, funds = 30 }
"#;

        /// A scratch directory for this process (left in place: tests never delete).
        fn scratch() -> PathBuf {
            let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
                .join(format!("std-utils-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("scratch dir");
            dir
        }

        #[test]
        fn std_helpers_run_in_the_vm() {
            let dir = scratch();
            std::fs::write(dir.join("std-utils-probe.antics"), OBJECT).unwrap();
            let spec = dir.join("std-utils-probe.test.toml");
            std::fs::write(&spec, SPEC).unwrap();
            let out = Command::new(env!("CARGO_BIN_EXE_sims-antics"))
                .arg("test")
                .arg(&spec)
                .output()
                .expect("run sims-antics");
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(out.status.success(), "{text}\n{out:?}");
            assert!(!text.contains("FAIL"), "{text}");
            let summary = text.lines().last().unwrap_or_default();
            assert_eq!(summary, "18 passed, 0 failed", "{text}");
        }
    }
}

/// PR I-4: scopes, primitive options, inherit-tree calls, read-only helper parameters, scoped
/// layout, and three lowering fixes.
mod pr_i4 {
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

    /// The `.air` body of the tree labelled `label`.
    fn tree<'a>(air: &'a str, label: &str) -> &'a str {
        let start = air
            .find(&format!("\"{label}\""))
            .unwrap_or_else(|| panic!("tree {label}:\n{air}"));
        let end = air[start..].find("\n}\n").map_or(air.len(), |e| start + e);
        &air[start..end]
    }

    /// `body`'s `.air` lines, in order, appear in `text` one after another (blank-insensitive).
    fn expect_seq(name: &str, text: &str, lines: &[&str]) {
        let got: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let found = got.windows(lines.len()).any(|w| w == lines);
        assert!(found, "{name}: missing\n{}\nin:\n{text}", lines.join("\n"));
    }

    /// One object with Sim tree `t(sim: Sim)` running `body`, after `uses` and `extra` members.
    fn sim_tree(uses: &str, extra: &str, body: &str) -> String {
        format!(
            r#"{uses}
object "Pr I4" {{
    guid: 0x7F40_0262;
    state n: i16;
{extra}
    #[bhav] fn t(sim: Sim) {{ {body} }}
}}
"#
        )
    }

    fn t_air(name: &str, uses: &str, extra: &str, body: &str) -> String {
        let air = expand(name, &sim_tree(uses, extra, body));
        tree(&air, "t").to_owned()
    }

    mod scopes {
        use super::*;

        #[test]
        fn temp_indexed_cells_read_their_temp_with_no_setup() {
            let t = t_air(
                "temp_indexed.antics",
                "",
                "",
                "temp[1] = stack_obj.sprite_flags[temp[3]]; \
                 stack_obj.sprite_flags[temp[2]] = 1; \
                 temp[4] = sim.person[temp[5]]; \
                 sim.person[temp[6]] = 2; \
                 temp[7] = stack_obj.person[temp[1]];",
            );
            for want in [
                "temp[1] := scope(27, 3);",
                "scope(27, 2) := 1;",
                "temp[4] := scope(30, 5);",
                "scope(30, 6) := 2;",
                "temp[7] := scope(31, 1);",
            ] {
                assert!(t.contains(want), "missing `{want}` in:\n{t}");
            }
            // A constant index still goes through temp[0] (scope 27 with setup).
            let t = t_air(
                "sprite_const.antics",
                "",
                "",
                "temp[1] = stack_obj.sprite_flags[4];",
            );
            expect_seq(
                "sprite_const",
                &t,
                &["temp[0] := 4;", "temp[1] := scope(27, 0);"],
            );
        }

        #[test]
        fn neighbor_cells_are_scopes_24_and_32() {
            let t = t_air(
                "neighbor.antics",
                "",
                "",
                "temp[0] = stack_obj.neighbor.age; \
                 temp[1] = stack_obj.neighbor.field[3]; \
                 temp[2] = stack_obj.neighbor.person[10]; \
                 temp[3] = stack_obj.neighbor.fame; \
                 temp[4] = sim.fame;",
            );
            for want in [
                "temp[0] := scope(24, 2);",
                "temp[1] := scope(24, 3);",
                "temp[2] := scope(32, 10);",
                "temp[3] := scope(32, 81);",
                "temp[4] := my.person[81];",
            ] {
                assert!(t.contains(want), "missing `{want}` in:\n{t}");
            }
        }

        #[test]
        fn neighbor_cells_refuse_writes_bad_fields_and_indices() {
            expect_err(
                "neighbor_write.antics",
                &sim_tree("", "", "stack_obj.neighbor.age = 1;"),
                "a neighbor cell is read-only",
            );
            expect_err(
                "neighbor_index.antics",
                &sim_tree("", "", "temp[0] = stack_obj.neighbor.field[10];"),
                "neighbor.field index 10 out of range 0..=9",
            );
            expect_err(
                "neighbor_field.antics",
                &sim_tree("", "", "temp[0] = stack_obj.neighbor.bogus;"),
                "unknown neighbor field 'bogus'",
            );
        }

        #[test]
        fn advert_delta_is_scope_12() {
            let t = t_air("advert.antics", "", "", "advert.delta[2] = 5;");
            assert!(t.contains("scope(12, 2) := 5;"), "{t}");
        }
    }

    mod prim_options {
        use super::*;

        #[test]
        fn pad_bytes_follow_the_primitive() {
            let t = t_air(
                "pads.antics",
                "",
                "",
                "play_sound(3, pad3: 0x01); play_sound(4, pad7: 0xff, pad6: 0xff); \
                 clear_motive_changes(pad2: 0x05);",
            );
            for want in [
                "play_sound(3, stack_obj) pad(3: 0x01);",
                "play_sound(4, stack_obj) pad(6: 0xff, 7: 0xff);",
                "clear_motive_changes() pad(2: 0x05);",
            ] {
                assert!(t.contains(want), "missing `{want}` in:\n{t}");
            }
        }

        #[test]
        fn pad_bytes_refuse_spelled_and_repeated_bytes() {
            for (name, body, needle) in [
                (
                    "pad_spelled",
                    "play_sound(3, pad0: 1);",
                    "operand byte 0 is spelled by the call's arguments, not 'pad0'",
                ),
                (
                    "pad_twice",
                    "play_sound(3, pad3: 1, pad3: 2);",
                    "'pad3' is given twice",
                ),
                (
                    "clear_args",
                    "clear_motive_changes(3);",
                    "takes no arguments besides 'padN: 0xNN' bytes",
                ),
            ] {
                expect_err(name, &sim_tree("", "", body), needle);
            }
        }

        #[test]
        fn reach_make_new_character_and_drop_onto_parameters() {
            let src = r#"object "Pr I4 Prims" {
    guid: 0x7F40_0263;
    state n: i16;
    #[bhav] fn t(sim: Sim, a: i16, b: i16) {
        drop_onto(src: 0, dst: b);
        if drop_onto(src: 1, dst: param[0]) { self.n = 1; }
        reach(mode: 1, moves: true, slot: b);
        reach(mode: 0, moves: false);
        let skin: i16 = 4;
        let age: i16 = 3;
        let sex: i16 = a;
        if !make_new_character(skin: skin, age: age, sex: sex, species: 1) { self.n = 2; }
    }
}
"#;
            let air = expand("prims.antics", src);
            let t = tree(&air, "t");
            for want in [
                "drop_onto(src: 0, dst: param[1]);",
                "if drop_onto(src: 1, dst: param[0]) {",
                "reach(mode: 1, moves: true, param: 1);",
                "reach(mode: 0, moves: false);",
                "make_new_character(skin: __v_skin_",
                ", species: 1)",
            ] {
                assert!(t.contains(want), "missing `{want}` in:\n{t}");
            }
            // Make New Character moves the cursor, so the tree puts `self` back for its write.
            expect_seq(
                "make_new_character",
                t,
                &["stack_obj := __self_obj;", "stack.attr[0] := 2;"],
            );
        }

        #[test]
        fn reach_and_make_new_character_check_their_arguments() {
            for (name, body, needle) in [
                (
                    "reach_no_slot",
                    "reach(mode: 1);",
                    "reach(mode: 1) reads the slot from a parameter; name it with 'slot:'",
                ),
                (
                    "reach_slot_mode0",
                    "reach(mode: 0, slot: 1);",
                    "reach reads 'slot:' only in mode 1",
                ),
                (
                    "mnc_species",
                    "let s: i16 = 1; make_new_character(skin: s, age: s, sex: s, species: 3);",
                    "species 3 out of range 0..=2",
                ),
            ] {
                expect_err(name, &sim_tree("", "", body), needle);
            }
        }
    }

    mod fixes {
        use super::*;

        /// An inline helper whose constant tail exits with a result tests a constant first, so
        /// the exit's last result is that tail's, not the call before it.
        #[test]
        fn a_constant_inline_tail_records_its_own_result() {
            let src = r#"use std::globals::*;

object "Pr I4 Tail" {
    guid: 0x7F40_0264;
    fn leave() -> bool {
        globals::standard_exit(50)?;
        false
    }
    #[bhav(id = 4100)]
    fn t(sim: Sim) -> bool {
        if sim.person[62] == 3 {
            return leave();
        }
        sim.person[62] += 1;
        leave()
    }
}
"#;
            let air = expand("tail.antics", src);
            let t = tree(&air, "t");
            assert_eq!(t.matches("if 1 == 0 {").count(), 2, "{t}");
            expect_seq("tail", t, &["if 1 == 0 {", "}", "return false;"]);
        }

        /// A foreign read after `stack_obj = o` in a tree that is not explicit puts back `o`,
        /// not the entry object.
        #[test]
        fn a_foreign_read_restores_the_assigned_cursor() {
            let src = r#"object "Pr I4 Cursor" {
    guid: 0x7F40_0265;
    #[bhav] fn t(sim: Sim, o: ObjectId, p: ObjectId) {
        stack_obj = o;
        temp[0] = p.attr[3];
        temp[1] = stack_obj.attr[1];
    }
}
"#;
            let air = expand("cursor.antics", src);
            expect_seq(
                "cursor",
                tree(&air, "t"),
                &[
                    "stack_obj := param[0];",
                    "let __pre_stack_obj_1 = stack_obj;",
                    "stack_obj := param[1];",
                    "let __fattr_0 = stack.attr[3];",
                    "stack_obj := __pre_stack_obj_1;",
                    "temp[0] := __fattr_0;",
                    "temp[1] := stack.attr[1];",
                ],
            );
        }

        /// In-place arithmetic into an observable cell goes through a local when a later operand
        /// read may fault, so a fault leaves the cell as it was.
        #[test]
        fn arithmetic_stages_before_a_faulting_read() {
            let t = t_air(
                "partial.antics",
                "",
                "",
                "temp[0] = 1 + stack_obj.attr[0]; temp[2] = 1 + stack_obj.object_type; \
                 temp[6] = -stack_obj.object_type; temp[5] = temp[5] - sim.motive.hunger; \
                 temp[7] = 3 * temp[1]; temp[4] = 1 + self.n;",
            );
            for seq in [
                &[
                    "__arith_0 := 1;",
                    "__arith_0 += scope(21, 9);",
                    "temp[2] := __arith_0;",
                ][..],
                &[
                    "__arith_0 := 0;",
                    "__arith_0 -= scope(21, 9);",
                    "temp[6] := __arith_0;",
                ],
                // Reads that cannot fault stay in place.
                &["temp[5] -= my.motive.hunger;"],
                &["temp[7] := 3;", "temp[7] *= temp[1];"],
                // `self` in a tree that never moves the cursor is the entry object.
                &["temp[4] := 1;", "temp[4] += stack.attr[0];"],
            ] {
                expect_seq("partial", &t, seq);
            }
            assert!(!t.contains("temp[2] := 1;"), "{t}");
            // Once the tree moves the cursor, `self` may name another object.
            let t = t_air(
                "partial_moved.antics",
                "",
                "",
                "stack_obj = temp[1]; temp[3] = 1 + self.n;",
            );
            expect_seq(
                "partial_moved",
                &t,
                &["__arith_0 += stack.attr[0];", "temp[3] := __arith_0;"],
            );
            // Writing `self` through the moved cursor is staged too: the first write can land
            // and the read still fault (a word past the object's own count, say).
            let t = t_air(
                "partial_self.antics",
                "",
                "",
                "stack_obj = temp[1]; self.n = 1 + self.graphic;",
            );
            expect_seq(
                "partial_self",
                &t,
                &[
                    "let __arith_0 = 1;",
                    "__arith_0 += stack.data[0];",
                    "stack.attr[0] := __arith_0;",
                ],
            );
        }
    }

    mod inherit_calls {
        use super::*;

        fn object(main: &str, inherit_body: &str) -> String {
            format!(
                r#"object "Pr I4 Inherit" {{
    guid: 0x7F40_0266;
    state n: i16;
    {main}
    #[bhav(id = 4200, args = 1, locals = 1)]
    fn open_door(sim: Sim, event: i16) {{ self.n = event; }}
    #[bhav(id = 4201)]
    fn plain(sim: Sim) {{ self.n = 3; }}
    #[bhav(id = 4202, context = inherit, locals = 1)]
    fn sing() {{ {inherit_body} }}
    #[bhav(id = 4203, context = inherit)]
    fn sing_outer() {{ sing(); }}
    #[bhav(id = 4204)]
    fn from_sim(sim: Sim) {{ sing_outer(); }}
}}
"#
            )
        }

        #[test]
        fn an_inherit_tree_calls_a_sim_tree_with_me() {
            let air = expand(
                "inherit_call.antics",
                &object("", "let ev = temp[1]; open_door(me, ev)?; plain(me);"),
            );
            let sing = tree(&air, "sing");
            assert!(sing.contains("if !call 4200(temps) {"), "{sing}");
            assert!(sing.contains("call 4201(temps)"), "{sing}");
            // Spelled with raw code, the tree is the same.
            let raw = expand(
                "inherit_raw.antics",
                &object(
                    "",
                    "raw { temp[0] := temp[1]; \
                     if !call 4200(temps) { return error; } } plain(me);",
                ),
            );
            let canon = |t: &str| t.split_whitespace().collect::<String>();
            assert_eq!(canon(sing), canon(tree(&raw, "sing")), "{air}\n{raw}");
        }

        #[test]
        fn the_sim_must_be_me() {
            expect_err(
                "inherit_stack.antics",
                &object("", "plain(stack_obj);"),
                "tree 'plain' expects 'me' (the inherit tree's runner) as its first argument",
            );
        }

        #[test]
        fn object_trees_may_not_call_them() {
            for (name, main) in [
                ("init_direct", "init { sing(); }"),
                ("init_through", "init { sing_outer(); }"),
            ] {
                expect_err(
                    name,
                    &object(main, "plain(me);"),
                    "is 'context = inherit' but runs a Sim tree on its runner",
                );
            }
            // An inherit tree that names no Sim tree stays callable from anywhere.
            expand(
                "init_free.antics",
                &object("init { sing_outer(); }", "temp[0] = 1;"),
            );
        }
    }

    mod read_only_params {
        use super::*;

        #[test]
        fn pure_helpers_read_stable_cells_in_place() {
            let t = t_air(
                "zero_copy.antics",
                "use std::math::*;",
                "",
                "temp[0] = min(temp[1], self.n); temp[4] = max(sim.age, global[3]);",
            );
            assert!(t.contains("if my.person[58] > global[3] {"), "{t}");
            // Nothing proves a Sim tree's entry cursor names a live object, so `self.n` is
            // read once, up front.
            expect_seq(
                "zero_copy",
                &t,
                &[
                    "let __arg_b_0 = stack.attr[0];",
                    "let __ret_min_1;",
                    "if temp[1] < __arg_b_0 {",
                ],
            );
        }

        #[test]
        fn faulting_or_changing_cells_are_still_copied() {
            // A stack object read may fault, so it is read once, up front.
            let t = t_air(
                "copy_stack.antics",
                "use std::math::*;",
                "",
                "temp[2] = min(stack_obj.attr[1], 4);",
            );
            expect_seq("copy_stack", &t, &["let __arg_a_0 = stack.attr[1];"]);
            // A helper that writes the cell's object gets a copy.
            let t = t_air(
                "copy_write.antics",
                "",
                "fn bump(x: i16) -> i16 { self.n += 1; x + self.n }",
                "temp[0] = bump(sim.age);",
            );
            assert!(t.contains("__arg_x"), "{t}");
        }
    }

    mod raw_layout {
        use super::*;

        fn layout(raw: &str) -> String {
            let air = expand(
                "raw_layout.antics",
                &sim_tree("", "", &format!("let unused = 5; {raw}")),
            );
            tree(&air, "t").to_owned()
        }

        /// The optimizer treats such a block like the structured code it spells: the trees
        /// come out the same.
        #[test]
        fn a_lowering_shaped_raw_block_leaves_the_tree_to_the_optimizer() {
            let canon = |t: &str| t.split_whitespace().collect::<String>();
            let extra = "    #[bhav(id = 4200)] fn p(sim: Sim) { }";
            for (raw, structured) in [
                (
                    "raw { if !call 4200(temps) { // a comment\n return error; } }",
                    "p(sim)?;",
                ),
                ("raw { temp[0] := 1; }", "temp[0] = 1;"),
                (
                    "raw { if temp[0] == 1 { temp[1] := 2; } else { temp[1] := 3; } }",
                    "if temp[0] == 1 { temp[1] = 2; } else { temp[1] = 3; }",
                ),
            ] {
                let body = |b: &str| {
                    let air = expand(
                        "raw_layout.antics",
                        &sim_tree("", extra, &format!("let unused = 5; temp[7] = unused; {b}")),
                    );
                    tree(&air, "t").to_owned()
                };
                let (r, s) = (body(raw), body(structured));
                assert_eq!(canon(&r), canon(&s), "{raw}:\n{r}\n{s}");
            }
            assert!(!layout("raw { temp[0] := 1; }").contains(" = 5;"));
        }

        #[test]
        fn other_raw_blocks_keep_the_layout() {
            for raw in [
                "raw { const K = 1; temp[0] := K; }",
                "raw { local[0] := 1; }",
                "raw { call 4200(1, 2); }",
                "raw { temp[0] := scope(21, 9); }",
                "raw { if expr(temp[0] := 1) { } }",
            ] {
                let t = layout(raw);
                assert!(t.contains(" = 5;"), "{raw}:\n{t}");
            }
        }
    }

    /// Regressions from the first review round.
    mod review_round_1 {
        use super::*;

        fn object(members: &str) -> String {
            format!(
                "object \"Pr I4 R1\" {{\n    guid: 0x7F40_0263;\n    state n: i16;\n{members}\n}}\n"
            )
        }

        fn t_of(name: &str, members: &str) -> String {
            tree(&expand(name, &object(members)), "t").to_owned()
        }

        /// The line printer strips only `//` comments, so a `#` comment keeps the layout.
        #[test]
        fn hash_comments_keep_the_raw_layout() {
            let t = t_of(
                "raw_hash.antics",
                "    #[bhav] fn t() { raw {\n        # note; temp[0] := 7;\n    } }",
            );
            assert!(t.contains("# note; temp[0] := 7;"), "{t}");
        }

        /// An inlined helper's early return does not copy a raw arm to each return site.
        #[test]
        fn raw_arms_are_not_copied_per_return_site() {
            let t = t_of(
                "raw_arm.antics",
                "    fn check() -> bool { if temp[0] == 1 { return true; } true }\n    \
                 #[bhav] fn t() { if check() { raw { temp[1] := 9; } } }",
            );
            assert_eq!(t.matches("temp[1] := 9;").count(), 1, "{t}");
        }

        /// A raw block's cursor stores survive to the exit.
        #[test]
        fn raw_cursor_stores_reach_the_exit() {
            let t = t_of(
                "raw_exit.antics",
                "    #[bhav] fn t(sim: Sim) { raw { find_best_object(4); stack_obj := 0; } }",
            );
            assert!(t.contains("stack_obj := 0;"), "{t}");
        }

        /// After a raw block moves the cursor, a foreign read puts back that cursor, not the
        /// entry object; in a loop, so does a read before the block.
        #[test]
        fn raw_cursor_writes_are_saved_around_moves() {
            let t = t_of(
                "raw_cursor.antics",
                "    #[bhav] fn t(sim: Sim, o: ObjectId, p: ObjectId) {\n        \
                 raw { stack_obj := param[0]; }\n        temp[0] = p.attr[3];\n    }",
            );
            expect_seq(
                "raw_cursor",
                &t,
                &[
                    "stack_obj := param[0];",
                    "let __pre_stack_obj_1 = stack_obj;",
                    "stack_obj := param[1];",
                    "let __fattr_0 = stack.attr[3];",
                    "stack_obj := __pre_stack_obj_1;",
                ],
            );
            let t = t_of(
                "raw_cursor_loop.antics",
                "    #[bhav] fn t(sim: Sim, o: ObjectId, p: ObjectId) {\n        \
                 while temp[1] < 3 {\n            temp[0] = p.attr[3];\n            \
                 temp[2] = stack_obj.attr[1];\n            \
                 raw { stack_obj := param[0]; }\n            temp[1] += 1;\n        }\n    }",
            );
            // The second iteration's `stack_obj.attr[1]` reads the raw block's cursor.
            assert!(t.contains("let __pre_stack_obj"), "{t}");
            assert!(!t.contains("__self_obj"), "{t}");
        }

        /// A word past what every object has is staged even when the destination goes through
        /// the same cursor, and so is an inherit tree's motive read (its runner may be an
        /// object).
        #[test]
        fn bounds_and_runner_decide_in_place_arithmetic() {
            let t = t_of(
                "arith_bounds.antics",
                "    #[bhav] fn t(sim: Sim) { stack_obj = temp[0]; self.n = 1 + self.attr[100]; }",
            );
            expect_seq(
                "arith_bounds",
                &t,
                &["let __arith_0 = 1;", "__arith_0 += stack.attr[100];"],
            );
            let air = expand(
                "arith_inherit.antics",
                &object(
                    "    #[bhav] #[context(inherit)] fn t() { temp[2] = 1 + me.motive.hunger; }\n    \
                     main { t(); }",
                ),
            );
            let t = tree(&air, "t");
            assert!(!t.contains("temp[2] := 1;"), "{t}");
        }

        /// An object reaches a functional's action and guard (`run_functional`, Find Best
        /// Object), so neither may call an inherit tree that runs a Sim tree.
        #[test]
        fn functionals_cannot_reach_sim_only_inherit_trees() {
            let msg =
                "so it can only be called from interactions, Sim trees, or other inherit trees";
            expect_err(
                "inherit_functional.antics",
                &object(
                    "    #[bhav] fn s(sim: Sim) { temp[0] = sim.fame; }\n    \
                     #[bhav] #[context(inherit)] fn g() { s(me); }\n    \
                     functional cook { run(sim) { g(); } }",
                ),
                msg,
            );
            expect_err(
                "inherit_guard.antics",
                &object(
                    "    #[bhav] fn s(sim: Sim) -> bool { sim.fame > 0 }\n    \
                     #[bhav] #[context(inherit)] fn g() -> bool { s(me) }\n    \
                     functional cook { score: 50, guard(sim) => g(), run(sim) {} }",
                ),
                msg,
            );
        }

        /// Code a literal condition discards emits no call, so it adds no inherit fact.
        #[test]
        fn discarded_calls_add_no_inherit_facts() {
            expand(
                "inherit_dead.antics",
                &object(
                    "    #[bhav] fn s(sim: Sim) { temp[0] = sim.fame; }\n    \
                     #[bhav] #[context(inherit)] fn g() { if false { s(me); } }\n    \
                     main { g(); }",
                ),
            );
        }

        /// A read-only helper argument that wakes the object (data word 25) is still copied.
        #[test]
        fn waking_reads_are_not_forwarded() {
            let t = t_of(
                "helper_wake.antics",
                "    fn ignore(x: i16 = self.data[25]) {}\n    #[bhav] fn t() { ignore(); }",
            );
            assert!(t.contains("= my.data[25];"), "{t}");
        }

        /// A temp-indexed destination never takes in-place arithmetic: the index may alias a
        /// cell the right side reads.
        #[test]
        fn temp_indexed_destinations_are_staged() {
            let t = t_of(
                "dynamic_alias.antics",
                "    #[bhav] fn t(sim: Sim) { temp[1] = 81; sim.person[temp[1]] = 1 + sim.fame; }",
            );
            assert!(t.contains("scope(30, 1) := __arith_0;"), "{t}");
        }

        /// A staged level reading `scope(30, 0)` is copied before the career goes into
        /// `temp[0]`.
        #[test]
        fn job_staging_copies_cells_indexed_by_temp_0() {
            let t = t_of(
                "job_scope30.antics",
                "    #[bhav] fn t(sim: Sim) { temp[0] = 5; temp[1] = job_data(3, sim.person[temp[0]], 1); }",
            );
            expect_seq(
                "job_scope30",
                &t,
                &["let __job_level_0 = scope(30, 0);", "temp[0] := 3;"],
            );
        }

        #[test]
        fn make_new_character_needs_i16_locals() {
            expect_err(
                "make_enum.antics",
                &object(
                    "    enum Look { Old = 3 }\n    #[bhav] fn t(sim: Sim) {\n        \
                     let i: Look = Look::Old;\n        make_new_character(skin: i, age: i, sex: i);\n    }",
                ),
                "make_new_character(skin:) must name an i16 local",
            );
        }
    }

    /// Regressions from the second review round.
    mod review_round_2 {
        use super::*;

        fn object(members: &str) -> String {
            format!("object \"Pr I4 R2\" {{\n    guid: 0x7F40_0264;\n{members}\n}}\n")
        }

        fn air_of(name: &str, members: &str, label: &str) -> String {
            tree(&expand(name, &object(members)), label).to_owned()
        }

        /// An object may run a functional, so its person reads are staged and never dropped.
        #[test]
        fn functional_person_reads_may_fault() {
            let t = air_of(
                "functional_fault.antics",
                "    fn ignore(x: i16) -> i16 { 7 }\n    functional cook {\n        score: 50,\n        \
                 run(sim) {\n            temp[0] = 1 + sim.age;\n            \
                 temp[1] = ignore(sim.age);\n        }\n    }",
                "Functional_Cook",
            );
            assert!(!t.contains("temp[0] := 1;"), "{t}");
            assert_eq!(t.matches("my.person[58]").count(), 2, "{t}");
        }

        /// A Sim tree's entry cursor is not proven live: an unused cursor argument is still
        /// read.
        #[test]
        fn cursor_arguments_are_read_eagerly() {
            let t = air_of(
                "cursor_arg.antics",
                "    state n: i16;\n    fn ignore(x: i16) -> i16 { 7 }\n    \
                 #[bhav] fn t(sim: Sim) { temp[1] = ignore(self.n); }",
                "t",
            );
            assert!(t.contains("= stack.attr[0];"), "{t}");
        }

        /// A `&mut` to a temp-indexed cell names the cell chosen at the call, even when the
        /// helper rewrites the index temp.
        #[test]
        fn temp_indexed_borrows_keep_their_cell() {
            let t = air_of(
                "borrow.antics",
                "    fn overwrite(word: &mut i16) {\n        temp[0] = 1;\n        *word = 42;\n    }\n    \
                 #[bhav] fn t(sim: Sim) {\n        overwrite(&mut self.sprite_flags[temp[0]]);\n        \
                 overwrite(&mut sim.person[temp[0]]);\n    }",
                "t",
            );
            for scope in ["27", "30"] {
                expect_seq(
                    "borrow",
                    &t,
                    &[
                        "temp[0] := 1;",
                        "temp[0] := __ref_idx_word_0;",
                        &format!("scope({scope}, 0) := 42;"),
                    ],
                );
            }
        }

        /// Raw cursor writes count in `repeat`, animation event loops, an inlined condition
        /// helper and a `run_functional` argument.
        #[test]
        fn raw_cursor_writes_count_in_every_loop_and_ahead_of_arms() {
            let body = "temp[0] = p.attr[0];\n            temp[1] = stack_obj.attr[0];\n            \
                        raw { stack_obj := param[1]; }";
            for (name, members) in [
                (
                    "repeat",
                    format!(
                        "    #[bhav] fn t(p: ObjectId, o: ObjectId) {{ repeat 2 {{ {body} }} }}"
                    ),
                ),
                (
                    "animate",
                    format!(
                        "    #[bhav] fn t(sim: Sim, p: ObjectId, o: ObjectId) {{\n        \
                         animate(clip: 8, table: 0, events: 2) |event| {{ {body} }}\n    }}"
                    ),
                ),
                (
                    "condition",
                    "    fn mv() -> bool { raw { stack_obj := param[1]; } temp[7] == 1 }\n    \
                     #[bhav] fn t(p: ObjectId, o: ObjectId) {\n        \
                     if mv() { temp[0] = p.attr[0]; temp[1] = stack_obj.attr[0]; }\n    }"
                        .to_owned(),
                ),
            ] {
                let t = air_of(&format!("{name}.antics"), &members, "t");
                assert!(t.contains("__pre_stack_obj"), "{name}:\n{t}");
                assert!(
                    !t.contains("__saved_stack_obj") && !t.contains("__self_obj"),
                    "{name}:\n{t}"
                );
            }
            let t = air_of(
                "rf_arg.antics",
                "    fn change() -> i16 { raw { stack_obj := param[0]; } 5 }\n    \
                 #[bhav] fn t(o: ObjectId) {\n        stack_obj.run_functional(cook, change());\n        \
                 temp[7] = stack_obj.attr[0];\n    }",
                "t",
            );
            let call = t.find("run_functional_tree(cook);").expect("call");
            assert!(t[call..].contains("stack_obj := __pre_stack_obj"), "{t}");
        }

        /// Make New Character leaves the new neighbor in the cursor, which a later foreign
        /// read puts back.
        #[test]
        fn make_new_character_moves_the_cursor() {
            let t = air_of(
                "make_cursor.antics",
                "    #[bhav] fn t(p: ObjectId) {\n        let skin = 0;\n        let age = 0;\n        \
                 let sex = 0;\n        \
                 if make_new_character(skin: skin, age: age, sex: sex, species: 0) {\n            \
                 temp[0] = p.attr[0];\n            temp[1] = stack_obj.neighbor.age;\n        }\n    }",
                "t",
            );
            assert!(t.contains("__pre_stack_obj"), "{t}");
        }
    }
}
