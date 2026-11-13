use super::super::super::super::super::headless_input::{ElementObservation, Input as UiInput};
use super::super::super::super::super::ui_publication::{
    JourneyGround, JourneyGroundInput, JourneyGroundPixel, Menu,
};
use super::*;
use sims_vm::action::Action;

#[path = "go_here/object_interaction.rs"]
mod object_interaction;
#[path = "go_here/walking.rs"]
mod walking;

pub(super) fn object_continuation(
    enabled: bool,
    continuation: impl FnOnce() -> Result<Value>,
) -> Result<Option<Value>> {
    if enabled {
        continuation().map(Some)
    } else {
        Ok(None)
    }
}

pub(super) fn run_object(journey: &mut Journey, prior: &Record) -> Result<Value> {
    object_interaction::run(journey, prior)
}

const BOB_OBJECT: i16 = 105;
const BOB_NEIGHBOR: i16 = 48;

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
struct Usage {
    frames: u32,
    service_polls: usize,
    effect_polls: usize,
    tasks: usize,
    commands: usize,
    pixel_scans: usize,
}

const LIMITS: Usage = Usage {
    frames: 128,
    service_polls: 128,
    effect_polls: 512,
    tasks: 128,
    commands: 256,
    pixel_scans: 64,
};

impl Usage {
    fn current(journey: &Journey) -> Self {
        Self {
            frames: journey.frames,
            service_polls: journey.service_polls,
            effect_polls: journey.effect_polls,
            tasks: journey.tasks,
            commands: journey.commands,
            pixel_scans: 0,
        }
    }

    fn since(self, start: Self) -> Result<Self> {
        Ok(Self {
            frames: self
                .frames
                .checked_sub(start.frames)
                .context("Go Here frame counter reset")?,
            service_polls: self
                .service_polls
                .checked_sub(start.service_polls)
                .context("Go Here service-poll counter reset")?,
            effect_polls: self
                .effect_polls
                .checked_sub(start.effect_polls)
                .context("Go Here effect-poll counter reset")?,
            tasks: self
                .tasks
                .checked_sub(start.tasks)
                .context("Go Here task counter reset")?,
            commands: self
                .commands
                .checked_sub(start.commands)
                .context("Go Here command counter reset")?,
            pixel_scans: self
                .pixel_scans
                .checked_sub(start.pixel_scans)
                .context("Go Here pixel-scan counter reset")?,
        })
    }

    fn check(self) -> Result<()> {
        ensure!(
            u64::from(self.frames)
                .checked_add(self.service_polls as u64)
                .is_some_and(|frames| frames <= u64::from(LIMITS.frames))
                && self.service_polls <= LIMITS.service_polls
                && self.effect_polls <= LIMITS.effect_polls
                && self.tasks <= LIMITS.tasks
                && self.commands <= LIMITS.commands
                && self.pixel_scans <= LIMITS.pixel_scans,
            "unsupported tutorial Go Here: separate phase budget exhausted: {self:?}"
        );
        Ok(())
    }

    fn reserve(self, delta: Self) -> Result<()> {
        Self {
            frames: self.frames.saturating_add(delta.frames),
            service_polls: self.service_polls.saturating_add(delta.service_polls),
            effect_polls: self.effect_polls.saturating_add(delta.effect_polls),
            tasks: self.tasks.saturating_add(delta.tasks),
            commands: self.commands.saturating_add(delta.commands),
            pixel_scans: self.pixel_scans.saturating_add(delta.pixel_scans),
        }
        .check()
    }

    fn run<T>(self, delta: Self, operation: impl FnOnce() -> T) -> Result<T> {
        self.reserve(delta)?;
        Ok(operation())
    }
}

