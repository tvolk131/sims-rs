use super::*;

// Frozen pre-optimization implementation. Keep this independent of `read` so
// shared-reader regressions cannot make both parse and validate pass together.
fn original_parse(data: &[u8]) -> Result<StringTable> {
    let mut r = Reader::new(data);
    let version = r.i16()?;
    if !matches!(version, -3..=0) {
        return r.fail(format!("unsupported string-table version {version}"));
    }
    if r.remaining() == 0 {
        return Ok(StringTable {
            version,
            entries: Vec::new(),
        });
    }
    let count = r.u16()? as usize;
    r.count(count, 1)?;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let language = if version == -3 { r.u8()?.max(1) } else { 1 };
        let raw_text = if version == 0 {
            let n = r.u8()? as usize;
            r.take(n)?.to_vec()
        } else {
            r.cstr()?.to_vec()
        };
        let raw_comment = if version <= -2 {
            r.cstr()?.to_vec()
        } else {
            Vec::new()
        };
        entries.push(TextEntry {
            language,
            text: decode(&raw_text, language),
            comment: decode(&raw_comment, language),
            raw_text,
            raw_comment,
        });
    }
    Ok(StringTable { version, entries })
}

fn error(result: Result<impl Sized>) -> std::result::Result<(), (usize, String)> {
    result.map(|_| ()).map_err(|e| (e.offset, e.message))
}

fn check(data: &[u8]) {
    let original = original_parse(data);
    let parsed = StringTable::parse(data);
    assert_eq!(
        error(StringTable::validate(data)),
        error(original_parse(data)),
        "{data:?}"
    );
    assert_eq!(
        error(StringTable::parse(data)),
        error(original_parse(data)),
        "{data:?}"
    );
    if let (Ok(a), Ok(b)) = (original, parsed) {
        assert_eq!(a.version, b.version);
        let values = |s: StringTable| {
            s.entries
                .into_iter()
                .map(|e| (e.language, e.text, e.comment, e.raw_text, e.raw_comment))
                .collect::<Vec<_>>()
        };
        assert_eq!(values(a), values(b), "{data:?}");
    }
}

fn table(version: i16) -> Vec<u8> {
    let mut bytes = version.to_le_bytes().to_vec();
    bytes.extend_from_slice(&3_u16.to_le_bytes());
    for (language, text, comment) in [
        (0, b"\x80hello".as_slice(), b"note".as_slice()),
        (15, b"\x82\xa0\xff".as_slice(), b"".as_slice()),
        (18, b"".as_slice(), b"\xff".as_slice()),
    ] {
        if version == -3 {
            bytes.push(language)
        }
        if version == 0 {
            bytes.push(text.len() as u8);
            bytes.extend(text)
        } else {
            bytes.extend(text);
            bytes.push(0)
        }
        if version <= -2 {
            bytes.extend(comment);
            bytes.push(0)
        }
    }
    bytes
}

#[test]
fn validation_preserves_all_versions_entries_truncations_and_trailing_bytes() {
    for version in -3..=0 {
        let bytes = table(version);
        for length in 0..=bytes.len() {
            check(&bytes[..length])
        }
        check(&version.to_le_bytes()); // Historical header-only empty table.
        let mut trailing = bytes.clone();
        trailing.extend([0xff, 0, 0x80]);
        check(&trailing);
        assert!(StringTable::validate(&trailing).is_ok());
        let mut zero = version.to_le_bytes().to_vec();
        zero.extend([0, 0, 255]);
        check(&zero);
        assert!(StringTable::validate(&zero).is_ok());
    }
    for version in [i16::MIN, -4, 1, i16::MAX] {
        check(&version.to_le_bytes())
    }
}

#[test]
fn validation_matches_original_errors_and_decoded_values_for_mutations() {
    for version in -3..=0 {
        let bytes = table(version);
        for at in 0..bytes.len() {
            for value in 0..=255 {
                let mut changed = bytes.clone();
                changed[at] = value;
                check(&changed);
            }
        }
    }
    // Deterministic arbitrary byte sequences exercise malformed headers,
    // counts and unterminated/length-prefixed data without extra dependencies.
    let mut seed = 0x74ca_1d03_67ab_58e1_u64;
    for length in 0..128 {
        for _ in 0..32 {
            let mut bytes = Vec::with_capacity(length);
            for _ in 0..length {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                bytes.push(seed as u8);
            }
            check(&bytes);
        }
    }
}
