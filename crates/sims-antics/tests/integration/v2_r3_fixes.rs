//! `.antics` v2 Round 3 lowering fixes: the dialog entry save, out-operand writes, the
//! `#[context(inherit)]` check, `self` before the first cursor move, `push_interaction` on an
//! `#[slot]` handle and `job_data` comparisons. All sources are synthetic.

use sims_antics::{PackResolverConfig, compile_v2_source, v2};

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

/// A synthetic object with the strings table `notes` (the `msg` string is `message`) around
/// `members`.
fn obj(message: &str, members: &str) -> String {
    format!(
        r#"object "Round Three" {{
    guid: 0x7F40_0900;
    state value: i16;
    strings 301 "Notes" {{ title = "T", msg = "{message}" }}
{members}
}}
"#
    )
}

/// The `.air` body of tree `name` (from its header to the next tree).
fn tree<'a>(air: &'a str, name: &str) -> &'a str {
    let start = [format!("\"{name}\" {{"), format!("\"{name}\" (")]
        .iter()
        .find_map(|h| air.find(h.as_str()))
        .unwrap_or_else(|| panic!("tree {name} in:\n{air}"));
    let rest = &air[start..];
    let end = rest[1..]
        .find("\ntree ")
        .or_else(|| rest[1..].find("\ninteraction "))
        .map_or(rest.len(), |i| i + 1);
    &rest[..end]
}

/// Tree `id`'s local count and its instructions as `(opcode, operands)`.
fn compiled(src: &str, id: u16) -> (u16, Vec<(u16, [u8; 8])>) {
    let tree = compile_v2_source("compiled.antics", src, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("must compile: {e}\n{src}"))
        .behaviors
        .remove(&id)
        .unwrap_or_else(|| panic!("tree {id}"));
    let nodes = tree
        .instructions
        .into_iter()
        .map(|n| (n.opcode, n.operands))
        .collect();
    (tree.locals, nodes)
}

/// The Relationship primitive's opcode.
const RELATIONSHIP: u16 = 26;

const SAVE: &str = "let __self_obj = stack_obj;";

/// A Sim tree that moves the cursor with a search (which makes `self` the saved entry object)
/// and never reads `self` afterwards, with a dialog and one unpinned local declared after the
/// save (so dropping the save moves it from word 1 to word 0).
const DIALOG_TREE: &str = r#"    #[bhav]
    fn a(sim: Sim) {
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
        let n = sim.person[3];
        stack_obj.attr[0] = n;
        dialog(kind: ok, title: title, message: msg);
    }"#;

/// The unread entry save of a dialog tree drops when no string of the object can read a frame
/// word, and when the words its strings read keep their occupants.
#[test]
fn dialog_entry_save_drops_when_no_text_reads_a_moved_word() {
    for message in ["Plain text", "Day $DayOfMonth", "$Local:5"] {
        let air = expand("save_drop.antics", &obj(message, DIALOG_TREE));
        assert!(!tree(&air, "a").contains(SAVE), "{message}:\n{air}");
    }
    // Every named local pinned: dropping the save moves no word that `$Local:1` reads.
    let pinned = r#"    #[bhav]
    fn a(sim: Sim) {
        #[slot(1)] let n = sim.person[3];
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
        stack_obj.attr[0] = n;
        dialog(kind: ok, title: title, message: msg);
    }"#;
    let air = expand("save_pinned.antics", &obj("Was $Local:1", pinned));
    assert!(!tree(&air, "a").contains(SAVE), "{air}");
    // The save is the highest word (after the pinned `n`), and the text reads only the word
    // below it: dropping it moves nothing. (`random_into` is not an Expression, so ordinary
    // dead-store elimination does not drop the save first.)
    let highest = r#"    #[bhav]
    fn a(sim: Sim) {
        #[slot(0)] let n: i16;
        random_into(n, 4)?;
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
        stack_obj.attr[0] = n;
        dialog(kind: ok, title: title, message: msg);
    }"#;
    let air = expand("save_highest.antics", &obj("Was $Local:0", highest));
    assert!(!tree(&air, "a").contains(SAVE), "{air}");
}

/// The save stays when a string can read a word that dropping it would move (`$Local:0` reads
/// the save's word, which `n` would take), or when a string's reads cannot be bounded (`$Me`).
#[test]
fn dialog_entry_save_stays_when_text_may_read_a_moved_word() {
    for message in ["Was $Local:0", "Now $Local:1", "Hello $Me"] {
        let air = expand("save_keep.antics", &obj(message, DIALOG_TREE));
        assert!(tree(&air, "a").contains(SAVE), "{message}:\n{air}");
    }
}

/// A dialog literal in a later tree adds a `$Local` string to the object's file, so an earlier
/// tree lowered before it was seen is lowered again with the final set of strings.
#[test]
fn a_later_dialog_literal_reaches_earlier_trees() {
    let members = format!(
        "{DIALOG_TREE}\n    #[bhav] fn b(sim: Sim) {{\n        dialog(kind: ok, title: \"T\", message: \"Was $Local:0\");\n    }}"
    );
    let air = expand("save_late.antics", &obj("Plain text", &members));
    assert!(tree(&air, "a").contains(SAVE), "{air}");
}

