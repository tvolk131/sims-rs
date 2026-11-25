use sims_formats::family_file::FamilyFile;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const FIRST_GUID: u32 = 0x1111_1111;
const SECOND_GUID: u32 = 0x2222_2222;

fn saved_header(tag: &[u8; 4], version: u32) -> Vec<u8> {
    let mut bytes = Vec::from(0_u32.to_le_bytes());
    bytes.extend(version.to_le_bytes());
    bytes.extend(tag);
    bytes
}

fn strings(entries: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::from((-3_i16).to_le_bytes());
    bytes.extend(u16::try_from(entries.len()).unwrap().to_le_bytes());
    for text in entries {
        bytes.push(1);
        bytes.extend(text.as_bytes());
        bytes.extend([0, 0]);
    }
    bytes
}

fn chunk(iff: &mut Vec<u8>, tag: &[u8; 4], id: u16, data: &[u8]) {
    iff.extend(tag);
    iff.extend(u32::try_from(76 + data.len()).unwrap().to_be_bytes());
    iff.extend(id.to_be_bytes());
    iff.extend(0_u16.to_be_bytes());
    iff.extend([0; 64]);
    iff.extend(data);
}

fn synthetic_file(include_family: bool, first_body: &[u8]) -> Vec<u8> {
    let mut iff = vec![0; 64];
    iff[..13].copy_from_slice(b"IFF FILE 2.5:");
    if include_family {
        let mut family = saved_header(b"IMAF", 7);
        for field in [42, 17, 200, 300, 0, 1, 2, FIRST_GUID, SECOND_GUID] {
            family.extend(field.to_le_bytes());
        }
        chunk(&mut iff, b"FAMI", 17, &family);
    }
    chunk(&mut iff, b"FAMs", 17, &strings(&["Example Family"]));

    let mut export = saved_header(b"iPXE", 0x3f);
    // The export list and the physical chunk order both differ from FAMI.
    for field in [17_u16, 2, 1, 0, 0, 0, 0, 0, 0] {
        export.extend(field.to_le_bytes());
    }
    export.push(1);
    chunk(&mut iff, b"EXPi", 0, &export);

    let mut neighbors = saved_header(b"SRBN", 63);
    neighbors.extend(2_u32.to_le_bytes());
    for (id, guid) in [(1_i16, FIRST_GUID), (2_i16, SECOND_GUID)] {
        neighbors.extend(1_u32.to_le_bytes());
        neighbors.extend(4_u32.to_le_bytes());
        neighbors.extend([0, 0]); // Empty name and alignment pad.
        neighbors.extend(0_u32.to_le_bytes());
        neighbors.extend(0_u32.to_le_bytes()); // No person data.
        neighbors.extend(id.to_le_bytes());
        neighbors.extend(guid.to_le_bytes());
        neighbors.extend((-1_i32).to_le_bytes());
        neighbors.extend(0_u32.to_le_bytes()); // No relationships.
    }
    chunk(&mut iff, b"NBRS", 1, &neighbors);
    chunk(&mut iff, b"THMB", 0, b"lot data");
    chunk(&mut iff, b"CTSS", 1, &strings(&["First", "Description"]));
    chunk(&mut iff, b"CTSS", 2002, &strings(&["Second"]));
    chunk(&mut iff, b"BMP_", 1, b"portrait data");
    chunk(&mut iff, b"uChr", 2, &strings(&["second body"]));
    chunk(&mut iff, b"uChr", 1, first_body);
    iff
}

#[test]
fn reads_a_family_and_matches_guids_through_neighbor_slots() {
    let mut body = strings(&["first body", "appearance"]);
    body.extend([0xa5; 2]); // The writer leaves one unused byte per entry.
    let parsed = FamilyFile::parse(&synthetic_file(true, &body)).unwrap();

    assert_eq!(parsed.family.house, 42);
    assert_eq!(parsed.family.number, 17);
    assert_eq!(parsed.family.members, [FIRST_GUID, SECOND_GUID]);
    assert_eq!(parsed.name.get(0, 1), Some("Example Family"));
    assert_eq!(parsed.export_info.family_id, 17);
    assert_eq!(&parsed.export_info.member_ids[..2], &[2, 1]);
    assert_eq!(parsed.export_info.language, Some(1));
    assert!(parsed.history.is_none());
    assert!(parsed.iff.get(b"THMB", 0).is_some());

    let first = &parsed.members[&1];
    assert_eq!(first.body_strings.get(0, 1), Some("first body"));
    assert_eq!(first.body_strings.get(1, 1), Some("appearance"));
    assert_eq!(first.ctss, Some(strings(&["First", "Description"])));
    assert_eq!(first.portrait.as_deref(), Some(b"portrait data".as_slice()));
    let second = &parsed.members[&2];
    assert_eq!(second.body_strings.get(0, 1), Some("second body"));
    assert!(second.ctss.is_none());
    assert_eq!(second.localized_ctss, Some(strings(&["Second"])));
    assert!(second.portrait.is_none());
    assert_eq!(parsed.member_chunk_id(FIRST_GUID), Some(1));
    assert_eq!(parsed.member_chunk_id(SECOND_GUID), Some(2));
    assert_eq!(parsed.member_chunk_id(0), None);
}

