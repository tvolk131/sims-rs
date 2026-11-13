//! Resource-name discovery for the linked household profile. Catalog names are
//! not clip names. Conflicting providers require an explicit selection; sorted
//! archive order is never treated as original expansion precedence.
use crate::Index;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use sims_formats::animation::{Animation, read_bcf, read_cfp};
use sims_vm::{Program, program::AnimationAsset};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

mod animation_catalogs;
pub use animation_catalogs::AnimationCatalogs;

#[derive(Debug, Default, serde::Serialize)]
pub struct Report {
    pub catalogs: usize,
    pub discovered_clips: usize,
    pub loaded_clips: usize,
    pub catalog_errors: Vec<String>,
    /// Includes unused/placeholder entries; execution still fails explicitly if
    /// a script requests one. This is not a claim of whole-game asset coverage.
    pub unresolved: Vec<String>,
}

pub fn load_linked_animations(index: &Index, program: &mut Program) -> Result<Report> {
    let mut names = BTreeSet::new();
    for module in program.modules.values() {
        let tables: BTreeSet<_> = module
            .object_animation_tables
            .values()
            .copied()
            .chain([129, 130, 131, 132])
            .chain((module.source == program.globals).then_some(150))
            .chain((module.source == program.globals).then_some(154))
            .chain((module.source == program.globals).then_some(156))
            .collect();
        for table in tables {
            if let Some(strings) = module.strings.get(&table) {
                names.extend(
                    strings
                        .iter()
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_ascii_lowercase()),
                );
            }
        }
    }
    load_named_animations(index, program, &names)
}

/// Resolve requested embedded clip names through BCF catalogs, with the same
/// provenance and conflicting-provider checks as linked script animation tables.
pub fn load_named_animations(
    index: &Index,
    program: &mut Program,
    names: &BTreeSet<String>,
) -> Result<Report> {
    load_named_animations_inner(index, program, names, false)
}

/// Explicit native runtime dependencies, including empty motion records. This
/// does not broaden the legacy linked-animation preload set in old snapshots.
pub fn load_named_native_animations(
    index: &Index,
    program: &mut Program,
    names: &BTreeSet<String>,
) -> Result<Report> {
    load_named_animations_inner(index, program, names, true)
}

fn load_named_animations_inner(
    index: &Index,
    program: &mut Program,
    names: &BTreeSet<String>,
    retain_empty: bool,
) -> Result<Report> {
    let mut scan = crate::resource_scan::Scan::default();
    load_named_animations_read(index, program, names, retain_empty, |r| scan.read(r))
}

pub(crate) fn load_named_animations_read(
    index: &Index,
    program: &mut Program,
    names: &BTreeSet<String>,
    retain_empty: bool,
    read: impl FnMut(&crate::Resource) -> Result<Vec<u8>>,
) -> Result<Report> {
    load_named_animations_using(
        index,
        program,
        names,
        retain_empty,
        read,
        |_, bytes, native| animation_catalogs::parse(&bytes, native),
    )
}

fn load_named_animations_using(
    index: &Index,
    program: &mut Program,
    names: &BTreeSet<String>,
    retain_empty: bool,
    mut read: impl FnMut(&crate::Resource) -> Result<Vec<u8>>,
    mut parse: impl FnMut(&crate::Resource, Vec<u8>, bool) -> Result<Arc<[Animation]>>,
) -> Result<Report> {
    let mut discovered = BTreeSet::new();
    let mut candidates: BTreeMap<String, Vec<(usize, Animation)>> = BTreeMap::new();
    let mut report = Report::default();
    for (id, resource) in index.resources.iter().enumerate() {
        if !resource.name.to_ascii_lowercase().ends_with(".cmx.bcf") {
            continue;
        }
        let bytes = read(resource)?;
        // The separate avatar parser owns skeleton/appearance catalogs.
        if bytes.get(..8) != Some(&[0; 8]) {
            continue;
        }
        let clips = match parse(resource, bytes, retain_empty) {
            Ok(clips) => clips,
            Err(e) => {
                report
                    .catalog_errors
                    .push(format!("{}: {e}", resource.locator));
                continue;
            }
        };
        for clip in clips.iter() {
            let normalized = clip.name.to_ascii_lowercase();
            if names.contains(&normalized) {
                candidates
                    .entry(normalized.clone())
                    .or_default()
                    .push((id, clip.clone()));
            }
            discovered.insert(normalized);
        }
        report.catalogs += 1;
    }
    report.discovered_clips = discovered.len();
    for name in names {
        // Explicit locomotion/idle selections remain authoritative until the
        // native content manager's search order has been recovered.
        if program.animations.contains_key(name) {
            continue;
        }
        let Some(providers) = candidates.get(name) else {
            report.unresolved.push(name.clone());
            continue;
        };
        let mut decoded: Option<AnimationAsset> = None;
        for (id, clip) in providers {
            let resource = &index.resources[*id];
            let cfp_name = format!("{}.cfp", clip.skill);
            let cfp_id = sibling(index, *id, &cfp_name)?;
            let cfp = &index.resources[cfp_id];
            let data = cfp.read()?;
            let asset = AnimationAsset {
                source: resource.locator.clone(),
                content_hash: format!("{:x}", Sha256::digest(resource.read()?)),
                samples_source: cfp.locator.clone(),
                samples_hash: format!("{:x}", Sha256::digest(&data)),
                samples: read_cfp(&data, clip.translation_count, clip.rotation_count)
                    .with_context(|| cfp.locator.clone())?,
                clip: clip.clone(),
            };
            if let Some(previous) = &decoded {
                if previous.clip != asset.clip || previous.samples != asset.samples {
                    bail!(
                        "conflicting animation {name}: {} and {}; select an explicit provider",
                        previous.source,
                        asset.source
                    );
                }
            } else {
                decoded = Some(asset);
            }
        }
        program.animations.insert(name.clone(), decoded.unwrap());
        report.loaded_clips += 1;
    }
    Ok(report)
}

