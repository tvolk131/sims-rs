//! `.antics` v2.4 Step 2: `#[bhav(.., dead_false = error)]` (a statement's dead false branch
//! written `false: error`, as the original compiler writes it), the `node_order` pin helpers
//! that find it, and in-place call staging kept off where a sibling tree's text may read the
//! frame words it changes. All sources are synthetic.

use sims_antics::node_order::{
    TreePin, match_tree_pin, match_tree_pin_compiled, wants_dead_false_error, with_dead_false_error,
};
use sims_antics::v2::{format_v2_source, is_v2_formatted};
use sims_antics::{CompiledObject, PackResolverConfig, compile_air, compile_v2_source};
use sims_formats::behavior::Instruction;

const ERROR: u8 = 0xFD;

fn compile(src: &str) -> CompiledObject {
    compile_v2_source("v24df.antics", src, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("must compile: {e}\n{src}"))
}

fn air(src: &str) -> String {
    sims_antics::v2::expand_v2_source_to_objects(
        "v24df.antics",
        src,
        &PackResolverConfig::default(),
    )
    .unwrap_or_else(|e| panic!("must expand: {e}\n{src}"))
    .remove(0)
    .air_source
}

fn expect_err(src: &str, needle: &str) {
    let err = compile_v2_source("v24df.antics", src, &PackResolverConfig::default())
        .expect_err(&format!("must fail:\n{src}"));
    assert!(
        err.message.contains(needle),
        "expected '{needle}' in: {err}"
    );
}

type Node = (u16, u8, u8, [u8; 8]);

fn nodes(instructions: &[Instruction]) -> Vec<Node> {
    instructions
        .iter()
        .map(|i| (i.opcode, i.on_true, i.on_false, i.operands))
        .collect()
}

/// An object with one Sim tree (id 4200) whose `#[bhav(..)]` adds `fields`.
fn object(fields: &str, body: &str) -> String {
    format!(
        "extern mod g {{ #[id(290)] fn two(a: i16, b: i16) -> bool; }}
object \"V2.4 Dead False Probe\" {{
    guid: 0x7F40_03E0;
    state n: i16;
    #[bhav(id = 4200{fields})] fn t(sim: Sim) {{
{body}
    }}
}}
"
    )
}

fn tree(fields: &str, body: &str) -> Vec<Instruction> {
    compile(&object(fields, body)).behaviors[&4200]
        .instructions
        .clone()
}

/// Statements of every kind the compiler gives a dead false branch, with a condition, a
/// fallible primitive statement and a call between them.
const BODY: &str = "        self.n = 3;
        temp[1] += 2;
        play_sound(1);
        idle(5);
        if self.n > 2 { self.n -= 1; }
        if g::two(temp[0], temp[1]) { temp[2] = 1; }
        let k = self.n;
        while temp[3] < k { temp[3] += 1; }
        raw { temp[4] := 9; }";

mod dead_false {
    use super::*;

    #[test]
    fn statements_that_cannot_fail_get_false_error() {
        let plain = tree("", BODY);
        let dead = tree(", dead_false = error", BODY);
        assert_eq!(plain.len(), dead.len());
        let mut flipped = 0;
        for (p, d) in plain.iter().zip(&dead) {
            assert_eq!(
                (p.opcode, p.operands, p.on_true),
                (d.opcode, d.operands, d.on_true)
            );
            if p.on_false == p.on_true && d.on_false == ERROR {
                flipped += 1;
            } else {
                assert_eq!(p.on_false, d.on_false, "{p:?} / {d:?}");
            }
        }
        // Every statement flips: the assignments (`self.n = 3`, `temp[1] += 2`, `self.n -= 1`,
        // `temp[2] = 1`, the loop's increment, the raw store), the local copy, the call's
        // staging store, the sound, and Idle's parameter store and Sleep. The conditions and the
        // call keep their branches.
        assert_eq!(flipped, 11, "{:#?}", nodes(&dead));
        assert_eq!(nodes(&dead), nodes(&with_dead_false_error(&plain)));
        assert!(wants_dead_false_error(&dead));
        assert!(!wants_dead_false_error(&plain));
    }

