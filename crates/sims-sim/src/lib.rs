//! A small deterministic host for the SimAntics VM. No renderer or filesystem.
pub mod animation;
pub mod architecture;
pub mod burn;
pub mod cancellation;
pub mod carry;
pub mod character;
mod character_animation;
pub mod character_profile;
mod character_runtime;
pub mod character_update;
pub mod clock;
pub mod contention;
pub mod create;
pub mod edit_history;
pub mod fixture_driver;
pub mod geometry;
pub mod head_look;
pub mod house_statistics;
pub mod house_valuation;
pub mod household;
pub mod immediate;
pub mod interaction;
pub mod locomotion;
pub mod lot_acceptance;
pub mod lot_adjacency;
pub mod lot_clear;
pub mod lot_controllers;
pub mod lot_creation;
pub mod lot_cursor;
pub mod lot_dynamic;
pub mod lot_environment;
pub mod lot_find_location;
pub mod lot_flags;
pub mod lot_geometry;
pub mod lot_ground;
pub mod lot_links;
pub mod lot_loading;
pub mod lot_pickup;
pub mod lot_placement;
pub mod lot_presenter;
pub mod lot_reach;
pub mod lot_removal;
pub mod lot_reset;
pub mod lot_rooms;
pub mod lot_route;
pub mod lot_runtime;
pub mod lot_selection;
pub mod lot_snap;
pub mod lot_transfer;
pub mod lot_walls;
pub mod native_turn;
pub mod native_wall_io;
pub mod native_wall_vertices;
pub mod object_catalog;
pub mod object_scan;
pub mod observations;
mod path_world;
pub mod person_creation;
pub mod person_notification;
pub mod person_reset;
pub mod person_update;
pub mod placement;
pub mod portals;
pub mod portrait_person;
pub mod pose;
pub mod reach;
pub mod region_path;
pub mod regions;
pub mod removal;
pub mod routing;
pub mod saved_geometry;
pub mod saved_matrix;
pub mod saved_types;
pub mod saved_world;
pub mod scheduler;
mod snapshot;
pub mod spatial;
pub mod surface_remap;
pub mod transfer;
pub mod turn;
pub mod ui_wait;
pub use geometry::{Body, Destination, Point, Rect, Room};
use serde::{Deserialize, Serialize};
use sims_vm::{
    ObjectId, Program, RouteRequest, SnapRequest, Start, Status, Stop, ThreadId, Trace, Vm,
};
pub use snapshot::Snapshot;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Action {
    pub owner: String,
    pub bhav: u16,
    pub actor: ObjectId,
    pub target: ObjectId,
    pub args: Vec<i16>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Route {
    pub request: u64,
    pub caller: ObjectId,
    pub target: ObjectId,
    pub target_body: Body,
    pub goal: Destination,
    /// Includes the start point. next-1 is the caller's current position.
    pub path: Vec<Point>,
    /// Imported search output before removing repeated points for locomotion.
    /// The native endpoint decision observes even zero-length final segments.
    pub original_path: Option<Vec<Point>>,
    pub next: usize,
    pub started_tick: u64,
    pub last_tick: u64,
    pub locomotion: locomotion::Locomotion,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Simulation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_scheduler: Option<scheduler::State>,
    /// Explicit synthetic object-update driver for authored test rooms.
    /// Never accepted by original household sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixture_driver: Option<fixture_driver::State>,
    /// Cooperative scheduler continuation. Diagnostic budgets must not restart
    /// a partially serviced actor or revisit lower IDs ahead of its next opcode.
    pub visit: Option<(ThreadId, Option<ObjectId>)>,
    pub visited: std::collections::BTreeSet<ThreadId>,
    pub immediate: Option<immediate::Execution>,
    #[serde(default)]
    pub cancellation: Option<cancellation::Execution>,
    #[serde(default)]
    pub reach_callback: Option<reach::Callback>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_wait_callback: Option<ui_wait::Callback>,
    pub backgrounds: BTreeMap<ObjectId, carry::Layer>,
    /// Initialized native characters. Installation is explicit until original
    /// load/reset and ordinary person updates are connected end to end.
    pub characters: BTreeMap<ObjectId, character::Resident>,
    #[serde(default)]
    pub retired_characters: BTreeMap<ObjectId, removal::character::Retired>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub retired_graphics: BTreeMap<ObjectId, removal::graphic::Retired>,
    /// Positive ordinary object+120 owners imported with their asset host.
    /// Retain the identity after registry unlink until its destructor completes.
    /// This does not infer that an omitted/historical native pointer was null.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub retained_ordinary_graphics: std::collections::BTreeSet<ObjectId>,
    #[serde(default)]
    pub native_character_settings: Option<character_update::Settings>,
    #[serde(default)]
    pub room_environment: Option<lot_environment::live::Live>,
    pub architecture: Option<architecture::Architecture>,
    /// Present for original imports; never reconstruct these links from bodies.
    pub native_lot: Option<lot_runtime::State>,
    pub person_factories: BTreeMap<ObjectId, person_creation::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portrait_person: Option<portrait_person::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synchronous_removal: Option<lot_removal::immediate::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ground_placement: Option<lot_ground::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_reconciliation: Option<lot_controllers::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_pickup: Option<lot_pickup::live::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_placement: Option<edit_history::snapshot::live_service::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_factory: Option<lot_cursor::Factory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_rotation: Option<lot_cursor::rotation::Root>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_release: Option<lot_cursor::release::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_cleanup: Option<lot_cursor::cleanup::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_selection: Option<lot_selection::Execution>,
    /// An installed application dispatcher owns selection's synchronous E4.
    /// Older/headless profiles retain the explicit Globals-only listener.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub native_selection_broadcasts: bool,
    #[serde(default)]
    pub native_indicator_pools: sims_vm::indicator_pools::Pools,
    pub native_creations: BTreeMap<u64, lot_creation::Execution>,
    pub native_transfers: BTreeMap<u64, lot_transfer::Execution>,
    #[serde(default)]
    pub native_reaches: BTreeMap<u64, lot_reach::live::Execution>,
    #[serde(default)]
    pub native_burn: Option<burn::live::Execution>,
    pub removals: Vec<sims_vm::removal::Pending>,
    pub deletion: Vec<removal::Deletion>,
    pub carry: BTreeMap<ObjectId, carry::Carry>,
    /// Registered objects outside the lot, including pending factories and
    /// completed Create Object mode 6 instances.
    pub unplaced: std::collections::BTreeSet<ObjectId>,
    pub turns: BTreeMap<ObjectId, turn::Turn>,
    pub household: Option<household::Household>,
    pub vm: Vm,
    pub room: Room,
    pub bodies: BTreeMap<ObjectId, Body>,
    pub routes: BTreeMap<ThreadId, Route>,
    pub route_feedback: BTreeMap<u64, routing::FailureFeedback>,
    pub route_control: BTreeMap<u64, contention::State>,
    pub portal_cache: BTreeMap<ObjectId, portals::Cache>,
    pub shoo_creation: Option<contention::Creation>,
    /// Target object -> owning interaction thread. Held through the animation boundary.
    pub reservations: BTreeMap<ObjectId, ThreadId>,
    /// Persisted host work: a passed eligibility thread starts the corresponding action.
    pub pending_actions: BTreeMap<ThreadId, Action>,
    pub animations: BTreeMap<ObjectId, animation::Playback>,
    pub interactions: BTreeMap<ObjectId, interaction::ActorQueue>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub tick: u64,
    pub thread: ThreadId,
    pub detail: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StepStop {
    Dialog,
    Waiting,
    Animation,
    Complete,
    Budget,
    Faulted,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Step {
    pub stop: StepStop,
    pub trace: Vec<Trace>,
    pub events: Vec<Event>,
}

/// Read-only host primitive reports inserted before an index in Step::trace.
/// These reports never participate in VM instruction budgets or saved state.
#[derive(Debug, Clone)]
pub struct HostPollTrace {
    pub before_instruction: usize,
    pub instruction: Trace,
}

impl Simulation {
    pub fn new(vm: Vm, room: Room, bodies: BTreeMap<ObjectId, Body>) -> Result<Self, String> {
        Self::from_placements(vm, room, bodies, Default::default(), None)
    }
    /// Construct the host around an existing lot, including off-lot controllers
    /// and upper-storey bodies. No object callbacks or clock ticks run here.
    pub fn from_placements(
        vm: Vm,
        room: Room,
        bodies: BTreeMap<ObjectId, Body>,
        unplaced: std::collections::BTreeSet<ObjectId>,
        architecture: Option<architecture::Architecture>,
    ) -> Result<Self, String> {
        let state = Self {
            native_scheduler: None,
            fixture_driver: None,
            visit: None,
            visited: Default::default(),
            architecture,
            native_lot: None,
            person_factories: Default::default(),
            portrait_person: None,
            synchronous_removal: None,
            ground_placement: None,
            controller_reconciliation: None,
            object_pickup: None,
            object_placement: None,
            cursor_factory: None,
            cursor_rotation: None,
            cursor_release: None,
            cursor_cleanup: None,
            native_selection: None,
            native_selection_broadcasts: false,
            native_indicator_pools: Default::default(),
            native_creations: Default::default(),
            native_transfers: Default::default(),
            native_reaches: Default::default(),
            native_burn: None,
            immediate: None,
            cancellation: None,
            reach_callback: None,
            ui_wait_callback: None,
            backgrounds: BTreeMap::new(),
            characters: BTreeMap::new(),
            retired_characters: BTreeMap::new(),
            retired_graphics: BTreeMap::new(),
            retained_ordinary_graphics: Default::default(),
            native_character_settings: None,
            room_environment: None,
            removals: vec![],
            deletion: vec![],
            carry: Default::default(),
            unplaced,
            turns: BTreeMap::new(),
            household: None,
            vm,
            room,
            bodies,
            routes: BTreeMap::new(),
            route_feedback: BTreeMap::new(),
            route_control: BTreeMap::new(),
            portal_cache: BTreeMap::new(),
            shoo_creation: None,
            reservations: BTreeMap::new(),
            pending_actions: BTreeMap::new(),
            animations: BTreeMap::new(),
            interactions: BTreeMap::new(),
        };
        state.validate_geometry()?;
        Ok(state)
    }
    pub fn queue_interaction(
        &mut self,
        program: &Program,
        check: u16,
        action: Action,
    ) -> Result<ThreadId, String> {
        if self.interactions.contains_key(&action.actor) {
            return Err("use the actor's FIFO queue for user-directed interactions".into());
        }
        if self.vm.threads.values().any(|t| {
            !matches!(t.status, Status::Returned(_) | Status::Faulted(_))
                && t.stack.first().is_some_and(|f| f.caller == action.actor)
        }) {
            return Err("actor already has an active interaction".into());
        }
        let id = self.vm.spawn(
            program,
            Start {
                owner: &action.owner,
                bhav: check,
                caller: action.actor,
                callee: action.target,
                stack_object: action.target,
                args: action.args.clone(),
                check: true,
            },
        )?;
        self.pending_actions.insert(id, action);
        Ok(id)
    }
    /// Finish cooperative work in this tick. Diagnostic budgets never advance time.
    /// The saved locomotion clock advances once per tick, regardless of repeated calls here.
    pub fn run_current_tick(&mut self, program: &Program, budget: usize) -> Result<Step, String> {
        if self.native_scheduler.is_none() || self.fixture_driver.is_some() {
            return Err("household execution requires the original registry scheduler; legacy snapshots cannot be silently upgraded".into());
        }
        self.run_scheduled_slice(program, budget, Self::next_registry_thread)
    }
    pub fn run_current_tick_with_host_polls(&mut self, program: &Program, budget: usize, polls: &mut Vec<HostPollTrace>) -> Result<Step, String> {
        if self.native_scheduler.is_none() || self.fixture_driver.is_some() {
            return Err("original household tick requires the native registry scheduler".into());
        }
        self.run_scheduled_slice_observed(program, budget, Self::next_registry_thread, Some(polls))
    }
    fn run_scheduled_slice(
        &mut self,
        program: &Program,
        budget: usize,
        next_thread: fn(&mut Self) -> Result<Option<ThreadId>, String>,
    ) -> Result<Step, String> {
        self.run_scheduled_slice_observed(program, budget, next_thread, None)
    }
    fn run_scheduled_slice_observed(
        &mut self, program: &Program, budget: usize,
        next_thread: fn(&mut Self) -> Result<Option<ThreadId>, String>,
        polls: Option<&mut Vec<HostPollTrace>>,
    ) -> Result<Step, String> {
        if self.cursor_work_pending() {
            return Err("cursor factory must return before household ticks".into());
        }
        if self.object_pickup.is_some() || self.object_placement.is_some() {
            return Err("object pickup or placement must return before household ticks".into());
        }
        if self.ground_placement.is_some() || self.controller_reconciliation.is_some() {
            return Err("ground input must return before household ticks".into());
        }
        if self.vm.player_menu.is_some() {
            return Err("player-menu query requires its synchronous owner".into());
        }
        if self.ui_wait_callback.is_some() {
            return Err("UI wait callback requires its synchronous owner".into());
        }
        if self.native_selection.is_some() {
            return Err("native selection requires its synchronous owner".into());
        }
        if self.synchronous_removal.is_some() {
            return Err("synchronous removal requires its synchronous owner".into());
        }
        if self.portrait_person.is_some() {
            return Err("temporary portrait person requires its synchronous owner".into());
        }
        if self
            .person_factories
            .values()
            .any(|s| s.parent.is_none() && !s.finalized)
        {
            return Err("direct person factory requires its synchronous owner".into());
        }
        self.validate_native_lot()?;
        if let Some(scheduler) = &self.native_scheduler {
            scheduler.validate(self)?;
        } else if self.vm.lot_rng_scope.is_some() {
            return Err("lot RNG scope has no native scheduler owner".into());
        }
        if self.vm.constructing_dialog() {
            return Ok(Step {
                stop: StepStop::Budget,
                trace: vec![],
                events: vec![],
            });
        }
        let result = self.run_current_tick_inner_observed(program, budget, next_thread, polls);
        self.sync_spatial_world();
        self.validate_native_lot()?;
        result
    }
    #[cfg(test)]
    fn run_current_tick_inner(
        &mut self,
        program: &Program,
        budget: usize,
        next_thread: fn(&mut Self) -> Result<Option<ThreadId>, String>,
    ) -> Result<Step, String> {
        self.run_current_tick_inner_observed(program, budget, next_thread, None)
    }
    fn run_current_tick_inner_observed(
        &mut self, program: &Program, budget: usize,
        next_thread: fn(&mut Self) -> Result<Option<ThreadId>, String>,
        mut polls: Option<&mut Vec<HostPollTrace>>,
    ) -> Result<Step, String> {
        if self.characters.values().any(|c| {
            c.evaluated_tick > self.vm.tick
                || (self.native_scheduler.is_none()
                    && c.evaluated_tick.saturating_add(1) < self.vm.tick)
        }) {
            return Err("native ordinary person update was skipped before this tick".into());
        }
        let mut step = Step {
            stop: StepStop::Waiting,
            trace: Vec::new(),
            events: Vec::new(),
        };
        if !self.retired_characters.is_empty() || !self.retired_graphics.is_empty() {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if self.vm.world.dialogs.modal.is_some() {
            step.stop = StepStop::Dialog;
            return Ok(step);
        }
        self.vm.resume_accessory_assets(program);
        if !self.pending_skill_profiles().is_empty() {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        let mut remaining = budget;
        if !self.drive_pending_native_transfers(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_pending_native_reaches(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_native_removal(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_pending_native_creations(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_pending_rejected_person_cleanup(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_native_burn(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_pending_factory_acceptance(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_pending_person_visitors(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_native_route_prefix(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_native_shoo(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_pending_native_movement(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_native_route_completion(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_native_snap(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_native_find_location(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_pending_native_turn(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.deletion.is_empty() {
            return self.finish_lot_tick(program, &mut remaining, step);
        }
        if !self.drive_reach_callback(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if !self.drive_cancellation(program, &mut remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        if self.immediate.is_some()
            && self
                .drive_immediate(program, &mut remaining, &mut step)?
                .is_none()
        {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        self.advance_animations(program, &mut step.events)?;
        self.sync_carry(program)?;
        self.service_interactions(program, &mut step.events)?;
        loop {
            // Native lot updates visit object IDs in ascending order. Host-created
            // check/action continuations for an actor finish before the next object.
            let next = next_thread(self)?;
            let Some(id) = next else {
                break;
            };
            if self.native_character_settings.is_some() {
                if let Some(actor) = self.vm.threads[&id].stack.first().map(|f| f.caller) {
                    if self
                        .vm
                        .world
                        .objects
                        .get(&actor)
                        .is_some_and(|o| o.person.is_some())
                        && !self.characters.contains_key(&actor)
                    {
                        return Err(format!(
                            "native person {actor} requires character initialization before its script visit"
                        ));
                    }
                }
            }
            let visit_actor = if let Some((_, actor)) = self.visit {
                actor
            } else {
                self.visited.insert(id);
                let actor = self
                    .vm
                    .scheduled_object_visit
                    .as_ref()
                    .map(|v| v.object())
                    .or_else(|| self.vm.threads[&id].stack.first().map(|f| f.caller));
                self.visit = Some((id, actor));
                self.begin_person_update(id);
                actor
            };
            // A suspended person tail has already completed its object/VM
            // visit. Resume the tail before executing any more script work.
            if self
                .characters
                .values()
                .any(|c| c.person_update.as_ref().is_some_and(|p| p.thread == id))
            {
                if !self.drive_native_person_update(program, id, &mut remaining, &mut step)? {
                    step.stop = StepStop::Budget;
                    return Ok(step);
                }
                self.service_interactions(program, &mut step.events)?;
                self.finish_registry_object()?;
                self.visit = None;
                continue;
            }
            loop {
                if !self.drive_reach_callback(program, &mut remaining, &mut step)? {
                    step.stop = StepStop::Budget;
                    return Ok(step);
                }
                if !self.drive_cancellation(program, &mut remaining, &mut step)? {
                    step.stop = StepStop::Budget;
                    return Ok(step);
                }
                self.sync_spatial_world();
                if self.native_lot.is_none() {
                    if let Some(person) = &mut self.vm.world.person {
                        let mut tiles: BTreeMap<(i32, i32, i32), Vec<i16>> = BTreeMap::new();
                        for &id in &person.object_order {
                            if self.unplaced.contains(&id) {
                                continue;
                            }
                            let body = self
                                .bodies
                                .get(&id)
                                .ok_or("enumerated object has no laboratory body")?;
                            tiles
                                .entry((body.level, body.position.x / 16, body.position.y / 16))
                                .or_default()
                                .push(id);
                        }
                        person.tile_chains = tiles.into_values().collect();
                    }
                }
                let status = self.vm.threads[&id].status.clone();
                match status {
                    Status::Ready | Status::Sleeping { .. } => {
                        if remaining == 0 {
                            step.stop = StepStop::Budget;
                            return Ok(step);
                        }
                        // Observe activation before the first action instruction, so
                        // commands deferred across a budget-paused check are invariant
                        // under the caller's diagnostic instruction budget.
                        let slice_budget = if self.interactions.values().any(|q| q.main == Some(id))
                        {
                            remaining.min(1)
                        } else {
                            remaining
                        };
                        let run = self.vm.run_slice(program, id, slice_budget)?;
                        self.sync_script_directions()?;
                        self.publish_budget_statistics();
                        remaining -= run.trace.len();
                        step.trace.extend(run.trace);
                        if matches!(run.stop, Stop::AwaitingDialog | Stop::AwaitingUiEffect) {
                            step.stop = StepStop::Budget;
                            return Ok(step);
                        }
                        self.service_interactions(program, &mut step.events)?;
                        if run.stop == Stop::AwaitingAccessory {
                            if self.vm.native_accessory_host {
                                step.stop = StepStop::Budget;
                                return Ok(step);
                            }
                            return Err("required accessory asset is not loaded".into());
                        }
                        if run.stop == Stop::Budget && remaining == 0 {
                            step.stop = StepStop::Budget;
                            return Ok(step);
                        }
                        if run.stop == Stop::Sleeping
                            && self.vm.threads[&id].status != Status::Ready
                            && !matches!(self.vm.threads[&id].status,Status::Sleeping{wake_tick} if wake_tick <= self.vm.tick)
                        {
                            break;
                        }
                    }
                    Status::AwaitingAccessory { name, .. } => {
                        if self.vm.native_accessory_host {
                            step.stop = StepStop::Budget;
                            return Ok(step);
                        }
                        return Err(format!("required accessory asset {name:?} is not loaded"));
                    }
                    Status::AwaitingDialog(_) | Status::AwaitingUiEffect(_) => {
                        step.stop = StepStop::Budget;
                        return Ok(step);
                    }
                    Status::AwaitingTransfer(request) => {
                        if self.native_lot.is_some() {
                            if !self.service_native_transfer(
                                program,
                                id,
                                &request,
                                &mut remaining,
                                &mut step,
                            )? {
                                step.stop = StepStop::Budget;
                                return Ok(step);
                            }
                            continue;
                        }
                        self.service_transfer(program, id, &request, &mut step.events)?;
                        self.sync_carry(program)?;
                    }
                    Status::AwaitingBurn(request) => {
                        if !self.service_burn(program, id, request, &mut remaining, &mut step)? {
                            step.stop = StepStop::Budget;
                            return Ok(step);
                        }
                    }
                    Status::AwaitingRemoval(request) => {
                        if !self.service_removal(
                            program,
                            id,
                            &request,
                            &mut remaining,
                            &mut step,
                        )? {
                            step.stop = StepStop::Budget;
                            return Ok(step);
                        }
                    }
                    Status::AwaitingPlacement(request) => {
                        if self.native_lot.is_some() {
                            let e =
                                lot_find_location::Execution::begin(self, program, id, &request)?;
                            self.native_lot.as_mut().unwrap().find_location = Some(e);
                            if !self.drive_native_find_location(
                                program,
                                &mut remaining,
                                &mut step,
                            )? {
                                step.stop = StepStop::Budget;
                                return Ok(step);
                            }
                            continue;
                        }
                        self.service_find_location(program, id, &request, &mut step.events)?;
                    }
                    Status::AwaitingImmediate(request) => {
                        if let Some(result) = self.begin_immediate(
                            program,
                            request.actor,
                            request.action,
                            Some((id, request.id)),
                        )? {
                            self.vm.finish_immediate(program, id, request.id, result)?;
                        } else if self
                            .drive_immediate(program, &mut remaining, &mut step)?
                            .is_none()
                        {
                            step.stop = StepStop::Budget;
                            return Ok(step);
                        }
                    }
                    Status::AwaitingCreation(request) => {
                        if !self.service_creation(
                            program,
                            id,
                            request,
                            &mut remaining,
                            &mut step,
                        )? {
                            step.stop = StepStop::Budget;
                            return Ok(step);
                        }
                    }
                    Status::AwaitingRoute(request) => {
                        if self.native_lot.is_some() {
                            if self.native_lot.as_ref().unwrap().shoo.is_some() {
                                if !self.drive_native_shoo(program, &mut remaining, &mut step)? {
                                    step.stop = StepStop::Budget;
                                    return Ok(step);
                                }
                                continue;
                            }
                            if self.native_lot.as_ref().unwrap().completion.is_some() {
                                if !self.drive_native_route_completion(
                                    program,
                                    &mut remaining,
                                    &mut step,
                                )? {
                                    step.stop = StepStop::Budget;
                                    return Ok(step);
                                }
                                continue;
                            }
                            if self
                                .characters
                                .get(&request.caller)
                                .is_some_and(|r| r.native_walk.is_some())
                            {
                                if !self.service_original_animator_path(
                                    program,
                                    id,
                                    &request,
                                    &mut remaining,
                                    &mut step,
                                )? {
                                    if self.characters[&request.caller]
                                        .native_walk
                                        .as_ref()
                                        .and_then(|w| w.poll.as_ref())
                                        .is_some_and(|poll| poll.request().is_some())
                                    {
                                        step.stop = StepStop::Budget;
                                        return Ok(step);
                                    }
                                    if let Some(polls) = polls.as_deref_mut() {
                                        // Retained0x1b reentry yielded without a new
                                        // VM instruction. Report that original call
                                        // without allocating a request or spending
                                        // another instruction from the slice budget.
                                        let recorded = |t: &Trace| t.tick == self.vm.tick
                                            && t.thread == id && t.location == request.origin && t.opcode == 27;
                                        if !step.trace.iter().any(recorded)
                                            && !polls.iter().any(|p| recorded(&p.instruction))
                                            && let Some(instruction) = program.modules.get(&request.origin.source)
                                                .and_then(|m| m.routines.get(&request.origin.bhav))
                                                .and_then(|r| r.body.instructions.get(request.origin.instruction))
                                            && instruction.opcode == 27
                                        {
                                            polls.push(HostPollTrace {before_instruction:step.trace.len(),
                                                instruction:Trace {tick:self.vm.tick,thread:id,
                                                    depth:self.vm.threads[&id].stack.len(),location:request.origin.clone(),
                                                    opcode:27,operands:instruction.operands,
                                                    effect:"await route service response".into()}});
                                        }
                                    }
                                    break;
                                }
                                continue;
                            }
                            if self.routes.contains_key(&id) {
                                if !self.service_original_movement(
                                    program,
                                    id,
                                    &request,
                                    &mut remaining,
                                    &mut step,
                                )? {
                                    if self.native_lot.as_ref().is_some_and(|n| {
                                        n.movement.is_some() || n.completion.is_some()
                                    }) {
                                        step.stop = StepStop::Budget;
                                        return Ok(step);
                                    }
                                    break;
                                }
                                continue;
                            }
                            if self.service_original_portals(
                                program,
                                id,
                                &request,
                                &mut step.events,
                            )? {
                                continue;
                            }
                            let execution =
                                lot_route::live::Execution::begin(self, program, id, &request)?;
                            self.native_lot.as_mut().unwrap().route = Some(execution);
                            if !self.drive_native_route_prefix(
                                program,
                                &mut remaining,
                                &mut step,
                            )? {
                                step.stop = StepStop::Budget;
                                return Ok(step);
                            }
                            continue;
                        }
                        if self.shoo_creation.is_some()
                            && !self.drive_shoo_creation(program, &mut remaining, &mut step)?
                        {
                            step.stop = StepStop::Budget;
                            return Ok(step);
                        }
                        if !self.service_route(program, id, &request, &mut step.events)? {
                            break;
                        }
                    }
                    Status::AwaitingSnap(request) => {
                        if self.native_lot.is_some() {
                            let e = lot_snap::Execution::begin(self, program, id, &request)?;
                            self.native_lot.as_mut().unwrap().snap = Some(e);
                            if !self.drive_native_snap(program, &mut remaining, &mut step)? {
                                step.stop = StepStop::Budget;
                                return Ok(step);
                            }
                            continue;
                        }
                        if self.shoo_creation.is_some() {
                            if !self.drive_shoo_creation(program, &mut remaining, &mut step)? {
                                step.stop = StepStop::Budget;
                                return Ok(step);
                            }
                        } else {
                            self.service_snap(program, id, &request, &mut step.events)?;
                        }
                    }
                    Status::AwaitingAnimation(request) => {
                        if self.characters.contains_key(&request.caller)
                            && self.native_command_profile(program, id, &request)?
                        {
                            self.defer_skill_profile(
                                request.caller,
                                character_profile::Origin::Command {
                                    thread: id,
                                    request: request.clone(),
                                },
                            )?;
                            step.stop = StepStop::Budget;
                            return Ok(step);
                        }
                        if program.animations.is_empty()
                            && !self.characters.contains_key(&request.caller)
                        {
                            break;
                        }
                        let ready =
                            self.service_animation(program, id, &request, &mut step.events)?;
                        self.sync_carry(program)?;
                        if !ready {
                            if self.characters.contains_key(&request.caller) {
                                if let Some(polls) = polls.as_deref_mut() {
                                    // A newly executed0x2c already has its VM report.
                                    // A retained request still invokes the native
                                    // command on its next object visit, without
                                    // allocating another request or changing budgets.
                                    if !step.trace.iter().any(|t| t.tick == self.vm.tick && t.thread == id && t.location == request.origin && t.opcode == 44) {
                                        let depth = self.vm.threads[&id].stack.len();
                                        polls.push(HostPollTrace { before_instruction: step.trace.len(), instruction: Trace { tick:self.vm.tick, thread:id, depth, location:request.origin.clone(), opcode:44, operands:request.operands, effect:"await animation service response".into() } });
                                    }
                                }
                            }
                            break;
                        }
                    }
                    Status::AwaitingReach(request) => {
                        if self.native_lot.is_some() {
                            match self.service_native_reach(
                                program,
                                id,
                                &request,
                                &mut remaining,
                                &mut step,
                            )? {
                                lot_reach::live::Outcome::Ready => {}
                                lot_reach::live::Outcome::Waiting => break,
                                lot_reach::live::Outcome::Budget => {
                                    step.stop = StepStop::Budget;
                                    return Ok(step);
                                }
                            }
                        } else if !self.service_reach(program, id, &request, &mut step.events)? {
                            break;
                        }
                    }
                    Status::AwaitingTurn(request) => {
                        if self.characters.contains_key(&request.caller) {
                            match self.service_native_turn(
                                program,
                                id,
                                &request,
                                &mut remaining,
                                &mut step,
                            )? {
                                native_turn::Outcome::Ready => {}
                                native_turn::Outcome::Waiting => break,
                                native_turn::Outcome::Budget => {
                                    step.stop = StepStop::Budget;
                                    return Ok(step);
                                }
                            }
                        } else if !self.service_turn(program, id, &request, &mut step.events)? {
                            break;
                        }
                    }
                    Status::Returned(value) => {
                        self.release(id);
                        if let Some(action) = self.pending_actions.remove(&id) {
                            self.event(
                                &mut step.events,
                                id,
                                format!("eligibility returned {value}"),
                            );
                            if value {
                                let next = self.vm.spawn(
                                    program,
                                    Start {
                                        owner: &action.owner,
                                        bhav: action.bhav,
                                        caller: action.actor,
                                        callee: action.target,
                                        stack_object: action.target,
                                        args: action.args,
                                        check: false,
                                    },
                                )?;
                                self.event(
                                    &mut step.events,
                                    next,
                                    "queued original interaction action".into(),
                                );
                            }
                        }
                        break;
                    }
                    Status::NativeSuspended(_) | Status::RegistrySuspended(_) | Status::NativeRegistryReentry(_) => {
                        return Err(
                            "retained native suspension requires an eligible registry Tick".into(),
                        );
                    }
                    Status::Faulted(_) => {
                        self.release(id);
                        step.stop = StepStop::Faulted;
                        return Ok(step);
                    }
                }
            }
            if let Some(actor) = visit_actor {
                if self.vm.object_threads.get(&actor) == Some(&id) && !self.vm.threads[&id].check {
                    self.vm.finish_object_update(actor, true)?;
                }
            }
            self.update_turn(program, id, &mut step.events)?;
            let native_person = self
                .vm
                .object_threads
                .iter()
                .any(|(actor, thread)| *thread == id && self.characters.contains_key(actor));
            if native_person {
                if !self.drive_native_person_update(program, id, &mut remaining, &mut step)? {
                    step.stop = StepStop::Budget;
                    return Ok(step);
                }
            } else {
                self.finish_person_update(id)?;
            }
            self.service_interactions(program, &mut step.events)?;
            self.finish_registry_object()?;
            self.visit = None;
        }
        self.retire_interaction_threads();
        self.finish_lot_tick(program, &mut remaining, step)
    }
    fn finish_lot_tick(
        &mut self,
        program: &Program,
        remaining: &mut usize,
        mut step: Step,
    ) -> Result<Step, String> {
        if self.vm.pump_dialogs()?.is_some() {
            step.stop = StepStop::Dialog;
            return Ok(step);
        }
        if !self.flush_removals(program, remaining, &mut step)? {
            step.stop = StepStop::Budget;
            return Ok(step);
        }
        self.visited.clear();
        step.stop = if self
            .vm
            .threads
            .values()
            .any(|t| matches!(t.status, Status::AwaitingAnimation(_)))
            && program.animations.is_empty()
        {
            StepStop::Animation
        } else if self
            .vm
            .threads
            .values()
            .all(|t| matches!(t.status, Status::Returned(_)))
        {
            StepStop::Complete
        } else {
            StepStop::Waiting
        };
        Ok(step)
    }
    fn validate_pass_boundary(&self) -> Result<(), String> {
        if self.cursor_work_pending() {
            return Err("cursor factory must return before household ticks".into());
        }
        if self.object_pickup.is_some() || self.object_placement.is_some() {
            return Err("object pickup or placement must return before household ticks".into());
        }
        if self.ground_placement.is_some() || self.controller_reconciliation.is_some() {
            return Err("ground input must return before household ticks".into());
        }
        if self.vm.player_menu.is_some() {
            return Err("player-menu query remains in current tick".into());
        }
        if self.ui_wait_callback.is_some() {
            return Err("UI wait callback remains in current tick".into());
        }
        if self.native_selection.is_some() {
            return Err("native selection remains in current tick".into());
        }
        if self.synchronous_removal.is_some() {
            return Err("synchronous removal remains in current tick".into());
        }
        if self.portrait_person.is_some() {
            return Err("temporary portrait work remains in current tick".into());
        }
        if !self.retired_characters.is_empty() || !self.retired_graphics.is_empty() {
            return Err("native graphic destruction remains in current tick".into());
        }
        if self.characters.values().any(|c| {
            c.refresh_tick.is_some()
                || c.ordinary_update.is_some()
                || c.person_update.is_some()
                || c.skill_profile.is_some()
                || c.native_turn
                    .as_ref()
                    .is_some_and(|t| t.poll.is_some() || t.physical.is_some())
        }) {
            return Err("native character refresh remains in current tick".into());
        }
        if self.native_lot.as_ref().is_some_and(|n| n.shoo.is_some()) {
            return Err("native shoo work remains in current tick".into());
        }
        if self.person_factories.values().any(|s| !s.finalized) {
            return Err("native person creation work remains in current tick".into());
        }
        if !self.native_transfers.is_empty() {
            return Err("native transfer work remains in current tick".into());
        }
        if self.native_reaches.values().any(|e| e.synchronous()) {
            return Err("native Reach host work remains in current tick".into());
        }
        if !self.native_creations.is_empty() {
            return Err("native object creation work remains".into());
        }
        if self.native_burn.is_some() {
            return Err("native Burn work remains in the current tick".into());
        }
        if self
            .native_lot
            .as_ref()
            .is_some_and(|n| n.removal.is_some())
        {
            return Err("native physical removal remains in current tick".into());
        }
        if !self.deletion.is_empty() {
            return Err("object deletion remains in current tick".into());
        }
        if self.shoo_creation.is_some() {
            return Err("shoo factory remains in current tick".into());
        }
        if self.reach_callback.is_some() {
            return Err("Reach callback remains in current tick".into());
        }
        if self.cancellation.is_some()
            || self
                .vm
                .world
                .actions
                .as_ref()
                .is_some_and(|a| !a.callbacks.is_empty())
        {
            return Err("action cancellation remains in current tick".into());
        }
        if self.visit.is_some() || !self.visited.is_empty() {
            return Err("cooperative lot pass remains in current tick".into());
        }
        if self.immediate.is_some() {
            return Err("immediate callback remains in current tick".into());
        }
        if !self.removals.is_empty() {
            return Err("deferred object deletion remains in this tick".into());
        }
        if self
            .household
            .as_ref()
            .is_some_and(|h| h.people.values().any(|p| p.started.is_some()))
        {
            return Err("person update remains in the current tick".into());
        }
        if self.animations.values().any(|p| p.last_tick < self.vm.tick) {
            return Err("animation work remains in the current tick".into());
        }
        if self
            .turns
            .values()
            .any(|p| p.last_tick != Some(self.vm.tick))
        {
            return Err("standing turn work remains in the current tick".into());
        }
        Ok(())
    }
    pub(crate) fn finish_pass_clock(&mut self, complete: bool) -> Result<(), String> {
        self.validate_pass_boundary()?;
        self.vm.finish_registry_pass(
            self.native_scheduler
                .as_ref()
                .ok_or("registry scheduler absent")?
                .pass()?,
        )?;
        self.advance_household_clock(complete);
        Ok(())
    }
    fn event(&self, events: &mut Vec<Event>, thread: ThreadId, detail: String) {
        events.push(Event {
            tick: self.vm.tick,
            thread,
            detail,
        });
    }
    fn release(&mut self, id: ThreadId) {
        self.routes.remove(&id);
        for resident in self.characters.values_mut() {
            if resident
                .native_turn
                .as_ref()
                .is_some_and(|t| t.thread == id)
            {
                resident.native_turn = None;
            }
            if resident
                .native_walk
                .as_ref()
                .is_some_and(|w| w.thread == id)
            {
                resident.native_walk = None;
            }
        }
        self.reservations.retain(|_, owner| *owner != id);
    }
    fn body(&self, id: ObjectId) -> Result<&Body, String> {
        self.bodies
            .get(&id)
            .ok_or_else(|| format!("object #{id} has no spatial body"))
    }
    fn clear(&self, caller: ObjectId, position: Point, ignore: Option<ObjectId>) -> bool {
        self.clear_with_person_policy(caller, position, ignore, false)
    }
    fn route_classification(&self, caller: ObjectId, position: Point) -> i32 {
        if self.clear(caller, position, None) {
            6
        } else if self.clear_with_person_policy(caller, position, None, true) {
            5
        } else {
            0
        }
    }
    fn clear_with_person_policy(
        &self,
        caller: ObjectId,
        position: Point,
        ignore: Option<ObjectId>,
        ignore_people: bool,
    ) -> bool {
        let Some(body) = self.bodies.get(&caller) else {
            return false;
        };
        self.clear_on_level(caller, body.level, position, ignore, ignore_people)
    }
    fn clear_on_level(
        &self,
        caller: ObjectId,
        level: i32,
        position: Point,
        ignore: Option<ObjectId>,
        ignore_people: bool,
    ) -> bool {
        let bounds = self.collision_bounds(caller, position);
        self.room.contains(bounds)
            && self
                .architecture
                .as_ref()
                .is_none_or(|a| a.bounds_clear(level, bounds, &self.active_portals(caller)))
            && (level != 1 || !self.room.obstacles.iter().any(|r| r.overlaps(bounds)))
            && self
                .blocking_object(caller, level, position, ignore, ignore_people)
                .is_none()
    }
    fn blocking_object(
        &self,
        caller: ObjectId,
        level: i32,
        position: Point,
        ignore: Option<ObjectId>,
        ignore_people: bool,
    ) -> Option<ObjectId> {
        let bounds = self.collision_bounds(caller, position);
        self.bodies.iter().find_map(|(&id, b)| {
            let blocks = id != caller
                && b.level == level
                && Some(id) != ignore
                && !(ignore_people && self.vm.world.objects[&id].person.is_some())
                && self.vm.world.objects[&id].container.is_none()
                && {
                    // The legacy grid fixture reserves an edge. Native lot
                    // routes use the person's current physical footprint;
                    // reserving a whole rectangle-graph segment would turn
                    // a moving Sim into a room-length obstacle.
                    if self.architecture.is_some() {
                        let moving = &self.vm.world.objects[&caller];
                        let other = &self.vm.world.objects[&id];
                        let mp = moving.person.is_some();
                        let op = other.person.is_some();
                        if mp || op {
                            let obstacle = if mp { other } else { moving };
                            let flags = obstacle.data.get(&8).copied().unwrap_or(0) as u16;
                            if if flags & 4 == 0 {
                                flags & 16 != 0
                            } else {
                                flags & 2 == 0
                            } {
                                return None;
                            }
                        } else {
                            let mf = moving.data.get(&8).copied().unwrap_or(0) as u16;
                            let of = other.data.get(&8).copied().unwrap_or(0) as u16;
                            if mf & 4 != 0 || of & 4 != 0 {
                                return (position.x / 16 == b.position.x / 16
                                    && position.y / 16 == b.position.y / 16
                                    && moving.data.get(&63).copied().unwrap_or(0)
                                        & other.data.get(&63).copied().unwrap_or(0)
                                        & 1
                                        != 0)
                                    .then_some(id);
                            }
                        }
                    }
                    let mut occupied = self.collision_bounds(id, b.position);
                    if self.architecture.is_none()
                        && let Some(next) = self
                            .routes
                            .values()
                            .find(|r| r.caller == id)
                            .and_then(|r| r.path.get(r.next))
                    {
                        let end = self.collision_bounds(id, *next);
                        occupied.min.x = occupied.min.x.min(end.min.x);
                        occupied.min.y = occupied.min.y.min(end.min.y);
                        occupied.max.x = occupied.max.x.max(end.max.x);
                        occupied.max.y = occupied.max.y.max(end.max.y);
                    }
                    bounds.overlaps(occupied)
                };
            blocks.then_some(id)
        })
    }

    fn occupied(&self, target: ObjectId, caller: ObjectId, id: ThreadId) -> Result<bool, String> {
        let obj = self
            .vm
            .world
            .objects
            .get(&target)
            .ok_or("missing target object")?;
        Ok(obj.slots.iter().flatten().any(|&x| x != caller)
            || self
                .reservations
                .get(&target)
                .is_some_and(|owner| *owner != id))
    }
    fn person(&mut self, actor: ObjectId, field: i16, value: i16) -> Result<(), String> {
        let data = self
            .vm
            .world
            .objects
            .get_mut(&actor)
            .and_then(|o| o.person.as_mut())
            .ok_or("route caller is not a person")?;
        *data
            .get_mut(&field)
            .ok_or_else(|| format!("person field {field} is not declared"))? = value;
        Ok(())
    }
    fn face(&mut self, actor: ObjectId, facing: u8) -> Result<(), String> {
        let o = self
            .vm
            .world
            .objects
            .get_mut(&actor)
            .ok_or("missing actor")?;
        *o.data
            .get_mut(&1)
            .ok_or("direction field is not declared")? = facing.trailing_zeros() as i16;
        self.bodies
            .get_mut(&actor)
            .ok_or("missing actor body")?
            .facing = facing;
        Ok(())
    }

    /// Direction is the native object's raw data word, including writes from
    /// BHAV expressions. The host body caches that word for geometry/rendering.
    fn sync_script_directions(&mut self) -> Result<(), String> {
        let mut changes = vec![];
        let mut objects = self.vm.world.objects.iter().peekable();
        for (&id, body) in &self.bodies {
            while objects.peek().is_some_and(|(key, _)| **key < id) {
                objects.next();
            }
            let object = objects
                .next_if(|(key, _)| **key == id)
                .map(|(_, object)| object)
                .expect("no entry found for key");
            let direction = *object.data.get(&1).ok_or("missing object direction")?;
            // BHAVs temporarily add to the raw direction word before wrapping.
            // Preserve that word in the VM; the eight-way spatial cache projects
            // its compass bits even at an instruction-budget boundary.
            let compass = (direction as u16 & 7) as u8;
            if compass == body.facing.trailing_zeros() as u8 {
                continue;
            }
            if object.multitile.is_some()
                || self.routes.values().any(|r| r.caller == id)
                || self.turns.contains_key(&id)
            {
                return Err("script direction write during group placement or active movement is unsupported".into());
            }
            changes.push((id, 1u8 << compass));
        }
        for (id, facing) in changes {
            self.bodies.get_mut(&id).unwrap().facing = facing;
        }
        Ok(())
    }
    fn fail_route(
        &mut self,
        program: &Program,
        id: ThreadId,
        request: &RouteRequest,
        code: i16,
        reason: &str,
        events: &mut Vec<Event>,
    ) -> Result<bool, String> {
        self.fail_route_blocked(program, id, request, code, 0, reason, events)
    }
    fn fail_route_blocked(
        &mut self,
        program: &Program,
        id: ThreadId,
        request: &RouteRequest,
        code: i16,
        blocker: ObjectId,
        reason: &str,
        events: &mut Vec<Event>,
    ) -> Result<bool, String> {
        let code = if code == 0 { 1 } else { code };
        self.person(request.caller, 62, code)?;
        self.release(id);
        if !request.no_failure_trees && self.vm.world.objects[&request.caller].definition.is_some()
        {
            if let Some((_, entry)) =
                self.vm
                    .resolve_named_tree(program, request.caller, "Route failure feedback")?
            {
                let owner = self.vm.world.objects[&request.target]
                    .definition
                    .as_ref()
                    .ok_or("route feedback target definition")?
                    .source
                    .clone();
                self.vm.push_route_service(
                    program,
                    id,
                    owner,
                    entry,
                    request.target,
                    Some(&[code, blocker, 0, 0]),
                )?;
                self.person(request.caller, 72, 0)?;
                self.route_feedback.insert(
                    request.id,
                    routing::FailureFeedback {
                        thread: id,
                        code,
                        blocker,
                    },
                );
                self.event(events,id,format!("route failure invokes original feedback: {reason}; result {code}, blocker #{blocker}"));
                return Ok(true);
            }
        }
        if self.native_lot.is_some() {
            self.begin_native_route_completion(id, request, false, i32::from(code))?;
        } else {
            self.complete_route(program, id, request, false)?;
        }
        self.event(
            events,
            id,
            format!("route failed: {reason}; RouteResult={code}; branch false"),
        );
        Ok(true)
    }
    fn obstructed_route(
        &mut self,
        program: &Program,
        id: ThreadId,
        request: &RouteRequest,
        events: &mut Vec<Event>,
    ) -> Result<bool, String> {
        if self.architecture.is_some() {
            return self.begin_route_retry(program, id, request, 3, events);
        }
        self.fail_route(program, id, request, 3, "path became blocked", events)
    }
    fn begin_route_retry(
        &mut self,
        program: &Program,
        id: ThreadId,
        request: &RouteRequest,
        result: i16,
        events: &mut Vec<Event>,
    ) -> Result<bool, String> {
        self.release(id);
        let state = self
            .route_control
            .get_mut(&request.id)
            .ok_or("missing route retry context")?;
        state.controller.result = result;
        state.controller.phase = contention::Phase::Retry;
        state.deciding = true;
        self.route_contention(program, id, request, events)
    }

    fn service_route(
        &mut self,
        program: &Program,
        id: ThreadId,
        request: &RouteRequest,
        events: &mut Vec<Event>,
    ) -> Result<bool, String> {
        if self.architecture.is_some() && !self.route_control.contains_key(&request.id) {
            self.route_control.insert(
                request.id,
                contention::State {
                    thread: id,
                    caller: request.caller,
                    controller: contention::Controller::new(self.body(request.caller)?.position),
                    link: None,
                    blocker_success: false,
                    callback: None,
                    deciding: false,
                    shoo_result: None,
                    last_portal: None,
                    original: None,
                },
            );
        }
        if let Some(call) = self.vm.threads[&id].service.as_ref()
            && matches!(&call.resume_status, Status::AwaitingRoute(r) if r.id == request.id)
        {
            let success = call.result.ok_or("route resumed before service returned")?;
            if self
                .route_control
                .get_mut(&request.id)
                .and_then(|s| s.callback.take())
                .is_some()
            {
                self.vm.pop_service(id)?;
                return self.route_contention(program, id, request, events);
            }
            if let Some(feedback) = self.route_feedback.remove(&request.id) {
                self.vm.pop_service(id)?;
                self.person(request.caller, 62, feedback.code)?;
                self.complete_route(program, id, request, false)?;
                self.event(
                    events,
                    id,
                    format!(
                        "route feedback completed; restored RouteResult={}; branch false",
                        feedback.code
                    ),
                );
                return Ok(true);
            }

            let def = self.vm.world.objects[&call.callee]
                .definition
                .as_ref()
                .ok_or("service definition")?;
            let portal = program.modules[&def.source]
                .object_functions
                .get(&def.object)
                .and_then(|f| f.get(15))
                .is_some_and(|f| f.action != 0 && f.action == call.bhav);
            let sit = program.modules[&def.source]
                .object_functions
                .get(&def.object)
                .and_then(|f| f.get(26))
                .is_some_and(|f| f.action != 0 && f.action == call.bhav);
            let callee = call.callee;
            self.vm.pop_service(id)?;
            self.release(id);
            if sit {
                if !success {
                    let code = self.vm.world.objects[&request.caller]
                        .person
                        .as_ref()
                        .unwrap()[&62];
                    return self.fail_route(
                        program,
                        id,
                        request,
                        if code == 0 { 5 } else { code },
                        "Sit service failed",
                        events,
                    );
                }
                self.person(request.caller, 62, 0)?;
                self.complete_route(program, id, request, true)?;
                self.event(
                    events,
                    id,
                    "route completed through original Sit service".into(),
                );
                return Ok(true);
            }
            if portal {
                if !success
                    && self.vm.world.objects[&request.caller].data[&29]
                        != self.vm.world.objects[&request.target].data[&29]
                {
                    let portals = self.portal_nodes();
                    self.portal_cache
                        .entry(request.caller)
                        .or_default()
                        .fail(&portals, callee);
                    self.event(
                        events,
                        id,
                        format!("portal #{callee} failed; trying native alternative"),
                    );
                }
            } else if !success
                || self.vm.world.objects[&request.caller]
                    .person
                    .as_ref()
                    .unwrap()[&0]
                    != 0
            {
                return self.fail_route(
                    program,
                    id,
                    request,
                    6,
                    "Stand failed to clear posture",
                    events,
                );
            }
        }

        if self
            .route_control
            .get(&request.id)
            .is_some_and(|s| s.deciding)
        {
            return self.route_contention(program, id, request, events);
        }
        if self.route_interrupted(id, request.caller)? {
            self.vm.threads.get_mut(&id).unwrap().interrupted = false;
            if self.architecture.is_some() {
                self.person(request.caller, 62, 4)?;
                self.release(id);
                self.complete_route(program, id, request, false)?;
                return Ok(true);
            }
            return self.fail_route(program, id, request, 4, "interrupted", events);
        }
        // The native route state machine does not reserve a whole target merely
        // because it contains something. Tables must remain reachable with food
        // on them; native seat candidate tests own seated occupancy. Preserve the
        // older isolated chair fixture's reservation policy outside architecture.
        if self.architecture.is_none() && self.occupied(request.target, request.caller, id)? {
            return self.fail_route(
                program,
                id,
                request,
                9,
                "chair occupied or reserved",
                events,
            );
        }
        if !self.routes.contains_key(&id) {
            if let Some((seat, _)) = self.vm.world.objects[&request.caller].container {
                if self.vm.world.objects[&request.caller]
                    .person
                    .as_ref()
                    .unwrap()[&0]
                    != 1
                {
                    return Err("routing from this contained posture is unsupported".into());
                }
                // The original state 1 prepares candidates before deciding
                // whether Stand is necessary. Reusing the current seat preserves
                // posture/entry flags and avoids a stand/sit cycle while dining.
                if self.architecture.is_some() {
                    if let sims_vm::routing::Destination::Slot { slot, version, .. } =
                        &request.destination
                    {
                        if slot.sitting > 0 {
                            let mut context = routing::standing_slot(
                                slot,
                                *version,
                                self.body(request.target)?,
                                self.body(request.caller)?,
                            )?;
                            context.walls = self.architecture.as_ref().unwrap().wall_mask(
                                self.body(request.target)?.level,
                                self.body(request.target)?.position,
                                self.body(request.target)?.facing.trailing_zeros() as u8,
                            );
                            let mut rng = self.vm.rng_state;
                            let (mut set, _) = routing::posture_candidates(
                                &context,
                                slot.sitting,
                                &mut rng,
                                |point| self.seat_at(program, request.caller, point),
                                |p, _| {
                                    self.route_classification(request.caller, Point::new(p.y, p.x))
                                },
                            )?;
                            routing::limit_candidates(&mut set, 48, &mut rng);
                            self.vm.rng_state = rng;
                            if routing::keeps_current_seat(
                                set.candidates.iter().map(|c| c.seat),
                                seat,
                                |id| {
                                    self.vm
                                        .world
                                        .objects
                                        .get(&id)
                                        .and_then(|o| o.definition.as_ref())
                                        .and_then(|d| {
                                            program.modules[&d.source]
                                                .object_functions
                                                .get(&d.object)
                                        })
                                        .and_then(|f| f.get(26))
                                        .is_some_and(|f| f.action != 0)
                                },
                            ) {
                                self.person(request.caller, 62, 0)?;
                                self.complete_route(program, id, request, true)?;
                                self.event(
                                    events,
                                    id,
                                    format!("route reused current seat #{seat}"),
                                );
                                return Ok(true);
                            }
                        }
                    }
                }
                let def = self.vm.world.objects[&seat]
                    .definition
                    .as_ref()
                    .ok_or("seat definition missing")?;
                let module = &program.modules[&def.source];
                let words = &module.object_definition_words[&def.object];
                if words.len() <= 0x84 / 2 {
                    return Err("incomplete seat definition".into());
                }
                let stand = module
                    .object_stand_functions
                    .get(&def.object)
                    .copied()
                    .unwrap_or(0);
                if words[0x40 / 2] == 0 || stand == 0 {
                    return self.fail_route(
                        program,
                        id,
                        request,
                        1,
                        "seat has no Sit/Stand function",
                        events,
                    );
                }
                let position = self.body(request.caller)?.position;
                let target = self.body(request.target)?.position;
                let departure = routing::select_departure(
                    Point::new(position.y, position.x),
                    Point::new(target.y, target.x),
                    self.body(seat)?.facing.trailing_zeros() as i16,
                    words[0x84 / 2] as u16,
                    |p, _| self.route_classification(request.caller, Point::new(p.y, p.x)),
                );
                let owner = def.source.clone();
                self.vm.push_service_call(
                    program,
                    id,
                    Start {
                        owner: &owner,
                        bhav: stand,
                        caller: request.caller,
                        callee: seat,
                        stack_object: seat,
                        args: vec![departure.flags as i16, 0, 0, 0],
                        check: false,
                    },
                )?;
                // 0x0045b6de writes this after scheduling Stand and before yielding.
                self.person(request.caller, 37, departure.flags as i16)?;
                self.event(
                    events,
                    id,
                    format!(
                        "route invokes original Stand on #{seat}; departure flags {:#04x}",
                        departure.flags
                    ),
                );
                return Ok(true);
            }
            let cross_room = match &request.destination {
                sims_vm::routing::Destination::Slot { slot, .. } => slot.flags & 0x800 != 0,
                sims_vm::routing::Destination::Relative(r) => r.flags & 0x800 != 0,
                _ => false,
            };
            if self.architecture.is_some()
                && (self.body(request.caller)?.level != self.body(request.target)?.level
                    || !cross_room
                        && self.vm.world.objects[&request.caller].data[&29]
                            != self.vm.world.objects[&request.target].data[&29])
            {
                let Some(door) = self.next_portal(request) else {
                    return self.fail_route(
                        program,
                        id,
                        request,
                        2,
                        "no connecting portal",
                        events,
                    );
                };
                let def = self.vm.world.objects[&door].definition.as_ref().unwrap();
                let owner = def.source.clone();
                let entry = program.modules[&owner].object_functions[&def.object][15].action;
                if entry == 0 {
                    return self.fail_route(
                        program,
                        id,
                        request,
                        2,
                        "portal has no crossing function",
                        events,
                    );
                }
                self.vm
                    .push_portal_service(program, id, owner, entry, door)?;
                let portals = self.portal_nodes();
                let actor = self.portal_location(request.caller).unwrap();
                self.portal_cache
                    .entry(request.caller)
                    .or_default()
                    .attempt(&portals, actor, door);
                self.route_control.get_mut(&request.id).unwrap().last_portal = Some(door);
                self.event(
                    events,
                    id,
                    format!("route invokes original portal function on #{door}"),
                );
                return Ok(true);
            }
            let target = self.body(request.target)?.clone();
            let caller = self.body(request.caller)?.clone();
            let supported_destination = request.destination.clone();
            let mut scores = Vec::new();
            let mut seats = Vec::new();
            let mut maximum_score = 0;
            let mut failed_result = 7;
            let mut blocked_seat = 0;
            let goals = if let sims_vm::routing::Destination::Slot { slot, version, .. } =
                &supported_destination
                && (geometry::destinations(slot, *version, &target, &caller).is_err()
                    || self.architecture.is_some()
                        && slot.flags & 255 != 0
                        && routing::standing_slot(slot, *version, &target, &caller).is_ok())
            {
                let mut context = routing::standing_slot(slot, *version, &target, &caller)?;
                if let Some(a) = &self.architecture {
                    context.walls = a.wall_mask(
                        target.level,
                        target.position,
                        target.facing.trailing_zeros() as u8,
                    );
                }
                let mut rng = self.vm.rng_state;
                let (mut set, blocker) = routing::posture_candidates(
                    &context,
                    slot.sitting,
                    &mut rng,
                    |point| self.seat_at(program, request.caller, point),
                    |p, _| self.route_classification(request.caller, Point::new(p.y, p.x)),
                )?;
                maximum_score = set.maximum;
                failed_result = if set.result == 0 {
                    1
                } else {
                    i16::from(set.result)
                };
                blocked_seat = blocker;
                routing::limit_candidates(
                    &mut set,
                    self.route_candidate_limit(request.caller),
                    &mut rng,
                );
                self.vm.rng_state = rng;
                for candidate in &set.candidates {
                    seats.push((
                        routing::candidate_destination(&context, candidate),
                        candidate.seat,
                    ));
                    scores.push((
                        routing::candidate_destination(&context, candidate),
                        candidate.score,
                    ));
                }
                scores.iter().map(|(goal, _)| *goal).collect()
            } else {
                routing::destinations(&supported_destination, &target, &caller)?
            };
            let goals: Vec<_> = goals
                .into_iter()
                .filter(|g| self.clear(request.caller, g.position, None))
                .collect();
            if goals.is_empty() {
                if self.architecture.is_some() {
                    return self.begin_route_retry(program, id, request, failed_result, events);
                }
                return self.fail_route_blocked(
                    program,
                    id,
                    request,
                    failed_result,
                    blocked_seat,
                    "no valid destination",
                    events,
                );
            }
            // The interactive chair profile prefers the local front even if another
            // approach is shorter. Test reachability, not just destination clearance:
            // an enclosed front slot must not prevent a usable side/back approach.
            // Generic routes and the headless compatibility fixtures keep their
            // existing shortest-route policy.
            let prefer_front = self.architecture.is_none()
                && matches!(&request.destination, sims_vm::routing::Destination::Slot { slot, .. } if slot.flags & 512 == 0)
                && self.interactions.values().any(|q| {
                    q.active.as_ref().is_some_and(|a| {
                        a.thread == id
                            && a.phase == interaction::Phase::Acting
                            && a.item.target == request.target
                    })
                });
            let front: Vec<_> = goals
                .iter()
                .copied()
                .filter(|g| prefer_front && g.entry_flags == 1)
                .collect();
            let find_path = |goals: &[Destination]| {
                geometry::pathfind(&self.room, caller.position, goals, |p| {
                    self.clear(request.caller, p, None)
                        && (cross_room
                            || self.architecture.as_ref().is_none_or(|a| {
                                a.room_at(caller.level, p)
                                    == a.room_at(caller.level, caller.position)
                            }))
                })
            };
            let preferred = if front.is_empty() {
                None
            } else {
                find_path(&front)
            };
            let ranked = || {
                if self.architecture.is_some() && !scores.is_empty() {
                    let weighted: Vec<_> = scores
                        .iter()
                        .filter(|(g, _)| goals.contains(g))
                        .map(|(g, score)| (*g, (16 * (maximum_score - score)).max(0) as u32 * 1000))
                        .collect();
                    return geometry::pathfind_scored(
                        &self.room,
                        caller.position,
                        &weighted,
                        |p| {
                            self.clear(request.caller, p, None)
                                && (cross_room
                                    || self.architecture.as_ref().is_none_or(|a| {
                                        a.room_at(caller.level, p)
                                            == a.room_at(caller.level, caller.position)
                                    }))
                        },
                    );
                }
                let mut tiers: Vec<_> = scores.iter().map(|(_, score)| *score).collect();
                tiers.sort_unstable();
                tiers.dedup();
                tiers.into_iter().rev().find_map(|score| {
                    let group: Vec<_> = scores
                        .iter()
                        .filter(|(_, s)| *s == score)
                        .map(|(g, _)| *g)
                        .collect();
                    find_path(&group)
                })
            };
            let found = if self.architecture.is_some() {
                let weighted: Vec<_> = goals
                    .iter()
                    .map(|g| {
                        let score = scores
                            .iter()
                            .find(|(candidate, _)| candidate == g)
                            .map_or(0, |(_, s)| (16 * (maximum_score - s)).max(0) as u32 * 1000);
                        (*g, score)
                    })
                    .collect();
                self.native_path(program, request.caller, &weighted, None, false, cross_room)?
            } else {
                preferred.or_else(ranked).or_else(|| find_path(&goals))
            };
            let Some((goal, path)) = found else {
                if self.architecture.is_some() {
                    return self.begin_route_retry(program, id, request, 3, events);
                }
                return self.fail_route(program, id, request, 3, "no collision-free path", events);
            };
            if let Some((_, seat)) = seats
                .iter()
                .find(|(candidate, seat)| *candidate == goal && *seat != 0)
            {
                let seat = *seat;
                self.person(request.caller, 37, goal.entry_flags)?;
                if self.vm.world.objects[&request.caller]
                    .container
                    .is_some_and(|(parent, _)| parent == seat)
                {
                    self.person(request.caller, 62, 0)?;
                    self.complete_route(program, id, request, true)?;
                    return Ok(true);
                }
                let d = self.vm.world.objects[&seat].definition.as_ref().unwrap();
                let owner = d.source.clone();
                let entry = program.modules[&owner].object_functions[&d.object][26].action;
                self.vm
                    .push_portal_service(program, id, owner, entry, seat)?;
                self.event(
                    events,
                    id,
                    format!(
                        "route invokes original Sit on #{seat}; entry flags {}",
                        goal.entry_flags
                    ),
                );
                return Ok(true);
            }
            let initial_facing = caller.facing;
            self.event(
                events,
                id,
                format!(
                    "route planned: {} segments to ({}, {}), entry flags {:#04x}",
                    path.len() - 1,
                    goal.position.x,
                    goal.position.y,
                    goal.entry_flags
                ),
            );
            if self.architecture.is_none() {
                self.reservations.insert(request.target, id);
            }
            self.routes.insert(
                id,
                Route {
                    request: request.id,
                    caller: request.caller,
                    target: request.target,
                    target_body: target,
                    goal,
                    locomotion: locomotion::Locomotion::plan(
                        program,
                        &path,
                        initial_facing,
                        goal.facing,
                    )?,
                    path,
                    original_path: None,
                    next: 1,
                    started_tick: self.vm.tick,
                    last_tick: self.vm.tick,
                },
            );
        }
        let route = self.routes[&id].clone();
        let carried_target = request.target == request.caller
            || self.vm.world.objects[&request.target]
                .container
                .is_some_and(|(parent, _)| parent == request.caller);
        if !carried_target && self.body(request.target)? != &route.target_body {
            return self.fail_route(program, id, request, 7, "target moved", events);
        }
        // Validate even while turning: a newly blocked next edge fails without sliding into it.
        if self.architecture.is_none()
            && let Some(&next) = route.path.get(route.next)
            && !geometry::edge_clear(route.path[route.next - 1], next, |p| {
                self.clear(request.caller, p, None)
            })
        {
            return self.obstructed_route(program, id, request, events);
        }
        if route.locomotion.elapsed < route.locomotion.duration() {
            if self.vm.tick == route.last_tick {
                return Ok(false);
            }
            let mut movement = route.locomotion.clone();
            movement.elapsed += 1;
            let next = movement.next();
            // Check every crossed edge and the partially traversed edge, including side clearance.
            for i in route.next..=next.min(route.path.len() - 1) {
                if self.architecture.is_some() {
                    break;
                }
                if !geometry::edge_clear(route.path[i - 1], route.path[i], |p| {
                    self.clear(request.caller, p, None)
                }) {
                    return self.obstructed_route(program, id, request, events);
                }
            }
            let position = movement.anchor(&route.path);
            if self.architecture.is_some()
                && !geometry::segment_clear(self.body(request.caller)?.position, position, |p| {
                    self.clear(request.caller, p, None)
                })
            {
                return self.obstructed_route(program, id, request, events);
            }
            self.face(request.caller, movement.facing())?;
            self.bodies.get_mut(&request.caller).unwrap().position = position;
            self.sync_contained_positions(request.caller)?;
            let active = self.routes.get_mut(&id).unwrap();
            active.next = next;
            active.last_tick = self.vm.tick;
            active.locomotion = movement;
            if next != route.next {
                let p = self.bodies[&request.caller].position;
                self.event(
                    events,
                    id,
                    format!("moved #{} to ({}, {})", request.caller, p.x, p.y),
                );
            }
            if self.routes[&id].locomotion.elapsed < self.routes[&id].locomotion.duration() {
                return Ok(false);
            }
        }
        self.person(request.caller, 37, route.goal.entry_flags)?;
        self.person(request.caller, 62, 0)?;
        self.face(request.caller, route.goal.facing)?;
        self.routes.remove(&id);
        self.complete_route(program, id, request, true)?;
        self.event(
            events,
            id,
            "route completed after actual movement; branch true".into(),
        );
        Ok(true)
    }
    fn service_snap(
        &mut self,
        program: &Program,
        id: ThreadId,
        request: &SnapRequest,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        let r = &request.route;
        let target = self.body(r.target)?.clone();
        let caller = self.body(r.caller)?.clone();
        use sims_vm::routing::Destination as SnapTarget;
        let (goals, snap_target, change_direction) = match &r.destination {
            SnapTarget::Slot { slot, version, .. } => (
                geometry::destinations(slot, *version, &target, &caller)?,
                slot.snap_target,
                slot.flags & 4096 != 0,
            ),
            SnapTarget::FirstSlot | SnapTarget::Front => {
                let front = matches!(r.destination, SnapTarget::Front);
                let (point, direction) = routing::direct_snap(
                    [target.position.y, target.position.x],
                    target.facing.trailing_zeros() as u8,
                    caller.facing.trailing_zeros() as u8,
                    front,
                )?;
                (
                    vec![Destination {
                        position: Point::new(point[1], point[0]),
                        facing: 1 << direction,
                        entry_flags: 0,
                    }],
                    if front { -1 } else { 0 },
                    false,
                )
            }
            SnapTarget::Relative(_) => {
                return Err("relative route descriptor cannot be snapped".into());
            }
        };
        if let Some(result) = self
            .vm
            .world
            .objects
            .get_mut(&r.caller)
            .unwrap()
            .data
            .get_mut(&52)
        {
            *result = 0;
        }
        if self.occupied(r.target, r.caller, id)? {
            return self.snap_failed(
                program,
                id,
                request,
                r.target,
                "target occupied or reserved",
                events,
            );
        }
        if snap_target < 0 {
            let Some(goal) = goals
                .iter()
                .find(|g| self.clear_on_level(r.caller, target.level, g.position, None, false))
            else {
                let blocker = goals.last().and_then(|g| {
                    self.blocking_object(r.caller, target.level, g.position, None, false)
                });
                if let Some(blocker) = blocker
                    && contention::snap_shoo_allowed(
                        self.vm.world.objects[&r.caller].person.is_some(),
                        request.flags,
                        11,
                        self.vm.world.objects[&blocker].person.is_some(),
                    )
                    && self.begin_shoo_factory(
                        program,
                        blocker,
                        contention::CreationOrigin::Snap {
                            thread: id,
                            request: request.clone(),
                        },
                    )?
                {
                    return Ok(());
                }
                return self.snap_failed(
                    program,
                    id,
                    request,
                    blocker.unwrap_or(0),
                    "no clear exit",
                    events,
                );
            };
            if let Some((parent, slot)) = self.vm.world.objects[&r.caller].container {
                self.vm.world.objects.get_mut(&parent).unwrap().slots[slot as usize] = None;
            }
            let actor = self.vm.world.objects.get_mut(&r.caller).unwrap();
            actor.container = None;
            actor.data.insert(2, 0);
            actor.data.insert(3, 0);
            let body = self.bodies.get_mut(&r.caller).unwrap();
            body.position = goal.position;
            body.level = target.level;
            body.elevation = 0;
            self.sync_contained_positions(r.caller)?;
            if change_direction {
                self.face(r.caller, goal.facing)?;
            }
            self.reservations.insert(r.target, id);
            self.vm.finish_snap(program, id, r.id, true)?;
            self.event(
                events,
                id,
                format!(
                    "snapped #{} to exit ({}, {}) from #{}",
                    r.caller, goal.position.x, goal.position.y, r.target
                ),
            );
            return Ok(());
        }
        let slot = usize::try_from(snap_target).map_err(|_| "invalid containment index")?;
        if self.vm.world.objects[&r.caller]
            .container
            .is_some_and(|c| c != (r.target, slot as u16))
        {
            return Err("moving between containers is unsupported".into());
        }
        let obj = &self.vm.world.objects[&r.target];
        if !matches!(obj.slots.get(slot), Some(None))
            && obj.slots.get(slot) != Some(&Some(r.caller))
        {
            return self.snap_failed(
                program,
                id,
                request,
                r.target,
                "containment slot unavailable",
                events,
            );
        }
        let def = obj
            .definition
            .as_ref()
            .ok_or("containment target has no definition")?;
        let container = program
            .modules
            .get(&def.source)
            .and_then(|m| m.slots.get(&def.slots))
            .and_then(|slots| slots.entries.iter().filter(|s| s.kind == 0).nth(slot))
            .ok_or("missing containment SLOT record")?;
        let position = geometry::offset(container, &target)?;
        let elevation = match container.height {
            0 | 1 | 5 => 0,
            7 => 7,
            _ => return Err("unsupported containment height".into()),
        };
        // Native contained placement validates its slot instead of testing the
        // floor footprint against every object (0x4329c1). The optional Snap
        // footprint check (0x4550a0) tests architecture, not neighboring furniture.
        let bounds = self.collision_bounds(r.caller, target.position);
        if !self.room.contains(bounds)
            || target.level == 1 && self.room.obstacles.iter().any(|o| o.overlaps(bounds))
            || self.architecture.as_ref().is_some_and(|a| {
                !a.bounds_clear(target.level, bounds, &self.active_portals(r.caller))
            })
        {
            return self.snap_failed(
                program,
                id,
                request,
                0,
                "containment placement blocked",
                events,
            );
        }
        let facing = if change_direction {
            goals.first().ok_or("missing snap goal")?.facing
        } else {
            caller.facing
        };
        self.face(r.caller, facing)?;
        self.vm.world.objects.get_mut(&r.target).unwrap().slots[slot] = Some(r.caller);
        let actor = self.vm.world.objects.get_mut(&r.caller).unwrap();
        actor.container = Some((r.target, slot as u16));
        actor.data.insert(2, r.target);
        actor.data.insert(3, slot as i16);
        let body = self.bodies.get_mut(&r.caller).unwrap();
        body.position = position;
        body.level = target.level;
        body.elevation = elevation;
        self.sync_contained_positions(r.caller)?;
        self.reservations.insert(r.target, id);
        self.vm.finish_snap(program, id, r.id, true)?;
        self.event(
            events,
            id,
            format!(
                "snapped #{} into #{} slot {}; position ({}, {}), height {}",
                r.caller, r.target, slot, position.x, position.y, elevation
            ),
        );
        Ok(())
    }
    fn snap_failed(
        &mut self,
        program: &Program,
        id: ThreadId,
        request: &SnapRequest,
        blocker: ObjectId,
        reason: &str,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        let o = self
            .vm
            .world
            .objects
            .get_mut(&request.route.caller)
            .ok_or("missing snap caller")?;
        *o.data
            .get_mut(&54)
            .ok_or("primitive result ID field is not declared")? = blocker;
        self.release(id);
        self.vm.finish_snap(program, id, request.route.id, false)?;
        self.event(events, id, format!("snap failed: {reason}; branch false"));
        Ok(())
    }
    pub(crate) fn validate_geometry(&self) -> Result<(), String> {
        self.room.validate()?;
        let pending: Vec<_> = self
            .vm
            .world
            .objects
            .keys()
            .copied()
            .filter(|&id| self.cursor_constructor_pending(id))
            .collect();
        for &id in &pending {
            self.validate_cursor_constructor(id)?;
        }
        if self.bodies.len() + self.unplaced.len() + pending.len() != self.vm.world.objects.len()
            || self
                .unplaced
                .iter()
                .any(|id| !self.vm.world.objects.contains_key(id) || self.bodies.contains_key(id))
        {
            return Err("every room object needs a body or explicit off-lot placement".into());
        }
        for (&id, b) in &self.bodies {
            b.validate(&self.room)?;
            if self
                .architecture
                .as_ref()
                .map_or(b.level != 1, |a| a.floor(b.level).is_none())
            {
                return Err("body on unavailable storey".into());
            }
            let object = self
                .vm
                .world
                .objects
                .get(&id)
                .ok_or("body without object")?;
            if object
                .data
                .get(&1)
                .is_none_or(|d| (*d as u16 & 7) != b.facing.trailing_zeros() as u16)
            {
                return Err("body and object direction disagree".into());
            }
            // Authored groups are placed atomically with a common facing.
            // Native SetObject moves members in order and yields to callbacks;
            // Snap can then change only the selected member's raw facing.
            // validate_native_lot checks each authoritative position/cache pair.
            if self.native_lot.is_none()
                && let Some(part) = &object.multitile
            {
                let head = self.body(part.head.unwrap_or(id))?;
                let [native_x, native_y] = sims_vm::multitile::world_offset(part.offset);
                if object.container.is_some()
                    || b.facing != head.facing
                    || b.elevation != head.elevation
                    || Some(b.level) != head.level.checked_add(part.offset[2])
                    || Some(b.position.x) != head.position.x.checked_add(native_y)
                    || Some(b.position.y) != head.position.y.checked_add(native_x)
                {
                    return Err("inconsistent authored multi-tile placement".into());
                }
            }
        }
        // Validate every body before collision queries touch any other body's fields.
        for (&id, b) in &self.bodies {
            let object = &self.vm.world.objects[&id];
            // A current placement query is not an invariant of an existing lot:
            // scripts create floor overlays and change collision flags in place.
            // Validate static geometry here; movement/placement owns dynamic checks.
            let clear = if let Some(a) = &self.architecture {
                let bounds = self.collision_bounds(id, b.position);
                let mut allowed = self.active_portals(id);
                allowed.push(id);
                self.room.contains(bounds)
                    && (object.data.get(&8).is_some_and(|f| f & 16 != 0)
                        || ((b.level != 1
                            || !self.room.obstacles.iter().any(|o| o.overlaps(bounds)))
                            && a.walls_clear(b.level, bounds, &allowed)))
            } else {
                self.clear(id, b.position, None)
            };
            if object.container.is_none() && !clear {
                return Err(format!(
                    "overlapping body or obstacle: #{id} at {:?}",
                    b.position
                ));
            }
            if let Some((parent, _)) = object.container {
                let mut ancestor = Some(parent);
                let mut seen = std::collections::BTreeSet::from([id]);
                while let Some(current) = ancestor {
                    if !seen.insert(current) {
                        return Err("cyclic room containment".into());
                    }
                    ancestor = self
                        .vm
                        .world
                        .objects
                        .get(&current)
                        .ok_or("missing container")?
                        .container
                        .map(|v| v.0);
                }
                self.body(parent)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod script_direction_tests;
