//! Original family membership, neighbor identities, traits and relationships.
//! Unknown fields are retained, including all 256 person words in newer saves.
use crate::Result;
use crate::lot::{header, padded_string};
use crate::reader::Reader;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub fn parse(bytes: &[u8]) -> Result<Self> {
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
        let count = r.u32()? as usize;
        r.count(count, 4)?;
        let mut members = Vec::with_capacity(count);
        for _ in 0..count {
            members.push(r.u32()?);
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub target: i32,
    pub values: Vec<i32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            let name = padded_string(&mut r)?;
            let unknown_zero = r.i32()?;
            let person_mode = r.i32()?;
            let mut person_data = Vec::new();
            if person_mode > 0 {
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
        Ok(Self {
            version,
            entries,
            tail,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryItem {
    pub kind: i32,
    pub guid: u32,
    pub count: u16,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        for (minimum, indices) in [
            (66, &[0, 1][..]),
            (68, &[2, 3][..]),
            (69, &[10][..]),
            (70, &[4, 5, 6, 7][..]),
            (73, &[8, 9][..]),
        ] {
            if layout_version >= minimum {
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
