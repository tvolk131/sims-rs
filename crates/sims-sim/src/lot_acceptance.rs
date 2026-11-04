//! Native base floor-placement acceptance (0x433311 / 0x4329c1, apply=false).
//! Wall fit and footprint collision are explicit services. This operation does
//! not mutate physical links, run placement callbacks, or commit a location.
use serde::{Deserialize, Serialize};
pub mod contained;
pub mod group;
pub mod live;
pub mod person;

/// Values read by the floor branch, in native physical X/Y order. `occupants`
/// is the live physical subtree's flags, including contained objects. Floor
/// coverings, water and pool regions are separate original arrays.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Input {
    pub size: usize,
    pub position: [i32; 2],
    pub level: i32,
    pub person: bool,
    pub placement_flags: u16,
    pub height_mask: i16,
    pub outside: i16,
    pub water: u8,
    pub pool: u8,
    pub heights: [u8; 4],
    pub covering: u16,
    pub tile_flags: u8,
    pub occupants: Vec<u16>,
}

impl Input {
    /// Resolve a person's floor query from the imported lot's actual arrays
    /// and physical enumeration. The runtime supports the two real floors;
    /// the native checker's level-3 aliasing is diagnostic-only.
    pub fn for_person(
        world: &sims_vm::World,
        placement: &crate::lot_placement::State,
        architecture: &crate::architecture::Architecture,
        object: sims_vm::ObjectId,
        position: [i32; 2],
        level: i32,
    ) -> Result<Self, String> {
        if world
            .objects
            .get(&object)
            .is_none_or(|o| o.person.is_none())
        {
            return Err("person floor query requires a person".into());
        }
        Self::for_object(world, placement, architecture, object, position, level)
    }
    pub fn for_object(
        world: &sims_vm::World,
        placement: &crate::lot_placement::State,
        architecture: &crate::architecture::Architecture,
        object: sims_vm::ObjectId,
        position: [i32; 2],
        level: i32,
    ) -> Result<Self, String> {
        let lot = architecture
            .original
            .as_ref()
            .ok_or("floor acceptance needs original lot")?;
        let links = &placement.links;
        links.validate_arrays()?;
        if links.size != lot.size || !(1..=2).contains(&level) {
            return Err("floor query outside supported original arrays".into());
        }
        let o = world
            .objects
            .get(&object)
            .ok_or("floor acceptance object absent")?;
        let word = |i| {
            o.data
                .get(&i)
                .copied()
                .ok_or("floor acceptance word absent")
        };
        let [column, row] = [(position[1] >> 4) as i8, (position[0] >> 4) as i8];
        if column < 1 || row < 1 || column as usize >= lot.size - 1 || row as usize >= lot.size - 1
        {
            // The native bounds gate returns before reading any tile arrays.
            // These unused environment fields must never create a world tile.
            return Ok(Self {
                size: lot.size,
                position,
                level,
                person: o.person.is_some(),
                placement_flags: word(42)? as u16,
                height_mask: word(4)?,
                outside: word(30)?,
                water: 0,
                pool: 0,
                heights: [0; 4],
                covering: 0,
                tile_flags: 0,
                occupants: vec![],
            });
        }
        if !links.on_lot(position) {
            return Err("wrapped physical coordinate placement adapter required".into());
        }
        let cell = column as usize * lot.size + row as usize;
        let tile = [column, row, level as i8];
        let mut occupants = vec![];
        if let Some(head) = links.head(world, tile) {
            for id in links.walk(world, head)? {
                occupants.push(
                    *world.objects[&id]
                        .data
                        .get(&8)
                        .ok_or("floor occupant flags absent")? as u16,
                );
            }
        }
        let floor = lot
            .floor(level)
            .ok_or("floor acceptance covering array absent")?;
        Ok(Self {
            size: lot.size,
            position,
            level,
            person: o.person.is_some(),
            placement_flags: word(42)? as u16,
            height_mask: word(4)?,
            outside: word(30)?,
            water: *lot.water.get(cell).ok_or("floor water value absent")?,
            pool: *lot.pool.get(cell).ok_or("floor pool value absent")?,
            heights: *lot.heights.get(cell).ok_or("floor terrain value absent")?,
            covering: *floor
                .source
                .coverings
                .get(cell)
                .ok_or("floor covering value absent")?,
            tile_flags: *placement
                .flags
                .get(level as usize - 1)
                .and_then(|f| f.get(cell))
                .ok_or("floor live flag value absent")?,
            occupants,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Outcome {
    pub accepted: bool,
    /// The native global is not cleared by this base checker. A successful
    /// check preserves it; the outer UI placement wrapper clears it separately.
    pub error: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Request {
    /// 0x476f5a, called before other environmental checks for non-persons.
    WallFit,
    /// 0x4a6757, only queried when choosing an unsupported-floor error code.
    ActiveLevel,
    /// 0x4337f6; the height-mask check occurs only after this succeeds.
    Collision,
    /// Shared environmental prefix completed; validate the native slot branch.
    Containment,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Reply {
    Geometry(Outcome),
    ActiveLevel(i32),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Floor {
    input: Input,
    #[serde(default)]
    contained: bool,
    phase: u8,
    error: i32,
    rejection: i32,
    pending: Option<Request>,
    accepted: Option<bool>,
}

impl Floor {
    pub fn new(input: Input, error: i32) -> Result<Self, String> {
        if !(2..=64).contains(&input.size) {
            return Err("invalid floor acceptance lot size".into());
        }
        Ok(Self {
            input,
            contained: false,
            phase: 0,
            error,
            rejection: 0,
            pending: None,
            accepted: None,
        })
    }
    pub fn new_contained(input: Input, error: i32) -> Result<Self, String> {
        let mut state = Self::new(input, error)?;
        state.contained = true;
        Ok(state)
    }
    pub fn outcome(&self) -> Option<Outcome> {
        self.accepted.map(|accepted| Outcome {
            accepted,
            error: self.error,
        })
    }
    fn reject(&mut self, code: i32) {
        self.error = code;
        self.accepted = Some(false);
    }
    pub fn next(&mut self) -> Option<Request> {
        if self.accepted.is_some() {
            return None;
        }
        if let Some(request) = self.pending {
            return Some(request);
        }
        let i = &self.input;
        if self.phase == 0 {
            // The original narrows shifted coordinates to signed bytes before
            // comparing bounds. It does not first clamp the physical integers.
            let x = (i.position[1] >> 4) as i8;
            let y = (i.position[0] >> 4) as i8;
            if x < 1 || y < 1 || x as usize >= i.size - 1 || y as usize >= i.size - 1 {
                self.reject(1);
                return None;
            }
            if !(1..=3).contains(&i.level) {
                self.reject(2);
                return None;
            }
            if i.level >= 2 && i.placement_flags & 0x1000 != 0 {
                self.reject(0x1f);
                return None;
            }
            self.phase = 1;
            if !i.person {
                self.pending = Some(Request::WallFit);
                return self.pending;
            }
        }
        let flags = i.placement_flags;
        if flags & 0x200 == 0 && i.level > 1 && i.covering == 0 {
            self.rejection = 0x12;
            self.pending = Some(Request::ActiveLevel);
            return self.pending;
        }
        if i.level == 1 && i.water != 255 {
            if flags & 4 == 0 && !i.occupants.iter().any(|f| f & 0x1000 != 0) {
                self.reject(0x1d);
                return None;
            }
        } else if flags & 0x2000 != 0 {
            self.reject(0x2f);
            return None;
        }
        if i.level == 1 {
            if flags & 0x800 == 0 && i.pool < 16 {
                self.reject(0x1d);
                return None;
            }
            if flags & 0x1000 != 0 && i.pool > 15 {
                self.reject(0x1e);
                return None;
            }
        }
        if !self.contained && flags & 0x100 == 0 && i.heights.iter().any(|h| *h != i.heights[0]) {
            self.reject(0x11);
            return None;
        }
        if !self.contained && flags & 1 == 0 && (i.covering != 0 || (i.level == 1 && i.pool == 16))
        {
            self.rejection = 0x13;
            self.pending = Some(Request::ActiveLevel);
            return self.pending;
        }
        if !self.contained && flags & 2 == 0 && i.level == 1 && i.covering == 0 {
            self.reject(0x14);
            return None;
        }
        if !self.contained && i.outside == 1 && i.tile_flags & 4 == 0 {
            self.reject(9);
            return None;
        }
        if !self.contained && i.outside == 2 && i.tile_flags & 4 != 0 {
            self.reject(10);
            return None;
        }
        self.pending = Some(if self.contained {
            Request::Containment
        } else {
            Request::Collision
        });
        self.pending
    }
    pub fn resume(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        if self.pending != Some(request) || self.accepted.is_some() {
            return Err("floor acceptance request mismatch".into());
        }
        match (request, reply) {
            (Request::ActiveLevel, Reply::ActiveLevel(level)) => {
                self.reject(self.rejection + if level == self.input.level { 0 } else { 0x17 });
            }
            (
                Request::WallFit | Request::Collision | Request::Containment,
                Reply::Geometry(result),
            ) => {
                self.error = result.error;
                if request == Request::Containment {
                    self.accepted = Some(result.accepted);
                } else if !result.accepted && !(request == Request::WallFit && self.contained) {
                    self.accepted = Some(false);
                } else if request == Request::Collision {
                    if self.input.height_mask & 1 == 0 {
                        self.reject(0x18);
                    } else {
                        self.accepted = Some(true);
                    }
                }
            }
            _ => return Err("floor acceptance reply type mismatch".into()),
        }
        self.pending = None;
        Ok(())
    }
}
