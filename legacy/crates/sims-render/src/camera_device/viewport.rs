//! Bind the recovered viewport caller to the replacement drawing context.
//! These identities belong to this backend, not to invented original windows.
//! Its full-scene renderer replaces native retained-strip copying. The original
//! caller still computes that strip and both viewport regions for saved state.
use super::{Device, Frame};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_runtime::original_session::Session;
use sims_sim::lot_presenter::viewport::{self, Function, Invocation, Reply, Request};
use std::collections::BTreeMap;
const VIEW: u32 = 1;
const RENDERER: u32 = 2;
const WINDOW: u32 = 3;
const SURFACE: u32 = 4;
const TARGET: u32 = 5;
const BACKEND: u32 = 6;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Binding {
    state: viewport::State,
    /// Logical viewport currently bound to the actual replacement draw context.
    bound_size: [i32; 2],
}
impl Binding {
    pub fn new(frame: Frame) -> Result<Self> {
        let [_, _, w, h] = frame.input.viewport;
        let width =
            i16::try_from(w).context("replacement viewport width exceeds signed renderer field")?;
        let height =
            i32::try_from(h).context("replacement viewport height exceeds native field")?;
        let size = [i32::from(width), height];
        let rect = [0, 0, size[0], size[1]];
        let result = Self {
            bound_size: size,
            state: viewport::State {
                views: BTreeMap::from([(
                    VIEW,
                    viewport::View {
                        window: WINDOW,
                        renderer: RENDERER,
                        surface: SURFACE,
                        rectangle: rect,
                        retained: [0; 4],
                    },
                )]),
                renderers: BTreeMap::from([(
                    RENDERER,
                    viewport::Renderer {
                        window: WINDOW,
                        width,
                    },
                )]),
                windows: BTreeMap::from([(
                    WINDOW,
                    viewport::Window {
                        origin: [0, 0],
                        size,
                        scale: viewport::scales(size),
                        rectangle: rect,
                        backend: BACKEND,
                        result: BACKEND,
                    },
                )]),
                // The display's full dimensions remain distinct from its active viewport.
                surfaces: BTreeMap::from([(SURFACE, size)]),
                targets: BTreeMap::from([(
                    TARGET,
                    [rect, [0, height, size[0], height.wrapping_mul(2)]],
                )]),
                target: TARGET,
            },
        };
        result.validate(frame)?;
        Ok(result)
    }
    pub fn validate(&self, frame: Frame) -> Result<()> {
        let s = &self.state;
        let [_, _, w, h] = frame.input.viewport;
        ensure!(
            w > 0 && w <= i16::MAX as u32 && h > 0 && h <= i32::MAX as u32,
            "replacement viewport dimensions unavailable"
        );
        ensure!(
            s.views.len() == 1
                && s.renderers.len() == 1
                && s.windows.len() == 1
                && s.surfaces.len() == 1
                && s.targets.len() == 1
                && s.target == TARGET,
            "replacement viewport ownership differs"
        );
        let v = s.views.get(&VIEW).context("viewport view absent")?;
        let r = s
            .renderers
            .get(&RENDERER)
            .context("viewport renderer absent")?;
        let window = s.windows.get(&WINDOW).context("viewport window absent")?;
        let source = s
            .surfaces
            .get(&SURFACE)
            .context("viewport display absent")?;
        ensure!(
            self.bound_size == window.size,
            "draw context and viewport window differ"
        );
        ensure!(
            v.window == WINDOW
                && v.renderer == RENDERER
                && v.surface == SURFACE
                && r.window == WINDOW
                && i32::from(r.width) == w as i32,
            "viewport links differ"
        );
        ensure!(
            source[0] == w as i32 && source[1] > 0 && h > 0 && h <= source[1] as u32,
            "viewport exceeds backing display"
        );
        ensure!(
            v.rectangle == [0, 0, w as i32, h as i32]
                && window.origin == [0, 0]
                && window.size == [w as i32, h as i32]
                && window.rectangle == v.rectangle
                && window.scale == viewport::scales(window.size)
                && window.backend == BACKEND
                && window.result == BACKEND,
            "viewport geometry differs from current frame"
        );
        ensure!(
            *s.targets.get(&TARGET).context("viewport target absent")?
                == [
                    v.rectangle,
                    [0, h as i32, w as i32, (h as i32).wrapping_mul(2)]
                ],
            "viewport target regions differ"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_viewport_rejects_corrupt_owner_maps_without_panicking() {
        let frame = Frame {
            input: sims_sim::pose::portrait::view::Input {
                viewport: [0, 0, 1000, 740],
                center: [0, 0],
                zoom: 1,
                rotation: 0,
            },
            floor: 1,
            tick: 0,
        };
        let mut b = Binding::new(frame).unwrap();
        let regions = b.state.targets.remove(&TARGET).unwrap();
        b.state.targets.insert(99, regions);
        assert!(b.validate(frame).is_err());
        let mut b = Binding::new(frame).unwrap();
        b.bound_size[0] = 1;
        assert!(b.validate(frame).is_err());
        assert!(
            Binding::new(Frame {
                input: sims_sim::pose::portrait::view::Input {
                    viewport: [0, 0, 32768, 740],
                    ..frame.input
                },
                ..frame
            })
            .is_err()
        );
    }
}
impl Device {
    /// Explicitly register an existing replacement drawing context. Loading a
    /// historical device never creates this owner as a side effect.
    pub fn bind_viewport(&mut self) -> Result<()> {
        ensure!(self.saved.viewport.is_none(), "viewport already bound");
        self.saved.viewport = Some(Binding::new(self.saved.frame)?);
        Ok(())
    }
    /// Execute4b136b and its sizing children against this actual drawing context.
    /// Runs synchronously at a settled device boundary; it never advances the VM.
    /// ViewControl expansion may use this when its real frontend owner is bound.
    pub fn resize_viewport_height(&mut self, session: &mut Session, height: i32) -> Result<()> {
        self.resize_surface(session, height, None)
    }
    /// Actual frontend display resize. Width is the renderer's native signed
    /// field;4b136b and its existing children publish the viewport afterwards.
    /// No simulation tick or interpreter service is executed by this host action.
    pub fn resize_display(
        &mut self,
        session: &mut Session,
        display: [u32; 2],
        height: u32,
    ) -> Result<()> {
        ensure!(
            display.iter().all(|n| (1..=4096).contains(n)) && height > 0 && height <= display[1],
            "replacement display dimensions unavailable"
        );
        self.resize_surface(session, height as i32, Some(display.map(|n| n as i32)))
    }
    fn resize_surface(
        &mut self,
        session: &mut Session,
        height: i32,
        display: Option<[i32; 2]>,
    ) -> Result<()> {
        ensure!(
            !session.camera_command_pending()
                && !session.interface_update_pending()
                && !session.control_panel_pending()
                && !session.ui_portrait_pending()
                && !self.frozen,
            "camera work owns the device viewport"
        );
        ensure!(
            Frame::current(session)? == self.saved.frame,
            "refresh the current camera frame before resizing"
        );
        let binding = self
            .saved
            .viewport
            .as_ref()
            .context("historical device has no viewport binding")?;
        binding.validate(self.saved.frame)?;
        ensure!(
            height > 0 && height <= display.unwrap_or(binding.state.surfaces[&SURFACE])[1],
            "requested viewport is outside the actual display"
        );
        let mut binding = self.saved.viewport.take().unwrap();
        if let Some(display) = display {
            binding.state.surfaces.insert(SURFACE, display);
            binding.state.renderers.get_mut(&RENDERER).unwrap().width = display[0] as i16;
        }
        let width = i32::from(binding.state.renderers[&RENDERER].width);
        let result = (|| -> Result<()> {
            let mut calls = vec![Invocation::new(VIEW, Function::Height(height))];
            while let Some(call) = calls.last_mut() {
                let Some(request) = call
                    .advance(&mut binding.state)
                    .map_err(anyhow::Error::msg)?
                else {
                    ensure!(call.complete(), "viewport child incomplete");
                    calls.pop();
                    if let Some(parent) = calls.last_mut() {
                        parent
                            .acknowledge(
                                parent.request().context("viewport parent request absent")?,
                                Reply::Done,
                            )
                            .map_err(anyhow::Error::msg)?;
                    }
                    continue;
                };
                let reply = match request {
                    Request::Renderer(owner) => {
                        ensure!(owner == VIEW, "viewport getter owner differs");
                        Reply::Renderer(binding.state.views[&owner].renderer)
                    }
                    Request::RendererHeight { renderer, height } => {
                        calls.push(Invocation::new(renderer, Function::RendererHeight(height)));
                        continue;
                    }
                    Request::Resize { window, size } => {
                        calls.push(Invocation::new(window, Function::Resize(size)));
                        continue;
                    }
                    Request::Backend { backend, window } => {
                        ensure!(
                            backend == BACKEND && window == WINDOW,
                            "viewport backend owner differs"
                        );
                        // This context binds a logical viewport to its existing renderer.
                        // Updated window geometry is live here; the following original
                        // synchronization child rebuilds/publishes the actual scene.
                        ensure!(
                            binding.state.windows[&window].size == [width, height],
                            "viewport backend size differs"
                        );
                        binding.bound_size = binding.state.windows[&window].size;
                        Reply::Backend(BACKEND)
                    }
                    Request::Synchronize(owner) => {
                        ensure!(owner == VIEW, "viewport synchronization owner differs");
                        let rect = binding.state.views[&owner].rectangle;
                        ensure!(
                            rect == [0, 0, binding.bound_size[0], binding.bound_size[1]],
                            "draw context viewport differs from caller geometry"
                        );
                        session
                            .host
                            .presentation
                            .presenter
                            .as_mut()
                            .context("viewport presenter absent")?
                            .camera
                            .as_mut()
                            .context("viewport camera absent")?
                            .settings
                            .viewport = rect;
                        session.host.presentation.viewport = rect;
                        session.host.presentation.camera = Some(session.retained_camera_input()?);
                        self.refresh(session)?;
                        Reply::Done
                    }
                };
                calls
                    .last_mut()
                    .unwrap()
                    .acknowledge(request, reply)
                    .map_err(anyhow::Error::msg)?;
            }
            binding.validate(self.saved.frame)
        })();
        self.saved.viewport = Some(binding);
        result
    }
}
