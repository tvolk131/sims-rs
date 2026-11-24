//! `.antics` v2 PR G: pilot bugfixes, zero-cost lowering, and missing builtins. All sources are
//! synthetic.

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
        r#"object "Pr G" {{
    guid: 0x7F40_0240;
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

fn sim_tree(body: &str) -> String {
    obj(&format!("    tree t (sim: Sim) {{ {body} }}"))
}

#[test]
fn v1_slot_primitives_bind_every_operand_by_name() {
    let air = expand(
        "v1.antics",
        &sim_tree(
            "route_to(slot: 0, source: 0, flags: 0); \
             route_to(front, source: global, flags: 2); \
             route_to(flags: 4, slot: front); \
             route_to(front); \
             snap(2, mode: front); \
             snap(slot: 1, mode: 3, flags: 1);",
        ),
    );
    for needle in [
        // `source: 0` is the routing source, never the flags.
        "route_to(0, source: 0);",
        "route_to(0, flags: 2, source: 2);",
        "route_to(0, flags: 4);",
        "route_to(0);",
        "snap(2, mode: 2);",
        "snap(1, flags: 1);",
    ] {
        assert!(air.contains(needle), "missing '{needle}':\n{air}");
    }
}

#[test]
fn v1_builtin_arguments_are_strict() {
    for (name, body, needle) in [
        (
            "route_unknown.antics",
            "route_to(slot: 0, bogus: 1);",
            "unknown route_to() argument 'bogus'",
        ),
        (
            "route_dup.antics",
            "route_to(front, flags: 1, flags: 2);",
            "duplicate route_to() argument 'flags'",
        ),
        (
            "route_dup_positional.antics",
            "route_to(front, slot: 0);",
            "duplicate route_to() argument 'slot'",
        ),
        (
            "route_positional_after_named.antics",
            "route_to(flags: 2, front);",
            "positional argument cannot follow a named argument in route_to()",
        ),
        (
            "route_too_many.antics",
            "route_to(front, 2, 1);",
            "too many positional arguments to route_to()",
        ),
        (
            "route_source_range.antics",
            "route_to(front, source: 5);",
            "route_to source",
        ),
        (
            "snap_mode_name.antics",
            "snap(0, mode: sideways);",
            "unknown snap mode 'sideways'",
        ),
        (
            "snap_source.antics",
            "snap(0, source: 1);",
            "unknown snap() argument 'source'",
        ),
        (
            "motive_named.antics",
            "set_motive_change(motive: 0, delta: 1, stop_at: 2);",
            "unknown set_motive_change() argument 'delta'",
        ),
        (
            "budget_named_amount.antics",
            "debit_funds(category: 3);",
            "budget call requires an amount argument",
        ),
        (
            "budget_dup.antics",
            "debit_funds(5, category: 1, category: 2);",
            "duplicate debit_funds() argument 'category'",
        ),
        (
            "stop_sounds_extra.antics",
            "stop_all_sounds(sim, 1);",
            "too many positional arguments to stop_all_sounds()",
        ),
        (
            "find_best_dup.antics",
            "let Some(o) = find_best_object(prep, flags: 4, flags: 0) else { return; };",
            "duplicate find_best_object() argument 'flags'",
        ),
        (
            "create_dup.antics",
            "create_object(0x7F40_0241, flags: 1, flags: 2);",
            "duplicate create_object() argument 'flags'",
        ),
    ] {
        expect_err(name, &sim_tree(body), needle);
    }
}

#[test]
fn n2_entry_save_only_when_the_entry_stack_object_is_read_after_a_move() {
    let tree = |body: &str| {
        obj(&format!(
            "    extern mod g {{ @id(300) fn probe() -> bool; }}\n    tree t (sim: Sim) -> bool {{ {body} }}"
        ))
    };
    // A call comes first and the loop moves `stack_obj`, but nothing reads the entry stack
    // object afterwards: no save, so the call sees the frame's entry state.
    let air = expand(
        "n2_unread.antics",
        &tree(
            "if g::probe() { return false; } \
             let count = 0; \
             for p in lot.sims() { if p.kind < 2 { count += 1; } } \
             if count > 2 { return true; } \
             return false;",
        ),
    );
    assert!(!air.contains("__self_obj"), "{air}");
    assert!(air.contains("\"t\" {\n    if call 300(temps) {"), "{air}");

    // Reading `self` after the loop needs the save and the restore.
    let air = expand(
        "n2_read.antics",
        &tree(
            "if g::probe() { return false; } \
             for p in lot.sims() { } \
             self.n = 1; \
             return true;",
        ),
    );
    assert!(air.contains("let __self_obj = stack_obj;"), "{air}");
    assert!(air.contains("stack_obj := __self_obj;"), "{air}");

    // A raw primitive that cannot write the stack object (Change Suit) needs no save before a
    // later read of `self`; an Expression storing into scope 10 does.
    let raw_then_self = |op: &str| {
        tree(&format!(
            "raw {{ {op} }} let o = self; if o == sim {{ return true; }} return false;"
        ))
    };
    let air = expand(
        "n2_raw_keeps.antics",
        &raw_then_self("op 0x0006 (1, 0, 1, 0, 0, 0, 0, 0);"),
    );
    assert!(!air.contains("__self_obj"), "{air}");
    let air = expand(
        "n2_raw_writes.antics",
        &raw_then_self("op 0x0002 (0, 0, 0, 0, 0, 5, 10, 7);"),
    );
    assert!(air.contains("let __self_obj = stack_obj;"), "{air}");
}

#[test]
fn empty_frame_tree_has_zero_nodes_and_plain_braces_still_return_true() {
    let cfg = PackResolverConfig::default();
    let src = obj(
        "    @id(4200) @frame(empty) tree unused \"Unused\" (sim: Sim, a: i16) {}\n    \
         @id(4201) tree plain \"Plain\" {}\n    \
         @id(4202) @frame(args: 2, empty, locals: 1) tree pinned \"Pinned\" {}",
    );
    let air = expand("empty.antics", &src);
    assert!(air.contains("tree 4200 \"Unused\" (args: 1) {\n}"), "{air}");
    assert!(
        air.contains("tree 4201 \"Plain\" {\n    return true;\n}"),
        "{air}"
    );
    assert!(
        air.contains("tree 4202 \"Pinned\" (args: 2, locals: 1) {\n}"),
        "{air}"
    );
    let compiled = compile_v2_source("empty.antics", &src, &cfg).expect("compile");
    let nodes = |id: u16| {
        compiled
            .behaviors
            .get(&id)
            .unwrap_or_else(|| panic!("no BHAV {id}"))
            .instructions
            .len()
    };
    assert_eq!(nodes(4200), 0);
    assert_eq!(nodes(4201), 1);
    assert_eq!(nodes(4202), 0);

    expect_err(
        "empty_body.antics",
        &obj("    @frame(empty) tree t { self.n = 1; }"),
        "@frame(empty) tree 't' has no nodes, so its body must be '{}'",
    );
    expect_err(
        "empty_dup.antics",
        &obj("    @frame(empty, empty) tree t {}"),
        "duplicate @frame field 'empty'",
    );
    // A return value is not required of a tree that has no nodes.
    expand(
        "empty_ret.antics",
        &obj("    @frame(empty) tree t -> bool {}"),
    );
}
