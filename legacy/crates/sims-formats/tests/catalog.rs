use serde_json::Value;
use sims_formats::avatar::{
    catalog::{Asset, Binary},
    index::Kind,
};

#[cfg(feature = "private-fixtures")]
#[test]
fn catalog_preserves_native_empty_motion_records_without_claiming_playback() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-catalog-motion.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 26);
    for row in report["cases"].as_array().unwrap() {
        let motion: Vec<u8> = serde_json::from_value(row["input"].clone()).unwrap();
        let fields: Vec<u32> = serde_json::from_value(row["original"]["fields"].clone()).unwrap();
        assert_eq!(row["original"]["returned"], 1);
        assert_eq!(row["original"]["consumed"], motion.len());
        let mut bytes = vec![];
        for count in [0u32, 0, 1] {
            bytes.extend(count.to_le_bytes());
        }
        bytes.extend(b"\x04test\x07samples");
        for value in [1000f32.to_bits(), 0, 0, fields[0], fields[0], 1] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(&motion);
        let mut reader = Binary::new(&bytes);
        assert_eq!(reader.count(Kind::Skeleton).unwrap(), Some(0));
        assert_eq!(reader.count(Kind::Appearance).unwrap(), Some(0));
        assert_eq!(reader.count(Kind::Animation).unwrap(), Some(1));
        let Asset::Animation(animation) = reader.read(Kind::Animation).unwrap() else {
            panic!()
        };
        let m = &animation.motions[0];
        assert_eq!(
            m.bone.as_bytes(),
            serde_json::from_value::<Vec<u8>>(row["original"]["bone"].clone()).unwrap()
        );
        assert_eq!(m.frames, fields[0]);
        assert_eq!(m.duration_ms.to_bits(), fields[1]);
        assert_eq!(m.translation, (fields[2] != 0).then_some(fields[4]));
        assert_eq!(m.rotation, (fields[3] != 0).then_some(fields[5]));
        assert!(m.properties.is_empty() && m.event_lists.is_empty());
        assert_eq!(reader.position(), bytes.len());
        assert_eq!(
            sims_formats::animation::read_bcf_native(&bytes).unwrap(),
            vec![animation.clone()]
        );
        if m.frames == 0 {
            assert!(
                sims_formats::animation::read_bcf(&bytes).is_err(),
                "legacy playback API must still reject unsampleable channels"
            );
        }
    }
}

#[test]
fn catalog_retains_success_before_a_later_short_read() {
    let mut bytes = (-1i32).to_le_bytes().to_vec();
    bytes.extend(1i32.to_le_bytes());
    bytes.extend(b"\x05MiXeD");
    for word in [7u32, 0, 0] {
        bytes.extend(word.to_le_bytes());
    }
    let mut reader = Binary::new(&bytes);
    assert_eq!(reader.count(Kind::Skeleton).unwrap(), Some(-1));
    assert_eq!(reader.count(Kind::Appearance).unwrap(), Some(1));
    let asset = reader.read(Kind::Appearance).unwrap();
    assert_eq!(asset.name(), "mixed");
    assert_eq!(reader.count(Kind::Animation).unwrap(), None);
    assert_eq!(reader.position(), bytes.len());
    assert!(reader.count(Kind::Animation).is_err());
}

#[test]
fn mixed_binary_catalog_decodes_all_three_groups_and_leaves_the_tail_unread() {
    fn word(b: &mut Vec<u8>, v: u32) {
        b.extend(v.to_le_bytes());
    }
    let mut bytes = vec![];
    word(&mut bytes, 1);
    bytes.extend(b"\x05adult");
    word(&mut bytes, 1);
    bytes.extend(b"\x04ROOT\x04NULL");
    word(&mut bytes, 0);
    for v in [0f32, 0., 0., 0., 0., 0., 1.] {
        word(&mut bytes, v.to_bits());
    }
    for v in [1, 1, 0, 0, 0] {
        word(&mut bytes, v);
    }
    word(&mut bytes, 1);
    bytes.extend(b"\x04BODY");
    for v in [7, 0, 0] {
        word(&mut bytes, v);
    }
    word(&mut bytes, 1);
    bytes.extend(b"\x04walk\x07samples");
    for v in [1000f32.to_bits(), 0, 0, 0, 0, 1] {
        word(&mut bytes, v);
    }
    bytes.extend(b"\x04ROOT");
    for v in [1, 1000f32.to_bits(), 0, 0, u32::MAX, u32::MAX, 0, 0] {
        word(&mut bytes, v);
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
