//! `lift --v2` (v2.5 Step 2B): a raw block refused alone because it is the last that keeps
//! its tree's layout is written native with `#[bhav(.., keep_layout)]`. Every object is
//! synthetic (GUIDs in the clean-room 0x7E00_xxxx range).

use sims_antics::compile_v2_source;
use sims_antics::v2::{
    DecompileV2Options, DecompileV2Report, PackResolverConfig, TreeCheck,
    decompile_iff_to_v2_report, lint_v2_source,
};
use sims_formats::iff::Iff;

fn build(source: &str) -> Vec<u8> {
    compile_v2_source("finale.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{e}\n{source}"))
        .iff_bytes
}

fn lift(source: &str, behavioral: bool) -> DecompileV2Report {
    lift_with(source, behavioral, DecompileV2Options::default())
}

fn lift_with(source: &str, behavioral: bool, options: DecompileV2Options) -> DecompileV2Report {
    let options = DecompileV2Options {
        behavioral,
        ..options
    };
    let iff = Iff::parse(build(source)).expect("parse IFF");
    let report = decompile_iff_to_v2_report(&iff, &options).expect("lift");
    assert!(report.compiled, "{}", report.source);
    let lints = lint_v2_source(&report.source, "lifted.antics").expect("lint");
    assert!(lints.is_empty(), "{lints:?}\n{}", report.source);
    report
}

#[test]
fn the_last_layout_anchor_is_written_native_with_keep_layout() {
    // Without a pin the optimizer merges the arms' equal tails; the raw block keeps the
    // layout, so the original keeps both. Spelled native alone, the block would let the
    // optimizer in; with `keep_layout` the tree keeps its layout. A label that holds the
    // word is not a pin.
    for label in ["", ", label = \"Holds keep_layout\""] {
        let source = format!(
            "use std::engine::RefreshMode;
object \"V2.5 Lift Finale Anchor\" {{
    guid: 0x7E00_01D1;
    state n: i16;
    state m: i16;
    #[bhav(id = 4200{label}, dead_false = error)] fn t(sim: Sim) {{
        if self.n == 0 {{
            self.m = 2;
            raw {{ scope(27, 0) := 1; }}
            stack_obj.refresh(RefreshMode::Redraw);
            return true;
        }} else {{
            stack_obj.refresh(RefreshMode::Redraw);
            return true;
        }}
    }}
}}
"
        );
        // Strict only: a behavioral proof does not need the original's nodes, so that lift
        // writes the tree native (tails merged) with no anchor to pin.
        let report = lift(&source, false);
        let lifted = &report.source;
        assert!(lifted.contains(", keep_layout)]"), "{lifted}");
        assert!(!lifted.contains("raw {"), "{lifted}");
        let t = report.trees.iter().find(|t| t.id == 4200).expect("tree");
        assert_eq!(t.raw_statements, 0, "{lifted}");
        assert!(
            matches!(t.check, TreeCheck::Identical | TreeCheck::Equivalent),
            "{t:?}\n{lifted}"
        );
        // The pin is what keeps the layout: without it the arms' tails merge.
        assert_ne!(
            build(lifted),
            build(&lifted.replace(", keep_layout", "")),
            "{lifted}"
        );
        let behavioral = lift(&source, true);
        assert!(
            !behavioral.source.contains("raw {"),
            "{}",
            behavioral.source
        );
    }
}

#[test]
fn a_catalog_arity_wider_than_a_byte_is_declared_unknown() {
    // `#[frame(args = N)]` takes a byte. No install gives a wider arity, but a caller's
    // catalog may; the declaration is then `unknown`, which proves no more.
    let source = "extern \"Finale Probe Lib\" mod probe {
    #[id(8205)] fn wide() -> Result;
}
object \"V2.5 Lift Finale Wide Arity\" {
    guid: 0x7E00_01D2;
    semi_global: \"Finale Probe Lib\";
    state n: i16;
    #[bhav(id = 4200)] fn t(sim: Sim) {
        probe::wide()?;
        self.n = 1;
    }
}
";
    for behavioral in [false, true] {
        let options = DecompileV2Options {
            library_arities: [(8205, 300)].into(),
            ..DecompileV2Options::default()
        };
        let report = lift_with(source, behavioral, options);
        let lifted = &report.source;
        assert!(
            lifted.contains("#[id(8205)] #[frame(args = unknown)] fn "),
            "{lifted}"
        );
        let t = report.trees.iter().find(|t| t.id == 4200).expect("tree");
        assert!(
            matches!(t.check, TreeCheck::Identical | TreeCheck::Equivalent),
            "{t:?}\n{lifted}"
        );
    }
}
