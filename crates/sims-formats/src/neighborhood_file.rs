//! `Neighborhood.iff` as the neighborhood save writes it (`Sims.exe` 1.91
//! 0x4252cc).
//!
//! The save creates `NTemp.iff` (0x6c2608) as a new resource file
//! (0x465949) and adds, in order:
//!
//! - NGBH #1 (0x464dbd, serializer 0x4254f6, version 0x49);
//! - NBRS #1 (0x43071e, serializer 0x430ad6, the same version);
//! - for each family in the family vector (+0x154), FAMI, FAMs and FAMh
//!   under the family's ID (0x410696); a family whose ID is -1 is skipped
//!   (0x4106ab);
//! - TATT #1 (0x43e0be → 0x43f114).
//!
//! The close (0x46b6d2) then writes the resource map, and the file is moved
//! onto `Neighborhood.iff` (0x46b42a, 0x5b9988). Every resource has flags
//! 0x10 and an empty label.
//!
//! The map lists each type's resources in the order they were added, but
//! not the types themselves: every neighborhood save the original was
//! recorded making lists FAMh, FAMs, NGBH, TATT, NBRS, FAMI, while the
//! chunks are in the order above. The type vector is filled by push_back
//! (0x418d30 → 0x41a793) and written in its order (0x41ac25), so something
//! between reorders it; that step is untraced. [`TYPE_ORDER`] is the
//! recorded order.
//!
//! The save serializes the neighborhood as it is in memory. The writeback
//! before it, when a lot is loaded, is not part of this module: data word 2
//! becomes the lot's house number (0x425363), and the lot's family gets its
//! lot value (0x4253ff). The family vector's order is the caller's: the
//! original keeps it sorted by number, then ID, both descending (0x425209).

use crate::iff::Iff;
use crate::iff_write::{ChunkWrite, write_new_file};
use crate::neighborhood::{Family, Neighborhood, Neighbors};
use crate::reader::{Reader, encode_western, western};
use crate::tatt::Tatt;
use crate::{Error, Result};
use std::collections::BTreeMap;

/// The flags of every resource the save adds.
const FLAGS: u16 = 0x10;
/// The resource map's type order in every recorded neighborhood save.
pub const TYPE_ORDER: [[u8; 4]; 6] = [*b"FAMh", *b"FAMs", *b"NGBH", *b"TATT", *b"NBRS", *b"FAMI"];
/// The family ID the save skips (0x4106ab compares +0x10c with -1).
pub const NO_FAMILY: u16 = 0xffff;

/// One family's resources.
#[derive(Debug, Clone)]
pub struct FamilyRecord {
    /// The family's ID (+0x10c), which is each resource's ID.
    pub id: u16,
    pub family: Family,
    /// The family's name (+4), saved as FAMs.
    pub name: String,
    /// The FAMh payload, saved as it is. The save copies it from the
    /// family's `.FAM` file when the family still has one (+0x140) and
    /// otherwise writes the record it read from the neighborhood (+0x144).
    /// Both captured cases are byte copies. `None` saves no FAMh.
    pub history: Option<Vec<u8>>,
}

/// What the save writes.
#[derive(Debug, Clone)]
pub struct NeighborhoodFile {
    pub neighborhood: Neighborhood,
    pub neighbors: Neighbors,
    /// The family vector, in order.
    pub families: Vec<FamilyRecord>,
    pub type_attributes: Tatt,
}

/// Bytes the save leaves uninitialized. Any values are faithful; these
/// reproduce a recorded file. The default leaves them all zero.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Uninitialized {
    /// The last byte of each family's FAMs, by family ID. The string
    /// table's writer (0x470696) allocates four bytes per entry beyond its
    /// two strings (0x47090e, 0x470927) but writes three: the language is
    /// written as a word (0x4709af) and the value overwrites its high
    /// byte. So a one-entry table ends in one byte of heap.
    pub family_names: BTreeMap<u16, u8>,
    /// The padding byte after each NBRS name that needs one, in record
    /// order. The stream's string writer moves past it without writing it
    /// (0x464cd7).
    pub neighbor_names: Vec<u8>,
    /// The resource map's alignment bytes, in map order (0x464cd7). Each
    /// entry with an empty label has one.
    pub map_padding: Vec<u8>,
}