    #[test]
    fn written_branches_stay() {
        // A raw node written with both branches to one place keeps them.
        let body = "        raw {
            @n0:
            expr(temp[0] := 1) -> true: @n1, false: @n1;
            @n1:
            expr(temp[1] := 2) -> true: true, false: error;
        }";
        let dead = tree(", dead_false = error", body);
        assert_eq!(nodes(&dead), nodes(&tree("", body)));
        assert_eq!((dead[0].on_true, dead[0].on_false), (1, 1));
    }

    #[test]
    fn it_formats_and_composes_with_the_other_pins() {
        let src = object(
            ", dead_false = error, share_tails, node_order = [0, 1]",
            "        self.n = 3; temp[0] = 1;",
        );
        let formatted = format_v2_source(&src).expect("format");
        assert!(formatted.contains("dead_false = error"), "{formatted}");
        assert!(is_v2_formatted(&formatted).expect("check"));
        assert_eq!(format_v2_source(&formatted).expect("format"), formatted);
        let t = tree(
            ", dead_false = error, share_tails, node_order = [0, 1]",
            "        self.n = 3; temp[0] = 1;",
        );
        assert_eq!(t[0].on_false, ERROR);
    }

    #[test]
    fn only_error_is_a_value() {
        for (fields, needle) in [
            (", dead_false = next", "'dead_false' takes 'error'"),
            (", dead_false = 1", "'dead_false' takes 'error'"),
            (", dead_false", "expected 'key = value'"),
            (
                ", dead_false = error, dead_false = error",
                "duplicate header field 'dead_false'",
            ),
        ] {
            expect_err(&object(fields, "        self.n = 3;"), needle);
        }
    }

    #[test]
    fn air_takes_it_in_the_tree_header() {
        let air = |header: &str| {
            let src = format!(
                "tree 4096 \"T\" ({header}) {{\n    temp[0] := 1;\n    if temp[0] > 0 {{\n        temp[1] := 2;\n    }}\n    return true;\n}}\n"
            );
            compile_air(&src).map(|m| {
                m.behaviors
                    .into_iter()
                    .find(|(_, b)| b.id == 4096)
                    .expect("tree 4096")
                    .1
                    .instructions
            })
        };
        let plain = air("args: 0").expect("plain");
        let dead = air("dead_false: error").expect("dead");
        assert_eq!(nodes(&dead), nodes(&with_dead_false_error(&plain)));
        assert_eq!(dead[0].on_false, ERROR);
        let err = air("dead_false: next").expect_err("next");
        assert!(
            err.message.contains("expected 'error' after 'dead_false:'"),
            "{err}"
        );
    }

    #[test]
    fn the_pin_helper_finds_dead_false_and_order() {
        let plain = tree("", BODY);
        let dead = tree(", dead_false = error", BODY);
        // The original: the dead-false tree with nodes 1 and 2 swapped.
        let mut order: Vec<u16> = (0..dead.len() as u16).collect();
        order.swap(1, 2);
        let fields = format!(
            ", dead_false = error, node_order = [{}]",
            order
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
        let original = tree(&fields, BODY);
        assert_ne!(nodes(&original), nodes(&dead));
        let pin = match_tree_pin(&plain, &original).expect("pin");
        assert_eq!(
            pin,
            TreePin {
                dead_false_error: true,
                share_tails: false,
                order: Some(order),
            }
        );
        assert_eq!(pin.bhav_fields(), fields);
        assert_eq!(nodes(&tree(&pin.bhav_fields(), BODY)), nodes(&original));
        // The flag alone, and nothing when the nodes already match.
        let pin = match_tree_pin(&plain, &dead).expect("pin");
        assert_eq!(pin.bhav_fields(), ", dead_false = error");
        assert_eq!(match_tree_pin(&plain, &plain), None);
        // An original with one dead false branch to the next node does not want the flag
        // everywhere, and neither does a tree with no statement that cannot fail.
        let mut mixed = dead.clone();
        mixed[0].on_false = mixed[0].on_true;
        assert!(!wants_dead_false_error(&mixed));
        assert!(!wants_dead_false_error(&[]));
    }

    /// A one-tree `.air` module's tree 4096.
    fn air_tree(header: &str, body: &str) -> Vec<Instruction> {
        compile_air(&format!("tree 4096 \"T\" ({header}) {{ {body} }}"))
            .unwrap_or_else(|e| panic!("must compile: {e}\n{body}"))
            .behaviors
            .into_values()
            .next()
            .expect("tree")
            .instructions
    }

    #[test]
    fn the_pin_helper_keeps_raw_nodes_and_matches_unreached_nodes() {
        // A raw always-true node written with both branches to one place stays under the flag;
        // the helper still finds the flag by keeping the nodes the original leaves live.
        let mixed = "temp[0] := 1; op 2 (1,2,0,0,0,5,8,7) -> true: true, false: true;";
        let plain = air_tree("args: 0", mixed);
        let flagged = air_tree("dead_false: error", mixed);
        assert_eq!(
            (flagged[0].on_false, flagged[1].on_false),
            (ERROR, plain[1].on_false)
        );
        assert!(!wants_dead_false_error(&flagged));
        let pin = match_tree_pin(&plain, &flagged).expect("pin");
        assert_eq!(pin.bhav_fields(), ", dead_false = error");
        // Every node flipped is a different tree, which the compiler cannot build from this
        // source: the pin is a prediction, which a caller confirms by compiling with it.
        let both = with_dead_false_error(&plain);
        assert_ne!(nodes(&both), nodes(&flagged));
        assert_eq!(
            match_tree_pin(&plain, &both).map(|p| p.bhav_fields()),
            Some(", dead_false = error".to_owned())
        );
        // A node no branch reaches matches by position.
        let unreached = "temp[0] := 1; return true; @unused: op 2 (1,2,0,0,0,5,8,7) -> true: true, false: error;";
        let plain = air_tree("args: 0", unreached);
        let flagged = air_tree("dead_false: error", unreached);
        let pin = match_tree_pin(&plain, &flagged).expect("pin");
        assert_eq!(pin.bhav_fields(), ", dead_false = error");
        assert_eq!(match_tree_pin(&plain, &plain), None);
        // `.air` refuses the field twice.
        let err =
            compile_air("tree 4096 \"T\" (dead_false: error, dead_false: error) { temp[0] := 1; }")
                .expect_err("duplicate");
        assert!(
            err.message.contains("duplicate header field 'dead_false'"),
            "{err}"
        );
    }

    #[test]
    fn the_compiled_pin_helper_is_exact() {
        // A statement and a raw node of the same instruction: the flag changes the statement
        // alone, which no rewrite of the plain bytes can tell, so only the flagged build finds it.
        let same = "temp[0] := 1; op 2 (0,0,1,0,0,5,8,7) -> true: true, false: true;";
        let plain = air_tree("args: 0", same);
        let flagged = air_tree("dead_false: error", same);
        assert_eq!(plain[0].operands, plain[1].operands);
        assert_eq!(
            (flagged[0].on_false, flagged[1].on_false),
            (ERROR, plain[1].on_false)
        );
        assert_eq!(match_tree_pin(&plain, &flagged), None);
        let pin = match_tree_pin_compiled(&plain, &flagged, &flagged).expect("pin");
        assert_eq!(pin.bhav_fields(), ", dead_false = error");
        assert_eq!(match_tree_pin_compiled(&plain, &flagged, &plain), None);
        // Shared tails that match as they stand: two identical nodes no branch reaches merge.
        let detached = "temp[0] := 1; return true; \
            @a: op 2 (1,2,0,0,0,5,8,7) -> true: true, false: error; \
            @b: op 2 (1,2,0,0,0,5,8,7) -> true: true, false: error;";
        let plain = air_tree("args: 0", detached);
        let original = air_tree("dead_false: error, share_tails: true", detached);
        assert!(original.len() < plain.len());
        let pin = match_tree_pin(&plain, &original).expect("pin");
        assert_eq!(pin.bhav_fields(), ", dead_false = error, share_tails");
    }

    #[test]
    fn calls_that_read_the_nodes_after_them_are_refused() {
        // A callee whose frame holds more than four arguments reads the rest from the nodes
        // after an inline call, whose false branches the flag rewrites.
        let src = |fields: &str, body: &str| {
            object(fields, body).replace(
                "    #[bhav(id = 4200",
                "    #[bhav(id = 4201, args = 6)] fn wide(sim: Sim, a: i16) -> bool { true }\n    #[bhav(id = 4200",
            )
        };
        let wide = "        wide(sim, 1); temp[1] = 1;";
        compile(&src("", wide));
        expect_err(
            &src(", dead_false = error", wide),
            "fn 't' has 'dead_false', which rewrites false branches, but this call passes its \
             arguments inline to tree 'wide', whose frame holds 6 arguments",
        );
        // Through temps the callee reads no node.
        compile(&src(
            ", dead_false = error",
            "        wide(sim, temp[0]); temp[1] = 1;",
        ));
        // `.air` refuses a callee it cannot see unless it is listed narrow.
        let call = |header: &str| {
            compile_air(&format!(
                "tree 4096 \"T\" ({header}) {{ call 290(1, 2); temp[1] := 1; }}"
            ))
        };
        let err = call("dead_false: error").expect_err("unlisted callee");
        assert!(err.message.contains("'dead_false' rewrites"), "{err}");
        call("dead_false: error, narrow_callees: [290]").expect("listed narrow");
        call("args: 0").expect("no flag");
        // A tree of the module is checked by the frame it compiles to, listed or not.
        let sibling = |header: &str, args: &str| {
            compile_air(&format!(
                "tree 4096 \"T\" ({header}) {{ call 4097({args}); temp[1] := 1; }}\n\
                 tree 4097 \"W\" {{ temp[0] := param[5]; }}\n"
            ))
        };
        let err = sibling("dead_false: error, narrow_callees: [4097]", "1, 2").expect_err("wide");
        assert!(err.message.contains("its frame holds 6"), "{err}");
        sibling("dead_false: error", "temps").expect("temps");
        // v2 lists a narrow callee a raw block calls in either `.air` spelling.
        for call in [
            "call 290(1, 2);",
            "subroutine(290, 1, 2);",
            "subroutine 290(1, 2);",
        ] {
            compile(&object(
                ", dead_false = error",
                &format!("        raw {{ {call} }} temp[1] = 1;"),
            ));
        }
    }
}

