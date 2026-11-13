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
