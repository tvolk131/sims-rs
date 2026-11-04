//! Explicit fresh replacement-client inputs, not recovered suspended camera
//! memory or a replay of original process startup. Original state-machine
//! semantics run on these inputs for every subsequent user camera command.
use super::*;
use sims_sim::lot_presenter::{
    self, hover, lifecycle, motion, open, room_masks, visibility, walls::cutaway::Mask,
};
use sims_vm::{
    commander::{
        Registry,
        house::{Fields, Interface, Receiver},
    },
    presenter::Order,
};

impl Device {
    ///Application opening after the frontend has constructed its resident
    ///portrait cache. Uses4f1ae9's selection rule and the actual445449 owner.
    ///This is separate from load completion; restored sessions never rerun it.
    pub fn publish_opening_household(
        &mut self,
        session: &mut Session,
        selection_override: u32,
    ) -> Result<()> {
        session.publish_opening_budget()?;
        if let Some(transport) = &mut session.host.presentation.transport {
            transport.controls.buy.dirty |= 0x4000;
        } else {
            self.saved.interface_flags |= 0x4000;
        }
        if !session.begin_opening_selection(selection_override)? {
            return Ok(());
        }
        for _ in 0..30000 {
            if session.house_notification_pending() {
                self.advance_notification(session, 256)?;
                continue;
            }
            let input = session.retained_camera_input()?;
            if session.advance_selection(input, 256)? {
                return Ok(());
            }
        }
        anyhow::bail!("opening selection work budget exceeded")
    }
    /// Synchronous startup for the application's background loading worker.
    /// Native work still lives in the resumable Session owners; callers needing
    /// intermediate checkpoints can use attach_for_load and those owners directly.
    pub fn open(
        session: &mut Session,
        width: u32,
        height: u32,
        clock: u32,
        config: completion::Config,
    ) -> Result<Self> {
        let mut device = Self::open_for_household(session, width, height, clock, config)?;
        device.publish_live_interface(session)?;
        Ok(device)
    }
    ///Stop after F6 so the application's portrait construction, budget and
    ///selection can run before its Live-mode publication (4f1ae9/4ce44f).
    pub fn open_for_household(
        session: &mut Session,
        width: u32,
        height: u32,
        clock: u32,
        config: completion::Config,
    ) -> Result<Self> {
        let tick = session.simulation.vm.tick;
        let mut device = Self::attach_for_load(session, width, height, clock)?;
        let mut complete = false;
        for _ in 0..30000 {
            let input = session.retained_camera_input()?;
            if session.advance_postload(input, 256)? {
                complete = true;
                break;
            }
        }
        ensure!(complete, "original post-load work budget exceeded");
        session.begin_load_completion(config)?;
        complete = false;
        for _ in 0..30000 {
            if device.advance_load_completion(session, 256)? {
                complete = true;
                break;
            }
        }
        ensure!(complete, "original house-opening work budget exceeded");
        ensure!(
            session.simulation.vm.tick == tick,
            "house loading advanced lot time"
        );
        Ok(device)
    }
    pub fn publish_live_interface(&mut self, session: &mut Session) -> Result<()> {
        let tick = session.simulation.vm.tick;
        // Binding explicitly selects Live mode. Publish it through the original
        // ordered mask2 pass; the saved game's Options-mode global is not a
        // declaration of the replacement application's current mode.
        session.begin_live_interface_update()?;
        let mut complete = false;
        for _ in 0..30000 {
            if self.advance_interface(session, 256)? {
                complete = true;
                break;
            }
        }
        ensure!(complete, "original Live interface work budget exceeded");
        session.bind_control_panel()?;
        self.saved.panel_state = Some(1);
        self.bind_transport(session)?;
        ensure!(
            session.simulation.vm.tick == tick,
            "house loading advanced lot time"
        );
        Ok(())
    }
    pub fn attach(session: &mut Session, width: u32, height: u32, clock: u32) -> Result<Self> {
        Self::attach_inner(session, width, height, clock, false)
    }
    /// Bind the replacement frontend before the original post-load tail/F6.
    /// The caller drives postload and then explicitly starts load completion.
    pub fn attach_for_load(
        session: &mut Session,
        width: u32,
        height: u32,
        clock: u32,
    ) -> Result<Self> {
        ensure!(
            session.options.native_postload && session.options.native_load_completion,
            "camera loading requires native postload and completion"
        );
        Self::attach_inner(session, width, height, clock, true)
    }
    fn attach_inner(
        session: &mut Session,
        width: u32,
        height: u32,
        clock: u32,
        loading: bool,
    ) -> Result<Self> {
        ensure!(
            session.host.presentation.presenter.is_none(),
            "replacement camera is already bound"
        );
        let level = session
            .simulation
            .vm
            .world
            .refresh
            .as_ref()
            .context("house refresh fields absent")?
            .visible_level;
        let zoom = *session
            .simulation
            .vm
            .world
            .globals
            .get(&2)
            .context("house zoom global absent")?;
        ensure!(
            (1..=3).contains(&zoom),
            "house has no initialized original zoom"
        );
        let rotation = session.simulation.vm.world.camera_rotation;
        let camera = OriginalHouse::camera(
            session,
            width,
            height,
            rotation,
            (1u32 << zoom) as f32 / 8.,
            level,
        );
        let input = crate::dialog::client::Inputs::camera(camera, 64)?.view;
        let projection = room_masks::Camera::initialized(input.center, zoom as u8)
            .map_err(anyhow::Error::msg)?;
        let view = open::State {
            origin: input.center,
            pending: [0, 0],
            zoom: zoom as u32,
            display_rotation: u32::from(rotation),
            widths: [16, 8, 8, 4].map(|v| v << zoom),
            floor: lifecycle::State {
                level,
                all_walls: 0,
                history: hover::History::house_view(),
                visibility: visibility::State {
                    enabled: 1,
                    dirty: 1,
                    include_visible: 1,
                    mask: Mask {
                        first_row: 0,
                        rows: vec![0; 64],
                    },
                },
            },
        };
        let mut depth_scales = [0; 4];
        for zoom in 1..=3 {
            depth_scales[zoom as usize] = Input { zoom, ..input }
                .build()
                .map_err(anyhow::Error::msg)?
                .far
                .to_bits();
        }
        let retained = camera::State::from_lot(
            session
                .simulation
                .architecture
                .as_ref()
                .and_then(|a| a.original.as_ref())
                .context("original camera terrain absent")?,
            &session
                .simulation
                .room_environment
                .as_ref()
                .context("camera rooms absent")?
                .manager,
            &view,
            camera::Settings {
                roof: Some(0),
                clock,
                application_mode: 2,
                viewport: [0, 0, width as i32, height as i32],
                bounds: [0, 63, 0, 63],
                world_rotation: u32::from(rotation),
                camera_mode: 1,
                moving: 0,
                projection_ready: 1,
                renderer_ready: true,
                graphics_enabled: true,
                ui_present: false,
                skip_edges: 0,
                depth_scales,
                floor_heights: projection.floor_heights,
                deferred: 0,
                locked: 0,
                attempts: 0,
                bottom_margin: 0,
                // The new frontend starts stationary and uses immediate camera
                // commands. These are client policy, not claimed native defaults.
                motion: motion::State {
                    x: [0; 4],
                    y: [0; 4],
                    active: 0,
                    policy: 0,
                    duration: 0,
                    speed_bits: 1f32.to_bits(),
                    rate_bits: 1f32.to_bits(),
                },
                duration_input: 0,
                application_value_bits: 1f32.to_bits(),
                saved_value_bits: 1f32.to_bits(),
            },
        )
        .map_err(anyhow::Error::msg)?;
        let inputs = lot_presenter::Inputs {
            force_dynamic: false,
            graphic_ready: session
                .host
                .ordinary
                .presenter_readiness(&session.simulation.vm, &session.program)?,
            // Fresh construction owns +188 on the object. Historical or
            // arbitrary process snapshots remain unknown; never fill them.
            keep_dynamic: Default::default(),
        };
        if session.simulation.vm.world.registry_order.is_some() {
            session.configure_registry_house_view(view, inputs, retained)?;
        } else {
            session.configure_house_view(
                view,
                inputs,
                Order {
                    dirty: true,
                    rotation: u32::from(rotation),
                    objects: vec![],
                },
                retained,
            )?;
        }
        let mut registry = Registry::default();
        // This is the replacement application's explicit registration order.
        // cSimsApp's original Windows Interface receiver is not installed: it
        // owns the cWinCPanel/cWinOptions tree, which this frontend does not have.
        // Iced's controls are published by the camera command/device instead.
        // Native head insertion/dispatch for the hosted listeners still run.
        for (i, receiver) in [
            Receiver::Module,
            Receiver::World,
            Receiver::Lot,
            Receiver::Globals,
            Receiver::Objects,
            Receiver::Viewer,
            Receiver::Renderer,
        ]
        .into_iter()
        .enumerate()
        {
            let id = i as u32 + 1;
            registry.construct(id).map_err(anyhow::Error::msg)?;
            registry.bind(id, receiver).map_err(anyhow::Error::msg)?;
        }
        let fields = Fields {
            viewer_interface: true,
            include_visible: 1,
            renderer_dirty: 0,
            ui: Interface {
                mask_owner_present: false,
                panel_active: false,
                shared_present: false,
                gated: 0,
                creation_gated: 0,
                dirty: 0,
            },
        };
        if loading {
            session.begin_postload(registry, fields)?;
        } else {
            session.bind_house_listeners(registry, fields)?;
        }
        session.bind_camera_ui(Ui {
            flags: 0,
            zoom: zoom as u32,
            rotation: u32::from(rotation),
            floor: level,
        })?;
        session.host.presentation.camera = Some(input);
        session.host.presentation.viewport = [0, 0, width as i32, height as i32];
        Self::new(session)
    }
}
