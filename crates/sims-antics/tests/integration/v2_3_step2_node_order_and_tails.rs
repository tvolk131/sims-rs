//! `.antics` v2.3 Step 2: `#[bhav(.., share_tails)]` merges the compiled nodes that run alike,
//! `#[bhav(.., node_order = [..])]` renumbers them, [`match_node_order`] finds the order that
//! turns one tree into another, and `light` names object data word 51. All sources are
//! synthetic.

use sims_antics::ir::BhavChunk;
use sims_antics::node_order::match_node_order;
use sims_antics::v2::lint::lint_v2_source;
use sims_antics::v2::{format_v2_source, is_v2_formatted};
use sims_antics::{
    CompiledObject, EquivOptions, EquivResult, LiveContext, LiveOptions, PackResolverConfig,
    check_bhav_equivalent_combined, compile_air, compile_v2_source,
};
use sims_formats::behavior::Instruction;

fn compile(src: &str) -> CompiledObject {
    compile_v2_source("step2.antics", src, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("must compile: {e}\n{src}"))
}

fn expect_err(src: &str, needle: &str) {
    let err = compile_v2_source("step2.antics", src, &PackResolverConfig::default())
        .expect_err(&format!("must fail:\n{src}"));
    assert!(
        err.message.contains(needle),
        "expected '{needle}' in: {err}"
    );
}

fn tree(c: &CompiledObject, id: u16) -> &BhavChunk {
    c.behaviors
        .get(&id)
        .unwrap_or_else(|| panic!("tree {id} in {:?}", c.behaviors.keys()))
}

type Node = (u16, u8, u8, [u8; 8]);

fn nodes(ins: &[Instruction]) -> Vec<Node> {
    ins.iter()
        .map(|i| (i.opcode, i.on_true, i.on_false, i.operands))
        .collect()
}

/// Node `i` of the result is `ins[order[i]]`, branches renumbered (written independently of
/// the crate's own renumbering).
fn permuted(ins: &[Instruction], order: &[u16]) -> Vec<Instruction> {
    let mut place = vec![0u8; ins.len()];
    for (i, &n) in order.iter().enumerate() {
        place[usize::from(n)] = i as u8;
    }
    let remap = |t: u8| {
        if usize::from(t) < ins.len() {
            place[usize::from(t)]
        } else {
            t
        }
    };
    order
        .iter()
        .map(|&n| {
            let i = &ins[usize::from(n)];
            Instruction {
                opcode: i.opcode,
                on_true: remap(i.on_true),
                on_false: remap(i.on_false),
                operands: i.operands,
            }
        })
        .collect()
}

/// Node 0 first, then the others backwards.
fn reversed_order(len: usize) -> Vec<u16> {
    std::iter::once(0).chain((1..len as u16).rev()).collect()
}

fn list(order: &[u16]) -> String {
    order
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// A synthetic object holding `items`.
fn obj(items: &str) -> String {
    format!(
        r#"object "V2.3 Step Two" {{
    guid: 0x7F40_02D0;
    state n: i16;
    state m: i16;
    {items}
}}
"#
    )
}

/// One tree `t` (id 4200) with `attrs` added to its `#[bhav(..)]`, and the `items` it uses.
fn obj_with(attrs: &str, items: &str, body: &str) -> String {
    obj(&format!(
        "{items}\n    #[bhav(id = 4200{attrs})] fn t(x: i16) -> bool {{ {body} }}"
    ))
}

/// Match arms that each end in the same loop and tail, as a structured lift writes a shared
/// tail of the original.
const DUPLICATED_ARMS: &str = "
        match x {
            1 => {
                self.m = 3;
                while self.n < 4 { self.n += 1; if self.n == 9 { return false; } }
                self.m = 0;
                return true;
            }
            2 => {
                self.m = 2;
                while self.n < 4 { self.n += 1; if self.n == 9 { return false; } }
                self.m = 0;
                return true;
            }
            _ => {
                self.m = 1;
                while self.n < 4 { self.n += 1; if self.n == 9 { return false; } }
                self.m = 0;
                return true;
            }
        }";

