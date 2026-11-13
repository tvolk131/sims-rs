//! Original loaded-record constructors, replacement and lazy attempts. The
//! attempted flag can be true with no asset: failed attempts are cached too.
use super::{catalog_index::Lazy, generator::fixed, name_map::Names};
use sims_formats::avatar::index::Kind;
use std::collections::BTreeMap;

pub type AssetId = u64;

pub trait Assets {
    fn name(&self, asset: AssetId) -> Result<Vec<u8>, String>;
    fn rename(&mut self, asset: AssetId, name: Vec<u8>) -> Result<(), String>;
    fn dispose(&mut self, asset: AssetId) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub location: Lazy,
    pub attempted: bool,
    pub asset: Option<AssetId>,
}
impl Record {
    pub fn lazy(location: Lazy) -> Self {
        Self {
            location,
            attempted: false,
            asset: None,
        }
    }
    pub fn loaded(mut location: Lazy, asset: AssetId, host: &impl Assets) -> Result<Self, String> {
        location.name = host.name(asset)?;
        Ok(Self {
            location,
            attempted: true,
            asset: Some(asset),
        })
    }
    /// 5a2207/5a23a8/5a2549 retain the original record's path/context and
    /// rename the incoming asset to that record's key. Only appearance names
    /// pass through 5954be's C-string lowercase/reassignment operation.
    pub fn replace(
        &mut self,
        kind: Kind,
        asset: AssetId,
        host: &mut impl Assets,
    ) -> Result<(), String> {
        if let Some(previous) = self.asset {
            host.dispose(previous)?;
            self.asset = None;
        }
        self.asset = Some(asset);
        let name = if kind == Kind::Appearance {
            lower(&self.location.name)
        } else {
            self.location.name.clone()
        };
        host.rename(asset, name)?;
        self.attempted = true;
        Ok(())
    }
}

pub fn lower(value: &[u8]) -> Vec<u8> {
    fixed(value, usize::MAX)
        .iter()
        .map(u8::to_ascii_lowercase)
        .collect()
}

pub trait Loader {
    type Handle;
    fn open(&mut self, path: &[u8], registry: &Registry) -> Result<Option<Self::Handle>, String>;
    /// Return the original load status, including ordinary nonzero failure.
    /// Host/infrastructure errors are separate from native failure statuses.
    fn load(&mut self, location: &Lazy, registry: &mut Registry) -> Result<u32, String>;
    fn close(&mut self, handle: Self::Handle, registry: &Registry) -> Result<(), String>;
}

/// Decoding boundary of ordinary 59f087/59e746 (all mode flags zero).
/// Native read failures return None, distinct from Rust support/host errors.
/// The decoder owns partially decoded allocations. Text-controller failure
/// disposes that allocation after reporting the error; binary does not. Those
/// allocator effects are separate from the shared retained-record semantics.
pub trait CatalogReader: Assets {
    fn count(&mut self, kind: Kind) -> Result<Option<i32>, String>;
    fn read(&mut self, kind: Kind, context: u32) -> Result<Option<AssetId>, String>;
    fn report_error(&mut self, catalog: &[u8]) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registered {
    Inserted,
    Replaced,
    Discarded,
}

#[derive(Debug, Default, Clone)]
pub struct Registry {
    names: [Names; 3],
    records: [BTreeMap<Vec<u8>, Record>; 3],
    /// 6f76bc, set around catalog loading, then cleared (not restored).
    pub loading_lazy: bool,
}
impl Registry {
    pub fn get(&self, kind: Kind, name: &[u8]) -> Option<&Record> {
        self.records[kind.index()].get(name)
    }
    pub fn get_mut(&mut self, kind: Kind, name: &[u8]) -> Option<&mut Record> {
        self.records[kind.index()].get_mut(name)
    }
    pub fn iter(&self, kind: Kind) -> impl Iterator<Item = &Record> {
        self.names[kind.index()]
            .iter()
            .map(move |name| &self.records[kind.index()][name])
    }
    pub fn insert(&mut self, kind: Kind, record: Record) -> bool {
        let k = kind.index();
        if self.names[k].contains(&record.location.name) {
            return false;
        }
        self.names[k].insert(record.location.name.clone());
        self.records[k].insert(record.location.name.clone(), record);
        true
    }
    /// Ordinary decoded-record registration shared by the binary and text
    /// controllers. A negative cached attempt is already owned, even if null.
    pub fn register_loaded(
        &mut self,
        kind: Kind,
        mut location: Lazy,
        asset: AssetId,
        host: &mut impl Assets,
    ) -> Result<Registered, String> {
        let name = host.name(asset)?;
        if let Some(record) = self.get_mut(kind, &name) {
            if record.attempted {
                host.dispose(asset)?;
                Ok(Registered::Discarded)
            } else {
                record.replace(kind, asset, host)?;
                Ok(Registered::Replaced)
            }
        } else {
            if kind == Kind::Appearance {
                location.catalog = lower(&location.catalog);
            }
            let record = Record::loaded(location, asset, host)?;
            self.insert(kind, record);
            Ok(Registered::Inserted)
        }
    }
    /// 59f087/59e746 with ordinary records. Earlier groups/records remain
    /// registered if a subsequent count or record fails. Failure is 0xffffffff.
    pub fn read_catalog(
        &mut self,
        location: &Lazy,
        host: &mut impl CatalogReader,
    ) -> Result<u32, String> {
        for kind in [Kind::Skeleton, Kind::Appearance, Kind::Animation] {
            let Some(count) = host.count(kind)? else {
                host.report_error(fixed(&location.catalog, usize::MAX))?;
                return Ok(u32::MAX);
            };
            for _ in 0..count {
                let Some(asset) = host.read(kind, location.context)? else {
                    host.report_error(fixed(&location.catalog, usize::MAX))?;
                    return Ok(u32::MAX);
                };
                self.register_loaded(kind, location.clone(), asset, host)?;
            }
        }
        Ok(0)
    }
    /// The unconditional 5a3810/5a38da/5a39d2 attempt. Native lookup callers
    /// separately guard the attempted flag; callers explicitly forcing an
    /// attempt can use this even when it is already set.
    pub fn attempt(
        &mut self,
        kind: Kind,
        name: &[u8],
        host: &mut impl Loader,
    ) -> Result<(), String> {
        let location = self
            .get(kind, name)
            .ok_or("native lazy record absent")?
            .location
            .clone();
        let path = [location.directory.as_slice(), location.catalog.as_slice()].concat();
        if let Some(handle) = host.open(fixed(&path, usize::MAX), self)? {
            self.loading_lazy = true;
            // The original ignores this status, retaining any partial record
            // changes made by the catalog before it reported failure.
            let _ = host.load(&location, self)?;
            self.loading_lazy = false;
            host.close(handle, self)?;
        }
        self.get_mut(kind, name)
            .ok_or("native lazy record removed during load")?
            .attempted = true;
        Ok(())
    }
}
