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
