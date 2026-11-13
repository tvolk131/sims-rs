//!4ad4bd: ordered diagonal projection, clipping and row dispatch. Pixel row
//! consumers remain saved children. No callback result is manufactured here.
use super::room_masks::{Camera, project};
use serde::{Deserialize, Serialize};

pub struct Inputs<'a> {
    pub size: i32,
    pub bounds: [i32; 4],
    pub camera: &'a Camera,
    pub diagonal: &'a [u8],
    /// Vertical-first packed clip: top,left,bottom,right.
    pub clip: [i16; 4],
    pub fast: u8,
    pub flags: [u8; 3], //6c724a,6c724b,6c7250
}
impl Inputs<'_> {
    fn validate(&self) -> Result<(), String> {
        if !(0..=64).contains(&self.size)
            || self.camera.zoom > 3
            || self.camera.floor_heights.len() <= self.camera.zoom as usize
            || self.diagonal.len() != 128
            || self.bounds.iter().any(|&x| !(0..64).contains(&x))
            || self.bounds[0] > self.bounds[1]
            || self.bounds[2] > self.bounds[3]
        {
            return Err("diagonal scan inputs absent/unsupported".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Fast,
    Ordinary,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub kind: Kind,
    pub point: [i8; 3],
    pub count: i32,
    pub step: [i8; 3],
    pub screen: [i32; 2],
    pub level: i32,
    pub above: u32,
    pub special: u8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Copied,
    Prepared,
    Origin,
    Project,
    Projected,
    Left,
    Right,
    Endpoint,
    Select,
    Tail,
    Second,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    stage: Stage,
    second: bool,
    index: i32,
    size: i32,
    base_height: i32,
    level: i32,
    above: u32,
    special: u8,
    point: [i8; 3],
    step: [i8; 3],
    screen: [i32; 2],
    count: i32,
    pending: Option<Request>,
}
impl Frame {
    pub fn new(level: i32, above: u32, special: u8, step: [i8; 3]) -> Self {
        Self {
            stage: Stage::Entry,
            second: false,
            index: 0,
            size: 0,
            base_height: 0,
            level,
            above,
            special,
            point: [0; 3],
            step,
            screen: [0; 2],
            count: 0,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn pending(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn index(&self) -> i32 {
        self.index
    }
    pub fn point(&self) -> [i8; 3] {
        self.point
    }
    pub fn screen(&self) -> [i32; 2] {
        self.screen
    }
    pub fn step(&self) -> [i8; 3] {
        self.step
    }
    pub fn count(&self) -> i32 {
        self.count
    }
    pub fn second(&self) -> bool {
        self.second
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(0..=64).contains(&self.size)
            || !(0..=64).contains(&self.index)
            || self.pending.is_some() && self.stage != Stage::Select
        {
            return Err("diagonal scan cursor differs".into());
        }
        Ok(())
    }
    pub fn acknowledge(
        &mut self,
        r: &Request,
        point: [i8; 3],
        step: [i8; 3],
    ) -> Result<(), String> {
        if self.pending.as_ref() != Some(r) {
            return Err("diagonal scan row child differs".into());
        }
        self.point = point;
        self.step = step;
        self.pending = None;
        self.stage = Stage::Tail;
        self.validate()
    }
    fn endpoint(&self) -> [i8; 3] {
        [
            self.point[0].wrapping_add(self.step[0].wrapping_mul(self.count as i8)),
            self.point[1].wrapping_add(self.step[1].wrapping_mul(self.count as i8)),
            self.point[2],
        ]
    }
    pub fn advance(&mut self, v: Inputs<'_>) -> Result<bool, String> {
        v.validate()?;
        self.validate()?;
        if self.pending.is_some() || self.complete() {
            return Ok(false);
        }
        let shift = (u32::from(v.camera.zoom) + 4) & 31;
        match self.stage {
            Stage::Entry => self.stage = Stage::Copied,
            Stage::Copied => {
                self.step[2] = 0;
                self.size = v.size;
                self.base_height = v.camera.floor_heights[v.camera.zoom as usize];
                self.stage = Stage::Prepared;
            }
            Stage::Prepared => {
                if self.index >= self.size {
                    self.stage = Stage::Second;
                } else {
                    self.point = [
                        if self.second { self.index as i8 } else { 0 },
                        if self.second {
                            self.size.wrapping_sub(1) as i8
                        } else {
                            self.index as i8
                        },
                        self.level as i8,
                    ];
                    self.stage = Stage::Origin;
                }
            }
            Stage::Origin => {
                self.point[0] = self.point[0].wrapping_add(v.bounds[0] as i8);
                self.point[1] = self.point[1].wrapping_add(v.bounds[2] as i8);
                self.stage = Stage::Project;
            }
            Stage::Project => {
                self.screen = project(v.camera, self.point);
                self.stage = Stage::Projected;
            }
            Stage::Projected => {
                let index = i32::from(self.point[0]) + i32::from(self.point[1]);
                let ceiling = *v
                    .diagonal
                    .get(
                        usize::try_from(index)
                            .map_err(|_| "diagonal elevation index below retained storage")?,
                    )
                    .ok_or("diagonal elevation outside retained storage")?;
                let top = self.screen[1]
                    .wrapping_sub(i32::from(ceiling).wrapping_shl(u32::from(v.camera.zoom) + 1))
                    .wrapping_sub(self.base_height.wrapping_mul(self.level));
                if self.second
                    && self.screen[1].wrapping_add(v.camera.bottom) < i32::from(v.clip[0])
                    || top >= i32::from(v.clip[2])
                {
                    self.stage = Stage::Tail;
                } else {
                    self.count = if self.second {
                        self.size
                            .wrapping_mul(2)
                            .wrapping_sub(i32::from(self.index as i8))
                            .wrapping_sub(i32::from(self.size.wrapping_sub(1) as i8))
                            .wrapping_sub(1)
                    } else {
                        i32::from(self.index as i8).wrapping_add(1)
                    };
                    self.stage = Stage::Left;
                }
            }
            Stage::Left => {
                let dx = i32::from(v.clip[1]).wrapping_sub(self.screen[0]);
                if dx > 0 {
                    let skip = dx.wrapping_shr(shift);
                    if skip != 0 {
                        self.count = self.count.wrapping_sub(skip);
                        for i in 0..2 {
                            self.point[i] =
                                self.point[i].wrapping_add(self.step[i].wrapping_mul(skip as i8));
                        }
                        self.screen[0] = self.screen[0].wrapping_add(skip.wrapping_shl(shift));
                    }
                }
                self.stage = if self.count > 0 {
                    Stage::Right
                } else {
                    Stage::Tail
                };
            }
            Stage::Right => {
                let extra = self
                    .count
                    .wrapping_shl(shift)
                    .wrapping_add(self.screen[0])
                    .wrapping_sub(i32::from(v.clip[3]));
                if extra > 0 {
                    self.count = self
                        .count
                        .wrapping_add(1i32.wrapping_sub(extra.wrapping_shr(shift)));
                    self.stage = Stage::Endpoint;
                } else {
                    self.stage = Stage::Select;
                }
            }
            Stage::Endpoint => {
                let p = self.endpoint().map(i32::from);
                if p[0] < v.bounds[0]
                    || p[0] > v.bounds[1]
                    || p[1] < v.bounds[2]
                    || p[1] > v.bounds[3]
                {
                    self.count = self.count.wrapping_sub(1);
                } else {
                    self.stage = Stage::Select;
                }
            }
            Stage::Select => {
                if self.count <= 0 {
                    self.stage = Stage::Tail;
                } else {
                    let kind =
                        if self.special != 0 && v.fast != 0 && (v.flags[0] != 0 || v.flags[1] != 0)
                        {
                            Some(Kind::Fast)
                        } else if v.flags[2] != 0 {
                            Some(Kind::Ordinary)
                        } else {
                            None
                        };
                    if let Some(kind) = kind {
                        self.pending = Some(Request {
                            kind,
                            point: self.point,
                            count: self.count,
                            step: self.step,
                            screen: self.screen,
                            level: self.level,
                            above: self.above,
                            special: self.special,
                        });
                    } else {
                        self.stage = Stage::Tail;
                    }
                }
            }
            Stage::Tail => {
                self.index = self.index.wrapping_add(1);
                self.stage = Stage::Prepared;
            }
            Stage::Second => {
                if self.second || self.size <= 1 {
                    self.stage = Stage::Complete;
                } else {
                    self.second = true;
                    self.index = 1;
                    self.stage = Stage::Prepared;
                }
            }
            Stage::Complete => unreachable!(),
        }
        self.validate()?;
        Ok(true)
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x4ad4bd => self.stage == Stage::Entry,
            0x4ad4df => self.stage == Stage::Copied,
            0x4ad548 => !self.second && self.stage == Stage::Project,
            0x4ad55b => !self.second && self.stage == Stage::Projected,
            0x4ad680 | 0x4ad6b2 => !self.second && self.stage == Stage::Tail,
            0x4ad702 => self.second && self.stage == Stage::Project,
            0x4ad715 => self.second && self.stage == Stage::Projected,
            0x4ad88f => self.second && self.stage == Stage::Tail,
            0x4ad89b => self.complete(),
            0x4aee4c => self.pending.as_ref().is_some_and(|r| r.kind == Kind::Fast),
            0x4ad8ac => self
                .pending
                .as_ref()
                .is_some_and(|r| r.kind == Kind::Ordinary),
            _ => false,
        }
    }
}
