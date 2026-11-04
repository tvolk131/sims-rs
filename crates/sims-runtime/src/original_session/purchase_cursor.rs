//!421986: create furniture, wrap it in4215e6's cursor, delete on failure,
//!then mark the returned cursor chain as a new purchase. No funds are charged.
//!The Move tool's later rotation, history capture and drop remain its caller.
use super::Session;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_vm::{Definition, ObjectId, construction::ObjectDate};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Furniture,
    BeginCursor,
    Cursor,
    Remove,
    Mark,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    move_owner: bool,
    tick: u64,
    definition: Option<Definition>,
    cursor_definition: Option<Definition>,
    level: i32,
    cheat: bool,
    furniture: Option<(ObjectId, u32)>,
    result: Option<ObjectId>,
    phase: Phase,
}
impl State {
    pub(super) fn move_owned(&self) -> bool {
        self.move_owner
    }
    pub(super) fn matches_move(&self, definition: Option<&Definition>, level: i32) -> bool {
        self.move_owner && self.definition.as_ref() == definition && self.level == level
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub fn furniture(&self) -> Option<ObjectId> {
        self.furniture.map(|(id, _)| id)
    }
    pub fn result(&self) -> Option<ObjectId> {
        self.result
    }
    pub(super) fn validate(&self, s: &Session) -> Result<()> {
        ensure!(
            self.tick == s.simulation.vm.tick,
            "purchase cursor crossed a lot tick"
        );
        let sim = &s.simulation;
        ensure!(
            (self.phase == Phase::Furniture)
                == sim
                    .native_selection
                    .as_ref()
                    .is_some_and(|e| e.direct_factory().is_some()
                        && e.direct_factory() == self.definition.as_ref()),
            "purchase furniture factory lost its owner"
        );
        ensure!(
            (self.phase == Phase::Cursor) == sim.cursor_factory.is_some(),
            "purchase cursor factory lost its owner"
        );
        if let Some(factory) = &sim.cursor_factory {
            ensure!(
                factory.matches_creation(
                    self.cursor_definition.as_ref(),
                    self.furniture().unwrap_or(0),
                    self.level
                ),
                "purchase cursor arguments changed"
            );
        }
        ensure!(
            (self.phase == Phase::Remove) == sim.synchronous_removal.is_some(),
            "purchase cursor cleanup lost its owner"
        );
        if let Some(removal) = &sim.synchronous_removal {
            ensure!(
                Some(removal.target()) == self.furniture(),
                "purchase cleanup target differs"
            );
        }
        if !matches!(self.phase, Phase::Remove | Phase::Complete) {
            if let Some((id, handle)) = self.furniture {
                let source = self
                    .definition
                    .as_ref()
                    .context("purchase furniture definition absent")?;
                let module = s
                    .program
                    .modules
                    .get(&source.source)
                    .context("purchase module absent")?;
                let words = sims_vm::construction::definition_words(&s.program, source)
                    .map_err(anyhow::Error::msg)?;
                //440b2d returns the first constructed part, not the multipart
                //catalog master. Check that exact resource-derived identity.
                let expected = if words[10] != 0 {
                    sims_vm::multitile::construction_plan(module, source.object)
                        .map_err(anyhow::Error::msg)?[0]
                } else {
                    source.object
                };
                ensure!(
                    sim.vm
                        .world
                        .scheduler_storage
                        .as_ref()
                        .and_then(|r| r.handle(id))
                        == Some(handle)
                        && sim
                            .vm
                            .world
                            .objects
                            .get(&id)
                            .and_then(|o| o.definition.as_ref())
                            .is_some_and(|d| d.source == source.source && d.object == expected),
                    "purchase furniture lifetime changed"
                );
            }
        }
        ensure!(
            self.result.is_none() || matches!(self.phase, Phase::Mark | Phase::Complete),
            "purchase cursor returned before its child"
        );
        if let Some(id) = self.result {
            ensure!(
                sim.vm
                    .world
                    .objects
                    .get(&id)
                    .is_some_and(|o| o.cursor.is_some()),
                "returned purchase cursor absent"
            );
        }
        Ok(())
    }
}
impl Session {
    pub fn purchase_cursor_pending(&self) -> bool {
        self.host.presentation.purchase_cursor.is_some()
    }
    ///The arguments are actual registered resources. None preserves the
    ///original null-definition branch, including deletion after cursor failure.
    pub fn begin_purchase_cursor(
        &mut self,
        definition: Option<Definition>,
        cursor_definition: Option<Definition>,
        level: i32,
        cheat: bool,
    ) -> Result<()> {
        self.transport_idle()?;
        self.begin_purchase_cursor_inner(definition, cursor_definition, level, cheat, false)
    }
    pub(super) fn begin_move_purchase_cursor(
        &mut self,
        definition: Option<Definition>,
        cursor_definition: Option<Definition>,
        level: i32,
        cheat: bool,
    ) -> Result<()> {
        ensure!(
            !self.purchase_cursor_pending()
                && !self.simulation.selection_work_pending()
                && !self.simulation.cursor_work_pending()
                && !self.house_notification_pending()
                && self.simulation.synchronous_removal.is_none(),
            "Move purchase overlaps another child"
        );
        self.begin_purchase_cursor_inner(definition, cursor_definition, level, cheat, true)
    }
    fn begin_purchase_cursor_inner(
        &mut self,
        definition: Option<Definition>,
        cursor_definition: Option<Definition>,
        level: i32,
        cheat: bool,
        move_owner: bool,
    ) -> Result<()> {
        ensure!(
            self.host.presentation.notifications.is_some(),
            "purchase needs the live notification registry"
        );
        for d in definition.iter().chain(cursor_definition.iter()) {
            sims_content::assets::load_object_guids(&self.index, &mut self.program, &[d.guid])?;
            sims_vm::construction::definition_words(&self.program, d)
                .map_err(anyhow::Error::msg)?;
        }
        self.simulation
            .architecture
            .as_mut()
            .context("purchase architecture absent")?
            .bind_footprints(&self.program)
            .map_err(anyhow::Error::msg)?;
        let phase = if let Some(d) = &definition {
            self.simulation
                .begin_native_object_factory(&self.program, d.clone())
                .map_err(anyhow::Error::msg)?;
            Phase::Furniture
        } else {
            Phase::BeginCursor
        };
        self.host.presentation.purchase_cursor = Some(State {
            move_owner,
            tick: self.simulation.vm.tick,
            definition,
            cursor_definition,
            level,
            cheat,
            furniture: None,
            result: None,
            phase,
        });
        Ok(())
    }
    ///Notification/view children are serviced by the existing application
    ///driver between calls. A diagnostic budget never advances the lot clock.
    pub fn advance_purchase_cursor(&mut self, budget: usize) -> Result<bool> {
        ensure!(
            budget > 0 && !self.house_notification_pending() && !self.view_refresh_pending(),
            "purchase cursor awaits its application child or a work budget"
        );
        self.host
            .presentation
            .purchase_cursor
            .as_ref()
            .context("purchase cursor absent")?
            .validate(self)?;
        let mut state = self.host.presentation.purchase_cursor.take().unwrap();
        let result = self.advance_purchase_cursor_inner(&mut state, budget);
        self.host.presentation.purchase_cursor = Some(state);
        result?;
        let state = self.host.presentation.purchase_cursor.as_ref().unwrap();
        state.validate(self)?;
        Ok(state.complete())
    }
    fn advance_purchase_cursor_inner(&mut self, state: &mut State, budget: usize) -> Result<()> {
        match state.phase {
            Phase::Furniture => {
                let ordinary = self.host.ordinary.graphics.keys().copied().collect();
                let (_, result) = self
                    .simulation
                    .advance_native_object_factory(&self.program, &ordinary, budget)
                    .map_err(anyhow::Error::msg)?;
                if let Some(result) = result {
                    state.furniture = result
                        .map(|id| {
                            self.simulation
                                .vm
                                .world
                                .scheduler_storage
                                .as_ref()
                                .and_then(|r| r.handle(id))
                                .map(|h| (id, h))
                                .context("created purchase furniture lifetime absent")
                        })
                        .transpose()?;
                    state.phase = Phase::BeginCursor;
                } else if let Some((
                    sims_sim::lot_selection::Request::CreationNotification(id),
                    false,
                )) = self.simulation.pending_native_selection()
                {
                    self.begin_factory_broadcast(id)?;
                } else {
                    self.service_selection_assets(self.retained_camera_input()?, budget)?;
                }
            }
            Phase::BeginCursor => {
                let clock = &self
                    .simulation
                    .household
                    .as_ref()
                    .context("purchase calendar absent")?
                    .clock;
                let date = ObjectDate {
                    year: clock.year,
                    month: clock.month,
                    day: clock.day,
                };
                if self
                    .simulation
                    .begin_cursor_factory(
                        &self.program,
                        state.cursor_definition.clone(),
                        state.furniture().unwrap_or(0),
                        state.level,
                        state.cheat,
                        date,
                    )
                    .map_err(anyhow::Error::msg)?
                {
                    state.phase = Phase::Cursor;
                } else if let Some(id) = state.furniture() {
                    self.simulation
                        .begin_synchronous_removal(&self.program, id)
                        .map_err(anyhow::Error::msg)?;
                    state.phase = Phase::Remove;
                } else {
                    //44123f resolves its signed ID; ID0 has no registry object.
                    state.phase = Phase::Complete;
                }
            }
            Phase::Cursor => {
                let factory = self
                    .simulation
                    .cursor_factory
                    .as_ref()
                    .context("purchase cursor child absent")?;
                if factory.pending_focus().is_some()
                    || factory.pending_created().is_some()
                    || factory.returned().is_some()
                {
                    if let Some(id) =
                        self.service_cursor_factory_return(self.retained_camera_input()?, budget)?
                    {
                        state.result = Some(id);
                        state.phase = Phase::Mark;
                    }
                } else {
                    if factory.placement().is_some() {
                        self.simulation
                            .begin_cursor_factory_placement(&self.program, state.cheat)
                            .map_err(anyhow::Error::msg)?;
                    }
                    self.simulation
                        .advance_cursor_factory(&self.program, budget)
                        .map_err(anyhow::Error::msg)?;
                    self.move_tool_assets()?;
                }
            }
            Phase::Remove => {
                let mut step = sims_sim::Step {
                    stop: sims_sim::StepStop::Budget,
                    trace: vec![],
                    events: vec![],
                };
                let done = self
                    .simulation
                    .drive_synchronous_removal(&self.program, &mut { budget }, &mut step)
                    .map_err(anyhow::Error::msg)?;
                self.move_tool_assets()?;
                if done {
                    ensure!(
                        Some(
                            self.simulation
                                .take_synchronous_removal()
                                .map_err(anyhow::Error::msg)?
                        ) == state.furniture(),
                        "purchase cleanup returned another object"
                    );
                    state.phase = Phase::Complete;
                }
            }
            Phase::Mark => {
                sims_vm::cursor::mark_purchase(
                    &mut self.simulation.vm.world,
                    state.result.context("purchase result absent")?,
                )
                .map_err(anyhow::Error::msg)?;
                state.phase = Phase::Complete;
            }
            Phase::Complete => (),
        }
        Ok(())
    }
    ///Transfer the returned cursor to its Move-tool caller. This neither
    ///places/releases furniture nor fabricates the caller's undo command.
    pub fn finish_purchase_cursor(&mut self) -> Result<Option<ObjectId>> {
        let state = self
            .host
            .presentation
            .purchase_cursor
            .as_ref()
            .context("purchase cursor absent")?;
        state.validate(self)?;
        ensure!(
            state.complete() && !self.house_notification_pending(),
            "purchase cursor has not returned"
        );
        Ok(self
            .host
            .presentation
            .purchase_cursor
            .take()
            .unwrap()
            .result)
    }
}
