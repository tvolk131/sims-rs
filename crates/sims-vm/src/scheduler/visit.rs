//! Explicit base-object Update ownership for a native scheduler Tick service.
//! Original474e88 re-enters Sleep on each call even when46d345's game clock has
//! not advanced. Rust instruction-budget yields must keep this same owner.
use crate::{ObjectId, Status, ThreadId, Vm};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visit {
    parent: Option<Box<Visit>>,
    pub(crate) object: ObjectId,
    thread: ThreadId,
    tick: u64,
    pub(crate) finished: bool,
}
impl Visit {
    pub fn chain(&self) -> impl Iterator<Item = &Self> {
        std::iter::successors(Some(self), |v| v.parent.as_deref())
    }
    pub fn object(&self) -> ObjectId {
        self.object
    }
    pub fn thread(&self) -> ThreadId {
        self.thread
    }
    pub fn finished(&self) -> bool {
        self.finished
    }
    pub(crate) fn owns(&self, object: ObjectId, thread: ThreadId, tick: u64) -> bool {
        self.object == object && self.thread == thread && self.tick == tick
    }
    pub fn validate(&self, vm: &Vm) -> Result<(), String> {
        let mut objects = std::collections::BTreeSet::new();
        for visit in self.chain() {
            if !objects.insert(visit.object) || objects.len() > crate::MAX_STACK {
                return Err("recursive or oversized native object visit stack".into());
            }
            visit.validate_one(vm)?;
        }
        Ok(())
    }
    fn validate_one(&self, vm: &Vm) -> Result<(), String> {
        if self.tick != vm.tick || vm.object_threads.get(&self.object) != Some(&self.thread) {
            return Err("native object visit lost its tick or registered thread".into());
        }
        let object = vm
            .world
            .objects
            .get(&self.object)
            .ok_or("native visit object absent")?;
        let thread = vm
            .threads
            .get(&self.thread)
            .ok_or("native visit thread absent")?;
        let ordinary_check = thread
            .callback
            .as_ref()
            .and_then(|b| b.chain().last())
            .map_or(thread.check, |b| b.resume_check);
        if object.idle.is_none()
            || ordinary_check
            || thread
                .stack
                .first()
                .is_some_and(|f| f.caller != self.object)
            || (self.finished && object.last_update != Some(self.tick))
        {
            return Err("invalid native object visit state".into());
        }
        Ok(())
    }
}
impl Vm {
    /// Constructors run synchronous Init before their immediate ordinary Main
    /// update. Stop at that boundary when a parent scheduler visit is active.
    pub fn prepare_factory_object_visit(&mut self, object: ObjectId) -> Result<(), String> {
        let Some(owner) = &self.scheduled_object_visit else {
            return Ok(());
        };
        if owner.object() == object {
            return Ok(());
        }
        let thread = self
            .object_threads
            .get(&object)
            .and_then(|id| self.threads.get(id))
            .ok_or("factory update thread absent")?;
        if !thread.check && thread.callback.is_none() {
            self.begin_nested_object_visit(object)?;
        }
        Ok(())
    }
    pub fn finish_factory_object_visit(&mut self, object: ObjectId) -> Result<(), String> {
        if self
            .scheduled_object_visit
            .as_ref()
            .is_some_and(|v| v.object() == object && v.chain().count() > 1)
        {
            self.end_scheduled_object_visit()?;
        }
        Ok(())
    }
    /// Call once when entering an actual object virtual Update, after46d553's
    /// enable/countdown gates. This does not itself choose an eligible object,
    /// run a person wrapper, finish a pass, or advance game time.
    pub fn begin_scheduled_object_visit(&mut self, object: ObjectId) -> Result<(), String> {
        if self.scheduled_object_visit.is_some() {
            return Err("native object visit already owns execution".into());
        }
        self.prepare_native_suspension_tick(object)?;
        self.begin_object_visit(object)
    }
    /// Explicit synchronous Update inside another object's engine call, such
    /// as a factory's immediate Main update. It does not consume a registry
    /// cursor. The parent stays suspended until this child returns.
    pub fn begin_nested_object_visit(&mut self, object: ObjectId) -> Result<(), String> {
        if self
            .object_threads
            .get(&object)
            .and_then(|id| self.threads.get(id))
            .is_some_and(|t| {
                matches!(
                    t.status,
                    Status::NativeSuspended(_) | Status::RegistrySuspended(_) | Status::NativeRegistryReentry(_)
                )
            })
        {
            return Err(
                "retained native suspension needs a registry Tick, not a nested Update".into(),
            );
        }
        let parent = self
            .scheduled_object_visit
            .as_ref()
            .ok_or("nested native visit has no parent")?;
        parent.validate(self)?;
        if parent.chain().count() >= crate::MAX_STACK || parent.chain().any(|v| v.object == object)
        {
            return Err("recursive or oversized nested native update".into());
        }
        self.begin_object_visit(object)
    }
    fn begin_object_visit(&mut self, object: ObjectId) -> Result<(), String> {
        let thread = *self
            .object_threads
            .get(&object)
            .ok_or("native visit has no registered thread")?;
        let mut visit = Visit {
            parent: None,
            object,
            thread,
            tick: self.tick,
            finished: false,
        };
        visit.validate(self)?;
        let target = self.threads.get_mut(&thread).unwrap();
        // A target-tree service (portal/Stand) is ordinary yielding script
        // work. Its suspended frames survive Update, unlike a synchronous
        // callback or a returned service whose host has not consumed it yet.
        if target.callback.is_some() || target.service.as_ref().is_some_and(|s| s.result.is_some())
        {
            return Err("native object visit overlaps a synchronous callback".into());
        }
        if matches!(target.status, Status::Faulted(_)) {
            return Err("native fault requires its error service before another visit".into());
        }
        //Sleeping is the replacement VM's host yield. The original VM stores
        //the remaining counter in the frame/registry, never in a future game
        //tick that would prevent run-while-paused controllers from waking.
        if matches!(
            target.status,
            Status::Sleeping { .. } | Status::NativeSuspended(_) | Status::RegistrySuspended(_) | Status::NativeRegistryReentry(_)
        ) {
            target.status = Status::Ready;
        }
        self.world
            .objects
            .get_mut(&object)
            .unwrap()
            .idle
            .as_mut()
            .unwrap()
            .last_run = Some(self.tick);
        //This timestamp is replacement-host deduplication, not native headline
        //storage.46e004 runs once per completed Update even at the same clock.
        if let Some(headline) = self
            .world
            .objects
            .get_mut(&object)
            .unwrap()
            .headline
            .as_mut()
        {
            headline.last_update = None;
        }
        visit.parent = self.scheduled_object_visit.take().map(Box::new);
        self.scheduled_object_visit = Some(visit);
        Ok(())
    }
    /// Release only after the base-object post-update tail has completed. Its
    /// person/animator wrapper, if any, remains the simulation host's owner.
    pub fn end_scheduled_object_visit(&mut self) -> Result<(), String> {
        let visit = self
            .scheduled_object_visit
            .as_ref()
            .ok_or("native object visit absent")?;
        visit.validate(self)?;
        if !visit.finished {
            return Err("native object update tail has not completed".into());
        }
        let thread = &self.threads[&visit.thread];
        if matches!(thread.status, Status::Ready | Status::Faulted(_))
            || thread.callback.is_some()
            || thread.service.as_ref().is_some_and(|s| s.result.is_some())
        {
            return Err("native object visit still owns runnable or synchronous work".into());
        }
        self.scheduled_object_visit = self
            .scheduled_object_visit
            .take()
            .unwrap()
            .parent
            .map(|p| *p);
        Ok(())
    }
}
