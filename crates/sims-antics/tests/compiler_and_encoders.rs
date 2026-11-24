use sims_antics::art::{DGRP_DIRECTIONS, DGRP_ZOOMS, FurnitureShape, generate_furniture_sprites};
use sims_antics::audio::{
    MAX_SAMPLE_RATE, SAMPLE_RATE, SoundPreset, SynthSpec, encode_pcm16_wav, synthesize_preset,
    synthesize_wav,
};
use sims_antics::compile::{compile_antics, compile_object_from_sources, package_far};
use sims_antics::encode::{
    encode_bcon, encode_bhav, encode_dgrp, encode_fwav, encode_iff_with_rsmp, encode_objd,
    encode_objf, encode_palt, encode_slot, encode_spr2, encode_string_table, encode_ttab,
};
use sims_antics::ir::{
    BconChunk, BhavChunk, DgrpChunk, FwavChunk, IFF_CONVERTED_FLAG, ObjdChunk, ObjdSpec, ObjfChunk,
    PaltChunk, SlotChunk, Spr2Chunk, Spr2Frame, Spr2Pixel, StringEntry, StringFormat,
    StringTableChunk, TtabChunk,
};
use sims_antics::lift::{format_bhav_antics, lift_bhav, lift_ttab};
use sims_antics::manifest::{ObjectManifest, SpriteSpec, parse_toml};
use sims_formats::behavior::{
    Behavior, Instruction, Interaction, InteractionTable, MotiveAdvertisement, ObjectDefinition,
};
use sims_formats::execution::{ObjectFunction, ObjectFunctions, Slot, Slots, constants};
use sims_formats::far;
use sims_formats::iff::{ChunkData, Iff};
use sims_formats::sprite::{DrawGroup, DrawSprite, DrawView, Palette, SpriteSet, render};
use sims_formats::strings::{Format, StringTable};

#[test]
fn bhav_formats_8000_through_8002_encode_and_8003_is_rejected() {
    let ins = Instruction {
        opcode: 2,
        on_true: 0xFE,
        on_false: 0xFF,
        operands: [1, 0, 2, 0, 0, 2, 7, 7],
    };
    for format in [0x8000u16, 0x8001, 0x8002] {
        let mut chunk = BhavChunk::new(4096, "Test", vec![ins.clone()]);
        chunk.format = format;
        chunk.kind = 1;
        chunk.args = 2;
        chunk.locals = 3;
        chunk.revision = 0x1234_5678;
        let bytes = encode_bhav(&chunk).unwrap();
        assert_eq!(bytes.len(), 24);
        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), format);
        let parsed = Behavior::parse(&bytes).unwrap();
        assert_eq!(parsed.format, format);
        assert_eq!(parsed.kind, 1);
        assert_eq!(parsed.args, 2);
        assert_eq!(parsed.locals, 3);
        assert_eq!(parsed.revision, 0x1234_5678);
        assert_eq!(parsed.instructions.len(), 1);
        assert_eq!(parsed.instructions[0].opcode, 2);
    }

    let mut unsupported = BhavChunk::new(4096, "Unsupported", vec![ins]);
    unsupported.format = 0x8003;
    assert!(encode_bhav(&unsupported).is_err());
    assert!(compile_antics(r#"tree 4096 "BadFmt" (format: 0x8003) { return true; }"#).is_err());
}

#[test]
fn string_table_all_formats_roundtrip() {
    let cases = [
        (StringFormat::Pascal, Format::Pascal),
        (StringFormat::Terminated, Format::Terminated),
        (StringFormat::Commented, Format::Commented),
        (StringFormat::Localized, Format::Localized),
        (StringFormat::MacResource, Format::MacResource),
    ];
    for (fmt, expected_fmt) in cases {
        let chunk = StringTableChunk::new(
            *b"STR#",
            300,
            "Sample",
            fmt,
            vec![
                StringEntry::with_comment("Alpha", "first note"),
                StringEntry::with_comment("Beta", "second note"),
            ],
        );
        let bytes = encode_string_table(&chunk).unwrap();
        let parsed = StringTable::parse(&bytes).unwrap();
        assert_eq!(parsed.format, expected_fmt);
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].text, "Alpha");
        assert_eq!(parsed.entries[1].text, "Beta");
        if matches!(fmt, StringFormat::Commented | StringFormat::Localized) {
            assert_eq!(parsed.entries[0].comment, "first note");
            assert_eq!(parsed.entries[1].comment, "second note");
        }
    }
}

