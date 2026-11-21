//! The data changes made by the neighborhood's family importer (0x428f05).

use crate::{Catalog, Error, ImportWardrobe, Result};
use sims_formats::family_file::{FamilyFile, FamilyMember};
use sims_formats::far::decode_name;
use sims_formats::iff::{ChunkData, Iff};
use sims_formats::lot::{GuidTable, ObjectTypes};
use sims_formats::neighborhood::{Family, Neighbor, Relationship};
use sims_formats::strings::{Format, StringTable, decode};
use std::collections::{BTreeMap, BTreeSet};

mod outfits;

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
    member: u16,
    language: u8,
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
        let [name, description] = character_text(file, member, id, options.language)?
            .map(|text| decode(&text, options.language));
        let number = reserve_character_file(&mut next_file, &mut character_files)?;
        let guid = new_guid();
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
                member: id,
                language: options.language,
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

/// The live resources of a made character after the import's resource edits
/// (0x4722b8 and 0x4293ae–0x4295ee). Use the source FAM passed to [`import`].
/// The destination file number can be set before cloning.
///
/// Call [`refresh_outfits`] to perform 0x4295f7 before serializing with
/// [`sims_formats::iff::write`]. The portrait renderer at 0x4e8c5d and
/// its BMP_ 2002–2007 writes at 0x4e9000 are separate.
pub fn clone_character(
    template: &Iff,
    character: &Character,
    guid: u32,
    file: &FamilyFile,
) -> Result<Vec<ChunkData>> {
    let id = character.member;
    let language = character.language;
    let member = file
        .members
        .get(&id)
        .ok_or_else(|| unsupported("0x4292bb", format!("member {id} has no uChr resource")))?;
    let body_chunk = file
        .iff
        .get(b"uChr", id)
        .ok_or_else(|| unsupported("0x4292bb", format!("member {id} has no uChr resource")))?;
    let body = raw_language_entries(
        &member.body_strings,
        Some(file.iff.data(body_chunk)),
        language,
        true,
    );
    let [name, description] = character_text(file, member, id, language)?;
    if body.len() != character.strings.len()
        || body
            .iter()
            .enumerate()
            .any(|(index, text)| character.strings.get(&index) != Some(&decode(text, language)))
        || decode(&name, language) != character.name
        || decode(&description, language) != character.description
    {
        return Err(unsupported(
            "0x4293ae",
            "clone text differs from the source member",
        ));
    }
    if !(0..FILE_LIMIT).contains(&character.file) {
        return Err(unsupported("0x4261a2", "invalid character file number"));
    }
    if guid == 0 || guid == character.template {
        return Err(unsupported("0x43bda9", "zero or colliding character GUID"));
    }

    let tags: BTreeSet<_> = template.chunks.iter().map(|chunk| chunk.tag).collect();
    let mut chunks = Vec::new();
    for tag in tags {
        let live = template.resource_order(&tag).map_err(|error| {
            Error::format("family import: 0x472331 template resource order", error)
        })?;
        if &tag == b"cst\0" && !live.is_empty() {
            return Err(unsupported(
                "0x4723e1",
                "legacy cst conversion is not ported",
            ));
        }
        for chunk in live {
            if &tag == b"XXXX" || (tag == *b"BMP_" && (2002..=2007).contains(&chunk.id)) {
                continue;
            }
            chunks.push(ChunkData {
                tag,
                id: chunk.id,
                flags: chunk.flags,
                label: chunk.label.clone(),
                data: template.data(chunk).to_vec(),
            });
        }
    }
    let mut definitions = chunks.iter_mut().filter(|chunk| chunk.tag == *b"OBJD");
    let definition = match (definitions.next(), definitions.next()) {
        (Some(definition), None) if definition.flags >> 8 == 0 => definition,
        _ => return Err(unsupported("0x47249d", "expected one default-variant OBJD")),
    };
    let data = &mut definition.data;
    if data.len() < 0x90
        || data[0x12..0x14] != 2_u16.to_le_bytes()
        || data[0x1c..0x20] != character.template.to_le_bytes()
    {
        return Err(unsupported(
            "0x4725d5",
            "OBJD does not define the selected person template",
        ));
    }
    if data[0x26..0x28] != 200_u16.to_le_bytes() {
        return Err(unsupported(
            "0x457fd7",
            "the template does not load STR# 200",
        ));
    }
    let catalog_id = u16::from_le_bytes(data[0x52..0x54].try_into().unwrap());
    data[0x8c..0x90].copy_from_slice(&character.template.to_le_bytes()); // 0x4725e4
    data[0x1c..0x20].copy_from_slice(&guid.to_le_bytes()); // 0x4725f3
    let mut label = format!("user{:05} - ", character.file).into_bytes();
    label.extend(&name);
    label.truncate(63); // 0x429596, 0x46f79a
    definition.label = decode_name(&label);

    let ctss = catalog_strings(template, catalog_id, language, [&name, &description])?;
    // 0x4063df removes legacy CATS after writing CTSS.
    chunks.retain(|chunk| {
        !(chunk.tag == *b"CATS" && chunk.id == catalog_id && chunk.flags >> 8 == 0)
    });
    for (tag, id, data) in [
        (
            *b"STR#",
            200,
            localized_strings(body.into_iter().map(|text| (1, text, &[][..])))?,
        ),
        (*b"CTSS", catalog_id, ctss),
    ] {
        let replacement = ChunkData {
            tag,
            id,
            flags: 0x10,
            label: String::new(),
            data,
        };
        if let Some(chunk) = chunks
            .iter_mut()
            .find(|chunk| chunk.tag == tag && chunk.id == id && chunk.flags >> 8 == 0)
        {
            *chunk = replacement;
        } else {
            chunks.push(replacement);
        }
    }
    Ok(chunks)
}

