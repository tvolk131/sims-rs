pub(super) use super::super::super::super::super::super::ui_publication::JourneyWalking as Native;
use super::*;
use sims_vm::autonomy::Placement;

const WALL_SECONDS: f64 = 10.0;
const TICK_LIMIT: u64 = 256;
const FRAME_TICK_RESERVATION: u64 = 16;
const BOUNDARY: &str = "walking stopped at first subsequent instruction/modal without responding";

pub(super) fn native(publication: &Published) -> Result<&Native> {
    publication
        .journey_walking
        .as_ref()
        .context("walking observer absent")?
        .as_ref()
        .map_err(|error| anyhow::anyhow!("walking observer: {error}"))
}

pub(super) fn preflight(journey: &Journey, delta: Usage) -> Result<()> {
    journey.bounded()?;
    global_capacity(Usage::current(journey), delta)
}

fn global_capacity(current: Usage, delta: Usage) -> Result<()> {
    ensure!(
        current
            .commands
            .checked_add(delta.commands)
            .is_some_and(|value| value < MAX_COMMANDS)
            && current
                .tasks
                .checked_add(delta.tasks)
                .is_some_and(|value| value < MAX_COMMANDS)
            && current
                .frames
                .checked_add(delta.frames)
                .is_some_and(|value| value < MAX_FRAMES)
            && current
                .service_polls
                .checked_add(delta.service_polls)
                .is_some_and(|value| value < MAX_POLLS)
            && current
                .effect_polls
                .checked_add(delta.effect_polls)
                .is_some_and(|value| value < MAX_POLLS),
        "walking next dispatch exceeds an inherited global ceiling"
    );
    Ok(())
}

pub(super) fn register_returned(journey: &mut Journey, task: Task<Event>) -> Result<()> {
    if let Some(task) = active_task(task) {
        let mut stream =
            iced_runtime::task::into_stream(task).context("nonempty walking task has no stream")?;
        journey.streams += 1;
        journey.tasks += 1;
        journey.idle = false;
        let sender = journey.sender.clone();
        journey.executor.spawn_ok(async move {
            while let Some(action) = stream.next().await {
                if sender.send(Delivery::Action(action)).is_err() {
                    return;
                }
            }
            let _ = sender.send(Delivery::Finished);
        });
    }
    journey.bounded()
}

pub(super) fn keep_then_observe<State, Output>(
    enabled: bool,
    state: &mut State,
    task: Task<Event>,
    keep: impl FnOnce(&mut State, Task<Event>) -> Result<()>,
    observe: impl FnOnce(&mut State) -> Result<Output>,
) -> Result<Output> {
    if !enabled {
        let observed = observe(state)?;
        keep(state, task)?;
        return Ok(observed);
    }
    let kept = keep(state, task);
    let observed = observe(state);
    kept?;
    observed
}

fn exact_modal(prior: &Record, current: &Record) -> Result<()> {
    instruction(prior, Some(prior), false)?;
    ensure!(
        current.id > prior.id
            && current.caller == prior.caller
            && current.caller > 0
            && current.source == SOURCE
            && current.origin.source == SOURCE
            && current.origin.bhav == 4125
            && current.origin.instruction == 5
            && current.operands == [0, 1, 16, 0, 0, 4, 0, 6]
            && current.stack_object == BOB_NEIGHBOR
            && current.status == 1
            && current.modal()
            && current.title == "Tutorial"
            && current.body == "Good, now watch as Bob walks to that location."
            && current.presentation
                == Some(sims_vm::dialog::presentation::Presentation {
                    anchored: true,
                    hide_title: true,
                    large: false,
                })
            && current.bitmap_icon == Some(5001)
            && current.icon_object.is_none()
            && current.job_icon.is_none()
            && current.global_bitmap_icon.is_none()
            && current.portrait.is_none()
            && current.buttons.len() == 1
            && current.buttons[0].label == "OK"
            && current.buttons[0].result == 2,
        "walking exact same-run modal metadata mismatch"
    );
    Ok(())
}