/// A `random_into` whose range is a constant (a literal or a BCON) stores the draw in its
/// destination on both exits, so the destination is written before it is read.
#[test]
fn random_into_a_constant_range_defines_its_destination() {
    // (The raw statement keeps ordinary dead-store elimination from dropping the save before
    // the analysis sees it.)
    let members = r#"    #[bhav]
    fn a(sim: Sim) {
        let n: i16;
        raw { temp[7] := 0; }
        stack_obj = 0;
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
        random_into(n, LIMIT)?;
        stack_obj.attr[0] = n;
    }
    const LIMIT = 6;"#;
    let air = expand("random_range.antics", &obj("Plain", members));
    assert!(!tree(&air, "a").contains(SAVE), "{air}");
    // A BCON range stays a `bcon[..]` cell in `.air`.
    let members = r#"    #[bhav]
    fn a(sim: Sim) {
        let n: i16;
        raw { temp[7] := 0; }
        stack_obj = 0;
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
        random_into(n, Limits.draw)?;
        stack_obj.attr[0] = n;
    }
    bcon 4097 "Limits" { draw = 6 }"#;
    let air = expand("random_bcon.antics", &obj("Plain", members));
    assert!(!tree(&air, "a").contains(SAVE), "{air}");
}

/// `create_object(.., mode: off_lot)` and `relationship` in directions 0 and 1 ignore their
/// local byte, so it costs no frame word; `relationship(dir: 2)` reads its local, and naming it
/// with `local:` puts it in the bound let's word.
#[test]
fn ignored_local_bytes_reserve_no_frame_word() {
    let src = obj(
        "Plain",
        r#"    #[bhav(id = 4300)]
    fn a(sim: Sim) {
        let n = sim.person[3];
        if !create_object(0x7F40_0901, mode: off_lot, flags: 1, dest: stack_obj) {
            return false;
        }
        relationship(column: 0, dir: 0, value: n);
        stack_obj.attr[0] = n;
    }
    #[bhav(id = 4301)]
    fn b(sim: Sim) {
        let who: i16;
        who = sim.person[4];
        let n = sim.person[3];
        relationship(column: 0, dir: 2, value: n, local: who);
        stack_obj.attr[0] = n;
    }"#,
    );
    assert_eq!(compiled(&src, 4300).0, 1, "only `n`");
    let (locals, nodes) = compiled(&src, 4301);
    assert_eq!(locals, 2, "`who` and `n`");
    let rel = nodes
        .iter()
        .find(|(op, _)| *op == RELATIONSHIP)
        .unwrap_or_else(|| panic!("relationship node in {nodes:?}"));
    assert_eq!(rel.1[3], 0, "the local byte names `who`'s word: {rel:?}");
}

/// An inherited tree whose two caller contexts lower to the same code up to local names passes
/// the check: a `for` item used as a value and through a field stages a cursor save, named
/// `__self_obj` in the Sim pass and `__saved_stack_obj` in the object pass.
#[test]
fn inherit_check_compares_after_canonical_local_names() {
    let src = obj(
        "Plain",
        r#"    #[bhav(context = inherit)]
    fn walk() {
        for p in lot.sims() {
            if !(p == me.data[11]) {
                if p.kind == 1 {
                    return true;
                }
            }
        }
        return false;
    }
    #[bhav]
    fn caller(sim: Sim) {
        walk()?;
    }"#,
    );
    expand("inherit_canonical.antics", &src);
}

/// A genuinely context-dependent inherited tree still fails, and the error points at the first
/// statement whose lowering differs (the targetless `play_sound`), not at the tree.
#[test]
fn inherit_check_points_at_the_differing_statement() {
    let src = r#"object "Round Three" {
    guid: 0x7F40_0902;
    sound ding = preset("ui_click");
    #[bhav(context = inherit)]
    fn beep() {
        me.data[3] = 1;
        if me.data[4] == 2 {
            play_sound(ding);
        }
        return true;
    }
}
"#;
    let err = compile_v2_source("inherit_span.antics", src, &PackResolverConfig::default())
        .expect_err("context-dependent");
    assert!(
        err.message.contains("depends on the caller's context"),
        "{err}"
    );
    let span = err.span.expect("span");
    assert_eq!((span.line, span.column), (8, 13), "{err}");
}

