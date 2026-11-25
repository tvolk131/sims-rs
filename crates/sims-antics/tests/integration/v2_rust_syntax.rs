//! `.antics` v2 Rust-like syntax (PR I-1): every new spelling compiles to exactly the trees and
//! the IFF its long-hand form does. Each case pairs a source using the new syntax with the
//! statement form it replaces; the pair must give the same `.air`, the same IFF bytes and the
//! same sound bank. All sources are synthetic.

use sims_antics::{PackResolverConfig, compile_v2_source, v2};

/// `.air`, IFF bytes and WAV bank (event name, bytes) of one build.
type Built = (String, Vec<u8>, Vec<(String, Vec<u8>)>);

/// `.air`, IFF bytes and WAV bank of `src` (compiled under one fixed source name).
fn build(src: &str) -> Built {
    let cfg = PackResolverConfig::default();
    let air = v2::expand_v2_source_to_objects("case.antics", src, &cfg)
        .unwrap_or_else(|e| panic!("must expand: {e}\n{src}"))
        .into_iter()
        .next()
        .expect("one object")
        .air_source;
    let compiled = compile_v2_source("case.antics", src, &cfg)
        .unwrap_or_else(|e| panic!("must compile: {e}\n{src}"));
    (
        air,
        compiled.iff_bytes,
        compiled.sounds_wav.into_iter().collect(),
    )
}

fn assert_same(case: &str, new: &str, old: &str) {
    let (new_air, new_iff, new_wav) = build(new);
    let (old_air, old_iff, old_wav) = build(old);
    assert_eq!(new_air, old_air, "{case}: .air differs");
    assert!(new_iff == old_iff, "{case}: IFF bytes differ");
    assert!(new_wav == old_wav, "{case}: sound bank differs");
}

fn expect_err(src: &str, needle: &str) {
    let cfg = PackResolverConfig::default();
    let err = v2::expand_v2_source_to_objects("case.antics", src, &cfg)
        .err()
        .unwrap_or_else(|| panic!("must fail with `{needle}`:\n{src}"))
        .to_string();
    assert!(
        err.contains(needle),
        "expected `{needle}`, got `{err}`\n{src}"
    );
}

/// A synthetic object with states `n`, `m` around `members`.
fn obj(members: &str) -> String {
    format!(
        r#"object "Syntax Probe" {{
    guid: 0x7F50_0001;
    state n: i16;
    state m: i16;
{members}
}}
"#
    )
}

#[test]
fn diverging_let_arms_match_the_statement_form() {
    assert_same(
        "let if",
        &obj("    interaction \"Use\" {
        run(sim) {
            let v = if self.n > 3 { self.n - 3 } else { return false; };
            self.m = v;
        }
    }"),
        &obj("    interaction \"Use\" {
        run(sim) {
            let v: i16;
            if self.n > 3 {
                v = self.n - 3;
            } else {
                return false;
            }
            self.m = v;
        }
    }"),
    );
    assert_same(
        "let match in a loop",
        &obj("    main {
        loop {
            let step = match self.n { 0 => break, 1 => continue, _ => self.n };
            self.m += step;
            sleep(1);
        }
    }"),
        &obj("    main {
        loop {
            let step: i16;
            match self.n {
                0 => { break; }
                1 => { continue; }
                _ => { step = self.n; }
            }
            self.m += step;
            sleep(1);
        }
    }"),
    );
}

