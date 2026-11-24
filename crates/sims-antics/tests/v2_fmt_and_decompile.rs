//! `.antics` v2 formatter (`sims-antics fmt`) and decompiler (`sims-antics lift --v2`).
//!
//! Every object here is synthetic (GUIDs in the clean-room 0x7E00_xxxx range); the
//! formatter is additionally exercised on the clean-room `open-pack` sources.

use sims_antics::v2::{
    DecompileV2Options, PackResolverConfig, TreeCheck, TreeStyle, decompile_iff_to_v2,
    decompile_iff_to_v2_report, format_v2_source, is_v2_formatted,
};
use sims_antics::{CompiledObject, compile_v2_source, expand_v2_to_air};
use sims_formats::iff::Iff;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A synthetic object that uses the idioms the decompiler recognises.
const LAMP: &str = r#"object "Synth Lamp" {
    guid: 0x7E00_0001;
    price: 10;
    category: lighting;
    description: "synthetic";
    sprite: Sprite::procedural("floor_lamp", primary: #F2E0AE, accent: #FCF0CE, trim: #745E3E);

    param boost: i16 = 35;
    state power: bool = false;
    state level @ 5: i16;
    sound click = preset("ui_click");

    bcon 4097 "Tuning" { cheap = 10, bright = 25 }

    @id(4200) tree helper "Helper Tree" (sim: Sim) {
        loop {
            if self.level > Tuning.bright { break; }
            self.level += 1;
        }
        for p in lot.sims() { temp[0] += 1; }
        animate(clip: 3, table: 1) |event| { temp[1] = event; }
        if !can_afford(5) { return false; }
        debit_funds(5);
        play_sound(click);
        random_into(temp[2], 10);
    }

    main {
        loop {
            sleep(30);
            if self.power { self.level += 2; }
        }
    }

    interaction "Turn On" {
        autonomy: 40,
        advertises: [room +40],
        guard(sim) => !self.power,
        run(sim) {
            self.power = true;
            sim.motive.room += boost;
            helper(sim);
        }
    }
}
"#;

/// A second synthetic object: object-context trees, a counted loop, a tree with
/// arguments and locals, and a tree called from another tree.
const COUNTER: &str = r#"object "Synth Counter" {
    guid: 0x7E00_0002;
    price: 25;
    sprite: Sprite::procedural("floor_lamp", primary: #6A5A48, accent: #CFC2A8, trim: #3A2E22);

    state count @ 3: i16;
    state limit @ 4: i16;

    bcon 4100 "Limits" { low = 2, high = 9 }

    @id(4300) tree bump "Bump" {
        if self.count < Limits.high {
            self.count += 1;
        } else {
            self.count = Limits.low;
        }
        return true;
    }

    @id(4302) tree census "Census" {
        temp[0] = 0;
        for o in lot.objects() {
            temp[0] += 1;
        }
        return true;
    }

    @id(4301) tree drain "Drain" {
        while self.count > 0 {
            self.count -= 1;
            bump();
        }
        return self.limit == 0;
    }

    init {
        self.count = 0;
        self.limit = Limits.high;
    }

    main {
        loop {
            sleep(10);
            bump();
            if self.count == Limits.high { drain(); }
        }
    }
}
"#;