#[test]
fn ttab_uncompressed_and_compressed_roundtrip_across_versions() {
    let interaction = Interaction {
        action: 4096,
        test: 4097,
        string_index: 0,
        flags: 0x0000_0005,
        autonomy: 50,
        attenuation_code: 1,
        attenuation: 0.025,
        joining_index: -1,
        motives: vec![
            MotiveAdvertisement {
                minimum: -100,
                delta: 45,
                personality: 0,
            },
            MotiveAdvertisement {
                minimum: -20,
                delta: -10,
                personality: 2,
            },
        ],
        flags2: 0x0000_0003,
    };

    for version in 4..=10u16 {
        for compressed in [false, true] {
            if compressed && version < 9 {
                continue;
            }
            let mut chunk = TtabChunk::new(128, "Actions", vec![interaction.clone()]);
            chunk.version = version;
            chunk.compressed = compressed;
            let bytes = encode_ttab(&chunk).unwrap();
            let parsed = InteractionTable::parse(&bytes).unwrap();
            assert_eq!(parsed.version, version);
            assert_eq!(parsed.interactions.len(), 1);
            let got = &parsed.interactions[0];
            assert_eq!(got.action, 4096);
            assert_eq!(got.test, 4097);
            assert_eq!(got.flags, 5);
            assert_eq!(got.autonomy, 50);
            assert_eq!(got.joining_index, -1);
            assert_eq!(got.attenuation.to_bits(), 0.025f32.to_bits());
            assert_eq!(got.motives.len(), 2);
            assert_eq!(got.motives[0].delta, 45);
            assert_eq!(got.motives[1].delta, -10);
            if version > 6 {
                assert_eq!(got.attenuation_code, 1);
                assert_eq!(got.motives[0].minimum, -100);
                assert_eq!(got.motives[1].personality, 2);
            }
            if version > 9 {
                assert_eq!(got.flags2, 3);
            }
        }
    }
}

#[test]
fn objd_objf_bcon_slot_and_iff_rsmp_roundtrip() {
    let mut objd = ObjdSpec::new(128, "Clean Bench", 0x7F00_0010);
    objd.price = 175;
    objd.base_graphic_id = 100;
    objd.tree_table_id = 128;
    objd.slots_id = 128;
    objd.catalog_strings_id = 2000;
    let objd_bytes = encode_objd(&objd.to_chunk()).unwrap();
    let parsed_objd = ObjectDefinition::parse(&objd_bytes).unwrap();
    assert_eq!(parsed_objd.guid, 0x7F00_0010);
    assert_eq!(parsed_objd.price, 175);
    assert_eq!(parsed_objd.graphic, 100);
    assert_eq!(parsed_objd.table, 128);
    assert_eq!(parsed_objd.slots, 128);
    assert_eq!(parsed_objd.catalog, 2000);

    let objf = ObjfChunk::new(
        128,
        "Functions",
        vec![
            ObjectFunction {
                condition: 0,
                action: 4096,
            },
            ObjectFunction {
                condition: 0,
                action: 4097,
            },
        ],
    );
    let objf_bytes = encode_objf(&objf).unwrap();
    let parsed_objf = ObjectFunctions::parse(&objf_bytes).unwrap();
    assert_eq!(parsed_objf.entries, objf.entries);

    let bcon = BconChunk::new(4096, "Tuning", vec![-100, 0, 42, 1000]);
    let bcon_bytes = encode_bcon(&bcon).unwrap();
    assert_eq!(constants(&bcon_bytes).unwrap(), vec![-100, 0, 42, 1000]);

    for version in 4..=10u32 {
        let slot = SlotChunk {
            id: 128,
            label: "Slots".into(),
            reserved: 0,
            version,
            entries: vec![Slot {
                kind: 3,
                offset: [0.0, -0.75, 0.0],
                standing: 1,
                sitting: 1,
                ground: 1,
                flags: 1,
                snap_target: -1,
                proximity: [16, 32, 48],
                max_size: 100,
                unknown: 0,
                gradient: 0.5,
                height: 5,
                facing: 2,
                resolution: 16,
            }],
            trailing: Vec::new(),
        };
        let slot_bytes = encode_slot(&slot).unwrap();
        let parsed_slots = Slots::parse(&slot_bytes).unwrap();
        assert_eq!(parsed_slots.version, version);
        assert_eq!(parsed_slots.entries.len(), 1);
        assert_eq!(parsed_slots.entries[0].kind, 3);
        assert_eq!(parsed_slots.entries[0].offset, [0.0, -0.75, 0.0]);
    }

    let iff_bytes = encode_iff_with_rsmp(&[
        ChunkData {
            tag: *b"OBJD",
            id: 128,
            flags: IFF_CONVERTED_FLAG,
            label: "Clean Bench".into(),
            data: objd_bytes,
        },
        ChunkData {
            tag: *b"BCON",
            id: 4096,
            flags: IFF_CONVERTED_FLAG,
            label: "Tuning".into(),
            data: bcon_bytes,
        },
    ])
    .unwrap();
    let iff = Iff::parse(iff_bytes.clone()).unwrap();
    assert!(iff.map_error().is_none());
    assert_eq!(iff.resource_order(b"OBJD").unwrap().len(), 1);
    assert_eq!(iff.resource_order(b"BCON").unwrap().len(), 1);

    // FAR archive roundtrip
    let far_bytes = package_far(&[("bench.iff", &iff_bytes)]).unwrap();
    let offset = far::manifest_offset(&far_bytes[..16], far_bytes.len() as u64).unwrap();
    let dir_len = far::directory_len(offset, far_bytes.len() as u64).unwrap();
    let entries = far::directory(
        &far_bytes[offset as usize..offset as usize + dir_len],
        far_bytes.len() as u64,
    )
    .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "bench.iff");
    assert!(entries[0].within_payload(offset));
    let start = entries[0].offset as usize;
    let end = start + entries[0].stored_size as usize;
    assert_eq!(&far_bytes[start..end], iff_bytes.as_slice());
}

