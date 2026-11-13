use anyhow::{Result, ensure};
use sims_content::{
    Index,
    avatar_installation::{Binding, Installation, complete_collection_asset_bindings},
    object_catalog::{Catalog, Settings},
    object_resources::{NOOP_CALLBACK, Overlay, Resources},
};
use sims_sim::object_scan::Paths;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "sims-object-owner-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn chunk(tag: &[u8; 4], id: u16, data: &[u8]) -> Vec<u8> {
    let mut b = tag.to_vec();
    b.extend(((76 + data.len()) as u32).to_be_bytes());
    b.extend(id.to_be_bytes());
    b.extend(0u16.to_be_bytes());
    b.extend(format!("resource-{id}").bytes());
    b.resize(76, 0);
    b.extend(data);
    b
}
fn iff(chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut b = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1".to_vec();
    b.resize(64, 0);
    for c in chunks {
        b.extend(c);
    }
    b
}
fn objd(version: u32, guid: u32, filtered: bool) -> Vec<u8> {
    let mut b = vec![0; 160];
    b[..4].copy_from_slice(&version.to_le_bytes());
    b[28..32].copy_from_slice(&guid.to_le_bytes());
    b[32] = filtered as u8;
    b
}
fn archive(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut b = b"FAR!byAZ".to_vec();
    b.extend(1u32.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    let mut m = (entries.len() as i32).to_le_bytes().to_vec();
    for (name, data) in entries {
        for n in [data.len(), data.len(), b.len(), name.len()] {
            m.extend((n as u32).to_le_bytes());
        }
        m.extend(name.bytes());
        b.extend(data);
    }
    let offset = b.len() as u32;
    b[12..16].copy_from_slice(&offset.to_le_bytes());
    b.extend(m);
    b
}
fn installation(index: &Index) -> Result<Installation<'_>> {
    let mut i = Installation::new(
        index,
        vec![Binding {
            directory: b"./Base/".to_vec(),
            physical: index.root.clone(),
        }],
    )?;
    i.register(b"./Base/", true, true)?;
    Ok(i)
}
fn settings(filter_flagged: bool) -> Settings {
    Settings {
        filter_flagged,
        global_directory: b"./Base/".to_vec(),
        global_people: 0,
        global: None,
    }
}

#[test]
fn buy_product_table_fallback_and_legacy_names_use_actual_resource_precedence() -> Result<()> {
    let root = Root::new();
    let mut table = vec![1, 0, 8, 0];
    table.resize(36, 0); // One version8 interaction, no motive advertisements.
    let semi = iff(&[chunk(b"TTAB", 10, &table)]);
    std::fs::write(root.0.join("semi.iff"), &semi)?;
    std::fs::write(root.0.join("global.iff"), &semi)?;
    let mut originals = vec![];
    for (ordinal, name) in [
        "empty",
        "fallback",
        "global_only",
        "graphic",
        "part",
        "master",
        "person",
    ]
    .into_iter()
    .enumerate()
    {
        let mut definition = objd(138, ordinal as u32 + 100, false);
        definition.resize(216, 0);
        definition[14..16].copy_from_slice(&10i16.to_le_bytes());
        definition[82..84].copy_from_slice(&2000u16.to_le_bytes());
        if matches!(name, "graphic" | "part" | "person") {
            definition[8] = 1;
        }
        if matches!(name, "part" | "master") {
            definition[20] = 7;
            definition[22..24]
                .copy_from_slice(&(if name == "master" { -1i16 } else { 0 }).to_le_bytes());
        }
        if name == "person" {
            definition[18] = 2;
        }
        let mut chunks = vec![chunk(b"OBJD", 1, &definition)];
        if name != "global_only" {
            chunks.push(chunk(b"GLOB", 1, b"semi\0"));
        }
        if name == "empty" {
            chunks.push(chunk(b"TTAB", 10, &[0, 0]));
        }
        if name == "graphic" {
            chunks.push(chunk(b"CATS", 2000, b"Legacy name\0Legacy description\0"));
            chunks.push(chunk(b"CTSS", 2000, b"invalid but shadowed"));
        }
        let bytes = iff(&chunks);
        let path = root.0.join(format!("{name}.iff"));
        std::fs::write(&path, &bytes)?;
        originals.push((path, bytes));
    }
    let index = Index::scan(&root.0)?;
    let i = installation(&index)?;
    let mut resources = Resources::new(&index, i.directories()?, i.directory_search());
    let global = resources
        .acquire(b"./Base/global", 0)
        .map_err(anyhow::Error::msg)?;
    let mut config = settings(false);
    config.global = global;
    let mut catalog = Catalog::new(resources, config);
    for name in [
        "empty",
        "fallback",
        "global_only",
        "graphic",
        "part",
        "master",
        "person",
    ] {
        sims_sim::object_catalog::register_file(
            format!("./Base/{name}.iff").as_bytes(),
            0,
            0,
            &mut catalog,
        )
        .map_err(anyhow::Error::msg)?;
    }
    let before = catalog.definitions.clone();
    let products = catalog.buy_products(1).map_err(anyhow::Error::msg)?;
    ensure!(catalog.definitions == before);
    ensure!(
        products
            .iter()
            .map(|p| p.source.guid)
            .collect::<std::collections::BTreeSet<_>>()
            == [101, 103, 105].into_iter().collect()
    );
    let graphic = products.iter().find(|p| p.source.guid == 103).unwrap();
    ensure!(graphic.name == b"Legacy name" && graphic.description == b"Legacy description");
    ensure!(
        products
            .iter()
            .filter(|p| p.source.guid != 103)
            .all(|p| p.name == b"*resource-1")
    );
    for (path, bytes) in originals {
        ensure!(std::fs::read(path)? == bytes);
    }
    Ok(())
}

