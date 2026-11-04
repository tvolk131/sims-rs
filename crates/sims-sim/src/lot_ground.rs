//! 48cbec's residential ground destination:4347f0 mode1, same room,
//! occupied tiles allowed, then a separate virtual acceptance and placement.
//! The caller owns440b2d construction and44123f cleanup on failure/cancel.
use crate::{
    Simulation, Step, lot_acceptance,
    lot_links::Descriptor,
    lot_placement,
    placement::{SearchTile, Spiral, search_tile_allowed},
};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, ThreadId, Vm};

pub const DESTINATION_GUID: u32 = 0x7c4;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Search,
    Recheck,
    Place,
    Complete(bool),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Execution {
    tick: u64,
    object: ObjectId,
    center: [i32; 2],
    level: i32,
    size: i32,
    room: i16,
    cursor: Spiral,
    fallback: bool,
    position: Option<[i32; 2]>,
    phase: Phase,
    acceptance: Option<lot_acceptance::live::Execution>,
    placement: Option<lot_placement::live::Execution>,
}
impl Execution {
    pub fn object(&self) -> ObjectId {
        self.object
    }
    pub fn complete(&self) -> bool {
        matches!(self.phase, Phase::Complete(_))
    }
    fn destination(&self) -> Descriptor {
        Descriptor {
            on_lot: true,
            position: self.position.unwrap(),
            level: self.level,
            container: None,
            slot: 0,
        }
    }
    pub(crate) fn owns_callback(
        &self,
        vm: &Vm,
        p: &Program,
        thread: ThreadId,
        depth: usize,
    ) -> bool {
        self.acceptance
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self
                .placement
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        if self.tick != s.vm.tick
            || s.native_lot.is_none()
            || s.native_selection.is_some()
            || s.synchronous_removal.is_some()
            || s.portrait_person.is_some()
            || s.vm.player_menu.is_some()
            || !(1..=2).contains(&self.level)
            || s.architecture
                .as_ref()
                .and_then(|a| a.original.as_ref())
                .is_none_or(|a| a.size as i32 != self.size || a.floor(self.level).is_none())
            || s.vm.world.objects.get(&self.object).is_none_or(|o| {
                o.multitile.is_some()
                    || o.person.is_some()
                    || o.definition
                        .as_ref()
                        .is_none_or(|d| d.guid != DESTINATION_GUID)
            })
            || self.cursor.direction() != -1
            || self.phase != Phase::Search && self.position.is_none()
            || self.fallback && self.phase == Phase::Search
            || self.acceptance.is_some() && self.position.is_none()
            || self.placement.is_some()
                && !matches!(self.phase, Phase::Place | Phase::Complete(true))
        {
            return Err("invalid ground placement continuation".into());
        }
        self.cursor.validate(self.size, self.center)?;
        if let Some(point) = self.position {
            if if self.fallback {
                self.cursor.clone().next().is_some() || point != self.center.map(|v| (v & !7) | 8)
            } else {
                self.cursor.last_point() != Some(point)
            } {
                return Err("ground placement candidate differs".into());
            }
        }
        if let Some(a) = &self.acceptance {
            a.validate(&s.vm, p)?;
            if a.binding() != (self.object, self.position.unwrap(), self.level) {
                return Err("ground acceptance lost its caller".into());
            }
        }
        if matches!(self.phase, Phase::Place | Phase::Complete(true))
            && self
                .acceptance
                .as_ref()
                .and_then(|a| a.outcome())
                .is_none_or(|o| !o.accepted)
        {
            return Err("ground placement bypassed final acceptance".into());
        }
        if self.phase == Phase::Complete(false)
            && self
                .acceptance
                .as_ref()
                .and_then(|a| a.outcome())
                .is_none_or(|o| o.accepted)
        {
            return Err("ground failure lacks rejected acceptance".into());
        }
        if let Some(e) = &self.placement {
            e.validate(&s.vm, p, self.object)?;
            if e.operation.binding() != (self.object, Some(self.destination()))
                || self.phase == Phase::Complete(true) && !e.complete
            {
                return Err("ground physical placement lost its caller".into());
            }
        } else if self.phase == Phase::Complete(true) {
            return Err("ground destination was not placed".into());
        }
        Ok(())
    }
}
impl Simulation {
    /// Input is native physical X/Y after47bc1f camera conversion. It is not a
    /// screen pixel or a tile coordinate. Lot99's special policy is caller-owned.
    pub fn begin_ground_placement(
        &mut self,
        p: &Program,
        object: ObjectId,
        point: [i32; 2],
        level: i32,
    ) -> Result<(), String> {
        if (self.ground_placement.is_some()
            || self.object_pickup.is_some()
            || self.object_placement.is_some()
            || self.cursor_work_pending())
            || self.native_selection.is_some()
            || self.synchronous_removal.is_some()
            || self.portrait_person.is_some()
            || self.vm.player_menu.is_some()
            || !self.deletion.is_empty()
            || self.vm.threads.values().any(|t| t.callback.is_some())
        {
            return Err("ground placement overlaps synchronous work".into());
        }
        let lot = self
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .ok_or("ground placement requires original architecture")?;
        let floor = lot.floor(level).ok_or("ground placement floor absent")?;
        let size = lot.size as i32;
        let cursor = Spiral::new(size, point)?;
        let [x, y] = cursor.center_tile();
        let e = Execution {
            tick: self.vm.tick,
            object,
            center: point,
            level,
            size,
            room: floor.rooms.primary[x as usize * lot.size + y as usize],
            cursor,
            fallback: false,
            position: None,
            phase: Phase::Search,
            acceptance: None,
            placement: None,
        };
        e.validate(self, p)?;
        self.ground_placement = Some(e);
        Ok(())
    }
    pub fn take_ground_placement(&mut self) -> Result<(ObjectId, Option<[i32; 2]>), String> {
        let e = self
            .ground_placement
            .as_ref()
            .ok_or("ground placement absent")?;
        let Phase::Complete(accepted) = e.phase else {
            return Err("ground placement incomplete".into());
        };
        let result = (e.object, accepted.then(|| e.position.unwrap()));
        self.ground_placement = None;
        Ok(result)
    }
    pub fn advance_ground_placement(&mut self, p: &Program, budget: usize) -> Result<Step, String> {
        self.ground_placement
            .as_ref()
            .ok_or("ground placement absent")?
            .validate(self, p)?;
        let mut e = self.ground_placement.take().unwrap();
        let mut step = Step {
            stop: crate::StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        let mut remaining = budget;
        let result = self.drive_ground_placement(p, &mut e, &mut remaining, &mut step);
        self.ground_placement = Some(e);
        self.sync_spatial_world();
        self.validate_native_lot()?;
        result?;
        Ok(step)
    }
    fn drive_ground_placement(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<(), String> {
        while !e.complete() {
            if *remaining == 0 {
                return Ok(());
            }
            if e.phase == Phase::Place {
                if e.placement.is_none() {
                    e.placement = Some(lot_placement::live::Execution::new_virtual(
                        lot_placement::VirtualOperation::set(
                            &self.vm.world,
                            &self.native_lot.as_ref().unwrap().placement.links,
                            e.object,
                            e.destination(),
                            true,
                        )?,
                    ));
                }
                if !self.drive_live_placement(p, e.placement.as_mut().unwrap(), remaining, step)? {
                    return Ok(());
                }
                //48cbec does not call the primitive's additional virtual+74.
                e.phase = Phase::Complete(true);
                continue;
            }
            if e.position.is_none() {
                let lot = self
                    .architecture
                    .as_ref()
                    .unwrap()
                    .original
                    .as_ref()
                    .unwrap();
                let floor = lot.floor(e.level).unwrap();
                let n = self.native_lot.as_ref().unwrap();
                for point in e.cursor.by_ref() {
                    let cell = (point[1] >> 4) as usize * lot.size + (point[0] >> 4) as usize;
                    if search_tile_allowed(
                        SearchTile {
                            segments: floor.source.walls[cell].segments,
                            head: n.placement.links.heads[e.level as usize - 1][cell],
                            room: floor.rooms.primary[cell],
                            pool: lot.pool[cell],
                            ground_flags: n.placement.flags[0][cell],
                        },
                        e.level,
                        2,
                        Some(e.room),
                        false,
                    ) {
                        e.position = Some(point);
                        break;
                    }
                }
                if e.position.is_none() {
                    // One same-room search, then the original bitwise fallback.
                    // Unlike primitive FindLocation, there is no second room pass.
                    e.position = Some(e.center.map(|v| (v & !7) | 8));
                    e.fallback = true;
                    e.phase = Phase::Recheck;
                }
            }
            if e.acceptance.is_none() {
                e.acceptance = Some(lot_acceptance::live::Execution::new(
                    self,
                    e.object,
                    e.position.unwrap(),
                    e.level,
                )?);
            }
            if !self.drive_native_acceptance(p, e.acceptance.as_mut().unwrap(), remaining, step)? {
                return Ok(());
            }
            let accepted = e.acceptance.as_ref().unwrap().outcome().unwrap().accepted;
            if e.phase == Phase::Recheck {
                e.phase = if accepted {
                    Phase::Place
                } else {
                    Phase::Complete(false)
                };
            } else {
                e.acceptance = None;
                if accepted {
                    e.phase = Phase::Recheck;
                } else {
                    e.position = None;
                }
            }
        }
        Ok(())
    }
}
