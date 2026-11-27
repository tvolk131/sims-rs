//! `.antics` v2.6 step 2A lint upgrades for #512: the linter checks a pack file's path with
//! `check_v2_file` and never reads it a second time, `manual_raw_has_builtin` spells raw
//! `local[N]` reads and writes natively (each finding byte-checked), `dead_nodes { .. }`
//! footers lint clean, and the `keep_layout` advice covers lifecycle trees.
//!
//! Every fixture here is synthetic (GUIDs in the clean-room 0x7E00_xxxx range). See
//! `docs/decisions/20261127-antics-v2-6-lint.md`.

use sims_antics::v2::PackResolverConfig;
use sims_antics::v2::lint::{LintDiagnostic, LintInput, lint_v2_source, lint_v2_sources};

const RULE: &str = "manual_raw_has_builtin";

/// A synthetic object holding `members`.
fn obj(members: &str) -> String {
    format!(
        "object \"Synth Probe\" {{\n    guid: 0x7E00_0760;\n    price: 1;\n    state n: i16;\n    state m: i16;\n{members}\n}}\n"
    )
}

/// Tree `probe` under `attr` running `body`.
fn tree(attr: &str, body: &str) -> String {
    obj(&format!("    {attr} fn probe() {{\n{body}\n    }}"))
}

/// Lifecycle tree `head` (its attributes and keyword) running `body`.
fn lifecycle(head: &str, body: &str) -> String {
    obj(&format!("    {head} {{\n{body}\n    }}"))
}

fn all(source: &str) -> Vec<LintDiagnostic> {
    lint_v2_source(source, "probe.antics").unwrap_or_else(|e| panic!("lint failed: {e}\n{source}"))
}

