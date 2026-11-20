//! What opening a neighborhood does to its families and records before
//! its view first shows: the reload of house 0, then the import of the
//! families in its `Patch` folder.
//!
//! **The order (0x557fd3, the neighborhood view's update).** While its
//! byte +0x1b0 is set, the view's update calls 0x55c958, which loads house 0
//! (0x55cb41 → 0x428843 → 0x428041 with house 0) and clears the byte. The
//! house load starts with the neighborhood's reload (0x424806). A later
//! update, with +0x1d5 set, asks 0x42c1d5 whether an import is pending (the
//! expansions mask, `[0x6e48bc]+0x66`, has Unleashed's bit 0x20, and the
//! neighborhood's option byte +0x1aa is set) and, if so, imports (0x42c0ff).
//! So the import sees the records the reload left: a template neighborhood
//! (`UserData2` to `UserData8`) saves no record of TemplateDog or
//! TemplateCat, and the reload makes them before an import clones them.
//!
//! **The import (0x42c0ff).** Before the scan (0x42c122 to 0x42c1ad), the
//! first family, in the reload's order, whose ID (+0x10c, its FAMI chunk
//! ID) is 4000 takes the largest family ID plus one, so that `Strays_4000`
//! keeps 4000. The scan (0x42bca4) imports each `*.FAM` file the folder
//! lists ([`sims_content::family_import`]); then the option byte is cleared
//! (0x42c1ba) and the neighborhood is saved (0x4252cc).
//!
//! **Not modeled:** the rest of house 0's load (its objects, which the
//! boot does not model for the base neighborhood either), the dialog the
//! scan shows, and the files it writes. The caller takes the made types,
//! records, families and lots from the result. The families' friend
//! counts are left as the file saved them: the save (0x42542e) counts
//! again those that are out of date, and every family is out of date
//! after a load (0x41003c), so the lot load counts them again too.

use crate::load::LoadError;
use crate::neighbors::{FAMILY, RECORDED_KINDS, Registered};
use sims_content::family_import::{self, Import, ImportInput, ImportOptions};
use sims_content::{Catalog, Neighborhood, PENDING_IMPORT};
use sims_formats::family_file::FamilyFile;
use sims_formats::neighborhood::{Family, Neighbor};
use std::collections::{BTreeMap, BTreeSet};

/// Unleashed's bit of the installed-expansions mask (0x42c10b).
pub const PETS: u8 = 0x20;

/// The family ID the pre-step frees (0x42c144).
pub const STRAYS: u16 = 4000;

/// One `*.FAM` file of the `Patch` folder, in the order the scan lists it.
pub struct Patch {
    pub name: String,
    pub file: FamilyFile,
}

/// What the import reads besides the neighborhood itself.
pub struct Context<'a> {
    pub catalog: &'a Catalog,
    /// Every registered definition, in order ([`crate::neighbors::registry`]).
    pub registry: &'a [Registered],
    /// The numbers of the `Characters/User%05d.iff` files on disk.
    pub character_files: BTreeSet<i32>,
    /// The GUIDs of the types registered from the `Characters` folder.
    pub character_guids: BTreeSet<u32>,
    /// The house numbers with a `HouseNN.iff` file.
    pub house_files: BTreeSet<u32>,
    /// Each lot's zoning ([`Neighborhood::zoning`]).
    pub zoning: BTreeMap<i16, i32>,
    /// The installed-expansions mask (`[0x6e48bc]+0x66`).
    pub expansions: u8,
    /// The game's one-based language code.
    pub language: u8,
}

/// What opening the neighborhood did.
#[derive(Debug, Default)]
pub struct Opened {
    /// The IDs of the records house 0's reload made, in the order made.
    pub made_records: Vec<i16>,
    /// The family the pre-step renumbered: its old and new ID.
    pub renumbered: Option<(u16, u16)>,
    /// Each file's import, in scan order.
    pub imports: Vec<(String, Import)>,
    /// The file counter after the last import (neighborhood +0x160).
    pub next_file: i32,
}

