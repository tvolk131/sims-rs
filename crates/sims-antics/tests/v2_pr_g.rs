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

#[test]
fn tier2_primitives_lower_to_their_opcodes() {
    let cfg = PackResolverConfig::default();
    let src = obj(r#"    const LIBRARY_SOUND = 0x0123;
    sound chime = preset("chime");
    @id(4210) tree prims "Prims" (sim: Sim) {
        let other = 0;
        let score = 0;
        change_suit(index: 2, scope: 1, remove: true);
        burn(mode: 1, flags: 1);
        look_towards(mode: 2);
        drop_onto(src: 0, dst: 1);
        if !find_location(mode: 3, flags: 1, local: other) { return false; }
        relationship(column: 0, dir: 2, flags: 0, local: other, value: score);
        old_relationship(write: true, column: 1, param: 0, dir: 0, flags: 0);
        generic_call(17);
        play_sound(chime, stack_obj, flags: 1);
        play_sound(LIBRARY_SOUND, self);
        play_sound(0x0042, false);
        stop_all_sounds(stack_obj);
        self.n = score;
    }
    @id(4211) tree object_side "Object Side" {
        play_sound(chime, stack_obj);
        play_sound(chime);
    }"#);
    let air = expand("tier2.antics", &src);
    for needle in [
        "change_suit(index: 2, scope: 1, remove: true);",
        "burn(mode: 1, flags: 1);",
        "look_towards(mode: 2);",
        "drop_onto(src: 0, dst: 1);",
        "find_location(mode: 3, flags: 1, local: __v_other_0)",
        "relationship(column: 0, dir: 2, flags: 0, local: __v_other_0, value: __v_score_1);",
        "old_relationship(write: true, column: 1, param: 0, dir: 0, flags: 0);",
        "generic_call(17);",
        "flags: 3);",
        "play_sound(291, stack_obj);",
        "play_sound(66, false);",
        "stop_all_sounds(true);",
    ] {
        assert!(air.contains(needle), "missing '{needle}':\n{air}");
    }
    // An object tree can play on its stack object; by default it plays on itself.
    let object_side = &air[air.find("\"Object Side\"").expect("object tree")..];
    assert!(
        object_side.contains("stack_obj);\n    play_sound("),
        "{object_side}"
    );
    assert!(object_side.contains(", false);"), "{object_side}");

    let compiled = compile_v2_source("tier2.antics", &src, &cfg).expect("compile");
    let opcodes: Vec<u16> = compiled.behaviors[&4210]
        .instructions
        .iter()
        .map(|i| i.opcode)
        .collect();
    for opcode in [6, 9, 22, 43, 16, 26, 24, 1, 23, 48] {
        assert!(opcodes.contains(&opcode), "opcode {opcode} in {opcodes:?}");
    }
    // The relationship read lands in `score`, which the tree then stores: the optimizer may
    // not forward `score`'s initial 0 past the primitive.
    assert!(air.contains("stack.attr[0] := __v_score_1;\n"), "{air}");

    for (name, body, needle) in [
        (
            "rel_value.antics",
            "let o = 0; relationship(column: 0, local: o);",
            "relationship() requires 'value:'",
        ),
        (
            "rel_value_type.antics",
            "relationship(column: 0, value: self.n);",
            "relationship(value:) must name an i16 local variable",
        ),
        (
            "suit_unknown.antics",
            "change_suit(index: 1, off: true);",
            "unknown change_suit() argument 'off'",
        ),
        (
            "generic_named.antics",
            "generic_call(id: 4);",
            "unknown generic_call() argument 'id'",
        ),
        (
            "sound_flag_two.antics",
            "play_sound(0x0042, flags: 2);",
            "play_sound flags 0x02 selects the stack object",
        ),
        (
            "sound_target.antics",
            "play_sound(0x0042, 3);",
            "play_sound target must be 'self', 'sim', 'stack_obj', or a literal bool",
        ),
    ] {
        expect_err(name, &sim_tree(body), needle);
    }
}

#[test]
fn tier2_sim_and_handle_fields() {
    let air = expand(
        "fields.antics",
        &obj("    bitflags Marks { High = 3 }\n    tree t (sim: Sim) { \
             sim.attr[1] = 4; \
             sim.data[3] = sim.attr[1]; \
             sim.priority = 50; \
             sim.personality.neat = sim.personality.nice + 1; \
             let c = sim.container; \
             if c == stack_obj { sim.data[4] = 1; } \
             if stack_obj.flags.contains(Marks::High) { self.n = 2; } \
             stack_obj.flags.set(Marks::High); }"),
    );
    for needle in [
        "my.attr[1] := 4;",
        "my.data[3] := my.attr[1];",
        "my.person.priority := 50;",
        "= my.person[2];",
        "my.person[7] := __arith_0;",
        "my.data[2]",
        "stack.data[8]",
    ] {
        assert!(air.contains(needle), "missing '{needle}':\n{air}");
    }
    for (name, body, needle) in [
        (
            "container_write.antics",
            "sim.container = 0;",
            "cannot assign to 'sim.container'",
        ),
        (
            "trait_unknown.antics",
            "self.n = sim.personality.grumpy;",
            "unknown personality trait 'grumpy'",
        ),
    ] {
        expect_err(name, &sim_tree(body), needle);
    }
}

#[test]
fn tier2_job_data_reads_scope_33_through_the_temps() {
    let src = obj(
        "    extern mod g { @id(300) fn rate(a: i16, b: i16) -> bool; }\n    \
         tree t (sim: Sim) { \
             let career = 3; \
             self.n = job_data(career, level: 2, field: 5); \
             temp[0] = 7; \
             self.n = job_data(career: 1, level: temp[0], field: 0); \
             if g::rate(self.n, job_data(4, 1, 2)) { self.n = 1; } \
         }",
    );
    let air = expand("job.antics", &src);
    assert!(
        air.contains("temp[1] := 2;") && air.contains("scope(33, 5)"),
        "{air}"
    );
    // A level read from `temp[0]` survives the career store.
    let at = air.find("scope(33, 0)").expect("second read");
    let before = &air[..at];
    let copy = before.rfind("= temp[0];").expect("level copied first");
    assert!(before[copy..].contains("temp[0] := 1;"), "{air}");
    expect_err(
        "job_named.antics",
        &sim_tree("self.n = job_data(career: 1, level: 0, row: 1);"),
        "unknown job_data() argument 'row'",
    );
}

#[test]
fn tier2_distance_to_measures_to_the_stack_object() {
    let air = expand(
        "distance.antics",
        &sim_tree(
            "let Some(o) = find_best_object(prep) else { return; }; \
             let d = distance_to(o); \
             self.n = d; \
             self.n = distance_to(stack_obj, from: o, temp: 3); \
             self.n = distance_to(o, from: stack_obj);",
        ),
    );
    for needle in [
        "get_distance_to(temp[0]);",
        "get_distance_to(temp[3], from: ",
        "= temp[3];",
    ] {
        assert!(air.contains(needle), "missing '{needle}':\n{air}");
    }
    // Moving to the target restores the entry stack object afterwards, and a `from` of the
    // stack object is read before the move.
    let body = &air[air.find("\"t\" {").expect("tree")..];
    let gets = body.matches("get_distance_to(").count();
    assert_eq!(gets, 3, "{body}");
    assert!(body.contains("__self_obj"), "{body}");
    let last = body.rfind("get_distance_to(").expect("third");
    let copy = body[..last].rfind("= stack_obj;").expect("from copied");
    let mv = body[..last].rfind("stack_obj := ").expect("moved");
    assert!(copy < mv, "{body}");
    for (name, body, needle) in [
        (
            "dist_type.antics",
            "self.n = distance_to(3);",
            "distance_to target must be an object",
        ),
        (
            "dist_temp.antics",
            "self.n = distance_to(stack_obj, temp: 8);",
            "distance_to temp",
        ),
    ] {
        expect_err(name, &sim_tree(body), needle);
    }
}
