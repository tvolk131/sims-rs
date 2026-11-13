use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use sims_content::{
    Index,
    avatar_catalog::{Catalog, Directories},
    avatar_mount::Sources,
};
use sims_formats::avatar::index::Kind;
use sims_sim::pose::clothing::catalog_mount::{Found, Manager};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

// Source check input, not an assertion about Windows FindFirst ordering or
// installer aliases. The scanner itself is separately native-verified.
fn supplied_loose_directory(
    sources: &mut Sources<'_>,
    index: &Index,
    physical: &Path,
    virtual_path: &[u8],
) -> Result<()> {
    let mut entries = vec![];
    if physical.is_dir() {
        entries = vec![
            Found {
                name: b".".to_vec(),
                attributes: 16,
            },
            Found {
                name: b"..".to_vec(),
                attributes: 16,
            },
        ];
        let mut paths = std::fs::read_dir(physical)?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();
        for path in paths {
            let name = path
                .file_name()
                .unwrap()
                .to_str()
                .context("non-Unicode test filename")?;
            let (name, _, invalid) = encoding_rs::WINDOWS_1252.encode(name);
            ensure!(!invalid, "test filename not Windows-1252");
            entries.push(Found {
                name: name.into_owned(),
                attributes: if path.is_dir() { 16 } else { 0 },
            });
        }
        for (id, resource) in index.resources.iter().enumerate() {
            if !resource.archived && resource.source.parent() == Some(physical) {
                let (name, _, invalid) = encoding_rs::WINDOWS_1252.encode(&resource.name);
                ensure!(!invalid, "test loose filename not Windows-1252");
                sources.bind_loose(&[virtual_path, name.as_ref()].concat(), id)?;
            }
        }
    }
    sources.set_directory(virtual_path, entries)
}

