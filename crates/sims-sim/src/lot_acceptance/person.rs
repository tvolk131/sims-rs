//! 0x4337f6 floor footprint collision. Uses live tile links and cached blocker
//! rectangles; callbacks can change either before enumeration continues.
use crate::{lot_links::Links, spatial};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, World};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Diagnostics {
    pub error: i32,
    pub blocker: Option<ObjectId>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Request {
    Function {
        object: ObjectId,
        other: ObjectId,
    },
    Walls {
        bounds: [i32; 4],
        level: i32,
    },
    /// Both operands have object flag 4. Resolve their catalog marker word 69.
    WallObjects {
        object: ObjectId,
        other: ObjectId,
    },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Reply {
    Function {
        allowed: bool,
        diagnostics: Diagnostics,
    },
    Walls(bool),
    WallObjects {
        both_windows_or_neither: bool,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Collision {
    object: ObjectId,
    position: [i32; 2],
    level: i32,
    bounds: [i32; 4],
    direction: usize,
    current: Option<ObjectId>,
    visited: BTreeSet<ObjectId>,
    phase: u8,
    pending: Option<Request>,
    accepted: Option<bool>,
    diagnostics: Diagnostics,
    ghost: bool,
    exclusion: i16,
    same_wall: bool,
}
fn word(w: &World, id: ObjectId, index: i16) -> Result<i16, String> {
    w.objects
        .get(&id)
        .and_then(|o| o.data.get(&index))
        .copied()
        .ok_or("person collision data absent".into())
}

impl Collision {
    pub(crate) fn binding(&self) -> (ObjectId, [i32; 2], i32) {
        (self.object, self.position, self.level)
    }
    pub(crate) fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn begin(
        w: &mut World,
        links: &Links,
        object: ObjectId,
        position: [i32; 2],
        level: i32,
        insets: u16,
        diagnostics: Diagnostics,
    ) -> Result<Self, String> {
        if w.objects.get(&object).is_none_or(|o| o.person.is_none()) {
            return Err("person collision requires a person".into());
        }
        Self::begin_object(w, links, object, position, level, 0, insets, diagnostics)
    }
    pub fn begin_object(
        w: &mut World,
        links: &Links,
        object: ObjectId,
        position: [i32; 2],
        level: i32,
        size: i16,
        insets: u16,
        diagnostics: Diagnostics,
    ) -> Result<Self, String> {
        links.validate_arrays()?;
        let o = w.objects.get(&object).ok_or("collision object absent")?;
        let person = o.person.as_ref();
        let field = |i| {
            person.map_or(Ok(0), |p| {
                p.get(&i).copied().ok_or("person collision field absent")
            })
        };
        let (p68, p84, extension) = (field(68)?, field(84)?, field(73)?);
        let is_person = person.is_some();
        let carried = w.objects[&object]
            .slots
            .first()
            .copied()
            .flatten()
            .filter(|id| w.objects.contains_key(id));
        let early = p68 & 1 != 0
            || (p84 & 1 != 0 && carried.is_none() && links.capture(w, object)?.container.is_none());
        let bounds = if early {
            [0; 4]
        } else {
            let facing = word(w, object, 1)?;
            if !(0..=7).contains(&facing) {
                return Err("person collision direction outside native footprint contract".into());
            }
            spatial::footprint(
                position,
                is_person,
                (p68 | p84) & 1 != 0,
                size,
                insets,
                extension as u16,
                facing as u8,
            )
        };
        Ok(Self {
            object,
            position,
            level,
            bounds,
            direction: 0,
            current: None,
            visited: BTreeSet::new(),
            phase: 0,
            pending: None,
            accepted: early.then_some(true),
            diagnostics,
            ghost: word(w, object, 8)? as u16 & 4 != 0,
            exclusion: if is_person { 0 } else { word(w, object, 63)? },
            same_wall: false,
        })
    }
    pub fn outcome(&self) -> Option<(bool, Diagnostics)> {
        self.accepted.map(|ok| (ok, self.diagnostics))
    }
    pub fn next(
        &mut self,
        w: &World,
        links: &Links,
        cached_bounds: &BTreeMap<ObjectId, [i32; 4]>,
    ) -> Result<Option<Request>, String> {
        if self.accepted.is_some() {
            return Ok(None);
        }
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            match self.phase {
                0 => {
                    if self.direction == 9 {
                        self.phase = 5;
                        continue;
                    }
                    let (dx, dy) = crate::routing::DIRECTION_OFFSETS
                        .get(self.direction)
                        .copied()
                        .unwrap_or((0, 0));
                    let x = ((self.position[1] >> 4) as i8).wrapping_add((dy / 16) as i8);
                    let y = ((self.position[0] >> 4) as i8).wrapping_add((dx / 16) as i8);
                    self.direction += 1;
                    self.visited.clear();
                    if x < 1
                        || y < 1
                        || x as usize >= links.size - 1
                        || y as usize >= links.size - 1
                    {
                        continue;
                    }
                    self.current = links.head(w, [x, y, self.level as i8]);
                    self.phase = 1;
                }
                1 => {
                    let Some(id) = self.current else {
                        self.phase = 0;
                        continue;
                    };
                    if !self.visited.insert(id) {
                        return Err("cyclic person collision tile chain".into());
                    }
                    let same_object = if w.objects[&self.object].multitile.is_some() {
                        sims_vm::multitile::chain(w, self.object)?.contains(&id)
                    } else {
                        id == self.object
                    };
                    if same_object {
                        self.phase = 4;
                        continue;
                    }
                    let flags = word(w, id, 8)? as u16;
                    let permits_person = |flags: u16| {
                        if flags & 4 != 0 {
                            flags & 2 == 0
                        } else {
                            flags & 16 != 0
                        }
                    };
                    let passes = if w.objects[&self.object].person.is_some() {
                        permits_person(flags)
                    } else if let Some(person) = &w.objects[&id].person {
                        person.get(&68).ok_or("collision person flag absent")? & 1 != 0
                            || permits_person(word(w, self.object, 8)? as u16)
                    } else if !self.ghost && flags & 4 == 0 {
                        false
                    } else {
                        // This branch is tile-local, with no overlap test and
                        // no blocker-pointer assignment. Initial globals survive.
                        if self.direction == 9 {
                            if word(w, id, 63)? & self.exclusion & 1 != 0 {
                                self.diagnostics.error = 11;
                                self.accepted = Some(false);
                                return Ok(None);
                            }
                            if self.ghost && flags & 4 != 0 {
                                let side = |id| wall_side(word(w, id, 13)?, word(w, id, 1)?);
                                let a = side(self.object)?;
                                self.same_wall = a != 0 && a == side(id)?;
                                self.pending = Some(Request::WallObjects {
                                    object: self.object,
                                    other: id,
                                });
                                return Ok(self.pending);
                            }
                        }
                        true
                    };
                    if passes {
                        self.phase = 4;
                        continue;
                    }
                    self.pending = Some(Request::Function {
                        object: self.object,
                        other: id,
                    });
                    self.phase = 2;
                    return Ok(self.pending);
                }
                2 => {
                    self.pending = Some(Request::Function {
                        object: self.current.ok_or("collision object lost")?,
                        other: self.object,
                    });
                    self.phase = 3;
                    return Ok(self.pending);
                }
                3 => {
                    let id = self.current.ok_or("collision object lost")?;
                    // These are cached native bounds, not a fresh footprint
                    // inferred from the blocker after its script has run.
                    let b = cached_bounds
                        .get(&id)
                        .ok_or("native collision bounds absent")?;
                    let a = self.bounds;
                    if b[2] < a[0] && a[2] < b[0] && b[3] < a[1] && a[3] < b[1] {
                        self.diagnostics = Diagnostics {
                            error: 11,
                            blocker: Some(id),
                        };
                        self.accepted = Some(false);
                        return Ok(None);
                    }
                    self.phase = 4;
                }
                4 => {
                    self.current =
                        links.slot(w, self.current.ok_or("collision object lost")?, 0)?;
                    self.phase = 1;
                }
                5 => {
                    if word(w, self.object, 42)? as u16 & 0x400 != 0 {
                        self.accepted = Some(true);
                        return Ok(None);
                    }
                    self.pending = Some(Request::Walls {
                        bounds: self.bounds,
                        level: self.level,
                    });
                    return Ok(self.pending);
                }
                _ => return Err("invalid person collision phase".into()),
            }
        }
    }
    pub fn resume(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        if self.pending != Some(request) || self.accepted.is_some() {
            return Err("person collision request mismatch".into());
        }
        match (request, reply) {
            (
                Request::Function { .. },
                Reply::Function {
                    allowed,
                    diagnostics,
                },
            ) => {
                self.diagnostics = diagnostics;
                if allowed {
                    self.phase = 4;
                }
            }
            (Request::Walls { .. }, Reply::Walls(blocked)) => {
                self.accepted = Some(!blocked);
                if blocked {
                    self.diagnostics.error = 7;
                }
            }
            (
                Request::WallObjects { .. },
                Reply::WallObjects {
                    both_windows_or_neither,
                },
            ) => {
                if both_windows_or_neither && self.same_wall {
                    self.diagnostics.error = 22;
                    self.accepted = Some(false);
                } else {
                    self.phase = 4;
                }
            }
            _ => return Err("person collision reply type mismatch".into()),
        }
        self.pending = None;
        Ok(())
    }
}

/// 0x4358c3, after native wall rotation table initialization. This is the
/// wall-placement mask (data13), not the cached adjoining-wall flags (data5).
pub fn wall_side(mask: i16, facing: i16) -> Result<u8, String> {
    if !(0..=7).contains(&facing) {
        return Err("wall attachment direction outside native table".into());
    }
    let side: u8 = match mask & 5 {
        1 => 2,
        4 => 8,
        _ => return Ok(0),
    };
    let shift = (facing / 2) as u32;
    Ok(((side << shift) | (side >> (4 - shift))) & 15)
}
