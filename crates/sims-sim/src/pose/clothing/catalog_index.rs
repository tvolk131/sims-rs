//! 59fc48's lazy index registration. Ordered source files, mounted directories
//! and search-path ownership are supplied by the content host.
use sims_formats::avatar::index::{IndexReader, Kind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lazy {
    pub name: Vec<u8>,
    pub directory: Vec<u8>,
    pub catalog: Vec<u8>,
    /// Stable host identity for the search-path set retained by the record.
    pub context: u32,
}
pub trait Host {
    fn contains(&mut self, kind: Kind, name: &[u8]) -> Result<bool, String>;
    /// Insert an unloaded record. The original sets its loaded flag false and
    /// resolved asset pointer null; registration must not resolve its catalog.
    fn insert(&mut self, kind: Kind, record: Lazy) -> Result<(), String>;
}

pub fn register(
    input: &mut IndexReader<'_>,
    directory: &[u8],
    context: u32,
    host: &mut impl Host,
) -> Result<(), String> {
    for entry in input {
        let entry = entry.map_err(|e| e.to_string())?;
        if !host.contains(entry.kind, &entry.name)? {
            let mut catalog = entry.catalog;
            catalog.extend_from_slice(b".bcf");
            host.insert(
                entry.kind,
                Lazy {
                    name: entry.name,
                    directory: directory.to_vec(),
                    catalog,
                    context,
                },
            )?;
        }
    }
    Ok(())
}
