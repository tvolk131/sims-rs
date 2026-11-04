//!4caf4e image publication and4cb040 size caller. Reference changes precede
//! deleting-destructor children; dimensions/layout precede the rectangle call.
pub mod bitmap;
pub mod surface;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
fn word(h: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(h[off..off + 4].try_into().unwrap())
}
fn set(h: &mut [u8], off: usize, v: u32) {
    h[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Destroy { image: u32, flags: u32 },
    Rectangle { window: u32, rectangle: [i32; 4] },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    window: u32,
    incoming: u32,
    pc: u8,
    old: u32,
    width: i32,
    height: i32,
    rectangle: [i32; 4],
    pending: Option<Request>,
}
impl Frame {
    pub fn new(window: u32, image: u32) -> Self {
        Self {
            window,
            incoming: image,
            pc: 0,
            old: 0,
            width: 0,
            height: 0,
            rectangle: [0; 4],
            pending: None,
        }
    }
    pub fn arguments(&self) -> (u32, u32) {
        (self.window, self.incoming)
    }
    pub fn complete(&self) -> bool {
        self.pc == 7
    }
    pub fn pending(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    /// Account for retained incoming/released old references before the wrapper
    /// assignment commits, including a cut immediately after child return.
    pub fn adjustments(&self) -> Vec<(u32, i32)> {
        match self.pc {
            1 => vec![(self.old, -1), (self.incoming, 1)],
            3 | 5 => vec![(self.old, -1)],
            _ => vec![],
        }
    }
    pub fn validate(&self, header: &[u8], counts: &BTreeMap<u32, u32>) -> Result<(), String> {
        if self.window == 0 || header.len() < 0x124 || self.pc > 7 {
            return Err("image publication owner/cursor absent".into());
        }
        if let Some(request) = &self.pending {
            let good = match request {
                Request::Destroy { image, flags: 1 } => {
                    matches!(self.pc, 1 | 3 | 5)
                        && *image == self.old
                        && *image != 0
                        && word(header, 0xdc) == *image
                        && counts.get(image) == Some(&0)
                }
                Request::Rectangle { window, rectangle } => {
                    self.pc == 6 && *window == self.window && *rectangle == self.rectangle
                }
                _ => false,
            };
            if !good {
                return Err("image publication child differs from retained wrapper/cursor".into());
            }
        }
        if self.complete() && self.pending.is_some() {
            return Err("completed image publication retains a child".into());
        }
        Ok(())
    }
    fn release(
        &mut self,
        header: &[u8],
        counts: &mut BTreeMap<u32, u32>,
    ) -> Result<Option<Request>, String> {
        self.old = word(header, 0xdc);
        if self.old == 0 {
            return Ok(None);
        }
        let count = counts
            .get_mut(&self.old)
            .ok_or("published image reference owner absent")?;
        *count = count.wrapping_sub(1);
        Ok((*count == 0).then_some(Request::Destroy {
            image: self.old,
            flags: 1,
        }))
    }
    pub fn advance(
        &mut self,
        header: &mut [u8],
        counts: &mut BTreeMap<u32, u32>,
        sizes: &BTreeMap<u32, [i32; 2]>,
    ) -> Result<Option<Request>, String> {
        self.validate(header, counts)?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        let request = loop {
            match self.pc {
                0 => {
                    if self.incoming != 0 {
                        let count = counts
                            .get_mut(&self.incoming)
                            .ok_or("incoming image reference owner absent")?;
                        *count = count.wrapping_add(1);
                    }
                    self.pc = 1;
                    if let Some(r) = self.release(header, counts)? {
                        break r;
                    }
                }
                1 => {
                    set(header, 0xdc, self.incoming);
                    if self.incoming == 0 {
                        self.width = 45;
                        self.height = 45;
                        self.pc = 6;
                        continue;
                    }
                    [self.width, self.height] = *sizes
                        .get(&self.incoming)
                        .ok_or("published image dimensions absent")?;
                    set(header, 0x118, 0);
                    set(header, 0x11c, 0);
                    self.pc = 2;
                }
                2 => {
                    let columns = match self.width {
                        45 => 1,
                        180 => 4,
                        270 => 6,
                        _ => 0,
                    };
                    if columns == 0 {
                        self.pc = 3;
                        if let Some(r) = self.release(header, counts)? {
                            break r;
                        }
                    } else {
                        set(header, 0x114, columns);
                        self.pc = 4;
                    }
                }
                3 => {
                    set(header, 0xdc, 0);
                    self.pc = 4;
                }
                4 => match self.height {
                    45 => {
                        set(header, 0x110, 1);
                        self.pc = 6;
                    }
                    135 => {
                        set(header, 0x110, 3);
                        set(header, 0x118, 1);
                        self.pc = 6;
                    }
                    _ => {
                        self.pc = 5;
                        if let Some(r) = self.release(header, counts)? {
                            break r;
                        }
                    }
                },
                5 => {
                    set(header, 0xdc, 0);
                    self.pc = 6;
                }
                6 => {
                    let [width, height] = if self.incoming == 0 {
                        [45, 45]
                    } else {
                        [
                            self.width
                                .checked_div(word(header, 0x114) as i32)
                                .ok_or("native image column division fault")?,
                            self.height
                                .checked_div(word(header, 0x110) as i32)
                                .ok_or("native image row division fault")?,
                        ]
                    };
                    let x = word(header, 0x7c) as i32;
                    let y = word(header, 0x80) as i32;
                    self.rectangle = [x, y, x.wrapping_add(width), y.wrapping_add(height)];
                    self.pc = 6;
                    break Request::Rectangle {
                        window: self.window,
                        rectangle: self.rectangle,
                    };
                }
                7 => return Ok(None),
                _ => unreachable!(),
            }
        };
        self.pending = Some(request.clone());
        Ok(Some(request))
    }
    pub fn acknowledge(
        &mut self,
        header: &[u8],
        counts: &BTreeMap<u32, u32>,
        request: &Request,
    ) -> Result<(), String> {
        self.validate(header, counts)?;
        if self.pending.as_ref() != Some(request) {
            return Err("image publication reply differs".into());
        }
        if matches!(request, Request::Rectangle { .. }) {
            self.pc = 7;
        }
        self.pending = None;
        Ok(())
    }
}