fn active_task(task: Task<Event>) -> Option<Task<Event>> {
    (task.units() != 0).then_some(task)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
struct Cut {
    generation: u64,
    host_widget_epoch: u64,
    publication_widget_epoch: u64,
    popup_epoch: u64,
}

impl Cut {
    fn current(journey: &Journey) -> Self {
        let publication = journey.host.publication_for_test();
        Self {
            generation: journey.host.current_generation_for_test(),
            host_widget_epoch: journey.host.widget_epoch_for_test(),
            publication_widget_epoch: publication.widget_epoch,
            popup_epoch: publication.native_popup_epoch,
        }
    }

    fn require(self, current: Self) -> Result<()> {
        ensure!(
            self == current,
            "unsupported tutorial Go Here: generation/widget/popup epoch changed"
        );
        Ok(())
    }
}

fn instruction(expected: &Record, current: Option<&Record>, modal: bool) -> Result<()> {
    ensure!(
        !modal && current == Some(expected),
        "unsupported tutorial Go Here: instruction/modal changed before enqueue proof"
    );
    ensure!(
        expected.id > 0
            && expected.stack_object == BOB_NEIGHBOR
            && Lesson::identify(expected)? == Lesson::GoHere,
        "unsupported tutorial Go Here: current exact Go Here lesson unavailable"
    );
    Ok(())
}

fn bob(residents: &[Resident]) -> Result<&Resident> {
    let candidates: Vec<_> = residents
        .iter()
        .filter(|resident| {
            resident.name == "Bob"
                || resident.object == BOB_OBJECT
                || resident.neighbor == BOB_NEIGHBOR
        })
        .collect();
    ensure!(
        candidates.len() == 1,
        "unsupported tutorial Go Here: current Bob identity absent/ambiguous"
    );
    let bob = candidates[0];
    ensure!(
        bob.name == "Bob"
            && bob.object == BOB_OBJECT
            && bob.neighbor == BOB_NEIGHBOR
            && bob.enabled,
        "unsupported tutorial Go Here: current enabled Bob 105/neighbor 48 unavailable"
    );
    Ok(bob)
}

fn entry(journey: &Journey, expected: &Record, cut: Cut) -> Result<Value> {
    journey.bounded()?;
    journey.healthy()?;
    cut.require(Cut::current(journey))?;
    let publication = journey.host.publication_for_test();
    instruction(expected, publication.dialog.as_ref(), publication.modal)?;
    ensure!(
        journey.idle && journey.streams == 0 && !publication.pending,
        "unsupported tutorial Go Here: Host/task/presentation owners not settled"
    );
    ensure!(
        publication.menu.is_none() && publication.native_popup.is_none(),
        "unsupported tutorial Go Here: competing popup/menu at entry"
    );
    ensure!(
        publication.session_present
            && publication.playing
            && expanded_people_panel(
                publication.panel_state,
                publication.root_visible,
                publication.people_visible
            )
            && live_disclosure_available(
                authoritative_mode(publication),
                publication.button(Widget::Live)
            ),
        "unsupported tutorial Go Here: visible expanded native Live mode 2 unavailable"
    );
    let bob = bob(&publication.residents)?;
    require_native_resident(publication, bob.object)?;
    ensure!(
        publication
            .art
            .as_ref()
            .is_some_and(|art| art.resident(bob.neighbor).is_some())
            && publication
                .journey_tutorial_buttons
                .as_ref()
                .is_some_and(|buttons| {
                    buttons.initialized
                        && buttons
                            .items
                            .iter()
                            .filter(|button| button.neighbor == bob.neighbor)
                            .count()
                            == 1
                        && buttons.image(bob.neighbor).is_some()
                }),
        "unsupported tutorial Go Here: visible native Bob portrait identity unavailable"
    );
    let camera = publication
        .journey_camera
        .as_ref()
        .context("unsupported tutorial Go Here: retained camera absent")?
        .as_ref()
        .map_err(|error| {
            anyhow::anyhow!("unsupported tutorial Go Here: retained camera: {error}")
        })?;
    ensure!(
        camera.viewport[0..2] == [0, 0]
            && camera.viewport[2..4].iter().all(|&axis| axis >= 160)
            && (1..=3).contains(&camera.zoom)
            && camera.rotation < 4,
        "unsupported tutorial Go Here: retained local camera invalid"
    );
    let scene = publication
        .scene
        .as_ref()
        .context("unsupported tutorial Go Here: scene absent")?;
    ensure!(
        [scene.width, scene.height] == camera.viewport[2..4],
        "unsupported tutorial Go Here: scene/retained viewport mismatch"
    );
    let current_tick = tick(publication)?;
    ensure!(
        publication
            .journey_camera_watch
            .as_ref()
            .is_some_and(|watch| {
                watch.tick == current_tick
                    && watch.lesson.as_ref() == Some(expected)
                    && watch.camera.as_ref() == Some(camera)
                    && watch.owner == Some(expected.caller)
                    && watch.owner_source.as_deref() == Some(SOURCE)
                    && !watch.callback_pending
                    && watch.poll_pending != Some(true)
            }),
        "unsupported tutorial Go Here: current lesson/camera owner observation unavailable"
    );
    Ok(
        json!({"record":expected,"body_sha256":Lesson::GoHere.digest(),"cut":cut,
        "tick":current_tick,"bob":{"object":bob.object,"neighbor":bob.neighbor},
        "native_mode":authoritative_mode(publication),"camera":camera,
        "scene_size":[scene.width,scene.height],"hud_height":publication.hud_height,
        "response_sent":false,"floor_point_qualified":false}),
    )
}

fn ground(publication: &Published) -> Result<&JourneyGround> {
    publication
        .journey_ground
        .as_ref()
        .context("unsupported tutorial Go Here: opt-in native ground observation absent")?
        .as_ref()
        .map_err(|error| anyhow::anyhow!("native ground observation: {error}"))
}

fn rect(bounds: Rectangle) -> [f32; 4] {
    [bounds.x, bounds.y, bounds.width, bounds.height]
}

fn point(
    sample: &JourneyGroundPixel,
    floor: i32,
    selected_floor: i32,
    size: [u32; 2],
    lot_size: usize,
    viewport: Rectangle,
    exclusions: &[Rectangle],
) -> Result<(Point, [i32; 2])> {
    ensure!(
        floor == selected_floor && (1..=2).contains(&floor),
        "ground selected floor changed"
    );
    ensure!(
        sample.error.is_none(),
        "native ground pick failed: {:?}",
        sample.error
    );
    ensure!(
        !size.contains(&0)
            && sample
                .pixel
                .iter()
                .zip(size)
                .all(|(&axis, size)| axis.is_finite() && axis >= 20. && axis < size as f32 - 20.)
            && rect(viewport).iter().all(|axis| axis.is_finite())
            && viewport.width > 0.
            && viewport.height > 0.,
        "ground pixel/viewport nonfinite, outside bounds, or inside edge-pan strip"
    );
    let native = sample
        .point
        .context("native ground pick returned no floor point")?;
    ensure!(sample.picked.is_none(), "ground pixel picked an object");
    let end = i32::try_from(lot_size)?
        .checked_sub(1)
        .and_then(|size| size.checked_mul(16))
        .context("ground active lot bounds invalid")?;
    ensure!(
        native.iter().all(|&axis| axis >= 16 && axis < end),
        "ground point outside active lot interior"
    );
    let scale = (viewport.width / size[0] as f32).min(viewport.height / size[1] as f32);
    ensure!(
        scale.is_finite() && scale > 0.,
        "ground letterbox scale invalid"
    );
    ensure!(
        exclusions
            .iter()
            .all(|bounds| rect(*bounds).iter().all(|axis| axis.is_finite())
                && bounds.width >= 0.
                && bounds.height >= 0.),
        "ground frontend exclusion bounds invalid"
    );
    let position = Point::new(
        viewport.x + (viewport.width - size[0] as f32 * scale) / 2. + sample.pixel[0] * scale,
        viewport.y + (viewport.height - size[1] as f32 * scale) / 2. + sample.pixel[1] * scale,
    );
    ensure!(
        viewport.contains(position) && exclusions.iter().all(|bounds| !bounds.contains(position)),
        "ground point covered by HUD, queue, tutorial, or another frontend control"
    );
    Ok((position, native))
}

fn layout(
    observation: &ElementObservation,
    publication: &Published,
    native: &JourneyGround,
) -> Result<(Rectangle, Vec<Rectangle>)> {
    let [width, height] = native.display_size;
    let scene = publication
        .scene
        .as_ref()
        .context("ground layout scene absent")?;
    ensure!(
        height > publication.hud_height && width > 0,
        "ground display/HUD extent invalid"
    );
    let viewport = Rectangle::new(
        Point::ORIGIN,
        iced::Size::new(width as f32, (height - publication.hud_height) as f32),
    );
    ensure!(
        observation.layouts.contains(&viewport)
            && [scene.width, scene.height] == native.camera.viewport[2..4],
        "actual laid-out play viewport/current scene unavailable"
    );
    let mut exclusions = vec![Rectangle::new(
        Point::new(0., viewport.height),
        iced::Size::new(width as f32, publication.hud_height as f32),
    )];
    let card = observation
        .layouts
        .iter()
        .copied()
        .filter(|bounds| {
            bounds.x > 0.
                && bounds.y == 0.
                && bounds.width > 0.
                && bounds.height > 0.
                && bounds.height < viewport.height
                && (bounds.x + bounds.width - width as f32).abs() < 0.01
        })
        .max_by(|left, right| (left.width * left.height).total_cmp(&(right.width * right.height)))
        .context("actual laid-out top-right tutorial exclusion unavailable")?;
    exclusions.push(card);
    for text in &observation.texts {
        for bounds in [Some(text.bounds), text.container].into_iter().flatten() {
            if bounds.width < viewport.width
                && bounds.height < viewport.height
                && !exclusions.contains(&bounds)
            {
                exclusions.push(bounds);
            }
        }
    }
    Ok((viewport, exclusions))
}

#[derive(serde::Deserialize)]
struct PopupOwner {
    kind: sims_runtime::original_session::live_menu::Kind,
    tick: u64,
    actor: (i16, u32),
    target: (i16, u32),
    clicked: Option<(i16, u32)>,
    command: Option<sims_runtime::original_session::live_menu::Command>,
}

fn offered_index(
    menu: &Menu,
    input: &JourneyGroundInput,
    owner: &PopupOwner,
    actor_lifetime: u32,
    native_point: [i32; 2],
    floor: i32,
) -> Result<usize> {
    use sims_runtime::original_session::live_menu::Kind;
    ensure!(
        !menu.preview
            && menu.ready
            && menu.popup_visible
            && menu.error.is_none()
            && input.ready
            && !input.complete
            && !input.actions.is_empty()
            && owner.kind == Kind::Ground
            && owner.command.is_none()
            && owner.clicked.is_none()
            && owner.tick == input.tick
            && owner.actor == (BOB_OBJECT, actor_lifetime)
            && input.actor == BOB_OBJECT
            && input.target == Some(owner.target.0)
            && input.target_lifetime == Some(owner.target.1)
            && input.point == native_point
            && input.floor == floor,
        "unsupported tutorial Go Here: current ready nonpreview Bob-owned ground popup unavailable"
    );
    let choices: Vec<_> = menu
        .popup
        .iter()
        .filter_map(|choice| match choice.intent.as_ref() {
            Some(Intent::GroundChoice(index)) if choice.label == "Go Here" => Some(*index),
            _ => None,
        })
        .collect();
    ensure!(
        choices.len() == 1,
        "unsupported tutorial Go Here: actual offered Go Here action absent/ambiguous; heading is not an action"
    );
    let index = choices[0];
    ensure!(
        menu.popup.len() == input.actions.len()
            && menu.popup.iter().enumerate().all(|(index, choice)| matches!(choice.intent, Some(Intent::GroundChoice(actual)) if actual == index))
            && input.actions.get(index).is_some_and(|action| action.target == owner.target.0
                && action.icon_owner == action.target && action.id == 0 && action.priority == 50
                && action.args == [0; 4] && action.label.is_none()
                && action.flags & 28 == 12 && action.flags & !13 == 0),
        "unsupported tutorial Go Here: stale/mismatched ground choice/action target"
    );
    Ok(index)
}

#[derive(Clone)]
struct Offer {
    cut: Cut,
    owner: sims_runtime::original_session::live_menu::State,
    input: JourneyGroundInput,
    index: usize,
    action: Action,
    serial: u32,
    previous: Vec<Action>,
    actor_lifetime: u32,
}

fn same_offered_action(offer: &Offer, index: usize, input: &JourneyGroundInput) -> Result<()> {
    ensure!(
        index == offer.index && input.actions.get(index) == Some(&offer.action),
        "Go Here offered action changed before activation"
    );
    Ok(())
}

fn fresh_enqueue(offer: &Offer, native: &JourneyGround, closed: bool) -> Result<Option<u32>> {
    ensure!(
        native.actor == BOB_OBJECT
            && native.actor_lifetime == offer.actor_lifetime
            && native.floor == offer.input.floor,
        "native enqueue actor lifetime/selected floor changed"
    );
    let matches: Vec<_> = native
        .queue
        .iter()
        .filter(|action| {
            let mut expected = offer.action.clone();
            expected.id = action.id;
            action.id > offer.serial
                && action.id <= native.serial
                && !offer.previous.iter().any(|old| old.id == action.id)
                && **action == expected
                && native.queue_targets.get(&action.target) == Some(&offer.input.target_lifetime)
        })
        .collect();
    ensure!(
        matches.len() <= 1,
        "ambiguous new Bob Go Here enqueue serial"
    );
    Ok((closed && native.input.is_none() && !native.object_input)
        .then(|| matches.first().map(|action| action.id))
        .flatten())
}

fn first_instruction(
    expected: &Record,
    current: Option<&Record>,
    modal: bool,
    accepted: bool,
) -> Result<bool> {
    if modal || current.is_some_and(|record| record != expected) {
        ensure!(accepted, "instruction/modal changed before enqueue proof");
        return Ok(true);
    }
    ensure!(
        accepted || current == Some(expected),
        "instruction disappeared before enqueue proof"
    );
    Ok(false)
}

#[derive(Default, serde::Serialize)]
struct Proof {
    floor_point_qualified: bool,
    viewport_left_press_sent: bool,
    viewport_left_release_sent: bool,
    popup_qualified: bool,
    hover_qualified: bool,
    action_activated: bool,
    native_enqueue_accepted: bool,
    new_bob_queue_serial: Option<u32>,
}

struct Driver<'a> {
    journey: &'a mut Journey,
    expected: &'a Record,
    start: Usage,
    cut: Cut,
    pixel_scans: usize,
    base: Option<JourneyGround>,
    offer: Option<Offer>,
    proof: Proof,
    first_new: Option<Value>,
    walking_enabled: bool,
    walking_base: Option<walking::Native>,
    walking_accept: Option<(u64, sims_vm::autonomy::Placement)>,
    handoff: Option<walking::Handoff>,
    last: &'static str,
}

