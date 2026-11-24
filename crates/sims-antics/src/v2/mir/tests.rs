//! Unit tests of the MIR layer: the emitter, text <-> MIR round trips, the typed instruction
//! view, and the backend pipeline's frame facts.

use super::*;

#[test]
fn emitter_writes_indented_lines_and_rewinds_to_marks() {
    let mut em = Emitter::new(1);
    em.line("let __a_1 = 0;");
    em.line("if __a_1 == 0 {");
    em.indent();
    let mark = em.mark();
    em.line("return true;");
    assert_eq!(em.since(mark), "        return true;\n");
    em.truncate(mark);
    em.line("return false;");
    em.dedent();
    em.line("}");
    assert_eq!(
        em.text(),
        "    let __a_1 = 0;\n    if __a_1 == 0 {\n        return false;\n    }\n"
    );
    let tail_at = em.mark();
    em.line("return true;");
    assert_eq!(em.split_off(tail_at), "    return true;\n");
    // Trial lowering: take the text out, write elsewhere, put it back and append.
    let before = em.take_text();
    assert_eq!(em.text(), "");
    let old = em.set_depth(0);
    em.line("break;");
    let arm = em.take_text();
    em.set_depth(old);
    em.set_text(before);
    em.append(&arm);
    assert_eq!(em.depth(), 1);
    assert!(em.into_text().ends_with("    }\nbreak;\n"));
}

#[test]
fn bodies_round_trip_through_mir() {
    let body = "    let __x_1 = my.attr[0];\n    if __x_1 == 1 {\n        my.attr[1] := 2;\n    } else if __x_1 == 2 {\n        while __x_1 < 5 {\n            __x_1 += 1;\n        }\n    } else {\n        dialog(\"a { b } -> c;\");\n    }\n    while true {\n        break;\n    }\n    return true;\n";
    let nodes = parse_body(body).expect("in the modelled subset");
    assert_eq!(nodes.len(), 4);
    assert!(matches!(&nodes[1], Stmt::If { arms, else_body: Some(_) } if arms.len() == 2));
    assert!(matches!(&nodes[2], Stmt::While { cond: None, .. }));
    let mut out = String::new();
    print_nodes(&nodes, 1, &mut out);
    assert_eq!(out, body);
}

#[test]
fn unmodelled_bodies_do_not_parse() {
    for body in [
        "    a: nop -> true;\n",
        "    x := 1; y := 2;\n",
        "    // comment\n    return true;\n",
        "    my.attr[0] := scope(25, 2);\n",
        "    if c {\n        return true;\n",
    ] {
        assert!(parse_body(body).is_none(), "{body:?}");
    }
}

#[test]
fn instructions_have_a_typed_view() {
    assert_eq!(
        classify("let __a_1 = my.attr[0];"),
        InstKind::Let {
            name: "__a_1",
            init: Some("my.attr[0]")
        }
    );
    assert_eq!(
        classify("let __b_2;"),
        InstKind::Let {
            name: "__b_2",
            init: None
        }
    );
    assert_eq!(
        classify("stack_obj := __self_obj;"),
        InstKind::Assign {
            lhs: "stack_obj",
            op: ":=",
            rhs: "__self_obj"
        }
    );
    assert_eq!(classify("return false;"), InstKind::Return);
    assert_eq!(classify("break;"), InstKind::Break);
    assert_eq!(classify("continue;"), InstKind::Continue);
    assert_eq!(classify("dialog(\"x := 1\");"), InstKind::Other);
    assert_eq!(
        stack_obj_store("stack_obj := __saved_stack_obj;"),
        Some("__saved_stack_obj")
    );
}

#[test]
fn pipeline_applies_frame_facts() {
    let no_slots = BTreeMap::new();
    let no_consts = BTreeMap::new();
    let no_names = BTreeSet::new();
    let mut consts = BTreeMap::new();
    consts.insert("K".to_owned(), "3".to_owned());
    consts.insert("D".to_owned(), "4".to_owned());
    let mut declared = BTreeSet::new();
    declared.insert("D".to_owned());
    let facts = FrameFacts {
        formats_text: false,
        keep_layout: true,
        has_self_obj_local: true,
        pinned: false,
        raw_consts: &consts,
        raw_declared: &declared,
        has_slots: false,
        pinned_locals: &no_slots,
    };
    // Layout kept: only the unread entry save goes; undeclared raw constants are prepended.
    let body = "    let __self_obj = stack_obj;\n    my.attr[0] := K;\n    return true;\n";
    assert_eq!(
        finish_body(body.to_owned(), &facts),
        "    const K = 3;\n    my.attr[0] := K;\n    return true;\n"
    );
    // A tree that formats text reads frame words by number: the save stays.
    let facts = FrameFacts {
        formats_text: true,
        raw_consts: &no_consts,
        raw_declared: &no_names,
        ..facts
    };
    assert_eq!(finish_body(body.to_owned(), &facts), body);
}
