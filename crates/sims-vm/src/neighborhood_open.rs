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
//! Each imported `*.FAM` file is then used up (0x429947 to 0x429a82): a
//! housed family's is moved onto its `HouseNN.iff` (the old house file
//! goes to `.tmp`, the family file takes its name, and the `.tmp` is
//! deleted), and a houseless family's is deleted. So the output's
//! `Patch` folder loses every file the open imported.
//!
//! **Not ported:**
//! - the save of `Neighborhood.iff` (0x4252cc), so the opened neighborhood
//!   is returned in memory, for [`crate::boot::Lot::load_in`];
//! - the portraits (BMP_ 2002 to 2007, 0x4e8c5d).

mod pools;

pub use pools::{LotTransition, OpenedPools, PoolInput, PoolPerson};

use crate::boot::BootError;
use sims_content::family_import::{clone_character, refresh_outfits};
use sims_content::{
    ArchivePool, AvatarRegistry, Catalog, ImportWardrobe, Index, Install, Neighborhood, People,
    skeleton_name,
};
use sims_core::CrtRng;
use sims_formats::family_file::FamilyFile;
use sims_formats::iff::{Iff, write};
use sims_formats::iff_write::{ChunkWrite, write_chunk_with_residue};
use sims_sim::neighbors::registry;
use sims_sim::patch_import::{self, Context, Patch, open, pending};
use std::collections::BTreeSet;
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
/// into `output` (its `Characters` and `Houses` folders must exist), and
/// removes the imported files from its `Patch` folder.
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
    let mut patch_paths = Vec::new();
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
        // Where the output copy has the file.
        let relative = path
            .strip_prefix(directory)
            .map_err(|_| format!("{}: a Patch file outside the neighborhood", path.display()))?;
        patch_paths.push(output.join(relative));
    }
    let patch = open(&mut neighborhood, &patches, &context, new_guid)?;
    let mut pool_crt = *crt;
    let skeletons = if pools.people.is_empty() {
        pools::Skeletons::default()
    } else {
        pool_skeletons(&install, &index, &catalog, &registry)?
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
            let mut message = format!("0x4722b8: {}: {error}", path.display());
            for path in &written {
                if let Err(error) = std::fs::remove_file(path) {
                    message += &format!("; and removing {} failed: {error}", path.display());
                }
            }
            return Err(message.into());
        }
    }
    // Last, once every file is written, so a failure above leaves the
    // copy's `Patch` folder as it was.
    use_up_imported(&patch, &patches, &patch_paths)?;
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

/// Uses up each imported `*.FAM` file in the output copy (0x429947 to
/// 0x429a82). A housed family's file was moved onto its house file, which
/// [`write_imports`] has written in its place; a houseless family's was
/// deleted (0x429a75 → 0x5b98d5). Either way the file is gone from
/// `Patch`. A file the copy lacks is left alone: the deletion's result is
/// not checked (0x429a87).
fn use_up_imported(
    opened: &patch_import::Opened,
    patches: &[Patch],
    paths: &[std::path::PathBuf],
) -> Result<(), BootError> {
    for (name, _) in &opened.imports {
        let at = patches
            .iter()
            .position(|patch| &patch.name == name)
            .ok_or("an import names no Patch file")?;
        let path = &paths[at];
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("0x429a82: {}: {error}", path.display()).into()),
        }
    }
    Ok(())
}

/// The bone scales a pool person's temporary actors copy (0x598da3).
/// The first actor is made from the pool's template, so it takes the
/// template's skeleton (0x426e8e). A second actor, made after the age is
/// set (0x426e97, 0x40cc2a), takes the one its profile names, `adult` or
/// `child`. The templates are the resolved definitions the pools can
/// pick (kinds 0x800, 0x200, 0x1000 and 0x400). Each skeleton is looked
/// up by name (0x5a0384) in the installed avatar files. A lookup that
/// fails is kept and reported only if a pool person needs it, so an
/// unrelated definition cannot stop the open.
fn pool_skeletons(
    install: &Install,
    index: &Index,
    catalog: &Catalog,
    registered: &[sims_sim::neighbors::Registered],
) -> Result<pools::Skeletons, BootError> {
    let mut skeletons = pools::Skeletons::default();
    let mut names = BTreeSet::from(["adult".to_owned(), "child".to_owned()]);
    for entry in registered {
        if !(matches!(entry.kind, 0x800 | 0x200 | 0x1000 | 0x400) && entry.resolves) {
            continue;
        }
        // The registry resolves a GUID to its last definition in reload
        // order, but the catalog keeps the index's; with two, which one
        // the registry gives is not followed here.
        let definitions = catalog
            .objects
            .iter()
            .filter(|object| object.guid == entry.guid)
            .count();
        let name = if definitions == 1 {
            skeleton_name(index, catalog, entry.guid).map_err(|error| error.to_string())
        } else {
            Err(format!(
                "{definitions} definitions have the GUID {:#010x}; reading the one the \
                 registry resolves to is unported",
                entry.guid
            ))
        };
        if let Ok(name) = &name {
            names.insert(name.clone());
        }
        skeletons.templates.insert(entry.guid, name);
    }
    let mut avatars = AvatarRegistry::load(install)?;
    // The registry as the startup scan left it (0x59f844, 0x584562), before
    // the lookups below load anything: which skeletons are loaded, and
    // which an `.ndx` names for loading on first use (0x5a03cd).
    let startup = People::load_each_with_registry(
        index,
        catalog,
        std::iter::empty(),
        &avatars,
        &ArchivePool::new(),
    )?
    .0;
    skeletons.eager = startup.skeletons_by_name.into_keys().collect();
    skeletons.indexed = startup.indexed_skeletons;
    skeletons.catalogs = startup.indexed_catalogs;
    for name in names {
        let scales = avatars
            .skeleton_scales(&name)
            .map_err(|error| error.to_string());
        skeletons.scales.insert(name, scales);
    }
    Ok(skeletons)
}
