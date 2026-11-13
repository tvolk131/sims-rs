//! Ordinary residential object clicks through48cbec/48c834 and48dd67.
//! The real VM owns menu checks, including ordered same-tile fallback checks.
//! Person joins retain an explicit boundary rather than a substitute menu.
use super::Session;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_vm::{Definition, ObjectId, action::Action};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Query,
    MenuFallback,
    PersonJoin,
    Ready,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Fallback {
    cursor: Option<sims_sim::lot_links::Cursor>,
    visited: Vec<ObjectId>,
    querying: bool,
    actions: Vec<Action>,
    identities: std::collections::BTreeMap<ObjectId, (Definition, u32)>,
}
impl Fallback {
    fn begin(s: &Session, clicked: ObjectId) -> Result<Self> {
        let at = s
            .simulation
            .vm
            .world
            .positions
            .get(&clicked)
            .context("fallback position absent")?;
        let links = &s
            .simulation
            .native_lot
            .as_ref()
            .context("fallback physical links absent")?
            .placement
            .links;
        let root = links.head(
            &s.simulation.vm.world,
            [(at.y >> 4) as i8, (at.x >> 4) as i8, at.level as i8],
        );
        Ok(Self {
            cursor: root.map(sims_sim::lot_links::Cursor::new),
            visited: vec![],
            querying: false,
            actions: vec![],
            identities: Default::default(),
        })
    }
    fn advance_cursor(&mut self, s: &Session) -> Result<()> {
        if let Some(cursor) = &mut self.cursor {
            cursor
                .advance(
                    &s.simulation
                        .native_lot
                        .as_ref()
                        .context("fallback physical links absent")?
                        .placement
                        .links,
                    &s.simulation.vm.world,
                )
                .map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    fn advance(
        &mut self,
        s: &mut Session,
        actor: ObjectId,
        clicked: ObjectId,
        budget: usize,
    ) -> Result<Option<Vec<Action>>> {
        if self.querying {
            let (actions, _) = s
                .simulation
                .vm
                .advance_player_menu(&s.program, budget)
                .map_err(anyhow::Error::msg)?;
            if let Some(actions) = actions {
                for action in &actions {
                    let world = &s.simulation.vm.world;
                    let definition = world
                        .objects
                        .get(&action.target)
                        .and_then(|o| o.definition.clone())
                        .context("fallback action definition absent")?;
                    let handle = world
                        .scheduler_storage
                        .as_ref()
                        .and_then(|r| r.handle(action.target))
                        .context("fallback action lifetime absent")?;
                    self.identities
                        .entry(action.target)
                        .or_insert((definition, handle));
                }
                self.actions.extend(actions);
                self.querying = false;
                //The original iterator advances after the check/destructor;
                //script mutations to slots and links must be observed here.
                self.advance_cursor(s)?;
            }
            return Ok(None);
        }
        let Some(id) = self.cursor.as_ref().and_then(|c| c.current) else {
            return Ok(Some(self.actions.clone()));
        };
        ensure!(
            !self.visited.contains(&id),
            "cyclic same-tile menu traversal"
        );
        //Match native short-circuit reads: the clicked object and portals do
        //not read the later definition flag at all.
        let eligible = if id == clicked {
            false
        } else {
            let d = s
                .simulation
                .vm
                .world
                .objects
                .get(&id)
                .and_then(|o| o.definition.as_ref())
                .context("fallback definition absent")?;
            let words = sims_vm::construction::definition_words(&s.program, d)
                .map_err(anyhow::Error::msg)?;
            if words[9] == 8 {
                false
            } else {
                let flags = *words
                    .get(0xb6 / 2)
                    .context("fallback definition flags absent")?
                    as u16;
                fallback_eligible(id, clicked, words[9], flags)
            }
        };
        if eligible {
            s.simulation
                .vm
                .begin_player_menu(&s.program, actor, id)
                .map_err(anyhow::Error::msg)?;
            self.querying = true;
        } else {
            self.advance_cursor(s)?;
        }
        self.visited.push(id);
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn fallback_eligibility_and_complete_enumeration_match_original() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/picker-tile-fallback.json"
        )))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 192);
        for row in cases {
            let input = &row["input"];
            let mut queries = vec![1_i16];
            if input["counts"][0] == 0 {
                for value in input["order"].as_array().unwrap() {
                    let id = value.as_i64().unwrap() as i16;
                    let definition = &input["definitions"][id as usize - 1];
                    if fallback_eligible(
                        id,
                        1,
                        definition[0].as_i64().unwrap() as i16,
                        definition[1].as_u64().unwrap() as u16,
                    ) {
                        queries.push(id);
                    }
                }
            }
            assert_eq!(
                serde_json::to_value(&queries).unwrap(),
                row["original"]["queries"]
            );
            let actions: u64 = queries
                .iter()
                .map(|id| input["counts"][*id as usize - 1].as_u64().unwrap())
                .sum();
            assert_eq!(actions, row["original"]["actions"].as_u64().unwrap());
            let destructors: Vec<_> = queries
                .iter()
                .skip(1)
                .chain(queries.iter().take(1))
                .copied()
                .collect();
            assert_eq!(
                serde_json::to_value(destructors).unwrap(),
                row["original"]["destructors"]
            );
            assert_eq!(row["original"]["final_temp0"], 42);
            for check in row["original"]["checks"].as_array().unwrap().iter().skip(1) {
                assert_eq!(
                    check["temp0"], 43,
                    "original child check lost its enclosing Temps"
                );
            }
        }
    }
}
fn fallback_eligible(id: ObjectId, clicked: ObjectId, kind: i16, flags: u16) -> bool {
    id != clicked && kind != 8 && flags & 1 != 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct State {
    tick: u64,
    actor: ObjectId,
    clicked: ObjectId,
    definition: Definition,
    target: ObjectId,
    phase: Phase,
    actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fallback: Option<Fallback>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cleanup: Option<sims_vm::player_menu::Cleanup>,
}
impl State {
    pub(super) fn opened_tick(&self) -> u64 {
        self.tick
    }
    pub(super) fn retime(&mut self, tick: u64) {
        self.tick = tick;
    }
    pub(super) fn clicked(&self) -> ObjectId {
        self.clicked
    }
    pub fn ready(&self) -> bool {
        self.phase == Phase::Ready
    }
    pub fn can_cancel(&self) -> bool {
        //48d392 finishes all checks before its caller can dispatch Escape.
        //Retain cancellation of an old unsupported boundary only when no
        //live collector/destructor has acquired that synchronous work.
        matches!(self.phase, Phase::Ready | Phase::PersonJoin)
            || (self.phase == Phase::MenuFallback
                && self.fallback.is_none()
                && self.cleanup.is_none())
    }
    pub fn target(&self) -> ObjectId {
        self.target
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
                || (self.ready() && s.menu_owns_tick(super::live_menu::Kind::Object, self.tick)))
                && !s.ground_input_pending()
                && s.simulation
                    .native_selected_person()
                    .map_err(anyhow::Error::msg)?
                    == Some(self.actor),
            "object input lost its caller or clock"
        );
        ensure!(
            s.simulation
                .vm
                .world
                .objects
                .get(&self.clicked)
                .is_some_and(|o| o.definition.as_ref() == Some(&self.definition)),
            "object input identity changed"
        );
        ensure!(
            sims_vm::multitile::interaction_target(
                &s.simulation.vm.world,
                &s.program,
                self.clicked
            )
            .map_err(anyhow::Error::msg)?
                == self.target,
            "object interaction target changed"
        );
        ensure!(
            (self.phase == Phase::Query || self.fallback.as_ref().is_some_and(|f| f.querying))
                == s.simulation.vm.player_menu.is_some(),
            "object menu query ownership differs"
        );
        ensure!(
            s.simulation
                .vm
                .world
                .objects
                .get(&self.target)
                .is_some_and(|o| o.person.is_some() == (self.phase == Phase::PersonJoin)),
            "object menu target kind differs"
        );
        if let Some(q) = &s.simulation.vm.player_menu {
            let expected = if let Some(f) = self.fallback.as_ref().filter(|f| f.querying) {
                sims_vm::multitile::interaction_target(
                    &s.simulation.vm.world,
                    &s.program,
                    f.cursor
                        .as_ref()
                        .and_then(|c| c.current)
                        .context("fallback query cursor absent")?,
                )
                .map_err(anyhow::Error::msg)?
            } else {
                self.target
            };
            ensure!(
                q.actor() == self.actor && q.target() == expected,
                "object menu query caller differs"
            );
        }
        ensure!(
            self.phase == Phase::Ready || self.actions.is_empty(),
            "unpublished object actions"
        );
        if let Some(f) = &self.fallback {
            ensure!(
                matches!(self.phase, Phase::MenuFallback | Phase::Ready)
                    && (!f.querying || self.phase == Phase::MenuFallback)
                    && (self.phase != Phase::Ready || self.actions == f.actions),
                "invalid same-tile menu continuation"
            );
            for action in &f.actions {
                let (definition, handle) = f
                    .identities
                    .get(&action.target)
                    .context("fallback action identity absent")?;
                let world = &s.simulation.vm.world;
                ensure!(
                    world
                        .objects
                        .get(&action.target)
                        .and_then(|o| o.definition.as_ref())
                        == Some(definition)
                        && world
                            .scheduler_storage
                            .as_ref()
                            .and_then(|r| r.handle(action.target))
                            == Some(*handle),
                    "fallback action lifetime ended"
                );
            }
        }
        if let Some(cleanup) = &self.cleanup {
            ensure!(
                self.phase == Phase::MenuFallback && cleanup.actor() == self.actor,
                "invalid outer menu cleanup owner"
            );
        }
        if self.ready() {
            ensure!(
                !self.actions.is_empty()
                    && s.host.presentation.picker_target == Some(self.actions[0].target),
                "object menu lost its picker target"
            );
            ensure!(
                self.actions.iter().all(|a| (a.target == self.target
                    || self
                        .fallback
                        .as_ref()
                        .is_some_and(|f| f.actions.contains(a)))
                    && a.icon_owner == a.target
                    && a.id == 0
                    && a.priority == 50
                    && (a
                        .label
                        .as_ref()
                        .is_some_and(|s| s.len() <= 65536 && !s.contains('\0'))
                        || (a.label.is_none() && a.args == [0; 4]))
                    && a.flags & 28 == 12
                    && a.flags & !13 == 0
                    && s.simulation
                        .vm
                        .action_menu(&s.program, a.target)
                        .is_ok_and(|entries| entries.iter().any(|e| e.id == a.menu_id
                            && e.action == a.routine
                            && a.flags & 1 == u32::from(e.flags & 0x100 != 0)))),
                "object menu action changed"
            );
        }
        Ok(())
    }
}
impl Session {
    pub fn object_input_pending(&self) -> bool {
        self.host.presentation.object_input.is_some()
    }
    pub fn player_input_pending(&self) -> bool {
        self.non_menu_input_pending() || self.ground_input_pending() || self.object_input_pending()
    }
    /// A published popup retains input ownership without owning a VM slice.
    /// Factory/query/cleanup and every other synchronous input still block.
    pub fn player_input_blocks_ticks(&self) -> bool {
        self.non_menu_input_pending()
            || (self.ground_input_pending()
                && !(self.running_menu()
                    && self
                        .host
                        .presentation
                        .ground_input
                        .as_ref()
                        .unwrap()
                        .ready()))
            || (self.object_input_pending()
                && !(self.running_menu()
                    && self
                        .host
                        .presentation
                        .object_input
                        .as_ref()
                        .unwrap()
                        .ready()))
    }
    fn non_menu_input_pending(&self) -> bool {
        self.purchase_cursor_pending()
            || self.mode_publication_pending()
            || self.publication_pending()
            || self.tool_selection_pending()
            || self.move_tool_pending()
            || self.move_button_pending()
            || self.simulation.object_pickup.is_some()
            || self.simulation.object_placement.is_some()
            || self.simulation.cursor_factory.is_some()
    }
    /// Native435286's interaction-table redirect is applied to the clicked
    /// part; it is not an unconditional selection of the group head.
    /// Returns false for48cc42's ordinary no-selected-person early return.
    pub fn begin_object_input(&mut self, clicked: ObjectId) -> Result<bool> {
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
                && !self.simulation.selection_work_pending()
                && self.simulation.vm.world.dialogs.modal.is_none()
                && !self.simulation.vm.constructing_dialog(),
            "object input overlaps synchronous UI work"
        );
        //48cc0e resets the old picker before48cc35 scans the person registry.
        //No selected person is a valid Live state, not a failed menu query.
        self.clear_picker_target()?;
        let Some(actor) = self.picker_input_actor()? else {
            return Ok(false);
        };
        let definition = self
            .simulation
            .vm
            .world
            .objects
            .get(&clicked)
            .and_then(|o| o.definition.clone())
            .context("clicked object definition absent")?;
        let target = sims_vm::multitile::interaction_target(
            &self.simulation.vm.world,
            &self.program,
            clicked,
        )
        .map_err(anyhow::Error::msg)?;
        let phase = if self.simulation.vm.world.objects[&target].person.is_some() {
            Phase::PersonJoin
        } else {
            self.simulation
                .vm
                .begin_player_menu(&self.program, actor, target)
                .map_err(anyhow::Error::msg)?;
            Phase::Query
        };
        self.host.presentation.object_input = Some(State {
            tick: self.simulation.vm.tick,
            actor,
            clicked,
            definition,
            target,
            phase,
            actions: vec![],
            fallback: None,
            cleanup: None,
        });
        Ok(true)
    }
    pub fn advance_object_input(&mut self, budget: usize) -> Result<()> {
        ensure!(budget > 0, "object menu needs a positive budget");
        self.host
            .presentation
            .object_input
            .as_ref()
            .context("object input absent")?
            .validate(self)?;
        let mut state = self.host.presentation.object_input.take().unwrap();
        let result = self.advance_object_input_inner(&mut state, budget);
        self.host.presentation.object_input =
            (result.is_err() || state.phase != Phase::Complete).then_some(state);
        result
    }
    fn advance_object_input_inner(&mut self, state: &mut State, budget: usize) -> Result<()> {
        match state.phase {
            Phase::Query => {
                let (completed, _) = self
                    .simulation
                    .vm
                    .advance_player_menu_retained(&self.program, budget)
                    .map_err(anyhow::Error::msg)?;
                if let Some(completed) = completed {
                    let actions = completed.actions;
                    if actions.is_empty() {
                        state.cleanup = Some(completed.cleanup);
                        state.phase = Phase::MenuFallback;
                        return Ok(());
                    }
                    completed
                        .cleanup
                        .restore(&mut self.simulation.vm)
                        .map_err(anyhow::Error::msg)?;
                    let target = actions[0].target;
                    let flags = self.simulation.vm.world.objects[&target]
                        .native_flags
                        .context("object picker flags absent")?;
                    self.simulation
                        .update_native_low_flags(&self.program, target, (flags & 0x1f) | 1)
                        .map_err(anyhow::Error::msg)?;
                    self.host.presentation.picker_target = Some(target);
                    state.actions = actions;
                    state.phase = Phase::Ready;
                }
            }
            Phase::MenuFallback => {
                if state.cleanup.is_none() && state.fallback.is_none() {
                    ensure!(
                        self.simulation
                            .vm
                            .action_menu(&self.program, state.target)
                            .map_err(anyhow::Error::msg)?
                            .iter()
                            .all(|e| e.check == 0),
                        "historical empty menu lost its outer Temps before tile fallback"
                    );
                }
                let mut fallback = state
                    .fallback
                    .take()
                    .map(Ok)
                    .unwrap_or_else(|| Fallback::begin(self, state.clicked))?;
                let result = fallback.advance(self, state.actor, state.clicked, budget);
                state.fallback = Some(fallback);
                if let Some(actions) = result? {
                    if let Some(cleanup) = state.cleanup.take() {
                        cleanup
                            .restore(&mut self.simulation.vm)
                            .map_err(anyhow::Error::msg)?;
                    }
                    let (target, bit) = actions
                        .first()
                        .map_or((state.clicked, 8), |a| (a.target, 1));
                    let flags = self.simulation.vm.world.objects[&target]
                        .native_flags
                        .context("fallback picker flags absent")?;
                    self.simulation
                        .update_native_low_flags(&self.program, target, (flags & 0x1f) | bit)
                        .map_err(anyhow::Error::msg)?;
                    self.host.presentation.picker_target = Some(target);
                    state.actions = actions;
                    state.phase = if state.actions.is_empty() {
                        Phase::Complete
                    } else {
                        Phase::Ready
                    };
                }
            }
            Phase::PersonJoin => anyhow::bail!("person menu requires original join enumeration"),
            Phase::Ready | Phase::Complete => (),
        }
        Ok(())
    }
    pub fn choose_object_action(&mut self, index: usize) -> Result<(u32, bool)> {
        ensure!(
            !self.clock_pending(),
            "elapsed caller still owns the menu return"
        );
        if let Some(menu) = &self.host.presentation.live_menu {
            menu.validate(self)?;
        }
        let state = self
            .host
            .presentation
            .object_input
            .as_ref()
            .context("object input absent")?;
        state.validate(self)?;
        ensure!(state.ready(), "object menu not ready");
        let action = state
            .actions
            .get(index)
            .context("object menu index absent")?
            .clone();
        let result = self
            .simulation
            .vm
            .enqueue_action(&self.program, state.actor, action)
            .map_err(anyhow::Error::msg)?;
        // No-Shift leaf activation closes the popup on success or rejection.
        // Existing objects never acquire the fresh ground helper's destructor.
        self.close_running_menu()?;
        self.host.presentation.object_input = None;
        Ok(result)
    }
    pub fn cancel_object_input(&mut self) -> Result<()> {
        ensure!(
            !self.clock_pending(),
            "elapsed caller still owns the menu return"
        );
        let state = self
            .host
            .presentation
            .object_input
            .as_ref()
            .context("object input absent")?;
        state.validate(self)?;
        ensure!(state.can_cancel(), "object query cannot be interrupted");
        if let Some(cleanup) = state.cleanup.clone() {
            cleanup
                .restore(&mut self.simulation.vm)
                .map_err(anyhow::Error::msg)?;
        }
        self.close_running_menu()?;
        self.host.presentation.object_input = None;
        Ok(())
    }
}
