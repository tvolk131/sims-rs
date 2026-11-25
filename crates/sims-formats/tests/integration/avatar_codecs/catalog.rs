//! The binary avatar catalog (`.bcf`) group reader.

use sims_formats::avatar::catalog::{Asset, Binary};
use sims_formats::avatar::index::Kind;

fn word(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_le_bytes());
}

/// A short read of a count is the native reader's "none": what was read
/// before it stays read, and the next count read is an error.
#[test]
fn catalog_keeps_what_it_read_before_a_later_short_read() {
    let mut bytes = (-1i32).to_le_bytes().to_vec();
    bytes.extend(1i32.to_le_bytes());
    bytes.extend(b"\x05MiXeD");
    for value in [7, 0, 0] {
        word(&mut bytes, value);
    }
    let mut reader = Binary::new(&bytes);
    assert_eq!(reader.count(Kind::Skeleton).unwrap(), Some(-1));
    assert_eq!(reader.count(Kind::Appearance).unwrap(), Some(1));
    let asset = reader.read(Kind::Appearance).unwrap();
    assert!(matches!(asset, Asset::Appearance(_)));
    // The appearance reader lowercases its name.
    assert_eq!(asset.name(), "mixed");
    assert_eq!(reader.count(Kind::Animation).unwrap(), None);
    assert_eq!(reader.position(), bytes.len());
    assert!(reader.count(Kind::Animation).is_err());
}

/// A catalog with one skeleton, one appearance and one animation decodes
/// all three groups in order and leaves whatever follows unread.
#[test]
fn a_mixed_catalog_decodes_all_three_groups_and_leaves_the_tail_unread() {
    let mut bytes = Vec::new();
    word(&mut bytes, 1);
    bytes.extend(b"\x05adult");
    word(&mut bytes, 1);
    bytes.extend(b"\x04ROOT\x04NULL");
    word(&mut bytes, 0);
    for value in [0f32, 0., 0., 0., 0., 0., 1.] {
        word(&mut bytes, value.to_bits());
    }
    for value in [1, 1, 0, 0, 0] {
        word(&mut bytes, value);
    }
    word(&mut bytes, 1);
    bytes.extend(b"\x04BODY");
    for value in [7, 0, 0] {
        word(&mut bytes, value);
    }
    word(&mut bytes, 1);
    bytes.extend(b"\x04walk\x07samples");
    for value in [1000f32.to_bits(), 0, 0, 0, 0, 1] {
        word(&mut bytes, value);
    }
    bytes.extend(b"\x04ROOT");
    for value in [1, 1000f32.to_bits(), 0, 0, u32::MAX, u32::MAX, 0, 0] {
        word(&mut bytes, value);
    }
    let end = bytes.len();
    bytes.extend(b"unread tail");
    let mut reader = Binary::new(&bytes);
    for (kind, name) in [
        (Kind::Skeleton, "adult"),
        (Kind::Appearance, "body"),
        (Kind::Animation, "walk"),
    ] {
        assert_eq!(reader.count(kind).unwrap(), Some(1));
        let asset = reader.read(kind).unwrap();
        assert_eq!(asset.kind(), kind);
        assert_eq!(asset.name(), name);
    }
    assert_eq!(reader.position(), end);
}
