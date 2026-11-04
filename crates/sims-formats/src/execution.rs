//! Data needed by the headless VM: GLOB links, constants, and slot records.
//! Format references and scope limitations are recorded in docs/architecture.md.
use crate::{
    Result,
    reader::{Reader, western},
};
use serde::{Deserialize, Serialize};

pub fn global_link(data: &[u8]) -> Result<String> {
    let mut r = Reader::new(data);
    let first = r.u8()?;
    let raw = if first < 48 {
        r.take(first as usize)?
    } else {
        // Both terminated and whole-payload filenames occur in reference readers.
        let n = data.iter().position(|b| *b == 0).unwrap_or(data.len());
        &data[..n]
    };
    if raw.is_empty() || raw.contains(&0) {
        return r.fail("empty or embedded-NUL GLOB name");
    }
    Ok(western(raw))
}

pub fn constants(data: &[u8]) -> Result<Vec<i16>> {
    let mut r = Reader::new(data);
    let n = r.u8()? as usize;
    r.u8()?; // Flags remain in the original IFF.
    r.count(n, 2)?;
    (0..n).map(|_| r.i16()).collect()
}

/// Event table metadata. Loading/overriding object callbacks is a separate engine service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectFunction {
    pub condition: u16,
    pub action: u16,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectFunctions {
    pub version: u32,
    pub entries: Vec<ObjectFunction>,
}
impl ObjectFunctions {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        r.u32()?;
        let version = r.u32()?;
        if version != 0 {
            return r.fail(format!("unsupported OBJf version {version}"));
        }
        if r.take(4)? != b"fJBO" {
            return r.fail("invalid OBJf magic");
        }
        let count = r.u32()? as usize;
        r.count(count, 4)?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push(ObjectFunction {
                condition: r.u16()?,
                action: r.u16()?,
            });
        }
        Ok(Self { version, entries })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Slot {
    pub kind: u16,
    pub offset: [f32; 3],
    pub standing: i32,
    pub sitting: i32,
    pub ground: i32,
    pub flags: u32,
    pub snap_target: i32,
    /// Proximities remain in the units of the stored SLOT version.
    pub proximity: [i32; 3],
    pub max_size: i32,
    pub unknown: i32,
    pub gradient: f32,
    pub height: i32,
    pub facing: i32,
    pub resolution: i32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Slots {
    pub version: u32,
    pub entries: Vec<Slot>,
}
impl Slots {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        r.u32()?;
        let version = r.u32()?;
        let stride = match version {
            4 | 5 => 34,
            6 => 54,
            7 => 58,
            8 => 62,
            9 => 66,
            10 => 70,
            _ => return r.fail(format!("unsupported SLOT version {version}")),
        };
        if r.take(4)? != b"TOLS" {
            return r.fail("invalid SLOT magic");
        }
        let count = r.u32()? as usize;
        r.count(count, stride)?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let kind = r.u16()?;
            let offset = [r.f32()?, r.f32()?, r.f32()?];
            let standing = r.i32()?;
            let sitting = r.i32()?;
            let ground = r.i32()?;
            let flags = r.u32()?;
            let snap_target = r.i32()?;
            let (proximity, max_size, unknown) = if version >= 6 {
                ([r.i32()?, r.i32()?, r.i32()?], r.i32()?, r.i32()?)
            } else {
                ([0; 3], 100, 0)
            };
            let gradient = if version >= 7 { r.f32()? } else { 0.0 };
            let height = if version >= 8 { r.i32()? } else { 0 };
            let facing = if version >= 9 { r.i32()? } else { -2 };
            let resolution = if version >= 10 { r.i32()? } else { 16 };
            if !gradient.is_finite() || offset.iter().any(|n| !n.is_finite()) {
                return r.fail("non-finite SLOT geometry");
            }
            entries.push(Slot {
                kind,
                offset,
                standing,
                sitting,
                ground,
                flags,
                snap_target,
                proximity,
                max_size,
                unknown,
                gradient,
                height,
                facing,
                resolution,
            });
        }
        Ok(Self { version, entries })
    }
    pub fn routing(&self, index: usize) -> Option<&Slot> {
        self.entries.iter().filter(|s| s.kind == 3).nth(index)
    }
}
