use sims_formats::{
    behavior::{Behavior, InteractionTable, branch},
    execution::{Slots, constants, global_link},
    far,
    iff::Iff,
    sprite::{DrawGroup, DrawSprite, DrawView, Frame, Palette, SpriteSet, render},
    strings::StringTable,
};
fn header() -> Vec<u8> {
    let mut b = vec![0; 64];
    b[..13].copy_from_slice(b"IFF FILE 2.5:");
    b
}
fn add_chunk(b: &mut Vec<u8>, tag: &[u8; 4], id: u16, data: &[u8]) {
    b.extend(tag);
    b.extend(((76 + data.len()) as u32).to_be_bytes());
    b.extend(id.to_be_bytes());
    b.extend(0u16.to_be_bytes());
    b.extend([0; 64]);
    b.extend(data);
}
fn frame(width: u16, height: u16, commands: &[u16], pixels: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(width.to_le_bytes());
    b.extend(height.to_le_bytes());
    b.extend(7u32.to_le_bytes());
    b.extend([0; 8]);
    for v in commands {
        b.extend(v.to_le_bytes());
    }
    b.extend(pixels);
    b
}
fn solid(z: u8, color: u8) -> Vec<u8> {
    let mut b = frame(1, 1, &[6, 0x2001], &[z, color]);
    b.extend(0xA000u16.to_le_bytes());
    b
}
fn spr2(frame: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    for v in [1000u32, 1, 1, 16] {
        b.extend(v.to_le_bytes());
    }
    b.extend(frame);
    b
}
fn palette() -> Palette {
    Palette(vec![[0, 0, 0], [255, 0, 0], [0, 255, 0]])
}
#[test]
fn unknown_iff_chunks_are_lossless_and_zero_lengths_rejected() {
    let mut b = header();
    add_chunk(&mut b, b"Q?!z", 17, &[0xFF, 0, 42]);
    let iff = Iff::parse(b.clone()).unwrap();
    assert_eq!(iff.bytes, b);
    assert_eq!(iff.data(&iff.chunks[0]), [0xFF, 0, 42]);
    b[68..72].copy_from_slice(&0u32.to_be_bytes());
    assert!(Iff::parse(b).is_err());
}
#[test]
fn only_version_2_5_headers_name_a_resource_map() {
    let mut b = header();
    b[..13].copy_from_slice(b"IFF FILE 2.0:");
    b[56..64].copy_from_slice(b"S 1996\0\0");
    add_chunk(&mut b, b"TEST", 2, &[]);
    add_chunk(&mut b, b"TEST", 1, &[]);
    let iff = Iff::parse(b.clone()).unwrap();
    let ids: Vec<u16> = iff
        .resource_order(b"TEST")
        .unwrap()
        .iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(ids, [2, 1]);
    b[..13].copy_from_slice(b"IFF FILE 2.5:");
    assert!(Iff::parse(b).unwrap().resource_order(b"TEST").is_err());
}
#[test]
fn every_iff_truncation_inside_a_chunk_is_rejected() {
    let mut b = header();
    add_chunk(&mut b, b"TEST", 0, &[9; 30]);
    for end in 65..b.len() {
        assert!(Iff::parse(b[..end].to_vec()).is_err(), "accepted {end}");
    }
}
#[test]
fn far_directory_rejects_bad_ranges_and_hostile_counts() {
    let mut b = 1u32.to_le_bytes().to_vec();
    for v in [10u32, 10, 16, 5] {
        b.extend(v.to_le_bytes());
    }
    b.extend(b"x.iff");
    assert_eq!(far::directory(&b, 26).unwrap()[0].name, "x.iff");
    assert!(far::directory(&b, 25).is_err());
    b[..4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(far::directory(&b, 100).is_err());
    let mut h = b"FAR!byAZ".to_vec();
    h.extend(1u32.to_le_bytes());
    h.extend(99u32.to_le_bytes());
    assert!(far::manifest_offset(&h, 100).is_err());
    // The directory is the rest of the file, within the cap.
    assert_eq!(far::directory_len(16, 116).unwrap(), 100);
    assert!(far::directory_len(16, 16 + far::MAX_DIRECTORY + 1).is_err());
}
#[test]
fn sprite_preserves_color_depth_and_transparent_tail() {
    let mut b = frame(3, 2, &[8, 0x2002], &[5, 1, 9, 2]);
    b.extend(0x8001u16.to_le_bytes());
    b.extend(0xA000u16.to_le_bytes());
    let f = Frame::decode(&b, &palette()).unwrap();
    assert_eq!(&f.rgba[..8], &[255, 0, 0, 255, 0, 255, 0, 255]);
    assert_eq!(f.depth, [5, 9, 255, 255, 255, 255]);
    assert!(f.rgba[8..].iter().all(|x| *x == 0));
}
#[test]
fn sprite_alpha_and_odd_run_padding() {
    let mut b = frame(1, 1, &[8, 0x4001], &[8, 1, 15, 0]);
    b.extend(0xA000u16.to_le_bytes());
    let f = Frame::decode(&b, &palette()).unwrap();
    assert_eq!(f.rgba, [255, 0, 0, 123]);
    assert_eq!(f.depth, [8]);
}
#[test]
fn pixel_only_runs_draw_palette_zero_opaque() {
    let mut b = frame(1, 1, &[6, 0xC001], &[0, 0]);
    b.extend(0xA000u16.to_le_bytes());
    assert_eq!(Frame::decode(&b, &palette()).unwrap().rgba, [0, 0, 0, 255]);
}
#[test]
fn malformed_sprite_runs_cannot_escape_rows_or_allocations() {
    let mut b = solid(1, 1);
    b[18..20].copy_from_slice(&0x2002u16.to_le_bytes());
    assert!(Frame::decode(&b, &palette()).is_err());
    assert!(Frame::decode(&solid(1, 9), &palette()).is_err());
    let b = frame(u16::MAX, u16::MAX, &[], &[]);
    assert!(Frame::decode(&b, &palette()).is_err());
    let b = solid(1, 1);
    for end in 0..b.len() {
        if end == b.len() - 2 {
            assert!(Frame::decode(&b[..end], &palette()).is_ok());
        } else {
            assert!(Frame::decode(&b[..end], &palette()).is_err());
        }
    }
}
#[test]
fn sprite_offsets_must_follow_the_header_and_stay_inside_payload() {
    let mut b = spr2(&solid(4, 1));
    assert_eq!(SpriteSet::parse(&b).unwrap().frames.len(), 1);
    b[12..16].copy_from_slice(&4u32.to_le_bytes());
    assert!(SpriteSet::parse(&b).is_err());
}
#[test]
fn sprite_end_marker_is_optional_only_after_all_rows() {
    let mut complete = solid(4, 1);
    complete.truncate(complete.len() - 2);
    assert!(Frame::decode(&complete, &palette()).is_ok());
    complete[2..4].copy_from_slice(&2u16.to_le_bytes());
    assert!(Frame::decode(&complete, &palette()).is_err());
}
#[test]
fn composite_depth_rejects_occluded_pixels() {
    let mut b = header();
    let mut p = vec![0; 4];
    p.extend(3u32.to_le_bytes());
    p.extend([0; 8]);
    p.extend([0, 0, 0, 255, 0, 0, 0, 255, 0]);
    add_chunk(&mut b, b"PALT", 1, &p);
    add_chunk(&mut b, b"SPR2", 1, &spr2(&solid(2, 1)));
    add_chunk(&mut b, b"SPR2", 2, &spr2(&solid(10, 2)));
    let iff = Iff::parse(b).unwrap();
    let sprites = (1..=2)
        .map(|id| DrawSprite {
            id,
            frame: 0,
            x: -7,
            y: -9,
            flags: 0,
            object: [0.0; 3],
        })
        .collect();
    let p = render(
        &iff,
        &DrawView {
            direction: 1,
            zoom: 3,
            sprites,
        },
    )
    .unwrap();
    assert_eq!(p.rgba, [255, 0, 0, 255]);
    assert_eq!(p.depth, [2]);
    assert_eq!(p.origin, [-7, -9]);
}
#[test]
fn bhav_versions_and_terminal_branches() {
    for v in [0x8000u16, 0x8001, 0x8002] {
        let mut b = v.to_le_bytes().to_vec();
        b.extend(1u16.to_le_bytes());
        b.extend([0; 8]);
        b.extend([2, 0, 0xFE, 0xFF]);
        b.extend([0; 8]);
        let h = Behavior::parse(&b).unwrap();
        assert_eq!(h.instructions.len(), 1);
        assert!(h.disassemble(|_| None).contains("RETURN TRUE"));
        for n in 0..b.len() {
            assert!(Behavior::parse(&b[..n]).is_err());
        }
    }
    assert_eq!(branch(5, 1), "INVALID (5)");
    assert!(Behavior::parse(&[3, 128, 0, 0]).is_err());
}
#[test]
fn strings_preserve_source_bytes_and_fall_back_to_english() {
    let bytes = [
        0xFD, 0xFF, 2, 0, 1, b'S', b'i', b't', 0, 0, 3, b'C', b'a', b'f', 0xE9, 0, 0,
    ];
    let s = StringTable::parse(&bytes).unwrap();
    assert_eq!(s.get(0, 3), Some("Café"));
    assert_eq!(s.get(0, 19), Some("Sit"));
    assert_eq!(s.entries[1].raw_text, b"Caf\xE9");
    assert!(StringTable::parse(&bytes[..bytes.len() - 1]).is_err());
}
#[test]
fn compressed_ttab_signed_field_widths_and_truncation() {
    let mut bits = Vec::<u8>::new();
    for (v, width) in [
        (0x1001u32, 16),
        (0x1002, 16),
        (2, 32),
        (1, 32),
        (0, 32),
        (3, 32),
        (0x3F800000, 32),
        (50, 32),
        (u32::MAX, 32),
        (u32::from((-100i16) as u16), 16),
        (32767, 16),
        (12, 16),
        (0, 16),
        (u32::from((-1000i16) as u16), 16),
        (0, 16),
    ] {
        if v == 0 {
            bits.push(0);
        } else {
            bits.extend([1, 1, 1]);
            for n in (0..width).rev() {
                bits.push(((v >> n) & 1) as u8);
            }
        }
    }
    let mut b = vec![1, 0, 9, 0, 1];
    for chunk in bits.chunks(8) {
        let mut byte = 0;
        for (i, v) in chunk.iter().enumerate() {
            byte |= v << (7 - i);
        }
        b.push(byte);
    }
    let table = InteractionTable::parse(&b).unwrap();
    assert_eq!(table.interactions[0].action, 0x1001);
    assert_eq!(table.interactions[0].autonomy, 50);
    assert_eq!(table.interactions[0].flags, 1);
    let i = &table.interactions[0];
    assert_eq!(
        (i.attenuation_code, i.attenuation, i.joining_index),
        (3, 1.0, -1)
    );
    assert_eq!(
        (
            i.motives[0].minimum,
            i.motives[0].delta,
            i.motives[0].personality
        ),
        (-100, 32767, 12)
    );
    assert_eq!(
        (
            i.motives[1].minimum,
            i.motives[1].delta,
            i.motives[1].personality
        ),
        (0, -1000, 0)
    );
    assert!(InteractionTable::parse(&b[..b.len() - 1]).is_err());
}
#[test]
fn an_empty_interaction_table_keeps_its_version_or_9() {
    for (bytes, version) in [(&[0, 0, 8, 0][..], 8), (&[0, 0, 9, 0, 0], 9), (&[0, 0], 9)] {
        let table = InteractionTable::parse(bytes).unwrap();
        assert_eq!(table.version, version);
        assert!(table.interactions.is_empty());
    }
}
#[test]
fn arbitrary_bytes_do_not_panic_or_trigger_unbounded_allocations() {
    let mut seed = 12345u32;
    for n in 0..400 {
        let bytes: Vec<_> = (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 24) as u8
            })
            .collect();
        let _ = Iff::parse(bytes.clone());
        let _ = far::directory(&bytes, 100000);
        let _ = Behavior::parse(&bytes);
        let _ = StringTable::parse(&bytes);
        let _ = InteractionTable::parse(&bytes);
        let _ = SpriteSet::parse(&bytes);
        let _ = DrawGroup::parse(&bytes);
        let _ = Palette::parse(&bytes);
        let _ = Slots::parse(&bytes);
        let _ = constants(&bytes);
        let _ = global_link(&bytes);
    }
}

