//! `lift --v2` (v2.3 step 3) pins a tree's node order and shared tails when they make it
//! compile to the original's nodes, shares the tails of a tree that would pass lint
//! `frame_headroom`, names `data[51]` as `light`, writes `if let Some(v) = f()` and
//! `let Some(v) = f() else { .. };` for `std::globals` `Option<T>` calls, and names the
//! locals of lifecycle blocks and trees written without the extra idioms. Since v2.4 the
//! structurer joins a tail the arms of an `if` share (`loop { .. break; }`) when that
//! builds fewer nodes. Every object is synthetic (GUIDs in the clean-room 0x7E00_xxxx
//! range).

use sims_antics::compile_v2_source;
use sims_antics::v2::{
    DecompileV2Options, DecompiledTree, PackResolverConfig, TreeCheck, TreeStyle,
    decompile_iff_to_v2_report, lint_v2_source,
};
use sims_formats::iff::Iff;

fn build(source: &str) -> Vec<u8> {
    compile_v2_source("leap.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{e}\n{source}"))
        .iff_bytes
}

fn nodes_of(source: &str, id: u16) -> usize {
    compile_v2_source("leap.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{e}\n{source}"))
        .behaviors[&id]
        .instructions
        .len()
}

/// Lifts `source`'s object: every tree proven, the text lints clean. Returns the text and
/// the trees.
fn lift(source: &str, behavioral: bool) -> (String, Vec<DecompiledTree>) {
    let options = DecompileV2Options {
        behavioral,
        ..DecompileV2Options::default()
    };
    let iff = Iff::parse(build(source)).expect("parse IFF");
    let report = decompile_iff_to_v2_report(&iff, &options).expect("lift");
    assert!(report.compiled);
    assert!(
        report
            .trees
            .iter()
            .all(|t| matches!(t.check, TreeCheck::Identical | TreeCheck::Equivalent)),
        "{:?}\n{}",
        report.trees,
        report.source
    );
    let lints = lint_v2_source(&report.source, "lifted.antics").expect("lint");
    assert!(lints.is_empty(), "{lints:?}\n{}", report.source);
    (report.source, report.trees)
}

fn tree(trees: &[DecompiledTree], id: u16) -> &DecompiledTree {
    trees
        .iter()
        .find(|t| t.id == id)
        .unwrap_or_else(|| panic!("tree {id} in {trees:?}"))
}

/// Tree `id` is written in v2 code, with no `raw { }`, and compiles to the original's
/// bytes.
fn idiomatic_and_identical(trees: &[DecompiledTree], id: u16, lifted: &str) {
    let t = tree(trees, id);
    assert_eq!(t.style, TreeStyle::Idiomatic, "{lifted}");
    assert_eq!(t.raw_statements, 0, "{lifted}");
    assert!(matches!(t.check, TreeCheck::Identical), "{t:?}\n{lifted}");
}

fn has(lifted: &str, spellings: &[&str]) {
    for s in spellings {
        assert!(lifted.contains(s), "missing {s:?} in:\n{lifted}");
    }
}

fn lacks(lifted: &str, spellings: &[&str]) {
    for s in spellings {
        assert!(!lifted.contains(s), "unexpected {s:?} in:\n{lifted}");
    }
}

/// An object holding one tree `t` (4300) with `attrs` in its header, and argument `x` when
/// `body` reads it.
fn object(guid: u32, attrs: &str, body: &str) -> String {
    let params = if body.contains("x ==") { "x: i16" } else { "" };
    format!(
        r#"object "Fable Leap" {{
    guid: {guid:#010X};
    state n: i16;
    state m: i16;
    #[bhav(id = 4300, label = "Leap"{attrs})]
    fn t({params}) -> bool {{ {body} }}
}}"#
    )
    .replace("0X", "0x")
}

/// Straight-line code with a yield: strict mode compares the node a yielding tree waits
/// at, so the nodes must be the original's.
const STRAIGHT: &str = "
        self.n = 1;
        sleep(1);
        self.m = 2;
        self.n = 3;
        return true;";

#[test]
fn strict_lift_pins_the_original_node_order() {
    let count = nodes_of(&object(0x7E00_01A0, "", STRAIGHT), 4300);
    assert!(count >= 4, "{count} nodes");
    // Node 0 first, then the others backwards: what a structured lift never writes.
    let order: Vec<String> = std::iter::once(0)
        .chain((1..count).rev())
        .map(|n| n.to_string())
        .collect();
    let attrs = format!(", node_order = [{}]", order.join(", "));
    let source = object(0x7E00_01A0, &attrs, STRAIGHT);
    let (lifted, trees) = lift(&source, false);
    has(&lifted, &[&attrs, "sleep(1);"]);
    idiomatic_and_identical(&trees, 4300, &lifted);
    // Behavioral mode proves the tree without the order, so it keeps the default one.
    let (lifted, _) = lift(&source, true);
    lacks(&lifted, &["node_order"]);
}

