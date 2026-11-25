//! `lift --v2` wave 3 spellings: the lint's typed `std` constants (with their `use` lines),
//! method forms and remaining `?`, applied only where the object builds to the same bytes; and
//! PR I-4's cells and options: `sprite_flags[temp[k]]`, `sim.person[temp[k]]`,
//! `stack_obj.person[temp[k]]`, `stack_obj.neighbor.*`, `advert.delta[m]`, `fame`, `padN:`
//! bytes on `play_sound` and `clear_motive_changes`, and the parameter slots of `reach`,
//! `make_new_character` and `drop_onto`. Each is lifted in strict and in behavioral mode. Every
//! object is synthetic (GUIDs in the clean-room 0x7E00_xxxx range).

use sims_antics::compile_v2_source;
use sims_antics::v2::{
    DecompileV2Options, PackResolverConfig, TreeCheck, decompile_iff_to_v2_report, lint_v2_source,
};
use sims_formats::iff::Iff;

fn compile(source: &str) -> Iff {
    let object = compile_v2_source("wave3.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{e}\n{source}"));
    Iff::parse(object.iff_bytes).expect("parse IFF")
}

/// Lifts `source`'s object in strict or behavioral mode: every tree proven, the text compiles
/// again and lints clean under every rule.
fn lift(source: &str, behavioral: bool) -> String {
    let options = DecompileV2Options {
        behavioral,
        ..DecompileV2Options::default()
    };
    let report = decompile_iff_to_v2_report(&compile(source), &options).expect("lift");
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
    let lints = lint_v2_source(&report.source, "lifted.antics").expect("lint");
    assert!(lints.is_empty(), "{lints:?}\n{}", report.source);
    report.source
}

/// Both modes, which must agree on every spelling checked here.
fn lift_both(source: &str) -> [String; 2] {
    [lift(source, false), lift(source, true)]
}

fn has(lifted: &str, spellings: &[&str]) {
    for s in spellings {
        assert!(lifted.contains(s), "missing {s:?} in:\n{lifted}");
    }
}

fn lacks(lifted: &str, spellings: &[&str]) {
    for s in spellings {
        assert!(!lifted.contains(s), "unexpected {s:?} in:\n{lifted}");
    }
}

#[test]
fn typed_constants_bring_their_imports_and_method_forms_replace_target_arguments() {
    let source = r#"object "Fable Wave3 Lamp" {
    guid: 0x7E00_0160;
    sound chime = fwav(0, wav: "fable_chime");
    #[bhav(id = 4300)]
    fn lit() {
        raw {
            my.data[8] flag_set 6;
        }
        play_sound(chime, stack_obj);
        refresh(target: 1, mode: 2);
        debit_funds(5, category: 3);
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &[
                "use std::engine::RefreshMode;\nuse std::flags::ObjectFlags;\n\nobject",
                "self.flags flag_set ObjectFlags::InUse;",
                "stack_obj.play_sound(fable_chime);",
                "stack_obj.refresh(RefreshMode::RecountRoom);",
            ],
        );
        lacks(
            &lifted,
            &[
                "flag_set 6;",
                "play_sound(fable_chime, stack_obj)",
                "refresh(target:",
                "raw {",
            ],
        );
        // The imports follow the header comments, each once.
        assert_eq!(lifted.matches("use std::flags::ObjectFlags;").count(), 1);
        assert!(lifted.starts_with("// Decompiled by"), "{lifted}");
    }
}

#[test]
fn temp_indexed_cells_lift_to_their_names() {
    let source = r#"object "Fable Wave3 Board" {
    guid: 0x7E00_0161;
    #[bhav(id = 4300, args = 1)]
    fn t(sim: Sim) {
        temp[1] = self.sprite_flags[temp[3]];
        self.sprite_flags[temp[2]] = 1;
        temp[4] = sim.person[temp[5]];
        sim.person[temp[6]] = 2;
        temp[7] = stack_obj.person[temp[1]];
    }
    #[bhav(id = 4301)]
    fn o() {
        temp[1] = stack_obj.sprite_flags[temp[0]];
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &[
                "temp[1] = self.sprite_flags[temp[3]];",
                "self.sprite_flags[temp[2]] = 1;",
                "temp[4] = sim.person[temp[5]];",
                "sim.person[temp[6]] = 2;",
                "temp[7] = stack_obj.person[temp[1]];",
                "temp[1] = stack_obj.sprite_flags[temp[0]];",
            ],
        );
        lacks(&lifted, &["scope(", "raw {"]);
    }
}

