//! Synthetic comparisons of inline helpers and their hand-inlined equivalents.

use sims_antics::{CompiledObject, PackResolverConfig, compile_v2_source};

const PROBE: u16 = 4300;

fn source(helpers: &str, args: &str, body: &str) -> String {
    format!(
        r#"object "Inline Probe" {{
    guid: 0x7F40_0501;
    state n: i16;
    state other: i16;
    const TEST_GUID = 0x7F40_0502;
    {helpers}
    @id({PROBE}) tree probe "Probe" {args} {{
        {body}
    }}
}}
"#
    )
}

fn compile(src: &str) -> CompiledObject {
    compile_v2_source("inline.antics", src, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{e}\n{src}"))
}

fn expand(src: &str) -> String {
    let air = &sims_antics::v2::expand_v2_source_to_objects(
        "inline.antics",
        src,
        &PackResolverConfig::default(),
    )
    .unwrap_or_else(|e| panic!("{e}\n{src}"))[0]
        .air_source;
    let start = air.find(&format!("tree {PROBE} ")).expect("probe tree");
    let tree = &air[start..];
    tree[..tree.find("\n}").expect("end of probe tree") + 2].to_owned()
}

fn assert_no_scaffolding(air: &str) {
    for prefix in ["__ret", "__fn_ret", "__arg", "__cond"] {
        assert!(!air.contains(prefix), "unexpected {prefix}:\n{air}");
    }
}

fn assert_inlined(helpers: &str, args: &str, with_helper: &str, hand_inline: &str, locals: u16) {
    let actual = source(helpers, args, with_helper);
    let expected = source("", args, hand_inline);
    let air = expand(&actual);
    assert_no_scaffolding(&air);
    assert_eq!(air, expand(&expected));
    let actual = compile(&actual);
    let expected = compile(&expected);
    let actual = &actual.behaviors[&PROBE];
    assert_eq!(actual.locals, locals);
    let nodes = |tree: &sims_antics::ir::BhavChunk| {
        tree.instructions
            .iter()
            .map(|i| (i.opcode, i.on_true, i.on_false, i.operands))
            .collect::<Vec<_>>()
    };
    assert_eq!(nodes(actual), nodes(&expected.behaviors[&PROBE]));
}

#[test]
fn void_return_breaks_out_of_the_helper() {
    assert_inlined(
        r#"fn write() {
            temp[1] = 2;
            if temp[0] == 0 { return; }
            temp[1] = 3;
        }"#,
        "",
        "write(); temp[2] = temp[1]; return true;",
        r#"loop {
            temp[1] = 2;
            if temp[0] == 0 { break; }
            temp[1] = 3;
            break;
        }
        temp[2] = temp[1];
        return true;"#,
        0,
    );
}

#[test]
fn early_and_conditional_bool_returns_target_the_caller_arms() {
    assert_inlined(
        r#"fn choose() -> bool {
            if temp[0] > 0 { return true; }
            temp[1] = 3;
            return temp[2] < 0;
        }"#,
        "",
        r#"if choose() { self.n = 4; } else { self.n = 5; }
        temp[3] = self.n;
        return true;"#,
        r#"loop {
            if temp[0] > 0 { self.n = 4; break; }
            temp[1] = 3;
            if temp[2] < 0 { self.n = 4; } else { self.n = 5; }
            break;
        }
        temp[3] = self.n;
        return true;"#,
        0,
    );
}

#[test]
fn negated_if_and_try_use_their_own_exit_polarities() {
    let helper = r#"fn ready() -> bool {
        if temp[0] == 0 { return false; }
        temp[1] = 3;
        return true;
    }"#;
    for (call, exit) in [
        ("if !ready() { return false; }", "return false;"),
        ("ready()?;", "return error;"),
    ] {
        assert_inlined(
            helper,
            "",
            &format!("{call} self.n = 2; return true;"),
            &format!("if temp[0] == 0 {{ {exit} }} temp[1] = 3; self.n = 2; return true;"),
            0,
        );
    }
}

#[test]
fn bool_tail_return_goes_straight_to_the_tree_exits() {
    assert_inlined(
        r#"fn ready() -> bool {
            if temp[0] == 0 { return false; }
            return temp[1] > 7 && temp[2] != 4;
        }"#,
        "",
        "return ready();",
        r#"if temp[0] == 0 { return false; }
        if temp[1] > 7 {
            if temp[2] != 4 { return true; }
            return false;
        }
        return false;"#,
        0,
    );
}