#[test]
fn globals_and_constants_are_bounded_and_preserve_signed_words() {
    assert_eq!(global_link(b"\x04Sofa").unwrap().unwrap(), "Sofa");
    assert_eq!(
        global_link(b"SofaGlobals\0").unwrap().unwrap(),
        "SofaGlobals"
    );
    assert!(global_link(b"\x10abc").is_err());
    assert!(global_link(b"\x03a\0b").is_err());
    assert_eq!(global_link(b"\0").unwrap(), None);
    assert_eq!(constants(&[2, 0, 255, 255, 0, 128]).unwrap(), [-1, -32768]);
    assert!(constants(&[255, 0, 0, 0]).is_err());
}

#[test]
fn slot_versions_offsets_and_truncations() {
    for (version, stride) in [
        (4u32, 34),
        (5, 34),
        (6, 54),
        (7, 58),
        (8, 62),
        (9, 66),
        (10, 70),
    ] {
        let mut b = vec![0; 4];
        b.extend(version.to_le_bytes());
        b.extend(b"TOLS");
        b.extend(1u32.to_le_bytes());
        b.extend(3u16.to_le_bytes());
        for n in [1.5f32, -2.0, 7.0] {
            b.extend(n.to_le_bytes());
        }
        for n in [1i32, 0, 0, 1, -1] {
            b.extend(n.to_le_bytes());
        }
        b.resize(16 + stride, 0);
        let slots = Slots::parse(&b).unwrap();
        assert_eq!(slots.routing(0).unwrap().offset, [1.5, -2.0, 7.0]);
        assert_eq!(slots.routing(0).unwrap().snap_target, -1);
        assert!(slots.routing(1).is_none());
        for end in 0..b.len() {
            assert!(Slots::parse(&b[..end]).is_err(), "v{version} at {end}");
        }
        b[18..22].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(Slots::parse(&b).is_err());
    }
}