fn behavioral() -> LiveOptions {
    LiveOptions {
        behavioral: true,
        ..LiveOptions::default()
    }
}

#[track_caller]
fn assert_equivalent(a: &BhavChunk, b: &BhavChunk, opts: &LiveOptions) {
    let ctx = LiveContext::default();
    let (_, result) =
        check_bhav_equivalent_combined(a, &ctx, b, &ctx, &EquivOptions::default(), opts);
    assert!(
        matches!(result, EquivResult::Equivalent { .. }),
        "expected Equivalent, got {result:?}"
    );
}

#[test]
fn share_tails_merges_duplicated_arms_and_loops() {
    let plain = compile(&obj_with("", "", DUPLICATED_ARMS));
    let shared = compile(&obj_with(", share_tails", "", DUPLICATED_ARMS));
    let (p, s) = (tree(&plain, 4200), tree(&shared, 4200));
    // Each arm keeps its own first store; the three loops and tails become one.
    assert!(
        s.instructions.len() + 8 <= p.instructions.len(),
        "{} nodes shared, {} plain",
        s.instructions.len(),
        p.instructions.len()
    );
    assert_eq!(nodes(&s.instructions)[0], nodes(&p.instructions)[0]);
    // The header is the same; only the node table changes.
    assert_eq!((s.args, s.locals, s.format), (p.args, p.locals, p.format));
    // The merged tree runs the same instructions on every run.
    assert_equivalent(p, s, &behavioral());
}

#[test]
fn share_tails_is_opt_in_and_keeps_distinct_nodes() {
    // The other trees of the object build the same bytes.
    let other = "#[bhav(id = 4201)] fn u() { self.n = 1; self.n = 1; }";
    let plain = compile(&obj_with("", other, DUPLICATED_ARMS));
    let shared = compile(&obj_with(", share_tails", other, DUPLICATED_ARMS));
    assert_eq!(
        nodes(&tree(&plain, 4201).instructions),
        nodes(&tree(&shared, 4201).instructions)
    );
    // A chain of equal stores is not merged: each store runs once more.
    let body = "self.n += 1; self.n += 1; self.n += 1; self.n > 2";
    let plain = compile(&obj_with("", "", body));
    let shared = compile(&obj_with(", share_tails", "", body));
    assert_eq!(
        nodes(&tree(&plain, 4200).instructions),
        nodes(&tree(&shared, 4200).instructions)
    );
    // Arms that return differently stay apart.
    let body = "if x == 1 { self.n = 2; return true; } self.n = 2; false";
    let plain = compile(&obj_with("", "", body));
    let shared = compile(&obj_with(", share_tails", "", body));
    assert_eq!(
        tree(&plain, 4200).instructions.len(),
        tree(&shared, 4200).instructions.len()
    );
}

/// Pins `body`'s tree to a reversed node order, written as `node_order`, and checks the
/// build is exactly the reversed tree. `extra` goes in the `#[bhav(..)]` before it.
fn pins(case: &str, extra: &str, items: &str, body: &str) {
    let plain = compile(&obj_with(extra, items, body));
    let p = &tree(&plain, 4200).instructions;
    assert!(p.len() > 2, "{case}: a tree to reorder");
    let order = reversed_order(p.len());
    let original = permuted(p, &order);
    assert_eq!(
        match_node_order(p, &original),
        Some(order.clone()),
        "{case}: the order is found"
    );
    let pinned = compile(&obj_with(
        &format!("{extra}, node_order = [{}]", list(&order)),
        items,
        body,
    ));
    let t = tree(&pinned, 4200);
    assert_eq!(nodes(&t.instructions), nodes(&original), "{case}");
    // The pinned tree runs like the plain one; only the node numbers differ.
    assert_equivalent(tree(&plain, 4200), t, &behavioral());
    // The identity order builds the plain bytes.
    let identity: Vec<u16> = (0..p.len() as u16).collect();
    let same = compile(&obj_with(
        &format!("{extra}, node_order = [{}]", list(&identity)),
        items,
        body,
    ));
    assert_eq!(nodes(&tree(&same, 4200).instructions), nodes(p), "{case}");
}