impl NeighborhoodFile {
    /// The file the save writes.
    pub fn to_bytes(&self, uninitialized: &Uninitialized) -> Result<Vec<u8>> {
        let mut payloads = vec![
            (*b"NGBH", 1, self.neighborhood.to_bytes()?),
            (
                *b"NBRS",
                1,
                self.neighbors
                    .to_bytes_with_padding(&uninitialized.neighbor_names)?,
            ),
        ];
        for record in &self.families {
            if record.id == NO_FAMILY {
                continue;
            }
            payloads.push((*b"FAMI", record.id, record.family.to_chunk(record.id)?));
            let stray = uninitialized
                .family_names
                .get(&record.id)
                .copied()
                .unwrap_or(0);
            payloads.push((*b"FAMs", record.id, family_name(&record.name, stray)?));
            if let Some(history) = &record.history {
                payloads.push((*b"FAMh", record.id, history.clone()));
            }
        }
        payloads.push((*b"TATT", 1, self.type_attributes.to_bytes()?));
        let chunks: Vec<_> = payloads
            .iter()
            .map(|(tag, id, payload)| ChunkWrite {
                tag: *tag,
                id: *id,
                flags: FLAGS,
                label: b"",
                payload,
            })
            .collect();
        let padding: Vec<[u8; 1]> = uninitialized.map_padding.iter().map(|&b| [b]).collect();
        let mut next = 0;
        write_new_file(&chunks, &TYPE_ORDER, |range| {
            let byte = match padding.get(next) {
                Some(byte) => &byte[..],
                None => &[0][..],
            };
            next += 1;
            (range.len() == 1).then_some(byte)
        })
    }

    /// Reads a file the save wrote, with the bytes it left uninitialized,
    /// so that [`NeighborhoodFile::to_bytes`] gives the same file back.
    /// Families are taken in FAMI order, which is the family vector's.
    pub fn parse(bytes: &[u8]) -> Result<(Self, Uninitialized)> {
        let iff = Iff::parse(bytes.to_vec())?;
        let required = |tag: &[u8; 4], id| {
            iff.get(tag, id).ok_or_else(|| {
                Error::new(0, format!("missing {} #{id}", String::from_utf8_lossy(tag)))
            })
        };
        let neighborhood = Neighborhood::parse(iff.data(required(b"NGBH", 1)?))?;
        let (neighbors, neighbor_names) =
            Neighbors::parse_with_padding(iff.data(required(b"NBRS", 1)?))?;
        let type_attributes = Tatt::parse(iff.data(required(b"TATT", 1)?))?;
        let mut uninitialized = Uninitialized {
            neighbor_names,
            ..Uninitialized::default()
        };
        let mut families = Vec::new();
        for chunk in iff.all(b"FAMI") {
            let id = chunk.id;
            let family = Family::parse_chunk(iff.data(chunk), id)?;
            let (name, stray) = parse_family_name(iff.data(required(b"FAMs", id)?))?;
            uninitialized.family_names.insert(id, stray);
            let history = iff.get(b"FAMh", id).map(|chunk| iff.data(chunk).to_vec());
            families.push(FamilyRecord {
                id,
                family,
                name,
                history,
            });
        }
        uninitialized.map_padding = map_padding(bytes)?;
        Ok((
            Self {
                neighborhood,
                neighbors,
                families,
                type_attributes,
            },
            uninitialized,
        ))
    }
}

