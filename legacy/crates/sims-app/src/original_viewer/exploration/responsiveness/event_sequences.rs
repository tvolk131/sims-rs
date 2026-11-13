//! Generated replacement-client input-lifetime checks. Driver RNG is independent
//! of all game streams. Exact materialized batches, not just seeds, are replayed.
//! These internal invariants add no native behavior/appearance parity credit.
use super::*;
use modal_lifetimes::pointer;
use sims_vm::interface::buy::input::Event as Navigate;

#[path = "event_sequences/modal_transition.rs"]
mod modal_transition;

const SIZES: [[u32; 2]; 3] = [[800, 600], [1000, 740], [1320, 900]];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Key {
    Enter,
    Escape,
    Space,
}
impl Key {
    fn code(self) -> u32 {
        match self {
            Self::Enter => 13,
            Self::Escape => 27,
            Self::Space => 32,
        }
    }
    fn event(self, down: bool) -> iced::Event {
        use iced::keyboard::{self, key};
        let (named, physical) = match self {
            Self::Enter => (key::Named::Enter, key::Code::Enter),
            Self::Escape => (key::Named::Escape, key::Code::Escape),
            Self::Space => (key::Named::Space, key::Code::Space),
        };
        let logical = keyboard::Key::Named(named);
        iced::Event::Keyboard(if down {
            keyboard::Event::KeyPressed {
                key: logical.clone(),
                modified_key: logical,
                physical_key: key::Physical::Code(physical),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::empty(),
                text: None,
                repeat: false,
            }
        } else {
            keyboard::Event::KeyReleased {
                key: logical.clone(),
                modified_key: logical,
                physical_key: key::Physical::Code(physical),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::empty(),
            }
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Event {
    // Preserve exact f32 bits, including coordinates after scroll transforms.
    Move([u32; 2]),
    Down,
    Up,
    Focus(bool),
    Resize([u32; 2]),
    Wheel(i16),
    Escape,
    Key { key: Key, down: bool },
}
impl Event {
    fn moved(point: [f32; 2]) -> Self {
        Self::Move(point.map(f32::to_bits))
    }
    fn iced(&self) -> iced::Event {
        match *self {
            Self::Move(p) => iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                position: iced::Point::new(f32::from_bits(p[0]), f32::from_bits(p[1])),
            }),
            Self::Down => pointer(true),
            Self::Up => pointer(false),
            Self::Focus(true) => iced::Event::Window(window::Event::Focused),
            Self::Focus(false) => iced::Event::Window(window::Event::Unfocused),
            Self::Resize(p) => {
                iced::Event::Window(window::Event::Resized(Size::new(p[0] as f32, p[1] as f32)))
            }
            Self::Wheel(y) => iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { x: 0., y: y as f32 },
            }),
            Self::Key { key, down } => key.event(down),
            Self::Escape => {
                use iced::keyboard::{self, key};
                iced::Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(key::Named::Escape),
                    modified_key: keyboard::Key::Named(key::Named::Escape),
                    physical_key: key::Physical::Code(key::Code::Escape),
                    location: keyboard::Location::Standard,
                    modifiers: keyboard::Modifiers::empty(),
                    text: None,
                    repeat: false,
                })
            }
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Action {
    ObjectChoice(usize),
    ObjectCancel,
    Dialog(u64, u32),
    Live,
    Buy,
    Needs,
    Options,
    Resident(i16),
    Navigate(u64, Navigate),
}
impl Action {
    fn message(&self) -> Message {
        match *self {
            Self::ObjectChoice(i) => Message::ObjectChoice(i),
            Self::ObjectCancel => Message::ObjectCancel,
            Self::Dialog(id, result) => Message::Dialog(id, result),
            Self::Live => Message::Live,
            Self::Buy => Message::Buy,
            Self::Needs => Message::Needs,
            Self::Options => Message::Options,
            Self::Resident(id) => Message::Resident(id),
            Self::Navigate(revision, event) => Message::BuyNavigate(revision, event),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Step {
    events: Vec<Event>,
    // Independent expected widget activations. Window/key subscription outputs
    // follow all widget messages, as in the pinned production runtime.
    actions: Vec<Action>,
    flush: bool,
    scroll_preserved: bool,
    reason: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Episode {
    barriers: Vec<Event>,
    // A second real runtime update occurs before any Viewer message application.
    split: usize,
    flush_before_release: bool,
    hover_first: bool,
}
fn episodes(seed: u64) -> Vec<Episode> {
    let mut rng = engine::Random::new(seed);
    (0..3)
        .map(|_| {
            let count = 1 + (rng.next_u64() % 3) as usize;
            let barriers = (0..count)
                .map(|_| {
                    if rng.next_u64() & 1 == 0 {
                        Event::Focus(false)
                    } else {
                        Event::Resize(SIZES[(rng.next_u64() % 3) as usize])
                    }
                })
                .collect();
            Episode {
                barriers,
                split: (rng.next_u64() % (count + 1) as u64) as usize,
                flush_before_release: rng.next_u64() & 1 == 0,
                hover_first: rng.next_u64() & 1 == 0,
            }
        })
        .collect()
}

/// A deliberately small lifetime oracle: interrupted presses cannot activate;
/// a new press after all interrupts may activate once. It has no game state/RNG.
fn released_actions(events: &[Event], held: &mut bool) -> usize {
    let mut count = 0;
    for event in events {
        match event {
            Event::Down => *held = true,
            Event::Up => {
                count += usize::from(*held);
                *held = false;
            }
            Event::Focus(false) | Event::Resize(_) => *held = false,
            _ => {}
        }
    }
    count
}
#[test]
fn generated_lifetimes_have_independent_replayable_seeds_and_oracle() -> Result<()> {
    let mut sizes = std::collections::BTreeSet::new();
    let mut focus = 0;
    let mut delayed = 0;
    let mut serialized = std::collections::BTreeSet::new();
    for seed in 0..16 {
        let plan = episodes(seed);
        ensure!(plan == episodes(seed));
        let bytes = serde_json::to_vec(&plan)?;
        ensure!(serde_json::from_slice::<Vec<Episode>>(&bytes)? == plan);
        serialized.insert(bytes);
        for episode in plan {
            let mut held = false;
            ensure!(released_actions(&[Event::Down], &mut held) == 0 && held);
            for event in &episode.barriers {
                match event {
                    Event::Resize(size) => {
                        sizes.insert(*size);
                    }
                    Event::Focus(false) => focus += 1,
                    _ => unreachable!(),
                }
            }
            ensure!(released_actions(&episode.barriers, &mut held) == 0 && !held);
            ensure!(released_actions(&[Event::Up], &mut held) == 0);
            ensure!(released_actions(&[Event::Down, Event::Up, Event::Up], &mut held) == 1);
            delayed += usize::from(!episode.flush_before_release);
        }
    }
    ensure!(serialized.len() == 16 && sizes.len() == 3 && focus > 0 && delayed > 0);
    Ok(())
}
#[test]
fn exact_event_roundtrip_and_bounded_failure_reduction() -> Result<()> {
    let trace = vec![
        Event::moved([0.25, -0.0]),
        Event::Down,
        Event::Focus(false),
        Event::Up,
    ];
    ensure!(serde_json::from_slice::<Vec<Event>>(&serde_json::to_vec(&trace)?)? == trace);
    // Reduction must preserve the actual prerequisites and same failure class;
    // a missing press/release is not accepted as the original stale activation.
    let original = vec![
        Event::Focus(true),
        Event::Down,
        Event::Focus(false),
        Event::Up,
        Event::Focus(true),
    ];
    let (reduced, attempts) = engine::minimize(&original, 12, |events| {
        Ok(events
            .windows(3)
            .any(|w| w == [Event::Down, Event::Focus(false), Event::Up]))
    })?;
    ensure!(reduced == [Event::Down, Event::Focus(false), Event::Up] && attempts <= 12);
    let mut held = false;
    ensure!(released_actions(&reduced, &mut held) == 0);
    Ok(())
}

fn ambient(m: &Message) -> bool {
    matches!(
        m,
        Message::ViewportEnter
            | Message::ViewportLeave
            | Message::ViewportMotion { .. }
            | Message::ViewportButton { down: false, .. }
    )
}
fn expected(step: &Step) -> Vec<Message> {
    let mut result: Vec<_> = step.actions.iter().map(Action::message).collect();
    for e in &step.events {
        match *e {
            Event::Focus(false) => result.push(Message::FocusLost),
            Event::Resize(s) => {
                result.push(Message::DisplaySize(Size::new(s[0] as f32, s[1] as f32)))
            }
            Event::Escape => result.push(Message::DialogKey(27)),
            Event::Key { key, down: true } => result.push(Message::DialogKey(key.code())),
            _ => {}
        }
    }
    result
}
fn labels(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .filter(|m| !ambient(m))
        .map(|m| format!("{m:?}"))
        .collect()
}
fn control(ui: &mut Input, app: &Application, action: &Action) -> Result<[f32; 2]> {
    let label = format!("{:?}", action.message());
    let before = app.observe()?;
    let restore_cursor = ui.cursor_event();
    let owners = ui.application_batch(&app.viewer, &[]).owners;
    let mut found = None;
    for point in ui.batch_centers(&app.viewer) {
        let events = [Event::moved(point).iced(), pointer(true), pointer(false)];
        let actual = ui.application_batch(&app.viewer, &events);
        if labels(&actual.messages) == [label.clone()] {
            found = Some(point);
            break;
        }
    }
    // Trial clicks never apply application messages. They must also not leave
    // MouseArea's hover state on a trial world point: a later genuine HUD move
    // would then publish a fictitious native move-tool Leave. Restore through
    // actual events from the same-layout journaled HUD point, not private state.
    let restored = ui.application_batch(&app.viewer, &[restore_cursor]);
    ensure!(
        labels(&restored.messages).is_empty() && restored.owners == owners,
        "control discovery changed scroll/focus owners"
    );
    ensure!(
        before == app.observe()?,
        "control discovery applied app messages"
    );
    found.with_context(|| format!("runtime control unavailable: {label}"))
}

struct Run<'a> {
    app: Application,
    reference: Application,
    ui: &'a mut Input,
    out: PathBuf,
    steps: Vec<Step>,
    rows: Vec<Value>,
    pending: Vec<Message>,
    intended: Vec<Message>,
    initial: Observation,
    seed: u64,
    campaign: String,
    acknowledgements: Vec<Value>,
}
impl<'a> Run<'a> {
    fn new(
        mut app: Application,
        ui: &'a mut Input,
        out: PathBuf,
        seed: u64,
        campaign: &str,
    ) -> Result<Self> {
        std::fs::create_dir(&out)?;
        ui.reset();
        // Establish HUD hover before the neutral baseline, through real input.
        // World hover is intentionally avoided: it is native move-tool input.
        let point = [
            app.viewer.display_size[0] as f32 * 0.5,
            app.viewer.display_size[1] as f32 - 2.,
        ];
        let batch = ui.application_batch(&app.viewer, &[Event::moved(point).iced()]);
        for m in batch.messages {
            app.dispatch(m)?;
        }
        let initial = app.observe()?;
        std::fs::write(out.join("initial.json"), app.checkpoint()?)?;
        let reference = app.fork()?;
        ensure!(reference.observe()? == initial);
        Ok(Self {
            app,
            reference,
            ui,
            out,
            steps: vec![],
            rows: vec![],
            pending: vec![],
            intended: vec![],
            initial,
            seed,
            campaign: campaign.into(),
            acknowledgements: vec![],
        })
    }
    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() && self.intended.is_empty() {
            return Ok(());
        }
        // Application-message boundaries are part of replay just as event
        // batches are. Do not silently apply pending messages between records.
        self.step(Step {
            events: vec![],
            actions: vec![],
            flush: true,
            scroll_preserved: false,
            reason: "explicit delayed-message flush".into(),
        })
    }
    fn apply_pending(&mut self) -> Result<()> {
        let viewport = self.ui.viewport_bounds(&self.app.viewer)?;
        let zoom = self.app.viewer.zoom;
        // Resize and native mode transitions have their own layout semantics.
        // Popup dismissal, focus/scroll/hover and resident/Needs controls must
        // not incidentally change the lot viewport's size or render scale.
        let modal_answer = modal_transition::expected_answer(&self.reference, &self.intended)?;
        let layout_change = modal_answer.is_some()
            || self
                .intended
                .iter()
                .any(|m| matches!(m, Message::DisplaySize(_) | Message::Buy | Message::Live));
        for m in self.pending.drain(..) {
            self.app.dispatch(m)?;
        }
        for m in self.intended.drain(..) {
            self.reference.dispatch(m)?;
        }
        if let Some(id) = modal_answer {
            let actual = modal_transition::continuation(&self.app, id)?;
            let reference = modal_transition::continuation(&self.reference, id)?;
            ensure!(
                actual == reference && self.app.observe()? == self.reference.observe()?,
                "modal_answer_dispatch_divergence"
            );
            self.acknowledgements
                .push(json!({"id":id,"phase":"ordinary_answer_before_drain",
                "batch_boundary":self.steps.len(),"actual":actual,"reference":reference}));
        }
        self.app.drain(&Limits::default())?;
        self.reference.drain(&Limits::default())?;
        if let Some(id) = modal_answer {
            let actual = modal_transition::continuation(&self.app, id)?;
            let reference = modal_transition::continuation(&self.reference, id)?;
            ensure!(actual == reference, "modal_continuation_divergence");
            self.acknowledgements
                .push(json!({"id":id,"phase":"ordinary_continuation_after_drain",
                "batch_boundary":self.steps.len(),"actual":actual,"reference":reference}));
            write_json(
                &self.out.join("acknowledgements.json"),
                &json!({"phases":self.acknowledgements}),
            )?;
        }
        ensure!(
            self.app.observe()? == self.reference.observe()?,
            "retained_state_divergence"
        );
        let after_viewport = self.ui.viewport_bounds(&self.app.viewer)?;
        ensure!(
            after_viewport == self.ui.viewport_bounds(&self.reference.viewer)?,
            "viewport_geometry_divergence"
        );
        ensure!(
            layout_change || (after_viewport == viewport && self.app.viewer.zoom == zoom),
            "passive_or_popup_viewport_jump"
        );
        Ok(())
    }
    fn step(&mut self, step: Step) -> Result<()> {
        self.steps.push(step.clone());
        // Journal the exact materialized events before running them: even an
        // unexpected panic/restore/handler failure has a pinned replay prefix.
        write_json(
            &self.out.join("replay.json"),
            &json!({"schema":1,"campaign":self.campaign,"seed":self.seed,"initial":self.initial,"steps":self.steps}),
        )?;
        let before = self.app.observe()?;
        let viewport = self.ui.viewport_bounds(&self.app.viewer)?;
        let previous = self.ui.application_batch(&self.app.viewer, &[]).owners;
        let events: Vec<_> = step.events.iter().map(Event::iced).collect();
        let started = Instant::now();
        let batch = self.ui.application_batch(&self.app.viewer, &events);
        let latency_us = started.elapsed().as_micros();
        let wanted = expected(&step);
        let actual = labels(&batch.messages);
        let expected = labels(&wanted);
        let state_unchanged = before == self.app.observe()?;
        let scroll_equal = previous == batch.owners;
        let scroll_moved = previous
            .scrolls
            .iter()
            .zip(&batch.owners.scrolls)
            .any(|(a, b)| a.translation != b.translation);
        let row = json!({"index":self.steps.len()-1,"step":step,"actual":actual,"expected":expected,
            "statuses":batch.statuses.iter().map(|s|format!("{s:?}")).collect::<Vec<_>>(),
            "all_messages":batch.messages.iter().map(|m|format!("{m:?}")).collect::<Vec<_>>(),
            "owners_before":previous,"owners_after":batch.owners,"widget_batch_us":latency_us,
            "before":before,"viewport_before":viewport,"widget_batch_state_neutral":state_unchanged,"scroll_moved":scroll_moved});
        self.rows.push(row);
        write_json(&self.out.join("journal.json"), &json!({"rows":self.rows}))?;
        ensure!(state_unchanged, "widget_update_mutated_app");
        ensure!(
            actual == expected,
            "unexpected_activation: actual={actual:?}, expected={expected:?}"
        );
        ensure!(
            !step.scroll_preserved || scroll_equal,
            "focus_lost_scroll_owner"
        );
        ensure!(
            step.reason != "actual catalogue scroll" || scroll_moved,
            "generated_scroll_did_not_move"
        );
        self.pending.extend(batch.messages);
        self.intended.extend(wanted);
        if step.flush {
            self.apply_pending()?;
        }
        Ok(())
    }
    fn events(&mut self, events: Vec<Event>, flush: bool, reason: &str) -> Result<()> {
        self.step(Step {
            events,
            actions: vec![],
            flush,
            scroll_preserved: reason == "focus preserves scroll",
            reason: reason.into(),
        })
    }
    fn locate(&mut self, action: &Action) -> Result<[f32; 2]> {
        let safe = [
            self.app.viewer.display_size[0] as f32 * 0.5,
            self.app.viewer.display_size[1] as f32 - 2.,
        ];
        self.events(
            vec![Event::moved(safe)],
            true,
            "journaled HUD hover before control lookup",
        )?;
        if let Ok(point) = control(self.ui, &self.app, action) {
            return Ok(point);
        }
        // The native view may enable a control that is below the actual Iced
        // scroll viewport. Reveal it with recorded real input; never bypass
        // clipping or substitute direct activation for a hidden widget.
        let wheel = match action {
            Action::Navigate(_, Navigate::Next) => -3,
            Action::Navigate(_, Navigate::Category(_) | Navigate::Back) => 3,
            _ => anyhow::bail!("rendered control unavailable: {action:?}"),
        };
        for _ in 0..8 {
            let owners = self.ui.application_batch(&self.app.viewer, &[]).owners;
            let scroll = owners
                .scrolls
                .iter()
                .find(|s| s.content[3] > s.bounds[3] + 1.)
                .context("target absent and no overflowing scroll viewport")?;
            let point = [
                scroll.bounds[0] + scroll.bounds[2] * 0.75,
                scroll.bounds[1] + scroll.bounds[3] * 0.5,
            ];
            self.events(
                vec![Event::moved(point), Event::Wheel(wheel)],
                true,
                "scroll to rendered control",
            )?;
            if let Ok(point) = control(self.ui, &self.app, action) {
                return Ok(point);
            }
        }
        anyhow::bail!("control remains absent after bounded real scrolling: {action:?}")
    }
    fn click(&mut self, action: Action, reason: &str) -> Result<()> {
        self.flush()?;
        let point = self.locate(&action)?;
        self.step(Step {
            events: vec![Event::moved(point), Event::Down, Event::Up],
            actions: vec![action],
            flush: true,
            scroll_preserved: false,
            reason: reason.into(),
        })
    }
    fn finish(&mut self) -> Result<()> {
        self.flush()?;
        let before = self.app.observe()?;
        let cold = self.app.restore(&self.app.checkpoint()?)?;
        ensure!(cold.observe()? == before, "cold_restore_divergence");
        let final_viewport = self.ui.viewport_bounds(&self.app.viewer)?;
        self.ui
            .batch_save(&self.app.viewer, &self.out.join("milestone.png"))?;
        ensure!(self.app.observe()? == before, "milestone_draw_mutated_app");
        write_json(
            &self.out.join("result.json"),
            &json!({"passed":true,"initial":self.initial,"final":before,
            "completed_ticks":before.tick-self.initial.tick,"batches":self.steps.len(),"cold_equal":true,
            "viewport_final":final_viewport,"scope":"legacy inferred private input; internal Iced/handler invariants, zero native parity credit"}),
        )?;
        Ok(())
    }
    fn preserve_error(&mut self, error: &anyhow::Error) -> Result<()> {
        write_json(
            &self.out.join("first-failure.json"),
            &json!({"error":format!("{error:#}"),"completed_steps":self.steps.len(),"rows":self.rows}),
        )?;
        std::fs::write(self.out.join("failure.json"), self.app.checkpoint()?)?;
        std::fs::write(
            self.out.join("failure-reference.json"),
            self.reference.checkpoint()?,
        )?;
        write_json(
            &self.out.join("failure-observations.json"),
            &json!({
            "actual":self.app.observe()?,"reference":self.reference.observe()?}),
        )?;
        // Rendering is diagnostic only. Preserve the original error even if a
        // failed application state can no longer produce a frame/checkpoint.
        let before = self.app.observe()?;
        let draw = self
            .ui
            .batch_save(&self.app.viewer, &self.out.join("failure.png"));
        ensure!(self.app.observe()? == before, "failure_frame_mutated_app");
        draw
    }
}

fn interrupt(run: &mut Run<'_>, episode: &Episode, action: Action) -> Result<()> {
    run.flush()?;
    let point = run.locate(&action)?;
    if episode.hover_first {
        run.events(vec![Event::moved(point)], true, "hover actual control")?;
    }
    run.events(
        vec![Event::moved(point), Event::Down],
        false,
        "retain held press",
    )?;
    let (a, b) = episode.barriers.split_at(episode.split);
    if !a.is_empty() {
        run.events(a.to_vec(), false, "first delayed interruption batch")?;
    }
    let mut second = b.to_vec();
    second.push(Event::Focus(true));
    if !episode.flush_before_release {
        second.push(Event::Up);
    }
    run.events(
        second,
        episode.flush_before_release,
        "second delayed interruption batch",
    )?;
    if episode.flush_before_release {
        let safe = [
            run.app.viewer.display_size[0] as f32 * 0.5,
            run.app.viewer.display_size[1] as f32 - 2.,
        ];
        run.events(
            vec![Event::moved(safe), Event::Up],
            true,
            "stale release after message flush at current HUD",
        )?;
    } else {
        run.flush()?;
    }
    // Stay within the declared HUD-only grammar. A small window's old HUD
    // coordinate can lie in the world after enlarging; native Enter/Leave then
    // has legitimate move-tool semantics and must not be called passive.
    let safe = [
        run.app.viewer.display_size[0] as f32 * 0.5,
        run.app.viewer.display_size[1] as f32 - 2.,
    ];
    run.events(
        vec![Event::moved(safe), Event::Focus(false), Event::Focus(true)],
        true,
        "focus preserves scroll",
    )?;
    Ok(())
}
fn revision(app: &Application) -> Result<u64> {
    Ok(app
        .viewer
        .device
        .as_ref()
        .context("device absent")?
        .panel_widgets()
        .context("panel absent")?
        .catalog()
        .context("catalog absent")?
        .revision())
}
fn prepare_popup(base: &Application) -> Result<Application> {
    let mut app = base.fork()?;
    select(&mut app, 196)?;
    app.execute(
        &Command::BeginObject {
            identity: Identity::get(app.session()?, 140)?,
        },
        &Limits::default(),
    )?;
    ensure!(app.session()?.object_input_pending() && app.menu().is_none());
    app.drain(&Limits::default())?;
    ensure!(app.menu().is_some_and(|m| !m.actions().is_empty()));
    Ok(app)
}

fn campaign(kind: &str) -> Result<()> {
    let out = output()?;
    let base = if kind == "modal" {
        modal_lifetimes::load_car_pool(&out)?
    } else {
        load(&out)?
    };
    let replay = std::env::var("SIMS_HUD_BATCH_REPLAY")
        .ok()
        .map(|path| -> Result<Value> { Ok(serde_json::from_slice(&std::fs::read(path)?)?) })
        .transpose()?;
    if let Some(record) = &replay {
        ensure!(
            record["schema"] == 1 && record["campaign"] == kind,
            "replay schema/campaign mismatch"
        );
    }
    let selected_seed = replay
        .as_ref()
        .map(|r| r["seed"].as_u64().context("replay seed absent"))
        .transpose()?
        .or(std::env::var("SIMS_HUD_BATCH_SEED")
            .ok()
            .map(|s| s.parse())
            .transpose()?);
    let seeds: Vec<u64> = if let Some(seed) = selected_seed {
        vec![seed]
    } else {
        match kind {
            "popup" => (0..8).collect(),
            "catalog" => (8..14).collect(),
            _ => (14..16).collect(),
        }
    };
    let mut ui = Input::new()?;
    let mut results = vec![];
    for seed in seeds {
        let app = if kind == "popup" {
            prepare_popup(&base)?
        } else {
            base.fork()?
        };
        let mut run = Run::new(app, &mut ui, out.join(format!("seed-{seed}")), seed, kind)?;
        write_json(
            &run.out.join("plan.json"),
            &json!({"seed":seed,"episodes":episodes(seed),"driver_rng":"SplitMix64 independent of game RNG"}),
        )?;
        let result = (|| -> Result<()> {
            if let Some(record) = &replay {
                let initial: Observation = serde_json::from_value(record["initial"].clone())?;
                ensure!(
                    run.initial == initial,
                    "replay initial owner/state/RNG mismatch"
                );
                let steps: Vec<Step> = serde_json::from_value(record["steps"].clone())?;
                for step in steps {
                    run.step(step)?;
                }
                return run.finish();
            }
            if kind == "catalog" {
                run.click(Action::Buy, "intentional native Buy transition")?;
                run.click(
                    Action::Navigate(revision(&run.app)?, Navigate::Category(1)),
                    "intentional Surfaces category",
                )?;
            }
            for episode in &episodes(seed) {
                let action = match kind {
                    "popup" => Action::ObjectChoice(0),
                    "catalog" => Action::Navigate(revision(&run.app)?, Navigate::Next),
                    _ => Action::Dialog(149, 2),
                };
                interrupt(&mut run, episode, action)?;
                if kind == "catalog" {
                    let owners = run.ui.application_batch(&run.app.viewer, &[]).owners;
                    let scroll = owners
                        .scrolls
                        .iter()
                        .find(|s| s.content[3] > s.bounds[3] + 1.)
                        .context("no overflowing catalog scroll")?;
                    let p = [
                        scroll.bounds[0] + scroll.bounds[2] * 0.75,
                        scroll.bounds[1] + scroll.bounds[3] * 0.5,
                    ];
                    run.events(
                        vec![
                            Event::moved(p),
                            Event::Wheel(if scroll.translation[1] > 0. { 1 } else { -2 }),
                        ],
                        true,
                        "actual catalogue scroll",
                    )?;
                    run.click(
                        Action::Navigate(revision(&run.app)?, Navigate::Category(1)),
                        "fresh category after cancelled press",
                    )?;
                }
            }
            match kind {
                "popup" if seed & 1 == 0 => {
                    run.click(Action::ObjectCancel, "fresh deliberate menu close")?
                }
                "popup" => run.events(
                    vec![Event::Escape],
                    true,
                    "intentional Escape menu dismissal",
                )?,
                "catalog" => {
                    run.click(Action::Live, "intentional native Live transition")?;
                    let residents: Vec<_> = run
                        .app
                        .session()?
                        .house
                        .household
                        .as_ref()
                        .context("household absent")?
                        .residents
                        .iter()
                        .map(|r| r.object as i16)
                        .collect();
                    let resident = residents[(seed as usize) % residents.len()];
                    run.click(Action::Resident(resident), "intentional resident selection")?;
                    run.click(Action::Needs, "fresh live-panel input")?;
                    run.click(Action::Buy, "resumed deliberate Buy input")?;
                }
                // Do not acknowledge a real suspended modal just to make a test
                // easy; ordinary newer deliberate buttons are covered elsewhere.
                _ => {}
            }
            run.finish()
        })();
        if let Err(error) = result {
            let evidence = run.preserve_error(&error).err().map(|e| e.to_string());
            results.push(json!({"seed":seed,"passed":false,"error":format!("{error:#}"),"evidence_error":evidence}));
            write_json(
                &out.join("report.json"),
                &json!({"campaign":kind,"results":results,"first_failure_seed":seed}),
            )?;
            return Err(error);
        }
        results.push(json!({"seed":seed,"passed":true,"batches":run.steps.len()}));
    }
    write_json(
        &out.join("report.json"),
        &json!({"campaign":kind,"results":results,"strict_original_parity_qualified":false,
        "provenance":"independent historical managed private household, legacy owner fields retained by ordinary restore",
        "latency":"diagnostic concurrent wall time only; no thresholds or speedup claims"}),
    )?;
    Ok(())
}
#[test]
#[ignore = "private legacy retained household, sixteen bounded generated seeds across three campaigns; offscreen Iced"]
fn generated_popup_batches() -> Result<()> {
    run_worker(|| campaign("popup"))
}
#[test]
#[ignore = "private legacy retained household; exact generated catalogue input batches"]
fn generated_catalog_batches() -> Result<()> {
    run_worker(|| campaign("catalog"))
}
#[test]
#[ignore = "private real suspended Car Pool continuation; exact generated modal batches"]
fn generated_modal_batches() -> Result<()> {
    run_worker(|| campaign("modal"))
}
