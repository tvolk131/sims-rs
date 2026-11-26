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
    let body = "    let __x_1 = my.attr[0];\n    if __x_1 == 1 {\n        my.attr[1] := 2;\n    } else if __x_1 == 2 {\n        while __x_1 < 5 {\n            __x_1 += 1;\n        }\n    } else {\n        call \"a { b } -> c;\"(temps);\n    }\n    while true {\n        break;\n    }\n    return true;\n";
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
    let no_origins = BTreeMap::new();
    let no_consts = BTreeMap::new();
    let no_names = BTreeSet::new();
    let mut consts = BTreeMap::new();
    consts.insert("K".to_owned(), "3".to_owned());
    consts.insert("D".to_owned(), "4".to_owned());
    let mut declared = BTreeSet::new();
    declared.insert("D".to_owned());
    let facts = FrameFacts {
        main_rules: false,
        fixed_frame: false,
        text_reads: TableLocals::Known(BTreeSet::new()),
        text_reads_besides_me: TableLocals::Known(BTreeSet::new()),
        keeps_exit_stack_obj: false,
        keep_layout: true,
        has_self_obj_local: true,
        pinned: false,
        raw_consts: &consts,
        raw_declared: &declared,
        has_slots: false,
        pinned_locals: &no_slots,
        local_origins: &no_origins,
        min_locals: 0,
        option_aliases: &BTreeSet::new(),
        aliased: &std::cell::Cell::new(false),
    };
    // Layout kept: only the unread entry save goes; undeclared raw constants are prepended.
    let body = "    let __self_obj = stack_obj;\n    my.attr[0] := K;\n    return true;\n";
    assert_eq!(
        finish_body(body.to_owned(), &facts),
        "    const K = 3;\n    my.attr[0] := K;\n    return true;\n"
    );
    // Text that may read any frame word only through a `$Me` name: with no named local to move,
    // the save still goes (its own word holds no source local).
    let facts = FrameFacts {
        text_reads: TableLocals::Unknown,
        raw_consts: &no_consts,
        raw_declared: &no_names,
        ..facts
    };
    assert_eq!(
        finish_body(body.to_owned(), &facts),
        "    my.attr[0] := K;\n    return true;\n"
    );
    // A named local after the save would move down a word that text may read: the save stays.
    let moving =
        "    let __self_obj = stack_obj;\n    let x = 1;\n    my.attr[0] := x;\n    return true;\n";
    assert_eq!(finish_body(moving.to_owned(), &facts), moving);
    // Text that may read the save's own word by number (`$Local:0`) keeps the save.
    let mut word_0 = BTreeSet::new();
    word_0.insert(0);
    let facts = FrameFacts {
        text_reads: TableLocals::Known(word_0.clone()),
        text_reads_besides_me: TableLocals::Known(word_0),
        ..facts
    };
    assert_eq!(finish_body(body.to_owned(), &facts), body);
    // Text unbounded besides any `$Me` (or a frame whose layout is observable) keeps it too.
    let facts = FrameFacts {
        text_reads: TableLocals::Unknown,
        text_reads_besides_me: TableLocals::Unknown,
        ..facts
    };
    assert_eq!(finish_body(body.to_owned(), &facts), body);
}
