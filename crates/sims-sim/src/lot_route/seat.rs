//! Native seat lookup used by the mixed-posture candidate scan. Uses the
//! imported tile heads and raw slot-zero chain, never render-body/ID order.
use crate::{Point, Simulation, routing::Seat};
use sims_vm::Program;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    Failed,
    AlreadySeated,
    Started,
}
pub trait Services {
    fn exists(&self, seat: i16) -> bool;
    fn function(&mut self, seat: i16) -> Result<u16, String>;
    fn container(&mut self) -> Result<Option<i16>, String>;
    fn species(&self) -> Result<i16, String>;
    fn push(&mut self, seat: i16, bhav: u16) -> Result<bool, String>;
}
/// Selected-seat branch of 0x45c38e, after publishing entry flags. A missing
/// Sit function is not a standing fallback, and current-seat reuse precedes
/// the species guard. The function lookup is repeated before the actual push.
pub fn dispatch(seat: i16, host: &mut impl Services) -> Result<Dispatch, String> {
    if !host.exists(seat) || host.function(seat)? == 0 {
        return Ok(Dispatch::Failed);
    }
    if host.container()? == Some(seat) {
        return Ok(Dispatch::AlreadySeated);
    }
    if host.species()? > 1 {
        return Ok(Dispatch::Failed);
    }
    let bhav = host.function(seat)?;
    Ok(if host.push(seat, bhav)? {
        Dispatch::Started
    } else {
        Dispatch::Failed
    })
}

struct Host<'a> {
    s: &'a mut Simulation,
    p: &'a Program,
    thread: u64,
    caller: i16,
}
impl Services for Host<'_> {
    fn exists(&self, seat: i16) -> bool {
        seat > 0 && self.s.vm.world.objects.contains_key(&seat)
    }
    fn function(&mut self, seat: i16) -> Result<u16, String> {
        let d = self.s.vm.world.objects[&seat]
            .definition
            .as_ref()
            .ok_or("selected seat resource absent")?;
        Ok(self
            .p
            .modules
            .get(&d.source)
            .and_then(|m| m.object_functions.get(&d.object))
            .and_then(|f| f.get(26))
            .ok_or("selected seat function table absent")?
            .action)
    }
    fn container(&mut self) -> Result<Option<i16>, String> {
        self.s
            .native_lot
            .as_ref()
            .unwrap()
            .placement
            .links
            .capture(&mut self.s.vm.world, self.caller)
            .map(|d| d.container)
    }
    fn species(&self) -> Result<i16, String> {
        self.s.vm.world.objects[&self.caller]
            .person
            .as_ref()
            .and_then(|p| p.get(&65))
            .copied()
            .ok_or_else(|| "selected seat person species absent".into())
    }
    fn push(&mut self, seat: i16, bhav: u16) -> Result<bool, String> {
        let owner = self.s.vm.world.objects[&seat]
            .definition
            .as_ref()
            .unwrap()
            .source
            .clone();
        self.s
            .vm
            .push_portal_service(self.p, self.thread, owner, bhav, seat)?;
        Ok(true)
    }
}
impl Simulation {
    pub(crate) fn service_original_seat(
        &mut self,
        p: &Program,
        e: &super::live::Execution,
        step: &mut crate::Step,
    ) -> Result<Dispatch, String> {
        let candidate = e
            .selected_seat
            .as_ref()
            .ok_or("selected seat candidate absent")?;
        crate::contention::pending_route(&self.vm.threads[&e.thread], e.request)
            .ok_or("selected seat route absent")?;
        match dispatch(
            candidate.seat,
            &mut Host {
                s: self,
                p,
                thread: e.thread,
                caller: e.caller,
            },
        )? {
            Dispatch::Started => {
                self.route_control
                    .get_mut(&e.request)
                    .unwrap()
                    .original
                    .as_mut()
                    .unwrap()
                    .seated = Some(candidate.seat);
                self.person(e.caller, 72, 0)?;
                self.event(
                    &mut step.events,
                    e.thread,
                    format!(
                        "original-lot route invokes Sit on #{}; entry flags {}",
                        candidate.seat, candidate.flags
                    ),
                );
                Ok(Dispatch::Started)
            }
            Dispatch::Failed => Ok(Dispatch::Failed),
            Dispatch::AlreadySeated => {
                let r = crate::contention::pending_route(&self.vm.threads[&e.thread], e.request)
                    .unwrap()
                    .clone();
                self.begin_native_route_completion(e.thread, &r, true, 0)?;
                Ok(Dispatch::AlreadySeated)
            }
        }
    }
    pub(crate) fn original_seat_at(
        &self,
        p: &Program,
        actor: i16,
        point: Point,
        level: i32,
    ) -> Result<Option<Seat>, String> {
        let links = &self
            .native_lot
            .as_ref()
            .ok_or("native seat world absent")?
            .placement
            .links;
        // 0x483414 calls the lot-bounds check before 0x43244d. Both use
        // signed-byte coarse coordinates, including their native wraparound.
        if !links.on_lot([point.x, point.y]) {
            return Ok(None);
        }
        let w = &self.vm.world;
        let Some(id) = links.first_blocker(w, [point.x, point.y], level)? else {
            return Ok(None);
        };
        let o = &w.objects[&id];
        let d = o.definition.as_ref().ok_or("seat lookup resource absent")?;
        let m = p
            .modules
            .get(&d.source)
            .ok_or("seat lookup module absent")?;
        let sit = m
            .object_functions
            .get(&d.object)
            .and_then(|f| f.get(26))
            .ok_or("seat function table absent")?
            .action
            != 0;
        // Native stops checking metadata after a missing Sit function.
        if !sit {
            return Ok(None);
        }
        let facing = *o.data.get(&1).ok_or("seat direction absent")?;
        let allowed = *m
            .object_definition_words
            .get(&d.object)
            .and_then(|w| w.get(66))
            .ok_or("seat entry mask absent")? as u16;
        let occupant = o
            .slots
            .first()
            .copied()
            .flatten()
            .filter(|id| *id > 0 && w.objects.contains_key(id))
            .unwrap_or(0);
        Ok(Some(Seat {
            id,
            facing,
            allowed,
            sit,
            other_occupant: if occupant == actor { 0 } else { occupant },
        }))
    }
}
