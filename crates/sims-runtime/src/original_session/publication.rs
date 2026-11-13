//!Saved4ce390 ->4ce44f. Native room/camera/drawing children run against their
//!live owners. Unbound selection/panel and tool-entry branches remain requests.
use super::{
    Session, camera_command, current_tool, notifications, presenter::selected_for_visibility,
};
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::lot_presenter::{camera, open};
use sims_vm::interface::{drawing, publication as frame, world_publication as world};
mod transport;

///Additional process/UI fields with no other owner in this frontend.
///Binding is explicit, not inferred from a legacy household save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub panel_owner: Option<u64>,
    pub closing: u8,
    pub walls: i32,
    pub tool_gate: u32,
    pub current_tool: Option<u32>,
    ///CPState+1ec, a portrait-cache index. Keep the historical misnamed wire key.
    #[serde(rename = "sound_object")]
    pub selected_person_index: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    context: Context,
    serial: u64,
    tick: u64,
    frame: frame::Frame,
    world: Option<world::Update>,
    camera: bool,
    drawing: Option<drawing::Change>,
    notice: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    tools: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    person: Option<Person>,
    /// An actual renderer-owned panel continuation. Historical saves stay
    /// unbound; claiming this child does not acknowledge its return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    panels: Option<PanelOwner>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    panels_started: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transport: Option<transport::Child>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelOwner {
    pub panel: u32,
    pub controls: u32,
    pub state: u32,
}
/// Published text inputs for the replacement frontend, in original encodings.
/// These are freshly read lot properties, not recovered Windows string objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelText {
    pub language: u8,
    pub funds: Vec<u8>,
    pub clock: Vec<u8>,
}
/// Sources for a newly attached frontend. Live/label rules are set by house
/// opening (4f2473..4f2508); the fire restriction is refreshed by 4ce81c.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelAccess {
    pub buy_blocked: bool,
    pub live_blocked: bool,
    pub labels_hidden: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Person {
    index: u32,
    target: Option<sims_vm::ObjectId>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Complete,
    Camera(camera_command::Progress),
    Tool(current_tool::Progress),
    Notification(notifications::Service),
    World(world::Request),
    Panels(Option<u64>),
}
impl State {
    pub fn walls(&self) -> i32 {
        self.context.walls
    }
    pub(super) fn set_deferred_walls(&mut self, value: u32) {
        self.context.walls = value as i32;
    }
    pub fn panel_request(&self) -> Option<(u64, Option<u64>)> {
        match self.frame.request()? {
            frame::Request::Panels(panel) => Some((self.serial, panel)),
            _ => None,
        }
    }
    pub fn panel_owner(&self) -> Option<(u64, PanelOwner)> {
        self.panels.map(|owner| (self.serial, owner))
    }
    pub fn pending(&self) -> bool {
        !self.frame.complete()
    }
    pub fn request(&self) -> Option<world::Request> {
        self.world.as_ref().and_then(world::Update::request)
    }
    pub fn camera_owner(&self) -> Option<u64> {
        self.camera.then_some(self.serial)
    }
    pub(super) fn context_current_tool(&self) -> Option<u32> {
        self.context.current_tool
    }
    pub(super) fn tool_owner(&self) -> Option<u64> {
        self.tools.then_some(self.serial)
    }
    pub fn camera_request(&self) -> Option<open::Request> {
        Some(match self.request()? {
            world::Request::Zoom(zoom) => open::Request::Zoom {
                zoom,
                force: 0,
                refresh_ui: 1,
            },
            world::Request::Rotation(rotation) => open::Request::Rotate {
                rotation,
                force: 0,
                refresh_ui: 1,
            },
            world::Request::Floor(level) => open::Request::Floor { level, force: 0 },
            world::Request::WallsDown(value) => open::Request::WallsDown(value as u8),
            world::Request::Cutaway(value) => open::Request::Cutaway(value as u8),
            world::Request::Roof(value) => open::Request::Roof {
                enabled: value as u8,
                force: 0,
            },
            _ => return None,
        })
    }
    pub fn drawing_written(&self) -> Option<u8> {
        self.drawing.as_ref().and_then(drawing::Change::written)
    }
    pub fn validate(&self, s: &Session) -> Result<()> {
        self.frame.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.panels_started == self.panels.is_some(),
            "panel publication lost its claimed continuation"
        );
        ensure!(
            self.serial != 0
                && self.context.panel_owner != Some(0)
                && self.context.current_tool != Some(0),
            "invalid publication identity"
        );
        if !self.pending() {
            ensure!(
                self.world.is_none()
                    && !self.camera
                    && self.drawing.is_none()
                    && !self.notice
                    && !self.tools,
                "returned publication retains children"
            );
            ensure!(
                self.person.is_none() && self.panels.is_none() && self.transport.is_none(),
                "returned publication retains selection"
            );
            return Ok(());
        }
        ensure!(
            self.tick == s.simulation.vm.tick,
            "publication crossed a lot tick"
        );
        if let Some(owner) = self.panels {
            ensure!(
                owner.panel != 0
                    && owner.controls != 0
                    && owner.state != 0
                    && owner.panel != owner.controls
                    && owner.panel != owner.state
                    && owner.controls != owner.state
                    && self.panel_request() == Some((self.serial, Some(u64::from(owner.panel))))
                    && self.world.is_none(),
                "panel backend lost its publication caller"
            );
        }
        let picker_selection = self.tools
            && s.host
                .presentation
                .current_tool
                .as_ref()
                .and_then(current_tool::State::picker_selection_owner)
                == Some(self.serial);
        ensure!(
            !s.tool_selection_pending()
                && (!s.move_tool_pending() || self.tools)
                && !s.ground_input_pending()
                && !s.object_input_pending()
                && !s.interface_update_pending()
                && !s.clock_pending()
                && !s.control_panel_pending()
                && !s.ui_wait_pending()
                && !s.ui_portrait_pending()
                && !s.load_completion_pending()
                && !s.postload_pending()
                && !s.view_refresh_pending()
                && !s.host.presentation.awaiting_house_open
                && (!s.host.ordinary.updates_pending()
                    || self.person.is_some()
                    || picker_selection)
                && (!s.simulation.selection_work_pending()
                    || self.person.is_some()
                    || picker_selection)
                && s.simulation.object_pickup.is_none()
                && s.simulation.object_placement.is_none()
                && s.simulation.cursor_factory.is_none()
                && s.simulation.vm.world.dialogs.modal.is_none()
                && !s.simulation.vm.constructing_dialog()
                && !s
                    .simulation
                    .vm
                    .threads
                    .values()
                    .any(|t| matches!(t.status, sims_vm::Status::AwaitingUiEffect(_))),
            "publication overlaps synchronous work"
        );
        ensure!(
            s.host.presentation.transport.is_some()
                && s.host.presentation.camera_ui.is_some()
                && s.host
                    .presentation
                    .tool_selection
                    .as_ref()
                    .and_then(|t| t.mode_context())
                    .is_some(),
            "publication field owner absent"
        );
        if let Some(w) = &self.world {
            w.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                self.frame.request() == Some(frame::Request::World(w.mask())),
                "world publication lost captured mask"
            );
        } else {
            ensure!(
                !self.camera
                    && self.drawing.is_none()
                    && !self.notice
                    && !self.tools
                    && self.person.is_none()
                    && self.transport.is_none(),
                "publication child lost world caller"
            );
        }
        if let Some(person) = &self.person {
            let child = s
                .simulation
                .native_selection
                .as_ref()
                .context("publication selection child absent")?;
            ensure!(
                !self.camera
                    && !self.tools
                    && !self.notice
                    && self.drawing.is_none()
                    && self.request() == Some(world::Request::SelectPersonIndex(person.index))
                    && child.publication_owner() == Some((self.serial, person.index))
                    && child.publication_target() == Some(person.target)
                    && s.host.presentation.person_selection_override.is_some(),
                "publication selection lost its caller"
            );
        } else {
            ensure!(
                s.simulation
                    .native_selection
                    .as_ref()
                    .is_none_or(|e| e.publication_owner().is_none()),
                "selection publication parent absent"
            );
        }
        if self.tools {
            ensure!(
                !self.camera
                    && self.drawing.is_none()
                    && !self.notice
                    && s.host
                        .presentation
                        .current_tool
                        .as_ref()
                        .and_then(current_tool::State::owner)
                        == self.request().map(|r| (self.serial, r)),
                "publication lost current-tool child"
            );
        }
        if self.camera {
            ensure!(
                self.camera_request().is_some() && self.drawing.is_none() && !self.notice,
                "publication camera overlaps drawing"
            );
            ensure!(
                s.host
                    .presentation
                    .camera_command
                    .as_ref()
                    .is_some_and(|c| c.publication_owner() == Some(self.serial)),
                "publication camera child absent"
            );
        } else {
            ensure!(!s.camera_command_pending(), "unowned publication camera");
        }
        if let Some(d) = &self.drawing {
            d.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                self.request() == Some(world::Request::DrawingMode(u32::from(d.value()))),
                "drawing lost world caller"
            );
        }
        if self.notice {
            ensure!(
                self.drawing
                    .as_ref()
                    .is_some_and(|d| d.written().is_some() && !d.complete())
                    && s.host
                        .presentation
                        .notifications
                        .as_ref()
                        .is_some_and(|n| n.dispatch.root_message() == Some((0x83, 0))),
                "drawing broadcast lost parent"
            );
        } else if !self.camera && !self.tools && self.person.is_none() && self.transport.is_none() {
            ensure!(
                !s.house_notification_pending(),
                "unowned publication notification"
            );
        }
        if let Some(child) = &self.transport {
            child.validate(self, s)?;
        }
        Ok(())
    }
}
impl Session {
    pub fn panel_access(&self) -> Result<PanelAccess> {
        let community = *self
            .simulation
            .vm
            .world
            .globals
            .get(&32)
            .context("panel lot kind absent")?
            == 1;
        Ok(PanelAccess {
            // Registry+38 is explicit membership, not a search for fire GUIDs.
            buy_blocked: !self.simulation.vm.world.fires.registered.is_empty(),
            live_blocked: community,
            labels_hidden: community,
        })
    }
    /// The funds/clock reads in 4ce81c, including 46d8c9's community-lot
    /// allowance and 4cecf5's signed upper cap. Both formatters already have
    /// original-code comparison fixtures. This does not change the lot.
    pub fn panel_text(&self) -> Result<PanelText> {
        let world = &self.simulation.vm.world;
        let global = |id| {
            world
                .globals
                .get(&id)
                .copied()
                .context("panel text global absent")
        };
        let locale = self
            .host
            .notifications
            .as_ref()
            .context("panel text locale absent")?
            .saved
            .locale;
        let balance = if global(32)? == 1 {
            999_999
        } else {
            world
                .budget
                .as_ref()
                .context("panel budget absent")?
                .balance
        };
        Ok(PanelText {
            language: locale.language.try_into()?,
            funds: locale.format(i64::from(balance.min(9_999_999))),
            clock: sims_vm::dialog::format_time(
                self.program
                    .text_language
                    .context("panel clock language absent")?,
                i32::from(global(0)?),
                global(5)? as i32 as u32,
                true,
                true,
            )
            .into_bytes(),
        })
    }
    /// Borrow the actual owners of ViewControl's scalar properties. Unknown
    /// services remain pending; no presentation defaults stand in for lot state.
    pub fn panel_number(
        &self,
        query: sims_vm::interface::view_control::Query,
    ) -> Result<Option<u32>> {
        use sims_vm::interface::view_control::Query;
        let p = &self.host.presentation;
        let ui = || p.camera_ui.context("panel camera properties absent");
        let global = |id| {
            self.simulation
                .vm
                .world
                .globals
                .get(&id)
                .copied()
                .context("panel lot global absent")
        };
        Ok(Some(match query {
            Query::Mode => {
                p.transport
                    .as_ref()
                    .context("panel CPState absent")?
                    .controls
                    .mode as u32
            }
            Query::Floor => ui()?.floor as u32,
            Query::UpperFloorAvailable | Query::BuildFloorAvailable => {
                // 4cfab1/4cfabd read the two flags produced by 479d9a.
                // Building support alone is not an upstairs walking surface.
                let flags = &self
                    .simulation
                    .architecture
                    .as_ref()
                    .and_then(|a| a.original.as_ref())
                    .and_then(|a| a.floor(2))
                    .context("panel original upper-floor flags absent")?
                    .flags;
                u32::from(if query == Query::UpperFloorAvailable {
                    flags.upper_contents
                } else {
                    flags.upper_support
                })
            }
            Query::Walls => p
                .publication
                .as_ref()
                .map(|p| p.context.walls)
                .or_else(|| p.frontend_context.as_ref().map(|f| f.walls))
                .context("panel publication wall owner absent")? as u32,
            Query::AvailableWalls => {
                if matches!(ui()?.floor, 1 | 2) {
                    7
                } else {
                    1
                }
            }
            Query::ZoomIn => u32::from((ui()?.zoom as i32) < 3),
            Query::ZoomOut => {
                u32::from((ui()?.zoom as i32) > if global(10)? == 99 { 2 } else { 1 })
            }
            Query::LotPaused => u32::from(global(17)? != 0),
            Query::Speed => global(16)? as i32 as u32,
            Query::SavedSpeedActive => u32::from(global(18)? >= 0),
            Query::SavedSpeed => global(18)? as i32 as u32,
            Query::Expansion => p
                .control_panel
                .as_ref()
                .context("panel expansion state absent")?
                .value() as u32,
            _ => return Ok(None),
        }))
    }
    /// Claim the saved 50a8ec call for a concrete frontend. There is deliberately
    /// no general panel acknowledgement API: its children still have to run.
    pub fn claim_publication_panels(&mut self, owner: PanelOwner) -> Result<u64> {
        self.validate_publication()?;
        let p = self
            .host
            .presentation
            .publication
            .as_mut()
            .context("publication absent")?;
        ensure!(
            p.panels.is_none()
                && p.panel_request() == Some((p.serial, Some(u64::from(owner.panel))))
                && owner.panel != 0
                && owner.controls != 0
                && owner.state != 0
                && owner.panel != owner.controls
                && owner.panel != owner.state
                && owner.controls != owner.state,
            "panel caller already owned or identities differ"
        );
        p.panels = Some(owner);
        p.panels_started = true;
        Ok(p.serial)
    }
    /// Return only a completed, matching50a8ec continuation. Its backend owns
    /// every acknowledged child; this does not clear or finish the frame itself.
    pub fn complete_publication_panels(
        &mut self,
        serial: u64,
        owner: PanelOwner,
        completed: &sims_vm::interface::panel_dispatch::Update,
    ) -> Result<()> {
        self.validate_publication()?;
        completed.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            completed.complete() && completed.matches(owner.state),
            "panel update has unfinished children"
        );
        let p = self
            .host
            .presentation
            .publication
            .as_mut()
            .context("publication absent")?;
        ensure!(
            p.panel_owner() == Some((serial, owner)),
            "panel return owner differs"
        );
        p.frame
            .acknowledge(frame::Request::Panels(Some(u64::from(owner.panel))))
            .map_err(anyhow::Error::msg)?;
        p.panels = None;
        p.panels_started = false;
        Ok(())
    }
    pub fn publication_pending(&self) -> bool {
        self.host
            .presentation
            .publication
            .as_ref()
            .is_some_and(State::pending)
    }
    pub(super) fn validate_publication(&self) -> Result<()> {
        if self.host.presentation.person_selection_override.is_some() {
            ensure!(
                self.host
                    .presentation
                    .transport
                    .as_ref()
                    .and_then(|s| s.controls.selected_person_index)
                    .is_some(),
                "bound person publication lost CPState selected index"
            );
        }
        if let Some(state) = &self.host.presentation.publication {
            state.validate(self)?;
        }
        self.validate_current_tool()?;
        if let Some(owner) = self
            .simulation
            .native_selection
            .as_ref()
            .and_then(|e| e.publication_owner())
        {
            ensure!(
                self.host
                    .presentation
                    .publication
                    .as_ref()
                    .is_some_and(|p| p.pending()
                        && p.person
                            .as_ref()
                            .is_some_and(|c| owner == (p.serial, c.index))),
                "selection lost publication owner"
            );
        }
        Ok(())
    }
    pub fn begin_interface_publication(&mut self, context: Context, force: u32) -> Result<()> {
        if self.host.presentation.frontend_context.is_some() {
            ensure!(
                context == self.retained_publication_context()?,
                "publication arguments conflict with retained native owners"
            );
        }
        ensure!(
            !self.publication_pending(),
            "interface publication already active"
        );
        let serial = self
            .host
            .presentation
            .publication
            .as_ref()
            .map_or(0, |p| p.serial)
            .checked_add(1)
            .context("publication serial exhausted")?;
        let state = State {
            context,
            serial,
            tick: self.simulation.vm.tick,
            frame: frame::Frame::new(force),
            world: None,
            camera: false,
            drawing: None,
            notice: false,
            tools: false,
            person: None,
            panels: None,
            panels_started: false,
            transport: None,
        };
        state.validate(self)?;
        self.publication_inputs(&state.context)?;
        self.host.presentation.publication = Some(state);
        Ok(())
    }
    /// A frontend that already owns a completed frame may publish a subsequent
    /// input's dirty fields using the same process bindings. No implicit owner
    /// is constructed for older or unbound frontends.
    pub fn republish_interface(&mut self) -> Result<bool> {
        let Some(previous) = &self.host.presentation.publication else {
            return Ok(false);
        };
        let context = previous.context.clone();
        self.begin_interface_publication(context, 0)?;
        Ok(true)
    }
    pub fn begin_walls_change(&mut self, walls: u32) -> Result<()> {
        ensure!(
            matches!(walls, 1 | 2 | 4),
            "walls change outside original wall modes"
        );
        self.ensure_camera_input_idle()?;
        ensure!(
            !self.publication_pending(),
            "walls change overlaps interface publication"
        );
        let presentation = &self.host.presentation;
        let floor = presentation
            .camera_ui
            .context("walls change camera properties absent")?
            .floor;
        let available = if matches!(floor, 1 | 2) { 7 } else { 1 };
        ensure!(
            walls & available != 0,
            "walls mode unavailable on this floor"
        );
        if presentation.publication.is_none() && presentation.frontend_context.is_none() {
            return self.begin_unbound_walls_change(walls);
        }
        ensure!(
            presentation.publication.is_some() && presentation.transport.is_some(),
            "walls change publication owners absent"
        );
        self.publication_inputs(&presentation.publication.as_ref().unwrap().context)?;
        let presentation = &mut self.host.presentation;
        presentation
            .publication
            .as_mut()
            .unwrap()
            .set_deferred_walls(walls);
        if let Some(frontend) = &mut presentation.frontend_context {
            frontend.walls = walls as i32;
        }
        presentation
            .transport
            .as_mut()
            .unwrap()
            .controls
            .mark_dirty(0x80);
        self.republish_interface()?;
        Ok(())
    }
    fn publication_inputs(&self, context: &Context) -> Result<world::Inputs> {
        let v = &self.host.presentation;
        let (fields, menu_present, menu_active, zoning) = if let Some(t) = &v.tool_selection {
            let mode = t
                .mode_context()
                .context("publication mode context absent")?;
            (t.fields(), mode.menu_present, mode.menu_active, mode.zoning)
        } else {
            let frontend = v
                .frontend_context
                .as_ref()
                .context("publication frontend inputs absent")?;
            let tools = v
                .retained_tool_inputs
                .as_ref()
                .context("publication retained tools absent")?;
            (
                tools.selected(),
                frontend.menu_present,
                frontend.menu_active,
                None,
            )
        };
        let ui = v
            .camera_ui
            .context("publication camera properties absent")?;
        let p = v
            .presenter
            .as_ref()
            .context("publication presenter absent")?;
        let g = &self.simulation.vm.world.globals;
        let lot_kind = *g.get(&32).context("publication lot kind absent")?;
        Ok(world::Inputs {
            menu_present,
            menu_active,
            closing: context.closing,
            include_visible: p.view.floor.visibility.include_visible,
            zoom: ui.zoom,
            rotation: ui.rotation,
            floor: ui.floor,
            walls: context.walls,
            tool: fields.active,
            tool_argument: fields.argument,
            tool_gate: context.tool_gate,
            current_tool: v
                .current_tool
                .as_ref()
                .map_or(context.current_tool, current_tool::State::current),
            selected_person_index: if v.person_selection_override.is_some() {
                v.transport
                    .as_ref()
                    .context("selection CPState absent")?
                    .controls
                    .selected_person_index
                    .context("selection CPState index unbound")? as u32
            } else {
                context.selected_person_index
            },
            lot_speed: *g.get(&16).context("publication lot speed absent")?,
            lot_kind,
            lot_number: *g.get(&10).context("publication lot number absent")?,
            zoning: if lot_kind == 1 {
                zoning.context("publication zoning absent")?
            } else {
                0
            },
        })
    }
    fn publication_reply(&mut self, request: world::Request) -> Result<()> {
        self.host
            .presentation
            .publication
            .as_mut()
            .context("publication absent")?
            .world
            .as_mut()
            .context("world publication absent")?
            .acknowledge(request)
            .map_err(anyhow::Error::msg)
    }
    pub fn complete_publication_window_reset(&mut self, cursor: u32) -> Result<()> {
        self.validate_publication()?;
        ensure!(cursor == 0, "publication window reset cursor differs");
        ensure!(
            self.host
                .presentation
                .publication
                .as_ref()
                .and_then(State::request)
                == Some(world::Request::WindowReset),
            "publication window reset lost its caller"
        );
        self.publication_reply(world::Request::WindowReset)
    }
    ///Only actual disabled-audio early returns can use this service boundary.
    pub fn acknowledge_publication_audio(&mut self, request: world::Request) -> Result<()> {
        self.validate_publication()?;
        ensure!(
            matches!(
                request,
                world::Request::PauseAudio
                    | world::Request::ResumeAudio
                    | world::Request::RefreshAudio
                    | world::Request::StopMusic
                    | world::Request::PlayMusic(_)
            ),
            "publication engine child cannot be externally acknowledged"
        );
        self.publication_reply(request)
    }
    ///The device lends its actual drawing byte.4c4222 writes it before83.
    pub fn advance_publication_drawing(&mut self, drawing: &mut u8) -> Result<()> {
        self.validate_publication()?;
        let p = self
            .host
            .presentation
            .publication
            .as_mut()
            .context("publication absent")?;
        let Some(world::Request::DrawingMode(value)) = p.request() else {
            anyhow::bail!("publication is not calling drawing mode");
        };
        let d = p.drawing.get_or_insert_with(|| drawing::Change::new(value));
        if let Some(expected) = d.written() {
            ensure!(
                *drawing == expected,
                "drawing owner changed during its broadcast"
            );
        }
        let needs_notice = d.advance(drawing).map_err(anyhow::Error::msg)?;
        if needs_notice {
            if !p.notice {
                self.begin_clock_notice(0x83, 0)?;
                self.host.presentation.publication.as_mut().unwrap().notice = true;
            }
        } else {
            p.drawing = None;
            self.publication_reply(world::Request::DrawingMode(value))?;
        }
        Ok(())
    }
    pub fn advance_interface_publication(&mut self, budget: usize) -> Result<Progress> {
        ensure!(budget > 0, "interface publication needs a work budget");
        self.validate_publication()?;
        ensure!(
            self.host.presentation.publication.is_some(),
            "publication absent"
        );
        for _ in 0..budget {
            let p = self.host.presentation.publication.as_ref().unwrap();
            if p.transport.is_some() {
                self.advance_publication_transport()?;
                return Ok(Progress::Pending);
            }
            if p.person.is_some() {
                if self.house_notification_pending() {
                    match self.advance_house_notification(self.retained_camera_input()?, 1)? {
                        notifications::Progress::Service(service) => {
                            return Ok(Progress::Notification(service));
                        }
                        _ => return Ok(Progress::Pending),
                    }
                }
                let ordinary = self.host.ordinary.graphics.keys().copied().collect();
                self.simulation
                    .advance_native_selection(&self.program, &ordinary, 1)
                    .map_err(anyhow::Error::msg)?;
                if self.service_selection_assets(self.retained_camera_input()?, 1)? {
                    let p = self.host.presentation.publication.as_mut().unwrap();
                    let request = p.request().unwrap();
                    p.person = None;
                    self.publication_reply(request)?;
                }
                return Ok(Progress::Pending);
            }
            if p.tools {
                let progress = self.advance_current_tool()?;
                if progress == current_tool::Progress::Complete {
                    let request = self
                        .host
                        .presentation
                        .publication
                        .as_ref()
                        .unwrap()
                        .request()
                        .unwrap();
                    self.finish_current_tool()?;
                    self.host.presentation.publication.as_mut().unwrap().tools = false;
                    self.publication_reply(request)?;
                    continue;
                }
                return Ok(Progress::Tool(progress));
            }
            if p.camera {
                let progress = self.advance_camera_command(self.retained_camera_input()?, 1)?;
                if progress == camera_command::Progress::Complete {
                    let p = self.host.presentation.publication.as_mut().unwrap();
                    let request = p.request().unwrap();
                    p.camera = false;
                    self.host
                        .presentation
                        .camera_command
                        .as_mut()
                        .unwrap()
                        .release_publication()?;
                    self.publication_reply(request)?;
                    continue;
                }
                return Ok(Progress::Camera(progress));
            }
            if p.notice {
                match self.advance_house_notification(self.retained_camera_input()?, 1)? {
                    notifications::Progress::Pending => (),
                    notifications::Progress::Service(service) => {
                        return Ok(Progress::Notification(service));
                    }
                    notifications::Progress::Complete(_) => {
                        let p = self.host.presentation.publication.as_mut().unwrap();
                        let request = p.request().unwrap();
                        p.drawing
                            .as_mut()
                            .unwrap()
                            .acknowledge()
                            .map_err(anyhow::Error::msg)?;
                        p.drawing = None;
                        p.notice = false;
                        self.publication_reply(request)?;
                    }
                }
                continue;
            }
            if p.world.is_some() {
                let input = self.publication_inputs(&p.context)?;
                let p = self.host.presentation.publication.as_mut().unwrap();
                let w = p.world.as_mut().unwrap();
                let request = w
                    .advance(
                        &self.host.presentation.transport.as_ref().unwrap().controls,
                        self.simulation
                            .vm
                            .world
                            .globals
                            .get_mut(&19)
                            .context("publication global19 absent")?,
                        input,
                    )
                    .map_err(anyhow::Error::msg)?;
                if let Some(request) = request {
                    if let Some(child) = transport::Child::new(request) {
                        p.transport = Some(child);
                        return Ok(Progress::Pending);
                    } else if let world::Request::SelectPersonIndex(index) = request
                        && self.host.presentation.person_selection_override.is_some()
                    {
                        let serial = p.serial;
                        if let sims_vm::interface::person_selection::Decision::Select(target) =
                            self.person_publication_decision(index)?
                        {
                            self.simulation
                                .begin_publication_selection(target, (serial, index))
                                .map_err(anyhow::Error::msg)?;
                            self.host.presentation.publication.as_mut().unwrap().person =
                                Some(Person { index, target });
                            return Ok(Progress::Pending);
                        }
                        self.publication_reply(request)?;
                        continue;
                    } else if matches!(
                        request,
                        world::Request::SelectTool(_)
                            | world::Request::ActivateTool { tool: Some(_), .. }
                    ) && self.host.presentation.current_tool.is_some()
                    {
                        let serial = p.serial;
                        self.begin_current_tool(serial, request)?;
                        self.host.presentation.publication.as_mut().unwrap().tools = true;
                        return Ok(Progress::Pending);
                    } else if let Some(camera_request) = p.camera_request() {
                        let camera = camera_command::State::publication(
                            self.simulation.vm.tick,
                            p.serial,
                            camera_request,
                        )?;
                        ensure!(
                            !self.camera_command_pending(),
                            "publication camera already active"
                        );
                        let p = self.host.presentation.publication.as_mut().unwrap();
                        p.camera = true;
                        self.host.presentation.camera_command = Some(camera);
                        return Ok(Progress::Pending);
                    } else if let world::Request::RoomMask(include_visible) = request {
                        let selected = selected_for_visibility(&self.simulation)?;
                        let p = self
                            .host
                            .presentation
                            .presenter
                            .as_mut()
                            .context("publication presenter absent")?;
                        let c = p.camera.as_ref().context("publication camera absent")?;
                        c.compose(
                            &mut p.view,
                            &camera::Inputs {
                                roof_controls_present: self.host.presentation.transport.is_some(),
                                lot: self
                                    .simulation
                                    .architecture
                                    .as_ref()
                                    .and_then(|a| a.original.as_ref())
                                    .context("publication lot absent")?,
                                rooms: &self
                                    .simulation
                                    .room_environment
                                    .as_ref()
                                    .context("publication rooms absent")?
                                    .manager,
                                application_mode: c.settings.application_mode,
                                selected: selected.as_ref(),
                                clock: c.settings.clock,
                            },
                            include_visible,
                        )
                        .map_err(anyhow::Error::msg)?;
                        self.publication_reply(request)?;
                    } else {
                        return Ok(Progress::World(request));
                    }
                } else {
                    let p = self.host.presentation.publication.as_mut().unwrap();
                    let w = p.world.take().unwrap();
                    ensure!(w.complete(), "world publication returned early");
                    p.frame
                        .acknowledge(frame::Request::World(w.mask()))
                        .map_err(anyhow::Error::msg)?;
                }
                continue;
            }
            let now = self
                .host
                .presentation
                .presenter
                .as_ref()
                .and_then(|p| p.camera.as_ref())
                .context("publication clock absent")?
                .settings
                .clock;
            let p = self.host.presentation.publication.as_mut().unwrap();
            let request = p
                .frame
                .advance(
                    &mut self.host.presentation.transport.as_mut().unwrap().controls,
                    frame::Inputs {
                        now,
                        panel_owner: p.context.panel_owner,
                    },
                )
                .map_err(anyhow::Error::msg)?;
            match request {
                Some(frame::Request::World(mask)) => p.world = Some(world::Update::new(mask)),
                Some(frame::Request::Panels(owner)) => return Ok(Progress::Panels(owner)),
                None => return Ok(Progress::Complete),
            }
        }
        self.validate_publication()?;
        Ok(Progress::Pending)
    }
}