fn compile(name: &str, source: &str) -> CompiledObject {
    compile_v2_source(name, source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{name} does not compile: {e}"))
}

fn parse_iff(bytes: &[u8]) -> Iff {
    Iff::parse(bytes.to_vec()).expect("compiled IFF parses")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("v2_fmt_and_decompile")
        .join(name);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sims-antics"))
}

// ---------------------------------------------------------------------------------------
// Decompiler
// ---------------------------------------------------------------------------------------

/// compile → IFF → decompile → format (idempotent) → recompile, with every private tree
/// proven against the original and no counterexample anywhere.
fn assert_round_trip(name: &str, source: &str) -> String {
    let original = compile(name, source);
    let iff = parse_iff(&original.iff_bytes);
    let report = decompile_iff_to_v2_report(&iff, &DecompileV2Options::default())
        .unwrap_or_else(|e| panic!("{name}: decompile failed: {e}"));
    assert!(report.compiled, "{name}: validation did not run");
    assert_eq!(report.counterexamples(), 0, "{name}: counterexamples");
    assert!(!report.trees.is_empty(), "{name}: no trees decompiled");
    for t in &report.trees {
        assert!(
            matches!(t.check, TreeCheck::Identical | TreeCheck::Equivalent),
            "{name}: tree {} is not proven: {:?}\n{}",
            t.id,
            t.check,
            report.source
        );
    }

    // The decompiled text is canonical: formatting it again changes nothing.
    assert!(is_v2_formatted(&report.source).expect("decompiled source parses"));
    assert_eq!(
        format_v2_source(&report.source).expect("format"),
        report.source
    );

    // It recompiles on its own, and every original private tree is back under its ID.
    let recompiled = compile("decompiled.antics", &report.source);
    for t in &report.trees {
        assert!(
            recompiled.behaviors.contains_key(&t.id),
            "{name}: tree {} missing after recompiling",
            t.id
        );
    }

    // The convenience wrapper returns the same text.
    assert_eq!(
        decompile_iff_to_v2(&iff, &DecompileV2Options::default()).expect("decompile"),
        report.source
    );
    report.source
}

#[test]
fn synthetic_lamp_round_trips_with_idioms() {
    let source = assert_round_trip("synth-lamp.antics", LAMP);
    for idiom in [
        "@id(4096) init {",
        "@id(4097) main {",
        "loop {",
        "if self.attr_0 != 0 {",
        "animate(clip: 3, table: 1, flags: 0, events: 0) |event| {",
        "temp[1] = event;",
        "if !can_afford(5, category: 0) {",
        "debit_funds(5, category: 0);",
        "random_into(temp[2], 10);",
        "tree can_turn_on_4098 \"Can_TurnOn\" (sim: Sim) {",
        "sim.motive[13] += SynthLampTuning.e0;",
        "guid: 0x7E00_0001;",
        "bcon 4097 \"Tuning\" {",
    ] {
        assert!(source.contains(idiom), "missing `{idiom}` in:\n{source}");
    }
    // The interaction table is kept as a comment naming its trees.
    assert!(source.contains("\"Turn On\""), "{source}");
}

#[test]
fn synthetic_counter_round_trips() {
    let source = assert_round_trip("synth-counter.antics", COUNTER);
    assert!(source.contains("while self.attr_3 > 0"), "{source}");
    assert!(source.contains("state attr_3 @ 3: i16;"), "{source}");
    assert!(source.contains("Limits.e1"), "{source}");
    assert!(source.contains("for o in lot.objects() {"), "{source}");
    assert!(source.contains("@id(4096) init {"), "{source}");
}

#[test]
fn validation_off_still_writes_formatted_source() {
    let original = compile("synth-lamp.antics", LAMP);
    let iff = parse_iff(&original.iff_bytes);
    let options = DecompileV2Options {
        validate: false,
        ..DecompileV2Options::default()
    };
    let report = decompile_iff_to_v2_report(&iff, &options).expect("decompile");
    assert!(!report.compiled);
    assert!(
        report
            .trees
            .iter()
            .all(|t| t.check == TreeCheck::NotChecked && t.style == TreeStyle::Idiomatic)
    );
    assert!(is_v2_formatted(&report.source).expect("parses"));
}

// ---------------------------------------------------------------------------------------
// Formatter
// ---------------------------------------------------------------------------------------

const MESSY: &str = r#"/// Doc comment on the object.
object   "Synth Messy"{guid:0x7E00_0003;price:5;
sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);
    // A line comment before a state.
  state  n @ 3 : i16; /* trailing block /* nested */ comment */
    bcon 4097 "K" { a = 1, b = 2 }
    @id(4200) tree t "T" {
        if self.n>K.a{self.n-=1;}else{self.n=-K.b;}   // trailing line comment
        raw {
              temp[0] := 1;
                temp[1] := 2;
        }
        return true;
    }
}
"#;

#[test]
fn formatter_is_idempotent_and_keeps_comments() {
    let once = format_v2_source(MESSY).expect("format");
    assert_ne!(once, MESSY);
    assert_eq!(format_v2_source(&once).expect("format twice"), once);
    assert!(is_v2_formatted(&once).expect("parses"));
    assert!(!is_v2_formatted(MESSY).expect("parses"));
    for comment in [
        "/// Doc comment on the object.",
        "// A line comment before a state.",
        "/* trailing block /* nested */ comment */",
        "// trailing line comment",
    ] {
        assert!(once.contains(comment), "lost `{comment}`:\n{once}");
    }
    assert!(once.contains("if self.n > K.a {"), "{once}");
    assert!(once.contains("self.n = -K.b;"), "{once}");
    // A raw body keeps its relative indentation.
    assert!(
        once.contains(
            "        raw {\n            temp[0] := 1;\n              temp[1] := 2;\n        }"
        ),
        "{once}"
    );
    // Formatting never changes what the object compiles to.
    assert_eq!(
        compile("messy.antics", MESSY).iff_bytes,
        compile("messy.antics", &once).iff_bytes
    );
}

#[test]
fn formatter_refuses_invalid_source() {
    assert!(format_v2_source("object \"X\" { guid: ; }").is_err());
}

