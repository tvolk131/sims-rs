//!Live4878f3/487105/487191 callers. The tool retains its held lifetime until the
//!original post44123f store; engine children are never externally acknowledged.
use super::Session;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::{
    edit_history::{self, object, snapshot},
    object_catalog::move_tool::{Groups, cleanup, dispatch, hover},
};
use sims_vm::{ObjectId, World};
pub mod button;
pub(super) mod close;
mod commit;
mod history;
pub mod motion;
pub mod pickup;
mod preview;
mod sale;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    pub store: edit_history::Store,
    pub objects: object::Objects,
    pub references: snapshot::References,
    pub budgets: object::budget::Budgets,
    ///Retained catalogue identities used by embedded ObjectUndoable snapshots.
    pub definitions: std::collections::BTreeMap<u32, sims_vm::Definition>,
    ///Actual ordered registration, when supplied by the application. Legacy
    ///fixtures with only definition handles cannot answer43ae0a.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    catalog: Option<sims_sim::object_catalog::definition::Catalog>,
    ///Retained DAT006e4ad4 input for ordinary/group Buy placement.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub placement_cheat: bool,
}
impl History {
    pub fn new(next_reference_key: u32) -> Self {
        Self {
            store: Default::default(),
            objects: Default::default(),
            references: snapshot::References::new(next_reference_key),
            budgets: Default::default(),
            definitions: Default::default(),
            catalog: None,
            placement_cheat: false,
        }
    }
    pub fn retain_catalog(
        &mut self,
        catalog: &mut sims_content::object_catalog::Catalog<'_>,
    ) -> Result<()> {
        ensure!(
            self.catalog.is_none() && self.definitions.is_empty() && self.objects.0.is_empty(),
            "history already retains definition identities"
        );
        let mut definitions = std::collections::BTreeMap::new();
        for row in 0..catalog.definitions.entries.len() {
            let id = u32::try_from(row)?
                .checked_add(1)
                .context("catalog identity overflow")?;
            definitions.insert(
                id,
                catalog
                    .portrait_definition(id)
                    .map_err(anyhow::Error::msg)?,
            );
        }
        self.catalog = Some(catalog.definitions.clone());
        self.definitions = definitions;
        Ok(())
    }
    pub fn definition_handle(&self, definition: &sims_vm::Definition) -> Result<u32, String> {
        let mut matches = self
            .definitions
            .iter()
            .filter_map(|(&id, d)| (d == definition).then_some(id));
        let id = matches
            .next()
            .ok_or("history snapshot definition was not retained")?;
        if matches.next().is_some() {
            return Err("history snapshot definition has ambiguous retained identities".into());
        }
        Ok(id)
    }
    ///43ab8f scans registered resource names in order and returns the first
    ///case-insensitive match; GUID-map replacement order is a different lookup.
    pub fn named_definition(&self, name: &[u8]) -> Result<Option<sims_vm::Definition>, String> {
        let catalog = self
            .catalog
            .as_ref()
            .ok_or("named definition catalog absent")?;
        for (index, entry) in catalog.entries.iter().enumerate() {
            if entry.name.eq_ignore_ascii_case(name) {
                return self
                    .definitions
                    .get(&(index as u32 + 1))
                    .cloned()
                    .map(Some)
                    .ok_or_else(|| "named definition identity absent".into());
            }
        }
        Ok(None)
    }
    fn validate_catalog(&self) -> Result<()> {
        if let Some(catalog) = &self.catalog {
            ensure!(
                catalog.entries.len() == self.definitions.len(),
                "history catalog size differs"
            );
            for (row, entry) in catalog.entries.iter().enumerate() {
                let id = u32::try_from(row)?
                    .checked_add(1)
                    .context("catalog identity overflow")?;
                let d = self
                    .definitions
                    .get(&id)
                    .context("history catalog definition absent")?;
                ensure!(
                    entry.index == row as u32 && entry.data.len() == 216,
                    "history catalog row differs"
                );
                ensure!(
                    entry.key.chunk == d.object
                        && u32::from_le_bytes(entry.data[28..32].try_into().unwrap()) == d.guid
                        && u16::from_le_bytes(entry.data[40..42].try_into().unwrap()) == d.slots,
                    "history catalog definition differs"
                );
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub tool: cleanup::State,
    pub inputs: cleanup::Inputs,
    pub history: History,
    ///Application-owned process group lists, retained explicitly. Loading an
    ///old household does not initialize the original static lists again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    groups: Option<Groups>,
    ///Fields first used by486f5a. Historical tools remain unbound until the
    ///application explicitly starts their first preview with no held object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview_fields: Option<preview::Fields>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_pointer: Option<[i16; 2]>,
    #[serde(default, skip_serializing_if = "button::Fields::is_default")]
    button_fields: button::Fields,
    call: Option<Execution>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Execution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner: Option<super::tool_selection::MoveCaller>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current: Option<super::current_tool::MoveCaller>,
    tick: u64,
    dispatch: dispatch::Dispatch,
    child: Option<Child>,
    retired_held: Option<(u32, ObjectId)>,
    retired_payload: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview: Option<preview::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    motion: Option<motion::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    commit: Option<commit::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pickup: Option<pickup::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sale: Option<sale::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    button_owner: Option<button::Operation>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Child {
    Detach(u32),
    Forget(u32),
    Delete {
        id: ObjectId,
        object: u32,
    },
    Kind {
        payload: u32,
        value: u32,
        call: object::kind::Call,
    },
    History(Box<history::Execution>),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Service {
    Input(cleanup::Request),
    Pickup(pickup::Request),
    ///57a54c(name,-4), after the original per-channel timer gate.
    Feedback(sims_sim::object_catalog::move_tool::sell::Feedback),
    History(edit_history::manager::Service),
    Object(object::driver::Service),
    Notification(super::notifications::Service),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Complete,
    Service(Service),
}
pub struct Advance {
    pub progress: Progress,
    pub instructions: usize,
}

fn id(world: &World, h: u32) -> Result<ObjectId, String> {
    world
        .scheduler_storage
        .as_ref()
        .and_then(|r| r.object_for_handle(h))
        .ok_or("move tool object lifetime ended".into())
}
struct Live<'a> {
    world: &'a World,
    objects: &'a mut object::Objects,
}
impl cleanup::View for Live<'_> {
    fn disposable(&self, h: u32) -> Result<bool, String> {
        Ok(self.world.objects[&id(self.world, h)?]
            .cursor
            .as_ref()
            .ok_or("held object is not a cursor")?
            .underlying
            .is_some())
    }
    fn script_id(&self, h: u32) -> Result<i16, String> {
        id(self.world, h)
    }
    fn clear_payload_cost(&mut self, payload: u32) -> Result<(), String> {
        self.objects.get_mut(payload)?.cost = Some(0);
        Ok(())
    }
}
impl hover::View for Live<'_> {
    fn lookup(&self, object: i16) -> Result<Option<u32>, String> {
        Ok(self
            .world
            .scheduler_storage
            .as_ref()
            .ok_or("move hover registry absent")?
            .handle(object))
    }
    fn flags(&self, h: u32) -> Result<u32, String> {
        self.world.objects[&id(self.world, h)?]
            .native_flags
            .ok_or("move hover native flags absent".into())
    }
}
impl State {
    pub fn button_hint(&self) -> Option<button::Hint> {
        self.button_fields.hint
    }
    pub fn sale_request(&self) -> Option<sims_sim::object_catalog::move_tool::sell::Request> {
        self.call.as_ref()?.sale.as_ref()?.request()
    }
    pub(super) fn current_owner(&self) -> Option<&super::current_tool::MoveCaller> {
        self.call.as_ref()?.current.as_ref()
    }
    pub(super) fn selection_owner(&self) -> Option<&super::tool_selection::MoveCaller> {
        self.call.as_ref()?.owner.as_ref()
    }
    pub(super) fn claim_selection(
        &mut self,
        owner: super::tool_selection::MoveCaller,
    ) -> Result<()> {
        let call = self.call.as_mut().context("move call absent")?;
        ensure!(
            call.owner.is_none(),
            "move call already has an interface owner"
        );
        call.owner = Some(owner);
        Ok(())
    }
    pub(super) fn catalog(&self) -> Result<&sims_sim::object_catalog::definition::Catalog> {
        self.history.catalog.as_ref().context("move catalog absent")
    }
    pub(super) fn classify_definition(
        &mut self,
        masters: &mut sims_sim::object_catalog::move_tool::selected::Masters,
        id: u32,
    ) -> Result<i32> {
        ensure!(!self.pending(), "move classification overlaps a callback");
        let result = self
            .groups
            .as_ref()
            .context("move groups absent")?
            .select_definition(
                &mut self.tool.tool,
                masters,
                self.history
                    .catalog
                    .as_ref()
                    .context("move catalog absent")?,
                id,
            )
            .map_err(anyhow::Error::msg);
        if let Some(f) = &self.preview_fields {
            self.history
                .budgets
                .0
                .get_mut(&f.budget)
                .context("Move budget owner absent")?
                .category = self.tool.tool.cursor;
        }
        result
    }
    ///The application supplies its actual tool/history owners. Restoring a
    ///house alone never invents a Buy tool or replaces an existing command log.
    pub fn new(tool: cleanup::State, inputs: cleanup::Inputs, history: History) -> Self {
        Self {
            tool,
            inputs,
            history,
            groups: None,
            preview_fields: None,
            last_pointer: None,
            button_fields: Default::default(),
            call: None,
        }
    }
    pub fn retain_activation_groups(&mut self, groups: Groups) -> Result<()> {
        ensure!(
            !self.pending() && self.groups.is_none() && self.history.catalog.is_some(),
            "move activation requires an idle tool and its retained catalog"
        );
        self.history.validate_catalog()?;
        self.groups = Some(groups);
        Ok(())
    }
    fn creating_cursor(&self) -> bool {
        self.preview_creating()
            || self
                .call
                .as_ref()
                .and_then(|c| c.pickup.as_ref())
                .is_some_and(pickup::Execution::owns_factory)
    }
    pub fn pending(&self) -> bool {
        self.call.is_some()
    }
    pub fn complete(&self) -> bool {
        self.call.as_ref().is_some_and(|e| {
            e.dispatch.complete()
                && e.child.is_none()
                && e.preview.as_ref().is_none_or(preview::Execution::complete)
                && e.motion.as_ref().is_none_or(motion::Execution::complete)
                && e.commit.as_ref().is_none_or(commit::Execution::complete)
                && e.pickup.as_ref().is_none_or(pickup::Execution::complete)
                && e.sale.as_ref().is_none_or(sale::Execution::complete)
        })
    }
    pub fn retained_deleted_held(&self) -> Option<(u32, ObjectId)> {
        self.call.as_ref()?.retired_held
    }
    ///The486ada result, retained until the enclosing Move call is finished.
    pub fn commit_outcome(&self) -> Option<bool> {
        self.call.as_ref()?.commit.as_ref()?.outcome()
    }
    pub(super) fn budget_caller(&self) -> Option<object::driver::BudgetCaller> {
        match self.call.as_ref()?.child.as_ref()? {
            Child::History(c) => c.budget_caller(),
            _ => None,
        }
    }
    pub(super) fn validate(&self, s: &Session) -> Result<()> {
        self.tool.validate().map_err(anyhow::Error::msg)?;
        self.history.store.validate().map_err(anyhow::Error::msg)?;
        self.history.validate_catalog()?;
        if let Some(f) = &self.preview_fields {
            f.validate(self)?;
        }
        self.history
            .references
            .validate()
            .map_err(anyhow::Error::msg)?;
        ensure!(
            self.history.definitions.keys().all(|&id| id != 0),
            "null history definition identity"
        );
        if let Some(target) = self
            .tool
            .held
            .and_then(|h| id(&s.simulation.vm.world, h).ok())
        {
            ensure!(
                s.simulation.vm.world.objects[&target]
                    .cursor
                    .as_ref()
                    .is_some_and(|c| c.postconstructed),
                "move held receiver is not a constructed cursor"
            );
        }
        ensure!(
            self.inputs.registry != 0 && self.tool.clipboard != 0,
            "move tool manager identity absent"
        );
        self.history
            .store
            .vector(edit_history::List::Undo(self.tool.clipboard))
            .map_err(anyhow::Error::msg)?;
        self.history
            .store
            .vector(edit_history::List::Redo(self.tool.clipboard))
            .map_err(anyhow::Error::msg)?;
        if let Some(payload) = self
            .tool
            .payload
            .filter(|&p| self.retired_payload() != Some(p))
        {
            self.history
                .store
                .command(payload)
                .map_err(anyhow::Error::msg)?;
            self.history
                .objects
                .get(payload)
                .map_err(anyhow::Error::msg)?;
        }
        let Some(e) = &self.call else {
            if let Some(h) = self.tool.held {
                id(&s.simulation.vm.world, h).map_err(anyhow::Error::msg)?;
            }
            return Ok(());
        };
        ensure!(
            [
                e.owner.is_some(),
                e.current.is_some(),
                e.button_owner.is_some()
            ]
            .into_iter()
            .filter(|b| *b)
            .count()
                <= 1,
            "Move has two interface owners"
        );
        ensure!(
            e.tick == s.simulation.vm.tick,
            "move tool crossed a lot tick"
        );
        e.dispatch.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            [
                e.preview.is_some(),
                e.motion.is_some(),
                e.commit.is_some(),
                e.pickup.is_some(),
                e.sale.is_some()
            ]
            .into_iter()
            .filter(|b| *b)
            .count()
                <= 1,
            "Move owns two pointer calls"
        );
        if let Some(p) = &e.pickup {
            p.validate(s, self)?;
        }
        if let Some(c) = &e.commit {
            c.validate(s, self)?;
        }
        if let Some(m) = &e.motion {
            m.validate(s, self)?;
        }
        if let Some(p) = &e.preview {
            p.validate(s, self)?;
        }
        if let Some(c) = &e.sale {
            c.validate(s, self)?;
        }
        ensure!(
            !e.dispatch.requires_catalog()
                || self.groups.is_some() && self.history.catalog.is_some(),
            "move activation lost its retained catalog/groups"
        );
        ensure!(
            !s.ground_input_pending()
                && !s.object_input_pending()
                && !s.transport_pending()
                && !s.clock_pending()
                && !s.camera_command_pending()
                && !s.interface_update_pending()
                && !s.control_panel_pending()
                && !s.ui_wait_pending()
                && !s.ui_portrait_pending()
                && !s.load_completion_pending()
                && !s.postload_pending()
                && !s.view_refresh_pending()
                && !s.host.ordinary.updates_pending()
                && (self.creating_cursor() || s.simulation.cursor_factory.is_none()),
            "saved move tool overlaps application work"
        );
        if let Some(payload) = self.retired_payload() {
            ensure!(
                self.tool.payload == Some(payload)
                    && !self.history.store.commands.contains_key(&payload)
                    && !self.history.objects.0.contains_key(&payload),
                "released move payload has invalid storage"
            );
        }
        if e.retired_payload.is_some() {
            ensure!(
                (e.dispatch.awaiting_payload_clear()
                    || e.commit
                        .as_ref()
                        .is_some_and(commit::Execution::awaiting_payload_clear))
                    && e.child.is_none(),
                "released move payload lost clear continuation"
            );
        }
        if let Some(h) = self.tool.held {
            if id(&s.simulation.vm.world, h).is_err() {
                let retained = e
                    .retired_held
                    .is_some_and(|(old, _)| old == h && e.dispatch.awaiting_held_clear());
                let deleting = matches!(e.child,Some(Child::Delete{object,..}) if object==h)
                    && s.simulation.synchronous_removal.is_some();
                ensure!(
                    (retained || deleting)
                        && s.simulation
                            .vm
                            .world
                            .scheduler_storage
                            .as_ref()
                            .unwrap()
                            .retired_handle(h),
                    "move tool lost its held lifetime owner"
                );
            }
        }
        if let Some((h, target)) = e.retired_held {
            ensure!(
                self.tool.held == Some(h)
                    && target > 0
                    && e.dispatch.awaiting_held_clear()
                    && s.simulation
                        .vm
                        .world
                        .scheduler_storage
                        .as_ref()
                        .unwrap()
                        .retired_handle(h),
                "invalid retired move-tool held pointer"
            );
        }
        let request = e.dispatch.service();
        match &e.child {
            Some(Child::Detach(h)) => {
                ensure!(
                    request
                        == Some(dispatch::Service::External(cleanup::Request::ObjectMode(
                            *h
                        ))),
                    "move detach lost caller"
                );
                ensure!(
                    s.simulation
                        .cursor_rotation
                        .as_ref()
                        .is_some_and(|r| r.removal() && r.binding().0 == *h),
                    "move detach lost engine child"
                );
            }
            Some(Child::Forget(h)) => {
                ensure!(
                    request
                        == Some(dispatch::Service::External(cleanup::Request::DetachObject(
                            *h
                        ))),
                    "move forget lost caller"
                );
                ensure!(
                    s.simulation
                        .cursor_cleanup
                        .as_ref()
                        .is_some_and(|r| r.cursor() == *h
                            && r.kind() == sims_sim::lot_cursor::cleanup::Kind::Forget),
                    "move forget lost engine child"
                );
            }
            Some(Child::Delete { id, object }) => {
                ensure!(
                    request
                        == Some(dispatch::Service::External(
                            cleanup::Request::DeleteObject {
                                registry: self.inputs.registry,
                                id: *id
                            }
                        )),
                    "move deletion lost caller"
                );
                ensure!(
                    s.simulation
                        .synchronous_removal
                        .as_ref()
                        .is_some_and(|r| r.target() == *id),
                    "move deletion lost engine child"
                );
                ensure!(
                    self.tool.held == Some(*object),
                    "move deletion held receiver changed"
                );
            }
            Some(Child::Kind {
                payload,
                value,
                call,
            }) => {
                call.validate(&self.history.store)
                    .map_err(anyhow::Error::msg)?;
                ensure!(
                    call.binding() == (*payload, *value)
                        && request
                            == Some(dispatch::Service::External(cleanup::Request::PayloadKind {
                                payload: *payload,
                                value: *value
                            })),
                    "move kind setter lost caller"
                );
            }
            Some(Child::History(call)) => {
                ensure!(
                    request == Some(dispatch::Service::External(call.request())),
                    "move history lost caller"
                );
                call.validate(s, &self.history, self.inputs.registry)?;
            }
            None => (),
        }
        ensure!(
            self.budget_caller()
                == s.host
                    .presentation
                    .object_budget
                    .as_ref()
                    .map(|b| b.caller()),
            "move history budget child differs"
        );
        ensure!(
            matches!(&e.child, Some(Child::History(c)) if c.owns_placement())
                || s.simulation.object_placement.is_none(),
            "unowned object placement overlaps move tool"
        );
        if let Some(placement) = &s.simulation.object_placement {
            ensure!(
                placement.placement_cheat() == self.history.placement_cheat,
                "history placement cheat input changed"
            );
        }
        ensure!(
            matches!(&e.child, Some(Child::History(c)) if c.owns_pickup())
                || self.creating_cursor()
                    && s.simulation.cursor_factory.as_ref().is_some_and(|factory| {
                        s.simulation
                            .object_pickup
                            .as_ref()
                            .is_some_and(|p| factory.owns_pickup(p))
                    })
                || s.simulation.object_pickup.is_none(),
            "unowned object pickup overlaps move tool"
        );
        ensure!(
            matches!(e.child, Some(Child::Detach(_)))
                || self.preview_orienting()
                || e.motion
                    .as_ref()
                    .is_some_and(motion::Execution::owns_cursor)
                || s.simulation.cursor_rotation.is_none(),
            "unowned cursor detach overlaps move tool"
        );
        ensure!(
            matches!(e.child, Some(Child::Forget(_))) || s.simulation.cursor_cleanup.is_none(),
            "unowned cursor cleanup overlaps move tool"
        );
        ensure!(
            e.commit
                .as_ref()
                .is_some_and(commit::Execution::owns_release)
                || s.simulation.cursor_release.is_none(),
            "unowned cursor release overlaps Move tool"
        );
        ensure!(
            matches!(e.child, Some(Child::Delete { .. }))
                || matches!(&e.child, Some(Child::History(c)) if c.owns_deletion())
                || self.creating_cursor()
                || s.simulation.synchronous_removal.is_none(),
            "unowned destruction overlaps move tool"
        );
        ensure!(
            matches!(&e.child, Some(Child::History(c)) if c.owns_factory())
                || self.creating_cursor()
                || s.simulation.native_selection.is_none(),
            "unowned factory or selection overlaps move tool"
        );
        ensure!(
            matches!(&e.child, Some(Child::History(c)) if c.owns_notification())
                || e.sale
                    .as_ref()
                    .is_some_and(sale::Execution::owns_notification)
                || self.creating_cursor()
                || !s.house_notification_pending()
                || matches!(e.child, Some(Child::Delete { .. })),
            "unowned notification overlaps move tool"
        );
        Ok(())
    }
    fn retired_payload(&self) -> Option<u32> {
        let e = self.call.as_ref()?;
        e.retired_payload.or_else(|| match &e.child {
            Some(Child::History(c)) => c.retired_payload(),
            _ => None,
        })
    }
}
impl Session {
    ///The exact44123f child owned by the retained Move/history call. A UI
    ///client must not mistake this deletion for an unowned portrait preview.
    pub fn move_tool_removal_target(&self) -> Option<ObjectId> {
        match self
            .host
            .presentation
            .move_tool
            .as_ref()?
            .call
            .as_ref()?
            .child
            .as_ref()?
        {
            Child::Delete { id, .. } => Some(*id),
            Child::History(c) => c.removal_target(),
            _ => None,
        }
    }
    ///486ada alone: actual furniture release and undo registration. The
    ///mouse-up caller still owns affordability, Reset and budget application.
    pub fn begin_move_tool_commit(&mut self) -> Result<()> {
        self.check_move_tool_entry()?;
        ensure!(
            !self.publication_pending(),
            "Move commit overlaps publication"
        );
        let current = self
            .host
            .presentation
            .current_tool
            .as_ref()
            .and_then(|s| s.current());
        ensure!(
            current.is_some_and(|h| matches!(
                self.host
                    .presentation
                    .tool_selection
                    .as_ref()
                    .and_then(|s| s.receivers().get(&h)),
                Some(super::tool_selection::Receiver::Move)
            )),
            "commit current tool is not Move"
        );
        let state = self
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("Move absent")?;
        state.validate(self)?;
        ensure!(
            !state.pending() && state.preview_fields.is_some(),
            "Move commit has no idle field owner"
        );
        self.host.presentation.move_tool.as_mut().unwrap().call = Some(Execution {
            button_owner: None,
            pickup: None,
            owner: None,
            current: None,
            tick: self.simulation.vm.tick,
            dispatch: dispatch::Dispatch::returned(),
            child: None,
            retired_held: None,
            retired_payload: None,
            preview: None,
            motion: None,
            commit: Some(commit::Execution::default()),
            sale: None,
        });
        Ok(())
    }
    ///Direct virtual+14/486f5a call. This does not enable pointer movement;
    ///the viewport wrapper487d3d does that only after this child returns.
    pub fn begin_move_tool_preview(&mut self) -> Result<()> {
        self.begin_move_preview(false)
    }
    ///Actual virtual+58/487d3d: preview first, then enable pointer movement.
    pub fn begin_move_tool_viewport_enter(&mut self) -> Result<()> {
        self.begin_move_preview(true)
    }
    fn begin_move_preview(&mut self, viewport_entry: bool) -> Result<()> {
        self.check_move_tool_entry()?;
        ensure!(
            !self.publication_pending(),
            "pointer entry overlaps interface publication"
        );
        let tool = self
            .host
            .presentation
            .current_tool
            .as_ref()
            .and_then(super::current_tool::State::current)
            .context("viewport current tool absent")?;
        ensure!(
            matches!(
                self.host
                    .presentation
                    .tool_selection
                    .as_ref()
                    .and_then(|s| s.receivers().get(&tool)),
                Some(super::tool_selection::Receiver::Move)
            ),
            "viewport current tool is not Move"
        );
        self.load_move_text()?;
        let state = self
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("Move tool absent")?;
        state.validate(self)?;
        self.host.presentation.move_tool.as_mut().unwrap().call = Some(Execution {
            button_owner: None,
            pickup: None,
            owner: None,
            current: None,
            tick: self.simulation.vm.tick,
            dispatch: dispatch::Dispatch::returned(),
            child: None,
            retired_held: None,
            retired_payload: None,
            motion: None,
            commit: None,
            sale: None,
            preview: Some(if viewport_entry {
                preview::Execution::viewport_entry(tool)
            } else {
                preview::Execution::new(tool)
            }),
        });
        Ok(())
    }
    ///Move an existing ordinary furniture preview using a point picked by
    ///the replacement renderer. Ordinary drag rotation shares this caller; special tools remain separate.
    pub fn begin_move_tool_motion(&mut self, input: motion::Input) -> Result<()> {
        self.check_move_tool_entry()?;
        ensure!(
            !self.publication_pending(),
            "pointer motion overlaps publication"
        );
        let current = self
            .host
            .presentation
            .current_tool
            .as_ref()
            .and_then(|s| s.current());
        ensure!(
            current.is_some_and(|h| matches!(
                self.host
                    .presentation
                    .tool_selection
                    .as_ref()
                    .and_then(|s| s.receivers().get(&h)),
                Some(super::tool_selection::Receiver::Move)
            )),
            "pointer current tool is not Move"
        );
        let state = self
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("Move absent")?;
        state.validate(self)?;
        ensure!(
            matches!(state.tool.tool.mode, 1 | 2) && state.preview_fields.is_some()
                || state.tool.held.is_none() && state.tool.tool.mode == 1,
            "pointer requires ordinary furniture or an empty Move tool"
        );
        let held = state.tool.held.unwrap_or(0);
        self.host.presentation.move_tool.as_mut().unwrap().call = Some(Execution {
            button_owner: None,
            pickup: None,
            owner: None,
            current: None,
            tick: self.simulation.vm.tick,
            dispatch: dispatch::Dispatch::returned(),
            child: None,
            retired_held: None,
            retired_payload: None,
            preview: None,
            motion: Some(motion::Execution::new(input, held)),
            commit: None,
            sale: None,
        });
        Ok(())
    }
    pub fn move_tool_pending(&self) -> bool {
        self.host
            .presentation
            .move_tool
            .as_ref()
            .is_some_and(State::pending)
    }
    pub fn bind_move_tool(&mut self, state: State) -> Result<()> {
        ensure!(
            self.host.presentation.move_tool.is_none() && !state.pending(),
            "move tool already bound or caller active"
        );
        self.check_move_tool_entry()?;
        state.validate(self)?;
        self.host.presentation.move_tool = Some(state);
        Ok(())
    }
    pub(super) fn check_move_tool_entry(&self) -> Result<()> {
        ensure!(
            !self.player_input_pending()
                && !self.transport_pending()
                && !self.clock_pending()
                && !self.camera_command_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_wait_pending()
                && !self.ui_portrait_pending()
                && !self.load_completion_pending()
                && !self.postload_pending()
                && !self.view_refresh_pending()
                && !self.house_notification_pending()
                && self.host.presentation.object_budget.is_none()
                && !self.host.presentation.awaiting_house_open
                && !self.host.ordinary.updates_pending()
                && !self.simulation.selection_work_pending()
                && !self.simulation.cursor_work_pending()
                && self.simulation.synchronous_removal.is_none()
                && self.simulation.vm.world.dialogs.modal.is_none()
                && !self.simulation.vm.constructing_dialog(),
            "move tool overlaps synchronous work"
        );
        Ok(())
    }
    pub fn begin_move_tool_cleanup(&mut self, kind: cleanup::Kind) -> Result<()> {
        self.check_move_tool_entry()?;
        self.host
            .presentation
            .move_tool
            .as_ref()
            .context("move tool absent")?
            .validate(self)?;
        self.host.presentation.move_tool.as_mut().unwrap().call = Some(Execution {
            button_owner: None,
            pickup: None,
            owner: None,
            current: None,
            tick: self.simulation.vm.tick,
            dispatch: dispatch::Dispatch::cleanup(kind),
            child: None,
            retired_held: None,
            retired_payload: None,
            preview: None,
            motion: None,
            commit: None,
            sale: None,
        });
        Ok(())
    }
    ///4878f3 retains its signed-word argument across the complete487191
    ///deactivation. The shared driver owns all history/object children.
    pub fn begin_move_tool_activation(&mut self, argument: u32) -> Result<()> {
        self.check_move_tool_entry()?;
        let state = self
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("move tool absent")?;
        state.validate(self)?;
        ensure!(
            state.groups.is_some() && state.history.catalog.is_some(),
            "move activation catalog/groups were not retained"
        );
        ensure!(
            self.simulation.vm.world.globals.contains_key(&26),
            "move activation simulator global26 absent"
        );
        self.host.presentation.move_tool.as_mut().unwrap().call = Some(Execution {
            button_owner: None,
            pickup: None,
            owner: None,
            current: None,
            tick: self.simulation.vm.tick,
            dispatch: dispatch::Dispatch::activate(argument),
            child: None,
            retired_held: None,
            retired_payload: None,
            preview: None,
            motion: None,
            commit: None,
            sale: None,
        });
        Ok(())
    }
    pub fn finish_move_tool_cleanup(&mut self) -> Result<()> {
        let state = self
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("move tool absent")?;
        state.validate(self)?;
        ensure!(state.complete(), "move tool has not returned");
        self.host.presentation.move_tool.as_mut().unwrap().call = None;
        Ok(())
    }
    ///A validated476a35/4ce44f caller starts the same concrete Move dispatch.
    ///The publication owns the ordinary-input barrier throughout this child.
    pub(super) fn begin_current_move(
        &mut self,
        owner: super::current_tool::MoveCaller,
    ) -> Result<()> {
        use super::current_tool::Operation;
        ensure!(
            !self.move_tool_pending() && !self.tool_selection_pending(),
            "current tool overlaps Move/interface work"
        );
        ensure!(
            self.host
                .presentation
                .publication
                .as_ref()
                .is_some_and(|p| p.tool_owner() == Some(owner.publication)),
            "current Move parent absent"
        );
        let state = self
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("Move absent")?;
        state.validate(self)?;
        let dispatch = match owner.operation {
            Operation::Preview => dispatch::Dispatch::returned(),
            Operation::Cleanup(kind) => dispatch::Dispatch::cleanup(kind),
            Operation::Activate(argument) => {
                ensure!(
                    state.groups.is_some()
                        && state.history.catalog.is_some()
                        && self.simulation.vm.world.globals.contains_key(&26),
                    "Move activation catalog/groups/global26 absent"
                );
                dispatch::Dispatch::activate(argument)
            }
        };
        let preview =
            (owner.operation == Operation::Preview).then(|| preview::Execution::new(owner.tool));
        self.host.presentation.move_tool.as_mut().unwrap().call = Some(Execution {
            button_owner: None,
            pickup: None,
            owner: None,
            current: Some(owner),
            tick: self.simulation.vm.tick,
            dispatch,
            child: None,
            retired_held: None,
            retired_payload: None,
            preview,
            motion: None,
            commit: None,
            sale: None,
        });
        Ok(())
    }
    ///Only input/window/control replies can come from the frontend. History,
    ///object lifetime and script work must execute through concrete children.
    pub fn acknowledge_move_tool_input(
        &mut self,
        request: cleanup::Request,
        reply: cleanup::Reply,
    ) -> Result<()> {
        ensure!(
            matches!(
                request,
                cleanup::Request::Key { .. }
                    | cleanup::Request::SetCursor { .. }
                    | cleanup::Request::ClearSelection(_)
            ),
            "move engine work cannot be externally acknowledged"
        );
        self.host
            .presentation
            .move_tool
            .as_ref()
            .context("move tool absent")?
            .validate(self)?;
        let state = self
            .host
            .presentation
            .move_tool
            .as_mut()
            .context("move tool absent")?;
        if state
            .call
            .as_ref()
            .and_then(|e| e.preview.as_ref())
            .and_then(|p| p.input(state))
            == Some(request)
        {
            let window = state.inputs.window;
            return state
                .call
                .as_mut()
                .unwrap()
                .preview
                .as_mut()
                .unwrap()
                .acknowledge(request, reply, window);
        }
        self.host
            .presentation
            .move_tool
            .as_mut()
            .unwrap()
            .call
            .as_mut()
            .context("move caller absent")?
            .dispatch
            .acknowledge(dispatch::Service::External(request), reply)
            .map_err(anyhow::Error::msg)
    }
    pub fn advance_move_tool(&mut self, budget: usize) -> Result<Advance> {
        ensure!(budget > 0, "move tool needs a positive budget");
        self.host
            .presentation
            .move_tool
            .as_ref()
            .context("move tool absent")?
            .validate(self)?;
        let mut state = self.host.presentation.move_tool.take().unwrap();
        let result = self.advance_move_tool_inner(&mut state, budget);
        self.host.presentation.move_tool = Some(state);
        self.host
            .presentation
            .move_tool
            .as_ref()
            .unwrap()
            .validate(self)?;
        result
    }
    pub(super) fn move_tool_assets(&mut self) -> Result<()> {
        let camera = self
            .host
            .presentation
            .camera
            .context("move tool camera absent")?;
        if self.simulation.pending_removal_graphic().is_some() {
            self.service_removal_graphic(camera)?;
        }
        self.service_cursor_appearance()?;
        self.service_cursor_release_animator()?;
        self.service_selection_assets(camera, 1000)?;
        self.host.service(&mut self.simulation, &mut self.program)?;
        if self.simulation.pending_cursor_cleanup_focus() {
            self.service_cursor_cleanup_focus()?;
        }
        Ok(())
    }
    fn advance_move_tool_inner(&mut self, state: &mut State, budget: usize) -> Result<Advance> {
        if state
            .call
            .as_ref()
            .is_some_and(|e| e.sale.is_some() && e.dispatch.complete() && e.child.is_none())
        {
            let mut sale = state.call.as_mut().unwrap().sale.take().unwrap();
            let result = sale.advance(self, state);
            state.call.as_mut().unwrap().sale = Some(sale);
            return Ok(Advance {
                progress: result?,
                instructions: 0,
            });
        }
        if state
            .call
            .as_ref()
            .is_some_and(|e| e.pickup.is_some() && e.dispatch.complete() && e.child.is_none())
        {
            let mut pickup = state.call.as_mut().unwrap().pickup.take().unwrap();
            let result = pickup.advance(self, state);
            state.call.as_mut().unwrap().pickup = Some(pickup);
            return Ok(Advance {
                progress: result?,
                instructions: 0,
            });
        }
        if state
            .call
            .as_ref()
            .is_some_and(|e| e.commit.is_some() && e.dispatch.complete() && e.child.is_none())
        {
            let mut commit = state.call.as_mut().unwrap().commit.take().unwrap();
            let result = commit.advance(self, state);
            state.call.as_mut().unwrap().commit = Some(commit);
            return result;
        }
        if state
            .call
            .as_ref()
            .is_some_and(|e| e.preview.is_some() && e.dispatch.complete() && e.child.is_none())
        {
            let mut preview = state.call.as_mut().unwrap().preview.take().unwrap();
            let result = preview.advance(self, state);
            state.call.as_mut().unwrap().preview = Some(preview);
            return Ok(Advance {
                progress: result?,
                instructions: 0,
            });
        }
        if state
            .call
            .as_ref()
            .is_some_and(|e| e.motion.is_some() && e.dispatch.complete() && e.child.is_none())
        {
            let mut motion = state.call.as_mut().unwrap().motion.take().unwrap();
            let result = motion.advance(self, state);
            state.call.as_mut().unwrap().motion = Some(motion);
            return Ok(Advance {
                progress: result?,
                instructions: 0,
            });
        }
        let e = state.call.as_mut().context("move caller absent")?;
        let mut instructions = 0;
        for _ in 0..budget {
            if let Some(child) = &mut e.child {
                let done = match child {
                    Child::Detach(_) => {
                        instructions += self
                            .simulation
                            .advance_cursor_detach(&self.program, 1)
                            .map_err(anyhow::Error::msg)?
                            .trace
                            .len();
                        self.move_tool_assets()?;
                        if self.simulation.cursor_rotation.as_ref().unwrap().complete() {
                            self.simulation
                                .finish_cursor_detach(&self.program)
                                .map_err(anyhow::Error::msg)?;
                            true
                        } else {
                            false
                        }
                    }
                    Child::Forget(_) => {
                        instructions += self
                            .simulation
                            .advance_cursor_cleanup(&self.program, 1)
                            .map_err(anyhow::Error::msg)?
                            .trace
                            .len();
                        self.move_tool_assets()?;
                        if self.simulation.cursor_cleanup.as_ref().unwrap().complete() {
                            self.simulation
                                .finish_cursor_cleanup(&self.program)
                                .map_err(anyhow::Error::msg)?;
                            true
                        } else {
                            false
                        }
                    }
                    Child::Delete { id, object } => {
                        let mut step = sims_sim::Step {
                            stop: sims_sim::StepStop::Budget,
                            trace: vec![],
                            events: vec![],
                        };
                        let done = self
                            .simulation
                            .drive_synchronous_removal(&self.program, &mut 1, &mut step)
                            .map_err(anyhow::Error::msg)?;
                        instructions += step.trace.len();
                        self.move_tool_assets()?;
                        if done {
                            ensure!(
                                self.simulation
                                    .take_synchronous_removal()
                                    .map_err(anyhow::Error::msg)?
                                    == *id
                            );
                            e.retired_held = Some((*object, *id));
                        }
                        done
                    }
                    Child::Kind { call, .. } => call
                        .advance(&state.history.store, &mut state.history.objects, 1)
                        .map_err(anyhow::Error::msg)?,
                    Child::History(call) => {
                        let progress =
                            call.advance(self, &mut state.history, state.inputs.registry)?;
                        //The history budget temporarily writes the very same
                        //native+24 word, then restores it after its DA child.
                        if let Some(f) = &state.preview_fields {
                            state.tool.tool.cursor = state.history.budgets.0[&f.budget].category;
                        }
                        match progress {
                            history::Step::Pending(n) => {
                                instructions += n;
                                false
                            }
                            history::Step::Complete => {
                                e.retired_payload = call.retired_payload();
                                true
                            }
                            history::Step::Service(r) => {
                                return Ok(Advance {
                                    progress: Progress::Service(r),
                                    instructions,
                                });
                            }
                        }
                    }
                };
                if done {
                    let request = e.dispatch.service().context("move child caller absent")?;
                    e.dispatch
                        .acknowledge(request, cleanup::Reply::Done)
                        .map_err(anyhow::Error::msg)?;
                    e.child = None;
                }
                continue;
            }
            let mut live = Live {
                world: &self.simulation.vm.world,
                objects: &mut state.history.objects,
            };
            let request = if e.dispatch.requires_catalog() {
                //Root+1c points to the simulator; its globals start at+10.
                //The487a87 word at+44 is global26, reread after child work.
                let gate = *live
                    .world
                    .globals
                    .get(&26)
                    .context("move activation simulator global26 absent")?;
                e.dispatch.advance(
                    &mut state.tool,
                    &mut live,
                    state.inputs,
                    state
                        .groups
                        .as_ref()
                        .context("move activation groups absent")?,
                    state
                        .history
                        .catalog
                        .as_ref()
                        .context("move activation catalog absent")?,
                    gate,
                )
            } else {
                e.dispatch
                    .advance_cleanup(&mut state.tool, &mut live, state.inputs)
            }
            .map_err(anyhow::Error::msg)?;
            //Activation/classification and the budget owner project one
            //native+24 word. Publish its actual store before any child runs.
            if let Some(f) = &state.preview_fields {
                state
                    .history
                    .budgets
                    .0
                    .get_mut(&f.budget)
                    .context("Move budget owner absent")?
                    .category = state.tool.tool.cursor;
            }
            if state.tool.held.is_none() {
                e.retired_held = None;
            }
            if state.tool.payload.is_none() {
                e.retired_payload = None;
            }
            let Some(service) = request else {
                return Ok(Advance {
                    progress: if e.preview.is_some()
                        || e.motion.is_some()
                        || e.commit.is_some()
                        || e.pickup.is_some()
                        || e.sale.is_some()
                    {
                        Progress::Pending
                    } else {
                        Progress::Complete
                    },
                    instructions,
                });
            };
            match service {
                dispatch::Service::Flags(r) => {
                    let target =
                        id(&self.simulation.vm.world, r.object).map_err(anyhow::Error::msg)?;
                    self.simulation
                        .update_native_low_flags(&self.program, target, r.low)
                        .map_err(anyhow::Error::msg)?;
                    e.dispatch
                        .acknowledge(service, cleanup::Reply::Done)
                        .map_err(anyhow::Error::msg)?;
                }
                dispatch::Service::External(r) => {
                    if let Some(call) = history::Execution::new(r)? {
                        e.child = Some(Child::History(Box::new(call)));
                        continue;
                    }
                    match r {
                        cleanup::Request::ObjectMode(h) => {
                            self.simulation
                                .begin_cursor_detach(
                                    &self.program,
                                    id(&self.simulation.vm.world, h).map_err(anyhow::Error::msg)?,
                                )
                                .map_err(anyhow::Error::msg)?;
                            e.child = Some(Child::Detach(h));
                        }
                        cleanup::Request::DetachObject(h) => {
                            self.simulation
                                .begin_cursor_cleanup(
                                    &self.program,
                                    id(&self.simulation.vm.world, h).map_err(anyhow::Error::msg)?,
                                    sims_sim::lot_cursor::cleanup::Kind::Forget,
                                )
                                .map_err(anyhow::Error::msg)?;
                            e.child = Some(Child::Forget(h));
                        }
                        cleanup::Request::DeleteObject { registry, id } => {
                            ensure!(
                                registry == state.inputs.registry,
                                "move deletion registry differs"
                            );
                            if let Some(h) = self
                                .simulation
                                .vm
                                .world
                                .scheduler_storage
                                .as_ref()
                                .unwrap()
                                .handle(id)
                            {
                                self.simulation
                                    .begin_synchronous_removal(&self.program, id)
                                    .map_err(anyhow::Error::msg)?;
                                e.child = Some(Child::Delete { id, object: h });
                            } else {
                                e.dispatch
                                    .acknowledge(service, cleanup::Reply::Done)
                                    .map_err(anyhow::Error::msg)?;
                            }
                        }
                        cleanup::Request::PayloadKind { payload, value } => {
                            e.child = Some(Child::Kind {
                                payload,
                                value,
                                call: object::kind::Call::new(payload, value),
                            })
                        }
                        cleanup::Request::Key { .. }
                        | cleanup::Request::SetCursor { .. }
                        | cleanup::Request::ClearSelection(_) => {
                            return Ok(Advance {
                                progress: Progress::Service(Service::Input(r)),
                                instructions,
                            });
                        }
                        _ => {
                            anyhow::bail!("move dispatch emitted an unbound concrete child: {r:?}")
                        }
                    }
                }
            }
        }
        Ok(Advance {
            progress: Progress::Pending,
            instructions,
        })
    }
}