/// Opens `neighborhood`: house 0's reload as far as the import reads it,
/// then, when one is pending, the import of `patches` in their order.
/// `new_guid` gives each new type's GUID (0x4725ff); the original draws
/// them from the clock and the C runtime's generator.
///
/// The neighborhood is left as the save after the import (0x4252cc) would
/// write it: the made records, the imported records and families, the
/// data words, and option 10 cleared.
pub fn open(
    neighborhood: &mut Neighborhood,
    patches: &[Patch],
    context: &Context<'_>,
    new_guid: &mut dyn FnMut() -> u32,
) -> Result<Opened, LoadError> {
    let mut opened = Opened {
        made_records: reload(neighborhood, context.registry)?,
        ..Opened::default()
    };
    rebuild_family_words(neighborhood)?;
    // 0x42c1d5: Unleashed's bit, then the option byte.
    if context.expansions & PETS == 0 {
        return Ok(opened);
    }
    let options = neighborhood.options.as_mut().ok_or_else(|| {
        LoadError::Neighborhood(
            "the neighborhood file has no NGBH #1, so whether an import is pending is the \
             constructor's (0x4244bc), which is not ported"
                .into(),
        )
    })?;
    if !options[PENDING_IMPORT] {
        return Ok(opened);
    }

    // 0x42c122 to 0x42c1ad. IDs are unique, so the reload's order (which
    // family 4000 is first) does not matter, and the reload checked that
    // none is negative as a signed word, so the unsigned max is the
    // signed one.
    if neighborhood.families.contains_key(&STRAYS) {
        let largest = neighborhood.families.keys().max().copied().unwrap_or(0);
        let id = largest.checked_add(1).ok_or_else(|| {
            LoadError::Neighborhood("renumbering family 4000 overflows its ID".into())
        })?;
        let family = neighborhood
            .families
            .remove(&STRAYS)
            .expect("ordered family");
        neighborhood.families.insert(id, family);
        opened.renumbered = Some((STRAYS, id));
    }

    // 0x42bca4.
    let options = ImportOptions {
        pets_enabled: true,
        language: context.language,
    };
    let mut character_files = context.character_files.clone();
    let mut character_guids = context.character_guids.clone();
    let mut next_file = 0; // the reload's (0x424e8e)
    for patch in patches {
        let input = ImportInput {
            catalog: context.catalog,
            neighbors: &neighborhood.neighbors,
            families: &neighborhood.families,
            next_file,
            character_files: &character_files,
            character_guids: &character_guids,
            house_files: &context.house_files,
            zoning: &context.zoning,
            neighborhood_data: neighborhood.data,
        };
        let import =
            family_import::import(&input, &patch.file, options, new_guid).map_err(|error| {
                LoadError::Neighborhood(format!("importing {}: {error}", patch.name))
            })?;
        apply(neighborhood, &import)?;
        rebuild_family_words(neighborhood)?; // 0x429873 → 0x425244
        character_files.extend(import.made.values().map(|made| made.file));
        character_guids.extend(import.made.keys().copied());
        next_file = import.next_file;
        opened.imports.push((patch.name.clone(), import));
    }
    opened.next_file = next_file;
    if let Some(options) = &mut neighborhood.options {
        options[PENDING_IMPORT] = false; // 0x42c1ba
    }
    Ok(opened)
}

/// House 0's reload (0x424806), on the file's records and families, as
/// far as the import reads them: [`crate::World::read_neighbors`] does the
/// same to the simulation's records at a lot's load.
///
/// - A record whose type no definition has fails, as there.
/// - Each family's members that no record has the GUID of are dropped.
/// - Each registered definition of a type in [`RECORDED_KINDS`] that no
///   record is linked to gets a record, in the last empty slot or a new
///   one at the end, with ID its place plus one, revision 0, no person data
///   and no relationships. Its name (+0x28, from the definition) and the
///   fields the file saves around it are not modeled.
///
/// - A family whose ID is negative as a signed word fails: the reader
///   skips ID 0xffff (0x424bb6) and keeps the others negative in +0x10c,
///   which the pre-step's signed max then passes over; neither is ported.
/// - A family other than family 0 left with no members fails: the house
///   load deletes it (0x428071 to 0x4280b1, 0x425efb), which is not
///   ported.
///
/// The families' order (0x425209) is not kept: [`rebuild_family_words`]
/// fails when it would matter. Returns the IDs of the records made.
fn reload(neighborhood: &mut Neighborhood, registry: &[Registered]) -> Result<Vec<i16>, LoadError> {
    let registered: BTreeSet<u32> = registry.iter().map(|d| d.guid).collect();
    let mut first = BTreeMap::new();
    for (place, record) in neighborhood.neighbors.iter().enumerate() {
        if let Some(record) = record {
            if !registered.contains(&record.guid) {
                return Err(LoadError::Neighborhood(format!(
                    "record {} has the person type 0x{:08X}, which no definition has; the \
                     reload deletes it (0x4259a2), which is not ported",
                    record.id, record.guid
                )));
            }
            first.entry(record.guid).or_insert(place);
        }
    }
    if !neighborhood.families.contains_key(&0) {
        return Err(LoadError::Neighborhood(
            "the neighborhood has no family 0; the reload adds one (0x425209), which the import \
             path does not port"
                .into(),
        ));
    }
    if let Some(id) = neighborhood.families.keys().find(|&&id| id >= 0x8000) {
        return Err(LoadError::Neighborhood(format!(
            "family {id:#06x} is negative as the reader's signed word; the reader skips 0xffff \
             and keeps the rest negative (0x424bb6), which is not ported"
        )));
    }
    for family in neighborhood.families.values_mut() {
        family.members.retain(|guid| first.contains_key(guid));
    }
    if let Some(id) = neighborhood
        .families
        .iter()
        .find(|(id, family)| **id != 0 && family.members.is_empty())
        .map(|(id, _)| *id)
    {
        return Err(LoadError::Neighborhood(format!(
            "family {id} has no member with a record; the house load deletes it (0x428071, \
             0x425efb), which is not ported"
        )));
    }
    let mut made = Vec::new();
    for definition in registry {
        let linked = definition.resolves && first.contains_key(&definition.guid);
        if RECORDED_KINDS.contains(&definition.kind) && !linked {
            made.push(make_record(&mut neighborhood.neighbors, definition.guid)?);
        }
    }
    Ok(made)
}

