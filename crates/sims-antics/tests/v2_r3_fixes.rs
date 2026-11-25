//! `.antics` v2 Round 3 lowering fixes: the dialog entry save, out-operand writes, the
//! `@context(inherit)` check, `self` before the first cursor move, `push_interaction` on an
//! `@slot` handle and `job_data` comparisons. All sources are synthetic.

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
const DIALOG_TREE: &str = r#"    tree a (sim: Sim) {
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
    let pinned = r#"    tree a (sim: Sim) {
        @slot(1) let n = sim.person[3];
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
    let highest = r#"    tree a (sim: Sim) {
        @slot(0) let n: i16;
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
        "{DIALOG_TREE}\n    tree b (sim: Sim) {{\n        dialog(kind: ok, title: \"T\", message: \"Was $Local:0\");\n    }}"
    );
    let air = expand("save_late.antics", &obj("Plain text", &members));
    assert!(tree(&air, "a").contains(SAVE), "{air}");
}

/// A `random_into` whose range is a constant (a literal or a BCON) stores the draw in its
/// destination on both exits, so the destination is written before it is read.
#[test]
fn random_into_a_constant_range_defines_its_destination() {
    let members = r#"    tree a (sim: Sim) {
        let n: i16;
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
    // A BCON range stays a `bcon[..]` cell in `.air`. (The raw statement keeps ordinary
    // dead-store elimination from dropping the save before the analysis sees it.)
    let members = r#"    tree a (sim: Sim) {
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
        r#"    @id(4300) tree a (sim: Sim) {
        let n = sim.person[3];
        if !create_object(0x7F40_0901, mode: off_lot, flags: 1, dest: stack_obj) {
            return false;
        }
        relationship(column: 0, dir: 0, value: n);
        stack_obj.attr[0] = n;
    }
    @id(4301) tree b (sim: Sim) {
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
        r#"    @context(inherit) tree walk {
        for p in lot.sims() {
            if !(p == me.data[11]) {
                if p.kind == 1 {
                    return true;
                }
            }
        }
        return false;
    }
    tree caller (sim: Sim) {
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
    @context(inherit) tree beep {
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
    assert_eq!((span.line, span.column), (7, 13), "{err}");
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
    fn both() {
        let ev: i16;
        let pick: i16;
        random_into(pick, 2)?;
        if pick == 0 {
            animate(clip: 1, table: 0, local: ev);
        }
    }
    tree a {
        react(1);
        react(2);
    }
    tree b {
        roll();
        roll();
    }
    tree c {
        both();
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
    tree a {
        peek(2);
        peek(3);
    }"#,
    );
    let air = expand("inline_read_first.antics", &src);
    let body = tree(&air, "a");
    assert_eq!(body.matches("let __inline_v_ev_").count(), 2, "{body}");
}

/// In a Sim tree that moves the cursor with a search, `self` reads the stack object in place
/// until the first statement that may move it; after that (and in a loop that moves it) it
/// switches to the entry object.
#[test]
fn self_reads_in_place_before_the_first_cursor_move() {
    let src = obj(
        "Plain",
        r#"    tree a (sim: Sim) {
        if self.value == 1 {
            return true;
        }
        stack_obj = 0;
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
        stack_obj.attr[0] = self.value;
    }
    tree b (sim: Sim) {
        loop {
            if self.value == 1 {
                break;
            }
            if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
                return false;
            }
        }
    }"#,
    );
    let air = expand("entry_cursor.antics", &src);
    let a = tree(&air, "a");
    assert!(a.contains(SAVE), "{a}");
    let first_read = a.find("if stack.attr[0] == 1 {").expect("in-place read");
    assert!(
        !a[..first_read].contains("stack_obj :="),
        "no switch before it:\n{a}"
    );
    assert!(
        a[first_read..].contains("stack_obj := __self_obj;"),
        "the later read switches:\n{a}"
    );
    let b = tree(&air, "b");
    assert!(b.contains("stack_obj := __self_obj;"), "{b}");
}

