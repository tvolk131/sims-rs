use anyhow::Result;
use sims_content::{
    Index, Resource,
    avatar_catalog::{Catalog, Directories},
};
use sims_formats::avatar::index::Kind;
use sims_sim::pose::clothing::catalog_index::Lazy;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Source {
    index: Index,
    dirs: Directories,
}
impl Source {
    fn new() -> Self {
        static SERIAL: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "sims-avatar-catalog-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self {
            index: Index {
                root,
                resources: vec![],
                archives: 0,
                diagnostics: vec![],
            },
            dirs: Directories::default(),
        }
    }
    fn add(&mut self, name: &str, bytes: &[u8]) -> usize {
        let path = self.index.root.join(name);
        std::fs::write(&path, bytes).unwrap();
        let id = self.index.resources.len();
        self.index.resources.push(Resource {
            locator: name.into(),
            name: name.into(),
            source: path,
            offset: 0,
            size: bytes.len() as u64,
            unpacked_size: bytes.len() as u64,
            archived: false,
            owned: None,
        });
        self.dirs
            .insert_first(b"./Supplied/", name.as_bytes(), id)
            .unwrap();
        id
    }
    fn register(&mut self, catalog: &mut Catalog, names: &[&[u8]]) -> Result<usize> {
        let mut bytes = 0i32.to_le_bytes().to_vec();
        bytes.extend((names.len() as i32).to_le_bytes());
        for name in names {
            string(&mut bytes, name);
            string(&mut bytes, b"Clothes.CMX");
        }
        bytes.extend(0i32.to_le_bytes());
        let source = self.add("Clothes.ndx", &bytes);
        assert_eq!(
            catalog.register_paths(&self.index, &mut self.dirs, &[b"./Supplied/".to_vec()], 17)?,
            1
        );
        Ok(source)
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.index.root);
    }
}
fn string(out: &mut Vec<u8>, name: &[u8]) {
    out.push(name.len() as u8);
    out.extend(name);
}
fn binary(names: &[&[u8]], complete: bool) -> Vec<u8> {
    let mut bytes = 0i32.to_le_bytes().to_vec();
    bytes.extend((names.len() as i32).to_le_bytes());
    for name in names {
        string(&mut bytes, name);
        for word in [7u32, 0, 0] {
            bytes.extend(word.to_le_bytes());
        }
    }
    if complete {
        bytes.extend(0i32.to_le_bytes());
    }
    bytes
}
fn location(name: &[u8]) -> Lazy {
    Lazy {
        name: vec![],
        directory: b"./Supplied/".to_vec(),
        catalog: name.to_vec(),
        context: 99,
    }
}

#[test]
fn plain_path_open_uses_first_readable_mounted_source() -> Result<()> {
    let mut source = Source::new();
    let first = source.add("First.bmp", b"first bytes");
    let later = source.add("Later.bmp", b"different later bytes");
    source.dirs.insert_first(b"./Early/", b"brush.bmp", first)?;
    source.dirs.insert_first(b"./Late/", b"BRUSH.BMP", later)?;
    let paths = vec![
        b"./Missing/".to_vec(),
        b"./Early/".to_vec(),
        b"./Late/".to_vec(),
    ];
    let open = || {
        source.dirs.open_plain_paths(
            &source.index,
            &paths,
            b"Brush",
            u32::from_be_bytes(*b"BMP\0"),
        )
    };
    assert_eq!(open()?, Some(first));
    std::fs::remove_file(&source.index.resources[first].source)?;
    assert_eq!(open()?, Some(later));
    std::fs::remove_file(&source.index.resources[later].source)?;
    assert_eq!(open()?, None);
    Ok(())
}

