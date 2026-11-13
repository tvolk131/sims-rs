//!4cea5d composition through actual tool callers and shared CPState/camera.
//!The outer caller remains suspended at any unbound engine service.
use super::{Frame, Progress, Service, State};
use crate::original_session::Session;
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_vm::interface::mode_change::{self as native, Change, Request};

///Explicit application/process bindings. The camera-enabled and notification
///bytes have no other owner in this frontend; never infer them on restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub camera_enabled: u8,
    pub notify_pending: u8,
    pub object_tool_address: u32,
    pub build_view_present: bool,
    pub zoning: Option<i32>,
    pub menu_present: bool,
    pub menu_active: u8,
    ///4df821's allocation log, memory snapshots and memory log are disabled
    ///explicitly by the replacement application, matching its no-op gates.
    pub memory_diagnostics_disabled: bool,
    ///Actual replacement undo/redo owner behind Root+18+8, supplied at setup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    history_manager: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    close: Option<Box<crate::original_session::move_tool::close::Close>>,
    ///Last completed4d31b7 arguments, used to validate the device owner.
    window_pause: Option<[u8; 2]>,
}
impl Context {
    pub fn new(object_tool_address: u32) -> Self {
        Self {
            camera_enabled: 0,
            notify_pending: 0,
            object_tool_address,
            build_view_present: false,
            zoning: None,
            menu_present: false,
            menu_active: 0,
            memory_diagnostics_disabled: true,
            history_manager: None,
            close: None,
            window_pause: None,
        }
    }
    pub fn window_pause(&self) -> Option<[u8; 2]> {
        self.window_pause
    }
    pub fn with_history_manager(mut self, manager: u32) -> Self {
        self.history_manager = Some(manager);
        self
    }
    pub(super) fn validate_close(
        &self,
        session: &Session,
        request: Option<&Request>,
    ) -> Result<()> {
        if let Some(close) = &self.close {
            ensure!(
                request
                    == Some(&Request::CloseObjectTool(
                        self.object_tool_address.wrapping_add(8)
                    )),
                "history close lost mode caller"
            );
            close.validate(session)?;
        }
        Ok(())
    }
}

pub(super) fn advance(
    call: &mut Change,
    context: &mut Context,
    session: &mut Session,
) -> Result<bool> {
    let camera = session
        .host
        .presentation
        .presenter
        .as_mut()
        .and_then(|p| p.camera.as_mut())
        .context("mode camera owner absent")?;
    let controls = &mut session
        .host
        .presentation
        .transport
        .as_mut()
        .context("mode CPState absent")?
        .controls;
    let globals = &session.simulation.vm.world.globals;
    let lot_mode = *globals.get(&32).context("mode global32 absent")?;
    let lot_number = *globals.get(&10).context("mode global10 absent")?;
    let input = native::Inputs {
        application_clock: camera.settings.clock,
        object_tool_address: context.object_tool_address,
        build_view_present: context.build_view_present,
        lot_mode,
        lot_number,
        zoning: if lot_mode == 1 {
            context
                .zoning
                .context("commercial zoning must be supplied by its actual owner")?
        } else {
            0
        },
        paused: controls.paused,
        menu_present: context.menu_present,
        menu_active: context.menu_active,
    };
    let mut fields = controls.mode_fields(context.camera_enabled, context.notify_pending);
    let result = call.advance(&mut fields, input);
    (context.camera_enabled, context.notify_pending) = controls.publish_mode_fields(fields);
    camera.settings.application_mode = controls.mode;
    Ok(result.map_err(anyhow::Error::msg)?.is_some())
}

impl State {
    pub fn bind_mode_context(&mut self, context: Context) -> Result<()> {
        ensure!(
            !self.pending() && self.mode.is_none(),
            "mode context already bound or busy"
        );
        self.mode = Some(context);
        Ok(())
    }
    pub fn mode_context(&self) -> Option<&Context> {
        self.mode.as_ref()
    }
    pub fn mode_window_owner(&self) -> Option<u64> {
        self.call.as_ref().and_then(|c| c.window)
    }
    pub fn mode_request(&self) -> Option<&Request> {
        match self.call.as_ref()?.frames.last()? {
            Frame::Mode(f) => f.request(),
            _ => None,
        }
    }
}

impl Session {
    ///4cea5d only dirties mode bit2.4ce44f must publish the corresponding
    ///simulator/drawing/audio state before allowing another lot pass.
    pub fn mode_publication_pending(&self) -> bool {
        self.host
            .presentation
            .tool_selection
            .as_ref()
            .and_then(State::mode_context)
            .is_some()
            && self
                .host
                .presentation
                .transport
                .as_ref()
                .is_some_and(|t| t.controls.mode != 2 && t.controls.buy.dirty & 2 != 0)
    }
    pub fn begin_mode_change(&mut self, target: i32, force: u8) -> Result<()> {
        ensure!(
            self.host
                .presentation
                .tool_selection
                .as_ref()
                .and_then(|s| s.mode.as_ref())
                .is_some(),
            "mode context absent"
        );
        self.begin_tool_frame(Frame::Mode(Change::new(target, force)))
    }
    pub fn claim_mode_window_pause(&mut self, paused: u8, overlay: u8) -> Result<u64> {
        self.validate_tool_selection_owner()?;
        let state = self
            .host
            .presentation
            .tool_selection
            .as_mut()
            .context("mode tool owner absent")?;
        ensure!(
            state.mode_window_owner().is_none()
                && state.mode_request()
                    == Some(&Request::WindowPause {
                        paused,
                        show_overlay: overlay
                    }),
            "mode window already owned or caller differs"
        );
        let serial = state
            .serial
            .checked_add(1)
            .context("mode callback serial exhausted")?;
        state.call.as_mut().context("mode caller absent")?.window = Some(serial);
        state.serial = serial;
        Ok(serial)
    }
    pub fn acknowledge_mode_window_pause(
        &mut self,
        serial: u64,
        paused: u8,
        overlay: u8,
    ) -> Result<()> {
        self.validate_tool_selection_owner()?;
        let state = self
            .host
            .presentation
            .tool_selection
            .as_mut()
            .context("mode tool owner absent")?;
        let c = state.call.as_mut().context("mode call absent")?;
        ensure!(
            c.window == Some(serial) && c.child.is_none() && c.drawing.is_none() && !c.notice,
            "mode window overlaps child"
        );
        let Frame::Mode(f) = c.frames.last_mut().context("mode window caller absent")? else {
            anyhow::bail!("mode window parent differs")
        };
        let r = Request::WindowPause {
            paused,
            show_overlay: overlay,
        };
        ensure!(f.request() == Some(&r), "mode window reply differs");
        f.acknowledge(&r).map_err(anyhow::Error::msg)?;
        c.window = None;
        state
            .mode
            .as_mut()
            .context("mode context absent")?
            .window_pause = Some([paused, overlay]);
        Ok(())
    }