/// Prefer the catalog's own archive/directory when resolving its embedded link.
/// This is dependency locality, not precedence between competing catalogs.
pub fn sibling(index: &Index, catalog: usize, name: &str) -> Result<usize> {
    let locator = &index.resources[catalog].locator;
    let local = if let Some((archive, entry)) = locator.split_once("::") {
        let directory = entry.rsplit_once(['\\', '/']).map_or("", |(d, _)| d);
        if directory.is_empty() {
            format!("{archive}::{name}")
        } else {
            format!("{archive}::{directory}\\{name}")
        }
    } else {
        locator
            .rsplit_once('/')
            .map_or_else(|| name.to_owned(), |(d, _)| format!("{d}/{name}"))
    };
    if index
        .resources
        .iter()
        .any(|r| r.locator.eq_ignore_ascii_case(&local))
    {
        index.find(&local)
    } else {
        index.find(name)
    }
}

/// Resolve explicit runtime factory dependencies by GUID, rejecting ambiguous
/// providers. Asset names are discovered from OBJD rather than guessed filenames.
pub fn load_object_guids(index: &Index, program: &mut Program, guids: &[u32]) -> Result<()> {
    load_object_guids_inner(index, program, guids, true).map(|_| ())
}
/// Native optional catalog lookup. An absent provider is a null resource;
/// malformed or ambiguous providers still fail explicitly.
pub fn load_object_guid_if_present(
    index: &Index,
    program: &mut Program,
    guid: u32,
) -> Result<bool> {
    Ok(load_object_guids_inner(index, program, &[guid], false)?.contains(&guid))
}
fn load_object_guids_inner(
    index: &Index,
    program: &mut Program,
    guids: &[u32],
    required: bool,
) -> Result<BTreeSet<u32>> {
    let wanted: BTreeSet<_> = guids.iter().copied().collect();
    let mut providers: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    let mut scan = crate::resource_scan::Scan::default();
    for resource in &index.resources {
        if !resource.name.to_ascii_lowercase().ends_with(".iff") {
            continue;
        }
        let bytes = scan.read(resource)?;
        for guid in crate::resource_scan::object_guids(&bytes)? {
            if wanted.contains(&guid) {
                providers
                    .entry(guid)
                    .or_default()
                    .insert(resource.locator.clone());
            }
        }
    }
    let mut loaded = BTreeSet::new();
    for guid in wanted {
        if !required && !providers.contains_key(&guid) {
            continue;
        }
        let sources = providers
            .get(&guid)
            .with_context(|| format!("no source for factory GUID {guid:08X}"))?;
        anyhow::ensure!(
            sources.len() == 1,
            "ambiguous factory GUID {guid:08X}: {sources:?}"
        );
        for (name, module) in
            crate::vm::load_program(index, sources.first().unwrap(), &program.globals)?.modules
        {
            program.modules.entry(name).or_insert(module);
        }
        loaded.insert(guid);
    }
    Ok(loaded)
}

#[cfg(test)]
#[path = "assets/guid_scan_tests.rs"]
mod guid_scan_tests;

#[cfg(test)]
#[path = "assets/named_animation_tests.rs"]
mod named_animation_tests;
