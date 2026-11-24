//! Synthetic coverage for the v2 frame cells and inherited-context private calls.

use sims_antics::{PackResolverConfig, compile_v2_source, v2};
use sims_formats::{behavior::Behavior, iff::Iff};

fn expand(name: &str, src: &str) -> String {
    let cfg = PackResolverConfig::default();
    compile_v2_source(name, src, &cfg).unwrap_or_else(|e| panic!("{name} must compile: {e}"));
    v2::expand_v2_source_to_objects(name, src, &cfg)
        .unwrap_or_else(|e| panic!("{name} must expand: {e}"))
        .into_iter()
        .next()
        .expect("one object")
        .air_source
}

fn obj(members: &str) -> String {
    format!(
        "object \"Frame Prism\" {{\n    guid: 0x7F40_0271;\n    state charge: i16;\n{members}\n}}\n"
    )
}

fn tree<'a>(air: &'a str, name: &str) -> &'a str {
    air.lines()
        .find(|l| l.starts_with("tree ") && l.contains(&format!("\"{name}\" ")))
        .and_then(|header| air[air.find(header)? + header.len()..].split_once("\n}\n"))
        .map(|(body, _)| body.trim_start_matches('\n'))
        .unwrap_or_else(|| panic!("tree {name} in:\n{air}"))
}

fn expect_err(members: &str, expected: &str) {
    let err = compile_v2_source(
        "refused.antics",
        &obj(members),
        &PackResolverConfig::default(),
    )
    .expect_err("expected a compile error");
    assert!(err.message.contains(expected), "{err}");
}