/// Refresh the STR# 200 of a successful [`clone_character`] using the
/// startup wardrobe, then write it back to the clone (0x4295f7, 0x457f60).
/// If selection reaches an unsupported branch, the clone remains intact.
pub fn refresh_outfits(chunks: &mut [ChunkData], wardrobe: &ImportWardrobe) -> Result<()> {
    let body = chunks
        .iter_mut()
        .find(|chunk| chunk.tag == *b"STR#" && chunk.id == 200 && chunk.flags >> 8 == 0)
        .ok_or_else(|| unsupported("0x457fd7", "imported STR# 200 is absent"))?;
    let table = StringTable::parse(&body.data)
        .map_err(|error| Error::format("family import: 0x587b4b imported STR# 200", error))?;
    if table.format != Format::Localized
        || localized_entries(&table, Some(&body.data))
            .any(|(language, entry)| language != 1 || !entry.raw_comment.is_empty())
    {
        return Err(unsupported(
            "0x587b4b",
            "expected the clone's language-1 body table",
        ));
    }
    let mut rows: Vec<_> = table
        .entries
        .into_iter()
        .map(|entry| entry.raw_text)
        .collect();
    outfits::refresh(&mut rows, wardrobe)?;
    body.data = localized_strings(rows.iter().map(|text| (1, text.as_slice(), &[][..])))?;
    Ok(())
}

fn catalog_strings(
    template: &Iff,
    id: u16,
    language: u8,
    name_and_description: [&[u8]; 2],
) -> Result<Vec<u8>> {
    let mut languages = BTreeMap::<u8, Vec<(&[u8], &[u8])>>::new();
    let bytes = template.get(b"CTSS", id).map(|chunk| template.data(chunk));
    let table = bytes
        .map(StringTable::parse)
        .transpose()
        .map_err(|error| Error::format("family import: 0x4706f9 template CTSS", error))?;
    if let Some(table) = &table {
        if table.format != Format::Localized {
            return Err(unsupported(
                "0x470ca7",
                "non-localized template CTSS is not ported",
            ));
        }
        for (code, entry) in localized_entries(table, bytes) {
            let code = if code == 0 { language } else { code };
            if !(1..=20).contains(&code) {
                return Err(unsupported("0x470e43", "invalid template CTSS language"));
            }
            languages
                .entry(code)
                .or_default()
                .push((&entry.raw_text, &entry.raw_comment));
        }
    }
    let active = languages.entry(language).or_default();
    active.resize(active.len().max(2), (&[], &[]));
    for (entry, text) in active.iter_mut().zip(name_and_description) {
        entry.0 = text; // 0x470722 updates text and retains the destination comment.
    }
    // 0x4707be backfills missing translations from the English bank. The
    // writer keeps only languages with at least one genuine localized row.
    if let Some(english) = languages.get(&1).cloned() {
        for entries in languages.values_mut() {
            if let Some(missing) = english.get(entries.len()..) {
                entries.extend_from_slice(missing);
            }
        }
    }
    localized_strings(languages.into_iter().flat_map(|(language, rows)| {
        rows.into_iter()
            .map(move |(text, comment)| (language, text, comment))
    }))
}