#[test]
fn clean_checkpoint_keeps_native_write_state_and_rejects_pending_allocations() -> Result<()> {
    use sims_sim::object_catalog::composite::Chunk;
    let root = Root::new();
    let original = iff(&[chunk(b"BMP_", 2002, b"old faces")]);
    let path = root.0.join("person.iff");
    std::fs::write(&path, &original)?;
    let index = Index::scan(&root.0)?;
    let overlay = Overlay::copy(&index, [index.find("person.iff")?]).map_err(anyhow::Error::msg)?;
    let i = installation(&index)?;
    let mut resources = Resources::owned(
        std::sync::Arc::new(index.clone()),
        i.directories()?,
        i.directory_search(),
        overlay,
    )
    .map_err(anyhow::Error::msg)?;
    let file = resources
        .acquire(b"./Base/person", 1)
        .map_err(anyhow::Error::msg)?
        .unwrap();
    let buffer = resources
        .allocate(b"replacement faces".to_vec())
        .map_err(anyhow::Error::msg)?;
    ensure!(resources.checkpoint().is_err());
    resources
        .add(
            file,
            Chunk {
                handle: buffer,
                tag: u32::from_be_bytes(*b"BMP_"),
                id: 2002,
                name: b"faces",
                variant: 0,
                converted: 1,
            },
        )
        .map_err(anyhow::Error::msg)?;
    ensure!(resources.checkpoint().is_err());
    resources
        .release_chunk(file, buffer)
        .map_err(anyhow::Error::msg)?;
    let saved = resources.checkpoint().map_err(anyhow::Error::msg)?;
    let i = installation(&index)?;
    let mut resumed = Resources::owned(
        std::sync::Arc::new(index.clone()),
        i.directories()?,
        i.directory_search(),
        saved.overlay.clone(),
    )
    .map_err(anyhow::Error::msg)?;
    ensure!(
        resumed
            .acquire(b"./Base/person", 1)
            .map_err(anyhow::Error::msg)?
            == Some(file)
    );
    resumed
        .restore_checkpoint(&serde_json::from_slice(&serde_json::to_vec(&saved)?)?)
        .map_err(anyhow::Error::msg)?;
    ensure!(resumed.checkpoint().map_err(anyhow::Error::msg)? == saved);
    let handle = resumed
        .by_id(file, u32::from_be_bytes(*b"BMP_"), 2002, 0)
        .map_err(anyhow::Error::msg)?;
    ensure!(resumed.data(handle).map_err(anyhow::Error::msg)? == b"replacement faces");
    resumed
        .release_chunk(file, handle)
        .map_err(anyhow::Error::msg)?;
    // Reopening must not erase the native modified flag and thereby turn an
    // unsupported map-rebuilding close into a falsely successful operation.
    ensure!(resumed.release(file).unwrap_err().contains("modified"));
    ensure!(std::fs::read(path)? == original);
    Ok(())
}

