//! Opening a neighborhood before one of its lots loads: house 0's reload
//! and the import of its `Patch` families
//! ([`sims_sim::patch_import::open`]), with the files the import writes.
//!
//! **What is written.** The original writes into the neighborhood's own
//! folder. Each made person type becomes `Characters/User%05d.iff`
//! (0x4722b8, then the import's resource edits and the outfit refresh,
//! 0x4293ae to 0x4295f7). A housed family's `*.FAM` file, with its GUID
//! table (Gtab #1) rewritten in place (0x429754), becomes its
//! `HouseNN.iff`. Here those files go into `output`, which the caller
//! makes as a copy of the neighborhood; the neighborhood's own folder is
//! not touched.
//!
//! **Not ported:**
//! - the save of `Neighborhood.iff` (0x4252cc), so the opened neighborhood
//!   is returned in memory, for [`crate::boot::Lot::load_in`];
//! - removing the imported `*.FAM` files (0x429a75 to 0x429a82);
//! - the portraits (BMP_ 2002 to 2007, 0x4e8c5d);
//! - the pools the open fills after the import
//!   (`docs/decisions/20261120-neighborhood-open-fills-five-pools.md`).

use crate::boot::BootError;
use sims_content::family_import::{clone_character, refresh_outfits};
use sims_content::{Catalog, ImportWardrobe, Index, Install, Neighborhood};
use sims_formats::family_file::FamilyFile;
use sims_formats::iff::{Iff, write};
use sims_formats::iff_write::{ChunkWrite, write_chunk_with_residue};
use sims_sim::neighbors::registry;
use sims_sim::patch_import::{Context, Opened, Patch, open};
use std::path::Path;

/// The context the import runs in: the complete collection's
/// expansions and the game's language (1, English).
const LANGUAGE: u8 = 1;

/// The converted bit of a resource's flags. The original's House56, 63,
/// 69, 76 and 77 after its UserData2 import all carry it on Gtab #1.
const CONVERTED: u16 = 0x10;

/// The bytes written where the original leaves its heap's, when no
/// captured file gives them. A longer run fails, naming the address.
static ZEROS: [u8; 4096] = [0; 4096];

/// Opens the neighborhood at `directory` of the install at `root`, as the
/// original does before its view first shows, and writes the files the
/// import makes into `output` (its `Characters` and `Houses` folders,
/// which must exist). `new_guid` gives each made type's GUID, in the order
/// the original draws them (0x4725ff).
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
        expansions: install.expansions.0,
        language: LANGUAGE,
    };
    let mut patches = Vec::new();
    for path in neighborhood.patches()? {
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
    let opened = open(&mut neighborhood, &patches, &context, new_guid)?;

    let wardrobe = ImportWardrobe::load(&install)?;
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
                    flags: table.flags | CONVERTED,
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
    neighborhood.directory = output.to_path_buf();
    Ok((neighborhood, opened))
}
