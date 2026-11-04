//! Resumable standing branch of 0x483414. Random draws and facing writes precede
//! each clearance callback; later lattice points must not be prepared early.
use super::{Candidate, CandidateSet, StandingRequest, transform};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct StandingCheck {
    pub position: [i32; 2],
    pub direction: u8,
    pub actor_facing: u8,
    pub(super) score: i32,
    pub(super) flags: u16,
    pub(super) seat: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StandingScan {
    request: StandingRequest,
    relative: u8,
    row: Option<i32>,
    column: i32,
    stop_direction: bool,
    encountered: [bool; 7],
    pending: Option<StandingCheck>,
    done: bool,
    set: CandidateSet,
}
impl StandingScan {
    pub fn new(r: StandingRequest) -> Result<Self, String> {
        validate_request(&r)?;
        let [min, max, optimal] = r.proximity;
        let done = !(1..=512).contains(&min)
            || !(min..=512).contains(&max)
            || !(min..=max).contains(&optimal);
        let set = CandidateSet {
            candidates: vec![],
            maximum: -1,
            result: 0,
            actor_facing: r.actor_facing,
        };
        Ok(Self {
            request: r,
            relative: 0,
            row: None,
            column: 0,
            stop_direction: false,
            encountered: [false; 7],
            pending: None,
            done,
            set,
        })
    }
    pub fn request(&self) -> &StandingRequest {
        &self.request
    }
    pub fn actor_facing(&self) -> u8 {
        self.set.actor_facing
    }
    /// Clearance functions may change the person's direction. Native -3 leaves
    /// that value alone, and other facing modes write only on entering a direction.
    pub fn observe_actor_facing(&mut self, facing: u8) -> Result<(), String> {
        if facing > 7 {
            return Err("invalid standing actor facing".into());
        }
        self.set.actor_facing = facing;
        Ok(())
    }
    pub fn pending(&self) -> Option<StandingCheck> {
        self.pending
    }
    pub fn outcome(&self) -> Option<&CandidateSet> {
        self.done.then_some(&self.set)
    }
    pub fn validate(&self) -> Result<(), String> {
        validate_request(&self.request)?;
        let r = &self.request;
        let invalid_proximity = Self::new(r.clone())?.done;
        if self.relative > 8
            || self.set.actor_facing > 7
            || self.set.candidates.len() > 16384
            || self.set.maximum
                != self
                    .set
                    .candidates
                    .iter()
                    .map(|c| c.score)
                    .fold(-1, i32::max)
            || self.set.candidates.iter().any(|c| {
                c.seat != 0
                    || !c.flags.is_power_of_two()
                    || c.flags > 128
                    || r.flags & u32::from(c.flags) == 0
            })
            || self.done
                && (self.pending.is_some()
                    || self.row.is_some()
                    || !invalid_proximity && self.relative != 8)
            || !self.done && self.relative == 8
            || self.row.is_some_and(|row| {
                !(-513..=0).contains(&row) || self.column < row || self.column > -row + 1
            })
        {
            return Err("invalid standing scan continuation".into());
        }
        if let Some(check) = self.pending {
            let row = self.row.ok_or("standing query without lattice row")?;
            let rotation = if r.flags & 0x200 != 0 { 0 } else { r.rotation };
            let direction = (rotation + self.relative) & 7;
            if check.direction != direction
                || check.seat != 0
                || check.flags != 1 << self.relative
                || check.actor_facing > 7
                || check.position
                    != transform(
                        r.target,
                        ((self.column - 1) * r.resolution) as f32 + r.offset[0],
                        (row * r.resolution) as f32 + r.offset[1],
                        direction,
                    )
            {
                return Err("standing query differs from lattice cursor".into());
            }
        }
        Ok(())
    }

    pub fn next(&mut self, rng: &mut u32) -> Result<Option<StandingCheck>, String> {
        if self.done {
            return Ok(None);
        }
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        let r = &self.request;
        let [min, max, optimal] = r.proximity;
        let rotation = if r.flags & 0x200 != 0 { 0 } else { r.rotation };
        let walls = if r.flags & 0x800 != 0 { 0 } else { r.walls };
        let adjacent = walls | walls.rotate_left(1) | walls.rotate_right(1);
        let weight = r.standing.max(r.ground) * 10;
        let reciprocal = 16.0f32 / r.resolution as f32;
        let low = (-f64::from(max) * f64::from(reciprocal) * 0.0625) as i32;
        let high = (-f64::from(min) * f64::from(reciprocal) * 0.0625) as i32;
        while self.relative < 8 {
            let flag = 1u8 << self.relative;
            let direction = (rotation + self.relative) & 7;
            if self.row.is_none() {
                if r.flags & u32::from(flag) == 0 || walls & flag != 0 {
                    self.encountered[2] |= r.flags & u32::from(flag) != 0 && walls & flag != 0;
                    self.relative += 1;
                    continue;
                }
                self.set.actor_facing = match r.facing {
                    -3 => self.set.actor_facing,
                    -2 => (direction + 4) & 7,
                    -1 => direction,
                    facing => (rotation + facing as u8) & 7,
                };
                self.row = Some(high);
                self.column = high;
                self.stop_direction = false;
            }
            let row = self.row.unwrap();
            if row < low {
                self.relative += 1;
                self.row = None;
                continue;
            }
            let last = -row - i32::from(r.flags & (1 << ((self.relative + 2) & 7)) != 0);
            if self.column > last {
                // Classification 3 stops the next row, not the rest of this row.
                if self.stop_direction {
                    self.relative += 1;
                    self.row = None;
                } else {
                    self.row = Some(row - 1);
                    self.column = row - 1;
                }
                continue;
            }
            let xf = (self.column * r.resolution) as f32;
            let yf = (row * r.resolution) as f32;
            self.column += 1;
            let point = transform(r.target, xf + r.offset[0], yf + r.offset[1], direction);
            let x = (f64::from(xf) + f64::from(r.offset[0])) as i32;
            let y = (f64::from(yf) + f64::from(r.offset[1])) as i32;
            let squared = x * x + y * y;
            if weight == 0 || squared < min * min || squared > max * max {
                continue;
            }
            let radial = f64::from(squared).sqrt() as i32;
            let penalty = (f64::from((radial - optimal).abs()) * f64::from(r.gradient)) as i32;
            let mut score = 100 + weight - penalty;
            if r.flags & 0x2000 != 0 {
                score += (sims_vm::rng::next(rng) % 20) as i32 - 10;
            }
            if adjacent & flag == 0 {
                score += 25;
            }
            if transform(r.target, x as f32, y as f32, direction) == r.preferred {
                score += 20;
            }
            let check = StandingCheck {
                position: point,
                direction,
                actor_facing: self.set.actor_facing,
                score,
                flags: u16::from(flag),
                seat: 0,
            };
            self.pending = Some(check);
            return Ok(Some(check));
        }
        if self.set.candidates.is_empty() {
            self.set.result = if self.encountered[5] {
                13
            } else if self.encountered[1] {
                8
            } else if self.encountered[2] || self.encountered[3] {
                11
            } else if self.encountered[4] {
                12
            } else {
                0
            };
        }
        self.done = true;
        Ok(None)
    }
    pub fn resume(&mut self, check: StandingCheck, classification: u8) -> Result<(), String> {
        if self.pending != Some(check) || classification > 6 || self.done {
            return Err("standing clearance reply mismatch".into());
        }
        if classification == 6 {
            if self.set.candidates.len() == 16384 {
                return Err("native candidate diagnostic limit exceeded".into());
            }
            self.set.maximum = self.set.maximum.max(check.score);
            self.set.candidates.push(Candidate {
                position: check.position,
                score: check.score,
                seat: 0,
                flags: check.flags,
            });
        }
        self.encountered[usize::from(classification)] = true;
        self.stop_direction |= classification == 3;
        self.pending = None;
        Ok(())
    }
}

pub(super) fn validate_request(r: &StandingRequest) -> Result<(), String> {
    if r.flags & !(0xff | 0x100 | 0x200 | 0x800 | 0x2000 | 0xc000) != 0
        || r.flags & 255 == 0
        || r.rotation > 7
        || r.actor_facing > 7
        || !(1..=512).contains(&r.resolution)
        || !r.gradient.is_finite()
        || r.gradient.abs() > 10000.0
        || r.offset.iter().any(|v| !v.is_finite() || v.abs() > 512.0)
        || !(-3..=7).contains(&r.facing)
        || !(-1000..=1000).contains(&r.standing)
        || !(-1000..=1000).contains(&r.ground)
        || r.target
            .iter()
            .chain(&r.preferred)
            .any(|v| v.abs_diff(0) > 32768)
    {
        return Err("unsupported native standing candidate context".into());
    }
    Ok(())
}