const HALVE: &str = "#[bhav(id = 4202)] #[ret(temp = 2)] fn halve(x: i16) -> Option<i16> {
        if x < 2 { return None; }
        Some(x / 2)
    }";

#[test]
fn node_order_pins_every_kind_of_body() {
    pins(
        "if/else",
        "",
        "",
        "if x == 1 { self.n = 2; } else { self.m = 3; } self.n > 1",
    );
    pins(
        "match",
        "",
        "",
        "match x { 1 => { self.n = 1; } 2 => { self.n = 2; } _ => { self.m = 0; } } true",
    );
    pins(
        "while loop",
        "",
        "",
        "while self.n < 4 { self.n += 1; if self.n == x { return false; } } true",
    );
    pins(
        "if let",
        "",
        HALVE,
        "if let Some(h) = halve(x) { self.n = h; } else { self.m = 1; } true",
    );
    pins(
        "raw block",
        "",
        "",
        "raw { local[0] := param[0]; if local[0] == 2 { my.attr[0] := 1; } } self.n == 1",
    );
    pins("after share_tails", ", share_tails", "", DUPLICATED_ARMS);
}

#[test]
fn node_order_applies_to_lifecycle_trees_and_air() {
    // A lifecycle tree takes the header fields too.
    let src = obj("#[bhav(id = 4210)] init { if self.n == 1 { self.m = 2; } self.m = 1; }");
    let plain = compile(&src);
    let p = &tree(&plain, 4210).instructions;
    let order = reversed_order(p.len());
    let pinned = compile(&src.replace(
        "#[bhav(id = 4210)]",
        &format!("#[bhav(id = 4210, node_order = [{}])]", list(&order)),
    ));
    assert_eq!(
        nodes(&tree(&pinned, 4210).instructions),
        nodes(&permuted(p, &order))
    );

    // `.air` takes `share_tails: true` and `node_order: [..]` in the tree header.
    let air = |header: &str| {
        let src = format!(
            "tree 4096 \"T\" ({header}) {{\n    @n0: op 0x0002 (0, 0, 0, 0, 0, 0x02, 7, 7) -> true: @n1, false: @n2;\n    @n1: op 0x0002 (1, 0, 0, 0, 0, 0x02, 7, 7) -> true: true, false: false;\n    @n2: op 0x0002 (1, 0, 0, 0, 0, 0x02, 7, 7) -> true: true, false: false;\n}}\n"
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
    assert_eq!(plain.len(), 3);
    let shared = air("share_tails: true").expect("shared");
    assert_eq!(
        nodes(&shared)
            .iter()
            .map(|n| (n.1, n.2))
            .collect::<Vec<_>>(),
        [(1, 1), (0xFE, 0xFF)]
    );
    let moved = air("node_order: [0, 2, 1]").expect("moved");
    assert_eq!(nodes(&moved), nodes(&permuted(&plain, &[0, 2, 1])));
    let off = air("share_tails: false").expect("off");
    assert_eq!(nodes(&off), nodes(&plain));
    let err = air("share_tails: 1").expect_err("not a flag");
    assert!(err.message.contains("'true' or 'false'"), "{err}");
    let err = air("node_order: [0, 1]").expect_err("short");
    assert!(err.message.contains("lists 2 node(s)"), "{err}");
}

#[test]
fn node_order_is_checked() {
    let body = "if x == 1 { self.n = 2; } self.n > 1";
    let len = tree(&compile(&obj_with("", "", body)), 4200)
        .instructions
        .len();
    assert!(len >= 3);
    let with = |order: &str| obj_with(&format!(", node_order = [{order}]"), "", body);
    let all: Vec<u16> = (0..len as u16).collect();
    expect_err(
        &with(&list(&all[..len - 1])),
        &format!(
            "node_order lists {} node(s), but the tree compiles to {len}",
            len - 1
        ),
    );
    let mut dup = all.clone();
    dup[len - 1] = 1;
    expect_err(&with(&list(&dup)), "node_order names node 1 twice");
    let mut out = all.clone();
    out[len - 1] = len as u16;
    expect_err(&with(&list(&out)), &format!("names node {len}"));
    let mut swapped = all.clone();
    swapped.swap(0, 1);
    expect_err(&with(&list(&swapped)), "node_order must start with 0");
    expect_err(&with("0, -1"), "'node_order' entry -1 is not a node number");
    expect_err(
        &obj_with(", node_order = 3", "", body),
        "'node_order' takes a list of node numbers",
    );
    expect_err(
        &obj_with(", node_order = [0], node_order = [0]", "", body),
        "duplicate header field 'node_order'",
    );
    expect_err(
        &obj_with(", share_tails, share_tails", "", body),
        "duplicate header field 'share_tails'",
    );
    expect_err(
        &obj_with(", share_tails = 1", "", body),
        "'share_tails' takes no value",
    );
    // `#[frame(..)]` takes them too.
    let framed = obj(&format!(
        "#[bhav(id = 4200)] #[frame(share_tails, node_order = [{}])] fn t(x: i16) -> bool {{ {body} }}",
        list(&all)
    ));
    compile(&framed);
}

#[test]
fn indexed_trees_keep_their_nodes_in_place() {
    // An indexed tree (kind 4) enters at the node its first argument names.
    let body = "if x == 1 { self.n = 2; return true; } self.n = 2; true";
    let plain = compile(&obj_with(", kind = 4", "", body));
    let len = tree(&plain, 4200).instructions.len();
    expect_err(
        &obj_with(", kind = 4, share_tails", "", body),
        "is an indexed tree (kind 4), which enters at the node its first argument names: 'share_tails' would merge",
    );
    expect_err(
        &obj_with(
            &format!(", kind = 4, node_order = [{}]", list(&reversed_order(len))),
            "",
            body,
        ),
        "its node_order must keep every node in place",
    );
    // Nothing moves: both are accepted and build the same bytes.
    let identity: Vec<u16> = (0..len as u16).collect();
    let body_apart = "if x == 1 { self.n = 2; return true; } self.m = 3; false";
    let kept = compile(&obj_with(
        &format!(
            ", kind = 4, share_tails, node_order = [{}]",
            list(&identity)
        ),
        "",
        body_apart,
    ));
    let apart = compile(&obj_with(", kind = 4", "", body_apart));
    assert_eq!(
        nodes(&tree(&kept, 4200).instructions),
        nodes(&tree(&apart, 4200).instructions)
    );
}

#[test]
fn match_node_order_needs_the_same_graph() {
    let c = compile(&obj_with("", "", DUPLICATED_ARMS));
    let p = &tree(&c, 4200).instructions;
    // Another length, an operand, or a return.
    assert_eq!(match_node_order(p, &p[..p.len() - 1]), None);
    let mut other = p.clone();
    other[1].operands[0] ^= 1;
    assert_eq!(match_node_order(p, &other), None);
    let mut other = p.clone();
    let last = other.len() - 1;
    other[last].on_true = if other[last].on_true == 0xFE {
        0xFF
    } else {
        0xFE
    };
    assert_eq!(match_node_order(p, &other), None);
    // A tree with a node no branch reaches is not matched.
    let mut unreached = p.clone();
    unreached.push(unreached[0].clone());
    let mut also = p.clone();
    also.push(also[0].clone());
    assert_eq!(match_node_order(&unreached, &also), None);
    assert_eq!(match_node_order(&[], &[]), Some(Vec::new()));
}

#[test]
fn the_attributes_format_and_lint_cleanly() {
    let order: Vec<u16> = reversed_order(74);
    let src = obj_with(
        &format!(
            ",share_tails,node_order=[{}]",
            list(&order).replace(", ", ",")
        ),
        "",
        "self.n > 1",
    );
    let formatted = format_v2_source(&src).expect("formats");
    assert!(is_v2_formatted(&formatted).expect("checks"), "{formatted}");
    assert_eq!(format_v2_source(&formatted).expect("again"), formatted);
    assert!(
        formatted.contains(&format!(
            "#[bhav(id = 4200, share_tails, node_order = [{}])]",
            list(&order)
        )),
        "{formatted}"
    );
}

#[test]
fn share_tails_closes_frame_headroom() {
    // Five arms, each ending in the same 40-store tail: over 200 nodes unless they merge.
    let tail: String = (0..40).map(|_| "self.n += 1; ").collect();
    let arms: String = (1..=5)
        .map(|k| format!("if x == {k} {{ self.m = {k}; {tail} return true; }} "))
        .collect();
    let body = format!("{arms} false");
    let plain = obj_with("", "", &body);
    let p = tree(&compile(&plain), 4200).instructions.len();
    assert!(p > 200, "{p} nodes");
    let found = lint_v2_source(&plain, "step2.antics").expect("lint");
    assert!(
        found.iter().any(|d| d.rule == "frame_headroom"),
        "{found:#?}"
    );
    let shared = obj_with(", share_tails", "", &body);
    let s = tree(&compile(&shared), 4200).instructions.len();
    assert!(s < 80, "{s} nodes");
    let found = lint_v2_source(&shared, "step2.antics").expect("lint");
    assert!(
        !found.iter().any(|d| d.rule == "frame_headroom"),
        "{found:#?}"
    );
    // A tree over the node limit before merging builds when the merged tree fits.
    let tail: String = (0..60).map(|_| "self.n += 1; ").collect();
    let arms: String = (1..=5)
        .map(|k| format!("if x == {k} {{ self.m = {k}; {tail} return true; }} "))
        .collect();
    let body = format!("{arms} false");
    expect_err(&obj_with("", "", &body), "exceeds maximum of 251");
    let s = tree(&compile(&obj_with(", share_tails", "", &body)), 4200)
        .instructions
        .len();
    assert!(s < 100, "{s} nodes");
}

#[test]
fn rewrites_keep_calls_that_read_the_nodes_after_them() {
    // A call with inline arguments to a callee whose frame holds more than four arguments
    // reads the rest from the nodes after the call, which share_tails and node_order move.
    const WIDE: &str = "#[bhav(id = 4201, args = 6)] fn wide(a: i16) -> bool { true }";
    let src = |attrs: &str, body: &str| format!("{EXTERNS}\n{}", obj_with(attrs, WIDE, body));
    let wide_body = "if wide(1) { self.n = 1; } true";
    compile(&src("", wide_body));
    expect_err(
        &src(", share_tails", wide_body),
        "fn 't' has 'share_tails', which moves nodes, but this call passes its arguments inline \
         to tree 'wide', whose frame holds 6 arguments",
    );
    expect_err(
        &src(", node_order = [0, 1, 2]", wide_body),
        "has 'node_order'",
    );
    // Through temps (more than four arguments, or none) the callee reads no node: fine.
    compile(&src(
        ", share_tails",
        "if wide(temp[0]) { self.n = 1; } true",
    ));

    // A narrow callee is listed in the `.air` header, and the tree shares tails.
    let narrow = src(", share_tails", "if g::two(1, 2) { self.n = 1; } true");
    let air = sims_antics::v2::expand_v2_source_to_objects(
        "step2.antics",
        &narrow,
        &PackResolverConfig::default(),
    )
    .expect("expands")
    .remove(0)
    .air_source;
    let header = air.lines().find(|l| l.contains("\"t\"")).expect("tree t");
    assert!(header.contains("narrow_callees: [290]"), "{header}");
    compile(&narrow);

    // A raw block's inline call is listed by its callee's signature; one to a wide or
    // undeclared callee is refused by the `.air` compiler.
    let raw = |call: &str| {
        src(
            ", share_tails",
            &format!("raw {{ if call {call} {{ my.attr[0] := 1; }} }} true"),
        )
    };
    compile(&raw("290(1, 2)"));
    expect_err(&raw("4201(1)"), "calls tree 4201 with inline arguments");
    expect_err(&raw("300(1)"), "calls tree 300 with inline arguments");
    compile(&raw("300(temps)"));

    // `.air` refuses an unlisted inline call, and takes a listed one or a `temps` call.
    let air = |header: &str, call: &str| {
        compile_air(&format!(
            "tree 4096 \"T\" ({header}) {{\n    if call {call} {{ return true; }}\n    return false;\n}}\n"
        ))
    };
    let err = air("share_tails: true", "290(1, 2)").expect_err("unlisted");
    assert!(
        err.message
            .contains("node 0 calls tree 290 with inline arguments"),
        "{err}"
    );
    let err = air("node_order: [0, 1]", "290(1, 2)").expect_err("unlisted");
    assert!(
        err.message.contains("List 290 in 'narrow_callees'"),
        "{err}"
    );
    air("share_tails: true, narrow_callees: [290]", "290(1, 2)").expect("listed");
    air("share_tails: true", "290(temps)").expect("temps");
    air("args: 0", "290(1, 2)").expect("no rewrite");
}

#[test]
fn review_round_1_rewrite_soundness() {
    // A sibling whose frame is inferred from its body (param[4]: five arguments) is checked by
    // the frame it compiles to, whatever its signature says.
    let inferred = obj(
        "#[bhav(id = 4201)] fn wide() -> bool { raw { my.attr[0] := param[4]; } true }
    #[bhav(id = 4200, node_order = [0, 2, 1, 3])] fn caller() {
        raw {
            call 4201(1) -> true: true, false: false;
            op 2 (0,0,0,0,0,2,7,7) -> true: true, false: true;
            op 3 (0,0,0,0,0,0,0,0) -> true: true, false: true;
        }
    }",
    );
    expect_err(
        &inferred,
        "calls tree 4201 with inline arguments, and its frame holds 5",
    );
    compile(&inferred.replace(", node_order = [0, 2, 1, 3]", ""));

    // Two declarations of one id that disagree on its frame grant nothing, typed or raw, and
    // a discarded typed call grants nothing either.
    let conflict = |body: &str| {
        format!(
            "extern mod g {{
    #[id(290)] fn narrow(a: i16) -> bool;
    #[id(290)] fn wide(a: i16, b: i16, c: i16, d: i16, e: i16) -> bool;
}}
{}",
            obj(&format!(
                "#[bhav(id = 4200, share_tails)] fn caller() -> bool {{ {body} }}"
            ))
        )
    };
    expect_err(
        &conflict("if g::narrow(1) { self.n = 1; } else { self.n = 2; } true"),
        "another declaration gives a wider frame",
    );
    expect_err(
        &conflict("if false { g::narrow(1); } raw { call 290(1); } true"),
        "calls tree 290 with inline arguments",
    );

    // `.air`: a reserved branch byte stays refused, and so does an oversized node_order tree.
    let air = |header: &str, first: &str, n: usize| {
        let mut src = format!("tree 4096 \"T\" ({header}) {{\n");
        src.push_str(&format!(
            "  op 2 (0,0,0,0,0,2,7,7) -> true: {first}, false: true;\n"
        ));
        for _ in 1..n {
            src.push_str("  op 2 (0,0,0,0,0,2,7,7) -> true: true, false: true;\n");
        }
        src.push_str("}\n");
        compile_air(&src)
    };
    let err = air("share_tails: true", "raw(251)", 252).expect_err("reserved byte");
    assert!(
        err.message.contains("branch byte 251 names no node"),
        "{err}"
    );
    let mut order: Vec<u16> = (0..254).collect();
    order.swap(1, 253);
    let err =
        air(&format!("node_order: [{}]", list(&order)), "raw(1)", 254).expect_err("oversized");
    assert!(err.message.contains("has 254 nodes"), "{err}");

    // A list entry is an integer with at most a `-`.
    let err = compile_v2_source(
        "step2.antics",
        &obj_with(", node_order = [-+0]", "", "true"),
        &PackResolverConfig::default(),
    )
    .expect_err("-+0");
    assert!(err.message.contains("expected a list entry"), "{err}");
}

