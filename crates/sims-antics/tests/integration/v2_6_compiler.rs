//! `.antics` v2.6 Step 1: writable `local[N]`, nodes no branch reaches kept as bytes
//! (`dead_nodes { .. }`, `.air` `dead_nodes: [..]`) and pinned with
//! [`match_tree_pin_quarantined`], identical `if` arms under `keep_layout`, the loader's
//! check without a read ([`check_v2_file`]) and the renamed `std::globals` entries. All
//! sources and library names are synthetic.

use sims_antics::node_order::{
    DeadNode, QuarantinedPin, dead_nodes_block, match_tree_pin_quarantined, split_dead_nodes,
};
use sims_antics::v2::loader::check_v2_file;
use sims_antics::v2::{format_v2_source, is_v2_formatted};
use sims_antics::{CompiledObject, PackResolverConfig, compile_air, compile_v2_source};
use sims_formats::behavior::Instruction;

fn compile(src: &str) -> CompiledObject {
    compile_v2_source("v26.antics", src, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("must compile: {e}\n{src}"))
}

fn expect_err(src: &str, needle: &str) {
    let err = compile_v2_source("v26.antics", src, &PackResolverConfig::default())
        .expect_err(&format!("must fail:\n{src}"));
    assert!(
        err.message.contains(needle),
        "expected '{needle}' in: {err}"
    );
}

fn air(src: &str) -> String {
    sims_antics::v2::expand_v2_source_to_objects("v26.antics", src, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("must expand: {e}\n{src}"))
        .remove(0)
        .air_source
}

/// The `.air` lines of tree `name` (header included), trimmed.
fn tree_lines(air: &str, name: &str) -> Vec<String> {
    let at = air
        .find(&format!("\"{name}\""))
        .unwrap_or_else(|| panic!("tree {name} in .air:\n{air}"));
    let start = air[..at].rfind('\n').map_or(0, |i| i + 1);
    air[start..]
        .lines()
        .take_while(|l| *l != "}")
        .map(|l| l.trim().to_owned())
        .collect()
}

type Node = (u16, u8, u8, [u8; 8]);

fn as_nodes(instructions: &[Instruction]) -> Vec<Node> {
    instructions
        .iter()
        .map(|i| (i.opcode, i.on_true, i.on_false, i.operands))
        .collect()
}

/// Tree `id`'s instructions.
fn instructions(c: &CompiledObject, id: u16) -> Vec<Instruction> {
    c.behaviors[&id].instructions.clone()
}

/// A synthetic object with `items` as members and one tree `t` (id 4200):
/// `#[bhav(id = 4200{attrs})] fn t() { body }`.
fn obj(items: &str, attrs: &str, body: &str) -> String {
    format!(
        r#"object "V2.6 Probe" {{
    guid: 0x7F40_0600;
    state n: i16;
    state m: i16;
    {items}
    #[bhav(id = 4200{attrs})] fn t() {{ {body} }}
}}
"#
    )
}

mod local_writes {
    use super::*;

    #[test]
    fn write_local_words_in_place() {
        let body = "local[0] = self.n; local[1] += 2; local[2] -= local[0]; \
                    while dec_gt(&mut local[0], 0) { self.n += 1; } \
                    let x = self.m + 1; self.m = x; \
                    if local[2] == 0 { local[2] = 1; }";
        let lines = tree_lines(&air(&obj("", "", body)), "t");
        for want in [
            "local[0] := my.attr[0];",
            "local[1] += 2;",
            "local[2] -= local[0];",
            "if !local[0] dec_gt 0 {",
            "local[2] := 1;",
        ] {
            assert!(lines.iter().any(|l| l == want), "{want} in {lines:#?}");
        }
        // The named local goes after every numbered word.
        assert!(
            lines.iter().any(|l| l.starts_with("let ")),
            "a named local stays named for .air to place: {lines:#?}"
        );
        let c = compile(&obj("", "", body));
        assert!(
            c.behaviors[&4200].locals >= 4,
            "x takes the word after local[2]"
        );
    }