#[test]
fn diverging_branches_in_nested_chains() {
    // A `break` in an `else if` value branch leaves the user's loop, not a loop the
    // short-circuit `||` lowering adds.
    assert_same(
        "break in a nested chain",
        &obj("    main {
        loop {
            let x = if self.n == 0 || self.n == 1 { 10 } else if self.n == 2 { break; } else { 20 };
            self.n = x;
        }
        loop { sleep(1); }
    }"),
        &obj("    main {
        loop {
            let x: i16;
            if self.n == 0 || self.n == 1 { x = 10; } else if self.n == 2 { break; } else { x = 20; }
            self.n = x;
        }
        loop { sleep(1); }
    }"),
    );
    // A nested chain whose every branch leaves is fine: the outer chain has the value.
    assert_same(
        "all-leaving else if",
        &obj("    interaction \"Use\" {
        run(sim) {
            let x = if self.n == 0 { 20 } else if self.n == 1 { return true; } else { return false; };
            self.n = x;
        }
    }"),
        &obj("    interaction \"Use\" {
        run(sim) {
            let x: i16;
            if self.n == 0 { x = 20; } else if self.n == 1 { return true; } else { return false; }
            self.n = x;
        }
    }"),
    );
    // A raw `break` or label branch can leave an endless loop, so the branch may fall through;
    // a raw `continue` keeps it looping.
    for raw in [
        "break;",
        "random(temp[0], 2) -> true: @after, false: @after;",
    ] {
        expect_err(
            &obj(&format!(
                "    main {{
        let x = if self.n == 0 {{ loop {{ raw {{ {raw} }} }} }} else {{ 20 }};
        raw {{ @after: }}
        self.n = x;
        loop {{ sleep(1); }}
    }}"
            )),
            "may jump out of it",
        );
    }
    build(&obj("    main {
        let x = if self.n == 0 { loop { raw { continue; } } } else { 20 };
        self.n = x;
        loop { sleep(1); }
    }"));
    // `const` bindings cannot take `world` either.
    expect_err(
        &obj("    main {
        const world = if self.n == 0 { 1 } else { 2 };
        self.n = world;
        loop { sleep(1); }
    }"),
        "cannot be used as a const binding",
    );
    // A keyword that names a binding is a value when it alone fills the braces.
    for name in ["animate", "loop"] {
        assert_same(
            "keyword-named value",
            &obj(&format!(
                "    main {{
        let {name} = self.n;
        let x = if self.n == 0 {{ {name} }} else {{ 1 }};
        self.m = x;
        loop {{ sleep(1); }}
    }}"
            )),
            &obj(&format!(
                "    main {{
        let {name} = self.n;
        let x: i16;
        if self.n == 0 {{ x = {name}; }} else {{ x = 1; }}
        self.m = x;
        loop {{ sleep(1); }}
    }}"
            )),
        );
    }
}

#[test]
fn tail_expressions_are_returns() {
    assert_same(
        "tail",
        &obj("    fn bigger(a: i16, b: i16) -> i16 {
        if a > b { a } else { b }
    }
    fn is_big(a: i16) -> bool {
        a > 2
    }
    interaction \"Use\" {
        guard(sim) => self.n > 0,
        run(sim) {
            self.m = bigger(self.n, 4);
            return is_big(self.n);
        }
    }"),
        &obj("    fn bigger(a: i16, b: i16) -> i16 {
        if a > b {
            return a;
        } else {
            return b;
        }
    }
    fn is_big(a: i16) -> bool {
        return a > 2;
    }
    interaction \"Use\" {
        guard(sim) => self.n > 0,
        run(sim) {
            self.m = bigger(self.n, 4);
            return is_big(self.n);
        }
    }"),
    );
    let pick = |body: &str| {
        obj(&format!(
            "    fn pick(a: i16) -> i16 {{
        {body}
    }}
    main {{
        self.n = pick(self.m);
        loop {{ sleep(1); }}
    }}"
        ))
    };
    assert_same(
        "match tail",
        &pick("match a { 0 => 1, 1 => { 5 } _ => a + 2 }"),
        &pick("match a { 0 => { return 1; } 1 => { return 5; } _ => { return a + 2; } }"),
    );
    // A match arm ended by ';' is a statement, not the value.
    assert_same(
        "statement arm",
        &obj("    interaction \"Use\" {
        guard(sim) { match self.n { 0 => { return false; } _ => sleep(1); } return true; }
        run(sim) { return true; }
    }"),
        &obj("    interaction \"Use\" {
        guard(sim) { match self.n { 0 => { return false; } _ => { sleep(1); } } return true; }
        run(sim) { return true; }
    }"),
    );
    // A tail after a `const X = if ..` ends each specialized copy.
    assert_same(
        "specialized tail",
        &pick("const K = if self.n > 0 { 10 } else { 20 };\n        K + a"),
        &pick("const K = if self.n > 0 { 10 } else { 20 };\n        return K + a;"),
    );
    expect_err(
        &obj("    main { self.n = 1; self.m loop { sleep(1); } }"),
        "expected ';'",
    );
}

