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
            &format!(
                "unknown functional kind '{old}': the function this name used to give is '{new}'"
            ),
        );
    }
    // v2 refuses a search the engine never answers.
    expect_err(
        &obj(
            "    main { let Some(_o) = find_best_object(stand) else { return; }; loop { sleep(1); } }",
        ),
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
    assert!(
        air.contains("my.data[64] := 7;"),
        "score word 64 missing:\n{air}"
    );
    let finder = obj(
        "    main { let Some(_o) = find_best_object(gardening) else { return; }; loop { sleep(1); } }",
    );
    let (air, _, _) = build(&finder);
    assert!(air.contains("find_best_object(gardening"), "{air}");
}

#[test]
fn local_extern_globals_merge_with_std() {
    let body = |externs: &str| {
        format!(
            "object \"Syntax Probe\" {{
    guid: 0x7F50_0001;
    state n: i16;
{externs}    interaction \"Use\" {{
        run(sim) {{
            globals::standard_entry()?;
            globals::robot_hum(2)?;
            if !globals::am_i_a_child() {{
                self.n = 1;
            }}
            globals::standard_exit(3)?;
        }}
    }}
}}
"
        )
    };
    let copied = "extern mod globals {
    #[id(272)] fn standard_entry() -> Result;
    #[id(273)] fn standard_exit(delay: i16) -> Result;
    #[id(363)] fn am_i_a_child() -> bool;
    #[id(4000)] fn robot_hum(level: i16) -> Result;
}

";
    let expected = format!("{copied}{}", body(""));
    // A top-level local block and an object-member block both add to std's.
    let local = "extern mod globals {
    #[id(4000)] fn robot_hum(level: i16) -> Result;
    #[id(272)] fn standard_entry() -> Result;
}

";
    assert_same(
        "top-level block merges",
        &format!("use std::globals::*;\n\n{local}{}", body("")),
        &expected,
    );
    assert_same(
        "object block merges",
        &format!(
            "use std::globals::*;\n\n{}",
            body(
                "    extern mod globals {\n        #[id(4000)] fn robot_hum(level: i16) -> Result;\n    }\n"
            )
        ),
        &expected,
    );
    // The same tree under a second name is fine when it is called the same way.
    assert_same(
        "alias of one tree",
        &format!(
            "use std::globals::*;\n\n{}",
            body(
                "    extern mod globals {\n        #[id(4000)] fn robot_hum(level: i16) -> Result;\n        #[id(272)] fn enter() -> Result;\n    }\n"
            )
        ),
        &expected,
    );
    // Genuine conflicts are refused, naming both declarations.
    expect_err(
        &format!(
            "use std::globals::*;\n\n{}",
            body(
                "    extern mod globals {\n        #[id(4000)] fn robot_hum(level: i16) -> Result;\n        #[id(274)] fn standard_entry() -> Result;\n    }\n"
            )
        ),
        "extern mod 'globals' declares fn 'standard_entry' twice, differently: `#[id(274)] fn standard_entry() -> Result`",
    );
    expect_err(
        &format!(
            "use std::globals::*;\n\nextern mod globals {{\n    #[id(4000)] fn robot_hum(level: i16) -> Result;\n    #[id(273)] fn leave() -> Result;\n}}\n\n{}",
            body("")
        ),
        "extern mod 'globals' calls tree 273 two ways",
    );
}

#[test]
fn run_functional_on_stack_obj_keeps_a_found_cursor() {
    // The callee runs in its own frame, so `stack_obj.run_functional(..)` leaves the tree's
    // `stack_obj` alone: no entry restore may replace the object Find Best Object found.
    let src = |call: &str| {
        format!(
            "object \"Syntax Probe\" {{
    guid: 0x7F50_0001;
    state n: i16;

    #[bhav(id = 4100, label = \"clean one\")]
    fn clean_one() -> bool {{
        stack_obj = 0;
        raw {{ if !find_best_object(clean, flags: 0) {{ return false; }} }}
        {call}
        stack_obj.data[25] = 50;
        if self.n > 0 {{
            return true;
        }}
        false
    }}
}}
"
        )
    };
    let (air, _, _) = build(&src("stack_obj.run_functional(clean);"));
    assert!(!air.contains("__saved_stack_obj"), "{air}");
    assert_same(
        "statement",
        &src("stack_obj.run_functional(clean);"),
        &src("raw { run_functional_tree(clean); }"),
    );
    // (The raw form prints on one line; the trees are the same.)
    assert!(
        build(&src(
            "if !stack_obj.run_functional(clean) { return false; }"
        ))
        .1 == build(&src(
            "raw { if !run_functional_tree(clean) { return false; } }"
        ))
        .1,
        "condition: IFF bytes differ"
    );
}