#[test]
fn spr2_v1000_v1001_and_dgrp_versions_roundtrip_and_render() {
    let palt = PaltChunk {
        id: 100,
        label: "Pal".into(),
        colors: vec![[0, 0, 0], [200, 120, 60], [90, 180, 220]],
    };
    let frame = Spr2Frame {
        width: 4,
        height: 3,
        flags: 0x07,
        palette_id: 100,
        transparent_index: 0,
        x: -2,
        y: -3,
        pixels: vec![
            // Row 0: fully transparent
            Spr2Pixel::default(),
            Spr2Pixel::default(),
            Spr2Pixel::default(),
            Spr2Pixel::default(),
            // Row 1: transparent, opaque, alpha, transparent
            Spr2Pixel::default(),
            Spr2Pixel {
                color: 1,
                depth: 80,
                alpha: 31,
            },
            Spr2Pixel {
                color: 2,
                depth: 90,
                alpha: 16,
            },
            Spr2Pixel::default(),
            // Row 2: 3 translucent pixels (odd count exercises padding byte)
            Spr2Pixel {
                color: 1,
                depth: 70,
                alpha: 20,
            },
            Spr2Pixel {
                color: 2,
                depth: 71,
                alpha: 20,
            },
            Spr2Pixel {
                color: 1,
                depth: 72,
                alpha: 20,
            },
            Spr2Pixel::default(),
        ],
    };
    for spr_version in [1000u32, 1001] {
        let spr2 = Spr2Chunk {
            id: 100,
            label: "Spr".into(),
            version: spr_version,
            default_palette: 100,
            frames: vec![frame.clone()],
        };
        let palt_bytes = encode_palt(&palt).unwrap();
        let spr2_bytes = encode_spr2(&spr2).unwrap();
        let parsed_palt = Palette::parse(&palt_bytes).unwrap();
        assert_eq!(parsed_palt.0.len(), 3);
        let parsed_spr = SpriteSet::parse(&spr2_bytes).unwrap();
        assert_eq!(parsed_spr.version, spr_version);
        assert_eq!(parsed_spr.frames.len(), 1);
        let iff = Iff::parse(
            encode_iff_with_rsmp(&[
                ChunkData {
                    tag: *b"PALT",
                    id: 100,
                    flags: IFF_CONVERTED_FLAG,
                    label: "Pal".into(),
                    data: palt_bytes,
                },
                ChunkData {
                    tag: *b"SPR2",
                    id: 100,
                    flags: IFF_CONVERTED_FLAG,
                    label: "Spr".into(),
                    data: spr2_bytes,
                },
            ])
            .unwrap(),
        )
        .unwrap();
        let decoded = parsed_spr
            .decode(iff.data(iff.get(b"SPR2", 100).unwrap()), 0, &iff)
            .unwrap();
        assert_eq!(decoded.width, 4);
        assert_eq!(decoded.height, 3);
        assert_eq!(decoded.x, -2);
        assert_eq!(decoded.y, -3);
        assert_eq!(decoded.depth[4 + 1], 80);
        assert_eq!(decoded.rgba[(4 + 1) * 4 + 3], 255);
    }

    for dgrp_version in 20000u16..=20004 {
        let dgrp = DgrpChunk {
            id: 100,
            label: "Group".into(),
            version: dgrp_version,
            views: vec![DrawView {
                direction: 1,
                zoom: 3,
                sprites: vec![DrawSprite {
                    id: 100,
                    frame: 0,
                    x: -16,
                    y: -32,
                    flags: 0,
                    object: [0.0, 0.0, 0.0],
                }],
            }],
        };
        let bytes = encode_dgrp(&dgrp).unwrap();
        let parsed = DrawGroup::parse(&bytes).unwrap();
        assert_eq!(parsed.version, dgrp_version);
        assert_eq!(parsed.views.len(), 1);
        assert_eq!(parsed.views[0].sprites[0].id, 100);
    }
}