    ///43dfc4's replacement renderer has rebuilt its view-dependent geometry.
    ///Imported definitions, StringSet values and script state are retained;
    ///discarding legacy decoded payloads must not clear these authoritative
    ///stores. The replacement uses different cache allocation lifetimes.
    pub fn acknowledge_mode_definition_refresh(&mut self) -> Result<()> {
        self.acknowledge_mode_resource_service(Request::RefreshObjects)
    }

    ///The replacement reclaimed renderer-owned resources while retaining its
    ///canonical immutable program/appearance inputs. No animation or skeleton
    ///is replayed to reconstruct a cache; their active state and RNG survive.
    pub fn acknowledge_mode_timed_resources(&mut self, clock: u32) -> Result<()> {
        self.acknowledge_mode_resource_service(Request::TimedResourceUpdates(clock))
    }

    fn acknowledge_mode_resource_service(&mut self, request: Request) -> Result<()> {
        self.validate_tool_selection_owner()?;
        let state = self
            .host
            .presentation
            .tool_selection
            .as_mut()
            .context("mode resource service lost tool owner")?;
        let call = state
            .call
            .as_mut()
            .context("mode resource service lost caller")?;
        ensure!(
            call.child.is_none() && call.drawing.is_none() && !call.notice && call.window.is_none(),
            "mode resource service overlaps a child service"
        );
        let Some(Frame::Mode(frame)) = call.frames.last_mut() else {
            anyhow::bail!("mode resource service lost mode caller")
        };
        ensure!(
            frame.request() == Some(&request),
            "resource completion does not match pending mode request"
        );
        frame.acknowledge(&request).map_err(anyhow::Error::msg)
    }

    pub(super) fn service_mode_request(
        &mut self,
        state: &mut State,
        r: Request,
    ) -> Result<Option<Progress>> {
        let c = state.call.as_mut().context("mode caller absent")?;
        let Frame::Mode(f) = c.frames.last_mut().context("mode frame absent")? else {
            anyhow::bail!("mode action differs")
        };
        ensure!(f.request() == Some(&r), "mode request changed");
        match r {
            Request::RefreshHouse => {
                self.refresh_original_house()?;
            }
            Request::ReconcileLotControllers => {
                if self.simulation.controller_reconciliation.is_none() {
                    self.begin_original_controllers()?;
                }
                match self.advance_original_controllers(1)? {
                    crate::original_session::controllers::Progress::Pending => return Ok(None),
                    crate::original_session::controllers::Progress::Service(service) => {
                        return Ok(Some(Progress::Service(Service::Notification(service))));
                    }
                    crate::original_session::controllers::Progress::Complete => {
                        self.finish_original_controllers()?
                    }
                }
            }
            Request::CloseObjectTool(address) => {
                let context = state
                    .mode
                    .as_mut()
                    .context("history close context absent")?;
                ensure!(
                    address == context.object_tool_address.wrapping_add(8),
                    "history close address differs"
                );
                if context.close.is_none() {
                    context.close = Some(Box::new(
                        crate::original_session::move_tool::close::Close::new(
                            context
                                .history_manager
                                .context("mode object-tool history owner absent")?,
                        ),
                    ));
                }
                use crate::original_session::move_tool::close::Progress as CloseProgress;
                match context.close.as_mut().unwrap().advance(self)? {
                    CloseProgress::Pending => return Ok(None),
                    CloseProgress::Service(service) => {
                        return Ok(Some(Progress::Service(Service::Move(service))));
                    }
                    CloseProgress::Complete => context.close = None,
                }
            }
            Request::ResetBuySelection => self
                .host
                .presentation
                .transport
                .as_mut()
                .context("mode CPState absent")?
                .controls
                .buy
                .reset_selection()
                .map_err(anyhow::Error::msg)?,
            Request::Notice(_) => ensure!(
                state
                    .mode
                    .as_ref()
                    .context("mode context absent")?
                    .memory_diagnostics_disabled,
                "enabled native memory diagnostics require their log owners"
            ),
            Request::Notify => {
                self.begin_clock_notice(0x91, 0)?;
                c.notice = true;
                return Ok(None);
            }
            _ => return Ok(Some(Progress::Service(Service::Mode(r)))),
        }
        f.acknowledge(&r).map_err(anyhow::Error::msg)?;
        Ok(None)
    }
}
