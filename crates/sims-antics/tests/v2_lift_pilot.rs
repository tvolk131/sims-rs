//! Synthetic round trips for additional v2 lift spellings and resource labels.

use sims_antics::compile_v2_source;
use sims_antics::v2::{
    DecompileV2Options, PackResolverConfig, TreeCheck, decompile_iff_to_v2_report,
};
use sims_formats::iff::{ChunkData, Iff};
use std::collections::BTreeMap;
use std::process::Command;

fn compile(source: &str) -> Iff {
    let object = compile_v2_source("pilot.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{e}\n{source}"));
    Iff::parse(object.iff_bytes).expect("parse IFF")
}

fn with_chunks(iff: &Iff, extra: Vec<ChunkData>) -> Iff {
    let mut chunks: Vec<_> = iff
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
    Iff::parse(sims_formats::iff::write(&chunks).expect("write IFF")).expect("parse IFF")
}

fn round_trip(iff: &Iff, options: &DecompileV2Options) -> String {
    let report = decompile_iff_to_v2_report(iff, options).expect("lift");
    assert!(report.compiled);
    assert!(
        report
            .trees
            .iter()
            .all(|t| matches!(t.check, TreeCheck::Identical | TreeCheck::Equivalent)),
        "{:?}\n{}",
        report.trees,
        report.source
    );
    compile(&report.source);
    report.source
}

#[test]
fn lifts_dialog_action_label_destroy_and_animation() {
    let iff = compile(
        r#"object "Fable Kiosk" {
    guid: 0x7E00_00A0;
    @id(4300) tree confirm {
        dialog(kind: yes_no, message: 3, title: 2)?;
        action_label(table: 431, index: 4);
        destroy_self(no_yield: true);
    }
    @id(4301) @frame(locals: 2) tree spin {
        raw {
            while !animate(clip: 11, table: 2, events: 1, flags: 32, local: 1) {}
            temp[3] := local[1];
        }
    }
    @id(4302) tree stop_spin {
        raw {
            while !animate(clip: 0, flags: 0, local: 1) {}
            action_string(table: 432, index: 5) pad 0xff;
        }
    }
}"#,
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    for spelling in [
        "dialog(kind: 1, message: 3, title: 2)?;",
        "action_label(table: 431, index: 4);",
        "destroy_self(no_yield: true);",
        "animate(clip: 11, table: 2, flags: 0, events: 1, local: slot_1);",
        "temp[3] = slot_1;",
        "animate(clip: 0, table: 0, flags: 0, events: 0);",
        "action_label(table: 432, index: 5);",
    ] {
        assert!(lifted.contains(spelling), "missing {spelling}\n{lifted}");
    }
}

#[test]
fn lifts_stack_sprite_flags() {
    let iff = compile(
        r#"object "Fable Tiles" {
    guid: 0x7E00_00A1;
    @id(4300) tree update_tiles {
        stack_obj.sprite_flags[3] = 1;
        temp[3] = stack_obj.sprite_flags[6];
    }
}"#,
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    assert!(
        lifted.contains("stack_obj.sprite_flags[3] = 1;"),
        "{lifted}"
    );
    assert!(
        lifted.contains("temp[3] = stack_obj.sprite_flags[6];"),
        "{lifted}"
    );
}

#[test]
fn lifts_literal_private_calls_and_names_library_calls() {
    let iff = compile(
        r#"extern mod services {
    @id(305) fn choose(a: i16, b: i16) -> bool;
    @id(306) fn older() -> bool;
    @id(307) fn fallback() -> bool;
}
object "Fable Calculator" {
    guid: 0x7E00_00A2;
    @id(4300) tree compute(a: i16, b: i16, c: i16, d: i16) {
        temp[3] = a;
        return true;
    }
    @id(4301) tree caller {
        if compute(1, 0, 0, 0) { temp[4] = 9; }
        if services::choose(-2, 3) { temp[5] = 10; }
        if services::older() { temp[6] = 11; }
        if services::fallback() { temp[7] = 12; }
    }
    @id(4302) tree conditional_tail {
        raw { call 4300(2, 0, 0, 0); }
    }
    @id(4303) tree forwards_temps {
        raw { call 4300(temps); }
    }
}"#,
    );
    let opts = DecompileV2Options {
        library_arities: BTreeMap::from([(305, 2), (306, 0), (307, 0)]),
        library_names: BTreeMap::from([
            (305, "Pick - A Dial".to_owned()),
            (306, "Pick A Dial".to_owned()),
        ]),
        ..Default::default()
    };
    let lifted = round_trip(&iff, &opts);
    for spelling in [
        "compute_4300(1, 0, 0, 0)",
        "globals::pick_a_dial(-2, 3)",
        "globals::pick_a_dial_306()",
        "globals::global_307()",
        "if !compute_4300(2, 0, 0, 0) {",
        "call 4300(temps);",
    ] {
        assert!(lifted.contains(spelling), "missing {spelling}\n{lifted}");
    }
}