#[test]
fn review_round_2_callee_libraries_and_frames() {
    let air = |src: &str| compile_air(src);
    let caller = |header: &str, callee: u16| {
        format!(
            "tree 4096 \"Caller\" ({header}) {{
  call {callee}(1) -> true: true, false: false;
  op 2 (0,0,0,0,0,2,7,7) -> true: true, false: true;
  op 3 (0,0,0,0,0,0,0,0) -> true: true, false: true;
}}
"
        )
    };
    // A global ID in the module is not the tree a private caller runs: the VM takes it from
    // the global library, so it must be listed.
    let shadow = format!(
        "{}tree 290 \"Copy\" (args: 1) {{ return false; }}\n",
        caller("node_order: [0, 2, 1]", 290)
    );
    let err = air(&shadow).expect_err("global copy");
    assert!(
        err.message.contains("List 290 in 'narrow_callees'"),
        "{err}"
    );
    // A private callee is judged by the frame the VM reads: format 0x8001 caps it at four.
    let capped = |format: &str| {
        format!(
            "{}tree 4097 \"Callee\" (format: {format}, args: 5) {{\n  op 2 (0,0,0,0,0,2,7,7) -> true: true, false: true;\n}}\n",
            caller("node_order: [0, 2, 1]", 4097)
        )
    };
    air(&capped("0x8001")).expect("four runtime args");
    air(&capped("0x8000")).expect("four runtime args");
    let err = air(&capped("0x8002")).expect_err("five args");
    assert!(err.message.contains("its frame holds 5 arguments"), "{err}");

    // An inlined module helper sees only its module's declaration, but the object's own wider
    // declaration of the same ID still refuses the rewrite.
    let src = "extern mod small { #[id(290)] fn narrow(a: i16) -> bool; }
