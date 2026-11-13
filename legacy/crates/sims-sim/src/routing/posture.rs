//! Resumable mixed standing/seated branch of 0x483414. Seat lookup is a
//! synchronous read; clearance may run scripts and suspend at any instruction.
use super::{Candidate, CandidateSet, Point, Seat, StandingCheck, StandingRequest, transform};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PostureScan {
    request: StandingRequest,
    sitting: i32,
    relative: u8,
    row: Option<i32>,
    column: i32,
    approach: i8,
    stop_direction: bool,
    encountered: [bool; 7],
    occupied_seat: bool,
    blocked_object: i16,
    pending: Option<StandingCheck>,
    done: bool,
    set: CandidateSet,
}
impl PostureScan {
    pub fn new(
        request: StandingRequest,
        sitting: i32,
        blocked_object: i16,
    ) -> Result<Self, String> {
        super::standing::validate_request(&request)?;
        if !(1..=1000).contains(&sitting) {
            return Err("unsupported sitting weight".into());
        }
        let [min, max, optimal] = request.proximity;
        let done = !(1..=512).contains(&min)
            || !(min..=512).contains(&max)
            || !(min..=max).contains(&optimal);
        let facing = request.actor_facing;
        Ok(Self {
            request,
            sitting,
            relative: 0,
            row: None,
            column: 0,
            approach: -1,
            stop_direction: false,
            encountered: [false; 7],
            occupied_seat: false,
            blocked_object,
            pending: None,
            done,
            set: CandidateSet {
                candidates: vec![],
                maximum: -1,
                result: 0,
                actor_facing: facing,
            },
        })
    }
    pub fn request(&self) -> &StandingRequest {
        &self.request
    }
    pub fn sitting(&self) -> i32 {
        self.sitting
    }
    pub fn actor_facing(&self) -> u8 {
        self.set.actor_facing
    }
    pub fn blocked_object(&self) -> i16 {
        self.blocked_object
    }
    pub fn observe_blocker(&mut self, value: i16) {
        self.blocked_object = value;
    }
    pub fn observe_actor_facing(&mut self, value: u8) -> Result<(), String> {
        if value > 7 {
            return Err("invalid posture actor facing".into());
        }
        self.set.actor_facing = value;
        Ok(())
    }
    pub fn pending(&self) -> Option<StandingCheck> {
        self.pending
    }
    pub fn outcome(&self) -> Option<&CandidateSet> {
        self.done.then_some(&self.set)
    }
    pub fn validate(&self) -> Result<(), String> {
        let invalid = Self::new(self.request.clone(), self.sitting, 0)?.done;
        if self.relative > 8
            || self.set.actor_facing > 7
            || !(-1..=9).contains(&self.approach)
            || self.set.candidates.len() > 16384
            || self.set.maximum
                != self
                    .set
                    .candidates
                    .iter()
                    .map(|c| c.score)
                    .fold(-1, i32::max)
            || self
                .set
                .candidates
                .iter()
                .any(|c| !c.flags.is_power_of_two() || c.flags > 128 || c.seat < 0)
            || self.done
                && (self.pending.is_some() || self.row.is_some() || !invalid && self.relative != 8)
            || !self.done && self.relative == 8
            || self.row.is_some_and(|r| {
                !(-33..=-1).contains(&r) || self.column < r || self.column > -r + 1
            })
        {
            return Err("invalid posture scan continuation".into());
        }
        if let Some(check) = self.pending {
            let row = self.row.ok_or("posture query without lattice row")?;
            let r = &self.request;
            let rotation = if r.flags & 512 != 0 { 0 } else { r.rotation };
            let direction = (rotation + self.relative) & 7;
            let approach = self.approach - 1;
            let flags = if check.seat == 0 {
                1 << self.relative
            } else {
                if !(0..8).contains(&approach) {
                    return Err("posture seat approach invalid".into());
                }
                1 << approach
            };
            if check.direction != direction
                || check.actor_facing > 7
                || check.flags != flags
                || check.position
                    != transform(
                        r.target,
                        (self.column * 16) as f32 + r.offset[0],
                        (row * 16) as f32 + r.offset[1],
                        direction,
                    )
            {
                return Err("posture query differs from lattice cursor".into());
            }
        }
        Ok(())
    }
    pub fn next(
        &mut self,
        rng: &mut u32,
        mut lookup: impl FnMut(Point, u8) -> Result<Option<Seat>, String>,
    ) -> Result<Option<StandingCheck>, String> {
        if self.done {
            return Ok(None);
        }
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        let r = &self.request;
        let [min, max, optimal] = r.proximity;
        let rotation = if r.flags & 512 != 0 { 0 } else { r.rotation };
        let walls = if r.flags & 2048 != 0 { 0 } else { r.walls };
        let adjacent = walls | walls.rotate_left(1) | walls.rotate_right(1);
        let standing = r.standing.max(r.ground) * 10;
        let sitting = self.sitting * 10;
        let low = -max / 16 - 1;
        let high = (-min / 16 + 1).min(-1);
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
                    n => (rotation + n as u8) & 7,
                };
                self.row = Some(high);
                self.column = high;
                self.approach = -1;
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
                if self.stop_direction {
                    self.relative += 1;
                    self.row = None;
                } else {
                    self.row = Some(row - 1);
                    self.column = row - 1;
                    self.approach = -1;
                }
                continue;
            }
            if self.approach > 8 {
                self.column += 1;
                self.approach = -1;
                continue;
            }
            let approach = self.approach;
            let point = transform(
                r.target,
                (self.column * 16) as f32 + r.offset[0],
                (row * 16) as f32 + r.offset[1],
                direction,
            );
            let mut x = (f64::from(self.column * 16) + f64::from(r.offset[0])) as i32;
            let mut y = (f64::from(row * 16) + f64::from(r.offset[1])) as i32;
            let choice = if approach == -1 || approach == 8 {
                (approach == if standing < sitting { 8 } else { -1 }).then_some((
                    standing,
                    0,
                    u16::from(flag),
                ))
            } else {
                let (dy, dx) = super::DIRECTION_OFFSETS[approach as usize];
                x += dx;
                y += dy;
                let tx = x >> 4;
                let ty = y >> 4;
                if if ty == row {
                    tx < row || tx > last
                } else {
                    ty > row && (tx <= row || tx >= last)
                } {
                    self.approach += 1;
                    continue;
                }
                let center = transform(r.target, x as f32, y as f32, direction);
                let seat = lookup(Point::new(center[0], center[1]), self.set.actor_facing)?;
                let entry = 1u16 << approach;
                seat.filter(|s| {
                    s.sit && s.facing == i16::from((direction + 4) & 7) && s.allowed & entry != 0
                })
                .and_then(|s| {
                    if s.other_occupant != 0 {
                        self.occupied_seat = true;
                        self.blocked_object = s.other_occupant;
                        None
                    } else {
                        Some((sitting, s.id, entry))
                    }
                })
            };
            self.approach += 1;
            let Some((weight, seat, flags)) = choice else {
                continue;
            };
            let squared = x * x + y * y;
            if weight == 0 || squared < min * min || squared > max * max {
                continue;
            }
            let radial = f64::from(squared).sqrt() as i32;
            let penalty = (f64::from((radial - optimal).abs()) * f64::from(r.gradient)) as i32;
            let mut score = 100 + weight - penalty;
            if r.flags & 8192 != 0 {
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
                flags,
                seat,
            };
            self.pending = Some(check);
            return Ok(Some(check));
        }
        if self.set.candidates.is_empty() {
            self.set.result = if self.occupied_seat {
                9
            } else if self.encountered[5] {
                13
            } else if self.encountered[1] {
                8
            } else if self.encountered[2] || self.encountered[3] {
                11
            } else if self.encountered[4] {
                12
            } else if standing == 0 {
                10
            } else {
                0
            };
        }
        self.done = true;
        Ok(None)
    }
    pub fn resume(&mut self, check: StandingCheck, classification: u8) -> Result<(), String> {
        if self.pending != Some(check) || classification > 6 || self.done {
            return Err("posture clearance reply mismatch".into());
        }
        if classification == 6 {
            if self.set.candidates.len() == 16384 {
                return Err("native candidate diagnostic limit exceeded".into());
            }
            self.set.maximum = self.set.maximum.max(check.score);
            self.set.candidates.push(Candidate {
                position: check.position,
                score: check.score,
                seat: check.seat,
                flags: check.flags,
            });
            self.column += 1;
            self.approach = -1;
        }
        self.encountered[classification as usize] = true;
        self.stop_direction |= classification == 3;
        self.pending = None;
        Ok(())
    }
}