/// Only a primitive's `self` target and a `self` cell compared with a constant are in place:
/// a write, a read into an argument and a constant-folded flag test are staged as on main.
#[test]
fn the_in_place_prefix_covers_only_targets_and_constant_comparisons() {
    let src = obj(
        "Plain",
        r#"    tree a (sim: Sim) {
        stop_all_sounds(target: self);
        if self.value == 1 {
            temp[4] = 1;
        }
        self.value += 1;
        set_to_next(all, dest: stack_obj);
    }"#,
    );
    let air = expand("prefix_sites.antics", &src);
    let a = tree(&air, "a");
    let cmp = a
        .find("if stack.attr[0] == 1")
        .expect("in-place comparison");
    assert!(
        !a[..cmp].contains("stack_obj :="),
        "the target is selected in place:\n{a}"
    );
    let write = a.find("stack.attr[0] +=").expect("the self write");
    assert!(
        a[cmp..write].contains("stack_obj := __self_obj;"),
        "the write switches as on main:\n{a}"
    );
}

/// `push_interaction(object: h)` with `h` an `@slot(N)` handle reads the object from word N.
#[test]
fn push_interaction_reads_a_slot_handle_in_place() {
    let src = obj(
        "Plain",
        r#"    tree a (sim: Sim) {
        @slot(1) let target = stack_obj;
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
        r#"    tree a (sim: Sim) {
        @slot(1) let target = stack_obj;
        push_interaction(menu: 1, object: target, priority: 0, flags: 2);
    }"#,
    );
    let err = compile_v2_source("push_flag.antics", &refused, &PackResolverConfig::default())
        .expect_err("explicit 0x02");
    assert!(err.message.contains("implied by an object handle"), "{err}");
}

/// `job_data(..) == k` reads the career cell in the comparison node itself; a comparison whose
/// other side lowers to setup keeps the copy.
#[test]
fn job_data_compares_without_a_copy() {
    let src = obj(
        "Plain",
        r#"    tree a (sim: Sim) {
        return job_data(sim.person[60], sim.person[61], 22) == 1;
    }
    tree b (sim: Sim) {
        return job_data(sim.person[60], sim.person[61], 22) == job_data(1, 2, 3);
    }"#,
    );
    let air = expand("job_cmp.antics", &src);
    let a = tree(&air, "a");
    assert!(a.contains("if scope(33, 22) == 1 {"), "{a}");
    assert!(!a.contains("__job"), "{a}");
    let b = tree(&air, "b");
    assert!(b.contains("let __job_0 = scope(33, 22);"), "{b}");
}

/// A raw label before the first cursor move may be reached again from after it (a raw back
/// edge), so source order is not control order: a tree whose raw blocks define a label reads
/// `self` through the entry object everywhere.
#[test]
fn a_raw_label_disables_in_place_self() {
    let src = obj(
        "Plain",
        r#"    tree a (sim: Sim) {
        raw { @again: temp[3] := 1; }
        if self.value == 1 {
            temp[4] = 1;
        }
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
        raw { random(temp[2], 1) -> true: @again, false: false; }
    }"#,
    );
    let air = expand("raw_back_edge.antics", &src);
    let a = tree(&air, "a");
    assert!(
        !a.contains("stack.attr[0] == 1"),
        "the self read is staged:\n{a}"
    );
}

/// `.air` keeps `param[0]` in the word after the named locals around an `idle` when a tree uses
/// both, so text that may read that word keeps the save that would move it.
#[test]
fn dialog_entry_save_stays_when_text_may_read_the_idle_scratch_word() {
    let members = r#"    @frame(args: 1) tree a (sim: Sim) {
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

/// The VM resolves a comparison's right operand first, so `job_data(..) <op> x` reads the cell
/// in place only when `x` cannot fault; a temp keeps the copy (the career lookup faults first).
#[test]
fn job_data_compares_in_place_only_against_a_constant() {
    let src = obj(
        "Plain",
        r#"    tree a (sim: Sim) {
        return job_data(9999, 1, 22) == temp[99];
    }"#,
    );
    let air = expand("job_temp.antics", &src);
    let a = tree(&air, "a");
    assert!(a.contains("let __job"), "{a}");
}