fn test_call() -> bool { small::narrow(1) }
object \"Conflict\" {
    guid: 0x7F40_02D0;
    extern mod big { #[id(290)] fn wide(a: i16, b: i16, c: i16, d: i16, e: i16) -> bool; }
    state n: i16;
    #[bhav(id = 4200, node_order = [0, 2, 1])] fn caller() -> bool {
        if test_call() { self.n = 1; } else { raw { op 3 (0,0,0,0,0,0,0,0); } }
        true
    }
}
";
    expect_err(src, "another declaration gives a wider frame");

    // A semi-global's library depends on the object that runs the call: no declaration lists
    // it, typed or raw.
    let semi = |body: &str| {
        format!(
            "extern mod lib {{ #[id(8192)] fn one(a: i16) -> bool; }}\n{}",
            obj(&format!(
                "#[bhav(id = 4200, share_tails)] fn caller() -> bool {{ {body} }}"
            ))
        )
    };
    expect_err(
        &semi("if lib::one(1) { self.n = 1; } true"),
        "a semi-global tree, whose library (and frame) depends on the object",
    );
    expect_err(
        &semi("raw { call 8192(1); } true"),
        "calls tree 8192 with inline arguments",
    );
    compile(&semi("raw { call 8192(temps); } true"));
}

#[test]
fn light_names_object_data_word_51() {
    let same = |a: &str, b: &str| {
        let (x, y) = (
            compile(&obj(&format!("#[bhav(id = 4220)] fn t() {{ {a} }}"))),
            compile(&obj(&format!("#[bhav(id = 4220)] fn t() {{ {b} }}"))),
        );
        assert_eq!(x.iff_bytes, y.iff_bytes, "{a} vs {b}");
    };
    same("self.light = 25;", "self.data[51] = 25;");
    same(
        "if self.light > 0 { self.n = self.light; }",
        "if self.data[51] > 0 { self.n = self.data[51]; }",
    );
    same(
        "for o in lot.objects() { o.light = 0; }",
        "for o in lot.objects() { o.data[51] = 0; }",
    );
    // A state of the same name still wins on `self`.
    let x = compile(&obj(
        "state light: i16;\n    #[bhav(id = 4220)] fn t() { self.light = 3; }",
    ));
    let y = compile(&obj(
        "state light: i16;\n    #[bhav(id = 4220)] fn t() { self.data[51] = 3; }",
    ));
    assert_ne!(x.iff_bytes, y.iff_bytes);
}

