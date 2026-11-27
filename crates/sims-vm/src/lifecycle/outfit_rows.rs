//! The outfit rows a lot wrote into people's body strings, carried into
//! their character files as the original writes them.
//!
//! The original keeps one set of body strings for each person type: the
//! definition's string table (word 19, `STR#` 200 for a character),
//! loaded once (0x457f60). The outfit editor writes it:
//! - its load (0x409315) writes each group of rows it fills (formal, swim
//!   and pajamas, high fashion, winter) straight into the character file
//!   (0x587bbd);
//! - its commit with 1 (0x409492: Generic Sims Call 24, 29 and 30) writes
//!   the table into the file when it differs from the one loaded
//!   (0x587aec, 0x587bbd).
//!
//! So an edit outlives the lot: every lot the person is on after it reads
//! the rows from the file. The port keeps those rows in
//! [`World::made_outfits`](sims_sim::World::made_outfits), which goes with
//! the lot's save. Here they are written into the person's character file
//! in the neighborhood store when a lot is saved or a trip commits, and
//! dropped from the saved lot, so a lot saved earlier cannot hold rows
//! older than the file. A trip stages the writes before it boots the next
//! lot, so that lot reads them from the file too, and its travelers carry
//! an empty set for each person written (see [`OutfitWrites::carried`]).
//!
//! A person whose type has no character file of its own in the store (one
//! the install defines, a type made on a lot, a file kept in a FAR
//! archive, or a table two people share) keeps its rows in the lot as
//! before.

use crate::Program;
use crate::boot::save::Saved;
use crate::outfits::encode_cp1252;
use sims_formats::iff::Iff;
use sims_formats::iff_write::{ChunkWrite, write_chunk_with_residue};
use std::collections::{BTreeMap, BTreeSet};

/// Object definition word 9 (byte 0x12), the type, and a person's.
const TYPE: usize = 0x12;
const PERSON: u16 = 2;
/// Object definition bytes 0x1c..0x20, the GUID.
const GUID: usize = 0x1c;
/// Object definition word 19 (byte 0x26), the body strings table.
const BODY_STRINGS: usize = 0x26;
/// The resource flag the native writer sets on what it writes.
const CONVERTED: u16 = 0x10;
/// The bytes written where the native writer leaves its heap's (the map's
/// alignment bytes, a split free block's header), which nothing reads.
static ZEROS: [u8; 64] = [0; 64];

type Rows = BTreeMap<usize, String>;
/// A resource as the read-back guard compares it: id, flags, resource
/// flags, label and data.
type Resource = (u16, u16, u16, Vec<u8>, Vec<u8>);

/// Drops from a saved lot's `made_outfits` the rows of each person the
/// lot's load purges (`purged`, a deleted visitor's person) and of each
/// character deleted since the lot was saved: listed in
/// `deleted_characters`, or deleted after `lot_epoch` (a GUID a later
/// character may reuse). The rows were that character's strings, which
/// went with its file (the original keeps them in the definition,
/// 0x587bbd), so no later person of the GUID reads them.
pub(crate) fn drop_deleted_rows(
    made_outfits: &mut BTreeMap<u32, Rows>,
    purged: &BTreeSet<u32>,
    deleted_characters: &BTreeSet<u32>,
    deleted_at_epoch: &BTreeMap<u32, u64>,
    lot_epoch: u64,
) {
    made_outfits.retain(|guid, _| {
        !purged.contains(guid)
            && !deleted_characters.contains(guid)
            && !deleted_at_epoch
                .get(guid)
                .is_some_and(|&deleted| lot_epoch < deleted)
    });
}

/// The character files to replace, and the people whose rows they now
/// hold.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct OutfitWrites {
    pub files: Vec<(String, Vec<u8>)>,
    pub persisted: BTreeSet<u32>,
}

impl OutfitWrites {
    /// Drops the rows of the people whose files hold them from `saved`,
    /// and any empty set of rows, so the lot reads them from the file.
    pub(crate) fn strip(&self, saved: &mut Saved) {
        saved.retain_made_outfits(|guid, rows| !rows.is_empty() && !self.persisted.contains(&guid));
    }

