//! `.antics` v2.5 Step 1A: the explicit `keep_layout` pin, `use .. as self` refused, 382
//! `get_up() -> bool`, `#[temp(N)]` parameters beside an `Option<T>` answer, extern frame
//! widths (`#[frame(args = N)]`, `#[frame(args = unknown)]`), and option bindings read in
//! place where a fixed frame cannot hold their copies. All sources and library names are
//! synthetic.

use sims_antics::v2::{format_v2_source, is_v2_formatted};
use sims_antics::{CompiledObject, PackResolverConfig, compile_air, compile_v2_source};

fn compile(src: &str) -> CompiledObject {
    compile_v2_source("v25.antics", src, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("must compile: {e}\n{src}"))
}

fn expect_err(src: &str, needle: &str) {
    let err = compile_v2_source("v25.antics", src, &PackResolverConfig::default())
        .expect_err(&format!("must fail:\n{src}"));
    assert!(
        err.message.contains(needle),
        "expected '{needle}' in: {err}"
    );
}

fn air(src: &str) -> String {
    sims_antics::v2::expand_v2_source_to_objects("v25.antics", src, &PackResolverConfig::default())
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

/// Tree `id`'s instructions and frame locals.
fn nodes(c: &CompiledObject, id: u16) -> (Vec<Node>, u16) {
    let t = &c.behaviors[&id];
    let n = t
        .instructions
        .iter()
        .map(|i| (i.opcode, i.on_true, i.on_false, i.operands))
        .collect();
    (n, t.locals)
}

/// A synthetic object with `head` meta fields, `items` as members and one tree `t` (id 4200):
/// `#[bhav(id = 4200{attrs})] fn t{sig} { body }`.
fn obj(head: &str, items: &str, attrs: &str, sig: &str, body: &str) -> String {
    format!(
        r#"object "V2.5 Probe" {{
    guid: 0x7F40_0300;
    {head}
    state n: i16;
    state m: i16;
    {items}
    #[bhav(id = 4200{attrs})] fn t{sig} {{ {body} }}
}}
"#
    )
}

const GLOBALS: &str = "use std::globals::*;";

mod keep_layout {
    use super::*;

    /// The body the backend pass rewrites: it forwards `a` into `b`.
    pub(super) const BODY: &str = "let a = self.n; let b = a + 1; self.m = b; self.n = 0;";

    #[test]
    fn pins_the_layout_a_raw_anchor_kept() {
        // The anchor: a raw block that keeps its layout, at the end of the body.
        let anchored = obj(
            "",
            "",
            "",
            "()",
            &format!("{BODY} raw {{ remove_object(0, flags: 3); }}"),
        );
        let pinned = obj(
            "",
            "",
            ", keep_layout",
            "()",
            &format!("{BODY} remove_object(me, flags: 3);"),
        );
        let unpinned = obj(
            "",
            "",
            "",
            "()",
            &format!("{BODY} remove_object(me, flags: 3);"),
        );
        let (a, p, u) = (compile(&anchored), compile(&pinned), compile(&unpinned));
        assert_eq!(nodes(&a, 4200), nodes(&p, 4200));
        assert_ne!(nodes(&p, 4200), nodes(&u, 4200), "the pin keeps `a`'s copy");
        let header = tree_lines(&air(&pinned), "t").remove(0);
        assert!(header.contains("keep_layout: true"), "{header}");
        let header = tree_lines(&air(&unpinned), "t").remove(0);
        assert!(!header.contains("keep_layout"), "{header}");
    }

    #[test]
    fn a_lifecycle_frame_takes_it() {
        let src = |frame: &str, tail: &str| {
            format!(
                "object \"V2.5 Probe\" {{\n    guid: 0x7F40_0300;\n    state n: i16;\n    state m: i16;\n    {frame} main {{ {BODY} {tail} }}\n}}\n"
            )
        };
        let label = |c: &CompiledObject| {
            *c.behaviors
                .iter()
                .find(|(_, t)| t.label == "Main")
                .expect("main")
                .0
        };
        let pinned = compile(&src(
            "#[frame(keep_layout)]",
            "remove_object(me, flags: 3);",
        ));
        let anchored = compile(&src("", "raw { remove_object(0, flags: 3); }"));
        assert_eq!(
            nodes(&pinned, label(&pinned)),
            nodes(&anchored, label(&anchored))
        );
    }

