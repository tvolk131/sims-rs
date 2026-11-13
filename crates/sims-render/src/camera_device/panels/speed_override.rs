use super::*;
use sims_vm::commander::house::{Receiver, Request as Listener, dispatch as notification};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Notice {
    pub(super) caller: notification::Service,
    pub(super) call: buttons::Call,
}

fn active(caller: &notification::Service) -> Result<bool> {
    let Listener::SpeedOverride { active } = caller.request else {
        anyhow::bail!("speed override notification request differs");
    };
    ensure!(
        caller.invocation.handler == Receiver::Interface
            && caller.invocation.event as u16 == 0x109
            && active == (caller.invocation.value != 0),
        "speed override notification identity differs"
    );
    Ok(active)
}

impl Widgets {
    pub(super) fn validate_speed_override(&self) -> Result<()> {
        let Some(notice) = &self.speed_override else {
            return Ok(());
        };
        let active = active(&notice.caller)?;
        notice.call.validate().map_err(anyhow::Error::msg)?;
        if let Some(request) = notice.call.request() {
            let valid = match request {
                buttons::Request::Register {
                    manager,
                    view,
                    interval,
                    argument,
                } => {
                    manager == self.owners.manager
                        && view == self.owners.controls
                        && interval == 500
                        && argument == 0
                }
                buttons::Request::Unregister { manager, view } => {
                    manager == self.owners.manager && view == self.owners.controls
                }
                buttons::Request::Selected { widget, value } => {
                    widget == self.fields.widget(control::Widget::Ultra) && value == 0
                }
                buttons::Request::Auxiliary { .. } => false,
            };
            ensure!(
                valid,
                "speed override pending widget or timer owner differs"
            );
        }
        ensure!(
            self.buttons.alternate == u8::from(active)
                && notice
                    .call
                    .matches(self.owners.controls, buttons::Kind::Dispatch(2))
                && self.call.dispatch.complete()
                && self.call.control.is_none()
                && self.call.buttons.is_none()
                && self.call.selection.is_none(),
            "speed override lost its ViewControl owner"
        );
        Ok(())
    }

    fn begin_speed_override(
        &mut self,
        caller: &notification::Service,
        panels_owned: bool,
    ) -> Result<()> {
        self.validate()?;
        let active = active(caller)?;
        ensure!(
            !panels_owned
                && self.speed_override.is_none()
                && self.call.dispatch.complete()
                && self.call.control.is_none()
                && self.call.buttons.is_none()
                && self.call.button.is_none()
                && self.call.selection.is_none(),
            "speed override overlaps panel publication"
        );
        self.buttons.alternate = u8::from(active);
        self.speed_override = Some(Notice {
            caller: caller.clone(),
            call: buttons::Call::new(self.owners.controls, buttons::Kind::Dispatch(2)),
        });
        Ok(())
    }

    pub(super) fn advance_speed_override_notice(
        &mut self,
        state: &State,
        window: &mut mode::Window,
    ) -> Result<()> {
        let (_, blocked, now) = window.panel_inputs();
        let input = buttons::Inputs {
            mode: state.mode,
            paused: state.paused,
            speed: state.speed,
            window_blocked: blocked,
            timer_owner: self.owners.manager,
            pause: self.owners.pause,
            ultra: self.fields.widget(control::Widget::Ultra),
        };
        let notice = self
            .speed_override
            .as_mut()
            .context("speed override caller absent")?;
        match notice
            .call
            .advance(&mut self.buttons, input)
            .map_err(anyhow::Error::msg)?
        {
            Some(
                request @ buttons::Request::Register {
                    manager,
                    view,
                    interval,
                    argument,
                },
            ) => {
                ensure!(
                    manager == self.owners.manager && view == self.owners.controls,
                    "speed override timer owner differs"
                );
                window
                    .timers_mut()
                    .register(Entry {
                        owner: view,
                        interval,
                        argument,
                        deadline: now.wrapping_add(interval),
                    })
                    .map_err(anyhow::Error::msg)?;
                notice
                    .call
                    .acknowledge(request)
                    .map_err(anyhow::Error::msg)?;
            }
            Some(request @ buttons::Request::Unregister { manager, view }) => {
                ensure!(
                    manager == self.owners.manager && view == self.owners.controls,
                    "speed override timer owner differs"
                );
                window
                    .timers_mut()
                    .remove(view)
                    .map_err(anyhow::Error::msg)?;
                notice
                    .call
                    .acknowledge(request)
                    .map_err(anyhow::Error::msg)?;
            }
            Some(request) => self.start_button(request)?,
            None => {
                ensure!(
                    notice.call.complete(),
                    "speed override button transition returned early"
                );
            }
        }
        Ok(())
    }
}