    /// Makes the rows of the people whose files now hold them the rows
    /// `program` loaded, for a lot that stays live after the commit. The
    /// original's table is the file's, so a later save compares against
    /// what this one wrote, and an edit back to an earlier row is still an
    /// edit.
    pub(crate) fn rebase(&self, program: &mut Program, made_outfits: &BTreeMap<u32, Rows>) {
        for guid in &self.persisted {
            let (Some(rows), Some(profile)) = (
                made_outfits.get(guid),
                program.people.profiles.get_mut(guid),
            ) else {
                continue;
            };
            for (&idx, row) in rows {
                let value = (!row.is_empty()).then(|| row.clone());
                match idx {
                    1 => profile.body = value,
                    2 => profile.head = value,
                    12 => profile.gender = value,
                    13 => profile.age = value,
                    14 => profile.skin = value,
                    n => match value {
                        Some(row) => {
                            profile.suit_rows.insert(n, row);
                        }
                        None => {
                            profile.suit_rows.remove(&n);
                        }
                    },
                }
            }
        }
    }

    /// `files` as they are once these writes are in.
    pub(crate) fn staged(&self, files: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
        let mut out = files.clone();
        out.extend(self.files.iter().cloned());
        out
    }

    /// The rows a traveler snapshot carries for `guid`: none of its own
    /// for a person whose file now holds them. The set is empty rather
    /// than missing, so it replaces whatever rows the lot it arrives on
    /// held from before, and the person reads the file.
    pub(crate) fn carried(&self, guid: u32, rows: Option<Rows>) -> Option<Rows> {
        if self.persisted.contains(&guid) {
            Some(Rows::new())
        } else {
            rows
        }
    }

    /// These writes followed by `later`'s, which replace any file both
    /// write (`later` was made from the files these staged).
    pub(crate) fn then(mut self, later: OutfitWrites) -> OutfitWrites {
        for (name, bytes) in later.files {
            match self.files.iter_mut().find(|(have, _)| *have == name) {
                Some(file) => file.1 = bytes,
                None => self.files.push((name, bytes)),
            }
        }
        self.persisted.extend(later.persisted);
        self
    }
}

/// What a lot's people were loaded with, which says which of its rows
/// are edits and whose rows a store file can hold.
pub(crate) struct Loaded<'a> {
    /// The lot's linked code: its profiles are the rows the lot loaded,
    /// and its `characters` the types whose winning definition is in the
    /// neighborhood's `Characters` (the store's files).
    pub program: &'a Program,
    /// The lot's world: a type it made (`characters.made`) reads its own
    /// strings first and is left out.
    pub world: &'a sims_sim::World,
}

impl Loaded<'_> {
    fn writable(&self, guid: u32) -> bool {
        self.program.characters.contains(&guid) && self.world.characters.made.get(&guid).is_none()
    }

    /// Row `idx` of `guid` as the lot loaded it, as `read_character_row`
    /// reads a profile; empty where it has none.
    fn row(&self, guid: u32, idx: usize) -> &str {
        let Some(profile) = self.program.people.profiles.get(&guid) else {
            return "";
        };
        let row = match idx {
            1 => profile.body.as_ref(),
            2 => profile.head.as_ref(),
            12 => profile.gender.as_ref(),
            13 => profile.age.as_ref(),
            14 => profile.skin.as_ref(),
            n => profile.suit_rows.get(&n),
        };
        row.map_or("", String::as_str)
    }

    /// The rows of `guid` in `rows` the lot changed from what it loaded.
    fn edits(&self, guid: u32, rows: &Rows) -> Rows {
        rows.iter()
            .filter(|&(&idx, row)| row != self.row(guid, idx))
            .map(|(&idx, row)| (idx, row.clone()))
            .collect()
    }
}