mod staging_text_guard {
    use super::*;

    const EXTERNS: &str = "extern mod g { #[id(290)] fn two(a: i16, b: i16) -> bool; }";

    fn staging_object(members: &str) -> String {
        format!(
            "{EXTERNS}\nobject \"V2.4 Staging Probe\" {{\n    guid: 0x7F40_03E1;\n    state n: i16;\n{members}\n}}\n"
        )
    }

    const PRODUCE: &str =
        "    #[bhav(id = 4200)] fn produce(x: i16) { if g::two(temp[1], temp[2]) { self.n = 1; } }";

    #[test]
    fn a_sibling_tree_that_shows_frame_words_keeps_the_copying_form() {
        // Alone, the call stages in place: no frame word.
        let alone = compile(&staging_object(PRODUCE));
        assert_eq!(alone.behaviors[&4200].locals, 0);
        // A sibling tree's dialog shows `$Local:0`, a word of its own frame that holds what the
        // last frame left there: `produce`'s staging must leave what the copying form does.
        let shown = staging_object(&format!(
            "{PRODUCE}\n    #[bhav(id = 4201)] fn show(x: i16) {{ dialog(kind: ok, title: \"T\", message: \"Value: $Local:0\"); }}"
        ));
        let text = air(&shown);
        let c = compile(&shown);
        assert!(
            c.behaviors[&4200].locals > 0,
            "staged in place beside text that names frame words:\n{text}"
        );
        // So a frame pinned too small for the copies is refused, as in any copy-staging object.
        expect_err(
            &shown.replace("(id = 4200)", "(id = 4200, locals = 0)"),
            "declares 0 locals but requires at least 2",
        );
        compile(&staging_object(
            &PRODUCE.replace("(id = 4200)", "(id = 4200, locals = 0)"),
        ));
    }

    #[test]
    fn a_string_no_tree_formats_reads_no_frame_word() {
        // The object's strings name `$Local:0`, but no tree formats text: nothing reads it, so
        // the call still stages in place, as on main.
        let unused = staging_object(&format!(
            "    strings 301 \"Unused\" {{ text = \"$Local:0\" }}\n{PRODUCE}"
        ));
        let c = compile(&unused);
        assert_eq!(
            nodes(&c.behaviors[&4200].instructions),
            nodes(&compile(&staging_object(PRODUCE)).behaviors[&4200].instructions)
        );
        assert_eq!(c.behaviors[&4200].locals, 0);
        compile(&unused.replace("(id = 4200)", "(id = 4200, locals = 0)"));
    }
}