/// Disjoint inlined copies of a helper share their local's word even when a primitive writes it
/// through an out-operand (`animate(local:)`, `random_into`); two differently named locals of one
/// helper keep their own words.
#[test]
fn inlined_out_operand_locals_share_across_copies_only() {
    let src = obj(
        "Plain",
        r#"    fn react(k: i16) {
        let ev: i16;
        animate(clip: k, table: 0, events: 0, local: ev) |_tick| {
            me.attr[1] = ev;
        }
    }
    fn roll() {
        let r: i16;
        random_into(r, 10)?;
        me.attr[2] = r;
    }
    fn pick_one() {
        let pick: i16;
        random_into(pick, 2)?;
        me.attr[3] = pick;
    }
    fn pick_event() {
        let ev: i16;
        random_into(ev, 3)?;
        me.attr[4] = ev;
    }
    #[bhav]
    fn a() {
        react(1);
        react(2);
    }
    #[bhav]
    fn b() {
        roll();
        roll();
    }
    #[bhav]
    fn c() {
        pick_one();
        pick_event();
    }"#,
    );
    let air = expand("inline_share.antics", &src);
    for (name, local) in [("a", "__inline_v_ev_"), ("b", "__inline_v_r_")] {
        let body = tree(&air, name);
        assert_eq!(
            body.matches(&format!("let {local}")).count(),
            1,
            "{name}:\n{body}"
        );
    }
    let body = tree(&air, "c");
    assert!(
        body.contains("__inline_v_ev_") && body.contains("__inline_v_pick_"),
        "{body}"
    );
}

/// A helper local read before a primitive writes it holds whatever its word held, so its
/// copies do not share a word.
#[test]
fn out_operand_locals_read_first_do_not_share() {
    let src = obj(
        "Plain",
        r#"    fn peek(k: i16) {
        let ev: i16;
        me.attr[0] = ev;
        random_into(ev, k)?;
        me.attr[1] = ev;
    }
    #[bhav]
    fn a() {
        peek(2);
        peek(3);
    }"#,
    );
    let air = expand("inline_read_first.antics", &src);
    let body = tree(&air, "a");
    assert_eq!(body.matches("let __inline_v_ev_").count(), 2, "{body}");
}

/// A Sim tree that moves the cursor selects `self` through the entry object everywhere, as on
/// main, even before the first move: the switch is an Expression that leaves the last result
/// true, which is what a faulting read after it leaves behind.
#[test]
fn self_is_switched_to_before_the_first_cursor_move() {
    let src = obj(
        "Plain",
        r#"    #[bhav(locals = 3)]
    fn a(sim: Sim) {
        if temp[0] == 0 { return true; }
        if self.data[10000] == 1 { return true; }
        set_to_next(all, dest: stack_obj);
        raw { const K = 7; temp[2] := K; }
    }"#,
    );
    let air = expand("self_switch.antics", &src);
    let a = tree(&air, "a");
    let read = a.find("stack.data[10000]").expect("the self read");
    assert!(
        a[..read].contains("stack_obj := __self_obj;"),
        "the read switches first:\n{a}"
    );
}

/// `push_interaction(object: h)` with `h` an `#[slot(N)]` handle reads the object from word N.
#[test]
fn push_interaction_reads_a_slot_handle_in_place() {
    let src = obj(
        "Plain",
        r#"    #[bhav]
    fn a(sim: Sim) {
        #[slot(1)] let target = stack_obj;
        stack_obj = sim.container;
        if !push_interaction(menu: 1, object: target, priority: 0, flags: 1, icon: target) {
            return false;
        }
    }"#,
    );
    let air = expand("push_slot.antics", &src);
    let body = tree(&air, "a");
    assert!(
        body.contains(
            "push_interaction(menu: 1, object: 1, priority: 0, flags: 3, icon: local[1])"
        ),
        "{body}"
    );
    assert!(
        !body.contains("local[0] :="),
        "no copy into word 0:\n{body}"
    );
    let refused = obj(
        "Plain",
        r#"    #[bhav]
    fn a(sim: Sim) {
        #[slot(1)] let target = stack_obj;
        push_interaction(menu: 1, object: target, priority: 0, flags: 2);
    }"#,
    );
    let err = compile_v2_source("push_flag.antics", &refused, &PackResolverConfig::default())
        .expect_err("explicit 0x02");
    assert!(err.message.contains("implied by an object handle"), "{err}");
}

/// `job_data(..) == k` keeps main's copy into a local: dropping it would remove a frame word,
/// and a tree's frame size is observable (Create Object passes `temp[0]` to a new object's
/// `main` only when that frame has a word for it).
#[test]
fn job_data_comparisons_keep_their_copy() {
    let src = obj(
        "Plain",
        r#"    #[bhav]
    fn a(sim: Sim) {
        return job_data(sim.person[60], sim.person[61], 22) == 1;
    }"#,
    );
    let air = expand("job_cmp.antics", &src);
    let a = tree(&air, "a");
    assert!(a.contains("let __job_0 = scope(33, 22);"), "{a}");
}