/// The writes that carry `made_outfits` into the character files of
/// `character_files` (the neighborhood's, by filename), for the people
/// whose winning definition is a store file and who are not a type made
/// on a lot ([`Loaded`]).
///
/// Only the rows the lot changed from what it loaded are written: a row
/// the lot holds as it loaded it is no edit, and the file may hold a
/// newer one since. A person with no such row whose file is found counts
/// as persisted without a write.
///
/// Each person's rows go into the body strings table of the one file
/// whose person definition has its GUID: the table is decoded as the
/// person's string set loads it (US-English rows, padded to 35,
/// [`sims_content::decode_body_strings`]), the rows are set in it, and
/// it is written back as the native writer writes a string set
/// (language 1, no comments, one spare byte a row). A file that already
/// holds the rows is not written, but its person still counts as
/// persisted. A person found in no file or in more than one, whose table
/// another person definition names too, whose table does not decode, or
/// whose file would read back any other resource differently, is left
/// out.
pub(crate) fn outfit_writes(
    character_files: &BTreeMap<String, Vec<u8>>,
    made_outfits: &BTreeMap<u32, Rows>,
    loaded: &Loaded<'_>,
) -> OutfitWrites {
    let mut out = OutfitWrites::default();
    let wanted = |guid: u32| {
        made_outfits.get(&guid).is_some_and(|rows| !rows.is_empty()) && loaded.writable(guid)
    };
    if !made_outfits.keys().any(|&guid| wanted(guid)) {
        return out;
    }
    // Where each wanted person's definition is: (file, table), or None
    // when two definitions have its GUID (one of them may be a member of
    // an archive, which the mount reads, and which may be the one that
    // wins); and how many person definitions name each table.
    let mut found: BTreeMap<u32, Option<(&str, u16)>> = BTreeMap::new();
    let mut readers: BTreeMap<(&str, u16), usize> = BTreeMap::new();
    for (name, bytes) in character_files {
        let Ok(iff) = Iff::parse(bytes.clone()) else {
            for guid in archived_people(bytes) {
                if wanted(guid) {
                    found.insert(guid, None);
                }
            }
            continue;
        };
        for (guid, table) in person_tables(&iff) {
            *readers.entry((name.as_str(), table)).or_default() += 1;
            if wanted(guid) {
                found
                    .entry(guid)
                    .and_modify(|place| *place = None)
                    .or_insert(Some((name.as_str(), table)));
            }
        }
    }
    let mut by_file: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
    for (guid, place) in found {
        // One table two people read holds one set of rows: an edit for
        // one would dress the other.
        if let Some(place) = place
            && readers.get(&place) == Some(&1)
        {
            by_file.entry(place.0).or_default().push(guid);
        }
    }
    for (name, guids) in by_file {
        let mut bytes = character_files[name].clone();
        let mut changed = false;
        for guid in guids {
            let edits = loaded.edits(guid, &made_outfits[&guid]);
            if edits.is_empty() {
                out.persisted.insert(guid);
                continue;
            }
            match with_rows(&bytes, guid, &edits, name) {
                Some(Some(next)) => {
                    bytes = next;
                    changed = true;
                    out.persisted.insert(guid);
                }
                Some(None) => {
                    out.persisted.insert(guid);
                }
                None => {}
            }
        }
        if changed {
            out.files.push((name.to_owned(), bytes));
        }
    }
    out
}

/// The GUIDs of the person definitions (default variant) in `iff`, with
/// their body strings table.
/// The GUIDs of the person definitions in the members of the FAR archive
/// `bytes`; none when it is not one.
fn archived_people(bytes: &[u8]) -> Vec<u32> {
    use sims_formats::far;
    let len = bytes.len() as u64;
    let Some(offset) = bytes
        .get(..16)
        .and_then(|header| far::manifest_offset(header, len).ok())
    else {
        return Vec::new();
    };
    let Ok(entries) = far::directory(&bytes[offset as usize..], len) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|entry| entry.within_payload(offset))
        .filter_map(|entry| {
            let start = entry.offset as usize;
            let member = bytes.get(start..start + entry.stored_size as usize)?;
            Iff::parse(member.to_vec()).ok()
        })
        .flat_map(|iff| person_tables(&iff).into_iter().map(|(guid, _)| guid))
        .collect()
}

fn person_tables(iff: &Iff) -> Vec<(u32, u16)> {
    iff.all(b"OBJD")
        .filter(|chunk| chunk.flags >> 8 == 0)
        .filter_map(|chunk| {
            let data = iff.data(chunk);
            let kind = u16::from_le_bytes(data.get(TYPE..TYPE + 2)?.try_into().ok()?);
            let guid = u32::from_le_bytes(data.get(GUID..GUID + 4)?.try_into().ok()?);
            let table =
                u16::from_le_bytes(data.get(BODY_STRINGS..BODY_STRINGS + 2)?.try_into().ok()?);
            (kind == PERSON).then_some((guid, table))
        })
        .collect()
}

