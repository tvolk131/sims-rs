//!Actual replacement window fields, pause indicator and shared timer registry.
//!4d31b7 is resumable across each backend write; timer dispatch is a separate
//!application frame, never a simulation tick or a fabricated service reply.
use super::{Device, Saved};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_runtime::original_session::Session;
use sims_vm::{
    interface::{
        mode_change::Request as ModeRequest,
        window_pause::{Change, Fields, Request},
    },
    ui_effect::widgets::timers::{Entry, Registry},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    owner: u32,
    fields: Fields,
    timers: Registry,
    indicator_visible: bool,
    clock: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    clock_remainder: u32,
    call: Option<Call>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Call {
    serial: u64,
    change: Change,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transport: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    property: Option<u32>,
}
fn is_zero(value: &u32) -> bool {
    *value == 0
}
impl Window {
    pub(super) fn panel_inputs(&self) -> (u32, u8, u32) {
        (self.owner, self.fields.paused, self.clock)
    }
    pub(super) fn timers_mut(&mut self) -> &mut Registry {
        &mut self.timers
    }
    ///An explicit new replacement window, not inferred original saved state.
    pub fn new(owner: u32, indicator: Option<u32>, interval: u32, paused: u8, clock: u32) -> Self {
        Self {
            owner,
            fields: Fields {
                paused,
                timer_active: 0,
                interval,
                indicator,
            },
            timers: Registry {
                entries: vec![],
                first: 0,
                second: 0,
            },
            indicator_visible: paused != 0 && indicator.is_some(),
            clock,
            clock_remainder: 0,
            call: None,
        }
    }
    pub fn fields(&self) -> &Fields {
        &self.fields
    }
    pub fn timers(&self) -> &Registry {
        &self.timers
    }
    pub fn indicator_visible(&self) -> bool {
        self.indicator_visible
    }
    pub fn pending(&self) -> bool {
        self.call.is_some()
    }
    pub fn clock(&self) -> u32 {
        self.clock
    }
    fn validate_clock(&self) -> Result<()> {
        ensure!(
            self.clock_remainder < 1_000_000,
            "mode window submillisecond clock invalid"
        );
        Ok(())
    }
    fn advance_clock(&mut self, elapsed: std::time::Duration) -> Result<()> {
        self.validate_clock()?;
        let nanos = elapsed.as_nanos() + u128::from(self.clock_remainder);
        self.clock = self.clock.wrapping_add((nanos / 1_000_000) as u32);
        self.clock_remainder = (nanos % 1_000_000) as u32;
        Ok(())
    }
    fn advance_pause(&mut self) -> Result<bool> {
        let call = self.call.as_mut().context("pause window has no caller")?;
        let request = call
            .change
            .advance(&mut self.fields)
            .map_err(anyhow::Error::msg)?;
        if let Some(request) = request {
            match request {
                Request::Unregister => {
                    self.timers.remove(self.owner).map_err(anyhow::Error::msg)?
                }
                Request::Register(interval) => self
                    .timers
                    .register(Entry {
                        owner: self.owner,
                        interval,
                        deadline: self.clock.wrapping_add(interval),
                        argument: 0,
                    })
                    .map_err(anyhow::Error::msg)?,
                Request::Visible { indicator, show } => {
                    ensure!(
                        self.fields.indicator == Some(indicator),
                        "pause indicator owner differs"
                    );
                    self.indicator_visible = show;
                }
            }
            call.change
                .acknowledge(&request)
                .map_err(anyhow::Error::msg)?;
            Ok(false)
        } else {
            ensure!(call.change.complete(), "pause window has not returned");
            Ok(true)
        }
    }
}

pub(super) fn validate(session: &Session, saved: &Saved) -> Result<()> {
    let tool = session.host.presentation.tool_selection.as_ref();
    let Some(window) = &saved.mode_window else {
        ensure!(
            tool.is_none_or(|t| t.mode_window_owner().is_none()
                && t.mode_context().is_none_or(|c| c.window_pause().is_none())),
            "mode window backend lost its completed/pending caller"
        );
        return Ok(());
    };
    window.validate_clock()?;
    ensure!(
        window.owner != 0 && window.fields.indicator != Some(0),
        "mode window identities invalid"
    );
    let tool = tool.context("mode window lost tool owner")?;
    let context = tool.mode_context().context("mode window context absent")?;
    let request = tool.mode_request();
    if let Some(call) = &window.call {
        call.change.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            call.transport.is_none() || call.property.is_none(),
            "pause window has concurrent callers"
        );
        if let Some(value) = call.property {
            ensure!(
                call.serial == 0
                    && session
                        .host
                        .presentation
                        .property_refresh
                        .as_ref()
                        .and_then(|p| p.window_request())
                        == Some(value)
                    && call.change.matches(value as u8, 0),
                "pause presentation lost its property caller"
            );
        } else if let Some(value) = call.transport {
            ensure!(
                call.serial == 0
                    && session.transport_window_request() == Some(value)
                    && call.change.matches(value as u8, 0),
                "pause presentation lost its transport caller"
            );
        } else {
            ensure!(
                matches!(request,
            Some(ModeRequest::WindowPause { paused, show_overlay }) if call.change.matches(*paused,*show_overlay))
                    && tool.mode_window_owner() == Some(call.serial),
                "pause presentation lost its mode caller"
            );
        }
    } else {
        ensure!(
            tool.mode_window_owner().is_none(),
            "mode caller lost its window continuation"
        );
    }
    if window.call.is_none()
        && session.transport_window_request().is_none()
        && session
            .host
            .presentation
            .property_refresh
            .as_ref()
            .and_then(|p| p.window_request())
            .is_none()
        && !matches!(request, Some(ModeRequest::WindowPause { .. }))
    {
        let controls = &session
            .host
            .presentation
            .transport
            .as_ref()
            .context("mode CPState absent")?
            .controls;
        let expected = if controls.mode == 2 {
            controls.paused
        } else {
            context.window_pause().map_or(controls.paused, |a| a[0])
        };
        ensure!(
            window.fields.paused == expected,
            "mode window pause differs from completed caller"
        );
    }
    ensure!(
        saved.transport_pause == Some(window.fields.paused),
        "pause widget differs from actual mode window"
    );
    // During callbacks the flag write deliberately trails registration/removal.
    if window.call.is_none() {
        ensure!(
            (window.fields.timer_active != 0)
                == window
                    .timers
                    .entries
                    .iter()
                    .any(|e| e.owner == window.owner),
            "window timer flag differs from registry"
        );
    }
    Ok(())
}