#[derive(serde::Serialize)]
pub(super) struct Handoff {
    prior: Record,
    modal: Record,
    instruction_cut: Cut,
    cut: Cut,
    actor_lifetime: u32,
    caller_lifetime: u32,
    target_lifetime: u32,
    accepted: Action,
    requested_destination: Placement,
    destination: Placement,
    enqueue_tick: u64,
    initial: Native,
    retained_streams: usize,
    retained_tasks: usize,
}

impl Handoff {
    pub(super) fn capture(driver: &Driver<'_>) -> Result<Self> {
        let publication = driver.journey.host.publication_for_test();
        let modal = publication
            .dialog
            .as_ref()
            .context("walking first modal record absent")?;
        ensure!(publication.modal, "walking handoff is not a modal");
        exact_modal(driver.expected, modal)?;
        let offer = driver
            .offer
            .as_ref()
            .context("walking accepted offer absent")?;
        let initial = native(publication)?.clone();
        let base = driver
            .walking_base
            .as_ref()
            .context("walking same-run entry absent")?;
        let serial = driver
            .proof
            .new_bob_queue_serial
            .context("walking accepted serial absent")?;
        ensure!(
            driver.proof.native_enqueue_accepted && serial > offer.serial,
            "walking enqueue was not genuinely accepted"
        );
        let mut accepted = offer.action.clone();
        accepted.id = serial;
        let (enqueue_tick, destination) = driver
            .walking_accept
            .context("walking same-run committed enqueue destination absent")?;
        ensure!(
            enqueue_tick <= initial.tick,
            "walking enqueue tick reversed"
        );
        let handoff = Self {
            prior: driver.expected.clone(),
            modal: modal.clone(),
            instruction_cut: driver.cut,
            cut: Cut::current(driver.journey),
            actor_lifetime: offer.actor_lifetime,
            caller_lifetime: base
                .owner_lifetime
                .context("walking entry caller lifetime absent")?,
            target_lifetime: offer
                .input
                .target_lifetime
                .context("walking target lifetime absent")?,
            accepted,
            requested_destination: Placement {
                x: offer.input.point[0],
                y: offer.input.point[1],
                level: offer.input.floor,
            },
            destination,
            enqueue_tick,
            initial,
            retained_streams: driver.journey.streams,
            retained_tasks: driver.journey.tasks,
        };
        ensure!(
            base.actor == BOB_OBJECT
                && base.actor_lifetime == Some(handoff.actor_lifetime)
                && base.owner == Some(handoff.prior.caller),
            "walking entry owner mismatch"
        );
        driver.cut.require(Cut {
            popup_epoch: driver.cut.popup_epoch,
            ..handoff.cut
        })?;
        handoff.owners(publication, true)?;
        Ok(handoff)
    }

    fn owners(&self, publication: &Published, queued: bool) -> Result<()> {
        self.persistent_owners(publication)?;
        self.native_owners(native(publication)?, queued)
    }

    fn persistent_owners(&self, publication: &Published) -> Result<()> {
        require_native_resident(publication, BOB_OBJECT)?;
        bob(&publication.residents)?;
        ensure!(
            native(publication)?.tick == tick(publication)?
                && publication.native_popup.is_none()
                && publication.menu.is_none()
                && publication.native_clock
                && publication.native_transport
                && authoritative_mode(publication) == Some(2),
            "walking competing presentation owner"
        );
        self.native_identity(native(publication)?)
    }

    fn native_identity(&self, current: &Native) -> Result<()> {
        ensure!(
            current.actor == BOB_OBJECT
                && current.actor_lifetime == Some(self.actor_lifetime)
                && current.owner == Some(self.modal.caller)
                && current.owner_lifetime == Some(self.caller_lifetime)
                && current.owner_source.as_deref() == Some(SOURCE)
                && current.serial == self.initial.serial
                && current.serial >= self.accepted.id
                && !current.input_pending,
            "walking competing or changed owner"
        );
        ensure!(
            current.position.is_some(),
            "walking committed actor position absent"
        );
        Ok(())
    }