/// Arms that end in the same tail, which `share_tails` builds once and a structured lift
/// joins (v2.4) rather than writing it in each arm. The tail yields, so strict mode
/// compares its nodes.
const SHARED_ARMS: &str = "
        if self.m == 1 {
            self.m = 3;
            self.n += 1;
            sleep(1);
            self.n += 2;
            return true;
        }
        if self.m == 2 {
            self.m = 2;
            self.n += 1;
            sleep(1);
            self.n += 2;
            return true;
        }
        self.m = 1;
        return false;";

#[test]
fn strict_lift_joins_the_tails_the_original_shares() {
    let plain = nodes_of(&object(0x7E00_01A1, "", SHARED_ARMS), 4300);
    let source = object(0x7E00_01A1, ", share_tails", SHARED_ARMS);
    assert!(nodes_of(&source, 4300) < plain);
    // The arms `break` to the tail they share, written once: no `share_tails` pin.
    let (lifted, trees) = lift(&source, false);
    has(&lifted, &["loop {", "break;"]);
    lacks(&lifted, &["share_tails"]);
    idiomatic_and_identical(&trees, 4300, &lifted);
}

#[test]
fn behavioral_lift_joins_tails_past_the_node_warning() {
    // Five arms, each ending in the same 40-store tail: over 200 nodes unless they merge.
    let tail: String = (0..40).map(|_| "self.n += 1; ").collect();
    let arms: String = (1..=5)
        .map(|k| format!("if x == {k} {{ self.m = {k}; {tail} return true; }} "))
        .collect();
    let body = format!("{arms} return false;");
    assert!(nodes_of(&object(0x7E00_01A2, "", &body), 4300) > 200);
    let source = object(0x7E00_01A2, ", share_tails", &body);
    assert!(nodes_of(&source, 4300) < 80);
    // `lift` checks that the text lints clean: no `frame_headroom`. The arms join their
    // shared tail (`loop { .. break; }`), which builds it once with no pin.
    let (lifted, trees) = lift(&source, true);
    has(&lifted, &["loop {"]);
    lacks(&lifted, &["share_tails", "node_order"]);
    assert_eq!(lifted.matches("self.attr_0 += 1;").count(), 40);
    assert_eq!(tree(&trees, 4300).style, TreeStyle::Idiomatic);
}

#[test]
fn data_word_51_is_written_light() {
    let source = r#"object "Fable Leap Light" {
    guid: 0x7E00_01A3;
    #[bhav(id = 4300, label = "Glow", locals = 0)]
    fn glow() {
        raw {
            my.data[51] := 25;
            temp[0] := stack.data[51];
        }
    }
}"#;
    for behavioral in [false, true] {
        let (lifted, _) = lift(source, behavioral);
        has(&lifted, &["self.light = 25;", "temp[0] = stack_obj.light;"]);
        lacks(&lifted, &["data[51]"]);
    }
}

#[test]
fn option_calls_bind_their_value() {
    let source = r#"use std::globals::*;

object "Fable Leap Option" {
    guid: 0x7E00_01A5;
    #[bhav(id = 4300, label = "Follow", locals = 0)]
    fn follow() {
        if globals::find_companion() {
            stack_obj = temp[0];
        } else {
            stack_obj = self.id;
        }
    }
    #[bhav(id = 4301, label = "Leave", locals = 0)]
    fn leave() -> bool {
        if !globals::find_companion() {
            return false;
        }
        stack_obj = temp[0];
        return true;
    }
}"#;
    let (lifted, _) = lift(source, true);
    has(
        &lifted,
        &[
            "if let Some(companion) = globals::find_companion() {",
            "let Some(companion) = globals::find_companion() else {",
            "stack_obj = companion;",
        ],
    );
    lacks(&lifted, &["temp[0]"]);
    // The text with the `if`s written back builds the same bytes.
    let older = lifted
        .replace(
            "if let Some(companion) = globals::find_companion() {",
            "if globals::find_companion() {",
        )
        .replace(
            "let Some(companion) = globals::find_companion() else {",
            "if !globals::find_companion() {",
        )
        .replace("};", "}")
        .replace("companion;", "temp[0];");
    assert_eq!(build(&older), build(&lifted), "{older}");
}