#[test]
fn neighbor_advertisement_and_fame_cells_lift_to_their_names() {
    let source = r#"object "Fable Wave3 Desk" {
    guid: 0x7E00_0162;
    #[bhav(id = 4300, args = 1)]
    fn t(sim: Sim) {
        temp[0] = stack_obj.neighbor.field[2];
        temp[1] = stack_obj.neighbor.field[3];
        temp[2] = stack_obj.neighbor.person[10];
        temp[6] = stack_obj.neighbor.fame;
        temp[3] = sim.fame;
        sim.fame = temp[3] + 1;
        advert.delta[2] = 5;
        temp[4] = advert.personality[1];
        temp[5] = advert.minimum[15];
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &[
                // In a Sim tree the object is `self`.
                "temp[0] = self.neighbor.age;",
                "temp[1] = self.neighbor.field[3];",
                "temp[2] = self.neighbor.person[10];",
                "temp[3] = sim.fame;",
                "sim.fame = slot_0;",
                "advert.delta[2] = 5;",
                "temp[4] = advert.personality[1];",
                "temp[5] = advert.minimum[15];",
                "temp[6] = self.neighbor.fame;",
            ],
        );
        lacks(&lifted, &["scope(", "person[81]", "raw {"]);
    }
}

#[test]
fn pad_bytes_lift_as_pad_options() {
    let source = r#"object "Fable Wave3 Bell" {
    guid: 0x7E00_0163;
    #[bhav(id = 4300)]
    fn t() {
        play_sound(3, pad3: 0x01);
        play_sound(4, flags: 4, pad6: 0xff, pad7: 0xfe);
        clear_motive_changes(pad0: 0x07, pad1: 0x07, pad2: 0x05, pad4: 0x14, pad6: 0x64);
        clear_motive_changes();
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &[
                "play_sound(3, pad3: 0x01);",
                "play_sound(4, flags: 4, pad6: 0xff, pad7: 0xfe);",
                "clear_motive_changes(pad0: 0x07, pad1: 0x07, pad2: 0x05, pad4: 0x14, pad6: 0x64);",
                "clear_motive_changes();",
            ],
        );
        lacks(&lifted, &[" pad(", "raw {"]);
    }
}

#[test]
fn parameter_slots_of_reach_make_new_character_and_drop_onto_lift() {
    let source = r#"object "Fable Wave3 Crib" {
    guid: 0x7E00_0164;
    #[bhav(id = 4300, args = 2)]
    fn t(sim: Sim, a: i16, b: i16) {
        drop_onto(src: 0, dst: b);
        drop_onto(src: param[0], dst: 3);
        reach(mode: 1, moves: true, slot: b);
        reach(mode: 0, moves: false);
        let skin: i16 = 4;
        let age: i16 = 3;
        let sex: i16 = a;
        make_new_character(skin: skin, age: age, sex: sex, species: 1);
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &[
                "drop_onto(src: 0, dst: param[1]);",
                "drop_onto(src: param[0], dst: 3);",
                "reach(mode: 1, moves: true, slot: param[1]);",
                "reach(mode: 0, moves: false);",
                "make_new_character(skin: slot_",
                ", species: 1)",
            ],
        );
        lacks(&lifted, &["raw {"]);
    }
}

#[test]
fn a_comparison_looping_on_itself_keeps_its_allow() {
    let source = r#"object "Fable Wave3 Trap" {
    guid: 0x7E00_0165;
    #[bhav(id = 4300)]
    fn t() {
        temp[0] = 2;
        raw {
            @top:
            expr(temp[0] dec_gt 0) -> true: @top, false: @top;
        }
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &[
                "// antics-lint: allow(unbounded_loop_without_yield): the original's node loops on itself\n",
            ],
        );
    }
}