#[test]
fn one_lazy_load_fills_neighbors_and_keeps_distinct_index_and_decoded_ownership() -> Result<()> {
    let mut source = Source::new();
    let mut catalog = Catalog::default();
    let ndx = source.register(&mut catalog, &[b"first", b"caf\xe9"])?;
    let bcf = source.add(
        "Clothes.CMX.bcf",
        &binary(&[b"FIRST", b"caf\xe9", b"extra"], true),
    );
    catalog
        .resolve(&source.index, &source.dirs, Kind::Appearance, b"first")?
        .unwrap();
    assert_eq!(catalog.iter(Kind::Appearance).count(), 3);
    for name in [b"first".as_slice(), b"caf\xe9"] {
        assert_eq!(catalog.index_source(Kind::Appearance, name), Some(ndx));
        assert_eq!(
            catalog
                .get(Kind::Appearance, name)
                .unwrap()
                .location
                .context,
            17
        );
        let asset = catalog.asset(Kind::Appearance, name).unwrap();
        assert_eq!((asset.source, asset.context), (bcf, 17));
    }
    assert_eq!(catalog.index_source(Kind::Appearance, b"extra"), None);
    // A second indexed name was filled by the first load; no file is needed.
    std::fs::remove_file(&source.index.resources[bcf].source)?;
    assert!(
        catalog
            .resolve(&source.index, &source.dirs, Kind::Appearance, b"caf\xe9")?
            .is_some()
    );
    let other = source.add("Other.BCF", &binary(&[b"first", b"new"], true));
    assert_eq!(
        catalog.load_catalog(&source.index, other, &location(b"Other.BCF"))?,
        0
    );
    assert_eq!(
        catalog.asset(Kind::Appearance, b"first").unwrap().source,
        bcf
    );
    assert_eq!(
        catalog.asset(Kind::Appearance, b"new").unwrap().source,
        other
    );
    assert_eq!(
        catalog
            .get(Kind::Appearance, b"new")
            .unwrap()
            .location
            .catalog,
        b"other.bcf"
    );
    Ok(())
}

#[test]
fn missing_catalog_is_negative_cached_even_when_a_provider_arrives_later() -> Result<()> {
    let mut source = Source::new();
    let mut catalog = Catalog::default();
    source.register(&mut catalog, &[b"first"])?;
    assert!(
        catalog
            .resolve(&source.index, &source.dirs, Kind::Appearance, b"first")?
            .is_none()
    );
    assert!(catalog.get(Kind::Appearance, b"first").unwrap().attempted);
    let bcf = source.add("Clothes.CMX.bcf", &binary(&[b"first", b"extra"], true));
    assert!(
        catalog
            .resolve(&source.index, &source.dirs, Kind::Appearance, b"first")?
            .is_none()
    );
    catalog.load_catalog(&source.index, bcf, &location(b"Clothes.CMX.bcf"))?;
    assert!(catalog.asset(Kind::Appearance, b"first").is_none());
    assert!(catalog.asset(Kind::Appearance, b"extra").is_some());
    Ok(())
}

#[test]
fn later_count_failure_retains_records_and_consumes_the_requested_attempt() -> Result<()> {
    let mut source = Source::new();
    let mut catalog = Catalog::default();
    source.register(&mut catalog, &[b"first", b"absent"])?;
    let bcf = source.add("Clothes.CMX.bcf", &binary(&[b"first"], false));
    assert!(
        catalog
            .resolve(&source.index, &source.dirs, Kind::Appearance, b"absent")?
            .is_none()
    );
    assert!(catalog.get(Kind::Appearance, b"absent").unwrap().attempted);
    assert_eq!(
        catalog.asset(Kind::Appearance, b"first").unwrap().source,
        bcf
    );
    assert_eq!(catalog.read_failures, [bcf]);
    catalog.resolve(&source.index, &source.dirs, Kind::Appearance, b"absent")?;
    assert_eq!(catalog.read_failures, [bcf]);
    Ok(())
}

#[test]
fn cmx_records_share_the_registry_and_keep_partial_success() -> Result<()> {
    let mut source = Source::new();
    let mut catalog = Catalog::default();
    source.register(&mut catalog, &[b"first"])?;
    // Existing lazy ownership is kept even when a different ordinary CMX fills it.
    let cmx = source.add(
        "Outfits.CMX",
        b"// test\nversion 300\n0\n2\nFIRST\n7\n0\n0\nsecond\n7\n0\n0\n",
    );
    assert_eq!(
        catalog.load_catalog(&source.index, cmx, &location(b"Outfits.CMX"))?,
        u32::MAX
    );
    let record = catalog.get(Kind::Appearance, b"first").unwrap();
    assert_eq!(record.location.catalog, b"Clothes.CMX.bcf");
    assert_eq!(record.location.context, 17);
    let asset = catalog.asset(Kind::Appearance, b"first").unwrap();
    assert_eq!((asset.source, asset.context), (cmx, 99));
    assert!(catalog.asset(Kind::Appearance, b"second").is_some());
    assert_eq!(catalog.read_failures, [cmx]);
    Ok(())
}

