//! Authored-room laboratory and its explicit fixture driver.
use super::{
    Args, dialog_view,
    support::{
        Primitive, atomic_save, entries, render_headless_element, room_theme, scene_cursor,
        validate_output,
    },
};
use iced::{
    Color, Element, Fill, Size, Task, Theme,
    widget::{self, Space, button, column, container, progress_bar, row, slider, text},
    window,
};
use sims_content::{
    Index, household,
    vm::{ChairFixture, ChairScenario, load_chair_animations, load_program, two_chair_room},
};
use sims_render::{Assets, Camera, Scene, room_scene};
use sims_sim::{Simulation, Snapshot, StepStop};
use sims_vm::Program;
use std::{path::Path, sync::Arc, time::Instant};

#[derive(Debug)]
struct Loaded {
    assets: Assets,
    dialog_icons: dialog_view::Icons,
    program: Program,
    initial: Simulation,
}
impl Loaded {
    fn load(args: &Args) -> anyhow::Result<Arc<Self>> {
        validate_output(&args.root, &args.save)?;
        if let Some(path) = &args.screenshot {
            validate_output(&args.root, path)?;
        }
        let index = Index::scan(&args.root)?;
        let (assets, program, initial) = if args.household || args.home || args.two_floor {
            let (program, initial) = if args.two_floor {
                sims_content::stairs::home(&index, 7)?
            } else if args.domestic {
                sims_content::domestic::home(&index)?
            } else if args.home {
                household::two_sim_home(&index)?
            } else {
                household::autonomous_household(&index)?
            };
            (
                Assets::load_simulation(&index, &program, &initial)?,
                program,
                initial,
            )
        } else {
            let assets = Assets::load(&index)?;
            let mut program = load_program(&index, "DiningChairs.iff", "Global.iff")?;
            load_chair_animations(&index, &mut program)?;
            let fixture =
                ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty)?;
            let initial = two_chair_room(&program, fixture, args.facing)?;
            (assets, program, initial)
        };
        Ok(Arc::new(Self {
            assets,
            dialog_icons: dialog_view::Icons::load(&index)?,
            program,
            initial,
        }))
    }
}
#[derive(Debug, Clone)]
enum Message {
    Loaded(Result<Arc<Loaded>, String>),
    Frame(Instant),
    Toggle,
    Step,
    Speed(f32),
    Restart,
    Rotate,
    Floor(i32),
    Zoom(f32),
    Save,
    Load,
    Captured(window::Screenshot),
    Select(Option<i16>),
    Resident(i16),
    Queue(i16, usize),
    Cancel(u64),
    DialogResponse(u64, u32),
}
struct Viewer {
    args: Args,
    loaded: Option<Arc<Loaded>>,
    dialog_art: Option<dialog_view::HudArt>,
    sim: Option<Simulation>,
    scene: Option<Arc<Scene>>,
    playing: bool,
    speed: f32,
    zoom: f32,
    rotation: u8,
    floor: i32,
    accumulator: f32,
    last_frame: Option<Instant>,
    status: String,
    error: Option<String>,
    frames: u8,
    capturing: bool,
    selected: Option<i16>,
    resident: i16,
}
impl Viewer {
    fn new(mut args: Args) -> (Self, Task<Message>) {
        if args.two_floor {
            args.domestic = true;
            args.home = true;
        }
        let camera = args.camera;
        let zoom = if args.home {
            0.52
        } else if args.household {
            0.7
        } else {
            1.
        };
        let work = args.clone();
        let task = Task::perform(
            async move { Loaded::load(&work).map_err(|e| format!("{e:#}")) },
            Message::Loaded,
        );
        (
            Self {
                resident: if args.two_floor {
                    [1, 13, 32, 33][args.resident as usize - 1]
                } else if args.home && args.resident == 2 {
                    13
                } else {
                    1
                },
                floor: if args.two_floor { args.floor } else { 1 },
                playing: !args.paused && args.screenshot.is_none(),
                args,
                loaded: None,
                dialog_art: None,
                sim: None,
                scene: None,
                speed: 1.,
                zoom,
                rotation: camera,
                accumulator: 0.,
                last_frame: None,
                status: "Loading original assets…".into(),
                error: None,
                frames: 0,
                capturing: false,
                selected: None,
            },
            task,
        )
    }
    fn refresh(&mut self) {
        if let (Some(l), Some(s)) = (&self.loaded, &self.sim) {
            if let Err(e) = l.dialog_icons.prepare(&s.vm.world.dialogs.records) {
                self.error = Some(e.to_string());
                self.playing = false;
            }
            self.dialog_art = Some(l.dialog_icons.publish_art());
            let fraction = if self.playing {
                self.accumulator.clamp(0., 0.03999) * 25.
            } else {
                0.
            };
            match room_scene(
                &l.assets,
                &l.program,
                s,
                Camera::room(s, 1100, 760, self.rotation, self.zoom).floor(self.floor),
                fraction,
            ) {
                Ok(scene) => self.scene = Some(Arc::new(scene)),
                Err(e) => {
                    self.error = Some(e.to_string());
                    self.playing = false;
                }
            }
        }
    }
    fn tick(&mut self) -> Result<(), String> {
        let (Some(l), Some(s)) = (&self.loaded, &mut self.sim) else {
            return Ok(());
        };
        if s.vm.world.dialogs.modal.is_some() {
            return Ok(());
        }
        s.advance_fixture_tick()?;
        let step = s.run_fixture_tick(&l.program, 2000)?;
        match step.stop {
            StepStop::Faulted | StepStop::Budget | StepStop::Animation => {
                return Err(format!("simulation stopped: {:?}", step.stop));
            }
            _ => (),
        }
        // These presentation notifications have no consumers yet. Drain just as
        // the headless household runner does; do not accumulate them indefinitely.
        s.vm.sounds.clear();
        s.vm.effects.clear();
        if let Some(h) = &mut s.household {
            h.clock_events.clear();
        }
        Ok(())
    }
    fn reset(&mut self) -> Result<(), String> {
        let Some(l) = &self.loaded else {
            return Ok(());
        };
        let mut sim = l.initial.clone();
        sim.vm.world.camera_rotation = self.rotation;
        for &chair in &self.args.queue {
            sim.enqueue(&l.program, 1, i16::from(chair) + 1, 0)?;
        }
        sim.run_fixture_tick(&l.program, 2000)?;
        self.sim = Some(sim);
        self.accumulator = 0.;
        self.error = None;
        self.status = if self.args.household || self.args.home {
            "Let the Sim choose, or select an object to queue an action."
        } else {
            "Choose a chair, then Sit. Add another chair to the queue to switch seats."
        }
        .into();
        self.selected = None;
        Ok(())
    }
    fn load(&mut self, path: &Path) -> anyhow::Result<()> {
        let l = self
            .loaded
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("assets are loading"))?;
        if path.metadata()?.len() > 16 * 1024 * 1024 {
            anyhow::bail!("snapshot exceeds 16 MiB");
        }
        let mut sim =
            Snapshot::from_json(&std::fs::read(path)?, &l.program).map_err(anyhow::Error::msg)?;
        if !sim.interactions.contains_key(&1)
            || !sim.bodies.contains_key(&3)
            || (self.args.home
                && (!sim.interactions.contains_key(&13) || sim.architecture.is_none()))
        {
            anyhow::bail!("This checkpoint does not contain the viewer's actor and room.");
        }
        let step = sim
            .run_fixture_tick(&l.program, 2000)
            .map_err(anyhow::Error::msg)?;
        if matches!(
            step.stop,
            StepStop::Budget | StepStop::Faulted | StepStop::Animation
        ) {
            anyhow::bail!("snapshot cannot resume in this viewer: {:?}", step.stop);
        }
        room_scene(
            &l.assets,
            &l.program,
            &sim,
            Camera::room(&sim, 1100, 760, sim.vm.world.camera_rotation, self.zoom)
                .floor(self.floor),
            0.,
        )?;
        self.rotation = sim.vm.world.camera_rotation;
        self.sim = Some(sim);
        self.accumulator = 0.;
        self.last_frame = None;
        self.playing = false;
        self.error = None;
        self.status = format!("Loaded {}", path.display());
        Ok(())
    }
    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::DialogResponse(id, result) => {
                if let (Some(l), Some(s)) = (&self.loaded, &mut self.sim) {
                    let outcome =
                        s.vm.answer_dialog(id, result)
                            .and_then(|()| s.run_fixture_tick(&l.program, 2000));
                    match outcome {
                        Ok(step)
                            if matches!(
                                step.stop,
                                StepStop::Waiting | StepStop::Complete | StepStop::Dialog
                            ) =>
                        {
                            self.accumulator = 0.;
                            self.last_frame = None;
                            self.status = "Response recorded.".into();
                        }
                        Ok(step) => {
                            self.error = Some(format!("simulation stopped: {:?}", step.stop));
                            self.playing = false;
                        }
                        Err(e) => {
                            self.error = Some(e);
                            self.playing = false;
                        }
                    }
                }
            }
            Message::Floor(level) => {
                if self
                    .sim
                    .as_ref()
                    .and_then(|s| s.architecture.as_ref())
                    .is_some_and(|a| a.floor(level).is_some())
                {
                    self.floor = level;
                    self.selected = None;
                }
            }
            Message::Resident(id) => {
                if self
                    .sim
                    .as_ref()
                    .is_some_and(|s| s.interactions.contains_key(&id))
                {
                    self.resident = id;
                    self.floor = self.sim.as_ref().unwrap().bodies[&id].level;
                    if self.selected == Some(id) {
                        self.selected = None;
                    }
                }
            }
            Message::Select(target) => {
                self.selected = target.map(|id| {
                    self.sim
                        .as_ref()
                        .and_then(|s| s.vm.world.objects.get(&id))
                        .and_then(|o| o.multitile.as_ref())
                        .and_then(|part| part.head)
                        .unwrap_or(id)
                })
            }
            Message::Queue(target, index) => {
                if let (Some(l), Some(s)) = (&self.loaded, &mut self.sim) {
                    let result = s
                        .enqueue(&l.program, self.resident, target, index)
                        .and_then(|_| s.run_fixture_tick(&l.program, 2000));
                    match result {
                        Ok(step)
                            if !matches!(
                                step.stop,
                                StepStop::Faulted | StepStop::Budget | StepStop::Animation
                            ) =>
                        {
                            self.status = format!(
                                "Queued {}.",
                                interaction_label(&l.program, s, target, index)
                            )
                        }
                        Ok(step) => {
                            self.error = Some(format!("simulation stopped: {:?}", step.stop));
                            self.playing = false;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
            Message::Cancel(id) => {
                if let (Some(l), Some(s)) = (&self.loaded, &mut self.sim) {
                    let result = s
                        .cancel_interaction(&l.program, self.resident, id)
                        .and_then(|_| s.run_fixture_tick(&l.program, 2000));
                    match result {
                        Ok(step)
                            if !matches!(
                                step.stop,
                                StepStop::Faulted | StepStop::Budget | StepStop::Animation
                            ) =>
                        {
                            self.status = "Cancellation requested.".into()
                        }
                        Ok(step) => {
                            self.error = Some(format!("simulation stopped: {:?}", step.stop));
                            self.playing = false;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
            Message::Loaded(result) => match result {
                Ok(l) => {
                    self.loaded = Some(l);
                    let result = (|| -> anyhow::Result<()> {
                        self.reset().map_err(anyhow::Error::msg)?;
                        if let Some(path) = self.args.load.clone() {
                            self.load(&path)?;
                        }
                        if self.args.at_tick > 1_000_000 {
                            anyhow::bail!("initial tick exceeds viewer limit");
                        }
                        while self.sim.as_ref().unwrap().vm.tick < (self.args.at_tick as u64) {
                            if self.sim.as_ref().unwrap().vm.world.dialogs.modal.is_some() {
                                break;
                            }
                            self.tick().map_err(anyhow::Error::msg)?;
                        }
                        Ok(())
                    })();
                    if let Err(e) = result {
                        self.error = Some(e.to_string());
                        self.playing = false;
                    }
                }
                Err(e) => {
                    self.error = Some(e);
                    self.playing = false;
                }
            },
            Message::Frame(now) => {
                let dt = self
                    .last_frame
                    .replace(now)
                    .map(|t| now.duration_since(t).as_secs_f32())
                    .unwrap_or(0.)
                    .min(0.25);
                if self.playing
                    && self
                        .sim
                        .as_ref()
                        .is_some_and(|s| s.vm.world.dialogs.modal.is_none())
                {
                    self.accumulator += dt * self.speed;
                    while self.accumulator >= 0.04
                        && self
                            .sim
                            .as_ref()
                            .is_some_and(|s| s.vm.world.dialogs.modal.is_none())
                    {
                        match self.tick() {
                            Ok(()) => self.accumulator -= 0.04,
                            Err(e) => {
                                self.error = Some(e);
                                self.playing = false;
                                self.accumulator = 0.;
                                break;
                            }
                        }
                    }
                    self.refresh();
                }
                if self.scene.is_some() && self.args.screenshot.is_some() && !self.capturing {
                    self.frames = self.frames.saturating_add(1);
                    if self.frames > 8 {
                        self.capturing = true;
                        return window::latest()
                            .and_then(window::screenshot)
                            .map(Message::Captured);
                    }
                }
                return Task::none();
            }
            Message::Toggle => {
                self.playing = !self.playing;
                self.accumulator = 0.;
                self.last_frame = None;
            }
            Message::Step => {
                self.playing = false;
                self.accumulator = 0.;
                if let Err(e) = self.tick() {
                    self.error = Some(e);
                }
            }
            Message::Speed(speed) => self.speed = speed,
            Message::Restart => {
                if let Err(e) = self.reset() {
                    self.error = Some(e);
                }
            }
            Message::Rotate => {
                self.rotation = (self.rotation + 1) % 4;
                if let Some(s) = &mut self.sim {
                    s.vm.world.camera_rotation = self.rotation;
                }
            }
            Message::Zoom(zoom) => self.zoom = zoom,
            Message::Save => {
                if let (Some(l), Some(s)) = (&self.loaded, &self.sim) {
                    let result = (|| -> anyhow::Result<()> {
                        validate_output(&self.args.root, &self.args.save)?;
                        if let Some(parent) = self.args.save.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        atomic_save(
                            &self.args.save,
                            &serde_json::to_vec_pretty(&Snapshot::capture(&l.program, s))?,
                        )?;
                        Ok(())
                    })();
                    match result {
                        Ok(()) => {
                            self.status =
                                format!("Saved tick {} to {}", s.vm.tick, self.args.save.display())
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
            }
            Message::Load => {
                let p = self.args.save.clone();
                if let Err(e) = self.load(&p) {
                    self.error = Some(e.to_string());
                }
            }
            Message::Captured(capture) => {
                if let Some(path) = self.args.screenshot.take() {
                    let result = (|| -> anyhow::Result<()> {
                        validate_output(&self.args.root, &path)?;
                        if let Some(parent) = path.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        image::save_buffer(
                            &path,
                            &capture.rgba,
                            capture.size.width,
                            capture.size.height,
                            image::ColorType::Rgba8,
                        )?;
                        Ok(())
                    })();
                    match result {
                        Ok(()) => eprintln!("Saved window screenshot: {}", path.display()),
                        Err(e) => self.error = Some(e.to_string()),
                    }
                    if self.args.exit_after_screenshot {
                        return iced::exit();
                    }
                }
            }
        }
        self.refresh();
        Task::none()
    }
    fn choices(&self, target: i16) -> Result<Vec<sims_sim::interaction::MenuChoice>, String> {
        let (Some(l), Some(s)) = (&self.loaded, &self.sim) else {
            return Ok(vec![]);
        };
        let Some(def) =
            s.vm.world
                .objects
                .get(&target)
                .and_then(|o| o.definition.as_ref())
        else {
            return Ok(vec![]);
        };
        let Some(entries) = l
            .program
            .modules
            .get(&def.source)
            .and_then(|m| m.object_interactions.get(&def.object))
        else {
            return Ok(vec![]);
        };
        entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| supported_action(&def.source, entry.action, self.args.domestic))
            .map(|(i, _)| s.interaction_choice(&l.program, self.resident, target, i))
            .collect()
    }
    fn view(&self) -> Element<'_, Message> {
        let header = row![
            column![
                text(if self.args.two_floor {
                    "A home across two floors"
                } else if self.args.home {
                    "A home for two"
                } else if self.args.household {
                    "The autonomous household"
                } else {
                    "The two-chair room"
                })
                .size(27),
                text(if self.args.two_floor {
                    "The Sims · four lives, two floors"
                } else if self.args.home {
                    "The Sims · two lives under one roof"
                } else if self.args.household {
                    "The Sims · one Sim, a day of their own"
                } else {
                    "The Sims · choose a seat, queue your next move"
                })
                .size(14)
                .color(Color::from_rgb8(152, 173, 181))
            ]
            .spacing(6),
            Space::new().width(Fill),
            text("SIMS / RUST")
                .size(13)
                .color(Color::from_rgb8(97, 199, 179))
        ]
        .align_y(iced::Alignment::Center);
        let time = self.sim.as_ref().map_or_else(
            || "Loading…".into(),
            |s| {
                s.household.as_ref().map_or_else(
                    || format!("Tick {}", s.vm.tick),
                    |h| {
                        format!(
                            "Day {} · {:02}:{:02}",
                            h.clock.elapsed_days + 1,
                            h.clock.hour,
                            h.clock.minute
                        )
                    },
                )
            },
        );
        let mut toolbar = row![
            button(if self.playing { "Pause" } else { "Play" }).on_press(Message::Toggle),
            button("Step")
                .on_press(Message::Step)
                .style(button::secondary),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);
        for speed in [1., 2., 4., 16.] {
            toolbar = toolbar.push(
                button(text(format!("{speed:.0}×")))
                    .on_press(Message::Speed(speed))
                    .style(if self.speed == speed {
                        button::primary
                    } else {
                        button::secondary
                    }),
            );
        }
        toolbar = toolbar
            .push(text(time).size(16))
            .push(Space::new().width(Fill))
            .push(
                button("Save")
                    .on_press(Message::Save)
                    .style(button::secondary),
            )
            .push(
                button("Load")
                    .on_press(Message::Load)
                    .style(button::secondary),
            )
            .push(
                button("Restart")
                    .on_press(Message::Restart)
                    .style(button::secondary),
            )
            .push(
                button("Rotate view")
                    .on_press(Message::Rotate)
                    .style(button::secondary),
            );
        let mut controls = column![
            text(if self.playing { "LIVE" } else { "PAUSED" })
                .size(12)
                .color(Color::from_rgb8(97, 199, 179))
        ]
        .spacing(14)
        .width(Fill);
        if let Some(s) = &self.sim {
            if self.args.two_floor {
                controls = controls.push(
                    row![
                        button("Downstairs").on_press(Message::Floor(1)).style(
                            if self.floor == 1 {
                                button::primary
                            } else {
                                button::secondary
                            }
                        ),
                        button("Upstairs")
                            .on_press(Message::Floor(2))
                            .style(if self.floor == 2 {
                                button::primary
                            } else {
                                button::secondary
                            }),
                    ]
                    .spacing(8),
                );
            }
            if self.args.home {
                controls = controls.push(
                    row![
                        button("Sim 1").on_press(Message::Resident(1)).style(
                            if self.resident == 1 {
                                button::primary
                            } else {
                                button::secondary
                            }
                        ),
                        button("Sim 2").on_press(Message::Resident(13)).style(
                            if self.resident == 13 {
                                button::primary
                            } else {
                                button::secondary
                            }
                        )
                    ]
                    .spacing(8),
                );
                if self.args.two_floor {
                    controls = controls.push(
                        row![
                            button("Sim 3").on_press(Message::Resident(32)).style(
                                if self.resident == 32 {
                                    button::primary
                                } else {
                                    button::secondary
                                }
                            ),
                            button("Sim 4").on_press(Message::Resident(33)).style(
                                if self.resident == 33 {
                                    button::primary
                                } else {
                                    button::secondary
                                }
                            ),
                        ]
                        .spacing(8),
                    );
                    controls = controls.push(
                        text(format!(
                            "Selected Sim · {}",
                            if s.bodies[&self.resident].level == 1 {
                                "Downstairs"
                            } else {
                                "Upstairs"
                            }
                        ))
                        .size(12),
                    );
                }
                let neighbor = s.vm.world.objects[&self.resident].person.as_ref().unwrap()[&31];
                let other = if neighbor == 1 { 2 } else { 1 };
                let relationship =
                    s.vm.world
                        .person
                        .as_ref()
                        .and_then(|p| p.neighbors.get(&neighbor))
                        .and_then(|n| n.relationships.get(&other))
                        .and_then(|r| r.first())
                        .copied()
                        .unwrap_or(0);
                controls = controls.push(
                    text(format!(
                        "Relationship with {} · {relationship}",
                        if neighbor == 1 { "Sim 2" } else { "Sim 1" }
                    ))
                    .size(12),
                );
            }
            if s.routes.values().any(|r| r.caller == self.resident) {
                controls = controls.push(text("Walking").size(21));
            }
            if let Some(m) =
                s.vm.world
                    .objects
                    .get(&self.resident)
                    .and_then(|o| o.motives)
            {
                let pairs = if self.args.household || self.args.home {
                    vec![
                        [("Hunger", 7), ("Energy", 5)],
                        [("Comfort", 6), ("Hygiene", 8)],
                        [("Bladder", 9), ("Fun", 15)],
                        [("Social", 14), ("Room", 13)],
                    ]
                } else {
                    vec![[("Comfort", 6), ("Energy", 5)]]
                };
                for pair in pairs {
                    controls = controls.push(
                        row![
                            need_meter(pair[0].0, f32::from_bits(m[pair[0].1])),
                            need_meter(pair[1].0, f32::from_bits(m[pair[1].1]))
                        ]
                        .spacing(16),
                    );
                }
            }
        }
        let mut actions = column![
            text("SELECT AN OBJECT")
                .size(12)
                .color(Color::from_rgb8(97, 199, 179))
        ]
        .spacing(8);
        let targets = if self.args.household || self.args.home {
            self.sim
                .as_ref()
                .map(|s| {
                    s.bodies
                        .keys()
                        .copied()
                        .filter(|id| {
                            *id != self.resident
                                && (!self.args.two_floor || s.bodies[id].level == self.floor)
                                && s.vm.world.objects.get(id).is_some_and(|o| {
                                    o.multitile.as_ref().is_none_or(|p| p.head.is_none())
                                        && o.definition.as_ref().is_some_and(|d| {
                                            self.loaded.as_ref().is_some_and(|l| {
                                                l.program.modules[&d.source]
                                                    .object_interactions
                                                    .get(&d.object)
                                                    .is_some_and(|entries| {
                                                        entries.iter().any(|e| {
                                                            supported_action(
                                                                &d.source,
                                                                e.action,
                                                                self.args.domestic,
                                                            )
                                                        })
                                                    })
                                            })
                                        })
                                })
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            vec![2, 3]
        };
        for pair in targets.chunks(2) {
            let mut buttons = row![].spacing(8);
            for &target in pair {
                buttons = buttons.push(
                    button(text(object_label(self.sim.as_ref(), target)))
                        .on_press(Message::Select(Some(target)))
                        .width(Fill)
                        .style(if self.selected == Some(target) {
                            button::primary
                        } else {
                            button::secondary
                        }),
                );
            }
            actions = actions.push(buttons);
        }
        if let Some(target) = self.selected {
            actions = actions.push(text(object_label(self.sim.as_ref(), target)).size(16));
            match self.choices(target) {
                Ok(menu) if menu.is_empty() => {
                    actions =
                        actions.push(text("No supported actions for this object yet.").size(12))
                }
                Ok(menu) => {
                    for choice in menu {
                        actions = actions.push(
                            button(text(choice.label))
                                .on_press_maybe(
                                    choice
                                        .available
                                        .then_some(Message::Queue(target, choice.index)),
                                )
                                .padding(9)
                                .width(Fill),
                        );
                    }
                }
                Err(e) => actions = actions.push(text(e).size(12)),
            }
        } else {
            actions = actions.push(text("Click furniture in the room, or choose above.").size(12));
        }
        controls = controls.push(actions);
        let mut queue = column![
            text("INTERACTION QUEUE")
                .size(12)
                .color(Color::from_rgb8(97, 199, 179))
        ]
        .spacing(8);
        if let (Some(l), Some(s)) = (&self.loaded, &self.sim) {
            if let Some(q) = s.interactions.get(&self.resident) {
                if let Some(a) = &q.active {
                    use sims_sim::interaction::Phase;
                    let label = if a.exit_blocked {
                        "Retry exit"
                    } else if a.phase == Phase::Seated {
                        "Stand up"
                    } else if a.phase == Phase::Exiting {
                        "Standing up…"
                    } else if a.cancel_requested {
                        "Cancelling…"
                    } else {
                        "Cancel"
                    };
                    queue = queue
                        .push(
                            text(format!(
                                "Now · {}",
                                s.vm.world
                                    .actions
                                    .as_ref()
                                    .and_then(|w| w.actors.get(&self.resident))
                                    .and_then(|a| a.active.as_ref())
                                    .map_or_else(
                                        || interaction_label(
                                            &l.program,
                                            s,
                                            a.item.target,
                                            a.item.menu_index
                                        ),
                                        |a| queued_label(&l.program, s, a)
                                    )
                            ))
                            .size(14),
                        )
                        .push(
                            button(label)
                                .on_press_maybe(
                                    (a.phase != Phase::Exiting
                                        && (!a.cancel_requested || a.exit_blocked))
                                        .then_some(Message::Cancel(a.item.id)),
                                )
                                .style(button::secondary),
                        );
                }
                if let Some(pending) = s.pending_interactions(self.resident) {
                    for (i, item) in pending.iter().enumerate() {
                        queue = queue.push(
                            row![
                                text(format!("{} · {}", i + 1, queued_label(&l.program, s, item)))
                                    .size(13)
                                    .width(Fill),
                                button("×")
                                    .on_press(Message::Cancel(u64::from(item.id)))
                                    .style(button::secondary)
                            ]
                            .align_y(iced::Alignment::Center)
                            .spacing(8),
                        );
                    }
                    if q.active.is_none() && pending.is_empty() {
                        queue = queue.push(
                            text(if self.args.household || self.args.home {
                                "Choosing what to do next…"
                            } else {
                                "Nothing queued"
                            })
                            .size(13),
                        );
                    }
                }
                if let Some(result) = &q.last_result {
                    queue = queue.push(text(result).size(12));
                }
            }
        }
        controls = controls
            .push(queue)
            .push(text("Zoom").size(13))
            .push(slider(0.4..=1.8, self.zoom, Message::Zoom).step(0.05_f32));
        let viewport: Element<'_, Message> = if let Some(scene) = &self.scene {
            widget::shader::Shader::new(Viewport(scene.clone()))
                .width(Fill)
                .height(Fill)
                .into()
        } else {
            container(text("Loading original assets…"))
                .center(Fill)
                .into()
        };
        let content = row![
            container(viewport)
                .width(Fill)
                .height(Fill)
                .style(|_| container::Style::default().background(Color::from_rgb8(24, 32, 37))),
            container(widget::scrollable(controls).spacing(8).width(304))
                .width(328)
                .padding(12)
                .height(Fill)
        ]
        .spacing(16)
        .height(Fill);
        let mut page = column![header, toolbar, content, text(&self.status).size(12)]
            .spacing(16)
            .padding(24);
        if let Some(error) = &self.error {
            page = page.push(text(error).color(Color::from_rgb8(250, 137, 120)));
        }
        if let (Some(art), Some(record)) = (
            self.dialog_art.as_ref(),
            self.sim.as_ref().and_then(|s| {
                s.vm.world
                    .dialogs
                    .modal
                    .and_then(|id| s.vm.world.dialogs.records.iter().find(|r| r.id == id))
            }),
        ) {
            widget::stack![
                page,
                widget::opaque(
                    container(dialog_view::view(record, art, Message::DialogResponse)).center(Fill)
                )
            ]
            .into()
        } else {
            page.into()
        }
    }
}
#[derive(Clone)]
struct Viewport(Arc<Scene>);
impl widget::shader::Program<Message> for Viewport {
    type State = ();
    type Primitive = Primitive;
    fn update(
        &self,
        _: &mut (),
        event: &iced::Event,
        bounds: iced::Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> Option<widget::shader::Action<Message>> {
        if matches!(
            event,
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
        ) {
            let p = scene_cursor(&self.0, bounds, cursor)?;
            Some(widget::shader::Action::publish(Message::Select(self.0.pick(p))).and_capture())
        } else {
            None
        }
    }
    fn mouse_interaction(
        &self,
        _: &(),
        bounds: iced::Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> iced::mouse::Interaction {
        if scene_cursor(&self.0, bounds, cursor)
            .and_then(|p| self.0.pick(p))
            .is_some()
        {
            iced::mouse::Interaction::Pointer
        } else {
            iced::mouse::Interaction::default()
        }
    }
    fn draw(&self, _: &(), _: iced::mouse::Cursor, _: iced::Rectangle) -> Primitive {
        Primitive {
            scene: self.0.clone(),
        }
    }
}
pub(super) fn run(args: Args) -> anyhow::Result<()> {
    if args.headless_screenshot {
        let loaded = Loaded::load(&args)?;
        let path = args.screenshot.clone().unwrap();
        let (mut viewer, _) = Viewer::new(args);
        let _ = viewer.update(Message::Loaded(Ok(loaded)));
        if let Some(error) = &viewer.error {
            anyhow::bail!("{error}");
        }
        render_headless(&viewer, &path)?;
        println!("Saved headless Iced frame: {}", path.display());
        return Ok(());
    }
    iced::application(
        move || Viewer::new(args.clone()),
        Viewer::update,
        Viewer::view,
    )
    .title(|viewer: &Viewer| {
        if viewer.args.two_floor {
            "Sims · A home across two floors".into()
        } else if viewer.args.home {
            "Sims · A home for two".into()
        } else if viewer.args.household {
            "Sims · The autonomous household".into()
        } else {
            "Sims · The two-chair room".into()
        }
    })
    .window_size(Size::new(1320., 900.))
    .theme(|_: &Viewer| room_theme())
    .subscription(|_: &Viewer| window::frames().map(Message::Frame))
    .run()?;
    Ok(())
}

/// The same view tree and custom GPU primitive as the window, without a window
/// server, event loop, or screen capture. Rendering cannot advance simulation.
fn render_headless(viewer: &Viewer, path: &Path) -> anyhow::Result<()> {
    render_headless_element(&viewer.args.root, path, viewer.view())
}
#[cfg(test)]
#[path = "laboratory/tests.rs"]
mod tests;

// This is the supported subset of the authored demo, not a substitute for the
// original engine's menu visibility rules. Each choice still runs its original check.
fn supported_action(source: &str, action: u16, domestic: bool) -> bool {
    if source.to_ascii_lowercase().ends_with("fridges.iff") && action == 4107 {
        return domestic;
    }

    let source = source.to_ascii_lowercase();
    [
        ("templateperson.iff", &[0x2051][..]),
        ("diningchairs.iff", &[4097][..]),
        ("fridges.iff", &[4103, 4113, 4098, 4107][..]),
        ("beds.iff", &[4106][..]),
        ("food.iff", &[4151, 4170, 4196, 4201][..]),
        ("sinks.iff", &[4104][..]),
        ("toilets.iff", &[4098][..]),
        ("recliners.iff", &[4097, 4103][..]),
        ("showerc.iff", &[4098][..]),
        ("bookcases.iff", &[4110][..]),
        ("trashpile.iff", &[4097][..]),
        ("trash1.iff", &[4100][..]),
        ("flood.iff", &[4097][..]),
    ]
    .iter()
    .any(|(name, actions)| source.ends_with(name) && actions.contains(&action))
}
fn object_label(sim: Option<&Simulation>, target: i16) -> String {
    if let Some(index) = [1, 13, 32, 33].iter().position(|id| *id == target)
        && sim.is_some_and(|s| s.interactions.contains_key(&target))
    {
        return format!("Sim {}", index + 1);
    }
    if target == 13
        && sim.is_some_and(|s| {
            s.vm.world
                .objects
                .get(&13)
                .is_some_and(|o| o.person.is_some())
        })
    {
        return "Sim 2".into();
    }
    if let Some(def) = sim
        .and_then(|s| s.vm.world.objects.get(&target))
        .and_then(|o| o.definition.as_ref())
    {
        let source = def.source.to_ascii_lowercase();
        for (file, name) in [
            ("fridges.iff", "Fridge"),
            ("sinks.iff", "Sink"),
            ("toilets.iff", "Toilet"),
            (
                "recliners.iff",
                if sim.is_some_and(|s| s.interactions.contains_key(&13)) {
                    if target == 20 || target == 21 {
                        "Recliner 2"
                    } else {
                        "Recliner 1"
                    }
                } else {
                    "Recliner"
                },
            ),
            (
                "beds.iff",
                match target {
                    20 => "Bed 1",
                    23 => "Bed 2",
                    34 => "Bed 3",
                    _ => "Bed 4",
                },
            ),
            ("counters.iff", "Counter"),
            ("stoves.iff", "Stove"),
            ("diningtables.iff", "Dining table"),
            ("food.iff", "Food"),
            ("trashpile.iff", "Trash"),
            ("showerc.iff", "Shower"),
            ("bookcases.iff", "Bookcase"),
            ("trash1.iff", "Indoor bin"),
            ("trashoutside.iff", "Outdoor bin"),
            ("flood.iff", "Puddle"),
        ] {
            if source.ends_with(file) {
                return name.into();
            }
        }
        if source.ends_with("diningchairs.iff") {
            return format!(
                "Chair {}",
                match target {
                    40 => 3,
                    41 => 4,
                    _ => target - 1,
                }
            );
        }
    }
    format!("Object {target}")
}
fn interaction_label(program: &Program, sim: &Simulation, target: i16, index: usize) -> String {
    let label = entries(program, sim, target)
        .get(index)
        .map_or("Interaction", |e| &e.label);
    format!("{label} · {}", object_label(Some(sim), target))
}
fn queued_label(program: &Program, sim: &Simulation, item: &sims_vm::action::Action) -> String {
    let label = item
        .label
        .as_deref()
        // Native label reads clear 0x80 while retaining the cached text.
        .filter(|label| !label.is_empty())
        .or_else(|| {
            entries(program, sim, item.target)
                .iter()
                .find(|e| e.id == item.menu_id)
                .map(|e| e.label.as_str())
        })
        .unwrap_or("Interaction");
    format!(
        "{label} · {}",
        object_label(
            Some(sim),
            if item.icon_owner > 0 {
                item.icon_owner
            } else {
                item.target
            }
        )
    )
}
fn need_meter(name: &'static str, value: f32) -> Element<'static, Message> {
    let fraction = ((value + 100.) / 200.).clamp(0., 1.);
    column![
        text(name).size(12),
        progress_bar(0.0..=1.0, fraction)
            .girth(7)
            .style(move |theme: &Theme| {
                let mut style = progress_bar::primary(theme);
                style.bar = Color::from_rgb8(
                    if fraction < 0.3 { 230 } else { 70 },
                    if fraction < 0.3 { 121 } else { 175 },
                    132,
                )
                .into();
                style
            })
    ]
    .spacing(5)
    .width(Fill)
    .into()
}