/// Person data word 61 rebuilt (0x425244), as after the reload and after
/// each file's import (0x429873): every record's is cleared, then each
/// family writes its ID into the first record of each member's GUID.
///
/// The original walks the families in its own order; a GUID two families
/// share makes that order matter, and that fails loudly. A record whose
/// person data stops before word 61 (a record the reload made, or one
/// saved without person data) has it as 0, and a member's such record
/// fails loudly.
fn rebuild_family_words(neighborhood: &mut Neighborhood) -> Result<(), LoadError> {
    let mut family_of = BTreeMap::new();
    for (&id, family) in &neighborhood.families {
        for &guid in &family.members {
            if let Some(other) = family_of.insert(guid, id)
                && other != id
            {
                return Err(LoadError::Neighborhood(format!(
                    "the person type 0x{guid:08X} belongs to families {other} and {id}; which \
                     one word 61 names depends on the families' order (0x425244), which is \
                     not kept"
                )));
            }
        }
    }
    let mut seen = BTreeSet::new();
    for record in neighborhood.neighbors.iter_mut().flatten() {
        let family = if seen.insert(record.guid) {
            family_of.get(&record.guid).copied().unwrap_or(0)
        } else {
            0
        };
        match record.person_data.get_mut(FAMILY) {
            Some(word) => {
                *word = i16::try_from(family).expect("the reload checked the family IDs");
            }
            None if family == 0 => {}
            None => {
                return Err(LoadError::Neighborhood(format!(
                    "record {} is family {family}'s member but saves no word 61 (0x425244)",
                    record.id
                )));
            }
        }
    }
    Ok(())
}

/// [`crate::neighbors::Neighbors::make_record`] on the file's records.
fn make_record(slots: &mut Vec<Option<Neighbor>>, guid: u32) -> Result<i16, LoadError> {
    let place = match slots.iter().rposition(Option::is_none) {
        Some(place) => place,
        None => {
            slots.push(None);
            slots.len() - 1
        }
    };
    let id = i16::try_from(place + 1).map_err(|_| {
        LoadError::Neighborhood("the neighborhood holds more records than a word counts".into())
    })?;
    slots[place] = Some(Neighbor {
        version: 4,
        version_extra: None,
        name: String::new(),
        unknown_zero: 0,
        person_mode: 0,
        person_data: Vec::new(),
        id,
        guid,
        unknown_negative: -1,
        relationships: Vec::new(),
    });
    Ok(id)
}

/// One file's import, applied to the neighborhood for the next.
fn apply(neighborhood: &mut Neighborhood, import: &Import) -> Result<(), LoadError> {
    for (&id, record) in &import.neighbors {
        let place = usize::try_from(id - 1)
            .map_err(|_| LoadError::Neighborhood(format!("imported record ID {id}")))?;
        if neighborhood.neighbors.len() <= place {
            neighborhood.neighbors.resize(place + 1, None);
        }
        if neighborhood.neighbors[place]
            .replace(record.clone())
            .is_some()
        {
            return Err(LoadError::Neighborhood(format!(
                "the import put record {id} in a slot already held"
            )));
        }
    }
    if let Some((id, family)) = &import.displaced_family {
        neighborhood.families.insert(*id, family.clone());
    }
    let family: Family = import.family.clone();
    if neighborhood
        .families
        .insert(import.family_id, family)
        .is_some()
    {
        return Err(LoadError::Neighborhood(format!(
            "the import's family ID {} is already held",
            import.family_id
        )));
    }
    neighborhood.data = import.neighborhood_data;
    Ok(())
}