impl Device {
    pub(super) fn advance_property_window(
        &mut self,
        session: &Session,
        value: u32,
    ) -> Result<bool> {
        validate(session, &self.saved)?;
        ensure!(
            session
                .host
                .presentation
                .property_refresh
                .as_ref()
                .and_then(|p| p.window_request())
                == Some(value),
            "property window caller differs"
        );
        let window = self
            .saved
            .mode_window
            .as_mut()
            .context("property pause window absent")?;
        if window.call.is_none() {
            window.call = Some(Call {
                serial: 0,
                change: Change::new(value as u8, 0),
                transport: None,
                property: Some(value),
            });
        }
        ensure!(
            window.call.as_ref().unwrap().property == Some(value),
            "pause window belongs to another caller"
        );
        let done = window.advance_pause()?;
        self.saved.transport_pause = Some(window.fields.paused);
        if done {
            window.call = None;
        }
        validate(session, &self.saved)?;
        Ok(done)
    }
    ///4cee4a calls the SAME4d31b7 window with overlay=0; mode transitions pass
    ///their own overlay argument. Retain both callers across saved UI slices.
    pub(super) fn advance_transport_window(
        &mut self,
        session: &Session,
        value: u32,
    ) -> Result<bool> {
        validate(session, &self.saved)?;
        ensure!(
            session.transport_window_request() == Some(value),
            "transport window caller differs"
        );
        let window = self
            .saved
            .mode_window
            .as_mut()
            .context("pause window absent")?;
        if window.call.is_none() {
            window.call = Some(Call {
                serial: 0,
                change: Change::new(value as u8, 0),
                transport: Some(value),
                property: None,
            });
        }
        ensure!(
            window.call.as_ref().unwrap().transport == Some(value),
            "pause window belongs to another caller"
        );
        let done = window.advance_pause()?;
        self.saved.transport_pause = Some(window.fields.paused);
        if done {
            window.call = None;
        }
        validate(session, &self.saved)?;
        Ok(done)
    }
    pub fn bind_mode_window(&mut self, session: &Session, window: Window) -> Result<()> {
        ensure!(
            self.saved.mode_window.is_none() && !window.pending(),
            "mode window already bound or busy"
        );
        let mut next = self.saved.clone();
        next.mode_window = Some(window);
        validate(session, &next)?;
        self.saved = next;
        Ok(())
    }
    pub fn mode_window(&self) -> Option<&Window> {
        self.saved.mode_window.as_ref()
    }
    ///Sample the frontend monotonic clock, not the household tick. The sample
    ///is consumed by native registration's wrapping deadline calculation.
    pub fn set_mode_clock(&mut self, now: u32) -> Result<()> {
        let window = self
            .saved
            .mode_window
            .as_mut()
            .context("mode window absent")?;
        window.clock = now;
        window.clock_remainder = 0;
        Ok(())
    }
    pub fn advance_mode_clock(&mut self, elapsed: std::time::Duration) -> Result<()> {
        self.saved
            .mode_window
            .as_mut()
            .context("mode window absent")?
            .advance_clock(elapsed)
    }
    pub(super) fn advance_mode_window(
        &mut self,
        session: &mut Session,
        paused: u8,
        overlay: u8,
    ) -> Result<()> {
        validate(session, &self.saved)?;
        let window = self
            .saved
            .mode_window
            .as_mut()
            .context("mode window not bound")?;
        if window.call.is_none() {
            let serial = session.claim_mode_window_pause(paused, overlay)?;
            window.call = Some(Call {
                serial,
                change: Change::new(paused, overlay),
                transport: None,
                property: None,
            });
        }
        let owned = window.call.as_ref().unwrap();
        ensure!(
            owned.transport.is_none()
                && owned.property.is_none()
                && owned.change.matches(paused, overlay),
            "window pause arguments changed"
        );
        let serial = owned.serial;
        let done = window.advance_pause()?;
        self.saved.transport_pause = Some(window.fields.paused);
        if done {
            session.acknowledge_mode_window_pause(serial, paused, overlay)?;
            window.call = None;
        }
        validate(session, &self.saved)
    }
}