    /// An `#[slot(N)] let` local is frame word `N`: a `local[N]` write that reads it is not
    /// built in place (`local[0] := 1; local[0] += local[0]` would read the word it just
    /// overwrote).
    #[test]
    fn a_write_that_reads_its_slot_local_copies_it_first() {
        let lines = tree_lines(
            &air(&obj(
                "",
                "",
                "#[slot(0)] let x = 7; local[0] = 1 + x; self.n = local[0];",
            )),
            "t",
        );
        assert!(
            !lines.iter().any(|l| l == "local[0] := 1;"),
            "the write may not land before the read: {lines:#?}"
        );
        // A word nothing aliases is still built in place.
        let lines = tree_lines(
            &air(&obj(
                "",
                "",
                "#[slot(0)] let x = 7; local[1] = 1 + x; self.n = local[1];",
            )),
            "t",
        );
        assert!(lines.iter().any(|l| l == "local[1] := 1;"), "{lines:#?}");
    }

    /// A `#[slot(N)]` binding's initializer, or an `if` arm writing it, may read `local[N]`:
    /// the value is built apart and stored last.
    #[test]
    fn a_slot_binding_reads_its_word_before_writing_it() {
        let init = compile(&obj(
            "",
            "",
            "local[0] = 7; #[slot(0)] let x = 1 + local[0]; self.n = x;",
        ));
        let temp = compile(&obj(
            "",
            "",
            "local[0] = 7; let r = 1 + local[0]; #[slot(0)] let x = r; self.n = x;",
        ));
        assert_eq!(
            as_nodes(&instructions(&init, 4200)),
            as_nodes(&instructions(&temp, 4200))
        );
        let lines = tree_lines(
            &air(&obj(
                "",
                "",
                "local[0] = 7; #[slot(0)] let x = if self.m == 0 { 1 + local[0] } else { 0 }; \
                 self.n = x;",
            )),
            "t",
        );
        assert!(
            !lines.iter().any(|l| l == "local[0] += local[0];"),
            "{lines:#?}"
        );
        let lines = tree_lines(
            &air(&obj(
                "",
                "",
                "local[0] = 1; #[slot(0)] let b = if self.m == 0 { self.n == 1 && local[0] == 1 \
                 } else { false }; if b { self.n = 5; }",
            )),
            "t",
        );
        // `local[0] := 1`, the arm's result, the `else`: the arm's chain is built apart.
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.starts_with("local[0] :="))
                .count(),
            3,
            "{lines:#?}"
        );
    }

    #[test]
    fn set_to_next_writes_a_local_word() {
        let body = "local[1] = 0; if set_to_next(person, dest: local[1]) { self.n = 1; }";
        compile(&obj("", "", body));
    }

    #[test]
    fn a_write_keeps_the_layout_as_a_read_does() {
        // The backend pass forwards `a` into `b` when nothing keeps the layout; a `local[N]`
        // write keeps it, as a read did.
        const BODY: &str = "let a = self.n; let b = a + 1; self.m = b;";
        let plain = compile(&obj("", "", BODY));
        let written = compile(&obj("", "", &format!("{BODY} local[0] = 1;")));
        let pinned = compile(&obj("", ", keep_layout", &format!("{BODY} local[0] = 1;")));
        assert_eq!(
            as_nodes(&instructions(&written, 4200)),
            as_nodes(&instructions(&pinned, 4200)),
            "the write keeps the layout as `keep_layout` does"
        );
        assert!(
            instructions(&written, 4200).len() > instructions(&plain, 4200).len(),
            "the extra node is the write, and `a`'s copy stays"
        );
    }

    #[test]
    fn a_pinned_push_interaction_refuses_it() {
        let src = r#"object "V2.6 Probe" {
    guid: 0x7F40_0600;
    state n: i16;
    #[bhav(id = 4200)] fn go(sim: Sim) {
        local[0] = 1;
        let o = stack_obj;
        stack_obj = sim;
        push_interaction(object: o, menu: 0, priority: 2, icon: o);
    }
}
"#;
        expect_err(src, "cannot share a tree with 'raw { ... }' or 'local[N]'");
    }

    #[test]
    fn fmt_leaves_them_alone() {
        let src = obj("", "", "local[0] = self.n; local[1] += 2;");
        let formatted = format_v2_source(&src).expect("formats");
        assert!(formatted.contains("local[0] = self.n;"), "{formatted}");
        assert!(formatted.contains("local[1] += 2;"), "{formatted}");
        assert!(is_v2_formatted(&formatted).expect("checks"));
    }
}