/// `.air` keeps `param[0]` in the word after the named locals around an `idle` when a tree uses
/// both, so text that may read that word keeps the save that would move it.
#[test]
fn dialog_entry_save_stays_when_text_may_read_the_idle_scratch_word() {
    let members = r#"    #[bhav(args = 1)]
    fn a(sim: Sim) {
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
        stack_obj.attr[0] = param[0];
        sleep(1);
        dialog(kind: ok, title: title, message: msg);
    }"#;
    let air = expand("save_scratch.antics", &obj("Was $Local:1", members));
    assert!(tree(&air, "a").contains(SAVE), "{air}");
    let air = expand("save_scratch_plain.antics", &obj("Plain text", members));
    assert!(!tree(&air, "a").contains(SAVE), "{air}");
}

/// An out-operand that writes on only one exit leaves the old value on the other: the loop
/// exits when `animate` returns true, which does not write `ev`, so its initial 7 must stay.
#[test]
fn an_out_operand_keeps_the_old_value_on_the_exit_it_does_not_write() {
    let src = obj(
        "Plain",
        r#"    #[bhav]
    fn a(sim: Sim) {
        let ev: i16 = 7;
        temp[0] = 1;
        while !animate(clip: 0, table: 0, local: ev) {}
        sim.person[3] = ev;
    }"#,
    );
    // `temp[0] = 1` is a plain Expression after the store, so dead-store elimination drops
    // the store if liveness calls it dead.
    let air = expand("out_keep.antics", &src);
    let a = tree(&air, "a");
    assert!(a.contains("= 7;"), "the initial store stays:\n{a}");
}

/// `#[frame(locals = K)]` sizes the frame with no `min_locals` floor, so an `#[slot]` word the body
/// never names does not hold the named locals above it: dropping the save would move the
/// `idle` scratch word that `$Local:1` reads.
#[test]
fn frame_locals_sets_no_allocation_floor() {
    let members = r#"    #[bhav(args = 1, locals = 8)]
    fn a(sim: Sim) {
        #[slot(7)] let spare: i16;
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
        temp[2] = param[0];
        sleep(1);
        dialog(kind: ok, title: title, message: msg);
    }"#;
    let air = expand("frame_floor.antics", &obj("Was $Local:1", members));
    let a = tree(&air, "a");
    assert!(!a.contains("min_locals"), "{a}");
    // `#[slot]` placement puts the kept save in word 0.
    assert!(a.contains("local[0] := stack_obj;"), "{a}");
}

/// The inherit check reads the entry object from `.air` tokens, so a comment naming it is not a
/// read, and neither is a raw block's own local, constant or label of that name: an inherited
/// tree gets no entry save for them, as on main. A raw block may not declare a name its tree's
/// lowering declares too, and may declare one otherwise. Discarded code declares nothing.
#[test]
fn inherit_check_ignores_comments_and_raw_blocks_declare_no_compiler_local() {
    let quiet = |raw: &str| {
        obj(
            "Plain",
            &format!(
                "    #[bhav(context = inherit)] fn quiet() {{\n        {raw}\n        return true;\n    }}"
            ),
        )
    };
    expand(
        "inherit_comment.antics",
        &quiet("raw {\n            // __self_obj\n            temp[0] := 1;\n        }"),
    );
    let own = [
        "raw { let __self_obj = 1; temp[0] := __self_obj; }",
        "raw { const __self_obj = 7; temp[0] := __self_obj; }",
        "raw { @__self_obj: temp[0] := 1; }",
    ];
    for raw in own {
        let air = expand("inherit_own.antics", &quiet(raw));
        assert!(
            !tree(&air, "quiet").contains("= stack_obj"),
            "{raw}:\n{air}"
        );
    }
    // A Sim tree whose lowering saves the entry object refuses a raw declaration of its name.
    let saving = |raw: &str| {
        obj(
            "Plain",
            &format!(
                "    #[bhav]
    fn s(sim: Sim) {{\n        set_to_next(all, dest: stack_obj);\n        \
                 stop_all_sounds(target: self);\n        {raw}\n    }}"
            ),
        )
    };
    for raw in own {
        let err = compile_v2_source(
            "saving_reserved.antics",
            &saving(raw),
            &PackResolverConfig::default(),
        )
        .expect_err(raw);
        assert!(
            err.message.contains("compiler's entry object"),
            "{raw}: {err}"
        );
    }
    // Code a constant condition discards emits nothing, so it declares nothing either.
    let air = expand(
        "discarded_reserved.antics",
        &saving("if false {\n            raw { let __self_obj = 1; }\n        }"),
    );
    assert!(tree(&air, "s").contains(SAVE), "{air}");
    // A tree whose lowering declares no such local may declare one, as on main.
    expand(
        "inherit_saved_name.antics",
        &quiet("raw { let __saved_stack_obj = 1; temp[0] := __saved_stack_obj; }"),
    );
    let plain = obj(
        "Plain",
        "    #[bhav] fn probe() {\n        raw { const __self_obj = 17; temp[3] := __self_obj; }\n    }",
    );
    let air = expand("raw_const_self.antics", &plain);
    assert!(
        tree(&air, "probe").contains("const __self_obj = 17;"),
        "{air}"
    );
}

