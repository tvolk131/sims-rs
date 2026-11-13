//! Replacement popup ownership for 48cbec -> 48d56c. Eligibility has returned;
//! the popup retains its records while the ordinary clock continues. It saves
//! and restores simulator+d4 and application stopping006e54a8 around its return.
use super::Session;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_vm::ObjectId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Ground,
    Object,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    Choose(usize),
    Cancel,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Queued { id: u32, accepted: bool },
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    kind: Kind,
    tick: u64,
    actor: (ObjectId, u32),
    target: (ObjectId, u32),
    clicked: Option<(ObjectId, u32)>,
    saved_poll_messages: u8,
    saved_stopping: u8,
    command: Option<Command>,
}
impl State {
    pub fn command(&self) -> Option<Command> {
        self.command
    }
    pub(super) fn owns(&self, kind: Kind, tick: u64, now: u64) -> bool {
        self.kind == kind && self.tick == tick && tick <= now
    }
    pub(super) fn validate(&self, s: &Session) -> Result<()> {
        ensure!(
            match self.kind {
                Kind::Ground => s.host.presentation.object_input.is_none(),
                Kind::Object => s.host.presentation.ground_input.is_none(),
            },
            "live menu has competing input owners"
        );
        let clock = s
            .host
            .presentation
            .clock
            .as_ref()
            .context("live menu clock absent")?;
        ensure!(
            clock.fields.poll_messages == 1,
            "live menu lost its message-poll scope"
        );
        ensure!(
            self.tick <= s.simulation.vm.tick,
            "live menu clock moved backwards"
        );
        let owner = match self.kind {
            Kind::Ground => s
                .host
                .presentation
                .ground_input
                .as_ref()
                .filter(|o| o.ready())
                .map(|o| (o.actor(), o.target().unwrap(), o.actions().len())),
            Kind::Object => s
                .host
                .presentation
                .object_input
                .as_ref()
                .filter(|o| o.ready())
                .map(|o| (o.actor(), o.target(), o.actions().len())),
        }
        .context("live menu lost its published input owner")?;
        ensure!(
            owner.0 == self.actor.0 && owner.1 == self.target.0,
            "live menu caller changed"
        );
        ensure!(
            match self.command {
                Some(Command::Choose(i)) => i < owner.2,
                _ => true,
            },
            "live menu command index absent"
        );
        // A reused script ID must never impersonate the pointer retained by
        // the original popup. Lifetime-ending callbacks remain an explicit
        // boundary until their original popup notification is established.
        for pair in [Some(self.actor), Some(self.target), self.clicked]
            .into_iter()
            .flatten()
        {
            ensure!(
                s.menu_identity(pair.0)? == pair,
                "live menu object lifetime ended"
            );
        }
        Ok(())
    }
}
impl Session {
    fn menu_identity(&self, id: ObjectId) -> Result<(ObjectId, u32)> {
        let handle = self
            .simulation
            .vm
            .world
            .scheduler_storage
            .as_ref()
            .and_then(|r| r.handle(id))
            .context("live menu object lifetime ended")?;
        Ok((id, handle))
    }
    pub fn running_menu(&self) -> bool {
        self.host.presentation.live_menu.is_some()
    }
    pub fn menu_command_pending(&self) -> bool {
        self.host
            .presentation
            .live_menu
            .as_ref()
            .is_some_and(|m| m.command.is_some())
    }
    pub(super) fn menu_owns_tick(&self, kind: Kind, tick: u64) -> bool {
        self.host
            .presentation
            .live_menu
            .as_ref()
            .is_some_and(|m| m.owns(kind, tick, self.simulation.vm.tick))
    }
    /// Explicit publication by a client displaying the ready menu. Historical
    /// query checkpoints retain their old owner until a client opens the popup.
    pub fn begin_running_menu(&mut self) -> Result<()> {
        ensure!(
            !self.running_menu() && !self.clock_pending(),
            "live menu overlaps an elapsed caller"
        );
        let (kind, tick, actor, target, clicked) =
            if let Some(o) = &self.host.presentation.ground_input {
                o.validate(self)?;
                ensure!(o.ready(), "ground menu is not published");
                (
                    Kind::Ground,
                    o.opened_tick(),
                    o.actor(),
                    o.target().unwrap(),
                    None,
                )
            } else {
                let o = self
                    .host
                    .presentation
                    .object_input
                    .as_ref()
                    .context("published menu absent")?;
                o.validate(self)?;
                ensure!(o.ready(), "object menu is not published");
                (
                    Kind::Object,
                    o.opened_tick(),
                    o.actor(),
                    o.target(),
                    Some(o.clicked()),
                )
            };
        let actor = self.menu_identity(actor)?;
        let target = self.menu_identity(target)?;
        let clicked = clicked.map(|id| self.menu_identity(id)).transpose()?;
        let clock = self
            .host
            .presentation
            .clock
            .as_mut()
            .context("live menu requires a bound clock")?;
        let state = State {
            kind,
            tick,
            actor,
            target,
            clicked,
            saved_poll_messages: clock.fields.poll_messages,
            saved_stopping: clock.stopping,
            command: None,
        };
        clock.fields.poll_messages = 1;
        self.host.presentation.live_menu = Some(state);
        Ok(())
    }
    pub fn submit_menu_command(&mut self, command: Command) -> Result<()> {
        let state = self
            .host
            .presentation
            .live_menu
            .as_ref()
            .context("live menu absent")?;
        state.validate(self)?;
        ensure!(state.command.is_none(), "live menu already has an input");
        if let Command::Choose(i) = command {
            let len = match state.kind {
                Kind::Ground => self
                    .host
                    .presentation
                    .ground_input
                    .as_ref()
                    .unwrap()
                    .actions()
                    .len(),
                Kind::Object => self
                    .host
                    .presentation
                    .object_input
                    .as_ref()
                    .unwrap()
                    .actions()
                    .len(),
            };
            ensure!(i < len, "live menu command index absent");
        }
        self.host.presentation.live_menu.as_mut().unwrap().command = Some(command);
        Ok(())
    }
    pub fn dispatch_menu_command(&mut self) -> Result<Option<Outcome>> {
        let Some(state) = &self.host.presentation.live_menu else {
            return Ok(None);
        };
        state.validate(self)?;
        // An input notification can arrive while a saved frame owns a nested
        // VM service. It remains queued through that return and PeekMessage.
        if self.clock_idle().is_err() || self.transport_pending() {
            return Ok(None);
        }
        let Some(command) = state.command else {
            return Ok(None);
        };
        let kind = state.kind;
        Ok(Some(match (kind, command) {
            (Kind::Ground, Command::Choose(i)) => {
                let (id, accepted) = self.choose_ground_action(i)?;
                Outcome::Queued { id, accepted }
            }
            (Kind::Object, Command::Choose(i)) => {
                let (id, accepted) = self.choose_object_action(i)?;
                Outcome::Queued { id, accepted }
            }
            (Kind::Ground, Command::Cancel) => {
                self.cancel_ground_input()?;
                Outcome::Cancelled
            }
            (Kind::Object, Command::Cancel) => {
                self.cancel_object_input()?;
                Outcome::Cancelled
            }
        }))
    }
    pub(super) fn close_running_menu(&mut self) -> Result<()> {
        if let Some(state) = &self.host.presentation.live_menu {
            ensure!(
                !self.clock_pending(),
                "elapsed caller still owns the menu return"
            );
            state.validate(self)?;
            let clock = self.host.presentation.clock.as_mut().unwrap();
            clock.fields.poll_messages = state.saved_poll_messages;
            clock.stopping = state.saved_stopping;
            // Only the returned popup may rebase its following synchronous
            // cleanup. Its original query is never replayed at the new tick.
            if let Some(o) = &mut self.host.presentation.ground_input {
                o.retime(self.simulation.vm.tick);
            }
            if let Some(o) = &mut self.host.presentation.object_input {
                o.retime(self.simulation.vm.tick);
            }
            self.host.presentation.live_menu = None;
        }
        Ok(())
    }
}
