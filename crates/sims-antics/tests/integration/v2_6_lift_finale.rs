//! `lift --v2` (v2.6 Step 2B): an original's unreachable nodes go back as a
//! `dead_nodes { }` footer, a tree the optimizer would rewrite is tried with
//! `#[bhav(.., keep_layout)]` before its idioms are dropped, and a local the tree leaves by
//! number is written (not only read) as `local[N]`. Every object is synthetic (GUIDs in the
//! clean-room 0x7E00_xxxx range).

use sims_antics::compile_v2_source;
use sims_antics::v2::{
    DecompileV2Options, DecompileV2Report, PackResolverConfig, TreeCheck, TreeStyle,
    decompile_iff_to_v2_report, lint_v2_source,
};
use sims_formats::iff::Iff;

fn build(source: &str) -> Vec<u8> {
    compile_v2_source("finale.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{e}\n{source}"))
        .iff_bytes
}

fn lift(source: &str, behavioral: bool) -> DecompileV2Report {
    let options = DecompileV2Options {
        behavioral,
        ..DecompileV2Options::default()
    };
    let iff = Iff::parse(build(source)).expect("parse IFF");
    let report = decompile_iff_to_v2_report(&iff, &options).expect("lift");
    assert!(report.compiled, "{}", report.source);
    let lints = lint_v2_source(&report.source, "lifted.antics").expect("lint");
    assert!(lints.is_empty(), "{lints:?}\n{}", report.source);
    report
}

/// The lifted body of tree `name`, from its `fn` line to its closing brace.
fn body<'a>(source: &'a str, name: &str) -> &'a str {
    let start = source
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("no fn {name}\n{source}"));
    let end = source[start..].find("\n    }\n").expect("end of fn") + start;
    &source[start..end]
}

#[test]
fn unreachable_nodes_go_back_as_a_dead_nodes_footer() {
    // Nodes 1 and 4 are never reached. They used to keep the whole tree raw.
    let source = "object \"V2.6 Lift Finale Dead Nodes\" {
    guid: 0x7E00_01E1;
    state n: i16;
    state m: i16;
    #[bhav(id = 4200)] fn t(sim: Sim) {
        self.n = 1;
        if self.m == 2 {
            self.n = 3;
        }
        self.m = 0;
        dead_nodes {
            1: op 0x0002(0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04, 0x00) -> true: 2, false: error;
            4: op 0x0002(0, 0, 0, 0, 0, 0, 0, 0) -> true: 251, false: false;
        }
    }
}
";
    let report = lift(source, false);
    let lifted = &report.source;
    let t = body(lifted, "t");
    assert!(t.contains("dead_nodes {"), "{lifted}");
    assert!(!t.contains("raw {"), "{lifted}");
    let tree = report.trees.iter().find(|t| t.id == 4200).expect("tree");
    assert_eq!(tree.style, TreeStyle::Idiomatic, "{lifted}");
    assert_eq!(tree.raw_statements, 0, "{lifted}");
    assert!(matches!(tree.check, TreeCheck::Identical), "{tree:?}");
    assert_eq!(build(lifted), build(source));
    // A behavioral proof does not need the original's nodes; whatever it writes is proven.
    let behavioral = lift(source, true);
    let tree = behavioral
        .trees
        .iter()
        .find(|t| t.id == 4200)
        .expect("tree");
    assert!(
        matches!(tree.check, TreeCheck::Identical | TreeCheck::Equivalent),
        "{tree:?}\n{}",
        behavioral.source
    );
}

#[test]
fn identical_arms_keep_their_nodes_with_keep_layout() {
    // Both arms store the same value: the optimizer merges them unless the layout is kept.
    // The raw stores that kept it before the idioms were dropped are now native.
    let source = "object \"V2.6 Lift Finale Arms\" {
    guid: 0x7E00_01E2;
    state n: i16;
    state m: i16;
    #[bhav(id = 4200, keep_layout)] fn t(sim: Sim) {
        if self.n == 0 {
            self.m = 1;
        } else {
            self.m = 1;
        }
        self.n = 2;
    }
}
";
    let report = lift(source, false);
    let lifted = &report.source;
    assert!(lifted.contains(", keep_layout)]"), "{lifted}");
    assert!(!lifted.contains("raw {"), "{lifted}");
    let tree = report.trees.iter().find(|t| t.id == 4200).expect("tree");
    assert!(matches!(tree.check, TreeCheck::Identical), "{tree:?}");
    assert_eq!(build(lifted), build(source));
    assert_ne!(
        build(lifted),
        build(&lifted.replace(", keep_layout", "")),
        "{lifted}"
    );
}

