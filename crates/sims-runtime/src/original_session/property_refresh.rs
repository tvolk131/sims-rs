//! Live Interface4ce41b composition. Property reads borrow the actual lot,
//! camera and CPState owners. The text/window/panel backend must perform its
//! child before acknowledging it; the enclosing notification stays suspended.
use super::{Session, notifications, publication::PanelOwner};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::{edit_history::List, lot_presenter::person_refresh};
use sims_vm::{
    commander::house::{Receiver, Request as Listener, dispatch},
    interface::{property_refresh as native, transport},
};

/// Replacement frontend identities, explicitly installed after its owners
/// exist. They are not addresses or fields recovered from a household save.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub panel: PanelOwner,
    pub view: u32,
    pub renderer: u32,
}
impl Binding {
    fn validate(self) -> Result<()> {
        let ids = [
            self.panel.panel,
            self.panel.controls,
            self.panel.state,
            self.view,
            self.renderer,
        ];
        ensure!(
            !ids.contains(&0)
                && ids
                    .into_iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == ids.len(),
            "property refresh identities absent or aliased"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub binding: Binding,
    serial: u64,
    undo_available: u8,
    call: Option<Execution>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Execution {
    tick: u64,
    caller: dispatch::Service,
    native: native::Call,
    pause: Option<transport::SetPause>,
    people: Option<person_refresh::Cursor>,
    panels: bool,
    funds_started: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selection_view_refresh: Option<sims_sim::lot_selection::NotificationViewRefresh>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending,
    /// Clock/SetFunds are real writes to the retained replacement text owner.
    Text {
        request: native::Request,
        language: u8,
        bytes: Vec<u8>,
    },
    WindowPause(u32),
    Panels(PanelOwner),
    Complete,
}
impl State {
    pub fn pending(&self) -> bool {
        self.call.is_some()
    }
    pub fn panel_owner(&self) -> Option<(u64, PanelOwner)> {
        self.call
            .as_ref()?
            .panels
            .then_some((self.serial, self.binding.panel))
    }
    pub fn panel_request(&self) -> Option<PanelOwner> {
        match self.call.as_ref()?.native.request()? {
            native::Request::Panels { panel, state }
                if panel == self.binding.panel.panel && state == self.binding.panel.state =>
            {
                Some(self.binding.panel)
            }
            _ => None,
        }
    }
    pub fn window_request(&self) -> Option<u32> {
        match self.call.as_ref()?.pause.as_ref()?.request()? {
            transport::Request::WindowPause(value) => Some(value),
            _ => None,
        }
    }
    pub fn validate(&self, s: &Session) -> Result<()> {
        self.binding.validate()?;
        let Some(c) = &self.call else { return Ok(()) };
        c.native.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.serial != 0
                && c.native.matches(self.binding.panel.state)
                && c.tick == s.simulation.vm.tick
                && c.selection_view_refresh
                    .is_none_or(|token| token.caller_serial() == self.serial)
                && c.caller.invocation.handler == Receiver::Interface
                && c.caller.request == Listener::UiRefresh
                && s.host
                    .presentation
                    .notifications
                    .as_ref()
                    .and_then(|n| n.pending())
                    == Some(&notifications::Service::Listener(c.caller.clone())),
            "property refresh lost its exact notification caller"
        );
        validate_selection_view_refresh(s, &c.caller, c.selection_view_refresh)?;
        if let Some(person) = s
            .simulation
            .native_selection
            .as_ref()
            .and_then(sims_sim::lot_selection::Execution::notification_view_person)
        {
            ensure!(
                c.people.as_ref().and_then(person_refresh::Cursor::pending) == Some(person),
                "property refresh lost its selection view person"
            );
        }
        if let Some(pause) = &c.pause {
            pause.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                matches!(c.native.request(), Some(native::Request::SetPaused(_))),
                "property pause lost its setter caller"
            );
        }
        if let Some(people) = &c.people {
            ensure!(
                c.pause.as_ref().and_then(transport::SetPause::request)
                    == Some(transport::Request::RefreshPeople),
                "property people lost pause caller"
            );
            let view = s
                .host
                .presentation
                .presenter
                .as_ref()
                .context("property presenter absent")?;
            people
                .validate(&person_refresh::Live {
                    world: &s.simulation.vm.world,
                    size: 64,
                    level: view.view.floor.level,
                })
                .map_err(anyhow::Error::msg)?;
        }
        ensure!(
            !c.panels
                || c.native.request()
                    == Some(native::Request::Panels {
                        panel: self.binding.panel.panel,
                        state: self.binding.panel.state,
                    }),
            "property panel lost its native caller"
        );
        ensure!(
            !c.funds_started || matches!(c.native.request(), Some(native::Request::SetFunds(_))),
            "property formatter lost its funds setter"
        );
        Ok(())
    }
}
fn validate_selection_view_refresh(
    session: &Session,
    caller: &dispatch::Service,
    token: Option<sims_sim::lot_selection::NotificationViewRefresh>,
) -> Result<()> {
    let Some(token) = token else {
        if session.simulation.selection_work_pending() {
            let notifications = session
                .host
                .presentation
                .notifications
                .as_ref()
                .context("property selection notifications absent")?;
            notifications.validate(&session.host.presentation, &session.simulation)?;
            validate_tokenless_selection_notification(
                session.simulation.pending_selection_notification(),
                notifications,
                caller,
            )?;
        }
        return Ok(());
    };
    let notifications = session
        .host
        .presentation
        .notifications
        .as_ref()
        .context("property selection notifications absent")?;
    notifications.validate(&session.host.presentation, &session.simulation)?;
    validate_selection_view_refresh_caller(notifications, caller, token.notification())?;
    session
        .simulation
        .validate_selection_notification_view_refresh(token)
        .map_err(anyhow::Error::msg)
}
fn validate_tokenless_selection_notification(
    pending: Option<(sims_sim::lot_selection::Notification, bool)>,
    notifications: &notifications::State,
    caller: &dispatch::Service,
) -> Result<()> {
    let (notification, started) =
        pending.context("property refresh has no selection notification parent")?;
    ensure!(
        started
            && matches!(
                notification.owner,
                sims_sim::lot_selection::NotificationOwner::Deletion { .. }
            ),
        "property refresh has no tokenless deletion notification parent"
    );
    validate_selection_view_refresh_caller(notifications, caller, notification)
}
fn validate_selection_view_refresh_caller(
    notifications: &notifications::State,
    caller: &dispatch::Service,
    notification: sims_sim::lot_selection::Notification,
) -> Result<()> {
    notifications
        .dispatch
        .validate()
        .map_err(anyhow::Error::msg)?;
    ensure!(
        caller.invocation.handler == Receiver::Interface
            && caller.request == Listener::UiRefresh
            && caller.invocation.event == sims_vm::selection::SIGNAL
            && caller.invocation.value == notification.payload as u32
            && notifications.dispatch.depth() == 1
            && notifications.pending() == Some(&notifications::Service::Listener(caller.clone()))
            && notifications.dispatch.pending().as_ref() == Some(caller)
            && notifications.dispatch.root_message()
                == Some((sims_vm::selection::SIGNAL, notification.payload as u32)),
        "property refresh selection notification origin differs"
    );
    Ok(())
}

impl Session {
    ///Explicitly register the replacement Interface listener. The supplied ID
    ///is a new registry lifetime; existing order/identities are left intact.
    ///The caller chooses this profile change, never the restore path.
    pub fn bind_property_listener(&mut self, id: u32) -> Result<()> {
        ensure!(
            !self.house_notification_pending()
                && !self.publication_pending()
                && self.property_state()?.call.is_none(),
            "Interface registration requires an idle boundary"
        );
        let n = self
            .host
            .presentation
            .notifications
            .as_mut()
            .context("house listeners absent")?;
        ensure!(
            id != 0
                && !n.dispatch.registry.nodes.contains_key(&id)
                && !n
                    .dispatch
                    .registry
                    .nodes
                    .values()
                    .any(|node| node.handler == Receiver::Interface),
            "Interface listener already present or identity occupied"
        );
        n.dispatch
            .registry
            .construct(id)
            .map_err(anyhow::Error::msg)?;
        n.dispatch
            .registry
            .bind(id, Receiver::Interface)
            .map_err(anyhow::Error::msg)
    }
    /// Install a new frontend owner only at an idle boundary. Restore leaves
    /// historical sessions unbound; it never upgrades notification registries.
    pub fn bind_property_refresh(&mut self, binding: Binding) -> Result<()> {
        binding.validate()?;
        ensure!(
            self.host.presentation.property_refresh.is_none()
                && !self.house_notification_pending()
                && !self.publication_pending()
                && !self.player_input_pending()
                && !self.camera_command_pending()
                && !self.transport_pending(),
            "property refresh already bound or busy"
        );
        self.host.presentation.camera_properties()?;
        ensure!(
            self.host.presentation.transport.is_some()
                && self.host.presentation.move_tool.is_some()
                && self.host.presentation.publication.is_some(),
            "property refresh owners absent"
        );
        self.host.presentation.property_refresh = Some(State {
            binding,
            serial: 0,
            undo_available: 0,
            call: None,
        });
        Ok(())
    }
    pub fn begin_property_refresh(&mut self, caller: &dispatch::Service) -> Result<()> {
        ensure!(
            caller.invocation.handler == Receiver::Interface
                && caller.request == Listener::UiRefresh
                && self
                    .host
                    .presentation
                    .notifications
                    .as_ref()
                    .and_then(|n| n.pending())
                    == Some(&notifications::Service::Listener(caller.clone())),
            "property notification differs"
        );
        let p = self.property_state()?;
        ensure!(p.call.is_none(), "property refresh already running");
        let serial = p
            .serial
            .checked_add(1)
            .context("property serial overflow")?;
        let native = native::Call::new(p.binding.panel.state, true).map_err(anyhow::Error::msg)?;
        let selection_view_refresh = if self.simulation.selection_work_pending() {
            let (notification, started) = self
                .simulation
                .pending_selection_notification()
                .context("property refresh overlaps unrelated selection work")?;
            ensure!(started, "property selection notification has not started");
            let notifications = self.host.presentation.notifications.as_ref().unwrap();
            notifications.validate(&self.host.presentation, &self.simulation)?;
            validate_selection_view_refresh_caller(notifications, caller, notification)?;
            match notification.owner {
                sims_sim::lot_selection::NotificationOwner::Selection { .. } => Some(
                    self.simulation
                        .selection_notification_view_refresh(notification, serial)
                        .map_err(anyhow::Error::msg)?,
                ),
                sims_sim::lot_selection::NotificationOwner::Deletion { .. } => {
                    validate_tokenless_selection_notification(
                        Some((notification, started)),
                        notifications,
                        caller,
                    )?;
                    None
                }
            }
        } else {
            None
        };
        let p = self
            .host
            .presentation
            .property_refresh
            .as_mut()
            .context("property refresh unbound")?;
        p.call = Some(Execution {
            tick: self.simulation.vm.tick,
            caller: caller.clone(),
            native,
            pause: None,
            people: None,
            panels: false,
            funds_started: false,
            selection_view_refresh,
        });
        p.serial = serial;
        Ok(())
    }
    pub fn acknowledge_property_text(&mut self, request: native::Request) -> Result<()> {
        ensure!(
            matches!(
                request,
                native::Request::Clock { .. } | native::Request::SetFunds(_)
            ),
            "non-text property service cannot be acknowledged as text"
        );
        self.property_state()?.validate(self)?;
        let call = self
            .host
            .presentation
            .property_refresh
            .as_mut()
            .unwrap()
            .call
            .as_mut()
            .unwrap();
        call.native
            .acknowledge(request, native::Reply::Done)
            .map_err(anyhow::Error::msg)?;
        call.funds_started = false;
        Ok(())
    }
    fn property_state(&self) -> Result<&State> {
        self.host
            .presentation
            .property_refresh
            .as_ref()
            .context("property refresh absent")
    }
    pub fn acknowledge_property_window(&mut self, value: u32) -> Result<()> {
        self.property_state()?.validate(self)?;
        self.host
            .presentation
            .property_refresh
            .as_mut()
            .unwrap()
            .call
            .as_mut()
            .context("property caller absent")?
            .pause
            .as_mut()
            .context("property pause absent")?
            .acknowledge(transport::Request::WindowPause(value))
            .map_err(anyhow::Error::msg)
    }
    pub fn claim_property_panels(&mut self, owner: PanelOwner) -> Result<u64> {
        self.property_state()?.validate(self)?;
        let p = self.host.presentation.property_refresh.as_mut().unwrap();
        let c = p.call.as_mut().context("property caller absent")?;
        ensure!(
            !c.panels
                && owner == p.binding.panel
                && c.native.request()
                    == Some(native::Request::Panels {
                        panel: owner.panel,
                        state: owner.state
                    }),
            "property panel already claimed or caller differs"
        );
        c.panels = true;
        Ok(p.serial)
    }
    pub fn complete_property_panels(
        &mut self,
        serial: u64,
        owner: PanelOwner,
        completed: &sims_vm::interface::panel_dispatch::Update,
    ) -> Result<()> {
        self.property_state()?.validate(self)?;
        completed.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            completed.complete()
                && completed.matches(owner.state)
                && self.property_state()?.panel_owner() == Some((serial, owner)),
            "property panel return differs"
        );
        let c = self
            .host
            .presentation
            .property_refresh
            .as_mut()
            .unwrap()
            .call
            .as_mut()
            .unwrap();
        c.native
            .acknowledge(
                native::Request::Panels {
                    panel: owner.panel,
                    state: owner.state,
                },
                native::Reply::Done,
            )
            .map_err(anyhow::Error::msg)?;
        c.panels = false;
        Ok(())
    }
    pub fn complete_property_refresh(&mut self, caller: &dispatch::Service) -> Result<()> {
        self.property_state()?.validate(self)?;
        let c = self
            .property_state()?
            .call
            .as_ref()
            .context("property caller absent")?;
        ensure!(
            &c.caller == caller
                && c.native.complete()
                && c.pause.is_none()
                && c.people.is_none()
                && !c.panels,
            "property refresh has unfinished children"
        );
        // The device returns to its actual house/camera caller immediately
        // after this release. That caller performs the notification return.
        self.host
            .presentation
            .property_refresh
            .as_mut()
            .unwrap()
            .call = None;
        Ok(())
    }
    pub fn advance_property_refresh(&mut self, budget: usize) -> Result<Progress> {
        ensure!(budget > 0, "property refresh requires a work budget");
        self.property_state()?.validate(self)?;
        let mut p = self.host.presentation.property_refresh.take().unwrap();
        let result = self.advance_property_refresh_inner(&mut p, budget);
        // Earlier native stores survive a subsequent unsupported service/error.
        self.host.presentation.property_refresh = Some(p);
        result
    }
    fn advance_property_refresh_inner(&mut self, p: &mut State, budget: usize) -> Result<Progress> {
        for _ in 0..budget {
            let c = p.call.as_mut().context("property caller absent")?;
            let ui = self.host.presentation.camera_properties()?;
            let mut fields = native::Fields {
                undo_available: p.undo_available,
                zoom: ui.zoom,
                rotation: ui.rotation,
                floor: ui.floor,
            };
            let m = self
                .host
                .presentation
                .move_tool
                .as_ref()
                .context("property history absent")?;
            let count = m
                .history
                .store
                .vector(List::Undo(m.tool.clipboard))
                .map_err(anyhow::Error::msg)?
                .len;
            let undo_vector_bytes = u32::try_from(count)?
                .checked_mul(4)
                .context("undo vector span overflow")?;
            let global = |id| {
                self.simulation
                    .vm
                    .world
                    .globals
                    .get(&id)
                    .copied()
                    .context("property global absent")
            };
            let input = native::Inputs {
                undo_vector_bytes,
                clock: [global(0)?, global(5)?],
                view: p.binding.view,
                panel: Some(p.binding.panel.panel),
            };
            let request = c
                .native
                .advance(
                    &mut self
                        .host
                        .presentation
                        .transport
                        .as_mut()
                        .context("property CPState absent")?
                        .controls,
                    &mut fields,
                    input,
                )
                .map_err(anyhow::Error::msg)?;
            p.undo_available = fields.undo_available;
            self.host.presentation.edit_camera_properties(|ui| {
                ui.zoom = fields.zoom;
                ui.rotation = fields.rotation;
                ui.floor = fields.floor;
            })?;
            let Some(request) = request else {
                ensure!(c.native.complete(), "property caller has not returned");
                return Ok(Progress::Complete);
            };
            use native::{Query as Q, Reply as V, Request as R};
            let reply = match request {
                R::Renderer => V::Number(p.binding.renderer),
                R::Clock {
                    renderer,
                    state,
                    words,
                } => {
                    ensure!(
                        renderer == p.binding.renderer && state == p.binding.panel.state,
                        "property clock identities differ"
                    );
                    let language = self
                        .program
                        .text_language
                        .context("property clock language absent")?;
                    let bytes = sims_vm::dialog::format_time(
                        language,
                        i32::from(words[0]),
                        words[1] as i32 as u32,
                        true,
                        true,
                    )
                    .into_bytes();
                    return Ok(Progress::Text {
                        request,
                        language: language.try_into()?,
                        bytes,
                    });
                }
                R::Funds => {
                    let g = &self.simulation.vm.world.globals;
                    let value = if *g.get(&32).context("property lot kind absent")? == 1 {
                        999_999
                    } else {
                        self.simulation
                            .vm
                            .world
                            .budget
                            .as_ref()
                            .context("property budget absent")?
                            .balance
                    };
                    V::Number(value as u32)
                }
                R::SetFunds(value) => {
                    //4cecf5 dirties before its renderer formatter callback.
                    if !c.funds_started {
                        self.host
                            .presentation
                            .transport
                            .as_mut()
                            .unwrap()
                            .controls
                            .mark_dirty(0x4000);
                        c.funds_started = true;
                    }
                    let locale = self
                        .host
                        .notifications
                        .as_ref()
                        .context("property locale absent")?
                        .saved
                        .locale;
                    return Ok(Progress::Text {
                        request,
                        language: locale.language.try_into()?,
                        bytes: locale.format(i64::from(value.min(9_999_999))),
                    });
                }
                R::Camera { view, query } => {
                    ensure!(view == p.binding.view, "property camera identity differs");
                    let v = self
                        .host
                        .presentation
                        .presenter
                        .as_ref()
                        .context("property presenter absent")?;
                    V::Number(match query {
                        Q::Zoom => v.view.zoom,
                        Q::Rotation => v.view.display_rotation,
                        Q::Floor => v.view.floor.level as u32,
                        Q::WallsDown => u32::from(v.view.floor.all_walls),
                        Q::Cutaway => u32::from(v.view.floor.visibility.enabled),
                        Q::Roof => u32::from(
                            v.camera
                                .as_ref()
                                .context("property camera absent")?
                                .settings
                                .roof
                                .context("property roof unknown")?,
                        ),
                    })
                }
                R::SelectedTool => {
                    self.selected_tool_camera_changed()?;
                    V::Done
                }
                R::Availability => V::Done, //46fb64 is a literal RET.
                R::Speed => V::Number(i32::from(
                    *self
                        .simulation
                        .vm
                        .world
                        .globals
                        .get(&16)
                        .context("property speed absent")?,
                ) as u32),
                R::SetSpeed(value) => {
                    self.host
                        .presentation
                        .transport
                        .as_mut()
                        .unwrap()
                        .controls
                        .set_speed(value)
                        .map_err(anyhow::Error::msg)?;
                    V::Done
                }
                R::Paused => V::Number(u32::from(
                    *self
                        .simulation
                        .vm
                        .world
                        .globals
                        .get(&17)
                        .context("property pause absent")?
                        != 0,
                )),
                R::SetPaused(value) => {
                    let pause = c
                        .pause
                        .get_or_insert_with(|| transport::SetPause::new(u32::from(value)));
                    match pause
                        .advance(&mut self.host.presentation.transport.as_mut().unwrap().controls)
                        .map_err(anyhow::Error::msg)?
                    {
                        Some(transport::Request::WindowPause(value)) => {
                            return Ok(Progress::WindowPause(value));
                        }
                        Some(transport::Request::RefreshPeople) => {
                            let people = c.people.get_or_insert_with(Default::default);
                            let level = self
                                .host
                                .presentation
                                .presenter
                                .as_ref()
                                .context("property view absent")?
                                .view
                                .floor
                                .level;
                            if self.advance_camera_people_with_selection_notification(
                                people,
                                level,
                                c.selection_view_refresh,
                            )? {
                                c.people = None;
                                pause
                                    .acknowledge(transport::Request::RefreshPeople)
                                    .map_err(anyhow::Error::msg)?;
                            }
                            continue;
                        }
                        None => {
                            ensure!(pause.complete(), "property pause has not returned");
                            c.pause = None;
                            V::Done
                        }
                    }
                }
                R::SetWalls { value, deferred } => {
                    ensure!(deferred == 1, "property walls unexpectedly call camera");
                    let p = &mut self.host.presentation;
                    ensure!(
                        p.publication.is_some() || p.frontend_context.is_some(),
                        "property wall owner absent"
                    );
                    if let Some(frame) = &mut p.publication {
                        frame.set_deferred_walls(value);
                    }
                    if let Some(fields) = &mut p.frontend_context {
                        fields.walls = value as i32;
                    }
                    self.host
                        .presentation
                        .transport
                        .as_mut()
                        .unwrap()
                        .controls
                        .mark_dirty(0x80);
                    V::Done
                }
                R::SelectedPerson => V::Number(self.selected_portrait_index()? as u32),
                R::Panels { panel, state } => {
                    ensure!(
                        panel == p.binding.panel.panel && state == p.binding.panel.state,
                        "property panel identities differ"
                    );
                    return Ok(Progress::Panels(p.binding.panel));
                }
            };
            c.native
                .acknowledge(request, reply)
                .map_err(anyhow::Error::msg)?;
        }
        Ok(Progress::Pending)
    }
}

#[cfg(test)]
#[path = "property_refresh/selection_notification_tests.rs"]
mod selection_notification_tests;