#[test]
fn writable_chunks_preserve_alias_ownership_and_reopen_from_the_saved_overlay() -> Result<()> {
    use sims_sim::object_catalog::composite::Chunk;
    let root = Root::new();
    let original = iff(&[
        chunk(b"BMP_", 2002, b"old faces"),
        chunk(b"OBJD", 128, b"definition"),
    ]);
    let path = root.0.join("person.iff");
    std::fs::write(&path, &original)?;
    let index = Index::scan(&root.0)?;
    let initial = Overlay::copy(&index, [index.find("person.iff")?]).map_err(anyhow::Error::msg)?;
    let i = installation(&index)?;
    let mut r = Resources::with_overlay(
        &index,
        i.directories()?,
        i.directory_search(),
        initial.clone(),
    )
    .map_err(anyhow::Error::msg)?;
    let overlay = (|| -> Result<Overlay, String> {
        let file = r.acquire(b"./Base/person", 1)?.unwrap();
        assert!(r.writable(file)?);
        let old = r.by_id(file, u32::from_be_bytes(*b"BMP_"), 2002, 0)?;
        r.delete(file, old)?;
        assert_eq!(r.error(file)?, 0);
        assert!(r.data(old).is_err() && r.data_mut(old).is_err());
        let buffer = r.allocate(b"replacement faces".to_vec())?;
        assert!(r.chunk_source(buffer).is_err());
        let input = |name: &'static [u8]| Chunk {
            handle: buffer,
            tag: u32::from_be_bytes(*b"BMP_"),
            id: 2002,
            name,
            variant: 0,
            converted: 1,
        };
        r.add(file, input(b"first name"))?;
        assert_eq!(r.error(file)?, 0);
        assert_eq!(
            r.by_id(file, u32::from_be_bytes(*b"BMP_"), 2002, 0)?,
            buffer
        );
        // Native aggregate add detaches a reused input before deleting the
        // previous record. Caller allocation survives that replacement.
        r.add(file, input(b"faces"))?;
        assert_eq!(r.error(file)?, 0);
        assert_eq!(r.data(buffer)?, b"replacement faces");
        r.release_chunk(file, buffer)?;
        assert_eq!(r.error(file)?, 0);
        assert!(r.data(buffer).is_err());
        r.flush(file)?;
        assert_eq!(r.error(file)?, 0);
        let reloaded = r.by_id(file, u32::from_be_bytes(*b"BMP_"), 2002, 0)?;
        assert_ne!(reloaded, buffer);
        assert_eq!(r.data(reloaded)?, b"replacement faces");
        r.release_chunk(file, reloaded)?;
        r.overlay()
    })()
    .map_err(anyhow::Error::msg)?;
    ensure!(overlay != initial && std::fs::read(path)? == original);
    overlay.validate(&index).map_err(anyhow::Error::msg)?;
    let i = installation(&index)?;
    let mut reopened = Resources::with_overlay(
        &index,
        i.directories()?,
        i.directory_search(),
        serde_json::from_slice(&serde_json::to_vec(&overlay)?)?,
    )
    .map_err(anyhow::Error::msg)?;
    (|| -> Result<(), String> {
        let file = reopened.acquire(b"./Base/person", 1)?.unwrap();
        for (tag, id, expected) in [
            (b"BMP_", 2002, b"replacement faces".as_slice()),
            (b"OBJD", 128, b"definition".as_slice()),
        ] {
            let h = reopened.by_id(file, u32::from_be_bytes(*tag), id, 0)?;
            assert_eq!(reopened.data(h)?, expected);
            if id == 2002 {
                assert_eq!(reopened.name(file, h)?, b"faces");
            }
            reopened.release_chunk(file, h)?;
        }
        reopened.release(file)?;
        assert_eq!(reopened.live_counts(), (0, 0, 0));
        Ok(())
    })()
    .map_err(anyhow::Error::msg)?;
    Ok(())
}

