//! `.antics` v2.4: the `light_mode` field (object data word 16), `self` refused as a binding,
//! option bindings read in place from their temp, the `semi_global` link (`GLOB`) with
//! library-tagged `extern "Lib" mod` blocks, and `std::globals` frames consulted by every
//! object. All sources and library names are synthetic.

use sims_antics::ir::IFF_CONVERTED_FLAG;
use sims_antics::v2::{format_v2_source, is_v2_formatted};
use sims_antics::{CompiledObject, PackResolverConfig, compile_v2_source};
use sims_formats::execution::global_link;
use sims_formats::iff::Iff;

fn compile(src: &str) -> CompiledObject {
    compile_v2_source("v24.antics", src, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("must compile: {e}\n{src}"))
}

fn expect_err(src: &str, needle: &str) {
    let err = compile_v2_source("v24.antics", src, &PackResolverConfig::default())
        .expect_err(&format!("must fail:\n{src}"));
    assert!(
        err.message.contains(needle),
        "expected '{needle}' in: {err}"
    );
}

fn air(src: &str) -> String {
    sims_antics::v2::expand_v2_source_to_objects("v24.antics", src, &PackResolverConfig::default())
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

/// A synthetic object: `head` (meta fields), `items` as members, then a Sim tree `t` (id 4200)
/// with `attrs` added to its `#[bhav(..)]` and `body`.
fn obj(head: &str, items: &str, attrs: &str, body: &str) -> String {
    format!(
        r#"object "V2.4 Probe" {{
    guid: 0x7F40_02E0;
    {head}
    state n: i16;
    {items}
    #[bhav(id = 4200{attrs})] fn t(sim: Sim) {{ {body} }}
}}
"#
    )
}

/// Asserts that two bodies of `t` compile to the same `.iff` bytes.
fn same_iff(case: &str, uses: &str, attrs: &str, a: &str, b: &str) {
    let sa = format!("{uses}\n{}", obj("", "", attrs, a));
    let sb = format!("{uses}\n{}", obj("", "", attrs, b));
    let (ca, cb) = (compile(&sa), compile(&sb));
    assert!(
        ca.iff_bytes == cb.iff_bytes,
        "{case}: .iff bytes differ\n--- a\n{:#?}\n--- b\n{:#?}",
        tree_lines(&air(&sa), "t"),
        tree_lines(&air(&sb), "t"),
    );
}

const GLOBALS: &str = "use std::globals::*;";

mod light_mode {
    use super::*;

    #[test]
    fn is_object_data_word_16() {
        same_iff(
            "self",
            "",
            "",
            "self.light_mode = 1; if self.light_mode == 1 { self.n = 2; }",
            "self.data[16] = 1; if self.data[16] == 1 { self.n = 2; }",
        );
        same_iff(
            "handle",
            "",
            "",
            "for o in lot.objects() { if o.light_mode == 1 { self.n += 1; } }",
            "for o in lot.objects() { if o.data[16] == 1 { self.n += 1; } }",
        );
    }
}

mod self_binding {
    use super::*;

    #[test]
    fn self_cannot_be_bound() {
        const NEEDLE: &str = "'self' always names the running object";
        for body in [
            "let self = sim.id; self.n = 1;",
            "let self: i16 = 1;",
            "for self in lot.objects() { temp[0] = 1; }",
            "if let Some(self) = globals::find_companion() { temp[0] = 1; }",
            "let Some(self) = globals::find_companion() else { return true; };",
            "const self = if temp[1] == 0 { 7 } else { 9 }; temp[2] = self;",
            "const (self, b) = if temp[1] == 0 { (7, 8) } else { (9, 10) }; temp[2] = b;",
        ] {
            expect_err(&format!("{GLOBALS}\n{}", obj("", "", "", body)), NEEDLE);
        }
        // A constant of the object or the file.
        expect_err(&obj("", "const self = 3;", "", ""), NEEDLE);
        expect_err(&format!("const self = 3;\n{}", obj("", "", "", "")), NEEDLE);
        // A tuning parameter, which a const binding may yield.
        expect_err(&obj("", "param self: i16 = 3;", "", ""), NEEDLE);
        // Parameters: a Sim parameter, a helper's, a tree's.
        let src = r#"object "V2.4 Probe" {
    guid: 0x7F40_02E0;
    #[bhav] fn t(self: Sim) { }
}
"#;
        expect_err(src, NEEDLE);
        let src = r#"fn helper(self: i16) -> i16 { self }
object "V2.4 Probe" {
    guid: 0x7F40_02E0;
    #[bhav] fn t() { temp[0] = helper(1); }
}
"#;
        expect_err(src, NEEDLE);
        let src = r#"object "V2.4 Probe" {
    guid: 0x7F40_02E0;
    #[bhav] fn t(self: i16) { }
}
"#;
        expect_err(src, NEEDLE);
    }
}