/// An out-operand that writes on only one exit leaves the old value on the other: the loop
/// exits when `animate` returns true, which does not write `ev`, so its initial 7 must stay.
#[test]
fn an_out_operand_keeps_the_old_value_on_the_exit_it_does_not_write() {
    let src = obj(
        "Plain",
        r#"    tree a (sim: Sim) {
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

/// A raw branch to a node by number may also jump back before the first move.
#[test]
fn a_positional_raw_branch_disables_in_place_self() {
    let src = obj(
        "Plain",
        r#"    tree a (sim: Sim) {
        if self.value == 1 {
            temp[4] = 1;
        }
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
        raw { random(temp[2], 1) -> true: 1, false: false; }
    }"#,
    );
    let air = expand("raw_positional.antics", &src);
    let a = tree(&air, "a");
    assert!(
        !a.contains("stack.attr[0] == 1"),
        "the self read is staged:\n{a}"
    );
}

/// A padded raw primitive runs the primitive it wraps: a padded search moves the cursor, so
/// it ends the in-place prefix.
#[test]
fn a_padded_raw_search_ends_the_in_place_prefix() {
    let src = obj(
        "Plain",
        r#"    tree a (sim: Sim) {
        raw { set_to_next(all) pad 0xA5; }
        if self.value == 1 {
            temp[4] = 1;
        }
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
    }"#,
    );
    let air = expand("raw_padded_search.antics", &src);
    let a = tree(&air, "a");
    assert!(
        !a.contains("stack.attr[0] == 1"),
        "the self read is staged:\n{a}"
    );
}

/// A padded raw action string formats text like a plain one, so a string that may read a
/// moved word keeps the save.
#[test]
fn a_padded_raw_text_primitive_keeps_the_save() {
    let members = r#"    tree a (sim: Sim) {
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
        let n = sim.person[3];
        stack_obj.attr[0] = n;
        // The space before `(` keeps the plain text scan from seeing the primitive.
        raw { action_string (table: 301, index: 2) pad 0xff; }
    }"#;
    let air = expand("raw_padded_text.antics", &obj("Was $Local:0", members));
    assert!(tree(&air, "a").contains(SAVE), "{air}");
}

/// `@frame(locals: K)` sizes the frame with no `min_locals` floor, so an `@slot` word the body
/// never names does not hold the named locals above it: dropping the save would move the
/// `idle` scratch word that `$Local:1` reads.
#[test]
fn frame_locals_sets_no_allocation_floor() {
    let members = r#"    @frame(args: 1, locals: 8) tree a (sim: Sim) {
        @slot(7) let spare: i16;
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
    // `@slot` placement puts the kept save in word 0.
    assert!(a.contains("local[0] := stack_obj;"), "{a}");
}

/// A raw block of the tree may branch to a label an inlined helper's raw block emits, so a
/// helper's label also rules out in-place `self`.
#[test]
fn a_helper_raw_label_disables_in_place_self() {
    let src = obj(
        "Plain",
        r#"    fn checkpoint() {
        raw { @again: temp[3] := 1; }
    }
    tree a (sim: Sim) {
        checkpoint();
        if self.value == 1 {
            temp[4] = 1;
        }
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
        raw { random(temp[2], 1) -> true: @again, false: false; }
    }"#,
    );
    let air = expand("raw_helper_label.antics", &src);
    let a = tree(&air, "a");
    assert!(
        !a.contains("stack.attr[0] == 1"),
        "the self read is staged:\n{a}"
    );
}

