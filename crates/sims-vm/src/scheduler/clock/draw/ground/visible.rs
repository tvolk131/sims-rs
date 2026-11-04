//!49cf02..49d237 terrain preparation. Every owner store is resumable.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Shade { point: [i32; 2] },
    Terrain { point: [i8; 3] },
    Variation { point: [i8; 2] },
    Covering { point: [i8; 3] },
    PoolInclusive { point: [i8; 3] },
    WallFlags { point: [i8; 3] },
    Snow,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Shade(u8),
    Terrain(i8),
    Variation(u16),
    Covering(u16),
    PoolInclusive(bool),
    WallFlags(u8),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Select,
    AskShade,
    Shade,
    AskTerrain,
    Terrain,
    AskVariation,
    Variation,
    Snow,
    SnowChild,
    ClearSnow,
    ParticleGate,
    Cover,
    Pool,
    Wall,
    Compute,
    Flag,
    Count,
    Zero,
    Quarter,
    Divide,
    Remainder,
    Complete,
    WallReply(u8),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    point: [i32; 2],
    relative: [i8; 2],
    zoom: u32,
    stage: Stage,
    corner: u8,
    constant: Option<u8>,
    shade: u8,
    terrain: i8,
    variation: u16,
    count: i32,
    flag: u8,
    quotient: u32,
    remainder: u32,
    store: Option<(u32, u8, u32)>,
    writes: Vec<(u32, u8, u32)>,
}
fn word(raw: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(raw[i..i + 4].try_into().unwrap())
}
impl Frame {
    pub fn new(point: [i32; 2], bounds: [i32; 4], zoom: u32) -> Self {
        Self {
            point,
            relative: [
                (point[0] as i8).wrapping_sub(bounds[0] as i8),
                (point[1] as i8).wrapping_sub(bounds[2] as i8),
            ],
            zoom,
            stage: Stage::Select,
            corner: 0,
            constant: None,
            shade: 0,
            terrain: 0,
            variation: 0,
            count: 0,
            flag: 0,
            quotient: 0,
            remainder: 0,
            store: None,
            writes: vec![],
        }
    }
    fn point3(&self) -> [i8; 3] {
        [self.point[0] as i8, self.point[1] as i8, 1]
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn result(&self) -> Option<u32> {
        self.complete().then_some(self.quotient)
    }
    pub fn last_store(&self) -> Option<(u32, u8, u32)> {
        self.store
    }
    pub fn pending(&self) -> Option<Request> {
        match self.stage {
            Stage::AskShade => {
                let [x, y] = self.point;
                Some(Request::Shade {
                    point: match self.corner {
                        0 => [x, y],
                        1 => [x, y.wrapping_add(1)],
                        2 => [x.wrapping_add(1), y.wrapping_add(1)],
                        _ => [x.wrapping_add(1), y],
                    },
                })
            }
            Stage::AskTerrain => Some(Request::Terrain {
                point: self.point3(),
            }),
            Stage::AskVariation => Some(Request::Variation {
                point: self.relative,
            }),
            Stage::Cover => Some(Request::Covering {
                point: self.point3(),
            }),
            Stage::Pool => Some(Request::PoolInclusive {
                point: self.point3(),
            }),
            Stage::Wall => Some(Request::WallFlags {
                point: self.point3(),
            }),
            Stage::SnowChild => Some(Request::Snow),
            _ => None,
        }
    }
    pub fn validate(&self, raw: &[u8]) -> Result<(), String> {
        if raw.len() < 0x388
            || self.corner > 4
            || matches!(self.stage, Stage::Shade | Stage::AskShade) && self.corner >= 4
            || !(1..=3).contains(&self.zoom)
        {
            return Err("terrain preparation owner differs".into());
        }
        for &(offset, size, value) in &self.writes {
            let i = offset as usize;
            let n = usize::from(size);
            if !matches!(
                (offset, size),
                (0x360..=0x363 | 0x378, 1) | (0x364 | 0x368 | 0x36c | 0x370 | 0x374, 4)
            ) || raw.get(i..i + n) != Some(&value.to_le_bytes()[..n])
            {
                return Err("terrain preparation suspended store differs".into());
            }
        }
        Ok(())
    }
    fn write(&mut self, raw: &mut [u8], offset: usize, size: usize, value: u32) {
        raw[offset..offset + size].copy_from_slice(&value.to_le_bytes()[..size]);
        self.store = Some((offset as u32, size as u8, value));
        self.writes.retain(|w| w.0 != offset as u32);
        self.writes.push((offset as u32, size as u8, value));
    }
    pub fn reply(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        if self.pending() != Some(request) {
            return Err("terrain preparation reply lost its caller".into());
        }
        match (request, reply) {
            (Request::Shade { .. }, Reply::Shade(v)) => {
                self.shade = v;
                self.stage = Stage::Shade;
            }
            (Request::Terrain { .. }, Reply::Terrain(v)) => {
                self.terrain = v;
                self.stage = Stage::Terrain;
            }
            (Request::Variation { .. }, Reply::Variation(v)) => {
                self.variation = v;
                self.stage = Stage::Variation;
            }
            (Request::Covering { .. }, Reply::Covering(v)) => {
                self.stage = if v == 0 { Stage::Pool } else { Stage::Wall }
            }
            (Request::PoolInclusive { .. }, Reply::PoolInclusive(v)) => {
                self.stage = if v { Stage::Zero } else { Stage::Compute }
            }
            (Request::WallFlags { .. }, Reply::WallFlags(v)) => {
                self.stage = Stage::WallReply(v);
            }
            _ => return Err("terrain preparation reply kind differs".into()),
        }
        self.store = None;
        Ok(())
    }
    pub fn advance(
        &mut self,
        raw: &mut [u8],
        active: i32,
        particle_height: i32,
    ) -> Result<bool, String> {
        self.validate(raw)?;
        if self.pending().is_some() || self.complete() {
            return Ok(false);
        }
        self.store = None;
        match self.stage {
            Stage::Select => {
                let mode = word(raw, 0x37c);
                self.constant = if mode != 0 {
                    Some(mode.wrapping_sub(1) as u8)
                } else if raw[0x8d] != 0
                    && (self.relative[0] <= 1
                        || self.relative[1] <= 1
                        || i32::from(self.relative[0]) >= active - 2
                        || i32::from(self.relative[1]) >= active - 2)
                {
                    Some(0)
                } else {
                    None
                };
                if let Some(v) = self.constant {
                    self.shade = v;
                    self.stage = Stage::Shade;
                } else {
                    self.stage = Stage::AskShade;
                }
            }
            Stage::Shade => {
                self.write(
                    raw,
                    [0x361, 0x360, 0x363, 0x362][self.corner as usize],
                    1,
                    u32::from(self.shade),
                );
                self.corner += 1;
                self.stage = if self.corner == 4 {
                    Stage::AskTerrain
                } else if self.constant.is_some() {
                    Stage::Shade
                } else {
                    Stage::AskShade
                };
            }
            Stage::Terrain => {
                self.write(raw, 0x368, 4, i32::from(self.terrain) as u32);
                self.stage = Stage::AskVariation;
            }
            Stage::Variation => {
                self.write(raw, 0x364, 4, u32::from(self.variation));
                self.stage = Stage::Snow;
            }
            Stage::Snow => {
                self.stage = if raw[0x81] != 0 && self.terrain >= -97 {
                    Stage::SnowChild
                } else {
                    Stage::ClearSnow
                };
            }
            Stage::ClearSnow => {
                self.write(raw, 0x36c, 4, 0);
                self.stage = Stage::ParticleGate;
            }
            Stage::ParticleGate => {
                self.stage = if raw[0x80] == 0 || raw[0x82] == 0 || particle_height == 0 {
                    Stage::Zero
                } else {
                    Stage::Cover
                }
            }
            Stage::Compute => {
                let value = i32::from(self.terrain);
                let lo = word(raw, 0x9c) as i32;
                let hi = word(raw, 0xa0) as i32;
                let a = word(raw, 0xd0) as i32;
                let b = word(raw, 0xd4) as i32;
                if value >= hi {
                    self.flag = 0;
                    self.count = b;
                } else if value <= lo {
                    self.flag = 1;
                    self.count = a;
                } else {
                    self.flag = 0;
                    let range = hi.wrapping_sub(lo);
                    self.count = if range < 1 {
                        0
                    } else {
                        (b.wrapping_sub(a).wrapping_mul(value.wrapping_sub(lo)) / range)
                            .wrapping_add(a)
                    };
                }
                self.stage = Stage::Flag;
            }
            Stage::Flag => {
                self.write(raw, 0x378, 1, u32::from(self.flag));
                self.stage = Stage::Count;
            }
            Stage::Count => {
                self.write(raw, 0x370, 4, self.count as u32);
                self.stage = Stage::Quarter;
            }
            Stage::Zero => {
                self.write(raw, 0x370, 4, 0);
                self.stage = Stage::Divide;
            }
            Stage::WallReply(v) => {
                self.stage = if v & 0x30 == 0 {
                    Stage::Zero
                } else {
                    Stage::Divide
                };
            }
            Stage::Quarter => {
                if self.zoom == 1 {
                    self.write(raw, 0x370, 4, ((word(raw, 0x370) as i32) / 4) as u32);
                }
                self.stage = Stage::Divide;
            }
            Stage::Divide => {
                let count = word(raw, 0x370);
                if count == 0 {
                    self.quotient = 0;
                    self.stage = Stage::Complete;
                } else {
                    let divisor = word(raw, 0xdc).wrapping_sub(count);
                    if divisor == 0 {
                        return Err("terrain preparation native division fault".into());
                    }
                    self.quotient = word(raw, 0x364) / divisor;
                    self.remainder = word(raw, 0x364) % divisor;
                    self.stage = Stage::Remainder;
                }
            }
            Stage::Remainder => {
                self.write(raw, 0x374, 4, self.remainder);
                self.stage = Stage::Complete;
            }
            Stage::AskShade
            | Stage::AskTerrain
            | Stage::AskVariation
            | Stage::Cover
            | Stage::Pool
            | Stage::Wall
            | Stage::SnowChild
            | Stage::Complete => return Ok(false),
        }
        self.validate(raw)?;
        Ok(true)
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x49cf02 => self.stage == Stage::Select,
            0x49cf6f => self.stage == Stage::AskShade && self.corner == 0,
            0x49cd43 => self.stage == Stage::AskShade,
            0x49cf78 => self.stage == Stage::Shade && self.corner == 0,
            0x49cf8d => self.stage == Stage::Shade && self.corner == 1,
            0x49cfb3 => self.stage == Stage::Shade && self.corner == 3,
            0x49cfb9 => self.stage == Stage::AskTerrain,
            0x49cfd6 => self.stage == Stage::Terrain,
            0x49cff1 => self.stage == Stage::AskVariation,
            0x49d001 => self.stage == Stage::Snow,
            0x49d0da => self.stage == Stage::ParticleGate,
            0x49d142 | 0x467690 => self.stage == Stage::Wall,
            0x49d14b => matches!(self.stage, Stage::WallReply(_)),
            0x49d155 => self.stage == Stage::Zero,
            0x49d237 => self.complete(),
            _ => false,
        }
    }
    pub fn wall_reply(&self) -> Option<u8> {
        if let Stage::WallReply(v) = self.stage {
            Some(v)
        } else {
            None
        }
    }
}