mod option_alias {
    use super::*;

    #[test]
    fn a_binding_reads_its_temp_in_place() {
        // `let .. else`: reads after other statements and in a condition, no frame word.
        same_iff(
            "let else",
            GLOBALS,
            "",
            "let Some(k) = globals::take_elapsed_periods(20) else { return true; };
             sim.person[28] -= k;
             self.n = 3;
             if k > 2 { self.n = k; }",
            "if !globals::take_elapsed_periods(20) { return true; }
             sim.person[28] -= temp[0];
             self.n = 3;
             if temp[0] > 2 { self.n = temp[0]; }",
        );
        // `if let`, with a write to another temp between the reads.
        same_iff(
            "if let",
            GLOBALS,
            "",
            "if let Some(k) = globals::take_elapsed_periods(20) {
                 sim.person[28] -= k;
                 temp[1] = 4;
                 self.n = k;
             }",
            "if globals::take_elapsed_periods(20) {
                 sim.person[28] -= temp[0];
                 temp[1] = 4;
                 self.n = temp[0];
             }",
        );
        // A loop that writes no temp may read it.
        same_iff(
            "loop",
            GLOBALS,
            "",
            "let Some(k) = globals::take_elapsed_periods(20) else { return true; };
             while self.n < k { self.n += 1; }",
            "if !globals::take_elapsed_periods(20) { return true; }
             while self.n < temp[0] { self.n += 1; }",
        );
        // A fixed frame with no locals holds it.
        let src = format!(
            "{GLOBALS}\n{}",
            obj(
                "",
                "",
                ", args = 0, locals = 0, format = 0x8002",
                "let Some(k) = globals::take_elapsed_periods(20) else { return true; };
                 sim.person[28] -= k;",
            )
        );
        assert_eq!(compile(&src).behaviors[&4200].locals, 0);
    }

    #[test]
    fn a_discarded_branch_leaves_no_alias() {
        // A folded-away branch is lowered only to check it, and its fresh names come again: its
        // option binding must not make a later hand-written copy of the same name go.
        same_iff(
            "discarded",
            GLOBALS,
            "",
            "if false {
                 if let Some(k) = globals::take_elapsed_periods(20) { self.n = k; }
             }
             let k = temp[0];
             self.n = k;
             self.n += k;",
            "let k = temp[0];
             self.n = k;
             self.n += k;",
        );
    }

    #[test]
    fn a_binding_the_temp_may_change_under_keeps_its_local() {
        // Each `after` may write temp[0] between two reads, or writes the binding: the binding
        // keeps its local, exactly as if it were copied to a named local by hand.
        for (case, after) in [
            ("call", "globals::idle_ticks(1)?;"),
            ("temp write", "temp[0] = 5;"),
            ("binding write", "k += 1;"),
            (
                "loop with a call",
                "while self.n < 3 { globals::idle_ticks(1)?; self.n += 1; }",
            ),
            ("raw temp write", "raw { temp[0] := 1; }"),
        ] {
            let aliased = format!(
                "let Some(k) = globals::take_elapsed_periods(20) else {{ return true; }};
                 self.n = k; {after} sim.person[28] -= k;"
            );
            let local = format!(
                "if !globals::take_elapsed_periods(20) {{ return true; }}
                 let mut k = temp[0]; self.n = k; {after} sim.person[28] -= k;"
            );
            same_iff(case, GLOBALS, "", &aliased, &local);
            let src = format!("{GLOBALS}\n{}", obj("", "", "", &aliased));
            let tree = tree_lines(&air(&src), "t");
            assert!(
                tree.iter()
                    .any(|l| l.contains("= temp[0];") && l.starts_with("let ")),
                "{case}: {tree:#?}"
            );
        }
        // A read inside a loop that calls: a later turn would see the call's write.
        let src = format!(
            "{GLOBALS}\n{}",
            obj(
                "",
                "",
                "",
                "let Some(k) = globals::take_elapsed_periods(20) else { return true; };
                 while self.n < k { globals::idle_ticks(1)?; self.n += 1; }",
            )
        );
        let tree = tree_lines(&air(&src), "t");
        assert!(
            tree.iter()
                .any(|l| l.starts_with("let ") && l.ends_with("= temp[0];")),
            "{tree:#?}"
        );
    }
}