/// `bytes` with `rows` set in the body strings of the person `guid`:
/// `Some(Some(file))` when that changes the table, `Some(None)` when the
/// table already holds them, and `None` when the table cannot be read or
/// written, or the file written would read back anything else
/// differently.
fn with_rows(bytes: &[u8], guid: u32, rows: &Rows, name: &str) -> Option<Option<Vec<u8>>> {
    let iff = Iff::parse(bytes.to_vec()).ok()?;
    if iff.map_error().is_some() {
        return None;
    }
    let (_, table) = person_tables(&iff)
        .into_iter()
        .find(|&(found, _)| found == guid)?;
    let strings = iff.get(b"STR#", table)?;
    let label = format!("{name} STR# #{table}");
    let mut loaded =
        sims_content::decode_body_strings(iff.data(strings), iff.resource_flags(strings), &label)
            .ok()?;
    let before = loaded.clone();
    for (&idx, value) in rows {
        if idx >= loaded.len() {
            loaded.resize(idx + 1, String::new());
        }
        loaded[idx].clone_from(value);
    }
    if loaded == before {
        return Some(None);
    }
    let payload = string_set(&loaded)?;
    let label_bytes = iff.label_bytes(strings).to_vec();
    let written = write_chunk_with_residue(
        bytes,
        ChunkWrite {
            tag: *b"STR#",
            id: table,
            flags: (strings.flags & 0xff00) | CONVERTED,
            label: &label_bytes,
            payload: &payload,
        },
        |range| ZEROS.get(..range.len()),
    )
    .ok()?;
    let after = Iff::parse(written.clone()).ok()?;
    let reads_back = |tag: &[u8; 4]| {
        let resources = |file: &Iff| -> Vec<Resource> {
            file.all(tag)
                .filter(|chunk| !(tag == b"STR#" && chunk.id == table && chunk.flags >> 8 == 0))
                .map(|chunk| {
                    (
                        chunk.id,
                        chunk.flags,
                        file.resource_flags(chunk),
                        file.label_bytes(chunk).to_vec(),
                        file.data(chunk).to_vec(),
                    )
                })
                .collect()
        };
        resources(&iff) == resources(&after)
    };
    let tags: BTreeSet<[u8; 4]> = iff
        .chunks
        .iter()
        .chain(&after.chunks)
        .map(|c| c.tag)
        .collect();
    let table_reads = after
        .get(b"STR#", table)
        .is_some_and(|chunk| after.data(chunk) == payload);
    (after.map_error().is_none() && table_reads && tags.iter().all(reads_back))
        .then_some(Some(written))
}

/// A string set as the native writer writes it (0x46fd8b, as the
/// family import's 0x46fc9f): format -3, each row in language 1 with an
/// empty comment, and one spare zero byte a row.
fn string_set(rows: &[String]) -> Option<Vec<u8>> {
    let count = i16::try_from(rows.len()).ok()?;
    let mut data = vec![0xfd, 0xff];
    data.extend(count.to_le_bytes());
    for row in rows {
        data.push(1);
        data.extend(encode_cp1252(row));
        data.push(0);
        data.push(0);
    }
    data.resize(data.len() + rows.len(), 0);
    Some(data)
}