#[test]
#[ignore = "requires original files and the private native NDX report"]
fn archived_index_names_resolve_to_actual_catalog_sources() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(
        repo.join("cache/native-original-avatar-index.json"),
    )?)?;
    let mut archives = BTreeMap::new();
    let mut resolved = [0; 3];
    let mut loose_references = 0;
    let mut loaded = [0; 3];
    let mut empty_motion_animations = 0;
    let mut empty_motion_channels = 0;
    let mut missing = vec![];
    let mut decoded_appearances = BTreeMap::new();
    let mut ordinary_sources = 0;
    let mut ordinary_records = [0; 3];
    let mut ordinary_failures: Vec<usize> = vec![];
    for row in report["cases"].as_array().context("native cases absent")? {
        let archive = row["source"]["archive"].as_str().unwrap();
        let ndx_name = row["source"]["name"].as_str().unwrap();
        let ndx_id = index.find(&format!("{archive}::{ndx_name}"))?;
        ensure!(
            format!("{:x}", Sha256::digest(index.resources[ndx_id].read()?))
                == row["sha256"].as_str().unwrap()
        );
        if !archives.contains_key(archive) {
            let physical = &index.resources[ndx_id].source;
            let filename = physical.file_name().unwrap().to_str().unwrap();
            let virtual_path = [b"./Supplied/".as_slice(), filename.as_bytes()].concat();
            let mut sources = Sources::new(&index);
            sources.bind_archive(&virtual_path, physical)?;
            sources.set_directory(
                b"./Supplied/",
                vec![
                    Found {
                        name: b".".to_vec(),
                        attributes: 16,
                    },
                    Found {
                        name: b"..".to_vec(),
                        attributes: 16,
                    },
                    Found {
                        name: filename.as_bytes().to_vec(),
                        attributes: 0,
                    },
                ],
            )?;
            let mut manager = Manager::default();
            sources.scan(&mut manager, b"./Supplied/", false, true)?;
            ensure!(manager.has_archives && sources.open_failures.is_empty());
            archives.insert(archive.to_string(), (manager, sources, BTreeSet::new()));
        }
        // One archive is mounted in isolation. Its complete original manifest
        // drives insertion order, synthetic directories and first-name wins.
        let split = |name: &[u8]| {
            name.iter()
                .rposition(|b| matches!(b, b'/' | b'\\'))
                .map_or(0, |n| n + 1)
        };
        let prefix = &ndx_name.as_bytes()[..split(ndx_name.as_bytes())];
        let virtual_directory = [b"./Supplied/".as_slice(), prefix].concat();
        let (manager, sources, scanned) = archives.get_mut(archive).unwrap();
        // Some indexes point outside their FAR to loose siblings. Apply the
        // verified scan's metadata overwrite, retaining the archived node's
        // hash position. Enumeration order/attributes are explicit test inputs.
        let loose_directory = index.resources[ndx_id]
            .source
            .parent()
            .unwrap()
            .join(std::str::from_utf8(prefix)?.replace('\\', "/"));
        if scanned.insert(virtual_directory.clone()) {
            supplied_loose_directory(sources, &index, &loose_directory, &virtual_directory)?;
            sources.scan(manager, &virtual_directory, false, false)?;
        }
        let mut dirs = sources.catalog_directories(manager)?;
        let mut catalog = Catalog::default();
        ensure!(
            catalog.load_startup_paths(
                &index,
                &mut dirs,
                &[virtual_directory.clone()],
                true,
                17,
                &mut 0
            )? == 1
        );
        ensure!(catalog.index_read_failures.is_empty() && catalog.startup_open_failures.is_empty());
        let mut ordinary = Catalog::default();
        ordinary_sources += ordinary.load_startup_paths(
            &index,
            &mut dirs,
            &[virtual_directory],
            false,
            99,
            &mut 0,
        )?;
        ordinary_failures.extend(&ordinary.read_failures);
        ensure!(ordinary.startup_open_failures.is_empty());
        for kind in [Kind::Skeleton, Kind::Appearance, Kind::Animation] {
            for record in ordinary.iter(kind) {
                let asset = ordinary
                    .asset(kind, &record.location.name)
                    .context("ordinary asset absent")?;
                ensure!(record.attempted && asset.context == 99 && asset.value.kind() == kind);
                ordinary_records[kind.index()] += 1;
            }
        }
        for kind in [Kind::Skeleton, Kind::Appearance, Kind::Animation] {
            let expected = row["original"]["maps"][kind.index()].as_array().unwrap();
            ensure!(catalog.iter(kind).count() == expected.len());
            for value in expected {
                let name: Vec<u8> = serde_json::from_value(value["name"].clone())?;
                let record = catalog.get(kind, &name).context("registered name absent")?;
                ensure!(catalog.index_source(kind, &name) == Some(ndx_id));
                ensure!(serde_json::to_value(&record.location.catalog)? == value["catalog"]);
                let source = match catalog.catalog_source(&dirs, kind, &name) {
                    Ok(source) => source,
                    Err(error) => {
                        missing.push(serde_json::json!({"index": index.resources[ndx_id].locator,
                            "name": String::from_utf8_lossy(&name), "error": error.to_string()}));
                        continue;
                    }
                };
                let asset = match catalog.resolve(&index, &dirs, kind, &name) {
                    Ok(Some(asset)) => asset,
                    result => {
                        missing.push(serde_json::json!({"index": index.resources[ndx_id].locator,
                            "name": String::from_utf8_lossy(&name), "catalog": index.resources[source].locator,
                            "error": match result { Err(e) => e.to_string(), _ => "load left record unresolved".into() }}));
                        continue;
                    }
                };
                ensure!(asset.source == source, "lazy load changed source ownership");
                ensure!(asset.context == 17 && asset.value.kind() == kind);
                if let sims_formats::avatar::catalog::Asset::Animation(a) = &asset.value {
                    let empty = a.motions.iter().filter(|m| m.frames == 0).count();
                    empty_motion_animations += usize::from(empty > 0);
                    empty_motion_channels += empty;
                }
                ensure!(
                    encoding_rs::WINDOWS_1252
                        .encode(asset.value.name())
                        .0
                        .as_ref()
                        == name
                );
                loaded[kind.index()] += 1;
                let id = catalog.get(kind, &name).unwrap().asset;
                catalog.resolve(&index, &dirs, kind, &name)?;
                ensure!(
                    catalog.get(kind, &name).unwrap().asset == id,
                    "second lookup replaced a loaded asset"
                );
                resolved[kind.index()] += 1;
                loose_references += usize::from(!index.resources[source].archived);
                if kind == Kind::Appearance {
                    let resource = &index.resources[source];
                    let appearances = decoded_appearances.entry(source).or_insert_with(|| {
                        sims_formats::avatar::read_bcf(&resource.read().unwrap())
                            .unwrap()
                            .appearances
                    });
                    ensure!(
                        appearances
                            .iter()
                            .any(|a| a.name.as_bytes().eq_ignore_ascii_case(&name)),
                        "NDX name {} absent from {}",
                        String::from_utf8_lossy(&name),
                        resource.locator
                    );
                }
            }
        }
    }
    let result = serde_json::json!({"indexes": report["cases"].as_array().unwrap().len(),
        "resolved": resolved, "loaded": loaded, "loose_references": loose_references,
        "empty_motion_animations": empty_motion_animations, "empty_motion_channels": empty_motion_channels,
        "decoded_appearance_catalogs": decoded_appearances.len(), "missing": missing,
        "mounted_archives": archives.len(),
        "ordinary_startup_sources": ordinary_sources,
        "ordinary_startup_records": ordinary_records,
        "ordinary_startup_read_failures": ordinary_failures,
        "scope": "Each NDX archive mounted with original manifest order, synthetic directories and full native file-map nodes; loose siblings overwrite through supplied sorted scan inputs. No installed/global archive order or Windows enumeration claim."});
    std::fs::write(
        repo.join("cache/original-avatar-index-links.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{result}");
    ensure!(missing.is_empty(), "unresolved original NDX catalog links");
    ensure!(resolved.iter().sum::<usize>() == 10225);
    ensure!(loaded == resolved);
    ensure!(loose_references == 192);
    Ok(())
}