#[test]
fn procedural_isometric_sprites_render_all_4_rotations_and_3_zooms() {
    for shape in [
        FurnitureShape::Bench,
        FurnitureShape::Cot,
        FurnitureShape::Basin,
    ] {
        let spec = SpriteSpec {
            shape,
            primary_rgb: [148, 98, 56],
            accent_rgb: [204, 168, 120],
            trim_rgb: [72, 48, 32],
        };
        let generated = generate_furniture_sprites(&spec, 100, 100, 100, "Fixture");
        assert_eq!(generated.spr2.frames.len(), 12);
        assert_eq!(generated.dgrp.views.len(), 12);
        assert!(generated.palt.colors.len() >= 4 && generated.palt.colors.len() <= 256);

        let iff = Iff::parse(
            encode_iff_with_rsmp(&[
                ChunkData {
                    tag: *b"PALT",
                    id: 100,
                    flags: IFF_CONVERTED_FLAG,
                    label: generated.palt.label.clone(),
                    data: encode_palt(&generated.palt).unwrap(),
                },
                ChunkData {
                    tag: *b"SPR2",
                    id: 100,
                    flags: IFF_CONVERTED_FLAG,
                    label: generated.spr2.label.clone(),
                    data: encode_spr2(&generated.spr2).unwrap(),
                },
                ChunkData {
                    tag: *b"DGRP",
                    id: 100,
                    flags: IFF_CONVERTED_FLAG,
                    label: generated.dgrp.label.clone(),
                    data: encode_dgrp(&generated.dgrp).unwrap(),
                },
            ])
            .unwrap(),
        )
        .unwrap();

        let dgrp = DrawGroup::parse(iff.data(iff.get(b"DGRP", 100).unwrap())).unwrap();
        for &dir in &DGRP_DIRECTIONS {
            for &zoom in &DGRP_ZOOMS {
                let view = dgrp
                    .views
                    .iter()
                    .find(|v| v.direction == dir && v.zoom == zoom)
                    .expect("missing rotation/zoom view");
                let preview = render(&iff, view).unwrap();
                let opaque = preview
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|px| px[3] > 0)
                    .count();
                assert!(
                    opaque > 20,
                    "shape {shape:?} dir {dir} zoom {zoom} rendered blank"
                );
            }
        }
    }
}

#[test]
fn synthesized_wav_presets_emit_valid_deterministic_riff_wave() {
    for preset in [
        SoundPreset::UiClick,
        SoundPreset::Chime,
        SoundPreset::WoodCreak,
        SoundPreset::WaterFlush,
        SoundPreset::SoftThud,
    ] {
        let wav1 = synthesize_preset(preset);
        let wav2 = synthesize_preset(preset);
        assert_eq!(wav1, wav2, "WAV synthesis must be bit-deterministic");
        assert!(wav1.len() > 44);
        assert_eq!(&wav1[0..4], b"RIFF");
        assert_eq!(&wav1[8..12], b"WAVE");
        assert_eq!(&wav1[12..16], b"fmt ");
        assert_eq!(u16::from_le_bytes([wav1[20], wav1[21]]), 1); // PCM
        assert_eq!(u16::from_le_bytes([wav1[22], wav1[23]]), 1); // mono
        assert_eq!(
            u32::from_le_bytes([wav1[24], wav1[25], wav1[26], wav1[27]]),
            SAMPLE_RATE
        );
        assert_eq!(&wav1[36..40], b"data");
        let data_len = u32::from_le_bytes([wav1[40], wav1[41], wav1[42], wav1[43]]) as usize;
        assert_eq!(wav1.len(), 44 + data_len);
        // Verify non-silent audio energy
        let peak = wav1[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b).unsigned_abs())
            .max()
            .unwrap_or(0);
        assert!(
            peak > 500,
            "preset {preset:?} produced near-silent waveform"
        );
    }
}

