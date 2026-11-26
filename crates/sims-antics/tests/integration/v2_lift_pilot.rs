//! Synthetic round trips for additional v2 lift spellings and resource labels.

use sims_antics::v2::lint_v2_source;
use sims_antics::v2::{
    DecompileV2Options, PackResolverConfig, TreeCheck, decompile_iff_to_v2_report,
};
use sims_antics::{IffEquivOptions, IffVerdict, check_iff_equivalent, compile_v2_source};
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
    #[bhav(id = 4300)]
    fn confirm() {
        dialog(kind: yes_no, message: 3, title: 2)?;
        action_label(table: 431, index: 4);
        destroy_self(no_yield: true);
    }
    #[bhav(id = 4301, locals = 2)]
    fn spin() {
        raw {
            while !animate(clip: 11, table: 2, events: 1, flags: 32, local: 1) {}
            temp[3] := local[1];
        }
    }
    #[bhav(id = 4302)]
    fn stop_spin() {
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
    #[bhav(id = 4300)]
    fn update_tiles() {
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
    #[id(305)] fn choose(a: i16, b: i16) -> bool;
    #[id(306)] fn older() -> bool;
    #[id(307)] fn fallback() -> bool;
}
object "Fable Calculator" {
    guid: 0x7E00_00A2;
    #[bhav(id = 4300)]
    fn compute(a: i16, b: i16, c: i16, d: i16) {
        temp[3] = a;
        return true;
    }
    #[bhav(id = 4301)]
    fn caller() {
        if compute(1, 0, 0, 0) { temp[4] = 9; }
        if services::choose(-2, 3) { temp[5] = 10; }
        if services::older() { temp[6] = 11; }
        if services::fallback() { temp[7] = 12; }
    }
    #[bhav(id = 4302)]
    fn conditional_tail() {
        raw { call 4300(2, 0, 0, 0); }
    }
    #[bhav(id = 4303)]
    fn forwards_temps() {
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
        "compute(1, 0, 0, 0)",
        "globals::pick_a_dial(-2, 3)",
        "globals::pick_a_dial_306()",
        "globals::global_307()",
        "return compute(2, 0, 0, 0);",
        "if !call 4300(temps) { return false; }",
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
    #[bhav(id = 4300)]
    fn ring() {
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
        "temp[0] = Dial::tuning_speed;",
        "temp[1] = Dial::e1;",
        "temp[2] = Dial::e2;",
        "temp[3] = Range::upper_band;",
        "sound fable_ping = fwav(6, wav: \"Fable-Ping\");",
        "sound fable_ping_8 = fwav(8, wav: \"Fable_Ping\");",
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
    #[bhav(id = 4300, label = "Dial - Adjust")]
    fn choose() { return true; }
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
        r#"extern mod aux { #[id(305)] fn use_dial() -> bool; }
object "Fable Remote" {
    guid: 0x7E00_00A7;
    #[bhav(id = 4300)]
    fn dial() { if aux::use_dial() { temp[2] = 42; } }
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
fn trees_showing_text_prove_through_lifted_strings() {
    // The lift writes the tables the trees show text from as `strings` members.
    let source = r#"object "Fable Sign" {
    guid: 0x7E00_00A4;
    strings 301 "Dialog prim string set" { ask = "Go on? $Local:0", head = "Sign \"A\"" }
    strings 431 "Action labels" { a = "Read", b = "Wave" }
    #[bhav(id = 4300)]
    fn confirm() {
        dialog(kind: yes_no, message: ask, title: head, icon_mode: 1)?;
    }
    #[bhav(id = 4301)]
    fn label() {
        action_label(table: 431, index: 2);
    }
    #[bhav(id = 4302)]
    fn plain() {
        temp[0] = 1;
    }
}"#;
    let iff = compile(source);
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    for line in [
        "strings 301 \"Dialog prim string set\" {",
        "go_on = \"Go on? $Local:0\",",
        "sign_a = \"Sign \\\"A\\\"\",",
        "strings 431 \"Action labels\" {",
        "wave = \"Wave\",",
    ] {
        assert!(lifted.contains(line), "missing {line}\n{lifted}");
    }

    // A table v2 cannot spell (a control character) is not written: the tree that shows
    // it cannot prove.
    let iff = with_data(
        &compile(source),
        (*b"STR#", 431),
        vec![
            0xFF, 0xFF, 2, 0, b'R', b'e', b'a', b'd', 0, b'W', 0x01, b'v', b'e', 0,
        ],
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
    match check(4301) {
        TreeCheck::Inconclusive(why) => assert!(why.contains("STR# 431"), "{why}"),
        other => panic!("tree 4301: {other:?}"),
    }
    for id in [4300, 4302] {
        assert!(
            matches!(check(id), TreeCheck::Identical | TreeCheck::Equivalent),
            "tree {id}: {:?}",
            check(id)
        );
    }
    assert!(
        report.source.contains(
            "// strings 431 is not written: a text holds a control character a v2 string does not spell."
        ),
        "{}",
        report.source
    );
}

/// The lift's check of tree `id` in `iff`, and the lifted source.
fn check_of(iff: &Iff, id: u16) -> (TreeCheck, String) {
    let report = decompile_iff_to_v2_report(iff, &DecompileV2Options::default()).expect("lift");
    let check = report
        .trees
        .iter()
        .find(|t| t.id == id)
        .unwrap_or_else(|| panic!("tree {id} in {:?}", report.trees))
        .check
        .clone();
    (check, report.source)
}

#[test]
fn tables_the_vm_reads_another_way_are_not_written() {
    let source = r#"object "Fable Notice" {
    guid: 0x7E00_00A6;
    strings 301 "Dialog prim string set" { ask = "Alpha" }
    #[bhav(id = 4300)]
    fn confirm() {
        dialog(kind: 0, message: 1, title: 1)?;
    }
}"#;
    // A version -3 entry: language, text, comment.
    let entry = |language: u8, text: &str| {
        let mut out = vec![language];
        out.extend_from_slice(text.as_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    };
    let table = |entries: &[(u8, &str)]| {
        let mut out = vec![0xFD, 0xFF];
        out.extend_from_slice(&u16::try_from(entries.len()).unwrap().to_le_bytes());
        for (language, text) in entries {
            out.extend(entry(*language, text));
        }
        out
    };
    // US English entries in two runs: the loader puts the later run first, so the
    // dialog's first string is not the first US English entry.
    let interleaved = with_data(
        &compile(source),
        (*b"STR#", 301),
        table(&[(1, "Alpha"), (2, "Alphe"), (1, "Beta")]),
    );
    // A second, deleted chunk of the ID.
    let deleted = with_chunks(
        &compile(source),
        vec![ChunkData {
            tag: *b"STR#",
            id: 301,
            flags: 0x10 | sims_formats::iff::DELETED,
            label: "Dialog prim string set".to_owned(),
            data: table(&[(1, "Gamma")]),
        }],
    );
    for (iff, why) in [
        (interleaved, "its US English entries are not in one run"),
        (deleted, "the file has more than one chunk of its ID"),
    ] {
        let (check, source) = check_of(&iff, 4300);
        match check {
            TreeCheck::Inconclusive(w) => assert!(w.contains("STR# 301"), "{w}"),
            other => panic!("tree 4300: {other:?}\n{source}"),
        }
        assert!(
            source.contains(&format!("// strings 301 is not written: {why}.")),
            "{source}"
        );
    }
}

/// `iff` with chunk `key`'s bytes replaced.
fn with_data(iff: &Iff, key: ([u8; 4], u16), data: Vec<u8>) -> Iff {
    let chunks: Vec<_> = iff
        .chunks
        .iter()
        .filter(|c| &c.tag != b"rsmp")
        .map(|c| ChunkData {
            tag: c.tag,
            id: c.id,
            flags: c.flags,
            label: c.label.clone(),
            data: if (c.tag, c.id) == key {
                data.clone()
            } else {
                iff.data(c).to_vec()
            },
        })
        .collect();
    Iff::parse(sims_formats::iff::write(&chunks).expect("write IFF")).expect("parse IFF")
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
    #[bhav(id = 4300)]
    fn confirm() {
        dialog(kind: yes_no, message: ask)?;
    }
    #[bhav(id = 4301)]
    fn note() {
        raw { show_string(table: 431, index: 1); }
    }
    #[bhav(id = 4302)]
    fn pick() {
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

#[test]
fn zero_node_trees_lift_as_frame_empty() {
    let iff = compile(
        r#"object "Fable Empty" {
    guid: 0x7E00_00B0;
    #[bhav(id = 4300, label = "start fire", args = 4, empty)]
    fn start_fire() {}
    #[bhav(id = 4301, label = "Other")]
    fn other() { temp[0] = 1; }
}"#,
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    assert!(
        lifted.contains("revision = 0, empty)]\n    fn start_fire() {}"),
        "{lifted}"
    );
}

#[test]
fn lifts_suit_burn_and_look_primitives() {
    let iff = compile(
        r#"object "Fable Costume Stand" {
    guid: 0x7E00_00B0;
    #[bhav(id = 4300)]
    fn adjust() {
        change_suit(index: 3, scope: 1, remove: true);
        if burn(mode: 1, flags: 2) { look_towards(mode: -1); }
    }
    #[bhav(id = 4301)]
    fn trim() {
        raw { change_suit(index: 4, scope: 1) pad(7: 0xff); }
    }
}"#,
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    for spelling in [
        "change_suit(index: 3, scope: 1, remove: true);",
        "change_suit(index: 4, scope: 1) pad(7: 0xff);",
        "burn(mode: 1, flags: 2)",
        "look_towards(mode: -1);",
    ] {
        assert!(lifted.contains(spelling), "missing {spelling}\n{lifted}");
    }
    assert_eq!(lifted.matches("raw {").count(), 1, "{lifted}");
}

#[test]
fn lifts_literal_drop_slots_and_supported_routing_modes() {
    let iff = compile(
        r#"object "Fable Parcel Rack" {
    guid: 0x7E00_00B1;
    #[bhav(id = 4300, args = 1)]
    fn place() {
        if route_to(0, flags: 1, source: 0) { snap(3, mode: 4); }
        drop_onto(src: -1, dst: 2);
    }
    #[bhav(id = 4301, args = 1)]
    fn parameter_slot() {
        raw { drop_onto(param[0], 2); }
    }
}"#,
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    for spelling in [
        "if route_to(0, flags: 1, source: 0) {",
        "snap(3, mode: 4);",
        "drop_onto(src: -1, dst: 2);",
        "drop_onto(src: param[0], dst: 2);",
    ] {
        assert!(lifted.contains(spelling), "missing {spelling}\n{lifted}");
    }
    assert!(!lifted.contains("raw {"), "{lifted}");
}

#[test]
fn lifts_location_local_and_generic_calls() {
    let iff = compile(
        r#"object "Fable Survey Kit" {
    guid: 0x7E00_00B2;
    #[bhav(id = 4300, locals = 2)]
    fn survey() {
        raw {
            local[1] := 12;
            if find_location(mode: 3, flags: 1, local: 1) {
                generic_call(41);
            }
            find_location(mode: 1, flags: 0);
        }
    }
    #[bhav(id = 4301, locals = 3)]
    fn pinned() {
        raw {
            local[1] := 12;
            local[2] := 14;
            find_location(mode: 1, flags: 1, local: 1);
        }
        temp[4] = 2;
    }
}"#,
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    for spelling in [
        "slot_1 = 12;",
        "if find_location(mode: 3, flags: 1, local: slot_1) {",
        "generic_call(41);",
        "find_location(mode: 1, flags: 0);",
        // `#[slot]` keeps slot 1 where the primitive reads it.
        "find_location(mode: 1, flags: 1, local: slot_1);",
        "temp[4] = 2;",
    ] {
        assert!(lifted.contains(spelling), "missing {spelling}\n{lifted}");
    }
}

#[test]
fn lifts_relationship_locals_and_other_value_operands() {
    let iff = compile(
        r#"object "Fable Matchbook" {
    guid: 0x7E00_00B3;
    #[bhav(id = 4300, args = 1, locals = 2)]
    fn score() {
        raw {
            local[0] := 11;
            relationship(column: 1, dir: 2, flags: 0, local: 0, value: local[1]);
            temp[3] := local[1];
            if old_relationship(write: false, column: 2, param: 0, dir: 0, flags: 1) {
                temp[4] := 9;
            }
        }
    }
    #[bhav(id = 4301, args = 1)]
    fn nonlocals() {
        raw {
            relationship(column: 1, dir: 0, flags: 4, value: 12);
            relationship(column: 1, dir: 0, flags: 4, value: temp[0]);
            relationship(column: 1, dir: 0, flags: 4, value: param[0]);
        }
    }
}"#,
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    for spelling in [
        "relationship(column: 1, dir: 2, flags: 0, local: slot_0, value: slot_1);",
        "temp[3] = slot_1;",
        "old_relationship(write: false, column: 2, param: 0, dir: 0, flags: 1)",
        "value: 12);",
        "value: temp[0]);",
        "value: param[0]);",
    ] {
        assert!(lifted.contains(spelling), "missing {spelling}\n{lifted}");
    }
    // Temps, parameters and (with flag 4) literals are values v2 names too, and `#[slot]`
    // pins the named locals: nothing stays raw.
    assert_eq!(lifted.matches("raw {").count(), 0, "{lifted}");
    assert!(lifted.contains("#[slot(1)] let slot_1: i16;"), "{lifted}");
}

#[test]
fn lifts_flagged_sound_and_keeps_the_reserved_flags_byte_raw() {
    let iff = compile(
        r#"object "Fable Bell Cabinet" {
    guid: 0x7E00_00B4;
    sound trill = preset("ui_click", wav: "Fable-Trill", id: 6);
    #[bhav(id = 4300)]
    fn ring() {
        play_sound(trill, stack_obj, flags: 5);
        play_sound(99, flags: 8);
        stop_all_sounds(stack_obj);
    }
    #[bhav(id = 4301)]
    fn reserved() {
        raw { play_sound(6, flags: 255); }
    }
}"#,
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    for spelling in [
        "stack_obj.play_sound(fable_trill, flags: 5);",
        "play_sound(99, flags: 8);",
        "stack_obj.stop_all_sounds();",
        "play_sound(6, flags: 255);",
    ] {
        assert!(lifted.contains(spelling), "missing {spelling}\n{lifted}");
    }
    assert_eq!(lifted.matches("raw {").count(), 1, "{lifted}");
}

#[test]
fn lifts_job_data_with_the_staged_career_and_level() {
    let iff = compile(
        r#"object "Fable Ledger" {
    guid: 0x7E00_00B5;
    #[bhav(id = 4300)]
    fn lookup() {
        temp[3] = job_data(4, 2, 5);
        temp[4] = job_data(temp[0], 3, 6);
    }
}"#,
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    for spelling in [
        "temp[3] = job_data(4, 2, 5);",
        "temp[4] = job_data(temp[0], 3, 6);",
    ] {
        assert!(lifted.contains(spelling), "missing {spelling}\n{lifted}");
    }
    assert!(!lifted.contains("raw {"), "{lifted}");
}

#[test]
fn lifts_distance_with_and_without_a_stack_object_move() {
    let iff = compile(
        r#"object "Fable Range Meter" {
    guid: 0x7E00_00B6;
    #[bhav(id = 4300)]
    fn measure() {
        temp[7] = distance_to(stack_obj, from: self, temp: 3);
    }
    #[bhav(id = 4301)]
    fn measure_self() {
        temp[7] = distance_to(self, temp: 2);
        temp[6] = stack_obj;
    }
    #[bhav(id = 4302)]
    fn keep_distance_in_temp() {
        raw { get_distance_to(temp[0]); }
        temp[7] = 2;
    }
}"#,
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    for spelling in [
        "temp[7] = distance_to(stack_obj, from: self.id, temp: 3);",
        "temp[7] = distance_to(self.id, temp: 2);",
        "temp[0] = distance_to(stack_obj);",
    ] {
        assert!(lifted.contains(spelling), "missing {spelling}\n{lifted}");
    }
    assert!(!lifted.contains("raw {"), "{lifted}");
}

#[test]
fn lifts_sim_attributes_data_container_and_personality() {
    let iff = compile(
        r#"object "Fable Profile Card" {
    guid: 0x7E00_00B7;
    #[bhav(id = 4300)]
    fn profile(sim: Sim) {
        sim.attr[3] = 1;
        sim.data[4] = sim.attr[3];
        temp[2] = sim.container;
        sim.personality.nice = sim.personality.active;
        sim.personality.playful = sim.personality.outgoing;
        sim.personality.neat = 6;
    }
}"#,
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    for spelling in [
        "sim.attr[3] = 1;",
        "sim.allowed_heights = sim.attr[3];",
        "temp[2] = sim.container;",
        "sim.personality.nice = sim.personality.active;",
        "sim.personality.playful = sim.personality.outgoing;",
        "sim.personality.neat = 6;",
    ] {
        assert!(lifted.contains(spelling), "missing {spelling}\n{lifted}");
    }
    assert!(!lifted.contains("raw {"), "{lifted}");
}

#[test]
fn lifts_handle_flags_and_preserves_numeric_flag_operations() {
    let iff = compile(
        r#"object "Fable Switchboard" {
    guid: 0x7E00_00B8;
    #[bhav(id = 4300)]
    fn switch() {
        stack_obj.flags = self.flags;
        if stack_obj.flags.is_empty() { self.flags = stack_obj.flags; }
        raw {
            stack.data[8] flag_set 13;
            if stack.data[8] flag_test 13 { stack.data[8] flag_clear 13; }
        }
    }
}"#,
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    for spelling in [
        "stack_obj.flags = self.flags;",
        "if stack_obj.flags.is_empty() {",
        "self.flags = stack_obj.flags;",
        "stack_obj.flags.set(ObjectFlags::FloatsOnWater);",
        "stack_obj.flags.contains(ObjectFlags::FloatsOnWater)",
        "stack_obj.flags.clear(ObjectFlags::FloatsOnWater);",
    ] {
        assert!(lifted.contains(spelling), "missing {spelling}\n{lifted}");
    }
    assert!(!lifted.contains("raw {"), "{lifted}");
}

#[test]
fn comparisons_returned_whole_lift_as_return() {
    let iff = compile(
        r#"object "Fable Gauge" {
    guid: 0x7E00_00B1;
    #[slot(2)] state level: i16;
    #[bhav(id = 4300, label = "Full")]
    fn full() { return self.level > 3; }
    #[bhav(id = 4301, label = "Empty")]
    fn empty() { return !(self.level == 0); }
    #[bhav(id = 4302, label = "Low")]
    fn low() { if self.level > 3 { return false; } else { return true; } }
    #[bhav(id = 4303, label = "Rich")]
    fn rich() { return can_afford(5, category: 0); }
    #[bhav(id = 4304, label = "Poor")]
    fn poor() { if can_afford(5, category: 0) { return false; } return true; }
}"#,
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    // `return !(c);` is the `if` returning false then true: the same node and exits.
    for spelling in [
        "return self.attr_2 > 3;",
        "return !(self.attr_2 == 0);",
        "return !(self.attr_2 > 3);",
        "return can_afford(5, category: 0);",
    ] {
        assert!(lifted.contains(spelling), "missing {spelling}\n{lifted}");
    }
    // A primitive's test in the other polarity keeps its `if`.
    assert!(!lifted.contains("return !(can_afford"), "{lifted}");
}

#[test]
fn redundant_bool_return_suggestions_prove_equivalent() {
    // The lint's suggestion, `return <condition>;`, against the `if` it replaces: a
    // primitive call with a `bool` literal argument (which must not count as foldable).
    let object = |body: &str| {
        format!(
            "object \"Fable Pantry\" {{\n    guid: 0x7E00_00B2;\n    #[slot(2)] state level: i16;\n    #[bhav(id = 4300, label = \"Ran\")] fn ran() {{\n        self.level = 1;\n{body}\n    }}\n}}"
        )
    };
    let flagged = object(
        "        if stack_obj.run_functional(4, change_icon: true) { return true; }\n        return false;",
    );
    let diags = lint_v2_source(&flagged, "pantry.antics").expect("lint");
    let d = diags
        .iter()
        .find(|d| d.rule == "redundant_bool_return")
        .unwrap_or_else(|| panic!("{diags:#?}"));
    let suggestion = d.suggestion.clone().expect("suggestion");
    let rewritten = object(&format!("        {suggestion}"));
    let a = compile_v2_source("a.antics", &flagged, &PackResolverConfig::default())
        .expect("compile")
        .iff_bytes;
    let b = compile_v2_source("b.antics", &rewritten, &PackResolverConfig::default())
        .expect("compile")
        .iff_bytes;
    let report = check_iff_equivalent(&a, &b, &IffEquivOptions::default()).expect("check");
    assert_eq!(report.verdict(), IffVerdict::Equivalent, "{report:#?}");
}

#[test]
fn an_empty_string_table_lifts_empty() {
    let iff = compile(
        r#"object "Fable Sign" {
    guid: 0x7E00_00B3;
    strings 431 "Empty" {}
    #[bhav(id = 4300)]
    fn label() { action_label(table: 431, index: 1); }
}"#,
    );
    let lifted = round_trip(&iff, &DecompileV2Options::default());
    assert!(lifted.contains("strings 431 \"Empty\" {}"), "{lifted}");
}

#[test]
fn picker_dialogs_read_no_strings() {
    // A lot picker (kind 5) returns before formatting any text: an unliftable `STR#` 301
    // does not make its tree inconclusive.
    let source = r#"object "Fable Map" {
    guid: 0x7E00_00B4;
    strings 301 "Dialog prim string set" { ask = "Alpha" }
    #[bhav(id = 4300)]
    fn pick() { dialog(kind: 5, message: 1)?; }
}"#;
    let mut table = vec![0xFD, 0xFF, 2, 0];
    for (language, text) in [(1u8, "Alpha"), (2, "Alphe")] {
        table.push(language);
        table.extend_from_slice(text.as_bytes());
        table.extend_from_slice(&[0, 0]);
    }
    // Not plain: a second, deleted chunk of the ID.
    let iff = with_chunks(
        &compile(source),
        vec![ChunkData {
            tag: *b"STR#",
            id: 301,
            flags: 0x10 | sims_formats::iff::DELETED,
            label: "Dialog prim string set".to_owned(),
            data: table,
        }],
    );
    let (check, source) = check_of(&iff, 4300);
    assert!(
        matches!(check, TreeCheck::Identical | TreeCheck::Equivalent),
        "{check:?}\n{source}"
    );
}

#[test]
fn a_lifecycle_tree_written_plain_says_so() {
    // An empty tree as the object's main: no `main` block holds it.
    let iff = compile(
        r#"object "Fable Plinth" {
    guid: 0x7E00_00B5;
    #[bhav(id = 4300, label = "Rest", empty)]
    fn rest() {}
}"#,
    );
    let objd = iff.chunks.iter().find(|c| &c.tag == b"OBJD").unwrap();
    let mut data = iff.data(objd).to_vec();
    // Word 3 (main), after the 4-byte header.
    data[10..12].copy_from_slice(&4300u16.to_le_bytes());
    let iff = with_data(&iff, (*b"OBJD", objd.id), data);
    let report = decompile_iff_to_v2_report(&iff, &DecompileV2Options::default()).expect("lift");
    let tree = report.trees.iter().find(|t| t.id == 4300).unwrap();
    assert!(tree.standalone_lifecycle, "{:?}", report.trees);
    assert!(
        report.source.contains(
            "// The object's main tree, written as a plain tree: the build gives the object its own `main` tree at a new ID."
        ),
        "{}",
        report.source
    );
}
