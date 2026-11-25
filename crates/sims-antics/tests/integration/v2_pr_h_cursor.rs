//! Synthetic regressions for v2 object iteration and the stack-object cursor.

use sims_antics::ir::BhavChunk;
use sims_antics::lower::{TARGET_ERROR, TARGET_FALSE, TARGET_TRUE};
use sims_antics::{PackResolverConfig, compile_air, compile_v2_source};

fn source(members: &str) -> String {
    format!(
        r#"object "Cursor Probe" {{
    guid: 0x7F40_0800;
    state value: i16;
    const PROBE = 0x7F40_0801;
    {members}
}}"#
    )
}

fn compile_tree(members: &str) -> BhavChunk {
    compile_v2_source(
        "cursor.antics",
        &source(members),
        &PackResolverConfig::default(),
    )
    .expect("compile synthetic cursor tree")
    .behaviors
    .remove(&4300)
    .expect("tree 4300")
}

fn compare_air(members: &str, air_body: &str) {
    let actual = compile_tree(members);
    let expected = compile_air(&format!("tree 4300 \"Expected\" {{\n{air_body}\n}}"))
        .unwrap()
        .behaviors
        .remove(&4300)
        .unwrap();
    assert_eq!(actual.args, expected.args);
    assert_eq!(actual.locals, expected.locals);
    let nodes = |tree: BhavChunk| {
        tree.instructions
            .into_iter()
            .map(|n| (n.opcode, n.operands, n.on_true, n.on_false))
            .collect::<Vec<_>>()
    };
    assert_eq!(nodes(actual), nodes(expected));
}

#[test]
fn for_else_targets_the_false_exit_without_a_flag() {
    compare_air(
        r#"#[bhav(id = 4300)]
fn scan(sim: Sim) -> bool {
            for visitor in lot.sims() {
                if visitor.age >= 18 { break; }
            } else {
                return false;
            }
            return true;
        }"#,
        r#"stack_obj := 0;
        while true {
            if set_to_next(person, dest: stack_obj) {
                if stack.person[58] >= 18 { break; }
                continue;
            }
            return false;
        }
        return true;"#,
    );
}

#[test]
fn for_else_handles_every_object_iterator() {
    for (iter, mode) in [
        ("lot.sims()", 1),
        ("lot.objects()", 0),
        ("objects_by_guid(PROBE)", 4),
        ("objects_of::<Self>()", 4),
        ("self.parts()", 3),
    ] {
        let tree = compile_tree(&format!(
            "#[bhav(id = 4300)] fn scan() {{ for candidate in {iter} {{ break; }} else {{ return error; }} return true; }}"
        ));
        let step = tree
            .instructions
            .iter()
            .find(|node| node.opcode == 31)
            .unwrap();
        assert_eq!(step.operands[4] & 0x7f, mode, "{iter}");
        assert_eq!(step.on_false, TARGET_ERROR, "{iter}");
        if mode == 3 {
            // SelfParts also exhausts when the successful step wraps around to the root.
            let wrap_test = &tree.instructions[usize::from(step.on_true)];
            assert_eq!(wrap_test.on_true, TARGET_ERROR);
            assert_eq!(wrap_test.on_false, TARGET_TRUE);
        } else {
            assert_eq!(step.on_true, TARGET_TRUE, "{iter}");
            assert_eq!(
                tree.locals,
                u16::from(iter.starts_with("objects_of")),
                "{iter}"
            );
        }
    }
}

#[test]
fn for_else_can_fall_through_and_continue_skips_it() {
    compare_air(
        r#"#[bhav(id = 4300)]
fn scan() {
            for item in lot.objects() {
                if item == 3 { break; }
                continue;
            } else {
                temp[1] = 7;
            }
            return temp[1] == 7;
        }"#,
        r#"let item = 0;
        while true {
            if set_to_next(all, dest: item) {
                if item == 3 { break; }
                continue;
            }
            temp[1] := 7;
            break;
        }
        if temp[1] == 7 { return true; }
        return false;"#,
    );
}

#[test]
fn for_else_restores_self_on_break_but_not_on_unmoved_exhaustion() {
    // Every iteration breaks after moving the cursor back, so exhaustion still has the entry
    // object and keeps the iterator's false result (no restore store replaces it).
    compare_air(
        r#"#[bhav(id = 4300)]
fn scan(sim: Sim) {
            for item in lot.sims() {
                stack_obj = item;
                temp[0] = stack_obj;
                break;
            } else {
                self.value = 7;
            }
            temp[1] = self.value;
        }"#,
        r#"let entry = stack_obj;
        let item = 0;
        while true {
            if set_to_next(person, dest: item) {
                stack_obj := item;
                temp[0] := stack_obj;
                stack_obj := entry;
                break;
            }
            stack.attr[0] := 7;
            break;
        }
        temp[1] := stack.attr[0];
        return true;"#,
    );
}

