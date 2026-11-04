//! Source-backed native NDX catalogs. The caller supplies mounted directory
//! entries and search paths in their actual order; scanning the extracted tree
//! alphabetically is not a substitute for the native mount/provider policy.
use crate::Index;
use anyhow::{Context, Result, ensure};
use sims_formats::avatar::catalog::{Asset, Binary};
use sims_formats::avatar::index::{IndexReader, Kind};
use sims_sim::pose::clothing::{
    catalog_files, catalog_index, catalog_mount,
    catalog_record::{AssetId, Assets, CatalogReader, Loader, Record, Registry},
    name_map::Files,
};
use std::collections::BTreeMap;
mod startup;

/// Explicit directory file maps. Non-resource entries still occupy hash nodes.
/// `from_mounted` preserves the entire map, including its collision chains;
/// `insert_first` is for callers supplying individual insertion operations.
#[derive(Debug, Default)]
pub struct Directories {
    maps: BTreeMap<Vec<u8>, Files<Option<usize>>>,
    // Filesystem wildcard searches are distinct from mounted hash-map iteration.
    // A supplied no-match result is enough for ordinary failed startup opens.
    no_matches: std::collections::BTreeSet<Vec<u8>>,
}
impl Directories {
    pub fn from_mounted(
        manager: &catalog_mount::Manager,
        mut source: impl FnMut(&[u8], &[u8], &catalog_mount::Metadata) -> Result<Option<usize>>,
    ) -> Result<Self> {
        let mut maps = BTreeMap::new();
        for (directory, files) in manager.directories() {
            maps.insert(
                directory.to_vec(),
                files.try_map(|name, metadata| source(directory, name, metadata))?,
            );
        }
        Ok(Self {
            maps,
            ..Default::default()
        })
    }
    pub fn supply_no_match(&mut self, pattern: &[u8]) -> Result<()> {
        self.no_matches
            .insert(Files::<()>::key(pattern).map_err(anyhow::Error::msg)?);
        Ok(())
    }
    pub fn insert_first(
        &mut self,
        directory: &[u8],
        filename: &[u8],
        source: usize,
    ) -> Result<bool> {
        let key = Files::<usize>::key(directory).map_err(anyhow::Error::msg)?;
        self.maps
            .entry(key)
            .or_default()
            .insert(filename, Some(source))
            .map_err(anyhow::Error::msg)
    }
    pub fn source(&self, directory: &[u8], filename: &[u8]) -> Result<Option<usize>> {
        let key = Files::<usize>::key(directory).map_err(anyhow::Error::msg)?;
        self.maps
            .get(&key)
            .map(|files| files.get(filename).map(|v| v.copied().flatten()))
            .transpose()
            .map(|v| v.flatten())
            .map_err(anyhow::Error::msg)
    }
    /// First readable resource in the original concrete-type plain-file pass.
    /// This does not supply type aliases or the later numbered/wildcard pass.
    pub fn open_plain_paths(
        &self,
        index: &Index,
        paths: &[Vec<u8>],
        name: &[u8],
        kind: u32,
    ) -> Result<Option<usize>> {
        startup::open_plain_paths(index, self, paths, name, kind)
    }
    fn path_source(&self, path: &[u8]) -> Result<Option<usize>> {
        let split = path
            .iter()
            .rposition(|b| matches!(b, b'/' | b'\\'))
            .map_or(0, |n| n + 1);
        self.source(&path[..split], &path[split..])
    }
}
impl catalog_files::Host for Directories {
    type Source = Option<usize>;
    fn files(&mut self, directory: &[u8]) -> Result<Option<&Files<Option<usize>>>, String> {
        Ok(self.maps.get(&Files::<usize>::key(directory)?))
    }
}