#[test]
fn me_reads_like_the_sim_handle_in_inherit_trees() {
    // In a `context = inherit` tree, `me` is whichever object runs it: its typed fields and
    // methods lower to the same `my.` cells as the Sim handle's.
    let src = |body: &str| {
        format!(
            "object \"Syntax Probe\" {{
    guid: 0x7F50_0001;
    state n: i16;
    sound ding = preset(\"ui_click\", wav: \"probe_ding\");

    #[bhav(id = 4100, label = \"me probe\", context = inherit)]
    fn me_probe() -> bool {{
{body}
        true
    }}
}}
"
        )
    };
    let (air, _, _) = build(&src("        if me.job_level == 0 { return false; }
        if me.skill.body > 300 { return false; }
        me.motive.mood = me.priority;
        temp[0] = me.id;
        temp[1] = me.personality.nice;
        me.play_sound(ding);
        me.refresh(2);"));
    for want in [
        "if my.person[57] == 0 {",
        "if my.person.body > 300 {",
        "my.motive.mood := my.person.priority;",
        "temp[0] := my.id;",
        "temp[1] := my.person[2];",
        "play_sound(1, false);",
        "refresh(target: me, mode: 2);",
    ] {
        assert!(air.contains(want), "missing `{want}`:\n{air}");
    }
    // `me` still yields to a local of that name.
    expect_err(
        &src("        let me = 3;\n        temp[0] = me.job_level;"),
        "unknown field 'me.job_level' (nothing in scope has it)",
    );
}

#[test]
fn budget_methods_are_bool_expressions() {
    // `sim.debit_funds(..)` / `sim.credit_funds(..)` test like the call forms: in conditions,
    // under `!` and `&&`, as values and as tails, with no extra locals or nodes.
    let pair = |method_form: &str| {
        let call_form = method_form
            .replace("sim.debit_funds(", "debit_funds(")
            .replace("sim.credit_funds(", "credit_funds(");
        (obj(method_form), obj(&call_form))
    };
    for (case, body) in [
        (
            "condition",
            "    interaction \"Use\" {
        run(sim) {
            if !sim.credit_funds(self.n, category: 2) {
                return false;
            }
            if sim.debit_funds(5) && self.n > 0 {
                self.m = 1;
            }
            while sim.debit_funds(1) {
                self.m += 1;
                sleep(1);
            }
        }
    }",
        ),
        (
            "value",
            "    interaction \"Use\" {
        run(sim) {
            let paid = sim.debit_funds(5);
            if paid {
                self.m = 1;
            }
        }
    }",
        ),
        (
            "tail",
            "    fn pay(sim: Sim) -> bool {
        sim.credit_funds(5)
    }
    interaction \"Use\" {
        run(sim) {
            if !pay(sim) {
                return false;
            }
            self.m = 1;
        }
    }",
        ),
    ] {
        let (new, old) = pair(body);
        assert_same(case, &new, &old);
    }
    let (air, _, _) = build(&obj("    fn pay(sim: Sim) -> bool {
        sim.credit_funds(5)
    }
    interaction \"Use\" {
        run(sim) {
            if !pay(sim) {
                return false;
            }
            self.m = 1;
        }
    }"));
    assert!(!air.contains("let "), "a tail needs no local:\n{air}");
    expect_err(
        &obj("    interaction \"Use\" {
        run(sim) {
            if self.debit_funds(5) { self.m = 1; }
        }
    }"),
        "'.debit_funds()' is called on the interaction's Sim",
    );
}

#[test]
fn bare_exit_match_arms_are_statements() {
    let looped = |arms: &str| {
        obj(&format!(
            "    main {{
        loop {{
            match self.n {{
{arms}
            }}
            sleep(1);
        }}
        loop {{ sleep(1); }}
    }}"
        ))
    };
    assert_same(
        "break / continue arms",
        &looped(
            "                1 => break,\n                2 => continue,\n                _ => self.n += 1,",
        ),
        &looped(
            "                1 => { break; }\n                2 => { continue; }\n                _ => { self.n += 1; }",
        ),
    );
    let returns = |arms: &str| {
        obj(&format!(
            "    interaction \"Use\" {{
        run(sim) {{
            match self.n {{
{arms}
            }}
            self.m = 1;
        }}
    }}"
        ))
    };
    assert_same(
        "return arms",
        &returns(
            "                0 => return false,\n                1 => return,\n                _ => {}",
        ),
        &returns(
            "                0 => { return false; }\n                1 => { return; }\n                _ => {}",
        ),
    );
    // The formatter keeps the bare forms parseable and stable.
    let src = looped(
        "                1 => break,\n                2 => continue,\n                _ => self.n += 1,",
    );
    let once = v2::format_v2_source(&src).expect("formats");
    assert_eq!(v2::format_v2_source(&once).expect("formats again"), once);
    assert_same("formatted", &once, &src);
}

/// The `.air` of a synthetic object whose `msg` string is `message`, around `members`.
fn air_with_message(message: &str, members: &str) -> String {
    let src = format!(
        r#"object "Text Words" {{
    guid: 0x7F40_0A20;
    strings 301 "Notes" {{ title = "T", msg = "{message}" }}
{members}
}}
"#
    );
    build(&src).0
}