    #[test]
    fn formats_and_refuses_misuse() {
        let src = obj("", "", ", keep_layout", "()", "self.n = 1;");
        let formatted = format_v2_source(&src).expect("formats");
        assert!(
            formatted.contains("#[bhav(id = 4200, keep_layout)]"),
            "{formatted}"
        );
        assert!(is_v2_formatted(&formatted).expect("parses"));
        assert_eq!(format_v2_source(&formatted).expect("formats"), formatted);
        expect_err(
            &obj("", "", ", keep_layout, keep_layout", "()", ""),
            "duplicate header field 'keep_layout'",
        );
        expect_err(
            &obj("", "", ", keep_layout = 1", "()", ""),
            "'keep_layout' takes no value",
        );
    }

    #[test]
    fn air_reads_the_header_field() {
        let tree = |field: &str| format!("tree 4096 \"T\" ({field}) {{ temp[0] := 1; }}");
        compile_air(&tree("keep_layout: true")).expect("true");
        compile_air(&tree("keep_layout: false")).expect("false");
        let err = compile_air(&tree("keep_layout: 1")).expect_err("1");
        assert!(
            err.message
                .contains("expected 'true' or 'false' after 'keep_layout:'"),
            "{err}"
        );
    }
}

mod use_as_self {
    use super::*;

    #[test]
    fn an_import_cannot_be_named_self() {
        const NEEDLE: &str = "'self' always names the running object and cannot be imported";
        for uses in [
            "use std::globals::find_companion as self;",
            "use std::globals::{find_companion as self};",
            "use std::flags::{ObjectFlags, ObjectFlags2 as self};",
        ] {
            expect_err(&format!("{uses}\n{}", obj("", "", "", "()", "")), NEEDLE);
        }
        // Another alias is fine.
        compile(&format!(
            "use std::flags::ObjectFlags as Flags;\n{}",
            obj("", "", "", "()", "self.n = Flags::Hidden as i16;")
        ));
    }
}

mod get_up {
    use super::*;

    #[test]
    fn global_382_answers_bool() {
        let src = format!(
            "{GLOBALS}\n{}",
            obj(
                "",
                "",
                "",
                "(sim: Sim)",
                "if !globals::get_up() { return false; } self.n = 1;"
            )
        );
        let tree = tree_lines(&air(&src), "t");
        assert!(
            tree.contains(&"if !call 382(temps) {".to_owned()),
            "{tree:#?}"
        );
        // A local `-> Result` declaration calls it another way, which the merge refuses.
        expect_err(
            &format!(
                "{GLOBALS}\nextern mod globals {{ #[id(382)] fn stand() -> Result; }}\n{}",
                obj("", "", "", "(sim: Sim)", "globals::stand()?;")
            ),
            "calls tree 382 two ways",
        );
    }
}

mod temp_params_with_option {
    use super::*;

    const LIB: &str = "extern mod g {
    #[id(300)] #[ret(temp = 0)] fn f(#[temp(1)] x: i16) -> Option<i16>;
    #[id(301)] #[ret(temp = 2)] fn h(a: i16, #[temp(3)] y: i16) -> Option<ObjectId>;
}";

    #[test]
    fn a_temp_parameter_beside_an_option_answer() {
        let src = format!(
            "{LIB}\n{}",
            obj(
                "",
                "",
                "",
                "(sim: Sim)",
                "let Some(k) = g::f(self.n) else { return true; };
                 sim.person[28] -= k;
                 if let Some(o) = g::h(4, 7) { stack_obj = o; }"
            )
        );
        let tree = tree_lines(&air(&src), "t");
        for want in [
            "temp[1] := stack.attr[0];",
            "if !call 300(temps) {",
            "my.person[28] -= temp[0];",
            "temp[3] := 7;",
            "if call 301(4) {",
            "stack_obj := temp[2];",
        ] {
            assert!(
                tree.contains(&want.to_owned()),
                "missing `{want}`: {tree:#?}"
            );
        }
    }
}

mod extern_frame {
    use super::*;