mod identical_arms {
    use super::*;

    /// Both arms of an `if` lower to the same nodes: the backend pass merges them unless
    /// `keep_layout` asks it to leave the nodes as lowered.
    #[test]
    fn keep_layout_keeps_both_arms() {
        let body = "if self.n == 1 { self.m = 2; } else { self.m = 2; } self.n = 0;";
        let merged = compile(&obj("", "", body));
        let kept = compile(&obj("", ", keep_layout", body));
        assert!(
            instructions(&kept, 4200).len() > instructions(&merged, 4200).len(),
            "kept {:?} merged {:?}",
            as_nodes(&instructions(&kept, 4200)),
            as_nodes(&instructions(&merged, 4200))
        );
    }
}

mod dead_nodes {
    use super::*;

    const BODY: &str = "self.n = 1; if self.m == 2 { self.n = 3; } self.m = 0;";
    const ENTRIES: &str = "1: op 0x0002(0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04, 0x00) -> \
                           true: 2, false: error; 4: op 0x0002(0, 0, 0, 0, 0, 0, 0, 0) -> true: \
                           251, false: false;";
    /// One dead node, as a tree body's footer.
    const ONE: &str = "dead_nodes { 1: op 2(0, 0, 0, 0, 0, 0, 0, 0) -> true: true, false: true; }";

    fn with_dead(attrs: &str, entries: &str) -> String {
        obj("", attrs, &format!("{BODY} dead_nodes {{ {entries} }}"))
    }

    #[test]
    fn go_back_at_their_numbers() {
        let plain = instructions(&compile(&obj("", "", BODY)), 4200);
        let built = instructions(&compile(&with_dead("", ENTRIES)), 4200);
        assert_eq!(built.len(), plain.len() + 2);
        let (live, dead) = split_dead_nodes(&built).expect("splits");
        assert_eq!(
            as_nodes(&live),
            as_nodes(&plain),
            "the reached nodes as compiled"
        );
        assert_eq!(
            dead,
            vec![
                DeadNode {
                    at: 1,
                    opcode: 2,
                    operands: [1, 0, 2, 0, 3, 0, 4, 0],
                    on_true: 2,
                    on_false: 0xFD,
                },
                DeadNode {
                    at: 4,
                    opcode: 2,
                    operands: [0; 8],
                    on_true: 251,
                    on_false: 0xFF,
                },
            ]
        );
        // A live branch to its next node skips the dead node in between.
        assert_eq!(built[0].on_true, 2, "{:?}", as_nodes(&built));
        let header = tree_lines(&air(&with_dead("", ENTRIES)), "t").remove(0);
        assert!(header.contains("dead_nodes: [1: op 0x0002("), "{header}");
    }

    #[test]
    fn the_quarantined_pin_rebuilds_the_original() {
        let plain = instructions(&compile(&obj("", "", BODY)), 4200);
        let count = plain.len();
        // An original with its nodes renumbered and two dead nodes among them.
        let order: Vec<String> = std::iter::once(0)
            .chain((1..count).rev())
            .map(|n| n.to_string())
            .collect();
        let original = instructions(
            &compile(&with_dead(
                &format!(", node_order = [{}]", order.join(", ")),
                ENTRIES,
            )),
            4200,
        );
        let QuarantinedPin { pin, dead_nodes } =
            match_tree_pin_quarantined(&plain, &original).expect("pins");
        let pin = pin.expect("the order");
        assert_eq!(dead_nodes.len(), 2);
        // The pin and the block, printed as the lift prints them, build the original.
        let block = dead_nodes_block(&dead_nodes, "    ");
        let src = obj("", &pin.bhav_fields(), &format!("{BODY}\n{block}"));
        assert_eq!(
            as_nodes(&instructions(&compile(&src), 4200)),
            as_nodes(&original),
            "{src}"
        );
        // Without dead nodes it is `match_tree_pin`'s answer; with none to pin, `None`.
        assert_eq!(match_tree_pin_quarantined(&plain, &plain), None);
        let (live, _) = split_dead_nodes(&original).expect("splits");
        let alone = match_tree_pin_quarantined(&plain, &live).expect("an order alone");
        assert!(alone.dead_nodes.is_empty());
        assert_eq!(alone.pin, Some(pin));
        // The reached nodes as compiled: only the dead nodes to put back.
        let plain_dead = instructions(&compile(&with_dead("", ENTRIES)), 4200);
        let q = match_tree_pin_quarantined(&plain, &plain_dead).expect("dead nodes");
        assert_eq!(q.pin, None);
        assert_eq!(q.dead_nodes.len(), 2);
    }