#[test]
fn reports_truncated_body_and_missing_family_at_file_offsets() {
    let body = strings(&["first body"]);
    let bytes = synthetic_file(true, &body[..body.len() - 1]); // Drop the comment terminator.
    let error = FamilyFile::parse(&bytes).unwrap_err();
    assert!(error.message.contains("uChr chunk 1"), "{error}");
    assert_eq!(error.offset, bytes.len());

    let mut bytes = synthetic_file(true, &body);
    let payload_offset = bytes.len() - body.len();
    bytes.pop(); // Leave the IFF chunk size intact.
    let error = FamilyFile::parse(&bytes).unwrap_err();
    assert!(error.message.contains("uChr IFF chunk"), "{error}");
    assert_eq!(error.offset, payload_offset);

    let bytes = synthetic_file(false, &body);
    let error = FamilyFile::parse(&bytes).unwrap_err();
    assert!(error.message.contains("missing FAMI chunk 17"), "{error}");
    assert_eq!(error.offset, bytes.len());
}

#[test]
fn reports_malformed_resource_map_at_its_payload_offset() {
    let mut bytes = synthetic_file(true, &strings(&["first body"]));
    let map_offset = bytes.len();
    bytes[60..64].copy_from_slice(&u32::try_from(map_offset).unwrap().to_be_bytes());
    chunk(&mut bytes, b"rsmp", 0, b"short");

    let error = FamilyFile::parse(&bytes).unwrap_err();
    assert!(error.message.contains("rsmp chunk"), "{error}");
    assert_eq!(error.offset, map_offset + 76);
}

#[expect(clippy::disallowed_methods, reason = "sort paths before using them")]
fn sorted_paths(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

fn family_files(directory: &Path) -> Vec<PathBuf> {
    sorted_paths(directory)
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("FAM"))
        })
        .collect()
}

#[test]
fn reads_the_complete_collections_family_files() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("skipping: set SIMS_INSTALL to read the install's family files");
        return;
    };
    let root = PathBuf::from(root);
    let mut files = family_files(&root.join("TemplateFamilyUnleashed"));
    assert_eq!(files.len(), 6);
    let patches: Vec<_> = sorted_paths(&root)
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("UserData"))
        })
        .map(|path| path.join("Patch"))
        .filter(|path| path.is_dir())
        .collect();
    assert!(!patches.is_empty());
    for patch in patches {
        files.extend(family_files(&patch));
    }
    files.push(root.join("UserData/Tutorial.FAM"));

    let mut charming = 0;
    for path in files {
        let parsed = FamilyFile::parse(&fs::read(&path).unwrap())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        for &guid in &parsed.family.members {
            assert!(
                parsed.member_chunk_id(guid).is_some(),
                "{}: no chunk ID for {guid:#010x}",
                path.display()
            );
        }
        if path.file_stem().is_some_and(|stem| stem == "Charming_13") {
            charming += 1;
            let adult = 0x64bc_cdfb;
            let dog = 0x67fd_dfd9;
            assert_eq!(parsed.family.members, [adult, dog]);
            assert_eq!(
                parsed.members.keys().copied().collect::<BTreeSet<_>>(),
                BTreeSet::from([126, 127])
            );
            assert_eq!(parsed.members[&126].body_strings.get(0, 1), Some("dog"));
            assert_eq!(parsed.members[&127].body_strings.get(0, 1), Some("adult"));
            assert_eq!(parsed.member_chunk_id(adult), Some(127));
            assert_eq!(parsed.member_chunk_id(dog), Some(126));
        }
    }
    assert!(charming > 0);
}
