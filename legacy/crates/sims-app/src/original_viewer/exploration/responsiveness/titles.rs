//! Keep resource-ID namespaces distinct without replacing authentic but odd
//! shipped catalog text with filenames or guessed human labels.
use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sims-app-catalog-label-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn chunk(tag: &[u8; 4], id: u16, label: &str, bytes: &[u8]) -> Vec<u8> {
    let mut result = tag.to_vec();
    result.extend_from_slice(&((76 + bytes.len()) as u32).to_be_bytes());
    result.extend_from_slice(&id.to_be_bytes());
    result.extend_from_slice(&16_u16.to_be_bytes());
    let mut name = [0; 64];
    name[..label.len()].copy_from_slice(label.as_bytes());
    result.extend_from_slice(&name);
    result.extend_from_slice(bytes);
    result
}
fn iff(chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE".to_vec();
    bytes.resize(64, 0);
    for chunk in chunks {
        bytes.extend_from_slice(chunk);
    }
    bytes
}
fn definition(id: u16, catalog: u16) -> Vec<u8> {
    let mut bytes = vec![0; 216];
    bytes[..4].copy_from_slice(&138_u32.to_le_bytes());
    bytes[18..20].copy_from_slice(&4_u16.to_le_bytes());
    bytes[28..32].copy_from_slice(&(u32::from(id) + 100).to_le_bytes());
    bytes[82..84].copy_from_slice(&catalog.to_le_bytes());
    chunk(
        b"OBJD",
        id,
        "Definition fallback is not the catalog title",
        &bytes,
    )
}
fn table(id: u16, name: &str) -> Vec<u8> {
    let mut bytes = vec![0xfd, 0xff, 1, 0, 1];
    bytes.extend_from_slice(name.as_bytes());
    bytes.extend_from_slice(&[0, 0]);
    chunk(b"CTSS", id, "", &bytes)
}
fn fixture() -> Result<(Fixture, sims_vm::Program, sims_vm::World)> {
    let root = Fixture::new()?;
    std::fs::write(
        root.0.join("private.iff"),
        iff(&[
            definition(10, 2000),
            definition(20, 2000),   // Two OBJD definitions sharing one catalog.
            definition(2000, 3000), // OBJD ID coincides with another catalog ID.
            definition(30, 4000),   // Missing catalog.
            definition(40, 5000),   // Present but empty catalog name.
            definition(50, 6000),   // Native CATS shadows this CTSS.
            table(2000, "Shared catalog title"),
            table(3000, "Distinct catalog title"),
            table(5000, ""),
            table(6000, "Shadowed CTSS title"),
            chunk(b"CATS", 6000, "", b"Legacy title\0Description\0"),
        ]),
    )?;
    // A matching OBJD ID and CTSS ID in another source cannot supply a label.
    std::fs::write(
        root.0.join("global.iff"),
        iff(&[definition(10, 2000), table(2000, "Other source title")]),
    )?;
    let index = Index::scan(&root.0)?;
    let program = sims_content::vm::load_program(&index, "private.iff", "global.iff")?;
    let mut world = sims_vm::World::default();
    for (id, object, source) in [
        (1, 10, "private.iff"),
        (2, 20, "private.iff"),
        (3, 2000, "private.iff"),
        (4, 30, "private.iff"),
        (5, 40, "private.iff"),
        (6, 50, "private.iff"),
        (7, 10, "global.iff"),
    ] {
        world.objects.insert(
            id,
            sims_vm::Object {
                definition: Some(sims_vm::Definition {
                    source: source.into(),
                    object,
                    guid: u32::from(object) + 100,
                    slots: 0,
                }),
                ..Default::default()
            },
        );
    }
    Ok((root, program, world))
}

#[test]
fn catalog_labels_follow_definition_links_not_coincident_resource_ids() -> Result<()> {
    let (_root, program, world) = fixture()?;
    let before = serde_json::to_vec(&(&program, &world))?;
    for (object, expected) in [
        (1, "Shared catalog title"),
        (2, "Shared catalog title"),
        (3, "Distinct catalog title"),
        (7, "Other source title"),
    ] {
        ensure!(object_catalog_label(&program, &world, object) == Some(expected));
    }
    ensure!(before == serde_json::to_vec(&(&program, &world))?);
    Ok(())
}

#[test]
fn absent_and_shadowed_catalog_labels_remain_explicit_boundaries() -> Result<()> {
    let (_root, program, world) = fixture()?;
    // The VM catalog currently omits CATS and empty/missing names. The client
    // preserves its Object-ID placeholder; it must not invent a filename name
    // or borrow a catalog from a different module. Native '*' fallback is a
    // separate already-documented loader boundary, not implemented here.
    for object in [4, 5, 6, 99] {
        ensure!(object_catalog_label(&program, &world, object).is_none());
    }
    Ok(())
}