    #[test]
    fn split_refuses_a_reached_branch_past_the_tree() {
        let ins = |t: u8, f: u8| Instruction {
            opcode: 2,
            on_true: t,
            on_false: f,
            operands: [0; 8],
        };
        assert!(split_dead_nodes(&[ins(1, 0xFE), ins(0xFE, 0xFF)]).is_some());
        assert!(split_dead_nodes(&[ins(5, 0xFE), ins(0xFE, 0xFF)]).is_none());
        assert!(split_dead_nodes(&[ins(0xFB, 0xFE)]).is_none());
        // An unreached node may branch anywhere.
        let (live, dead) =
            split_dead_nodes(&[ins(0xFE, 0xFF), ins(0xFB, 9)]).expect("dead bytes stay");
        assert_eq!(live.len(), 1);
        assert_eq!(
            (dead[0].at, dead[0].on_true, dead[0].on_false),
            (1, 0xFB, 9)
        );
    }

    #[test]
    fn bad_entries_are_refused() {
        for (entries, needle) in [
            (
                "0: op 0x0002(0, 0, 0, 0, 0, 0, 0, 0) -> true: 1, false: 1;",
                "its entry cannot be a dead node",
            ),
            (
                "1: op 2(0, 0, 0, 0, 0, 0, 0, 0) -> true: true, false: true; \
                 1: op 2(0, 0, 0, 0, 0, 0, 0, 0) -> true: true, false: true;",
                "dead node 1 is listed twice",
            ),
            (
                "40: op 2(0, 0, 0, 0, 0, 0, 0, 0) -> true: true, false: true;",
                "past the tree's last node",
            ),
            (
                "1: op 2(0, 0, 0, 0, 0, 0, 0, 0) -> true: 256, false: true;",
                "is not a byte",
            ),
            (
                "1: op 2(0, 0, 0, 0, 0, 0, 0) -> true: 1, false: true;",
                "operand byte",
            ),
            ("", "empty 'dead_nodes { }' block"),
        ] {
            expect_err(&with_dead("", entries), needle);
        }
    }

    #[test]
    fn only_at_the_end_of_a_tree_body() {
        expect_err(
            &obj("", "", &format!("self.n = 1; {ONE} self.m = 2;")),
            "must end its tree's body",
        );
        expect_err(
            &obj("", "", &format!("if self.n == 1 {{ {ONE} }}")),
            "goes only at the end of a tree's body",
        );
        expect_err(
            &obj(
                &format!("fn helper() {{ self.n = 1; {ONE} }}"),
                "",
                "helper();",
            ),
            "goes only at the end of a tree's body",
        );
        // A lifecycle body takes one.
        let main = format!(
            "object \"V2.6 Probe\" {{\n    guid: 0x7F40_0600;\n    state n: i16;\n    \
             #[id(4200)] main {{ self.n = 1; {ONE} }}\n}}\n"
        );
        let header = tree_lines(&air(&main), "Main").remove(0);
        assert!(header.contains("dead_nodes: [1: op 0x0002("), "{header}");
    }

