//! Saved ground-command core from residential48cbec. Viewport hit testing,
//! tile-menu fallback and popup layout remain caller work. Picker flags are
//! published by the live query, and older saves retain their absent picker owner.
//! The native factory, physical placement,
//! live menu checks and failed/cancelled44123f cleanup each retain their owner.
use super::Session;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::{lot_ground::DESTINATION_GUID, lot_selection, pose::portrait::view::Input};
use sims_vm::{Definition, ObjectId, action::Action};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Factory,
    Placement,
    Query,
    ///48d392 must enumerate other eligible objects on this tile before the
    /// caller can conclude that the menu is empty. Retain that boundary.
    MenuFallback,
    Ready,
    Removal,
    Complete,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct State {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    picker_owned: bool,
    tick: u64,
    actor: ObjectId,
    point: [i32; 2],
    floor: i32,
    definition: Definition,
    target: Option<ObjectId>,
    phase: Phase,
    actions: Vec<Action>,
}
impl State {
    pub(super) fn opened_tick(&self) -> u64 {
        self.tick
    }
    pub(super) fn retime(&mut self, tick: u64) {
        self.tick = tick;
    }
    pub fn ready(&self) -> bool {
        self.phase == Phase::Ready
    }
    pub fn can_cancel(&self) -> bool {
        self.ready() || self.phase == Phase::MenuFallback
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub fn target(&self) -> Option<ObjectId> {
        self.target
    }
    /// The ground caller owns the shared destructor only during cancellation
    /// or rejected placement. A menu alone does not authorize a removal.
    pub fn removal_target(&self) -> Option<ObjectId> {
        (self.phase == Phase::Removal)
            .then_some(self.target)
            .flatten()
    }
    pub fn actor(&self) -> ObjectId {
        self.actor
    }
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }
    pub(super) fn validate(&self, s: &Session) -> Result<()> {
        ensure!(
            (self.tick == s.simulation.vm.tick
                || (self.ready() && s.menu_owns_tick(super::live_menu::Kind::Ground, self.tick)))
                && s.house.number != 99
                && self.definition.guid == DESTINATION_GUID
                && (1..=2).contains(&self.floor)
                && s.simulation
                    .native_selected_person()
                    .map_err(anyhow::Error::msg)?
                    == Some(self.actor),
            "ground input lost its original caller"
        );
        if !matches!(
            self.phase,
            Phase::Factory | Phase::Removal | Phase::Complete
        ) {
            ensure!(
                self.target.is_some_and(|id| s
                    .simulation
                    .vm
                    .world
                    .objects
                    .get(&id)
                    .is_some_and(|o| o.definition.as_ref() == Some(&self.definition))),
                "ground destination identity differs"
            );
        }
        ensure!(
            (self.phase == Phase::Factory)
                == s.simulation
                    .native_selection
                    .as_ref()
                    .is_some_and(|e| e.direct_factory() == Some(&self.definition)),
            "ground factory ownership differs"
        );
        ensure!(
            (self.phase == Phase::Placement) == s.simulation.ground_placement.is_some(),
            "ground physical continuation differs"
        );
        if let Some(e) = &s.simulation.ground_placement {
            ensure!(
                Some(e.object()) == self.target,
                "ground placement object differs"
            );
        }
        ensure!(
            (self.phase == Phase::Query) == s.simulation.vm.player_menu.is_some(),
            "ground menu ownership differs"
        );
        if let Some(q) = &s.simulation.vm.player_menu {
            ensure!(
                q.actor() == self.actor && Some(q.target()) == self.target,
                "ground query caller differs"
            );
        }
        ensure!(
            (self.phase == Phase::Removal) == s.simulation.synchronous_removal.is_some(),
            "ground cleanup ownership differs"
        );
        if let Some(e) = &s.simulation.synchronous_removal {
            ensure!(
                Some(e.target()) == self.target,
                "ground cleanup object differs"
            );
        }
        ensure!(
            self.phase != Phase::Factory || self.target.is_none(),
            "premature factory result"
        );
        ensure!(
            self.actions.is_empty() || self.phase == Phase::Ready,
            "unpublished ground menu"
        );
        if self.picker_owned && self.phase == Phase::Ready {
            ensure!(
                s.host.presentation.picker_target == self.target,
                "ground menu lost picker target"
            );
        }
        if self.phase == Phase::Ready {
            ensure!(!self.actions.is_empty(), "empty ground menu must clean up");
            let entries = s
                .simulation
                .vm
                .action_menu(&s.program, self.target.unwrap())
                .map_err(anyhow::Error::msg)?;
            ensure!(
                self.actions.iter().all(|a| Some(a.target) == self.target
                    && a.icon_owner == a.target
                    && a.id == 0
                    && a.priority == 50
                    && a.args == [0; 4]
                    && a.label.is_none()
                    && a.flags & 28 == 12
                    && a.flags & !13 == 0
                    && entries
                        .iter()
                        .any(|e| e.id == a.menu_id && e.action == a.routine)),
                "ground menu record differs"
            );
        }
        Ok(())
    }
}
impl Session {
    ///48c834 clears the previous target's low flags in two ordered calls.
    /// A stale cached script ID is looked up afresh, exactly like the native tool.
    pub fn clear_picker_target(&mut self) -> Result<()> {
        ensure!(
            !self.player_input_pending(),
            "player input owns picker flags"
        );
        self.clear_picker_target_fields()
    }
    /// Shared48c834 body for a validated current-tool Reset as well as a new
    /// player click. The caller's publication may be pending, but no live menu
    /// may still own the target or an eligibility traversal.
    pub(super) fn clear_picker_target_fields(&mut self) -> Result<()> {
        ensure!(
            !self.ground_input_pending()
                && !self.object_input_pending()
                && self.simulation.vm.player_menu.is_none(),
            "picker reset overlaps a live menu"
        );
        if let Some(id) = self.host.presentation.picker_target {
            if self.simulation.vm.world.objects.contains_key(&id) {
                for mask in [0x1e, 0x17] {
                    let flags = self.simulation.vm.world.objects[&id]
                        .native_flags
                        .context("picker target flags absent")?;
                    self.simulation
                        .update_native_low_flags(&self.program, id, flags & mask)
                        .map_err(anyhow::Error::msg)?;
                }
            }
        }
        self.host.presentation.picker_target = None;
        Ok(())
    }
    pub fn ground_input_pending(&self) -> bool {
        self.host.presentation.ground_input.is_some()
    }
    /// Point is unrotated native physical X/Y, supplied by the viewport's inverse
    /// projection. This entry does not synthesize a mouse or tutorial response.
    /// Returns false at48cc42 when no original person is selected.
    pub fn begin_ground_input(&mut self, point: [i32; 2], floor: i32) -> Result<bool> {
        ensure!(
            !self.player_input_pending()
                && !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending()
                && !self.load_completion_pending()
                && !self.postload_pending()
                && !self.view_refresh_pending()
                && !self.house_notification_pending()
                && !self.host.presentation.awaiting_house_open
                && !self.host.ordinary.updates_pending()
                && self.simulation.vm.world.dialogs.modal.is_none()
                && !self.simulation.vm.constructing_dialog(),
            "ground input overlaps synchronous UI work"
        );
        //The shared48cbec caller resets the picker before selecting its actor;
        //the no-person branch must not construct a Go Here helper or query.
        self.clear_picker_target()?;
        let Some(actor) = self.picker_input_actor()? else {
            return Ok(false);
        };
        ensure!(
            self.house.number != 99,
            "lot99 ground policy requires original48d346"
        );
        ensure!((1..=2).contains(&floor), "ground floor unsupported");
        ensure!(
            self.host.presentation.notifications.is_some(),
            "ground input requires live notification registry"
        );
        sims_content::assets::load_object_guids(
            &self.index,
            &mut self.program,
            &[DESTINATION_GUID],
        )?;
        self.simulation
            .architecture
            .as_mut()
            .context("ground architecture absent")?
            .bind_footprints(&self.program)
            .map_err(anyhow::Error::msg)?;
        let mut definitions = self.program.modules.values().flat_map(|m| {
            m.object_definition_words
                .iter()
                .filter_map(move |(&object, w)| {
                    (w.len() > 20
                        && (u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16))
                            == DESTINATION_GUID)
                        .then(|| Definition {
                            source: m.source.clone(),
                            object,
                            guid: DESTINATION_GUID,
                            slots: w[20] as u16,
                        })
                })
        });
        let definition = definitions.next().context("Go Here resource absent")?;
        ensure!(
            definitions.next().is_none(),
            "Go Here resource is ambiguous"
        );
        self.simulation
            .begin_native_object_factory(&self.program, definition.clone())
            .map_err(anyhow::Error::msg)?;
        self.host.presentation.ground_input = Some(State {
            picker_owned: true,
            tick: self.simulation.vm.tick,
            actor,
            point,
            floor,
            definition,
            target: None,
            phase: Phase::Factory,
            actions: vec![],
        });
        Ok(true)
    }
    /// House notifications are serviced by the shared camera/application driver
    /// between calls. This method never advances a household tick.
    pub fn advance_ground_input(&mut self, camera: Input, budget: usize) -> Result<()> {
        camera.build().map_err(anyhow::Error::msg)?;
        ensure!(
            budget > 0 && !self.house_notification_pending() && !self.view_refresh_pending(),
            "ground input awaits its notification/view child or a positive budget"
        );
        self.host
            .presentation
            .ground_input
            .as_ref()
            .context("ground input absent")?
            .validate(self)?;
        let mut state = self.host.presentation.ground_input.take().unwrap();
        let result = self.advance_ground_input_inner(&mut state, camera, budget);
        self.host.presentation.ground_input = Some(state);
        result
    }
    fn advance_ground_input_inner(
        &mut self,
        s: &mut State,
        camera: Input,
        budget: usize,
    ) -> Result<()> {
        match s.phase {
            Phase::Factory => {
                let ordinary = self.host.ordinary.graphics.keys().copied().collect();
                let (_, result) = self
                    .simulation
                    .advance_native_object_factory(&self.program, &ordinary, budget)
                    .map_err(anyhow::Error::msg)?;
                if let Some(id) = result {
                    s.target = id;
                    if let Some(id) = id {
                        self.simulation
                            .begin_ground_placement(&self.program, id, s.point, s.floor)
                            .map_err(anyhow::Error::msg)?;
                        s.phase = Phase::Placement;
                    } else {
                        s.phase = Phase::Complete;
                    }
                } else if let Some((lot_selection::Request::CreationNotification(id), false)) =
                    self.simulation.pending_native_selection()
                {
                    self.begin_factory_broadcast(id)?;
                } else {
                    self.service_selection_assets(camera, budget)?;
                }
            }
            Phase::Placement => {
                self.simulation
                    .advance_ground_placement(&self.program, budget)
                    .map_err(anyhow::Error::msg)?;
                if self
                    .simulation
                    .ground_placement
                    .as_ref()
                    .unwrap()
                    .complete()
                {
                    let (id, at) = self
                        .simulation
                        .take_ground_placement()
                        .map_err(anyhow::Error::msg)?;
                    if at.is_some() {
                        self.simulation
                            .vm
                            .begin_player_menu(&self.program, s.actor, id)
                            .map_err(anyhow::Error::msg)?;
                        s.phase = Phase::Query;
                    } else {
                        self.start_ground_cleanup(s)?;
                    }
                }
            }
            Phase::Query => {
                let (actions, _) = self
                    .simulation
                    .vm
                    .advance_player_menu(&self.program, budget)
                    .map_err(anyhow::Error::msg)?;
                if let Some(actions) = actions {
                    if actions.is_empty() {
                        s.phase = Phase::MenuFallback;
                        anyhow::bail!("ground menu requires original same-tile fallback");
                    } else {
                        if s.picker_owned {
                            let id = actions[0].target;
                            let flags = self.simulation.vm.world.objects[&id]
                                .native_flags
                                .context("picker target flags absent")?;
                            self.simulation
                                .update_native_low_flags(&self.program, id, (flags & 0x1f) | 1)
                                .map_err(anyhow::Error::msg)?;
                            self.host.presentation.picker_target = Some(id);
                        }
                        s.actions = actions;
                        s.phase = Phase::Ready;
                    }
                }
            }
            Phase::MenuFallback => {
                anyhow::bail!("ground menu requires original same-tile fallback");
            }
            Phase::Removal => {
                let mut step = sims_sim::Step {
                    stop: sims_sim::StepStop::Budget,
                    trace: vec![],
                    events: vec![],
                };
                let mut remaining = budget;
                let done = self
                    .simulation
                    .drive_synchronous_removal(&self.program, &mut remaining, &mut step)
                    .map_err(anyhow::Error::msg)?;
                self.service_selection_assets(camera, budget)?;
                if !self.house_notification_pending() {
                    self.host.service(&mut self.simulation, &mut self.program)?;
                }
                if done {
                    ensure!(
                        Some(
                            self.simulation
                                .take_synchronous_removal()
                                .map_err(anyhow::Error::msg)?
                        ) == s.target
                    );
                    s.phase = Phase::Complete;
                }
            }
            Phase::Ready | Phase::Complete => (),
        }
        Ok(())
    }
    fn start_ground_cleanup(&mut self, s: &mut State) -> Result<()> {
        self.simulation
            .begin_synchronous_removal(
                &self.program,
                s.target.context("ground cleanup target absent")?,
            )
            .map_err(anyhow::Error::msg)?;
        s.actions.clear();
        s.phase = Phase::Removal;
        Ok(())
    }
    /// Normal48dd67 leaf activation (without Shift): enqueue its original
    /// priority50 action. Accepted commands transfer helper ownership to scripts.
    pub fn choose_ground_action(&mut self, index: usize) -> Result<(u32, bool)> {
        ensure!(
            !self.clock_pending(),
            "elapsed caller still owns the menu return"
        );
        if let Some(menu) = &self.host.presentation.live_menu {
            menu.validate(self)?;
        }
        let s = self
            .host
            .presentation
            .ground_input
            .as_ref()
            .context("ground input absent")?;
        s.validate(self)?;
        ensure!(s.ready(), "ground menu not ready");
        let action = s
            .actions
            .get(index)
            .context("ground action index absent")?
            .clone();
        let result = self
            .simulation
            .vm
            .enqueue_action(&self.program, s.actor, action)
            .map_err(anyhow::Error::msg)?;
        if result.1 {
            self.close_running_menu()?;
            self.host.presentation.ground_input = None;
        } else {
            self.cancel_ground_input()?;
        }
        Ok(result)
    }
    pub fn cancel_ground_input(&mut self) -> Result<()> {
        ensure!(
            !self.clock_pending(),
            "elapsed caller still owns the menu return"
        );
        let s = self
            .host
            .presentation
            .ground_input
            .as_ref()
            .context("ground input absent")?;
        s.validate(self)?;
        ensure!(
            s.ready() || s.phase == Phase::MenuFallback,
            "ground menu not ready to cancel"
        );
        self.close_running_menu()?;
        let mut s = self.host.presentation.ground_input.take().unwrap();
        let result = self.start_ground_cleanup(&mut s);
        self.host.presentation.ground_input = Some(s);
        result
    }
    pub fn finish_ground_input(&mut self) -> Result<()> {
        let s = self
            .host
            .presentation
            .ground_input
            .as_ref()
            .context("ground input absent")?;
        s.validate(self)?;
        ensure!(s.complete(), "ground input not complete");
        self.host.presentation.ground_input = None;
        Ok(())
    }
}
