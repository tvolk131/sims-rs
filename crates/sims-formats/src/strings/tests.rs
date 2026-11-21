use super::*;

fn texts(table: &StringTable) -> Vec<(u8, &str, &str)> {
    table
        .entries
        .iter()
        .map(|e| (e.language, e.text.as_str(), e.comment.as_str()))
        .collect()
}

#[test]
fn reads_each_versioned_layout() {
    let pascal = b"\x00\x00\x02\x00\x02hi\x03you";
    let table = StringTable::parse(pascal).unwrap();
    assert_eq!(table.format, Format::Pascal);
    assert_eq!(texts(&table), [(1, "hi", ""), (1, "you", "")]);

    let terminated = b"\xff\xff\x01\x00hi\0";
    let table = StringTable::parse(terminated).unwrap();
    assert_eq!(table.format, Format::Terminated);
    assert_eq!(texts(&table), [(1, "hi", "")]);

    let commented = b"\xfe\xff\x01\x00hi\0note\0";
    let table = StringTable::parse(commented).unwrap();
    assert_eq!(table.format, Format::Commented);
    assert_eq!(texts(&table), [(1, "hi", "note")]);

    // Language 0 is read as English (1).
    let localized = b"\xfd\xff\x02\x00\x00hi\0\0\x05\xe9t\xe9\0c\0";
    let table = StringTable::parse(localized).unwrap();
    assert_eq!(table.format, Format::Localized);
    assert_eq!(texts(&table), [(1, "hi", ""), (5, "été", "c")]);
    assert_eq!(table.get(0, 5), Some("été"));
    assert_eq!(table.get(1, 5), None);
    assert_eq!(table.get(0, 9), Some("hi"));
}

#[test]
fn reads_mac_resources_with_a_big_endian_count() {
    let table = StringTable::parse(b"\x00\x02\x02hi\x03you").unwrap();
    assert_eq!(table.format, Format::MacResource);
    assert_eq!(texts(&table), [(1, "hi", ""), (1, "you", "")]);
    // A count of 256 reads as the little-endian word 1.
    let mut bytes = vec![0x01, 0x00];
    bytes.extend(std::iter::repeat_n(0, 256));
    let table = StringTable::parse(&bytes).unwrap();
    assert_eq!(table.format, Format::MacResource);
    assert_eq!(table.entries.len(), 256);
    // 128 strings start 00 80, a negative word read as a version.
    let mut bytes = vec![0x00, 0x80];
    bytes.extend(std::iter::repeat_n(0, 128));
    let table = StringTable::parse(&bytes).unwrap();
    assert_eq!(table.format, Format::MacResource);
    assert_eq!(table.entries.len(), 128);
}

#[test]
fn version_word_alone_is_an_empty_table() {
    for version in [0i16, -1, -2, -3] {
        let table = StringTable::parse(&version.to_le_bytes()).unwrap();
        assert!(table.entries.is_empty());
    }
}

#[test]
fn rejects_truncation() {
    // Other leading words are Mac counts, here of strings that are absent.
    assert!(StringTable::parse(&(-4i16).to_le_bytes()).is_err());
    assert!(StringTable::parse(&i16::MIN.to_le_bytes()).is_err());
    assert!(StringTable::parse(b"").is_err());
    assert!(StringTable::parse(b"\x00").is_err());
    let whole = b"\xfe\xff\x01\x00hi\0note\0";
    for length in 3..whole.len() {
        assert!(StringTable::parse(&whole[..length]).is_err(), "{length}");
    }
    assert!(StringTable::parse(b"\x00\x02\x02hi").is_err());
}

#[test]
fn a_resource_is_read_in_its_flags_byte_order() {
    const LITTLE: u16 = 0x10;
    let read = |data: &[u8], flags| texts(&StringTable::parse_chunk(data, flags).unwrap()).len();
    // Version -1: big-endian, both words swap; little-endian, neither.
    assert_eq!(read(b"\xff\xff\x00\x02a\0b\0", 0), 2);
    assert_eq!(read(b"\xff\xff\x02\x00a\0b\0", LITTLE), 2);
    // Versions -2 and -3: only the version word swaps.
    assert_eq!(read(b"\xff\xfe\x01\x00hi\0note\0", 0), 1);
    let table = StringTable::parse_chunk(b"\xff\xfd\x01\x00\x01hi\0\0", 0).unwrap();
    assert_eq!(table.format, Format::Localized);
    // Any other word is a count of length-prefixed strings, in the
    // resource's order.
    assert_eq!(read(b"\x00\x02\x02hi\x03you", 0), 2);
    assert_eq!(read(b"\x02\x00\x02hi\x03you", LITTLE), 2);
    // Version 0 and negative counts are empty.
    assert_eq!(read(b"\x00\x00\x02\x00\x02hi\x03you", LITTLE), 0);
    assert_eq!(read(b"\x00\x80", LITTLE), 0);
    // So is a version's count of 0x8000 or more, strings or not.
    let mut big = b"\xff\xff\x00\x80".to_vec();
    big.extend(std::iter::repeat_n(0, 0x8000));
    assert_eq!(read(&big, LITTLE), 0);
    assert_eq!(read(b"\xff\xfe\x00\x80", 0), 0);
}

#[test]
fn decode_keeps_leading_bom_bytes_in_the_code_page() {
    assert_eq!(decode(b"\xef\xbb\xbfadult", 1), "\u{ef}\u{bb}\u{bf}adult");
    assert_eq!(decode(b"\xff\xfeA", 1), "\u{ff}\u{fe}A");
    let table = StringTable::parse(b"\xff\xff\x01\x00\xef\xbb\xbfadult\0").unwrap();
    assert_eq!(table.get(0, 1), Some("\u{ef}\u{bb}\u{bf}adult"));
}