    #[test]
    fn a_tail_value_may_stand_before_them() {
        let src = format!(
            "object \"V2.6 Probe\" {{\n    guid: 0x7F40_0600;\n    state n: i16;\n    \
             #[bhav(id = 4200)] fn t() -> bool {{ self.n == 1 {ONE} }}\n}}\n"
        );
        let tail = instructions(&compile(&src), 4200);
        let ret = src.replace("self.n == 1 dead_nodes", "return self.n == 1; dead_nodes");
        assert_ne!(ret, src);
        assert_eq!(
            as_nodes(&tail),
            as_nodes(&instructions(&compile(&ret), 4200))
        );
    }

    /// A composed lifecycle tree: the template's block carries `#[frame]`, the object's the
    /// footer, which is no second `#[frame]`.
    #[test]
    fn a_composed_lifecycle_tree_takes_one_footer() {
        let src = |object_init: &str, template_init: &str| {
            format!(
                "template Base {{\n    state n: i16;\n    {template_init}\n}}\nobject \"V2.6 \
                 Probe\" from Base {{\n    guid: 0x7F40_0600;\n    {object_init}\n}}\n"
            )
        };
        let header = tree_lines(
            &air(&src(
                &format!("init {{ self.n = 2; {ONE} }}"),
                "#[frame(locals = 3)] init { self.n = 1; }",
            )),
            "Init",
        )
        .remove(0);
        assert!(header.contains("locals: 3"), "{header}");
        assert!(header.contains("dead_nodes: [1: op 0x0002("), "{header}");
        expect_err(
            &src(
                &format!("init {{ self.n = 2; {ONE} }}"),
                &format!("init {{ self.n = 1; {ONE} }}"),
            ),
            "'dead_nodes' ends more than one block of the 'Init' tree",
        );
        expect_err(
            &src(
                "#[frame(locals = 3)] init { self.n = 2; }",
                "#[frame(locals = 3)] init { self.n = 1; }",
            ),
            "'#[frame]' on more than one block of the 'Init' tree",
        );
        // An empty `#[frame()]` is still a `#[frame]`.
        expect_err(
            &src(
                "#[frame(locals = 3)] init { self.n = 2; }",
                "#[frame()] init { self.n = 1; }",
            ),
            "'#[frame]' on more than one block of the 'Init' tree",
        );
    }

    #[test]
    fn a_tail_panic_may_stand_before_them() {
        let src = obj("", "", &format!("panic!() {ONE}"));
        let semi = obj("", "", &format!("panic!(); {ONE}"));
        assert_eq!(
            as_nodes(&instructions(&compile(&src), 4200)),
            as_nodes(&instructions(&compile(&semi), 4200))
        );
    }

    #[test]
    fn an_indexed_tree_refuses_them() {
        expect_err(&with_dead(", kind = 4", ENTRIES), "indexed tree (kind 4)");
    }

    #[test]
    fn a_wide_inline_call_refuses_them() {
        const WIDE: &str = "#[bhav(id = 4201, args = 6)] fn wide(a: i16) -> bool { true }";
        let src = obj(
            WIDE,
            "",
            "if wide(1) { self.n = 1; } dead_nodes { 1: op 2(0, 0, 0, 0, 0, 0, 0, 0) -> \
             true: true, false: true; }",
        );
        expect_err(&src, "fn 't' has 'dead_nodes', which moves nodes");
        // `.air` makes the same check.
        let err = compile_air(
            "tree 4096 \"T\" (dead_nodes: [1: op 0x0002(0 0 0 0 0 0 0 0) -> true: true, false: \
             true;]) {\n    call 300(1, 2, 3, 4);\n    return true;\n}\n",
        )
        .expect_err("a wide call");
        assert!(err.message.contains("'dead_nodes' move"), "{err}");
    }

