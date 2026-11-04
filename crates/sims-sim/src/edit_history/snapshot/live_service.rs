//!Live snapshot virtual+4c/+60/+64 calls, owned by the precise ObjectUndoable
//!service frame. Reuses the native ordinary/group implementations and callbacks.
use super::{lifecycle, placement};
use crate::{
    Simulation, Step, StepStop,
    edit_history::object::driver,
    lot_cursor::{acceptance, release, rotation},
    lot_links::Descriptor,
    lot_placement,
};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, ThreadId, Vm};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Work {
    Rotate(rotation::Execution),
    Check(acceptance::Check),
    Place(release::Put),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    tick: u64,
    caller: driver::ServiceCaller,
    cheat: bool,
    work: Work,
}
fn request(service: driver::Service) -> Result<placement::Request, String> {
    match service {
        driver::Service::Placement(r)
        | driver::Service::Restore(lifecycle::Request::Placement(r)) => Ok(r),
        _ => Err("history service is not snapshot placement".into()),
    }
}
fn id(vm: &Vm, h: u32) -> Result<ObjectId, String> {
    vm.world
        .scheduler_storage
        .as_ref()
        .and_then(|r| r.object_for_handle(h))
        .filter(|id| vm.world.objects.contains_key(id))
        .ok_or("snapshot placement lifetime ended".into())
}
fn descriptor(vm: &Vm, p: placement::Place) -> Result<Descriptor, String> {
    Ok(Descriptor {
        on_lot: true,
        position: p.position,
        level: p.level,
        container: p.container.map(|h| id(vm, h)).transpose()?,
        slot: p.slot,
    })
}
fn query(vm: &Vm, p: placement::Place) -> Result<acceptance::Query, String> {
    Ok(acceptance::Query {
        object: p.object,
        position: p.position,
        level: p.level,
        container: p
            .container
            .map(|h| id(vm, h).map(|id| (id, p.slot)))
            .transpose()?,
    })
}
impl Execution {
    pub fn caller(&self) -> driver::ServiceCaller {
        self.caller
    }
    pub fn placement_cheat(&self) -> bool {
        self.cheat
    }
    pub fn complete(&self) -> bool {
        match &self.work {
            Work::Rotate(e) => e.complete(),
            Work::Check(e) => e.outcome().is_some(),
            Work::Place(e) => e.complete(),
        }
    }
    pub(crate) fn owns_callback(&self, vm: &Vm, p: &Program, t: ThreadId, d: usize) -> bool {
        match &self.work {
            Work::Rotate(e) => e.owns_callback(vm, p, t, d),
            Work::Check(e) => e.owns_callback(vm, p, t, d),
            Work::Place(e) => e.owns_callback(vm, p, t, d),
        }
    }
    pub(crate) fn graphic(&self) -> Option<&lot_placement::live::Execution> {
        match &self.work {
            Work::Rotate(e) => e.graphic(),
            Work::Place(e) => e.graphic(),
            _ => None,
        }
    }
    pub(crate) fn graphic_mut(&mut self) -> Option<&mut lot_placement::live::Execution> {
        match &mut self.work {
            Work::Rotate(e) => e.graphic_mut(),
            Work::Place(e) => e.graphic_mut(),
            _ => None,
        }
    }
    pub(crate) fn animator(&self) -> Option<u32> {
        match &self.work {
            Work::Place(e) => e.animator(),
            _ => None,
        }
    }
    pub(crate) fn acknowledge_animator(&mut self, object: u32) -> Result<(), String> {
        match &mut self.work {
            Work::Place(e) => e.acknowledge_animator(object),
            _ => Err("history has no Animator child".into()),
        }
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        if self.tick != s.vm.tick
            || s.native_lot.is_none()
            || s.cursor_work_pending()
            || s.object_pickup.is_some()
            || s.ground_placement.is_some()
            || s.synchronous_removal.is_some()
            || s.native_selection.is_some()
            || s.portrait_person.is_some()
            || s.vm.player_menu.is_some()
            || s.visit.is_some()
            || s.vm.scheduled_object_visit.is_some()
            || s.native_scheduler
                .as_ref()
                .is_some_and(|n| n.request().is_some())
        {
            return Err("snapshot placement overlaps synchronous work".into());
        }
        let handle = match request(self.caller.service)? {
            placement::Request::Rotate(h) => h,
            placement::Request::Check(p) | placement::Request::Place(p) => p.object,
            _ => return Err("snapshot pickup is not a placement owner".into()),
        };
        let object = &s.vm.world.objects[&id(&s.vm, handle)?];
        if object.person.is_some() || object.cursor.is_some() {
            return Err("snapshot placement requires ordinary/group virtuals".into());
        }
        let valid = match (&self.work, request(self.caller.service)?) {
            (Work::Rotate(e), placement::Request::Rotate(h)) => {
                e.validate(s, p)?;
                e.binding() == (h, 1) && e.cheat() == self.cheat
            }
            (Work::Check(e), placement::Request::Check(args)) => {
                e.validate(&s.vm, p)?;
                e.binding() == query(&s.vm, args)? && e.cheat() == self.cheat
            }
            (Work::Place(e), placement::Request::Place(args)) => {
                e.validate(s, p)?;
                e.binding() == (args.object, descriptor(&s.vm, args)?)
            }
            _ => false,
        };
        if !valid {
            return Err("snapshot placement child differs from caller".into());
        }
        Ok(())
    }
    fn reply(&self) -> Result<driver::Reply, String> {
        if !self.complete() {
            return Err("snapshot placement has not returned".into());
        }
        let answer = match (&self.work, request(self.caller.service)?) {
            (Work::Check(e), placement::Request::Check(p)) => placement::Reply::Check {
                value: u32::from(e.outcome().unwrap()),
                position: p.position,
            },
            (Work::Rotate(_), placement::Request::Rotate(_))
            | (Work::Place(_), placement::Request::Place(_)) => placement::Reply::Done,
            _ => return Err("snapshot placement reply kind differs".into()),
        };
        Ok(match self.caller.service {
            driver::Service::Placement(_) => driver::Reply::Placement(answer),
            _ => driver::Reply::Restore(lifecycle::Reply::Placement(answer)),
        })
    }
}
impl Simulation {
    pub fn begin_object_command_placement(
        &mut self,
        p: &Program,
        caller: &driver::Execution,
        cheat: bool,
    ) -> Result<(), String> {
        if self.object_placement.is_some()
            || !self.deletion.is_empty()
            || self.cursor_has_active_vm_callback()
            || self.person_factories.values().any(|f| !f.finalized)
        {
            return Err("snapshot placement overlaps synchronous work".into());
        }
        let caller = caller
            .service_caller()
            .ok_or("snapshot placement caller absent")?;
        let work = match request(caller.service)? {
            placement::Request::Rotate(h) => {
                Work::Rotate(rotation::Execution::new(self, h, 1, cheat)?)
            }
            placement::Request::Check(args) => {
                Work::Check(acceptance::Check::new(self, query(&self.vm, args)?, cheat)?)
            }
            placement::Request::Place(args) => Work::Place(release::Put::new(
                self,
                args.object,
                descriptor(&self.vm, args)?,
            )?),
            _ => return Err("snapshot pickup requires its own driver".into()),
        };
        let e = Execution {
            tick: self.vm.tick,
            caller,
            cheat,
            work,
        };
        e.validate(self, p)?;
        self.validate_native_lot()?;
        self.object_placement = Some(e);
        Ok(())
    }
    pub fn advance_object_command_placement(
        &mut self,
        p: &Program,
        budget: usize,
    ) -> Result<Step, String> {
        if budget == 0 {
            return Err("snapshot placement requires positive budget".into());
        }
        self.object_placement
            .as_ref()
            .ok_or("snapshot placement absent")?
            .validate(self, p)?;
        let mut e = self.object_placement.take().unwrap();
        let mut step = Step {
            stop: StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        let mut left = budget;
        let result = match &mut e.work {
            Work::Rotate(c) => c.advance(self, p, &mut left, &mut step),
            Work::Check(c) => c.advance(self, p, &mut left, &mut step),
            Work::Place(c) => c.advance(self, p, &mut left, &mut step),
        };
        self.object_placement = Some(e);
        self.publish_live_placement_caches()?;
        self.validate_geometry()?;
        self.validate_native_lot()?;
        result?;
        self.object_placement.as_ref().unwrap().validate(self, p)?;
        Ok(step)
    }
    pub fn finish_object_command_placement(
        &mut self,
        p: &Program,
        caller: &mut driver::Execution,
    ) -> Result<(), String> {
        let e = self
            .object_placement
            .as_ref()
            .ok_or("snapshot placement absent")?;
        e.validate(self, p)?;
        if caller.service_caller() != Some(e.caller) {
            return Err("snapshot placement caller changed".into());
        }
        caller.acknowledge(e.caller.service, e.reply()?)?;
        self.object_placement = None;
        Ok(())
    }
}
