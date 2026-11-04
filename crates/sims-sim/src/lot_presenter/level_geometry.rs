//!54045b's two ordered, indirect geometry vectors. Child drawing is suspended.
//! Headers are read again after each child; their ends are never cached.
use serde::{Deserialize, Serialize};

pub trait Words {
    fn word(&self, address: u32) -> Result<u32, String>;
}
///53bfb5's unsigned orientation dispatch, including its default branch.
pub fn orientation_mask(value: u32) -> u32 {
    match value {
        2 | 3 => 2,
        4 | 5 => 32,
        6 | 7 => 16,
        _ => 1,
    }
}
///53f2bd/461706. Coordinates are wrapping bytes; the level is preserved.
pub fn tile_point(mut point: [u8; 3], orientation: u32) -> [u8; 3] {
    if matches!(orientation, 2 | 4 | 7) {
        point[0] = point[0].wrapping_sub(1);
    }
    if matches!(orientation, 0 | 4 | 6) {
        point[1] = point[1].wrapping_sub(1);
    }
    point
}
///53cd2a's complete unsigned classification.
pub fn special_kind(value: u32) -> bool {
    matches!(value, 2 | 12 | 13 | 14)
}
///4ac14b. Preserve signed wrapping/shift behavior, including its default mask.
pub fn segment_bounds(mask: u32, zoom: u8, floor_height: i32) -> [i32; 4] {
    let z = u32::from(zoom);
    let a = 8i32.wrapping_shl(z);
    let b = 4i32.wrapping_shl(z);
    let mut r = match mask {
        1 => [
            (-1i32).wrapping_sub(a),
            1i32.wrapping_sub(floor_height),
            -1,
            a.wrapping_add(1),
        ],
        2 => [-1, floor_height.wrapping_neg(), a.wrapping_sub(1), a],
        16 => [
            (-1i32).wrapping_sub(a),
            b.wrapping_sub(floor_height),
            a.wrapping_sub(1),
            b,
        ],
        32 => {
            let width = 8i32.wrapping_shr(3u32.wrapping_sub(z));
            [width.wrapping_neg(), floor_height.wrapping_neg(), width, a]
        }
        _ => [0; 4],
    };
    r[1] = r[1].wrapping_add((-1i32).wrapping_shl(z));
    r
}
///53e8c2..53e8f2, after the original OffsetRect call.
pub fn segment_overlaps(rect: [i32; 4], clip: [i32; 4]) -> bool {
    (rect[3].wrapping_sub(clip[1]).wrapping_sub(1)
        | clip[3].wrapping_sub(rect[1]).wrapping_sub(1)
        | clip[2].wrapping_sub(rect[0]).wrapping_sub(1)
        | rect[2].wrapping_sub(clip[0]).wrapping_sub(1))
        >= 0
}
pub fn segment_heights(orientation: u32, h: [u8; 4]) -> [u8; 2] {
    match orientation {
        1 => [h[1], h[0].min(h[1])],
        3 => [h[1], h[2].min(h[1])],
        5 => [h[1], h[3].min(h[1])],
        6 => [h[0], h[0].min(h[2])],
        _ => [h[1]; 2],
    }
}
pub fn project(camera: &super::room_masks::Camera, point: [u8; 3]) -> [i32; 2] {
    super::room_masks::project(camera, point.map(|v| v as i8))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    First,
    Second,
}
impl Kind {
    pub fn entry(self) -> u32 {
        match self {
            Self::First => 0x53db67,
            Self::Second => 0x53c227,
        }
    }
    pub fn return_pc(self) -> u32 {
        match self {
            Self::First => 0x540480,
            Self::Second => 0x5404a7,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub kind: Kind,
    pub model: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Compare,
    Call,
    Returned,
    Second,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    model: u32,
    level: i32,
    kind: Kind,
    stage: Stage,
    header: u32,
    cursor: u32,
    pending: Option<Request>,
}
impl Frame {
    pub fn new(model: u32, level: i32) -> Self {
        Self {
            model,
            level,
            kind: Kind::First,
            stage: Stage::Entry,
            header: 0,
            cursor: 0,
            pending: None,
        }
    }
    pub fn at_entry(&self) -> bool {
        self.stage == Stage::Entry
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn at_child_return(&self, kind: Kind) -> bool {
        self.stage == Stage::Returned && self.kind == kind
    }
    pub fn pending(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn cursor(&self) -> u32 {
        self.cursor
    }
    pub fn header(&self) -> u32 {
        self.header
    }
    pub fn slot(&self) -> u32 {
        self.model
            .wrapping_add((self.level as u32).wrapping_mul(4))
            .wrapping_add(if self.kind == Kind::First { 12 } else { 4 })
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.pending.is_some() != (self.stage == Stage::Call)
            || self.pending.as_ref().is_some_and(|r| r.kind != self.kind)
        {
            return Err("geometry vector lost its actual child".into());
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, request: &Request) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(request) {
            return Err("geometry vector child differs".into());
        }
        self.pending = None;
        self.stage = Stage::Returned;
        self.validate()
    }
    pub fn advance(&mut self, words: &impl Words) -> Result<bool, String> {
        self.validate()?;
        match self.stage {
            Stage::Entry | Stage::Second => {
                let header = words.word(self.slot())?;
                let cursor = words.word(header)?;
                self.header = header;
                self.cursor = cursor;
                self.stage = Stage::Compare;
            }
            Stage::Compare => {
                if self.cursor == words.word(self.header.wrapping_add(4))? {
                    if self.kind == Kind::First {
                        self.kind = Kind::Second;
                        self.stage = Stage::Second;
                    } else {
                        self.stage = Stage::Complete;
                    }
                } else {
                    let model = words.word(self.cursor)?;
                    self.pending = Some(Request {
                        kind: self.kind,
                        model,
                    });
                    self.stage = Stage::Call;
                }
            }
            Stage::Returned => {
                self.header = words.word(self.slot())?;
                self.cursor = self.cursor.wrapping_add(4);
                self.stage = Stage::Compare;
            }
            Stage::Call | Stage::Complete => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
}