    #[test]
    fn air_takes_them_in_the_tree_header() {
        let build = |header: &str| {
            compile_air(&format!(
                "tree 4096 \"T\" ({header}) {{\n    temp[0] := 1;\n    temp[1] := 2;\n    \
                 return true;\n}}\n"
            ))
            .map(|m| {
                m.behaviors
                    .into_iter()
                    .find(|(_, b)| b.id == 4096)
                    .expect("tree 4096")
                    .1
                    .instructions
            })
        };
        let plain = build("args: 0").expect("plain");
        let dead = build(
            "dead_nodes: [1: op 0x0002(1 2 3 4 5 6 7 8) -> true: 0, false: error; 3: op 9(0, 0, \
             0, 0, 0, 0, 0, 0) -> true: 252, false: 7;]",
        )
        .expect("dead");
        assert_eq!(dead.len(), plain.len() + 2);
        let (live, nodes) = split_dead_nodes(&dead).expect("splits");
        assert_eq!(as_nodes(&live), as_nodes(&plain));
        assert_eq!(
            nodes.iter().map(DeadNode::entry_text).collect::<Vec<_>>(),
            [
                "1: op 0x0002(0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08) -> true: 0, false: \
                 error;",
                "3: op 0x0009(0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00) -> true: 252, false: \
                 7;",
            ]
        );
        let err = build(
            "dead_nodes: [1: op 2(0 0 0 0 0 0 0 0) -> true: 0, false: 0;], dead_nodes: [2: op \
             2(0 0 0 0 0 0 0 0) -> true: 0, false: 0;]",
        )
        .expect_err("twice");
        assert!(
            err.message.contains("duplicate header field 'dead_nodes'"),
            "{err}"
        );
        let err =
            build("dead_nodes: [], dead_nodes: [1: op 2(0 0 0 0 0 0 0 0) -> true: 0, false: 0;]")
                .expect_err("an empty list counts");
        assert!(
            err.message.contains("duplicate header field 'dead_nodes'"),
            "{err}"
        );
    }

    #[test]
    fn fmt_keeps_one_entry_a_line() {
        let messy = obj("", "", &format!("{BODY} dead_nodes {{ {ENTRIES} }}"));
        let formatted = format_v2_source(&messy).expect("formats");
        assert!(
            formatted.contains(
                "\n        dead_nodes {\n            1: op 0x0002(0x01, 0x00, 0x02, 0x00, 0x03, \
                 0x00, 0x04, 0x00) -> true: 2, false: error;\n            4: op 0x0002(0, 0, 0, \
                 0, 0, 0, 0, 0) -> true: 251, false: false;\n        }\n"
            ),
            "{formatted}"
        );
        assert!(is_v2_formatted(&formatted).expect("checks"));
        assert_eq!(format_v2_source(&formatted).expect("formats"), formatted);
        // The printed block is formatted as it stands.
        let (_, dead) = split_dead_nodes(&instructions(&compile(&messy), 4200)).expect("splits");
        let printed = obj(
            "",
            "",
            &format!("{BODY}\n{}", dead_nodes_block(&dead, "        ")),
        );
        let printed = format_v2_source(&printed).expect("formats");
        assert!(
            printed.contains(&dead_nodes_block(&dead, "        ")),
            "{printed}"
        );
    }
}

mod loader {
    use super::*;

    #[test]
    fn check_v2_file_checks_without_reading() {
        let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("v26_loader");
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("probe.antics");
        std::fs::write(&file, obj("", "", "self.n = 1;")).expect("write");
        let checked = check_v2_file(&file, None).expect("checks");
        assert_eq!(
            checked.canonical_path(),
            std::fs::canonicalize(&file).expect("canonical")
        );
        let (source, config) =
            sims_antics::v2::loader::read_v2_checked_file(&file, None).expect("reads");
        assert_eq!(checked.read().expect("reads"), source);
        assert_eq!(checked.config.pack_root, config.pack_root);
        assert!(check_v2_file(&dir, None).is_err(), "a directory is refused");
        assert!(check_v2_file(&dir.join("missing.antics"), None).is_err());
    }
}

mod renamed_globals {
    use super::*;

    #[test]
    fn the_clean_room_names_resolve() {
        for call in [
            "free_to_join()",
            "face_opposite_way()",
            "on_downtown_lot()",
            "on_residential_lot()",
            "on_vacation_lot()",
            "claim_all_tiles()",
            "release_all_tiles(0)",
            "report_needs_repair()",
            "depart_lot()",
            "pick_own_image(0)",
        ] {
            let src = format!(
                "use std::globals::*;\n{}",
                obj("", "", &format!("let _ = globals::{call};"))
            );
            compile(&src);
        }
    }
}