const EXTERNS: &str = "extern mod g {
    #[id(290)] fn two(a: i16, b: i16) -> bool;
}";

/// The `.air` body of tree `t`, its lines trimmed.
fn air_of_t(src: &str) -> Vec<String> {
    let air = sims_antics::v2::expand_v2_source_to_objects(
        "step2.antics",
        src,
        &PackResolverConfig::default(),
    )
    .unwrap_or_else(|e| panic!("must expand: {e}\n{src}"))
    .into_iter()
    .next()
    .expect("one object")
    .air_source;
    let at = air.find("\"t\"").expect("tree t");
    air[at..]
        .lines()
        .skip(1)
        .take_while(|l| *l != "}")
        .map(|l| l.trim().to_owned())
        .collect()
}

#[test]
fn locals_and_parameters_beside_temps_already_need_no_copy() {
    // The copying form snapshots a temp argument, and the optimizer forwards the snapshot
    // when nothing between can change or observe it: a local or a parameter of the running
    // tree beside a temp costs no copy (the step 2 review of #480's staging).
    let src = |body: &str| {
        format!(
            "{EXTERNS}\nobject \"Staging\" {{\n    guid: 0x7F40_02D1;\n    state n: i16;\n    #[bhav(id = 4230)] fn t(x: i16) {{ {body} }}\n}}\n"
        )
    };
    for (body, stores) in [
        (
            "let k = self.n; if g::two(temp[1], k) { self.n = 1; }",
            ["temp[0] := temp[1];", "temp[1] := __v_k_0;"],
        ),
        (
            "if g::two(temp[1], x) { self.n = 1; }",
            ["temp[0] := temp[1];", "temp[1] := param[0];"],
        ),
    ] {
        let text = air_of_t(&src(body));
        let at = text
            .iter()
            .position(|l| l == stores[0])
            .unwrap_or_else(|| panic!("{body}: {text:#?}"));
        assert_eq!(text[at + 1], stores[1], "{body}: {text:#?}");
        assert!(
            !text.iter().any(|l| l.contains("__arg")),
            "{body}: {text:#?}"
        );
    }
    // An attribute of the stack object is a single read that runs no code: the copying form
    // reads it where it stores it, as before this step.
    let text = air_of_t(&src(
        "if g::two(temp[1], stack_obj.attr[2]) { self.n = 1; }",
    ));
    assert!(
        text.iter().any(|l| l == "temp[1] := stack.attr[2];"),
        "{text:#?}"
    );
    // Arithmetic on a temp is worked out in a local before any store, so the temp it reads
    // still holds the caller's value.
    let text = air_of_t(&src("if g::two(temp[1], temp[0] + 1) { self.n = 1; }"));
    assert!(text.iter().any(|l| l.contains("__arith")), "{text:#?}");
    assert!(text.iter().any(|l| l.starts_with("let __arg")), "{text:#?}");
}