#[test]
fn typed_engine_fields_read_the_raw_cells() {
    assert_same(
        "world/object/person fields",
        &obj("    interaction \"Use\" {
        run(sim) {
            self.n = world.hour + world.minute;
            if self.broken == 0 {
                self.m = sim.job_level;
            }
        }
    }"),
        &obj("    interaction \"Use\" {
        run(sim) {
            self.n = global[0] + global[5];
            if self.data[15] == 0 {
                self.m = sim.person[57];
            }
        }
    }"),
    );
}

#[test]
fn std_globals_matches_a_copied_extern_block() {
    let body = "object \"Syntax Probe\" {
    guid: 0x7F50_0001;
    state n: i16;
    interaction \"Use\" {
        run(sim) {
            globals::standard_entry()?;
            if !globals::am_i_a_child() {
                self.n = 1;
            }
            globals::standard_exit(3)?;
        }
    }
}
";
    assert_same(
        "use std::globals::*",
        &format!("use std::globals::*;\n\n{body}"),
        &format!(
            "extern mod globals {{
    #[id(272)] fn standard_entry() -> Result;
    #[id(273)] fn standard_exit(delay: i16) -> Result;
    #[id(363)] fn am_i_a_child() -> bool;
}}

{body}"
        ),
    );
    expect_err(
        &format!("use std::nope::*;\n\n{body}"),
        "unknown built-in module",
    );
}

#[test]
fn world_cannot_name_an_import() {
    let mut cfg = PackResolverConfig::default();
    cfg.virtual_modules.insert(
        "crate::library".to_owned(),
        "pub extern bcon Tuning {\n    #[addr(16521)] hour;\n}\n".to_owned(),
    );
    let src = "use crate::library::Tuning as world;\n\nobject \"Syntax Probe\" {\n    guid: 0x7F50_0001;\n    state n: i16;\n    main { self.n = world.hour; loop { sleep(1); } }\n}\n";
    let err = v2::expand_v2_source_to_objects("case.antics", src, &cfg)
        .err()
        .unwrap_or_else(|| panic!("an import named 'world' must fail"))
        .to_string();
    assert!(err.contains("cannot name an import"), "{err}");
}

#[test]
fn guid_consts_resolve_like_literals() {
    assert_same(
        "guid const",
        "const PROBE_GUID = 0x7F50_0001;

object \"Syntax Probe\" {
    const RED_GUID = 0x7F50_0002;
    guid: PROBE_GUID;
    state n: i16;
    variant \"Red Probe\" {
        guid: RED_GUID;
        price: 5;
    }
}
",
        "object \"Syntax Probe\" {
    guid: 0x7F50_0001;
    state n: i16;
    variant \"Red Probe\" {
        guid: 0x7F50_0002;
        price: 5;
    }
}
",
    );
    expect_err(
        "object \"Syntax Probe\" { guid: NOPE; state n: i16; }",
        "unknown const 'NOPE' in 'guid:'",
    );
}

#[test]
fn bcon_paths_read_like_fields() {
    let src = |sep: &str| {
        obj(&format!(
            "    bcon 4097 \"Prices\" {{
        snack = 10,
        meal = 25,
    }}
    main {{
        self.n = Prices{sep}snack + Prices{sep}meal;
        if self.n > Prices{sep}meal {{
            self.m += prices{sep}snack;
        }}
        sleep(Prices{sep}snack);
        loop {{ sleep(1); }}
    }}"
        ))
    };
    assert_same("Prices::snack", &src("::"), &src("."));
}

