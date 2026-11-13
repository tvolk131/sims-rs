//!4b9613 ->4b97ea ->4c9ceb, with each context store retained separately.
//! Geometry and OffsetRect are explicit children; table reads are borrowed.
//! This models stored values with FCW003f, not x87 exception-status flags.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    pub flags: u32,
    pub value: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caches {
    pub alpha: Cache,
    pub beta: Cache,
}
pub struct Depth<'a> {
    pub origin_y: i32,
    pub clip: [i32; 4],
    pub heights: [i32; 3],
    pub extents: [i32; 3],
    pub limit: i32,
    pub level_depth: [i32; 3],
    pub row_start: i32,
    pub rows: &'a [i32],
    pub curve_start: i32,
    pub curve: &'a [i16],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Height {
        physical: [i32; 2],
    },
    Screen {
        physical: [i32; 2],
        level: i32,
        height: u8,
    },
    OffsetRect {
        rect: [i32; 4],
        delta: [i32; 2],
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Height(u32),
    Screen([i16; 2]),
    Rect([i32; 4]),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    X,
    Y,
    Height,
    HeightStore,
    Level,
    Screen,
    ScreenX,
    ScreenY,
    Normalize,
    Alpha,
    AlphaValue,
    DepthCall,
    Beta,
    BetaValue,
    DepthRange,
    RowY,
    RowBase,
    RowFloor,
    Correction,
    FinalY,
    FallbackZero,
    FallbackY,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementUpdate {
    physical: [i32; 2],
    level: i32,
    height_override: i32,
    rotation: u8,
    stage: Stage,
    pending: Option<Request>,
    height: u32,
    screen: [i32; 2],
    alt: u32,
    scaled: u32,
    sqrt: u32,
    normalized: [i32; 4],
    offset: i32,
    y: i32,
    fallback: i32,
    written: u32,
    values: [u32; 11],
    store: Option<(u32, u32)>,
}
fn int(v: f32) -> Result<i32, String> {
    if !v.is_finite() || v < i64::MIN as f32 || v >= 9223372036854775808.0f32 {
        return Err("placement conversion needs native indefinite/FP status owner".into());
    }
    Ok(v.trunc() as i64 as i32)
}
impl PlacementUpdate {
    pub fn new(physical: [i32; 2], level: i32, height_override: i32, rotation: u8) -> Self {
        Self {
            physical,
            level,
            height_override,
            rotation,
            stage: Stage::X,
            pending: None,
            height: 0,
            screen: [0; 2],
            alt: 0,
            scaled: 0,
            sqrt: 0,
            normalized: [0; 4],
            offset: 0,
            y: 0,
            fallback: 0,
            written: 0,
            values: [0; 11],
            store: None,
        }
    }
    ///4b9991 shares454f29/4c9ceb after refreshing an already placed context.
    /// No XY/height/level placement stores or level normalization are replayed.
    pub fn depth_from_context() -> Self {
        let mut value=Self::new([0,0],1,-1,0);value.stage=Stage::Alpha;value
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn written_mask(&self) -> u32 {
        self.written
    }
    pub fn last_store(&self) -> Option<(u32, u32)> {
        self.store
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x4b9613 => self.stage == Stage::X,
            0x4b9633 => self.stage == Stage::Height && self.pending.is_none(),
            0x4b9658 => self.stage == Stage::HeightStore,
            0x4b9660 | 0x4b97ea => self.stage == Stage::Screen && self.pending.is_none(),
            0x4b9881 => matches!(self.pending, Some(Request::Screen { .. })),
            0x4b9883 => self.stage == Stage::ScreenX,
            0x4b9924 => self.stage == Stage::Normalize,
            0x4b9935 | 0x454f29 => self.stage == Stage::Alpha,
            0x454f4f | 0x4b993a => self.stage == Stage::DepthCall && self.pending.is_none(),
            0x4b997b | 0x4c9ceb => matches!(self.pending, Some(Request::OffsetRect { .. })),
            0x4c9d72 => self.stage == Stage::RowY,
            0x4c9d9a => self.stage == Stage::RowBase,
            0x4c9e31 | 0x4b9980 | 0x4b9665 | 0x4af199 => self.complete(),
            _ => false,
        }
    }
    pub fn validate_words(&self, w: &[u32; 29]) -> Result<(), String> {
        if self.rotation > 3 || self.written & !0x63f != 0 {
            return Err("placement cursor/field owner differs".into());
        }
        if let Some(r) = self.pending {
            if !matches!(
                (self.stage, r),
                (Stage::Height, Request::Height { .. })
                    | (Stage::Screen, Request::Screen { .. })
                    | (Stage::DepthCall, Request::OffsetRect { .. })
            ) {
                return Err("placement child escaped its caller".into());
            }
        }
        for (i, &v) in self.values.iter().enumerate() {
            if self.written & (1 << i) != 0 && w[i] != v {
                return Err("placement lost its preceding context store".into());
            }
        }
        Ok(())
    }
    fn write(&mut self, w: &mut [u32; 29], index: usize, value: u32) {
        w[index] = value;
        self.values[index] = value;
        self.written |= 1 << index;
        self.store = Some(((index * 4) as u32, value));
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        if self.pending != Some(request) {
            return Err("placement reply is not pending".into());
        }
        match (request, reply) {
            (Request::Height { .. }, Reply::Height(v)) => {
                self.height = v;
                self.stage = Stage::HeightStore;
            }
            (Request::Screen { .. }, Reply::Screen(v)) => {
                self.screen = [i32::from(v[1]), i32::from(v[0])];
                self.stage = Stage::ScreenX;
            }
            (Request::OffsetRect { .. }, Reply::Rect(v)) => {
                self.normalized = v;
                self.stage = Stage::Beta;
            }
            _ => return Err("placement reply kind differs".into()),
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance_words(
        &mut self,
        w: &mut [u32; 29],
        c: &mut Caches,
        d: &Depth<'_>,
    ) -> Result<bool, String> {
        self.validate_words(w)?;
        if self.pending.is_some() || self.complete() {
            return Ok(false);
        }
        self.store = None;
        let zoom = w[0x20 / 4];
        if !(1..=3).contains(&zoom) {
            return Err("placement zoom owner is unobserved".into());
        }
        let q = (3 - zoom) as usize;
        match self.stage {
            Stage::X => {
                self.write(w, 0, (self.physical[1] as f32).to_bits());
                self.stage = Stage::Y;
            }
            Stage::Y => {
                self.write(w, 1, (self.physical[0] as f32).to_bits());
                self.stage = Stage::Height;
            }
            Stage::Height => {
                if w[0x64 / 4] & 1 != 0 {
                    self.height = 0;
                    self.stage = Stage::HeightStore;
                } else if self.height_override != -1 {
                    self.height = (self.height_override as f32).to_bits();
                    self.stage = Stage::HeightStore;
                } else {
                    self.pending = Some(Request::Height {
                        physical: self.physical,
                    });
                }
            }
            Stage::HeightStore => {
                self.write(w, 2, self.height);
                self.stage = Stage::Level;
            }
            Stage::Level => {
                self.write(w, 3, self.level as u32);
                self.stage = Stage::Screen;
            }
            Stage::Screen => {
                let mut x = int(f32::from_bits(w[0]) + 0.5)?;
                let mut y = int(f32::from_bits(w[1]) + 0.5)?;
                if w[0x64 / 4] & 1 == 0 {
                    [x, y] = match self.rotation {
                        1 => [1024i32.wrapping_sub(y), x],
                        2 => [1024i32.wrapping_sub(x), 1024i32.wrapping_sub(y)],
                        3 => [y, 1024i32.wrapping_sub(x)],
                        _ => [x, y],
                    };
                    let height = int(f32::from_bits(w[2]) + 0.5)? as u8;
                    self.pending = Some(Request::Screen {
                        physical: [y, x],
                        level: w[3] as i32,
                        height,
                    });
                } else {
                    let [x, y, rx, ry] = [
                        x >> 4,
                        y >> 4,
                        (w[0x54 / 4] as i32) >> 4,
                        (w[0x50 / 4] as i32) >> 4,
                    ]
                    .map(|v| i32::from(v as i8));
                    self.screen = [
                        (w[4] as i32).wrapping_add(
                            x.wrapping_sub(rx)
                                .wrapping_sub(y)
                                .wrapping_add(ry)
                                .wrapping_mul(8i32.wrapping_shl(zoom)),
                        ),
                        (w[5] as i32).wrapping_add(
                            x.wrapping_sub(rx)
                                .wrapping_sub(ry)
                                .wrapping_add(y)
                                .wrapping_mul(4i32.wrapping_shl(zoom)),
                        ),
                    ];
                    self.stage = Stage::ScreenX;
                }
            }
            Stage::ScreenX => {
                self.write(w, 4, self.screen[0] as u32);
                self.stage = Stage::ScreenY;
            }
            Stage::ScreenY => {
                self.write(w, 5, self.screen[1] as u32);
                self.stage = Stage::Normalize;
            }
            Stage::Normalize => {
                if w[3] == 0 {
                    self.write(w, 3, 1);
                }
                self.stage = Stage::Alpha;
            }
            Stage::Alpha => {
                if c.alpha.flags & 1 == 0 {
                    self.sqrt = 6.0f32.sqrt().to_bits();
                    c.alpha.flags |= 1;
                    self.stage = Stage::AlphaValue;
                } else {
                    self.stage = Stage::DepthCall;
                }
            }
            Stage::AlphaValue => {
                c.alpha.value = (1.0 / (f32::from_bits(self.sqrt) * 2.0)).to_bits();
                self.stage = Stage::DepthCall;
            }
            Stage::DepthCall => {
                self.alt = ((f32::from_bits(c.alpha.value) * f32::from_bits(w[2])) * 3.0).to_bits();
                self.y = int((1i32.wrapping_shl(zoom + 1) as f32) * f32::from_bits(w[2]) + 0.5)?
                    .wrapping_sub(4i32.wrapping_shl(zoom))
                    .wrapping_add(w[5] as i32);
                self.pending = Some(Request::OffsetRect {
                    rect: d.clip,
                    delta: [d.clip[0].wrapping_neg(), d.clip[1].wrapping_neg()],
                });
            }
            Stage::Beta => {
                // The input scalar is spilled before the cached inverse helper.
                self.scaled = (f32::from_bits(self.alt) * f32::from_bits(0x3eaaaaab)).to_bits();
                if c.beta.flags & 1 == 0 {
                    self.sqrt = 6.0f32.sqrt().to_bits();
                    c.beta.flags |= 1;
                    self.stage = Stage::BetaValue;
                } else {
                    self.stage = Stage::DepthRange;
                }
            }
            Stage::BetaValue => {
                c.beta.value = (f32::from_bits(self.sqrt) * 2.0).to_bits();
                self.stage = Stage::DepthRange;
            }
            Stage::DepthRange => {
                self.scaled =
                    (f32::from_bits(self.scaled) * f32::from_bits(c.beta.value)).to_bits();
                self.offset = d.heights[q].wrapping_mul((w[3] as i32).wrapping_sub(1));
                self.y = self
                    .y
                    .wrapping_add(4i32.wrapping_shl(zoom))
                    .wrapping_add(self.offset);
                self.fallback = self.normalized[1].wrapping_sub(d.extents[q]);
                self.stage = if self.y < self.fallback
                    || self.y >= self.normalized[1].wrapping_add(d.limit)
                {
                    Stage::FallbackZero
                } else {
                    Stage::RowY
                };
            }
            Stage::RowY => {
                self.write(w, 9, self.y.wrapping_sub(self.offset) as u32);
                self.stage = Stage::RowBase;
            }
            Stage::RowBase => {
                let index = self
                    .y
                    .wrapping_sub(d.origin_y)
                    .checked_sub(d.row_start)
                    .and_then(|i| usize::try_from(i).ok())
                    .ok_or("placement depth row precedes retained table")?;
                let value = *d
                    .rows
                    .get(index)
                    .ok_or("placement depth row outside retained table")?;
                self.write(w, 10, value as u32);
                self.stage = Stage::RowFloor;
            }
            Stage::RowFloor => {
                let value = w[10].wrapping_add(
                    d.level_depth[q].wrapping_mul((w[3] as i32).wrapping_sub(1)) as u32,
                );
                self.write(w, 10, value);
                self.stage = Stage::Correction;
            }
            Stage::Correction => {
                if f32::from_bits(self.alt).abs() <= f32::from_bits(0x358637bd) {
                    self.stage = Stage::Complete;
                } else {
                    let value = f32::from_bits(self.scaled);
                    let k = int(value.floor())?;
                    let index = k
                        .checked_sub(d.curve_start)
                        .and_then(|i| usize::try_from(i).ok())
                        .ok_or("placement height curve precedes retained table")?;
                    let a = i32::from(
                        *d.curve
                            .get(index)
                            .ok_or("placement height curve outside retained table")?,
                    );
                    let b = i32::from(
                        *d.curve
                            .get(index + 1)
                            .ok_or("placement height curve neighbor outside retained table")?,
                    );
                    let add =
                        int((b.wrapping_sub(a) as f32) * (value - k as f32) + 0.5)?.wrapping_add(a);
                    self.write(w, 10, w[10].wrapping_add(add as u32));
                    self.stage = Stage::FinalY;
                }
            }
            Stage::FinalY => {
                let subtract =
                    int((1i32.wrapping_shl(zoom + 1) as f32) * f32::from_bits(self.scaled) + 0.5)?;
                self.write(w, 9, w[9].wrapping_sub(subtract as u32));
                self.stage = Stage::Complete;
            }
            Stage::FallbackZero => {
                self.write(w, 10, 0);
                self.stage = Stage::FallbackY;
            }
            Stage::FallbackY => {
                self.write(w, 9, self.fallback as u32);
                self.stage = Stage::Complete;
            }
            Stage::Complete => return Ok(false),
        }
        self.validate_words(w)?;
        Ok(true)
    }
}
