use sims_formats::family_file::FamilyFile;
use sims_formats::iff::{Iff, ResourceMap};
use sims_formats::iff_write::{ChunkWrite, write_chunk, write_chunk_with_residue};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const NEW_RESOURCE: ChunkWrite<'_> = ChunkWrite {
    tag: *b"TEST",
    id: 7,
    flags: 0x10,
    label: b"new",
    payload: b"abc",
};

fn header() -> Vec<u8> {
    b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1\0\0\0\0".to_vec()
}

fn append(bytes: &mut Vec<u8>, tag: &[u8; 4], id: u16, flags: u16, label: &[u8], payload: &[u8]) {
    bytes.extend(tag);
    bytes.extend(((76 + payload.len()) as u32).to_be_bytes());
    bytes.extend(id.to_be_bytes());
    bytes.extend(flags.to_be_bytes());
    bytes.extend(label);
    bytes.extend(std::iter::repeat_n(0, 64 - label.len()));
    bytes.extend(payload);
}

fn map_offset(bytes: &[u8]) -> usize {
    u32::from_be_bytes(bytes[60..64].try_into().unwrap()) as usize
}

fn read_map(iff: &Iff) -> ResourceMap {
    let map = iff
        .chunks
        .iter()
        .find(|chunk| chunk.offset == map_offset(&iff.bytes))
        .unwrap();
    assert_eq!((map.tag, map.id, map.flags), (*b"rsmp", 0, 0x10));
    ResourceMap::parse(iff.data(map)).unwrap()
}

#[test]
fn reuses_an_exact_free_block() {
    let mut bytes = header();
    append(&mut bytes, b"XXXX", 70, 4, b"", b"old");
    let next = bytes.len();
    append(&mut bytes, b"KEEP", 13, 0, b"k", b"keep");
    let output = write_chunk(&bytes, NEW_RESOURCE).unwrap();
    assert_eq!(&output[next..bytes.len()], &bytes[next..]);
    assert_eq!(map_offset(&output), bytes.len());

    let iff = Iff::parse(output).unwrap();
    let chunk = iff.get(b"TEST", 7).unwrap();
    assert_eq!(chunk.offset, 64);
    assert_eq!(iff.data(chunk), b"abc");
    assert_eq!(read_map(&iff).unused_bytes, 0);
}

#[test]
fn splits_a_free_block_and_uses_the_following_header() {
    let mut bytes = header();
    append(&mut bytes, b"XXXX", 70, 0x14, b"free", &[0xa5; 253]);
    let next = bytes.len();
    append(&mut bytes, b"KEEP", 13, 0x10, b"k\0tail", b"keep");
    let output = write_chunk(&bytes, NEW_RESOURCE).unwrap();
    let map_offset = map_offset(&output);
    assert_eq!(map_offset, 64 + 79);
    assert_eq!(&output[next..], &bytes[next..]);

    let iff = Iff::parse(output).unwrap();
    let map = read_map(&iff);
    assert_eq!(map.unused_bytes, 250);
    let remainder = &iff.chunks[2];
    assert_eq!(remainder.tag, *b"XXXX");
    assert_eq!(remainder.flags, 4);
    assert_eq!(remainder.id, 13);
    assert_eq!(remainder.label, "k");
    assert_eq!(
        &iff.bytes[remainder.offset + 12..remainder.payload.start],
        &bytes[next + 12..next + 76]
    );
    assert_eq!(iff.data(remainder), &bytes[remainder.payload.clone()]);
}

#[test]
fn appends_when_a_block_would_leave_less_than_a_header() {
    let mut bytes = header();
    append(&mut bytes, b"XXXX", 70, 4, b"old", &[0xa5; 15]);
    let output = write_chunk(&bytes, NEW_RESOURCE).unwrap();
    assert_eq!(&output[64..bytes.len()], &bytes[64..]);
    assert_eq!(map_offset(&output), bytes.len() + 79);
    let iff = Iff::parse(output).unwrap();
    assert_eq!(iff.get(b"TEST", 7).unwrap().offset, bytes.len());
    assert_eq!(read_map(&iff).unused_bytes, 91);
}

#[test]
fn replaces_an_existing_resource_in_the_same_map_slot() {
    let mut bytes = header();
    append(&mut bytes, b"XXXX", 2, 4, b"", b"old");
    append(&mut bytes, b"KEEP", 13, 0, b"k", b"keep");
    let original = bytes.len();
    append(&mut bytes, b"TEST", 7, 0x10, b"x", b"previous payload");
    append(&mut bytes, b"TEST", 1, 0x10, b"y", b"other payload");
    let output = write_chunk(&bytes, NEW_RESOURCE).unwrap();

    let iff = Iff::parse(output).unwrap();
    let erased = iff
        .chunks
        .iter()
        .find(|chunk| chunk.offset == original)
        .unwrap();
    assert_eq!((erased.tag, erased.id, erased.flags), (*b"XXXX", 7, 0x14));
    assert_eq!(erased.label, "x");
    assert_eq!(iff.data(erased), b"previous payload");
    let map = read_map(&iff);
    let entries = &map.types[b"TEST"];
    assert_eq!((entries[0].id, entries[0].offset), (7, 64));
    assert_eq!(entries[0].label, "new");
    assert_eq!(entries[1].id, 1);
    assert_eq!(iff.data(iff.get(b"TEST", 7).unwrap()), b"abc");
}