    fn native_owners(&self, current: &Native, queued: bool) -> Result<()> {
        self.native_identity(current)?;
        let target = current
            .targets
            .get(&self.accepted.target)
            .context("walking target no longer owned")?;
        ensure!(
            target.lifetime == Some(self.target_lifetime)
                && target.position == Some(self.destination),
            "walking target lifetime/committed placement changed"
        );
        let live: Vec<_> = current
            .active
            .iter()
            .chain(current.pending.iter())
            .collect();
        ensure!(
            live.iter().all(|action| action.id == self.accepted.id) && live.len() <= 1,
            "walking unexpected queued gameplay"
        );
        let matches = live
            .iter()
            .filter(|action| ***action == self.accepted)
            .count();
        if queued {
            ensure!(
                matches == 1
                    && current.active == self.initial.active
                    && current.pending == self.initial.pending
                    && current.previous == self.initial.previous,
                "walking accepted active/pending/previous ownership changed before response"
            );
        } else {
            let previous = current.previous.as_ref().is_some_and(|action| {
                let mut expected = self.accepted.clone();
                expected.flags = action.flags;
                action.flags & !32 == self.accepted.flags & !32 && *action == expected
            });
            ensure!(
                matches == 1 || (live.is_empty() && previous),
                "walking accepted serial/action no longer owned"
            );
        }
        Ok(())
    }

    fn progress(
        &self,
        current: &Native,
        record: Option<&Record>,
        attempted: bool,
        accepted: bool,
    ) -> Result<(bool, bool)> {
        self.native_identity(current)?;
        let next = attempted && next_instruction(&self.prior, &self.modal, record)?;
        if next {
            let record = record.unwrap();
            ensure!(
                record.caller == self.modal.caller
                    && record.source == SOURCE
                    && record.origin.source == SOURCE,
                "walking subsequent instruction changed same-run caller/source"
            );
        } else {
            self.native_owners(current, !attempted)?;
        }
        Ok((next, accepted && moved(&self.initial, current)))
    }

    fn ready(&self, publication: &Published, cut: Cut, streams: usize, idle: bool) -> Result<()> {
        self.cut.require(cut)?;
        ensure!(
            publication.modal && publication.dialog.as_ref() == Some(&self.modal),
            "walking stale/repeated modal identity"
        );
        exact_modal(&self.prior, publication.dialog.as_ref().unwrap())?;
        self.owners(publication, true)?;
        let current = native(publication)?;
        ensure!(
            current.tick == self.initial.tick
                && current.clock_pending == self.initial.clock_pending
                && current.position == self.initial.position,
            "walking pre-response notification changed native tick/clock/committed position"
        );
        response_gate(current.children_pending, streams, idle)?;
        Ok(())
    }
}

pub(super) fn notification_event(event: Event) -> Result<Event> {
    ensure!(
        matches!(event, Event::Completion),
        "walking unexpected asynchronous gameplay effect"
    );
    Ok(event)
}

fn response_gate(children_pending: bool, streams: usize, idle: bool) -> Result<()> {
    ensure!(
        streams == 0 && idle && !children_pending,
        "walking response requires zero registered streams, idle Host, and no presentation children"
    );
    Ok(())
}

fn finish_drained(
    streams: usize,
    host_idle: bool,
    finish: impl FnOnce() -> Result<()>,
) -> Result<bool> {
    if streams != 0 {
        return Ok(false);
    }
    ensure!(
        host_idle,
        "walking owner is not idle without a registered notification stream"
    );
    finish()?;
    Ok(true)
}

fn notification(
    initial: &Native,
    current: &Native,
    expected: &Record,
    record: Option<&Record>,
    modal: bool,
) -> Result<()> {
    ensure!(
        modal
            && record == Some(expected)
            && current.tick == initial.tick
            && current.clock_pending == initial.clock_pending
            && current.position == initial.position
            && current.actor == initial.actor
            && current.actor_lifetime == initial.actor_lifetime
            && current.owner == initial.owner
            && current.owner_lifetime == initial.owner_lifetime,
        "walking notification settle changed modal/tick/clock/position/ownership"
    );
    Ok(())
}

