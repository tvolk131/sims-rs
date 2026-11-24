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