/// `$Me` writes a name into the text that later passes may read as a token, so it may read any
/// frame word: the unread entry save still drops when every other word keeps its local (the
/// save's own word holds no source local). Explicit tokens keep the strict rule: one that may
/// read the save's own word, or a word a named local would move to, keeps the save.
#[test]
fn me_text_drops_an_entry_save_that_moves_no_named_local() {
    // The save, by name or (under `#[slot]` placement) by word.
    let saves =
        |air: &str| air.contains("let __self_obj = stack_obj;") || air.contains("] := stack_obj;");
    // `n` is pinned below the save, so dropping the save moves no named local.
    let pinned = r#"    #[bhav]
    fn a(sim: Sim) {
        #[slot(0)] let n: i16;
        random_into(n, 4)?;
        if !set_to_next(guid, 0x7F40_0A21, dest: stack_obj) {
            return false;
        }
        stack_obj.attr[0] = n;
        dialog(kind: ok, title: title, message: msg);
    }"#;
    for message in ["Hello $Me", "$Me rolled $Local:0"] {
        let air = air_with_message(message, pinned);
        assert!(!saves(&air), "{message}:\n{air}");
    }
    // `$Local:1` is the save's own word.
    let air = air_with_message("$Me saw $Local:1", pinned);
    assert!(saves(&air), "{air}");
    // A name without `$` can still complete a token the source starts right before the `$Me`
    // (the name `1` makes `$Local:1`, and `$Local:0` then writes the digits `$TimeLocal:`
    // reads).
    for message in ["Saw $Local:$Me", "$TimeLocal:$Me$Local:0", "$Loc$Me"] {
        let air = air_with_message(message, pinned);
        assert!(saves(&air), "{message}:\n{air}");
    }
    // `n` declared after the save would move down a word.
    let moving = r#"    #[bhav]
    fn a(sim: Sim) {
        if !set_to_next(guid, 0x7F40_0A21, dest: stack_obj) {
            return false;
        }
        let n = sim.person[3];
        stack_obj.attr[0] = n;
        dialog(kind: ok, title: title, message: msg);
    }"#;
    let air = air_with_message("Hello $Me", moving);
    assert!(saves(&air), "{air}");
    // `.air` keeps `param[0]` around an `idle` in the word after the named locals, which
    // dropping the save would move.
    let scratch = r#"    #[bhav(args = 1)]
    fn a(sim: Sim) {
        if !set_to_next(guid, 0x7F40_0A21, dest: stack_obj) {
            return false;
        }
        stack_obj.attr[0] = param[0];
        sleep(1);
        dialog(kind: ok, title: title, message: msg);
    }"#;
    let air = air_with_message("Hello $Me", scratch);
    assert!(saves(&air), "{air}");
    // Without the `param[0]` read there is no scratch word.
    let air = air_with_message("Hello $Me", &scratch.replace("param[0]", "1"));
    assert!(!saves(&air), "{air}");
    // A string unbounded besides `$Me` keeps the save.
    let air = air_with_message("Hello $Object", pinned);
    assert!(saves(&air), "{air}");
}

/// A bool helper whose tail is a constant, returned through `return leave();` in a tree whose
/// raw block keeps the layout (so the helper is inlined for its tail condition), keeps its
/// constant test: the test sets the last result, which ending an active action records
/// (`release_use`), and the call before it may have left the other one.
#[test]
fn constant_helper_tails_keep_their_test_after_raw_code() {
    for (tail, test) in [("false", "if 1 == 0 {"), ("true", "if 1 == 1 {")] {
        let src = format!(
            r#"use std::globals::*;

object "Tail Probe" {{
    guid: 0x7F40_0A30;
    fn leave() -> bool {{
        globals::standard_exit(50)?;
        {tail}
    }}
    #[bhav(id = 4100)]
    fn t(sim: Sim) -> bool {{
        loop {{
            raw {{ if !find_best_object(8, flags: 0) {{ break; }} }}
            if sim.person[62] == 3 {{
                return leave();
            }}
            sim.person[62] += 1;
        }}
        leave()
    }}
}}
"#
        );
        let air = build(&src).0;
        let t = &air[air.find("tree 4100").expect("tree 4100")..];
        assert_eq!(t.matches(test).count(), 2, "{air}");
    }
}

/// A lint `allow` on its own line above an item's attributes covers the item, as it does
/// directly above the item; and `frame_headroom` suggests a `#[bhav] fn`.
#[test]
fn lint_allow_above_attributes_covers_the_item() {
    use sims_antics::v2::lint::lint_v2_source;
    let many_nodes: String = (0..205).map(|_| "        sim.person[3] += 1;\n").collect();
    let source = |allow: &str| {
        format!(
            "object \"Lint Probe\" {{\n    guid: 0x7F40_0A31;\n{allow}    #[bhav(label = \"Big\")]\n    fn big(sim: Sim) {{\n{many_nodes}    }}\n}}\n"
        )
    };
    let headroom = |src: &str| {
        lint_v2_source(src, "probe.antics")
            .unwrap_or_else(|e| panic!("lint failed: {e}"))
            .into_iter()
            .filter(|d| d.rule == "frame_headroom")
            .collect::<Vec<_>>()
    };
    let found = headroom(&source(""));
    assert_eq!(found.len(), 1, "{found:#?}");
    let help = found[0].help.as_deref().unwrap_or_default();
    assert!(
        help.contains("`#[bhav] fn`") && !help.contains("`tree`"),
        "{help}"
    );
    let found = headroom(&source("    // antics-lint: allow(frame_headroom)\n"));
    assert!(found.is_empty(), "{found:#?}");
}