/// Two helpers' out-operand locals that happen to share a spelling are distinct source locals:
/// they keep their own words, while copies of one declaration still share.
#[test]
fn same_spelled_out_operand_locals_of_different_helpers_do_not_share() {
    let src = obj(
        "Plain",
        r#"    fn first() {
        let event: i16 = me.attr[5];
        me.attr[5] = 0;
        me.attr[1] = event;
    }
    fn second() {
        let event: i16;
        random_into(event, 4)?;
        me.attr[2] = event;
    }
    #[bhav]
    fn a() {
        first();
        second();
    }
    #[bhav]
    fn b() {
        second();
        second();
    }"#,
    );
    let air = expand("inline_same_spelling.antics", &src);
    let a = tree(&air, "a");
    assert_eq!(a.matches("let __inline_v_event_").count(), 2, "{a}");
    let b = tree(&air, "b");
    assert_eq!(b.matches("let __inline_v_event_").count(), 1, "{b}");
}

/// `let Some(x)` bindings of different helpers are distinct declarations, and copies of one
/// declaration stay one group even when a specialization drops the out-operand write from one.
#[test]
fn out_operand_groups_follow_declarations_through_let_some_and_specialization() {
    let src = obj(
        "Plain",
        r#"    fn first(target: ObjectId) {
        let Some(value) = target.run_functional::<i16>(prep) else { return; };
        random_into(value, 4)?;
        temp[3] = value;
    }
    fn second(target: ObjectId) {
        let Some(value) = target.run_functional::<i16>(prep) else { return; };
        random_into(value, 6)?;
        temp[4] = value;
    }
    fn pick(draw: bool) {
        let value: i16;
        if draw {
            random_into(value, 4)?;
        } else {
            value = temp[0];
        }
        temp[3] = value;
        temp[4] = value;
    }
    fn nested(draw: bool) {
        pick(draw);
    }
    #[bhav]
    fn a() {
        first(stack_obj);
        second(stack_obj);
    }
    #[bhav]
    fn split_mode() {
        nested(true);
        nested(false);
    }"#,
    );
    let air = expand("origins.antics", &src);
    let a = tree(&air, "a");
    assert_eq!(a.matches("let __inline_v_value_").count(), 2, "{a}");
    let split = tree(&air, "split_mode");
    assert_eq!(split.matches("let __inline_v_value_").count(), 1, "{split}");
}

/// Every Expression node sets the scratch pointer that a later read of global 34 depends on, so
/// a tree that may read global 34 keeps its dialog entry save, as on main.
#[test]
fn a_tree_that_may_read_global_34_keeps_its_entry_save() {
    let src = obj(
        "Plain",
        r#"    #[bhav]
    fn a(sim: Sim) {
        raw { @entry: if 0 == global[34] { return true; } }
        set_to_next(all, dest: stack_obj);
        dialog(kind: ok, title: title, message: msg);
        return false;
    }"#,
    );
    let air = expand("global34_save.antics", &src);
    assert!(tree(&air, "a").contains(SAVE), "{air}");
}

/// A source constant spelled like the entry object is a constant to the raw block that reads
/// it, so an inherited tree needs no entry save for it.
#[test]
fn a_source_constant_named_like_the_entry_object_is_not_the_entry_object() {
    let src = r#"object "Round Three" {
    guid: 0x7F40_0900;
    const __self_obj = 7;
    #[bhav(context = inherit)]
    fn a() {
        raw { temp[0] := __self_obj; }
    }
}
"#;
    let air = expand("inherit_const.antics", src);
    let a = tree(&air, "a");
    assert!(!a.contains("= stack_obj"), "{a}");
}

/// An out-operand store makes an earlier store dead, but a tree that may read global 34 keeps
/// it: the store is an Expression, which sets the scratch pointer that read depends on.
#[test]
fn a_tree_that_may_read_global_34_keeps_stores_out_operands_kill() {
    let src = obj(
        "Plain",
        r#"    #[bhav]
    fn a() {
        let n: i16 = 7;
        if 0 == global[34] {
            temp[2] = 1;
        }
        random_into(n, 1)?;
        temp[1] = n;
    }"#,
    );
    let air = expand("out_global34.antics", &src);
    assert!(tree(&air, "a").contains("= 7;"), "{air}");
    // Without the read, the store is dead.
    let air = expand(
        "out_no_global.antics",
        &src.replace("global[34]", "global[5]"),
    );
    assert!(!tree(&air, "a").contains("= 7;"), "{air}");
}

/// A raw `op` whose opcode is a named constant may be an Expression reading global 34 too, so
/// a text tree with one keeps its unread entry save as on main, where text that reads no word
/// would otherwise let it go.
#[test]
fn a_raw_op_with_a_named_opcode_may_read_global_34() {
    let src = |raw: &str| {
        obj(
            "Plain",
            &format!(
                r#"    const EXPR = 2;
    #[bhav]
    fn a(sim: Sim) {{
        set_to_next(all, dest: stack_obj);
        {raw}
        dialog(kind: ok, title: title, message: msg);
    }}"#
            ),
        )
    };
    let air = expand(
        "raw_named_op.antics",
        &src("raw { op EXPR (0, 0, 34, 0, 0, 3, 14, 6); }"),
    );
    assert!(tree(&air, "a").contains(SAVE), "{air}");
    let air = expand("raw_plain_op.antics", &src("raw { temp[2] := 1; }"));
    assert!(!tree(&air, "a").contains(SAVE), "{air}");
}

