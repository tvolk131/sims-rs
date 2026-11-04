//!4acf4e..4ad19e: ordered background/damage intersections and clear callbacks.
//! The initialized temporary-vector path retains unused allocation bytes.
//! CRT initialization, vector growth and pixel clears are explicit children.
use super::regions::{List, Rect};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy)]
pub struct Controls {
    pub enabled: u8,
    pub behind: u8,
    pub full: u8,
    pub viewport: Rect,
    pub delta: [i32; 2],
    pub flags: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    InitializeTemporary,
    GrowTemporary { rectangle: Rect },
    Color { rectangle: Rect },
    Depth { rectangle: Rect },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Reset,
    ResetReturned,
    Outer,
    Offset,
    Clipped,
    Pair,
    Append,
    PairReturned,
    RegionsDone,
    Color,
    ColorReturned,
    Depth,
    DepthReturned,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    stage: Stage,
    outer: usize,
    inner: usize,
    clear: usize,
    rectangle: Rect,
    damage: Rect,
    candidate: Option<Rect>,
    full: bool,
    pending: Option<Request>,
}
fn valid(r: Rect) -> bool {
    r[0] < r[2] && r[1] < r[3]
}
fn clamp(r: Rect, b: Rect) -> Rect {
    [
        r[0].max(b[0]).min(b[2]),
        r[1].max(b[1]).min(b[3]),
        r[2].min(b[2]).max(b[0]),
        r[3].min(b[3]).max(b[1]),
    ]
}
fn overlap(a: Rect, b: Rect) -> bool {
    // Keep the original wrapped differences and signed OR test, rather than
    // replacing it with ordinary interval comparisons at extreme coordinates.
    a[3].wrapping_sub(b[1]).wrapping_sub(1)
        | a[2].wrapping_sub(b[0]).wrapping_sub(1)
        | b[3].wrapping_sub(a[1]).wrapping_sub(1)
        | b[2].wrapping_sub(a[0]).wrapping_sub(1)
        >= 0
}
impl Frame {
    pub fn new() -> Self {
        Self {
            stage: Stage::Entry,
            outer: 0,
            inner: 0,
            clear: 0,
            rectangle: [0; 4],
            damage: [0; 4],
            candidate: None,
            full: false,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn pending(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn indices(&self) -> (usize, usize) {
        (self.outer, self.inner)
    }
    pub fn rectangle(&self) -> Rect {
        self.rectangle
    }
    pub fn damage(&self) -> Rect {
        self.damage
    }
    pub fn candidate(&self) -> Option<Rect> {
        self.candidate
    }
    pub fn validate(&self, source: &List, damage: &List, temporary: &List) -> Result<(), String> {
        source.validate()?;
        damage.validate()?;
        temporary.validate()?;
        if self.outer > source.len() || self.inner > damage.len() || self.clear > temporary.len() {
            return Err("background vector cursor differs".into());
        }
        if self.pending.as_ref().is_some_and(|p| {
            !matches!(
                (self.stage, p),
                (Stage::Entry, Request::InitializeTemporary)
                    | (Stage::Append, Request::GrowTemporary { .. })
                    | (Stage::Color, Request::Color { .. })
                    | (Stage::Depth, Request::Depth { .. })
            )
        }) {
            return Err("background child lost its caller".into());
        }
        Ok(())
    }
    pub fn acknowledge_clear(&mut self, request: &Request) -> Result<(), String> {
        if self.pending.as_ref() != Some(request) {
            return Err("background clear child differs".into());
        }
        self.stage = match request {
            Request::Color { .. } => Stage::ColorReturned,
            Request::Depth { .. } => Stage::DepthReturned,
            _ => return Err("background child requires its actual owner".into()),
        };
        self.pending = None;
        Ok(())
    }
    pub fn advance(
        &mut self,
        c: Controls,
        source: &List,
        damage: &List,
        temporary: &mut List,
    ) -> Result<bool, String> {
        self.validate(source, damage, temporary)?;
        if self.pending.is_some() || self.complete() {
            return Ok(false);
        }
        match self.stage {
            Stage::Entry => {
                if c.enabled == 0 || c.behind != 0 {
                    self.stage = Stage::Complete;
                } else if c.full != 0 {
                    self.full = true;
                    self.rectangle = c.viewport;
                    self.stage = Stage::Color;
                } else if temporary.header[12] & 1 == 0 {
                    self.pending = Some(Request::InitializeTemporary);
                } else {
                    self.stage = Stage::Reset;
                }
            }
            Stage::Reset => {
                let start: [u8; 4] = temporary.header[..4].try_into().unwrap();
                temporary.header[4..8].copy_from_slice(&start);
                self.stage = Stage::ResetReturned;
            }
            Stage::ResetReturned => self.stage = Stage::Outer,
            Stage::Outer => {
                if self.outer == source.len() {
                    self.stage = Stage::RegionsDone;
                } else {
                    self.rectangle = source.rectangle(self.outer)?;
                    self.stage = Stage::Offset;
                }
            }
            Stage::Offset => {
                for (i, v) in self.rectangle.iter_mut().enumerate() {
                    *v = v.wrapping_add(c.delta[i & 1]);
                }
                self.stage = Stage::Clipped;
            }
            Stage::Clipped => {
                self.rectangle = clamp(self.rectangle, c.viewport);
                if valid(self.rectangle) {
                    self.inner = 0;
                    self.stage = Stage::Pair;
                } else {
                    self.outer += 1;
                    self.stage = Stage::Outer;
                }
            }
            Stage::Pair => {
                if self.inner == damage.len() {
                    self.outer += 1;
                    self.stage = Stage::Outer;
                } else {
                    self.damage = damage.rectangle(self.inner)?;
                    if overlap(self.rectangle, self.damage) {
                        let r = clamp(self.rectangle, self.damage);
                        self.candidate = Some(r);
                        self.stage = if valid(r) {
                            Stage::Append
                        } else {
                            Stage::PairReturned
                        };
                    } else {
                        self.stage = Stage::PairReturned;
                    }
                }
            }
            Stage::Append => {
                let rectangle = self.candidate.ok_or("background append rectangle absent")?;
                if temporary.append(rectangle)? {
                    self.stage = Stage::PairReturned;
                } else {
                    self.pending = Some(Request::GrowTemporary { rectangle });
                }
            }
            Stage::PairReturned => {
                self.inner += 1;
                self.stage = Stage::Pair;
            }
            Stage::RegionsDone => {
                if temporary.len() == 0 {
                    self.stage = Stage::Complete;
                } else {
                    self.rectangle = temporary.rectangle(0)?;
                    self.stage = Stage::Color;
                }
            }
            Stage::Color => {
                if c.flags & 1 != 0 {
                    self.pending = Some(Request::Color {
                        rectangle: self.rectangle,
                    });
                } else {
                    self.stage = Stage::ColorReturned;
                }
            }
            Stage::ColorReturned => self.stage = Stage::Depth,
            Stage::Depth => {
                if c.flags & 2 != 0 {
                    self.pending = Some(Request::Depth {
                        rectangle: self.rectangle,
                    });
                } else {
                    self.stage = Stage::DepthReturned;
                }
            }
            Stage::DepthReturned => {
                if self.full {
                    self.stage = Stage::Complete;
                } else {
                    self.clear += 1;
                    if self.clear == temporary.len() {
                        self.stage = Stage::Complete;
                    } else {
                        self.rectangle = temporary.rectangle(self.clear)?;
                        self.stage = Stage::Color;
                    }
                }
            }
            Stage::Complete => unreachable!(),
        }
        self.validate(source, damage, temporary)?;
        Ok(true)
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x4acf4e => self.stage == Stage::Entry,
            0x4b3e06 => self.stage == Stage::Reset,
            0x4acfb5 => self.stage == Stage::ResetReturned,
            0x4acff5 => self.stage == Stage::Offset,
            0x4acffb => self.stage == Stage::Clipped,
            0x4b3ddb => self.stage == Stage::Append,
            0x4ad0a9 => self.stage == Stage::PairReturned,
            0x4ad0ee => self.stage == Stage::RegionsDone,
            0x4aaa26 => matches!(self.pending, Some(Request::Color { .. })),
            0x4aaa97 => matches!(self.pending, Some(Request::Depth { .. })),
            0x4ad133 => self.stage == Stage::ColorReturned,
            0x4ad14d => self.stage == Stage::DepthReturned,
            0x4ad19e => self.stage == Stage::Complete,
            _ => false,
        }
    }
}