impl Device {
    pub(crate) fn advance_speed_override(
        &mut self,
        session: &mut Session,
        caller: &notification::Service,
        budget: usize,
    ) -> Result<bool> {
        ensure!(budget > 0, "speed override needs a work budget");
        validate(session, &self.saved)?;
        active(caller)?;
        let panels_owned = session
            .host
            .presentation
            .publication
            .as_ref()
            .and_then(|state| state.panel_owner())
            .is_some()
            || session
                .host
                .presentation
                .property_refresh
                .as_ref()
                .and_then(|state| state.panel_owner())
                .is_some();
        ensure!(!panels_owned, "speed override overlaps panel publication");
        ensure!(
            session
                .host
                .presentation
                .notifications
                .as_ref()
                .and_then(|state| state.pending())
                == Some(
                    &sims_runtime::original_session::notifications::Service::Listener(
                        caller.clone()
                    )
                ),
            "speed override lost its exact notification owner"
        );
        let widgets = self
            .saved
            .panel_widgets
            .as_mut()
            .context("speed override ViewControl absent")?;
        let window = self
            .saved
            .mode_window
            .as_mut()
            .context("speed override window absent")?;
        let controls = &session
            .host
            .presentation
            .transport
            .as_ref()
            .context("speed override CPState absent")?
            .controls;
        if let Some(notice) = &widgets.speed_override {
            ensure!(
                notice.caller == *caller,
                "speed override notification caller changed"
            );
        } else {
            widgets.begin_speed_override(caller, panels_owned)?;
        }
        for _ in 0..budget {
            if widgets
                .speed_override
                .as_ref()
                .is_some_and(|notice| notice.call.complete())
            {
                return Ok(true);
            }
            if widgets.call.button.is_some() {
                widgets.advance_button()?;
            } else {
                widgets.advance_speed_override_notice(controls, window)?;
            }
            widgets.validate()?;
            if widgets
                .speed_override
                .as_ref()
                .is_some_and(|notice| notice.call.complete())
            {
                validate(session, &self.saved)?;
                return Ok(true);
            }
        }
        validate(session, &self.saved)?;
        Ok(false)
    }

