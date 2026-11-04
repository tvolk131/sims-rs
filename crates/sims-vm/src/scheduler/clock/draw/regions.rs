//!5cdf18 rectangle registration. Ordered removals retain unused allocation
//! bytes. Append/allocation and the5ccfe0 coalescer are explicit child services.
use serde::{Deserialize, Serialize};
mod coalesce;
pub type Rect = [i32; 4];
fn word(h: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(h[o..o + 4].try_into().unwrap())
}
fn set(h: &mut [u8], o: usize, v: u32) {
    h[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct List {
    pub header: Vec<u8>,
    pub backing: Vec<u8>,
}
impl List {
    ///5ccfe0's cached/short paths and pairwise separated rectangle path.
    /// Overlap/splitting and numerically ambiguous decisions remain children.
    /// A false reply leaves every byte unchanged.
    pub fn coalesce_separated(&mut self, enabled: u32, weight: f32) -> Result<bool, String> {
        self.validate()?;
        if enabled == 0 {
            return Ok(true);
        }
        if self.header[0xc] != 0 || self.len() < 2 {
            self.header[0xc] = 1;
            return Ok(true);
        }
        if !weight.is_finite() {
            return Ok(false);
        }
        for i in 0..self.len() {
            let a = self.rectangle(i)?;
            if a[0] >= a[2] || a[1] >= a[3] {
                return Ok(false);
            }
            for j in 0..i {
                let b = self.rectangle(j)?;
                if !(a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1]) {
                    return Ok(false);
                }
            }
        }
        self.finish_coalescing(weight)
    }
    fn finish_coalescing(&mut self, weight: f32) -> Result<bool, String> {
        if self.len() < 2 {
            self.header[0xc] = 1;
            return Ok(true);
        }
        let mut bounds = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
        let mut area = 0f32;
        for i in 0..self.len() {
            let a = self.rectangle(i)?;
            bounds[0] = bounds[0].min(a[0]);
            bounds[1] = bounds[1].min(a[1]);
            bounds[2] = bounds[2].max(a[2]);
            bounds[3] = bounds[3].max(a[3]);
            let size = a[2]
                .wrapping_sub(a[0])
                .wrapping_mul(a[3].wrapping_sub(a[1]));
            area = (f64::from(area) + f64::from(size)) as f32;
        }
        let union = bounds[2]
            .wrapping_sub(bounds[0])
            .wrapping_mul(bounds[3].wrapping_sub(bounds[1]));
        let epsilon = f64::from(f32::from_bits(0x358637bd));
        let merge = if f64::from(weight.abs()) > epsilon {
            if area == 0. {
                return Ok(false);
            }
            let ratio = f64::from(union) / f64::from(area);
            // Native compares its x87 quotient with the double1.1. Keep a
            // conservative boundary for precision/rounding control modes.
            if !ratio.is_finite() || (ratio - 1.1).abs() < 1e-6 {
                return Ok(false);
            }
            ratio < 1.1
        } else {
            (f64::from(union) - f64::from(area)).abs() <= epsilon
        };
        if merge {
            let begin = word(&self.header, 0);
            set(&mut self.header, 4, begin);
            if !self.append(bounds)? {
                return Err("coalescer lost existing allocation".into());
            }
            set(&mut self.header, 0x10, 0);
        }
        self.header[0xc] = 1;
        Ok(true)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.header.len() != 24 {
            return Err("rectangle vector header absent".into());
        }
        let (b, e, c) = (
            word(&self.header, 0),
            word(&self.header, 4),
            word(&self.header, 8),
        );
        if b > e
            || e > c
            || (e - b) % 16 != 0
            || (c - b) % 16 != 0
            || self.backing.len() != (c - b) as usize
            || c - b > 65536
            || b == 0 && c != 0
        {
            return Err("rectangle allocation extent differs".into());
        }
        Ok(())
    }
    pub fn len(&self) -> usize {
        ((word(&self.header, 4) - word(&self.header, 0)) / 16) as usize
    }
    pub fn rectangle(&self, index: usize) -> Result<Rect, String> {
        self.validate()?;
        if index >= self.len() {
            return Err("rectangle index absent".into());
        }
        Ok(std::array::from_fn(|i| {
            word(&self.backing, index * 16 + i * 4) as i32
        }))
    }
    pub fn append(&mut self, r: Rect) -> Result<bool, String> {
        self.validate()?;
        let at = self.len() * 16;
        if at == self.backing.len() {
            return Ok(false);
        }
        for (i, v) in r.iter().enumerate() {
            set(&mut self.backing, at + i * 4, *v as u32);
        }
        let e = word(&self.header, 4).wrapping_add(16);
        set(&mut self.header, 4, e);
        Ok(true)
    }
    fn erase(&mut self, index: usize) -> Result<(), String> {
        self.validate()?;
        let n = self.len();
        if index >= n {
            return Err("rectangle erase index absent".into());
        }
        self.backing
            .copy_within((index + 1) * 16..n * 16, index * 16);
        let e = word(&self.header, 4) - 16;
        set(&mut self.header, 4, e);
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Append { rectangle: Rect },
    Coalesce,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Scan,
    Append,
    Increment,
    Coalesce,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    input: Rect,
    rectangle: Rect,
    index: usize,
    before: usize,
    stage: Stage,
    result: Option<bool>,
    pending: Option<Request>,
}
impl Frame {
    pub fn new(input: Rect) -> Self {
        Self {
            input,
            rectangle: input,
            index: 0,
            before: 0,
            stage: Stage::Entry,
            result: None,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn result(&self) -> Option<bool> {
        self.result
    }
    pub fn pending(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn validate(&self, list: &List) -> Result<(), String> {
        list.validate()?;
        if self.index > 4096 || self.before > 4096 || self.complete() != self.result.is_some() {
            return Err("rectangle caller cursor differs".into());
        }
        if let Some(r) = &self.pending {
            if !matches!((self.stage,r),(Stage::Append,Request::Append{rectangle})if *rectangle==self.rectangle)
                && !matches!((self.stage, r), (Stage::Coalesce, Request::Coalesce))
            {
                return Err("rectangle child differs".into());
            }
        }
        Ok(())
    }
    pub fn advance(&mut self, list: &mut List, enabled: u32) -> Result<Option<Request>, String> {
        self.validate(list)?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        for _ in 0..8194 {
            match self.stage {
                Stage::Entry => {
                    if self.input[0] >= self.input[2]
                        || self.input[1] >= self.input[3]
                        || enabled == 0
                    {
                        self.result = Some(false);
                        self.stage = Stage::Complete;
                        continue;
                    }
                    self.rectangle[0] &= !3;
                    self.rectangle[2] = self.rectangle[2].wrapping_add(3) & !3;
                    self.stage = Stage::Scan;
                }
                Stage::Scan => {
                    if self.index < list.len() {
                        let old = list.rectangle(self.index)?;
                        let r = self.rectangle;
                        if r[0] >= old[0] && old[2] >= r[2] && old[1] <= r[1] && old[3] >= r[3] {
                            self.result = Some(false);
                            self.stage = Stage::Complete;
                            continue;
                        }
                        if r[0] <= old[0] && r[2] >= old[2] && r[1] <= old[1] && r[3] >= old[3] {
                            list.erase(self.index)?;
                        } else {
                            self.index += 1;
                        }
                        continue;
                    }
                    let count = (list.len() as i32).min(word(&list.header, 0x10) as i32);
                    list.header[0xc] = 0;
                    set(&mut list.header, 0x10, count as u32);
                    self.before = list.len();
                    self.stage = Stage::Append;
                }
                Stage::Append => {
                    let r = Request::Append {
                        rectangle: self.rectangle,
                    };
                    self.pending = Some(r.clone());
                    return Ok(Some(r));
                }
                Stage::Increment => {
                    let count = word(&list.header, 0x10).wrapping_add(1);
                    set(&mut list.header, 0x10, count);
                    if count as i32 > 8 {
                        self.stage = Stage::Coalesce;
                    } else {
                        self.result = Some(true);
                        self.stage = Stage::Complete;
                    }
                }
                Stage::Coalesce => {
                    self.pending = Some(Request::Coalesce);
                    return Ok(self.pending.clone());
                }
                Stage::Complete => return Ok(None),
            }
        }
        Err("rectangle caller iteration budget".into())
    }
    pub fn acknowledge(&mut self, list: &mut List, r: &Request) -> Result<(), String> {
        self.validate(list)?;
        if self.pending.as_ref() != Some(r) {
            return Err("rectangle reply is not pending".into());
        }
        match self.stage {
            Stage::Append => {
                if list.len() != self.before + 1 || list.rectangle(self.before)? != self.rectangle {
                    return Err("rectangle append did not publish its child".into());
                }
                self.stage = Stage::Increment;
            }
            Stage::Coalesce => {
                set(&mut list.header, 0x10, 0);
                self.result = Some(true);
                self.stage = Stage::Complete;
            }
            _ => return Err("rectangle reply outside child".into()),
        }
        self.pending = None;
        self.validate(list)
    }
}