#[test]
fn fwav_sounds_match_preset_ids() {
    let src = |a: &str, b: &str| {
        obj(&format!(
            "    sound ping = {a};
    sound pong = {b};
    interaction \"Use\" {{
        run(sim) {{
            play_sound(ping);
            play_sound(pong);
        }}
    }}"
        ))
    };
    assert_same(
        "fwav",
        &src("fwav(6, wav: \"probe_ping\")", "fwav(9)"),
        &src(
            "preset(\"ui_click\", wav: \"probe_ping\", id: 6)",
            "preset(\"ui_click\", id: 9)",
        ),
    );
}

#[test]
fn const_params_specialize_and_require_constants() {
    let src = |param: &str, calls: &str| {
        obj(&format!(
            "    fn pick({param}: i16) {{
        const X = if k == 1 {{ 10 }} else {{ 20 }};
        self.n = X;
    }}
    main {{
        {calls}
        loop {{ sleep(1); }}
    }}"
        ))
    };
    assert_same(
        "const param",
        &src("const k", "pick(1); pick(2);"),
        &src("k", "pick(1); pick(2);"),
    );
    expect_err(
        &src("const k", "pick(self.m);"),
        "needs a compile-time constant argument",
    );
    expect_err(
        &obj("    fn bad(const k: i16) { k = 2; }\n    main { bad(1); loop { sleep(1); } }"),
        "cannot be assigned or borrowed",
    );
    // A tunable `param` is a run-time BCON read, not a constant.
    expect_err(
        &obj("    param k_tuning: i16 = 1;
    fn pick(const k: i16) {
        const X = if k == 1 { 10 } else { 20 };
        self.n = X;
    }
    main { pick(k_tuning); loop { sleep(1); } }"),
        "needs a compile-time constant argument",
    );
}