// Both listings are sorted before use.
#[allow(clippy::disallowed_methods)]
fn copy_formatted(from: &Path, to: &Path, count: &mut usize) {
    std::fs::create_dir_all(to).expect("create dir");
    let mut entries: Vec<_> = std::fs::read_dir(from)
        .expect("read dir")
        .map(|e| e.expect("dir entry").path())
        .collect();
    entries.sort();
    for path in entries {
        let target = to.join(path.file_name().expect("file name"));
        if path.is_dir() {
            copy_formatted(&path, &target, count);
        } else if path.extension().is_some_and(|e| e == "antics") {
            let source = std::fs::read_to_string(&path).expect("read source");
            let formatted =
                format_v2_source(&source).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert_eq!(
                format_v2_source(&formatted).expect("format twice"),
                formatted,
                "{} is not idempotent",
                path.display()
            );
            std::fs::write(&target, formatted).expect("write formatted copy");
            *count += 1;
        } else {
            std::fs::copy(&path, &target).expect("copy file");
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)]
fn formatting_open_pack_keeps_expanded_air_identical() {
    let pack = workspace_root().join("open-pack");
    let copy = scratch("open-pack-formatted");
    let mut count = 0;
    copy_formatted(&pack, &copy, &mut count);
    assert!(count > 0, "no .antics files under open-pack");

    let mut objects: Vec<PathBuf> = std::fs::read_dir(pack.join("objects"))
        .expect("read objects")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "antics"))
        .collect();
    objects.sort();
    assert!(!objects.is_empty());
    for original in &objects {
        let formatted = copy
            .join("objects")
            .join(original.file_name().expect("name"));
        let before = expand_v2_to_air(original, Some(&pack))
            .unwrap_or_else(|e| panic!("{}: {e}", original.display()));
        let after = expand_v2_to_air(&formatted, Some(&copy))
            .unwrap_or_else(|e| panic!("{}: {e}", formatted.display()));
        assert_eq!(before, after, "{} expands differently", original.display());
    }
}

// ---------------------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------------------

#[test]
fn cli_fmt_check_rewrite_and_stdout() {
    let dir = scratch("cli-fmt");
    let tidy = dir.join("tidy.antics");
    let messy = dir.join("messy.antics");
    std::fs::write(&tidy, format_v2_source(MESSY).expect("format")).expect("write");
    std::fs::write(&messy, MESSY).expect("write");

    let out = cli()
        .arg("fmt")
        .arg("--check")
        .arg(&dir)
        .output()
        .expect("run fmt");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let listed = String::from_utf8_lossy(&out.stdout);
    assert!(listed.contains("messy.antics"), "{listed}");
    assert!(!listed.contains("tidy.antics"), "{listed}");

    let out = cli()
        .arg("fmt")
        .arg("--stdout")
        .arg(&messy)
        .output()
        .expect("run fmt");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8(out.stdout).expect("utf-8"),
        format_v2_source(MESSY).expect("format")
    );
    assert_eq!(std::fs::read_to_string(&messy).expect("read"), MESSY);

    let out = cli().arg("fmt").arg(&dir).output().expect("run fmt");
    assert!(out.status.success(), "{out:?}");
    let out = cli()
        .arg("fmt")
        .arg("--check")
        .arg(&dir)
        .output()
        .expect("run fmt");
    assert!(out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty());

    let out = cli()
        .args(["fmt", "--check", "--stdout"])
        .arg(&messy)
        .output()
        .expect("run fmt");
    assert!(!out.status.success());
}

#[test]
fn cli_lift_v2_prints_compiling_source() {
    let dir = scratch("cli-lift");
    let iff = dir.join("synth_lamp.iff");
    std::fs::write(&iff, compile("synth-lamp.antics", LAMP).iff_bytes).expect("write IFF");

    let out = cli()
        .args(["lift", "--v2", "--no-catalog"])
        .arg(&iff)
        .output()
        .expect("run lift --v2");
    assert!(out.status.success(), "{out:?}");
    let source = String::from_utf8(out.stdout).expect("utf-8");
    let summary = String::from_utf8_lossy(&out.stderr);
    assert!(summary.starts_with("decompiled "), "{summary}");
    assert!(summary.contains(" 0 different"), "{summary}");
    assert!(is_v2_formatted(&source).expect("parses"));
    compile("lifted.antics", &source);

    // The default `.air` lift is unchanged by the flag's existence.
    let air = cli()
        .args(["lift", "--no-catalog"])
        .arg(&iff)
        .output()
        .expect("run lift");
    assert!(air.status.success(), "{air:?}");
    assert!(String::from_utf8_lossy(&air.stdout).contains("tree 4200 "));

    for conflict in ["--raw", "--list", "--no-comments"] {
        let out = cli()
            .args(["lift", "--v2", conflict])
            .arg(&iff)
            .output()
            .expect("run lift");
        assert!(!out.status.success(), "--v2 {conflict} was accepted");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("--v2"),
            "{out:?}"
        );
    }
}