impl Driver<'_> {
    fn usage(&self) -> Result<Usage> {
        let mut usage = Usage::current(self.journey).since(self.start)?;
        usage.pixel_scans = self.pixel_scans;
        Ok(usage)
    }

    fn reserve(&self, delta: Usage) -> Result<()> {
        self.usage()?.reserve(delta)?;
        if self.walking_enabled {
            walking::preflight(self.journey, delta)?;
        }
        Ok(())
    }

    fn observe(&mut self) -> Result<()> {
        let publication = self.journey.host.publication_for_test().clone();
        let changed = publication.modal
            || publication
                .dialog
                .as_ref()
                .is_some_and(|record| record != self.expected);
        if changed {
            if self.first_new.is_none() {
                let first = json!({"instruction":publication.dialog,"modal":publication.modal,
                    "cut":Cut::current(self.journey),"tick_label":publication.tick_label,
                    "enqueue_proved_before_instruction":self.proof.native_enqueue_accepted,
                    "response_sent":false});
                self.journey
                    .events
                    .push(json!({"kind":"tutorial_go_here_first_instruction","observation":first}));
                self.first_new = Some(first);
            }
            first_instruction(
                self.expected,
                publication.dialog.as_ref(),
                publication.modal,
                self.proof.native_enqueue_accepted,
            )?;
            if self.walking_enabled {
                self.journey.bounded()?;
                self.journey.healthy()?;
                self.journey.host.frontend_for_test()?;
                self.usage()?.check()?;
                self.handoff = Some(walking::Handoff::capture(self)?);
            }
            self.last = "first_new_instruction_or_modal_after_accepted_enqueue_without_response";
            anyhow::bail!(
                "tutorial Go Here stopped at first new instruction/modal without responding"
            );
        }
        first_instruction(
            self.expected,
            publication.dialog.as_ref(),
            publication.modal,
            self.proof.native_enqueue_accepted,
        )?;
        let current = Cut::current(self.journey);
        self.cut.require(Cut {
            popup_epoch: self.cut.popup_epoch,
            ..current
        })?;
        self.journey.bounded()?;
        self.journey.healthy()?;
        self.journey.host.frontend_for_test()?;
        let native = ground(&publication)?;
        ensure!(
            native.widget_epoch == publication.widget_epoch
                && native.tick == tick(&publication)?
                && native.actor == BOB_OBJECT
                && !native.object_input,
            "current ground observation lost selected Bob/tick/widget ownership"
        );
        require_native_resident(&publication, BOB_OBJECT)?;
        bob(&publication.residents)?;
        self.usage()?.check()?;
        if let Some(base) = &self.base {
            ensure!(
                native.actor_lifetime == base.actor_lifetime && native.floor == base.floor,
                "ground actor lifetime/selected floor changed during Go Here"
            );
        }
        if self.proof.action_activated && !self.proof.native_enqueue_accepted {
            let offer = self
                .offer
                .as_ref()
                .context("activated ground offer absent")?;
            let closed = publication.native_popup.is_none() && publication.menu.is_none();
            if let Some(serial) = fresh_enqueue(offer, native, closed)? {
                if self.walking_enabled {
                    let observed = walking::native(&publication)?;
                    let target = observed
                        .targets
                        .get(&offer.action.target)
                        .context("walking accepted destination absent")?;
                    ensure!(
                        observed.tick == native.tick
                            && target.lifetime == offer.input.target_lifetime,
                        "walking accepted target/tick lifetime mismatch"
                    );
                    self.walking_accept = Some((
                        observed.tick,
                        target
                            .position
                            .context("walking accepted committed destination placement absent")?,
                    ));
                }
                self.proof.native_enqueue_accepted = true;
                self.proof.new_bob_queue_serial = Some(serial);
                self.last = "native_bob_enqueue_serial_action_target_and_popup_ground_closure";
                self.journey
                    .events
                    .push(json!({"kind":"tutorial_go_here_enqueue_accepted",
                    "actor":native.actor,"actor_lifetime":native.actor_lifetime,"serial":serial,
                    "offered_action":offer.action,"target_lifetime":offer.input.target_lifetime,
                    "queue":native.queue,"popup_closed":true,"ground_closed":true,
                    "tick":native.tick,"submission_is_not_acceptance":true}));
            }
        }
        Ok(())
    }

    fn submit(&mut self, task: Task<Event>) -> Result<()> {
        let Some(task) = active_task(task) else {
            return self.usage()?.check();
        };
        self.reserve(Usage {
            tasks: 1,
            ..Default::default()
        })?;
        self.journey.submit(task)?;
        self.usage()?.check()
    }

    fn update(&mut self, event: Event) -> Result<()> {
        self.observe()?;
        if self.walking_enabled {
            self.reserve(Usage {
                commands: 1,
                effect_polls: 1,
                tasks: 1,
                ..Default::default()
            })?;
        }
        let task = self.usage()?.run(
            Usage {
                commands: 1,
                effect_polls: 1,
                tasks: 1,
                ..Default::default()
            },
            || {
                if !matches!(event, Event::Completion) {
                    self.journey.events.push(
                        json!({"kind":"tutorial_go_here_host_input","event":format!("{event:?}")}),
                    );
                }
                self.journey.commands += 1;
                self.journey.effect_polls += 1;
                self.journey.host.update(event)
            },
        )?;
        walking::keep_then_observe(
            self.walking_enabled,
            self,
            task,
            |driver, task| {
                if driver.walking_enabled {
                    let registered = walking::register_returned(driver.journey, task);
                    let checked = driver.usage()?.check();
                    registered.and(checked)
                } else {
                    driver.submit(task)
                }
            },
            Self::observe,
        )
    }

    fn drain(&mut self) -> Result<()> {
        self.journey.idle = false;
        loop {
            self.observe()?;
            if self.journey.streams == 0 {
                self.update(Event::Completion)?;
                if self.journey.streams == 0 && self.journey.host.frontend_idle_for_test() {
                    self.journey.idle = true;
                    return self.observe();
                }
                continue;
            }
            self.reserve(Usage {
                effect_polls: 1,
                ..Default::default()
            })?;
            self.journey.effect_polls += 1;
            let remaining =
                (self.journey.wall_seconds - self.journey.wall.elapsed().as_secs_f64()).max(0.001);
            let delivery = match self
                .journey
                .receiver
                .recv_timeout(Duration::from_secs_f64(remaining.min(0.1)))
            {
                Ok(delivery) => delivery,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(error) => return Err(error.into()),
            };
            match delivery {
                Delivery::Finished => {
                    self.journey.streams = self
                        .journey
                        .streams
                        .checked_sub(1)
                        .context("Go Here stream counter underflow")?;
                }
                Delivery::Action(iced_runtime::Action::Output(event)) => {
                    let event = if self.walking_enabled {
                        walking::notification_event(event)?
                    } else {
                        event
                    };
                    self.update(event)?;
                }
                Delivery::Action(action) => anyhow::bail!(
                    "Go Here cannot complete unsupported Iced runtime action: {action:?}"
                ),
            }
        }
    }

    fn frame(&mut self, advance: bool) -> Result<()> {
        self.observe()?;
        self.reserve(if advance {
            Usage {
                frames: 1,
                ..Default::default()
            }
        } else {
            Usage {
                service_polls: 1,
                ..Default::default()
            }
        })?;
        if advance {
            self.journey.frames += 1;
            self.journey.millis += 40;
        } else {
            self.journey.service_polls += 1;
        }
        self.update(Event::Frame(
            self.journey.epoch + Duration::from_millis(self.journey.millis),
        ))?;
        self.drain()
    }

    fn send(&mut self, publication: &Published, message: Message) -> Result<()> {
        self.observe()?;
        self.reserve(Usage {
            commands: 1,
            effect_polls: 1,
            tasks: 1,
            ..Default::default()
        })?;
        if matches!(message, Message::ViewportButton { down: true, .. }) {
            ensure!(
                !self.proof.viewport_left_press_sent,
                "duplicate Go Here ground press"
            );
            self.proof.viewport_left_press_sent = true;
        }
        if matches!(message, Message::ViewportButton { down: false, .. }) {
            self.proof.viewport_left_release_sent = true;
        }
        if matches!(message, Message::GroundChoice(_)) {
            ensure!(
                !self.proof.action_activated,
                "duplicate Go Here action activation"
            );
            self.proof.action_activated = true;
        }
        let event = self.journey.host.ui_event_for_test(publication, message);
        self.update(event)?;
        self.drain()
    }

    fn batch(
        &mut self,
        input: &mut UiInput,
        events: &[iced::Event],
    ) -> Result<(Arc<Published>, Vec<Message>, Vec<iced::event::Status>)> {
        self.observe()?;
        let publication = self.journey.host.publication_for_test().clone();
        let native = ground(&publication)?;
        let batch = input.element_batch(
            self.journey.host.frontend_for_test()?.view(),
            native.display_size,
            events,
        );
        Ok((publication, batch.messages, batch.statuses))
    }

    fn observation(&mut self, input: &mut UiInput) -> Result<ElementObservation> {
        self.observe()?;
        let publication = self.journey.host.publication_for_test().clone();
        let observation = input.element_observation(
            self.journey.host.frontend_for_test()?.view(),
            ground(&publication)?.display_size,
        )?;
        ensure!(
            observation.messages.is_empty(),
            "observational redraw emitted application messages: {:?}",
            observation.messages
        );
        Ok(observation)
    }

    fn motion(
        &mut self,
        input: &mut UiInput,
        event: iced::Event,
        pixel: Option<[f32; 2]>,
    ) -> Result<()> {
        let (publication, messages, _) = self.batch(input, &[event])?;
        if let Some(pixel) = pixel {
            ensure!(
                messages.iter().any(|message| matches!(message,
                Message::ViewportMotion { pixel: Some(actual), pan: [0, 0] } if *actual == pixel)),
                "real viewport did not publish the qualified local ground pixel"
            );
        }
        for message in messages {
            ensure!(
                matches!(
                    message,
                    Message::ViewportEnter
                        | Message::ViewportLeave
                        | Message::ViewportMotion { pan: [0, 0], .. }
                ),
                "cursor observation emitted a non-motion command: {message:?}"
            );
            self.send(&publication, message)?;
        }
        Ok(())
    }

    fn current_offer(&self) -> Result<()> {
        let offer = self.offer.as_ref().context("Go Here offer absent")?;
        offer.cut.require(Cut::current(self.journey))?;
        let publication = self.journey.host.publication_for_test();
        ensure!(
            publication.native_popup.as_ref() == Some(&offer.owner),
            "Go Here popup lifetime/command changed"
        );
        let native = ground(publication)?;
        let input = native
            .input
            .as_ref()
            .context("Go Here ground owner ended")?;
        let owner: PopupOwner = serde_json::from_value(serde_json::to_value(&offer.owner)?)?;
        let index = offered_index(
            publication.menu.as_ref().context("Go Here menu ended")?,
            input,
            &owner,
            offer.actor_lifetime,
            offer.input.point,
            offer.input.floor,
        )?;
        same_offered_action(offer, index, input)
    }

    fn execute(&mut self) -> Result<()> {
        let evidence = entry(self.journey, self.expected, self.cut)?;
        if self.walking_enabled {
            ensure!(
                self.journey.host.frontend_idle_for_test(),
                "walking prefix requires zero queued Host commands at entry"
            );
            self.walking_base =
                Some(walking::native(self.journey.host.publication_for_test())?.clone());
        }
        self.last = "current_exact_go_here_selected_bob_visible_live_settled";
        self.journey
            .events
            .push(json!({"kind":"tutorial_go_here_entry","evidence":evidence}));
        self.observe()?;
        let publication = self.journey.host.publication_for_test().clone();
        let base = ground(&publication)?.clone();
        ensure!(
            Arc::ptr_eq(
                &base.scene,
                publication.scene.as_ref().context("ground scene absent")?
            ) && publication
                .journey_camera
                .as_ref()
                .and_then(|input| input.as_ref().ok())
                == Some(&base.camera),
            "native ground probe/current scene or retained input differs"
        );
        self.base = Some(base.clone());
        let mut input = UiInput::new()?;
        let geometry = self.observation(&mut input)?;
        let (viewport, exclusions) = layout(&geometry, &publication, &base)?;
        let mut qualified = None;
        'grid: for row in [9, 7, 11, 5, 13, 3, 15, 1] {
            for column in [3, 5, 7, 9, 11, 13, 1, 15] {
                let pixel = [
                    base.scene.width as f32 * column as f32 / 16.,
                    base.scene.height as f32 * row as f32 / 16.,
                ];
                let sample = self.usage()?.run(
                    Usage {
                        pixel_scans: 1,
                        ..Default::default()
                    },
                    || {
                        self.pixel_scans += 1;
                        base.sample(pixel)
                    },
                )??;
                if let Some(error) = &sample.error {
                    anyhow::bail!("native ground pick failed after counted probe: {error}");
                }
                if let Ok((position, native_point)) = point(
                    &sample,
                    base.floor,
                    base.floor,
                    [base.scene.width, base.scene.height],
                    base.lot_size,
                    viewport,
                    &exclusions,
                ) {
                    qualified = Some((sample, position, native_point));
                    break 'grid;
                }
            }
        }
        let (sample, position, native_point) = qualified.context(
            "unsupported tutorial Go Here: bounded native ground probes produced no UI-uncovered empty floor point",
        )?;
        self.proof.floor_point_qualified = true;
        self.last = "current_native_empty_selected_floor_point_and_actual_layout_exclusions";
        self.journey.events.push(json!({"kind":"tutorial_go_here_ground_point","sample":sample,
            "floor":base.floor,"tick":base.tick,"retained_input":base.camera,"cut":self.cut,
            "actor":base.actor,"actor_lifetime":base.actor_lifetime,"viewport":rect(viewport),
            "exclusions":exclusions.into_iter().map(rect).collect::<Vec<_>>(),
            "position":[position.x,position.y],"probe_count":self.pixel_scans,"reachability_proved":false}));
        self.motion(
            &mut input,
            iced::Event::Mouse(mouse::Event::CursorMoved { position }),
            Some(sample.pixel),
        )?;
        self.cut.require(Cut::current(self.journey))?;
        let current = ground(self.journey.host.publication_for_test())?;
        ensure!(
            current.tick == base.tick
                && current.camera == base.camera
                && current.display_size == base.display_size
                && self.journey.host.publication_for_test().hud_height == publication.hud_height
                && Arc::ptr_eq(&current.scene, &base.scene),
            "current native ground geometry changed before click; point is not reused"
        );
        let (source, messages, statuses) = self.batch(
            &mut input,
            &[
                iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ],
        )?;
        ensure!(
            statuses == [iced::event::Status::Captured; 2]
                && messages.len() == 2
                && matches!(&messages[0], Message::ViewportButton { down: true, pixel, picked: None } if *pixel == sample.pixel)
                && matches!(&messages[1], Message::ViewportButton { down: false, pixel, picked: None } if *pixel == sample.pixel),
            "actual viewport press/release did not emit the single qualified empty-ground click: {messages:?}"
        );
        for message in messages {
            self.send(&source, message)?;
        }
        self.last = "real_viewport_left_click_dispatched_once";
        loop {
            self.observe()?;
            let publication = self.journey.host.publication_for_test().clone();
            let native = ground(&publication)?;
            if native.input.as_ref().is_some_and(|input| input.ready) {
                let input = native.input.as_ref().unwrap();
                let owner = publication
                    .native_popup
                    .as_ref()
                    .context("ready ground menu lacks actual running popup owner")?;
                let metadata: PopupOwner = serde_json::from_value(serde_json::to_value(owner)?)?;
                let index = offered_index(
                    publication
                        .menu
                        .as_ref()
                        .context("ready ground menu publication absent")?,
                    input,
                    &metadata,
                    base.actor_lifetime,
                    native_point,
                    base.floor,
                )?;
                self.offer = Some(Offer {
                    cut: Cut::current(self.journey),
                    owner: owner.clone(),
                    input: input.clone(),
                    index,
                    action: input.actions[index].clone(),
                    serial: native.serial,
                    previous: native.queue.clone(),
                    actor_lifetime: base.actor_lifetime,
                });
                break;
            }
            ensure!(
                native.input.is_some() || publication.pending,
                "unsupported tutorial Go Here: ground query closed without an offered action (including same-tile empty fallback)"
            );
            self.frame(false)?;
        }
        self.proof.popup_qualified = true;
        self.last = "current_ready_nonpreview_bob_ground_popup_actual_go_here_choice";
        let offer = self.offer.as_ref().unwrap().clone();
        self.journey.events.push(json!({"kind":"tutorial_go_here_popup","cut":offer.cut,"owner":offer.owner,
            "ground":offer.input,"index":offer.index,"offered_action":offer.action,"native_serial_before":offer.serial}));
        self.motion(
            &mut input,
            iced::Event::Mouse(mouse::Event::CursorLeft),
            None,
        )?;
        self.current_offer()?;
        let away = self.observation(&mut input)?;
        let buttons: Vec<_> = away
            .texts
            .iter()
            .filter(|text| text.text == "Go Here")
            .collect();
        ensure!(
            buttons.len() == 1,
            "actual laid-out Go Here button absent/ambiguous"
        );
        let button = buttons[0];
        let bounds = button
            .container
            .context("actual Go Here button container unavailable")?;
        let center = button.bounds.center();
        ensure!(
            bounds.contains(center),
            "actual Go Here text center outside its button"
        );
        let native = ground(self.journey.host.publication_for_test())?;
        ensure!(
            center.y
                < (native.display_size[1] - self.journey.host.publication_for_test().hud_height)
                    as f32
                && !away.layouts.iter().any(|card| card.x > 0.
                    && card.y == 0.
                    && (card.x + card.width - native.display_size[0] as f32).abs() < 0.01
                    && card.height
                        < (native.display_size[1]
                            - self.journey.host.publication_for_test().hud_height)
                            as f32
                    && card.contains(center)),
            "actual offered Go Here button is covered by tutorial/HUD"
        );
        self.motion(
            &mut input,
            iced::Event::Mouse(mouse::Event::CursorMoved { position: center }),
            None,
        )?;
        self.current_offer()?;
        let hover = self.observation(&mut input)?;
        ensure!(
            hover.interaction == mouse::Interaction::Pointer
                && hover
                    .texts
                    .iter()
                    .filter(|text| text.text == "Go Here"
                        && text.container == Some(bounds)
                        && text.bounds.contains(center))
                    .count()
                    == 1,
            "actual offered Go Here button did not produce real hover interaction/draw evidence"
        );
        self.proof.hover_qualified = true;
        self.last = "actual_frontend_go_here_button_hover_interaction_and_draw";
        self.journey.events.push(json!({"kind":"tutorial_go_here_hover","cut":offer.cut,"index":offer.index,
            "button_bounds":rect(bounds),"position":[center.x,center.y],"label":"Go Here",
            "away_interaction":format!("{:?}",away.interaction),"hover_interaction":format!("{:?}",hover.interaction),
            "path":"actual Frontend styled button -> retained UserInterface update -> UserInterface draw",
            "draw_commands_observed":true,"gpu_execution_or_raster_readback_proved":false,
            "original_foot_cursor_proved":false,"cursor_idle_artwork_is_not_foot_cursor_proof":true}));
        self.current_offer()?;
        let (source, messages, statuses) = self.batch(
            &mut input,
            &[
                iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ],
        )?;
        ensure!(
            statuses == [iced::event::Status::Captured; 2]
                && messages.len() == 1
                && matches!(messages[0], Message::GroundChoice(index) if index == offer.index),
            "actual offered Go Here button did not emit exactly one genuine GroundChoice: {messages:?}"
        );
        self.current_offer()?;
        for message in messages {
            self.send(&source, message)?;
        }
        if !self.proof.native_enqueue_accepted {
            self.last = "actual_go_here_button_activated_submission_only";
        }
        loop {
            self.frame(true)?;
        }
    }
}