#[test]
fn me_cells_lower_to_my_cells_in_both_contexts() {
    let air = expand(
        "frame_cells.antics",
        &obj(r#"
    tree object_probe {
        me.person[9] = me.data[8];
        me.motive[3] = me.person[9];
        me.attr[2] = me.motive[3];
        self.charge = 5;
    }
    tree sim_probe (actor: Sim) {
        me.person[9] = actor.data[8];
        me.motive[3] = me.person[9];
        me.attr[2] = me.motive[3];
        self.charge = 5;
    }
"#),
    );
    let cells = "    my.person[9] := my.data[8];\n    my.motive[3] := my.person[9];\n    my.attr[2] := my.motive[3];\n";
    assert_eq!(
        tree(&air, "object_probe"),
        format!("{cells}    my.attr[0] := 5;\n    return true;")
    );
    assert_eq!(
        tree(&air, "sim_probe"),
        format!("{cells}    stack.attr[0] := 5;\n    return true;")
    );
}

#[test]
fn me_preserves_existing_handle_bindings() {
    let air = expand(
        "bound_me.antics",
        &obj(r#"
    tree bound_probe (me: ObjectId) {
        me.attr[2] = 8;
    }
    tree actor_probe (me: Sim) {
        me.attr[2] = 8;
    }
"#),
    );
    assert!(tree(&air, "bound_probe").contains("stack_obj := param[0];"));
    assert!(tree(&air, "bound_probe").contains("stack.attr[2] := 8;"));
    assert_eq!(
        tree(&air, "actor_probe"),
        "    my.attr[2] := 8;\n    return true;"
    );
}

#[test]
fn both_callers_inherit_their_live_stack_object_with_no_wrapper() {
    let src = obj(r#"
    @id(4200) tree object_probe {
        stack_obj = self.id;
        shared_probe()?;
    }
    @id(4201) tree sim_probe (actor: Sim) {
        shared_probe()?;
    }
    @id(4202) @context(inherit) tree shared_probe {
        stack_obj.attr[3] = me.data[4];
        stack_obj = me.slot[0];
        if stack_obj == 0 { return false; }
    }
"#);
    let air = expand("inherited.antics", &src);
    let call = "    if !call 4202(temps) {\n        return error;\n    }\n    return true;";
    assert_eq!(
        tree(&air, "object_probe"),
        format!("    stack_obj := my.id;\n{call}")
    );
    assert_eq!(tree(&air, "sim_probe"), call);
    assert_eq!(
        tree(&air, "shared_probe"),
        "    stack.attr[3] := my.data[4];\n    stack_obj := my.slot[0];\n    if stack_obj == 0 {\n        return false;\n    }\n    return true;"
    );

    let compiled = compile_v2_source("inherited.antics", &src, &PackResolverConfig::default())
        .expect("compile");
    let iff = Iff::parse(compiled.iff_bytes).expect("IFF");
    let bhav = |id| {
        let chunk = iff.get(b"BHAV", id).expect("BHAV chunk");
        Behavior::parse(iff.data(chunk)).expect("BHAV")
    };
    for (id, call_index) in [(4200, 1), (4201, 0)] {
        let caller = bhav(id);
        assert_eq!(caller.locals, 0);
        assert_eq!(caller.instructions.len(), call_index + 1);
        let ins = &caller.instructions[call_index];
        assert_eq!((ins.opcode, ins.on_true, ins.on_false), (4202, 0xFE, 0xFD));
        assert_eq!(ins.operands, [0xFF; 8]);
    }
    let callee = bhav(4202);
    assert_eq!((callee.locals, callee.args), (0, 0));
    assert_eq!(callee.instructions[0].opcode, 2);
    assert_eq!(callee.instructions[0].operands, [3, 0, 4, 0, 0, 5, 1, 3]);
}

#[test]
fn inherited_tree_preserves_raw_layout_and_ordinary_abi() {
    let src = obj(r#"
    @id(4200) @context(inherit) tree cursor_step (n: i16) {
        temp[0] = n;
        raw {
            local[0] := stack_obj;
            stack_obj := my.data[7];
            if local[0] == stack_obj { return false; }
        }
    }
    @id(4201) @context(inherit) tree delegate_step {
        cursor_step(3)?;
    }
"#);
    let air = expand("inherited_raw.antics", &src);
    assert_eq!(
        tree(&air, "cursor_step"),
        "    temp[0] := param[0];\n    local[0] := stack_obj;\n    stack_obj := my.data[7];\n    if local[0] == stack_obj { return false; }\n    return true;"
    );
    assert!(tree(&air, "delegate_step").contains("if !call 4200(3)"));
    let formatted = v2::format_v2_source(&src).expect("format inherited tree");
    assert_eq!(expand("inherited_raw.antics", &formatted), air);
    v2::lint_v2_source(&formatted, "inherited_raw.antics").expect("lint inherited tree");
}

#[test]
fn rejects_unproven_frame_assumptions() {
    expect_err(
        "tree a (actor: Sim) { b(); } tree b {}",
        "tree 'b' runs in the object's own context",
    );
    expect_err(
        "tree a { stack_obj = self.id; b(stack_obj); } tree b (actor: Sim) {}",
        "tree 'b' runs in the Sim-on-object context",
    );
    expect_err(
        "tree a (actor: Sim) { b(stack_obj); } tree b (actor: Sim) {}",
        "tree 'b' expects the Sim handle as its first argument",
    );
    expect_err(
        "@context(inherit) tree a { self.charge = 1; }",
        "@context(inherit) tree 'a' depends on the caller's context",
    );
    expect_err(
        "@context(inherit) tree a { b(); } tree b {}",
        "@context(inherit) tree 'a' depends on the caller's context",
    );
    expect_err(
        "@context(inherit) tree a { destroy_self(); }",
        "@context(inherit) tree 'a' depends on the caller's context",
    );
    expect_err(
        "@context(inherit) tree a (actor: Sim) {}",
        "@context(inherit) cannot declare a Sim parameter",
    );
    expect_err(
        "@context(inherit) main {}",
        "@context(inherit) is only allowed on a standalone 'tree'",
    );
    expect_err("tree a { me.person[256] = 0; }", "me.person index");
    expect_err(
        "tree a { me.motive[16] = 0; }",
        "motive index 16 out of 0..16 range",
    );
}