#[test]
#[ignore = "requires original CMX files"]
fn original_text_catalogs_register_their_decoded_records() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let mut counts = [0; 2]; // installed content / Creator reference files
    let mut appearances = 0;
    for (source, resource) in index.resources.iter().enumerate() {
        if !resource.name.to_ascii_lowercase().ends_with(".cmx") {
            continue;
        }
        // Individual source loading verifies decoding/registration, not a
        // guessed global precedence among identically named installed files.
        let mut catalog = Catalog::default();
        let mut directories = Directories::default();
        directories.insert_first(b"./Supplied/", resource.name.as_bytes(), source)?;
        ensure!(
            catalog.load_startup_paths(
                &index,
                &mut directories,
                &[b"./Supplied/".to_vec()],
                false,
                17,
                &mut 0
            )? == 1
        );
        ensure!(catalog.read_failures.is_empty() && catalog.startup_open_failures.is_empty());
        let expected = sims_formats::avatar::read_cmx(&resource.read()?)?;
        ensure!(catalog.iter(Kind::Appearance).count() == expected.appearances.len());
        for value in expected.appearances {
            let key = encoding_rs::WINDOWS_1252.encode(&value.name).0;
            let actual = catalog
                .asset(Kind::Appearance, &key)
                .context("CMX appearance not registered")?;
            ensure!(actual.source == source && actual.context == 17);
            let sims_formats::avatar::catalog::Asset::Appearance(a) = &actual.value else {
                anyhow::bail!("wrong asset kind");
            };
            ensure!(
                a.name == value.name
                    && a.kind == value.kind
                    && a.bindings.len() == value.bindings.len()
            );
            for (a, b) in a.bindings.iter().zip(&value.bindings) {
                ensure!(a.bone == b.bone && a.mesh == b.mesh && a.censor == b.censor);
            }
            appearances += 1;
        }
        counts[usize::from(resource.locator.contains("Creator"))] += 1;
    }
    ensure!(counts.iter().sum::<usize>() == 304);
    let report = serde_json::json!({"catalogs": counts, "appearances": appearances,
        "scope": "Each CMX loaded independently; Creator references are validation inputs, not installed providers."});
    std::fs::write(
        repo.join("cache/original-avatar-cmx-loads.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
