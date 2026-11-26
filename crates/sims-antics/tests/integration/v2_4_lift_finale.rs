//! `lift --v2` (v2.4 Step 3) writes the `dead_false = error` pin a strict tree needs to
//! build the original's nodes, and an object's semi-global link: `semi_global: "Name";`
//! with its semi-global trees declared in `extern "Name" mod semi_globals`. Every object is
//! synthetic (GUIDs in the clean-room 0x7E00_xxxx range).

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
        // The semi-global tree the link test calls, as an install's catalog gives it.
        library_arities: [(8201, 1), (8202, 6), (8204, 2)].into(),
        ..DecompileV2Options::default()
    };
    let iff = Iff::parse(build(source)).expect("parse IFF");
    let report = decompile_iff_to_v2_report(&iff, &options).expect("lift");
    assert!(report.compiled, "{}", report.source);
    let lints = lint_v2_source(&report.source, "lifted.antics").expect("lint");
    assert!(lints.is_empty(), "{lints:?}\n{}", report.source);
    report
}

/// The object's `GLOB` chunk payload.
fn glob(iff_bytes: Vec<u8>) -> Option<Vec<u8>> {
    let iff = Iff::parse(iff_bytes).expect("iff");
    iff.chunks
        .iter()
        .find(|ch| &ch.tag == b"GLOB")
        .map(|ch| iff.bytes[ch.payload.clone()].to_vec())
}

#[test]
fn a_strict_lift_pins_dead_false_error() {
    // Stores cannot fail: the original writes their false branches `error`, and numbers its
    // nodes in another order. A division by `temp[1]` faults at its own node, so only the
    // original's node numbers prove the tree.
    let source = "object \"V2.4 Lift Finale Dead False\" {
    guid: 0x7E00_01AB;
    state n: i16;
    #[bhav(id = 4200, dead_false = error, node_order = [0, 2, 1, 3, 4, 5, 6, 8, 7])] fn t(sim: Sim) {
        self.n = 3;
        if self.n > temp[0] {
            self.n -= 1;
        }
        temp[2] = self.n / temp[1];
        temp[3] = temp[2] / temp[0];
    }
}
";
    let report = lift(source, false);
    assert!(
        report
            .source
            .contains("dead_false = error, node_order = [0, 2, 1, 3, 4, 5, 6, 8, 7]"),
        "{}",
        report.source
    );
    let t = report.trees.iter().find(|t| t.id == 4200).expect("tree");
    assert_eq!(t.style, TreeStyle::Idiomatic, "{}", report.source);
    assert_eq!(t.raw_statements, 0, "{}", report.source);
    assert!(
        matches!(t.check, TreeCheck::Identical),
        "{t:?}\n{}",
        report.source
    );
    // The lifted text builds the original's bytes.
    assert_eq!(build(&report.source), build(source));
}

#[test]
fn a_semi_global_link_and_its_calls_are_lifted() {
    let source = "extern \"Finale Probe Lib\" mod probe {
    #[id(8201)] fn narrow(a: i16) -> Result;
}
object \"V2.4 Lift Finale Link\" {
    guid: 0x7E00_01AC;
    semi_global: \"Finale Probe Lib\";
    state n: i16;
    #[bhav(id = 4200)] fn t(sim: Sim) {
        probe::narrow(1)?;
        self.n = 1;
    }
}
";
    for behavioral in [false, true] {
        let report = lift(source, behavioral);
        let lifted = &report.source;
        assert!(
            lifted.contains("    semi_global: \"Finale Probe Lib\";\n"),
            "{lifted}"
        );
        assert!(
            lifted.contains("extern \"Finale Probe Lib\" mod semi_globals {"),
            "{lifted}"
        );
        assert!(!lifted.contains("extern mod semi_globals"), "{lifted}");
        // The call itself is kept, with its argument, and the tree proven.
        assert!(
            lifted.contains("semi_globals::semi_global_8201(1)?;\n"),
            "{lifted}"
        );
        let t = report.trees.iter().find(|t| t.id == 4200).expect("tree");
        assert_eq!(t.raw_statements, 0, "{lifted}");
        assert!(
            matches!(t.check, TreeCheck::Identical | TreeCheck::Equivalent),
            "{t:?}\n{lifted}"
        );
        assert_eq!(
            glob(build(lifted)).as_deref(),
            Some(&b"Finale Probe Lib\0"[..])
        );
    }
}

#[test]
fn semi_global_trees_of_unknown_width_stay_out_of_the_library_block() {
    // A library block is trusted to declare a tree's whole frame, so a call to one of its
    // trees with four or fewer words is taken as narrow. A tree the lift only sees called
    // through the temps (no literal arguments) is declared with no parameters: in the
    // library block only when its catalog arity is 4 or fewer (8204), and otherwise in an
    // untagged block (8202, six arguments; 8203, not in the catalog).
    let source = "extern \"Finale Probe Lib\" mod probe {
    #[id(8202)] fn wide() -> Result;
    #[id(8203)] fn unknown() -> Result;
    #[id(8204)] fn pair() -> Result;
}
object \"V2.4 Lift Finale Wide\" {
    guid: 0x7E00_01AE;
    semi_global: \"Finale Probe Lib\";
    state n: i16;
    #[bhav(id = 4200)] fn t(sim: Sim) {
        probe::wide()?;
        probe::unknown()?;
        probe::pair()?;
        self.n = 1;
    }
}
";
    for behavioral in [false, true] {
        let report = lift(source, behavioral);
        let lifted = &report.source;
        let block = |head: &str| {
            let at = lifted
                .find(head)
                .unwrap_or_else(|| panic!("{head}\n{lifted}"));
            lifted[at..at + lifted[at..].find("\n}").expect("block end")].to_owned()
        };
        let library = block("extern \"Finale Probe Lib\" mod semi_globals {");
        let wide = block("extern mod semi_globals_wide {");
        assert!(library.contains("#[id(8204)]"), "{lifted}");
        for id in ["#[id(8202)]", "#[id(8203)]"] {
            assert!(wide.contains(id), "{lifted}");
            assert!(!library.contains(id), "{lifted}");
        }
        assert!(lifted.contains("semi_globals_wide::"), "{lifted}");
        let t = report.trees.iter().find(|t| t.id == 4200).expect("tree");
        assert!(
            matches!(t.check, TreeCheck::Identical | TreeCheck::Equivalent),
            "{t:?}\n{lifted}"
        );
    }
}

#[test]
fn an_object_without_a_link_gets_none() {
    let source = "object \"V2.4 Lift Finale No Link\" {
    guid: 0x7E00_01AD;
    state n: i16;
    #[bhav(id = 4200)] fn t(sim: Sim) {
        self.n = 1;
    }
}
";
    let report = lift(source, false);
    assert!(!report.source.contains("semi_global:"), "{}", report.source);
    assert_eq!(glob(build(&report.source)), None);
}