/// The inherit check reads the entry object from `.air` tokens, so a comment naming it is not a
/// read; a raw block may not declare a local, constant or label of that name.
#[test]
fn inherit_check_ignores_comments_and_raw_blocks_cannot_declare_the_entry_object() {
    let quiet = |raw: &str| {
        obj(
            "Plain",
            &format!(
                "    @context(inherit) tree quiet {{\n        {raw}\n        return true;\n    }}"
            ),
        )
    };
    expand(
        "inherit_comment.antics",
        &quiet("raw {\n            // __self_obj\n            temp[0] := 1;\n        }"),
    );
    for raw in [
        "raw { let __self_obj = 1; temp[0] := __self_obj; }",
        "raw { const __self_obj = 7; temp[0] := __self_obj; }",
        "raw { @__self_obj: temp[0] := 1; }",
        "raw { let __saved_stack_obj = 1; temp[0] := __saved_stack_obj; }",
    ] {
        let err = compile_v2_source(
            "inherit_reserved.antics",
            &quiet(raw),
            &PackResolverConfig::default(),
        )
        .expect_err(raw);
        assert!(
            err.message.contains("compiler's entry object"),
            "{raw}: {err}"
        );
    }
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
    tree a {
        first();
        second();
    }
    tree b {
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

/// A raw label can reach the tree from routes the source does not show as a call: a parameter
/// default that calls a helper, and a helper named like the `animate` statement. The emitted
/// `.air` decides, so neither keeps the in-place `self` prefix.
#[test]
fn raw_labels_from_defaults_and_statement_named_helpers_disable_in_place_self() {
    let tail = r#"
        if self.value == 1 {
            temp[4] = 1;
        }
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
        raw { random(temp[2], 1) -> true: @again, false: false; }
    }"#;
    let default = format!(
        r#"    fn checkpoint() -> bool {{
        raw {{ @again: temp[3] := 1; }}
        return temp[0] == 0;
    }}
    fn use_default(x: i16 = if checkpoint() {{ 1 }} else {{ 2 }}) {{
        temp[0] = x;
    }}
    tree a (sim: Sim) {{
        use_default();{tail}"#
    );
    let animate = format!(
        r#"    fn animate() {{
        raw {{ @again: temp[3] := 1; }}
    }}
    tree a (sim: Sim) {{
        animate();{tail}"#
    );
    for (name, members) in [
        ("raw_default.antics", default),
        ("raw_animate.antics", animate),
    ] {
        let air = expand(name, &obj("Plain", &members));
        let a = tree(&air, "a");
        assert!(
            !a.contains("stack.attr[0] == 1"),
            "{name}: the self read is staged:\n{a}"
        );
    }
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
    tree a {
        first(stack_obj);
        second(stack_obj);
    }
    tree split_mode {
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
/// a tree that may read global 34 keeps the Expression that stages a `self` read, and keeps its
/// dialog entry save, as on main.
#[test]
fn a_tree_that_may_read_global_34_keeps_its_expressions() {
    let src = obj(
        "Plain",
        r#"    tree a (sim: Sim) {
        if self.value == 1 {
            temp[2] = global[34];
        }
        set_to_next(all, dest: stack_obj);
        return false;
    }"#,
    );
    let air = expand("global34_self.antics", &src);
    let a = tree(&air, "a");
    assert!(
        !a.contains("stack.attr[0] == 1"),
        "the self read is staged:\n{a}"
    );
    // Another global leaves the comparison in place.
    let air = expand(
        "global5_self.antics",
        &src.replace("global[34]", "global[5]"),
    );
    let a = tree(&air, "a");
    assert!(a.contains("stack.attr[0] == 1"), "in place:\n{a}");
    let src = obj(
        "Plain",
        r#"    tree a (sim: Sim) {
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
    @context(inherit) tree a {
        raw { temp[0] := __self_obj; }
    }
}
"#;
    let air = expand("inherit_const.antics", src);
    let a = tree(&air, "a");
    assert!(!a.contains("= stack_obj"), "{a}");
}

/// A `let Some` binding is allocated after its `else` body is lowered; its declaration is still
/// the `let Some`, so two copies of a helper whose constant argument picks different arms share
/// the binding's word.
#[test]
fn a_let_some_binding_keeps_its_declaration_after_the_else_body() {
    let src = r#"object "Round Three" {
    guid: 0x7F40_0900;
    fn pick(target: ObjectId, mode: bool) {
        let Some(value) = target.run_functional::<i16>(prep) else {
            if mode {
                temp[2] = 1;
                return;
            } else {
                temp[2] = 2;
                return;
            }
        };
        random_into(value, 4)?;
        temp[3] = value;
    }
    @frame(locals: 3) tree a {
        pick(stack_obj, true);
        pick(stack_obj, false);
    }
}
"#;
    let air = expand("let_some_origin.antics", src);
    assert!(tree(&air, "a").contains("(locals: 3)"), "{air}");
}

/// An out-operand store makes an earlier store dead, but a tree that may read global 34 keeps
/// it: the store is an Expression, which sets the scratch pointer that read depends on.
#[test]
fn a_tree_that_may_read_global_34_keeps_stores_out_operands_kill() {
    let src = obj(
        "Plain",
        r#"    tree a {
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

/// A raw `op` whose opcode is a named constant may be an Expression reading global 34 too.
#[test]
fn a_raw_op_with_a_named_opcode_may_read_global_34() {
    let src = obj(
        "Plain",
        r#"    const EXPR = 2;
    tree a (sim: Sim) {
        let r: i16;
        random_into(r, 1);
        stop_all_sounds(target: self);
        raw { op EXPR (0, 0, 34, 0, 0, 3, 14, 6); }
        set_to_next(all, dest: stack_obj);
    }"#,
    );
    let air = expand("raw_named_op.antics", &src);
    let a = tree(&air, "a");
    assert!(
        a.contains("stack_obj := __self_obj;"),
        "self is selected by a switch:\n{a}"
    );
}

/// Dropping the staging local of an in-place `self` read moves the named locals after it, so a
/// tree whose text may read a frame word gets no prefix.
#[test]
fn text_that_reads_frame_words_disables_in_place_self() {
    let src = obj(
        "Number $Local:2",
        r#"    tree a (sim: Sim) {
        if self.value == 1 {
            sim.person[4] = 1;
        }
        let n = sim.person[3];
        stack_obj.attr[1] = n;
        dialog(kind: ok, title: title, message: msg, icon_mode: 1);
        set_to_next(all, dest: stack_obj);
        stack_obj.attr[1] = self.value;
    }"#,
    );
    let air = expand("text_prefix.antics", &src);
    let a = tree(&air, "a");
    assert!(
        !a.contains("stack.attr[0] == 1"),
        "the self read is staged:\n{a}"
    );
}

/// The VM resolves a comparison's right operand first, so an in-place `self` read on the left
/// is staged unless the right side is a constant: a faulting right operand must not fault
/// before the `self` read does.
#[test]
fn an_in_place_self_comparison_keeps_its_operand_order() {
    let src = |rhs: &str| {
        obj(
            "Plain",
            &format!(
                r#"    tree a (sim: Sim) {{
        if self.value == {rhs} {{
            return false;
        }}
        set_to_next(all, dest: stack_obj);
    }}"#
            ),
        )
    };
    let air = expand("cmp_temp.antics", &src("temp[8]"));
    let a = tree(&air, "a");
    assert!(
        a.contains("let __pre_stack_obj"),
        "staged before a temp:\n{a}"
    );
    let air = expand("cmp_const.antics", &src("3"));
    let a = tree(&air, "a");
    assert!(
        a.contains("if stack.attr[0] == 3"),
        "in place against a constant:\n{a}"
    );
}

