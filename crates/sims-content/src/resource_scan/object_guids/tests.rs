use super::*;
use sims_formats::iff::Iff;

fn reference(bytes: &[u8]) -> Result<Vec<u32>> {
    let iff = Iff::parse(bytes.to_vec())?;
    Ok(iff
        .all(b"OBJD")
        .filter_map(|c| {
            let data = iff.data(c);
            (data.len() >= 32).then(|| u32::from_le_bytes(data[28..32].try_into().unwrap()))
        })
        .collect())
}
fn compare(bytes: &[u8]) {
    let outcome = |r: Result<Vec<u32>>| r.map_err(|e| (e.offset, e.message));
    assert_eq!(outcome(parse(bytes)), outcome(reference(bytes)), "{} bytes", bytes.len());
}
fn header() -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
    bytes
}
fn chunk(bytes: &mut Vec<u8>, tag: &[u8; 4], id: u16, flags: u16, label: [u8; 64], payload: &[u8]) {
    bytes.extend(tag);
    bytes.extend((76u32 + payload.len() as u32).to_be_bytes());
    bytes.extend(id.to_be_bytes());
    bytes.extend(flags.to_be_bytes());
    bytes.extend(label);
    bytes.extend(payload);
}
fn objd(guid: u32, length: usize) -> Vec<u8> {
    let mut data = vec![0; length];
    if length >= 32 {
        data[28..32].copy_from_slice(&guid.to_le_bytes());
    }
    data
}
#[test]
fn projection_preserves_physical_order_flags_labels_and_short_records() {
    let mut bytes = header();
    for (tag, id, flags, guid, length) in [
        (*b"OBJD", 300, 4, 0x12345678, 216),
        (*b"STR#", 3, 0, 0x99999999, 80),
        (*b"OBJD", 1, 0xffff, 0xffffffff, 32),
        (*b"OBJD", 300, 0, 0x12345678, 31),
        (*b"OBJD", 0, 0, 0, 32),
        (*b"OBJD", 300, 0, 0x12345678, 33),
    ] {
        chunk(&mut bytes, &tag, id, flags, [0xff; 64], &objd(guid, length));
    }
    assert_eq!(parse(&bytes).unwrap(), vec![0x12345678, 0xffffffff, 0, 0x12345678]);
    compare(&bytes);
    // Full parser does not interpret this map pointer until resource_order is
    // requested; discovery must not add a new rsmp validity gate.
    bytes[60..64].copy_from_slice(&u32::MAX.to_be_bytes());
    compare(&bytes);
    bytes[9..12].copy_from_slice(b"2.0");
    compare(&bytes);
}
#[test]
fn every_truncation_keeps_the_full_parser_error_offset_and_message() {
    let mut bytes = header();
    chunk(&mut bytes, b"OBJD", 1, 4, [0x81; 64], &objd(0x12345678, 32));
    chunk(&mut bytes, b"JUNK", 2, 0, [0x9d; 64], &[7; 35]);
    chunk(&mut bytes, b"OBJD", 3, 0, [b'a'; 64], &objd(0xffffffff, 216));
    for end in 0..=bytes.len() {
        compare(&bytes[..end]);
    }
    // A complete desired OBJD is not returned successfully before later
    // unrelated metadata/payload has been checked.
    let mut bad = bytes.clone();
    bad.push(0);
    assert!(parse(&bad).is_err());
    compare(&bad);
}
#[test]
fn header_mutations_length_domains_and_infallible_label_bytes_match() {
    let mut bytes = header();
    chunk(&mut bytes, b"OBJD", 128, 0, [0; 64], &objd(0x87654321, 216));
    for offset in 0..140 {
        for bit in 0..8 {
            let mut changed = bytes.clone();
            changed[offset] ^= 1 << bit;
            compare(&changed);
        }
    }
    for length in [0u32, 1, 4, 75, 76, 77, 107, 108, 292, 293, u32::MAX] {
        let mut changed = bytes.clone();
        changed[68..72].copy_from_slice(&length.to_be_bytes());
        compare(&changed);
    }
    for label in 0..=255 {
        let mut changed = bytes.clone();
        changed[76..140].fill(label);
        compare(&changed);
    }
    for length in 0..=33 {
        let mut changed = header();
        chunk(&mut changed, b"OBJD", 1, 0, [0; 64], &objd(1, length));
        compare(&changed);
    }
}