/// Only an Expression (opcode 2) whose right operand is global 34 (scope byte 7 = 6, index bytes
/// 2..4 = 34) reads it that way: a raw `op` with another literal opcode or operand reads none, so
/// the object's other trees keep this branch's rules (here, an initializer an out-operand kills
/// goes).
#[test]
fn only_a_raw_expression_or_ui_effect_naming_global_34_reads_it() {
    let src = |raw: &str| {
        obj(
            "Plain",
            &format!(
                r#"    #[bhav]
    fn a() {{
        let n: i16 = 7;
        if 0 == global[5] {{
            temp[2] = 1;
        }}
        random_into(n, 1)?;
        temp[1] = n;
    }}
    #[bhav]
    fn b() {{
        raw {{ {raw} }}
    }}"#
            ),
        )
    };
    let air = expand(
        "raw_global34_op.antics",
        &src("op 2 (0, 0, 34, 0, 0, 3, 14, 6);"),
    );
    assert!(tree(&air, "a").contains("= 7;"), "{air}");
    // Bytes separated by spaces encode the same instruction.
    let air = expand(
        "raw_global34_spaced.antics",
        &src("op 2 (0 0 34 0 0 3 14 6);"),
    );
    assert!(tree(&air, "a").contains("= 7;"), "{air}");
    // A UI Effect (opcode 34) whose variable (scope byte 1, index bytes 2..4) is global 34.
    let air = expand(
        "raw_global34_ui_effect.antics",
        &src("op 34 (2, 6, 34, 0, 0, 0, 0, 0);"),
    );
    assert!(tree(&air, "a").contains("= 7;"), "{air}");
    for raw in [
        "op 2 (0, 0, 5, 0, 0, 3, 14, 6);",
        "op 2 (0 0 5 0 0 3 14 6);",
        "op 4 (0, 0, 0, 0, 0, 0, 0, 0);",
        "op 34 (2, 6, 5, 0, 0, 0, 0, 0);",
        "op 34 (2, 7, 34, 0, 0, 0, 0, 0);",
    ] {
        let air = expand("raw_other_op.antics", &src(raw));
        assert!(!tree(&air, "a").contains("= 7;"), "{raw}:\n{air}");
    }
}

/// Create Object with flag 0x10 or 0x02 writes word 0 of the new object's bottom frame, its
/// `main`. A bottom frame with a local read before it is written keeps main's rules, so the
/// same local lives in word 0; one whose locals are all written first may still drop an
/// initializer an out-operand write kills.
#[test]
fn a_bottom_frame_that_reads_a_local_first_keeps_main_rules() {
    let src = |use_line: &str| {
        obj(
            "Plain",
            &format!(
                r#"    main {{
        let a: i16 = 7;
        let incoming: i16;
        temp[2] = 1;
        let n = a;
        temp[3] = 1;
        random_into(n, 1)?;
        {use_line}
        temp[1] = n;
    }}"#
            ),
        )
    };
    let air = expand("bottom_reads_first.antics", &src("me.attr[0] = incoming;"));
    let main = tree(&air, "Main");
    assert!(main.contains("= 7;"), "{air}");
    let first = main.find("let ").expect("a local");
    assert!(main[first..].starts_with("let __v_a_0 = 7;"), "{air}");
    let air = expand("bottom_writes_first.antics", &src("me.attr[0] = 1;"));
    assert!(!tree(&air, "Main").contains("= 7;"), "{air}");
}

/// A call carries the scratch pointer into the callee, so a tree whose callee may read global 34
/// keeps the Expressions main keeps: every tree of an object any of whose trees may read it
/// takes main's rules.
#[test]
fn a_callee_that_may_read_global_34_keeps_main_rules_in_its_caller() {
    let src = |cell: &str| {
        obj(
            "Plain",
            &format!(
                r#"    #[bhav(context = inherit)]
    fn reader() {{
        raw {{ my.motive[0] := {cell}; temp[2] := my.motive[0]; }}
        return true;
    }}
    #[bhav]
    fn witness(sim: Sim) {{
        reader();
        if temp[4] == 0 {{ return true; }}
        set_to_next(all, dest: stack_obj);
        dialog(kind: ok, title: title, message: msg);
    }}"#
            ),
        )
    };
    let air = expand("callee_global34.antics", &src("global[34]"));
    assert!(tree(&air, "witness").contains(SAVE), "{air}");
    let air = expand("callee_global5.antics", &src("global[5]"));
    assert!(!tree(&air, "witness").contains(SAVE), "{air}");
}

