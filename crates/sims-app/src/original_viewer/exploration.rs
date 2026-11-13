//! Headless exploration through the actual Iced application's input handlers.
//! No fixture scheduler, player save writes, synthetic menu entries, or RNG reseeding.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_runtime::exploration::{
    self as engine, Candidate, Driver, Limits, Observation, Plan, Report,
};
use std::path::PathBuf;
use std::time::Duration;
#[path = "exploration/milestone.rs"]
mod milestone;
use milestone::Milestone;
#[path = "exploration/buy_edit_cycle.rs"]
mod buy_edit_cycle;
#[path = "exploration/checkpoint.rs"]
mod checkpoint;
#[cfg(test)]
#[path = "exploration/interaction_sequences.rs"]
mod interaction_sequences;
#[cfg(test)]
#[path = "exploration/native_owned_adapter.rs"]
mod native_owned_adapter;
#[cfg(test)]
#[path = "exploration/native_frame_clock_tests.rs"]
mod native_frame_clock_tests;
#[cfg(test)]
#[path = "exploration/native_owned_continuation.rs"]
mod native_owned_continuation;
#[cfg(test)]
#[path = "exploration/responsiveness.rs"]
mod responsiveness;
#[cfg(test)]
#[path = "exploration/startup_play.rs"]
mod startup_play;
#[cfg(test)]
#[path = "exploration/stair_sequences.rs"]
mod stair_sequences;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Identity {
    object: i16,
    handle: u32,
}
impl Identity {
    fn get(s: &Session, object: i16) -> Result<Self> {
        Ok(Self {
            object,
            handle: s
                .simulation
                .vm
                .world
                .scheduler_storage
                .as_ref()
                .and_then(|r| r.handle(object))
                .context("object lifetime is absent")?,
        })
    }
    fn check(&self, s: &Session) -> Result<()> {
        ensure!(
            *self == Self::get(s, self.object)?,
            "stale object identity: {self:?}"
        );
        Ok(())
    }
}

fn default_frame_millis() -> u32 {
    40
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Resident {
        identity: Identity,
    },
    Object {
        identity: Identity,
    },
    /// Return while the original eligibility collector is still suspended.
    BeginObject {
        identity: Identity,
    },
    Viewport {
        pixel: [u32; 2],
        expected: Option<Identity>,
    },
    /// Poll one application frame without changing the virtual clock.
    Poll,
    /// Bounded virtual-time wait for an observable game state.
    Wait {
        milestone: Milestone,
        max_ticks: u64,
        max_frames: u32,
        #[serde(default = "default_frame_millis")]
        frame_millis: u32,
    },
    Check {
        milestone: Milestone,
    },
    /// Yield back to the input generator when a modal or pause interrupts time.
    AdvanceBurst {
        ticks: u64,
    },
    /// Resolve exactly one currently offered entry, never synthesize an action.
    ChooseLabel {
        label: String,
    },
    CancelAll {
        actor: Identity,
    },
    Choice {
        index: usize,
        expected: sims_vm::action::Action,
    },
    GroundChoice {
        index: usize,
        expected: sims_vm::action::Action,
    },
    CloseMenu,
    CancelAction {
        actor: Identity,
        serial: u32,
    },
    Advance {
        millis: u32,
    },
    Playing {
        value: bool,
    },
    Resize {
        width: u32,
        height: u32,
    },
    Rotate,
    ZoomIn,
    ZoomOut,
    Floor {
        level: i32,
    },
    Buy,
    ExpandPanel,
    BuyNavigate {
        revision: u64,
        event: sims_vm::interface::buy::input::Event,
    },
    BuyProduct {
        revision: u64,
        index: usize,
        expected_definition: u32,
        expected_guid: u32,
    },
    ViewportEnter,
    ViewportLeave,
    ViewportMotion {
        pixel: [u32; 2],
    },
    ViewportButton {
        down: bool,
        pixel: [u32; 2],
        expected: Option<Identity>,
    },
    Escape,
    SellFurniture {
        held: u32,
    },
    Live,
    Needs,
    Dialog {
        id: u64,
        result: u32,
        body_sha256: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    mode: String,
    plan: Plan,
    setup: Vec<Command>,
    commands: Vec<Command>,
    /// Sweep all targets when None. Limits are explicit in the final coverage report.
    max_targets: Option<usize>,
    target_scope: String,
    target_objects: Option<Vec<i16>>,
    resident_objects: Option<Vec<i16>>,
    max_actions_per_target: usize,
    action_frames: u32,
    screenshot_limit: usize,
    screenshot_every_commands: usize,
    failure_checkpoint_limit: usize,
    replay: Option<PathBuf>,
    minimize_attempts: usize,
    benchmark: bool,
    save_final: bool,
    progress_burst_ticks: u64,
    checkpoint_after_commands: Vec<usize>,
    /// One-based successful Wait/Check boundaries. The comparison branch is
    /// cold-restored before the following command; the source stays warm.
    cold_restore_after_milestones: Vec<usize>,
    screenshot_after_commands: Vec<usize>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            mode: "explore".into(),
            plan: Plan::default(),
            setup: vec![],
            commands: vec![],
            max_targets: None,
            target_scope: "drawn".into(),
            target_objects: None,
            resident_objects: None,
            max_actions_per_target: 16,
            action_frames: 8,
            screenshot_limit: 0,
            screenshot_every_commands: 0,
            replay: None,
            failure_checkpoint_limit: 8,
            minimize_attempts: 0,
            benchmark: false,
            save_final: false,
            progress_burst_ticks: 0,
            checkpoint_after_commands: vec![],
            cold_restore_after_milestones: vec![],
            screenshot_after_commands: vec![],
        }
    }
}