pub(super) fn run(journey: &mut Journey, expected: &Record) -> Result<Value> {
    run_prefix(journey, expected, false).map(|(receipt, _)| receipt)
}

pub(super) fn run_walking(journey: &mut Journey, expected: &Record) -> Result<Value> {
    let (_, handoff) = run_prefix(journey, expected, true)?;
    walking::run(journey, handoff.context("walking live handoff absent")?)
}

fn run_prefix(
    journey: &mut Journey,
    expected: &Record,
    walking_enabled: bool,
) -> Result<(Value, Option<walking::Handoff>)> {
    let start = Usage::current(journey);
    let cut = Cut::current(journey);
    journey.events.push(json!({"kind":"tutorial_go_here_start","phase_limits":LIMITS,
        "expected_instruction":expected,"cut":cut,"camera_transition_frame_limit_unchanged":TRANSITION_FRAMES,
        "camera_total_frame_limit_unchanged":TOTAL_FRAMES}));
    let mut driver = Driver {
        journey,
        expected,
        start,
        cut,
        pixel_scans: 0,
        base: None,
        offer: None,
        proof: Proof::default(),
        first_new: None,
        walking_enabled,
        walking_base: None,
        walking_accept: None,
        handoff: None,
        last: "camera_bob_handoff",
    };
    let result = driver.execute();
    if walking_enabled {
        if driver.journey.streams != 0 || !driver.journey.host.frontend_idle_for_test() {
            driver.journey.idle = false;
        }
        driver.journey.events.push(json!({"kind":"tutorial_walking_prefix_task_handback",
            "registered_streams":driver.journey.streams,"total_registered_tasks":driver.journey.tasks,
            "idle":driver.journey.idle,"reason":result.as_ref().err().map(|error| format!("{error:#}")),
            "cleanup":"same Journey retains all registered streams; no cancellation, forced drain, or owner recovery"}));
    }
    let completed = driver.proof.native_enqueue_accepted
        && driver.first_new.is_some()
        && (!walking_enabled
            || (driver.handoff.is_some() && result.as_ref().err().is_some_and(|error| {
                error.to_string()
                    == "tutorial Go Here stopped at first new instruction/modal without responding"
            })));
    let publication = driver.journey.host.publication_for_test();
    let receipt = json!({"kind":"tutorial_go_here_stop","status":if completed {"enqueue_and_first_instruction_boundary"} else {"unsupported"},
        "partial":!completed,"reason":result.as_ref().err().map(|error| format!("{error:#}")),
        "last_proven_boundary":driver.last,"proof":driver.proof,"usage":driver.usage().ok(),"phase_limits":LIMITS,
        "current_instruction":publication.dialog,"modal":publication.modal,"first_new_instruction":driver.first_new,
        "instruction_changed_before_enqueue_proof":!driver.proof.native_enqueue_accepted
            && (publication.modal || publication.dialog.as_ref() != Some(expected)),
        "nonmodal_response_sent":false,"movement_qualified":false,"arrival_qualified":false,"betty_qualified":false,
        "household_switching_qualified":false,"tutorial_completion_qualified":false,"strict_original_parity_ticks":0,
        "generic_journey_continuation_permitted":false});
    driver.journey.events.push(receipt.clone());
    if completed {
        Ok((receipt, driver.handoff.take()))
    } else {
        result?;
        anyhow::bail!("Go Here stopped without accepted enqueue and first instruction")
    }
}

#[cfg(test)]
#[path = "go_here/tests.rs"]
mod boundary_tests;

#[cfg(test)]
#[cfg(feature = "private-fixtures")]
#[path = "go_here/private_tests.rs"]
#[rustfmt::skip]
mod tests;