/// Review round 1: the alias drops a node and a frame word, so it runs only where neither is
/// observable, as a call's in-place staging does (`copy_staging`).
mod option_alias_observability {
    use super::*;

    const BODY: &str = "let Some(k) = globals::take_elapsed_periods(20) else { return true; };
             self.n = k; self.m = k;";
    const MANUAL: &str = "if !globals::take_elapsed_periods(20) { return true; }
             let k = temp[0]; self.n = k; self.m = k;";

    type Node = (u16, u8, u8, [u8; 8]);

    fn nodes(c: &CompiledObject, id: u16) -> (Vec<Node>, u16) {
        let t = &c.behaviors[&id];
        let n = t
            .instructions
            .iter()
            .map(|i| (i.opcode, i.on_true, i.on_false, i.operands))
            .collect();
        (n, t.locals)
    }

    fn object(members: &str) -> String {
        format!(
            "{GLOBALS}\nobject \"V2.4 Probe\" {{\n    guid: 0x7F40_02E0;\n    state n: i16;\n    state m: i16;\n{members}\n}}\n"
        )
    }

    #[test]
    fn an_indexed_tree_keeps_its_copy() {
        // A kind-4 tree is entered at a numbered node: dropping the copy would move them.
        let c = compile(&object(&format!(
            "    #[bhav(id = 4200, kind = 4)] fn a(entry: i16) {{ {BODY} }}
    #[bhav(id = 4201, kind = 4)] fn b(entry: i16) {{ {MANUAL} }}"
        )));
        assert_eq!(nodes(&c, 4200), nodes(&c, 4201));
    }

    #[test]
    fn a_bottom_frame_keeps_its_only_word() {
        // Create Object writes word 0 of a new object's bottom frame.
        let c = compile(&object(&format!("    main {{ {BODY} }}")));
        let main = c
            .behaviors
            .values()
            .find(|t| t.label == "Main")
            .expect("main");
        assert_eq!(main.locals, 1);
    }

    #[test]
    fn a_frame_word_another_tree_reads_stays() {
        // `observe` reads the frame word `seed` left, which `produce`'s copy overwrites.
        let members = |produce: &str| {
            format!(
                "    main {{ seed(7); produce(1); observe(1); }}
    #[bhav(id = 4200)] fn produce(x: i16) {{ {produce} }}
    #[bhav(id = 4201, locals = 1)] fn observe(x: i16) {{ temp[7] = local[0]; }}
    #[bhav(id = 4202, locals = 1)] fn seed(x: i16) {{ raw {{ local[0] := param[0]; }} }}"
            )
        };
        let a = compile(&object(&members(BODY)));
        let b = compile(&object(&members(MANUAL)));
        assert_eq!(nodes(&a, 4200), nodes(&b, 4200));
        assert_eq!(nodes(&a, 4200).1, 1);
    }

    #[test]
    fn text_that_names_frame_words_keeps_the_copy() {
        // A sibling tree's dialog may show a word this tree's frame left behind.
        let members = |produce: &str| {
            format!(
                "    #[bhav(id = 4200)] fn produce(x: i16) {{ {produce} }}
    #[bhav(id = 4201)] fn show(x: i16) {{ dialog(kind: ok, title: \"T\", message: \"Value: $Local:0\"); }}"
            )
        };
        let a = compile(&object(&members(BODY)));
        let b = compile(&object(&members(MANUAL)));
        assert_eq!(nodes(&a, 4200), nodes(&b, 4200));
        assert_eq!(nodes(&a, 4200).1, 1);
    }