#[test]
fn temp_range_loops_count_in_the_temp() {
    let (air, _, _) = build(&obj("    main {
        #[temp(0)]
        for i in 0..18 {
            self.n += i;
        }
        #[temp(1)]
        for j in (0..self.n).rev() {
            if j == 3 {
                continue;
            }
            self.m -= j;
        }
        loop { sleep(1); }
    }"));
    let (plain, _, _) = build(&obj("    main {
        for i in 0..18 {
            self.n += i;
        }
        for j in (0..self.n).rev() {
            if j == 3 {
                continue;
            }
            self.m -= j;
        }
        loop { sleep(1); }
    }"));
    // The same nodes, with the counters in temps instead of frame locals (a step is spelled
    // `scope(8, N)`, the same operand as `temp[N]`).
    let renamed = plain
        .replace("__v_j_2 -= 1;", "scope(8, 1) -= 1;")
        .replace("let __v_i_0 = ", "temp[0] := ")
        .replace("__v_i_0", "temp[0]")
        .replace("__range_1", "__range_0")
        .replace("let __v_j_2 = ", "temp[1] := ")
        .replace("__v_j_2", "temp[1]");
    assert_eq!(air, renamed);
    expect_err(
        &obj("    main { #[temp(0)] for i in 0..3 { temp[0] = 4; } loop { sleep(1); } }"),
        "writes its counter 'temp[0]'",
    );
    expect_err(
        &obj(
            "    main { #[temp(0)] for i in 0..3 { #[temp(0)] for k in 0..2 { self.n += k; } } loop { sleep(1); } }",
        ),
        "writes its counter 'temp[0]'",
    );
    expect_err(
        &obj("    main { #[temp(0)] for o in lot.sims() { } loop { sleep(1); } }"),
        "applies to range loops",
    );
    // A `defer` in the body, raw code (in either spelling of the cell), and a write that
    // repeats the loop's own step text after a discarded `continue` are all writes.
    for body in [
        "relationship(column: 1, dir: 0, flags: 0, value: temp[0]);",
        "self.n = distance_to(self, temp: 0);",
        "defer { temp[0] = 0; } self.n += i;",
        "raw { temp [ 0 ]:= 0; }",
        "raw { scope(8, 0) := 0; }",
        "if false { continue; } if i == 32767 { break; } temp[0] += 1;",
    ] {
        let src = obj(&format!(
            "    main {{ #[temp(0)] for i in 0..=32767 {{ {body} }} loop {{ sleep(1); }} }}"
        ));
        let cfg = PackResolverConfig::default();
        let err = v2::expand_v2_source_to_objects("case.antics", &src, &cfg)
            .err()
            .unwrap_or_else(|| panic!("must fail:\n{src}"))
            .to_string();
        assert!(
            err.contains("writes its counter")
                || err.contains("contains 'raw")
                || err.contains("may write it"),
            "{body}: {err}"
        );
    }
}

#[test]
fn methods_match_the_call_forms() {
    assert_same(
        "methods",
        &obj("    sound ping = fwav(3);
    interaction \"Use\" {
        run(sim) {
            self.play_sound(ping);
            self.refresh(0);
            sim.debit_funds(5, category: 2);
        }
    }"),
        &obj("    sound ping = fwav(3);
    interaction \"Use\" {
        run(sim) {
            play_sound(ping);
            refresh(target: self, mode: 0);
            debit_funds(5, category: 2);
        }
    }"),
    );
}

/// IFF bytes of a synthetic `.air` object whose Init tree is `body`.
fn air_iff(body: &str) -> Vec<u8> {
    let manifest = "[object]\nname = \"Kind Probe\"\nguid = 0x7F50_0002\ninit = \"Init\"\n";
    let src = format!("tree 4096 \"Init\" {{ {body} return true; }}");
    sims_antics::compile_object_from_sources(manifest, &[("k.antics", &src)])
        .unwrap_or_else(|e| panic!("must compile: {e}\n{src}"))
        .iff_bytes
}

#[test]
fn functional_kind_names_follow_the_engine_table() {
    // Run Functional Tree / Find Best Object function numbers (sims-vm
    // `run_functional_tree.rs` `callback`, `find_best_object.rs` `wanted`).
    let kinds = [
        ("prep", 0),
        ("cook", 1),
        ("surface", 2),
        ("dispose", 3),
        ("eat", 4),
        ("pickup_from_slot", 5),
        ("wash_dish", 6),
        ("eating_surface", 7),
        ("sit", 8),
        ("stand", 9),
        ("serving_surface", 10),
        ("clean", 11),
        ("gardening", 12),
        ("wash_hands", 13),
        ("repair", 14),
    ];
    for (name, n) in kinds {
        assert!(
            air_iff(&format!("run_functional_tree({name});"))
                == air_iff(&format!("run_functional_tree({n});")),
            "run_functional_tree({name}) is not function {n}"
        );
        if !matches!(name, "eat" | "pickup_from_slot" | "stand") {
            assert!(
                air_iff(&format!("find_best_object({name});"))
                    == air_iff(&format!("find_best_object({n});")),
                "find_best_object({name}) is not function {n}"
            );
        }
    }
    assert!(air_iff("find_best_object(category4);") == air_iff("find_best_object(15);"));

    // The names an off-by-one table used are gone, with a pointer to the new name.
    for (old, new) in [
        ("serve", "gardening"),
        ("rest", "repair"),
        ("pick_up", "eating_surface"),
    ] {
        expect_err(
            &obj(&format!(
                "    main {{ let Some(_o) = find_best_object({old}) else {{ return; }}; loop {{ sleep(1); }} }}"
            )),
            &format!("unknown functional kind '{old}': the function this name used to give is '{new}'"),
        );
    }
    // v2 refuses a search the engine never answers.
    expect_err(
        &obj("    main { let Some(_o) = find_best_object(stand) else { return; }; loop { sleep(1); } }"),
        "find_best_object(stand) always fails",
    );
    // A v2 `functional gardening` is OBJf entry 16, found by function 12, scored by word 64.
    let garden = r#"object "Garden Bed" {
    guid: 0x7F50_0003;
    functional gardening {
        score: 7,
        run(sim) {
            return;
        }
    }
    main { loop { sleep(1); } }
}
"#;
    let (air, _, _) = build(garden);
    assert!(air.contains("my.data[64] := 7;"), "score word 64 missing:\n{air}");
    let finder = obj("    main { let Some(_o) = find_best_object(gardening) else { return; }; loop { sleep(1); } }");
    let (air, _, _) = build(&finder);
    assert!(air.contains("find_best_object(gardening"), "{air}");
}