#[test]
fn copied_resource_access_preserves_originals_identity_and_native_query_errors() -> Result<()> {
    let root = Root::new();
    let original = archive(&[
        ("person.iff", iff(&[chunk(b"OBJD", 1, b"primary")])),
        ("person.stx", iff(&[chunk(b"OBJD", 2, b"copied companion")])),
        ("readonly.iff", iff(&[chunk(b"OBJD", 3, b"read only")])),
    ]);
    let path = root.0.join("Objects.far");
    std::fs::write(&path, &original)?;
    let index = Index::scan(&root.0)?;
    let source = index.find("person.stx")?;
    let copied = Overlay::copy(&index, [source]).map_err(anyhow::Error::msg)?;
    assert!(Overlay::copy(&index, [source, source]).is_err());
    let saved = serde_json::to_vec(&copied)?;
    for overlay_enabled in [false, true] {
        for people in [0, 1] {
            let i = installation(&index)?;
            let overlay = if overlay_enabled {
                serde_json::from_slice(&saved)?
            } else {
                Overlay::default()
            };
            let mut resources =
                Resources::with_overlay(&index, i.directories()?, i.directory_search(), overlay)
                    .map_err(anyhow::Error::msg)?;
            let mut run = || -> Result<(), String> {
                let id = resources.acquire(b"./Base/person", people)?.unwrap();
                assert!(resources.valid(id)?);
                assert_eq!(
                    resources.by_id(id, u32::from_be_bytes(*b"BMP_"), 999, 0)?,
                    0
                );
                assert_eq!(resources.error(id)?, -98);
                assert_eq!(resources.writable(id)?, overlay_enabled && people != 0);
                assert_eq!(resources.error(id)?, 0);
                let handle = resources.by_id(id, u32::from_be_bytes(*b"OBJD"), 2, 0)?;
                assert_eq!(resources.data(handle)?, b"copied companion");
                assert_eq!(
                    resources.chunk_source(handle)?,
                    index.resources[source].locator
                );
                resources.release(id)?;
                assert_eq!(resources.live_counts(), (0, 0, 0));
                let id = resources.acquire(b"./Base/readonly", 1)?.unwrap();
                assert!(resources.valid(id)?);
                assert!(!resources.writable(id)?);
                resources.release(id)?;
                Ok(())
            };
            run().map_err(anyhow::Error::msg)?;
            if overlay_enabled {
                assert_eq!(resources.overlay().map_err(anyhow::Error::msg)?, copied);
            }
            assert_eq!(std::fs::read(&path)?, original);
        }
    }
    // Same length and locator, but a different source must not rebind a save.
    let mut changed = original.clone();
    changed[16 + 64 + 76 + b"primary".len() + 64 + 76] ^= 1;
    std::fs::write(&path, changed)?;
    assert!(
        copied
            .validate(&index)
            .unwrap_err()
            .contains("source changed")
    );
    Ok(())
}

