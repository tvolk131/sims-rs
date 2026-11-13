//! Historical extraction scans used only by isolated compatibility tests.
#![allow(dead_code)] // Each consumer selects either the controller or storage scan.
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use sims_content::{Index, catalog::Controller, saves::DefinitionSource};
use sims_formats::{far, iff::Iff};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

fn phase(source: &str) -> Option<u8> {
    let source = source.to_ascii_lowercase();
    let root = source.split('/').next()?;
    Some(match root {
        "gamedata_all_recursive" | "gamedata" if source.contains("/objects/") => 0,
        "gamedata_all_recursive" | "gamedata" if source.contains("/userobjects/") => 1,
        "downloads" => 3,
        "expansionpack" | "expansionpack1" => 4,
        "expansionpack2" => 5,
        "expansionpack3" => 6,
        "expansionpack4" => 7,
        "deluxe" => 8,
        "expansionpack5" => 9,
        "expansionpack6" => 10,
        "expansionshared" => 11,
        "templatenpcs" => 12,
        "expansionpack7" => 13,
        _ => return None,
    })
}

fn registered_sources(index: &Index) -> Result<Vec<usize>> {
    let mut manifests = BTreeMap::new();
    let mut sources = Vec::new();
    for (id, r) in index.resources.iter().enumerate() {
        let Some(phase) = phase(&r.locator) else {
            continue;
        };
        if !r.is_iff() {
            continue;
        }
        let ordinal = if r.archived {
            if !manifests.contains_key(&r.source) {
                manifests.insert(
                    r.source.clone(),
                    read_archive(&r.source, r.source.metadata()?.len())?,
                );
            }
            manifests[&r.source]
                .iter()
                .position(|e| e.name == r.name && u64::from(e.offset) == r.offset)
                .context("indexed resource missing from FAR manifest")?
        } else {
            0
        };
        sources.push((phase, r.source.clone(), ordinal, id));
    }
    sources.sort();
    Ok(sources.into_iter().map(|(_, _, _, id)| id).collect())
}

/// Legacy extraction inventory for isolated historical probes. Live household
/// loading uses `object_catalog::Catalog::global_controllers`, whose order and
/// winning sources come from the actual mounted resource manager.
pub fn global_controllers(index: &Index) -> Result<Vec<Controller>> {
    let mut result = Vec::new();
    let mut seen = BTreeMap::<u32, (String, u16)>::new();
    for id in registered_sources(index)? {
        let r = &index.resources[id];
        let iff = Iff::parse(r.read()?).with_context(|| r.locator.clone())?;
        for c in iff.all(b"OBJD") {
            let b = iff.data(c);
            ensure!(b.len() >= 0x56, "short catalog OBJD in {}", r.locator);
            let word = |i| u16::from_le_bytes(b[i..i + 2].try_into().unwrap());
            let kind = word(0x12);
            if !matches!(kind, 7 | 12) || word(0x54) == 0 {
                continue;
            }
            let guid = u32::from_le_bytes(b[28..32].try_into().unwrap());
            let hash = format!("{:x}", Sha256::digest(&iff.bytes));
            if let Some(old) = seen.insert(guid, (hash.clone(), c.id)) {
                ensure!(
                    old == (hash, c.id),
                    "ambiguous global controller GUID {guid:08x}"
                );
                continue;
            }
            result.push(Controller {
                guid,
                label: c.label.clone(),
                definition: DefinitionSource {
                    source: r.locator.clone(),
                    sha256: hash,
                    chunk: c.id,
                    kind,
                    multipart: word(0x14) != 0,
                },
            });
        }
    }
    Ok(result)
}

/// Native 0x43ba52 registers a storage record for each nonzero signed count
/// at OBJD +0xb4. Alias GUID +0xb8 is used at access time, not allocation time.
/// This legacy extraction inventory is retained for isolated probes. Live
/// households use object_catalog::Catalog::resource_storage in native order.
pub fn resource_storage(index: &Index) -> Result<Vec<sims_vm::resource_storage::Definition>> {
    let mut result = Vec::new();
    for id in registered_sources(index)? {
        let resource = &index.resources[id];
        let iff = Iff::parse(resource.read()?)?;
        for chunk in iff.all(b"OBJD") {
            let b = iff.data(chunk);
            // 0x431759 extends short OBJD resources with zero bytes. Read the
            // catalog fields with that same tail rule, including partial words.
            let count = i16::from_le_bytes([
                b.get(180).copied().unwrap_or(0),
                b.get(181).copied().unwrap_or(0),
            ]);
            if count == 0 {
                continue;
            }
            result.push(sims_vm::resource_storage::Definition {
                guid: u32::from_le_bytes(b[28..32].try_into().unwrap()),
                count,
                reset_per_lot: b.get(182).copied().unwrap_or(0) & 2 != 0,
                source: resource.locator.clone(),
                object: chunk.id,
                content_hash: format!("{:x}", Sha256::digest(&iff.bytes)),
            });
        }
    }
    Ok(result)
}

fn read_archive(path: &Path, len: u64) -> Result<Vec<far::Entry>> {
    let mut f = File::open(path)?;
    let mut header = [0; 16];
    f.read_exact(&mut header)?;
    let offset = far::manifest_offset(&header, len)?;
    let directory_size = len - u64::from(offset);
    if directory_size > 64 * 1024 * 1024 {
        bail!("FAR directory exceeds 64 MiB limit");
    }
    f.seek(SeekFrom::Start(u64::from(offset)))?;
    let mut directory = vec![0; directory_size as usize];
    f.read_exact(&mut directory)?;
    Ok(far::directory(&directory, len)?)
}
