//! Linking against a synthetic install. No game data.

use sims_content::{Catalog, Index, Install};
use sims_vm::{Callback, Program, Tree};
use std::path::PathBuf;

struct Dir(PathBuf);
impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("sims-vm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
    fn file(&self, relative: &str, bytes: &[u8]) -> &Self {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
        self
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn iff(chunks: &[(&[u8; 4], u16, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
    for (tag, id, data) in chunks {
        bytes.extend(*tag);
        bytes.extend((76 + data.len() as u32).to_be_bytes());
        bytes.extend(id.to_be_bytes());
        bytes.extend([0; 66]);
        bytes.extend(data);
    }
    bytes
}

/// A BHAV with the given revision whose instructions call `calls` in turn.
fn bhav(revision: u32, calls: &[u16]) -> Vec<u8> {
    let mut bytes = 0x8002u16.to_le_bytes().to_vec();
    bytes.extend((calls.len().max(1) as u16).to_le_bytes());
    bytes.extend([0, 0, 0, 0]);
    bytes.extend(revision.to_le_bytes());
    for (n, &opcode) in calls.iter().enumerate() {
        bytes.extend(opcode.to_le_bytes());
        bytes.extend([n as u8 + 1, 254]);
        bytes.extend([0; 8]);
    }
    if calls.is_empty() {
        bytes.extend(2u16.to_le_bytes());
        bytes.extend([254, 255]);
        bytes.extend([0; 8]);
    }
    bytes
}

fn bcon(values: &[i16]) -> Vec<u8> {
    let mut bytes = vec![values.len() as u8, 0];
    for value in values {
        bytes.extend(value.to_le_bytes());
    }
    bytes
}

fn objd(guid: u32, init: u16, main: u16) -> Vec<u8> {
    let mut bytes = vec![0; 216];
    bytes[..4].copy_from_slice(&138u32.to_le_bytes());
    bytes[0x0a..0x0c].copy_from_slice(&main.to_le_bytes());
    bytes[28..32].copy_from_slice(&guid.to_le_bytes());
    bytes[0x56..0x58].copy_from_slice(&init.to_le_bytes());
    bytes
}

fn objf(entries: &[(u16, u16)]) -> Vec<u8> {
    let mut bytes = vec![0; 8];
    bytes.extend(b"fJBO");
    bytes.extend((entries.len() as u32).to_le_bytes());
    for (condition, action) in entries {
        bytes.extend(condition.to_le_bytes());
        bytes.extend(action.to_le_bytes());
    }
    bytes
}

/// Global.iff, a semi-global file, a lamp that links it, and a chair with
/// an OBJf table and no semi-global.
fn setup(dir: &Dir) -> (Index, Catalog) {
    let global = iff(&[
        (b"BHAV", 256, bhav(1, &[2])),
        (b"BHAV", 257, bhav(0x0001_0000, &[256])),
        (b"BCON", 256, bcon(&[10, 11])),
        (b"BCON", 257, bcon(&[20])),
    ]);
    dir.file("GameData/Behavior.iff", b"")
        .file("GameData/Global/Global.iff", &global)
        .file(
            "GameData/Objects/LampGlobals.iff",
            &iff(&[
                (b"BHAV", 8192, bhav(5, &[256, 8193])),
                (b"BHAV", 8193, bhav(7, &[8192])),
                (b"BCON", 8193, bcon(&[-3, -4])),
            ]),
        )
        .file(
            "GameData/Objects/Lamp.iff",
            &iff(&[
                (b"OBJD", 128, objd(0x1111, 4096, 4097)),
                (b"GLOB", 128, b"\x0bLampGlobals".to_vec()),
                (b"BHAV", 4096, bhav(2, &[8192, 9000, 257])),
                (b"BHAV", 4097, bhav(3, &[])),
                (b"BCON", 4096, bcon(&[99])),
            ]),
        )
        .file(
            "GameData/Objects/Chair.iff",
            &iff(&[
                (b"OBJD", 128, objd(0x2222, 4096, 4096)),
                (b"OBJf", 128, objf(&[(0, 4097), (0, 0)])),
                (b"BHAV", 4096, bhav(9, &[])),
                (b"BHAV", 4097, bhav(4, &[8192])),
            ]),
        );
    let index = Index::build(&Install::detect(&dir.0).unwrap()).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    (index, catalog)
}

#[test]
fn resolves_trees_by_id_range() {
    let dir = Dir::new("trees");
    let (index, catalog) = setup(&dir);
    let program = Program::link(&index, &catalog, [0x1111, 0x2222]).unwrap();
    let lamp = catalog.object(0x1111).unwrap().resource;
    let chair = catalog.object(0x2222).unwrap().resource;
    let name = |tree: Tree<'_>| match tree {
        Tree::Behavior(resource, b) => {
            format!("{} r{}", index.resources[resource].name(), b.revision)
        }
        Tree::Primitive(id) => format!("primitive {id}"),
        Tree::Missing => "missing".into(),
    };
    assert_eq!(name(program.tree(lamp, 2)), "primitive 2");
    assert_eq!(name(program.tree(lamp, 256)), "Global.iff r1");
    assert_eq!(name(program.tree(lamp, 4097)), "Lamp.iff r3");
    assert_eq!(name(program.tree(chair, 4097)), "Chair.iff r4");
    assert_eq!(name(program.tree(lamp, 8193)), "LampGlobals.iff r7");
    // The chair links no semi-global file.
    assert_eq!(name(program.tree(chair, 8192)), "missing");
    assert_eq!(name(program.tree(lamp, 300)), "missing");
    assert_eq!(program.libraries.len(), 4);
}

#[test]
fn reads_constants_by_table_and_entry() {
    let dir = Dir::new("constants");
    let (index, catalog) = setup(&dir);
    let program = Program::link(&index, &catalog, [0x1111, 0x2222]).unwrap();
    let lamp = catalog.object(0x1111).unwrap().resource;
    let chair = catalog.object(0x2222).unwrap().resource;
    assert_eq!(program.constant(lamp, 0), Some(99));
    assert_eq!(program.constant(lamp, (65 << 7) | 1), Some(-4));
    assert_eq!(program.constant(lamp, 128 << 7), Some(10));
    assert_eq!(program.constant(lamp, (128 << 7) | 1), Some(11));
    assert_eq!(program.constant(lamp, 129 << 7), Some(20));
    assert_eq!(program.constant(lamp, (129 << 7) | 1), None);
    assert_eq!(program.constant(chair, 0), None);
    assert_eq!(program.constant(chair, 64 << 7), None);
    assert_eq!(program.constant(lamp, 192 << 7), None);
}

#[test]
fn callbacks_come_from_objf_or_the_definition() {
    let dir = Dir::new("callbacks");
    let (index, catalog) = setup(&dir);
    let program = Program::link(&index, &catalog, [0x1111, 0x2222]).unwrap();
    let lamp = catalog.object(0x1111).unwrap();
    let chair = catalog.object(0x2222).unwrap();
    assert_eq!(program.callback(lamp, Callback::Init), 4096);
    assert_eq!(program.callback(lamp, Callback::Main), 4097);
    assert_eq!(program.callback(chair, Callback::Init), 4097);
    assert_eq!(program.callback(chair, Callback::Main), 0);
}

#[test]
fn signatures_sum_reachable_revisions_once() {
    let dir = Dir::new("signatures");
    let (index, catalog) = setup(&dir);
    let program = Program::link(&index, &catalog, [0x1111, 0x2222]).unwrap();
    let lamp = catalog.object(0x1111).unwrap().resource;
    let chair = catalog.object(0x2222).unwrap().resource;
    // Lamp 4096 (2) calls 8192 (5), missing 9000 and 257 (0x10000); 8192
    // calls 256 (1) and 8193 (7), which calls 8192 again.
    assert_eq!(
        program.signature(lamp, 4096),
        (2 << 16) + 2 + 5 + 7 + 1 + 0x1_0000
    );
    assert_eq!(program.signature(lamp, 4097), (3 << 16) + 3);
    // The chair's 8192 is missing: it adds nothing.
    assert_eq!(program.signature(chair, 4097), (4 << 16) + 4);
    assert_eq!(program.signature(chair, 0), 0);
}

#[test]
fn every_guid_needs_a_definition() {
    let dir = Dir::new("missing");
    let (index, catalog) = setup(&dir);
    let error = Program::link(&index, &catalog, [0x3333])
        .unwrap_err()
        .to_string();
    assert!(error.contains("0x00003333"), "{error}");
}

/// One version 7 TTAB entry with the given autonomy threshold.
fn ttab(autonomy: u32) -> Vec<u8> {
    let mut bytes = 1u16.to_le_bytes().to_vec();
    bytes.extend(7u16.to_le_bytes());
    bytes.extend(4100u16.to_le_bytes()); // action
    bytes.extend(0u16.to_le_bytes()); // test
    for value in [0, 0, 0, 0, 0, autonomy, 0] {
        // motives, flags, string, attenuation code and value, autonomy,
        // joining index
        bytes.extend(value.to_le_bytes());
    }
    bytes
}

/// A version 4 SLOT chunk with one entry of each kind given.
fn slot(kinds: &[u16]) -> Vec<u8> {
    let mut bytes = vec![0; 4];
    bytes.extend(4u32.to_le_bytes());
    bytes.extend(b"TOLS");
    bytes.extend((kinds.len() as u32).to_le_bytes());
    for kind in kinds {
        bytes.extend(kind.to_le_bytes());
        bytes.extend([0; 32]);
    }
    bytes
}

#[test]
fn interaction_tables_and_slots_are_found_through_the_definition() {
    let dir = Dir::new("tables");
    let (_, _) = setup(&dir);
    // The lamp names table 200, which only its semi-global file has, and
    // SLOT 300. The desk names table 201, which no reader decodes, and a
    // SLOT chunk its file lacks. The stool is a tile whose group's catalog
    // definition (sub-index -1) names the table.
    let word = |bytes: &mut Vec<u8>, index: usize, value: i16| {
        bytes[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
    };
    let mut lamp = objd(0x1111, 4096, 4097);
    word(&mut lamp, 7, 200);
    word(&mut lamp, 20, 300);
    let mut desk = objd(0x3333, 0, 0);
    word(&mut desk, 7, 201);
    word(&mut desk, 20, 301);
    let mut stool = objd(0x4444, 0, 0);
    word(&mut stool, 7, -1);
    word(&mut stool, 10, 6);
    let mut stools = objd(0x4445, 0, 0);
    word(&mut stools, 7, 202);
    word(&mut stools, 10, 6);
    word(&mut stools, 11, -1);
    let mut bad_table = 1u16.to_le_bytes().to_vec();
    bad_table.extend(3u16.to_le_bytes());
    dir.file(
        "GameData/Objects/LampGlobals.iff",
        &iff(&[(b"TTAB", 200, ttab(50))]),
    )
    .file(
        "GameData/Objects/Lamp.iff",
        &iff(&[
            (b"OBJD", 128, lamp),
            (b"GLOB", 128, b"\x0bLampGlobals".to_vec()),
            (b"SLOT", 300, slot(&[0, 3, 0, 1, 3])),
        ]),
    )
    .file(
        "GameData/Objects/Desk.iff",
        &iff(&[
            (b"OBJD", 128, desk),
            (b"TTAB", 201, bad_table),
            (b"OBJD", 129, stool),
            (b"OBJD", 130, stools),
            (b"TTAB", 202, ttab(7)),
        ]),
    );
    let index = Index::build(&Install::detect(&dir.0).unwrap()).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let program = Program::link(&index, &catalog, catalog.objects.iter().map(|o| o.guid)).unwrap();
    let lamp = catalog.object(0x1111).unwrap();
    let desk = catalog.object(0x3333).unwrap();
    let chair = catalog.object(0x2222).unwrap();
    let autonomy = |object: &sims_content::Object| {
        program
            .interactions(object.resource, object.chunk)
            .map(|table| table.map(|t| t.interactions[0].autonomy))
    };
    assert_eq!(autonomy(lamp), Ok(Some(50)));
    assert_eq!(autonomy(catalog.object(0x4444).unwrap()), Ok(Some(7)));
    assert_eq!(autonomy(catalog.object(0x4445).unwrap()), Ok(Some(7)));
    // The chair names no table.
    assert_eq!(autonomy(chair), Ok(None));
    let error = autonomy(desk).unwrap_err();
    assert!(error.contains("TTAB #201"), "{error}");
    // Only kind 0 slots hold objects.
    assert_eq!(program.containers(lamp.resource, lamp.chunk), 2);
    assert_eq!(program.containers(desk.resource, desk.chunk), 0);
    assert_eq!(program.containers(chair.resource, chair.chunk), 0);
    // Only kind 3 entries are routing slots, counted among themselves:
    // the lamp's two are its SLOT entries 1 and 4, past a container and a
    // headline.
    let routing = |i| program.routing_slot(lamp.resource, lamp.chunk, i);
    assert_eq!(
        routing(0).map(|(entry, slot)| (entry, slot.kind)),
        Some((1, 3))
    );
    assert_eq!(
        routing(1).map(|(entry, slot)| (entry, slot.kind)),
        Some((4, 3))
    );
    assert!(routing(2).is_none());
    assert!(program.routing_slot(desk.resource, desk.chunk, 0).is_none());
    // A SLOT chunk that does not decode stops the link.
    dir.file(
        "GameData/Objects/Desk.iff",
        &iff(&[(b"OBJD", 128, objd(0x3333, 0, 0)), (b"SLOT", 5, vec![0; 6])]),
    );
    let index = Index::build(&Install::detect(&dir.0).unwrap()).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let error = Program::link(&index, &catalog, [0x3333]).unwrap_err();
    assert!(error.to_string().contains("SLOT #5"), "{error}");
}

/// An unpacked CARR chunk with `levels` levels, each numbered 100 + i
/// with the second string "title".
fn carr(levels: i32) -> Vec<u8> {
    let mut bytes = [0u32.to_le_bytes(), 1u32.to_le_bytes(), *b"RRAC"].concat();
    // Not packed, then an empty name (a byte, which keeps the stream even).
    bytes.extend([0, 0]);
    bytes.extend(levels.to_le_bytes());
    for _ in 0..levels {
        for i in 0..21i32 {
            bytes.extend((100 + i).to_le_bytes());
        }
        // The strings, each padded to an even length.
        bytes.extend([0, 0]);
        bytes.extend(b"title\0");
        bytes.extend([0, 0, 0, 0, 0, 0]);
    }
    bytes
}

#[test]
fn careers_come_from_work_iff_in_resource_order() {
    let dir = Dir::new("careers");
    let (index, catalog) = setup(&dir);
    assert_eq!(Program::link(&index, &catalog, []).unwrap().careers, None);
    // Chunk IDs are sign-extended, and each chunk's levels are kept.
    dir.file(
        "GameData/Objects/work.iff",
        &iff(&[
            (b"CARR", 3, carr(1)),
            (b"CARR", 0xffff, carr(0)),
            (b"STR#", 5, vec![]),
            (b"CARR", 1, carr(2)),
        ]),
    );
    let index = Index::build(&Install::detect(&dir.0).unwrap()).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let program = Program::link(&index, &catalog, []).unwrap();
    let careers = program.careers.unwrap();
    let ids: Vec<_> = careers.iter().map(|c| c.id).collect();
    assert_eq!(ids, [3, -1, 1]);
    let levels: Vec<_> = careers.iter().map(|c| c.levels.len()).collect();
    assert_eq!(levels, [1, 0, 2]);
    assert_eq!(careers[2].levels[1].values[20], 120);
    assert_eq!(careers[2].levels[1].strings[1], "title");
    assert!(program.zoning.is_empty());
    // A CARR chunk that does not decode stops the link.
    dir.file(
        "GameData/Objects/work.iff",
        &iff(&[(b"CARR", 7, vec![0; 4])]),
    );
    let index = Index::build(&Install::detect(&dir.0).unwrap()).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let error = Program::link(&index, &catalog, []).unwrap_err();
    assert!(error.to_string().contains("CARR #7"), "{error}");
    // A Careers.txt overrides the table through a loader that is not
    // ported, so there is no list to give.
    dir.file("GameData/Objects/Careers.txt", b"");
    let index = Index::build(&Install::detect(&dir.0).unwrap()).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let program = Program::link(&index, &catalog, []).unwrap();
    assert_eq!(program.careers, None);
}

/// The catalog's order, checked against the original on a real install
/// when `SIMS_INSTALL` names one. Game data is never committed, so CI
/// skips this.
///
/// When the original loads a lot it creates the lot-wide controllers the
/// lot lacks, walking its catalog: every definition of type 7 or 12 with a
/// nonzero global word. A capture of the original, taken by the old
/// prototype's harness as the Complete Collection loaded a lot with none,
/// holds the 141 objects that made, in creation order. The digest below is
/// SHA-256 of their GUIDs in that order, as little-endian words. Selecting
/// the same definitions from our catalog, in our order, must give it.
#[test]
fn the_catalog_lists_controllers_in_the_originals_order() {
    use sha2::{Digest, Sha256};
    const CAPTURED: &str = "3b30c2e6279467a816d49e377f02bc9da2ed6bf5875eed09dcde094cde298a78";
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let install = Install::detect(std::path::Path::new(&root)).unwrap();
    let neighborhood =
        sims_content::Neighborhood::open(&install.file("UserData").unwrap()).unwrap();
    let mut index = Index::build(&install).unwrap();
    index
        .add_characters(&neighborhood.characters().unwrap().unwrap())
        .unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let program = Program::link(&index, &catalog, catalog.objects.iter().map(|o| o.guid)).unwrap();
    let mut digest = Sha256::new();
    let mut count = 0;
    for object in &catalog.objects {
        let words = program.definition(object.resource, object.chunk).unwrap();
        // Word 9 is the type and word 42 the global word.
        if matches!(words[9], 7 | 12) && words[42] != 0 {
            digest.update(object.guid.to_le_bytes());
            count += 1;
        }
    }
    assert_eq!(count, 141);
    let digest: String = digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(digest, CAPTURED);
}

/// A sprite header: the version and the count, in either byte order.
fn sprite(version: i32, count: i32, big_endian: bool) -> Vec<u8> {
    let words = [version, count, 0];
    if big_endian {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    } else {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }
}

/// [`iff`] with each chunk's flags.
fn flagged(chunks: &[(&[u8; 4], u16, u16, Vec<u8>)]) -> sims_formats::iff::Iff {
    let mut bytes = vec![0; 64];
    bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
    for (tag, id, flags, data) in chunks {
        bytes.extend(*tag);
        bytes.extend((76 + data.len() as u32).to_be_bytes());
        bytes.extend(id.to_be_bytes());
        bytes.extend(flags.to_be_bytes());
        bytes.extend([0; 64]);
        bytes.extend(data);
    }
    sims_formats::iff::Iff::parse(bytes).unwrap()
}

#[test]
fn headline_sprites_count_their_frames_as_the_sprite_cache_does() {
    // Without the conversion flag (0x10) an SPR#'s header is swapped:
    // big-endian. With it, little-endian.
    let frames = sims_vm::sprite_frames(&flagged(&[
        (b"SPR#", 600, 0, sprite(502, 9, true)),
        (b"SPR#", 601, 0x10, sprite(503, 18, false)),
        (b"SPR#", 602, 0, sprite(506, 15, true)),
        (b"SPR#", 603, 0x10, sprite(501, 7, false)),
        (b"SPR#", 604, 0x10, sprite(499, 9, false)),
        // Any version from 500 up keeps its count, read in the flag's
        // order whatever it looks like in the other.
        (b"SPR#", 606, 0, sprite(65536, 9, true)),
        (b"SPR#", 607, 0x10, sprite(0x1f6 << 16, 9, false)),
        // An SPR2 counts only where no SPR# has the ID.
        (b"SPR2", 600, 0, sprite(1000, 30, false)),
        (b"SPR2", 605, 0, sprite(1000, 6, false)),
    ]))
    .unwrap();
    assert_eq!(
        frames.into_iter().collect::<Vec<_>>(),
        [
            (600, 9),
            (601, 9),
            (602, 3),
            (603, 3),
            (604, 0),
            (605, 6),
            (606, 9),
            (607, 9)
        ]
    );
    // The cache looks sprites up as resources: a deleted SPR# (flag 4),
    // even a truncated one, or one of another variant (a high byte in its
    // flags) is not one, and the SPR2 of the ID counts.
    let frames = sims_vm::sprite_frames(&flagged(&[
        (b"SPR#", 600, 4, sprite(502, 9, true)),
        (b"SPR#", 601, 4, vec![1, 2]),
        (b"SPR#", 602, 0x100, sprite(502, 9, true)),
        (b"SPR2", 600, 0, sprite(1000, 6, false)),
        (b"SPR2", 602, 0, sprite(1000, 4, false)),
    ]))
    .unwrap();
    assert_eq!(frames.into_iter().collect::<Vec<_>>(), [(600, 6), (602, 4)]);
    // A live SPR# too short for its header fails.
    let error = sims_vm::sprite_frames(&flagged(&[(b"SPR#", 600, 0, vec![1, 0, 0])]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("SPR# #600"), "{error}");
}

#[test]
fn an_install_without_the_headline_sprites_reads_none() {
    let dir = Dir::new("icons");
    let (_, _) = setup(&dir);
    let install = Install::detect(&dir.0).unwrap();
    assert_eq!(sims_vm::icon_frames(&install).unwrap(), None);
    dir.file(
        "GameData/Sprites.iff",
        &iff(&[(b"SPR#", 616, sprite(502, 9, true))]),
    );
    let frames = sims_vm::icon_frames(&install).unwrap().unwrap();
    assert_eq!(frames[&616], 9);
}

/// An IFF of chunks with flags (2 bytes after the ID).
fn iff_with_flags(chunks: &[(&[u8; 4], u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
    for (tag, id, flags, data) in chunks {
        bytes.extend(*tag);
        bytes.extend((76 + data.len() as u32).to_be_bytes());
        bytes.extend(id.to_be_bytes());
        bytes.extend(flags.to_be_bytes());
        bytes.extend([0; 64]);
        bytes.extend(data);
    }
    bytes
}

/// `iff_with_flags(chunks)` with a version-0 resource map listing
/// `entries` as (tag, index into `chunks` of the chunk it points at, ID,
/// flags).
fn with_map(
    chunks: &[(&[u8; 4], u16, u16, Vec<u8>)],
    entries: &[(&[u8; 4], usize, u16, u16)],
) -> Vec<u8> {
    let mut offsets = Vec::new();
    let mut at = 64u32;
    for (_, _, _, data) in chunks {
        offsets.push(at);
        at += 76 + data.len() as u32;
    }
    let mut types: Vec<&[u8; 4]> = entries.iter().map(|e| e.0).collect();
    types.dedup();
    let mut map = vec![0; 8];
    map.extend(b"pmsr");
    map.extend(0u32.to_le_bytes());
    map.extend((types.len() as u32).to_le_bytes());
    for tag in types {
        let listed: Vec<_> = entries.iter().filter(|e| e.0 == tag).collect();
        map.extend(u32::from_be_bytes(*tag).to_le_bytes());
        map.extend((listed.len() as u32).to_le_bytes());
        for (_, chunk, id, flags) in listed {
            map.extend(offsets[*chunk].to_le_bytes());
            map.extend(id.to_le_bytes());
            map.extend(flags.to_le_bytes());
            map.extend([0, 0]);
        }
    }
    let mut all = chunks.to_vec();
    all.push((b"rsmp", 0, 0x10, map));
    let mut bytes = iff_with_flags(&all);
    bytes[60..64].copy_from_slice(&at.to_be_bytes());
    bytes
}

/// A FAR archive (version 1a) of members `(name, unpacked size, bytes)`.
fn far(members: &[(&str, u32, &[u8])]) -> Vec<u8> {
    let mut data: Vec<u8> = Vec::new();
    let mut directory = (members.len() as u32).to_le_bytes().to_vec();
    for (name, unpacked, bytes) in members {
        directory.extend(unpacked.to_le_bytes());
        directory.extend((bytes.len() as u32).to_le_bytes());
        directory.extend((16 + data.len() as u32).to_le_bytes());
        directory.extend((name.len() as u32).to_le_bytes());
        directory.extend(name.as_bytes());
        data.extend(*bytes);
    }
    let mut far = b"FAR!byAZ".to_vec();
    far.extend(1u32.to_le_bytes());
    far.extend((16 + data.len() as u32).to_le_bytes());
    far.extend(data);
    far.extend(directory);
    far
}

/// A string table of NUL-terminated strings (version -1).
fn terminated(texts: &[&str]) -> Vec<u8> {
    let mut bytes = (-1i16).to_le_bytes().to_vec();
    bytes.extend((texts.len() as u16).to_le_bytes());
    for text in texts {
        bytes.extend(text.as_bytes());
        bytes.push(0);
    }
    bytes
}

/// A string table of entries with a language (version -3).
fn localized(entries: &[(u8, &str)]) -> Vec<u8> {
    let mut bytes = (-3i16).to_le_bytes().to_vec();
    bytes.extend((entries.len() as u16).to_le_bytes());
    for (language, text) in entries {
        bytes.push(*language);
        bytes.extend(text.as_bytes());
        bytes.extend([0, 0]);
    }
    bytes
}

#[test]
fn a_string_table_is_read_as_the_loader_finds_it() {
    let dir = Dir::new("loaded-strings");
    let (_, _) = setup(&dir);
    // Flag 0x10: the header is as the loader reads it (0x41d3a7).
    const LE: u16 = 0x10;
    const DELETED: u16 = sims_formats::iff::DELETED | LE;
    dir.file(
        "GameData/Objects/Lamp.iff",
        &iff_with_flags(&[
            (b"OBJD", 128, 0, objd(0x1111, 4096, 4097)),
            (b"GLOB", 128, 0, b"\x0bLampGlobals".to_vec()),
            (b"BHAV", 4096, 0, bhav(2, &[8192, 9000, 257])),
            (b"BHAV", 4097, 0, bhav(3, &[])),
            (b"BCON", 4096, 0, bcon(&[99])),
            // A live table, then a deleted one of the ID (0x41c542).
            (b"STR#", 301, LE, terminated(&["$Job"])),
            (b"STR#", 301, DELETED, localized(&[(1, "Plain"), (2, "x")])),
            // Only a deleted table.
            (b"STR#", 302, DELETED, terminated(&["Gone"])),
            // Two runs of US English: the later goes in front (0x470482).
            (
                b"STR#",
                303,
                LE,
                localized(&[(1, "a"), (2, "x"), (1, "b"), (0, "c")]),
            ),
            // Entries with a language, none in US English (0x47006b).
            (b"STR#", 304, LE, localized(&[(3, "\x01$Job"), (3, "x")])),
            // One run: as the file has it.
            (b"STR#", 305, LE, localized(&[(2, "x"), (1, "a"), (1, "b")])),
            // A header the loader byte-swaps first (0x41d3a7).
            (b"STR#", 309, 0, terminated(&["a"])),
            // Version 0: the leading word is the count (0x4700d7).
            (b"STR#", 310, LE, b"\0\0\x01\0\x04$Job".to_vec()),
            // A count above 0x7fff is none (0x46ff50).
            (
                b"STR#",
                311,
                LE,
                [&b"\xff\xff\0\x80"[..], &[0; 0x8000]].concat(),
            ),
            // No count.
            (b"STR#", 312, LE, b"\xfd\xff".to_vec()),
            // A deleted table that does not parse does not stop the link.
            (b"STR#", 313, LE, terminated(&["live"])),
            (b"STR#", 313, DELETED, Vec::new()),
            // A legacy table, live and deleted.
            (b"\0CST", 306, 0, b"^a^".to_vec()),
            (b"\0CST", 307, DELETED, b"^a^".to_vec()),
            (b"CST\0", 308, 0, b"^a^".to_vec()),
        ]),
    );
    let index = Index::build(&Install::detect(&dir.0).unwrap()).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let program = Program::link(&index, &catalog, [0x1111]).unwrap();
    let lamp = &program.libraries[&catalog.object(0x1111).unwrap().resource];
    let strings = |texts: &[&str]| Some(texts.iter().map(|t| t.to_string()).collect());
    assert_eq!(lamp.strings[&301], ["Plain"]);
    assert_eq!(lamp.loaded_strings[&301], strings(&["$Job"]));
    assert_eq!(lamp.loaded_strings[&302], None);
    assert_eq!(lamp.loaded_strings[&303], strings(&["b", "c", "a"]));
    assert_eq!(lamp.loaded_strings.get(&305), None);
    assert_eq!(lamp.strings[&305], ["a", "b"]);
    assert_eq!(lamp.strings[&313], ["live"]);
    assert_eq!(lamp.loaded_strings.get(&313), None);
    let unported: Vec<(u16, &str)> = lamp
        .unported_strings
        .iter()
        .map(|(&id, why)| (id, *why))
        .collect();
    let at = ["0x47006b", "0x41d3a7", "0x4700d7", "0x46ff50", "0x46ff42"];
    assert_eq!(
        unported.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        [304, 309, 310, 311, 312]
    );
    for ((_, why), at) in unported.iter().zip(at) {
        assert!(why.contains(at), "{why}");
    }
    assert_eq!(lamp.legacy_strings, std::collections::BTreeSet::from([306]));
    // With a resource map, the flags tested are the map's (0x41b7fa to
    // 0x41b813): a map stating no flag 0x10 for a chunk that has it.
    let chunks = [
        (b"OBJD", 128, 0, objd(0x1111, 4096, 4097)),
        (b"GLOB", 128, 0, b"\x0bLampGlobals".to_vec()),
        (b"BHAV", 4096, 0, bhav(2, &[8192, 9000, 257])),
        (b"BHAV", 4097, 0, bhav(3, &[])),
        (b"BCON", 4096, 0, bcon(&[99])),
        (b"STR#", 301, LE, terminated(&["$Neighbor"])),
    ];
    let link = |bytes: Vec<u8>| {
        dir.file("GameData/Objects/Lamp.iff", &bytes);
        let index = Index::build(&Install::detect(&dir.0).unwrap()).unwrap();
        let catalog = Catalog::build(&index).unwrap();
        let program = Program::link(&index, &catalog, [0x1111]).unwrap();
        let lamp = &program.libraries[&catalog.object(0x1111).unwrap().resource];
        lamp.unported_strings.get(&301).copied()
    };
    let entries = |str_flags: u16, str_chunk: usize| {
        let mut entries: Vec<(&[u8; 4], usize, u16, u16)> = chunks
            .iter()
            .enumerate()
            .map(|(i, (tag, id, flags, _))| (*tag, i, *id, *flags))
            .collect();
        entries[5] = (b"STR#", str_chunk, 301, str_flags);
        entries
    };
    assert_eq!(link(with_map(&chunks, &entries(LE, 5))), None);
    let why = link(with_map(&chunks, &entries(0, 5))).unwrap();
    assert!(why.contains("0x41d3a7"), "{why}");
    // A map whose entry does not match the chunk it points at, which the
    // loader reads as it is (0x41bfdd to 0x41c032), fails the link. (In a
    // file with a definition, the catalog leaves the file out first.)
    dir.file(
        "GameData/Objects/Lamp.iff",
        &with_map(&chunks, &entries(LE, 5)),
    );
    let globals = [
        (b"BHAV", 8192, 0, bhav(5, &[256, 8193])),
        (b"BHAV", 8193, 0, bhav(7, &[8192])),
        (b"BCON", 8193, 0, bcon(&[-3, -4])),
        (b"STR#", 301, LE, terminated(&["$Neighbor"])),
    ];
    let globals_entries = |str_chunk: usize| {
        let mut entries: Vec<(&[u8; 4], usize, u16, u16)> = globals
            .iter()
            .enumerate()
            .map(|(i, (tag, id, flags, _))| (*tag, i, *id, *flags))
            .collect();
        entries[3] = (b"STR#", str_chunk, 301, LE);
        entries
    };
    for (str_chunk, fails) in [(3, false), (2, true)] {
        dir.file(
            "GameData/Objects/LampGlobals.iff",
            &with_map(&globals, &globals_entries(str_chunk)),
        );
        let index = Index::build(&Install::detect(&dir.0).unwrap()).unwrap();
        let catalog = Catalog::build(&index).unwrap();
        let linked = Program::link(&index, &catalog, [0x1111]);
        assert_eq!(linked.is_err(), fails);
    }
}

// Test-only subprocess deadline to fail loudly if a FIFO open blocks.
#[cfg(unix)]
#[allow(clippy::disallowed_methods)]
fn run_fifo_probe(test_name: &str, env_var: &str, root: &std::path::Path) -> Result<(), String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let mut child = Command::new(exe)
        .args(["--exact", test_name, "--test-threads=1", "--nocapture"])
        .env(env_var, root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn {test_name}: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait().map_err(|e| format!("try_wait: {e}"))? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "FIFO probe {test_name} timed out after 5s (likely blocked opening FIFO)"
                ));
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    if !status.success() {
        return Err(format!(
            "FIFO probe {test_name} failed ({status}):\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }
    if !stdout.contains("1 passed") {
        return Err(format!(
            "FIFO probe {test_name} did not run expected test:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }
    Ok(())
}

#[test]
fn career_texts_end_only_when_work_holds_no_career_token() {
    #[cfg(unix)]
    if let Some(root) = std::env::var_os("SIMS_FIFO_PROBE_WORK") {
        let root = PathBuf::from(root);
        let index = Index::build(&Install::detect(&root).unwrap()).unwrap();
        let catalog = Catalog::build(&index).unwrap();
        let fifo = root.join("ExpansionShared/ExtrasFAR");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(
            !Program::link(&index, &catalog, [])
                .unwrap()
                .career_texts_end
        );
        return;
    }
    let dir = Dir::new("career-texts");
    let (index, catalog) = setup(&dir);
    // An install with Hot Date, which mounts `ExpansionShared` (an
    // archive with no members).
    dir.file("ExpansionPack3/ExpansionPack3.far", &far(&[]));
    let link = |dir: &Dir| {
        let index = Index::build(&Install::detect(&dir.0).unwrap()).unwrap();
        let catalog = Catalog::build(&index).unwrap();
        Program::link(&index, &catalog, [])
            .unwrap()
            .career_texts_end
    };
    // No work.iff.
    assert!(
        !Program::link(&index, &catalog, [])
            .unwrap()
            .career_texts_end
    );
    // A career's strings may hold `$`, but no career token; the string
    // tables no `$` at all, deleted ones included.
    let work = |table: Vec<u8>, flags: u16| {
        iff_with_flags(&[(b"CARR", 1, 0, carr(2)), (b"STR#", 4, flags | 0x10, table)])
    };
    dir.file("ExpansionShared/work.iff", &work(terminated(&["Clerk"]), 0));
    assert!(link(&dir));
    for (table, flags) in [
        (terminated(&["x$JobDesc:0"]), 0),
        (terminated(&["a $ sign"]), 0),
        (terminated(&["x$JobDesc:0"]), sims_formats::iff::DELETED),
        (localized(&[(3, "\x01$Job")]), 0),
        // No `$`, but entries with a language and none in US English,
        // which the loader reads again for English (0x47006b).
        (localized(&[(2, "\x01Clerk"), (2, "x")]), 0),
    ] {
        dir.file("ExpansionShared/work.iff", &work(table, flags));
        assert!(!link(&dir));
    }
    // A legacy table that holds a `$`.
    dir.file(
        "ExpansionShared/work.iff",
        &iff_with_flags(&[
            (b"CARR", 1, 0, carr(2)),
            (b"\0CST", 4, 0, b"^$Job^".to_vec()),
        ]),
    );
    assert!(!link(&dir));
    // A `work.fam` beside it, whose resources are searched too (0x46f539).
    dir.file("ExpansionShared/work.iff", &work(terminated(&["Clerk"]), 0));
    // Any other component of the `Work` set beside it, whose resources
    // are searched too (0x46f539, 0x46b627, 0x406a29).
    for companion in ["work.fam", "Work.wll", "work.flr", "work.dir"] {
        let path = format!("ExpansionShared/{companion}");
        dir.file(&path, b"");
        assert!(!link(&dir), "{companion}");
        std::fs::remove_file(dir.0.join(&path)).unwrap();
    }
    assert!(link(&dir));
    // So does one in an archive there, and an archive there with a member
    // the index leaves out (compressed), which may be one.
    for archive in [far(&[("work.wll", 0, b"")]), far(&[("other.iff", 9, b"x")])] {
        dir.file("ExpansionShared/ExpansionShared.far", &archive);
        assert!(!link(&dir));
    }
    dir.file(
        "ExpansionShared/ExpansionShared.far",
        &far(&[("other.iff", 0, b"")]),
    );
    assert!(link(&dir));
    // An archive is any file whose name ends in `far`, with no dot
    // (0x5ba86d to 0x5ba8a7); one that cannot be read may be any.
    for archive in [far(&[("work.fam", 0, b"")]), b"not an archive".to_vec()] {
        dir.file("ExpansionShared/ExtrasFAR", &archive);
        assert!(!link(&dir));
    }
    std::fs::remove_file(dir.0.join("ExpansionShared/ExtrasFAR")).unwrap();
    // An entry that is not a directory is tried as one (0x5ba7d4): a
    // dangling link or non-regular file (such as a named pipe, which must
    // fail closed without blocking on open) cannot be read as an archive;
    // a directory whose name ends in `far` is not an archive.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.0.join("gone"), dir.0.join("ExpansionShared/ExtrasFAR"))
            .unwrap();
        assert!(!link(&dir));
        std::fs::remove_file(dir.0.join("ExpansionShared/ExtrasFAR")).unwrap();
        assert!(link(&dir));

        let probe = run_fifo_probe(
            "career_texts_end_only_when_work_holds_no_career_token",
            "SIMS_FIFO_PROBE_WORK",
            &dir.0,
        );
        let _ = std::fs::remove_file(dir.0.join("ExpansionShared/ExtrasFAR"));
        probe.unwrap();
        assert!(link(&dir));
    }
    std::fs::create_dir(dir.0.join("ExpansionShared/ExtrasFAR")).unwrap();
    assert!(link(&dir));
    std::fs::remove_dir(dir.0.join("ExpansionShared/ExtrasFAR")).unwrap();
    // An archive member whose name holds a path is placed in a
    // subdirectory (0x5b9dbc), so is not the `Work` set's, though its
    // archive is in `ExpansionShared`.
    std::fs::remove_file(dir.0.join("ExpansionShared/work.iff")).unwrap();
    let member = work(terminated(&["Clerk"]), 0);
    for name in ["work.iff", "sub\\work.iff", "sub/work.iff"] {
        dir.file(
            "ExpansionShared/ExpansionShared.far",
            &far(&[(name, member.len() as u32, &member)]),
        );
        assert_eq!(link(&dir), name == "work.iff", "{name}");
    }
    // Nor is one beside `work.iff`.
    dir.file("ExpansionShared/work.iff", &member);
    dir.file(
        "ExpansionShared/ExpansionShared.far",
        &far(&[("sub\\work.fam", 0, b"")]),
    );
    assert!(link(&dir));
    dir.file(
        "ExpansionShared/ExpansionShared.far",
        &far(&[("other.iff", 0, b"")]),
    );
    // A resource map that does not match the chunks, or that states a
    // table's flags without 0x10.
    let chunks = [
        (b"CARR", 1, 0x10, carr(2)),
        (b"STR#", 4, 0x10, terminated(&["Clerk"])),
    ];
    for (entries, holds) in [
        (vec![(b"CARR", 0, 1, 0x10), (b"STR#", 1, 4, 0x10)], true),
        (vec![(b"CARR", 0, 1, 0x10), (b"STR#", 1, 4, 0)], false),
    ] {
        dir.file("ExpansionShared/work.iff", &with_map(&chunks, &entries));
        assert_eq!(link(&dir), holds);
    }
    // A map that does not match fails the link (`careers`).
    dir.file(
        "ExpansionShared/work.iff",
        &with_map(&chunks, &[(b"CARR", 0, 1, 0x10), (b"STR#", 0, 4, 0x10)]),
    );
    let index = Index::build(&Install::detect(&dir.0).unwrap()).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    assert!(Program::link(&index, &catalog, []).is_err());
    dir.file("ExpansionShared/work.iff", &work(terminated(&["Clerk"]), 0));
    assert!(link(&dir));
    // Another `work.iff` elsewhere, which the original does not open as
    // `Work` but the index finds too, before it or after it.
    for other in ["GameData/Objects/work.iff", "TemplateNPCs/work.iff"] {
        dir.file(other, &work(terminated(&["Clerk"]), 0));
        assert!(!link(&dir), "{other}");
        std::fs::remove_file(dir.0.join(other)).unwrap();
    }
    assert!(link(&dir));
    // With no pack that mounts `ExpansionShared`, the index finds only
    // another `work.iff`, which `careers` reads, while the original opens
    // `Work` there still.
    std::fs::remove_file(dir.0.join("ExpansionPack3/ExpansionPack3.far")).unwrap();
    dir.file(
        "GameData/Objects/work.iff",
        &work(terminated(&["Clerk"]), 0),
    );
    assert!(!link(&dir));
    // Only one, but not where the original opens `Work`.
    std::fs::remove_file(dir.0.join("ExpansionShared/work.iff")).unwrap();
    assert!(!link(&dir));
}

#[test]
fn the_default_labels_hold_no_token_only_when_no_string_table_of_them_holds_a_dollar() {
    #[cfg(unix)]
    if let Some(root) = std::env::var_os("SIMS_FIFO_PROBE_LABELS") {
        let root = PathBuf::from(root);
        assert!(!sims_vm::labels_hold_no_token(&root).unwrap());
        return;
    }
    let dir = Dir::new("labels");
    let (_, _) = setup(&dir);
    let install = Install::detect(&dir.0).unwrap();
    assert!(!sims_vm::labels_hold_no_token(&install.root).unwrap());
    let labels =
        |table: Vec<u8>, flags: u16| iff_with_flags(&[(b"STR#", 142, flags | 0x10, table)]);
    dir.file(
        sims_vm::LABEL_FILE,
        &labels(terminated(&["OK", "Cancel"]), 0),
    );
    assert!(sims_vm::labels_hold_no_token(&install.root).unwrap());
    for (table, flags) in [
        (terminated(&["OK", "$Job"]), 0),
        (terminated(&["$Job"]), sims_formats::iff::DELETED),
        // No `$`, but none in US English: the loader, its fallback on
        // (0x435fee), reads it again for English (0x47006b), which may
        // take `\x01` for a language and load `Quit`.
        (localized(&[(2, "\x01Quit"), (2, "x")]), 0),
    ] {
        dir.file(sims_vm::LABEL_FILE, &labels(table, flags));
        assert!(!sims_vm::labels_hold_no_token(&install.root).unwrap());
    }
    // A legacy CST table of the ID, which the loader falls back to
    // (0x4702c7), counts too; one of another ID does not.
    for (id, holds) in [(142, false), (143, true)] {
        dir.file(
            sims_vm::LABEL_FILE,
            &iff_with_flags(&[
                (b"STR#", 142, 0x10, terminated(&["OK"])),
                (b"\0CST", id, 0, b"$Job".to_vec()),
            ]),
        );
        assert_eq!(sims_vm::labels_hold_no_token(&install.root).unwrap(), holds);
    }
    dir.file(sims_vm::LABEL_FILE, &labels(terminated(&["OK"]), 0));
    assert!(sims_vm::labels_hold_no_token(&install.root).unwrap());
    // A resource map that points ID 142 at another chunk: the original
    // reads that chunk (0x41bfdd to 0x41c032, 0x41d313 to 0x41d341).
    let chunks = [
        (b"STR#", 142, 0x10, terminated(&["OK"])),
        (b"STR#", 143, 0x10, terminated(&["$Neighbor"])),
    ];
    for (entries, holds) in [
        (vec![(b"STR#", 0, 142, 0x10), (b"STR#", 1, 143, 0x10)], true),
        (vec![(b"STR#", 1, 142, 0x10)], false),
        // A table read in a way not ported may be read past its end.
        (vec![(b"STR#", 0, 142, 0), (b"STR#", 1, 143, 0x10)], false),
        // Another ID's is not read.
        (vec![(b"STR#", 0, 142, 0x10), (b"STR#", 1, 143, 0)], true),
    ] {
        dir.file(sims_vm::LABEL_FILE, &with_map(&chunks, &entries));
        assert_eq!(sims_vm::labels_hold_no_token(&install.root).unwrap(), holds);
    }
    // A table with fewer strings than its count is read past its end.
    dir.file(
        sims_vm::LABEL_FILE,
        &labels(b"\xff\xff\x02\0OK\0".to_vec(), 0),
    );
    assert!(!sims_vm::labels_hold_no_token(&install.root).unwrap());
    dir.file(sims_vm::LABEL_FILE, &labels(terminated(&["OK"]), 0));
    // Any other component of the `UIText` set beside it.
    for companion in ["UIText.fam", "uitext.wll", "UIText.flr", "UIText.dir"] {
        let path = format!("GameData/{companion}");
        dir.file(&path, b"");
        assert!(
            !sims_vm::labels_hold_no_token(&install.root).unwrap(),
            "{companion}"
        );
        std::fs::remove_file(dir.0.join(&path)).unwrap();
    }
    // An archive there that cannot be read as a regular file (a dangling
    // link or a named pipe, which must fail closed without blocking on
    // open, 0x5ba7d4) may hold any member; a directory whose name ends in
    // `far` is not an archive.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.0.join("gone"), dir.0.join("GameData/UIFAR")).unwrap();
        assert!(!sims_vm::labels_hold_no_token(&install.root).unwrap());
        std::fs::remove_file(dir.0.join("GameData/UIFAR")).unwrap();
        assert!(sims_vm::labels_hold_no_token(&install.root).unwrap());

        let fifo = dir.0.join("GameData/UIFAR");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success());
        let probe = run_fifo_probe(
            "the_default_labels_hold_no_token_only_when_no_string_table_of_them_holds_a_dollar",
            "SIMS_FIFO_PROBE_LABELS",
            &install.root,
        );
        let _ = std::fs::remove_file(&fifo);
        probe.unwrap();
        assert!(sims_vm::labels_hold_no_token(&install.root).unwrap());
    }
    std::fs::create_dir(dir.0.join("GameData/UIFAR")).unwrap();
    assert!(sims_vm::labels_hold_no_token(&install.root).unwrap());
    std::fs::remove_dir(dir.0.join("GameData/UIFAR")).unwrap();
}

/// Minimal valid 1x1 24-bit Windows BMP (58 bytes), decodable by
/// `sims_formats::bitmap::bmp`.
fn tiny_bmp() -> Vec<u8> {
    let mut bmp = vec![0u8; 58];
    bmp[0..2].copy_from_slice(b"BM");
    bmp[2..6].copy_from_slice(&58u32.to_le_bytes()); // file size
    bmp[10..14].copy_from_slice(&54u32.to_le_bytes()); // pixel offset
    bmp[14..18].copy_from_slice(&40u32.to_le_bytes()); // DIB header size
    bmp[18..22].copy_from_slice(&1i32.to_le_bytes()); // width = 1
    bmp[22..26].copy_from_slice(&1i32.to_le_bytes()); // height = 1
    bmp[26..28].copy_from_slice(&1u16.to_le_bytes()); // planes = 1
    bmp[28..30].copy_from_slice(&24u16.to_le_bytes()); // bpp = 24
    bmp[34..38].copy_from_slice(&4u32.to_le_bytes()); // image size
    bmp
}

/// Raw IFF bytes for a single object definition plus extra flagged chunks.
fn iff_bytes(chunks: &[(&[u8; 4], u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
    for (tag, id, flags, data) in chunks {
        bytes.extend(*tag);
        bytes.extend((76 + data.len() as u32).to_be_bytes());
        bytes.extend(id.to_be_bytes());
        bytes.extend(flags.to_be_bytes());
        bytes.extend([0; 64]);
        bytes.extend(data);
    }
    bytes
}

/// Minimal 1-entry FAR v1 archive holding `(name, bytes)`.
fn far_archive(name: &str, payload: &[u8]) -> Vec<u8> {
    let manifest_offset = 16u32 + payload.len() as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"FAR!byAZ");
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&manifest_offset.to_le_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(&1u32.to_le_bytes()); // entry count
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&(name.len() as u32).to_le_bytes());
    out.extend_from_slice(name.as_bytes());
    out
}

#[test]
fn portrait_2002_uses_neutral_flags_and_marks_far_members_read_only() {
    let dir = Dir::new("portrait-2002");
    dir.file("GameData/Behavior.iff", &iff_bytes(&[]));
    dir.file("GameData/Global/Global.iff", &iff_bytes(&[]));

    // 1. Loose file with neutral (high byte 0) BMP_ 2002 -> has_portrait_2002 = true, read_only = false.
    let neutral_guid = 0x9000_0001u32;
    dir.file(
        "GameData/Objects/neutral.iff",
        &iff_bytes(&[
            (b"OBJD", 128, 0x0010, objd(neutral_guid, 0, 0)),
            (b"BMP_", 2002, 0x0010, tiny_bmp()),
        ]),
    );

    // 2. Loose file with English-only (high byte 1) BMP_ 2002 -> has_portrait_2002 = false
    // because 0x41bab1 -> 0x41bad8 forces the lookup key's high byte to 0.
    let lang1_guid = 0x9000_0002u32;
    dir.file(
        "GameData/Objects/lang1.iff",
        &iff_bytes(&[
            (b"OBJD", 128, 0x0010, objd(lang1_guid, 0, 0)),
            (b"BMP_", 2002, 0x0110, tiny_bmp()),
        ]),
    );

    // 3. Archive (.far) member without BMP_ 2002 -> read_only = true, has_portrait_2002 = false.
    let archived_guid = 0x9000_0003u32;
    let archived_iff = iff_bytes(&[(b"OBJD", 128, 0x0010, objd(archived_guid, 0, 0))]);
    dir.file(
        "GameData/Objects/Archived.far",
        &far_archive("Archived.iff", &archived_iff),
    );

    // 4. Loose file with uncompressed 32-bit 1x1 BMP_ 2002 -> has_portrait_2002 = true
    // because the native decoder (`0x4ded4a` -> `0x4df06a..0x4df122`) accepts it.
    let bmp32_guid = 0x9000_0004u32;
    let mut bmp32 = tiny_bmp();
    bmp32[28..30].copy_from_slice(&32u16.to_le_bytes());
    dir.file(
        "GameData/Objects/bmp32.iff",
        &iff_bytes(&[
            (b"OBJD", 128, 0x0010, objd(bmp32_guid, 0, 0)),
            (b"BMP_", 2002, 0x0010, bmp32),
        ]),
    );

    // 5. Loose file with 57-byte 1x1 24-bit BMP_ 2002 (missing unused final-row
    // padding byte) -> has_portrait_2002 = true because `0x4df117` advances
    // past padding and exits at `0x4df122` without reading it.
    let bmp57_guid = 0x9000_0005u32;
    dir.file(
        "GameData/Objects/bmp57.iff",
        &iff_bytes(&[
            (b"OBJD", 128, 0x0010, objd(bmp57_guid, 0, 0)),
            (b"BMP_", 2002, 0x0010, tiny_bmp()[..57].to_vec()),
        ]),
    );

    // 6. Loose file with truncated (55-byte) BMP_ 2002 -> has_portrait_2002 = false.
    let truncated_guid = 0x9000_0006u32;
    dir.file(
        "GameData/Objects/truncated.iff",
        &iff_bytes(&[
            (b"OBJD", 128, 0x0010, objd(truncated_guid, 0, 0)),
            (b"BMP_", 2002, 0x0010, tiny_bmp()[..55].to_vec()),
        ]),
    );

    let install = Install::detect(&dir.0).unwrap();
    let index = Index::build(&install).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let program = Program::link(
        &index,
        &catalog,
        [
            neutral_guid,
            lang1_guid,
            archived_guid,
            bmp32_guid,
            bmp57_guid,
            truncated_guid,
        ],
    )
    .unwrap();

    let lib = |guid| {
        let obj = catalog.object(guid).unwrap();
        &program.libraries[&obj.resource]
    };
    assert!(lib(neutral_guid).has_portrait_2002);
    assert!(!lib(neutral_guid).read_only);
    assert!(!lib(lang1_guid).has_portrait_2002);
    assert!(!lib(lang1_guid).read_only);
    assert!(!lib(archived_guid).has_portrait_2002);
    assert!(lib(archived_guid).read_only);
    assert!(lib(bmp32_guid).has_portrait_2002);
    assert!(!lib(bmp32_guid).read_only);
    assert!(lib(bmp57_guid).has_portrait_2002);
    assert!(!lib(truncated_guid).has_portrait_2002);
}