    #[test]
    fn an_unproven_inline_call_keeps_the_copy() {
        // The tagged declaration is narrow, but another of the ID is wide: the call may read
        // the nodes after it.
        let src = |body: &str| {
            format!(
                r#"extern "ProbeGlobals" mod probe {{ #[id(8201)] #[ret(temp = 0)] fn pick(a: i16) -> Option<i16>; }}
object "V2.4 Probe" {{
    guid: 0x7F40_02E0;
    semi_global: "ProbeGlobals";
    state n: i16;
    state m: i16;
    extern mod loose {{ #[id(8201)] fn w(a: i16, b: i16, c: i16, d: i16, e: i16, f: i16, g: i16) -> Result; }}
    #[bhav(id = 4200)] fn t(sim: Sim) {{ {body} }}
}}
"#
            )
        };
        let a = compile(&src(
            "let Some(k) = probe::pick(1) else { return true; }; self.m = k; self.n = k;",
        ));
        let b = compile(&src(
            "if !probe::pick(1) { return true; } let k = temp[0]; self.m = k; self.n = k;",
        ));
        assert_eq!(nodes(&a, 4200), nodes(&b, 4200));
    }
}

mod semi_global {
    use super::*;

    /// The object's `GLOB` chunk: (id, flags, payload).
    fn glob(c: &CompiledObject) -> Option<(u16, u16, Vec<u8>)> {
        let iff = Iff::parse(c.iff_bytes.clone()).expect("iff");
        iff.chunks
            .iter()
            .find(|ch| &ch.tag == b"GLOB")
            .map(|ch| (ch.id, ch.flags, iff.bytes[ch.payload.clone()].to_vec()))
    }

    #[test]
    fn the_link_is_a_glob_chunk() {
        let src = obj("semi_global: \"Probe Globals_2\";", "", "", "");
        let (id, flags, data) = glob(&compile(&src)).expect("GLOB chunk");
        assert_eq!((id, flags), (128, IFF_CONVERTED_FLAG));
        assert_eq!(data, b"Probe Globals_2\0");
        assert_eq!(
            global_link(&data).expect("reads"),
            Some("Probe Globals_2".to_owned())
        );
        assert_eq!(glob(&compile(&obj("", "", "", ""))), None);
        // A name of 47 characters is the longest; a first character below '0' would read as a
        // length prefix.
        let long = "A".repeat(47);
        let (_, _, data) = glob(&compile(&obj(
            &format!("semi_global: \"{long}\";"),
            "",
            "",
            "",
        )))
        .expect("GLOB chunk");
        assert_eq!(global_link(&data).expect("reads"), Some(long.clone()));
        for bad in [
            String::new(),
            " Lead".to_owned(),
            "-Lead".to_owned(),
            "Dotted.iff".to_owned(),
            format!("{long}A"),
        ] {
            expect_err(
                &obj(&format!("semi_global: \"{bad}\";"), "", "", ""),
                "semi-global library name",
            );
        }
    }

    #[test]
    fn link_and_library_blocks_format_idempotently() {
        let src = "extern   \"ProbeGlobals\" mod probe {\n#[id(8201)] fn narrow(a: i16) -> Result;\n}\nobject \"V2.4 Probe\" {\nguid: 0x7F40_02E0;\nsemi_global:   \"ProbeGlobals\";\n}\n";
        let once = format_v2_source(src).expect("formats");
        assert!(is_v2_formatted(&once).expect("formats"), "{once}");
        assert!(
            once.contains("extern \"ProbeGlobals\" mod probe {"),
            "{once}"
        );
        assert!(
            once.contains("    semi_global: \"ProbeGlobals\";"),
            "{once}"
        );
    }

    const PROBE_LIB: &str = r#"extern "ProbeGlobals" mod probe {
    #[id(8201)] fn narrow(a: i16) -> Result;
}"#;

    /// Tree `t` sharing tails around `call` (a statement).
    fn shared(head: &str, items: &str, call: &str) -> String {
        format!(
            "{PROBE_LIB}\n{}",
            obj(
                head,
                items,
                ", share_tails",
                &format!(
                    "{call} if self.n > 0 {{ self.n = 1; return true; }} self.n = 1; return true;"
                ),
            )
        )
    }

    #[test]
    fn a_library_declaration_proves_its_tree_narrow_for_linked_objects() {
        let linked = shared("semi_global: \"ProbeGlobals\";", "", "probe::narrow(1)?;");
        let header = tree_lines(&air(&linked), "t").remove(0);
        assert!(header.contains("narrow_callees: [8201]"), "{header}");
        compile(&linked);
        // The engine matches the link ignoring case.
        compile(&shared(
            "semi_global: \"PROBEGLOBALS\";",
            "",
            "probe::narrow(1)?;",
        ));
        // A raw inline call too.
        compile(&shared(
            "semi_global: \"ProbeGlobals\";",
            "",
            "raw { call 8201(1, 0, 0, 0); }",
        ));
        // A narrow declaration another declaration of the same ID contradicts (of the library,
        // or with no library) stays refused.
        expect_err(
            &shared(
                "semi_global: \"ProbeGlobals\";",
                "extern \"ProbeGlobals\" mod again { #[id(8201)] fn w(a: i16, b: i16, c: i16, d: i16, e: i16) -> Result; }",
                "probe::narrow(1)?;",
            ),
            "has 'share_tails'",
        );
        expect_err(
            &shared(
                "semi_global: \"ProbeGlobals\";",
                "extern mod loose { #[id(8201)] fn w(a: i16, b: i16, c: i16, d: i16, e: i16) -> Result; }",
                "probe::narrow(1)?;",
            ),
            "whose tree id another declaration gives a wider frame",
        );
        // Another library's declaration of the ID is another tree, so it neither proves nor
        // contradicts; a declaration with no library proves nothing on its own.
        let other = r#"extern "OtherGlobals" mod other { #[id(8203)] fn w(a: i16, b: i16, c: i16, d: i16, e: i16) -> Result; }
extern mod loose { #[id(8203)] fn n(a: i16) -> Result; }"#;
        let src = format!(
            "{other}\n{}",
            obj(
                "semi_global: \"ProbeGlobals\";",
                "",
                ", share_tails",
                "loose::n(1)?; if self.n > 0 { self.n = 1; return true; } self.n = 1; return true;",
            )
        );
        expect_err(&src, "no 'extern \"Name\" mod' of that library declares it");
        // ... and does not veto a narrow declaration of the linked library.
        let src = format!(
            "{other}\n{PROBE_LIB}\n{}",
            obj(
                "semi_global: \"ProbeGlobals\";",
                "",
                ", share_tails",
                "probe::narrow(1)?; if self.n > 0 { self.n = 1; return true; } self.n = 1; return true;",
            )
        )
        .replace("8203", "8201");
        compile(&src);
    }

    #[test]
    fn a_library_block_is_only_for_its_linked_objects() {
        expect_err(
            &shared("", "", "probe::narrow(1)?;"),
            "extern mod 'probe' declares trees of semi-global library \"ProbeGlobals\", but this object links no library",
        );
        expect_err(
            &shared("semi_global: \"OtherGlobals\";", "", "probe::narrow(1)?;"),
            "but this object links \"OtherGlobals\"",
        );
        // Global IDs are the same for every object, so a library block declares none.
        expect_err(
            &format!(
                "extern \"ProbeGlobals\" mod probe {{ #[id(280)] fn idle(t: i16) -> Result; }}\n{}",
                obj("", "", "", "")
            ),
            "is a global tree, the same for every object",
        );
        expect_err(
            &format!(
                "extern \"ProbeGlobals\" bcon probe {{ }}\n{}",
                obj("", "", "", "")
            ),
            "only 'extern mod' takes a semi-global library",
        );
        // An object's block merges with the file's only when both name the same library.
        expect_err(
            &shared(
                "semi_global: \"ProbeGlobals\";",
                "extern mod probe { #[id(8201)] fn narrow(a: i16) -> Result; }",
                "probe::narrow(1)?;",
            ),
            "is declared for",
        );
        compile(&shared(
            "semi_global: \"ProbeGlobals\";",
            "extern \"probeglobals\" mod probe { #[id(8201)] fn narrow(a: i16) -> Result; }",
            "probe::narrow(1)?;",
        ));
    }
}

mod std_frames {
    use super::*;

    #[test]
    fn std_globals_frames_hold_without_the_import() {
        // A raw inline call to a global tree `std::globals` declares is narrow whether or not
        // the file imports it.
        let src = obj(
            "",
            "",
            ", share_tails",
            "raw { call 425(3, 0, 0, 0); } if self.n > 0 { self.n = 1; return true; } self.n = 1; return true;",
        );
        let header = tree_lines(&air(&src), "t").remove(0);
        assert!(header.contains("narrow_callees: [425]"), "{header}");
        compile(&src);
        // An ID it does not declare is still refused.
        let src = obj(
            "",
            "",
            ", share_tails",
            "raw { call 4000(3, 0, 0, 0); } if self.n > 0 { self.n = 1; return true; } self.n = 1; return true;",
        );
        expect_err(&src, "List 4000 in 'narrow_callees'");
    }

    #[test]
    fn the_new_std_globals_compile_as_declared() {
        // 334 answers (`-> bool`); 425 propagates its failure (`-> Result`).
        let uses = format!(
            "{GLOBALS}\nextern mod legacy {{ #[id(334)] fn c(a: i16, b: i16) -> bool; #[id(425)] fn r(a: i16) -> Result; }}"
        );
        same_iff(
            "334 and 425",
            &uses,
            "",
            "if !globals::charge_off_home_lot(20, 1) { return false; } globals::run_named_trees_until(3)?;",
            "if !legacy::c(20, 1) { return false; } legacy::r(3)?;",
        );
    }
}