/// A raw block may declare a qualified constant whose first part is spelled like the entry
/// object: it declares neither the entry object nor a read of it.
#[test]
fn a_qualified_raw_constant_is_not_the_entry_object() {
    let src = r#"object "Round Three" {
    guid: 0x7F40_0900;
    @context(inherit) tree a {
        raw { const __self_obj.tuning = 42; temp[0] := __self_obj.tuning; }
    }
}
"#;
    let air = expand("raw_qualified.antics", src);
    assert!(!tree(&air, "a").contains("= stack_obj"), "{air}");
}

/// An in-place `self` read is staged and ordered as main stages the switched one: a loop
/// header reads it on every iteration, and the operand left of it is read first.
#[test]
fn an_in_place_self_read_keeps_main_staging_order() {
    let src = obj(
        "Plain",
        r#"    tree a (sim: Sim) {
        while self.value < sim.person[3] {
            self.value += 1;
        }
        if !set_to_next(guid, 0x7F40_0903, dest: stack_obj) {
            return false;
        }
    }
    tree b (sim: Sim) {
        if temp[8] == self.data[25] {
            return false;
        }
        set_to_next(all, dest: stack_obj);
    }
    tree c (sim: Sim) {
        if job_data(level: temp[8], career: self.data[25], field: 0) == 1 {
            return false;
        }
        set_to_next(all, dest: stack_obj);
    }"#,
    );
    let air = expand("self_order.antics", &src);
    let a = tree(&air, "a");
    let header = a
        .find("while true")
        .expect("the loop reads self in its body");
    assert!(
        !a[..header].contains("stack.attr[0]"),
        "no read before the loop:\n{a}"
    );
    let b = tree(&air, "b");
    let (temp, data) = (
        b.find("temp[8]").unwrap(),
        b.find("stack.data[25]").unwrap(),
    );
    assert!(temp < data, "temp[8] is read first:\n{b}");
    let c = tree(&air, "c");
    let (temp, data) = (
        c.find("temp[8]").unwrap(),
        c.find("stack.data[25]").unwrap(),
    );
    assert!(temp < data, "the level is read first:\n{c}");
}

