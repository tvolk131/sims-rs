use serde_json::{Value, json};
use sims_formats::iff::{Iff, ResourceMap};
fn header() -> Vec<u8> {
    let mut b = vec![0; 8];
    b.extend(b"pmsr");
    b.extend(0u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(u32::from_be_bytes(*b"CARR").to_le_bytes());
    b
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_resource_map_record_decoding() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/resource-map-records.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let bytes = row["bytes"].as_str().unwrap();
        let bytes = (0..bytes.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&bytes[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            bytes.len(),
            row["original"]["cursor"].as_u64().unwrap() as usize
        );
        let mut b = header();
        b.extend(1u32.to_le_bytes());
        b.extend(&bytes[12..]);
        let map = ResourceMap::parse(&b).unwrap();
        let mut expected = row["original"].clone();
        expected.as_object_mut().unwrap().remove("cursor");
        assert_eq!(json!(map.types[b"CARR"][0]), expected);
        assert!(ResourceMap::parse(&b[..b.len() - 1]).is_err());
    }
}
fn chunk(tag: &[u8; 4], id: u16, flags: u16, payload: &[u8]) -> Vec<u8> {
    let mut b = tag.to_vec();
    b.extend(((payload.len() + 76) as u32).to_be_bytes());
    b.extend(id.to_be_bytes());
    b.extend(flags.to_be_bytes());
    b.extend([0; 64]);
    b.extend(payload);
    b
}
#[test]
fn map_ordinal_order_overrides_file_and_numeric_id_order() {
    let mut map = header();
    map.extend(3u32.to_le_bytes());
    for (offset, id) in [(274u32, 2u16), (198, 7), (350, 1)] {
        map.extend(offset.to_le_bytes());
        map.extend(id.to_le_bytes());
        map.extend(0u16.to_le_bytes());
        map.extend([0; 2]);
    }
    assert_eq!(map.len(), 58);
    let mut b = vec![0; 64];
    b[..13].copy_from_slice(b"IFF FILE 2.5:");
    b[60..64].copy_from_slice(&64u32.to_be_bytes());
    b.extend(chunk(b"rsmp", 0, 0, &map));
    for id in [7, 2, 1] {
        b.extend(chunk(b"CARR", id, 0, &[]));
    }
    let mut iff = Iff::parse(b).unwrap();
    assert_eq!(
        iff.resource_order(b"CARR")
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        [2, 7, 1]
    );
    iff.chunks[1].id = 9;
    assert!(iff.resource_order(b"CARR").is_err());
    iff.bytes[60..64].fill(0);
    iff.chunks[2].flags = 4;
    assert_eq!(
        iff.resource_order(b"CARR")
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        [9, 1]
    );
}
