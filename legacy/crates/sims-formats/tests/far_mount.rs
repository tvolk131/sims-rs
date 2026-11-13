use sims_formats::far::mount::Reader;

#[derive(Debug, PartialEq, Eq)]
struct Observed {
    count: Option<i32>,
    entries: Vec<(Vec<u8>, u32, u32)>,
    error: Option<(usize, String)>,
    position: usize,
    manifest_offset: Option<u32>,
}

fn observe(mut reader: Reader<'_>) -> Observed {
    let count = reader.count();
    let mut entries = Vec::new();
    let mut error = None;
    if let Some(count) = count {
        for _ in 0..count {
            match reader.entry() {
                Ok(Some(entry)) => entries.push((entry.name, entry.offset, entry.size)),
                Ok(None) => break,
                Err(failure) => {
                    error = Some((failure.offset, failure.message));
                    break;
                }
            }
        }
    }
    Observed {
        count,
        entries,
        error,
        position: reader.position,
        manifest_offset: reader.manifest_offset,
    }
}

fn compare(bytes: &[u8]) {
    let header = &bytes[..bytes.len().min(16)];
    let directory = header
        .get(12..16)
        .and_then(|offset| bytes.get(u32::from_le_bytes(offset.try_into().unwrap()) as usize..))
        .unwrap_or_default();
    assert_eq!(
        observe(Reader::with_directory(header, directory)),
        observe(Reader::new(bytes)),
        "{bytes:?}"
    );
}

fn archive() -> Vec<u8> {
    let mut bytes = b"FAR!byAZ".to_vec();
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(48u32.to_le_bytes());
    bytes.resize(48, 19);
    bytes.extend(2i32.to_le_bytes());
    for (name, offset, size) in [
        (b"Sub/one\0ignored".as_slice(), 16u32, 7u32),
        (b"two", 29, 4),
    ] {
        for word in [size, size, offset, name.len() as u32] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend(name);
    }
    bytes
}

#[test]
fn separated_directory_preserves_every_truncated_read_and_absolute_position() {
    let bytes = archive();
    for length in 0..=bytes.len() {
        compare(&bytes[..length]);
    }
    let result = observe(Reader::new(&bytes));
    assert_eq!(
        result.entries,
        [(b"Sub/one".to_vec(), 16, 7), (b"two".to_vec(), 29, 4)]
    );
    assert_eq!(result.position, bytes.len());
}

#[test]
fn separated_directory_preserves_unvalidated_headers_and_signed_counts() {
    let original = archive();
    for position in 0..16 {
        for value in 0..=255 {
            let mut bytes = original.clone();
            bytes[position] = value;
            compare(&bytes);
        }
    }
    for count in [i32::MIN, -1, 0, 1, 2, 3, i32::MAX] {
        let mut bytes = original.clone();
        bytes[48..52].copy_from_slice(&count.to_le_bytes());
        compare(&bytes);
    }
}

#[test]
fn separated_directory_preserves_entry_errors_and_partial_progress() {
    let original = archive();
    for position in 52..original.len() {
        for value in [0, 1, 127, 255] {
            let mut bytes = original.clone();
            bytes[position] = value;
            compare(&bytes);
        }
    }
    let mut bytes = original;
    bytes[64..68].copy_from_slice(&2048u32.to_le_bytes());
    compare(&bytes);
    assert_eq!(
        observe(Reader::new(&bytes)).error,
        Some((68, "FAR filename exceeds native mount buffer".into()))
    );
}
