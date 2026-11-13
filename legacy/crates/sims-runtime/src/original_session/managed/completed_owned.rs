//! Schema2: historical import receipts and current owned resource versions are
//! separate. No mutable starting-path fallback or catalog bootstrap is allowed.
use super::{Catalog, Content};
use crate::original_session::{Saved, Session, completed_baseline};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_content::{
    Index,
    owned_resources::{Entry, Neighborhood},
    saves::House,
};
use sims_vm::Program;
use std::collections::{BTreeMap, BTreeSet};
pub(in crate::original_session) mod audit;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Metadata {
    pub house: House,
    pub modules: BTreeMap<String, String>,
    pub immutable: BTreeMap<String, String>,
}
#[derive(Serialize)]
struct BorrowedMetadata<'a> {
    house: &'a House,
    modules: &'a BTreeMap<String, String>,
    immutable: &'a BTreeMap<String, String>,
}
pub(super) fn decode(bytes: &[u8]) -> Result<Metadata> {
    let (metadata, rest) = postcard::take_from_bytes(bytes)
        .map_err(|e| anyhow::anyhow!("decode completed managed metadata: {e}"))?;
    ensure!(rest.is_empty(), "trailing completed managed metadata");
    Ok(metadata)
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn owned(index: &Index, source: &str) -> Result<String> {
    let r = &index.resources[index.find(source)?];
    let bytes = r
        .owned
        .as_ref()
        .with_context(|| format!("completed managed mutable source is unowned: {source}"))?;
    ensure!(
        r.size == bytes.len() as u64 && r.unpacked_size == r.size && !r.archived,
        "completed managed owned resource extent differs: {source}"
    );
    Ok(digest(bytes))
}
fn capture(index: &Index, source: &str) -> Result<Neighborhood> {
    let (directory, file) = source
        .rsplit_once('/')
        .context("completed managed neighborhood identity absent")?;
    ensure!(
        file.eq_ignore_ascii_case("Neighborhood.iff"),
        "completed managed neighborhood identity differs"
    );
    let prefix = format!("{directory}/").to_ascii_lowercase();
    let mut entries = vec![];
    for r in &index.resources {
        if !r.locator.to_ascii_lowercase().starts_with(&prefix) {
            continue;
        }
        // Deliberately no Resource::read before proving ownership. Schema2
        // keeps all supplied entries, including extensions other than iff/fam.
        let hash = owned(index, &r.locator)?;
        entries.push(Entry {
            locator: r.locator.clone(),
            name: r.name.clone(),
            sha256: hash,
            bytes: r.owned.as_ref().unwrap().to_vec(),
        });
    }
    let result = Neighborhood {
        directory: directory.into(),
        entries,
    };
    result.validate()?;
    Ok(result)
}
pub(super) fn mount(index: &Index, namespace: &Neighborhood) -> Result<Index> {
    // Remove every starting neighborhood before any read, even when the GUI
    // passes a full installation Index. A missing embedded entry stays absent.
    let mut immutable = index.clone();
    immutable
        .resources
        .retain(|r| !completed_baseline::mutable_locator(&r.locator));
    completed_baseline::immutable_index(&immutable)?;
    namespace.mount(&immutable)
}
fn modules(program: &Program) -> BTreeMap<String, String> {
    program
        .modules
        .iter()
        .map(|(s, m)| (s.clone(), m.content_hash.clone()))
        .collect()
}
fn insert(hashes: &mut BTreeMap<String, String>, source: &str, hash: &str) -> Result<()> {
    ensure!(
        hash.len() == 64
            && hash
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "completed managed source version/hash unknown: {source}"
    );
    if let Some(old) = hashes.insert(source.into(), hash.into()) {
        ensure!(
            old == hash,
            "completed managed source versions conflict: {source}"
        );
    }
    Ok(())
}
fn required(
    house: &House,
    catalog: &Catalog,
    modules: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    // Historical mutable hashes are provenance only. Historical immutable
    // assets remain pinned, alongside the exact current Program/catalog.
    for (s, h) in house
        .sources
        .iter()
        .filter(|(s, _)| !completed_baseline::mutable_locator(s))
        .chain(modules.iter())
        .chain(catalog.accessories.values().flat_map(|a| a.iter()))
        .chain(
            catalog
                .resource_storage
                .iter()
                .flatten()
                .map(|d| (&d.source, &d.content_hash)),
        )
    {
        insert(&mut result, s, h)?;
    }
    Ok(result)
}
fn immutable(required: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    required
        .iter()
        .filter(|(s, _)| !completed_baseline::mutable_locator(s))
        .map(|(s, h)| (s.clone(), h.clone()))
        .collect()
}
pub(super) fn validate(
    index: &Index,
    house: &House,
    catalog: &Catalog,
    module_hashes: &BTreeMap<String, String>,
    immutable_hashes: &BTreeMap<String, String>,
    declared_modules: &BTreeSet<String>,
) -> Result<()> {
    ensure!(
        module_hashes.keys().cloned().collect::<BTreeSet<_>>() == *declared_modules,
        "completed managed module version manifest differs"
    );
    owned(index, &house.source)?;
    owned(index, &house.neighborhood_source)?;
    let required = required(house, catalog, module_hashes)?;
    ensure!(
        immutable(&required) == *immutable_hashes,
        "completed managed immutable manifest differs"
    );
    validate_hashes(index, &required)
}
fn validate_hashes(index: &Index, required: &BTreeMap<String, String>) -> Result<()> {
    for (source, hash) in required {
        let current = if completed_baseline::mutable_locator(&source) {
            owned(index, &source)?
        } else {
            digest(&index.resources[index.find(&source)?].read()?)
        };
        ensure!(
            current == *hash,
            "completed managed current source version differs: {source}"
        );
    }
    Ok(())
}
pub(super) fn validate_modules(
    program: &Program,
    expected: &BTreeMap<String, String>,
) -> Result<()> {
    ensure!(
        modules(program) == *expected,
        "completed managed resident module/current source version mapping unsupported"
    );
    Ok(())
}
pub(super) fn save(session: &Session) -> Result<Saved> {
    let owned = capture(&session.index, &session.house.neighborhood_source)?;
    let catalog = Catalog::capture(session);
    let modules = modules(&session.program);
    let immutable = immutable(&required(&session.house, &catalog, &modules)?);
    validate(
        &session.index,
        &session.house,
        &catalog,
        &modules,
        &immutable,
        &session.program.modules.keys().cloned().collect(),
    )?;
    completed_baseline::resources(
        &session.index,
        &session.house.source,
        &session.house.neighborhood_source,
        &session.program,
    )?;
    let house = postcard::to_allocvec(&BorrowedMetadata {
        house: &session.house,
        modules: &modules,
        immutable: &immutable,
    })?;
    let mut result = session.save();
    result.version = 2;
    result.managed = Some(Box::new(Content {
        schema: 2,
        house,
        owned,
        catalog,
    }));
    Ok(result)
}

#[cfg(test)]
mod tests;
