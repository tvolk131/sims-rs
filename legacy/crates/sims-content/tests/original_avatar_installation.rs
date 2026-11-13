use anyhow::{Result, ensure};
use sims_content::{
    Index,
    avatar_installation::{Installation, complete_collection_asset_bindings},
};
use sims_sim::pose::clothing::catalog_files::Host;
use std::collections::BTreeSet;

#[test]
#[ignore = "requires extracted Complete Collection files; emits private native-probe inputs"]
fn original_object_definition_normalization_inputs() -> Result<()> {
    use sims_content::avatar_installation::{ObjectVisit, ObjectVisitor};
    use sims_sim::{object_catalog::definition::normalize, object_scan::Paths};
    #[derive(Default)]
    struct Inventory(Vec<ObjectVisit>);
    impl ObjectVisitor for Inventory {
        fn count(&self) -> usize {
            0
        }
        fn file(&mut self, _: &mut Installation<'_>, visit: ObjectVisit) -> Result<()> {
            self.0.push(visit);
            Ok(())
        }
        fn changed(&mut self) -> Result<()> {
            anyhow::bail!("inventory cannot change a catalog")
        }
    }
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let mut installation =
        Installation::new(&index, complete_collection_asset_bindings(&index.root))?;
    let initial = installation.initial_mounts(b"./", 255, false)?;
    let paths = Paths {
        objects: initial[1].clone(),
        user_objects: initial[2].clone(),
        downloads: initial.last().unwrap().clone(),
        expansions: std::array::from_fn(|i| initial[3 + i].clone()),
        shared: initial[11].clone(),
        npcs: initial[12].clone(),
        ..Default::default()
    };
    let mut inventory = Inventory::default();
    installation.scan_objects(&paths, 255, 0, false, &mut inventory)?;
    let directories = installation.directories()?;
    let mut chunks = vec![];
    for visit in &inventory.0 {
        let split = visit
            .path
            .iter()
            .rposition(|b| matches!(b, b'/' | b'\\'))
            .map_or(0, |i| i + 1);
        let source = directories
            .source(&visit.path[..split], &visit.path[split..])?
            .ok_or_else(|| {
                anyhow::anyhow!("scanned IFF has no mounted source: {:?}", visit.path)
            })?;
        let resource = &index.resources[source];
        let iff = sims_formats::iff::Iff::parse(resource.read()?)?;
        for chunk in iff.resource_order(b"OBJD")? {
            let data = iff.data(chunk);
            let mut expected = data.to_vec();
            normalize(&mut expected).map_err(anyhow::Error::msg)?;
            chunks.push(serde_json::json!({"source":format!("{}#OBJD:{}",resource.locator,chunk.id), "data":data, "expected":expected}));
        }
    }
    ensure!(!chunks.is_empty());
    std::fs::write(
        repo.join("cache/native-object-definitions-original-input.json"),
        serde_json::to_vec(&serde_json::json!({"files":inventory.0.len(),"chunks":chunks}))?,
    )?;
    println!(
        "{} IFF files supplied {} original OBJD normalization cases (private cache only)",
        inventory.0.len(),
        chunks.len()
    );
    Ok(())
}

#[test]
#[ignore = "requires extracted Complete Collection files"]
fn original_object_scan_visits_sources_in_native_phase_and_file_order() -> Result<()> {
    use sims_content::avatar_installation::{ObjectVisit, ObjectVisitor};
    use sims_sim::object_scan::Paths;
    #[derive(Default)]
    struct Inventory {
        visits: Vec<ObjectVisit>,
    }
    impl ObjectVisitor for Inventory {
        // This diagnostic records IFF callbacks; it does not claim to create
        // OBJD catalog entries or populate the eligible-provider vector.
        fn count(&self) -> usize {
            0
        }
        fn file(&mut self, installation: &mut Installation<'_>, visit: ObjectVisit) -> Result<()> {
            ensure!(
                installation.exists(&visit.path)?,
                "scanned source absent: {:?}",
                visit.path
            );
            self.visits.push(visit);
            Ok(())
        }
        fn changed(&mut self) -> Result<()> {
            anyhow::bail!("inventory cannot notify a catalog change")
        }
    }
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let mut installation =
        Installation::new(&index, complete_collection_asset_bindings(&index.root))?;
    let initial = installation.initial_mounts(b"./", 255, false)?;
    // Explicit already-constructed object paths from the selected initial
    // registration sequence. People is deliberately omitted from this asset-
    // only inventory; neighborhood population is separate. No all-roots mount.
    let paths = Paths {
        objects: initial[1].clone(),
        user_objects: initial[2].clone(),
        downloads: initial.last().unwrap().clone(),
        expansions: std::array::from_fn(|i| initial[3 + i].clone()),
        shared: initial[11].clone(),
        npcs: initial[12].clone(),
        ..Default::default()
    };
    let before = installation
        .manager
        .directories()
        .map(|(d, f)| (d.to_vec(), f.clone()))
        .collect::<Vec<_>>();
    let archives = installation
        .manager
        .archives()
        .map(|(p, id)| (p.to_vec(), id))
        .collect::<Vec<_>>();
    let mut inventory = Inventory::default();
    installation.scan_objects(&paths, 255, 0, false, &mut inventory)?;
    ensure!(!inventory.visits.is_empty() && installation.open_failures().is_empty());
    ensure!(
        before
            == installation
                .manager
                .directories()
                .map(|(d, f)| (d.to_vec(), f.clone()))
                .collect::<Vec<_>>()
    );
    ensure!(
        archives
            == installation
                .manager
                .archives()
                .map(|(p, id)| (p.to_vec(), id))
                .collect::<Vec<_>>()
    );
    let mut counts = std::collections::BTreeMap::new();
    for visit in &inventory.visits {
        *counts.entry(visit.category).or_insert(0usize) += 1;
    }
    std::fs::write(
        repo.join("cache/original-object-scan.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "scope":"ordered native IFF callbacks over explicitly bound sources and initial registrations; OBJD registration/provider construction remains pending",
            "people_path":"omitted from this explicit asset-only inventory; neighborhood scan is separate",
            "filesystem_order":"dot entries then Windows-1252 names sorted by ASCII-insensitive bytes",
            "archives":archives.len(),"directory_maps":before.len(),"counts_by_category":counts,"visits":inventory.visits
        }))?,
    )?;
    eprintln!(
        "{} original IFF callbacks across {} categories; {} archives, {} directory maps unchanged",
        inventory.visits.len(),
        counts.len(),
        archives.len(),
        before.len()
    );
    Ok(())
}