#[test]
fn option_reads_stop_at_a_call() {
    // The call may change `temp[0]`: the read after it keeps its number.
    let source = r#"use std::globals::*;

object "Fable Leap Option Call" {
    guid: 0x7E00_01A6;
    #[bhav(id = 4300, label = "Follow", locals = 0)]
    fn follow() {
        if globals::find_companion() {
            stack_obj = temp[0];
            stop_all_sounds();
            self.attr[0] = temp[0];
        }
    }
}"#;
    let (lifted, _) = lift(source, true);
    has(
        &lifted,
        &[
            "Some(companion) = globals::find_companion()",
            "stack_obj = companion;",
            "stop_all_sounds();",
            "self.attr_0 = temp[0];",
        ],
    );
}

#[test]
fn lifecycle_blocks_name_their_locals() {
    let source = r#"object "Fable Leap Main" {
    guid: 0x7E00_01A7;
    #[bhav(id = 4096, locals = 1)]
    main {
        raw {
            local[0] := 5;
            my.attr[0] := local[0];
        }
    }
}"#;
    for behavioral in [false, true] {
        let (lifted, trees) = lift(source, behavioral);
        has(&lifted, &["slot_0 = 5;"]);
        lacks(&lifted, &["local[0]", "raw {"]);
        assert_eq!(tree(&trees, 4096).raw_statements, 0, "{lifted}");
    }
}

#[test]
fn a_body_past_the_node_limit_joins_its_tails() {
    // Six arms, each ending in the same 45-store tail: a structured body that copies the
    // tail into each arm goes past what a tree may hold. The structurer writes the arms in a
    // loop they `break` out of, and the tail once after it, so the tree lifts in both modes
    // with no pin.
    let tail: String = (0..45).map(|_| "self.n += 1; ").collect();
    let arms: String = (1..=6)
        .map(|k| format!("if x == {k} {{ self.m = {k}; {tail} return true; }} "))
        .collect();
    let body = format!("{arms} return false;");
    let unshared = object(0x7E00_01A8, "", &body);
    let refused = compile_v2_source("leap.antics", &unshared, &PackResolverConfig::default())
        .expect_err("too many nodes")
        .to_string();
    assert!(refused.contains("exceeds maximum"), "{refused}");
    let source = object(0x7E00_01A8, ", share_tails", &body);
    for behavioral in [false, true] {
        let (lifted, trees) = lift(&source, behavioral);
        assert_eq!(tree(&trees, 4300).style, TreeStyle::Idiomatic, "{lifted}");
        assert_eq!(tree(&trees, 4300).raw_statements, 0, "{lifted}");
        assert_eq!(lifted.matches("self.attr_0 += 1;").count(), 45, "{lifted}");
        assert_eq!(lifted.matches("break;").count(), 6, "{lifted}");
        has(&lifted, &["loop {"]);
        lacks(&lifted, &["share_tails", "node_order"]);
    }
}

#[test]
fn option_reads_stop_at_a_flag_write() {
    // `temp[0] flag_set 1` writes the temp: the binding covers the read before it only.
    let source = r#"use std::globals::*;

object "Fable Leap Option Flag" {
    guid: 0x7E00_01A9;
    #[bhav(id = 4300, label = "Follow", locals = 0)]
    fn follow() {
        if globals::find_companion() {
            stack_obj = temp[0];
            temp[0] flag_set 1;
            self.attr[0] = temp[0];
        }
    }
}"#;
    let (lifted, _) = lift(source, true);
    has(
        &lifted,
        &[
            "Some(companion) = globals::find_companion()",
            "stack_obj = companion;",
            "temp[0] flag_set 1;",
            "self.attr_0 = temp[0];",
        ],
    );
}

#[test]
fn wide_inline_calls_get_no_pin() {
    // The original runs its nodes in an order no structured lift writes, around a call that
    // yields. The call before the last store passes its argument inline to a tree whose
    // frame holds six, which reads its other arguments from the nodes after it: no pin may
    // move them, and the structurer does not reorder around it. The tree is proven as its
    // labeled nodes, with no pin.
    let source = r#"object "Fable Leap Wide" {
    guid: 0x7E00_01AA;
    state n: i16;
    #[bhav(id = 4301, args = 6)]
    fn wide(a: i16) -> bool { true }
    #[bhav(id = 4302)]
    fn nap() -> bool { sleep(1); true }
    #[bhav(id = 4300, label = "Leap", locals = 0)]
    fn t() {
        raw {
            @n0:
            expr(my.attr[0] := 1) -> true: @n3, false: error;
            @n1:
            expr(my.attr[0] := 3) -> true: true, false: error;
            @n2:
            call 4301(1) -> true: @n1, false: @n1;
            @n3:
            call 4302(temps) -> true: @n2, false: @n2;
        }
    }
}"#;
    let (lifted, _) = lift(source, false);
    lacks(&lifted, &["node_order", "share_tails"]);
}
