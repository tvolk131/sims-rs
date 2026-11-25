//! `.antics` v2 formatter (`sims-antics fmt`) and decompiler (`sims-antics lift --v2`).
//!
//! Every object here is synthetic (GUIDs in the clean-room 0x7E00_xxxx range); the
//! formatter is additionally exercised on the clean-room `open-pack` sources.

use sims_antics::encode::{encode_bcon, encode_bhav};
use sims_antics::ir::{BconChunk, BhavChunk};
use sims_antics::v2::{
    DecompileV2Options, PackResolverConfig, TreeCheck, TreeStyle, decompile_iff_to_v2,
    decompile_iff_to_v2_report, format_v2_source, is_v2_formatted,
};
use sims_antics::{CompiledObject, compile_v2_source, expand_v2_to_air};
use sims_formats::behavior::Instruction;
use sims_formats::iff::{ChunkData, Iff};
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
    #[slot(5)] state level: i16;
    sound click = preset("ui_click");

    bcon 4097 "Tuning" { cheap = 10, bright = 25 }

    #[bhav(id = 4200, label = "Helper Tree")]
    fn helper(sim: Sim) {
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

    #[slot(3)] state count: i16;
    #[slot(4)] state limit: i16;

    bcon 4100 "Limits" { low = 2, high = 9 }

    #[bhav(id = 4300, label = "Bump")]
    fn bump() {
        if self.count < Limits.high {
            self.count += 1;
        } else {
            self.count = Limits.low;
        }
        return true;
    }

    #[bhav(id = 4302, label = "Census")]
    fn census() {
        temp[0] = 0;
        for o in lot.objects() {
            temp[0] += 1;
        }
        return true;
    }

    #[bhav(id = 4301, label = "Drain")]
    fn drain() {
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
        // A lifecycle block takes a header only when the default one does not do: `main`'s
        // `sleep(30)` stages its ticks in `param[0]`, which the default header gives it.
        "#[id(4096)]\n    init {",
        "#[id(4097)]\n    main {",
        "sleep(30);",
        "loop {",
        "if self.attr_0 != 0 {",
        "animate(clip: 3, table: 1, flags: 0, events: 0) |event| {",
        "temp[1] = event;",
        "if !can_afford(5, category: 0) {",
        "debit_funds(5, category: 0);",
        "random_into(temp[2], 10);",
        "label = \"Can_TurnOn\"",
        "fn can_turn_on_4098(sim: Sim) {",
        "sim.motive[13] += SynthLampTuning.e0;",
        "guid: 0x7E00_0001;",
        "bcon 4097 \"Tuning\" {",
        // The FWAV keeps its ID and event name.
        "= fwav(1, wav: ",
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
    assert!(source.contains("#[slot(3)] state attr_3: i16;"), "{source}");
    assert!(source.contains("Limits.e1"), "{source}");
    // The census loop's cursor is `stack_obj` itself (no loop local) since v2 PR F.
    assert!(source.contains("for o in lot.objects() {"), "{source}");
    assert!(!source.contains("set_to_next"), "{source}");
    assert!(source.contains("#[id(4096)]\n    init {"), "{source}");
}

/// v2 PR F shapes: `for` loops whose cursor is `stack_obj` itself, sound ID 0, two FWAVs
/// naming one event, and a tree frame's kind byte.
const CURSOR: &str = r#"object "Synth Cursor" {
    guid: 0x7E00_0003;
    price: 25;
    sprite: Sprite::procedural("floor_lamp", primary: #6A5A48, accent: #CFC2A8, trim: #3A2E22);

    sound chime = preset("ui_click", wav: "synth_chime", id: 0);
    sound chime_again = preset("ui_click", wav: "synth_chime", id: 7);

    #[bhav(id = 4302, label = "Census")]
    fn census() {
        temp[0] = 0;
        for o in lot.objects() {
            if o.data[3] > 0 {
                temp[0] += o.attr[1];
            }
        }
        return true;
    }

    #[bhav(id = 4303, label = "People")]
    fn people() {
        temp[1] = 0;
        for p in lot.sims() {
            if p.kind == 0 {
                temp[1] += p.person[5];
            }
        }
        return true;
    }

    #[bhav(id = 4304, label = "Kinded", kind = 3)]
    fn kinded() {
        play_sound(chime);
        play_sound(chime_again);
        return true;
    }

    #[bhav(id = 4305, label = "Chime For")]
    fn chime_for(sim: Sim) {
        sim.motive[13] += 1;
        play_sound(chime);
        play_sound(chime, sim);
        return true;
    }

    main {
        loop {
            sleep(10);
            census();
            people();
            kinded();
        }
    }
}
"#;

#[test]
fn zero_local_loops_sounds_and_frame_kinds_round_trip() {
    let source = assert_round_trip("synth-cursor.antics", CURSOR);
    for idiom in [
        "for o in lot.objects() {",
        "if o.data[3] > 0 {",
        "temp[0] += o.attr[1];",
        "for p in lot.sims() {",
        "if p.kind == 0 {",
        "temp[1] += p.personality.playful;",
        "#[bhav(id = 4304, label = \"Kinded\", args = 0, locals = 0, format = 0x8002, revision = 0, kind = 3)]\n    fn kinded_4304() {",
        "sound sound_0 = fwav(0, wav: \"synth_chime\");",
        "sound sound_7 = fwav(7, wav: \"synth_chime\");",
        "play_sound(sound_0);",
        "play_sound(sound_7);",
        // In an interaction the default target is the object, `sim` the other one.
        "        play_sound(sound_0);\n        play_sound(sound_0, sim);\n",
    ] {
        assert!(source.contains(idiom), "missing `{idiom}` in:\n{source}");
    }
    assert!(!source.contains("set_to_next"), "{source}");
    // Trees without a kind keep the plain frame.
    assert!(
        source.contains("#[bhav(id = 4303, label = \"People\", args = 0, locals = 0, format = 0x8002, revision = 0)]\n"),
        "{source}"
    );
    // `census` prefers the Sim context (it reads the stack object's attributes) but `main`
    // calls it: it is written for either context.
    assert!(
        source.contains(
            "#[bhav(id = 4302, label = \"Census\", args = 0, locals = 0, format = 0x8002, revision = 0, context = inherit)]\n"
        ),
        "{source}"
    );
}

const PR_F_SUGAR: &str = r#"enum Mode { Off, Low, High }
object "Synth Picker" {
    guid: 0x7E00_0007; price: 40;
    sprite: Sprite::procedural("floor_lamp", primary: #504030, accent: #A09080, trim: #201810);
    state mode: Mode;
    #[slot(2)] state level: i16;
    #[bhav(id = 4200, label = "Pick", kind = 3)]
    fn pick() {
        self.level = if self.level>3 {1} else if self.level > 1 { 2 }   else {3};
        self.level = match self.mode { Mode::Off|Mode::Low => 1,
            Mode::High => 2 };
        match self.mode {
            Mode::Off |   Mode::Low => { self.level += 1; }
            _ => {}
        }
        return true;
    }
}
"#;

#[test]
fn formatter_handles_if_expressions_or_patterns_and_frame_kinds() {
    let formatted = format_v2_source(PR_F_SUGAR).expect("format");
    assert!(is_v2_formatted(&formatted).expect("check"), "{formatted}");
    assert_eq!(
        format_v2_source(&formatted).expect("reformat"),
        formatted,
        "idempotent"
    );
    for expected in [
        "    #[bhav(id = 4200, label = \"Pick\", kind = 3)]\n    fn pick() {\n",
        "        self.level = if self.level > 3 {\n            1\n        } else if self.level > 1 {\n            2\n        } else {\n            3\n        };\n",
        "            Mode::Off | Mode::Low => 1,\n",
        "            Mode::Off | Mode::Low => {\n                self.level += 1;\n            }\n",
    ] {
        assert!(
            formatted.contains(expected),
            "missing {expected:?} in\n{formatted}"
        );
    }
    // Formatting changes no compiled byte.
    assert_eq!(
        compile("picker.antics", PR_F_SUGAR).iff_bytes,
        compile("picker.antics", &formatted).iff_bytes
    );
}

/// Re-encodes `iff` with `extra` chunks appended after its own.
fn with_chunks(iff: &Iff, extra: Vec<ChunkData>) -> Iff {
    let mut chunks: Vec<ChunkData> = iff
        .chunks
        .iter()
        .filter(|c| &c.tag != b"rsmp")
        .map(|c| ChunkData {
            tag: c.tag,
            id: c.id,
            flags: c.flags,
            label: c.label.clone(),
            data: iff.data(c).to_vec(),
        })
        .collect();
    chunks.extend(extra);
    parse_iff(&sims_formats::iff::write(&chunks).expect("write IFF"))
}

fn bcon(id: u16, label: &str, values: Vec<i16>) -> ChunkData {
    let chunk = BconChunk {
        id,
        label: label.to_owned(),
        flags: 0,
        values,
        trailing: Vec::new(),
    };
    ChunkData {
        tag: *b"BCON",
        id,
        flags: 0x10,
        label: label.to_owned(),
        data: encode_bcon(&chunk).expect("encode BCON"),
    }
}

#[test]
fn constant_tables_follow_the_vm() {
    let iff = parse_iff(&compile("synth-counter.antics", COUNTER).iff_bytes);
    let iff = with_chunks(
        &iff,
        vec![
            // A later chunk of an ID replaces the earlier one, as in the VM.
            bcon(4100, "Limits", vec![3, 7]),
            // Labels whose fallback names collide.
            bcon(4101, "Limits4102", vec![1]),
            bcon(4102, "Limits", vec![2]),
            // Only the first 128 entries are addressable.
            bcon(4103, "Wide", (0..130).collect()),
        ],
    );
    let report =
        decompile_iff_to_v2_report(&iff, &DecompileV2Options::default()).expect("decompile");
    let source = &report.source;
    assert_eq!(report.counterexamples(), 0, "{source}");
    assert!(
        report
            .trees
            .iter()
            .all(|t| matches!(t.check, TreeCheck::Identical | TreeCheck::Equivalent)),
        "{:?}\n{source}",
        report.trees
    );
    assert!(
        source.contains("bcon 4100 \"Limits\" {\n        e0 = 3,\n        e1 = 7,"),
        "{source}"
    );
    assert!(source.contains("bcon 4101 \"Limits4102\""), "{source}");
    assert!(source.contains("bcon 4102 \"Limits4102_2\""), "{source}");
    assert!(
        source.contains("e127 = 127,") && !source.contains("e128"),
        "{source}"
    );
}

#[test]
fn a_tree_v2_cannot_write_is_omitted_not_fatal() {
    let iff = parse_iff(&compile("synth-counter.antics", COUNTER).iff_bytes);
    // 251 nodes, each calling a private tree with the caller's temps: the most a tree
    // holds, so the `return true;` v2 adds after a raw body does not fit.
    let instructions = (0..251u16)
        .map(|i| Instruction {
            opcode: 4300,
            on_true: if i == 250 { 0xfe } else { (i + 1) as u8 },
            on_false: 0xfd,
            operands: [0xff; 8],
        })
        .collect();
    let mut long = BhavChunk::new(4400, "Long", instructions);
    long.format = 0x8002;
    let iff = with_chunks(
        &iff,
        vec![ChunkData {
            tag: *b"BHAV",
            id: 4400,
            flags: 0x10,
            label: "Long".to_owned(),
            data: encode_bhav(&long).expect("encode BHAV"),
        }],
    );
    let report =
        decompile_iff_to_v2_report(&iff, &DecompileV2Options::default()).expect("decompile");
    assert_eq!(report.counterexamples(), 0);
    let long = report
        .trees
        .iter()
        .find(|t| t.id == 4400)
        .expect("tree 4400");
    assert!(
        matches!(&long.check, TreeCheck::Omitted(why) if why.contains("251 instructions")),
        "{:?}",
        long.check
    );
    assert!(
        report
            .source
            .contains("// tree 4400 \"Long\" is not written")
    );
    compile("decompiled.antics", &report.source);
}

/// A synthetic object whose trees touch attributes past its attribute count.
const SPARSE: &str = r#"object "Synth Sparse" {
    guid: 0x7E00_0003;
    price: 5;
    attributes: 4;
    sprite: Sprite::procedural("floor_lamp", primary: #504030, accent: #A09080, trim: #201810);

    #[slot(2)] state level: i16;

    #[bhav(id = 4500, label = "Poke")]
    fn poke() {
        self.level += 1;
        raw { my.attr[20] := 1; }
        return true;
    }
}
"#;

fn proven(check: &TreeCheck) -> bool {
    matches!(check, TreeCheck::Identical | TreeCheck::Equivalent)
}

#[test]
fn the_attribute_count_is_kept_and_bounds_states() {
    let iff = parse_iff(&compile("synth-sparse.antics", SPARSE).iff_bytes);
    let report =
        decompile_iff_to_v2_report(&iff, &DecompileV2Options::default()).expect("decompile");
    let source = &report.source;
    assert!(source.contains("attributes: 4;"), "{source}");
    // A `state` past the count would grow the object, so attribute 20 stays `raw`.
    assert!(source.contains("#[slot(2)] state attr_2: i16;"), "{source}");
    assert!(!source.contains("attr_20"), "{source}");
    assert!(
        report.trees.iter().all(|t| proven(&t.check)),
        "{:?}\n{source}",
        report.trees
    );
}

fn bhav(id: u16, label: &str, instructions: Vec<Instruction>) -> ChunkData {
    let mut tree = BhavChunk::new(id, label, instructions);
    tree.format = 0x8002;
    ChunkData {
        tag: *b"BHAV",
        id,
        flags: 0x10,
        label: label.to_owned(),
        data: encode_bhav(&tree).expect("encode BHAV"),
    }
}

fn node(opcode: u16, operands: [u8; 8]) -> Instruction {
    Instruction {
        opcode,
        on_true: 0xfe,
        on_false: 0xfd,
        operands,
    }
}

#[test]
fn a_sound_v2_cannot_label_leaves_its_player_inconclusive() {
    let iff = parse_iff(&compile("synth-counter.antics", COUNTER).iff_bytes);
    let long_name = "s".repeat(70);
    let mut fwav = long_name.clone().into_bytes();
    fwav.push(0);
    let iff = with_chunks(
        &iff,
        vec![
            ChunkData {
                tag: *b"FWAV",
                id: 5,
                flags: 0x10,
                label: "Long".to_owned(),
                data: fwav,
            },
            // Play Sound Event 5.
            bhav(4410, "Chime", vec![node(23, [5, 0, 0, 0, 0, 0, 0, 0])]),
        ],
    );
    let report =
        decompile_iff_to_v2_report(&iff, &DecompileV2Options::default()).expect("decompile");
    assert!(!report.source.contains(&long_name), "{}", report.source);
    let chime = report
        .trees
        .iter()
        .find(|t| t.id == 4410)
        .expect("tree 4410");
    assert!(
        matches!(&chime.check, TreeCheck::Inconclusive(why) if why.contains("chunk label")),
        "{:?}",
        chime.check
    );
}

#[test]
fn constant_table_names_stay_distinct_for_the_compiler() {
    let iff = parse_iff(&compile("synth-counter.antics", COUNTER).iff_bytes);
    let iff = with_chunks(
        &iff,
        vec![
            // Same name as `Limits` up to case and underscores.
            bcon(4104, "LI_MITS", vec![1]),
            bcon(4105, "Library_Consts", vec![2]),
            // A label at the length limit still has room for its suffix.
            bcon(4106, &"Long".repeat(15), vec![3]),
            bcon(4107, &"Long".repeat(15), vec![4]),
        ],
    );
    let report =
        decompile_iff_to_v2_report(&iff, &DecompileV2Options::default()).expect("decompile");
    let source = &report.source;
    assert!(source.contains("bcon 4104 \"LIMITS4104\""), "{source}");
    assert!(
        source.contains("bcon 4105 \"LibraryConsts4105\""),
        "{source}"
    );
    let long = "Long".repeat(15);
    assert!(
        source.contains(&format!("bcon 4106 \"{long}\"")),
        "{source}"
    );
    assert!(
        source.contains(&format!("bcon 4107 \"{}4107\"", &long[..51])),
        "{source}"
    );
    assert!(
        report.trees.iter().all(|t| proven(&t.check)),
        "{:?}\n{source}",
        report.trees
    );
    compile("decompiled.antics", source);
}

#[test]
fn a_caller_of_an_unproven_tree_is_not_proven() {
    let iff = parse_iff(&compile("synth-counter.antics", COUNTER).iff_bytes);
    // 4400 cannot be written (251 nodes); 4401 calls it, 4402 calls 4401.
    let long = (0..251u16)
        .map(|i| Instruction {
            on_true: if i == 250 { 0xfe } else { (i + 1) as u8 },
            ..node(4300, [0xff; 8])
        })
        .collect();
    let iff = with_chunks(
        &iff,
        vec![
            bhav(4400, "Long", long),
            bhav(4401, "Caller", vec![node(4400, [0xff; 8])]),
            bhav(4402, "Outer", vec![node(4401, [0xff; 8])]),
        ],
    );
    let report =
        decompile_iff_to_v2_report(&iff, &DecompileV2Options::default()).expect("decompile");
    let check = |id| {
        &report
            .trees
            .iter()
            .find(|t| t.id == id)
            .expect("tree")
            .check
    };
    assert!(matches!(check(4400), TreeCheck::Omitted(_)));
    // The checker itself sees that 4400 changed shape; 4402's own code is the same.
    assert!(!proven(check(4401)), "{:?}", check(4401));
    assert!(
        matches!(check(4402), TreeCheck::Inconclusive(why) if why.contains("calls tree 4401")),
        "{:?}",
        check(4402)
    );
    assert!(proven(check(4300)));
}

/// Expression `match` and catalog `variant`s, written untidily.
const MATCH_AND_VARIANTS: &str = r#"enum Mode { Off, Low, High }
object "Synth Dimmer" {
    guid: 0x7E00_0004; price: 40;
    sprite: Sprite::procedural("floor_lamp", primary: #504030, accent: #A09080, trim: #201810);
    state mode: Mode;
    #[slot(2)] state level: i16;
    fn pick(mode: Mode) -> i16 { return match mode { Mode::Off => 0, Mode::Low => 10, Mode::High => 20 }; }
    fn band(x: i16) -> i16 {
        return match x { -32768..=-1 => -1, 0 => 0, 1..=9 => 1, _ => 2 };
    }
    init {
        self.mode = Mode::Low;
        self.level = match self.mode { Mode::Off => pick(Mode::High),
            Mode::Low => -band(3) * 2, // the usual case
            _ => 7 };
        let big = match self.level { 0 => self.level > 3, _ => !(self.level > 5) };
        if big { self.level = 1; }
    }
    #[bhav(label = "Init Budget")]
    fn init_budget() { self.level = 2; }
    #[bhav(label = "Main Budget")]
    fn main_budget() { loop { sleep(20); self.level += 1; } }
    variant "Budget Dimmer" { guid: 0x7E00_0005, price: 15, description: "A cheaper dimmer.", init: init_budget, main: main_budget }
    /// Same trees, another price.
    variant "Plain Dimmer" { guid: 0x7E00_0006 }
}
"#;

#[test]
fn formatter_keeps_empty_frames() {
    let source = r#"object "Synth Empty" {
    guid: 0x7E00_0008; price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #504030, accent: #A09080, trim: #201810);
    #[bhav(id = 4300, label = "A", empty)]
    fn a() {   }
    #[bhav(id = 4301, label = "B", args = 4, empty)]
    fn b() {
    }
}
"#;
    let formatted = format_v2_source(source).expect("format");
    assert!(is_v2_formatted(&formatted).expect("check"), "{formatted}");
    assert_eq!(
        format_v2_source(&formatted).expect("reformat"),
        formatted,
        "idempotent"
    );
    for expected in [
        "    #[bhav(id = 4300, label = \"A\", empty)]\n    fn a() {}\n",
        "    #[bhav(id = 4301, label = \"B\", args = 4, empty)]\n    fn b() {}\n",
    ] {
        assert!(
            formatted.contains(expected),
            "missing {expected:?} in:\n{formatted}"
        );
    }
    // Formatting keeps the 0-node trees.
    let iff = |s: &str| {
        compile_v2_source("empty.antics", s, &PackResolverConfig::default())
            .expect("compile")
            .iff_bytes
    };
    assert_eq!(iff(source), iff(&formatted));
}

#[test]
fn formatter_handles_pr_h_attributes_and_constant_bindings() {
    let source = r#"object "Synth PR H" {
    guid: 0x7E00_0009; price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #504030, accent: #A09080, trim: #201810);
    #[slot(2)] state level: i16;
    #[bhav(id = 4300, label = "Helper", context = inherit)]
    fn helper() { me.attr[2] = 1; }
    #[id(4301)] #[frame(args = 4)] main { temp[0] = 1; }
    #[bhav(id = 4302, label = "Probe")]
    fn probe() {
        #[slot( 3 )] let x: i16;
        const(A, B) = if temp[0] > 0 {(1, 2)}   else {  (3, 4) };
        const C = if temp[1] > 0 { 5 } else { 6 };
        const D = if temp[1] > 1 {
            7
        } else {
            8
        };
        const E = if temp[1] > 2 { 9 } else { return false; };
        temp[2] = A + B + C + D + E;
        self.level = x;
    }
}
"#;
    let formatted = format_v2_source(source).expect("format");
    assert!(is_v2_formatted(&formatted).expect("check"), "{formatted}");
    assert_eq!(
        format_v2_source(&formatted).expect("reformat"),
        formatted,
        "idempotent"
    );
    for expected in [
        "    #[bhav(id = 4300, label = \"Helper\", context = inherit)]\n    fn helper() {\n",
        "        #[slot(3)] let x: i16;\n",
        "        const (A, B) = if temp[0] > 0 { (1, 2) } else { (3, 4) };\n",
        "        const C = if temp[1] > 0 { 5 } else { 6 };\n",
        "        const D = if temp[1] > 1 {\n            7\n        } else {\n            8\n        };\n",
        "        const E = if temp[1] > 2 {\n            9\n        } else {\n            return false;\n        };\n",
    ] {
        assert!(
            formatted.contains(expected),
            "missing {expected:?} in:\n{formatted}"
        );
    }
    let iff = |s: &str| {
        compile_v2_source("pr_h.antics", s, &PackResolverConfig::default())
            .expect("compile")
            .iff_bytes
    };
    assert_eq!(iff(source), iff(&formatted));
}

#[test]
fn formatter_handles_match_expressions_and_variants() {
    let formatted = format_v2_source(MATCH_AND_VARIANTS).expect("format");
    assert!(is_v2_formatted(&formatted).expect("check"), "{formatted}");
    assert_eq!(
        format_v2_source(&formatted).expect("reformat"),
        formatted,
        "idempotent"
    );
    for expected in [
        "        return match mode {\n            Mode::Off => 0,\n            Mode::Low => 10,\n            Mode::High => 20\n        };\n",
        "            -32768..=-1 => -1,\n",
        "            Mode::Low => -band(3) * 2, // the usual case\n",
        "            _ => !(self.level > 5)\n        };\n",
        "\n\n    variant \"Budget Dimmer\" {\n        guid: 0x7E00_0005,\n        price: 15,\n",
        "\n\n    /// Same trees, another price.\n    variant \"Plain Dimmer\" {\n        guid: 0x7E00_0006\n    }\n}\n",
    ] {
        assert!(
            formatted.contains(expected),
            "missing {expected:?} in\n{formatted}"
        );
    }
    // Formatting changes no compiled byte.
    assert_eq!(
        compile("dimmer.antics", MATCH_AND_VARIANTS).iff_bytes,
        compile("dimmer.antics", &formatted).iff_bytes
    );
}

#[test]
fn catalog_variants_round_trip() {
    let iff = parse_iff(&compile("synth-dimmer.antics", MATCH_AND_VARIANTS).iff_bytes);
    let report =
        decompile_iff_to_v2_report(&iff, &DecompileV2Options::default()).expect("decompile");
    let source = &report.source;
    assert!(
        report.trees.iter().all(|t| proven(&t.check)),
        "{:?}\n{source}",
        report.trees
    );
    assert!(
        !report
            .resources_not_reproduced
            .iter()
            .any(|r| r.starts_with("OBJD")),
        "{:?}",
        report.resources_not_reproduced
    );
    assert!(!source.contains("multi-tile"), "{source}");
    assert!(
        source.contains("    variant \"Budget Dimmer\" {\n        guid: 0x7E00_0005,\n        price: 15,\n        description: \"A cheaper dimmer.\",\n        init: "),
        "{source}"
    );
    assert!(
        source.contains(
            "    variant \"Plain Dimmer\" {\n        guid: 0x7E00_0006,\n        price: 40,\n"
        ),
        "{source}"
    );
    let again = parse_iff(&compile("decompiled.antics", source).iff_bytes);
    let guids = |iff: &Iff| {
        iff.chunks
            .iter()
            .filter(|c| &c.tag == b"OBJD")
            .map(|c| c.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(guids(&iff).len(), 3);
    assert_eq!(guids(&again).len(), 3);
}

#[test]
fn a_variant_v2_cannot_spell_is_reported() {
    let iff = parse_iff(&compile("synth-counter.antics", COUNTER).iff_bytes);
    let objd = iff.chunks.iter().find(|c| &c.tag == b"OBJD").expect("OBJD");
    // A catalog variant whose init is the object's main tree, which v2 writes as the
    // `main` block.
    let mut data = iff.data(objd).to_vec();
    let main = [data[10], data[11]];
    data[28..32].copy_from_slice(&0x7E00_0009u32.to_le_bytes());
    data[86..88].copy_from_slice(&main);
    let iff = with_chunks(
        &iff,
        vec![ChunkData {
            tag: *b"OBJD",
            id: objd.id + 1,
            flags: objd.flags,
            label: "Odd Counter".to_owned(),
            data,
        }],
    );
    let report =
        decompile_iff_to_v2_report(&iff, &DecompileV2Options::default()).expect("decompile");
    let source = &report.source;
    assert!(
        source.contains("// variant \"Odd Counter\" (0x7E00_0009) is not written: its init tree 4097 is written as the object's main block."),
        "{source}"
    );
    assert!(!source.contains("multi-tile"), "{source}");
    assert!(
        report
            .resources_not_reproduced
            .iter()
            .any(|r| r.starts_with("OBJD") && r.contains("Odd Counter")),
        "{:?}",
        report.resources_not_reproduced
    );
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
  #[slot(3)] state n: i16; /* trailing block /* nested */ comment */
    bcon 4097 "K" { a = 1, b = 2 }
    #[bhav(id = 4200, label = "T")]
    fn t() {
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
fn formatter_handles_unicode_indentation_and_raw_comments() {
    let source = "object \"Synth Odd\" {\n    guid: 0x7E00_0004;\n    #[bhav(id = 4200, label = \"T\")] fn t() {\n        raw // why raw\n        {\n            temp[0] := 1;\n \u{a0}           temp[1] := 2;\n        }\n        raw /* a */ { temp[2] := 3; }\n        return true;\n    }\n}\n";
    compile("odd.antics", source);
    let once = format_v2_source(source).expect("format");
    assert_eq!(format_v2_source(&once).expect("format twice"), once);
    assert!(once.contains("raw // why raw\n        {\n"), "{once}");
    assert!(once.contains("raw /* a */ { temp[2] := 3; }"), "{once}");
    assert_eq!(
        compile("odd.antics", source).iff_bytes,
        compile("odd.antics", &once).iff_bytes
    );
}

#[test]
fn end_of_line_comments_stay_on_the_line_they_end() {
    // A one-line block the formatter expands keeps a trailing comment (and lint
    // directive) on its opening line, not after the closing `}`.
    let source = "object \"Synth Eol\" {\n    guid: 0x7E00_0005;\n    price: 1;\n    #[slot(2)] state level: i16;\n    state on: bool;\n    fn on() -> bool {\n        if !self.on { return false; } // antics-lint: allow(redundant_bool_return)\n        return true;\n    }\n    #[bhav(label = \"T\")] fn t() {\n        if self.level > 2 { self.level = 0; } // gap: why\n        self.on = on();\n    }\n}\n";
    let once = format_v2_source(source).expect("format");
    assert_eq!(format_v2_source(&once).expect("format twice"), once);
    assert!(
        once.contains(
            "        if !self.on { // antics-lint: allow(redundant_bool_return)\n            return false;\n        }\n"
        ),
        "{once}"
    );
    assert!(
        once.contains(
            "        if self.level > 2 { // gap: why\n            self.level = 0;\n        }\n"
        ),
        "{once}"
    );
    // The directive still covers the statement, before and after formatting.
    for text in [source, once.as_str()] {
        let found = sims_antics::v2::lint::lint_v2_source(text, "eol.antics").expect("lint");
        assert!(
            found.iter().all(|d| d.rule != "redundant_bool_return"),
            "{found:#?}\n{text}"
        );
    }
    assert_eq!(
        compile("eol.antics", source).iff_bytes,
        compile("eol.antics", &once).iff_bytes
    );
}

#[test]
fn end_of_line_directives_cover_else_arms_and_follow_raw_blocks() {
    // The directive lands in the first arm; it covers the whole `if` / `else` chain. A
    // multi-line `raw` block earlier in the tree does not stop the relocation.
    let source = "object \"Synth Eol\" {\n    guid: 0x7E00_0006;\n    price: 1;\n    #[slot(2)] state level: i16;\n    #[bhav(label = \"T\")] fn t() {\n        raw {\n            temp[0] := 1;\n        }\n        if self.level > 2 { self.level = 0; } // gap: why\n        if self.level > 1 { sleep(1); } else { while true { sleep(1); } } // antics-lint: allow(while_true)\n    }\n}\n";
    let once = format_v2_source(source).expect("format");
    assert_eq!(format_v2_source(&once).expect("format twice"), once);
    assert!(
        once.contains(
            "        if self.level > 2 { // gap: why\n            self.level = 0;\n        }\n"
        ),
        "{once}"
    );
    assert!(
        once.contains("        if self.level > 1 { // antics-lint: allow(while_true)\n"),
        "{once}"
    );
    for text in [source, once.as_str()] {
        let found = sims_antics::v2::lint::lint_v2_source(text, "eol.antics").expect("lint");
        assert!(
            found.iter().all(|d| d.rule != "while_true"),
            "{found:#?}\n{text}"
        );
    }
    // The block opens on the line a multi-line raw block ends on.
    let source = "object \"Synth Eol\" {\n    guid: 0x7E00_0007;\n    price: 1;\n    #[slot(2)] state level: i16;\n    #[bhav(label = \"T\")] fn t() {\n        raw {\n            temp[0] := 1;\n        } if self.level > 1 { while true { sleep(1); } } // antics-lint: allow(while_true)\n    }\n}\n";
    let once = format_v2_source(source).expect("format");
    assert_eq!(format_v2_source(&once).expect("format twice"), once);
    for text in [source, once.as_str()] {
        let found = sims_antics::v2::lint::lint_v2_source(text, "eol.antics").expect("lint");
        assert!(
            found.iter().all(|d| d.rule != "while_true"),
            "{found:#?}\n{text}"
        );
    }
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

/// PR E spellings the decompiler writes back: `match` chains, `else if`, `inc_lt` /
/// `dec_gt`, `self.slot[i]`, `param[n]`, `notify_out_of_idle()`, `set_dynamic_sprite`,
/// and `push_interaction(object: ..)`.
const PR_E_IDIOMS: &str = r#"object "Synth Switchboard" {
    guid: 0x7E00_0007;
    price: 5;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);
    #[slot(2)] state level: i16;
    #[slot(3)] state mode: i16;

    #[bhav(id = 4400, label = "Chain")]
    fn chain() {
        if self.level == 1 {
            self.mode = 4;
        } else if self.level == 2 {
            self.mode = 5;
        } else if self.level == -7 {
            self.mode = 6;
        } else {
            self.mode = 9;
        }
        if self.mode == 1 {
            self.level = 1;
        } else if self.level > 2 {
            self.level = 2;
        } else {
            self.level = 0;
        }
        self.mode = 0;
    }

    #[bhav(id = 4401, label = "Counted")]
    fn counted() {
        temp[0] = 0;
        while inc_lt(&mut temp[0], 5) {
            self.level += 1;
        }
        while dec_gt(&mut temp[1], 0) {
            self.level -= 1;
        }
    }

    #[bhav(id = 4402, label = "Slots", args = 2)]
    fn slots() {
        self.level = self.slot[0];
        notify_out_of_idle();
        param[1] = 3;
        self.mode = param[0];
    }

    #[bhav(id = 4403, label = "Sim Side")]
    fn sim_side(sim: Sim) {
        set_dynamic_sprite(index: 2, value: 1);
        self.level = self.slot[1];
        push_interaction(menu: 2, object: self, priority: 50, flags: 0);
        push_interaction(menu: 3, object: sim, priority: 40, flags: 4);
    }

    #[bhav(id = 4404, label = "Pushes")]
    fn pushes() {
        push_interaction(menu: 2, object: self, priority: 50, flags: 0);
        push_interaction(menu: 2, object: stack_obj, priority: 50, flags: 0);
    }
}
"#;

#[test]
fn pr_e_idioms_round_trip() {
    let source = assert_round_trip("synth-switchboard.antics", PR_E_IDIOMS);
    for idiom in [
        "match self.attr_2 {\n            1 => {\n                self.attr_3 = 4;",
        "            -7 => {\n",
        "            _ => {\n                self.attr_3 = 9;",
        "} else if self.attr_2 > 2 {",
        "while inc_lt(&mut temp[0], 5) {",
        "while dec_gt(&mut temp[1], 0) {",
        "self.attr_2 = self.slot[0];",
        "notify_out_of_idle();",
        "param[1] = 3;",
        "self.attr_3 = param[0];",
        "set_dynamic_sprite(index: 2, value: 1);",
        "self.attr_2 = self.slot[1];",
        "push_interaction(menu: 2, object: self, priority: 50, flags: 0);",
        "push_interaction(menu: 3, object: sim, priority: 40, flags: 4);",
        "push_interaction(menu: 2, object: stack_obj, priority: 50, flags: 0);",
    ] {
        assert!(source.contains(idiom), "missing `{idiom}` in:\n{source}");
    }
    let iff = parse_iff(&compile("synth-switchboard.antics", PR_E_IDIOMS).iff_bytes);
    let report =
        decompile_iff_to_v2_report(&iff, &DecompileV2Options::default()).expect("decompile");
    for t in report
        .trees
        .iter()
        .filter(|t| (4400..=4404).contains(&t.id))
    {
        assert_eq!(t.style, TreeStyle::Idiomatic, "tree {}\n{source}", t.id);
        assert_eq!(t.raw_statements, 0, "tree {}\n{source}", t.id);
    }
}

#[test]
fn a_relowering_idiom_that_is_not_proven_falls_back_inside_the_tree() {
    // `run_functional_tree` as written by hand: v2's `stack_obj.run_functional(4)` adds a
    // `stack_obj` restore, so the decompiler may only keep it if the checker proves it;
    // either way the tree stays idiomatic around it.
    const SOURCE: &str = r#"object "Synth Relay" {
    guid: 0x7E00_0008;
    price: 5;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);
    #[slot(2)] state level: i16;

    #[bhav(id = 4410, label = "Relay")]
    fn relay() {
        stack_obj = 0;
        raw { run_functional_tree(4); }
        self.level = 3;
    }
}
"#;
    let iff = parse_iff(&compile("synth-relay.antics", SOURCE).iff_bytes);
    let report =
        decompile_iff_to_v2_report(&iff, &DecompileV2Options::default()).expect("decompile");
    let relay = report.trees.iter().find(|t| t.id == 4410).unwrap();
    assert!(proven(&relay.check), "{:?}\n{}", relay.check, report.source);
    assert_eq!(relay.style, TreeStyle::Idiomatic, "{}", report.source);
    assert!(
        report.source.contains("self.attr_2 = 3;"),
        "{}",
        report.source
    );
}

#[test]
fn an_extra_idiom_that_does_not_compile_is_dropped_before_the_tree_is_lowered() {
    // The `local[0]` write and the `push_interaction` pair up as
    // `push_interaction(.., object: self)`, which clashes with the raw block left after
    // it; the tree must keep its other statements idiomatic, as it did without the idiom.
    const SOURCE: &str = r#"object "Synth Patch" {
    guid: 0x7E00_0009;
    price: 5;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);

    #[bhav(id = 4420, label = "Patch")]
    fn patch() {
        raw {
            local[0] := my.id;
            push_interaction(menu: 0, object: 0, priority: 0, flags: 2);
            temp[1] := stack.data[0];
        }
        temp[0] = 3;
    }
}
"#;
    let iff = parse_iff(&compile("synth-patch.antics", SOURCE).iff_bytes);
    let report =
        decompile_iff_to_v2_report(&iff, &DecompileV2Options::default()).expect("decompile");
    let patch = report.trees.iter().find(|t| t.id == 4420).unwrap();
    assert!(proven(&patch.check), "{:?}\n{}", patch.check, report.source);
    assert_ne!(patch.style, TreeStyle::Raw, "{}", report.source);
    assert!(report.source.contains("temp[0] = 3;"), "{}", report.source);
}
