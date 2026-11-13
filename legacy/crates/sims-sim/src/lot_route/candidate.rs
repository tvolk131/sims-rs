//! Ordered world services for native destination clearance (0x481439).
//! The check can be suspended around the floor query's original function-5
//! callbacks. No physical placement or path search is performed here.
use crate::{lot_acceptance::person::Diagnostics, spatial::PlacementResult};
use serde::{Deserialize, Serialize};
use sims_vm::ObjectId;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Input {
    pub size: usize,
    /// Native physical X/Y, before signed-byte tile narrowing.
    pub position: [i32; 2],
    pub target: [i32; 2],
    pub level: i32,
    pub room: i16,
    pub flags: u32,
    pub direction: Option<u8>,
    pub ignored: Option<ObjectId>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Request {
    Room([i8; 3]),
    Walls { tile: [i8; 3], rotation: u8 },
    Heights { candidate: [i8; 2], target: [i8; 2] },
    Placement { position: [i32; 2], level: i32 },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Reply {
    Room(i16),
    Walls(u8),
    Heights { candidate: [u8; 4], target: [u8; 4] },
    Placement(PlacementResult),
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Outcome {
    pub classification: u8,
    pub diagnostics: Diagnostics,
    /// Routing context +0x8c, distinct from the global placement blocker.
    pub blocked_object: i16,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Check {
    input: Input,
    phase: u8,
    pending: Option<Request>,
    diagnostics: Diagnostics,
    blocked_object: i16,
    classification: Option<u8>,
}
fn tile(p: [i32; 2]) -> [i8; 2] {
    [(p[1] >> 4) as i8, (p[0] >> 4) as i8]
}
impl Check {
    pub(crate) fn input(&self) -> &Input {
        &self.input
    }
    pub(crate) fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn new(
        input: Input,
        diagnostics: Diagnostics,
        blocked_object: i16,
    ) -> Result<Self, String> {
        if !(2..=64).contains(&input.size)
            || !(1..=2).contains(&input.level)
            || input.direction.is_some_and(|d| d > 7)
        {
            return Err("unsupported native candidate clearance input".into());
        }
        Ok(Self {
            input,
            phase: 0,
            pending: None,
            diagnostics,
            blocked_object,
            classification: None,
        })
    }
    pub fn diagnostics(&self) -> Diagnostics {
        self.diagnostics
    }
    pub fn outcome(&self) -> Option<Outcome> {
        self.classification.map(|classification| Outcome {
            classification,
            diagnostics: self.diagnostics,
            blocked_object: self.blocked_object,
        })
    }
    pub fn next(&mut self) -> Option<Request> {
        if self.classification.is_some() {
            return None;
        }
        if self.pending.is_some() {
            return self.pending;
        }
        let i = &self.input;
        let [column, row] = tile(i.position);
        let at = [column, row, i.level as i8];
        loop {
            let request = match self.phase {
                0 => {
                    if column < 1
                        || row < 1
                        || column as usize >= i.size - 1
                        || row as usize >= i.size - 1
                    {
                        self.classification = Some(0);
                        return None;
                    }
                    self.phase = 1;
                    (i.flags & 0x800 == 0).then_some(Request::Room(at))
                }
                1 => {
                    self.phase = 2;
                    i.direction
                        .filter(|_| i.flags & 0x800 == 0)
                        .map(|d| Request::Walls {
                            tile: at,
                            rotation: d.wrapping_sub(4) & 7,
                        })
                }
                2 => {
                    self.phase = 3;
                    (i.flags & 0x8000 == 0).then_some(Request::Heights {
                        candidate: [column, row],
                        target: tile(i.target),
                    })
                }
                3 => {
                    // The native checker clears only the placement blocker,
                    // and only after the room/wall/height gates have passed.
                    self.diagnostics.blocker = None;
                    self.phase = 4;
                    Some(Request::Placement {
                        position: i.position,
                        level: i.level,
                    })
                }
                _ => return None,
            };
            if request.is_some() {
                self.pending = request;
                return request;
            }
        }
    }
    pub fn resume(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        if self.pending != Some(request) || self.classification.is_some() {
            return Err("candidate clearance request mismatch".into());
        }
        match (request, reply) {
            (Request::Room(_), Reply::Room(room)) => {
                if room != self.input.room {
                    self.classification = Some(2);
                }
            }
            (Request::Walls { .. }, Reply::Walls(mask)) => {
                if mask & 1 != 0 || self.input.direction.unwrap() & 1 != 0 && mask & 0x82 != 0 {
                    self.classification = Some(3);
                }
            }
            (Request::Heights { .. }, Reply::Heights { candidate, target }) => {
                if candidate != target || candidate.iter().any(|h| *h != candidate[0]) {
                    self.classification = Some(4);
                }
            }
            (Request::Placement { .. }, Reply::Placement(result)) => {
                self.diagnostics = Diagnostics {
                    error: result.error,
                    blocker: result.blocker.map(|(id, _)| id),
                };
                let (classification, blocker) = crate::spatial::classify(
                    false,
                    0x8800,
                    true,
                    -1,
                    0,
                    0,
                    0,
                    result,
                    self.input.ignored,
                );
                self.classification = Some(classification);
                if let Some(id) = blocker {
                    self.blocked_object = id;
                }
            }
            _ => return Err("candidate clearance reply type mismatch".into()),
        }
        self.pending = None;
        Ok(())
    }

    /// Bind the non-script services to the original arrays and physical object
    /// iterator. The caller must execute Placement through native floor checking.
    pub fn world_reply(
        request: Request,
        world: &sims_vm::World,
        links: &crate::lot_links::Links,
        lot: &crate::lot_geometry::Lot,
    ) -> Result<Option<Reply>, String> {
        let cell = |[column, row]: [i8; 2]| {
            if column < 0 || row < 0 || column as usize >= lot.size || row as usize >= lot.size {
                Err("candidate world query outside original arrays")
            } else {
                Ok(column as usize * lot.size + row as usize)
            }
        };
        Ok(Some(match request {
            Request::Room([x, y, level]) => Reply::Room(
                lot.floor(i32::from(level))
                    .ok_or("candidate room floor absent")?
                    .rooms
                    .primary[cell([x, y])?],
            ),
            Request::Walls { tile, rotation } => {
                let [x, y, level] = tile;
                //4359ea calls433087 before touching either walls or slots.
                //That gate excludes the active border as well as off-map tiles.
                let active = super::active_size(world, links.size)?;
                let border = active.checked_sub(1).ok_or("empty native route extent")?;
                if x < 1 || y < 1 || x as usize >= border || y as usize >= border {
                    return Ok(Some(Reply::Walls(0)));
                }
                let i = cell([x, y])?;
                let floor = lot
                    .floor(i32::from(level))
                    .ok_or("candidate wall floor absent")?;
                let mut object_mask = 0;
                if let Some(head) = links.head(world, tile) {
                    for id in links.walk(world, head)? {
                        let o = &world.objects[&id];
                        let mask = *o.data.get(&53).ok_or("route object wall mask absent")?;
                        if mask != 0 {
                            let facing = *o.data.get(&1).ok_or("route object facing absent")?;
                            object_mask |= crate::spatial::object_wall_mask(mask, facing);
                        }
                    }
                }
                Reply::Walls(crate::spatial::wall_mask(
                    floor.source.walls[i].segments,
                    object_mask,
                    rotation,
                ))
            }
            Request::Heights { candidate, target } => Reply::Heights {
                candidate: lot.heights[cell(candidate)?],
                target: lot.heights[cell(target)?],
            },
            Request::Placement { .. } => return Ok(None),
        }))
    }
}

impl crate::Simulation {
    /// Shared resumable 0x481439 adapter for departure and destination scans.
    pub(crate) fn drive_candidate_clearance(
        &mut self,
        p: &sims_vm::Program,
        caller: ObjectId,
        check: &mut Check,
        floor: &mut Option<crate::lot_acceptance::live::Execution>,
        remaining: &mut usize,
        step: &mut crate::Step,
    ) -> Result<bool, String> {
        loop {
            if let Some(mut execution) = floor.take() {
                let result = self.drive_native_acceptance(p, &mut execution, remaining, step);
                *floor = Some(execution);
                if !result? {
                    return Ok(false);
                }
                let outcome = floor
                    .as_ref()
                    .unwrap()
                    .outcome()
                    .ok_or("native route floor result absent")?;
                let diagnostics = self.native_lot.as_ref().unwrap().diagnostics;
                let blocker = diagnostics
                    .blocker
                    .map(|id| {
                        let d = self
                            .vm
                            .world
                            .objects
                            .get(&id)
                            .and_then(|o| o.definition.as_ref())
                            .ok_or("native route blocker absent")?;
                        let kind = p
                            .modules
                            .get(&d.source)
                            .and_then(|m| m.object_definition_words.get(&d.object))
                            .and_then(|w| w.get(9))
                            .ok_or("native route blocker kind absent")?;
                        Ok::<_, String>((id, *kind))
                    })
                    .transpose()?;
                let request = check.pending().ok_or("native route lost floor request")?;
                check.resume(
                    request,
                    Reply::Placement(crate::spatial::PlacementResult {
                        allowed: outcome.accepted,
                        error: outcome.error,
                        blocker,
                    }),
                )?;
                *floor = None;
                continue;
            }
            let Some(request) = check.next() else {
                self.native_lot.as_mut().unwrap().diagnostics = check
                    .outcome()
                    .ok_or("native route clearance absent")?
                    .diagnostics;
                return Ok(true);
            };
            let native = self.native_lot.as_ref().unwrap();
            let lot = self
                .architecture
                .as_ref()
                .unwrap()
                .original
                .as_ref()
                .unwrap();
            if let Some(reply) =
                Check::world_reply(request, &self.vm.world, &native.placement.links, lot)?
            {
                check.resume(request, reply)?;
            } else if let Request::Placement { position, level } = request {
                self.native_lot.as_mut().unwrap().diagnostics = check.diagnostics();
                *floor = Some(crate::lot_acceptance::live::Execution::new(
                    self, caller, position, level,
                )?);
            }
        }
    }
}