    pub(crate) fn finish_speed_override(
        &mut self,
        service: &sims_runtime::original_session::notifications::Service,
    ) -> Result<()> {
        let sims_runtime::original_session::notifications::Service::Listener(caller) = service
        else {
            return Ok(());
        };
        if !matches!(caller.request, Listener::SpeedOverride { .. }) {
            return Ok(());
        }
        let widgets = self
            .saved
            .panel_widgets
            .as_mut()
            .context("speed override ViewControl absent")?;
        let notice = widgets
            .speed_override
            .as_ref()
            .context("speed override caller absent")?;
        ensure!(
            notice.caller == *caller && notice.call.complete() && widgets.call.button.is_none(),
            "speed override child has not returned"
        );
        widgets.speed_override = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_vm::commander::Invocation;

    fn caller(value: u32) -> notification::Service {
        notification::Service {
            invocation: Invocation {
                recipient: 8,
                handler: Receiver::Interface,
                event: 0x109,
                value,
            },
            request: Listener::SpeedOverride { active: value != 0 },
        }
    }

    fn widgets() -> Result<Widgets> {
        let owners = Owners {
            panel: 10000,
            state: 10001,
            controls: 10002,
            lot: 10003,
            manager: 10004,
            panels: std::array::from_fn(|index| 10010 + index as u32),
            widgets: std::array::from_fn(|index| 10100 + index as u32),
            pause: 10200,
        };
        let mut widgets = Widgets::fresh(owners, 0, 1);
        let input = dispatch::Inputs {
            mode: 2,
            dirty: 0,
            controls: Some(widgets.owners.controls),
            build: None,
            buy: None,
            people: None,
            options: None,
            camera: None,
        };
        while let Some(request) = widgets
            .call
            .dispatch
            .advance(input)
            .map_err(anyhow::Error::msg)?
        {
            widgets
                .call
                .dispatch
                .acknowledge(request)
                .map_err(anyhow::Error::msg)?;
        }
        widgets.validate()?;
        Ok(widgets)
    }

    #[test]
    fn native_speed_override_real_button_timer_callbacks_restore_without_replay() -> Result<()> {
        fn run(restore: bool) -> Result<(Widgets, mode::Window)> {
            let mut widgets = widgets()?;
            let mut controls = State::constructed(760);
            controls.mode = 2;
            let before = controls.clone();
            let mut window = mode::Window::new(9000, Some(9001), 80, 0, u32::MAX - 100);
            let mut saw_unregister = false;
            let mut saw_changed = false;
            for (index, value) in [1, 256, u32::MAX, 0].into_iter().enumerate() {
                let caller = caller(value);
                widgets.begin_speed_override(&caller, false)?;
                ensure!(widgets.buttons.alternate == u8::from(value != 0));
                for operation in 0..100 {
                    widgets.advance(&mut controls, &mut window)?;
                    widgets.validate()?;
                    saw_unregister |= widgets.buttons.timer_registered != 0
                        && !window
                            .timers()
                            .entries
                            .iter()
                            .any(|entry| entry.owner == widgets.owners.controls);
                    saw_changed |= widgets
                        .call
                        .button
                        .as_ref()
                        .is_some_and(|button| button.pending == Some(button::Request::Changed));
                    if restore {
                        (widgets, window) =
                            serde_json::from_slice(&serde_json::to_vec(&(&widgets, &window))?)?;
                        widgets.validate()?;
                    }
                    if widgets.speed_override.as_ref().unwrap().call.complete() {
                        break;
                    }
                    ensure!(operation < 99, "speed override transition stalled");
                }
                let completed = widgets.clone();
                widgets.advance(&mut controls, &mut window)?;
                ensure!(
                    widgets == completed,
                    "completed speed override replayed its dispatcher"
                );
                let phase = if index % 2 == 0 { 3 } else { 0 };
                ensure!(widgets.button_phase() == phase);
                ensure!((widgets.buttons.timer_registered != 0) == (phase == 3));
                ensure!(window.timers().entries.len() == usize::from(phase == 3));
                if phase == 3 {
                    let entry = &window.timers().entries[0];
                    ensure!(
                        entry.owner == widgets.owners.controls
                            && entry.interval == 500
                            && entry.argument == 0
                            && entry.deadline == 399
                    );
                    ensure!(!widgets.button(control::Widget::Ultra).selected);
                    widgets
                        .widgets
                        .get_mut(&widgets.fields.widget(control::Widget::Ultra))
                        .unwrap()
                        .selected = 1;
                    widgets
                        .widgets
                        .get_mut(&widgets.fields.widget(control::Widget::Ultra))
                        .unwrap()
                        .painted_selected = true;
                } else {
                    ensure!(!widgets.button(control::Widget::Ultra).selected);
                }
                widgets.speed_override = None;
                widgets.validate()?;
                ensure!(controls == before);
                ensure!(widgets.enabled.values().all(|value| *value == 1));
            }
            ensure!(saw_unregister && saw_changed);
            Ok((widgets, window))
        }
        ensure!(run(false)? == run(true)?);
        Ok(())
    }

    #[test]
    fn native_speed_override_rejects_bad_identity_or_concurrent_publication_without_writes()
    -> Result<()> {
        let mut widgets = widgets()?;
        for change in 0..3 {
            let mut caller = caller(1);
            match change {
                0 => caller.invocation.handler = Receiver::Viewer,
                1 => caller.invocation.event = 0xc2,
                _ => caller.invocation.value = 0,
            }
            let before = widgets.clone();
            ensure!(widgets.begin_speed_override(&caller, false).is_err());
            ensure!(widgets == before);
        }
        let before = widgets.clone();
        ensure!(widgets.call.dispatch.complete());
        ensure!(widgets.begin_speed_override(&caller(1), true).is_err());
        ensure!(widgets == before);
        widgets.call.dispatch = dispatch::Update::new(widgets.owners.state);
        let before = widgets.clone();
        ensure!(widgets.begin_speed_override(&caller(1), false).is_err());
        ensure!(widgets == before);
        Ok(())
    }

    #[test]
    fn native_speed_override_restore_rejects_redirected_widget_and_timer_requests() -> Result<()> {
        let mut widgets = widgets()?;
        let mut controls = State::constructed(760);
        controls.mode = 2;
        let mut window = mode::Window::new(9000, Some(9001), 80, 0, 1000);
        widgets.begin_speed_override(&caller(1), false)?;
        let input = buttons::Inputs {
            mode: controls.mode,
            paused: controls.paused,
            speed: controls.speed,
            window_blocked: 0,
            timer_owner: widgets.owners.manager,
            pause: widgets.owners.pause,
            ultra: widgets.fields.widget(control::Widget::Ultra),
        };
        let request = widgets
            .speed_override
            .as_mut()
            .unwrap()
            .call
            .advance(&mut widgets.buttons, input)
            .map_err(anyhow::Error::msg)?;
        ensure!(matches!(request, Some(buttons::Request::Register { .. })));
        widgets.validate()?;
        for (field, value) in [
            ("manager", widgets.owners.manager + 1),
            ("view", widgets.owners.controls + 1),
            ("interval", 501),
            ("argument", 1),
        ] {
            let mut saved = serde_json::to_value(&widgets)?;
            saved["speed_override"]["call"]["pending"]["Register"][field] = value.into();
            ensure!(
                serde_json::from_value::<Widgets>(saved)?
                    .validate()
                    .is_err()
            );
        }
        while !widgets.speed_override.as_ref().unwrap().call.complete() {
            widgets.advance(&mut controls, &mut window)?;
        }
        widgets.speed_override = None;
        widgets.begin_speed_override(&caller(0), false)?;
        while widgets.call.button.is_none() {
            widgets.advance(&mut controls, &mut window)?;
        }
        widgets.validate()?;
        let mut saved = serde_json::to_value(&widgets)?;
        let wrong = widgets.fields.widget(control::Widget::Normal);
        saved["speed_override"]["call"]["pending"]["Selected"]["widget"] = wrong.into();
        saved["call"]["button"]["widget"] = wrong.into();
        ensure!(
            serde_json::from_value::<Widgets>(saved)?
                .validate()
                .is_err()
        );
        let mut saved = serde_json::to_value(&widgets)?;
        saved["speed_override"]["call"]["pending"] =
            serde_json::json!({"Auxiliary":{"widget":widgets.owners.pause,"value":0}});
        ensure!(
            serde_json::from_value::<Widgets>(saved)?
                .validate()
                .is_err()
        );
        Ok(())
    }
}
