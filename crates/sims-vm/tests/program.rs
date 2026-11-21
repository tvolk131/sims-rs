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
}

#[test]
fn career_texts_end_only_when_work_holds_no_career_token() {
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
        iff_with_flags(&[(b"CARR", 1, 0, carr(2)), (b"STR#", 4, flags, table)])
    };
    dir.file("ExpansionShared/work.iff", &work(terminated(&["Clerk"]), 0));
    assert!(link(&dir));
    for (table, flags) in [
        (terminated(&["x$JobDesc:0"]), 0),
        (terminated(&["a $ sign"]), 0),
        (terminated(&["x$JobDesc:0"]), sims_formats::iff::DELETED),
        (localized(&[(3, "\x01$Job")]), 0),
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
    let dir = Dir::new("labels");
    let (_, _) = setup(&dir);
    let install = Install::detect(&dir.0).unwrap();
    assert!(!sims_vm::labels_hold_no_token(&install.root).unwrap());
    let labels = |table: Vec<u8>, flags: u16| iff_with_flags(&[(b"STR#", 142, flags, table)]);
    dir.file(
        sims_vm::LABEL_FILE,
        &labels(terminated(&["OK", "Cancel"]), 0),
    );
    assert!(sims_vm::labels_hold_no_token(&install.root).unwrap());
    for (table, flags) in [
        (terminated(&["OK", "$Job"]), 0),
        (terminated(&["$Job"]), sims_formats::iff::DELETED),
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
                (b"STR#", 142, 0, terminated(&["OK"])),
                (b"\0CST", id, 0, b"$Job".to_vec()),
            ]),
        );
        assert_eq!(sims_vm::labels_hold_no_token(&install.root).unwrap(), holds);
    }
    dir.file(sims_vm::LABEL_FILE, &labels(terminated(&["OK"]), 0));
    assert!(sims_vm::labels_hold_no_token(&install.root).unwrap());
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
}