#[test]
fn shared_owner_preserves_component_identity_release_reload_and_last_reference_cleanup()
-> Result<()> {
    let root = Root::new();
    std::fs::create_dir(root.0.join("Sub"))?;
    let raw = objd(136, 7, false);
    std::fs::write(
        root.0.join("Objects.far"),
        archive(&[
            ("chair.iff", iff(&[chunk(b"OBJD", 1, &raw)])),
            ("chair.stx", iff(&[chunk(b"OBJD", 2, b"second component")])),
            (
                "Sub/chair.iff",
                iff(&[chunk(b"OBJD", 8, b"distinct file, same cache key")]),
            ),
        ]),
    )?;
    std::fs::write(
        root.0.join("loose.iff"),
        iff(&[chunk(b"OBJD", 3, b"loose file")]),
    )?;
    // Bad IFF discarded; a valid later companion still opens the aggregate.
    std::fs::write(root.0.join("partial.iff"), b"invalid header")?;
    std::fs::write(
        root.0.join("partial.stx"),
        iff(&[chunk(b"OBJD", 4, b"survivor")]),
    )?;
    let index = Index::scan(&root.0)?;
    let i = installation(&index)?;
    let mut r = Resources::new(&index, i.directories()?, i.directory_search());
    let mut check = || -> Result<(), String> {
        let chair = r.acquire(b"./Base/chair", 0)?.unwrap();
        assert_eq!(r.live_counts(), (1, 2, 0));
        let alias = r.acquire(b"./Base/Sub/chair", 0)?.unwrap();
        assert_eq!(alias, chair);
        assert_eq!(r.filename(chair)?, b"./Base/chair");
        assert_eq!(r.count(chair, u32::from_be_bytes(*b"OBJD"))?, 2);
        let a = r.ordinal(chair, u32::from_be_bytes(*b"OBJD"), 1, NOOP_CALLBACK)?;
        let b = r.ordinal(chair, u32::from_be_bytes(*b"OBJD"), 2, NOOP_CALLBACK)?;
        assert_ne!(a, b);
        assert_eq!(r.id(chair, b)?, 2);
        assert_eq!(r.name(chair, b)?, b"resource-2");
        assert_eq!(r.data(b)?, b"second component");
        sims_sim::object_catalog::definition::normalize(r.data_mut(a)?)?;
        assert_eq!(r.data(a)?.len(), 216);
        r.release_chunk(chair, a)?;
        assert_eq!(r.error(chair)?, 0);
        assert!(r.data(a).is_err());
        r.release_chunk(chair, a)?;
        assert_eq!(r.error(chair)?, -99);
        let reloaded = r.by_id(chair, u32::from_be_bytes(*b"OBJD"), 1, NOOP_CALLBACK)?;
        assert_eq!(r.data(reloaded)?, raw);
        assert_eq!(
            r.by_id(chair, u32::from_be_bytes(*b"OBJD"), 1, 0)?,
            reloaded
        );
        r.release(chair)?;
        assert!(r.valid(alias)?);
        r.release(alias)?;
        assert_eq!(r.live_counts(), (0, 0, 0));
        let other = r.acquire(b"./Base/Sub/chair", 0)?.unwrap();
        let h = r.ordinal(other, u32::from_be_bytes(*b"OBJD"), 1, 0)?;
        assert_eq!(r.id(other, h)?, 8);
        r.release(other)?;
        for (name, id) in [(b"./Base/loose".as_slice(), 3), (b"./Base/partial", 4)] {
            let file = r.acquire(name, 1)?.unwrap();
            let h = r.ordinal(file, u32::from_be_bytes(*b"OBJD"), 1, 0)?;
            assert_eq!(r.id(file, h)?, id);
            r.release(file)?;
            assert_eq!(r.live_counts(), (0, 0, 0));
        }
        assert!(r.acquire(b"./Base/missing", 0)?.is_none());
        assert_eq!(r.live_counts(), (0, 0, 0));
        Ok(())
    };
    check().map_err(anyhow::Error::msg)
}

#[test]
fn catalog_registers_synchronously_filters_and_retains_real_provider_order() -> Result<()> {
    let root = Root::new();
    std::fs::write(
        root.0.join("chair.iff"),
        iff(&[
            chunk(b"OBJD", 1, &objd(136, 7, false)),
            chunk(b"OBJD", 2, &objd(138, 8, true)),
            chunk(b"GLOB", 1, b"semi\0"),
        ]),
    )?;
    std::fs::write(root.0.join("semi.iff"), iff(&[]))?;
    let index = Index::scan(&root.0)?;
    for filter in [false, true] {
        let mut i = installation(&index)?;
        let resources = Resources::new(&index, i.directories()?, i.directory_search());
        let mut c = Catalog::new(resources, settings(filter));
        let paths = Paths {
            downloads: b"./Base/".to_vec(),
            ..Default::default()
        };
        i.scan_objects(&paths, 0, 0, true, &mut c)?;
        assert_eq!(c.definitions.entries.len(), if filter { 1 } else { 2 });
        assert_eq!(c.notifications, 1);
        assert_eq!(
            c.providers().map_err(anyhow::Error::msg)?,
            [b".\\base\\".to_vec()]
        );
        let entry = &c.definitions.entries[0];
        assert_eq!(entry.data.len(), 216);
        assert_eq!(&entry.data[..4], &138u32.to_le_bytes());
        assert!(entry.semiglobal.is_some());
        assert_eq!(c.sprites.entries.len(), 1);
        assert_eq!(
            c.sprites.entries[0].references,
            c.definitions.entries.len() as u32
        );
        let before = c.definitions.clone();
        i.scan_objects(&paths, 0, 0, true, &mut c)?;
        assert_eq!(c.definitions, before);
        assert_eq!(c.notifications, 1);
        // Only GLOB stays loaded: OBJD was normalized, copied, then discarded.
        assert_eq!(c.resources.live_counts().2, 1);
    }
    Ok(())
}