#[test]
fn a_result_tree_whose_failure_is_only_passed_on_lifts_with_question_mark() {
    let source = r#"use std::globals::*;

object "Fable Wave3 Bench" {
    guid: 0x7E00_0166;
    #[bhav(id = 4300)]
    fn t() {
        globals::init_object()?;
        self.weight = 1000;
    }
}"#;
    for lifted in lift_both(source) {
        has(&lifted, &["globals::init_object()?;"]);
        lacks(&lifted, &["if !globals::init_object()", "raw {"]);
    }
}

#[test]
fn make_new_character_names_locals_only_it_reads() {
    let source = r#"object "Fable Wave3 Cradle" {
    guid: 0x7E00_0167;
    #[bhav(id = 4300, args = 1, locals = 3)]
    fn t(sim: Sim) {
        raw {
            make_new_character(skin: local[0], age: local[1], sex: local[2], species: 0);
        }
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &["make_new_character(skin: slot_0, age: slot_1, sex: slot_2, species: 0);"],
        );
        lacks(&lifted, &["raw {"]);
    }
}

#[test]
fn slot_parameter_0_lifts_inside_an_animation_handler() {
    let source = r#"object "Fable Wave3 Shelf" {
    guid: 0x7E00_0168;
    #[bhav(id = 4300, args = 1)]
    fn t(sim: Sim, a: i16) {
        animate(clip: 3, table: 1, events: 2) |event| {
            reach(mode: 1, moves: true, slot: param[0]);
            drop_onto(src: param[0], dst: 3);
        }
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &[
                "reach(mode: 1, moves: true, slot: param[0]);",
                "drop_onto(src: param[0], dst: 3);",
            ],
        );
        lacks(&lifted, &["raw {"]);
    }
}

#[test]
fn an_inherit_tree_names_its_runners_temp_indexed_and_fame_words() {
    let source = r#"object "Fable Wave3 Gong" {
    guid: 0x7E00_0169;
    #[bhav(id = 4300, label = "Strike", locals = 0)]
    fn strike(sim: Sim) {
        raw {
            my.motive[1] := 2;
            if call 4301(temps) {
                temp[0] := 1;
            }
        }
    }
    #[bhav(id = 4301, label = "Hum", locals = 0)]
    fn hum() {
        raw {
            my.data[5] := 1;
            my.data[6] := 1;
            my.data[7] := 1;
            temp[1] := scope(30, 3);
            temp[2] := my.person[81];
            temp[4] := stack.person[81];
        }
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &[
                "context = inherit",
                "temp[1] = me.person[temp[3]];",
                "temp[2] = me.fame;",
                "temp[4] = stack_obj.fame;",
            ],
        );
        lacks(&lifted, &["raw {", "person[81]"]);
    }
}

#[test]
fn a_stack_object_cursor_loop_names_its_binding_s_cells() {
    let source = r#"object "Fable Wave3 Census" {
    guid: 0x7E00_016A;
    #[bhav(id = 4300)]
    fn t(sim: Sim) {
        for p in lot.sims() {
            temp[0] += p.person[temp[3]];
            temp[1] += p.neighbor.age;
            temp[2] += p.fame;
        }
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &[
                "for p in lot.sims() {",
                "p.person[temp[3]]",
                "p.neighbor.age",
                "p.fame",
            ],
        );
        lacks(&lifted, &["raw {", "set_to_next"]);
    }
}

#[test]
fn an_inherit_tree_tests_its_runner_s_type_through_me_id() {
    let source = r#"object "Fable Wave3 Kiosk" {
    guid: 0x7E00_016C;
    #[bhav(id = 4300, label = "Ask", locals = 0)]
    fn ask() {
        raw {
            if call 4301(temps) {
                temp[0] := 1;
            }
        }
    }
    #[bhav(id = 4301, label = "Kind", locals = 0)]
    fn kind() {
        raw {
            temp[1] := scope(30, 3);
            if test_object_type(0x7E00016C, my.data[11]) {
                return true;
            }
        }
        return false;
    }
}"#;
    for lifted in lift_both(source) {
        has(
            &lifted,
            &[
                "context = inherit",
                "temp[1] = me.person[temp[3]];",
                "me.id.is_type(",
            ],
        );
        lacks(&lifted, &["raw {", "data[11].is_type"]);
    }
}