#[test]
fn a_literal_bool_argument_needs_no_test_or_slot() {
    assert_inlined(
        "fn choose(value: bool) -> bool { return value; }",
        "",
        "if choose(false) { self.n = 7; } else { self.n = 8; } return true;",
        "self.n = 8; return true;",
        0,
    );
}

#[test]
fn i16_returns_write_the_let_destination() {
    assert_inlined(
        "fn pick() -> i16 { if temp[0] > 0 { return 8; } return temp[1]; }",
        "",
        "let x = pick(); temp[2] = x; temp[3] = x; return true;",
        r#"let x: i16;
        loop {
            if temp[0] > 0 { x = 8; break; }
            x = temp[1];
            break;
        }
        temp[2] = x; temp[3] = x; return true;"#,
        1,
    );
}

#[test]
fn stable_cells_and_literals_are_substituted() {
    assert_inlined(
        "fn send(v: i16) { self.other = v; temp[4] = v; temp[5] = v; }",
        "(word: i16)",
        "send(temp[1]); send(word); send(self.n); send(16); return true;",
        r#"self.other = temp[1]; temp[4] = temp[1]; temp[5] = temp[1];
        self.other = word;
        temp[4] = word; temp[5] = word;
        self.other = self.n;
        temp[4] = self.n; temp[5] = self.n;
        self.other = 16;
        temp[4] = 16; temp[5] = 16;
        return true;"#,
        0,
    );
}

#[test]
fn argument_snapshots_survive_effects_and_mutable_aliases() {
    let cases = [
        (
            "fn inner(v: i16) { temp[1] = 5; temp[2] = v; temp[3] = v; }",
            "",
            "inner(temp[1]);",
            "temp[1]",
        ),
        (
            "fn inner(v: i16) { random_into(temp[1], 1); temp[2] = v; temp[3] = v; }",
            "",
            "inner(temp[1]);",
            "temp[1]",
        ),
        (
            "fn inner(v: i16) { param[0] = 5; temp[2] = v; temp[3] = v; }",
            "(word: i16)",
            "inner(word);",
            "param[0]",
        ),
        (
            "fn inner(v: i16, r: &mut i16) { *r = 5; temp[2] = v; temp[3] = v; }",
            "",
            "inner(self.n, &mut self.n);",
            "my.attr[0]",
        ),
        (
            "fn inner(v: i16, actor: Sim) { stack_obj = actor.container; temp[2] = v; temp[3] = v; }",
            "(sim: Sim)",
            "inner(self.n, sim);",
            "stack.attr[0]",
        ),
        (
            "fn clobber() -> i16 { temp[1] = 5; return 6; }
             fn inner(v: i16, unused: i16) { temp[2] = v; temp[3] = v; }",
            "",
            "inner(temp[1], clobber());",
            "temp[1]",
        ),
    ];
    for (helper, args, call, cell) in cases {
        let src = source(helper, args, &format!("{call} return true;"));
        compile(&src);
        let air = expand(&src);
        let copy = air
            .lines()
            .find(|line| line.contains("let __arg_v_"))
            .unwrap_or_else(|| panic!("missing argument snapshot:\n{air}"));
        assert!(copy.ends_with(&format!("= {cell};")), "{air}");
    }
}

#[test]
fn sequential_helper_locals_reuse_slots_and_overlapping_ones_do_not() {
    let src = source(
        r#"fn first() {
            let a = temp[0];
            let live = temp[1];
            temp[2] = a;
            temp[3] = live;
            temp[4] = a;
            temp[5] = live;
        }
        fn second() {
            let different = temp[2];
            temp[3] = different;
            temp[4] = different;
        }"#,
        "",
        "first(); second(); first(); return true;",
    );
    let air = expand(&src);
    assert_no_scaffolding(&air);
    assert_eq!(compile(&src).behaviors[&PROBE].locals, 2, "{air}");
    assert_eq!(air.matches("let __inline_v_").count(), 2, "{air}");
}

#[test]
fn copies_of_a_caller_arm_share_its_locals_and_bind_in_the_caller() {
    let src = source(
        r#"fn choose() -> bool {
            let caller = 600;
            if temp[0] > 0 { return true; }
            return temp[1] > caller;
        }"#,
        "",
        r#"let caller = temp[2];
        if choose() {
            let a = temp[3];
            temp[4] = a;
            temp[5] = a;
            temp[6] = caller;
        } else {
            temp[6] = caller;
        }
        return true;"#,
    );
    let air = expand(&src);
    assert_no_scaffolding(&air);
    assert_eq!(
        air.lines().filter(|l| l.contains("let __v_a_")).count(),
        1,
        "{air}"
    );
    assert_eq!(air.matches("temp[4] := __v_a_").count(), 2, "{air}");
    assert!(!air.contains("temp[6] := 600"), "{air}");
    compile(&src);
}