#[test]
fn present_directory_queries_stop_explicitly_instead_of_hiding_assets() -> Result<()> {
    let root = Root::new();
    std::fs::create_dir(root.0.join("thing.dir"))?;
    let index = Index::scan(&root.0)?;
    let i = installation(&index)?;
    let mut r = Resources::new(&index, i.directories()?, i.directory_search());
    let id = r
        .acquire(b"./Base/thing", 0)
        .map_err(anyhow::Error::msg)?
        .unwrap();
    assert!(
        r.count(id, u32::from_be_bytes(*b"OBJD"))
            .unwrap_err()
            .contains("present DIR")
    );
    assert!(
        r.acquire(b"./Base/missing", 0)
            .unwrap_err()
            .contains("stopped at unresolved boundary")
    );
    Ok(())
}

#[test]
#[ignore = "requires extracted Complete Collection assets; emits private catalog/provider evidence"]
fn original_catalog_supplies_ordered_avatar_providers() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let mut i = Installation::new(&index, complete_collection_asset_bindings(&index.root))?;
    let initial = i.initial_mounts(b"./", 255, false)?;
    let paths = Paths {
        objects: initial[1].clone(),
        user_objects: initial[2].clone(),
        downloads: initial.last().unwrap().clone(),
        expansions: std::array::from_fn(|n| initial[3 + n].clone()),
        shared: initial[11].clone(),
        npcs: initial[12].clone(),
        ..Default::default()
    };
    let resources = Resources::new(&index, i.directories()?, i.directory_search());
    // Explicit asset-only diagnostic inputs. Neighborhood People, manager global
    // acquisition and derivation of these startup flags are separate work.
    let mut c = Catalog::new(
        resources,
        Settings {
            filter_flagged: false,
            global_directory: initial[0].clone(),
            global_people: 0,
            global: None,
        },
    );
    i.scan_objects(&paths, 255, 0, false, &mut c)?;
    let providers = c.providers().map_err(anyhow::Error::msg)?;
    ensure!(!providers.is_empty());
    ensure!(c.visits.len() == 1231 && c.definitions.entries.len() == 5289);
    ensure!(providers.len() == 70 && c.sprites.entries.len() == 1230);
    let retained = c.definitions.entries.len()
        + c.definitions
            .entries
            .iter()
            .filter(|e| e.semiglobal.is_some())
            .count();
    ensure!(
        c.resources
            .cache
            .entries()
            .map(|e| e.references as usize)
            .sum::<usize>()
            == retained
    );
    ensure!(
        c.sprites
            .entries
            .iter()
            .map(|e| e.references as usize)
            .sum::<usize>()
            == c.definitions.entries.len()
    );
    let loaded = i.load_catalogs(b"./", b"./GameData/", 255, &providers)?;
    ensure!(
        loaded.catalog.index_read_failures.is_empty()
            && loaded.catalog.read_failures.is_empty()
            && loaded.catalog.startup_open_failures.is_empty()
    );
    use sims_formats::avatar::index::Kind;
    let asset_counts = [Kind::Skeleton, Kind::Appearance, Kind::Animation]
        .map(|kind| loaded.catalog.iter(kind).count());
    let mut clothing = sims_content::clothing_catalog::Names::default();
    clothing.load(&loaded, &mut i.directories()?)?;
    ensure!(!clothing.bodies.is_empty() && !clothing.textures.names.is_empty());
    std::fs::write(
        repo.join("cache/original-clothing-catalog-names.json"),
        serde_json::to_vec(&clothing)?,
    )?;
    let mut rows = vec![];
    for entry in &c.definitions.entries {
        rows.push(serde_json::json!({
            "resource":entry.key.resource,"chunk":entry.key.chunk,"guid":u32::from_le_bytes(entry.data[28..32].try_into().unwrap()),
            "resource_path":c.resources.filename(entry.key.resource).map_err(anyhow::Error::msg)?,
            "semiglobal":entry.semiglobal,"sprite":entry.sprite_resource,"flags":entry.flags,"name":entry.name,
        }));
    }
    let report = serde_json::json!({
        "scope":"actual mounted resources through native-matched component, cache, registration, sprite-pool and provider algorithms; explicit asset-only startup inputs; not a live Reset or full native startup comparison",
        "settings":{"filter_flagged":false,"global":null,"global_people":0,"people_path":"omitted"},
        "visits":c.visits,"entries":rows,"providers":providers,"sprites":c.sprites.entries.len(),
        "live_counts":c.resources.live_counts(),"component_opens":c.resources.component_opens().len(),
        "clothing_name_counts":[clothing.bodies.len(),clothing.textures.names.len()],"avatar_records":asset_counts,"retained_resource_references":retained,"avatar_visits":loaded.visits,"appearances":loaded.paths.appearances,"animations":loaded.paths.animations,"textures":loaded.textures,
        "avatar_index_failures":loaded.catalog.index_read_failures,"avatar_read_failures":loaded.catalog.read_failures,
        "avatar_startup_open_failures":loaded.catalog.startup_open_failures.len(),
    });
    std::fs::write(
        repo.join("cache/original-object-catalog-providers.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "{} file visits, {} registered definitions, {} provider directories, {:?} avatar visits",
        c.visits.len(),
        c.definitions.entries.len(),
        providers.len(),
        loaded.visits
    );
    Ok(())
}

