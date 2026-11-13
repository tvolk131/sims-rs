//! Explicit person-draw resource preparation. Sampling a published frame never
//! enters this caller. The saved cursor can stop between synchronous native
//! 590ca9..590cbd calls without replaying a completed person's bindings.
use super::Session;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_vm::ObjectId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    tick: u64,
    objects: Vec<ObjectId>,
    next: usize,
}
impl State {
    pub fn validate(&self, session: &Session) -> Result<()> {
        ensure!(
            self.tick == session.simulation.vm.tick,
            "person draw preparation crossed a tick"
        );
        ensure!(
            self.next <= self.objects.len(),
            "person draw cursor outside its list"
        );
        let unique: std::collections::BTreeSet<_> = self.objects.iter().copied().collect();
        ensure!(
            unique.len() == self.objects.len(),
            "person draw contains duplicate owners"
        );
        ensure!(
            self.objects
                .iter()
                .all(|id| session.simulation.characters.contains_key(id)
                    && session.host.characters.contains_key(id)),
            "person draw lost a bound character"
        );
        Ok(())
    }
}
impl Session {
    pub fn person_draw_preparation_pending(&self) -> bool {
        self.host.presentation.person_draw.is_some()
    }
    /// Existing appearance/animator callers must return first. In particular a
    /// changed outfit word does not authorize replacing their partial bindings.
    pub fn person_draw_preparation_ready(&self) -> bool {
        !self.host.presentation.awaiting_house_open
            && !self.host.ordinary.updates_pending()
            && !self.transport_pending()
            && !self.tool_selection_pending()
            && !self.menu_command_pending()
            && !self.clock_pending()
            && !self.camera_command_pending()
            && !self.ui_wait_pending()
            && !self.interface_update_pending()
            && !self.control_panel_pending()
            && !self.ui_portrait_pending()
            && !self.house_notification_pending()
            && !self.view_refresh_pending()
            && !self.load_completion_pending()
            && !self.postload_pending()
            && !self.publication_pending()
            && !self.simulation.selection_work_pending()
            && !self.player_input_blocks_ticks()
            && self.simulation.vm.scheduled_object_visit.is_none()
            && !self
                .simulation
                .vm
                .threads
                .values()
                .any(|thread| matches!(thread.status, sims_vm::Status::AwaitingUiEffect(_)))
            && self.simulation.characters.values().all(|r| {
                !r.character.busy()
                    && r.ordinary_update.is_none()
                    && r.person_update.is_none()
                    && r.skill_profile.is_none()
            })
    }
    /// Keep the presenter's retained source order and membership. This does
    /// not classify a new object or substitute script scheduler registry order.
    pub fn person_draw_objects(&self) -> Result<Vec<ObjectId>> {
        let Some(presenter) = &self.host.presentation.presenter else {
            return Ok(self.simulation.characters.keys().copied().collect());
        };
        let members: std::collections::BTreeSet<_> = presenter
            .lists
            .ordinary
            .iter()
            .chain(&presenter.lists.special)
            .chain(&presenter.lists.dynamic)
            .filter(|id| {
                self.simulation.characters.contains_key(id) && self.host.characters.contains_key(id)
            })
            .copied()
            .collect();
        let order = match (&self.simulation.vm.world.registry_order, &presenter.order) {
            (Some(cache), None) => &cache.objects,
            (None, Some(cache)) => &cache.objects,
            _ => anyhow::bail!("person draw has ambiguous spatial-cache ownership"),
        };
        let objects: Vec<_> = order
            .iter()
            .filter(|id| members.contains(id))
            .copied()
            .collect();
        ensure!(
            objects
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                == members,
            "classified person lost its retained draw order"
        );
        Ok(objects)
    }
    /// The caller supplies its actual draw-owner order, independently of the
    /// simulation scheduler. No person word or location manufactures an outfit.
    pub fn begin_person_draw_preparation(&mut self, objects: Vec<ObjectId>) -> Result<()> {
        ensure!(
            !self.person_draw_preparation_pending(),
            "person draw preparation already pending"
        );
        ensure!(
            self.person_draw_preparation_ready(),
            "another caller owns person preparation"
        );
        let state = State {
            tick: self.simulation.vm.tick,
            objects,
            next: 0,
        };
        state.validate(self)?;
        self.host.presentation.person_draw = Some(state);
        Ok(())
    }
    pub fn advance_person_draw_preparation(&mut self, budget: usize) -> Result<bool> {
        ensure!(budget > 0, "person draw preparation needs a budget");
        self.host
            .presentation
            .person_draw
            .as_ref()
            .context("person draw preparation absent")?
            .validate(self)?;
        ensure!(
            self.person_draw_preparation_ready(),
            "another caller owns person preparation"
        );
        for _ in 0..budget {
            let state = self.host.presentation.person_draw.as_ref().unwrap();
            if state.next == state.objects.len() {
                self.host.presentation.person_draw = None;
                return Ok(true);
            }
            let id = state.objects[state.next];
            self.host
                .prepare_draw_appearance(&mut self.simulation, &self.program, id)?;
            self.host.presentation.person_draw.as_mut().unwrap().next += 1;
        }
        Ok(false)
    }
}