#[test]
fn explicit_steps_keep_the_current_cursor_and_allow_temp_writes() {
    compare_air(
        r#"#[bhav(id = 4300, locals = 0)]
fn scan(sim: Sim) -> bool {
            while set_to_next(guid, PROBE, dest: stack_obj) {
                temp[0] = stack_obj;
                if stack_obj.graphic == 3 { return true; }
            }
            return false;
        }"#,
        r#"while set_to_next(4, guid: 0x7f400801, dest: stack_obj) {
            temp[0] := stack_obj;
            if stack.data[0] == 3 { return true; }
        }
        return false;"#,
    );
    compare_air(
        r#"#[bhav(id = 4300, locals = 0)]
fn step() -> bool {
            return set_to_next(part, dest: stack_obj);
        }"#,
        "if set_to_next(3, dest: stack_obj) { return true; } return false;",
    );
}

#[test]
fn category_and_on_tile_keep_the_operand_bytes() {
    let tree = compile_tree(
        r#"#[bhav(id = 4300, locals = 0, args = 1)]
fn scan() -> bool {
            if !set_to_next(category, guid: PROBE, dest: stack_obj) { return false; }
            return set_to_next(8);
        }"#,
    );
    let [category, on_tile] = tree.instructions.as_slice() else {
        panic!("expected two primitives: {:?}", tree.instructions);
    };
    assert_eq!((category.opcode, on_tile.opcode), (31, 31));
    assert_eq!(category.operands, [1, 8, 0x40, 0x7f, 0x86, 10, 0, 0]);
    assert_eq!(on_tile.operands, [0, 0, 0, 0, 8, 0, 0, 0]);
    assert_eq!(category.on_false, TARGET_FALSE);
    assert_eq!(on_tile.on_true, TARGET_TRUE);
    assert_eq!(tree.args, 1);
}

#[test]
fn create_object_can_leave_the_result_in_stack_obj() {
    compare_air(
        r#"#[bhav(id = 4300, locals = 0)]
fn spawn() -> bool {
            if !create_object(PROBE, mode: here, dest: stack_obj) { return false; }
            temp[2] = stack_obj;
            return true;
        }"#,
        r#"if !create_object(0x7f400801, mode: here, flags: 0) { return false; }
        temp[2] := stack_obj;
        return true;"#,
    );
    compare_air(
        r#"#[bhav(id = 4300, locals = 0)]
fn spawn() -> bool {
            return create_object::<Self>(dest: stack_obj);
        }"#,
        "if create_object(0x7f400800, mode: here, flags: 0) { return true; } return false;",
    );
}

#[test]
fn sim_self_access_preserves_the_cursor_at_a_join_and_on_the_back_edge() {
    compare_air(
        r#"#[bhav(id = 4300)]
fn scan(sim: Sim) {
            while temp[1] != 0 {
                if temp[2] != 0 { set_to_next(part, dest: stack_obj); }
                self.value = 7;
                temp[0] = stack_obj;
            }
        }"#,
        r#"let entry = stack_obj;
        while temp[1] != 0 {
            if temp[2] != 0 { set_to_next(3, dest: stack_obj); }
            let cursor = stack_obj;
            stack_obj := entry;
            stack.attr[0] := 7;
            stack_obj := cursor;
            temp[0] := stack_obj;
        }
        return true;"#,
    );
}

#[test]
fn cursor_syntax_rejects_unsupported_forms() {
    for (body, expected) in [
        ("for i in 0..4 {} else {}", "requires an object iterator"),
        (
            "loop { for o in lot.objects() {} else { break; } }",
            "cannot target an enclosing loop",
        ),
        (
            "loop { for o in lot.objects() {} else { continue; } }",
            "cannot target an enclosing loop",
        ),
        (
            "for o in lot.objects() {} else { temp[0] = o; }",
            "unknown identifier 'o'",
        ),
        (
            "set_to_next(part, dest: self.value);",
            "steps 'stack_obj', a local",
        ),
        ("set_to_next(128);", "out of range"),
        ("create_object(PROBE, dest: temp[0]);", "expects stack_obj"),
    ] {
        let src = source(&format!("#[bhav(id = 4300)] fn probe() {{ {body} }}"));
        let err = compile_v2_source("cursor.antics", &src, &PackResolverConfig::default())
            .expect_err(body);
        assert!(err.message.contains(expected), "{body}: {err}");
    }
}