fn next_instruction(prior: &Record, modal: &Record, record: Option<&Record>) -> Result<bool> {
    if let Some(record) = record {
        if record != prior && record != modal {
            ensure!(
                record.id > modal.id,
                "walking stale or repeated subsequent delivery"
            );
            return Ok(true);
        }
    }
    Ok(false)
}

fn moved(initial: &Native, current: &Native) -> bool {
    current.actor == initial.actor
        && current.actor_lifetime.is_some()
        && current.actor_lifetime == initial.actor_lifetime
        && current.position.is_some()
        && initial.position.is_some()
        && current.position != initial.position
}

fn frame_tick_capacity(initial: u64, current: u64) -> Result<()> {
    ensure!(
        current
            .checked_sub(initial)
            .and_then(|delta| delta.checked_add(FRAME_TICK_RESERVATION))
            .is_some_and(|reserved| reserved <= TICK_LIMIT),
        "walking next frame tick reservation exceeds cap"
    );
    Ok(())
}

fn phase_capacity(usage: Usage, delta: Usage) -> Result<()> {
    usage.reserve(delta)?;
    ensure!(
        usage.pixel_scans == 0 && delta.pixel_scans == 0,
        "walking pixel scans forbidden"
    );
    Ok(())
}

#[derive(Default, serde::Serialize)]
struct Latch {
    attempted: bool,
    accepted: bool,
    resumes: u8,
    movement: bool,
}

impl Latch {
    fn response(&mut self, record: &Record) -> Result<Message> {
        ensure!(
            !self.attempted,
            "walking response already attempted; no retry"
        );
        let button = record
            .buttons
            .first()
            .context("walking offered response absent")?;
        self.attempted = true;
        Ok(Message::Dialog(record.id, button.result))
    }

    fn resume(&mut self) -> Result<()> {
        ensure!(
            self.resumes == 0,
            "walking playback resume already attempted"
        );
        self.resumes = 1;
        Ok(())
    }
}

struct Continuation<'a> {
    journey: &'a mut Journey,
    handoff: Handoff,
    start: Usage,
    wall: Instant,
    latch: Latch,
    next: Option<Record>,
}

