//! `lift --v2` (v2.5 Step 1B): a local the tree leaves by number is read as `local[N]`, and
//! the `if let` statements of a tree short of locals are tried after the others. Every
//! object is synthetic (GUIDs in the clean-room 0x7E00_xxxx range).

use sims_antics::compile_v2_source;
use sims_antics::v2::{
    DecompileV2Options, DecompileV2Report, PackResolverConfig, TreeCheck, TreeStyle,
    decompile_iff_to_v2_report, lint_v2_source,
};
use sims_formats::iff::Iff;

fn build(source: &str) -> Vec<u8> {
    compile_v2_source("locals.antics", source, &PackResolverConfig::default())
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
    // Since v2.6 Step 2A the lint spells a raw `local[N]` store natively when that builds the
    // same bytes; the lift still writes such stores raw (a Track 9 follow-up).
    let lints: Vec<_> = lints
        .into_iter()
        .filter(|d| {
            !(d.rule == "manual_raw_has_builtin"
                && d.help
                    .as_deref()
                    .is_some_and(|h| h.ends_with("v2 reads and writes `local[N]` natively")))
        })
        .collect();
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
fn a_local_left_by_number_is_read_as_local_n() {
    // Named, the tree is the optimizer's to rewrite (it forwards the stored values), so its
    // locals stay numbered and the raw stores keep its layout. The statements that only read
    // them are v2 code, which builds the same nodes.
    let source = "object \"V2.5 Lift Locals Reads\" {
    guid: 0x7E00_01C1;
    state n: i16;
    #[bhav(id = 4200, locals = 2)] fn t(sim: Sim) {
        raw {
            local[0] := 5;
            local[1] := stack_obj;
        }
        temp[0] = local[0];
        stack_obj = local[1];
        self.n = temp[0];
    }
}
";
    for behavioral in [false, true] {
        let report = lift(source, behavioral);
        let t = body(&report.source, "t");
        for read in ["temp[0] = local[0];", "stack_obj = local[1];"] {
            assert!(t.contains(read), "{read}\n{}", report.source);
        }
        let tree = report.trees.iter().find(|t| t.id == 4200).expect("tree");
        assert_eq!(tree.style, TreeStyle::Idiomatic, "{}", report.source);
        assert_eq!(tree.raw_statements, 2, "{}", report.source);
        if !behavioral {
            assert!(matches!(tree.check, TreeCheck::Identical), "{tree:?}");
            assert_eq!(build(&report.source), build(source));
        }
    }
}

/// Thirty trees whose `if let` needs a frame word their header lacks, then `free`, after
/// `head` (more trees, ahead of them).
fn short_trees_source(guid: u32, head: &str) -> String {
    let mut trees = String::new();
    for i in 0..30 {
        trees.push_str(&format!(
            "    #[bhav(id = {})] fn short_{i}(sim: Sim) {{
        raw {{
            expr(my.data[40] flag_set 8) pad 0xff;
        }}
        if globals::find_companion() {{
            stack_obj = temp[0];
        }}
    }}
",
            4200 + i
        ));
    }
    format!(
        "use std::globals::*;
object \"V2.5 Lift Locals If Let {guid:x}\" {{
    guid: 0x{guid:x};
{head}{trees}    #[bhav(id = 4250)] fn free(sim: Sim) {{
        if globals::find_companion() {{
            stack_obj = temp[0];
        }}
    }}
}}
"
    )
}

#[test]
fn if_lets_in_short_trees_do_not_starve_the_others() {
    // Each `short` tree keeps its layout (a padded raw store), so an `if let` binding there
    // needs a frame word its header does not declare. Thirty of them ahead of `free` used to
    // spend the whole build budget before `free`'s statement was tried.
    let source = short_trees_source(0x7E00_01C2, "");
    let report = lift(&source, false);
    assert!(
        body(&report.source, "free").contains("let Some(companion) = globals::find_companion()"),
        "{}",
        report.source
    );
    assert!(
        body(&report.source, "short_0").contains("if !globals::find_companion()"),
        "{}",
        report.source
    );
    assert_eq!(build(&report.source), build(&source));
}

#[test]
fn every_tree_a_failed_read_build_blames_keeps_its_reads_raw() {
    // Each `bad` tree's padded read builds other bytes as `local[N]`. One build names all four,
    // so the build after it reads `good`'s locals in place (review round 1: one blamed tree a
    // build used to leave the budget spent before `good` was reached).
    let mut trees = String::new();
    for i in 0..4 {
        trees.push_str(&format!(
            "    #[bhav(id = {}, locals = 2)] fn bad_{i}(sim: Sim) {{
        raw {{
            local[0] := 5;
            local[1] := stack_obj;
            expr(temp[0] := local[0]) pad 0xff;
        }}
        stack_obj = local[1];
        self.n = temp[0];
    }}
",
            4200 + i
        ));
    }
    let source = format!(
        "object \"V2.5 Lift Locals Blame\" {{
    guid: 0x7E00_01C3;
    state n: i16;
{trees}    #[bhav(id = 4250, locals = 2)] fn good(sim: Sim) {{
        raw {{
            local[0] := 5;
            local[1] := stack_obj;
        }}
        temp[0] = local[0];
        stack_obj = local[1];
        self.n = temp[0];
    }}
}}
"
    );
    let report = lift(&source, false);
    let good = body(&report.source, "good");
    assert!(good.contains("temp[0] = local[0];"), "{}", report.source);
    assert!(good.contains("stack_obj = local[1];"), "{}", report.source);
    assert_eq!(build(&report.source), build(&source));
}

#[test]
fn if_lets_all_in_short_trees_get_the_whole_budget() {
    // Every statement is in a tree the census calls short, so they are searched as one group
    // with the whole budget, as before the split (review round 1: they used to get 8 builds,
    // which the four refused loops spend before the four reads are reached). The last read is
    // the tree's tail, written `let..else`.
    let mut ifs = String::new();
    for i in 0..4 {
        ifs.push_str(&format!(
            "        if globals::find_companion() {{
            while temp[0] > {i} {{
                temp[0] -= 1;
            }}
        }}
"
        ));
    }
    for _ in 0..4 {
        ifs.push_str(
            "        if globals::find_companion() {
            stack_obj = temp[0];
        }
",
        );
    }
    let source = format!(
        "use std::globals::*;
object \"V2.5 Lift Locals If Let Budget\" {{
    guid: 0x7E00_01C4;
    #[bhav(id = 4200, locals = 0)] fn probe(sim: Sim) {{
{ifs}    }}
}}
"
    );
    let report = lift(&source, false);
    assert_eq!(
        report
            .source
            .matches("Some(companion) = globals::find_companion()")
            .count(),
        4,
        "{}",
        report.source
    );
    assert_eq!(build(&report.source), build(&source));
}

#[test]
fn a_statement_that_breaks_the_census_build_leaves_the_rest_counted() {
    // `poison`'s `let..else` candidate does not build (its else block falls through), which
    // fails the one build of every statement; the halves still name the short trees (review
    // round 2: the census used to name none, and `free` starved again).
    let poison = "    #[bhav(id = 4199)] fn poison(sim: Sim) {
        if !globals::find_companion() {
            raw {
                expr(my.data[40] flag_set 8) pad 0xff;
            }
        }
        stack_obj = temp[0];
        temp[1] = stack_obj;
    }
";
    let source = short_trees_source(0x7E00_01C5, poison);
    let report = lift(&source, false);
    assert!(
        body(&report.source, "free").contains("let Some(companion) = globals::find_companion()"),
        "{}",
        report.source
    );
    assert_eq!(build(&report.source), build(&source));
}