fn localized_strings<'a>(
    rows: impl IntoIterator<Item = (u8, &'a [u8], &'a [u8])>,
) -> Result<Vec<u8>> {
    let mut data = vec![0xfd, 0xff, 0, 0];
    let mut count = 0;
    for (language, text, comment) in rows {
        data.push(language);
        data.extend(text);
        data.push(0);
        data.extend(comment);
        data.push(0);
        count += 1;
    }
    let count_word =
        i16::try_from(count).map_err(|_| unsupported("0x46fcdb", "too many character strings"))?;
    data[2..4].copy_from_slice(&count_word.to_le_bytes());
    // Both native writers allocate one extra byte per row (0x46fc9f,
    // 0x47090e). Fill the unused space with zeroes.
    data.resize(data.len() + count, 0);
    Ok(data)
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
            // 0x42920f to 0x42922e: the house is cleared first, so the
            // worth 0x4272c5 gives (from 0x410271) is the budget plus the
            // architecture value, and that becomes the budget before the
            // architecture value is zeroed.
            let mut family = (*family).clone();
            family.house = 0;
            family.budget = family.budget.wrapping_add(family.architecture_value);
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

/// Tries the neighborhood's file counter (+0x160), advancing on every
/// attempt. An occupied file (0x4722b8 returning -0x2b) retries without
/// cloning an OBJD or drawing its GUID. The attempt at 31999 resets the
/// counter to zero and ends the loop, even if the file was occupied.
pub fn reserve_character_file(next_file: &mut i32, files: &mut BTreeSet<i32>) -> Result<i32> {
    if !(0..FILE_LIMIT).contains(next_file) {
        return Err(unsupported(
            "0x4261a2",
            "character file counter outside 0..32000 is not ported",
        ));
    }
    loop {
        let candidate = *next_file;
        *next_file = (candidate + 1) % FILE_LIMIT;
        if files.insert(candidate) {
            return Ok(candidate);
        }
        if *next_file == 0 {
            return Err(unsupported(
                "0x4262c0",
                "no character file number available before the counter wraps",
            ));
        }
    }
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
) -> Result<[Vec<u8>; 2]> {
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
        return Ok([Vec::new(), Vec::new()]);
    };
    let location = format!("family import: 0x42945d CTSS for member {id}");
    let table = StringTable::parse(bytes).map_err(|error| Error::format(&location, error))?;
    let entries = raw_language_entries(&table, Some(bytes), language, fallback);
    if fallback && entries.len() < 2 {
        return Err(unsupported(
            "0x4294ad",
            format!("localized CTSS for member {id} lacks a name or description"),
        ));
    }
    Ok(std::array::from_fn(|index| {
        entries.get(index).copied().unwrap_or_default().to_vec()
    }))
}

fn language_entries(
    table: &StringTable,
    raw: Option<&[u8]>,
    language: u8,
    fallback: bool,
) -> Vec<String> {
    raw_language_entries(table, raw, language, fallback)
        .into_iter()
        .map(|text| decode(text, language))
        .collect()
}

fn raw_language_entries<'a>(
    table: &'a StringTable,
    raw: Option<&[u8]>,
    language: u8,
    fallback: bool,
) -> Vec<&'a [u8]> {
    if table.format != Format::Localized {
        return table
            .entries
            .iter()
            .map(|entry| entry.raw_text.as_slice())
            .collect();
    }
    let mut entries = Vec::new();
    let mut english = Vec::new();
    for (encoded, entry) in localized_entries(table, raw) {
        if encoded == 0 || encoded == language {
            entries.push(entry.raw_text.as_slice());
        }
        if encoded == 1 {
            english.push(entry.raw_text.as_slice());
        }
    }
    if entries.is_empty() && fallback {
        return english;
    }
    entries
}

fn localized_entries<'a, 'b>(
    table: &'a StringTable,
    raw: Option<&'b [u8]>,
) -> impl Iterator<Item = (u8, &'a sims_formats::strings::TextEntry)> + use<'a, 'b> {
    let mut offset = 4;
    table.entries.iter().map(move |entry| {
        // StringTable normalizes encoded language 0 to 1. The original
        // treats 0 as the active language (0x46fffa, 0x470e3d).
        let encoded = raw
            .and_then(|bytes| bytes.get(offset))
            .copied()
            .unwrap_or(entry.language);
        offset += entry.raw_text.len() + entry.raw_comment.len() + 3;
        (encoded, entry)
    })
}

fn import_lot(iff: &Iff, house: u32, mut pairs: Vec<(u32, u32)>) -> Result<ImportedLot> {
    let source_types = iff
        .get(b"objt", 0)
        .ok_or_else(|| unsupported("0x43dd76", format!("lot {house} has no objt #0")))?;
    let mut object_types = ObjectTypes::parse(iff.data(source_types))
        .map_err(|error| Error::format("family import: 0x43a516 objt #0", error))?;
    // The import reads at 0x429652 and appends at 0x429754.
    let table = if let Some(chunk) = iff.get(b"Gtab", 1) {
        let mut table = GuidTable::parse(iff.data(chunk))
            .map_err(|error| Error::format("family import: 0x41578e/0x4158a4 Gtab #1", error))?;
        table.pairs.append(&mut pairs);
        table
    } else {
        GuidTable { version: 0, pairs }
    };

    for entry in &mut object_types.entries {
        entry.guid = table.translate(entry.guid);
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
    chunks.entry(*b"Gtab").or_default().push(LotChunk {
        id: 1,
        flags: 0,
        label: String::new(),
        data: table.to_bytes(),
    });
    Ok(ImportedLot {
        house,
        chunks,
        object_types,
    })
}