#[test]
fn return_through_a_loop_and_cancellation_keep_their_fallbacks() {
    let inner_loop = source(
        r#"fn choose() -> bool {
            repeat 2 { if temp[0] > 0 { return true; } }
            return false;
        }"#,
        "",
        "if choose() { self.n = 1; } return true;",
    );
    let air = expand(&inner_loop);
    // The liveness pass can reuse the caller's condition slot for the propagation flag.
    assert!(
        air.contains("\n        if __") && air.contains("\n            break;"),
        "{air}"
    );
    assert!(air.contains("__ret_choose"), "{air}");
    compile(&inner_loop);

    let cancel = source(
        r#"fn choose() -> bool {
            sleep(1);
            if temp[0] > 0 { return true; }
            return false;
        }"#,
        "(sim: Sim)",
        "repeat 2 cancel_on_abort { if choose() { self.n = 1; } } return true;",
    );
    let air = expand(&cancel);
    assert!(
        air.contains("__abort") && air.contains("__ret_choose"),
        "{air}"
    );
    compile(&cancel);
}

#[test]
fn helper_and_caller_defers_run_after_the_bool_is_decided() {
    assert_inlined(
        r#"fn choose() -> bool {
            defer { temp[0] = 9; }
            return temp[0] > 0;
        }"#,
        "",
        "defer { temp[0] = 8; } return choose();",
        r#"if temp[0] > 0 {
            temp[0] = 9;
            temp[0] = 8;
            return true;
        }
        temp[0] = 9;
        temp[0] = 8;
        return false;"#,
        0,
    );
}

#[test]
fn loop_exits_and_raw_arms_use_ordinary_if_lowering() {
    for call in [
        "loop { if choose() { break; } self.n += 1; } return true;",
        "if choose() { raw { temp[0] := 1; } } return true;",
    ] {
        let src = source(
            "fn choose() -> bool { if temp[0] > 0 { return true; } return false; }",
            "",
            call,
        );
        let air = expand(&src);
        assert!(air.contains("__ret_choose"), "{air}");
        compile(&src);
    }
}

#[test]
fn guid_parameters_forward_literals_and_default_consts() {
    assert_inlined(
        r#"fn inner(g: Guid) { manage_inventory(op: 0, kind: 0, flags: 0, guid: g); }
        fn outer(g: Guid = TEST_GUID) { inner(g); }"#,
        "",
        "outer(); outer(0x7F40_0503); return true;",
        r#"manage_inventory(op: 0, kind: 0, flags: 0, guid: 0x7F40_0502);
        manage_inventory(op: 0, kind: 0, flags: 0, guid: 0x7F40_0503);
        return true;"#,
        0,
    );
    let cfg = PackResolverConfig::default();
    for (body, expected) in [
        ("use_guid(temp[0]);", "expected a GUID literal"),
        ("use_guid(0x1_0000_0000);", "out of u32 range"),
        (
            "let TEST_GUID = temp[0]; use_guid(TEST_GUID);",
            "found 'TEST_GUID'",
        ),
    ] {
        let src = source("fn use_guid(g: Guid) {}", "", body);
        let err = compile_v2_source("inline.antics", &src, &cfg).expect_err("invalid Guid");
        assert!(err.message.contains(expected), "{err}");
    }
    let src = source(
        "fn use_guid(g: Guid) { g = 1; }",
        "",
        "use_guid(TEST_GUID); return true;",
    );
    let err = compile_v2_source("inline.antics", &src, &cfg).expect_err("immutable Guid");
    assert!(err.message.contains("'g' is not assignable"), "{err}");
}

#[test]
fn result_helper_error_remains_the_tree_error_exit() {
    let helper = r#"fn check() -> Result {
            if temp[0] > 0 { return error; }
            if temp[1] > 0 { return false; }
            return true;
        }"#;
    assert_inlined(
        helper,
        "",
        "return check();",
        r#"if temp[0] > 0 { return error; }
        if temp[1] > 0 { return false; }
        return true;"#,
        0,
    );
    assert_inlined(
        helper,
        "",
        "check()?; self.n = 1; return true;",
        r#"if temp[0] > 0 { return error; }
        if temp[1] > 0 { return error; }
        self.n = 1;
        return true;"#,
        0,
    );
}

#[test]
fn an_existing_enum_named_result_remains_a_value_type() {
    let src = source(
        r#"enum Result { Missing, Present }
        fn classify() -> Result { return Result::Present; }"#,
        "",
        "let outcome = classify(); return outcome == Result::Present;",
    );
    compile(&src);
}