/// FAMs as 0x410696 writes it: a version -3 string table (0x4705f8,
/// 0x470966) with the name as its one entry, in language 1 (0x471234 with
/// 1, written as the language's index plus one at 0x4709a2) and with an
/// empty comment, then the byte the writer leaves uninitialized.
pub fn family_name(name: &str, stray: u8) -> Result<Vec<u8>> {
    let encoded =
        encode_western(name).ok_or_else(|| Error::new(0, "family name is not Windows-1252"))?;
    let mut bytes = vec![0xfd, 0xff, 1, 0, 1];
    bytes.extend(encoded);
    bytes.extend([0, 0, stray]);
    Ok(bytes)
}

/// The name and the uninitialized byte of a FAMs that
/// [`family_name`] would write.
pub fn parse_family_name(bytes: &[u8]) -> Result<(String, u8)> {
    let mut r = Reader::new(bytes);
    if r.u16()? != 0xfffd || r.u16()? != 1 || r.u8()? != 1 {
        return r.fail("FAMs is not a one-entry language-1 table");
    }
    let name = western(r.cstr()?);
    if !r.cstr()?.is_empty() {
        return r.fail("FAMs entry has a comment");
    }
    let stray = r.u8()?;
    if r.remaining() != 0 {
        return r.fail("FAMs has more than one uninitialized byte");
    }
    Ok((name, stray))
}