#[test]
fn unsupported_record_is_not_silently_cached_as_native_failure() -> Result<()> {
    let mut source = Source::new();
    let mut catalog = Catalog::default();
    source.register(&mut catalog, &[b"first", b"second"])?;
    let mut bytes = binary(&[b"first", b"second"], true);
    let at = bytes.len() - 12;
    bytes[at..at + 4].copy_from_slice(&1u32.to_le_bytes()); // unsupported property bag
    source.add("Clothes.CMX.bcf", &bytes);
    let error = catalog
        .resolve(&source.index, &source.dirs, Kind::Appearance, b"second")
        .unwrap_err();
    assert!(error.to_string().contains("support boundary"));
    assert!(!catalog.get(Kind::Appearance, b"second").unwrap().attempted);
    assert!(catalog.asset(Kind::Appearance, b"first").is_some());
    assert!(catalog.read_failures.is_empty());
    assert!(
        catalog
            .resolve(&source.index, &source.dirs, Kind::Appearance, b"first")
            .is_err(),
        "a partially loaded record must not hide an unresolved decoder boundary"
    );
    Ok(())
}

#[test]
fn ndx_registration_after_an_ordinary_load_does_not_reassign_loaded_ownership() -> Result<()> {
    let mut source = Source::new();
    let mut catalog = Catalog::default();
    let bcf = source.add("First.BCF", &binary(&[b"first"], true));
    catalog.load_catalog(&source.index, bcf, &location(b"First.BCF"))?;
    let id = catalog.get(Kind::Appearance, b"first").unwrap().asset;
    let ndx = source.register(&mut catalog, &[b"first", b"second"])?;
    let first = catalog.get(Kind::Appearance, b"first").unwrap();
    assert_eq!(first.asset, id);
    assert_eq!(first.location.context, 99);
    assert_eq!(first.location.catalog, b"first.bcf");
    assert_eq!(catalog.index_source(Kind::Appearance, b"first"), None);
    assert_eq!(catalog.index_source(Kind::Appearance, b"second"), Some(ndx));
    assert!(!catalog.get(Kind::Appearance, b"second").unwrap().attempted);
    Ok(())
}

#[test]
fn startup_visits_text_before_binary_and_continues_after_partial_reads() -> Result<()> {
    let mut source = Source::new();
    let text = source.add(
        "Winner.CMX",
        b"// fixture\nversion 300\n0\n1\nshared\n7\n0\n0\n",
    );
    let binary = source.add("Later.BCF", &binary(&[b"shared", b"later"], true));
    // Supply the binary in the earlier search directory; native alias order
    // still visits every CMX path before the BCF paths.
    source
        .dirs
        .insert_first(b"./Earlier/", b"Later.BCF", binary)?;
    let mut catalog = Catalog::default();
    let mut flags = 0x12345678;
    assert_eq!(
        catalog.load_startup_paths(
            &source.index,
            &mut source.dirs,
            &[b"./Earlier/".to_vec(), b"./Supplied/".to_vec()],
            false,
            77,
            &mut flags
        )?,
        2
    );
    assert_eq!(flags, 0x12345678);
    assert_eq!(catalog.read_failures, [text]);
    assert_eq!(
        catalog.asset(Kind::Appearance, b"shared").unwrap().source,
        text
    );
    assert_eq!(
        catalog.asset(Kind::Appearance, b"later").unwrap().source,
        binary
    );
    assert_eq!(
        catalog.asset(Kind::Appearance, b"later").unwrap().context,
        77
    );
    assert_eq!(
        catalog
            .get(Kind::Appearance, b"later")
            .unwrap()
            .location
            .directory,
        b"./Earlier/"
    );
    assert_eq!(
        catalog
            .get(Kind::Appearance, b"shared")
            .unwrap()
            .location
            .catalog,
        b"winner.cmx"
    );
    Ok(())
}

