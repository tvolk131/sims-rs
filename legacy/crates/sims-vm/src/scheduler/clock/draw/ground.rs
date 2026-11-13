//!49ce09 common/flag/position writes. Zoom rebuild and visible drawing remain
//! explicit children. Context publication is a retained reference, not a pointer.
use serde::{Deserialize, Serialize};
pub mod visible;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Common,
    Zoom { zoom: u32 },
    Visible,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Cell,
    D0,
    D1,
    D2,
    D3,
    Clear,
    Mode,
    Zoom,
    Y,
    X,
    Alias,
    Bounds,
    Complete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Store {
    Owner { offset: u32, size: u8, value: u32 },
    ContextAlias,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    point: [i32; 2],
    zoom: u32,
    stage: Stage,
    pending: Option<Request>,
    cell: u32,
    depth: u16,
    alias: bool,
    store: Option<Store>,
    writes: Vec<(u32, u8, u32)>,
}
fn word(raw: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap())
}
impl Frame {
    pub fn new(point: [i32; 2], zoom: u32) -> Self {
        Self {
            point,
            zoom,
            stage: Stage::Entry,
            pending: None,
            cell: 0,
            depth: 0,
            alias: false,
            store: None,
            writes: vec![],
        }
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn alias_written(&self) -> bool {
        self.alias
    }
    pub fn last_store(&self) -> Option<Store> {
        self.store
    }
    pub fn validate(&self, raw: &[u8]) -> Result<(), String> {
        if raw.len() < 0x388 || !(1..=3).contains(&self.zoom) {
            return Err("ground delegate owner differs".into());
        }
        let offsets: &[u32] = match self.stage {
            Stage::Entry | Stage::Cell => &[],
            Stage::D0 => &[0xc],
            Stage::D1 => &[0xc, 0x18],
            Stage::D2 => &[0xc, 0x18, 0x1a],
            Stage::D3 => &[0xc, 0x18, 0x1a, 0x1c],
            Stage::Clear => &[0xc, 0x18, 0x1a, 0x1c, 0x1e],
            Stage::Mode | Stage::Zoom | Stage::X => &[0xc, 0x18, 0x1a, 0x1c, 0x1e, 0x37c],
            Stage::Y => &[0xc, 0x18, 0x1a, 0x1c, 0x1e, 0x37c, 0x384],
            Stage::Alias | Stage::Bounds | Stage::Complete => {
                &[0xc, 0x18, 0x1a, 0x1c, 0x1e, 0x37c, 0x384, 0x380]
            }
        };
        if self.alias != matches!(self.stage, Stage::Bounds | Stage::Complete)
            || !self.writes.iter().map(|w| w.0).eq(offsets.iter().copied())
        {
            return Err("ground delegate retained write lifetime differs".into());
        }
        if let Some(r) = self.pending {
            if !matches!(
                (self.stage, r),
                (Stage::Entry, Request::Common)
                    | (Stage::Zoom, Request::Zoom { .. })
                    | (Stage::Bounds, Request::Visible)
            ) {
                return Err("ground delegate child escaped caller".into());
            }
        }
        for &(offset, size, value) in &self.writes {
            let i = offset as usize;
            let n = usize::from(size);
            if !matches!(
                (offset, size),
                (0xc, 4) | (0x18 | 0x1a | 0x1c | 0x1e, 2) | (0x37c | 0x380 | 0x384, 4)
            ) || raw.get(i..i + n) != Some(&value.to_le_bytes()[..n])
            {
                return Err("ground delegate suspended store differs".into());
            }
        }
        Ok(())
    }
    pub fn common(&mut self, cell: u32, depth: i32) -> Result<(), String> {
        if self.pending != Some(Request::Common) {
            return Err("ground common reply absent".into());
        }
        self.cell = cell;
        self.depth = depth.wrapping_add(0x50) as u16;
        self.pending = None;
        self.store = None;
        self.stage = Stage::Cell;
        Ok(())
    }
    pub fn complete_visible(&mut self)->Result<(),String>{if self.pending!=Some(Request::Visible){return Err("ground visible return has no caller".into());}self.pending=None;self.store=None;self.stage=Stage::Complete;Ok(())}
    fn write(&mut self, raw: &mut [u8], offset: usize, size: usize, value: u32) {
        raw[offset..offset + size].copy_from_slice(&value.to_le_bytes()[..size]);
        self.store = Some(Store::Owner {
            offset: offset as u32,
            size: size as u8,
            value,
        });
        self.writes.retain(|&(o, _, _)| o != offset as u32);
        self.writes.push((offset as u32, size as u8, value));
    }
    pub fn advance(
        &mut self,
        raw: &mut [u8],
        flags: Option<u8>,
        bounds: [i32; 4],
    ) -> Result<bool, String> {
        self.validate(raw)?;
        if self.pending.is_some() || self.complete() {
            return Ok(false);
        }
        self.store = None;
        match self.stage {
            Stage::Entry => self.pending = Some(Request::Common),
            Stage::Cell => {
                self.write(raw, 0xc, 4, self.cell);
                self.stage = Stage::D0;
            }
            Stage::D0 | Stage::D1 | Stage::D2 | Stage::D3 => {
                let n = match self.stage {
                    Stage::D0 => 0,
                    Stage::D1 => 1,
                    Stage::D2 => 2,
                    _ => 3,
                };
                self.write(raw, 0x18 + 2 * n, 2, u32::from(self.depth));
                self.stage = match n {
                    0 => Stage::D1,
                    1 => Stage::D2,
                    2 => Stage::D3,
                    _ => Stage::Clear,
                };
            }
            Stage::Clear => {
                self.write(raw, 0x37c, 4, 0);
                self.stage = Stage::Mode;
            }
            Stage::Mode => {
                if let Some(f) = flags {
                    let mode = if f & 1 != 0 {
                        1
                    } else if f & 2 != 0 {
                        2
                    } else {
                        0
                    };
                    if mode != 0 {
                        self.write(raw, 0x37c, 4, mode);
                    }
                }
                self.stage = Stage::Zoom;
            }
            Stage::Zoom => {
                if word(raw, 0xd8) != self.zoom {
                    self.pending = Some(Request::Zoom { zoom: self.zoom });
                } else {
                    self.stage = Stage::X;
                }
            }
            Stage::X => {
                self.write(raw, 0x384, 4, self.point[0].wrapping_shl(4) as u32);
                self.stage = Stage::Y;
            }
            Stage::Y => {
                self.write(raw, 0x380, 4, self.point[1].wrapping_shl(4) as u32);
                self.stage = Stage::Alias;
            }
            Stage::Alias => {
                self.alias = true;
                self.store = Some(Store::ContextAlias);
                self.stage = Stage::Bounds;
            }
            Stage::Bounds => {
                let [x, y] = self.point.map(|v| i32::from(v as i8));
                if x < bounds[0].wrapping_add(1)
                    || x > bounds[1].wrapping_sub(1)
                    || y < bounds[2].wrapping_add(1)
                    || y > bounds[3].wrapping_sub(1)
                {
                    self.stage = Stage::Complete;
                } else {
                    self.pending = Some(Request::Visible);
                }
            }
            Stage::Complete => return Ok(false),
        }
        self.validate(raw)?;
        Ok(true)
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x49ce09 | 0x49ce29 | 0x4a48eb => self.stage == Stage::Entry,
            0x49ce2e => self.stage == Stage::Clear,
            0x49ce42 => self.stage == Stage::Mode,
            0x49ce7c | 0x49cea4 | 0x49dfbd => self.stage == Stage::Zoom,
            0x49cea9 => self.stage == Stage::X,
            0x49cec1 => self.stage == Stage::Alias,
            0x49cecc => self.stage == Stage::Bounds,
            0x49d237 => self.complete(),
            _ => false,
        }
    }
}