    /// An object linked to `ProbeGlobals` whose tree `t` shares tails after `call`, beside the
    /// library block `lib`.
    fn shared(lib: &str, call: &str) -> String {
        format!(
            "extern \"ProbeGlobals\" mod probe {{\n    {lib}\n}}\n{}",
            obj(
                "semi_global: \"ProbeGlobals\";",
                "",
                ", share_tails",
                "()",
                &format!(
                    "{call} if self.n > 0 {{ self.n = 1; return true; }} self.n = 1; return true;"
                ),
            )
        )
    }

    #[test]
    fn a_temps_called_tree_declares_its_width() {
        // Called through the temps, it declares no positional parameter: the width says how
        // wide its frame is, and a literal call of it is narrow when that is 4 or less.
        let narrow = shared(
            "#[id(8201)] #[frame(args = 2)] fn tick() -> bool;",
            "probe::tick(); raw { call 8201(1, 2); }",
        );
        let lines = tree_lines(&air(&narrow), "t");
        assert!(lines[0].contains("narrow_callees: [8201]"), "{lines:#?}");
        assert!(
            lines.iter().any(|l| l.contains("call 8201(temps)")),
            "{lines:#?}"
        );
        compile(&narrow);
    }

    #[test]
    fn a_wide_or_unknown_width_proves_nothing() {
        for frame in ["#[frame(args = 6)]", "#[frame(args = unknown)]"] {
            // A raw literal call is not listed, so the `.air` compiler refuses it.
            expect_err(
                &shared(
                    &format!("#[id(8201)] {frame} fn tick() -> bool;"),
                    "raw { call 8201(1, 2); }",
                ),
                "calls tree 8201 with inline arguments",
            );
            // A source literal call keeps the layout, which `share_tails` refuses.
            expect_err(
                &shared(
                    &format!("#[id(8201)] {frame} fn tick(a: i16) -> bool;"),
                    "probe::tick(1);",
                ),
                "share_tails",
            );
        }
        // Another declaration of the ID that is wide still vetoes a narrow one.
        expect_err(
            &shared(
                "#[id(8201)] #[frame(args = 2)] fn tick() -> bool;\n    #[id(8201)] #[frame(args = unknown)] fn tock() -> bool;",
                "raw { call 8201(1, 2); }",
            ),
            "calls tree 8201 with inline arguments",
        );
    }

    #[test]
    fn the_width_holds_the_positional_parameters() {
        let lib = |f: &str| format!("extern mod g {{ {f} }}\n{}", obj("", "", "", "()", ""));
        expect_err(
            &lib("#[id(300)] #[frame(args = 1)] fn f(a: i16, b: i16) -> bool;"),
            "declares 2 positional parameters, more than its '#[frame(args = 1)]'",
        );
        compile(&lib(
            "#[id(300)] #[frame(args = 2)] fn f(a: i16, #[temp(4)] b: i16) -> bool;",
        ));
        for (frame, needle) in [
            (
                "#[frame(locals = 1)]",
                "an extern fn takes '#[frame(args = N)]'",
            ),
            ("#[frame(args = 300)]", "'args' must fit in a byte"),
            (
                "#[frame(args = wide)]",
                "an extern fn takes '#[frame(args = N)]'",
            ),
            (
                "#[frame(args = 1)] #[frame(args = 1)]",
                "duplicate #[frame]",
            ),
        ] {
            expect_err(&lib(&format!("#[id(300)] {frame} fn f() -> bool;")), needle);
        }
    }

    /// A tree `t` that runs the body the backend pass rewrites, then `tail`, beside a block
    /// declaring tree 300 narrow and, if `veto`, wide or unknown under another name too.
    fn layout(attrs: &str, veto: &str, tail: &str) -> CompiledObject {
        compile(&format!(
            "extern mod g {{\n    #[id(300)] #[frame(args = 1)] fn narrow(a: i16) -> bool;\n    {veto}\n}}\n{}",
            obj(
                "",
                "",
                attrs,
                "()",
                &format!("{} {tail}", keep_layout::BODY)
            )
        ))
    }

    #[test]
    fn a_literal_call_not_proven_narrow_keeps_the_layout() {
        // Its narrow declaration is vetoed by another, so the callee may read the nodes after
        // the call: the backend pass leaves the tree as lowered, exactly like the pin.
        for veto in [
            "#[id(300)] #[frame(args = unknown)] fn other() -> bool;",
            "#[id(300)] #[frame(args = 6)] fn other() -> bool;",
            "#[id(300)] fn other(a: i16, b: i16, c: i16, d: i16, e: i16) -> bool;",
        ] {
            let call = "g::narrow(1);";
            assert_eq!(
                nodes(&layout("", veto, call), 4200),
                nodes(&layout(", keep_layout", veto, call), 4200),
                "{veto}"
            );
        }
        // Proven narrow, the pass still runs.
        assert_ne!(
            nodes(&layout("", "", "g::narrow(1);"), 4200),
            nodes(&layout(", keep_layout", "", "g::narrow(1);"), 4200)
        );
    }

