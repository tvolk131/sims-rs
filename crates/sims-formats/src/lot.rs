//! Saved-house chunk readers. This layer preserves serialized data; loading
//! live engine objects and deciding when to reset them belongs in sims-content.
use crate::reader::{Reader, western};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// Widened 14-byte wall cell; old saves widen its six byte-sized words on load.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct WallTile {
    pub segments: u8,
    pub flags: u8,
    pub styles: [u16; 2],
    pub patterns: [u16; 4],
}

pub(crate) fn header(r: &mut Reader<'_>, magic: &[u8; 4]) -> Result<u32> {
    r.u32()?;
    let version = r.u32()?;
    if r.take(4)? != magic {
        return r.fail("invalid saved-resource signature");
    }
    Ok(version)
}
pub(crate) fn padded_string(r: &mut Reader<'_>) -> Result<String> {
    let s = western(r.cstr()?);
    if r.pos & 1 != 0 {
        r.skip(1)?;
    }
    Ok(s)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Array {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub encoding: u32,
    /// Native order is column-major: (x * height + y) * stride.
    pub data: Vec<u8>,
}
impl Array {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        r.u32()?;
        let width = r.u32()? as usize;
        let height = r.u32()? as usize;
        let stride = r.u32()? as usize;
        let encoding = r.u32()?;
        // Explicit allocation cap for untrusted custom lots, not a world-size rule.
        let len = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(stride))
            .filter(|&n| n > 0 && n <= 64 * 1024 * 1024)
            .ok_or_else(|| Error::new(4, "invalid/oversized array dimensions"))?;
        let mut data = Vec::with_capacity(len);
        if encoding == 0 {
            data.extend_from_slice(r.take(len)?);
        } else if encoding != 1 {
            return r.fail(format!("unsupported array encoding {encoding}"));
        }
        while data.len() < len {
            let command = r.u16()?;
            let count = usize::from(command & 0x7fff);
            if count == 0 {
                return r.fail("zero-length array run");
            }
            if count > len - data.len() {
                return r.fail("array run exceeds dimensions");
            }
            if command & 0x8000 != 0 {
                let value = r.u8()?;
                r.skip(1)?;
                data.resize(data.len() + count, value);
            } else {
                data.extend_from_slice(r.take(count)?);
                if count & 1 != 0 {
                    r.skip(1)?;
                }
            }
        }
        if r.remaining() != 0 {
            return r.fail("trailing array data");
        }
        Ok(Self {
            width,
            height,
            stride,
            encoding,
            data,
        })
    }
    pub fn cell(&self, x: usize, y: usize) -> Option<&[u8]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let start = (x * self.height + y) * self.stride;
        self.data.get(start..start + self.stride)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn array_header(width: u32, height: u32, stride: u32, encoding: u32) -> Vec<u8> {
        [0, width, height, stride, encoding]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect()
    }
    #[test]
    fn rle_is_bytewise_even_for_walls_and_truncation_never_fills_zeros() {
        let mut input = array_header(2, 1, 8, 1);
        input.extend_from_slice(&[0x09, 0x80, 0xab, 0, 7, 0, 1, 2, 3, 4, 5, 6, 7, 0]);
        let array = Array::parse(&input).unwrap();
        assert_eq!(array.cell(0, 0).unwrap(), &[0xab; 8]);
        assert_eq!(array.cell(1, 0).unwrap(), &[0xab, 1, 2, 3, 4, 5, 6, 7]);
        for end in 0..input.len() {
            assert!(Array::parse(&input[..end]).is_err(), "truncation {end}");
        }
        input[20] = 17;
        assert!(Array::parse(&input).is_err());
        assert!(Array::parse(&array_header(u32::MAX, u32::MAX, 14, 1)).is_err());
    }
    #[test]
    fn rectangular_array_addresses_columns_and_rejects_trailing_data() {
        let mut input = array_header(2, 3, 1, 0);
        input.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let array = Array::parse(&input).unwrap();
        assert_eq!(array.cell(1, 1).unwrap(), &[5]);
        assert!(array.cell(2, 0).is_none());
        input.push(0);
        assert!(Array::parse(&input).is_err());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectType {
    pub guid: u32,
    /// Two native 32-bit values compared during resource linking to decide
    /// whether saved object execution needs resetting (0x0043a516).
    pub init_signature: u32,
    pub main_signature: u32,
    pub id: u16,
    pub kind: u16,
    pub name: String,
    pub extra: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectTypes {
    pub version: u32,
    pub entries: Vec<ObjectType>,
}
impl ObjectTypes {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let version = header(&mut r, b"tjbo")?;
        if !(2..=3).contains(&version) {
            return r.fail(format!("unsupported objt version {version}"));
        }
        let mut entries = Vec::new();
        while r.remaining() != 0 {
            let guid = r.u32()?;
            if guid == 0 {
                continue;
            }
            let init_signature = r.u32()?;
            let main_signature = r.u32()?;
            let id = r.u16()?;
            let kind = r.u16()?;
            let name = padded_string(&mut r)?;
            let extra = if version == 3 { Some(r.u32()?) } else { None };
            if entries.iter().any(|e: &ObjectType| e.id == id) {
                return r.fail("duplicate object type id");
            }
            entries.push(ObjectType {
                guid,
                init_signature,
                main_signature,
                id,
                kind,
                name,
                extra,
            });
        }
        Ok(Self { version, entries })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Simulator {
    pub version: u32,
    pub globals: Vec<i16>,
    pub ticks: u32,
    /// Native lot +0x1a8, sign-extended by 46cfb3. Zero starts a new registry
    /// pass; a false object Update can preserve a one-based slot here.
    pub scheduler_cursor: i16,
    pub funds: i32,
    pub lot_rng: u32,
    pub global_rng: u32,
    pub pending_funds: i32,
    pub lot_value: i32,
    pub objects_value: i32,
    pub architecture_value: i32,
    pub budget_days: Vec<Option<[i32; 8]>>,
    pub tail: Vec<u8>,
}
impl Simulator {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let version = header(&mut r, b"IMIS")?;
        // 0x33 and 0x38 occur only in the base game's template houses. Their
        // fields through the budget read plausibly with this layout; 0x33's
        // unread tail is 24 bytes shorter. Not yet checked against the
        // original loader.
        if !(0x33..=0x49).contains(&version) {
            return r.fail(format!("unsupported SIMI version {version}"));
        }
        let mut globals = Vec::new();
        for _ in 0..if version >= 0x41 { 64 } else { 32 } {
            globals.push(r.i16()?);
        }
        let ticks = r.u32()?;
        let scheduler_cursor = r.i16()?;
        let funds = r.i32()?;
        let lot_rng = r.u32()?;
        let global_rng = r.u32()?;
        let pending_funds = r.i32()?;
        let lot_value = r.i32()?;
        let objects_value = r.i32()?;
        let architecture_value = r.i32()?;
        let mut budget_days = Vec::new();
        for _ in 0..6 {
            let valid = r.i32()?;
            if valid == 0 {
                budget_days.push(None);
            } else {
                let mut day = [0; 8];
                for value in &mut day {
                    *value = r.i32()?;
                }
                budget_days.push(Some(day));
            }
        }
        let tail = r.take(r.remaining())?.to_vec();
        Ok(Self {
            version,
            globals,
            ticks,
            scheduler_cursor,
            funds,
            lot_rng,
            global_rng,
            pending_funds,
            lot_value,
            objects_value,
            architecture_value,
            budget_days,
            tail,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HouseView {
    pub version: u32,
    pub fields: [i32; 4],
    pub camera_direction: i16,
    pub unknown_shorts: [i16; 2],
    pub unknown_word: u32,
    pub roof: String,
    pub tail: Vec<u8>,
}
impl HouseView {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let version = header(&mut r, b"SUOH")?;
        let mut fields = [0; 4];
        for f in &mut fields {
            *f = r.i32()?;
        }
        let camera_direction = r.i16()?;
        let unknown_shorts = [r.i16()?, r.i16()?];
        let unknown_word = r.u32()?;
        let roof = western(r.cstr()?);
        let tail = r.take(r.remaining())?.to_vec();
        Ok(Self {
            version,
            fields,
            camera_direction,
            unknown_shorts,
            unknown_word,
            roof,
            tail,
        })
    }
}

/// HOUS 1 is the House model, distinct from the HOUS 0 viewer resource.
/// The native 0x00416568 stores an aligned string then six pairs of i32s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct House {
    pub version: u32,
    pub name: String,
    pub pairs: [[i32; 2]; 6],
    pub tail: Vec<u8>,
}
impl House {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let version = header(&mut r, b"SUOH")?;
        if !(0x26..=0x49).contains(&version) {
            return r.fail(format!("unsupported house model version {version}"));
        }
        let name = padded_string(&mut r)?;
        let mut pairs = [[0; 2]; 6];
        for pair in &mut pairs {
            pair[0] = r.i32()?;
            pair[1] = r.i32()?;
        }
        let tail = r.take(r.remaining())?.to_vec();
        Ok(Self {
            version,
            name,
            pairs,
            tail,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceEntry {
    /// Saved lot ID; resource catalogs assign different runtime IDs on load.
    pub id: u16,
    /// Material number inside the named file, independent of zoom offsets.
    pub local: u16,
    /// Native nearest-material fallback key (category and RGB channels).
    pub fallback: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceMapping {
    pub name: String,
    pub entries: Vec<SurfaceEntry>,
}
pub fn surface_map(bytes: &[u8], chunk_id: u16, floor: bool) -> Result<Vec<SurfaceMapping>> {
    let mut r = Reader::new(bytes);
    let version = header(&mut r, if floor { b"mRLF" } else { b"mLAW" })?;
    if version != 0 || chunk_id > 1 {
        return r.fail("unsupported surface map version/id");
    }
    let count = r.u32()? as usize;
    // Empty names and empty per-file vectors are both serializable. Bound
    // allocations by remaining bytes without inventing a nonempty record.
    r.count(count, 6)?;
    let mut entries = Vec::new();
    for _ in 0..count {
        let name = padded_string(&mut r)?;
        let n = r.u32()? as usize;
        r.count(n, if chunk_id == 0 { 6 } else { 8 })?;
        let mut materials = Vec::with_capacity(n);
        for _ in 0..n {
            // 0x4a34a4 reads byte IDs for legacy chunk 0. 0x4a32a3
            // reads both IDs as words for chunk 1; neither is a flags byte.
            let (id, local) = if chunk_id == 0 {
                (u16::from(r.u8()?), u16::from(r.u8()?))
            } else {
                (r.u16()?, r.u16()?)
            };
            materials.push(SurfaceEntry {
                id,
                local,
                fallback: r.u32()?,
            });
        }
        entries.push(SurfaceMapping {
            name,
            entries: materials,
        });
    }
    if r.remaining() != 0 {
        return r.fail("trailing surface map bytes");
    }
    Ok(entries)
}

#[cfg(test)]
mod simulator_tests {
    use super::*;

    fn simulator(version: u32) -> Vec<u8> {
        let mut b = vec![0; 4];
        b.extend(version.to_le_bytes());
        b.extend(b"IMIS");
        let globals = if version >= 0x41 { 64 } else { 32 };
        for g in 0..globals {
            b.extend((g as i16).to_le_bytes());
        }
        b.extend(9175u32.to_le_bytes());
        b.extend(0i16.to_le_bytes());
        for n in [94500u32, 0xea8b_32d0, 0xeb1b, 0, 4500, 0, 0] {
            b.extend(n.to_le_bytes());
        }
        b.extend(1i32.to_le_bytes());
        for n in 1..=8i32 {
            b.extend(n.to_le_bytes());
        }
        for _ in 0..5 {
            b.extend(0i32.to_le_bytes());
        }
        b.extend([7; 3]);
        b
    }

    #[test]
    fn reads_template_house_versions() {
        for version in [0x33, 0x38, 0x3e, 0x41, 0x49] {
            let sim = Simulator::parse(&simulator(version)).unwrap();
            assert_eq!(sim.globals.len(), if version >= 0x41 { 64 } else { 32 });
            assert_eq!(sim.globals[31], 31);
            assert_eq!(sim.ticks, 9175);
            assert_eq!(sim.funds, 94500);
            assert_eq!(sim.lot_rng, 0xea8b_32d0);
            assert_eq!(sim.lot_value, 4500);
            assert_eq!(sim.budget_days[0], Some([1, 2, 3, 4, 5, 6, 7, 8]));
            assert!(sim.budget_days[1..].iter().all(Option::is_none));
            assert_eq!(sim.tail, [7; 3]);
        }
        for version in [0x32, 0x4a] {
            assert!(Simulator::parse(&simulator(version)).is_err());
        }
    }
}