/// Application-owned state omitted from a player save but relevant to exact
/// test continuation. Instants are offsets from a fixed driver epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Local {
    diagnostics: bool,
    options_open: bool,
    display_size: [u32; 2],
    tutorial_highlight: Option<i32>,
    resident: i16,
    selected: Option<i16>,
    floor: i32,
    zoom: f32,
    playing: bool,
    speed: f32,
    last_ms: Option<u64>,
    clock_origin: Option<(u64, u32)>,
    accumulator: f32,
    pan: [i32; 2],
    shift: bool,
    viewport_enter_requested: bool,
    move_inputs: std::collections::VecDeque<MoveInput>,
    status: String,
    error: Option<String>,
    menu_error: Option<String>,
    millis: u64,
}
struct Application {
    viewer: Viewer,
    epoch: Instant,
    millis: u64,
    paths: std::collections::BTreeSet<String>,
    progress_burst_ticks: u64,
}
impl Application {
    /// Install the same Ready result consumed by the GUI's Loaded message.
    /// With no --load this retains the complete original startup lifecycle.
    fn install_ready(args: &Args, ready: Arc<Ready>, epoch: Instant, limits: &Limits) -> Result<Self> {
        let (mut viewer, _) = Viewer::new(args.clone());
        viewer.install(ready)?;
        viewer.last = Some(epoch);
        if let Some(clock) = viewer.session.as_ref().and_then(|s| s.host.presentation.clock.as_ref()) {
            viewer.clock_origin = Some((epoch, clock.now));
        }
        let mut app = Self { viewer, epoch, millis: 0, paths: Default::default(), progress_burst_ticks: 0 };
        app.drain(limits)?;
        Ok(app)
    }
    fn session(&self) -> Result<&Session> {
        self.viewer.session.as_ref().context("session absent")
    }
    fn local(&self) -> Local {
        let v = &self.viewer;
        Local {
            diagnostics: v.diagnostics,
            options_open: v.options_open,
            display_size: v.display_size,
            tutorial_highlight: v.tutorial_highlight,
            resident: v.resident,
            selected: v.selected,
            floor: v.floor,
            zoom: v.zoom,
            playing: v.playing,
            speed: v.speed,
            last_ms: v
                .last
                .map(|t| t.duration_since(self.epoch).as_millis() as u64),
            clock_origin: v
                .clock_origin
                .map(|(t, n)| (t.duration_since(self.epoch).as_millis() as u64, n)),
            accumulator: v.accumulator,
            pan: v.pan,
            shift: v.shift,
            viewport_enter_requested: v.viewport_enter_requested,
            move_inputs: v.move_inputs.clone(),
            status: v.status.clone(),
            error: v.error.clone(),
            menu_error: v.menu_error.clone(),
            millis: self.millis,
        }
    }
    fn apply_local(&mut self, state: Local) {
        self.millis = state.millis;
        let v = &mut self.viewer;
        v.diagnostics = state.diagnostics;
        v.options_open = state.options_open;
        v.display_size = state.display_size;
        v.tutorial_highlight = state.tutorial_highlight;
        v.resident = state.resident;
        v.selected = state.selected;
        v.floor = state.floor;
        v.zoom = state.zoom;
        v.playing = state.playing;
        v.speed = state.speed;
        v.last = state.last_ms.map(|n| self.epoch + Duration::from_millis(n));
        v.clock_origin = state
            .clock_origin
            .map(|(n, t)| (self.epoch + Duration::from_millis(n), t));
        v.accumulator = state.accumulator;
        v.pan = state.pan;
        v.shift = state.shift;
        v.viewport_enter_requested = state.viewport_enter_requested;
        v.move_inputs = state.move_inputs;
        v.status = state.status;
        v.error = state.error;
        v.menu_error = state.menu_error;
        v.publish_hud_cluster();
    }
    fn capture(&self, managed: bool) -> Result<Value> {
        let s = self.session()?;
        Ok(
            json!({"schema":1,"session":if managed {s.save_managed()?} else {s.save()},
            "dialog_client":self.viewer.client.as_ref().context("dialog client absent")?.save(s)?,
            "camera_device":self.viewer.device.as_ref().map(|d| d.save(s)).transpose()?,
            "application":self.local()}),
        )
    }
    fn sample_paths(&mut self) {
        if let Some(s) = &self.viewer.session {
            for t in s.simulation.vm.threads.values() {
                for f in &t.stack {
                    self.paths.insert(format!(
                        "{}:{}:{}",
                        f.location.source, f.location.bhav, f.location.instruction
                    ));
                }
            }
        }
    }
    fn check_errors(&self) -> Result<()> {
        ensure!(
            self.viewer.error.is_none(),
            "{}",
            self.viewer.error.as_deref().unwrap_or("")
        );
        ensure!(
            self.viewer.menu_error.is_none(),
            "{}",
            self.viewer.menu_error.as_deref().unwrap_or("")
        );
        Ok(())
    }
    fn dispatch(&mut self, message: Message) -> Result<()> {
        let _ = self.viewer.update(message);
        self.sample_paths();
        self.check_errors()
    }
    fn drain(&mut self, limits: &Limits) -> Result<()> {
        let now = self.epoch + Duration::from_millis(self.millis);
        for _ in 0..limits.service_polls {
            self.check_errors()?;
            // An original modal needs an explicit recorded player command.
            if self.session()?.simulation.vm.world.dialogs.modal.is_some()
                && !self.viewer.presentation_children_pending()
            {
                return Ok(());
            }
            if !self.viewer.presentation_pending() {
                self.viewer.refresh()?;
                return self.check_errors();
            }
            self.dispatch(Message::Frame(now))?;
        }
        anyhow::bail!("exploration synchronous-service poll budget exceeded")
    }
    fn menu(&self) -> Option<&sims_runtime::original_session::object_input::State> {
        self.viewer
            .session
            .as_ref()?
            .host
            .presentation
            .object_input
            .as_ref()
            .filter(|m| m.ready())
    }
    fn targets(&self) -> Result<Vec<Identity>> {
        let s = self.session()?;
        s.simulation
            .vm
            .world
            .positions
            .keys()
            .copied()
            .filter(|id| {
                s.simulation
                    .vm
                    .world
                    .objects
                    .get(id)
                    .is_some_and(|o| o.definition.is_some() && o.person.is_none())
            })
            .map(|id| Identity::get(s, id))
            .collect()
    }
    fn drawn_targets(&self) -> Result<Vec<Identity>> {
        let drawn: std::collections::BTreeSet<_> = self
            .viewer
            .scene
            .as_ref()
            .into_iter()
            .flat_map(|s| &s.draws)
            .filter_map(|d| d.object)
            .collect();
        Ok(self
            .targets()?
            .into_iter()
            .filter(|id| drawn.contains(&id.object))
            .collect())
    }
    fn residents(&self) -> Result<Vec<Identity>> {
        let s = self.session()?;
        s.house
            .household
            .as_ref()
            .context("household absent")?
            .residents
            .iter()
            .filter_map(|resident| resident_object(s, resident))
            .map(|identity| Identity::get(s, identity))
            .collect()
    }
    fn screenshot(&self, path: &Path) -> Result<()> {
        let before = self.observe()?;
        crate::support::render_headless_element_at_size(
            &self.viewer.args.root,
            path,
            self.viewer.view(),
            self.viewer.display_size,
        )?;
        ensure!(
            self.observe()? == before,
            "headless rendering mutated retained state"
        );
        Ok(())
    }
}
impl Driver for Application {
    type Command = Command;
    fn observe(&self) -> Result<Observation> {
        let vm = &self.session()?.simulation.vm;
        Ok(Observation {
            tick: vm.tick,
            rng: [Some(vm.rng_state), vm.lot_rng_state, vm.crt_rng_state],
            state_sha256: engine::digest(&serde_json::to_vec(&self.capture(false)?)?),
        })
    }
    fn candidates(&self) -> Result<Vec<Candidate<Command>>> {
        let s = self.session()?;
        let mut out = vec![];
        let mut add = |command, weight| out.push(Candidate { command, weight });
        if let Some(id) = s.simulation.vm.world.dialogs.modal {
            let record = s
                .simulation
                .vm
                .world
                .dialogs
                .records
                .iter()
                .find(|r| r.id == id)
                .context("modal record absent")?;
            for button in &record.buttons {
                add(
                    Command::Dialog {
                        id,
                        result: button.result,
                        body_sha256: engine::digest(record.body.as_bytes()),
                    },
                    1,
                );
            }
            return Ok(out);
        }
        if let Some(menu) = self.menu() {
            for (index, action) in menu.actions().iter().enumerate() {
                if action.flags & 16 == 0 {
                    add(
                        Command::Choice {
                            index,
                            expected: action.clone(),
                        },
                        4,
                    );
                }
            }
            add(Command::CloseMenu, 3);
            add(Command::Advance { millis: 40 }, 1);
            return Ok(out);
        }
        if let Some(menu) = s
            .host
            .presentation
            .ground_input
            .as_ref()
            .filter(|m| m.ready())
        {
            for (index, action) in menu.actions().iter().enumerate() {
                if action.flags & 16 == 0 {
                    add(
                        Command::GroundChoice {
                            index,
                            expected: action.clone(),
                        },
                        4,
                    );
                }
            }
            add(Command::CloseMenu, 3);
            add(Command::Advance { millis: 40 }, 1);
            return Ok(out);
        }
        if self.viewer.presentation_pending() {
            add(Command::Poll, 1);
            return Ok(out);
        }
        let live = s
            .host
            .presentation
            .presenter
            .as_ref()
            .and_then(|p| p.camera.as_ref())
            .is_some_and(|c| c.settings.application_mode == 2);
        let mut target_count = 8;
        if live {
            if s.simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?
                .is_none()
            {
                for identity in self.residents()? {
                    add(Command::Resident { identity }, 1);
                }
                return Ok(out);
            }
            let targets = self.drawn_targets()?;
            target_count = targets.len().max(8) as u32;
            for identity in targets {
                add(Command::Object { identity }, 3);
            }
            for identity in self.residents()? {
                add(Command::Resident { identity }, 3);
            }
            if let Some(actor) = s
                .simulation
                .vm
                .world
                .actions
                .as_ref()
                .and_then(|a| a.actors.get(&self.viewer.resident))
            {
                for action in actor.active.iter().chain(actor.queue.pending.iter()) {
                    add(
                        Command::CancelAction {
                            actor: Identity::get(s, self.viewer.resident)?,
                            serial: action.id,
                        },
                        3,
                    );
                }
            }
            add(Command::Buy, 2);
            add(Command::Needs, 2);
            if let Some(scene) = &self.viewer.scene {
                for [x, y] in [[0.25, 0.25], [0.5, 0.5], [0.75, 0.75]] {
                    let point = [scene.width as f32 * x, scene.height as f32 * y];
                    let expected = scene
                        .pick(point)
                        .map(|id| Identity::get(s, id))
                        .transpose()?;
                    add(
                        Command::Viewport {
                            pixel: point.map(f32::to_bits),
                            expected,
                        },
                        2,
                    );
                }
            }
        } else {
            add(Command::Live, 10);
        }
        // Balance input categories rather than letting hundreds of objects
        // overwhelm waits and transport commands in a large household.
        if self.progress_burst_ticks > 0 && self.viewer.playing {
            add(
                Command::AdvanceBurst {
                    ticks: self.progress_burst_ticks,
                },
                target_count * 6,
            );
        } else {
            add(Command::Advance { millis: 200 }, target_count * 2);
        }
        add(
            Command::Playing {
                value: !self.viewer.playing,
            },
            if self.viewer.playing {
                2
            } else {
                target_count * 2
            },
        );
        add(
            Command::Resize {
                width: 800,
                height: 600,
            },
            1,
        );
        add(
            Command::Resize {
                width: 1320,
                height: 900,
            },
            1,
        );
        add(Command::Rotate, 1);
        add(Command::ZoomIn, 1);
        add(Command::ZoomOut, 1);
        add(
            Command::Floor {
                level: 3 - self.viewer.floor,
            },
            1,
        );
        Ok(out)
    }
    fn begin_tick_trace(&mut self) {
        self.viewer.exploration_ticks = Some(vec![]);
    }
    fn take_tick_trace(&mut self) -> Vec<Observation> {
        self.viewer.exploration_ticks.take().unwrap_or_default()
    }
    fn execute(&mut self, command: &Command, limits: &Limits) -> Result<()> {
        match command {
            Command::AdvanceBurst { ticks } => {
                ensure!((1..=512).contains(ticks), "invalid progress burst");
                let start = self.session()?.simulation.vm.tick;
                let wall = Instant::now();
                for _ in 0..limits.service_polls {
                    if self.session()?.simulation.vm.tick.saturating_sub(start) >= *ticks
                        || !self.viewer.playing
                        || self.session()?.simulation.vm.world.dialogs.modal.is_some()
                    {
                        return Ok(());
                    }
                    ensure!(
                        wall.elapsed().as_secs_f64() < limits.wall_seconds,
                        "progress burst wall limit"
                    );
                    self.execute(&Command::Advance { millis: 250 }, limits)?;
                }
                anyhow::bail!("progress burst poll limit");
            }
            Command::Check { milestone } => {
                ensure!(
                    milestone.reached(self, self.session()?.simulation.vm.tick)?,
                    "milestone assertion failed: {milestone:?}"
                );
            }
            Command::Wait {
                milestone,
                max_ticks,
                max_frames,
                frame_millis,
            } => {
                ensure!(
                    *max_ticks > 0
                        && *max_frames > 0
                        && *max_frames <= 100_000
                        && (1..=250).contains(frame_millis),
                    "invalid milestone limits"
                );
                let start = self.session()?.simulation.vm.tick;
                let wall = Instant::now();
                for frame in 0..=*max_frames {
                    if milestone.reached(self, start)? {
                        return Ok(());
                    }
                    let elapsed = self.session()?.simulation.vm.tick.saturating_sub(start);
                    ensure!(
                        frame < *max_frames
                            && elapsed < *max_ticks
                            && wall.elapsed().as_secs_f64() < limits.wall_seconds,
                        "milestone timeout after {elapsed} ticks/{frame} frames: {milestone:?}"
                    );
                    self.execute(
                        &Command::Advance {
                            millis: *frame_millis,
                        },
                        limits,
                    )?;
                }
                unreachable!()
            }
            Command::ChooseLabel { label } => {
                let choices: Vec<_> = self
                    .menu()
                    .context("object menu not ready")?
                    .actions()
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| a.label.as_ref() == Some(label) && a.flags & 16 == 0)
                    .map(|(i, a)| (i, a.clone()))
                    .collect();
                ensure!(
                    choices.len() == 1,
                    "expected one enabled {label:?} entry, got {}",
                    choices.len()
                );
                let (index, expected) = choices.into_iter().next().unwrap();
                return self.execute(&Command::Choice { index, expected }, limits);
            }
            Command::CancelAll { actor } => {
                actor.check(self.session()?)?;
                let serials: Vec<_> = self
                    .session()?
                    .simulation
                    .vm
                    .world
                    .actions
                    .as_ref()
                    .and_then(|a| a.actors.get(&actor.object))
                    .context("actor queue absent")?
                    .active
                    .iter()
                    .chain(
                        self.session()?
                            .simulation
                            .vm
                            .world
                            .actions
                            .as_ref()
                            .unwrap()
                            .actors[&actor.object]
                            .queue
                            .pending
                            .iter(),
                    )
                    .map(|a| a.id)
                    .collect();
                ensure!(!serials.is_empty(), "no actions to cancel");
                for serial in serials {
                    self.execute(
                        &Command::CancelAction {
                            actor: actor.clone(),
                            serial,
                        },
                        limits,
                    )?;
                }
                return Ok(());
            }
            Command::Resident { identity } => {
                identity.check(self.session()?)?;
                self.dispatch(Message::Resident(identity.object))?;
            }
            Command::Object { identity } | Command::BeginObject { identity } => {
                identity.check(self.session()?)?;
                self.dispatch(Message::Select(Some(identity.object)))?;
                if matches!(command, Command::BeginObject { .. }) {
                    return self.check_errors();
                }
            }
            Command::Viewport { pixel, expected } => {
                let point = pixel.map(f32::from_bits);
                ensure!(
                    point.iter().all(|n| n.is_finite() && *n >= 0.),
                    "invalid viewport coordinate"
                );
                if let Some(id) = expected {
                    id.check(self.session()?)?;
                }
                ensure!(
                    self.viewer
                        .scene
                        .as_ref()
                        .context("viewport absent")?
                        .pick(point)
                        == expected.as_ref().map(|id| id.object),
                    "viewport hit changed"
                );
                self.dispatch(Message::ViewportClick(point))?;
            }
            Command::Poll => {
                return self.dispatch(Message::Frame(
                    self.epoch + Duration::from_millis(self.millis),
                ));
            }
            Command::Choice { index, expected } => {
                ensure!(
                    self.menu().and_then(|m| m.actions().get(*index)) == Some(expected),
                    "stale menu action"
                );
                self.dispatch(Message::ObjectChoice(*index))?;
            }
            Command::GroundChoice { index, expected } => {
                ensure!(
                    self.session()?
                        .host
                        .presentation
                        .ground_input
                        .as_ref()
                        .filter(|m| m.ready())
                        .and_then(|m| m.actions().get(*index))
                        == Some(expected),
                    "stale ground menu action"
                );
                self.dispatch(Message::GroundChoice(*index))?;
            }
            Command::CloseMenu => {
                if self.menu().is_some() {
                    self.dispatch(Message::ObjectCancel)?;
                } else {
                    ensure!(
                        self.session()?
                            .host
                            .presentation
                            .ground_input
                            .as_ref()
                            .is_some_and(|m| m.ready()),
                        "menu is not open"
                    );
                    self.dispatch(Message::GroundCancel)?;
                }
            }
            Command::CancelAction { actor, serial } => {
                actor.check(self.session()?)?;
                ensure!(
                    self.viewer.resident == actor.object,
                    "cancellation actor differs"
                );
                let a = self
                    .session()?
                    .simulation
                    .vm
                    .world
                    .actions
                    .as_ref()
                    .and_then(|a| a.actors.get(&actor.object))
                    .context("action owner absent")?;
                ensure!(
                    a.active
                        .iter()
                        .chain(a.queue.pending.iter())
                        .any(|a| a.id == *serial),
                    "stale action serial"
                );
                self.dispatch(Message::Cancel(u64::from(*serial)))?;
            }
            Command::Advance { millis } => {
                ensure!(
                    (1..=250).contains(millis),
                    "frame time outside bounded range"
                );
                self.millis = self
                    .millis
                    .checked_add(u64::from(*millis))
                    .context("virtual clock overflow")?;
                self.dispatch(Message::Frame(
                    self.epoch + Duration::from_millis(self.millis),
                ))?;
            }
            Command::Playing { value } => {
                if self.viewer.playing != *value {
                    self.dispatch(Message::Toggle)?;
                }
            }
            Command::Resize { width, height } => {
                ensure!(
                    (320..=4096).contains(width) && (240..=4096).contains(height),
                    "invalid viewport"
                );
                self.dispatch(Message::DisplaySize(Size::new(
                    *width as f32,
                    *height as f32,
                )))?;
            }
            Command::Rotate => self.dispatch(Message::Rotate)?,
            Command::ZoomIn => self.dispatch(Message::ZoomIn)?,
            Command::ZoomOut => self.dispatch(Message::ZoomOut)?,
            Command::Floor { level } => {
                ensure!((1..=2).contains(level), "invalid floor");
                self.dispatch(Message::Floor(*level))?;
            }
            Command::Buy => self.dispatch(Message::Buy)?,
            Command::ExpandPanel => self.dispatch(Message::ExpandPanel)?,
            Command::BuyNavigate { revision, event } => {
                ensure!(
                    !self.viewer.presentation_pending()
                        && self.session()?.simulation.vm.world.dialogs.modal.is_none(),
                    "Buy navigation is blocked by a retained input owner"
                );
                ensure!(
                    buy_edit_cycle::catalog(self)?.accepts(*revision, *event),
                    "stale or unavailable Buy navigation"
                );
                self.dispatch(Message::BuyNavigate(*revision, *event))?;
            }
            Command::BuyProduct {
                revision,
                index,
                expected_definition,
                expected_guid,
            } => {
                ensure!(
                    !self.viewer.presentation_pending()
                        && self.session()?.simulation.vm.world.dialogs.modal.is_none(),
                    "Buy product is blocked by a retained input owner"
                );
                let catalog = buy_edit_cycle::catalog(self)?;
                ensure!(
                    catalog.accepts_product(*revision, *index)
                        && catalog.product(*index).is_some_and(|p| {
                            p.definition == *expected_definition && p.source.guid == *expected_guid
                        }),
                    "stale or unavailable Buy product"
                );
                self.dispatch(Message::BuyProduct(*revision, *index))?;
            }
            Command::ViewportEnter => self.dispatch(Message::ViewportEnter)?,
            Command::ViewportLeave => self.dispatch(Message::ViewportLeave)?,
            Command::ViewportMotion { pixel } => {
                let pixel = buy_edit_cycle::pixel(*pixel)?;
                ensure!(self.viewer.current_move(), "Move viewport is not active");
                self.dispatch(Message::ViewportMotion {
                    pixel: Some(pixel),
                    pan: [0, 0],
                })?;
            }
            Command::ViewportButton {
                down,
                pixel,
                expected,
            } => {
                let pixel = buy_edit_cycle::pixel(*pixel)?;
                ensure!(self.viewer.current_move(), "Move viewport is not active");
                if let Some(identity) = expected {
                    identity.check(self.session()?)?;
                }
                ensure!(
                    self.viewer
                        .scene
                        .as_ref()
                        .context("viewport absent")?
                        .pick(pixel)
                        == expected.as_ref().map(|id| id.object),
                    "Move viewport hit changed"
                );
                self.dispatch(Message::ViewportButton {
                    down: *down,
                    pixel,
                    picked: expected.as_ref().map(|id| id.object),
                })?;
            }
            Command::Escape => self.dispatch(Message::DialogKey(27))?,
            Command::SellFurniture { held } => {
                ensure!(
                    matches!(self.viewer.sell_message(), Some(Message::Sell(actual)) if actual == *held),
                    "stale or unavailable Sell cursor"
                );
                self.dispatch(Message::Sell(*held))?;
            }
            Command::Live => self.dispatch(Message::Live)?,
            Command::Needs => self.dispatch(Message::Needs)?,
            Command::Dialog {
                id,
                result,
                body_sha256,
            } => {
                let d = &self.session()?.simulation.vm.world.dialogs;
                let r = d
                    .records
                    .iter()
                    .find(|r| r.id == *id && Some(*id) == d.modal)
                    .context("stale modal")?;
                ensure!(
                    engine::digest(r.body.as_bytes()) == *body_sha256
                        && r.buttons.iter().any(|b| b.result == *result),
                    "stale dialog response"
                );
                self.dispatch(Message::Dialog(*id, *result))?;
            }
        }
        self.drain(limits)
    }
    fn fork(&self) -> Result<Self> {
        let s = self.session()?.fork_for_exploration()?;
        let client = DialogClient::restore(
            &s,
            self.viewer
                .client
                .as_ref()
                .context("client absent")?
                .save(self.session()?)?,
        )?;
        let (mut viewer, _) = Viewer::new(self.viewer.args.clone());
        viewer.client = Some(client);
        viewer.session = Some(s);
        viewer.device = self
            .viewer
            .device
            .as_ref()
            .map(CameraDevice::fork_for_exploration);
        viewer.icons = self
            .viewer
            .icons
            .as_ref()
            .map(|i| i.fork_for_exploration().map(Arc::new))
            .transpose()?;
        viewer.art = self.viewer.art.clone();
        viewer.scene = self.viewer.scene.clone();
        let mut result = Self {
            viewer,
            epoch: self.epoch,
            millis: self.millis,
            paths: self.paths.clone(),
            progress_burst_ticks: self.progress_burst_ticks,
        };
        result.apply_local(self.local());
        Ok(result)
    }
    fn checkpoint(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(&self.capture(true)?)?)
    }
    fn restore(&self, bytes: &[u8]) -> Result<Self> {
        Self::restore_checkpoint(
            &self.viewer.args,
            self.session()?.index(),
            self.viewer.icons.as_ref().context("icons absent")?,
            self.epoch,
            bytes,
        )
    }
}
impl Application {
    fn restore_checkpoint(
        args: &Args,
        index: &Index,
        icons: &Arc<dialog_view::Icons>,
        epoch: Instant,
        bytes: &[u8],
    ) -> Result<Self> {
        let value: Value = serde_json::from_slice(bytes)?;
        ensure!(
            value["schema"] == 1,
            "unknown exploration checkpoint schema"
        );
        let saved: Saved = serde_json::from_value(value["session"].clone())?;
        ensure!(
            saved.version == 2,
            "exploration checkpoint must own its neighborhood"
        );
        ensure!(
            Some(&saved.source) == args.original.as_ref(),
            "checkpoint belongs to another house"
        );
        let s = Session::restore(index, saved)?;
        let client = DialogClient::restore_managed(
            &s,
            serde_json::from_value(value["dialog_client"].clone())?,
        )?;
        let device = value
            .get("camera_device")
            .filter(|v| !v.is_null())
            .map(|v| CameraDevice::restore(&s, serde_json::from_value(v.clone())?))
            .transpose()?;
        let (mut viewer, _) = Viewer::new(args.clone());
        viewer.client = Some(client);
        viewer.session = Some(s);
        viewer.device = device;
        viewer.icons = Some(Arc::new(icons.fork_for_exploration()?));
        viewer.art = viewer
            .icons
            .as_ref()
            .map(|icons| Arc::new(icons.publish_art()));
        viewer.scene = viewer.device.as_ref().map(|d| d.scene().clone());
        let mut result = Self {
            viewer,
            epoch,
            millis: 0,
            paths: Default::default(),
            progress_burst_ticks: 0,
        };
        result.apply_local(serde_json::from_value(value["application"].clone())?);
        Ok(result)
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
struct ArtifactBudget {
    screenshots: usize,
    failure_checkpoints: usize,
}
fn run_case(
    app: &mut Application,
    plan: Plan,
    commands: Option<&[Command]>,
    output: &Path,
    name: &str,
    budget: &mut ArtifactBudget,
    every: usize,
    checkpoints: &[usize],
    screenshots: &[usize],
) -> Result<Report<Command>> {
    use std::io::Write;
    let mut journal = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join(format!("{name}-journal.jsonl")))?;
    writeln!(
        journal,
        "{}",
        json!({"schema":1,"plan":plan,"initial":app.observe()?})
    )?;
    engine::run_observed(app, plan, commands, |app, event| {
        match event {
            engine::Event::Input {
                index,
                command,
                before,
            } => {
                let line =
                    serde_json::to_vec(&json!({"input":index,"command":command,"before":before}))?;
                journal.write_all(&line)?;
                journal.write_all(b"\n")?;
                journal.flush()?;
            }
            engine::Event::Completed { index, after } => {
                writeln!(journal, "{}", json!({"completed":index,"after":after}))?;
                if checkpoints.contains(&(index + 1)) {
                    std::fs::write(
                        output.join(format!("{name}-step-{}.json", index + 1)),
                        app.checkpoint()?,
                    )?;
                }
                if ((every > 0 && (index + 1) % every == 0) || screenshots.contains(&(index + 1)))
                    && budget.screenshots > 0
                {
                    budget.screenshots -= 1;
                    app.screenshot(&output.join(format!("{name}-step-{}.png", index + 1)))?;
                }
            }
        }
        Ok(())
    })
}
fn preserve(
    app: &Application,
    output: &Path,
    name: &str,
    report: &Report<Command>,
    budget: &mut ArtifactBudget,
) -> Result<()> {
    write_json(&output.join(format!("{name}.json")), report)?;
    println!(
        "{name}: {} commands, {} comparisons, stop={}, failure={}",
        report.steps.len(),
        report.comparisons,
        report.stop,
        report.failure.is_some()
    );
    if !report.passed() && budget.failure_checkpoints > 0 {
        budget.failure_checkpoints -= 1;
        match app.checkpoint() {
            Ok(bytes) => std::fs::write(output.join(format!("{name}-failure-state.json")), bytes)?,
            Err(e) => {
                write_json(
                    &output.join(format!("{name}-capture-error.json")),
                    &format!("{e:#}"),
                )?;
                write_json(
                    &output.join(format!("{name}-unvalidated-state.json")),
                    &json!({
                    "warning":"diagnostic state only; capture validation failed",
                    "session":app.session()?.save(), "application":app.local()}),
                )?;
            }
        }
    }
    if budget.screenshots > 0 {
        let result = app.screenshot(&output.join(format!("{name}.png")));
        if let Err(e) = result {
            write_json(
                &output.join(format!("{name}-render-error.json")),
                &format!("{e:#}"),
            )?;
        }
        budget.screenshots -= 1;
    }
    Ok(())
}

pub(super) fn run(args: Args) -> Result<()> {
    std::thread::Builder::new()
        .name("gameplay-exploration".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || run_inner(args))?
        .join()
        .map_err(|_| anyhow::anyhow!("exploration worker panicked; inspect preserved evidence"))?
}
fn run_inner(mut args: Args) -> Result<()> {
    // New evidence directories only. Never overwrite the player's save or an
    // earlier experiment, even when the caller left --save at its default.
    let config: Config =
        serde_json::from_slice(&std::fs::read(args.exploration_plan.as_ref().unwrap())?)?;
    ensure!(
        ["explore", "sweep", "script", "replay"].contains(&config.mode.as_str()),
        "unknown exploration mode"
    );
    ensure!(
        config.cold_restore_after_milestones.is_empty()
            || ["script", "replay"].contains(&config.mode.as_str()),
        "milestone restores require script/replay mode"
    );
    ensure!(
        ["drawn", "placed"].contains(&config.target_scope.as_str()),
        "unknown target scope"
    );
    ensure!(
        config.max_actions_per_target > 0 && config.action_frames <= 1024,
        "invalid sweep budget"
    );
    let output = args.exploration_output.as_ref().unwrap().clone();
    validate_output(&args.root, &output.join("unused-player-save.json"))?;
    std::fs::create_dir(&output).context("refusing to overwrite exploration evidence")?;
    write_json(&output.join("config.json"), &config)?;
    args.save = output.join("unused-player-save.json");
    args.paused = true;
    let started = Instant::now();
    let ready = Arc::new(Ready::load(&args)?);
    let epoch = Instant::now();
    let bytes = args.load.as_ref().map(std::fs::read).transpose()?;
    let input: Option<Value> = bytes.as_ref().map(|b|serde_json::from_slice(b)).transpose()?;
    let mut initial =
        if input.as_ref().is_some_and(|v|v.get("application").is_some() && v.get("schema") == Some(&json!(1))) {
            Application::restore_checkpoint(
                &args,
                ready
                    .restore_index
                    .as_ref()
                    .context("restore index absent")?,
                &ready.icons,
                epoch,
                bytes.as_ref().unwrap(),
            )?
        } else {
            Application::install_ready(&args, ready, epoch, &config.plan.limits)?
        };
    initial.progress_burst_ticks = config.progress_burst_ticks;
    let load_seconds = started.elapsed().as_secs_f64();
    if !config.setup.is_empty() {
        std::fs::write(output.join("before-setup.json"), initial.checkpoint()?)?;
    }
    let setup = run_case(
        &mut initial,
        Plan {
            compare_fork: false,
            cold_restore_at: None,
            cold_restore_at_commands: vec![],
            minimum_completed_ticks: 0,
            ..config.plan.clone()
        },
        Some(&config.setup),
        &output,
        "setup",
        &mut ArtifactBudget {
            screenshots: 0,
            failure_checkpoints: 0,
        },
        0,
        &[],
        &[],
    )?;
    write_json(&output.join("setup.json"), &setup)?;
    ensure!(setup.failure.is_none(), "setup failed; see setup.json");
    std::fs::write(output.join("initial.json"), initial.checkpoint()?)?;
    if config.benchmark {
        let before = initial.observe()?;
        let mut forks = vec![];
        for _ in 0..3 {
            let start = Instant::now();
            let fork = initial.fork()?;
            forks.push(start.elapsed().as_secs_f64());
            ensure!(fork.observe()? == before, "benchmark fork changed state");
        }
        let start = Instant::now();
        let bytes = initial.checkpoint()?;
        let snapshot_seconds = start.elapsed().as_secs_f64();
        let start = Instant::now();
        let cold = initial.restore(&bytes)?;
        let restore_seconds = start.elapsed().as_secs_f64();
        ensure!(
            cold.observe()? == before,
            "benchmark cold restore changed state"
        );
        write_json(
            &output.join("benchmark.json"),
            &json!({"load_seconds":load_seconds,"fork_seconds":forks,
            "checkpoint_seconds":snapshot_seconds,"checkpoint_bytes":bytes.len(),"cold_restore_seconds":restore_seconds,
            "full_state_equal":true,"scope":"full application, excludes subsequent gameplay and image rendering"}),
        )?;
    }
    let mut artifacts = ArtifactBudget {
        screenshots: config.screenshot_limit,
        failure_checkpoints: config.failure_checkpoint_limit,
    };
    let mut summary = vec![];
    let mut all_paths = std::collections::BTreeSet::new();
    if config.mode == "sweep" {
        ensure!(
            initial
                .session()?
                .simulation
                .vm
                .world
                .dialogs
                .modal
                .is_none()
                && !initial.viewer.presentation_pending()
                && initial.viewer.people_panel_visible(),
            "sweep requires a ready visible Live household; add explicit startup/dialog commands to setup"
        );
        ensure!(
            initial
                .session()?
                .host
                .presentation
                .presenter
                .as_ref()
                .and_then(|p| p.camera.as_ref())
                .is_some_and(|c| c.settings.application_mode == 2),
            "sweep requires Live mode"
        );
        let candidates = if config.target_scope == "drawn" {
            initial.drawn_targets()?
        } else {
            initial.targets()?
        };
        let targets: Vec<_> = candidates
            .into_iter()
            .filter(|t| {
                config
                    .target_objects
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&t.object))
            })
            .collect();
        let residents: Vec<_> = initial
            .residents()?
            .into_iter()
            .filter(|t| {
                config
                    .resident_objects
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&t.object))
            })
            .collect();
        ensure!(
            !targets.is_empty() && !residents.is_empty(),
            "sweep has no matching residents or objects"
        );
        let total_targets = targets.len();
        for resident in residents {
            for target in targets
                .iter()
                .take(config.max_targets.unwrap_or(usize::MAX))
            {
                let name = format!("resident-{}-object-{}", resident.object, target.object);
                let mut app = initial.fork()?;
                let commands = vec![
                    Command::Resident {
                        identity: resident.clone(),
                    },
                    Command::Object {
                        identity: target.clone(),
                    },
                ];
                let opened = run_case(
                    &mut app,
                    config.plan.clone(),
                    Some(&commands),
                    &output,
                    &name,
                    &mut artifacts,
                    config.screenshot_every_commands,
                    &config.checkpoint_after_commands,
                    &config.screenshot_after_commands,
                )?;
                if opened.failure.is_none() {
                    ensure!(
                        app.session()?
                            .simulation
                            .native_selected_person()
                            .map_err(anyhow::Error::msg)?
                            == Some(resident.object),
                        "sweep resident selection was ignored; no coverage claimed"
                    );
                }
                preserve(&app, &output, &name, &opened, &mut artifacts)?;
                all_paths.extend(app.paths.iter().cloned());
                let actions = app.menu().map(|m| m.actions().to_vec()).unwrap_or_default();
                summary.push(json!({"name":name,"report":format!("{name}.json"),"offered":actions.len(),"failure":opened.failure,"outcome":opened.outcome,"targets_in_checkpoint":total_targets}));
                if opened.failure.is_some() {
                    continue;
                }
                for (index, action) in actions
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| a.flags & 16 == 0)
                    .take(config.max_actions_per_target)
                {
                    let mut branch = initial.fork()?;
                    let mut commands = commands.clone();
                    commands.push(Command::Choice {
                        index,
                        expected: action.clone(),
                    });
                    commands.push(Command::Playing { value: true });
                    commands
                        .extend((0..config.action_frames).map(|_| Command::Advance { millis: 40 }));
                    let leaf = format!("{name}-action-{index}");
                    let report = run_case(
                        &mut branch,
                        config.plan.clone(),
                        Some(&commands),
                        &output,
                        &leaf,
                        &mut artifacts,
                        config.screenshot_every_commands,
                        &config.checkpoint_after_commands,
                        &config.screenshot_after_commands,
                    )?;
                    preserve(&branch, &output, &leaf, &report, &mut artifacts)?;
                    all_paths.extend(branch.paths.iter().cloned());
                    summary.push(json!({"name":leaf,"report":format!("{leaf}.json"),"label":action.label,"failure":report.failure,"outcome":report.outcome}));
                }
            }
        }
    } else {
        let mut app = initial.fork()?;
        let expected: Option<Report<Command>> = config
            .replay
            .as_ref()
            .map(|p| Ok::<_, anyhow::Error>(serde_json::from_slice(&std::fs::read(p)?)?))
            .transpose()?;
        ensure!(
            config.mode != "replay" || expected.is_some(),
            "replay report absent"
        );
        let commands = if config.mode == "script" {
            Some(config.commands.clone())
        } else {
            expected.as_ref().map(Report::commands)
        };
        let plan = expected
            .as_ref()
            .map_or(config.plan.clone(), |r| r.plan.clone());
        let plan = checkpoint::schedule(
            plan,
            commands.as_deref(),
            &config.cold_restore_after_milestones,
        )?;
        let report = run_case(
            &mut app,
            plan,
            commands.as_deref(),
            &output,
            "run",
            &mut artifacts,
            config.screenshot_every_commands,
            &config.checkpoint_after_commands,
            &config.screenshot_after_commands,
        )?;
        preserve(&app, &output, "run", &report, &mut artifacts)?;
        if config.save_final {
            std::fs::write(output.join("final.json"), app.checkpoint()?)?;
        }
        if let Some(expected) = &expected {
            engine::verify_replay(expected, &report)?;
        }
        if config.minimize_attempts > 0
            && let Some(failure) = &report.failure
        {
            let message = failure.message.clone();
            let (minimal, attempts) =
                engine::minimize(&report.commands(), config.minimize_attempts, |commands| {
                    let mut branch = initial.fork()?;
                    let replay = engine::run(
                        &mut branch,
                        Plan {
                            compare_fork: false,
                            cold_restore_at: None,
                            cold_restore_at_commands: vec![],
                            ..config.plan.clone()
                        },
                        Some(commands),
                    )?;
                    Ok(replay
                        .failure
                        .as_ref()
                        .is_some_and(|f| f.kind == failure.kind && f.message == message))
                })?;
            write_json(
                &output.join("minimized.json"),
                &json!({"commands":minimal,"attempts":attempts,"same_failure":message}),
            )?;
        }
        all_paths.extend(app.paths);
        summary.push(json!({"name":"run","failure":report.failure,"steps":report.steps.len(),"comparisons":report.comparisons,
            "stop":report.stop,"outcome":report.outcome,"cold_restores":report.cold_restores,"tick_comparisons":report.tick_comparisons,
            "completed_ticks":report.steps.last().and_then(|s|s.after.as_ref()).map_or(0,|s|s.tick.saturating_sub(report.initial.tick))}));
    }
    let failed = summary.iter().filter(|s| !s["failure"].is_null()).count();
    let incomplete = summary
        .iter()
        .filter(|s| s["outcome"] == "incomplete")
        .count();
    let render_failures = std::fs::read_dir(&output)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .ends_with("-render-error.json")
        })
        .count();
    write_json(
        &output.join("summary.json"),
        &json!({"schema":1,"source":initial.session()?.house.source,
        "input_kind":if args.load.is_some(){"restored_checkpoint"}else{"fresh_original_household"},
        "load_seconds":load_seconds,"total_seconds":started.elapsed().as_secs_f64(),
        "branches":summary,"failed_branches":failed,"incomplete_branches":incomplete,"render_failures":render_failures,"sampled_script_locations":all_paths,
        "target_scope":config.target_scope,
        "target_scope_note":"drawn = objects with a draw in the starting scene; may include occluded fragments. placed = all positioned non-person objects. Object commands use the actual Select handler; Viewport commands additionally verify pixel picking.",
        "original_engine_executed":false,"strict_native_parity_ticks":0}),
    )?;
    println!(
        "Exploration evidence: {} ({failed} failing branches)",
        output.display()
    );
    ensure!(
        failed == 0,
        "exploration found {failed} failing branches; evidence preserved"
    );
    ensure!(
        incomplete == 0,
        "{incomplete} incomplete branches; minimum coverage not reached"
    );
    ensure!(
        render_failures == 0,
        "{render_failures} headless visual checks failed; evidence preserved"
    );
    Ok(())
}

/// Tick-boundary digest excludes derived renderer caches and is in addition to
/// full application comparisons at command boundaries. Never mutates game state.
pub(super) fn tick_observation(viewer: &Viewer) -> Result<Observation> {
    let s = viewer.session.as_ref().context("session absent at tick")?;
    Ok(Observation {
        tick: s.simulation.vm.tick,
        rng: [
            Some(s.simulation.vm.rng_state),
            s.simulation.vm.lot_rng_state,
            s.simulation.vm.crt_rng_state,
        ],
        state_sha256: engine::digest_serialized(&(&s.simulation, &s.host.presentation))?,
    })
}