/// Round 1 review regressions: a loop-local `me`, `me` in an object's own tree, a `bcon me`
/// table, an arm assigning a binding named `break`, and the untyped `category4` search.
#[test]
fn review_round_1_name_regressions() {
    // A loop-local `me` is a fresh cursor like any other name.
    let looped = |name: &str| {
        build(&format!(
            r#"object "Loop Probe" {{
    guid: 0x7F40_0A40;
    #[bhav]
    fn run(sim: Sim) {{
        for item in lot.objects() {{
            let {name}: ObjectId = sim.id;
            temp[0] = {name}.room;
            temp[1] = item.data[11];
        }}
    }}
}}
"#
        ))
    };
    assert_eq!(looped("me"), looped("other"));
    // In an object's own tree `me` is the object, which has no Sim fields.
    expect_err(
        "object \"Init Probe\" {\n    guid: 0x7F40_0A41;\n    init { temp[0] = me.job_level; }\n}\n",
        "unknown field 'me.job_level'",
    );
    // A `bcon me` table keeps its name there.
    let air = build(
        "extern bcon me { #[addr(16521)] probe; }\nobject \"Init Probe\" {\n    guid: 0x7F40_0A41;\n    init { temp[0] = me.probe; }\n}\n",
    )
    .0;
    assert!(air.contains("temp[0] := bcon[16521];"), "{air}");
    // `break` followed by an assignment is the binding, not the exit.
    let air = build(&obj(
        "    #[bhav]\n    fn a() {\n        let break: i16 = 0;\n        match self.n {\n            _ => break += 1,\n        }\n        temp[0] = break;\n    }",
    ))
    .0;
    assert!(air.contains("+= 1"), "{air}");
    // The untyped search takes `category4` (function 15), as raw `.air` does.
    let air = build(&obj(
        "    #[bhav]\n    fn a(sim: Sim) {\n        let Some(_o) = find_best_object(category4) else { return; };\n    }",
    ))
    .0;
    assert!(air.contains("find_best_object(category4)"), "{air}");
}

/// Extern merges across modules are refused when both sides name module types (they resolve in
/// different scopes), and every alias of a tree is checked: neither depends on import order.
#[test]
fn extern_merges_do_not_depend_on_import_order() {
    let compile = |modules: &[(&str, &str)], src: &str| {
        let mut cfg = PackResolverConfig::default();
        for (name, body) in modules {
            cfg.virtual_modules
                .insert((*name).to_owned(), (*body).to_owned());
        }
        v2::expand_v2_source_to_objects("case.antics", src, &cfg).map(|o| o[0].air_source.clone())
    };
    let a = "pub enum Mode { A = 10 }\npub extern mod bridge { #[id(4000)] fn act(mode: Mode) -> Result; }\n";
    let b = "pub enum Mode { B = 20 }\npub extern mod bridge { #[id(4000)] fn act(mode: Mode) -> Result; }\n";
    let ping = "pub extern mod bridge { #[id(4001)] fn ping() -> Result; }\n";
    let object =
        "object \"Bridge Probe\" {\n    guid: 0x7F40_0A42;\n    init { bridge::ping()?; }\n}\n";
    for (first, second) in [("a", "b"), ("b", "a")] {
        let src = format!(
            "use crate::{first}::bridge;\nuse crate::{second}::bridge;\n{}",
            object.replace("ping()", "act(1)")
        );
        let err = compile(&[("crate::a", a), ("crate::b", b)], &src).expect_err("two Mode types");
        assert!(
            err.to_string()
                .contains("both name types of their own module"),
            "{err}"
        );
    }
    let mut airs = Vec::new();
    for (first, second) in [("a", "p"), ("p", "a")] {
        let src = format!("use crate::{first}::bridge;\nuse crate::{second}::bridge;\n{object}");
        airs.push(
            compile(&[("crate::a", a), ("crate::p", ping)], &src)
                .unwrap_or_else(|e| panic!("{first} then {second}: {e}")),
        );
    }
    assert_eq!(airs[0], airs[1]);
    // Tree 4000 under `left(i16)` and `right()`: `middle(i16)` conflicts with `right` in
    // either order.
    let dep = "pub extern mod bridge { #[id(4000)] fn middle(mode: i16) -> Result; }\n";
    for local in [
        "extern mod bridge {\n    #[id(4000)] fn left(mode: i16) -> Result;\n    #[id(4000)] fn right() -> bool;\n}\n",
        "extern mod bridge {\n    #[id(4000)] fn right() -> bool;\n    #[id(4000)] fn left(mode: i16) -> Result;\n}\n",
    ] {
        let src = format!(
            "use crate::dep::bridge;\n{local}{}",
            object.replace("bridge::ping()?;", "if bridge::right() { temp[0] = 7; }")
        );
        let err = compile(&[("crate::dep", dep)], &src).expect_err("tree 4000 called two ways");
        assert!(
            err.to_string().contains("calls tree 4000 two ways"),
            "{err}"
        );
    }
}