#[derive(Debug, Default)]
pub struct Catalog {
    registry: Registry,
    index_sources: [BTreeMap<Vec<u8>, usize>; 3],
    assets: Pool,
    // A Rust support/host error is not a native return status. Stop subsequent
    // loading until the cache is rebuilt after addressing that boundary, so a
    // partially filled record cannot hide the error on a second lookup.
    fault: Option<String>,
    /// Catalogs reporting an ordinary native read failure. Earlier successful
    /// records remain installed even if the requested record is among them.
    pub read_failures: Vec<usize>,
    /// Startup NDX files that ended during a native stream read. Earlier
    /// registrations remain, and the startup controller visits later sources.
    pub index_read_failures: Vec<usize>,
    pub startup_open_failures: Vec<catalog_files::Resource<Option<usize>>>,
}
impl Catalog {
    fn ready(&self) -> Result<()> {
        if let Some(error) = &self.fault {
            anyhow::bail!("catalog stopped at an unresolved host boundary: {error}");
        }
        Ok(())
    }
    fn completed<T>(&mut self, result: Result<T, String>) -> Result<T> {
        result.map_err(|error| {
            self.fault = Some(error.clone());
            anyhow::Error::msg(error)
        })
    }
    /// Register in original type/path/hash order. A failed file retains records
    /// already registered, and the returned error identifies its source. Neither
    /// registration nor enumeration opens a referenced catalog/mesh/animation.
    pub fn register_paths(
        &mut self,
        index: &Index,
        directories: &mut Directories,
        paths: &[Vec<u8>],
        context: u32,
    ) -> Result<usize> {
        self.ready()?;
        let sources = catalog_files::enumerate(
            paths,
            u32::from_be_bytes(*b"INDX"),
            Some(&[u32::from_be_bytes(*b"NDX\0")]),
            directories,
        )
        .map_err(anyhow::Error::msg)?;
        for source in &sources {
            let source_id = source
                .source
                .context("NDX entry is not a readable resource")?;
            let resource = index
                .resources
                .get(source_id)
                .context("NDX resource ID absent")?;
            ensure!(
                resource.name.to_ascii_lowercase().ends_with(".ndx"),
                "index map points to non-NDX resource {}",
                resource.locator
            );
            let data = resource.read()?;
            let mut reader = IndexReader::new(&data);
            catalog_index::register(
                &mut reader,
                &source.directory,
                context,
                &mut Registration {
                    indexed: self,
                    source: source_id,
                },
            )
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("register {}", resource.locator))?;
        }
        Ok(sources.len())
    }
    pub fn get(&self, kind: Kind, name: &[u8]) -> Option<&Record> {
        self.registry.get(kind, name)
    }
    pub fn iter(&self, kind: Kind) -> impl Iterator<Item = &Record> {
        self.registry.iter(kind)
    }
    pub fn index_source(&self, kind: Kind, name: &[u8]) -> Option<usize> {
        self.index_sources[kind.index()].get(name).copied()
    }
    pub fn asset(&self, kind: Kind, name: &[u8]) -> Option<&Loaded> {
        self.get(kind, name)?
            .asset
            .and_then(|id| self.assets.values.get(&id))
    }
    /// Resolve once. A missing catalog, or an ordinary failed read, consumes
    /// the attempt just as 5a3810/5a38da/5a39d2 do. Unsupported Rust decoding
    /// and host errors remain errors; they do not become cached native failure.
    pub fn resolve(
        &mut self,
        index: &Index,
        directories: &Directories,
        kind: Kind,
        name: &[u8],
    ) -> Result<Option<&Loaded>> {
        self.ready()?;
        if self.get(kind, name).is_some_and(|r| !r.attempted) {
            let result = self.registry.attempt(
                kind,
                name,
                &mut Loading {
                    index,
                    directories,
                    assets: &mut self.assets,
                    read_failures: &mut self.read_failures,
                },
            );
            self.completed(result)?;
        }
        Ok(self.asset(kind, name))
    }
    /// Load an explicitly selected ordinary CMX/BCF catalog, sharing the same
    /// registry as NDX references. Startup source ordering is supplied separately.
    pub fn load_catalog(
        &mut self,
        index: &Index,
        source: usize,
        location: &catalog_index::Lazy,
    ) -> Result<u32> {
        self.ready()?;
        let result = decode_catalog(
            index,
            source,
            location,
            &mut self.registry,
            &mut self.assets,
            &mut self.read_failures,
        );
        self.completed(result)
    }
    /// Locate the exact catalog recorded by NDX in its supplied directory.
    /// Native lazy loading may subsequently register other records from that
    /// catalog; callers must perform that separate load before marking it loaded.
    pub fn catalog_source(
        &self,
        directories: &Directories,
        kind: Kind,
        name: &[u8],
    ) -> Result<usize> {
        let reference = self
            .get(kind, name)
            .context("native indexed avatar name absent")?;
        directories
            .source(&reference.location.directory, &reference.location.catalog)?
            .with_context(|| {
                format!(
                    "indexed catalog {} absent",
                    String::from_utf8_lossy(&reference.location.catalog)
                )
            })
    }
}

