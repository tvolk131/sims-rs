use sims_antics::art::{DGRP_DIRECTIONS, DGRP_ZOOMS, FurnitureShape, generate_furniture_sprites};
use sims_antics::audio::{SAMPLE_RATE, SoundPreset, synthesize_preset};
use sims_antics::compile::{compile_antics, package_far};
use sims_antics::encode::{
    encode_bcon, encode_bhav, encode_dgrp, encode_iff_with_rsmp, encode_objd, encode_objf,
    encode_palt, encode_slot, encode_spr2, encode_string_table, encode_ttab,
};
use sims_antics::ir::{
    BconChunk, BhavChunk, DgrpChunk, IFF_CONVERTED_FLAG, ObjdSpec, ObjfChunk, PaltChunk, SlotChunk,
    Spr2Chunk, Spr2Frame, Spr2Pixel, StringEntry, StringFormat, StringTableChunk, TtabChunk,
};
use sims_antics::lift::{format_bhav_antics, lift_bhav};
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
fn bhav_formats_8000_through_8003_encode_cleanly() {
    let ins = Instruction {
        opcode: 2,
        on_true: 0xFE,
        on_false: 0xFF,
        operands: [1, 0, 2, 0, 0, 2, 7, 7],
    };
    for format in [0x8000u16, 0x8001, 0x8002, 0x8003] {
        let mut chunk = BhavChunk::new(4096, "Test", vec![ins.clone()]);
        chunk.format = format;
        chunk.kind = 1;
        chunk.args = 2;
        chunk.locals = 3;
        chunk.revision = 0x1234_5678;
        let bytes = encode_bhav(&chunk).unwrap();
        assert_eq!(bytes.len(), 24);
        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), format);
        if format <= 0x8002 {
            let parsed = Behavior::parse(&bytes).unwrap();
            assert_eq!(parsed.format, format);
            assert_eq!(parsed.kind, 1);
            assert_eq!(parsed.args, 2);
            assert_eq!(parsed.locals, 3);
            assert_eq!(parsed.revision, 0x1234_5678);
            assert_eq!(parsed.instructions.len(), 1);
            assert_eq!(parsed.instructions[0].opcode, 2);
        }
    }
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