/// Row `idx` of the body strings of the person `guid` in the one file of
/// `character_files` that defines it, for tests.
#[cfg(test)]
pub(crate) fn stored_row(
    character_files: &BTreeMap<String, Vec<u8>>,
    guid: u32,
    idx: usize,
) -> Option<String> {
    let mut found = character_files.iter().filter_map(|(name, bytes)| {
        let iff = Iff::parse(bytes.clone()).ok()?;
        person_tables(&iff)
            .iter()
            .any(|&(found, _)| found == guid)
            .then_some((name, iff))
    });
    let (name, iff) = found.next()?;
    assert!(found.next().is_none(), "two files define 0x{guid:08x}");
    let definition = iff.all(b"OBJD").find(|chunk| {
        chunk.flags >> 8 == 0
            && iff.data(chunk).get(GUID..GUID + 4) == Some(&guid.to_le_bytes()[..])
    })?;
    let data = iff.data(definition);
    let table = u16::from_le_bytes(data.get(BODY_STRINGS..BODY_STRINGS + 2)?.try_into().ok()?);
    let strings = iff.get(b"STR#", table)?;
    let rows =
        sims_content::decode_body_strings(iff.data(strings), iff.resource_flags(strings), name)
            .ok()?;
    rows.get(idx).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_formats::iff_write::write_new_file;

    const GUID_A: u32 = 0x1234_5678;
    const GUID_B: u32 = 0x0abc_def0;

    fn definition(guid: u32, kind: u16) -> Vec<u8> {
        let mut data = vec![0u8; 0x50];
        data[TYPE..TYPE + 2].copy_from_slice(&kind.to_le_bytes());
        data[GUID..GUID + 4].copy_from_slice(&guid.to_le_bytes());
        data[BODY_STRINGS..BODY_STRINGS + 2].copy_from_slice(&200u16.to_le_bytes());
        data
    }

    /// A character file with one definition of `guid` and its body
    /// strings, rows 0 to 34 named after their places.
    fn character(guid: u32, kind: u16) -> Vec<u8> {
        let rows: Vec<String> = (0..35).map(|idx| format!("row {idx}")).collect();
        let objd = definition(guid, kind);
        let strings = string_set(&rows).unwrap();
        write_new_file(
            &[
                ChunkWrite {
                    tag: *b"OBJD",
                    id: 128,
                    flags: 0,
                    label: b"person",
                    payload: &objd,
                },
                ChunkWrite {
                    tag: *b"STR#",
                    id: 200,
                    flags: CONVERTED,
                    label: b"body strings",
                    payload: &strings,
                },
            ],
            &[],
            |range| ZEROS.get(..range.len()),
        )
        .unwrap()
    }

    /// A lot whose characters are A and B, loaded with no rows.
    fn program(characters: &[u32]) -> Program {
        Program {
            global: 0,
            semi_globals: BTreeMap::new(),
            libraries: BTreeMap::new(),
            careers: None,
            career_texts_end: false,
            labels_hold_no_token: false,
            zoning: BTreeMap::new(),
            animations: Default::default(),
            people: Default::default(),
            icon_frames: None,
            wardrobe: None,
            ordered_wardrobe: None,
            character_files: Default::default(),
            expansions: 0,
            characters: characters.iter().copied().collect(),
        }
    }

    fn writes_with(
        files: &BTreeMap<String, Vec<u8>>,
        rows: &BTreeMap<u32, BTreeMap<usize, String>>,
        program: &Program,
    ) -> OutfitWrites {
        let world = sims_sim::World::empty();
        outfit_writes(
            files,
            rows,
            &Loaded {
                program,
                world: &world,
            },
        )
    }

    fn writes(
        files: &BTreeMap<String, Vec<u8>>,
        rows: &BTreeMap<u32, BTreeMap<usize, String>>,
    ) -> OutfitWrites {
        writes_with(files, rows, &program(&[GUID_A, GUID_B]))
    }

    /// A FAR 1a archive holding `member` as `m.iff`.
    fn archive(member: &[u8]) -> Vec<u8> {
        let manifest = 16 + member.len() as u32;
        let mut far = b"FAR!byAZ".to_vec();
        far.extend(1u32.to_le_bytes());
        far.extend(manifest.to_le_bytes());
        far.extend(member);
        far.extend(1u32.to_le_bytes());
        far.extend((member.len() as u32).to_le_bytes());
        far.extend((member.len() as u32).to_le_bytes());
        far.extend(16u32.to_le_bytes());
        far.extend(5u32.to_le_bytes());
        far.extend(b"m.iff");
        assert_eq!(archived_people(&far), vec![GUID_A]);
        far
    }

    fn rows(file: &[u8]) -> Vec<String> {
        let iff = Iff::parse(file.to_vec()).unwrap();
        let strings = iff.get(b"STR#", 200).unwrap();
        sims_content::decode_body_strings(iff.data(strings), iff.resource_flags(strings), "test")
            .unwrap()
    }

    fn made(guid: u32, rows: &[(usize, &str)]) -> BTreeMap<u32, BTreeMap<usize, String>> {
        BTreeMap::from([(
            guid,
            rows.iter()
                .map(|&(idx, row)| (idx, row.to_owned()))
                .collect(),
        )])
    }

    #[test]
    fn rows_go_into_the_persons_file() {
        let files = BTreeMap::from([
            ("a.iff".to_owned(), character(GUID_A, PERSON)),
            ("b.iff".to_owned(), character(GUID_B, PERSON)),
        ]);
        let writes = writes(&files, &made(GUID_A, &[(30, "formal"), (1, "body")]));
        assert_eq!(writes.persisted, BTreeSet::from([GUID_A]));
        let [(name, file)] = &writes.files[..] else {
            panic!("{:?}", writes.files.len());
        };
        assert_eq!(name, "a.iff");
        let written = rows(file);
        assert_eq!(written.len(), 35);
        assert_eq!(written[1], "body");
        assert_eq!(written[30], "formal");
        assert_eq!(written[2], "row 2");
        // The definition and the table's label are kept, and the table
        // carries the flag the native writer sets.
        let iff = Iff::parse(file.clone()).unwrap();
        assert_eq!(person_tables(&iff), [(GUID_A, 200)]);
        let strings = iff.get(b"STR#", 200).unwrap();
        assert_eq!(iff.label_bytes(strings), b"body strings");
        assert_eq!(strings.flags & CONVERTED, CONVERTED);
    }

    #[test]
    fn rows_past_the_table_grow_it() {
        let files = BTreeMap::from([("a.iff".to_owned(), character(GUID_A, PERSON))]);
        let writes = writes(&files, &made(GUID_A, &[(40, "late")]));
        let written = rows(&writes.files[0].1);
        assert_eq!(written.len(), 41);
        assert_eq!(written[40], "late");
        assert_eq!(written[35], "");
    }

    #[test]
    fn a_file_that_holds_the_rows_is_not_written_but_counts() {
        let files = BTreeMap::from([("a.iff".to_owned(), character(GUID_A, PERSON))]);
        let equal = writes(&files, &made(GUID_A, &[(30, "row 30")]));
        assert!(equal.files.is_empty());
        assert_eq!(equal.persisted, BTreeSet::from([GUID_A]));
        // A second save of written rows changes nothing.
        let first = writes(&files, &made(GUID_A, &[(30, "formal")]));
        let files = BTreeMap::from([("a.iff".to_owned(), first.files[0].1.clone())]);
        let second = writes(&files, &made(GUID_A, &[(30, "formal")]));
        assert!(second.files.is_empty());
        assert_eq!(second.persisted, BTreeSet::from([GUID_A]));
    }

    #[test]
    fn people_without_one_file_of_their_own_keep_their_rows() {
        let wanted = made(GUID_A, &[(30, "formal")]);
        for files in [
            // Two files define the person.
            BTreeMap::from([
                ("a.iff".to_owned(), character(GUID_A, PERSON)),
                ("b.iff".to_owned(), character(GUID_A, PERSON)),
            ]),
            // An archive member defines it too, and may win.
            BTreeMap::from([
                ("a.iff".to_owned(), character(GUID_A, PERSON)),
                ("b.far".to_owned(), archive(&character(GUID_A, PERSON))),
            ]),
            // The definition is not a person's.
            BTreeMap::from([("a.iff".to_owned(), character(GUID_A, 4))]),
            // No file defines it, or a file does not parse.
            BTreeMap::from([("b.iff".to_owned(), character(GUID_B, PERSON))]),
            BTreeMap::from([("a.iff".to_owned(), b"not a file".to_vec())]),
        ] {
            assert_eq!(writes(&files, &wanted), OutfitWrites::default());
        }
    }

    #[test]
    fn a_table_two_people_share_and_made_types_are_left_out() {
        // Two person definitions in one file whose word 19 names the same
        // table.
        let mut both = character(GUID_A, PERSON);
        let second = definition(GUID_B, PERSON);
        both = sims_formats::iff_write::write_chunk_with_residue(
            &both,
            ChunkWrite {
                tag: *b"OBJD",
                id: 129,
                flags: 0,
                label: b"second",
                payload: &second,
            },
            |range| ZEROS.get(..range.len()),
        )
        .unwrap();
        let files = BTreeMap::from([("a.iff".to_owned(), both)]);
        let mut wanted = made(GUID_A, &[(30, "formal")]);
        wanted.extend(made(GUID_B, &[(30, "other")]));
        assert_eq!(writes(&files, &wanted), OutfitWrites::default());
        // With only one of them editing, the other still reads the table,
        // so it is left as it is.
        let one = writes(&files, &made(GUID_A, &[(30, "formal")]));
        assert_eq!(one, OutfitWrites::default());
        // A person whose winning definition is not a store file (one the
        // install or Downloads defines) is left out.
        let files = BTreeMap::from([("a.iff".to_owned(), character(GUID_A, PERSON))]);
        let elsewhere = writes_with(
            &files,
            &made(GUID_A, &[(30, "formal")]),
            &program(&[GUID_B]),
        );
        assert_eq!(elsewhere, OutfitWrites::default());
        // So is a type made on a lot.
        let mut world = sims_sim::World::empty();
        world.characters.made.insert(
            GUID_A,
            sims_sim::characters::Character {
                template: GUID_B,
                file: 1,
                strings: BTreeMap::new(),
            },
        );
        let made_type = outfit_writes(
            &files,
            &made(GUID_A, &[(30, "formal")]),
            &Loaded {
                program: &program(&[GUID_A]),
                world: &world,
            },
        );
        assert_eq!(made_type, OutfitWrites::default());
    }

    #[test]
    fn only_rows_the_lot_changed_are_written() {
        let files = BTreeMap::from([("a.iff".to_owned(), character(GUID_A, PERSON))]);
        let mut program = program(&[GUID_A]);
        let profile = sims_content::Profile {
            body: Some("loaded body".to_owned()),
            suit_rows: BTreeMap::from([(30, "loaded formal".to_owned())]),
            ..Default::default()
        };
        program.people.profiles.insert(GUID_A, profile);
        // Rows as the lot loaded them are no edit: nothing is written, and
        // the file still holds them.
        let same = made(GUID_A, &[(1, "loaded body"), (30, "loaded formal")]);
        let unchanged = writes_with(&files, &same, &program);
        assert!(unchanged.files.is_empty());
        assert_eq!(unchanged.persisted, BTreeSet::from([GUID_A]));
        // Only the changed row goes in; the others keep the file's.
        let mut edited = same.clone();
        edited
            .get_mut(&GUID_A)
            .unwrap()
            .insert(31, "swim".to_owned());
        let one = writes_with(&files, &edited, &program);
        let written = rows(&one.files[0].1);
        assert_eq!(written[31], "swim");
        assert_eq!((&written[1][..], &written[30][..]), ("row 1", "row 30"));
    }

    #[test]
    fn staged_carried_and_then() {
        let files = BTreeMap::from([
            ("a.iff".to_owned(), character(GUID_A, PERSON)),
            ("b.iff".to_owned(), character(GUID_B, PERSON)),
        ]);
        let first = writes(&files, &made(GUID_A, &[(30, "formal")]));
        let staged = first.staged(&files);
        assert_eq!(rows(&staged["a.iff"])[30], "formal");
        assert_eq!(staged["b.iff"], files["b.iff"]);
        // A later write from the staged files keeps the earlier one.
        let later = writes(&staged, &made(GUID_A, &[(31, "swim")]));
        let all = first.then(later);
        assert_eq!(all.files.len(), 1);
        let written = rows(&all.files[0].1);
        assert_eq!((&written[30][..], &written[31][..]), ("formal", "swim"));
        // A persisted person travels with an empty set, anyone else with
        // their own.
        let theirs = Some(BTreeMap::from([(1, "body".to_owned())]));
        assert_eq!(all.carried(GUID_A, theirs.clone()), Some(BTreeMap::new()));
        assert_eq!(all.carried(GUID_B, theirs.clone()), theirs);
    }
}
