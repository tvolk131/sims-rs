//! The data changes made by the neighborhood's family importer (0x428f05).

use crate::{Catalog, Error, Result};
use sims_formats::family_file::{FamilyFile, FamilyMember};
use sims_formats::iff::Iff;
use sims_formats::lot::ObjectTypes;
use sims_formats::neighborhood::{Family, Neighbor, Relationship};
use sims_formats::strings::{Format, StringTable, decode};
use std::collections::{BTreeMap, BTreeSet};

pub const TEMPLATE_PERSON: u32 = 0x7fd96b54;
pub const TEMPLATE_DOG: u32 = 0x4a70df92;
pub const TEMPLATE_CAT: u32 = 0x7bea0977;

const FILE_LIMIT: i32 = 0x7d00;
const FAMILY_WORD: usize = 61;
const CLEANUP_LOT_WORD: usize = 67;

/// The state at neighborhood open, after registering the install's people
/// (0x424e72) and assigning existing family membership (0x425244).
pub struct ImportInput<'a> {
    pub catalog: &'a Catalog,
    /// One-based record IDs, with vacant slots retained.
    pub neighbors: &'a [Option<Neighbor>],
    /// Families indexed by their FAMI chunk ID.
    pub families: &'a BTreeMap<u16, Family>,
    pub next_file: i32,
    /// Numbers occupied by both on-disk character files and earlier imports.
    pub character_files: &'a BTreeSet<i32>,
    /// Types registered from the Characters directory, including earlier imports.
    /// The original only deletes records with such a definition (0x423f30).
    pub character_guids: &'a BTreeSet<u32>,
    pub house_files: &'a BTreeSet<u32>,
    /// Lot numbers and their native zoning codes. Residential is 1.
    pub zoning: &'a BTreeMap<i16, i32>,
    pub neighborhood_data: [i16; 16],
}