#[test]
fn compacts_when_the_unused_counter_reaches_ten_kibibytes() {
    let mut bytes = header();
    append(&mut bytes, b"XXXX", 3, 4, b"", &[0xa5; 0x2800 + 3]);
    append(&mut bytes, b"KEEP", 13, 0, b"k", b"keep");

    let iff = Iff::parse(write_chunk(&bytes, NEW_RESOURCE).unwrap()).unwrap();
    assert_eq!(iff.get(b"TEST", 7).unwrap().offset, 64);
    assert_eq!(iff.get(b"KEEP", 13).unwrap().offset, 64 + 79);
    assert_eq!(read_map(&iff).unused_bytes, 0);
    assert_eq!(iff.chunks.len(), 3);
}

#[test]
fn reproduces_old_town_house_writes_with_captured_native_residue() {
    let Some(install) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        return;
    };
    let families = [
        ("Burb_11.FAM", 63),
        ("Charming_13.FAM", 69),
        ("Goth Sr_12.FAM", 77),
        ("Hick_10.FAM", 76),
        ("Kat_6.FAM", 56),
    ];
    for (name, house) in families {
        let before = fs::read(install.join("UserData2/Patch").join(name)).unwrap();
        let family = FamilyFile::parse(&before).unwrap();
        assert_eq!(family.family.house, house, "{name}");
        assert!(family.iff.get(b"Gtab", 1).is_none(), "{name}");
        let original =
            fs::read(install.join(format!("UserData/Houses/House{house:02}.iff"))).unwrap();
        let expected = Iff::parse(original).unwrap();
        let gtab = expected.get(b"Gtab", 1).unwrap();
        let write = ChunkWrite {
            tag: *b"Gtab",
            id: 1,
            flags: 0x10,
            label: b"",
            payload: expected.data(gtab),
        };

        // Enumerate only the bytes the native allocator/serializer leaves
        // unwritten. The rest of the reference is used solely for comparison.
        let eof_split = [56, 76, 77].contains(&house);
        let mut residue = rsmp_padding(&expected);
        if eof_split {
            let split = gtab.payload.end;
            assert_eq!(expected.bytes[split..split + 4], *b"XXXX");
            residue.insert((split + 8, split + 10));
            residue.insert((split + 12, split + 76));
        }
        let error = write_chunk(&before, write).unwrap_err();
        let address = if eof_split { "0x41a4da" } else { "0x464cd7" };
        assert!(error.message.contains(address), "{name}: {error}");

        let actual = write_chunk_with_residue(&before, write, |range| {
            assert!(
                residue.remove(&(range.start, range.end)),
                "{name}: {range:?}"
            );
            expected.bytes.get(range)
        })
        .unwrap();
        assert!(residue.is_empty(), "{name}: {residue:?}");
        assert_eq!(actual.len(), expected.bytes.len(), "{name}");
        let difference = actual.iter().zip(&expected.bytes).position(|(a, b)| a != b);
        assert_eq!(difference, None, "first differing byte for {name}");
    }
}

fn rsmp_padding(iff: &Iff) -> BTreeSet<(usize, usize)> {
    let base = map_offset(&iff.bytes) + 76;
    let mut pos = base + 20;
    let mut padding = BTreeSet::new();
    let le32 = |at| u32::from_le_bytes(iff.bytes[at..at + 4].try_into().unwrap());
    for _ in 0..le32(base + 16) {
        let count = le32(pos + 4);
        pos += 8;
        for _ in 0..count {
            pos += 8;
            pos += iff.bytes[pos..].iter().position(|&byte| byte == 0).unwrap() + 1;
            if (pos - base) & 1 != 0 {
                padding.insert((pos, pos + 1));
                pos += 1;
            }
        }
    }
    padding
}

/// A new file takes each resource once: a second write of a key would
/// leave a hole whose bytes are not the map's alignment bytes.
#[test]
fn a_new_file_refuses_a_key_written_twice() {
    use sims_formats::iff_write::write_new_file;
    let first = ChunkWrite {
        payload: &[7; 300],
        ..NEW_RESOURCE
    };
    let second = ChunkWrite {
        payload: b"",
        ..NEW_RESOURCE
    };
    let error = write_new_file(&[first, second], &[], |_| None).unwrap_err();
    assert!(error.to_string().contains("written twice"), "{error}");
    let other = ChunkWrite { id: 8, ..second };
    let bytes = write_new_file(&[first, other], &[], |_| None).unwrap();
    let iff = Iff::parse(bytes).unwrap();
    assert_eq!(iff.data(iff.get(b"TEST", 7).unwrap()), &[7; 300]);
    assert_eq!(iff.data(iff.get(b"TEST", 8).unwrap()), b"");
}