#[test]
#[ignore = "requires original assets and the unfiltered original catalog reports"]
fn original_startup_filter_and_global_resource_preserve_clothing_inputs() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let mut installation =
        Installation::new(&index, complete_collection_asset_bindings(&index.root))?;
    let initial = installation.initial_mounts(b"./", 255, false)?;
    let paths = Paths {
        objects: initial[1].clone(),
        user_objects: initial[2].clone(),
        downloads: initial.last().unwrap().clone(),
        expansions: std::array::from_fn(|n| initial[3 + n].clone()),
        shared: initial[11].clone(),
        npcs: initial[12].clone(),
        ..Default::default()
    };
    let mut resources = Resources::new(
        &index,
        installation.directories()?,
        installation.directory_search(),
    );
    let global_path = [initial[0].as_slice(), b"Global"].concat();
    let global = resources
        .acquire(&global_path, 0)
        .map_err(anyhow::Error::msg)?;
    ensure!(global.is_some());
    // 413f7d/413f86 sets EDI=1; 41415f passes it to 43ae81. The
    // latter writes +1240. The fresh 43a81e constructor clears +1243.
    let mut catalog = Catalog::new(
        resources,
        Settings {
            filter_flagged: true,
            global_directory: initial[0].clone(),
            global_people: 0,
            global,
        },
    );
    installation.scan_objects(&paths, 255, 0, false, &mut catalog)?;
    ensure!(
        catalog
            .definitions
            .entries
            .iter()
            .all(|e| e.global == global)
    );
    let providers = catalog.providers().map_err(anyhow::Error::msg)?;
    let previous: serde_json::Value = serde_json::from_slice(&std::fs::read(
        repo.join("cache/original-object-catalog-providers.json"),
    )?)?;
    ensure!(
        serde_json::json!(providers) == previous["providers"],
        "native startup filter changes avatar providers"
    );
    let loaded = installation.load_catalogs(b"./", b"./GameData/", 255, &providers)?;
    let mut names = sims_content::clothing_catalog::Names::default();
    names.load(&loaded, &mut installation.directories()?)?;
    let earlier: sims_content::clothing_catalog::Names = serde_json::from_slice(&std::fs::read(
        repo.join("cache/original-clothing-catalog-names.json"),
    )?)?;
    ensure!(
        names == earlier,
        "native startup filter changes ordered clothing names"
    );
    let report = serde_json::json!({"scope":"Source-backed startup filter/global acquisition with flags from original 413ccc/43a81e/43ae81 listing; no claim of complete native startup. Neighborhood People and global animation/tuning initialization remain separate.",
        "filter_flagged":true,"global_people":0,"global":global,"global_path":global_path,
        "definitions":catalog.definitions.entries.len(),"visits":catalog.visits.len(),"providers":providers.len(),
        "clothing_name_counts":[names.bodies.len(),names.textures.names.len()],"same_ordered_providers_and_clothing_as_unfiltered":true});
    std::fs::write(
        repo.join("cache/original-filtered-clothing-catalog.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "{} filtered definitions, {} providers; global acquired and ordered clothing inputs unchanged",
        catalog.definitions.entries.len(),
        providers.len()
    );
    Ok(())
}