#[derive(Debug)]
pub struct Loaded {
    /// Source of the decoded record, independent of the NDX record's retained
    /// directory/context and independently queryable via `index_source`.
    pub source: usize,
    pub context: u32,
    pub value: Asset,
}
#[derive(Debug, Default)]
struct Pool {
    next: AssetId,
    values: BTreeMap<AssetId, Loaded>,
}
impl Pool {
    fn insert(&mut self, value: Loaded) -> Result<AssetId, String> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or("avatar asset identity exhausted")?;
        self.values.insert(self.next, value);
        Ok(self.next)
    }
}
impl Assets for Pool {
    fn name(&self, asset: AssetId) -> Result<Vec<u8>, String> {
        let name = self
            .values
            .get(&asset)
            .ok_or("decoded avatar asset absent")?
            .value
            .name();
        let (bytes, _, invalid) = encoding_rs::WINDOWS_1252.encode(name);
        if invalid {
            return Err("avatar name is not Windows-1252".into());
        }
        Ok(bytes.into_owned())
    }
    fn rename(&mut self, asset: AssetId, name: Vec<u8>) -> Result<(), String> {
        self.values
            .get_mut(&asset)
            .ok_or("decoded avatar asset absent")?
            .value
            .set_name(encoding_rs::WINDOWS_1252.decode(&name).0.into_owned());
        Ok(())
    }
    fn dispose(&mut self, asset: AssetId) -> Result<(), String> {
        self.values
            .remove(&asset)
            .ok_or("disposed avatar asset absent")?;
        Ok(())
    }
}
enum Decoder<'a> {
    Binary(Binary<'a>),
    Text(sims_formats::avatar::Cmx<'a>),
}
impl Decoder<'_> {
    fn count(&mut self, kind: Kind) -> sims_formats::Result<Option<i32>> {
        match self {
            Self::Binary(r) => r.count(kind),
            Self::Text(r) => r.count(kind),
        }
    }
    fn read(&mut self, kind: Kind) -> sims_formats::Result<Asset> {
        match self {
            Self::Binary(r) => r.read(kind),
            Self::Text(r) => r.read(kind),
        }
    }
}
struct Decoding<'a> {
    reader: Decoder<'a>,
    source: usize,
    assets: &'a mut Pool,
    read_failures: &'a mut Vec<usize>,
}
impl Assets for Decoding<'_> {
    fn name(&self, asset: AssetId) -> Result<Vec<u8>, String> {
        self.assets.name(asset)
    }
    fn rename(&mut self, asset: AssetId, name: Vec<u8>) -> Result<(), String> {
        self.assets.rename(asset, name)
    }
    fn dispose(&mut self, asset: AssetId) -> Result<(), String> {
        self.assets.dispose(asset)
    }
}
impl CatalogReader for Decoding<'_> {
    fn count(&mut self, kind: Kind) -> Result<Option<i32>, String> {
        self.reader
            .count(kind)
            .map_err(|e| format!("catalog support boundary: {e}"))
    }
    fn read(&mut self, kind: Kind, context: u32) -> Result<Option<AssetId>, String> {
        let value = self
            .reader
            .read(kind)
            .map_err(|e| format!("catalog support boundary: {e}"))?;
        self.assets
            .insert(Loaded {
                source: self.source,
                context,
                value,
            })
            .map(Some)
    }
    fn report_error(&mut self, _: &[u8]) -> Result<(), String> {
        self.read_failures.push(self.source);
        Ok(())
    }
}
fn decode_catalog(
    index: &Index,
    source: usize,
    location: &catalog_index::Lazy,
    registry: &mut Registry,
    assets: &mut Pool,
    read_failures: &mut Vec<usize>,
) -> Result<u32, String> {
    let resource = index
        .resources
        .get(source)
        .ok_or("catalog resource ID absent")?;
    let bytes = resource.read().map_err(|e| format!("{e:#}"))?;
    decode_catalog_bytes(
        index,
        source,
        &bytes,
        location,
        registry,
        assets,
        read_failures,
    )
}
fn decode_catalog_bytes(
    index: &Index,
    source: usize,
    bytes: &[u8],
    location: &catalog_index::Lazy,
    registry: &mut Registry,
    assets: &mut Pool,
    read_failures: &mut Vec<usize>,
) -> Result<u32, String> {
    let resource = index
        .resources
        .get(source)
        .ok_or("catalog resource ID absent")?;
    let reader = if location.catalog.to_ascii_lowercase().ends_with(b".cmx") {
        Decoder::Text(
            sims_formats::avatar::Cmx::new(bytes)
                .map_err(|e| format!("{}: {e}", resource.locator))?,
        )
    } else {
        Decoder::Binary(Binary::new(bytes))
    };
    registry
        .read_catalog(
            location,
            &mut Decoding {
                reader,
                source,
                assets,
                read_failures,
            },
        )
        .map_err(|e| format!("{}: {e}", resource.locator))
}
struct Loading<'a> {
    index: &'a Index,
    directories: &'a Directories,
    assets: &'a mut Pool,
    read_failures: &'a mut Vec<usize>,
}
impl Loader for Loading<'_> {
    type Handle = std::fs::File;
    fn open(&mut self, path: &[u8], _: &Registry) -> Result<Option<Self::Handle>, String> {
        let Some(source) = self
            .directories
            .path_source(path)
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        let resource = self
            .index
            .resources
            .get(source)
            .ok_or("catalog resource ID absent")?;
        match std::fs::File::open(&resource.source) {
            Ok(file) => Ok(Some(file)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("preflight {}: {error}", resource.locator)),
        }
    }
    fn load(
        &mut self,
        location: &catalog_index::Lazy,
        registry: &mut Registry,
    ) -> Result<u32, String> {
        let path = [location.directory.as_slice(), location.catalog.as_slice()].concat();
        let path = path.split(|&b| b == 0).next().unwrap();
        let source = self
            .directories
            .path_source(path)
            .map_err(|e| e.to_string())?
            .ok_or("opened catalog disappeared")?;
        decode_catalog(
            self.index,
            source,
            location,
            registry,
            self.assets,
            self.read_failures,
        )
    }
    fn close(&mut self, handle: Self::Handle, _: &Registry) -> Result<(), String> {
        drop(handle);
        Ok(())
    }
}
struct Registration<'a> {
    indexed: &'a mut Catalog,
    source: usize,
}
impl catalog_index::Host for Registration<'_> {
    fn contains(&mut self, kind: Kind, name: &[u8]) -> Result<bool, String> {
        Ok(self.indexed.registry.get(kind, name).is_some())
    }
    fn insert(&mut self, kind: Kind, lazy: catalog_index::Lazy) -> Result<(), String> {
        self.indexed.index_sources[kind.index()].insert(lazy.name.clone(), self.source);
        if !self.indexed.registry.insert(kind, Record::lazy(lazy)) {
            return Err("duplicate NDX registration without prior lookup".into());
        }
        Ok(())
    }
}
