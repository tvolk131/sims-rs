//! Original family membership, neighbor identities, traits and relationships.
//! Unknown fields are retained, including all 256 person words in newer saves.
use crate::Result;
use crate::lot::header;
use crate::reader::{Out, Reader, western};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Family {
    pub version: u32,
    pub house: i32,
    pub number: i32,
    pub budget: i32,
    pub architecture_value: i32,
    pub friends: i32,
    pub flags: u32,
    pub members: Vec<u32>,
    pub tail: Vec<u8>,
}
impl Family {
    /// 4104ce reads the six signed magic words only for FAMI versions >8,
    /// after the version8 secondary-house field (+13c). The complete tail
    /// stays preserved for other fields and future format work.
    pub fn magic_history(&self) -> Result<Option<[i16; 6]>> {
        if self.version <= 8 {
            return Ok(None);
        }
        let mut reader = Reader::new(&self.tail);
        reader.i32()?;
        let mut values = [0; 6];
        for value in &mut values {
            *value = reader.i16()?;
        }
        Ok(Some(values))
    }

    /// Secondary/vacation house number stored at `tail[0..4]` for FAMI
    /// versions >= 8 (`0x425dcc`, offset `+0x13c`). Returns `0` for older
    /// versions or truncated tails.
    #[must_use]
    pub fn secondary_house(&self) -> i32 {
        if self.version >= 8 && self.tail.len() >= 4 {
            i32::from_le_bytes(self.tail[0..4].try_into().unwrap())
        } else {
            0
        }
    }

    /// Writes the secondary house number into `tail[0..4]` for FAMI versions
    /// >= 8. Returns `false` if this record does not carry a secondary house.
    pub fn set_secondary_house(&mut self, house: i32) -> bool {
        if self.version < 8 || self.tail.len() < 4 {
            return false;
        }
        self.tail[0..4].copy_from_slice(&house.to_le_bytes());
        true
    }

    /// Upgrades a v7/v8 FAMI record to v9 (matching native `0x410696` which
    /// always serializes version 9), preserving any v8 secondary-house word at
    /// `tail[0..4]` and zero-initializing the six magic-history words at
    /// `tail[4..16]` when absent.
    pub fn upgrade_to_v9(&mut self) {
        if self.version >= 9 && self.tail.len() >= 16 {
            return;
        }
        let secondary = self.secondary_house();
        if self.version < 8 {
            let extra = std::mem::take(&mut self.tail);
            let mut new_tail = Vec::with_capacity(16 + extra.len());
            new_tail.extend_from_slice(&0i32.to_le_bytes());
            new_tail.extend_from_slice(&[0u8; 12]);
            new_tail.extend_from_slice(&extra);
            self.tail = new_tail;
        } else if self.version == 8 {
            let rest = if self.tail.len() > 4 {
                self.tail[4..].to_vec()
            } else {
                Vec::new()
            };
            let mut new_tail = Vec::with_capacity(16 + rest.len());
            new_tail.extend_from_slice(&secondary.to_le_bytes());
            new_tail.extend_from_slice(&[0u8; 12]);
            new_tail.extend_from_slice(&rest);
            self.tail = new_tail;
        } else if self.tail.len() < 16 {
            self.tail.resize(16, 0);
        }
        self.version = 9;
    }