/// Round 2 review regressions: `stack_obj.run_functional` with arguments keeps the found
/// cursor, a short arm's `return` binding, and the typed search's score check for every
/// scored kind.
#[test]
fn review_round_2_regressions() {
    let tree = |air: &str, name: &str| {
        let at = air.find(&format!("\"{name}\"")).expect("tree");
        let rest = &air[at..];
        rest[..rest.find("\n}\n").expect("tree end")].to_owned()
    };
    let air = build(&obj(r#"    #[bhav]
    fn a(sim: Sim) -> bool {
        raw { if !find_best_object(15) { return false; } }
        if !stack_obj.run_functional(4, 7) { return false; }
        stack_obj.data[25] = 1;
        return true;
    }
    #[bhav]
    fn b(sim: Sim, other: ObjectId) -> bool {
        raw { if !find_best_object(15) { return false; } }
        if !stack_obj.run_functional(4, other.data[3]) { return false; }
        stack_obj.data[25] = 1;
        return true;
    }
    #[bhav]
    fn c(sim: Sim) -> bool {
        raw { if !find_best_object(15) { return false; } }
        if !stack_obj.run_functional(4, set_to_next(guid, 0x7F40_0A53, dest: stack_obj)) {
            return false;
        }
        return true;
    }"#))
    .0;
    // A literal argument leaves the cursor alone.
    let a = tree(&air, "a");
    assert!(!a.contains("stack_obj :="), "{a}");
    // Argument code that moves the cursor: the found one is saved first and selected again.
    let b = tree(&air, "b");
    let saved = b.find("let __rf_obj_1 = stack_obj;").expect("cursor saved");
    let moved = b.find("stack_obj := param[0];").expect("argument read");
    let call = b.find("run_functional_tree(4)").expect("call");
    let reselect = b
        .find("stack_obj := __rf_obj_1;")
        .expect("cursor selected again");
    assert!(saved < moved && moved < reselect && reselect < call, "{b}");
    assert!(!b[call..].contains("stack_obj :="), "{b}");
    // A primitive argument that steps the cursor: the receiver is still the found one.
    let c = tree(&air, "c");
    let saved = c.find("let __rf_obj").expect("cursor saved");
    let step = c.find("set_to_next(").expect("argument step");
    let reselect = c
        .find("stack_obj := __rf_obj")
        .expect("cursor selected again");
    let call = c.find("run_functional_tree(4)").expect("call");
    assert!(saved < step && step < reselect && reselect < call, "{c}");
    // A binding named `return` assigned in a short arm, and `return <name>` of a binding whose
    // name is an assignment word.
    let air = build(&obj(
        "    #[bhav]\n    fn a(return: ObjectId, flag_set: bool) -> bool {\n        match self.n {\n            0 => return.person[3] += 1,\n            1 => return flag_set,\n            _ => {}\n        }\n        return true;\n    }",
    ))
    .0;
    assert!(air.contains("+= 1"), "{air}");
    // The typed search's fallback checks the kind's own score word (gardening: data word 64).
    let air = build(
        r#"object "Garden Bed" {
    guid: 0x7F40_0A51;
    functional gardening { score: 0, run(sim) { return; } }
    interaction "Tend" {
        run(sim) {
            let Some(bed) = find_best_object::<Self>(gardening) else { return; };
            bed.data[2] = 1;
        }
    }
}
"#,
    )
    .0;
    // Only a zero score is skipped, as the primitive skips it (a negative one still counts).
    assert!(air.contains("if stack.data[64] != 0 {"), "{air}");
}

/// Round 2 extern merge regressions: exact duplicates still check every alias, built-in types
/// merge across modules, and an object's block cannot take a module's typed declarations.
#[test]
fn extern_merge_round_2_regressions() {
    let compile = |modules: &[(&str, &str)], src: &str| {
        let mut cfg = PackResolverConfig::default();
        for (name, body) in modules {
            cfg.virtual_modules
                .insert((*name).to_owned(), (*body).to_owned());
        }
        v2::expand_v2_source_to_objects("case.antics", src, &cfg)
            .map(|o| o[0].air_source.clone())
            .map_err(|e| e.to_string())
    };
    let object =
        "object \"Bridge Probe\" {\n    guid: 0x7F40_0A52;\n    init { bridge::common(1)?; }\n}\n";
    let a = "pub extern mod bridge {\n    #[id(4000)] fn common(n: i16) -> Result;\n    #[id(4000)] fn alias() -> bool;\n}\n";
    let b = "pub extern mod bridge { #[id(4000)] fn common(n: i16) -> Result; }\n";
    for (first, second) in [("a", "b"), ("b", "a")] {
        let src = format!("use crate::{first}::bridge;\nuse crate::{second}::bridge;\n{object}");
        let err = compile(&[("crate::a", a), ("crate::b", b)], &src).expect_err("two ABIs");
        assert!(err.contains("calls tree 4000 two ways"), "{err}");
    }
    let take = "pub extern mod bridge { #[id(4000)] fn take(h: Height) -> Result; }\n";
    let put = "pub extern mod bridge { #[id(4001)] fn put(p: Placement) -> Result; }\n";
    let src = format!(
        "use crate::t::bridge;\nuse crate::p::bridge;\n{}",
        object.replace("bridge::common(1)?;", "bridge::ping_none();")
    );
    let err = compile(&[("crate::t", take), ("crate::p", put)], &src)
        .expect_err("ping_none is not declared");
    assert!(!err.contains("both name types"), "{err}");
    let typed = "pub enum Mode { Shared = 10 }\npub extern mod bridge { #[id(4000)] fn send(mode: Mode) -> Result; }\n";
    let src = "use crate::typed::bridge;\nuse crate::typed::Mode as ImportedMode;\nobject \"Bridge Probe\" {\n    guid: 0x7F40_0A52;\n    enum Mode { Local = 20 }\n    extern mod bridge { #[id(4001)] fn ping() -> Result; }\n    init { bridge::send(ImportedMode::Shared)?; }\n}\n";
    let err = compile(&[("crate::typed", typed)], src).expect_err("typed module block");
    assert!(err.contains("names a type of its module"), "{err}");
}

/// The `.air` body of tree `label` in `air` (from its header line to the closing brace).
fn tree_body<'a>(air: &'a str, label: &str) -> &'a str {
    let header = format!(" \"{label}\"");
    let start = air
        .lines()
        .scan(0, |at, line| {
            let here = *at;
            *at += line.len() + 1;
            Some((here, line))
        })
        .find(|(_, line)| line.contains(&header) && line.ends_with('{'))
        .map(|(at, _)| at)
        .unwrap_or_else(|| panic!("no tree '{label}':\n{air}"));
    let end = air[start..]
        .find("\n}")
        .map_or(air.len(), |e| start + e + 2);
    &air[start..end]
}