fn add_flagged(b: &mut Vec<u8>, tag: &[u8; 4], id: u16, flags: u16, data: &[u8]) -> u32 {
    let offset = b.len() as u32;
    b.extend(tag);
    b.extend(((76 + data.len()) as u32).to_be_bytes());
    b.extend(id.to_be_bytes());
    b.extend(flags.to_be_bytes());
    b.extend([0; 64]);
    b.extend(data);
    offset
}
/// Appends a version-0 resource map listing `entries` as (tag, offset,
/// id, flags), each with an empty name, and points the header at it.
fn add_map(b: &mut Vec<u8>, entries: &[(&[u8; 4], u32, u16, u16)]) {
    let mut types: Vec<&[u8; 4]> = entries.iter().map(|e| e.0).collect();
    types.dedup();
    let mut map = vec![0; 8];
    map.extend(b"pmsr");
    map.extend(0u32.to_le_bytes());
    map.extend((types.len() as u32).to_le_bytes());
    for tag in types {
        let listed: Vec<_> = entries.iter().filter(|e| e.0 == tag).collect();
        map.extend(u32::from_be_bytes(*tag).to_le_bytes());
        map.extend((listed.len() as u32).to_le_bytes());
        for (_, offset, id, flags) in listed {
            map.extend(offset.to_le_bytes());
            map.extend(id.to_le_bytes());
            map.extend(flags.to_le_bytes());
            // An empty name, padded to an even length.
            map.extend([0, 0]);
        }
    }
    let at = add_flagged(b, b"rsmp", 0, 0, &map);
    b[60..64].copy_from_slice(&at.to_be_bytes());
}
fn payloads<'a>(iff: &'a Iff, tag: &'a [u8; 4]) -> Vec<&'a [u8]> {
    let all: Vec<_> = iff.all(tag).map(|c| iff.data(c)).collect();
    let ordered: Vec<_> = iff
        .resource_order(tag)
        .unwrap()
        .into_iter()
        .map(|c| iff.data(c))
        .collect();
    assert_eq!(all, ordered);
    all
}
#[test]
fn deleted_chunks_are_not_resources() {
    let mut b = header();
    add_flagged(&mut b, b"STR#", 1, 4, b"dead");
    add_flagged(&mut b, b"STR#", 1, 0, b"live");
    add_flagged(&mut b, b"STR#", 2, 4, b"gone");
    let iff = Iff::parse(b).unwrap();
    assert_eq!(iff.data(iff.get(b"STR#", 1).unwrap()), b"live");
    assert!(iff.get(b"STR#", 2).is_none());
    assert_eq!(payloads(&iff, b"STR#"), [b"live"]);
    // The chunks themselves are all still there.
    assert_eq!(iff.chunks.len(), 3);
}
#[test]
fn a_later_chunk_replaces_an_earlier_one_in_its_place() {
    let mut b = header();
    add_flagged(&mut b, b"STR#", 1, 0, b"old");
    add_flagged(&mut b, b"STR#", 2, 0, b"two");
    add_flagged(&mut b, b"STR#", 1, 0, b"new");
    let iff = Iff::parse(b).unwrap();
    assert_eq!(iff.data(iff.get(b"STR#", 1).unwrap()), b"new");
    assert_eq!(payloads(&iff, b"STR#"), [&b"new"[..], b"two"]);
}
#[test]
fn the_flags_high_byte_is_part_of_a_resource_key() {
    let mut b = header();
    add_flagged(&mut b, b"BMP_", 7, 0x0410, b"high");
    add_flagged(&mut b, b"BMP_", 7, 0x0010, b"plain");
    add_flagged(&mut b, b"BMP_", 8, 0x0100, b"only high");
    let iff = Iff::parse(b).unwrap();
    assert_eq!(iff.data(iff.get(b"BMP_", 7).unwrap()), b"plain");
    assert!(iff.get(b"BMP_", 8).is_none());
    assert_eq!(
        payloads(&iff, b"BMP_"),
        [&b"high"[..], b"plain", b"only high"]
    );
}
#[test]
fn a_resource_map_decides_lookup_as_well_as_order() {
    let mut b = header();
    let old = add_flagged(&mut b, b"STR#", 1, 0, b"old");
    let new = add_flagged(&mut b, b"STR#", 1, 0, b"new");
    let other = add_flagged(&mut b, b"STR#", 2, 0, b"two");
    // The map lists 2 first, then the replacement, then the old chunk
    // again under the same key, which the first entry keeps.
    add_map(
        &mut b,
        &[
            (b"STR#", other, 2, 0),
            (b"STR#", new, 1, 0),
            (b"STR#", old, 1, 0),
        ],
    );
    let iff = Iff::parse(b).unwrap();
    assert_eq!(iff.data(iff.get(b"STR#", 1).unwrap()), b"new");
    assert_eq!(payloads(&iff, b"STR#"), [&b"two"[..], b"new"]);
}
#[test]
fn a_resource_map_can_choose_the_earlier_chunk() {
    let mut b = header();
    let old = add_flagged(&mut b, b"STR#", 1, 0, b"old");
    add_flagged(&mut b, b"STR#", 1, 0, b"new");
    add_flagged(&mut b, b"STR#", 2, 0, b"unlisted");
    let low = add_flagged(&mut b, b"STR#", 3, 0x10, b"low bits");
    // A later entry for a listed key is dropped without being checked,
    // and the low flag bits are the map's to state.
    add_map(
        &mut b,
        &[
            (b"STR#", old, 1, 0),
            (b"STR#", 9999, 1, 0),
            (b"STR#", low, 3, 0),
        ],
    );
    let iff = Iff::parse(b).unwrap();
    // Without the map, the later chunk would replace the earlier one.
    assert_eq!(iff.data(iff.get(b"STR#", 1).unwrap()), b"old");
    assert!(iff.get(b"STR#", 2).is_none());
    assert_eq!(payloads(&iff, b"STR#"), [&b"old"[..], b"low bits"]);
}
#[test]
fn a_map_listing_a_map_is_ignored() {
    let mut b = header();
    let first = add_flagged(&mut b, b"STR#", 1, 0, b"one");
    add_flagged(&mut b, b"STR#", 2, 0, b"two");
    // Offset 0 is no chunk: the map is not checked once it lists a map.
    add_map(&mut b, &[(b"STR#", first, 1, 0), (b"rsmp", 0, 0, 0)]);
    let iff = Iff::parse(b).unwrap();
    assert_eq!(payloads(&iff, b"STR#"), [&b"one"[..], b"two"]);
    // Nor is a map ever a resource.
    assert!(iff.get(b"rsmp", 0).is_none());
    assert_eq!(iff.all(b"rsmp").count(), 0);
}
#[test]
fn lookup_survives_a_resource_map_that_cannot_be_used() {
    let mut b = header();
    add_flagged(&mut b, b"STR#", 1, 4, b"dead");
    add_flagged(&mut b, b"STR#", 1, 0, b"live");
    // The map's entry names a chunk that is not there.
    add_map(&mut b, &[(b"STR#", 9999, 1, 0)]);
    let iff = Iff::parse(b).unwrap();
    assert!(iff.resource_order(b"STR#").is_err());
    assert_eq!(iff.data(iff.get(b"STR#", 1).unwrap()), b"live");
    assert_eq!(iff.all(b"STR#").count(), 1);
}
#[test]
fn far_members_must_lie_between_the_header_and_the_directory() {
    let entry = |offset, stored_size| far::Entry {
        name: "x".into(),
        raw_name: b"x".to_vec(),
        offset,
        stored_size,
        unpacked_size: stored_size,
    };
    assert!(entry(16, 10).within_payload(26));
    // Over the header, as an archive that holds itself would be.
    assert!(!entry(0, 26).within_payload(26));
    // Into the directory.
    assert!(!entry(16, 11).within_payload(26));
    assert!(!entry(u32::MAX, u32::MAX).within_payload(u32::MAX));
}