#[cfg(test)]
mod clock_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn native_mode_clock_fractional_carry_wrap_and_cold_restore() -> Result<()> {
        let mut window = Window::new(9000, Some(9001), 80, 1, u32::MAX);
        window.advance_clock(Duration::from_nanos(750_000))?;
        ensure!(window.clock() == u32::MAX && window.clock_remainder == 750_000);
        window = serde_json::from_slice(&serde_json::to_vec(&window)?)?;
        window.validate_clock()?;
        window.advance_clock(Duration::from_nanos(250_001))?;
        ensure!(window.clock() == 0 && window.clock_remainder == 1);
        window.advance_clock(Duration::from_millis(u64::from(u32::MAX) + 3))?;
        ensure!(window.clock() == 2 && window.clock_remainder == 1);
        ensure!(window.fields.paused == 1 && window.timers.entries.is_empty());
        Ok(())
    }

    #[test]
    fn native_mode_clock_legacy_zero_and_invalid_remainder() -> Result<()> {
        let window = Window::new(9000, None, 80, 0, 1234);
        let mut saved = serde_json::to_value(&window)?;
        ensure!(saved.get("clock_remainder").is_none());
        let restored: Window = serde_json::from_value(saved.clone())?;
        restored.validate_clock()?;
        ensure!(restored == window);
        saved["clock_remainder"] = 1_000_000.into();
        let mut invalid: Window = serde_json::from_value(saved)?;
        ensure!(invalid.validate_clock().is_err());
        let before = invalid.clone();
        ensure!(invalid.advance_clock(Duration::from_millis(1)).is_err());
        ensure!(invalid == before);
        Ok(())
    }
}
