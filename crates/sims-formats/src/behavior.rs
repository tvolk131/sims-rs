use crate::reader::Reader;
use crate::{Error, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Instruction {
    pub opcode: u16,
    pub on_true: u8,
    pub on_false: u8,
    pub operands: [u8; 8],
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct Behavior {
    pub format: u16,
    pub kind: u8,
    pub args: u8,
    pub locals: u16,
    pub instructions: Vec<Instruction>,
}
impl Behavior {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let format = r.u16()?;
        if !matches!(format, 0x8000..=0x8002) {
            return r.fail(format!("unsupported BHAV version {format:#06x}"));
        }
        let count = r.u16()? as usize;
        // Native 1.91 consults kind in all versions, and argument/local bytes in
        // 0x8001 too. Preserve raw header fields; execution applies version limits.
        let kind = r.u8()?;
        let args = r.u8()?;
        let locals = r.u16()?;
        r.skip(4)?;
        r.count(count, 12)?;
        let mut instructions = Vec::with_capacity(count);
        for _ in 0..count {
            instructions.push(Instruction {
                opcode: r.u16()?,
                on_true: r.u8()?,
                on_false: r.u8()?,
                operands: r.take(8)?.try_into().unwrap(),
            });
        }
        Ok(Self {
            format,
            kind,
            args,
            locals,
            instructions,
        })
    }
    pub fn disassemble(&self, resolve: impl Fn(u16) -> Option<String>) -> String {
        use std::fmt::Write;
        let mut out = format!(
            "BHAV {:#06x} · {} instructions · {} arguments · {} locals\n\n",
            self.format,
            self.instructions.len(),
            self.args,
            self.locals
        );
        for (i, ins) in self.instructions.iter().enumerate() {
            let operation = if ins.opcode < 256 {
                format!("primitive {:#06x}", ins.opcode)
            } else {
                let scope = if ins.opcode < 4096 {
                    "global"
                } else if ins.opcode < 8192 {
                    "local"
                } else {
                    "semiglobal"
                };
                format!(
                    "call {scope} {:#06x}{}",
                    ins.opcode,
                    resolve(ins.opcode)
                        .map(|s| format!("  {s}"))
                        .unwrap_or_default()
                )
            };
            let operands = ins
                .operands
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            let _ = writeln!(
                out,
                "{i:03}  {operation}\n     true → {}   false → {}\n     operands  {operands}",
                branch(ins.on_true, self.instructions.len()),
                branch(ins.on_false, self.instructions.len())
            );
        }
        out
    }
}
pub fn branch(target: u8, count: usize) -> String {
    match target {
        0xFD => "ERROR".into(),
        0xFE => "RETURN TRUE".into(),
        0xFF => "RETURN FALSE".into(),
        n if (n as usize) < count => format!("{n:03}"),
        n => format!("INVALID ({n})"),
    }
}

#[derive(Debug, Clone)]
pub struct ObjectDefinition {
    pub stand_function: u16,
    /// 2 is a person.
    pub object_type: u16,
    /// Nonzero for every tile of a multi-tile object: the group it belongs to.
    pub master_id: u16,
    pub animation_table: u16,
    pub version: u32,
    pub graphic: u16,
    pub graphics: u16,
    pub table: u16,
    pub guid: u32,
    pub price: u16,
    pub catalog: u16,
    pub slots: u16,
}
impl ObjectDefinition {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let version = r.u32()?;
        if !matches!(version, 136 | 138 | 139 | 140 | 141 | 142) {
            return r.fail(format!("unsupported OBJD version {version}"));
        }
        r.skip(2)?;
        let graphic = r.u16()?;
        let graphics = r.u16()?;
        r.skip(4)?;
        let table = r.u16()?;
        r.skip(2)?; // interaction group
        let object_type = r.u16()?;
        let master_id = r.u16()?;
        r.skip(4)?; // sub-index, wash-hands tree
        let animation_table = r.u16()?;
        let guid = r.u32()?;
        r.skip(4)?;
        let price = r.u16()?;
        r.skip(2)?;
        let slots = r.u16()?;
        // CatalogStringsID is word 39 after the version; preceding fields stay raw in IFF.
        let mut fields = Reader::new(data);
        fields.skip(4 + 31 * 2)?;
        let stand_function = fields.u16()?;
        fields.skip(7 * 2)?;
        let catalog = fields.u16()?;
        Ok(Self {
            stand_function,
            object_type,
            master_id,
            animation_table,
            version,
            graphic,
            graphics,
            table,
            guid,
            price,
            catalog,
            slots,
        })
    }
}
#[derive(Debug, Clone)]
pub struct MotiveAdvertisement {
    pub minimum: i16,
    pub delta: i16,
    pub personality: u16,
}
#[derive(Debug, Clone)]
pub struct Interaction {
    pub action: u16,
    pub test: u16,
    pub string_index: u32,
    pub flags: u32,
    pub autonomy: u32,
    pub attenuation_code: u32,
    pub attenuation: f32,
    pub joining_index: i32,
    pub motives: Vec<MotiveAdvertisement>,
    pub flags2: u32,
}
#[derive(Debug, Clone)]
pub struct InteractionTable {
    pub version: u16,
    pub interactions: Vec<Interaction>,
}
impl InteractionTable {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let count = r.u16()? as usize;
        if count == 0 {
            return Ok(Self {
                version: 0,
                interactions: Vec::new(),
            });
        }
        let version = r.u16()?;
        if !(4..=10).contains(&version) {
            return r.fail(format!("unsupported TTAB version {version}"));
        }
        let compressed = version >= 9 && r.u8()? == 1;
        let mut fields = Fields {
            data: &data[r.pos..],
            bit: 0,
            compressed,
        };
        if count > data.len() {
            return r.fail("impossible TTAB entry count");
        }
        let mut interactions = Vec::with_capacity(count);
        for _ in 0..count {
            let action = fields.read(16)? as u16;
            let test = fields.read(16)? as u16;
            let motives = fields.read(32)? as usize;
            if motives > 256 {
                return r.fail("TTAB motive count exceeds limit");
            }
            let flags = fields.read(32)?;
            let string_index = fields.read(32)?;
            let attenuation_code = if version > 6 { fields.read(32)? } else { 0 };
            let attenuation = f32::from_bits(fields.read(32)?);
            let autonomy = fields.read(32)?;
            let joining_index = fields.read(32)? as i32;
            let mut advertisements = Vec::with_capacity(motives);
            for _ in 0..motives {
                let minimum = if version > 6 {
                    fields.read(16)? as i16
                } else {
                    0
                };
                let delta = fields.read(16)? as i16;
                let personality = if version > 6 {
                    fields.read(16)? as u16
                } else {
                    0
                };
                advertisements.push(MotiveAdvertisement {
                    minimum,
                    delta,
                    personality,
                });
            }
            let flags2 = if version > 9 { fields.read(32)? } else { 0 };
            interactions.push(Interaction {
                action,
                test,
                string_index,
                flags,
                autonomy,
                attenuation_code,
                attenuation,
                joining_index,
                motives: advertisements,
                flags2,
            });
        }
        Ok(Self {
            version,
            interactions,
        })
    }
}
struct Fields<'a> {
    data: &'a [u8],
    bit: usize,
    compressed: bool,
}
impl Fields<'_> {
    fn bits(&mut self, n: usize) -> Result<u32> {
        if n > self.data.len().saturating_mul(8).saturating_sub(self.bit) {
            return Err(Error::new(self.bit / 8, "truncated TTAB bit field"));
        }
        let mut value = 0;
        for _ in 0..n {
            value = (value << 1) | u32::from((self.data[self.bit / 8] >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Ok(value)
    }
    fn read(&mut self, size: usize) -> Result<u32> {
        if !self.compressed {
            let p = self.bit / 8;
            let mut r = Reader::new(&self.data[p..]);
            let v = if size == 16 {
                u32::from(r.u16()?)
            } else {
                r.u32()?
            };
            self.bit += size;
            return Ok(v);
        }
        if self.bits(1)? == 0 {
            return Ok(0);
        }
        let code = self.bits(2)? as usize;
        let width = if size == 16 {
            [5, 8, 13, 16][code]
        } else {
            [6, 11, 21, 32][code]
        };
        let value = self.bits(width)?;
        let signed = if width < 32 && value & (1 << (width - 1)) != 0 {
            value | (!0u32 << width)
        } else {
            value
        };
        Ok(if size == 16 { signed & 0xFFFF } else { signed })
    }
}
