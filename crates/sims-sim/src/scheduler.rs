//! Household owner of native46d553's registry cursor and suspended call.
//! A budget suspension retains the same pass. A completed partial pass emits
//! a world update at the same game time, then the next invocation starts anew.
use crate::Simulation;
use serde::{Deserialize, Serialize};
use sims_vm::{
    ThreadId,
    scheduler::{Inputs, Pass, Reply, Request, registry::Borrowed},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub cursor: i32,
    pass: Option<Pass>,
    tick: Option<u64>,
}
impl State {
    /// A new Rust delivery epoch at a proved, completed native scheduler
    /// boundary. This retains the source cursor; it neither creates nor runs
    /// a native registry pass. The importer must supply the boundary proof.
    pub fn from_completed_cursor(cursor: i32) -> Result<Self, String> {
        if cursor < 0 {
            return Err("negative completed registry cursor".into());
        }
        Ok(Self {
            cursor,
            pass: None,
            tick: None,
        })
    }
    pub fn result(&self) -> Option<bool> {
        self.pass.as_ref().and_then(Pass::result)
    }
    pub fn request(&self) -> Option<&Request> {
        self.pass.as_ref().and_then(Pass::request)
    }
    pub(crate) fn pass(&self) -> Result<&Pass, String> {
        self.pass
            .as_ref()
            .ok_or_else(|| "native registry pass has not begun".into())
    }
    pub(crate) fn reset_pass(&mut self) {
        self.pass = None;
        self.tick = None;
    }
    pub fn validate(&self, sim: &Simulation) -> Result<(), String> {
        if self.cursor < 0
            || sim.vm.world.scheduler_storage.is_none()
            || self.pass.is_some() != self.tick.is_some()
            || self.tick.is_some_and(|tick| tick != sim.vm.tick)
        {
            return Err("invalid native lot scheduler owner".into());
        }
        if let Some(pass) = &self.pass {
            pass.validate()?;
        }
        let active = self.pass.as_ref().is_some_and(|p| p.result().is_none());
        if active != sim.vm.lot_rng_scope.is_some() || sim.vm.lot_rng_state.is_none() {
            return Err("native registry pass lost its RNG scope or lot seed".into());
        }
        if let Some(scope) = &sim.vm.lot_rng_scope {
            scope.validate(&sim.vm)?;
        }
        if sim
            .cursor_factory
            .as_ref()
            .is_some_and(|f| f.owns_visit(&sim.vm))
        {
            if sim.visit.is_some() || self.request().is_some() {
                return Err("cursor Update overlaps a registry Tick".into());
            }
            return Ok(());
        }
        if let Some(visit) = &sim.vm.scheduled_object_visit {
            let root = visit.chain().last().unwrap();
            let Some(Request::Tick { object, tick }) = self.request() else {
                return Err("native object visit has no pending scheduler Tick".into());
            };
            if *tick != sim.vm.tick as u32
                || sim
                    .vm
                    .world
                    .scheduler_storage
                    .as_ref()
                    .unwrap()
                    .object_for_handle(*object)
                    != Some(root.object())
                || sim.visit != Some((root.thread(), Some(root.object())))
            {
                return Err("native scheduler Tick differs from its VM visit".into());
            }
        } else if sim.visit.is_some() {
            return Err("native simulation visit lost its VM owner".into());
        }
        Ok(())
    }
}
impl Simulation {
    /// Enter after real loading/activation, with the cursor read from SIMI.
    /// Old snapshots without registry history must not be silently upgraded.
    pub fn enable_registry_scheduler(&mut self, cursor: i32) -> Result<(), String> {
        if self.native_scheduler.is_some()
            || self.fixture_driver.is_some()
            || self.vm.lot_rng_scope.is_some()
            || self.cursor_work_pending()
            || self.visit.is_some()
            || !self.visited.is_empty()
            || self.vm.scheduled_object_visit.is_some()
            || cursor < 0
        {
            return Err(
                "native scheduler requires an unowned pass boundary and valid saved cursor".into(),
            );
        }
        self.registry_inputs()?;
        Borrowed::new(&mut self.vm)?;
        self.require_native_scheduler_hosts()?;
        if self.vm.lot_rng_state.is_none() {
            return Err("native scheduler requires the retained lot RNG seed".into());
        }
        self.native_scheduler = Some(State {
            cursor,
            pass: None,
            tick: None,
        });
        Ok(())
    }
    fn require_native_scheduler_hosts(&self) -> Result<(), String> {
        if !self.interactions.is_empty()
            || !self.animations.is_empty()
            || !self.carry.is_empty()
            || !self.turns.is_empty()
            || !self.pending_actions.is_empty()
            || self
                .vm
                .world
                .objects
                .iter()
                .any(|(id, object)| object.person.is_some() && !self.characters.contains_key(id))
        {
            return Err(
                "native scheduler requires native person/animation hosts, not laboratory adapters"
                    .into(),
            );
        }
        Ok(())
    }
    fn registry_inputs(&self) -> Result<Inputs, String> {
        let word = |id| {
            self.vm
                .world
                .globals
                .get(&id)
                .copied()
                .ok_or_else(|| format!("native scheduler global {id} absent"))
        };
        Ok(Inputs {
            paused: word(17)?,
            mode: word(19)?,
            lot_number: word(10)?,
            tick: self.vm.tick as u32,
        })
    }
    pub(crate) fn next_registry_thread(&mut self) -> Result<Option<ThreadId>, String> {
        if let Some((thread, _)) = self.visit {
            // The suspended Update owns its synchronous factories. A child
            // may already be registered while its character is still being
            // assembled; finish that owner before checking a new selection.
            return Ok(Some(thread));
        }
        self.require_native_scheduler_hosts()?;
        let inputs = self.registry_inputs()?;
        let state = self
            .native_scheduler
            .as_mut()
            .ok_or("native scheduler absent")?;
        if state.pass.is_none() {
            let pass = Pass::begin(inputs, &Borrowed::new(&mut self.vm)?, &mut state.cursor)?;
            self.vm.begin_lot_rng_scope()?;
            state.pass = Some(pass);
            state.tick = Some(self.vm.tick);
        }
        let returning = state.pass.as_ref().unwrap().result().is_none();
        let request = state.pass.as_mut().unwrap().advance(
            &mut Borrowed::new(&mut self.vm)?,
            &mut state.cursor,
            inputs,
        )?;
        match request {
            None => {
                if returning {
                    self.vm.end_lot_rng_scope()?;
                }
                Ok(None)
            }
            Some(Request::ObjectError { object }) => Err(format!(
                "original439a6f error presentation/reset requires its host for lifetime {object}"
            )),
            Some(Request::Tick { object, .. }) => {
                let id = Borrowed::new(&mut self.vm)?.object_id(object)?;
                self.vm.begin_scheduled_object_visit(id)?;
                let thread = self.vm.scheduled_object_visit.as_ref().unwrap().thread();
                if let Some(character) = self.characters.get_mut(&id) {
                    character.animation_yield = None;
                }
                for reach in self.native_reaches.values_mut() {
                    reach.rearm_object_visit(thread);
                }
                Ok(Some(thread))
            }
        }
    }
    pub(crate) fn finish_registry_object(&mut self) -> Result<(), String> {
        let Some(state) = &mut self.native_scheduler else {
            return Ok(());
        };
        let pass = state.pass.as_mut().ok_or("native scheduler pass absent")?;
        let request = pass
            .request()
            .cloned()
            .ok_or("native scheduler Tick absent")?;
        let Request::Tick { object, .. } = request else {
            return Err("native scheduler is awaiting error service".into());
        };
        let id = Borrowed::new(&mut self.vm)?.object_id(object)?;
        if self
            .vm
            .scheduled_object_visit
            .as_ref()
            .is_none_or(|v| v.object() != id || v.chain().count() != 1)
        {
            return Err("native scheduler Tick still owns a nested update".into());
        }
        self.vm.end_scheduled_object_visit()?;
        // Only actual completed VM+person Update reaches here. Budget/resource
        // suspensions and faults retain the pending service and never reply true.
        pass.acknowledge(&request, Reply::Tick(1))
    }
    /// Finish the source pass (including partial WorldUpdate), after the host
    /// has serviced pending callbacks. Returns whether game time advanced.
    pub fn finish_registry_pass(&mut self) -> Result<bool, String> {
        self.native_scheduler
            .as_ref()
            .ok_or("native scheduler absent")?
            .validate(self)?;
        let complete = self
            .native_scheduler
            .as_ref()
            .and_then(State::result)
            .ok_or("native registry pass has not returned")?;
        self.finish_pass_clock(complete)?;
        self.native_scheduler.as_mut().unwrap().reset_pass();
        Ok(complete)
    }
}