/// The alignment bytes of the file's resource map, in map order.
fn map_padding(bytes: &[u8]) -> Result<Vec<u8>> {
    let header = bytes
        .get(60..64)
        .ok_or_else(|| Error::new(0, "missing IFF header"))?;
    let offset = u32::from_be_bytes(header.try_into().unwrap()) as usize;
    let size = bytes
        .get(offset + 4..offset + 8)
        .map(|size| u32::from_be_bytes(size.try_into().unwrap()) as usize)
        .filter(|&size| size >= 76 && offset + size <= bytes.len())
        .ok_or_else(|| Error::new(offset, "invalid resource map"))?;
    let map = &bytes[offset + 76..offset + size];
    let mut r = Reader::new(map);
    r.take(16)?;
    let types = r.u32()?;
    let mut padding = Vec::new();
    for _ in 0..types {
        r.u32()?;
        let entries = r.u32()?;
        for _ in 0..entries {
            r.take(8)?;
            r.cstr()?;
            if r.pos & 1 != 0 {
                padding.push(r.u8()?);
            }
        }
    }
    Ok(padding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::neighborhood::{Inventory, InventoryItem, Neighbor, Relationship};
    use crate::tatt::Entry;

    fn family(house: i32, number: i32, members: Vec<u32>) -> Family {
        Family {
            version: 9,
            house,
            number,
            budget: 20_000,
            architecture_value: 0,
            friends: 2,
            flags: 1,
            members,
            tail: vec![7, 0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0],
        }
    }

    fn neighbor(id: i16, guid: u32) -> Neighbor {
        Neighbor {
            version: 10,
            version_extra: Some(9),
            name: format!("Café {id}"),
            unknown_zero: 0,
            person_mode: 1,
            person_data: (0..256).map(|word| word as i16 - 100).collect(),
            id,
            guid,
            unknown_negative: -1,
            relationships: vec![Relationship {
                target: 2,
                values: vec![25, -40],
            }],
        }
    }

    fn sample() -> NeighborhoodFile {
        let neighborhood = Neighborhood {
            version: 0x49,
            data: std::array::from_fn(|index| index as i16 * 3 - 5),
            inventories: vec![
                None,
                Some(Inventory {
                    neighbor: 1,
                    items: vec![InventoryItem {
                        kind: 2,
                        guid: 0x1234_5678,
                        count: 3,
                    }],
                }),
            ],
            options: [
                true, false, true, true, false, false, true, false, true, true, false,
            ],
            option_bytes: Vec::new(),
            tail: Vec::new(),
        };
        let neighbors = Neighbors {
            version: 0x49,
            entries: vec![Some(neighbor(1, 0xdead_0001)), None, Some(neighbor(2, 7))],
            tail: Vec::new(),
        };
        let type_attributes = Tatt {
            version: 0,
            packed: true,
            entries: vec![
                Entry {
                    guid: 0x8000_0001,
                    values: vec![0, 1, -1, 300, -32768],
                    count_end: 0,
                    end: 0,
                },
                Entry {
                    guid: 5,
                    values: Vec::new(),
                    count_end: 0,
                    end: 0,
                },
            ],
            count_end: 0,
            consumed: 0,
        };
        NeighborhoodFile {
            neighborhood,
            neighbors,
            families: vec![
                FamilyRecord {
                    id: 3,
                    family: family(20, 9, vec![1, 2]),
                    name: "Pröbe".to_owned(),
                    history: Some(vec![
                        0, 0, 0, 0, 1, 0, 0, 0, b'h', b'M', b'A', b'F', 1, 0x80,
                    ]),
                },
                FamilyRecord {
                    id: NO_FAMILY,
                    family: family(0, 8, vec![9]),
                    name: "Gone".to_owned(),
                    history: None,
                },
                FamilyRecord {
                    id: 0,
                    family: family(0, 0, Vec::new()),
                    name: "Default".to_owned(),
                    history: None,
                },
            ],
            type_attributes,
        }
    }

    #[test]
    fn a_saved_file_reads_back_and_saves_the_same_bytes() {
        let file = sample();
        let uninitialized = Uninitialized {
            family_names: BTreeMap::from([(3, 0x95), (0, 0x27)]),
            // "Café 1" and "Café 2" are six bytes, so each needs a pad.
            neighbor_names: vec![0x5a, 0xa5],
            map_padding: vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x11, 0x22],
        };
        let bytes = file.to_bytes(&uninitialized).unwrap();
        let (read, recorded) = NeighborhoodFile::parse(&bytes).unwrap();
        assert_eq!(recorded, uninitialized);
        assert_eq!(read.to_bytes(&recorded).unwrap(), bytes);

        // The order and the resources of a fresh save.
        let iff = Iff::parse(bytes.clone()).unwrap();
        let order: Vec<_> = iff
            .chunks
            .iter()
            .map(|chunk| (chunk.kind(), chunk.id, chunk.flags, chunk.label.clone()))
            .collect();
        let expected: Vec<_> = [
            ("NGBH", 1),
            ("NBRS", 1),
            ("FAMI", 3),
            ("FAMs", 3),
            ("FAMh", 3),
            ("FAMI", 0),
            ("FAMs", 0),
            ("TATT", 1),
            ("rsmp", 0),
        ]
        .into_iter()
        .map(|(tag, id)| (tag.to_owned(), id, 0x10, String::new()))
        .collect();
        assert_eq!(order, expected);

        assert_eq!(read.families.len(), 2);
        assert_eq!(read.families[0].name, "Pröbe");
        assert_eq!(read.families[0].family.members, [1, 2]);
        assert_eq!(read.families[1].family.secondary_house(), 7);
        assert_eq!(
            read.families[1].family.magic_history().unwrap(),
            Some([1, 2, 3, 4, 5, 6])
        );
        assert_eq!(read.neighbors.entries.len(), 3);
        assert_eq!(read.neighbors.entries[0].as_ref().unwrap().name, "Café 1");
        assert_eq!(read.neighborhood.options, file.neighborhood.options);
        assert_eq!(read.neighborhood.data, file.neighborhood.data);
        assert_eq!(read.neighborhood.inventories, file.neighborhood.inventories);
        let values: Vec<_> = read
            .type_attributes
            .entries
            .iter()
            .map(|entry| (entry.guid, entry.values.clone()))
            .collect();
        assert_eq!(
            values,
            [(0x8000_0001, vec![0, 1, -1, 300, -32768]), (5, Vec::new())]
        );
    }

    #[test]
    fn family_zero_has_no_member_list() {
        let record = family(0, 0, vec![4]);
        let zero = record.to_chunk(0).unwrap();
        let other = record.to_chunk(5).unwrap();
        assert_eq!(other.len(), zero.len() + 8);
        assert_eq!(zero.len(), 52);
        assert_eq!(Family::parse_chunk(&zero, 0).unwrap().tail, record.tail);
        // Read without the ID, the secondary house (7) is taken as a count.
        assert!(Family::parse(&zero).is_err());

        // With no secondary house the ungated read succeeds, but wrongly:
        // the magic words are short. `for_chunk` recovers the record.
        let mut homeless = record.clone();
        homeless.set_secondary_house(0);
        let zero = homeless.to_chunk(0).unwrap();
        let misread = Family::parse(&zero).unwrap();
        assert_eq!(misread.tail.len(), 12);
        assert!(misread.magic_history().is_err());
        let read = misread.for_chunk(0).unwrap();
        assert_eq!(read.magic_history().unwrap(), Some([1, 2, 3, 4, 5, 6]));
        assert_eq!(read.to_chunk(0).unwrap(), zero);
        assert_eq!(misread.for_chunk(3).unwrap(), misread);
    }

    #[test]
    fn an_older_family_record_is_saved_at_version_nine() {
        let mut old = family(12, 3, vec![1]);
        old.version = 7;
        old.tail = Vec::new();
        let saved = Family::parse_chunk(&old.to_chunk(3).unwrap(), 3).unwrap();
        assert_eq!(saved.version, 9);
        assert_eq!(saved.house, 12);
        assert_eq!(saved.members, [1]);
        assert_eq!(saved.tail, [0; 16]);
    }

    #[test]
    fn family_names_are_one_entry_tables_with_a_stray_byte() {
        assert_eq!(
            family_name("Kat", 0x6c).unwrap(),
            [0xfd, 0xff, 1, 0, 1, b'K', b'a', b't', 0, 0, 0x6c]
        );
        assert_eq!(
            parse_family_name(&family_name("Zoë", 9).unwrap()).unwrap(),
            ("Zoë".to_owned(), 9)
        );
        assert!(family_name("\u{4e00}", 0).is_err());
        assert!(parse_family_name(&[0xfd, 0xff, 2, 0, 1, b'a', 0, 0, 0]).is_err());
    }

    #[test]
    fn neighbors_read_at_the_old_layout_are_saved_at_the_new_one() {
        let mut old = neighbor(4, 9);
        old.version = 4;
        old.version_extra = None;
        old.person_data = vec![5; 80];
        let neighbors = Neighbors {
            version: 0x30,
            entries: vec![Some(old)],
            tail: vec![1, 2],
        };
        let saved = Neighbors::parse(&neighbors.to_bytes().unwrap()).unwrap();
        assert_eq!(saved.version, 0x49);
        assert!(saved.tail.is_empty());
        let saved = saved.entries[0].clone().unwrap();
        assert_eq!((saved.version, saved.version_extra), (10, Some(9)));
        assert_eq!(saved.person_data.len(), 256);
        assert_eq!(&saved.person_data[..80], &[5; 80]);
        assert!(saved.person_data[80..].iter().all(|&word| word == 0));
    }

    #[test]
    fn option_bytes_are_saved_as_read_while_they_agree() {
        let mut neighborhood = sample().neighborhood;
        neighborhood.option_bytes = vec![2, 0, 7, 1, 1, 0, 0, 0, 0, 3, 9];
        neighborhood.options = [
            true, false, true, true, false, false, false, false, true, true, true,
        ];
        // Stream order is 0 1 2 3 10 4 5 6 7 8 9; option 4 now disagrees.
        neighborhood.options[4] = true;
        let bytes = neighborhood.to_bytes().unwrap();
        assert_eq!(
            &bytes[bytes.len() - 11..],
            [2, 0, 7, 1, 1, 1, 0, 0, 0, 3, 9]
        );
        let read = Neighborhood::parse(&bytes).unwrap();
        assert_eq!(read.options, neighborhood.options);
    }
}
