//! Opening a neighborhood before one of its lots loads. House 0's
//! reload and the `Patch` import ([`sims_sim::patch_import::open`])
//! precede the five houseless pools.
//!
//! **What is written.** The original writes into the neighborhood's own
//! folder. Each made person type becomes `Characters/User%05d.iff`
//! (0x4722b8, then the import's resource edits and the outfit refresh,
//! 0x4293ae to 0x4295f7). A housed family's `*.FAM` file, with its GUID
//! table (Gtab #1) rewritten in place (pairs appended at 0x429754, the
//! write from 0x4297f7), becomes its `HouseNN.iff`. Here those files and
//! the recorded pool characters go into `output`, the caller's copy
//! of the neighborhood.
//!
//! **Not ported:**
//! - the save of `Neighborhood.iff` (0x4252cc), so the opened neighborhood
//!   is returned in memory, for [`crate::boot::Lot::load_in`];
//! - removing the imported `*.FAM` files (0x429a75 to 0x429a82);
//! - the portraits (BMP_ 2002 to 2007, 0x4e8c5d).

mod pools;

pub use pools::{LotTransition, OpenedPools, PoolInput, PoolPerson};

use crate::boot::BootError;
use sims_content::family_import::{clone_character, refresh_outfits};
use sims_content::{Catalog, ImportWardrobe, Index, Install, Neighborhood, People};
use sims_core::CrtRng;
use sims_formats::family_file::FamilyFile;
use sims_formats::iff::{Iff, write};
use sims_formats::iff_write::{ChunkWrite, write_chunk_with_residue};
use sims_sim::neighbors::registry;
use sims_sim::patch_import::{self, Context, Patch, open, pending};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// The context the import runs in: the complete collection's
/// expansions and the game's language (1, English).
const LANGUAGE: u8 = 1;

/// The flags word the resource writer gives a resource it marks converted
/// (0x419f80; otherwise 0, 0x419f88). The original's House56, 63, 69, 76
/// and 77 after its UserData2 import all carry it on Gtab #1.
const CONVERTED: u16 = 0x10;

/// The bytes written where the original leaves its heap's, when no
/// captured file gives them. A longer run fails, naming the address.
static ZEROS: [u8; 4096] = [0; 4096];

/// The Patch imports and the pool conversions performed by this open.
#[derive(Debug)]
pub struct Opened {
    pub patch: patch_import::Opened,
    pub pools: OpenedPools,
}

/// Opens the neighborhood at `directory` of the install at `root`, as the
/// original does before its view first shows and, if `pools.transition`
/// is supplied, before entering a lot. Writes new characters and houses
/// into `output` (its `Characters` and `Houses` folders must exist).
/// `new_guid` supplies the Patch import's GUIDs in draw order (0x4725ff).
/// `crt` starts at the first pool boundary, after any draws made by the
/// original's startup, navigation and Patch import. The open advances
/// it through the pools. The caller supplies their recorded inputs
/// explicitly, including which files to write.
///
/// The in-place write of a house's GUID table leaves some bytes as the
/// original's heap held them ([`sims_formats::iff_write`]). `captured`
/// may give, by house number, a file the original wrote, whose bytes are
/// taken there; otherwise they are 0. The lot load reads none of them.
pub fn open_neighborhood(
    root: &Path,
    directory: &Path,
    output: &Path,
    new_guid: &mut dyn FnMut() -> u32,
    captured: &dyn Fn(u32) -> Option<Vec<u8>>,
    pools: &PoolInput,
    crt: &mut CrtRng,
) -> Result<(Neighborhood, Opened), BootError> {
    let install = Install::detect(root)?;
    let mut neighborhood = Neighborhood::open(directory)?;
    let mut index = Index::build(&install)?;
    let characters = neighborhood.characters()?;
    if let Some(characters) = &characters {
        index.add_characters(characters)?;
    }
    let catalog = Catalog::build(&index)?;
    let registry = registry(&index, &catalog);
    let character_mount = index
        .mounts
        .iter()
        .position(|mount| mount.path == Index::CHARACTERS);
    let context = Context {
        catalog: &catalog,
        registry: &registry,
        character_files: characters
            .as_deref()
            .map(|characters| crate::program::character_files(&index, characters))
            .unwrap_or_default(),
        character_guids: catalog
            .objects
            .iter()
            .filter(|object| Some(index.resources[object.resource].mount) == character_mount)
            .map(|object| object.guid)
            .collect(),
        house_files: neighborhood.houses()?.into_keys().collect(),
        zoning: neighborhood.zoning(u16::from(install.expansions.0))?,
        lots: neighborhood.lot_rows()?,
        expansions: install.expansions.0,
        language: LANGUAGE,
    };
    // The scan reads the files only when an import is pending (0x42c1d5).
    let mut patches = Vec::new();
    let scanned = if pending(&neighborhood, install.expansions.0)? {
        neighborhood.patches()?
    } else {
        Vec::new()
    };
    for path in scanned {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("{}: a Patch file name that is not text", path.display()))?;
        if !name.to_ascii_lowercase().ends_with(".fam") {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        patches.push(Patch {
            name: name.to_owned(),
            file: FamilyFile::parse(&bytes)?,
        });
    }
    let patch = open(&mut neighborhood, &patches, &context, new_guid)?;
    let mut pool_crt = *crt;
    let skeletons = if pools.people.is_empty() {
        BTreeMap::new()
    } else {
        human_skeletons(&install, &index, &catalog)?
    };
    let filled = pools::fill(
        &mut neighborhood,
        &context,
        &patch,
        pools,
        &skeletons,
        &mut pool_crt,
    )?;
    write_imports(
        &install, &index, &catalog, &patches, &patch, output, captured,
    )?;
    // The pool people's files. If one fails, the ones this open wrote
    // are removed again, so the same inputs can be retried.
    let mut written = Vec::new();
    for (number, iff) in filled.files {
        let path = output.join(format!("Characters/User{number:05}.iff"));
        let result = std::fs::File::create_new(&path).and_then(|mut file| {
            written.push(path.clone());
            file.write_all(&iff.bytes)
        });
        if let Err(error) = result {
            for path in &written {
                let _ = std::fs::remove_file(path);
            }
            return Err(format!("0x4722b8: {}: {error}", path.display()).into());
        }
    }
    neighborhood.directory = output.to_path_buf();
    *crt = pool_crt;
    Ok((
        neighborhood,
        Opened {
            patch,
            pools: filled.report,
        },
    ))
}