#[derive(Debug, Clone, Copy)]
pub struct ImportOptions {
    pub pets_enabled: bool,
    /// The game's current one-based language code.
    pub language: u8,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            pets_enabled: false,
            language: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Character {
    pub template: u32,
    pub file: i32,
    /// The imported STR# 200 text, indexed from zero.
    pub strings: BTreeMap<usize, String>,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct Import {
    /// Made types by their new GUID.
    pub made: BTreeMap<u32, Character>,
    /// New records by their destination slot ID.
    pub neighbors: BTreeMap<i16, Neighbor>,
    pub neighbor_ids: BTreeMap<u16, i16>,
    pub guid_map: BTreeMap<u32, u32>,
    pub next_file: i32,
    /// Destination chunk ID for FAMI, FAMs and FAMh.
    pub family_id: u16,
    pub family: Family,
    /// The single FAMs string the destination writes in the active language.
    pub family_name: String,
    pub family_history: Option<Vec<u8>>,
    /// Updated FAMI for the occupant of the destination lot, when present.
    pub displaced_family: Option<(u16, Family)>,
    pub neighborhood_data: [i16; 16],
    /// The global simulator RNG state restored by reading SIMI #1
    /// (0x46cfb3, 0x46f44d). None when the file has no simulator.
    pub global_rng: Option<u32>,
    /// A housed family's patched IFF replaces HouseNN.iff. A houseless
    /// family's source is deleted (0x429a75–0x429a82).
    pub lot: Option<ImportedLot>,
}

#[derive(Debug, Clone)]
pub struct ImportedLot {
    pub house: u32,
    /// Live resources of the moved IFF, including its updated Gtab #1.
    /// Each vector retains IFF resource order for its tag.
    pub chunks: BTreeMap<[u8; 4], Vec<LotChunk>>,
    /// The objt table after applying Gtab, as the original lot reader sees it.
    pub object_types: ObjectTypes,
}

#[derive(Debug, Clone)]
pub struct LotChunk {
    pub id: u16,
    pub flags: u16,
    pub label: String,
    pub data: Vec<u8>,
}

/// Imports one parsed file without filesystem writes. The callback is
/// `timeGetTime().wrapping_shl(16) ^ u32::from(crt_rand15())` in the
/// original (0x4725ff). It runs once per copied OBJD, in EXPi member order,
/// after finding an available file and before advancing the counter for
/// that attempt and allocating the record.
/// Errors discard the output, although earlier calls to the callback stand.
pub fn import(
    input: &ImportInput<'_>,
    file: &FamilyFile,
    options: ImportOptions,
    new_guid: &mut dyn FnMut() -> u32,
) -> Result<Import> {
    if !(1..=20).contains(&options.language) {
        return Err(unsupported(
            "0x470256",
            "language outside the native string tables",
        ));
    }
    if file.iff.get(b"FINV", 2).is_some() {
        return Err(unsupported(
            "0x42a92e",
            "transfer of FINV inventories is not ported",
        ));
    }

    let (family_id, family_number) = family_numbers(input.families, file.export_info.family_id)?;
    let family_name = family_name(file, options.language)?;
    let house = file.family.house;
    let displaced_family = displace_family(input, house)?;
    let mut neighborhood_data = input.neighborhood_data;
    if i32::from(neighborhood_data[2]) == house {
        neighborhood_data[2] = 0;
    }
    let global_rng = if let Some(simi) = file.iff.get(b"SIMI", 1) {
        let simulator = sims_formats::lot::Simulator::parse(file.iff.data(simi))
            .map_err(|error| Error::format("family import: 0x4290e2 SIMI #1", error))?;
        if simulator.globals[26] != 0 {
            return Err(unsupported(
                "0x429903",
                "resetting type attributes for SIMI global 26 is not ported",
            ));
        }
        Some(simulator.global_rng)
    } else {
        None
    };

    let mut made = BTreeMap::new();
    let mut neighbor_ids = BTreeMap::new();
    let mut guid_map = BTreeMap::new();
    let mut guid_pairs = Vec::new();
    let mut members = Vec::new();
    let mut character_files = input.character_files.clone();
    let mut next_file = input.next_file;
    let mut vacancies: Vec<_> = input
        .neighbors
        .iter()
        .enumerate()
        .filter_map(|(slot, record)| record.is_none().then_some(slot))
        .collect();
    let mut slots = input.neighbors.len();
    for id in file
        .export_info
        .member_ids
        .into_iter()
        .take_while(|&id| id != 0)
    {
        let member = file
            .members
            .get(&id)
            .ok_or_else(|| unsupported("0x4292bb", format!("member {id} has no uChr resource")))?;
        let source = exported_neighbor(file, id)?;
        let raw_body = file.iff.get(b"uChr", id).map(|chunk| file.iff.data(chunk));
        let body = language_entries(&member.body_strings, raw_body, options.language, true);
        let template = choose_template(&body, id, options.pets_enabled)?;
        check_template(input, template)?;
        let (name, description) = character_text(file, member, id, options.language)?;
        let number = reserve_file(&mut next_file, &mut character_files)?;
        let guid = new_guid();
        next_file = (number + 1) % FILE_LIMIT;
        if guid == 0
            || input.catalog.object(guid).is_some()
            || made.contains_key(&guid)
            || input
                .neighbors
                .iter()
                .flatten()
                .any(|record| record.guid == guid)
        {
            return Err(unsupported(
                "0x43bda9",
                format!("registering a colliding or zero GUID {guid:#010x} is not ported"),
            ));
        }

        let new_id = allocate_neighbor(id, &mut vacancies, &mut slots)?;
        if neighbor_ids.insert(id, new_id).is_some() || guid_map.insert(source.guid, guid).is_some()
        {
            return Err(unsupported(
                "0x429754",
                "ambiguous exported member IDs or GUIDs are not ported",
            ));
        }
        guid_pairs.push((source.guid, guid));
        members.push((id, source, new_id, guid));
        made.insert(
            guid,
            Character {
                template,
                file: number,
                strings: body.into_iter().enumerate().collect(),
                name,
                description,
            },
        );
    }

    let neighbors = members
        .iter()
        .map(|&(_, source, id, guid)| {
            (
                id,
                copy_neighbor(source, id, guid, &members, family_id, made[&guid].file),
            )
        })
        .collect();
    let family = Family {
        version: 9, // 0x410696 always writes revision 9.
        house,
        number: family_number,
        budget: file.family.budget,
        architecture_value: file.family.architecture_value,
        // 0x427bd7 requires reciprocal relationships with nonmembers.
        // The new records only have relationships to one another.
        friends: 0,
        flags: file.family.flags & 1,
        members: members.iter().map(|&(_, _, _, guid)| guid).collect(),
        // The constructor initializes the secondary house and six magic words.
        tail: vec![0; 16],
    };
    let lot = if house == 0 {
        None
    } else {
        Some(import_lot(&file.iff, house as u32, guid_pairs)?)
    };

    Ok(Import {
        made,
        neighbors,
        neighbor_ids,
        guid_map,
        next_file,
        family_id,
        family,
        family_name,
        family_history: file.history.clone(),
        displaced_family,
        neighborhood_data,
        global_rng,
        lot,
    })
}

fn unsupported(address: &str, message: impl std::fmt::Display) -> Error {
    Error::Unsupported(format!("family import: {address} {message}"))
}

fn family_numbers(families: &BTreeMap<u16, Family>, exported: u16) -> Result<(u16, i32)> {
    // Allocation can fail before the importer checks whether it can keep
    // the export ID (0x425dea, 0x42913f).
    let free = (1..=i16::MAX as u16)
        .find(|id| !families.contains_key(id))
        .ok_or_else(|| unsupported("0x425e50", "no free family ID"))?;
    if exported > i16::MAX as u16 {
        return Err(unsupported("0x41031b", "negative exported family ID"));
    }
    let id = if families.contains_key(&exported) {
        free
    } else {
        exported
    };
    let number = families
        .values()
        .map(|family| family.number)
        .max()
        .unwrap_or(0)
        .max(0)
        .wrapping_add(1);
    if id == 0 {
        return Err(unsupported(
            "0x425244",
            "importing into the default family ID is not ported",
        ));
    }
    Ok((id, number))
}

fn displace_family(input: &ImportInput<'_>, house: i32) -> Result<Option<(u16, Family)>> {
    if house == 0 {
        return Ok(None);
    }
    let lot = i16::try_from(house)
        .ok()
        .filter(|&lot| lot > 0)
        .ok_or_else(|| unsupported("0x42731e", format!("invalid lot number {house}")))?;
    if input.zoning.get(&lot) != Some(&1) {
        return Err(unsupported(
            "0x42731e",
            format!("lot {house} is not zoned residential"),
        ));
    }
    if !input.house_files.contains(&(house as u32)) {
        return Err(unsupported(
            "0x4299e2",
            format!("House{house:02}.iff is missing before replacement"),
        ));
    }
    if input.neighbors.iter().flatten().any(|record| {
        record.person_data.get(CLEANUP_LOT_WORD) == Some(&lot)
            && input.character_guids.contains(&record.guid)
            && !input
                .families
                .values()
                .any(|family| family.members.contains(&record.guid))
    }) {
        return Err(unsupported(
            "0x426fcd",
            "cleanup of former residents without a family is not ported",
        ));
    }
    let mut occupants = Vec::new();
    for (&id, family) in input.families {
        let secondary = if family.version >= 8 {
            let bytes = family
                .tail
                .get(..4)
                .ok_or_else(|| unsupported("0x4104ce", "missing secondary house field"))?;
            i32::from_le_bytes(bytes.try_into().unwrap())
        } else {
            0
        };
        if family.house == house || secondary == house {
            occupants.push((id, family, secondary));
        }
    }
    match occupants.as_slice() {
        [] => Ok(None),
        [(id, family, secondary)] if *secondary != house => {
            let mut family = (*family).clone();
            family.house = 0;
            family.architecture_value = 0;
            family.flags &= !8;
            Ok(Some((*id, family)))
        }
        _ => Err(unsupported(
            "0x42731e",
            "lot remains assigned to a family after clearing its first occupant",
        )),
    }
}

fn exported_neighbor(file: &FamilyFile, id: u16) -> Result<&Neighbor> {
    if id <= i16::MAX as u16
        && let Some(Some(record)) = file.neighbors.entries.get(usize::from(id) - 1)
    {
        if record.person_mode >= 0 {
            return Ok(record);
        }
        return Err(unsupported(
            "0x423e8e",
            format!("negative person revision for member {id}"),
        ));
    }
    Err(unsupported(
        "0x42965d",
        format!("member {id} has no exported NBRS record"),
    ))
}

fn choose_template(body: &[String], id: u16, pets_enabled: bool) -> Result<u32> {
    // The native lookup takes a one-based index and compares via strstr.
    let gender = body.get(12).ok_or_else(|| {
        unsupported(
            "0x4292de",
            format!("member {id} has no thirteenth body string"),
        )
    })?;
    let template = if gender.contains("dog") {
        TEMPLATE_DOG
    } else if gender.contains("cat") {
        TEMPLATE_CAT
    } else {
        TEMPLATE_PERSON
    };
    if template != TEMPLATE_PERSON && !pets_enabled {
        return Err(unsupported(
            "0x42601e",
            "drop of a pet without the pets flag is not ported",
        ));
    }
    Ok(template)
}

fn check_template(input: &ImportInput<'_>, guid: u32) -> Result<()> {
    let template = input
        .catalog
        .object(guid)
        .filter(|object| {
            object.kind == 2
                && input
                    .neighbors
                    .iter()
                    .flatten()
                    .any(|record| record.guid == guid)
        })
        .ok_or_else(|| {
            unsupported(
                "0x426335",
                format!("missing template record or definition for {guid:#010x}"),
            )
        })?;
    if input
        .catalog
        .objects
        .iter()
        .filter(|object| object.guid == guid)
        .count()
        != 1
    {
        return Err(unsupported(
            "0x42571f",
            format!("multiple definitions of template {guid:#010x} are not ported"),
        ));
    }
    if input
        .catalog
        .objects
        .iter()
        .filter(|object| object.resource == template.resource)
        .count()
        != 1
    {
        return Err(unsupported(
            "0x47249d",
            format!("multiple OBJDs in template {guid:#010x} are not ported"),
        ));
    }
    Ok(())
}

fn reserve_file(next_file: &mut i32, files: &mut BTreeSet<i32>) -> Result<i32> {
    if *next_file < 0 {
        return Err(unsupported(
            "0x4261a2",
            "negative character file counter is not ported",
        ));
    }
    while *next_file < FILE_LIMIT {
        let candidate = *next_file;
        if files.insert(candidate) {
            return Ok(candidate);
        }
        *next_file += 1;
    }
    Err(unsupported(
        "0x4262c0",
        "no character file number available before the counter wraps",
    ))
}

fn allocate_neighbor(id: u16, vacancies: &mut Vec<usize>, slots: &mut usize) -> Result<i16> {
    let requested = usize::from(id);
    if requested > *slots {
        vacancies.extend(*slots..requested - 1);
        *slots = requested;
        return Ok(id as i16);
    }
    if let Ok(index) = vacancies.binary_search(&(requested - 1)) {
        vacancies.remove(index);
        return Ok(id as i16);
    }
    let slot = vacancies.pop().unwrap_or_else(|| {
        *slots += 1;
        *slots - 1
    });
    i16::try_from(slot + 1).map_err(|_| {
        unsupported(
            "0x425884",
            "neighbor slot exceeds the positive signed ID range",
        )
    })
}

// Each pair is the person word and the minimum revision in 0x423827.
const PERSON_FIELDS: &[(i32, &[usize])] = &[
    (
        1,
        &[
            2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 16, 20, 15, 26, 17, 18, 46, 47, 48, 49, 50, 51, 52,
            53, 13, 14, 58, 60, 61,
        ],
    ),
    (2, &[56, 57, 63]),
    (3, &[65]),
    (4, &[67, 68]),
    (5, &[69]),
    (6, &[74]),
    (7, &[54, 55, 75, 76]),
    (8, &[80, 81]),
    (9, &[82]),
    (10, &[83, 84, 85, 86]),
];

fn copy_neighbor(
    source: &Neighbor,
    id: i16,
    guid: u32,
    members: &[(u16, &Neighbor, i16, u32)],
    family_id: u16,
    file: i32,
) -> Neighbor {
    let mut person_data = vec![0; 256];
    for &(_, indices) in PERSON_FIELDS
        .iter()
        .take_while(|(since, _)| *since <= source.person_mode)
    {
        for &index in indices {
            // Older NBRS layouts read 80 words into the zeroed record.
            person_data[index] = source.person_data.get(index).copied().unwrap_or(0);
        }
    }
    person_data[FAMILY_WORD] = family_id as i16; // 0x425244.
    let relationships = members
        .iter()
        .filter_map(|&(exported_id, _, id, _)| {
            let row = source
                .relationships
                .iter()
                .find(|row| row.target == i32::from(exported_id))?;
            (!row.values.is_empty()).then(|| Relationship {
                target: i32::from(id),
                values: row.values.clone(),
            })
        })
        .collect();
    Neighbor {
        version: 10,
        version_extra: Some(9),
        name: format!("user{file:05}"),
        unknown_zero: 0,
        person_mode: source.person_mode,
        person_data,
        id,
        guid,
        unknown_negative: -1,
        relationships,
    }
}

fn family_name(file: &FamilyFile, language: u8) -> Result<String> {
    if let Some(chunk) = file.iff.get(b"CTSS", 1000) {
        let bytes = file.iff.data(chunk);
        let table = StringTable::parse(bytes)
            .map_err(|error| Error::format("family import: 0x429001 CTSS #1000", error))?;
        return language_entries(&table, Some(bytes), language, true)
            .into_iter()
            .next()
            .ok_or_else(|| unsupported("0x429013", "CTSS #1000 has no family name"));
    }
    let bytes = file
        .iff
        .get(b"FAMs", file.export_info.family_id)
        .map(|chunk| file.iff.data(chunk));
    Ok(language_entries(&file.name, bytes, language, false)
        .into_iter()
        .next()
        .or_else(|| {
            file.name
                .entries
                .iter()
                .filter(|entry| entry.language <= 20)
                .min_by_key(|entry| entry.language)
                .map(|entry| entry.text.clone())
        })
        .unwrap_or_default())
}

fn character_text(
    file: &FamilyFile,
    member: &FamilyMember,
    id: u16,
    language: u8,
) -> Result<(String, String)> {
    let same_id = || {
        if file.iff.get(b"CATS", id).is_some() {
            return Err(unsupported(
                "0x406222",
                format!("CATS text for member {id} is not ported"),
            ));
        }
        Ok(member.ctss.as_deref())
    };
    let (bytes, fallback) = if let Some(bytes) = &member.localized_ctss {
        (Some(bytes.as_slice()), true)
    } else {
        (same_id()?, false)
    };
    let Some(bytes) = bytes else {
        return Ok((String::new(), String::new()));
    };
    let location = format!("family import: 0x42945d CTSS for member {id}");
    let table = StringTable::parse(bytes).map_err(|error| Error::format(&location, error))?;
    let entries = language_entries(&table, Some(bytes), language, fallback);
    if fallback && entries.len() < 2 {
        return Err(unsupported(
            "0x4294ad",
            format!("localized CTSS for member {id} lacks a name or description"),
        ));
    }
    let text = |index: usize| entries.get(index).cloned().unwrap_or_default();
    Ok((text(0), text(1)))
}

fn language_entries(
    table: &StringTable,
    raw: Option<&[u8]>,
    language: u8,
    fallback: bool,
) -> Vec<String> {
    if table.format != Format::Localized {
        return table
            .entries
            .iter()
            .map(|entry| decode(&entry.raw_text, language))
            .collect();
    }
    let mut entries = Vec::new();
    let mut english = Vec::new();
    let mut offset = 4;
    for entry in &table.entries {
        // StringTable normalizes encoded language 0 to 1. The original
        // treats 0 as the active language (0x46fffa, 0x470e3d).
        let encoded = raw
            .and_then(|bytes| bytes.get(offset))
            .copied()
            .unwrap_or(entry.language);
        offset += entry.raw_text.len() + entry.raw_comment.len() + 3;
        if encoded == 0 || encoded == language {
            entries.push(decode(&entry.raw_text, language));
        }
        if encoded == 1 {
            english.push(decode(&entry.raw_text, language));
        }
    }
    if entries.is_empty() && fallback {
        entries = english;
    }
    entries
}

fn import_lot(iff: &Iff, house: u32, mut pairs: Vec<(u32, u32)>) -> Result<ImportedLot> {
    let source_types = iff
        .get(b"objt", 0)
        .ok_or_else(|| unsupported("0x43dd76", format!("lot {house} has no objt #0")))?;
    let mut object_types = ObjectTypes::parse(iff.data(source_types))
        .map_err(|error| Error::format("family import: 0x43a516 objt #0", error))?;
    if let Some(table) = iff.get(b"Gtab", 1) {
        let bytes = iff.data(table);
        // The version word is not checked: the import writes 0
        // (0x4157b6 passes it to 0x415835), as in the first
        // neighborhood's Old Town houses.
        if bytes.get(8..12) != Some(b"batG") {
            return Err(unsupported("0x41578e", "unrecognized Gtab #1 header"));
        }
        let count = bytes
            .get(12..16)
            .map(|n| u32::from_le_bytes(n.try_into().unwrap()));
        let entries = bytes
            .get(16..)
            .filter(|data| Some(data.len() / 8) == count.map(|n| n as usize) && data.len() % 8 == 0)
            .ok_or_else(|| unsupported("0x4158a4", "incomplete Gtab #1 pairs"))?;
        let mut existing: Vec<_> = entries
            .as_chunks::<8>()
            .0
            .iter()
            .map(|pair| {
                (
                    u32::from_le_bytes(pair[..4].try_into().unwrap()),
                    u32::from_le_bytes(pair[4..].try_into().unwrap()),
                )
            })
            .collect();
        existing.append(&mut pairs);
        pairs = existing;
    }

    for entry in &mut object_types.entries {
        // 0x415751 restarts its search after each hop and marks a pair
        // used for the current lookup, so even cycles terminate.
        let mut used = BTreeSet::new();
        while let Some((index, &(_, new))) = pairs
            .iter()
            .enumerate()
            .find(|(index, (old, _))| *old == entry.guid && !used.contains(index))
        {
            used.insert(index);
            entry.guid = new;
        }
    }
    let tags: BTreeSet<_> = iff.chunks.iter().map(|chunk| chunk.tag).collect();
    let mut chunks = BTreeMap::<_, Vec<_>>::new();
    for tag in tags {
        for chunk in iff.all(&tag) {
            if tag == *b"Gtab" && chunk.id == 1 && chunk.flags >> 8 == 0 {
                continue;
            }
            chunks.entry(tag).or_default().push(LotChunk {
                id: chunk.id,
                flags: chunk.flags,
                label: chunk.label.clone(),
                data: iff.data(chunk).to_vec(),
            });
        }
    }
    // Version 0 (0x4157b6), as the shipped Old Town houses' tables have.
    let mut table = b"\0\0\0\0\0\0\0\0batG".to_vec();
    table.extend((pairs.len() as u32).to_le_bytes());
    for (old, new) in pairs {
        table.extend(old.to_le_bytes());
        table.extend(new.to_le_bytes());
    }
    chunks.entry(*b"Gtab").or_default().push(LotChunk {
        id: 1,
        flags: 0,
        label: String::new(),
        data: table,
    });
    Ok(ImportedLot {
        house,
        chunks,
        object_types,
    })
}
