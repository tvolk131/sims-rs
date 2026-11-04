use anyhow::Result;
use sims_content::avatar_installation::{Binding, Installation};
use sims_content::{Index, avatar_catalog::Catalog, avatar_mount::Sources};
use sims_formats::avatar::{catalog::Asset, index::Kind};
use sims_sim::pose::clothing::{
    catalog_files::Host,
    catalog_mount::{Found, Manager},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "sims-avatar-mount-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn archive(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut out = b"FAR!byAZ".to_vec();
    out.extend(1u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    let mut manifest = (entries.len() as i32).to_le_bytes().to_vec();
    for (name, bytes) in entries {
        for value in [
            bytes.len() as u32,
            bytes.len() as u32,
            out.len() as u32,
            name.len() as u32,
        ] {
            manifest.extend(value.to_le_bytes());
        }
        manifest.extend(name.as_bytes());
        out.extend(bytes);
    }
    let offset = out.len() as u32;
    out[12..16].copy_from_slice(&offset.to_le_bytes());
    out.extend(manifest);
    out
}
fn string(out: &mut Vec<u8>, name: &[u8]) {
    out.push(name.len() as u8);
    out.extend(name);
}
fn index_bytes() -> Vec<u8> {
    let mut out = 0i32.to_le_bytes().to_vec();
    out.extend(1i32.to_le_bytes());
    string(&mut out, b"outfit");
    string(&mut out, b"Outfit.CMX");
    out.extend(0i32.to_le_bytes());
    out
}
fn catalog(kind: u32) -> Vec<u8> {
    let mut out = 0i32.to_le_bytes().to_vec();
    out.extend(1i32.to_le_bytes());
    string(&mut out, b"outfit");
    for value in [kind, 0, 0, 0] {
        out.extend(value.to_le_bytes());
    }
    out
}
fn found(name: &str, attributes: u32) -> Found {
    Found {
        name: name.as_bytes().to_vec(),
        attributes,
    }
}

#[test]
fn object_scan_uses_original_maps_and_keeps_temporary_managers_private() -> Result<()> {
    use sims_content::avatar_installation::{ObjectVisit, ObjectVisitor};
    use sims_sim::{object_scan::Paths, pose::clothing::catalog_mount::Metadata};
    #[derive(Default)]
    struct Visitor {
        visits: Vec<ObjectVisit>,
        changed: usize,
    }
    impl ObjectVisitor for Visitor {
        fn count(&self) -> usize {
            self.visits.len()
        }
        fn file(&mut self, _: &mut Installation<'_>, visit: ObjectVisit) -> Result<()> {
            self.visits.push(visit);
            Ok(())
        }
        fn changed(&mut self) -> Result<()> {
            self.changed += 1;
            Ok(())
        }
    }
    let root = Root::new();
    std::fs::create_dir_all(root.0.join("Sub"))?;
    std::fs::create_dir_all(root.0.join("Skip.DIR"))?;
    std::fs::write(root.0.join("Chair.IFF"), b"loose object input")?;
    std::fs::write(root.0.join("Sub/Loose.iff"), b"nested object input")?;
    std::fs::write(root.0.join("Skip.DIR/Never.iff"), b"excluded directory")?;
    std::fs::write(root.0.join("Ignore.txt"), b"not an object")?;
    std::fs::write(
        root.0.join("Objects.far"),
        archive(&[
            ("Fridge.iff", vec![1]),
            ("ArchiveOnly/Virtual.iff", vec![2]),
        ]),
    )?;
    let index = Index::scan(&root.0)?;
    for mounted in [false, true] {
        let mut installation = Installation::new(
            &index,
            vec![Binding {
                directory: b"./Base/".to_vec(),
                physical: root.0.clone(),
            }],
        )?;
        if mounted {
            installation.register(b"./Base/", true, true)?;
        } else {
            // This child is globally registered despite an unregistered root.
            // The temporary traversal must neither consult nor reuse it.
            installation
                .manager
                .directory_mut(b"./Base/ArchiveOnly/")
                .unwrap()
                .insert(b"intruder.iff", Metadata::default())
                .unwrap();
        }
        let before = installation
            .manager
            .directories()
            .map(|(d, f)| (d.to_vec(), f.clone()))
            .collect::<Vec<_>>();
        let paths = Paths {
            objects: b"./Base/".to_vec(),
            people: b"./Base/".to_vec(),
            downloads: b"./Base/".to_vec(),
            ..Default::default()
        };
        let mut visitor = Visitor::default();
        installation.scan_objects(&paths, 0, 7, true, &mut visitor)?;
        let after = installation
            .manager
            .directories()
            .map(|(d, f)| (d.to_vec(), f.clone()))
            .collect::<Vec<_>>();
        assert_eq!(before, after);
        assert_eq!(visitor.changed, 1);
        let per_phase = if mounted { 4 } else { 3 };
        assert_eq!(visitor.visits.len(), per_phase * 3);
        let first = visitor.visits[..per_phase]
            .iter()
            .map(|v| v.path.clone())
            .collect::<Vec<_>>();
        for (i, (category, people)) in [(0, 7), (1, 1), (2, 7)].into_iter().enumerate() {
            let phase = &visitor.visits[i * per_phase..(i + 1) * per_phase];
            assert!(
                phase
                    .iter()
                    .all(|v| v.category == category && v.people == people)
            );
            assert_eq!(
                phase.iter().map(|v| v.path.clone()).collect::<Vec<_>>(),
                first
            );
        }
        assert!(
            visitor
                .visits
                .iter()
                .all(|v| !v.path.ends_with(b"intruder.iff")
                    && !v.path.ends_with(b"never.iff")
                    && !v.path.ends_with(b"ignore.txt"))
        );
        assert_eq!(
            visitor
                .visits
                .iter()
                .filter(|v| v.path.ends_with(b"virtual.iff"))
                .count(),
            if mounted { 3 } else { 0 }
        );
        assert!(
            visitor
                .visits
                .iter()
                .filter(|v| v.path.ends_with(b"fridge.iff"))
                .all(|v| v.writable == 0)
        );
        assert!(installation.open_failures().is_empty());
    }
    Ok(())
}

#[test]
fn virtual_installation_connects_native_paths_mounts_and_loaded_lazy_catalogs() -> Result<()> {
    let root = Root::new();
    std::fs::create_dir_all(root.0.join("Base/Skins"))?;
    std::fs::create_dir_all(root.0.join("Base/Animation"))?;
    for (name, kind) in [("z-last.far", 9), ("A-first.far", 7)] {
        std::fs::write(
            root.0.join("Base/Skins").join(name),
            archive(&[("Outfit.cmx.bcf", catalog(kind))]),
        )?;
    }
    let mut ndx = 0i32.to_le_bytes().to_vec();
    ndx.extend(1i32.to_le_bytes());
    string(&mut ndx, b"lazyoutfit");
    string(&mut ndx, b"Lazy.CMX");
    ndx.extend(0i32.to_le_bytes());
    let mut lazy = 0i32.to_le_bytes().to_vec();
    lazy.extend(1i32.to_le_bytes());
    string(&mut lazy, b"lazyoutfit");
    for value in [29u32, 0, 0, 0] {
        lazy.extend(value.to_le_bytes());
    }
    std::fs::write(
        root.0.join("Base/Animation/anim.far"),
        archive(&[("anim.ndx", ndx), ("Lazy.cmx.bcf", lazy)]),
    )?;
    let index = Index::scan(&root.0)?;
    let mut installation = Installation::new(
        &index,
        vec![Binding {
            directory: b"./GameData/".to_vec(),
            physical: root.0.join("Base"),
        }],
    )?;
    assert!(installation.physical_exists(b"GAMEdata//./sKins/")?);
    assert!(!installation.registered(b"./GameData/Skins/")?);
    assert!(!installation.exists(b"./GameData/Skins/Outfit.cmx.bcf")?);
    assert!(!installation.physical_exists(b"./GameData/Skins/Outfit.cmx.bcf")?);
    assert!(installation.physical_exists(b"./GameData/Skins/A-first.far\0ignored")?);
    assert!(!installation.physical_exists(b"./GameData/Skins/A-first.far/")?);
    assert!(!installation.physical_exists(b"\0ignored")?);
    assert!(installation.physical_exists(b"../Base/Skins").is_err());
    let mut loaded = installation.load_catalogs(b"./", b"./GameData/", 0, &[])?;
    assert_eq!(loaded.visits, [1, 1]);
    assert!(installation.exists(b"./GameData/Skins/Outfit.cmx.bcf")?);
    assert!(!installation.physical_exists(b"./GameData/Skins/Outfit.cmx.bcf")?);
    assert!(installation.registered(b"./GameData/Skins/")?);
    assert!(!installation.registered(b"./GameData/Skins")?);
    assert!(
        !installation.registered(b"./GameData/Textures2/")?,
        "missing scan cannot manufacture a registered directory"
    );
    assert!(installation.open_failures().is_empty());
    assert!(loaded.catalog.startup_open_failures.is_empty());
    assert!(loaded.catalog.read_failures.is_empty());
    let outfit = loaded.catalog.asset(Kind::Appearance, b"outfit").unwrap();
    assert_eq!(outfit.context, 1);
    assert_eq!(
        outfit.source,
        index.find("Base/Skins/A-first.far::Outfit.cmx.bcf")?
    );
    assert!(
        !loaded
            .catalog
            .get(Kind::Appearance, b"lazyoutfit")
            .unwrap()
            .attempted
    );
    let dirs = installation.directories()?;
    let asset = loaded
        .catalog
        .resolve(&index, &dirs, Kind::Appearance, b"lazyoutfit")?
        .unwrap();
    assert_eq!(asset.context, 2);
    assert_eq!(
        asset.source,
        index.find("Base/Animation/anim.far::Lazy.cmx.bcf")?
    );
    // The virtual filesystem is a snapshot. Later host files cannot silently
    // change the declared namespace or its ordering during this startup.
    std::fs::write(root.0.join("Base/Skins/new.txt"), b"new")?;
    assert!(!installation.physical_exists(b"./GameData/Skins/new.txt")?);
    Ok(())
}

#[test]
fn virtual_installation_rejects_ambiguous_bindings_and_preserves_exact_manager_keys() -> Result<()>
{
    let root = Root::new();
    std::fs::create_dir(root.0.join("Empty"))?;
    let index = Index::scan(&root.0)?;
    let binding = Binding {
        directory: b"./Virtual/".to_vec(),
        physical: root.0.join("Empty"),
    };
    assert!(Installation::new(&index, vec![binding.clone(), binding.clone()]).is_err());
    let mut installation = Installation::new(&index, vec![binding])?;
    installation.register(b"Virtual/", false, true)?;
    assert!(installation.registered(b"VIRTUAL/")?);
    assert!(!installation.registered(b"./Virtual/")?);
    installation.register(b"./Virtual/", true, true)?;
    assert!(installation.registered(b"./Virtual/")?);
    assert_eq!(installation.manager.directories().count(), 2);
    Ok(())
}

#[test]
fn source_mounts_preserve_loose_override_first_archive_and_catalog_ownership() -> Result<()> {
    let root = Root::new();
    for (name, kind) in [("First.far", 7), ("Second.far", 9)] {
        std::fs::write(
            root.0.join(name),
            archive(&[
                ("Wardrobe.ndx", index_bytes()),
                ("Outfit.cmx.bcf", catalog(kind)),
                ("Nested/keep.txt", vec![kind as u8]),
            ]),
        )?;
    }
    std::fs::write(root.0.join("Outfit.cmx.bcf"), catalog(11))?;
    let index = Index::scan(&root.0)?;
    for reverse in [false, true] {
        for loose_first in [false, true] {
            let mut sources = Sources::new(&index);
            sources.bind_archive(b"./Skins/First.far", &root.0.join("First.far"))?;
            sources.bind_archive(b"./Skins/Second.far", &root.0.join("Second.far"))?;
            let loose_id = index.find("Outfit.cmx.bcf")?;
            sources.bind_loose(b"./Skins/Outfit.cmx.bcf", loose_id)?;
            let mut entries = vec![found(".", 16), found("..", 16)];
            if loose_first {
                entries.push(found("Outfit.cmx.bcf", 1));
            }
            let order = if reverse {
                ["Second.far", "First.far"]
            } else {
                ["First.far", "Second.far"]
            };
            entries.extend(order.map(|n| found(n, 0)));
            if !loose_first {
                entries.push(found("Outfit.cmx.bcf", 1));
            }
            entries.push(found("Nested", 16));
            sources.set_directory(b"./Skins/", entries)?;
            sources.set_directory(b"./Skins/Nested/", vec![found(".", 16), found("..", 16)])?;
            let mut manager = Manager::default();
            sources.scan(&mut manager, b"./Skins/", true, true)?;
            assert!(sources.open_failures.is_empty());
            let files = manager.directory(b"./Skins/").unwrap().unwrap();
            let loose = files.get(b"Outfit.cmx.bcf").unwrap().unwrap();
            assert_eq!(
                (loose.archived, loose.offset, loose.size, loose.writable),
                (0, 0, 0, 0)
            );
            assert_eq!(
                loose.archive,
                if loose_first {
                    0
                } else if reverse {
                    2
                } else {
                    1
                }
            );
            let mut dirs = sources.catalog_directories(&manager)?;
            let mapped = dirs.files(b"./Skins/").unwrap().unwrap();
            assert_eq!(mapped.len(), files.len());
            assert_eq!(mapped.bucket_count(), files.bucket_count());
            assert_eq!(
                mapped.iter().map(|(n, _)| n).collect::<Vec<_>>(),
                files.iter().map(|(n, _)| n).collect::<Vec<_>>()
            );
            assert_eq!(mapped.get(b"First.far").unwrap(), Some(&None));
            assert_eq!(mapped.get(b"Nested").unwrap(), Some(&None));
            let mut catalog = Catalog::default();
            assert_eq!(
                catalog.register_paths(&index, &mut dirs, &[b"./Skins/".to_vec()], 17)?,
                1
            );
            let ndx = index.find(&format!("{}::Wardrobe.ndx", order[0]))?;
            assert_eq!(catalog.index_source(Kind::Appearance, b"outfit"), Some(ndx));
            let asset = catalog
                .resolve(&index, &dirs, Kind::Appearance, b"outfit")?
                .unwrap();
            assert_eq!(asset.source, loose_id);
            let Asset::Appearance(appearance) = &asset.value else {
                panic!("wrong asset");
            };
            assert_eq!(appearance.kind, 11);
            let nested = dirs.source(b"./Skins/Nested/", b"keep.txt")?.unwrap();
            assert_eq!(
                index.resources[nested].read()?,
                vec![if reverse { 9 } else { 7 }]
            );
        }
    }
    Ok(())
}

#[test]
fn missing_scan_inputs_are_errors_and_open_failures_remain_visible() -> Result<()> {
    let root = Root::new();
    let index = Index::scan(&root.0)?;
    let mut sources = Sources::new(&index);
    let mut manager = Manager::default();
    assert!(
        sources
            .scan(&mut manager, b"./Absent/", false, true)
            .is_err()
    );
    assert!(manager.directories().next().is_none());
    sources.set_directory(
        b"./Supplied/",
        vec![found(".", 16), found("..", 16), found("Missing.far", 0)],
    )?;
    sources.scan(&mut manager, b"./Supplied/", false, true)?;
    assert_eq!(
        sources.open_failures,
        vec![b".\\supplied\\missing.far".to_vec()]
    );
    assert!(manager.archives().next().is_none());
    let mut dirs = sources.catalog_directories(&manager)?;
    assert_eq!(dirs.files(b"./Supplied/").unwrap().unwrap().len(), 1);
    Ok(())
}
