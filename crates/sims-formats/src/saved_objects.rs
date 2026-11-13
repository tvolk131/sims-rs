//! Original ObjM state. Parsing a saved thread does not promise that its native
//! primitive continuation can yet be resumed by the Rust VM.
use crate::{
    Error, Result,
    field::Fields,
    lot::{ObjectTypes, header},
    reader::Reader,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub stack_object: i16,
    pub tree: u16,
    pub node: u16,
    pub parameters: Vec<i16>,
    pub locals: Vec<i16>,
    pub primitive_state: i32,
    pub code_owner_type: i16,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub present: i32,
    pub target: i32,
    pub values: Vec<i32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slot {
    pub unknown: i16,
    pub object: i16,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Multipart {
    pub offset: [i32; 3],
    pub group: [i32; 3],
    pub parent: i16,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interaction {
    pub uid: i32,
    pub caller: i16,
    pub target: i16,
    pub icon: i16,
    pub table_index: i32,
    pub args: [i16; 4],
    pub priority: i32,
    pub action: u16,
    pub attenuation: f32,
    pub flags: u32,
}
impl Interaction {
    fn read(f: &mut Fields<'_>) -> Result<Self> {
        if f.i32()? != 0 {
            return Err(Error::new(f.byte_position(), "unknown interaction header"));
        }
        Ok(Self {
            uid: f.i32()?,
            caller: f.i16()?,
            target: f.i16()?,
            icon: f.i16()?,
            table_index: f.i32()?,
            args: [f.i16()?, f.i16()?, f.i16()?, f.i16()?],
            priority: f.i32()?,
            action: f.u16()?,
            attenuation: f.f32()?,
            flags: f.u32()?,
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Accessory {
    pub name: String,
    pub binding: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectUse {
    pub target: i16,
    pub stack_depth: i32,
    pub unknown: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotiveDelta {
    pub motive: i32,
    pub delta: f32,
    pub stop_at: f32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    pub animation_events: i32,
    pub engaged: i32,
    /// Body, body texture, two unknown strings, left hand/texture, right
    /// hand/texture, head/texture, in native save order.
    pub appearance: [String; 10],
    pub accessories: Vec<Accessory>,
    /// Foreground, carry, base animation serialized descriptors.
    pub animations: [String; 3],
    pub routing_state: i32,
    /// 0x5918ba: head basis heading, target heading, accumulated turn, target
    /// point XYZ, then animator placement compensation (two coordinates and
    /// heading, starting at +0x8c).
    /// Disabled tracking can retain stale target-point values.
    pub floats: [f32; 9],
    pub previous_motives: [f32; 16],
    pub motives: [f32; 16],
    pub data: Vec<i16>,
    pub routing_frames: i32,
    pub active: Interaction,
    pub last: Interaction,
    pub queue: Vec<Interaction>,
    pub object_uses: Vec<ObjectUse>,
    pub motive_deltas: Vec<MotiveDelta>,
}
impl Person {
    fn read(f: &mut Fields<'_>, version: u32) -> Result<Self> {
        let animation_events = f.i32()?;
        let engaged = f.i32()?;
        let mut appearance: [String; 10] = Default::default();
        for s in &mut appearance {
            *s = f.string()?;
        }
        let count = f.count(16)?;
        let mut accessories = Vec::new();
        for _ in 0..count {
            accessories.push(Accessory {
                name: f.string()?,
                binding: f.string()?,
            });
        }
        let animations = [f.string()?, f.string()?, f.string()?];
        let routing_state = f.i32()?;
        let mut floats = [0.; 9];
        for v in &mut floats {
            *v = f.f32()?;
        }
        // 0x0045cf2d reads current motives (+0x78c), then the previous
        // update values (+0x7cc). The external format reference reverses them.
        let mut motives = [0.; 16];
        for v in &mut motives {
            *v = f.f32()?;
        }
        let mut previous_motives = [0.; 16];
        for v in &mut previous_motives {
            *v = f.f32()?;
        }
        let data = shorts(f, if version >= 0x47 { 256 } else { 80 })?;
        let routing_frames = f.i32()?;
        let active = Interaction::read(f)?;
        let last = Interaction::read(f)?;
        let count = f.count(16)?;
        let mut queue = Vec::new();
        for _ in 0..count {
            queue.push(Interaction::read(f)?);
        }
        let count = f.count(3)?;
        let mut object_uses = Vec::new();
        for _ in 0..count {
            object_uses.push(ObjectUse {
                target: f.i16()?,
                stack_depth: f.i32()?,
                unknown: f.u8()?,
            });
        }
        let count = f.count(3)?;
        let mut motive_deltas = Vec::new();
        for _ in 0..count {
            motive_deltas.push(MotiveDelta {
                motive: f.i32()?,
                delta: f.f32()?,
                stop_at: f.f32()?,
            });
        }
        Ok(Self {
            animation_events,
            engaged,
            appearance,
            accessories,
            animations,
            routing_state,
            floats,
            previous_motives,
            motives,
            data,
            routing_frames,
            active,
            last,
            queue,
            object_uses,
            motive_deltas,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub id: u16,
    pub type_id: u16,
    pub guid: u32,
    /// Native rectangle order: minimum Y, minimum X, maximum Y, maximum X.
    pub footprint: [i32; 4],
    pub position: [i32; 3],
    pub unknown_short: i16,
    pub attributes: Vec<i16>,
    pub temps: Vec<i16>,
    pub data: Vec<i16>,
    /// The native serializer writes one obsolete zero word after all 72 data words.
    pub obsolete_word: i16,
    /// Native frame-arena allocation size, not execution flags (0x00474fc3).
    pub stack_capacity_bytes: i32,
    pub stack: Vec<Frame>,
    pub relationship_flag: i32,
    pub relationships: Vec<Relationship>,
    /// First entry is the next object on this tile; remaining entries are containers.
    pub slots: Vec<Slot>,
    pub dynamic_sprites: Vec<i16>,
    pub multipart: Option<Multipart>,
    pub portal: Option<Vec<f32>>,
    pub person: Option<Person>,
    /// Native registry +0x24 entry: signed countdown low16, scheduler flags high16.
    /// None when the record ends first, as every `*Placeholder` record in the
    /// shipped and played houses does (0 to 7 padding bits remain, against
    /// 24 or more for every other object). Why the original omits it is not
    /// known yet; see `docs/decisions.md`.
    pub registry_word: Option<i32>,
    pub consumed_bits: usize,
    /// Retain raw bytes, including unrecognized trailing fields and bit padding.
    pub record: Vec<u8>,
}
fn shorts(f: &mut Fields<'_>, count: usize) -> Result<Vec<i16>> {
    if count > f.remaining_bits() {
        return Err(Error::new(
            f.byte_position(),
            "short array exceeds field stream",
        ));
    }
    (0..count).map(|_| f.i16()).collect()
}
fn short_count(f: &mut Fields<'_>) -> Result<usize> {
    let count = f.i16()?;
    if count < 0 || count as usize > f.remaining_bits() {
        return Err(Error::new(f.byte_position(), "invalid short field count"));
    }
    Ok(count as usize)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Objects {
    pub version: u32,
    /// Native saved registration order, distinct from numeric ID order. Load
    /// appends to the registry list (0x4434cd); post-load walks that list.
    pub order: Vec<u16>,
    pub types_by_id: BTreeMap<u16, u16>,
    pub instances: BTreeMap<u16, Instance>,
    pub tail: Vec<u8>,
}
impl Objects {
    /// Multipart status must come from the linked original OBJD, rather than
    /// guessing from an empty label. Missing resources are a recoverable import error.
    pub fn parse(
        bytes: &[u8],
        types: &ObjectTypes,
        multipart: impl Fn(u32) -> Option<bool>,
    ) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let version = header(&mut r, b"MjbO")?;
        if !(0x3e..=0x49).contains(&version) {
            return r.fail(format!("unsupported ObjM version {version}"));
        }
        if r.u8()? != 1 {
            return r.fail("unsupported ObjM compression");
        }
        let mut f = Fields::new(&bytes[r.pos..], r.pos);
        let mut types_by_id = BTreeMap::new();
        let mut order = Vec::new();
        loop {
            let id = f.u16()?;
            if id == 0 {
                break;
            }
            let kind = f.u16()?;
            if types_by_id.insert(id, kind).is_some() {
                return r.fail("duplicate saved object ID");
            }
            if id > i16::MAX as u16 {
                return r.fail("saved object ID exceeds the native signed registry");
            }
            order.push(id);
        }
        r.skip(f.byte_position())?;
        let mut instances = BTreeMap::new();
        for expected_id in &order {
            let end = (r.u32()? as usize)
                .checked_add(12)
                .ok_or_else(|| Error::new(r.pos, "object offset overflow"))?;
            if end <= r.pos || end > bytes.len() {
                return r.fail("saved object record outside chunk");
            }
            let origin = r.pos;
            let record = r.take(end - origin)?;
            let mut f = Fields::new(record, origin);
            let footprint = [f.i32()?, f.i32()?, f.i32()?, f.i32()?];
            let position = [f.i32()?, f.i32()?, f.i32()?];
            let unknown_short = f.i16()?;
            let n = short_count(&mut f)?;
            let attributes = shorts(&mut f, n)?;
            let temps = shorts(&mut f, 8)?;
            let data = shorts(&mut f, 72)?;
            let obsolete_word = f.i16()?;
            let id = data[11] as u16;
            if id != *expected_id {
                return r.fail("saved instance order disagrees with registration table");
            }
            let type_id = *types_by_id
                .get(&id)
                .ok_or_else(|| Error::new(origin, format!("record ID {id} missing from table")))?;
            let typ = types
                .entries
                .iter()
                .find(|t| t.id == type_id)
                .ok_or_else(|| {
                    Error::new(origin, format!("missing saved object type {type_id}"))
                })?;
            let result = (|| -> Result<Instance> {
                let n = f.count(7)?;
                let stack_capacity_bytes = f.i32()?;
                if stack_capacity_bytes < 0 {
                    return Err(Error::new(f.byte_position(), "negative frame arena size"));
                }
                let mut stack = Vec::new();
                for _ in 0..n {
                    let stack_object = f.i16()?;
                    let tree = f.u16()?;
                    let node = f.u16()?;
                    let local_count = f.u8()? as usize;
                    let param_count = f.u8()? as usize;
                    let parameters = shorts(&mut f, param_count)?;
                    let locals = shorts(&mut f, local_count)?;
                    stack.push(Frame {
                        stack_object,
                        tree,
                        node,
                        parameters,
                        locals,
                        primitive_state: f.i32()?,
                        code_owner_type: f.i16()?,
                    });
                }
                let relationship_flag = f.i32()?;
                if relationship_flag >= 0 {
                    return Err(Error::new(
                        f.byte_position(),
                        "unsupported saved relationship encoding",
                    ));
                }
                let n = f.count(1)?;
                let mut relationships = Vec::new();
                for _ in 0..n {
                    let present = f.i32()?;
                    if present == 0 {
                        relationships.push(Relationship {
                            present,
                            target: 0,
                            values: Vec::new(),
                        });
                        continue;
                    }
                    let target = f.i32()?;
                    let n = f.count(1)?;
                    let values = (0..n).map(|_| f.i32()).collect::<Result<Vec<_>>>()?;
                    relationships.push(Relationship {
                        present,
                        target,
                        values,
                    });
                }
                let n = short_count(&mut f)?;
                let mut slots = Vec::new();
                for _ in 0..n {
                    slots.push(Slot {
                        unknown: f.i16()?,
                        object: f.i16()?,
                    });
                }
                let n = short_count(&mut f)?;
                let dynamic_sprites = shorts(&mut f, n)?;
                let is_multi = multipart(typ.guid).ok_or_else(|| {
                    Error::new(f.byte_position(), format!("missing OBJD {:08x}", typ.guid))
                })?;
                let multipart = if is_multi {
                    Some(Multipart {
                        offset: [f.i32()?, f.i32()?, f.i32()?],
                        group: [f.i32()?, f.i32()?, f.i32()?],
                        parent: f.i16()?,
                    })
                } else {
                    None
                };
                let portal = if typ.kind == 8 {
                    let n = f.count(1)?;
                    Some((0..n).map(|_| f.f32()).collect::<Result<Vec<_>>>()?)
                } else {
                    None
                };
                let person = if typ.kind == 2 {
                    Some(Person::read(&mut f, version)?)
                } else {
                    None
                };
                // Present only if at least one more byte of the record remains.
                let registry_word = if f.remaining_bits() >= 8 {
                    Some(f.i32()?)
                } else {
                    None
                };
                Ok(Instance {
                    id,
                    type_id,
                    guid: typ.guid,
                    footprint,
                    position,
                    unknown_short,
                    attributes,
                    temps,
                    data,
                    obsolete_word,
                    stack_capacity_bytes,
                    stack,
                    relationship_flag,
                    relationships,
                    slots,
                    dynamic_sprites,
                    multipart,
                    portal,
                    person,
                    registry_word,
                    consumed_bits: f.bit_position(),
                    record: record.to_vec(),
                })
            })()
            .map_err(|e| {
                Error::new(
                    origin + e.offset,
                    format!("object {id} ({:08x}): {}", typ.guid, e.message),
                )
            })?;
            if instances.insert(id, result).is_some() {
                return r.fail("duplicate instance ID");
            }
        }
        let tail = r.take(r.remaining())?.to_vec();
        Ok(Self {
            version,
            order,
            types_by_id,
            instances,
            tail,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes an MSB-first field stream, the inverse of [`Fields`].
    #[derive(Default)]
    struct Writer {
        bits: Vec<bool>,
    }
    impl Writer {
        fn raw(&mut self, value: u32, width: usize) {
            for bit in (0..width).rev() {
                self.bits.push(value >> bit & 1 == 1);
            }
        }
        fn signed(&mut self, value: i32, widths: [usize; 4]) -> &mut Self {
            if value == 0 {
                self.bits.push(false);
                return self;
            }
            let selector = widths
                .iter()
                .position(|&w| w == 32 || (-(1 << (w - 1))..1 << (w - 1)).contains(&value))
                .unwrap();
            self.bits.push(true);
            self.raw(selector as u32, 2);
            self.raw(
                value as u32 & (u32::MAX >> (32 - widths[selector])),
                widths[selector],
            );
            self
        }
        fn i16(&mut self, value: i16) -> &mut Self {
            self.signed(value.into(), [5, 8, 13, 16])
        }
        fn i32(&mut self, value: i32) -> &mut Self {
            self.signed(value, [6, 11, 21, 32])
        }
        fn bytes(&self) -> Vec<u8> {
            self.bits
                .chunks(8)
                .map(|byte| {
                    byte.iter()
                        .enumerate()
                        .fold(0, |acc, (n, &bit)| acc | u8::from(bit) << (7 - n))
                })
                .collect()
        }
    }

    fn header(magic: &[u8; 4], version: u32) -> Vec<u8> {
        let mut bytes = vec![0; 4];
        bytes.extend(version.to_le_bytes());
        bytes.extend(magic);
        bytes
    }

    /// One ordinary object record: no attributes, no stack frames, no
    /// relationships, one empty slot, no dynamic sprites.
    fn record(id: i16, registry_word: Option<i32>, padding: usize) -> Vec<u8> {
        let mut w = Writer::default();
        // Footprint and position, then the unknown short and attribute count.
        for _ in 0..7 {
            w.i32(0);
        }
        w.i16(0).i16(0);
        for _ in 0..8 {
            w.i16(0);
        }
        for n in 0..72 {
            w.i16(if n == 11 { id } else { 0 });
        }
        w.i16(0).i32(0).i32(512).i32(-1).i32(0);
        w.i16(1).i16(0).i16(0).i16(0);
        if let Some(word) = registry_word {
            w.i32(word);
        }
        while w.bits.len() % 8 != 0 {
            w.bits.push(false);
        }
        w.raw(0, padding);
        w.bytes()
    }

    fn objects(records: &[Vec<u8>]) -> Result<Objects> {
        let mut types = header(b"tjbo", 2);
        types.extend(0x1234_5678u32.to_le_bytes());
        types.extend([0; 8]);
        types.extend(7u16.to_le_bytes());
        types.extend(4u16.to_le_bytes());
        types.extend(b"Chair\0");
        let types = ObjectTypes::parse(&types)?;
        let mut bytes = header(b"MjbO", 0x45);
        bytes.push(1);
        let mut table = Writer::default();
        for id in 1..=records.len() as i16 {
            table.i16(id).i16(7);
        }
        table.i16(0);
        bytes.extend(table.bytes());
        for record in records {
            let end = (bytes.len() + 4 + record.len() - 12) as u32;
            bytes.extend(end.to_le_bytes());
            bytes.extend(record);
        }
        Objects::parse(&bytes, &types, |_| Some(false))
    }

    #[test]
    fn the_registry_word_is_read_only_when_a_byte_remains() {
        let found = objects(&[
            record(1, Some(0x0002_0005), 0),
            // A placeholder-style record: ends on a byte boundary right after
            // the dynamic sprites.
            record(2, None, 0),
            record(3, Some(-3), 0),
        ])
        .unwrap();
        let words: Vec<_> = found.instances.values().map(|i| i.registry_word).collect();
        assert_eq!(words, [Some(0x0002_0005), None, Some(-3)]);
        assert_eq!(found.order, [1, 2, 3]);
    }

    #[test]
    fn a_record_shorter_than_its_fields_is_an_error() {
        let mut short = record(1, Some(0x0002_0005), 0);
        short.truncate(8);
        assert!(objects(&[short]).is_err());
    }
}