#[test]
fn low_level_and_high_level_antics_compile_and_diagnose_errors() {
    // Low-level exact roundtrip
    let low_src = r#"
tree 4096 "ExactTree" (format: 0x8002, kind: 0, args: 1, locals: 0, revision: 0x00000001) {
    @entry:
    expr(my.attr[0] := 7) -> true: @sleep_step, false: error;
    @sleep_step:
    sleep(param[0]) -> true: true, false: false;
}
bcon 4096 "Rates" (flags: 0) = [5, 10, 25];
"#;
    let compiled = compile_antics(low_src).unwrap();
    let bhav = &compiled.behaviors[&4096];
    assert_eq!(bhav.instructions.len(), 2);
    assert_eq!(bhav.instructions[0].opcode, 2);
    assert_eq!(bhav.instructions[0].on_true, 1);
    assert_eq!(bhav.instructions[0].on_false, 0xFD);
    assert_eq!(bhav.instructions[1].opcode, 0);
    assert_eq!(bhav.instructions[1].on_true, 0xFE);
    assert_eq!(compiled.constants[&4096].values, vec![5, 10, 25]);

    // Lift back to text and recompile -> byte-identical BHAV!
    let orig_bytes = encode_bhav(bhav).unwrap();
    let lifted = lift_bhav(4096, "ExactTree", &orig_bytes).unwrap();
    let lifted_text = format_bhav_antics(&lifted);
    let recompiled = compile_antics(&lifted_text).unwrap();
    let re_bytes = encode_bhav(&recompiled.behaviors[&4096]).unwrap();
    assert_eq!(orig_bytes, re_bytes);

    // Unresolved label produces span diagnostic
    let bad_label = r#"
tree 4096 "Broken" {
    @start:
    op 0x0002 (0, 0, 0, 0, 0, 2, 7, 7) -> true: @missing_label, false: false;
}
"#;
    let err = compile_antics(bad_label).unwrap_err();
    assert!(err.span.is_some());
    assert!(err.message.contains("missing_label"), "{err}");

    // Out-of-bounds node index rejected
    let bad_node = r#"
tree 4096 "OutOfBounds" {
    op 0x0002 (0, 0, 0, 0, 0, 2, 7, 7) -> true: 99, false: false;
}
"#;
    let err = compile_antics(bad_node).unwrap_err();
    assert!(err.message.contains("out of bounds"), "{err}");
}

#[test]
fn toml_manifest_parser_handles_tables_arrays_and_inline_tables() {
    let toml_src = r#"
# Clean-room object manifest
[object]
name = "Open Pine Bench"
description = "Clean-room starter seating."
guid = 0x7F00_0001
price = 120
init = "Init"
main = "Main"
scripts = ["bench.antics"]

[sprite]
shape = "bench"
primary_rgb = [148, 98, 56]
accent_rgb = [196, 152, 102]
trim_rgb = [72, 48, 30]

[[interaction]]
name = "Sit and Rest"
action = "SitAndRest"
test = "CanSit"
autonomy = 60
attenuation = 0.02
motives = [
    { motive = "comfort", min = -100, delta = 45, personality = 0 },
]

[[slot]]
kind = 3
offset = [0.0, -0.75, 0.0]
facing = 0

[[sound]]
id = 1
name = "open_bench_creak"
preset = "wood_creak"
"#;
    let raw = parse_toml(toml_src).unwrap();
    assert!(raw.contains_key("object"));
    let manifest = ObjectManifest::parse(toml_src).unwrap();
    assert_eq!(manifest.guid, 0x7F00_0001);
    assert_eq!(manifest.interactions.len(), 1);
    assert_eq!(manifest.interactions[0].motives[0].motive_index, 6);
    assert_eq!(manifest.slots.len(), 1);
    assert_eq!(manifest.sounds.len(), 1);
}