/// A tree whose nodes are out of the compiler's order (so its lift is pinned with
/// `node_order`) that stores to and reads two locals, ending with a padded read of
/// `local[1]` that stays raw and keeps the locals by number.
fn shuffled_locals_tree(id: u16, name: &str, padded_read: bool) -> String {
    shuffled_locals_tree_with(id, name, padded_read, true)
}

/// [`shuffled_locals_tree`], with the padded read of `local[1]` at the end of the body when
/// `tail`. Without it, a tree that builds the original's bytes natively once its layout is
/// kept is pinned and proven with its locals named.
fn shuffled_locals_tree_with(id: u16, name: &str, padded_read: bool, tail: bool) -> String {
    let read = if padded_read {
        "expr(temp[0] := local[0]) pad 0xff"
    } else {
        "expr(temp[0] := local[0])"
    };
    let (n2, n9) = if tail {
        (
            "@n9",
            "\n            @n9: expr(my.attr[1] := local[1]) pad 0xfe -> true: true, false: error;",
        )
    } else {
        ("true", "")
    };
    format!(
        "    #[bhav(id = {id}, locals = 2)] fn {name}(sim: Sim) {{
        raw {{
            @n0: expr(local[0] := 5) -> true: @n3, false: error;
            @n1: expr(stack_obj := local[1]) -> true: @n2, false: error;
            @n2: expr(my.attr[0] := temp[0]) -> true: {n2}, false: error;
            @n3: expr(local[1] := stack_obj) -> true: @n4, false: error;
            @n4: expr(local[0] += 2) -> true: @n5, false: error;
            @n5: {read} -> true: @n1, false: error;{n9}
        }}
    }}
"
    )
}

#[test]
fn a_local_left_by_number_is_written_as_local_n() {
    let source = format!(
        "object \"V2.6 Lift Finale Local Writes\" {{
    guid: 0x7E00_01E3;
    state n: i16;
{}}}
",
        shuffled_locals_tree(4200, "t", false)
    );
    let report = lift(&source, false);
    let t = body(&report.source, "t");
    for statement in [
        "local[0] = 5;",
        "local[1] = stack_obj;",
        "local[0] += 2;",
        "temp[0] = local[0];",
        "stack_obj = local[1];",
    ] {
        assert!(t.contains(statement), "{statement}\n{}", report.source);
    }
    assert_eq!(t.matches("raw {").count(), 1, "{}", report.source);
    let tree = report.trees.iter().find(|t| t.id == 4200).expect("tree");
    assert_eq!(tree.raw_statements, 1, "{}", report.source);
    assert!(matches!(tree.check, TreeCheck::Identical), "{tree:?}");
    assert_eq!(build(&report.source), build(&source));
}

#[test]
fn every_tree_a_failed_read_build_blames_keeps_its_reads_raw() {
    // Each `bad` tree's padded read builds other bytes as `local[N]`. One build names all four,
    // so the build after it writes `good`'s locals in place (v2.5 review round 1: one blamed
    // tree a build used to leave the budget spent before `good` was reached).
    let mut trees = String::new();
    for i in 0..4 {
        trees.push_str(&shuffled_locals_tree(4200 + i, &format!("bad_{i}"), true));
    }
    trees.push_str(&shuffled_locals_tree(4250, "good", false));
    let source = format!(
        "object \"V2.6 Lift Finale Blame\" {{
    guid: 0x7E00_01E4;
    state n: i16;
{trees}}}
"
    );
    let report = lift(&source, false);
    let good = body(&report.source, "good");
    for statement in ["temp[0] = local[0];", "local[1] = stack_obj;"] {
        assert!(good.contains(statement), "{statement}\n{}", report.source);
    }
    for i in 0..4 {
        let bad = body(&report.source, &format!("bad_{i}"));
        assert!(
            bad.contains("expr(temp[0] := local[0]) pad 0xff"),
            "{}",
            report.source
        );
    }
    assert_eq!(build(&report.source), build(&source));
}

