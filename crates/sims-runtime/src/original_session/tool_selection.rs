//!Live4cefce/4cef9b/4ceff9/4cf360 composition. Tool callbacks retain their
//!actual receiver and caller; CPState fields remain owned by transport.
use super::{Session, move_tool};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::object_catalog::move_tool::{cleanup, selected::Masters};
use sims_vm::interface::{
    buy::{definition, input, product, tool as buy},
    mode_change, tool,
};
use std::collections::BTreeMap;
pub mod mode;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Receiver {
    Move,
    ///cObjPickerTool+3c is undefined until48e21b receives0 or1.
    Picker {
        mode: Option<i32>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoveOperation {
    Activate(u32),
    Cleanup(cleanup::Kind),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveCaller {
    serial: u64,
    tool: u32,
    operation: MoveOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Frame {
    CatalogProduct(product::Click),
    CatalogInput(input::Navigation),
    Mode(mode_change::Change),
    Change(tool::Change),
    Definition(definition::Selection),
    Buy(buy::Selection),
}
#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    CatalogProduct(product::Request),
    CatalogInput(input::Request),
    Mode(mode_change::Request),
    Tool(tool::Request),
    Definition(definition::Request),
    Guid(u32),
}
impl Frame {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::CatalogProduct(c) => c.validate(),
            Self::CatalogInput(c) => c.validate(),
            Self::Mode(c) => c.validate(),
            Self::Change(c) => c.validate(),
            Self::Definition(c) => c.validate(),
            Self::Buy(c) => c.validate(),
        }
    }
    fn nested(&self) -> Result<Option<Frame>, String> {
        Ok(match self {
            Self::CatalogProduct(c) => (c.request() == Some(product::Request::BuyTool))
                .then(|| Self::Buy(buy::Selection::new(3))),
            Self::CatalogInput(c) => match c.request() {
                Some(input::Request::Tool { index, argument }) => {
                    Some(Self::Change(tool::Change::select(index, argument)))
                }
                _ => None,
            },
            Self::Mode(c) => match c.request() {
                Some(mode_change::Request::ClearTool) => {
                    Some(Self::Change(tool::Change::replace(None, 0)?))
                }
                Some(mode_change::Request::SelectBuyTool(value)) => {
                    Some(Self::Buy(buy::Selection::new(*value)))
                }
                Some(mode_change::Request::SelectLiveTool) => {
                    Some(Self::Change(tool::Change::select(6, 0)))
                }
                _ => None,
            },
            Self::Definition(c) => match c.request() {
                Some(definition::Request::Replace { tool, argument }) => {
                    Some(Self::Change(tool::Change::replace(Some(tool), argument)?))
                }
                Some(definition::Request::SelectDefault) => {
                    Some(Self::Change(tool::Change::select(7, u32::MAX)))
                }
                _ => None,
            },
            Self::Buy(c) => match c.request() {
                Some(buy::Request::Clear) => Some(Self::Change(tool::Change::replace(None, 0)?)),
                Some(buy::Request::Select { index, argument }) => {
                    Some(Self::Change(tool::Change::select(index, argument)))
                }
                Some(buy::Request::SelectDefinition(guid)) => {
                    Some(Self::Definition(definition::Selection::new(guid)))
                }
                _ => None,
            },
            Self::Change(_) => None,
        })
    }
    fn accepts(&self, child: &Frame) -> bool {
        match (self, child) {
            (Self::CatalogProduct(p), Self::Buy(c)) => {
                p.request() == Some(product::Request::BuyTool) && c.selects(3)
            }
            (Self::CatalogInput(p), Self::Change(c)) => match p.request() {
                Some(input::Request::Tool { index, argument }) => c.selects(index, argument),
                _ => false,
            },
            (Self::Mode(p), Self::Change(c)) => match p.request() {
                Some(mode_change::Request::ClearTool) => c.replaces(None, 0),
                Some(mode_change::Request::SelectLiveTool) => c.selects(6, 0),
                _ => false,
            },
            (Self::Mode(p), Self::Buy(c)) => match p.request() {
                Some(mode_change::Request::SelectBuyTool(value)) => c.selects(*value),
                _ => false,
            },
            (Self::Definition(p), Self::Change(c)) => match p.request() {
                Some(definition::Request::Replace { tool, argument }) => {
                    c.replaces(Some(tool), argument)
                }
                Some(definition::Request::SelectDefault) => c.selects(7, u32::MAX),
                _ => false,
            },
            (Self::Buy(p), Self::Change(c)) => match p.request() {
                Some(buy::Request::Clear) => c.replaces(None, 0),
                Some(buy::Request::Select { index, argument }) => c.selects(index, argument),
                _ => false,
            },
            (Self::Buy(p), Self::Definition(c)) => {
                p.request() == Some(buy::Request::SelectDefinition(c.guid()))
            }
            _ => false,
        }
    }
    fn child_returned(&mut self) -> Result<(), String> {
        match self {
            Self::CatalogProduct(c) => {
                c.acknowledge(product::Request::BuyTool, product::Reply::Done)
            }
            Self::CatalogInput(c) => {
                c.acknowledge(c.request().ok_or("catalog input child lost caller")?)
            }
            Self::Mode(c) => c.acknowledge(&c.request().cloned().ok_or("mode child lost caller")?),
            Self::Definition(c) => c.acknowledge(
                c.request().ok_or("definition child lost caller")?,
                definition::Reply::Done,
            ),
            Self::Buy(c) => c.acknowledge(
                c.request().ok_or("Buy child lost caller")?,
                buy::Reply::Done,
            ),
            Self::Change(_) => Err("tool change cannot own a dispatch frame".into()),
        }
    }
    fn action(&self) -> Option<Action> {
        match self {
            Self::CatalogProduct(c) => c.request().map(Action::CatalogProduct),
            Self::CatalogInput(c) => c.request().map(Action::CatalogInput),
            Self::Mode(c) => c.request().cloned().map(Action::Mode),
            Self::Change(c) => c.request().cloned().map(Action::Tool),
            Self::Definition(c) => c.request().map(Action::Definition),
            Self::Buy(c) => match c.request() {
                Some(buy::Request::Guid(id)) => Some(Action::Guid(id)),
                _ => None,
            },
        }
    }
    fn advance(
        &mut self,
        fields: &mut tool::Fields,
        controls: &mut sims_vm::interface::state::State,
    ) -> Result<bool, String> {
        Ok(match self {
            Self::CatalogProduct(c) => c.advance(&mut controls.buy)?.is_some(),
            Self::CatalogInput(c) => c.advance(&mut controls.buy)?.is_some(),
            Self::Mode(_) => return Err("mode needs its live field owners".into()),
            Self::Change(c) => c.advance(fields)?.is_some(),
            Self::Definition(c) => c.advance(fields)?.is_some(),
            Self::Buy(c) => c.advance(controls)?.is_some(),
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Execution {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    button_owner: bool,
    tick: u64,
    frames: Vec<Frame>,
    child: Option<MoveCaller>,
    drawing: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    notice: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    window: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    slots: [Option<u32>; 12],
    receivers: BTreeMap<u32, Receiver>,
    fields: tool::Fields,
    ///Resident UI catalog-item+28 -> definition; never a script object ID.
    items: BTreeMap<u32, u32>,
    ///49290f initializes the tool's UI_error timestamp to zero. Historical
    ///frontends attach this previously unused feedback channel explicitly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_time: Option<u32>,
    ///Previously unused492ad4/492b05 channels. Explicit first sale attachment
    ///does not reset the shared492a72 error timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sale_times: Option<[u32; 2]>,
    pub(super) masters: Masters,
    serial: u64,
    call: Option<Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode: Option<mode::Context>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Service {
    CatalogProduct(product::Request),
    CatalogInput(input::Request),
    DrawingMode(u8),
    Move(move_tool::Service),
    Mode(mode_change::Request),
    Notification(super::notifications::Service),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Complete,
    Service(Service),
}

impl State {
    pub(super) fn bind_sale_feedback(&mut self) {
        self.error_time.get_or_insert(0);
        self.sale_times.get_or_insert([0; 2]);
    }
    pub(super) fn feedback_time_mut(
        &mut self,
        kind: sims_sim::object_catalog::move_tool::sell::Feedback,
    ) -> Result<&mut u32> {
        use sims_sim::object_catalog::move_tool::sell::Feedback;
        match kind {
            Feedback::Blocked => self
                .error_time
                .as_mut()
                .context("tool error timestamp unbound"),
            Feedback::MoneyBack => Ok(&mut self
                .sale_times
                .as_mut()
                .context("tool sale timestamps unbound")?[0]),
            Feedback::Delete => Ok(&mut self
                .sale_times
                .as_mut()
                .context("tool sale timestamps unbound")?[1]),
        }
    }
    pub(super) fn button_owned(&self) -> bool {
        self.call.as_ref().is_some_and(|c| c.button_owner)
    }
    pub(super) fn claim_button(&mut self) -> Result<()> {
        let c = self.call.as_mut().context("button selector absent")?;
        ensure!(
            matches!(c.frames.as_slice(),[Frame::Buy(b)] if b.selects(3)),
            "button requires selector3"
        );
        c.button_owner = true;
        Ok(())
    }
    pub(super) fn receiver_mut(&mut self, id: u32) -> Result<&mut Receiver> {
        self.receivers.get_mut(&id).context("tool receiver absent")
    }
    ///Explicit application binding, not an import of an original UI stack.
    pub fn new(
        slots: [Option<u32>; 12],
        receivers: BTreeMap<u32, Receiver>,
        fields: tool::Fields,
    ) -> Self {
        Self {
            slots,
            receivers,
            fields,
            items: Default::default(),
            error_time: Some(0),
            sale_times: Some([0; 2]),
            masters: Default::default(),
            serial: 0,
            call: None,
            mode: None,
        }
    }
    pub fn fields(&self) -> &tool::Fields {
        &self.fields
    }
    pub fn receivers(&self) -> &BTreeMap<u32, Receiver> {
        &self.receivers
    }
    pub fn pending(&self) -> bool {
        self.call.is_some()
    }
    pub fn complete(&self) -> bool {
        self.call.as_ref().is_some_and(|c| {
            c.frames.is_empty()
                && c.child.is_none()
                && c.drawing.is_none()
                && !c.notice
                && c.window.is_none()
        })
    }
    pub fn retain_catalog_item(&mut self, item: u32, definition: u32) -> Result<()> {
        ensure!(
            !self.pending() && item != 0 && definition != 0 && !self.items.contains_key(&item),
            "catalog item already retained or null"
        );
        self.items.insert(item, definition);
        Ok(())
    }
    fn move_operation(&self, a: &Action) -> Option<(u32, MoveOperation)> {
        let (id, operation) = match a {
            Action::Tool(tool::Request::Activate { tool, argument }) => {
                (*tool, MoveOperation::Activate(*argument))
            }
            Action::Tool(tool::Request::Deactivate(tool)) => {
                (*tool, MoveOperation::Cleanup(cleanup::Kind::Deactivate))
            }
            Action::Definition(definition::Request::Reset(tool)) => {
                (*tool, MoveOperation::Cleanup(cleanup::Kind::Reset))
            }
            _ => return None,
        };
        matches!(self.receivers.get(&id), Some(Receiver::Move)).then_some((id, operation))
    }
    pub(super) fn validate(&self, s: &Session) -> Result<()> {
        let m = s
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("interface move tool absent")?;
        let catalog = m.catalog()?;
        ensure!(
            s.host.presentation.transport.is_some(),
            "interface CPState owner absent"
        );
        ensure!(
            !self.receivers.contains_key(&0)
                && self
                    .slots
                    .iter()
                    .flatten()
                    .all(|id| self.receivers.contains_key(id)),
            "tool table has unbound receivers"
        );
        ensure!(
            self.fields
                .active
                .is_none_or(|id| self.receivers.contains_key(&id)),
            "active tool receiver absent"
        );
        ensure!(
            self.receivers
                .values()
                .filter(|r| matches!(r, Receiver::Move))
                .count()
                == 1,
            "interface requires one concrete move receiver"
        );
        ensure!(
            self.slots[7].is_none_or(|id| matches!(self.receivers[&id], Receiver::Move))
                && self.slots[6]
                    .is_none_or(|id| matches!(self.receivers[&id], Receiver::Picker { .. })),
            "native tool slot class differs"
        );
        for (&id, &d) in &self.items {
            ensure!(
                id != 0 && d != 0 && catalog.entries.get(d as usize - 1).is_some(),
                "catalog item lost definition"
            );
        }
        for (&id, &d) in &self.masters.0 {
            ensure!(
                id != 0
                    && d != 0
                    && catalog.entries.get(id as usize - 1).is_some()
                    && catalog.entries.get(d as usize - 1).is_some(),
                "master cache lost definition"
            );
        }
        let Some(c) = &self.call else { return Ok(()) };
        ensure!(
            !c.button_owner
                || c.frames.is_empty()
                || matches!(c.frames.first(),Some(Frame::Buy(b)) if b.selects(3)),
            "button selector lost its root"
        );
        ensure!(
            c.tick == s.simulation.vm.tick && c.frames.len() <= 4,
            "tool selection lost clock/depth"
        );
        ensure!(
            !s.ground_input_pending()
                && !s.object_input_pending()
                && !s.transport_pending()
                && !s.clock_pending()
                && !s.interface_update_pending()
                && !s.camera_command_pending()
                && !s.control_panel_pending()
                && !s.ui_wait_pending()
                && !s.ui_portrait_pending()
                && !s.postload_pending()
                && !s.load_completion_pending()
                && !s.view_refresh_pending(),
            "tool selection overlaps application work"
        );
        for (n, f) in c.frames.iter().enumerate() {
            f.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                !matches!(f, Frame::CatalogInput(_) | Frame::CatalogProduct(_)) || n == 0,
                "catalog input must own the outer tool call"
            );
            ensure!(
                !matches!(f, Frame::Mode(_)) || (n == 0 && self.mode.is_some()),
                "mode transition lost its context/root"
            );
            if let Some(child) = c.frames.get(n + 1) {
                ensure!(f.accepts(child), "tool selection child lost caller");
            } else {
                ensure!(
                    f.nested().map_err(anyhow::Error::msg)?.is_none(),
                    "tool selection lost concrete child"
                );
            }
        }
        let action = c.frames.last().and_then(Frame::action);
        if let Some(mode) = &self.mode {
            mode.validate_close(s, self.mode_request())?;
        }
        ensure!(
            m.pending() == c.child.is_some(),
            "tool selection move callback differs"
        );
        if let Some(child) = &c.child {
            ensure!(
                child.serial != 0
                    && child.serial == self.serial
                    && m.selection_owner() == Some(child)
                    && action.as_ref().and_then(|a| self.move_operation(a))
                        == Some((child.tool, child.operation.clone()))
                    && c.drawing.is_none(),
                "tool selection lost exact move callback"
            );
        }
        if let Some(id) = c.drawing {
            ensure!(
                action == Some(Action::Tool(tool::Request::Deactivate(id)))
                    && matches!(self.receivers.get(&id), Some(Receiver::Picker { .. })),
                "drawing write lost picker caller"
            );
        }
        if c.notice {
            ensure!(
                action == Some(Action::Mode(mode_change::Request::Notify))
                    && c.child.is_none()
                    && c.drawing.is_none()
                    && c.window.is_none()
                    && s.host
                        .presentation
                        .notifications
                        .as_ref()
                        .is_some_and(|n| n.dispatch.root_message() == Some((0x91, 0))),
                "mode notification lost its caller"
            );
        }
        if let Some(serial) = c.window {
            ensure!(
                serial != 0
                    && serial == self.serial
                    && matches!(
                        action,
                        Some(Action::Mode(mode_change::Request::WindowPause { .. }))
                    )
                    && c.child.is_none()
                    && c.drawing.is_none()
                    && !c.notice,
                "mode window lost its exact caller"
            );
        }
        Ok(())
    }
}
impl Session {
    /// 4cee3d reads CPState+4c, not the global current-tool pointer. The
    /// picker inherits empty50c25d; the Move override is487241. This callback
    /// has no suspendable work for ordinary furniture and does not publish UI.
    pub(super) fn selected_tool_camera_changed(&mut self) -> Result<()> {
        let Some(selection) = self.host.presentation.tool_selection.as_ref() else {
            if let Some(inputs) = &self.host.presentation.retained_tool_inputs {
                ensure!(
                    inputs.selected_picker_or_null(),
                    "retained selected-tool camera callback requires its native receiver"
                );
                // Actual667f1c+3c is50c25d, the empty Picker override.
                return Ok(());
            }
            // Frontends that have not installed the indexed tools have no
            // selected CPState tool. Do not infer one from the global picker.
            ensure!(
                self.host.presentation.move_tool.is_none(),
                "camera selected-tool owner absent"
            );
            return Ok(());
        };
        let Some(id) = selection.fields.active else {
            return Ok(());
        };
        match selection
            .receivers
            .get(&id)
            .context("camera selected-tool receiver absent")?
        {
            Receiver::Picker { .. } => Ok(()),
            Receiver::Move => {
                // The native override reads the viewer's zoom and dimensions
                // before testing held+3c. Validate those actual inputs first.
                self.retained_camera_input()?;
                let m = self
                    .host
                    .presentation
                    .move_tool
                    .as_mut()
                    .context("camera Move receiver absent")?;
                ensure!(!m.pending(), "camera callback overlaps Move execution");
                m.tool
                    .tool
                    .camera_changed(m.tool.held.is_some())
                    .map_err(anyhow::Error::msg)
            }
        }
    }

    pub fn tool_selection_pending(&self) -> bool {
        self.host
            .presentation
            .tool_selection
            .as_ref()
            .is_some_and(State::pending)
    }
    pub(super) fn validate_tool_selection_owner(&self) -> Result<()> {
        if let Some(state) = &self.host.presentation.tool_selection {
            state.validate(self)?;
        }
        if let Some(owner) = self
            .host
            .presentation
            .move_tool
            .as_ref()
            .and_then(move_tool::State::selection_owner)
        {
            ensure!(
                self.host
                    .presentation
                    .tool_selection
                    .as_ref()
                    .and_then(|s| s.call.as_ref())
                    .and_then(|c| c.child.as_ref())
                    == Some(owner),
                "move callback lost its interface parent"
            );
        }
        Ok(())
    }
    pub fn bind_tool_selection(&mut self, state: State) -> Result<()> {
        ensure!(
            self.host.presentation.retained_tool_inputs.is_none(),
            "constructed indexed tools cannot replace retained process inputs"
        );
        self.check_move_tool_entry()?;
        ensure!(
            self.host.presentation.tool_selection.is_none() && !state.pending(),
            "tool interface already bound or running"
        );
        state.validate(self)?;
        self.host.presentation.tool_selection = Some(state);
        Ok(())
    }
    fn begin_tool_frame(&mut self, frame: Frame) -> Result<()> {
        self.check_move_tool_entry()?;
        self.validate_tool_selection_owner()?;
        let state = self
            .host
            .presentation
            .tool_selection
            .as_mut()
            .context("tool interface absent")?;
        state.call = Some(Execution {
            button_owner: false,
            tick: self.simulation.vm.tick,
            frames: vec![frame],
            child: None,
            drawing: None,
            notice: false,
            window: None,
        });
        Ok(())
    }
    pub fn begin_indexed_tool(&mut self, index: i32, argument: u32) -> Result<()> {
        self.begin_tool_frame(Frame::Change(tool::Change::select(index, argument)))
    }
    pub fn begin_buy_product(&mut self, item: u32, definition: u32) -> Result<()> {
        self.check_move_tool_entry()?;
        self.validate_tool_selection_owner()?;
        ensure!(
            !self.publication_pending() && item != 0 && definition != 0,
            "Buy product overlaps publication or has null identity"
        );
        ensure!(
            self.host
                .presentation
                .transport
                .as_ref()
                .is_some_and(|t| t.controls.mode == 1),
            "Buy product outside Buy mode"
        );
        ensure!(
            self.host
                .presentation
                .move_tool
                .as_ref()
                .unwrap()
                .catalog()?
                .entries
                .get(definition as usize - 1)
                .is_some(),
            "Buy product definition absent"
        );
        let state = self
            .host
            .presentation
            .tool_selection
            .as_mut()
            .context("Buy tool owner absent")?;
        if let Some(existing) = state.items.get(&item) {
            ensure!(
                *existing == definition,
                "resident Buy item changed definition"
            );
        } else {
            state.retain_catalog_item(item, definition)?;
        }
        // Explicit first attachment of the previously unused catalog error
        // channel; the old frontend could not issue product-click feedback.
        state.error_time.get_or_insert(0);
        self.begin_tool_frame(Frame::CatalogProduct(product::Click::new(item)))
    }
    pub fn acknowledge_buy_product(
        &mut self,
        request: product::Request,
        reply: product::Reply,
    ) -> Result<()> {
        self.validate_tool_selection_owner()?;
        ensure!(
            request != product::Request::BuyTool,
            "Buy tool callback requires its real child"
        );
        let state = self
            .host
            .presentation
            .tool_selection
            .as_mut()
            .context("Buy product owner absent")?;
        let Some(Frame::CatalogProduct(c)) = state.call.as_mut().and_then(|c| c.frames.last_mut())
        else {
            anyhow::bail!("Buy product caller absent")
        };
        c.acknowledge(request, reply).map_err(anyhow::Error::msg)
    }
    ///492a72 on tool7, on this frontend's explicitly disabled-audio profile.
    ///The original57a54c early return still precedes the timestamp store.
    pub fn buy_error_feedback_disabled_audio(&mut self) -> Result<()> {
        self.validate_tool_selection_owner()?;
        let now = self
            .host
            .presentation
            .presenter
            .as_ref()
            .and_then(|p| p.camera.as_ref())
            .context("Buy feedback application clock absent")?
            .settings
            .clock;
        let state = self
            .host
            .presentation
            .tool_selection
            .as_mut()
            .context("Buy feedback tool absent")?;
        ensure!(
            state
                .call
                .as_ref()
                .and_then(|c| c.frames.last())
                .and_then(Frame::action)
                == Some(Action::CatalogProduct(product::Request::ErrorFeedback)),
            "Buy feedback caller differs"
        );
        let tool = tool::lookup(&state.slots, 7).context("Buy feedback tool7 absent")?;
        ensure!(
            matches!(state.receivers[&tool], Receiver::Move),
            "Buy feedback receiver differs"
        );
        let last = state
            .error_time
            .as_mut()
            .context("Buy feedback timestamp unbound")?;
        if last.wrapping_add(400) < now {
            *last = now;
        }
        Ok(())
    }
    pub fn begin_buy_navigation(&mut self, event: input::Event) -> Result<()> {
        ensure!(
            !self.publication_pending(),
            "Buy input overlaps publication"
        );
        ensure!(
            self.host
                .presentation
                .transport
                .as_ref()
                .is_some_and(|t| t.controls.mode == 1),
            "Buy input outside Buy mode"
        );
        self.begin_tool_frame(Frame::CatalogInput(
            input::Navigation::new(event).map_err(anyhow::Error::msg)?,
        ))
    }
    pub fn acknowledge_buy_input(&mut self, request: input::Request) -> Result<()> {
        self.validate_tool_selection_owner()?;
        ensure!(
            matches!(request, input::Request::CategorySelected(_)),
            "catalog tool return needs its real child"
        );
        let Some(Frame::CatalogInput(c)) = self
            .host
            .presentation
            .tool_selection
            .as_mut()
            .and_then(|s| s.call.as_mut())
            .and_then(|c| c.frames.last_mut())
        else {
            anyhow::bail!("catalog input owner absent")
        };
        c.acknowledge(request).map_err(anyhow::Error::msg)
    }
    pub fn begin_tool_replacement(&mut self, tool: Option<u32>, argument: u32) -> Result<()> {
        self.begin_tool_frame(Frame::Change(
            tool::Change::replace(tool, argument).map_err(anyhow::Error::msg)?,
        ))
    }
    pub fn begin_buy_definition(&mut self, guid: u32) -> Result<()> {
        self.begin_tool_frame(Frame::Definition(definition::Selection::new(guid)))
    }
    pub fn begin_buy_tool(&mut self, value: i32) -> Result<()> {
        self.begin_tool_frame(Frame::Buy(buy::Selection::new(value)))
    }
    pub fn finish_tool_selection(&mut self) -> Result<()> {
        self.validate_tool_selection_owner()?;
        let state = self
            .host
            .presentation
            .tool_selection
            .as_mut()
            .context("tool interface absent")?;
        ensure!(state.complete(), "tool interface has not returned");
        state.call = None;
        Ok(())
    }
    pub fn acknowledge_tool_drawing(&mut self, value: u8) -> Result<()> {
        self.validate_tool_selection_owner()?;
        ensure!(
            value == tool::PICKER_DEACTIVATED_DRAWING_MODE,
            "picker drawing reply differs"
        );
        let call = self
            .host
            .presentation
            .tool_selection
            .as_mut()
            .and_then(|s| s.call.as_mut())
            .context("drawing caller absent")?;
        let id = call.drawing.context("drawing write absent")?;
        let Some(Frame::Change(c)) = call.frames.last_mut() else {
            anyhow::bail!("drawing frame absent")
        };
        c.acknowledge(&tool::Request::Deactivate(id), tool::Reply::Done)
            .map_err(anyhow::Error::msg)?;
        call.drawing = None;
        Ok(())
    }
    pub fn advance_tool_selection(&mut self, budget: usize) -> Result<Progress> {
        ensure!(budget > 0, "tool interface needs a positive budget");
        self.validate_tool_selection_owner()?;
        let mut state = self
            .host
            .presentation
            .tool_selection
            .take()
            .context("tool interface absent")?;
        let result = self.advance_tool_selection_inner(&mut state, budget);
        self.host.presentation.tool_selection = Some(state);
        self.validate_tool_selection_owner()?;
        result
    }
    fn advance_tool_selection_inner(
        &mut self,
        state: &mut State,
        budget: usize,
    ) -> Result<Progress> {
        for _ in 0..budget {
            let c = state.call.as_mut().context("tool caller absent")?;
            if c.drawing.is_some() {
                return Ok(Progress::Service(Service::DrawingMode(
                    tool::PICKER_DEACTIVATED_DRAWING_MODE,
                )));
            }
            if c.child.is_some() {
                match self.advance_move_tool(1)?.progress {
                    move_tool::Progress::Pending => continue,
                    move_tool::Progress::Service(r) => {
                        return Ok(Progress::Service(Service::Move(r)));
                    }
                    move_tool::Progress::Complete => {
                        self.finish_move_tool_cleanup()?;
                        match c.frames.last_mut().context("move caller frame absent")? {
                            Frame::Change(f) => f.acknowledge(
                                &f.request().cloned().context("move tool request absent")?,
                                tool::Reply::Done,
                            ),
                            Frame::Definition(f) => f.acknowledge(
                                f.request().context("move reset request absent")?,
                                definition::Reply::Done,
                            ),
                            Frame::Buy(_)
                            | Frame::Mode(_)
                            | Frame::CatalogInput(_)
                            | Frame::CatalogProduct(_) => {
                                Err("Buy selector cannot own move callback directly".into())
                            }
                        }
                        .map_err(anyhow::Error::msg)?;
                        c.child = None;
                        continue;
                    }
                }
            }
            if c.notice {
                match self.advance_house_notification(self.retained_camera_input()?, 1)? {
                    super::notifications::Progress::Pending => continue,
                    super::notifications::Progress::Service(r) => {
                        return Ok(Progress::Service(Service::Notification(r)));
                    }
                    super::notifications::Progress::Complete(_) => {
                        let Frame::Mode(f) =
                            c.frames.last_mut().context("mode notice parent absent")?
                        else {
                            anyhow::bail!("mode notice frame differs")
                        };
                        f.acknowledge(&mode_change::Request::Notify)
                            .map_err(anyhow::Error::msg)?;
                        c.notice = false;
                        continue;
                    }
                }
            }
            let Some(frame) = c.frames.last_mut() else {
                return Ok(Progress::Complete);
            };
            let pending = if let Frame::Mode(f) = frame {
                mode::advance(f, state.mode.as_mut().context("mode context absent")?, self)?
            } else {
                let controls = &mut self
                    .host
                    .presentation
                    .transport
                    .as_mut()
                    .context("CPState absent")?
                    .controls;
                frame
                    .advance(&mut state.fields, controls)
                    .map_err(anyhow::Error::msg)?
            };
            if !pending {
                c.frames.pop();
                if let Some(parent) = c.frames.last_mut() {
                    parent.child_returned().map_err(anyhow::Error::msg)?;
                }
                continue;
            }
            if let Some(child) = frame.nested().map_err(anyhow::Error::msg)? {
                c.frames.push(child);
                continue;
            }
            let action = frame.action().context("tool leaf request absent")?;
            if let Some((id, operation)) = state.move_operation(&action) {
                let serial = state
                    .serial
                    .checked_add(1)
                    .context("tool callback serial exhausted")?;
                match operation {
                    MoveOperation::Activate(a) => self.begin_move_tool_activation(a)?,
                    MoveOperation::Cleanup(k) => self.begin_move_tool_cleanup(k)?,
                }
                let owner = MoveCaller {
                    serial,
                    tool: id,
                    operation,
                };
                self.host
                    .presentation
                    .move_tool
                    .as_mut()
                    .unwrap()
                    .claim_selection(owner.clone())?;
                state.serial = serial;
                state.call.as_mut().unwrap().child = Some(owner);
                continue;
            }
            let c = state.call.as_mut().unwrap();
            match action {
                Action::CatalogProduct(request) => {
                    return Ok(Progress::Service(Service::CatalogProduct(request)));
                }
                Action::CatalogInput(request) => {
                    return Ok(Progress::Service(Service::CatalogInput(request)));
                }
                Action::Mode(request) => {
                    if let Some(progress) = self.service_mode_request(state, request)? {
                        return Ok(progress);
                    }
                }
                Action::Tool(r) => {
                    let reply = match r {
                        tool::Request::Lookup(index) => tool::Reply::Tool(
                            tool::lookup(&state.slots, index)
                                .context("indexed tool slot is null")?,
                        ),
                        tool::Request::Activate { tool: id, argument } => {
                            let Receiver::Picker { mode } = state
                                .receivers
                                .get_mut(&id)
                                .context("activated tool receiver absent")?
                            else {
                                anyhow::bail!("move callback bypassed driver")
                            };
                            tool::activate_picker(mode, argument);
                            tool::Reply::Done
                        }
                        tool::Request::Deactivate(id) => {
                            ensure!(
                                matches!(state.receivers.get(&id), Some(Receiver::Picker { .. })),
                                "deactivated tool receiver absent"
                            );
                            c.drawing = Some(id);
                            return Ok(Progress::Service(Service::DrawingMode(
                                tool::PICKER_DEACTIVATED_DRAWING_MODE,
                            )));
                        }
                    };
                    let Frame::Change(f) = c.frames.last_mut().unwrap() else {
                        unreachable!()
                    };
                    f.acknowledge(&r, reply).map_err(anyhow::Error::msg)?;
                }
                Action::Guid(item) => {
                    let id = *state
                        .items
                        .get(&item)
                        .context("selected catalog item is not resident")?;
                    let catalog = self
                        .host
                        .presentation
                        .move_tool
                        .as_ref()
                        .unwrap()
                        .catalog()?;
                    let entry = &catalog.entries[id as usize - 1];
                    let guid = u32::from_le_bytes(entry.data[28..32].try_into().unwrap());
                    let Frame::Buy(f) = c.frames.last_mut().unwrap() else {
                        unreachable!()
                    };
                    f.acknowledge(buy::Request::Guid(item), buy::Reply::Guid(guid))
                        .map_err(anyhow::Error::msg)?;
                }
                Action::Definition(r) => {
                    let m = self.host.presentation.move_tool.as_mut().unwrap();
                    let answer = match r {
                        definition::Request::Find(guid) => m
                            .catalog()?
                            .guids
                            .get(&guid)
                            .map_or(0, |row| *row as u32 + 1),
                        definition::Request::LookupTool => {
                            tool::lookup(&state.slots, 7).context("definition tool7 absent")?
                        }
                        definition::Request::Ordinal(id) => {
                            m.catalog()?
                                .entries
                                .get(id.checked_sub(1).context("null definition")? as usize)
                                .context("definition absent")?
                                .index
                        }
                        definition::Request::Classify {
                            tool: id,
                            definition,
                        } => {
                            ensure!(
                                matches!(state.receivers.get(&id), Some(Receiver::Move)),
                                "classification receiver differs"
                            );
                            m.classify_definition(&mut state.masters, definition)? as u32
                        }
                        _ => anyhow::bail!("unbound definition request: {r:?}"),
                    };
                    let Frame::Definition(f) = c.frames.last_mut().unwrap() else {
                        unreachable!()
                    };
                    f.acknowledge(r, definition::Reply::Number(answer))
                        .map_err(anyhow::Error::msg)?;
                }
            }
        }
        Ok(if state.complete() {
            Progress::Complete
        } else {
            Progress::Pending
        })
    }
}