    /// Writes the six signed magic words back into `self.tail[4..16]` for FAMI
    /// versions > 8, preserving the secondary-house word (`tail[0..4]`) and any
    /// bytes past offset 16. Returns `false` if the family version or tail does
    /// not carry a magic history.
    pub fn set_magic_history(&mut self, values: [i16; 6]) -> bool {
        if self.version <= 8 || self.tail.len() < 16 {
            return false;
        }
        for (index, value) in values.into_iter().enumerate() {
            let at = 4 + index * 2;
            self.tail[at..at + 2].copy_from_slice(&value.to_le_bytes());
        }
        true
    }
    /// Reads a FAMI record as if it lists members. The original's reader
    /// (0x4104ce) reads the member list only for a family whose ID is not
    /// zero (0x4105e7), so for family 0 use [`Family::parse_chunk`].
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Self::parse_members(bytes, true)
    }

    /// Reads FAMI chunk `id` as the original does (0x410655 sets the
    /// family's ID before 0x4104ce reads it): family 0 has no member list
    /// (0x4105e7), so its record goes straight on to the secondary house.
    pub fn parse_chunk(bytes: &[u8], id: u16) -> Result<Self> {
        Self::parse_members(bytes, id != 0)
    }

    /// The record [`Family::parse_chunk`] would read for chunk `id` from
    /// the bytes `self` was read from by [`Family::parse`]. Only family 0
    /// differs: its "member count" was really the secondary house.
    pub fn for_chunk(&self, id: u16) -> Result<Self> {
        if id != 0 {
            return Ok(self.clone());
        }
        let mut out = Out::header(self.version, b"IMAF");
        self.fixed_fields(&mut out);
        out.len32(self.members.len())?;
        for &member in &self.members {
            out.u32(member);
        }
        out.0.extend(&self.tail);
        Self::parse_chunk(&out.0, 0)
    }

    /// FAMI chunk `id` as 0x410696 saves it: always version 9 (0x4106eb),
    /// the member list only for a nonzero ID (0x4105e7), then the secondary
    /// house (+0x13c) and the six magic words (+0x14e). A record of an
    /// older version is upgraded as [`Family::upgrade_to_v9`] does, and tail
    /// bytes past the magic words are not saved.
    pub fn to_chunk(&self, id: u16) -> Result<Vec<u8>> {
        let mut family = self.clone();
        family.upgrade_to_v9();
        let mut out = Out::header(9, b"IMAF");
        family.fixed_fields(&mut out);
        if id != 0 {
            out.len32(family.members.len())?;
            for &member in &family.members {
                out.u32(member);
            }
        }
        out.0.extend(&family.tail[..16]);
        Ok(out.0)
    }

    fn fixed_fields(&self, out: &mut Out) {
        out.i32(self.house);
        out.i32(self.number);
        out.i32(self.budget);
        out.i32(self.architecture_value);
        out.i32(self.friends);
        out.u32(self.flags);
    }

    fn parse_members(bytes: &[u8], has_members: bool) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let version = header(&mut r, b"IMAF")?;
        if !(7..=9).contains(&version) {
            return r.fail(format!("unsupported FAMI version {version}"));
        }
        let house = r.i32()?;
        let number = r.i32()?;
        let budget = r.i32()?;
        let architecture_value = r.i32()?;
        let friends = r.i32()?;
        let flags = r.u32()?;
        let mut members = Vec::new();
        if has_members {
            let count = r.u32()? as usize;
            r.count(count, 4)?;
            members.reserve(count);
            for _ in 0..count {
                members.push(r.u32()?);
            }
        }
        let tail = r.take(r.remaining())?.to_vec();
        Ok(Self {
            version,
            house,
            number,
            budget,
            architecture_value,
            friends,
            flags,
            members,
            tail,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relationship {
    pub target: i32,
    pub values: Vec<i32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Neighbor {
    pub version: u32,
    pub version_extra: Option<i32>,
    pub name: String,
    pub unknown_zero: i32,
    pub person_mode: i32,
    pub person_data: Vec<i16>,
    pub id: i16,
    pub guid: u32,
    pub unknown_negative: i32,
    pub relationships: Vec<Relationship>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neighbors {
    pub version: u32,
    pub entries: Vec<Option<Neighbor>>,
    pub tail: Vec<u8>,
}
impl Neighbors {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Ok(Self::parse_with_padding(bytes)?.0)
    }

    /// Reads NBRS and also returns each name's padding byte, in record
    /// order, for a name that needed one. The writer leaves those bytes
    /// uninitialized (0x464cd7).
    pub fn parse_with_padding(bytes: &[u8]) -> Result<(Self, Vec<u8>)> {
        let mut padding = Vec::new();
        let mut r = Reader::new(bytes);
        let version = header(&mut r, b"SRBN")?;
        let count = r.u32()? as usize;
        r.count(count, 4)?;
        let mut entries = Vec::new();
        for _ in 0..count {
            let present = r.i32()?;
            if present == 0 {
                entries.push(None);
                continue;
            }
            if present != 1 {
                return r.fail(format!("unknown neighbor presence flag {present}"));
            }
            let version = r.u32()?;
            if version != 4 && version != 10 {
                return r.fail(format!("unsupported neighbor version {version}"));
            }
            let version_extra = if version == 10 { Some(r.i32()?) } else { None };
            let name = western(r.cstr()?);
            if r.pos & 1 != 0 {
                padding.push(r.u8()?);
            }
            let unknown_zero = r.i32()?;
            let person_mode = r.i32()?;
            let mut person_data = Vec::new();
            // 0x423e8e: any nonzero mode, negative too.
            if person_mode != 0 {
                for _ in 0..if version == 4 { 80 } else { 256 } {
                    person_data.push(r.i16()?);
                }
            }
            let id = r.i16()?;
            let guid = r.u32()?;
            let unknown_negative = r.i32()?;
            let rel_count = r.u32()? as usize;
            r.count(rel_count, 12)?;
            let mut relationships = Vec::new();
            for _ in 0..rel_count {
                if r.u32()? != 1 {
                    return r.fail("unsupported relationship key count");
                }
                let target = r.i32()?;
                let count = r.u32()? as usize;
                r.count(count, 4)?;
                let mut values = Vec::new();
                for _ in 0..count {
                    values.push(r.i32()?);
                }
                relationships.push(Relationship { target, values });
            }
            entries.push(Some(Neighbor {
                version,
                version_extra,
                name,
                unknown_zero,
                person_mode,
                person_data,
                id,
                guid,
                unknown_negative,
                relationships,
            }));
        }
        let tail = r.take(r.remaining())?.to_vec();
        Ok((
            Self {
                version,
                entries,
                tail,
            },
            padding,
        ))
    }

    /// NBRS as the neighborhood save writes it (0x43071e, serializer
    /// 0x430ad6) at stream version 0x49: the slot count, then for each slot
    /// a presence word and the record (0x423e08). Every record is written
    /// at the current layout whatever version it was read at: the words 10
    /// and 9 (0x423e1a, 0x423e34), the name, the word at +0x1c, the person
    /// mode, 256 person words when the mode is nonzero (0x423e99; a record
    /// read with 80 has the rest zero), the ID, the GUID, and the
    /// relationship map in its new form (0x4650ee writes -1, then
    /// 0x465543). `version` and `tail` are not written. Name padding
    /// bytes are zero; see [`Neighbors::to_bytes_with_padding`].
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.to_bytes_with_padding(&[])
    }

    /// [`Neighbors::to_bytes`] with the bytes the writer leaves
    /// uninitialized after the names that need padding, in record order
    /// (zero past the end of `padding`).
    pub fn to_bytes_with_padding(&self, padding: &[u8]) -> Result<Vec<u8>> {
        let mut pads = padding.iter().copied();
        let mut out = Out::header(0x49, b"SRBN");
        out.len32(self.entries.len())?;
        for entry in &self.entries {
            let Some(neighbor) = entry else {
                out.i32(0);
                continue;
            };
            out.i32(1);
            out.u32(10);
            out.i32(9);
            out.padded_string(&neighbor.name, || pads.next().unwrap_or(0))?;
            out.i32(neighbor.unknown_zero);
            out.i32(neighbor.person_mode);
            if neighbor.person_mode != 0 {
                if neighbor.person_data.len() > 256 {
                    return Err(crate::Error::new(out.0.len(), "more than 256 person words"));
                }
                for index in 0..256 {
                    out.i16(neighbor.person_data.get(index).copied().unwrap_or(0));
                }
            }
            out.i16(neighbor.id);
            out.u32(neighbor.guid);
            out.i32(-1);
            out.len32(neighbor.relationships.len())?;
            for relationship in &neighbor.relationships {
                out.u32(1);
                out.i32(relationship.target);
                out.len32(relationship.values.len())?;
                for &value in &relationship.values {
                    out.i32(value);
                }
            }
        }
        Ok(out.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryItem {
    pub kind: i32,
    pub guid: u32,
    pub count: u16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub neighbor: i16,
    pub items: Vec<InventoryItem>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neighborhood {
    pub version: u32,
    pub data: [i16; 16],
    /// Original vector slots, including null entries (0x42feb2).
    pub inventories: Vec<Option<Inventory>>,
    /// Native booleans at neighborhood offsets 0x1a0..=0x1aa. Names and
    /// consumers remain separate from this versioned storage reader.
    pub options: [bool; 11],
    /// Encoded option bytes in file order, before native boolean normalization.
    pub option_bytes: Vec<u8>,
    pub tail: Vec<u8>,
}
impl Neighborhood {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let version = header(&mut r, b"HBGN")?;
        // 0x4254f6 compares the stream's version as a signed x86 integer.
        let layout_version = version as i32;
        let mut data = [0; 16];
        let word_count = if layout_version < 20 { 1 } else { 16 };
        for word in &mut data[..word_count] {
            *word = r.i16()?;
        }
        let mut inventories = Vec::new();
        if layout_version >= 64 {
            let count = r.u32()? as usize;
            r.count(count, 4)?;
            for _ in 0..count {
                if r.i32()? == 0 {
                    inventories.push(None);
                    continue;
                }
                let neighbor = r.i16()?;
                let count = r.u32()? as usize;
                r.count(count, 10)?;
                let mut items = Vec::new();
                for _ in 0..count {
                    items.push(InventoryItem {
                        kind: r.i32()?,
                        guid: r.u32()?,
                        count: r.u16()?,
                    });
                }
                inventories.push(Some(Inventory { neighbor, items }));
            }
        }
        // The 0x1aa option appears before 0x1a4..0x1a9 in the stream.
        // Missing fields get the reader's version defaults, not zeroed storage.
        let mut options = [
            false, true, false, true, false, true, false, true, false, true, true,
        ];
        let mut option_bytes = Vec::new();
        for (minimum, indices) in OPTION_ORDER {
            if layout_version >= minimum as i32 {
                for &index in indices {
                    let value = r.u8()?;
                    options[index] = value != 0;
                    option_bytes.push(value);
                }
            }
        }
        let tail = r.take(r.remaining())?.to_vec();
        Ok(Self {
            version,
            data,
            inventories,
            options,
            option_bytes,
            tail,
        })
    }
}

/// The option bytes in stream order with the NGBH version that added them
/// (0x4254f6).
const OPTION_ORDER: [(u32, &[usize]); 5] = [
    (66, &[0, 1]),
    (68, &[2, 3]),
    (69, &[10]),
    (70, &[4, 5, 6, 7]),
    (73, &[8, 9]),
];

impl Neighborhood {
    /// NGBH #1 as the neighborhood save writes it (0x4252cc passes its
    /// version argument, 0x49, to the two-way serializer 0x4254f6): the
    /// sixteen data words, the inventory vector (0x42feb2: a presence word
    /// per slot, then the neighbor ID and the items), and the eleven
    /// option bytes in stream order. The serializer writes the bytes in
    /// memory, which are the bytes it read, so a byte from `option_bytes`
    /// that still agrees with `options` is written as read; otherwise the
    /// option is written as 0 or 1. `version` and `tail` are not written.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Out::header(0x49, b"HBGN");
        for &word in &self.data {
            out.i16(word);
        }
        out.len32(self.inventories.len())?;
        for slot in &self.inventories {
            let Some(inventory) = slot else {
                out.i32(0);
                continue;
            };
            out.i32(1);
            out.i16(inventory.neighbor);
            out.len32(inventory.items.len())?;
            for item in &inventory.items {
                out.i32(item.kind);
                out.u32(item.guid);
                out.u16(item.count);
            }
        }
        let mut read = self.option_bytes.iter();
        for (_, indices) in OPTION_ORDER {
            for &index in indices {
                let option = self.options[index];
                let byte = read
                    .next()
                    .copied()
                    .filter(|&byte| (byte != 0) == option)
                    .unwrap_or(u8::from(option));
                out.u8(byte);
            }
        }
        Ok(out.0)
    }
}