#[test]
fn bcon_and_sound_identifiers_follow_resource_labels() {
    let iff = compile(
        r#"object "Fable Ringer" {
    guid: 0x7E00_00A3;
    bcon 4100 "Dial" { first = 2, second = 5, third = 8 }
    bcon 4101 "Range" { chosen = 13 }
    sound original = preset("ui_click", wav: "Fable-Ping", id: 6);
    sound other = preset("ui_click", wav: "Fable_Ping", id: 8);
    @id(4300) tree ring {
        temp[0] = Dial.first;
        temp[1] = Dial.second;
        temp[2] = Dial.third;
        temp[3] = Range.chosen;
        play_sound(original);
        play_sound(other);
    }
}"#,
    );
    let mut tuning_labels = vec![0; 4];
    tuning_labels.extend([2, 0, 0, 0]);
    tuning_labels.extend(b"NCRT");
    tuning_labels.extend([1, 0, 0, 0]);
    tuning_labels.extend([0; 8]);
    tuning_labels.extend(b"\x0b Upper band".iter().copied());
    tuning_labels.extend([0; 6]);
    let iff = with_chunks(
        &iff,
        vec![
            ChunkData {
                tag: *b"STR#",
                id: 4100,
                flags: 0x10,
                label: "Dial labels".to_owned(),
                data: b"\xff\xff\x02\x00Tuning Speed\0Tuning-Speed\0".to_vec(),
            },
            ChunkData {
                tag: *b"TRCN",
                id: 4101,
                flags: 0x10,
                label: "Range labels".to_owned(),
                data: tuning_labels,
            },
        ],
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    for spelling in [
        "temp[0] = Dial.tuning_speed;",
        "temp[1] = Dial.e1;",
        "temp[2] = Dial.e2;",
        "temp[3] = Range.upper_band;",
        "sound fable_ping = preset(\"ui_click\", wav: \"Fable-Ping\", id: 6);",
        "sound fable_ping_8 = preset(\"ui_click\", wav: \"Fable_Ping\", id: 8);",
        "play_sound(fable_ping);",
        "play_sound(fable_ping_8);",
    ] {
        assert!(lifted.contains(spelling), "missing {spelling}\n{lifted}");
    }
}

#[test]
fn checks_each_definition_and_lists_shared_interactions_once() {
    let iff = compile(
        r#"object "Fable Dial" {
    guid: 0x7E00_00A4;
    price: 12;
    interaction "Turn Dial" {
        autonomy: 10,
        run(sim) { return true; }
    }
    variant "Fable Dial Plus" { guid: 0x7E00_00A5, price: 27 }
}"#,
    );
    let source = round_trip(&iff, &DecompileV2Options::default());
    assert_eq!(source.matches("Turn Dial").count(), 1, "{source}");
    assert!(source.contains("variant \"Fable Dial Plus\""), "{source}");
    let again = compile(&source);
    let objds = |iff: &Iff| -> BTreeMap<u16, Vec<u8>> {
        iff.chunks
            .iter()
            .filter(|c| c.tag == *b"OBJD")
            .map(|c| (c.id, iff.data(c).to_vec()))
            .collect()
    };
    assert_eq!(objds(&iff), objds(&again));

    let primary = iff.all(b"OBJD").next().expect("primary definition");
    let id = primary.id + 9;
    let mut data = iff.data(primary).to_vec();
    data[28..32].copy_from_slice(&0x7E00_00A8u32.to_le_bytes());
    data[14..16].copy_from_slice(&9u16.to_le_bytes()); // an independent TTAB link
    let augmented = with_chunks(
        &iff,
        vec![ChunkData {
            tag: *b"OBJD",
            id,
            label: "Fable Uneven Dial".to_owned(),
            flags: primary.flags,
            data,
        }],
    );
    let report =
        decompile_iff_to_v2_report(&augmented, &DecompileV2Options::default()).expect("lift");
    assert!(
        report
            .resources_not_reproduced
            .contains(&format!("OBJD {id}"))
    );
    assert_eq!(report.source.matches("Turn Dial").count(), 1);
}