#[test]
#[ignore = "requires extracted Complete Collection files"]
fn original_virtual_installation_maps_sources_without_creator_or_installer_files() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let bindings = complete_collection_asset_bindings(&index.root);
    let mut installation = Installation::new(&index, bindings.clone())?;
    let initial_paths = installation.initial_mounts(b"./", 255, false)?;
    ensure!(initial_paths.first() == Some(&b"./GameData/Global/".to_vec()));
    ensure!(initial_paths.last() == Some(&b"./Downloads/".to_vec()));
    let initial_archives = installation.manager.archives().count();
    let initial_directories = installation.manager.directories().count();
    // Explicit diagnostic mount input, NOT a claim that original startup
    // registers all roots recursively in this sequence. The remaining object
    // scanner/initial manager lifetime work is documented separately.
    for binding in &bindings {
        installation.register(&binding.directory, true, true)?;
    }
    ensure!(
        installation.open_failures().is_empty(),
        "virtual installation failed to open an archive"
    );
    let mut dirs = installation.directories()?;
    let mut sources = BTreeSet::new();
    let mut ndx = BTreeSet::new();
    let mut entries = 0;
    for (directory, files) in installation.manager.directories() {
        let projected = dirs.files(directory).unwrap().unwrap();
        ensure!(files.bucket_count() == projected.bucket_count());
        ensure!(
            files
                .iter()
                .map(|(n, _)| n)
                .eq(projected.iter().map(|(n, _)| n))
        );
        for (name, source) in projected.iter() {
            entries += 1;
            if let Some(id) = source {
                let resource = &index.resources[*id];
                ensure!(
                    !resource.locator.starts_with("Creator_") && !resource.locator.starts_with('_')
                );
                ensure!(
                    bindings
                        .iter()
                        .any(|b| resource.source.starts_with(&b.physical))
                );
                sources.insert(*id);
                if name.ends_with(b".ndx") {
                    ndx.insert(*id);
                }
            }
        }
    }
    ensure!(
        ndx.len() == 94,
        "expected all 94 original mounted NDX files, got {}",
        ndx.len()
    );
    ensure!(
        installation
            .manager
            .directory(b"./Deluxe/Skins/")
            .map_err(anyhow::Error::msg)?
            .is_some()
    );
    let report = serde_json::json!({
        "scope": "read-only virtual filesystem and explicit diagnostic recursive mount sequence; NOT original startup registration order or live Reset",
        "filesystem_order": "dot entries, then Windows-1252 names sorted by ASCII-insensitive bytes",
        "native_initial_mount_paths": initial_paths.iter().map(|p| String::from_utf8_lossy(p)).collect::<Vec<_>>(),
        "native_initial_archives": initial_archives,
        "native_initial_directories": initial_directories,
        "bindings": bindings.iter().map(|b| serde_json::json!({"virtual": String::from_utf8_lossy(&b.directory), "physical": b.physical})).collect::<Vec<_>>(),
        "archives": installation.manager.archives().count(),
        "directories": installation.manager.directories().count(),
        "map_entries": entries,
        "resolved_source_ranges": sources.len(),
        "indexes": ndx.iter().map(|&id| &index.resources[id].locator).collect::<Vec<_>>()
    });
    std::fs::write(
        repo.join("cache/original-avatar-virtual-installation.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    eprintln!(
        "mapped {} sources, {} directory entries, {} archives, {} NDX",
        sources.len(),
        entries,
        installation.manager.archives().count(),
        ndx.len()
    );
    Ok(())
}