#[test]
fn sol_round1_compiler_and_encoder_regressions() {
    // 1. P0: Label before `return false;` must return false (0xFF), never fall through or return true.
    let labeled_ret = compile_antics(
        r#"
tree 4096 "LabeledReturn" {
    @start:
    return false;
    my.attr[0] := 99;
}
"#,
    )
    .unwrap();
    let bhav = &labeled_ret.behaviors[&4096];
    assert_eq!(bhav.instructions[0].on_true, 0xFF);
    assert_eq!(bhav.instructions[0].on_false, 0xFF);

    // 2. P0: Two-part BCON addressing uses (table << 7) | entry and validates bounds.
    let bcon_tree = compile_antics(
        r#"
tree 4096 "ReadBcon" {
    my.attr[0] := bcon[1:0];
    my.attr[1] := bcon[64:5];
    return true;
}
"#,
    )
    .unwrap();
    let ops0 = bcon_tree.behaviors[&4096].instructions[0].operands;
    let ops1 = bcon_tree.behaviors[&4096].instructions[1].operands;
    assert_eq!(u16::from_le_bytes([ops0[2], ops0[3]]), 128);
    assert_eq!(u16::from_le_bytes([ops1[2], ops1[3]]), (64 << 7) | 5);
    assert!(compile_antics(r#"tree 4096 "Bad" { my.attr[0] := bcon[192:0]; }"#).is_err());
    assert!(compile_antics(r#"tree 4096 "Bad" { my.attr[0] := bcon[0:128]; }"#).is_err());

    // 3. P0: Stale compressed TTAB hints must not corrupt mutated fields.
    let mut base_ttab = TtabChunk::new(
        128,
        "T",
        vec![Interaction {
            action: 4, // fits in 5-bit signed hint
            test: 0,
            string_index: 0,
            flags: 0,
            autonomy: 50,
            attenuation_code: 0,
            attenuation: 0.0,
            joining_index: -1,
            motives: Vec::new(),
            flags2: 0,
        }],
    );
    base_ttab.version = 10;
    base_ttab.compressed = true;
    let bytes = encode_ttab(&base_ttab).unwrap();
    let mut lifted = lift_ttab(128, "T", &bytes).unwrap();
    lifted.interactions[0].action = 4096; // no longer fits in 5-bit hint!
    let reencoded = encode_ttab(&lifted).unwrap();
    let reparsed = InteractionTable::parse(&reencoded).unwrap();
    assert_eq!(reparsed.interactions[0].action, 4096);

    // 4. P0 & P1: Fresh IFF rejects duplicate resources, preserves distinct variants & flags == 0.
    let mac_str = encode_string_table(&StringTableChunk::new(
        *b"STR#",
        300,
        "Mac",
        StringFormat::MacResource,
        vec![StringEntry::english("OnlyOne")],
    ))
    .unwrap();
    let iff_bytes = encode_iff_with_rsmp(&[
        ChunkData {
            tag: *b"STR#",
            id: 300,
            flags: 0, // big-endian Mac resource: must NOT be mutated to 0x10!
            label: "Mac".into(),
            data: mac_str,
        },
        ChunkData {
            tag: *b"BMP_",
            id: 1,
            flags: IFF_CONVERTED_FLAG,
            label: "V0".into(),
            data: vec![1, 2],
        },
        ChunkData {
            tag: *b"BMP_",
            id: 1,
            flags: 0x0210, // distinct variant 2: must NOT replace variant 0!
            label: "V2".into(),
            data: vec![3, 4],
        },
    ])
    .unwrap();
    let iff = Iff::parse(iff_bytes).unwrap();
    assert!(iff.map_error().is_none());
    let str_chunk = iff.get(b"STR#", 300).unwrap();
    assert_eq!(iff.resource_flags(str_chunk), 0);
    let parsed_mac =
        StringTable::parse_chunk(iff.data(str_chunk), iff.resource_flags(str_chunk)).unwrap();
    assert_eq!(parsed_mac.entries.len(), 1);
    assert_eq!(parsed_mac.entries[0].text, "OnlyOne");
    assert_eq!(iff.resource_order(b"BMP_").unwrap().len(), 2);

    // Duplicate resource identity rejected
    assert!(
        encode_iff_with_rsmp(&[
            ChunkData {
                tag: *b"FWAV",
                id: 1,
                flags: IFF_CONVERTED_FLAG,
                label: "A".into(),
                data: b"a\0".to_vec(),
            },
            ChunkData {
                tag: *b"FWAV",
                id: 1,
                flags: IFF_CONVERTED_FLAG,
                label: "B".into(),
                data: b"b\0".to_vec(),
            },
        ])
        .is_err()
    );

    // 5. P1: Frame inference rejects unaddressable indices and format limits.
    assert!(compile_antics(r#"tree 4096 "P" { my.attr[0] := param[255]; }"#).is_err());
    assert!(compile_antics(r#"tree 4096 "L" { local[255] := 1; }"#).is_err());
    assert!(compile_antics(r#"tree 4096 "L8000" (format: 0x8000) { local[0] := 1; }"#).is_err());
    assert!(compile_antics(r#"tree 4096 "L8001" (format: 0x8001) { local[4] := 1; }"#).is_err());
    assert!(
        compile_antics(r#"tree 4096 "L256" (format: 0x8002, locals: 256) { return true; }"#)
            .is_err()
    );

    // 6. P1: Call syntax preserves `temps` in conditions & rejects sentinel collision.
    let cond_temps = compile_antics(
        r#"
tree 4096 "CondTemps" {
    if call 4097(temps) {
        return true;
    }
    return false;
}
"#,
    )
    .unwrap();
    assert_eq!(
        cond_temps.behaviors[&4096].instructions[0].operands,
        [0xFF; 8]
    );
    assert!(compile_antics(r#"tree 4096 "Mix" { call 4097(1, temps); }"#).is_err());
    assert!(compile_antics(r#"tree 4096 "MixSub" { subroutine(4097, temps, 1); }"#).is_err());
    assert!(compile_antics(r#"tree 4096 "Coll" { call 4097(-1, -1, -1, -1); }"#).is_err());

    // 7. P1: Dialog operand layout places message in byte 2 and title/buttons in bytes 0,3,4,6.
    let dlg = compile_antics(
        r#"
tree 4096 "Dlg" {
    dialog(0, 1);
    dialog(kind: 1, message: 2, title: 3, button1: 4, button2: 5, button3: 6, flags: 1);
    return true;
}
"#,
    )
    .unwrap();
    let d0 = dlg.behaviors[&4096].instructions[0].operands;
    assert_eq!(d0, [0, 0, 1, 0, 0, 0, 0, 0]);
    let d1 = dlg.behaviors[&4096].instructions[1].operands;
    assert_eq!(d1, [6, 0, 2, 4, 5, 1, 3, 1]);

    // 8. P1: Manifest validates sections and compiles [[constant]] and [[callback]].
    assert!(
        ObjectManifest::parse(
            r#"
[object]
name = "Bad"
guid = 0x7F00_0099
[interaction]
name = "WrongSectionType"
action = 4096
"#
        )
        .is_err()
    );
    assert!(
        ObjectManifest::parse(
            r#"
[object]
name = "Bad"
guid = 0x7F00_0099
[unknown_section]
foo = 1
"#
        )
        .is_err()
    );
    let obj_with_consts = compile_object_from_sources(
        r#"
[object]
name = "With Consts"
guid = 0x7F00_0098
init = "Init"

[[constant]]
id = 4096
label = "Rates"
values = [10, 20, 30]

[[callback]]
index = 7
action = "RoomsChanged"
"#,
        &[(
            "s.antics",
            r#"
tree 4096 "Init" { return true; }
tree 4097 "RoomsChanged" { return true; }
"#,
        )],
    )
    .unwrap();
    let compiled_iff = Iff::parse(obj_with_consts.iff_bytes).unwrap();
    assert_eq!(
        constants(compiled_iff.data(compiled_iff.get(b"BCON", 4096).unwrap())).unwrap(),
        vec![10, 20, 30]
    );
    let objf =
        ObjectFunctions::parse(compiled_iff.data(compiled_iff.get(b"OBJf", 128).unwrap())).unwrap();
    assert_eq!(objf.entries[0].action, 4096);
    assert_eq!(objf.entries[7].action, 4097);

    // 9. P1: Depth limits & iterative negation prevent stack overflow.
    let deep_not = format!(
        "tree 4096 \"N\" {{ if {}my.attr[0] == 1 {{ return true; }} return false; }}",
        "!".repeat(512)
    );
    assert!(compile_antics(&deep_not).is_ok());
    let deep_if = format!(
        "tree 4096 \"Deep\" {{ {} return true; {} }}",
        "if my.attr[0] == 1 { ".repeat(80),
        "}".repeat(80)
    );
    assert!(compile_antics(&deep_if).is_err());
    let deep_toml = format!("a = {}{}", "[".repeat(80), "]".repeat(80));
    assert!(parse_toml(&deep_toml).is_err());

    // 10. P1: Encoder & audio bounds.
    let bad_objd_v137 = ObjdChunk {
        id: 128,
        label: "Bad".into(),
        version: 137,
        words: vec![0; 108],
        trailing_byte: None,
    };
    assert!(encode_objd(&bad_objd_v137).is_err());
    let short_objd = ObjdChunk {
        id: 128,
        label: "Short".into(),
        version: 138,
        words: vec![0; 10],
        trailing_byte: None,
    };
    assert!(encode_objd(&short_objd).is_err());
    let huge_spr2 = Spr2Chunk {
        id: 100,
        label: "Huge".into(),
        version: 1001,
        default_palette: 100,
        frames: vec![Spr2Frame {
            width: 5000,
            height: 5000,
            flags: 7,
            palette_id: 100,
            transparent_index: 0,
            x: 0,
            y: 0,
            pixels: Vec::new(),
        }],
    };
    assert!(encode_spr2(&huge_spr2).is_err());
    // FWAV rejects non-Latin-1 (e.g. '€') so sims-vm decoding matches sounds_wav keys.
    assert!(
        encode_fwav(&FwavChunk {
            id: 1,
            label: "Euro".into(),
            sound_name: "price_€".into(),
        })
        .is_err()
    );
    // Custom WAV bounds reject huge durations & invalid rates before allocating.
    assert!(
        synthesize_wav(&SynthSpec {
            sample_rate: MAX_SAMPLE_RATE,
            duration_ms: u32::MAX,
            start_hz: 440.0,
            end_hz: 440.0,
            harmonic_mix: 0.0,
            noise_mix: 0.0,
            attack_ms: 1,
            decay_rate: 1.0,
            amplitude: 0.5,
            seed: 1,
        })
        .is_err()
    );
    assert!(encode_pcm16_wav(u32::MAX, &[0, 1]).is_err());

    // 11. Sol Round 2 P1 fixes: implicit param[0] in set_to_next(6) and animate(),
    // plus rejecting raw `op` mixed with high-level `idle()`.
    let stn6 = compile_antics(
        r#"
tree 4096 "Stn6Idle" {
    idle(2);
    set_to_next(6);
    return true;
}
"#,
    )
    .unwrap();
    assert!(
        stn6.behaviors[&4096].args >= 1,
        "set_to_next(6) must infer args >= 1"
    );
    assert!(
        stn6.behaviors[&4096].locals >= 1,
        "set_to_next(6) + idle() must allocate scratch local to preserve param[0]"
    );

    let anim_p0 = compile_antics(
        r#"
tree 4096 "AnimParam0Idle" {
    idle(2);
    animate(clip: 0, flags: 4);
    return true;
}
"#,
    )
    .unwrap();
    assert!(anim_p0.behaviors[&4096].args >= 1);
    assert!(
        anim_p0.behaviors[&4096].locals >= 1,
        "animate(clip: 0, flags: 4) + idle() must preserve param[0]"
    );

    let anim_ev = compile_antics(
        r#"
tree 4096 "AnimEvents" {
    animate(clip: 1, events: 2);
    return true;
}
"#,
    )
    .unwrap();
    assert!(
        anim_ev.behaviors[&4096].args >= 1,
        "foreground animate with events != 0 writes param[0], so args >= 1 must be inferred"
    );

    // Mixing raw `op` and high-level `idle()` is rejected so raw locals cannot be clobbered.
    assert!(
        compile_antics(
            r#"
tree 4096 "MixedRawIdle" (locals: 1) {
    op 0x0002 (0, 0, 7, 0, 0, 5, 25, 7) -> true: 1, false: 1;
    idle(2);
    my.attr[0] := param[0];
    return true;
}
"#
        )
        .is_err()
    );
}

/// `compile_pack_dir` rejects a catalog variant whose GUID another object of the pack
/// also defines, not only clashing primary and tile GUIDs.
#[test]
fn pack_dir_rejects_variant_guid_collisions_across_objects() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("variant-guid-collision-{}", std::process::id()));
    let pack = |name: &str, second_guid: &str| {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("objects")).expect("mkdir objects");
        std::fs::write(
            dir.join("pack.toml"),
            "[pack]\nid = \"variant_guids\"\nname = \"Variant GUIDs\"\nversion = \"0.1.0\"\n\
             archive = \"variant-guids.far\"\nobjects = [\"first\", \"second\"]\n",
        )
        .expect("write pack.toml");
        std::fs::write(
            dir.join("objects/first.antics"),
            "object \"First\" {\n    guid: 0x7F51_0001;\n    state level: i16;\n    \
             init { self.level = 1; }\n    variant \"First Variant\" { guid: 0x7F51_0002 }\n}\n",
        )
        .expect("write first");
        std::fs::write(
            dir.join("objects/second.antics"),
            format!(
                "object \"Second\" {{\n    guid: {second_guid};\n    state level: i16;\n    \
                 init {{ self.level = 2; }}\n}}\n"
            ),
        )
        .expect("write second");
        dir
    };
    let clash = sims_antics::compile_pack_dir(&pack("clash", "0x7F51_0002"))
        .expect_err("a variant GUID shared with another object must be rejected");
    assert!(
        clash.to_string().contains("duplicate GUID 0x7f510002"),
        "{clash}"
    );
    let distinct = sims_antics::compile_pack_dir(&pack("distinct", "0x7F51_0003"))
        .expect("distinct GUIDs compile");
    assert_eq!(distinct.objects.len(), 2);
}