fn of(source: &str, rule: &str) -> Vec<LintDiagnostic> {
    all(source).into_iter().filter(|d| d.rule == rule).collect()
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

/// `source` with `raw` written as `native`, and the tree's header `attr` as `pinned_attr`.
fn rewrite(source: &str, raw: &str, native: &str, attr: &str, pinned_attr: &str) -> String {
    assert_eq!(source.matches(raw).count(), 1, "{raw:?}");
    assert_eq!(source.matches(attr).count(), 1, "{attr:?}");
    source
        .replacen(raw, native, 1)
        .replacen(attr, pinned_attr, 1)
}

/// A body the backend pass rewrites (it forwards `a` into `b`), so a kept layout shows.
const BODY: &str =
    "        let a = self.n;\n        let b = a + 1;\n        self.m = b;\n        self.n = 0;";

const PROBE: &str = "#[bhav(label = \"Probe\")]";
const PROBE_PINNED: &str = "#[bhav(label = \"Probe\", keep_layout)]";

const BHAV_PIN: &str = "add `keep_layout` to the tree's `#[bhav(..)]`";
const FRAME_PIN: &str = "add `keep_layout` to the tree's `#[frame(..)]`";
const NEW_FRAME_PIN: &str = "add `#[frame(keep_layout)]` to the tree";
const LOCAL_HELP: &str = "v2 reads and writes `local[N]` natively";

// --- the pack file is checked, never read a second time --------------------------------------

/// An anchor whose native form needs the pin (in `probe`), and a `local[N]` block (in
/// `other`): both findings stand only on a build.
const ANCHOR: &str = "raw { remove_object(0, flags: 3); }";
const LOCAL: &str = "raw { local[0] := 5; }";

fn two_trees() -> String {
    obj(&format!(
        "    {PROBE} fn probe() {{\n{BODY}\n        {ANCHOR}\n    }}\n    #[bhav] fn other() {{\n        {LOCAL}\n    }}"
    ))
}

/// `source` linted as file `objects/probe.antics` of a pack in `CARGO_TARGET_TMPDIR`, whose
/// disk copy `write` makes (or not); its `manual_raw_has_builtin` findings.
fn lint_in_pack(name: &str, source: &str, write: impl Fn(&std::path::Path)) -> Vec<LintDiagnostic> {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("v2_6_lint")
        .join(name);
    std::fs::create_dir_all(dir.join("objects")).unwrap();
    std::fs::write(
        dir.join("pack.toml"),
        "[pack]\nname = \"Probe\"\nversion = \"0.1.0\"\nauthor = \"x\"\nlicense = \"MIT\"\narchive = \"probe.far\"\nobjects = [\"probe\"]\n",
    )
    .unwrap();
    let path = dir.join("objects").join("probe.antics");
    write(&path);
    lint_v2_sources(&[LintInput {
        name: "probe.antics".to_owned(),
        source: source.to_owned(),
        path: Some(path),
        pack_dir: Some(dir),
    }])
    .unwrap()
    .into_iter()
    .filter(|d| d.rule == RULE)
    .collect()
}

#[test]
fn a_pack_file_that_stops_decoding_on_disk_keeps_its_byte_checked_findings() {
    let source = two_trees();
    iff(&source);
    // On disk the file no longer decodes: a second read of it fails, the text the linter
    // was handed does not.
    let found = lint_in_pack("undecodable", &source, |path| {
        std::fs::write(path, b"\xff\xfe not utf-8").unwrap();
        assert!(std::fs::read_to_string(path).is_err());
    });
    assert_eq!(found.len(), 2, "{found:#?}");
    let at = |raw: &str| {
        found
            .iter()
            .find(|d| (d.line, d.column) == position(&source, raw))
            .unwrap_or_else(|| panic!("no finding at {raw:?}: {found:#?}"))
    };
    let help = at(ANCHOR).help.clone().unwrap_or_default();
    assert!(help.contains(BHAV_PIN), "{help}");
    let help = at(LOCAL).help.clone().unwrap_or_default();
    assert!(help.contains(LOCAL_HELP), "{help}");
}

#[test]
fn a_pack_file_whose_path_fails_its_check_has_no_byte_checked_findings() {
    // No file at the path: `check` rejects it, so nothing is built and only the finding that
    // needs no build stands, without the pin advice.
    let source = two_trees();
    let found = lint_in_pack("missing", &source, |path| {
        assert!(!path.exists(), "{}", path.display());
    });
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!((found[0].line, found[0].column), position(&source, ANCHOR));
    assert_eq!(
        found[0].help.as_deref(),
        Some("use remove_object(me, flags: 3)")
    );
}

// --- manual_raw_has_builtin: `local[N]` reads and writes -------------------------------------

/// Asserts the block `raw` of `source` (a tree under `attr`) has one finding whose help
/// writes it as `native`, and that `native` builds the same bytes: as is, or with the pin
/// (as `pinned_attr`) when the help says the tree takes it, and then not without.
fn local_stands(source: &str, raw: &str, native: &str, attr: &str, pinned_attr: &str) {
    let d = one(source, RULE);
    assert_eq!((d.line, d.column), position(source, raw), "{d:#?}");
    let help = d.help.as_deref().unwrap_or_default();
    assert!(
        help.starts_with(&format!("write the block as `{native}`; ")),
        "{help}\n{source}"
    );
    assert!(help.contains(LOCAL_HELP), "{help}\n{source}");
    let original = iff(source);
    let plain = rewrite(source, raw, native, attr, attr);
    if !help.contains("keep_layout") {
        assert_eq!(iff(&plain), original, "{plain}");
        none(&plain, RULE);
        return;
    }
    assert_ne!(try_iff(&plain), Some(original.clone()), "{plain}");
    let pinned = rewrite(source, raw, native, attr, pinned_attr);
    assert_eq!(iff(&pinned), original, "{pinned}");
    none(&pinned, RULE);
}

/// Each raw `local[N]` line the rule spells, and its v2 statement.
const LOCAL_FORMS: &[(&str, &str)] = &[
    ("local[0] := 5", "local[0] = 5;"),
    ("local[0] := -3", "local[0] = -3;"),
    ("local[1] := local[0]", "local[1] = local[0];"),
    ("temp[0] := local[0]", "temp[0] = local[0];"),
    ("local[0] := temp[1]", "local[0] = temp[1];"),
    ("local[0] := stack_obj", "local[0] = stack_obj;"),
    ("stack_obj := local[0]", "stack_obj = local[0];"),
    ("local[0] += 2", "local[0] += 2;"),
    ("local[0] -= temp[1]", "local[0] -= temp[1];"),
    ("local[0] *= 3", "local[0] *= 3;"),
    ("local[0] /= 2", "local[0] /= 2;"),
    ("local[0] %= 3", "local[0] %= 3;"),
    ("local[0] &= 6", "local[0] &= 6;"),
    ("local[0] |= 1", "local[0] |= 1;"),
    ("local[0] ^= 4", "local[0] ^= 4;"),
    ("local[0] flag_set 2", "local[0] flag_set 2;"),
    (
        "local[0] flag_clear temp[1]",
        "local[0] flag_clear temp[1];",
    ),
    (
        "set_to_next(person, dest: local[0])",
        "set_to_next(person, dest: local[0]);",
    ),
    // `.air` reads `into:` as `dest:`, the only key v2 takes.
    (
        "set_to_next(all, into: local[0])",
        "set_to_next(all, dest: local[0]);",
    ),
];

#[test]
fn each_local_form_stands_on_a_build() {
    for (line, native) in LOCAL_FORMS {
        let raw = format!("raw {{ {line}; }}");
        // Alone, then after a body the backend pass rewrites.
        for body in [String::new(), format!("{BODY}\n")] {
            let source = tree(PROBE, &format!("{body}        {raw}"));
            local_stands(&source, &raw, native, PROBE, PROBE_PINNED);
        }
    }
}

#[test]
fn a_local_block_in_a_sim_s_body_stands_on_a_build() {
    // No header to pin: the native form is tried alone.
    let raw = "raw { stack_obj := local[0]; }";
    let source = obj(&format!(
        "    interaction \"Poke\" {{\n        run(sim) {{\n            self.n = 1;\n            {raw}\n            self.m = 2;\n        }}\n    }}"
    ));
    let help = one(&source, RULE).help.unwrap_or_default();
    assert!(help.contains("`stack_obj = local[0];`"), "{help}");
    assert!(!help.contains("keep_layout"), "{help}");
    assert_eq!(
        iff(&source.replacen(raw, "stack_obj = local[0];", 1)),
        iff(&source)
    );
}

#[test]
fn a_block_of_local_lines_and_builtins_stands_whole() {
    let raw = "raw { local[0] := 1; remove_object(0, flags: 3); temp[0] := local[0]; }";
    let native = "local[0] = 1; remove_object(me, flags: 3); temp[0] = local[0];";
    for body in ["", BODY] {
        let source = tree(PROBE, &format!("{body}\n        {raw}"));
        local_stands(&source, raw, native, PROBE, PROBE_PINNED);
    }
}

#[test]
fn the_help_writes_every_statement_of_the_block_in_order() {
    // A repeated statement stays: the block is rewritten whole, and dropping the second
    // `local[0] = 1;` would leave 3 for the read after it.
    let raw = "raw { local[0] := 1; local[0] += 2; local[0] := 1; }";
    let native = "local[0] = 1; local[0] += 2; local[0] = 1;";
    let source = tree(PROBE, &format!("        {raw}\n        self.n = local[0];"));
    local_stands(&source, raw, native, PROBE, PROBE_PINNED);
    let dropped = source.replacen(raw, "local[0] = 1; local[0] += 2;", 1);
    assert_ne!(try_iff(&dropped), Some(iff(&source)));
}

#[test]
fn a_local_block_in_a_lifecycle_tree_stands_on_a_build() {
    let raw = "raw { local[0] += 2; }";
    for head in ["main", "init", "#[frame(locals = 3)] cleanup"] {
        let source = lifecycle(head, &format!("{BODY}\n        {raw}"));
        let d = one(&source, RULE);
        let help = d.help.as_deref().unwrap_or_default();
        assert!(help.contains("`local[0] += 2;`"), "{help}");
        let plain = source.replacen(raw, "local[0] += 2;", 1);
        if !help.contains("keep_layout") {
            assert_eq!(iff(&plain), iff(&source), "{plain}");
        }
    }
}

#[test]
fn local_blocks_past_the_trial_budget_are_not_reported_and_their_allow_is_not_stale() {
    // Eight trials a file; the ninth block is left untried.
    let trees: Vec<String> = (0..9)
        .map(|k| {
            let allow = if k == 8 {
                "        // antics-lint: allow(manual_raw_has_builtin)\n"
            } else {
                ""
            };
            format!(
                "    #[bhav] fn probe{k}() {{\n{allow}        raw {{ local[0] := {k}; }}\n    }}"
            )
        })
        .collect();
    let source = obj(&trees.join("\n"));
    iff(&source);
    let found = all(&source);
    let raw = |k: usize| format!("raw {{ local[0] := {k}; }}");
    for k in 0..8 {
        assert!(
            found
                .iter()
                .any(|d| d.rule == RULE && (d.line, d.column) == position(&source, &raw(k))),
            "{k}: {found:#?}"
        );
    }
    assert_eq!(of(&source, RULE).len(), 8, "{found:#?}");
    none(&source, "stale_lint_allow");
}

#[test]
fn a_local_block_is_left_raw_where_the_rewrite_is_not_exact() {
    for raw in [
        // A comment: the rewrite would drop it.
        "raw {\n            local[0] := 5; // kept\n        }",
        // A cell the rule does not spell.
        "raw { local[0] := my.attr[0]; }",
        // A `param[N]` the tree's arguments do not cover.
        "raw { local[0] := param[3]; }",
        // A line no form spells beside a local line.
        "raw { local[0] := 5; idle(1); }",
    ] {
        let source = tree(PROBE, &format!("{BODY}\n        {raw}"));
        iff(&source);
        none(&source, RULE);
    }
}

// --- dead_nodes { .. } footers ---------------------------------------------------------------

#[test]
fn a_dead_nodes_footer_lints_clean() {
    const DEAD: &str = "        dead_nodes {\n            1: op 0x0002(0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04, 0x00) -> true: 2, false: error;\n            4: op 0x0002(0, 0, 0, 0, 0, 0, 0, 0) -> true: 251, false: false;\n        }";
    for (head, body) in [
        (
            "#[bhav(id = 4200)] fn probe()",
            "        self.n = 1;\n        if self.m == 2 {\n            self.n = 3;\n        }\n        self.m = 0;",
        ),
        ("#[bhav(id = 4200)] fn probe()", BODY),
        ("main", BODY),
    ] {
        let source = obj(&format!("    {head} {{\n{body}\n{DEAD}\n    }}"));
        iff(&source);
        let found = all(&source);
        assert!(found.is_empty(), "{source}\n{found:#?}");
    }
}

// --- keep_layout advice in lifecycle trees ---------------------------------------------------

/// Asserts the anchor `raw` of lifecycle tree `head` gets the pin advice `advice`, and that
/// its native form builds the same bytes with the header `pinned_head` and not without.
fn lifecycle_needs_pin(head: &str, pinned_head: &str, advice: &str) {
    let raw = "raw { remove_object(0, flags: 3); }";
    let native = "remove_object(me, flags: 3);";
    let source = lifecycle(head, &format!("{BODY}\n        {raw}"));
    let d = one(&source, RULE);
    assert_eq!((d.line, d.column), position(&source, raw), "{d:#?}");
    let help = d.help.as_deref().unwrap_or_default();
    assert!(help.contains(advice), "{help}\n{source}");
    let original = iff(&source);
    let plain = rewrite(&source, raw, native, head, head);
    assert_ne!(try_iff(&plain), Some(original.clone()), "{plain}");
    let pinned = rewrite(&source, raw, native, head, pinned_head);
    assert_eq!(iff(&pinned), original, "{pinned}");
    none(&pinned, RULE);
}

#[test]
fn a_lifecycle_tree_takes_the_pin() {
    for (head, pinned, advice) in [
        ("main", "#[frame(keep_layout)] main", NEW_FRAME_PIN),
        ("init", "#[frame(keep_layout)] init", NEW_FRAME_PIN),
        ("load", "#[frame(keep_layout)] load", NEW_FRAME_PIN),
        ("reset", "#[frame(keep_layout)] reset", NEW_FRAME_PIN),
        ("cleanup", "#[frame(keep_layout)] cleanup", NEW_FRAME_PIN),
        (
            "#[id(4100)] main",
            "#[id(4100)] #[frame(keep_layout)] main",
            NEW_FRAME_PIN,
        ),
        (
            "#[frame(locals = 2)] main",
            "#[frame(locals = 2, keep_layout)] main",
            FRAME_PIN,
        ),
        ("#[frame()] main", "#[frame(keep_layout)] main", FRAME_PIN),
        (
            "#[frame(locals = 2,)] #[id(4100)] main",
            "#[frame(locals = 2, keep_layout)] #[id(4100)] main",
            FRAME_PIN,
        ),
        (
            "#[bhav(locals = 2)] main",
            "#[bhav(locals = 2, keep_layout)] main",
            BHAV_PIN,
        ),
    ] {
        lifecycle_needs_pin(head, pinned, advice);
    }
}

#[test]
fn the_pin_goes_in_the_header_of_the_blocks_composed_tree() {
    // Two `init` blocks build one tree under one header: the pin goes in the block that has
    // the header (or before the first block), whichever block holds the anchor.
    let raw = "raw { remove_object(0, flags: 3); }";
    let native = "remove_object(me, flags: 3);";
    for (first, pinned, advice) in [
        (
            "#[frame(locals = 4)] init",
            "#[frame(locals = 4, keep_layout)] init",
            FRAME_PIN,
        ),
        ("init", "#[frame(keep_layout)] init", NEW_FRAME_PIN),
    ] {
        let source = obj(&format!(
            "    {first} {{\n{BODY}\n    }}\n    init {{\n{BODY}\n        {raw}\n    }}"
        ));
        let d = one(&source, RULE);
        let help = d.help.as_deref().unwrap_or_default();
        let line = position(&source, first).0;
        assert!(
            help.contains(&format!("{advice} on line {line}")),
            "{help}\n{source}"
        );
        let original = iff(&source);
        let at = source.find(first).unwrap();
        let rewritten = |head: &str| {
            let s = format!("{}{head}{}", &source[..at], &source[at + first.len()..]);
            s.replacen(raw, native, 1)
        };
        assert_ne!(try_iff(&rewritten(first)), Some(original.clone()));
        assert_eq!(iff(&rewritten(pinned)), original);
        none(&rewritten(pinned), RULE);
    }
}

#[test]
fn a_dead_nodes_footer_is_no_header_for_the_pin() {
    // The first `init` ends in a footer; the header is the second block's.
    let raw = "raw { remove_object(0, flags: 3); }";
    let dead = "dead_nodes { 1: op 2(0, 0, 0, 0, 0, 0, 0, 0) -> true: true, false: true; }";
    let head = "#[frame(locals = 4)] init";
    let source = obj(&format!(
        "    init {{\n{BODY}\n        {dead}\n    }}\n    {head} {{\n{BODY}\n        {raw}\n    }}"
    ));
    let original = iff(&source);
    let help = one(&source, RULE).help.unwrap_or_default();
    let line = position(&source, head).0;
    assert!(
        help.contains(&format!("{FRAME_PIN} on line {line}")),
        "{help}\n{source}"
    );
    let pinned = rewrite(
        &source,
        raw,
        "remove_object(me, flags: 3);",
        head,
        "#[frame(locals = 4, keep_layout)] init",
    );
    assert_eq!(iff(&pinned), original, "{pinned}");
}

#[test]
fn a_replaced_main_is_not_tried() {
    // Lowering keeps only the last `main`: a rewrite of the first builds the same bytes
    // whatever it says, so it proves nothing.
    let source = obj(&format!(
        "    main {{\n        raw {{ local[0] := 5; }}\n    }}\n    main {{\n{BODY}\n    }}"
    ));
    iff(&source);
    none(&source, RULE);
    // The last one is tried.
    let source = obj(&format!(
        "    main {{\n{BODY}\n    }}\n    main {{\n        raw {{ local[0] := 5; }}\n    }}"
    ));
    let help = one(&source, RULE).help.unwrap_or_default();
    assert!(help.contains("`local[0] = 5;`"), "{help}");
}

#[test]
fn a_pinned_lifecycle_tree_gets_no_advice() {
    let raw = "raw { remove_object(0, flags: 3); }";
    for head in [
        "#[frame(keep_layout)] main",
        "#[bhav(locals = 2, keep_layout)] init",
    ] {
        let source = lifecycle(head, &format!("{BODY}\n        {raw}"));
        iff(&source);
        let d = one(&source, RULE);
        let help = d.help.as_deref().unwrap_or_default();
        assert!(!help.contains("keep_layout"), "{help}\n{source}");
    }
}