/// The raw-layout path keeps the entry save of a tree that may read global 34.
#[test]
fn a_raw_layout_tree_that_may_read_global_34_keeps_its_entry_save() {
    let src = obj(
        "Plain",
        r#"    tree a (sim: Sim) {
        let n: i16;
        raw { temp[2] := global[34]; }
        set_to_next(all, dest: stack_obj);
        random_into(n, Limits.draw)?;
        temp[3] = n;
    }
    bcon 4097 "Limits" { draw = 6 }"#,
    );
    let air = expand("raw_global34.antics", &src);
    assert!(tree(&air, "a").contains(SAVE), "{air}");
}

/// Dropping a `job_data` comparison's copy removes a node, so a tree with a raw branch to a node
/// by number keeps the copy: the number would count the removed node.
#[test]
fn a_positional_raw_branch_keeps_the_job_data_copy() {
    let src = r#"object "Round Three" {
    guid: 0x7F40_0900;
    @frame(locals: 1) tree a {
        raw { op 2 (0, 0, 0, 0, 0, 2, 7, 7) -> true: 6, false: 6; }
        if job_data(1, 1, 0) == 1 {
            temp[3] = 21;
        }
        raw { if 1 == 1 { return false; } }
        temp[4] = 10;
        return true;
    }
}
"#;
    let air = expand("job_positional.antics", src);
    let a = tree(&air, "a");
    assert!(!a.contains("if scope(33, 0) == 1"), "the copy stays:\n{a}");
}

/// A raw `call` names a tree, not the entry object, even when the tree is spelled like it.
#[test]
fn a_raw_call_to_a_tree_named_like_the_entry_object_is_not_a_read() {
    let src = r#"object "Round Three" {
    guid: 0x7F40_0900;
    @context(inherit) tree __self_obj {
        return true;
    }
    @context(inherit) tree a {
        raw { call __self_obj; }
        return true;
    }
}
"#;
    let air = expand("raw_call_named.antics", src);
    assert!(!tree(&air, "a").contains("= stack_obj"), "{air}");
}