/// PR I-3: `remove_object(target)` / `target.remove()` is Remove Object Instance itself, with
/// the target's mode (0 me, 1 the stack object) and no other node or local.
#[test]
fn remove_object_names_me_or_the_stack_object() {
    assert_same(
        "call and method forms",
        &obj("    #[bhav]
    fn a() -> bool {
        remove_object(stack_obj, no_yield: true);
        if !remove_object(me) {
            return false;
        }
        remove_object(stack_obj, skip_resets: true, flags: 4)
    }"),
        &obj("    #[bhav]
    fn a() -> bool {
        stack_obj.remove(no_yield: true);
        if !me.remove() {
            return false;
        }
        stack_obj.remove(flags: 6)
    }"),
    );
    let air = build(&obj("    #[bhav]
    fn a() -> bool {
        remove_object(stack_obj, no_yield: true);
        if !remove_object(me) {
            return false;
        }
        remove_object(stack_obj, skip_resets: true, flags: 4)
    }"))
    .0;
    let body = tree_body(&air, "a");
    assert!(
        body.contains("    remove_object(stack_obj, flags: 1);\n    if !remove_object(me) {"),
        "{body}"
    );
    assert!(
        body.contains("if remove_object(stack_obj, flags: 6) {"),
        "{body}"
    );
    assert!(!body.contains("let "), "no local:\n{body}");
    // `self` is the running object in its own trees and the stack object in a Sim tree, as for
    // `destroy_self()`.
    assert_same(
        "self.remove() is destroy_self()",
        &obj("    #[bhav]
    fn a() { self.remove(no_yield: true); }
    interaction \"Use\" {
        run(sim) { remove_object(self); }
    }"),
        &obj("    #[bhav]
    fn a() { destroy_self(no_yield: true); }
    interaction \"Use\" {
        run(sim) { destroy_self(); }
    }"),
    );
    // A loop's stack-object binding is the stack object.
    let air = build(&obj("    #[bhav]
    fn a() {
        while set_to_next(person) {
            remove_object(stack_obj);
        }
    }"))
    .0;
    assert!(air.contains("remove_object(stack_obj);"), "{air}");

    expect_err(
        &obj("    #[bhav] fn a(o: ObjectId) { remove_object(o); }"),
        "select another object first ('stack_obj = x;') and remove 'stack_obj'",
    );
    expect_err(
        &obj("    #[bhav(context = inherit)] fn a() { self.remove(); }"),
        "'context = inherit' tree runs on whichever object calls it",
    );
    expect_err(
        &obj("    interaction \"Use\" { run(sim) { set_to_next(all); remove_object(self); } }"),
        "this Sim tree moves 'stack_obj', so 'self' is no longer the stack object",
    );
    expect_err(
        &obj("    #[bhav] fn a() { remove_object(flags: 1); }"),
        "takes its target first",
    );
    expect_err(
        &obj("    #[bhav] fn a() { remove_object(stack_obj, 1); }"),
        "remove_object() only takes named options",
    );
    expect_err(
        &obj("    #[bhav] fn a() { stack_obj.remove(flags: 256); }"),
        "remove flags 256 out of range 0..=255",
    );
    expect_err(
        &obj("    #[bhav] fn a() { remove_object(stack_obj, now: true); }"),
        "unknown remove_object option 'now'",
    );
}

/// PR I-3: untyped `find_best_object(kind)` is a condition: Find Best Object's own node, which
/// leaves the found object in `stack_obj` on its true exit, with no local. The tree then keeps
/// its cursor through later builtins, as for `set_to_next`.
#[test]
fn find_best_object_is_a_condition() {
    let air = build(&obj("    #[bhav]
    fn a() -> bool {
        if find_best_object(cook) {
            stack_obj.attr[0] = 1;
            return true;
        }
        if !find_best_object(cook, flags: 4) {
            return false;
        }
        stack_obj.attr[1] = 2;
        true
    }"))
    .0;
    let body = tree_body(&air, "a");
    assert!(
        body.contains("    if find_best_object(cook) {\n        stack.attr[0] := 1;"),
        "{body}"
    );
    assert!(
        body.contains("    if !find_best_object(cook, flags: 4) {\n        return false;"),
        "{body}"
    );
    assert!(!body.contains("let "), "no local:\n{body}");
    // In a Sim tree `self` stays the object the tree started on after the search.
    let air = build(&obj("    interaction \"Use\" {
        run(sim) {
            if find_best_object(cook) {
                stack_obj.attr[0] = 1;
                self.n = 2;
            }
        }
    }"))
    .0;
    let body = tree_body(&air, "Use");
    assert!(body.contains("let __self_obj = stack_obj;"), "{body}");
    assert!(
        body.contains("stack.attr[0] := 1;\n        stack_obj := __self_obj;"),
        "{body}"
    );
    // `let Some(x) = find_best_object(..)` still puts the cursor back itself, so it does not
    // make `stack_obj` the tree's own cursor: no `__pre_stack_obj` save around a later move.
    let air = build(&obj("    #[bhav]
    fn a(o: ObjectId) -> bool {
        let Some(x) = find_best_object(cook) else { return false; };
        self.n = o.attr[0];
        true
    }"))
    .0;
    assert!(!air.contains("__pre_stack_obj"), "{air}");
    expect_err(
        &obj(
            "    #[bhav] fn a() -> bool { if find_best_object::<Self>(cook) { return true; } false }",
        ),
        "find_best_object::<T>() binds the object it checks against T",
    );
}

/// PR I-3: `set_to_next(.., dest: x)` steps a local, `param[N]` or `temp[N]` (read as the
/// cursor and written with the next one); stepping one leaves `stack_obj` alone, so a Sim tree
/// keeps no entry save for it.
#[test]
fn set_to_next_steps_a_named_cell() {
    let air = build(&obj("    #[bhav(args = 2)]
    fn a() -> bool {
        let anchor = stack_obj;
        let c: ObjectId = 0;
        if !set_to_next(beside, dest: c, local: anchor) {
            return false;
        }
        if set_to_next(part, dest: param[1]) {
            temp[0] = 1;
        }
        set_to_next(neighbor, dest: temp[2]);
        if set_to_next(person, guid: 0x1234) {
            return true;
        }
        stack_obj = c;
        true
    }"))
    .0;
    let body = tree_body(&air, "a");
    for want in [
        "let __v_anchor_0 = stack_obj;",
        "if !set_to_next(9, dest: __v_c_1, local: __v_anchor_0) {",
        "if set_to_next(3, dest: param[1]) {",
        "set_to_next(5, dest: temp[2]);",
        "if set_to_next(1, guid: 0x00001234) {",
    ] {
        assert!(body.contains(want), "{want}:\n{body}");
    }
    // A Sim tree stepping a local keeps no `__self_obj`; stepping `stack_obj` does.
    let sim = |dest: &str| {
        build(&obj(&format!(
            "    interaction \"Use\" {{
        run(sim) {{
            let c: ObjectId = 0;
            while set_to_next(person, dest: {dest}) {{
                self.n += 1;
            }}
        }}
    }}"
        )))
        .0
    };
    let air = sim("c");
    assert!(
        air.contains("while set_to_next(1, dest: __v_c_0) {\n        stack.attr[0] += 1;"),
        "{air}"
    );
    assert!(!air.contains("__self_obj"), "{air}");
    assert!(sim("stack_obj").contains("let __self_obj = stack_obj;"));

    expect_err(
        &obj("    #[bhav] fn a(x: i16) { set_to_next(person, dest: x); }"),
        "parameter 'x' is immutable; step its word with 'dest: param[0]'",
    );
    expect_err(
        &obj(
            "    #[bhav] fn a() -> bool { let Some(o) = create_object::<Self>() else { return false; }; set_to_next(person, dest: o) }",
        ),
        "is a typed object handle",
    );
    expect_err(
        &obj("    #[bhav] fn a() { let b = true; set_to_next(person, dest: b); }"),
        "needs an i16 or ObjectId local",
    );
    expect_err(
        &obj("    #[bhav] fn a() { set_to_next(person, dest: self.n); }"),
        "set_to_next(dest:) steps 'stack_obj', a local, 'param[N]', or 'temp[N]'",
    );
}

/// PR I-3: an object's definition words 9 and 11 (`object_type`, `sub_index`), whether its
/// definition has a function (`has_function[N]`), and job data under the current temps
/// (`job_field(F)`) read the engine cells in place: on the stack object, with no local.
#[test]
fn definition_fields_and_job_fields_read_in_place() {
    let air = build(&obj("    #[bhav]
    fn a() -> bool {
        if stack_obj.object_type == 2 {
            return true;
        }
        while stack_obj.sub_index != 257 {
            set_to_next(part);
        }
        temp[0] = 3;
        temp[1] = 2;
        if job_field(3) < me.person[10] {
            return false;
        }
        self.n = job_field(1);
        stack_obj.has_function[26]
    }"))
    .0;
    let body = tree_body(&air, "a");
    for want in [
        "if scope(21, 9) == 2 {",
        "while scope(21, 11) != 257 {",
        "if scope(33, 3) < my.person[10] {",
        "my.attr[0] := scope(33, 1);",
        "if scope(35, 26) != 0 {",
    ] {
        assert!(body.contains(want), "{want}:\n{body}");
    }
    assert!(!body.contains("let "), "no local:\n{body}");
    // In a Sim tree `self` is the stack object; an object handle moves it there and back.
    let air = build(&obj("    interaction \"Use\" {
        run(sim) {
            if self.sub_index == -1 {
                self.n = 1;
            }
        }
    }
    #[bhav]
    fn b(o: ObjectId) {
        self.n = o.object_type;
    }"))
    .0;
    assert!(
        tree_body(&air, "Use").contains("if scope(21, 11) == -1 {"),
        "{air}"
    );
    assert!(
        tree_body(&air, "b").contains("stack_obj := param[0];\n    my.attr[0] := scope(21, 9);"),
        "{air}"
    );
    // A state of the same name wins.
    let air = build(&obj(
        "    state sub_index: i16;\n    #[bhav] fn a() { self.n = self.sub_index; }",
    ))
    .0;
    assert!(!tree_body(&air, "a").contains("scope(21"), "{air}");
    // Job data's level is copied before the career's staging when it reads the temps.
    let air = build(&obj("    #[bhav]
    fn a() { self.n = job_data(3, job_field(2), 5); }"))
    .0;
    assert!(
        tree_body(&air, "a").contains(
            "let __job_level_0 = scope(33, 2);\n    temp[0] := 3;\n    temp[1] := __job_level_0;"
        ),
        "{air}"
    );
    // A call's staged arguments copy job data first.
    let air = build(&obj("    #[bhav]
    fn a() { b(1, job_field(4)); }
    #[bhav]
    fn b(x: i16, y: i16) {}"))
    .0;
    let a = tree_body(&air, "a");
    assert!(a.contains("= scope(33, 4);"), "{a}");
    assert!(!a.contains("temp[1] := scope(33"), "{a}");

    expect_err(
        &obj("    #[bhav] fn a() { stack_obj.object_type = 3; }"),
        "'stack_obj.object_type' is read-only (word 9 of the object's definition)",
    );
    expect_err(
        &obj("    #[bhav] fn a(o: ObjectId) { o.has_function[2] = true; }"),
        "'has_function[N]' is read-only",
    );
    expect_err(
        &obj("    interaction \"Use\" { run(sim) { self.n = sim.object_type; } }"),
        "unknown Sim property 'sim.object_type'",
    );
    expect_err(
        &obj("    #[bhav] fn a() { self.n = job_field(); }"),
        "job_field(field) requires a field number",
    );
}

/// PR I-3: a tree whose source assigns `stack_obj` keeps a restore it ends with (the popped
/// frame's stack object is observable); the compiler's own exit restores still go.
#[test]
fn a_source_restore_before_the_exit_stays() {
    let air = build(&obj("    #[bhav(context = inherit)]
    fn a() {
        #[slot(0)]
        let saved = stack_obj;
        stack_obj = me.id;
        temp[0] = stack_obj.attr[1];
        stack_obj = saved;
    }
    #[bhav]
    fn b(o: ObjectId) {
        self.n = o.attr[0];
    }"))
    .0;
    assert!(
        tree_body(&air, "a").contains("    stack_obj := local[0];\n    return true;"),
        "{air}"
    );
    let b = tree_body(&air, "b");
    assert!(b.contains("stack_obj := param[0];"), "{b}");
    assert!(
        !b.contains("stack_obj := __saved_stack_obj;\n    return true;"),
        "{b}"
    );
}

/// PR I-3: a raw `global[NAME]` whose constant is not 34 cannot read global 34, so the tree
/// is not held to main's rules for it (here: the unread Sim entry save drops).
#[test]
fn a_constant_global_index_other_than_34_reads_no_global_34() {
    let src = |slot: i16| {
        format!(
            r#"const SLOT = {slot};
object "Global Probe" {{
    guid: 0x7F50_0003;
    state n: i16;
    #[bhav]
    fn a(sim: Sim) {{
        let n: i16;
        raw {{ temp[2] := global[SLOT]; }}
        set_to_next(all, dest: stack_obj);
        random_into(n, Limits.draw)?;
        temp[3] = n;
    }}
    bcon 4097 "Limits" {{ draw = 6 }}
}}
"#
        )
    };
    assert!(!build(&src(5)).0.contains("__self_obj"));
    assert!(build(&src(34)).0.contains("let __self_obj = stack_obj;"));
}