/// Declaration groups keep words main's coalescing shares, so a tree they would push to the
/// largest frame (255 locals) coalesces as main does and still compiles.
#[test]
fn declaration_groups_never_overflow_the_largest_frame() {
    let mut src = String::from("object \"Round Three\" {\n    guid: 0x7F40_0900;\n");
    for i in 0..16 {
        src.push_str(&format!(
            "    fn h{i}() {{ let n{i}: i16 = me.attr[0]; random_into(n{i}, 4)?; temp[0] = n{i}; }}\n"
        ));
    }
    src.push_str("    #[bhav] fn a() {\n");
    for i in 0..240 {
        src.push_str(&format!("        let x{i}: i16;\n"));
    }
    for i in (0..240).step_by(2) {
        src.push_str(&format!(
            "        if x{i} != x{} {{ return false; }}\n",
            i + 1
        ));
    }
    for i in 0..16 {
        src.push_str(&format!("        h{i}();\n"));
    }
    src.push_str("        return true;\n    }\n}\n");
    expand("largest_frame.antics", &src);
}

/// Keeping an unread entry save is not always safe: it is an Expression, and a read of global
/// 34 before any other Expression faults without one. A raw-layout tree that main drops the
/// save of drops it too.
#[test]
fn an_unread_entry_save_goes_before_a_global_34_read_as_on_main() {
    let src = obj(
        "Plain",
        r#"    #[bhav(locals = 1)]
    fn a(sim: Sim) {
        raw { temp[2] := global[34]; }
        set_to_next(all, dest: stack_obj);
        return true;
    }"#,
    );
    let air = expand("global34_first.antics", &src);
    assert!(!tree(&air, "a").contains(SAVE), "{air}");
}

/// A raw block may declare a qualified constant whose first part is spelled like the entry
/// object: it declares neither the entry object nor a read of it.
#[test]
fn a_qualified_raw_constant_is_not_the_entry_object() {
    let src = r#"object "Round Three" {
    guid: 0x7F40_0900;
    #[bhav(context = inherit)]
    fn a() {
        raw { const __self_obj.tuning = 42; temp[0] := __self_obj.tuning; }
    }
}
"#;
    let air = expand("raw_qualified.antics", src);
    assert!(!tree(&air, "a").contains("= stack_obj"), "{air}");
}

/// The raw-layout path keeps the entry save of a tree that may read global 34 where main does:
/// there a `random` with a constant range does not define its destination (main's literal-range
/// rule), so the local after the save is read before it is written. Elsewhere the range defines
/// it and the unread save goes.
#[test]
fn a_raw_layout_tree_that_may_read_global_34_keeps_its_entry_save() {
    let src = obj(
        "Plain",
        r#"    #[bhav]
    fn a(sim: Sim) {
        let n: i16;
        raw { const K = 2; temp[K] := global[34]; }
        set_to_next(all, dest: stack_obj);
        random_into(n, Limits.draw)?;
        temp[3] = n;
    }
    bcon 4097 "Limits" { draw = 6 }"#,
    );
    let air = expand("raw_global34.antics", &src);
    assert!(tree(&air, "a").contains(SAVE), "{air}");
    let air = expand(
        "raw_global5.antics",
        &src.replace("global[34]", "global[5]"),
    );
    assert!(!tree(&air, "a").contains(SAVE), "{air}");
}

/// A raw `call x` or `subroutine(x)` names a tree, not the entry object, even when the tree is
/// spelled like it.
#[test]
fn a_raw_call_to_a_tree_named_like_the_entry_object_is_not_a_read() {
    let src = r#"object "Round Three" {
    guid: 0x7F40_0900;
    #[bhav(context = inherit)]
    fn __self_obj() {
        return true;
    }
    #[bhav(context = inherit)]
    fn a() {
        raw { call __self_obj; }
        return true;
    }
}
"#;
    let air = expand("raw_call_named.antics", src);
    assert!(!tree(&air, "a").contains("= stack_obj"), "{air}");
    for form in ["subroutine(__self_obj);", "subroutine __self_obj;"] {
        let src = src.replace("call __self_obj;", form);
        let air = expand("raw_subroutine_named.antics", &src);
        assert!(!tree(&air, "a").contains("= stack_obj"), "{form}:\n{air}");
    }
}