#[test]
fn cli_uses_explicit_global_labels() {
    let library_object = compile(
        r#"object "Fable Aux" {
    guid: 0x7E00_00A6;
    @id(4300) tree choose "Dial - Adjust" { return true; }
}"#,
    );
    let behavior = library_object.get(b"BHAV", 4300).expect("library tree");
    let global_bytes = sims_formats::iff::write(&[ChunkData {
        tag: *b"BHAV",
        id: 305,
        flags: behavior.flags,
        label: behavior.label.clone(),
        data: library_object.data(behavior).to_vec(),
    }])
    .expect("write global IFF");
    let caller = compile(
        r#"extern mod aux { @id(305) fn use_dial() -> bool; }
object "Fable Remote" {
    guid: 0x7E00_00A7;
    @id(4300) tree dial { if aux::use_dial() { temp[2] = 42; } }
}"#,
    );
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("v2-lift-pilot");
    std::fs::create_dir_all(&dir).expect("create test directory");
    let globals_path = dir.join("pilot-globals.iff");
    let object_path = dir.join("pilot-remote.iff");
    std::fs::write(&globals_path, global_bytes).expect("write globals fixture");
    std::fs::write(&object_path, &caller.bytes).expect("write object fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_sims-antics"))
        .args(["lift", "--v2"])
        .arg(object_path)
        .arg("--globals")
        .arg(globals_path)
        .output()
        .expect("run CLI");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lifted = String::from_utf8(output.stdout).expect("source");
    assert!(lifted.contains("fn dial_adjust() -> bool;"), "{lifted}");
    assert!(lifted.contains("globals::dial_adjust()"), "{lifted}");
}

#[test]
fn trees_showing_text_the_lift_does_not_write_are_inconclusive() {
    // The lift writes no `strings`, so a tree reading a table the original has cannot
    // prove: the rebuilt object shows different (no) text.
    let iff = compile(
        r#"object "Fable Sign" {
    guid: 0x7E00_00A4;
    strings 301 "Dialog prim string set" { ask = "Go on?", head = "Sign" }
    strings 431 "Action labels" { a = "Read", b = "Wave" }
    @id(4300) tree confirm {
        dialog(kind: yes_no, message: ask, title: head)?;
    }
    @id(4301) tree label {
        action_label(table: 431, index: 2);
    }
    @id(4302) tree plain {
        temp[0] = 1;
    }
}"#,
    );
    let report = decompile_iff_to_v2_report(&iff, &DecompileV2Options::default()).expect("lift");
    let check = |id: u16| {
        &report
            .trees
            .iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("tree {id} in {:?}", report.trees))
            .check
    };
    for (id, table) in [(4300, 301), (4301, 431)] {
        match check(id) {
            TreeCheck::Inconclusive(why) => {
                assert!(why.contains(&format!("STR# {table}")), "{why}");
            }
            other => panic!("tree {id}: {other:?}"),
        }
    }
    assert!(matches!(
        check(4302),
        TreeCheck::Identical | TreeCheck::Equivalent
    ));
}

/// `iff` with chunk `from` retagged `to` (same id and bytes).
fn retagged(iff: &Iff, from: ([u8; 4], u16), to: [u8; 4]) -> Iff {
    let chunks: Vec<_> = iff
        .chunks
        .iter()
        .filter(|c| &c.tag != b"rsmp")
        .map(|c| ChunkData {
            tag: if (c.tag, c.id) == from { to } else { c.tag },
            id: c.id,
            flags: c.flags,
            label: c.label.clone(),
            data: iff.data(c).to_vec(),
        })
        .collect();
    Iff::parse(sims_formats::iff::write(&chunks).expect("write IFF")).expect("parse IFF")
}

#[test]
fn fallback_text_tables_and_tableless_dialogs() {
    let iff = compile(
        r#"object "Fable Board" {
    guid: 0x7E00_00A5;
    strings 301 "Dialog prim string set" { ask = "Go on?" }
    strings 431 "Notes" { a = "Read" }
    @id(4300) tree confirm {
        dialog(kind: yes_no, message: ask)?;
    }
    @id(4301) tree note {
        raw { show_string(table: 431, index: 1); }
    }
    @id(4302) tree pick {
        dialog(kind: 5)?;
    }
}"#,
    );
    // The original's loader falls back on a legacy `CST` 301 and a `CTSS` 431.
    let iff = retagged(&iff, (*b"STR#", 301), *b"\0CST");
    let iff = retagged(&iff, (*b"STR#", 431), *b"CTSS");
    let report = decompile_iff_to_v2_report(&iff, &DecompileV2Options::default()).expect("lift");
    let check = |id: u16| {
        &report
            .trees
            .iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("tree {id} in {:?}", report.trees))
            .check
    };
    for id in [4300, 4301] {
        assert!(
            matches!(check(id), TreeCheck::Inconclusive(_)),
            "tree {id}: {:?}",
            check(id)
        );
    }
    // A dialog naming no string reads no table.
    assert!(
        !matches!(check(4302), TreeCheck::Inconclusive(why) if why.contains("STR#")),
        "{:?}",
        check(4302)
    );
}