    #[test]
    fn a_discarded_wide_call_keeps_nothing() {
        // Discarded code emits no call, so it changes no byte.
        let veto = "#[id(300)] #[frame(args = unknown)] fn other(a: i16) -> bool;";
        for tail in ["if false { g::other(1); }", "if false { g::narrow(1); }"] {
            assert_eq!(
                nodes(&layout("", veto, tail), 4200),
                nodes(&layout("", veto, ""), 4200),
                "{tail}"
            );
        }
    }

    #[test]
    fn formats_and_merges_by_width() {
        let src = format!(
            "extern \"ProbeGlobals\" mod probe {{\n#[id(8201)]   #[frame(args = unknown)] fn tick() -> bool;\n}}\n{}",
            obj(
                "semi_global: \"ProbeGlobals\";",
                "",
                "",
                "()",
                "probe::tick();"
            )
        );
        let formatted = format_v2_source(&src).expect("formats");
        assert!(
            formatted.contains("    #[id(8201)] #[frame(args = unknown)] fn tick() -> bool;"),
            "{formatted}"
        );
        assert!(is_v2_formatted(&formatted).expect("parses"));
        compile(&formatted);
        // One name keeps one meaning: a merge of two declarations of it must agree on the width.
        expect_err(
            &format!(
                "{GLOBALS}\nextern mod globals {{ #[id(382)] #[frame(args = 2)] fn get_up() -> bool; }}\n{}",
                obj("", "", "", "(sim: Sim)", "")
            ),
            "declares fn 'get_up' twice, differently",
        );
        // The same width spelled out merges.
        compile(&format!(
            "{GLOBALS}\nextern mod globals {{ #[id(382)] #[frame(args = 0)] fn get_up() -> bool; }}\n{}",
            obj("", "", "", "(sim: Sim)", "")
        ));
        expect_err(
            &format!(
                "{GLOBALS}\nextern mod globals {{ #[id(382)] #[frame(args = unknown)] fn get_up() -> bool; }}\n{}",
                obj("", "", "", "(sim: Sim)", "")
            ),
            "declares fn 'get_up' twice, differently",
        );
    }
}

/// Option bindings read in place where a fixed frame (`locals = N`) cannot hold their copies,
/// even beside text that names frame words: the copying form does not build there.
mod fixed_frame_option_alias {
    use super::*;

    /// An object with a dialog naming `$Local:0` (so the bindings are not read in place on
    /// their own) and tree `t` with `attrs` and `body`.
    fn with_text(attrs: &str, body: &str) -> String {
        format!(
            "{GLOBALS}\n{}",
            obj(
                "",
                "#[bhav(id = 4201)] fn show(x: i16) { dialog(kind: ok, title: \"T\", message: \"Value: $Local:0\"); }",
                attrs,
                "(sim: Sim)",
                body,
            )
        )
    }

    const LET_ELSE: &str = "let Some(k) = globals::take_elapsed_periods(20) else { return true; };
             sim.person[28] -= k;";
    const LET_ELSE_MANUAL: &str = "if !globals::take_elapsed_periods(20) { return true; }
             sim.person[28] -= temp[0];";
    const FIXED: &str = ", args = 0, locals = 0, format = 0x8002";

