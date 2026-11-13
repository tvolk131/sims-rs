//!Actual476a35 owner and shared Move callbacks. The replacement household
//!viewer implements native51d372 at both tool-listener slots.
use super::{Session, move_tool, tool_selection::Receiver};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::object_catalog::move_tool::cleanup;
use sims_vm::interface::{current_tool as native, world_publication as world};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    source: String,
    language: u8,
    data: Vec<u8>,
}
impl Text {
    fn load(s: &Session) -> Result<Self> {
        //486e7f:415336(0,9) ->GameData/UIText;47028b reads STR#137.
        let resource = &s.index().resources[s.index().find("UIText.iff")?];
        let bytes = resource.read()?;
        let iff = sims_formats::iff::Iff::parse(bytes)?;
        let chunk = iff
            .get(b"STR#", 137)
            .context("Move UIText STR#137 absent")?;
        let data = iff.data(chunk).to_vec();
        sims_formats::strings::StringTable::parse(&data)?;
        Ok(Self {
            source: resource.locator.clone(),
            language: s
                .program
                .text_language
                .context("Move text language absent")?
                .try_into()?,
            data,
        })
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.source.is_empty() && self.language != 0,
            "Move text owner invalid"
        );
        sims_formats::strings::StringTable::validate(&self.data)?;
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    Cleanup(cleanup::Kind),
    Activate(u32),
    Preview,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveCaller {
    pub(super) publication: u64,
    pub(super) serial: u64,
    pub(super) tool: u32,
    pub(super) operation: Operation,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Execution {
    publication: u64,
    request: world::Request,
    selection: Option<native::Change>,
    child: Option<MoveCaller>,
    complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    picker_selection: Option<sims_vm::ObjectId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    current: Option<u32>,
    listeners: native::Listeners,
    move_text: Option<Text>,
    serial: u64,
    call: Option<Execution>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Service {
    Move(move_tool::Service),
    Notification(super::notifications::Service),
    ///Retain actual unsupported tool callbacks; never claim their return.
    Callback(native::Request),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Complete,
    Service(Service),
}

impl State {
    /// Retained identities supplied by a qualified completed-boundary importer.
    /// The Rust delivery epoch has not published a callback. Unlike the fresh
    /// application binder, this does not construct/register any listeners.
    pub(super) fn from_completed_inputs(
        current: Option<u32>,
        listeners: native::Listeners,
    ) -> Self {
        Self {
            current,
            listeners,
            move_text: None,
            serial: 0,
            call: None,
        }
    }
    pub fn current(&self) -> Option<u32> {
        self.current
    }
    pub fn request(&self) -> Option<native::Request> {
        self.call.as_ref()?.selection.as_ref()?.request()
    }
    pub fn move_text_loaded(&self) -> bool {
        self.move_text.is_some()
    }
    pub(super) fn owner(&self) -> Option<(u64, world::Request)> {
        self.call.as_ref().map(|c| (c.publication, c.request))
    }
    pub(super) fn move_owner(&self) -> Option<&MoveCaller> {
        self.call.as_ref()?.child.as_ref()
    }
    pub(super) fn picker_selection_owner(&self) -> Option<u64> {
        let call = self.call.as_ref()?;
        call.picker_selection.map(|_| call.publication)
    }
    fn validate(&self, s: &Session) -> Result<()> {
        ensure!(self.current != Some(0), "invalid current tool");
        //This frontend owns exactly one household viewer. Its two virtual
        //callbacks are the recovered ret4. It has no synthetic extra listeners.
        ensure!(
            self.listeners.head == Some(1) && self.listeners.next == [(1, None)].into(),
            "replacement tool listener differs"
        );
        ensure!(
            s.host.presentation.presenter.is_some(),
            "tool listener lost viewer"
        );
        if let Some(text) = &self.move_text {
            text.validate()?;
        }
        let Some(c) = &self.call else {
            return Ok(());
        };
        ensure!(c.publication != 0, "tool publication identity absent");
        match (&c.request, &c.selection) {
            (world::Request::SelectTool(target), Some(change)) => {
                change.validate().map_err(anyhow::Error::msg)?;
                ensure!(change.target() == *target, "current tool target differs");
                ensure!(
                    !c.complete || change.complete(),
                    "tool returned before selection"
                );
            }
            (world::Request::ActivateTool { tool: Some(id), .. }, None) if *id != 0 => (),
            _ => anyhow::bail!("current-tool caller differs"),
        }
        ensure!(
            !c.complete || c.child.is_none(),
            "returned tool retains Move child"
        );
        ensure!(
            s.move_tool_pending() == c.child.is_some()
                && (c.child.is_some()
                    || c.picker_selection.is_some()
                    || !s.house_notification_pending()),
            "current tool contains unowned Move/notification work"
        );
        if let Some(target) = c.picker_selection {
            ensure!(c.child.is_none()
                && matches!(c.selection.as_ref().and_then(native::Change::request),Some(native::Request::Enter(_)))
                && s.simulation.native_selection.as_ref().is_some_and(|e|e.target()==Some(target) && e.direct_factory().is_none()),
                "picker selection lost its native child");
        }
        if let Some(child) = &c.child {
            ensure!(
                child.serial != 0
                    && child.publication == c.publication
                    && child.serial == self.serial
                    && Some(child)
                        == s.host
                            .presentation
                            .move_tool
                            .as_ref()
                            .and_then(move_tool::State::current_owner),
                "current-tool Move child lost owner"
            );
            let expected = match c.selection.as_ref().and_then(native::Change::request) {
                Some(native::Request::Reset(id)) => {
                    Some((id, Operation::Cleanup(cleanup::Kind::Reset)))
                }
                Some(native::Request::Enter(id)) => Some((id, Operation::Preview)),
                None => match c.request {
                    world::Request::ActivateTool {
                        tool: Some(id),
                        argument_low,
                    } => Some((id, Operation::Activate(u32::from(argument_low)))),
                    _ => None,
                },
                _ => None,
            };
            ensure!(
                expected == Some((child.tool, child.operation)),
                "Move callback arguments differ"
            );
        }
        Ok(())
    }
}
impl Session {
    ///48c8e4 after the backend has applied492a22/4d2406(0). The ordinary
    ///selected-person branch ends here; missing selection drives445501 and
    ///the ordinary retained selection lifecycle before acknowledging Enter.
    pub fn acknowledge_picker_enter(&mut self, id: u32) -> Result<()> {
        self.validate_current_tool()?;
        ensure!(
            self.host
                .presentation
                .current_tool
                .as_ref()
                .and_then(State::request)
                == Some(native::Request::Enter(id)),
            "picker Enter does not match current-tool caller"
        );
        let picker = if let Some(tools) = &self.host.presentation.tool_selection {
            matches!(tools.receivers().get(&id), Some(Receiver::Picker { .. }))
        } else {
            self.host
                .presentation
                .retained_tool_inputs
                .as_ref()
                .is_some_and(|tools| tools.is_picker(id))
        };
        ensure!(picker, "current-tool Enter is not a picker");
        self.host.presentation.picker_activation_word = Some(0);
        if self
            .simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?
            .is_none()
        {
            ensure!(
                self.host.presentation.person_selection_override == Some(0),
                "picker cycling requires the explicit ordinary selection policy"
            );
            let people = self
                .simulation
                .vm
                .world
                .person
                .as_ref()
                .context("picker person registry absent")?;
            let members = people
                .families
                .get(&people.controlled_family)
                .context("picker family membership absent")?
                .iter()
                .map(|id| {
                    people
                        .neighbors
                        .get(id)
                        .map(|n| n.guid)
                        .context("picker family neighbor absent")
                })
                .collect::<Result<Vec<_>>>()?;
            let target = sims_vm::selection::cycle_family(None, &members, |guid| {
                people.person_order.iter().copied().find(|id| {
                    self.simulation
                        .vm
                        .world
                        .objects
                        .get(id)
                        .and_then(|o| o.definition.as_ref())
                        .is_some_and(|d| d.guid == guid)
                })
            });
            if let Some(target) = target {
                self.simulation
                    .begin_native_selection(Some(target), true)
                    .map_err(anyhow::Error::msg)?;
                self.host
                    .presentation
                    .current_tool
                    .as_mut()
                    .unwrap()
                    .call
                    .as_mut()
                    .unwrap()
                    .picker_selection = Some(target);
                return Ok(());
            }
        }
        self.host
            .presentation
            .current_tool
            .as_mut()
            .unwrap()
            .call
            .as_mut()
            .unwrap()
            .selection
            .as_mut()
            .unwrap()
            .acknowledge(native::Request::Enter(id))
            .map_err(anyhow::Error::msg)
    }
    pub(super) fn load_move_text(&mut self) -> Result<()> {
        if self
            .host
            .presentation
            .current_tool
            .as_ref()
            .context("current tools absent")?
            .move_text
            .is_none()
        {
            let text = Text::load(self)?;
            self.host
                .presentation
                .current_tool
                .as_mut()
                .unwrap()
                .move_text = Some(text);
        }
        Ok(())
    }
    ///Explicit replacement process binding, never inferred from an original
    ///house file. Allowed at an unstarted SelectTool boundary when restoring.
    pub fn bind_current_tools(&mut self, current: Option<u32>) -> Result<()> {
        ensure!(
            self.host.presentation.current_tool.is_none()
                && !self.move_tool_pending()
                && !self.tool_selection_pending(),
            "current tools already bound or busy"
        );
        if let Some(p) = self
            .host
            .presentation
            .publication
            .as_ref()
            .filter(|p| p.pending())
        {
            ensure!(
                matches!(p.request(), Some(world::Request::SelectTool(_)))
                    && p.tool_owner().is_none()
                    && p.context_current_tool() == current,
                "current tools cannot bind inside another publication child"
            );
        }
        let mut listeners = native::Listeners::default();
        listeners.construct(1).map_err(anyhow::Error::msg)?;
        listeners.register(1).map_err(anyhow::Error::msg)?;
        let state = State {
            current,
            listeners,
            move_text: None,
            serial: 0,
            call: None,
        };
        state.validate(self)?;
        self.host.presentation.current_tool = Some(state);
        Ok(())
    }
    pub(super) fn validate_current_tool(&self) -> Result<()> {
        if let Some(s) = &self.host.presentation.current_tool {
            s.validate(self)?;
            if let Some((owner, request)) = s.owner() {
                ensure!(
                    self.host
                        .presentation
                        .publication
                        .as_ref()
                        .is_some_and(
                            |p| p.tool_owner() == Some(owner) && p.request() == Some(request)
                        ),
                    "current tool lost publication parent"
                );
            }
        }
        if let Some(owner) = self
            .host
            .presentation
            .move_tool
            .as_ref()
            .and_then(move_tool::State::current_owner)
        {
            ensure!(
                self.host
                    .presentation
                    .current_tool
                    .as_ref()
                    .and_then(State::move_owner)
                    == Some(owner),
                "Move callback lost current tool"
            );
        }
        Ok(())
    }
    pub(super) fn begin_current_tool(
        &mut self,
        publication: u64,
        request: world::Request,
    ) -> Result<()> {
        let state = self
            .host
            .presentation
            .current_tool
            .as_mut()
            .context("current tools unbound")?;
        ensure!(state.call.is_none(), "current tool already running");
        let selection = match request {
            world::Request::SelectTool(target) => {
                Some(native::Change::new(target).map_err(anyhow::Error::msg)?)
            }
            world::Request::ActivateTool { tool: Some(_), .. } => None,
            _ => anyhow::bail!("unrelated current-tool request"),
        };
        state.call = Some(Execution {
            publication,
            request,
            selection,
            child: None,
            complete: false,
            picker_selection: None,
        });
        Ok(())
    }
    pub(super) fn finish_current_tool(&mut self) -> Result<()> {
        self.validate_current_tool()?;
        let s = self.host.presentation.current_tool.as_mut().unwrap();
        ensure!(
            s.call.as_ref().is_some_and(|c| c.complete),
            "current tool has not returned"
        );
        s.call = None;
        Ok(())
    }
    pub(super) fn advance_current_tool(&mut self) -> Result<Progress> {
        self.validate_current_tool()?;
        let mut s = self
            .host
            .presentation
            .current_tool
            .take()
            .context("current tools absent")?;
        let result = self.advance_current_tool_inner(&mut s);
        self.host.presentation.current_tool = Some(s);
        self.validate_current_tool()?;
        result
    }
    fn advance_current_tool_inner(&mut self, s: &mut State) -> Result<Progress> {
        let c = s.call.as_mut().context("current tool call absent")?;
        if c.complete {
            return Ok(Progress::Complete);
        }
        if c.picker_selection.is_some() {
            if self.house_notification_pending() {
                return Ok(
                    match self.advance_house_notification(self.retained_camera_input()?, 1)? {
                        super::notifications::Progress::Service(service) => {
                            Progress::Service(Service::Notification(service))
                        }
                        _ => Progress::Pending,
                    },
                );
            }
            let ordinary = self.host.ordinary.graphics.keys().copied().collect();
            self.simulation
                .advance_native_selection(&self.program, &ordinary, 1)
                .map_err(anyhow::Error::msg)?;
            if self.simulation.native_selection.is_none() {
                let selection = c.selection.as_mut().context("picker Enter parent absent")?;
                selection
                    .acknowledge(selection.request().context("picker Enter request absent")?)
                    .map_err(anyhow::Error::msg)?;
                c.picker_selection = None;
            } else if self
                .simulation
                .pending_selection_notification()
                .is_some_and(|(_, started)| !started)
            {
                self.begin_selection_broadcast()?;
            } else if let Some((
                sims_sim::lot_selection::Request::CreationNotification(id),
                false,
            )) = self.simulation.pending_native_selection()
            {
                self.begin_factory_broadcast(id)?;
            } else {
                self.service_selection_assets(self.retained_camera_input()?, 1)?;
            }
            return Ok(Progress::Pending);
        }
        if c.child.is_some() {
            return Ok(match self.advance_move_tool(1)?.progress {
                move_tool::Progress::Pending => Progress::Pending,
                move_tool::Progress::Service(r) => Progress::Service(Service::Move(r)),
                move_tool::Progress::Complete => {
                    self.finish_move_tool_cleanup()?;
                    c.child = None;
                    if let Some(selection) = &mut c.selection {
                        selection
                            .acknowledge(selection.request().context("tool callback absent")?)
                            .map_err(anyhow::Error::msg)?;
                    } else {
                        c.complete = true;
                    }
                    Progress::Pending
                }
            });
        }
        let request = if let Some(selection) = &mut c.selection {
            let Some(r) = selection
                .advance(&mut s.current, &s.listeners)
                .map_err(anyhow::Error::msg)?
            else {
                c.complete = true;
                return Ok(Progress::Complete);
            };
            Some(r)
        } else {
            None
        };
        let (id, operation) = match request {
            Some(r @ (native::Request::Old { .. } | native::Request::New { .. })) => {
                //668744+2c/+30 both point at51d372: ret4.
                c.selection
                    .as_mut()
                    .unwrap()
                    .acknowledge(r)
                    .map_err(anyhow::Error::msg)?;
                return Ok(Progress::Pending);
            }
            Some(native::Request::Reset(id)) => (id, Operation::Cleanup(cleanup::Kind::Reset)),
            Some(native::Request::Enter(id)) => (id, Operation::Preview),
            None => match c.request {
                world::Request::ActivateTool {
                    tool: Some(id),
                    argument_low,
                } => (id, Operation::Activate(u32::from(argument_low))),
                _ => anyhow::bail!("missing concrete tool call"),
            },
        };
        if self.host.presentation.tool_selection.is_none() {
            let picker = self
                .host
                .presentation
                .retained_tool_inputs
                .as_ref()
                .context("retained tool receivers absent")?
                .is_picker(id);
            ensure!(
                picker,
                "native stock-tool callback requires its retained receiver implementation"
            );
            match (request, operation) {
                (Some(r @ native::Request::Reset(_)), _) => {
                    self.clear_picker_target_fields()?;
                    c.selection
                        .as_mut()
                        .unwrap()
                        .acknowledge(r)
                        .map_err(anyhow::Error::msg)?;
                }
                (None, Operation::Activate(argument)) => {
                    self.host
                        .presentation
                        .retained_tool_inputs
                        .as_mut()
                        .unwrap()
                        .activate_picker(id, argument)?;
                    c.complete = true;
                }
                (Some(r), _) => return Ok(Progress::Service(Service::Callback(r))),
                _ => anyhow::bail!("unsupported retained Picker operation"),
            }
            return Ok(Progress::Pending);
        }
        let tools = self
            .host
            .presentation
            .tool_selection
            .as_mut()
            .context("tool receivers absent")?;
        match tools.receiver_mut(id)? {
            Receiver::Picker { mode } => {
                match (request, operation) {
                    (Some(r @ native::Request::Reset(_)), _) => {
                        //48c90f->48c834(0,-1,-1): clear the old target with
                        //both ordered low-flag setters.48d392 with null target
                        //clears the menu and returns without eligibility work.
                        //The replacement menu owner must already be closed.
                        self.clear_picker_target_fields()?;
                        c.selection
                            .as_mut()
                            .unwrap()
                            .acknowledge(r)
                            .map_err(anyhow::Error::msg)?;
                    }
                    (None, Operation::Activate(argument)) => {
                        sims_vm::interface::tool::activate_picker(mode, argument);
                        c.complete = true;
                    }
                    (Some(r), _) => return Ok(Progress::Service(Service::Callback(r))),
                    _ => anyhow::bail!("unsupported picker operation"),
                }
                return Ok(Progress::Pending);
            }
            Receiver::Move => (),
        }
        if matches!(request, Some(native::Request::Enter(_))) {
            if s.move_text.is_none() {
                s.move_text = Some(Text::load(self)?);
                return Ok(Progress::Pending);
            }
        }
        let serial = s
            .serial
            .checked_add(1)
            .context("current tool serial exhausted")?;
        let owner = MoveCaller {
            publication: c.publication,
            serial,
            tool: id,
            operation,
        };
        self.begin_current_move(owner.clone())?;
        c.child = Some(owner);
        s.serial = serial;
        Ok(Progress::Pending)
    }
}
