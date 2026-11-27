//! `.antics` v2.5 step 2A lint upgrades for #503: `duplicate_std_global` compares frame
//! widths, `manual_raw_has_builtin` names `keep_layout` where the native form needs it, and
//! `prefer_if_let` in trees whose header fixes the frame.
//!
//! Every fixture here is synthetic (GUIDs in the clean-room 0x7E00_xxxx range). See
//! `docs/decisions/20261126-antics-v2-5-lint.md`.

use sims_antics::v2::PackResolverConfig;
use sims_antics::v2::lint::{LintDiagnostic, lint_v2_source};

/// A synthetic object with `uses` before it and `members` in it.
fn obj(uses: &str, members: &str) -> String {
    format!(
        "{uses}object \"Synth Probe\" {{\n    guid: 0x7E00_0750;\n    price: 1;\n    state n: i16;\n    state m: i16;\n{members}\n}}\n"
    )
}

/// Tree `probe` under `attr` (`#[bhav(..)]` and any other attribute) running `body`.
fn tree(uses: &str, attr: &str, params: &str, body: &str) -> String {
    obj(
        uses,
        &format!("    {attr} fn probe({params}) {{\n{body}\n    }}"),
    )
}

fn of(source: &str, rule: &str) -> Vec<LintDiagnostic> {
    lint_v2_source(source, "probe.antics")
        .unwrap_or_else(|e| panic!("lint failed: {e}\n{source}"))
        .into_iter()
        .filter(|d| d.rule == rule)
        .collect()
}

fn one(source: &str, rule: &str) -> LintDiagnostic {
    let found = of(source, rule);
    assert_eq!(
        found.len(),
        1,
        "expected one {rule} in\n{source}\ngot {found:#?}"
    );
    found.into_iter().next().unwrap()
}

fn none(source: &str, rule: &str) {
    let found = of(source, rule);
    assert!(
        found.is_empty(),
        "unexpected {rule} in\n{source}\n{found:#?}"
    );
}

fn try_iff(source: &str) -> Option<Vec<u8>> {
    sims_antics::compile_v2_source("probe.antics", source, &PackResolverConfig::default())
        .ok()
        .map(|c| c.iff_bytes)
}

