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
}

#[test]
fn version_word_alone_is_an_empty_table() {
    for version in [0i16, -1, -2, -3] {
        let table = StringTable::parse(&version.to_le_bytes()).unwrap();
        assert!(table.entries.is_empty());
    }
}

#[test]
fn rejects_unknown_versions_and_truncation() {
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