#[test]
fn a_write_that_builds_other_bytes_costs_the_tree_no_reads() {
    // The padded store builds other bytes as `local[1] = stack_obj;`, so the tree keeps its
    // writes raw; its reads still go in place.
    let tree = shuffled_locals_tree_with(4200, "t", false, false).replace(
        "expr(local[1] := stack_obj) ->",
        "expr(local[1] := stack_obj) pad 0xff ->",
    );
    let source = format!(
        "object \"V2.6 Lift Finale Write Blame\" {{
    guid: 0x7E00_01E5;
    state n: i16;
{tree}}}
"
    );
    let report = lift(&source, false);
    let t = body(&report.source, "t");
    for statement in ["temp[0] = local[0];", "stack_obj = local[1];"] {
        assert!(t.contains(statement), "{statement}\n{}", report.source);
    }
    assert!(
        t.contains("expr(local[1] := stack_obj) pad 0xff"),
        "{}",
        report.source
    );
    assert!(!t.contains("local[0] = 5;"), "{}", report.source);
    assert_eq!(build(&report.source), build(&source));
}

#[test]
fn an_inherit_tree_measures_from_me() {
    // A Sim tree calls `gauge`, an object tree, so `gauge` is written `context = inherit`.
    // There `self` is refused; the runner's id is `me.id`.
    let source = "object \"V2.6 Lift Finale Inherit Distance\" {
    guid: 0x7E00_01E7;
    state n: i16;
    #[bhav(id = 4300, locals = 0)] fn ring(sim: Sim) {
        raw {
            my.motive[1] := 2;
            if call 4301(temps) {
                temp[0] := 1;
            }
        }
    }
    #[bhav(id = 4301, locals = 0)] fn gauge() {
        raw {
            my.attr[0] := 1;
            get_distance_to(temp[0], from: my.data[11]);
            stack.data[3] := temp[0];
        }
    }
}
";
    let report = lift(source, false);
    let gauge = body(&report.source, "gauge");
    assert!(
        gauge.contains("distance_to(stack_obj, from: me.id)"),
        "{}",
        report.source
    );
    assert!(!gauge.contains("raw {"), "{}", report.source);
    assert!(
        report.source.contains("context = inherit"),
        "{}",
        report.source
    );
    let tree = report.trees.iter().find(|t| t.id == 4301).expect("tree");
    assert!(matches!(tree.check, TreeCheck::Identical), "{tree:?}");
    assert_eq!(build(&report.source), build(source));
}

#[test]
fn a_local_is_never_an_is_type_receiver() {
    // `local[N]` is a plain word: a type test of the object a local holds stays raw, and
    // costs the tree none of its other locals.
    let tree = shuffled_locals_tree_with(4200, "t", false, false).replace(
        "@n2: expr(my.attr[0] := temp[0]) -> true: true, false: error;",
        "@n2: test_object_type(0x7E00_01E8, local[1]) -> true: true, false: false;",
    );
    let source = format!(
        "object \"V2.6 Lift Finale Type Test\" {{
    guid: 0x7E00_01E8;
    state n: i16;
{tree}}}
"
    );
    let report = lift(&source, false);
    let t = body(&report.source, "t");
    assert!(!t.contains("local[1].is_type"), "{}", report.source);
    for statement in [
        "local[0] = 5;",
        "temp[0] = local[0];",
        "stack_obj = local[1];",
    ] {
        assert!(t.contains(statement), "{statement}\n{}", report.source);
    }
    assert_eq!(build(&report.source), build(&source));
}

#[test]
fn a_raw_local_store_the_locals_pass_left_is_written_native() {
    // The tree's padded store builds other bytes as `local[1] = stack_obj;`, so the locals
    // pass keeps the tree's writes raw. The store to `local[0]` after it has a raw block of
    // its own (a native statement comes between them), which builds the same bytes written
    // natively, as lint `manual_raw_has_builtin` asks (#515).
    let tree = shuffled_locals_tree_with(4200, "t", false, false)
        .replace(
            "expr(local[1] := stack_obj) -> true: @n4,",
            "expr(local[1] := stack_obj) pad 0xff -> true: @n6,",
        )
        .replace(
            "            @n5:",
            "            @n6: expr(my.attr[1] := 3) -> true: @n7, false: error;\n            \
             @n7: set_to_next(guid, guid: 0x7E0001EA, dest: local[0]) -> true: @n8, false: @n8;\n            \
             @n8: expr(my.attr[1] := 4) -> true: @n4, false: error;\n            @n5:",
        );
    let source = format!(
        "object \"V2.6 Lift Finale Raw Local Block\" {{
    guid: 0x7E00_01E9;
    state n: i16;
    state m: i16;
{tree}}}
"
    );
    let report = lift(&source, false);
    let t = body(&report.source, "t");
    assert!(t.contains("local[0] += 2;"), "{}", report.source);
    assert!(
        t.contains("set_to_next(guid, guid: GUID_7E0001EA, dest: local[0]);"),
        "{}",
        report.source
    );
    assert!(
        t.contains("expr(local[1] := stack_obj) pad 0xff"),
        "{}",
        report.source
    );
    let tree = report.trees.iter().find(|t| t.id == 4200).expect("tree");
    assert_eq!(tree.raw_statements, 2, "{}", report.source);
    assert!(matches!(tree.check, TreeCheck::Identical), "{tree:?}");
    assert_eq!(build(&report.source), build(&source));
}