fn iff(source: &str) -> Vec<u8> {
    sims_antics::compile_v2_source("probe.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("must compile: {e}\n{source}"))
        .iff_bytes
}

/// The 1-based line and column of the first occurrence of `needle` in `source`.
fn position(source: &str, needle: &str) -> (usize, usize) {
    source
        .lines()
        .enumerate()
        .find_map(|(i, l)| l.find(needle).map(|c| (i + 1, c + 1)))
        .unwrap_or_else(|| panic!("{needle:?} not in source"))
}

/// A body the backend pass rewrites (it forwards `a` into `b`), so a kept layout shows.
const BODY: &str =
    "        let a = self.n;\n        let b = a + 1;\n        self.m = b;\n        self.n = 0;";

const PIN_HELP: &str = "add `keep_layout` to the tree's `#[bhav(..)]`";

// --- duplicate_std_global: the frame width is part of the ABI --------------------------------

/// A tree calling global 280 (`std::globals::idle_ticks(n: i16) -> Result`) through `decl`, a
/// local `extern mod more` declaration, after [`BODY`].
fn local_280(decl: &str) -> String {
    format!(
        "extern mod more {{\n    {decl}\n}}\n{}",
        tree(
            "",
            "#[bhav(label = \"Probe\")]",
            "",
            &format!("{BODY}\n        more::wait_ticks(1)?;")
        )
    )
}

/// [`local_280`] with the local module dropped and the std tree called, as the help says.
fn std_280() -> String {
    tree(
        "use std::globals::*;\n",
        "#[bhav(label = \"Probe\")]",
        "",
        &format!("{BODY}\n        globals::idle_ticks(1)?;"),
    )
}

#[test]
fn a_wide_or_unknown_local_declaration_of_a_std_id_is_not_a_duplicate() {
    for frame in ["#[frame(args = 6)]", "#[frame(args = unknown)]"] {
        let source = local_280(&format!(
            "#[id(280)] {frame} fn wait_ticks(n: i16) -> Result;"
        ));
        none(&source, "duplicate_std_global");
        // The declaration vetoes the tree's narrow frame, so the call keeps the layout: dropping
        // it for the std declaration would change the bytes.
        assert_ne!(iff(&source), iff(&std_280()), "{frame}");
    }
}

#[test]
fn a_local_declaration_of_the_std_width_is_still_a_duplicate() {
    // `args = 1` is the width the one positional parameter gives, as `std::globals` has it.
    for frame in ["", "#[frame(args = 1)] "] {
        let source = local_280(&format!(
            "#[id(280)] {frame}fn wait_ticks(n: i16) -> Result;"
        ));
        let d = one(&source, "duplicate_std_global");
        assert!(d.message.contains("globals::idle_ticks"), "{d:#?}");
        assert_eq!(iff(&source), iff(&std_280()), "{frame}");
    }
    // A narrow width other than the std one is another ABI: not the std declaration.
    let source = local_280("#[id(280)] #[frame(args = 2)] fn wait_ticks(n: i16) -> Result;");
    none(&source, "duplicate_std_global");
}

// --- manual_raw_has_builtin: `keep_layout` where the raw block kept the layout --------------

/// `source` with its raw block `raw` written as `native`, and `keep_layout` added to the probe
/// tree's attribute `attr` as `pinned_attr`.
fn rewrite(source: &str, raw: &str, native: &str, attr: &str, pinned_attr: &str) -> String {
    assert_eq!(source.matches(raw).count(), 1, "{raw:?}");
    assert_eq!(source.matches(attr).count(), 1, "{attr:?}");
    source
        .replacen(raw, native, 1)
        .replacen(attr, pinned_attr, 1)
}

/// Asserts the anchor `raw` of `source` (a tree under `attr`) gets the pin advice, and that
/// its native form `native` builds the same bytes with `keep_layout` (as `pinned_attr`) and
/// not without it.
fn needs_pin(source: &str, raw: &str, native: &str, attr: &str, pinned_attr: &str) {
    let d = one(source, "manual_raw_has_builtin");
    assert_eq!((d.line, d.column), position(source, raw), "{d:#?}");
    let help = d.help.as_deref().unwrap_or_default();
    assert!(help.contains(PIN_HELP), "{help}\n{source}");
    let original = iff(source);
    let plain = rewrite(source, raw, native, attr, attr);
    assert_ne!(try_iff(&plain), Some(original.clone()), "{plain}");
    let pinned = rewrite(source, raw, native, attr, pinned_attr);
    assert_eq!(iff(&pinned), original, "{pinned}");
    none(&pinned, "manual_raw_has_builtin");
}

const PROBE: &str = "#[bhav(label = \"Probe\")]";
const PROBE_PINNED: &str = "#[bhav(label = \"Probe\", keep_layout)]";

#[test]
fn an_anchor_s_native_form_takes_keep_layout() {
    // One anchor of each form the lint writes out exactly.
    for (raw, native) in [
        (
            "raw { remove_object(0, flags: 3); }",
            "remove_object(me, flags: 3);",
        ),
        (
            "raw { play_sound(7, stack_obj) pad(6: 0xff, 7: 0xff); }",
            "play_sound(7, stack_obj, pad6: 0xff, pad7: 0xff);",
        ),
        (
            "raw { clear_motive_changes() pad(2: 0x05, 6: 0x64); }",
            "clear_motive_changes(pad2: 0x05, pad6: 0x64);",
        ),
        (
            "raw { temp[0] := scope(24, 9); }",
            "temp[0] = stack_obj.neighbor.family_friend_count;",
        ),
        (
            "raw { scope(27, 0) := 1; }",
            "stack_obj.sprite_flags[temp[0]] = 1;",
        ),
    ] {
        let source = tree("", PROBE, "", &format!("{BODY}\n        {raw}"));
        needs_pin(&source, raw, native, PROBE, PROBE_PINNED);
    }
}

#[test]
fn the_pin_goes_in_any_bhav_attribute() {
    let raw = "raw { remove_object(0, flags: 3); }";
    let native = "remove_object(me, flags: 3);";
    for (attr, pinned) in [
        ("#[bhav]", "#[bhav(keep_layout)]"),
        ("#[bhav()]", "#[bhav(keep_layout)]"),
        ("#[bhav(id = 4200,)]", "#[bhav(id = 4200, keep_layout)]"),
        (
            "#[bhav(id = 4200, locals = 2)]",
            "#[bhav(id = 4200, locals = 2, keep_layout)]",
        ),
        // Other attributes between it and `fn`.
        (
            "#[bhav(id = 4200)] #[frame(locals = 2)]",
            "#[bhav(id = 4200, keep_layout)] #[frame(locals = 2)]",
        ),
    ] {
        let source = tree("", attr, "", &format!("{BODY}\n        {raw}"));
        needs_pin(&source, raw, native, attr, pinned);
    }
}

#[test]
fn a_native_form_that_builds_alike_needs_no_pin() {
    // Nothing for the backend pass to rewrite: the native form builds the same bytes as is.
    let raw = "raw { remove_object(0, flags: 3); }";
    let source = tree("", PROBE, "", &format!("        {raw}"));
    let d = one(&source, "manual_raw_has_builtin");
    let help = d.help.as_deref().unwrap_or_default();
    assert_eq!(help, "use remove_object(me, flags: 3)");
    let plain = source.replacen(raw, "remove_object(me, flags: 3);", 1);
    assert_eq!(iff(&plain), iff(&source));
}

#[test]
fn no_pin_is_offered_where_it_cannot_be_written_or_tried() {
    let raw = "raw { remove_object(0, flags: 3); }";
    for source in [
        // The tree already has it: the block keeps nothing more.
        tree("", PROBE_PINNED, "", &format!("{BODY}\n        {raw}")),
        // A comment in the block: the rewrite would drop it.
        tree(
            "",
            PROBE,
            "",
            &format!(
                "{BODY}\n        raw {{\n            remove_object(0, flags: 3); // kept\n        }}"
            ),
        ),
        // A lifecycle tree has no `#[bhav(..)]`.
        obj("", &format!("    main {{\n{BODY}\n        {raw}\n    }}")),
    ] {
        iff(&source);
        let d = one(&source, "manual_raw_has_builtin");
        let help = d.help.as_deref().unwrap_or_default();
        assert!(!help.contains("keep_layout"), "{help}\n{source}");
    }
    // A form the help spells with `..` has no exact statement to try.
    let source = tree(
        "",
        PROBE,
        "",
        &format!("{BODY}\n        raw {{ set_to_next(person, dest: temp[0]); }}"),
    );
    iff(&source);
    let d = one(&source, "manual_raw_has_builtin");
    assert_eq!(
        d.help.as_deref(),
        Some("use set_to_next(.., dest: ..)"),
        "{d:#?}"
    );
}

#[test]
fn a_block_of_several_statements_takes_the_pin_when_each_is_exact() {
    let raw = "raw { temp[1] := scope(33, 4); scope(27, 0) := 1; }";
    let source = tree("", PROBE, "", &format!("{BODY}\n        {raw}"));
    needs_pin(
        &source,
        raw,
        "temp[1] = job_field(4);\n        stack_obj.sprite_flags[temp[0]] = 1;",
        PROBE,
        PROBE_PINNED,
    );
}

// --- prefer_if_let: fixed `locals = N` frames (#503) -----------------------------------------
// #503 reads an option binding in place where the tree's fixed frame cannot hold its copy, so
// the lint's byte-checked trial now keeps these. Each rewrite builds the same bytes.

/// An object whose other tree's text names `$Local:0` (so a binding is not read in place for
/// that reason alone), with tree `probe(sim: Sim)` under `#[bhav(label = "Probe"{header})]`.
fn fixed(header: &str, body: &str) -> String {
    obj(
        "use std::globals::*;\n",
        &format!(
            "    #[bhav(id = 4201)] fn show(x: i16) {{\n        dialog(kind: ok, title: \"T\", message: \"Value: $Local:0\");\n    }}\n    #[bhav(label = \"Probe\"{header})] fn probe(sim: Sim) {{\n{body}\n    }}"
        ),
    )
}

const LOCALS_0: &str = ", args = 0, locals = 0, format = 0x8002";
const LOCALS_1: &str = ", args = 0, locals = 1, format = 0x8002";

/// Asserts `source` has one `prefer_if_let` at `at` suggesting `want`, and that `fixed` builds
/// the same bytes with no finding left.
fn if_let_rewrite(source: &str, at: &str, want: &str, fixed: &str) {
    let d = one(source, "prefer_if_let");
    assert_eq!(d.suggestion.as_deref(), Some(want), "{d:#?}");
    assert_eq!((d.line, d.column), position(source, at), "{d:#?}");
    assert_eq!(
        iff(source),
        iff(fixed),
        "rewrite builds other bytes:\n{fixed}"
    );
    none(fixed, "prefer_if_let");
}

#[test]
fn a_fixed_frame_takes_let_else() {
    for (header, before) in [
        (LOCALS_0, ""),
        // A frame of one word with a local of its own: the copy would need a second.
        (
            LOCALS_1,
            "        let mut w = self.m;\n        w += 1;\n        self.m = w;\n",
        ),
    ] {
        let source = fixed(
            header,
            &format!(
                "{before}        if !globals::take_elapsed_periods(20) {{\n            return true;\n        }}\n        sim.person[28] -= temp[0];"
            ),
        );
        let rewritten = fixed(
            header,
            &format!(
                "{before}        let Some(periods) = globals::take_elapsed_periods(20) else {{\n            return true;\n        }};\n        sim.person[28] -= periods;"
            ),
        );
        if_let_rewrite(
            &source,
            "if !globals::",
            "let Some(periods) = globals::take_elapsed_periods(20) else { .. };",
            &rewritten,
        );
    }
}

#[test]
fn a_fixed_frame_takes_if_let() {
    let source = fixed(
        LOCALS_0,
        "        if globals::find_companion() {\n            self.n = 1;\n            stack_obj = temp[0];\n        }",
    );
    let rewritten = fixed(
        LOCALS_0,
        "        if let Some(companion) = globals::find_companion() {\n            self.n = 1;\n            stack_obj = companion;\n        }",
    );
    if_let_rewrite(
        &source,
        "if globals::",
        "if let Some(companion) = globals::find_companion() {",
        &rewritten,
    );
}

#[test]
fn a_fixed_frame_that_must_keep_the_copy_has_no_finding() {
    let manual = "        if !globals::take_elapsed_periods(20) {\n            return true;\n        }\n        sim.person[28] -= temp[0];";
    for source in [
        // A kept layout or an indexed tree keeps the copy, which the frame cannot hold.
        fixed(&format!("{LOCALS_0}, keep_layout"), manual),
        fixed(&format!("{LOCALS_0}, kind = 4"), manual),
        // The tree's own text may show the word the copy fills.
        fixed(
            LOCALS_0,
            &format!(
                "        dialog(kind: ok, title: \"T\", message: \"Value: $Local:0\");\n{manual}"
            ),
        ),
    ] {
        iff(&source);
        none(&source, "prefer_if_let");
    }
}

#[test]
fn a_fixed_frame_renames_only_the_reads_before_the_temp_changes() {
    // A call between the reads may write `temp[0]`: only the first read is the value.
    let body = |head: &str, first: &str, close: &str| {
        format!(
            "        {head} {{\n            return true;\n        }}{close}\n        self.n = {first};\n        globals::idle_ticks(1)?;\n        sim.person[28] -= temp[0];"
        )
    };
    let source = fixed(
        LOCALS_0,
        &body("if !globals::take_elapsed_periods(20)", "temp[0]", ""),
    );
    let d = one(&source, "prefer_if_let");
    let help = d.help.as_deref().unwrap_or_default();
    assert!(
        help.contains("rename `temp[0]` to `periods` at 14:18 only"),
        "{help}"
    );
    let rewritten = fixed(
        LOCALS_0,
        &body(
            "let Some(periods) = globals::take_elapsed_periods(20) else",
            "periods",
            ";",
        ),
    );
    if_let_rewrite(
        &source,
        "if !globals::",
        "let Some(periods) = globals::take_elapsed_periods(20) else { .. };",
        &rewritten,
    );
}

// --- the byte checks build the text the linter read ------------------------------------------

#[test]
fn a_pack_file_is_checked_against_the_text_the_linter_read() {
    // The block is discarded code, so it keeps no layout: its native form builds alike, and
    // the pin would change the bytes (the backend pass then leaves `BODY` as lowered).
    let raw = "raw { remove_object(0, flags: 3); }";
    let source = tree(
        "",
        PROBE,
        "",
        &format!("{BODY}\n        if false {{\n            {raw}\n        }}"),
    );
    let pinned = rewrite(
        &source,
        raw,
        "remove_object(me, flags: 3);",
        PROBE,
        PROBE_PINNED,
    );
    assert_ne!(iff(&source), iff(&pinned), "the pin must change the build");
    // On disk the file is already the pinned rewrite; the linter was handed the original.
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("v2_5_lint")
        .join("snapshot");
    std::fs::create_dir_all(dir.join("objects")).unwrap();
    std::fs::write(
        dir.join("pack.toml"),
        "[pack]\nname = \"Probe\"\nversion = \"0.1.0\"\nauthor = \"x\"\nlicense = \"MIT\"\narchive = \"probe.far\"\nobjects = [\"probe\"]\n",
    )
    .unwrap();
    let path = dir.join("objects").join("probe.antics");
    std::fs::write(&path, &pinned).unwrap();
    let found: Vec<LintDiagnostic> =
        sims_antics::v2::lint::lint_v2_sources(&[sims_antics::v2::lint::LintInput {
            name: "probe.antics".to_owned(),
            source: source.clone(),
            path: Some(path),
            pack_dir: Some(dir),
        }])
        .unwrap()
        .into_iter()
        .filter(|d| d.rule == "manual_raw_has_builtin")
        .collect();
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(
        found[0].help.as_deref(),
        Some("use remove_object(me, flags: 3)"),
        "{found:#?}"
    );
}