#[test]
fn startup_index_short_read_keeps_records_and_visits_later_and_missing_files() -> Result<()> {
    let mut source = Source::new();
    let mut bytes = 0i32.to_le_bytes().to_vec();
    bytes.extend(1i32.to_le_bytes());
    string(&mut bytes, b"kept");
    string(&mut bytes, b"Catalog.CMX");
    let partial = source.add("Partial.ndx", &bytes);
    let missing = source.add("Missing.ndx", &[]);
    std::fs::remove_file(&source.index.resources[missing].source)?;
    bytes = 0i32.to_le_bytes().to_vec();
    bytes.extend(1i32.to_le_bytes());
    string(&mut bytes, b"later");
    string(&mut bytes, b"Later.CMX");
    bytes.extend(0i32.to_le_bytes());
    let later = source.add("Later.ndx", &bytes);
    let mut directories = Directories::default();
    for (path, name, id) in [
        (b"./One/".as_slice(), b"Partial.ndx".as_slice(), partial),
        (b"./Two/", b"Missing.ndx", missing),
        (b"./Three/", b"Later.ndx", later),
    ] {
        directories.insert_first(path, name, id)?;
    }
    directories.supply_no_match(b"././Two//_*_missing.NDX")?;
    let mut catalog = Catalog::default();
    assert_eq!(
        catalog.load_startup_paths(
            &source.index,
            &mut directories,
            &[b"./One/".to_vec(), b"./Two/".to_vec(), b"./Three/".to_vec()],
            true,
            99,
            &mut 0
        )?,
        3
    );
    assert_eq!(catalog.index_read_failures, [partial]);
    assert_eq!(catalog.startup_open_failures.len(), 1);
    assert_eq!(catalog.startup_open_failures[0].source, Some(missing));
    assert_eq!(
        catalog.index_source(Kind::Appearance, b"kept"),
        Some(partial)
    );
    assert_eq!(
        catalog.index_source(Kind::Appearance, b"later"),
        Some(later)
    );
    assert_eq!(
        catalog
            .get(Kind::Appearance, b"kept")
            .unwrap()
            .location
            .context,
        99
    );
    assert!(!catalog.get(Kind::Appearance, b"kept").unwrap().attempted);
    Ok(())
}

#[test]
fn startup_ndx_unsafe_string_is_a_host_fault_and_stops_later_sources() -> Result<()> {
    let mut source = Source::new();
    let mut bytes = 1i32.to_le_bytes().to_vec();
    bytes.push(255);
    bytes.extend(1024u32.to_le_bytes());
    let oversized = source.add("Oversized.ndx", &bytes);
    let later = source.add("Later.ndx", &[0; 12]);
    let mut directories = Directories::default();
    directories.insert_first(b"./One/", b"Oversized.ndx", oversized)?;
    directories.insert_first(b"./Two/", b"Later.ndx", later)?;
    let mut catalog = Catalog::default();
    let paths = [b"./One/".to_vec(), b"./Two/".to_vec()];
    let error = catalog
        .load_startup_paths(&source.index, &mut directories, &paths, true, 99, &mut 0)
        .unwrap_err();
    assert!(error.to_string().contains("native buffer capacity"));
    assert!(catalog.index_read_failures.is_empty());
    assert!(
        catalog
            .load_startup_paths(&source.index, &mut directories, &paths, true, 99, &mut 0)
            .is_err()
    );
    Ok(())
}

#[test]
fn startup_numbered_entry_tries_plain_name_before_its_enumerated_source() -> Result<()> {
    for plain_exists in [false, true] {
        let mut source = Source::new();
        let numbered = source.add("_17_Clothes.BCF", &binary(&[b"numbered"], true));
        let plain = if plain_exists {
            Some(source.add("Clothes.BCF", &binary(&[b"plain"], true)))
        } else {
            None
        };
        let mut catalog = Catalog::default();
        assert_eq!(
            catalog.load_startup_paths(
                &source.index,
                &mut source.dirs,
                &[b"./Supplied/".to_vec()],
                false,
                77,
                &mut 0
            )?,
            1 + usize::from(plain_exists)
        );
        if let Some(plain) = plain {
            assert_eq!(
                catalog.asset(Kind::Appearance, b"plain").unwrap().source,
                plain
            );
            assert!(catalog.get(Kind::Appearance, b"numbered").is_none());
        } else {
            assert_eq!(
                catalog.asset(Kind::Appearance, b"numbered").unwrap().source,
                numbered
            );
        }
    }
    Ok(())
}

#[test]
fn startup_does_not_turn_unknown_os_wildcard_results_into_missing_files() -> Result<()> {
    let mut source = Source::new();
    let missing = source.add("Missing.BCF", &[]);
    std::fs::remove_file(&source.index.resources[missing].source)?;
    let mut catalog = Catalog::default();
    let error = catalog
        .load_startup_paths(
            &source.index,
            &mut source.dirs,
            &[b"./Supplied/".to_vec()],
            false,
            77,
            &mut 0,
        )
        .unwrap_err();
    assert!(error.to_string().contains("needs an OS search result"));
    assert!(catalog.startup_open_failures.is_empty());
    Ok(())
}