/// Pinning a `push_interaction` handle into word 0 fills word 0 from the start, which a
/// primitive may read implicitly (`relationship` without `local:` reads the word-0 pair), so the
/// tree pins exactly where main's literal-range analysis does, whatever its text reads.
#[test]
fn push_interaction_pins_only_where_main_pins() {
    let members = |range: &str, text: &str| {
        format!(
            r#"    #[bhav]
    fn a(sim: Sim) {{
        let h = stack_obj;
        let n: i16;
        random_into(n, {range})?;
        temp[3] = n;
        relationship(column: 0, dir: 2, value: temp[2]);
        {text}
        push_interaction(menu: 1, object: h, priority: 0);
    }}
    bcon 4097 "Limits" {{ draw = 1 }}"#
        )
    };
    // A BCON range is not one main's analysis reads, so the copy stays, with or without text.
    for (name, text) in [
        (
            "Was $Local:1",
            "dialog(kind: ok, title: title, message: msg);",
        ),
        ("Plain", ""),
    ] {
        let air = expand("pin_bcon.antics", &obj(name, &members("Limits.draw", text)));
        let a = tree(&air, "a");
        assert!(
            !a.contains("local[0] := stack_obj;"),
            "the copy stays:\n{a}"
        );
    }
    // A literal range lets main pin, whatever the text reads, and the tree pins as main.
    let air = expand(
        "pin_literal.antics",
        &obj(
            "Was $Local:1",
            &members("1", "dialog(kind: ok, title: title, message: msg);"),
        ),
    );
    let a = tree(&air, "a");
    assert!(a.contains("local[0] := stack_obj;"), "pinned:\n{a}");
    // An initialized local is defined before the draw, so main pins with a BCON range too, on
    // the body its own rules give (which keep the initializer the draw overwrites).
    let initialized = members("Limits.draw", "").replace("let n: i16;", "let n: i16 = 7;");
    let air = expand("pin_initialized.antics", &obj("Plain", &initialized));
    let a = tree(&air, "a");
    assert!(a.contains("local[0] := stack_obj;"), "pinned:\n{a}");
    assert!(a.contains("= 7;"), "the initializer stays:\n{a}");
}

/// Create Object with flag 0x10 writes word 0 of the new object's bottom frame (and faults when
/// it has none), so a text tree that may run as one keeps main's text rule: its declarations
/// keep their words. A tree with no text empties its frame as on main.
#[test]
fn a_bottom_frame_text_tree_keeps_main_text_rule() {
    let src = |text: &str| {
        obj(
            "Plain",
            &format!(
                "    main {{\n        let spare: i16 = 7;\n        temp[1] = 1;\n        {text}\n    }}"
            ),
        )
    };
    let (locals, _) = compiled(&src("dialog(kind: ok, title: title, message: msg);"), 4097);
    assert_eq!(locals, 1);
    let (locals, _) = compiled(&src(""), 4097);
    assert_eq!(locals, 0);
    // A local read before it is written stays in the word main gives it, after `spare`.
    let read_first = obj(
        "Plain",
        "    main {\n        let spare: i16;\n        let inherited: i16;\n        \
         me.data[11] = inherited;\n        dialog(kind: ok, title: title, message: msg);\n    }",
    );
    assert_eq!(compiled(&read_first, 4097).0, 2);
    // The inherit check's Sim pass over such a tree takes the same rule, so both passes agree.
    expand(
        "inherit_bottom.antics",
        &obj(
            "Plain",
            "    #[bhav(context = inherit)]
    fn a() {\n        let spare: i16 = 7;\n        temp[1] = 1;\n        \
             dialog(kind: ok, title: title, message: msg);\n    }",
        ),
    );
}

/// A tree that may read global 34 shares inlined out-operand locals as main does, so it fits
/// the frame main fits it in.
#[test]
fn a_tree_that_may_read_global_34_shares_out_operand_locals_as_main() {
    let src = r#"object "Round Three" {
    guid: 0x7F40_0900;
    fn first() {
        let event: i16 = temp[5];
        temp[4] = 0;
        temp[1] = event;
    }
    fn second() {
        let event: i16 = temp[6];
        temp[5] = 0;
        random_into(event, 4)?;
        temp[2] = event;
    }
    #[bhav]
    fn a() {
        first();
        second();
        temp[3] = global[34];
    }
}
"#;
    assert_eq!(compiled(src, 4098).0, 1, "global 34: main's coalescing");
    // Without the read, the two helpers' locals keep their own words.
    let plain = src.replace("        temp[3] = global[34];\n", "");
    assert_eq!(compiled(&plain, 4098).0, 2, "declaration groups");
    // So does a tree whose header fixes its frame size, by `locals:` or by a format.
    expand(
        "fixed_frame.antics",
        &plain.replace("    #[bhav] fn a() {", "    #[bhav(locals = 1)] fn a() {"),
    );
}

/// `#[frame(format = 0x8001)]` holds four locals, so its tree shares inlined out-operand locals as
/// main does and fits them.
#[test]
fn a_format_limited_frame_shares_out_operand_locals_as_main() {
    let helpers: String = (0..5)
        .map(|i| {
            format!(
                "    fn h{i}() {{\n        let n{i}: i16 = me.attr[0];\n        random_into(n{i}, 4)?;\n        temp[{i}] = n{i};\n    }}\n"
            )
        })
        .collect();
    let src = format!(
        "object \"Round Three\" {{\n    guid: 0x7F40_0900;\n{helpers}    #[bhav(format = 0x8001)] fn a() {{\n        h0();\n        h1();\n        h2();\n        h3();\n        h4();\n    }}\n}}\n"
    );
    expand("format_limit.antics", &src);
}