impl Continuation<'_> {
    fn usage(&self) -> Result<Usage> {
        Usage::current(self.journey).since(self.start)
    }

    fn reserve(&self, delta: Usage) -> Result<()> {
        phase_capacity(self.usage()?, delta)?;
        preflight(self.journey, delta)?;
        ensure!(
            self.wall.elapsed().as_secs_f64() < WALL_SECONDS,
            "walking wall limit"
        );
        Ok(())
    }

    fn observe(&mut self) -> Result<()> {
        let publication = self.journey.host.publication_for_test().clone();
        let current = native(&publication)?.clone();
        self.journey.events.push(json!({"kind":"tutorial_walking_publication", "cut":Cut::current(self.journey),
            "tick":current.tick,"modal":publication.modal,"record":publication.dialog,
            "children_pending":current.children_pending,"clock_pending":current.clock_pending,
            "position":current.position,"active":current.active,"pending":current.pending,
            "previous":current.previous,"response_status":current.dialog_statuses.get(&self.handoff.modal.id),
            "registered_streams":self.journey.streams}));
        self.reserve(Usage::default())?;
        self.journey.healthy()?;
        self.journey.host.frontend_for_test()?;
        self.handoff.cut.require(Cut::current(self.journey))?;
        ensure!(
            current
                .tick
                .checked_sub(self.handoff.initial.tick)
                .is_some_and(|delta| delta <= TICK_LIMIT),
            "walking native tick delta limit/reset"
        );
        self.handoff.persistent_owners(&publication)?;
        if self.latch.attempted && current.dialog_statuses.get(&self.handoff.modal.id) == Some(&2) {
            self.latch.accepted = true;
        }
        let (next, movement) = self.handoff.progress(
            &current,
            publication.dialog.as_ref(),
            self.latch.attempted,
            self.latch.accepted,
        )?;
        self.latch.movement |= movement;
        if next {
            self.next = publication.dialog.clone();
            anyhow::bail!(BOUNDARY);
        }
        if !self.latch.attempted {
            notification(
                &self.handoff.initial,
                &current,
                &self.handoff.modal,
                publication.dialog.as_ref(),
                publication.modal,
            )?;
        } else if !self.latch.accepted {
            ensure!(
                publication.modal && publication.dialog.as_ref() == Some(&self.handoff.modal),
                "walking response outcome uncertain; no retry"
            );
        } else {
            ensure!(
                !publication.modal,
                "walking accepted response still owns modal"
            );
        }
        Ok(())
    }

    fn dispatch(&mut self, event: Event) -> Result<()> {
        self.observe()?;
        self.reserve(Usage {
            commands: 1,
            effect_polls: 1,
            tasks: 1,
            ..Default::default()
        })?;
        self.journey.idle = false;
        self.journey.commands += 1;
        self.journey.effect_polls += 1;
        let task = self.journey.host.update(event);
        keep_then_observe(
            true,
            self,
            task,
            |continuation, task| {
                let registered = register_returned(continuation.journey, task);
                let checked = continuation.usage()?.check();
                registered.and(checked)
            },
            Self::observe,
        )
    }

    fn drain(&mut self) -> Result<()> {
        loop {
            self.observe()?;
            if finish_drained(
                self.journey.streams,
                self.journey.host.frontend_idle_for_test(),
                || {
                    self.journey.idle = true;
                    self.observe()
                },
            )? {
                return Ok(());
            }
            self.reserve(Usage {
                effect_polls: 1,
                ..Default::default()
            })?;
            self.journey.effect_polls += 1;
            let remaining = (WALL_SECONDS - self.wall.elapsed().as_secs_f64())
                .min(self.journey.wall_seconds - self.journey.wall.elapsed().as_secs_f64());
            ensure!(remaining > 0.0, "walking receive wall limit");
            match self
                .journey
                .receiver
                .recv_timeout(Duration::from_secs_f64(remaining.min(0.1)))
            {
                Ok(Delivery::Finished) => {
                    self.journey.streams = self
                        .journey
                        .streams
                        .checked_sub(1)
                        .context("walking Finished without a registered stream")?;
                    self.observe()?;
                }
                Ok(Delivery::Action(iced_runtime::Action::Output(event))) => {
                    self.dispatch(notification_event(event)?)?;
                }
                Ok(delivery) => {
                    let _ = delivery;
                    anyhow::bail!(
                        "walking unsupported asynchronous delivery; no gameplay effect dispatched"
                    );
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn execute(&mut self) -> Result<()> {
        ensure!(
            self.journey.streams == self.handoff.retained_streams
                && self.journey.tasks == self.handoff.retained_tasks,
            "walking registered task handoff changed"
        );
        self.drain()?;
        let publication = self.journey.host.publication_for_test().clone();
        self.handoff.ready(
            &publication,
            Cut::current(self.journey),
            self.journey.streams,
            self.journey.idle && self.journey.host.frontend_idle_for_test(),
        )?;
        self.reserve(Usage {
            commands: 1,
            effect_polls: 1,
            tasks: 1,
            ..Default::default()
        })?;
        ensure!(
            self.journey.dialogs < 16,
            "walking inherited response ceiling"
        );
        let message = self.latch.response(publication.dialog.as_ref().unwrap())?;
        self.journey.dialogs += 1;
        let event = self.journey.host.ui_event_for_test(&publication, message);
        self.dispatch(event)?;
        self.drain()?;
        ensure!(
            self.latch.accepted,
            "walking native response status not accepted; no retry"
        );
        loop {
            self.observe()?;
            let publication = self.journey.host.publication_for_test().clone();
            let current = native(&publication)?;
            if !publication.playing && !current.clock_pending && !current.children_pending {
                ensure!(
                    self.latch.resumes == 0
                        && !publication.pending
                        && publication
                            .pause_button
                            .is_some_and(|button| button.enabled),
                    "walking enabled one-shot playback resume unavailable"
                );
                self.reserve(Usage {
                    commands: 1,
                    effect_polls: 1,
                    tasks: 1,
                    ..Default::default()
                })?;
                self.latch.resume()?;
                let event = self
                    .journey
                    .host
                    .ui_event_for_test(&publication, Message::Toggle);
                self.dispatch(event)?;
                self.drain()?;
                ensure!(
                    self.journey.host.publication_for_test().playing,
                    "walking resume not accepted"
                );
                continue;
            }
            frame_tick_capacity(self.handoff.initial.tick, current.tick)?;
            self.reserve(Usage {
                frames: 1,
                commands: 1,
                effect_polls: 1,
                tasks: 1,
                ..Default::default()
            })?;
            self.journey.frames += 1;
            self.journey.millis += 40;
            let event =
                Event::Frame(self.journey.epoch + Duration::from_millis(self.journey.millis));
            self.dispatch(event)?;
            self.drain()?;
        }
    }
}

pub(super) fn run(journey: &mut Journey, handoff: Handoff) -> Result<Value> {
    let start = Usage::current(journey);
    journey
        .events
        .push(json!({"kind":"tutorial_walking_start","handoff":handoff,
        "limits":{"advancing_and_service_frames":128,"effect_polls":512,"nonempty_task_streams":128,
            "commands":256,"pixel_scans":0,"responses":1,"resumes":1,"native_tick_delta":TICK_LIMIT,
            "wall_seconds":WALL_SECONDS,"bounded_by_remaining_global_time":true}}));
    let mut continuation = Continuation {
        journey,
        handoff,
        start,
        wall: Instant::now(),
        latch: Latch::default(),
        next: None,
    };
    let result = continuation.execute();
    if continuation.journey.streams != 0 || !continuation.journey.host.frontend_idle_for_test() {
        continuation.journey.idle = false;
    }
    let boundary = continuation.next.is_some()
        && continuation.latch.accepted
        && result
            .as_ref()
            .err()
            .is_some_and(|error| error.to_string() == BOUNDARY);
    let terminal = native(continuation.journey.host.publication_for_test()).ok();
    let receipt = json!({"kind":"tutorial_walking_stop", "status":if boundary {"first_subsequent_instruction_boundary"} else {"unsupported"},
        "reason":result.as_ref().err().map(|error| format!("{error:#}")),"usage":continuation.usage().ok(),
        "response_attempted":continuation.latch.attempted,"native_response_accepted":continuation.latch.accepted,
        "resumes":continuation.latch.resumes,"movement_qualified":continuation.latch.movement,
        "native_tick_delta":terminal.and_then(|current| current.tick.checked_sub(continuation.handoff.initial.tick)),
        "committed_position_evidence":{"actor":BOB_OBJECT,"actor_lifetime":continuation.handoff.actor_lifetime,
            "start":continuation.handoff.initial.position,"current":terminal.and_then(|current| current.position),
            "destination":continuation.handoff.destination},
        "arrival_qualified":false,"correlated_successful_completion_qualified":false,
        "committed_endpoint_qualified":false,"inferred_routing_configuration_verified":false,
        "next_instruction":continuation.next,"next_response_sent":false,
        "retained_task_ownership":{"registered_streams":continuation.journey.streams,
            "total_registered_tasks":continuation.journey.tasks,"idle":continuation.journey.idle,
            "cleanup":"registered streams retained in the same Journey; no cancellation, forced drain, or owner recovery"},
        "betty_qualified":false,"household_switching_qualified":false,"tutorial_completion_qualified":false,
        "generic_journey_continuation_permitted":false,"strict_original_parity_ticks":0});
    continuation.journey.events.push(receipt.clone());
    if boundary {
        Ok(receipt)
    } else {
        result?;
        anyhow::bail!("walking stopped without a subsequent instruction")
    }
}

#[cfg(test)]
#[cfg(feature = "private-fixtures")]
#[rustfmt::skip]
#[path = "walking/tests.rs"]
mod tests;