#[test]
fn an_unchecked_lift_keeps_no_guid_constant_for_a_raw_step() {
    // Without validation no `polish` runs, so the raw step keeps its literal and the
    // constant kept for its native spelling goes (Sol review round 1).
    let source = "object \"V2.6 Lift Finale Unchecked Guid\" {
    guid: 0x7E00_01EB;
    state n: i16;
    #[bhav(id = 4200, locals = 1)] fn t(sim: Sim) {
        raw {
            set_to_next(11, guid: 0x7E00FFA3, dest: local[0]);
            my.attr[0] := local[0];
        }
    }
}
";
    let options = DecompileV2Options {
        validate: false,
        ..DecompileV2Options::default()
    };
    let iff = Iff::parse(build(source)).expect("parse IFF");
    let report = decompile_iff_to_v2_report(&iff, &options).expect("lift");
    assert!(!report.source.contains("const GUID_"), "{}", report.source);
    let lints = lint_v2_source(&report.source, "lifted.antics").expect("lint");
    assert!(
        lints.iter().all(|d| d.rule != "unused_decl"),
        "{lints:?}\n{}",
        report.source
    );
}

#[test]
fn a_tree_needing_keep_layout_and_dead_nodes_gets_both() {
    // Identical arms (kept by `keep_layout`) and an unreachable node: the pin predicted from
    // the build without the layout found nothing, so it is predicted again with it (Sol
    // review round 1).
    let source = "object \"V2.6 Lift Finale Arms And Dead Nodes\" {
    guid: 0x7E00_01EC;
    state n: i16;
    state m: i16;
    #[bhav(id = 4200, keep_layout)] fn t(sim: Sim) {
        if self.n == 0 {
            self.m = 1;
        } else {
            self.m = 1;
        }
        self.n = 2;
        dead_nodes {
            1: op 0x0002(0, 0, 0, 0, 0, 0, 0, 0) -> true: true, false: error;
        }
    }
}
";
    let report = lift(source, false);
    let lifted = &report.source;
    let t = body(lifted, "t");
    assert!(lifted.contains(", keep_layout)]"), "{lifted}");
    assert!(t.contains("dead_nodes {"), "{lifted}");
    assert!(!t.contains("raw {"), "{lifted}");
    let tree = report.trees.iter().find(|t| t.id == 4200).expect("tree");
    assert_eq!(tree.raw_statements, 0, "{lifted}");
    assert!(matches!(tree.check, TreeCheck::Identical), "{tree:?}");
    assert_eq!(build(lifted), build(source));
}

#[test]
fn a_framed_lifecycle_tree_takes_keep_layout() {
    // A `main` written with its whole header (`revision = 1`) as `#[bhav(..)]` whose two
    // arms end alike: the backend pass merges the tails unless the layout is kept (Sol
    // review round 2).
    let source = "use std::engine::RefreshMode;
object \"V2.6 Lift Finale Lifecycle Tail\" {
    guid: 0x7E00_01ED;
    state n: i16;
    state m: i16;
    #[bhav(id = 4200, revision = 1, keep_layout)]
    main {
        if self.n == 0 {
            self.m = 2;
            stack_obj.refresh(RefreshMode::Redraw);
            return true;
        } else {
            stack_obj.refresh(RefreshMode::Redraw);
            return true;
        }
    }
}
";
    let report = lift(source, false);
    let lifted = &report.source;
    assert!(lifted.contains(", keep_layout)]"), "{lifted}");
    assert!(!lifted.contains("raw {"), "{lifted}");
    let tree = report.trees.iter().find(|t| t.id == 4200).expect("tree");
    assert_eq!(tree.style, TreeStyle::Idiomatic, "{lifted}");
    assert!(matches!(tree.check, TreeCheck::Identical), "{tree:?}");
    assert_eq!(build(lifted), build(source));
}