fn write_imports(
    install: &Install,
    index: &Index,
    catalog: &Catalog,
    patches: &[Patch],
    opened: &patch_import::Opened,
    output: &Path,
    captured: &dyn Fn(u32) -> Option<Vec<u8>>,
) -> Result<(), BootError> {
    if opened.imports.is_empty() {
        return Ok(());
    }
    let wardrobe = ImportWardrobe::load(install)?;
    for (name, import) in &opened.imports {
        let patch = patches
            .iter()
            .find(|patch| &patch.name == name)
            .ok_or("an import names no Patch file")?;
        for (&guid, character) in &import.made {
            let definition = catalog.object(character.template).ok_or_else(|| {
                format!("template {:#010x} has no definition", character.template)
            })?;
            let template = Iff::parse(index.resources[definition.resource].read()?)?;
            let mut resources = clone_character(&template, character, guid, &patch.file)?;
            refresh_outfits(&mut resources, &wardrobe)?;
            let path = output.join(format!("Characters/User{:05}.iff", character.file));
            std::fs::write(&path, write(&resources)?)
                .map_err(|error| format!("{}: {error}", path.display()))?;
        }
        if let Some(lot) = &import.lot {
            let table = lot
                .chunks
                .get(b"Gtab")
                .and_then(|chunks| {
                    chunks
                        .iter()
                        .find(|chunk| chunk.id == 1 && chunk.flags >> 8 == 0)
                })
                .ok_or("an imported lot has no Gtab #1")?;
            let captured = captured(lot.house);
            let bytes = write_chunk_with_residue(
                &patch.file.iff.bytes,
                ChunkWrite {
                    tag: *b"Gtab",
                    id: 1,
                    flags: CONVERTED,
                    label: table.label.as_bytes(),
                    payload: &table.data,
                },
                |range| match &captured {
                    Some(file) => file.get(range),
                    None => ZEROS.get(..range.len()),
                },
            )?;
            let path = output.join(format!("Houses/House{:02}.iff", lot.house));
            std::fs::write(&path, bytes).map_err(|error| format!("{}: {error}", path.display()))?;
        }
    }
    Ok(())
}

/// Each person type's skeleton's own bone scales, by lowercase skeleton
/// name, as the installed skeleton files give them. A pool person's actor
/// copies the skeleton its profile names (0x598da3), which draws by
/// these scales. A name two files define with different scales fails.
fn human_skeletons(
    install: &Install,
    index: &Index,
    catalog: &Catalog,
) -> Result<BTreeMap<String, Vec<i32>>, BootError> {
    let person_types = catalog
        .objects
        .iter()
        .filter(|object| object.kind == 2)
        .map(|object| object.guid);
    let (people, _) = People::load_each(install, index, catalog, person_types)?;
    let mut skeletons = BTreeMap::new();
    for skeleton in people.skeletons.values() {
        let own = skeleton.scales.get(skeleton.place).ok_or_else(|| {
            format!(
                "0x598da3: the skeleton {:?} is not in its file",
                skeleton.name
            )
        })?;
        let name = skeleton.name.to_ascii_lowercase();
        if let Some(other) = skeletons.insert(name, own.clone())
            && &other != own
        {
            return Err(format!(
                "0x598da3: two skeletons named {:?} have different scales",
                skeleton.name
            )
            .into());
        }
    }
    Ok(skeletons)
}