    #[test]
    fn a_frame_too_small_for_the_copy_reads_in_place() {
        let a = compile(&with_text(FIXED, LET_ELSE));
        let b = compile(&with_text(FIXED, LET_ELSE_MANUAL));
        assert_eq!(nodes(&a, 4200), nodes(&b, 4200));
        assert_eq!(nodes(&a, 4200).1, 0);
        // `if let`, and a binding read after other statements.
        let a = compile(&with_text(
            FIXED,
            "if let Some(c) = globals::find_companion() { self.n = 1; stack_obj = c; }",
        ));
        let b = compile(&with_text(
            FIXED,
            "if globals::find_companion() { self.n = 1; stack_obj = temp[0]; }",
        ));
        assert_eq!(nodes(&a, 4200), nodes(&b, 4200));
        // A frame of one word with a local of its own, which the copy would need a second for.
        let a = compile(&with_text(
            ", args = 0, locals = 1, format = 0x8002",
            &format!("let mut w = self.m; w += 1; self.m = w; {LET_ELSE}"),
        ));
        let b = compile(&with_text(
            ", args = 0, locals = 1, format = 0x8002",
            &format!("let mut w = self.m; w += 1; self.m = w; {LET_ELSE_MANUAL}"),
        ));
        assert_eq!(nodes(&a, 4200), nodes(&b, 4200));
    }

    #[test]
    fn a_frame_with_room_keeps_the_copy() {
        // The copy fits: the text may read the word it fills, so it stays (#498's guard).
        let src = with_text(", args = 0, locals = 1, format = 0x8002", LET_ELSE);
        let tree = tree_lines(&air(&src), "t");
        assert!(
            tree.iter()
                .any(|l| l.starts_with("let ") && l.ends_with("= temp[0];")),
            "{tree:#?}"
        );
        // With no header the frame grows to hold it.
        let src = with_text("", LET_ELSE);
        assert_eq!(nodes(&compile(&src), 4200).1, 1);
    }

    #[test]
    fn a_copy_the_temp_may_change_under_still_fails_to_fit() {
        // A call between the reads may write temp[0], so the binding needs its copy, which the
        // frame cannot hold: the `.air` compiler's error stands.
        expect_err(
            &with_text(
                FIXED,
                "let Some(k) = globals::take_elapsed_periods(20) else { return true; };
                 self.n = k; globals::idle_ticks(1)?; sim.person[28] -= k;",
            ),
            "declares 0 locals but requires at least 1",
        );
        // So does a kept layout, or an indexed tree entered at a numbered node.
        expect_err(
            &with_text(&format!("{FIXED}, keep_layout"), LET_ELSE),
            "declares 0 locals but requires at least 1",
        );
        expect_err(
            &with_text(&format!("{FIXED}, kind = 4"), LET_ELSE),
            "declares 0 locals but requires at least 1",
        );
    }

    #[test]
    fn the_trees_own_frame_word_text_keeps_the_copy() {
        // Its own text may show the word the copy fills, so the bindings are not read in place
        // for it, and the `.air` compiler's error stands.
        expect_err(
            &with_text(
                FIXED,
                &format!(
                    "dialog(kind: ok, title: \"T\", message: \"Value: $Local:0\"); {LET_ELSE}"
                ),
            ),
            "declares 0 locals but requires at least 1",
        );
    }

    #[test]
    fn the_idle_scratch_word_counts() {
        // `idle` beside `param[0]` takes a scratch word after the named locals: the copy and
        // the scratch overflow one word, the scratch alone fits it.
        let fixed = ", args = 1, locals = 1, format = 0x8002";
        let body = |tail: &str| format!("self.n = x; idle(1); {tail}");
        let src = |tail: &str| {
            format!(
                "{GLOBALS}\n{}",
                obj(
                    "",
                    "#[bhav(id = 4201)] fn show(x: i16) { dialog(kind: ok, title: \"T\", message: \"Value: $Local:0\"); }",
                    fixed,
                    "(sim: Sim, x: i16)",
                    &body(tail),
                )
            )
        };
        let a = compile(&src(LET_ELSE));
        let b = compile(&src(LET_ELSE_MANUAL));
        assert_eq!(nodes(&a, 4200), nodes(&b, 4200));
        assert_eq!(nodes(&a, 4200).1, 1);
    }

    #[test]
    fn without_frame_word_text_nothing_changes() {
        // No text: the binding reads in place whatever the header, as before.
        for attrs in ["", FIXED, ", args = 0, locals = 1, format = 0x8002"] {
            let a = compile(&format!(
                "{GLOBALS}\n{}",
                obj("", "", attrs, "(sim: Sim)", LET_ELSE)
            ));
            let b = compile(&format!(
                "{GLOBALS}\n{}",
                obj("", "", attrs, "(sim: Sim)", LET_ELSE_MANUAL)
            ));
            assert_eq!(nodes(&a, 4200), nodes(&b, 4200), "{attrs}");
        }
    }
}
