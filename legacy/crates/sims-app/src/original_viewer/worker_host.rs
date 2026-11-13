//! Iced shell around the unchanged synchronous household owner. Only one
//! engine update runs at a time. Frames keep painting the last publication;
//! they do not enqueue extra simulation work while that update is executing.
use super::owned_worker::{Owner, SubmitError, Worker};
use super::ui_publication::{Published, view::Frontend};
use super::*;
use super::super::support::window_session;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone)]
pub(super) enum Event {
    /// Subscription events and explicit headless native-call scheduling.
    Widget {
        generation: u64,
        message: Message,
    },
    /// Commands emitted by an actual immutable UI publication.
    Ui {
        generation: u64,
        message: Message,
        popup: PopupSource,
    },
    Effect {
        generation: u64,
        message: Message,
    },
    Frame(Instant),
    WindowCapture(super::window_capture::Event),
    // Existing headless close drivers retain their direct test-only route.
    #[cfg(test)]
    Close(window::Id),
    Completion,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PopupSource {
    owner: Option<sims_runtime::original_session::live_menu::State>,
    epoch: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shell {
    diagnostics: bool,
    options_open: bool,
    revision: u64,
}
struct Work {
    message: Message,
    camera_shortcut: bool,
    // A clone of this shell lifetime's authority, retained through dispatch.
    // Revocation also reaches work already submitted but not yet applied.
    save_shortcut: Option<Arc<AtomicBool>>,
    shell: Shell,
    popup: Option<PopupSource>,
    #[cfg(test)]
    gate: Option<TestGate>,
}
#[cfg(test)]
struct TestGate {
    started: std::sync::mpsc::SyncSender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}
struct Reply {
    task: Task<Message>,
    load_completion: Option<control_wiring::LoadCompletion>,
    copy_completion: Option<control_wiring::CopyCompletion>,
    copy_owner: save_copy::Owner,
    committed_load: Option<std::path::PathBuf>,
}
impl Reply {
    fn ignored(owner: save_copy::Owner) -> Self { Self { task: Task::none(), load_completion: None, copy_completion: None, copy_owner: owner, committed_load: None } }
}
struct Engine {
    viewer: Box<Viewer>,
    popup: Option<sims_runtime::original_session::live_menu::State>,
    popup_epoch: u64,
    #[cfg(test)]
    audit_epoch: Option<(Instant, u32)>,
    #[cfg(test)]
    has_applied: bool,
    #[cfg(test)]
    object_progress: bool,
    #[cfg(test)]
    object_progress_fixture: Option<Box<ObjectProgressFixture>>,
}
#[cfg(test)]
struct ObjectProgressFixture {
    publication: Published,
    vm: sims_vm::Vm,
}

impl Engine {
    fn new(mut viewer: Viewer) -> Self {
        viewer.recent_loads_in_host = true;
        viewer.committed_load = None;
        let popup = viewer
            .session
            .as_ref()
            .and_then(|s| s.host.presentation.live_menu.clone());
        Self {
            viewer: Box::new(viewer),
            popup,
            popup_epoch: 0,
            #[cfg(test)]
            audit_epoch: None,
            #[cfg(test)]
            has_applied: false,
            #[cfg(test)]
            object_progress: false,
            #[cfg(test)]
            object_progress_fixture: None,
        }
    }
}
#[cfg(test)]
impl Engine {
    fn capture_object_progress(&self, publication: &mut Published) {
        if !self.object_progress {
            return;
        }
        if let (Some(Ok(native)), Some(Ok(object))) = (
            publication.journey_walking.as_ref(),
            publication.journey_object.as_mut(),
        ) {
            let (vm, route, visit) = match (
                self.object_progress_fixture.as_ref(),
                self.viewer.session.as_ref(),
            ) {
                (Some(fixture), _) => (&fixture.vm, None, None),
                (None, Some(session)) => {
                    let simulation = &session.simulation;
                    let route = simulation
                        .vm
                        .object_threads
                        .get(&native.actor)
                        .and_then(|thread| simulation.routes.get(thread));
                    (&simulation.vm, route, simulation.visit)
                }
                (None, None) => return,
            };
            object.progress = Some(Box::new(
                super::ui_publication::JourneyObjectProgress::capture(
                    vm,
                    native.actor,
                    route,
                    visit,
                ),
            ));
        }
    }
}

impl Owner for Engine {
    type Command = Work;
    type Reply = Reply;
    type Published = Published;
    fn begin_observed_work(&self) -> Option<window_session::loaded_input::WorkScope> {
        self.viewer.owner_work()
    }
    fn observation_monitor(&self) -> Option<window_session::loaded_input::Monitor> {
        self.viewer.loaded_input.as_ref().map(|registration| registration.monitor())
    }
    fn apply(&mut self, work: Work) -> std::result::Result<Self::Reply, String> {
        let _span = window_session::span("worker_apply", || {
            let command = match &work.message {
                Message::Frame(_) => "frame",
                Message::Toggle => "toggle",
                Message::NativeSpeed(_) => "native_speed",
                _ => "other",
            };
            serde_json::json!({"command": command})
        });
        #[cfg(test)]
        if let Some(gate) = work.gate {
            gate.started.send(()).map_err(|e| e.to_string())?;
            gate.resume.recv().map_err(|e| e.to_string())?;
        }
        // These two flags belong exclusively to the replacement shell. The
        // native handler sees their exact value before its next ordinary call.
        self.viewer.diagnostics = work.shell.diagnostics;
        self.viewer.options_open = work.shell.options_open;
        self.viewer.application_revision = work.shell.revision;
        if work.save_shortcut.as_ref().is_some_and(|authority| {
            !authority.load(Ordering::Acquire) || self.viewer.save_shortcut_blocked()
        }) {
            return Ok(Reply::ignored(self.viewer.copy_owner.clone()));
        }
        if work.camera_shortcut && (self.viewer.camera_shortcuts_blocked()
            || self.viewer.options_open || self.viewer.diagnostics
            || self.viewer.application_confirmation.is_some()
            || !self.viewer.camera_command_enabled(&work.message))
        {
            return Ok(Reply::ignored(self.viewer.copy_owner.clone()));
        }
        if matches!(
            work.message,
            Message::GroundChoice(_)
                | Message::GroundCancel
                | Message::ObjectChoice(_)
                | Message::ObjectCancel
        ) && let Some(source) = work.popup
        {
            let current = self
                .viewer
                .session
                .as_ref()
                .and_then(|s| s.host.presentation.live_menu.as_ref());
            // A leaf belongs to its original popup, not whichever popup now
            // occupies the screen or reuses a script ID. Compare the retained
            // native owner, never publication age or an object's position.
            // Historical unbound menus retain their existing direct handlers.
            if (source.owner.is_some() || current.is_some())
                && (source.owner.as_ref() != current || source.epoch != self.popup_epoch)
            {
                return Ok(Reply::ignored(self.viewer.copy_owner.clone()));
            }
        }
        let reply = match work.message {
            Message::CopyTo(path, ticket) => {
                // Host owns window focus. Its current dispatch check already
                // accepted this write; a later blur does not cancel consumption.
                self.viewer.copy_focused = true;
                self.viewer.update_dispatch(Message::CopyTo(path, ticket), true)
            }
            other => self.viewer.update(other),
        };
        #[cfg(test)]
        { self.has_applied = true; }
        #[cfg(test)]
        if self.viewer.loaded_input_test_fault == Some("worker_apply_panic") {
            panic!("authored loaded-input apply panic after real update");
        }
        #[cfg(test)]
        self.viewer.loaded_input_test_boundary("post_update_popup_counter").map_err(|error| format!("{error:#}"))?;
        let popup = self
            .viewer
            .session
            .as_ref()
            .and_then(|s| s.host.presentation.live_menu.clone());
        if popup != self.popup {
            self.popup_epoch = self
                .popup_epoch
                .checked_add(1)
                .ok_or("UI popup lifetime counter exhausted")?;
            self.popup = popup;
        }
        Ok(Reply { task: reply, load_completion: self.viewer.load_completion.take(),
            copy_completion: self.viewer.copy_completion.take(), copy_owner: self.viewer.copy_owner.clone(),
            committed_load: self.viewer.committed_load.take() })
    }
    fn publish(&self) -> std::result::Result<Published, String> {
        let _span = window_session::span("publication_capture", || serde_json::Value::Null);
        #[cfg(test)]
        if self.has_applied {
            self.viewer.loaded_input_test_boundary("worker_publication").map_err(|error| format!("{error:#}"))?;
            if self.viewer.loaded_input_test_fault == Some("worker_publication_panic") {
                panic!("authored loaded-input publication panic after real update");
            }
        }
        let mut publication = Published::capture(&self.viewer);
        #[cfg(test)]
        if let Some(fixture) = self.object_progress_fixture.as_ref() {
            publication = fixture.publication.clone();
        }
        publication.native_popup_epoch = self.popup_epoch;
        #[cfg(test)]
        self.capture_object_progress(&mut publication);
        #[cfg(test)]
        let publication = {
            let mut publication = publication;
            if let Some((epoch, millis)) = self.audit_epoch {
                publication.audit = Some(
                    capture_for_test(&self.viewer, epoch, millis).map_err(|e| format!("{e:#}"))?,
                );
            }
            publication
        };
        Ok(publication)
    }
    fn replaces(command: &Work) -> bool {
        replacement_message(&command.message)
    }
}
fn replacement_message(message: &Message) -> bool {
    matches!(
        message,
        Message::Load | Message::LoadFrom(..) | Message::Restart | Message::ConfiguredRestart(_)
    )
}
struct Queued {
    generation: u64,
    work: Work,
}
pub(super) struct Host {
    window_capture: super::window_capture::Capture,
    player_shortcuts: super::player_shortcuts::State,
    save_shortcut_authority: Arc<AtomicBool>,
    camera_shortcuts: super::camera_shortcuts::State,
    load_picker: Option<load_picker::Picker>,
    last_load_path: Option<std::path::PathBuf>,
    recent_loads: recent_loads::History,
    picker_feedback: Option<String>,
    copy_notice: Option<String>,
    in_flight_load: Option<(super::owned_worker::Ticket, load_picker::Ticket)>,
    in_flight_copy: Option<(super::owned_worker::Ticket, load_picker::Ticket)>,
    copy_seed: std::path::PathBuf,
    copy_owner: save_copy::Owner,
    last_copy_path: Option<std::path::PathBuf>,
    copy_focused: bool,
    worker: Option<Worker<Engine>>,
    frontend: Frontend,
    queued: VecDeque<Queued>,
    replacing: bool,
    error: Option<String>,
    closing: Option<window::Id>,
    close_sent: bool,
    shell_revision: u64,
    application_confirmation: Option<neighborhood_usability::Confirmation>,
    #[cfg(test)]
    next_gate: Option<TestGate>,
    #[cfg(test)]
    next_notification: Option<Box<dyn FnOnce() + Send>>,
}
impl Host {
    pub fn new(args: Args) -> (Self, Task<Event>) {
        let (owner, task) = Viewer::new_recorded(args);
        Self::from_initial_load(owner, task)
    }
    #[cfg(test)]
    pub(super) fn new_with_loader_probe(
        args: Args,
        probe: player_timing::LoaderProbe,
    ) -> (Self, Task<Event>) {
        let (owner, task) = Viewer::new_with_loader_probe(args, probe);
        Self::from_initial_load(owner, task)
    }
    #[cfg(test)]
    pub(super) fn new_with_object_progress_for_test(
        args: Args,
        probe: Option<player_timing::LoaderProbe>,
    ) -> (Self, Task<Event>) {
        let (owner, task) = match probe {
            Some(probe) => Viewer::new_with_loader_probe(args, probe),
            None => Viewer::new(args),
        };
        let fallback = Arc::new(Published::capture(&owner));
        let engine = Self::object_progress_engine(owner);
        Self::start_initial_worker(fallback, engine, task)
    }
    #[cfg(test)]
    fn object_progress_engine(owner: Viewer) -> Engine {
        let mut engine = Engine::new(owner);
        engine.object_progress = true;
        engine
    }

    #[cfg(test)]
    pub(super) fn from_object_progress_fixture_for_test(
        viewer: Viewer,
        publication: Published,
        vm: sims_vm::Vm,
        enabled: bool,
    ) -> Self {
        let fallback = Arc::new(publication.clone());
        let mut engine = if enabled {
            Self::object_progress_engine(viewer)
        } else {
            Engine::new(viewer)
        };
        engine.popup = publication.native_popup.clone();
        engine.popup_epoch = publication.native_popup_epoch;
        engine.object_progress_fixture = Some(Box::new(ObjectProgressFixture { publication, vm }));
        let (host, task) = Self::start_initial_worker(fallback, engine, Task::none());
        assert_eq!(task.units(), 0);
        drop(task);
        host
    }

    fn from_initial_load(owner: Viewer, task: Task<Message>) -> (Self, Task<Event>) {
        let fallback = Arc::new(Published::capture(&owner));
        Self::start_initial_worker(fallback, Engine::new(owner), task)
    }
    fn start_initial_worker(
        fallback: Arc<Published>,
        engine: Engine,
        task: Task<Message>,
    ) -> (Self, Task<Event>) {
        let recent_loads = engine.viewer.recent_loads.snapshot();
        let copy_seed = engine.viewer.args.save.clone();
        let copy_owner = engine.viewer.copy_owner.clone();
        match Worker::start(engine) {
            Ok(worker) => {
                let frontend = Frontend {
                    data: worker.published().clone(),
                    diagnostics: false,
                    options_open: false,
                    widget_epoch: 0,
                };
                (
                    Self {
                        window_capture: super::window_capture::Capture::new(window_session::enabled()),
                        player_shortcuts: Default::default(),
                        save_shortcut_authority: Arc::new(AtomicBool::new(true)),
                        camera_shortcuts: Default::default(),
                        load_picker: None,
                        last_load_path: None,
                        recent_loads,
                        picker_feedback: None,
                        copy_notice: None,
                        in_flight_load: None,
                        in_flight_copy: None,
                        copy_seed,
                        copy_owner,
                        last_copy_path: None,
                        copy_focused: true,
                        worker: Some(worker),
                        frontend,
                        queued: VecDeque::new(),
                        replacing: false,
                        error: None,
                        closing: None,
                        close_sent: false,
                        shell_revision: 0,
                        application_confirmation: None,
                        #[cfg(test)]
                        next_gate: None,
                        #[cfg(test)]
                        next_notification: None,
                    },
                    task.map(|message| Event::Effect {
                        generation: 0,
                        message,
                    }),
                )
            }
            Err(error) => {
                // No hidden inline fallback, owner clone or automatic restart.
                drop(error.owner);
                (
                    Self {
                        window_capture: super::window_capture::Capture::new(window_session::enabled()),
                        player_shortcuts: Default::default(),
                        save_shortcut_authority: Arc::new(AtomicBool::new(true)),
                        camera_shortcuts: Default::default(),
                        load_picker: None,
                        last_load_path: None,
                        recent_loads,
                        picker_feedback: None,
                        copy_notice: None,
                        in_flight_load: None,
                        in_flight_copy: None,
                        copy_seed,
                        copy_owner,
                        last_copy_path: None,
                        copy_focused: true,
                        worker: None,
                        frontend: Frontend {
                            data: fallback,
                            diagnostics: false,
                            options_open: false,
                            widget_epoch: 0,
                        },
                        queued: VecDeque::new(),
                        replacing: false,
                        error: Some(format!("Cannot start household worker: {}", error.error)),
                        closing: None,
                        close_sent: false,
                        shell_revision: 0,
                        application_confirmation: None,
                        #[cfg(test)]
                        next_gate: None,
                        #[cfg(test)]
                        next_notification: None,
                    },
                    Task::none(),
                )
            }
        }
    }
    fn generation(&self) -> u64 {
        self.worker.as_ref().map_or(0, Worker::generation)
    }
    fn shell(&self) -> Shell {
        Shell {
            diagnostics: self.frontend.diagnostics,
            options_open: self.frontend.options_open,
            revision: self.shell_revision,
        }
    }
    fn transport_waiting(&self) -> bool {
        self.closing.is_none()
            && !self.close_sent
            && !self.replacing
            && self.application_confirmation.is_none()
            && self.frontend.error.is_none()
            && self.frontend.native_transport
            && self.frontend.pending
            && !self.frontend.modal
            && self.queued.front().is_some_and(|next| {
                next.generation == self.generation()
                    && matches!(next.work.message, Message::Toggle | Message::NativeSpeed(_))
            })
    }
    fn poll(&mut self) -> Task<Event> {
        let Some(worker) = &mut self.worker else {
            return Task::none();
        };
        let completion = worker.poll();
        // The broker may advance generation during poll. Invalidate capture
        // immediately, before effects, incoming replies or publication acks.
        let capture = self.window_capture.synchronize(worker.generation(), Instant::now())
            .map(Event::WindowCapture);
        let task = match completion {
            Ok(Some(completion)) => {
                let changed_generation = completion.generation != completion.ticket.generation;
                self.frontend.data = worker.published().clone();
                // Loading a new owner also ends presses in the old widget
                // tree. This shell-only fence never changes the native/save
                // widget epoch or any gameplay owner.
                self.frontend.widget_epoch = self
                    .frontend
                    .data
                    .widget_epoch
                    .wrapping_add(completion.generation);
                if changed_generation {
                    self.application_confirmation = None;
                    self.last_copy_path = None;
                    self.copy_notice = None;
                    if self.copy_picker() && !self.load_picker.as_ref().unwrap().pending() { self.load_picker = None; }
                    // Input created by the previous world cannot be replayed on
                    // reused native IDs after Load/Restart.
                    let rejected: Vec<_> = self.queued.iter()
                        .filter(|q| q.generation != completion.generation)
                        .filter_map(|q| Self::load_ticket(&q.work.message)).collect();
                    let rejected_copies: Vec<_> = self.queued.iter()
                        .filter(|q| q.generation != completion.generation)
                        .filter_map(|q| Self::copy_ticket(&q.work.message)).collect();
                    self.queued.retain(|q| q.generation == completion.generation);
                    for ticket in rejected_copies {
                        self.settle_copy(control_wiring::CopyCompletion { ticket,
                            feedback: Some("Copy was not dispatched because its household generation changed. Retry Save a Copy….".into()) });
                    }
                    for ticket in rejected {
                        self.settle_picker(control_wiring::LoadCompletion { ticket,
                            feedback: Some("Load was rejected before consumption because its household generation changed. Retry Load.".into()) });
                    }
                }
                self.replacing = self.queued.iter().any(|q| Engine::replaces(&q.work))
                    || (!self.frontend.data.session_present
                        && self.frontend.data.error.is_none()
                        && (changed_generation || self.replacing));
                if completion.quarantined {
                    self.application_confirmation = None;
                    self.error = Some("Household owner failed; reopen the app to recover.".into());
                }
                match completion.reply {
                    Ok(reply) => {
                        // Worker::poll accepted the exact request. A convenience
                        // entry also requires its healthy coherent publication
                        // and the one-shot full-tail Load receipt.
                        let healthy_load = !completion.quarantined && completion.publication.is_some()
                            && completion.ticket.generation.checked_add(1) == Some(completion.generation)
                            && self.error.is_none() && self.frontend.error.is_none();
                        if self.in_flight_load.is_none() && reply.load_completion.is_none() && healthy_load {
                            if let Some(path) = reply.committed_load.as_deref() { self.recent_loads.record(path); }
                        }
                        self.copy_owner = reply.copy_owner;
                        if let Some((worker_ticket, picker_ticket)) = self.in_flight_copy.take() {
                            if worker_ticket == completion.ticket {
                                let healthy = !changed_generation && !completion.quarantined
                                    && self.error.is_none() && self.frontend.error.is_none();
                                match reply.copy_completion {
                                    Some(copy) if healthy && self.load_picker.as_ref().is_some_and(|p|
                                        p.pending_ticket(&picker_ticket) && p.pending_ticket(&copy.ticket)) => self.settle_copy(copy),
                                    _ => self.settle_copy(control_wiring::CopyCompletion { ticket: picker_ticket,
                                        feedback: Some("Copy completion was not confirmed by the healthy owner publication; check the destination before retrying Save a Copy….".into()) }),
                                }
                            } else { self.in_flight_copy = Some((worker_ticket, picker_ticket)); }
                        }
                        if let Some((worker_ticket, picker_ticket)) = self.in_flight_load.take() {
                            if worker_ticket == completion.ticket {
                                match reply.load_completion {
                                    Some(load) if self.load_picker.as_ref().is_some_and(|picker|
                                        picker.pending_ticket(&picker_ticket) && picker.pending_ticket(&load.ticket)) => {
                                        if healthy_load && load.feedback.is_none() {
                                            if let Some(path) = reply.committed_load.as_deref().filter(|path| load.ticket.matches_path(path)) {
                                                self.recent_loads.record(path);
                                            }
                                        }
                                        self.settle_picker(load);
                                    }
                                    _ => self.settle_picker(control_wiring::LoadCompletion {
                                        ticket: picker_ticket,
                                        feedback: Some("Load completion did not authorize this picker attempt; the request has ended without picker success.".into()),
                                    }),
                                }
                            } else {
                                // Never use another worker request's publication as
                                // evidence that this selected path was consumed.
                                self.in_flight_load = Some((worker_ticket, picker_ticket));
                            }
                        }
                        let generation = completion.generation;
                        reply.task.map(move |message| Event::Effect {
                            generation,
                            message,
                        })
                    }
                    Err(error) => {
                        self.reject_flight(&error);
                        self.error = Some(error);
                        self.reject_queued_loads("Load could not be dispatched because the household owner failed.");
                        Task::none()
                    }
                }
            }
            Ok(None) => Task::none(),
            Err(error) => {
                let feedback = format!("Household worker disconnected: {error}");
                self.reject_flight(&feedback);
                self.error = Some(feedback);
                self.reject_queued_loads("Load was rejected before consumption because the worker disconnected.");
                Task::none()
            }
        };
        Task::batch([capture, task])
    }
    fn recent_choices_enabled(&self) -> bool {
        self.picker_enabled() && self.copy_focused && !self.frontend.player_shortcuts_blocked
            && !self.frontend.pending && self.queued.is_empty()
            && self.in_flight_load.is_none() && self.in_flight_copy.is_none()
            && self.worker.as_ref().is_some_and(|worker| !worker.busy())
    }
    fn copy_picker(&self) -> bool {
        self.load_picker.as_ref().is_some_and(|p| p.purpose() == load_picker::Purpose::Copy)
    }
    fn copy_enabled(&self) -> bool {
        self.picker_enabled() && self.copy_focused && !self.frontend.player_shortcuts_blocked
    }
    fn picker_enabled(&self) -> bool {
        !self.frontend.load_picker_blocked && self.frontend.session_present
            && self.frontend.error.is_none() && self.error.is_none()
            && self.frontend.options_open && !self.frontend.diagnostics
            && !self.frontend.modal && self.application_confirmation.is_none()
            && !self.replacing && self.closing.is_none() && !self.close_sent
    }
    // Retain failed-attempt text without intercepting existing fatal recovery.
    fn picker_owns_input(&self) -> bool {
        self.load_picker.as_ref().is_some_and(|picker| picker.pending()
            || (self.frontend.options_open && !self.frontend.diagnostics))
            && self.error.is_none() && self.frontend.error.is_none()
    }
    fn camera_blocked(&self) -> bool {
        self.frontend.camera_shortcuts_blocked || self.frontend.modal
            || !self.frontend.session_present || self.frontend.error.is_some()
            || self.error.is_some() || self.frontend.options_open || self.frontend.diagnostics
            || self.picker_owns_input() || self.application_confirmation.is_some()
            || self.replacing || self.closing.is_some() || self.close_sent
    }
    fn invalidate_save_shortcuts(&mut self) {
        self.save_shortcut_authority.store(false, Ordering::Release);
        // Old work can never regain authority after focus/overlay/owner changes.
        // There is no wrapping counter and no reset of the physical-key latch.
        self.save_shortcut_authority = Arc::new(AtomicBool::new(true));
    }
    fn save_shortcut_blocked(&self) -> bool {
        self.camera_blocked() || self.frontend.load_picker_blocked
    }
    fn load_ticket(message: &Message) -> Option<load_picker::Ticket> {
        match message { Message::LoadFrom(_, ticket) => Some(ticket.clone()), _ => None }
    }
    fn copy_ticket(message: &Message) -> Option<load_picker::Ticket> {
        match message { Message::CopyTo(_, ticket) => Some(ticket.clone()), _ => None }
    }
    fn settle_copy(&mut self, completion: control_wiring::CopyCompletion) {
        if completion.ticket.purpose() != load_picker::Purpose::Copy { return; }
        if self.load_picker.as_mut().is_some_and(|p| p.settle(&completion.ticket, completion.feedback.clone()))
            && completion.feedback.is_none() { self.load_picker = None; }
    }
    fn settle_picker(&mut self, completion: control_wiring::LoadCompletion) {
        if completion.ticket.purpose() != load_picker::Purpose::Load { return; }
        if self.load_picker.as_mut().is_some_and(|picker| picker.settle(&completion.ticket, completion.feedback.clone()))
            && completion.feedback.is_none()
        {
            self.load_picker = None;
            self.frontend.options_open = false;
            self.application_confirmation = None;
            self.shell_revision = self.shell_revision.wrapping_add(1);
        }
    }
    fn reject_flight(&mut self, reason: &str) {
        if let Some((_, ticket)) = self.in_flight_copy.take() {
            self.settle_copy(control_wiring::CopyCompletion { ticket, feedback: Some(format!("{reason} Copy completion was not confirmed; check the destination before retrying Save a Copy….")) });
        }
        if let Some((_, ticket)) = self.in_flight_load.take() {
            // Used only after the broker proves completion/failure/disconnect,
            // never for a user Cancel while consumption may still be running.
            self.settle_picker(control_wiring::LoadCompletion { ticket, feedback: Some(reason.into()) });
        }
    }
    fn reject_queued_loads(&mut self, reason: &str) {
        let copies: Vec<_> = self.queued.iter().filter_map(|q| Self::copy_ticket(&q.work.message)).collect();
        self.queued.retain(|q| !matches!(q.work.message, Message::CopyTo(..)));
        for ticket in copies { self.settle_copy(control_wiring::CopyCompletion { ticket,
            feedback: Some(format!("{reason} Copy was not dispatched; retry Save a Copy….")) }); }
        let tickets: Vec<_> = self.queued.iter().filter_map(|q| Self::load_ticket(&q.work.message)).collect();
        self.queued.retain(|q| !matches!(q.work.message, Message::LoadFrom(..)));
        for ticket in tickets {
            self.settle_picker(control_wiring::LoadCompletion { ticket, feedback: Some(reason.into()) });
        }
    }
    fn enqueue(&mut self, generation: u64, message: Message) {
        self.enqueue_origin(generation, message, None, false, false);
    }
    fn enqueue_origin(&mut self, generation: u64, message: Message, popup: Option<PopupSource>, camera_shortcut: bool, save_shortcut: bool) {
        let shell = self.shell();
        let drain_transport = matches!(message, Message::Frame(_)) && self.transport_waiting();
        // This is the existing consecutive-Motion coalescing rule, applied
        // before dispatch only when no enter/leave/button/shell boundary lies
        // between the motions. No completed engine call is coalesced.
        if matches!(message, Message::ViewportMotion { .. })
            && let Some(last) = self.queued.back_mut()
            && last.generation == generation
            && last.work.shell == shell
            && last.work.popup == popup
            && matches!(last.work.message, Message::ViewportMotion { .. })
        {
            last.work.message = message;
            return;
        }
        if self.queued.len() >= 256 && !drain_transport {
            if let Some(ticket) = Self::copy_ticket(&message) {
                self.settle_copy(control_wiring::CopyCompletion { ticket, feedback: Some("Copy was not dispatched because the input queue is full. Retry Save a Copy….".into()) });
            }
            if let Some(ticket) = Self::load_ticket(&message) {
                self.settle_picker(control_wiring::LoadCompletion { ticket,
                    feedback: Some("Load was rejected before consumption because the input queue is full.".into()) });
                self.replacing = false;
            }
            self.error =
                Some("Household input queue is full; no input was executed out of order.".into());
            return;
        }
        let queued = Queued {
            generation,
            work: Work {
                message,
                camera_shortcut,
                save_shortcut: save_shortcut.then(|| self.save_shortcut_authority.clone()),
                shell,
                popup,
                #[cfg(test)]
                gate: self.next_gate.take(),
            },
        };
        if drain_transport {
            self.queued.push_front(queued);
        } else {
            self.queued.push_back(queued);
        }
    }
    fn pump(&mut self) -> Task<Event> {
        if self.error.is_some() {
            self.reject_queued_loads("Load was rejected before consumption because the application is unavailable.");
            return Task::none();
        }
        if self.transport_waiting() { return Task::none(); }
        if self.worker.as_ref().is_none_or(Worker::busy) { return Task::none(); }
        while let Some(mut next) = self.queued.pop_front() {
            let stale = next.generation != self.generation();
            let refused_load = matches!(next.work.message, Message::LoadFrom(..))
                && (self.frontend.load_picker_blocked || self.frontend.error.is_some()
                    || !self.frontend.options_open || self.frontend.diagnostics
                    || self.application_confirmation.is_some());
            let refused_copy = if let Message::CopyTo(path, ticket) = &next.work.message {
                !self.copy_enabled() || !self.load_picker.as_ref().is_some_and(|p|
                    p.purpose() == load_picker::Purpose::Copy && p.pending_ticket(ticket)
                        && ticket.matches_path(path) && path.as_os_str() == std::ffi::OsStr::new(p.value()))
            } else { false };
            if stale || refused_copy || refused_load {
                if let Some(ticket) = Self::copy_ticket(&next.work.message) {
                    self.settle_copy(control_wiring::CopyCompletion { ticket, feedback: Some("Copy was rejected before writing because its owner, generation or Options guards changed. Retry Save a Copy….".into()) });
                }
                if let Some(ticket) = Self::load_ticket(&next.work.message) {
                    self.settle_picker(control_wiring::LoadCompletion { ticket,
                        feedback: Some("Load was rejected before consumption because its current household or Options guards changed. Retry Load.".into()) });
                    self.replacing = self.queued.iter().any(|q| Engine::replaces(&q.work));
                }
                continue;
            }
            if (next.work.camera_shortcut && self.camera_blocked())
                || next.work.save_shortcut.as_ref().is_some_and(|authority|
                    !authority.load(Ordering::Acquire) || self.save_shortcut_blocked())
                || (self.picker_owns_input() && !control_wiring::picker_allows(&next.work.message, self.frontend.modal))
            { continue; }
            if next.work.save_shortcut.is_some() {
                // Recheck the shell now, rather than restoring enqueue-time
                // Options/Diagnostics when the engine finally sees this Save.
                next.work.shell = self.shell();
            }
            let load_ticket = Self::load_ticket(&next.work.message);
            let copy_ticket = Self::copy_ticket(&next.work.message);
            let (sender, receiver) = iced::futures::channel::oneshot::channel();
            #[cfg(test)]
            let notification = self.next_notification.take();
            match self.worker.as_mut().unwrap().submit_notifying(
                next.work,
                Some(Box::new(move || {
                    let _ = sender.send(());
                    #[cfg(test)]
                    if let Some(notification) = notification { notification(); }
                })),
            ) {
                Ok(ticket) => {
                    if let Some(load_ticket) = load_ticket { self.in_flight_load = Some((ticket, load_ticket)); }
                    if let Some(copy_ticket) = copy_ticket { self.in_flight_copy = Some((ticket, copy_ticket)); }
                    return Task::perform(async move { let _ = receiver.await; }, |()| Event::Completion);
                }
                Err(SubmitError::Busy(work)) => self.queued.push_front(Queued { generation: next.generation, work }),
                Err(error) => {
                    let (work, reason) = match error {
                        SubmitError::Disconnected(work) => (work, "Household worker disconnected."),
                        SubmitError::Quarantined(work) => (work, "Household owner requires application restart."),
                        SubmitError::Exhausted(work) => (work, "Household worker ticket space exhausted."),
                        SubmitError::Busy(_) => unreachable!(),
                    };
                    if let Some(ticket) = Self::copy_ticket(&work.message) {
                        self.settle_copy(control_wiring::CopyCompletion { ticket, feedback: Some(format!("{reason} Retry Save a Copy….")) });
                    }
                    if let Some(ticket) = Self::load_ticket(&work.message) {
                        self.settle_picker(control_wiring::LoadCompletion { ticket, feedback: Some(reason.into()) });
                    }
                    self.error = Some(reason.into());
                    self.reject_queued_loads(reason);
                }
            }
            break;
        }
        Task::none()
    }
    pub fn update(&mut self, event: Event) -> Task<Event> {
        let task = self.poll();
        let now = Instant::now();
        if let Event::WindowCapture(event) = event {
            let outcome = self.window_capture.handle(event, now);
            // Fresh monotonic time at the ack poll, after reply/enqueue handling.
            self.window_capture.tick(Instant::now());
            if let Some(id) = outcome.close_requested {
                self.invalidate_save_shortcuts();
                if !self.close_sent { self.closing = Some(id); }
            }
            let next = self.pump();
            return Task::batch([task, outcome.task.map(Event::WindowCapture), next, self.close_when_idle()]);
        }
        self.window_capture.tick(Instant::now());
        let (generation, message, effect, popup) = match event {
            Event::WindowCapture(_) => unreachable!("capture handled above"),
            #[cfg(test)]
            Event::Close(id) => {
                self.invalidate_save_shortcuts();
                // Direct close is only a legacy headless fixture route.
                let event = self.window_capture.close_requested_for_test(id);
                drop(self.window_capture.handle(event, now));
                // Unlike a synchronous handler, the responsive shell can see
                // Close while Save is still executing. Drain accepted calls
                // before closing; never cut an accepted atomic save short.
                if !self.close_sent {
                    self.closing = Some(id);
                }
                let next = self.pump();
                return Task::batch([task, next, self.close_when_idle()]);
            }
            Event::Completion => {
                // A completion is a wake, not authority to accept a result or
                // advance game time. poll() above checks the exact ticket.
                let next = self.pump();
                return Task::batch([task, next, self.close_when_idle()]);
            }
            Event::Frame(now) => {
                // The shell's additional paints/polls cannot produce additional
                // VM visits while the previous original caller is executing.
                let next = self.pump();
                if self.error.is_none()
                    && self.closing.is_none()
                    && !self.close_sent
                    && !self.replacing
                    && self.application_confirmation.is_none()
                    && (self.queued.is_empty() || self.transport_waiting())
                    && self.worker.as_ref().is_some_and(|w| !w.busy())
                {
                    self.enqueue(self.generation(), Message::Frame(now));
                    let frame = self.pump();
                    return Task::batch([task, next, frame, self.close_when_idle()]);
                }
                return Task::batch([task, next, self.close_when_idle()]);
            }
            Event::Widget {
                generation,
                message,
            } => (generation, message, false, None),
            Event::Ui {
                generation,
                message,
                popup,
            } => (generation, message, false, Some(popup)),
            Event::Effect {
                generation,
                message,
            } => (generation, message, true, None),
        };
        // Window focus loss remains a release fence across owner generations.
        let focus_lost = matches!(&message, Message::FocusLost
            | Message::PlayerShortcut(player_shortcuts::Input::FocusLost));
        // Stale shell/owner messages are rejected below and cannot revoke a
        // queued or already-submitted Save belonging to the current owner.
        let current_boundary = generation == self.generation()
            && (matches!(&message, Message::Options | Message::Diagnostics | Message::ChooseLoad
                | Message::ChooseSaveCopy | Message::LoadPicker(_)
                | Message::RequestApplicationChoice(_) | Message::CancelApplicationChoice(_)
                | Message::ConfirmApplicationChoice(_)) || replacement_message(&message));
        if focus_lost || current_boundary {
            self.invalidate_save_shortcuts();
        }
        if !effect && generation == self.generation() {
            match &message {
                Message::FocusLost | Message::PlayerShortcut(player_shortcuts::Input::FocusLost) => {
                    self.copy_focused = false;
                    if let Some(picker) = self.load_picker.as_mut() { picker.focus_lost(); }
                }
                Message::PlayerShortcut(player_shortcuts::Input::FocusGained) => self.copy_focused = true,
                _ => {}
            }
        }
        let (message, camera_shortcut, save_shortcut) = if let Message::PlayerShortcut(input) = message {
            use super::player_shortcuts::{Context, Decision, Input};
            control_wiring::relay_camera(&mut self.camera_shortcuts, input, generation, self.worker.as_ref().map_or(0, Worker::generation));
            let file_picker = matches!(input, Input::OpenPressed { .. } | Input::SaveCopyPressed { .. });
            let context = Context {
                generation: self.generation(),
                modal: self.frontend.modal,
                blocked: self.frontend.player_shortcuts_blocked
                    || !self.frontend.session_present
                    || self.frontend.error.is_some()
                    || self.error.is_some()
                    || (self.frontend.options_open && !file_picker)
                    || self.frontend.diagnostics
                    || self.application_confirmation.is_some()
                    || self.picker_owns_input()
                    || self.replacing || self.closing.is_some() || self.close_sent,
            };
            let decision = self.player_shortcuts.route(input, generation, context);
            let message = if matches!(input, Input::FocusLost) {
                Message::FocusLost
            } else {
                match decision {
                    Decision::Toggle => Message::Toggle,
                    Decision::DialogSpace => Message::DialogKey(32),
                    Decision::Save => {
                        if effect || self.save_shortcut_blocked() { return task; }
                        Message::Save
                    }
                    Decision::Load | Decision::SaveCopy => {
                        // An idle, current owner publication is required before
                        // shell-only opening. Never enqueue picker intentions
                        // behind effects, Save/Load or other accepted work.
                        if effect || generation != self.generation() || !self.copy_focused
                            || self.frontend.load_picker_blocked || self.frontend.pending
                            || self.load_picker.is_some() || !self.queued.is_empty()
                            || self.in_flight_load.is_some() || self.in_flight_copy.is_some()
                            || self.worker.as_ref().is_none_or(Worker::busy)
                        { return task; }
                        self.invalidate_save_shortcuts();
                        if !self.frontend.options_open {
                            // Match the existing Options shell transition;
                            // retain this update's single head poll task.
                            self.frontend.options_open = true;
                            self.shell_revision = self.shell_revision.wrapping_add(1);
                        }
                        if decision == Decision::Load { Message::ChooseLoad }
                        else { Message::ChooseSaveCopy }
                    }
                    Decision::Ignore => return task,
                }
            };
            (message, false, decision == Decision::Save)
        } else if let Message::CameraShortcut(input) = message {
            let current = camera_shortcuts::Context { generation: self.generation(), blocked: self.camera_blocked() };
            let Some(action) = self.camera_shortcuts.route(input, generation, current) else { return task; };
            (control_wiring::camera_command(action), true, false)
        } else { (message, false, false) };
        if generation != self.generation() {
            return task;
        }
        if (self.closing.is_some() || self.close_sent) && !effect {
            return task;
        }
        // LoadFrom is private queue work, never accepted from a replayed UI/effect.
        if matches!(message, Message::LoadFrom(..) | Message::CopyTo(..)) { return task; }
        let message = if !effect && !self.frontend.modal && self.picker_owns_input() && matches!(message, Message::DialogKey(27) | Message::PickerEscape) {
            self.load_picker.as_ref().map_or(message.clone(), |picker| Message::LoadPicker(picker.cancel_event()))
        } else if matches!(message, Message::PickerEscape) { return task; }
        else { message };
        let message = match message {
            Message::ChooseSaveCopy => {
                if !effect && self.load_picker.is_none() && self.copy_enabled() {
                    let seed = self.last_copy_path.clone().unwrap_or_else(|| save_copy::seed(&self.copy_seed));
                    match load_picker::Picker::new_copy(&seed, self.copy_owner.clone()) {
                        Ok(picker) => {
                            let focus = picker.opening_task();
                            self.load_picker = Some(picker); self.picker_feedback = None; self.copy_notice = None;
                            return Task::batch([task, focus]);
                        }
                        Err(error) => self.picker_feedback = Some(format!("{error:#}")),
                    }
                }
                return task;
            }
            Message::ChooseLoad => {
                if !effect && self.load_picker.is_none() && self.picker_enabled() {
                    let seed = self.last_load_path.as_deref().unwrap_or(&self.frontend.load_seed);
                    match load_picker::Picker::new(seed).map(|p| p.with_recent(&self.recent_loads)) {
                        Ok(picker) => {
                            let focus = picker.opening_task();
                            self.load_picker = Some(picker); self.picker_feedback = None; self.copy_notice = None;
                            return Task::batch([task, focus]);
                        }
                        Err(error) => self.picker_feedback = Some(format!("{error:#}")),
                    }
                }
                return task;
            }
            Message::LoadPicker(event) => {
                if effect || !self.picker_enabled() || (self.copy_picker() && !self.copy_enabled()) { return task; }
                if event.is_recent_choice() && !self.recent_choices_enabled() { return task; }
                match self.load_picker.as_mut().map(|picker| picker.update(event)) {
                    Some(load_picker::Outcome::FocusPath) => {
                        let focus = self.load_picker.as_ref().unwrap().opening_task();
                        return Task::batch([task, focus]);
                    }
                    Some(load_picker::Outcome::Submit { path, ticket }) => {
                        if ticket.purpose() == load_picker::Purpose::Copy {
                            self.last_copy_path = Some(path.clone());
                            Message::CopyTo(path, ticket)
                        } else {
                            self.last_load_path = Some(path.clone());
                            Message::LoadFrom(path, ticket)
                        }
                    }
                    Some(load_picker::Outcome::Cancelled) => {
                        if self.copy_picker() { self.copy_notice = Some(control_wiring::COPY_CANCELLED.into()); }
                        self.load_picker = None;
                        return task;
                    }
                    _ => return task,
                }
            }
            other => other,
        };
        if self.picker_owns_input() && !control_wiring::picker_allows(&message, self.frontend.modal) {
            return task;
        }
        let message = if effect {
            message
        } else {
            super::application_escape(
                message,
                self.frontend.options_open,
                self.frontend.modal,
                self.application_confirmation,
            )
        };
        match message {
            Message::Options => {
                self.application_confirmation = None;
                self.frontend.options_open = !self.frontend.options_open;
                self.shell_revision = self.shell_revision.wrapping_add(1);
                return task;
            }
            Message::Diagnostics => {
                self.application_confirmation = None;
                self.frontend.options_open = false;
                self.frontend.diagnostics = !self.frontend.diagnostics;
                self.shell_revision = self.shell_revision.wrapping_add(1);
                return task;
            }
            _ => {}
        }
        if self.error.is_some() {
            return task;
        }
        if self.replacing && !effect {
            return task;
        }
        let message = match message {
            Message::RequestApplicationChoice(choice) => {
                if !effect && self.frontend.options_open && self.frontend.session_present {
                    self.shell_revision = self.shell_revision.wrapping_add(1);
                    self.application_confirmation = Some(neighborhood_usability::Confirmation {
                        choice,
                        revision: self.shell_revision,
                    });
                }
                return task;
            }
            Message::CancelApplicationChoice(confirmation) => {
                if !effect && self.application_confirmation == Some(confirmation) {
                    self.application_confirmation = None;
                    self.shell_revision = self.shell_revision.wrapping_add(1);
                }
                return task;
            }
            Message::ConfirmApplicationChoice(confirmation) => {
                if effect
                    || !self.frontend.options_open
                    || self.application_confirmation != Some(confirmation)
                {
                    return task;
                }
                self.application_confirmation = None;
                self.frontend.options_open = false;
                self.shell_revision = self.shell_revision.wrapping_add(1);
                Message::ConfiguredRestart(confirmation.choice)
            }
            Message::ConfiguredRestart(_) => return task,
            other => other,
        };
        if self.application_confirmation.is_some()
            && !effect
            && !matches!(message, Message::Load | Message::Restart)
        {
            return task;
        }
        if matches!(message, Message::Save | Message::Load | Message::Loaded(_)) || replacement_message(&message) { self.copy_notice = None; }
        if replacement_message(&message) {
            self.application_confirmation = None;
            // A diagnostic Load or a new owner supersedes the shell's last
            // picker request. Use the next owner's real typed seed afterward.
            if !matches!(message, Message::LoadFrom(..)) {
                self.last_load_path = None;
            }
            self.replacing = true;
        }
        self.enqueue_origin(generation, message, popup, camera_shortcut, save_shortcut);
        let next = self.pump();
        Task::batch([task, next])
    }
    fn close_when_idle(&mut self) -> Task<Event> {
        if self.worker.as_ref().is_some_and(Worker::busy)
            || (self.error.is_none() && !self.queued.is_empty())
        {
            return Task::none();
        }
        if let Some(id) = self.closing.take() {
            // The existing busy/accepted-queue drain condition is UI intent,
            // not completion of a still-open delivery/notification guard.
            if let Some(worker) = &self.worker { worker.record_ui_retirement_intent(); }
            self.close_sent = true;
            window::close(id)
        } else {
            Task::none()
        }
    }
    fn view_messages(&self) -> Element<'_, Message> {
        let view = self.frontend.view_with_copy_gate(
            self.application_confirmation,
            self.error.is_none() && !self.replacing && self.frontend.session_present,
            !self.picker_owns_input()
                && self.closing.is_none()
                && self.picker_feedback.is_none()
                && !self.worker.as_ref().is_some_and(Worker::busy),
            self.copy_enabled() && self.load_picker.is_none(),
        );
        let view = if let Some(feedback) = self.picker_feedback.as_deref().or(self.copy_notice.as_deref()) {
            widget::stack![view, feedback_notice::view(feedback, self.frontend.hud_height)].into()
        } else { view };
        control_wiring::picker_overlay_with_recent_gate(view, self.load_picker.as_ref().filter(|_| self.picker_owns_input()), self.picker_enabled() && (!self.copy_picker() || self.copy_enabled()), self.frontend.modal, self.recent_choices_enabled())
    }
    pub fn view(&self) -> Element<'_, Event> {
        let generation = self.generation();
        let popup = PopupSource {
            owner: self.frontend.data.native_popup.clone(),
            epoch: self.frontend.data.native_popup_epoch,
        };
        let view = self.view_messages()
            .map(move |message| Event::Ui { generation, message, popup: popup.clone() });
        let view = if let Some(error) = self
            .error
            .as_deref()
            .or_else(|| self.closing.map(|_| "Closing…"))
        {
            widget::stack![
                view,
                container(text(error).size(12).color(Color::from_rgb8(245, 131, 115)))
                    .width(Fill)
                    .height(Fill)
                    .align_y(iced::alignment::Vertical::Bottom)
            ]
            .into()
        } else {
            view
        };
        let observation = self.observation_value().and_then(window_session::state);
        window_session::observe(view, observation)
    }
    fn observation_value(&self) -> Option<serde_json::Value> {
        self.frontend.data.window_observation.as_ref().map(|observation| {
            let mut value = observation.as_ref().clone();
            value["shell"] = serde_json::json!({
                "revision":self.shell_revision,"generation":self.generation(),
                "options":self.frontend.options_open,"diagnostics":self.frontend.diagnostics,
                "confirmation":self.application_confirmation.is_some(),
                "widget_epoch":self.frontend.data.widget_epoch,
                "popup_epoch":self.frontend.data.native_popup_epoch,
                "replacing":self.replacing,"closing":self.closing.is_some(),"error":self.error
            });
            value["owned_window_candidate"] = self.window_capture.diagnostics();
            value["worker_busy"] = self.worker.as_ref().is_some_and(Worker::busy).into();
            value
        })
    }
    pub fn subscription(&self) -> iced::Subscription<Event> {
        iced::Subscription::batch([
            window::frames().map(Event::Frame),
            self.window_capture.subscription().map(Event::WindowCapture),
            iced::event::listen_with(subscription_event)
                .with(self.generation())
                .map(|(generation, message)| Event::Widget {
                    generation,
                    message,
                }),
        ])
    }
    #[cfg(test)]
    pub(super) fn from_viewer_for_test(viewer: Viewer) -> Self {
        let diagnostics = viewer.diagnostics;
        let options_open = viewer.options_open;
        let widget_epoch = viewer.widget_epoch;
        let recent_loads = viewer.recent_loads.snapshot();
        let copy_seed = viewer.args.save.clone();
        let copy_owner = viewer.copy_owner.clone();
        let worker = match Worker::start(Engine::new(viewer)) {
            Ok(worker) => worker,
            Err(error) => panic!("test worker failed: {}", error.error),
        };
        Self {
            frontend: Frontend {
                // The keyboard latch belongs to Host, outside publications.
                data: worker.published().clone(),
                diagnostics,
                options_open,
                widget_epoch,
            },
            worker: Some(worker),
            window_capture: super::window_capture::Capture::new(window_session::enabled()),
            player_shortcuts: Default::default(),
            save_shortcut_authority: Arc::new(AtomicBool::new(true)),
            camera_shortcuts: Default::default(),
            load_picker: None,
            last_load_path: None,
            recent_loads,
            picker_feedback: None,
            copy_notice: None,
            in_flight_load: None,
            in_flight_copy: None,
            copy_seed,
            copy_owner,
            last_copy_path: None,
            copy_focused: true,
            queued: VecDeque::new(),
            replacing: false,
            error: None,
            closing: None,
            close_sent: false,
            shell_revision: 0,
            application_confirmation: None,
            next_gate: None,
            next_notification: None,
        }
    }
    #[cfg(test)]
    pub(super) fn notification_for_test(&mut self, notification: Box<dyn FnOnce() + Send>) {
        assert!(self.next_notification.is_none());
        self.next_notification = Some(notification);
    }
    #[cfg(test)]
    pub(super) fn disconnect_results_for_test(&mut self) {
        self.worker.as_mut().unwrap().disconnect_results_for_test();
    }
    #[cfg(test)]
    pub(super) fn loaded_input_publication_for_test(&self) -> Option<serde_json::Value> {
        self.frontend.data.window_observation.as_ref().map(|value| value["loaded_input"].clone())
    }
    #[cfg(test)]
    pub(super) fn settle_for_test(&mut self) -> Result<Vec<Task<Event>>> {
        let deadline = Instant::now() + std::time::Duration::from_secs(60);
        let mut effects = Vec::new();
        loop {
            let task = self.poll();
            if task.units() != 0 {
                effects.push(task);
            }
            // This explicit test driver polls the broker itself. Its wake
            // tasks are redundant; native Iced effects from poll() above are
            // still returned intact to the test.
            drop(self.pump());
            ensure!(self.error.is_none(), "worker error: {:?}", self.error);
            if self.queued.is_empty() && self.worker.as_ref().is_none_or(|w| !w.busy()) {
                return Ok(effects);
            }
            ensure!(
                Instant::now() < deadline,
                "worker did not reach its requested boundary"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[cfg(test)]
    pub(super) fn recover_for_test(mut self) -> Viewer {
        let mut owner = *self.worker.take().unwrap().recover().ok().unwrap().viewer;
        owner.diagnostics = self.frontend.diagnostics;
        owner.options_open = self.frontend.options_open;
        owner.recent_loads = self.recent_loads;
        owner.recent_loads_in_host = false;
        owner
    }
    #[cfg(test)]
    pub(super) fn recover_panicked_for_test(mut self) -> bool {
        self.worker.take().unwrap().recover().is_err()
    }
    #[cfg(test)]
    pub(super) fn audited_for_test(viewer: Viewer, epoch: Instant, millis: u32) -> Self {
        let diagnostics = viewer.diagnostics;
        let options_open = viewer.options_open;
        let widget_epoch = viewer.widget_epoch;
        let mut engine = Engine::new(viewer);
        engine.audit_epoch = Some((epoch, millis));
        let recent_loads = engine.viewer.recent_loads.snapshot();
        let copy_seed = engine.viewer.args.save.clone();
        let copy_owner = engine.viewer.copy_owner.clone();
        let worker = match Worker::start(engine) {
            Ok(w) => w,
            Err(e) => panic!("audit worker: {}", e.error),
        };
        Self {
            frontend: Frontend {
                data: worker.published().clone(),
                diagnostics,
                options_open,
                widget_epoch,
            },
            worker: Some(worker),
            window_capture: super::window_capture::Capture::new(window_session::enabled()),
            player_shortcuts: Default::default(),
            save_shortcut_authority: Arc::new(AtomicBool::new(true)),
            camera_shortcuts: Default::default(),
            load_picker: None,
            last_load_path: None,
            recent_loads,
            picker_feedback: None,
            copy_notice: None,
            in_flight_load: None,
            in_flight_copy: None,
            copy_seed,
            copy_owner,
            last_copy_path: None,
            copy_focused: true,
            queued: VecDeque::new(),
            replacing: false,
            error: None,
            closing: None,
            close_sent: false,
            shell_revision: 0,
            application_confirmation: None,
            next_gate: None,
            next_notification: None,
        }
    }
    #[cfg(test)]
    pub(super) fn audit_for_test(&self) -> Result<serde_json::Value> {
        let mut audit = self
            .frontend
            .data
            .audit
            .clone()
            .context("worker audit not enabled")?;
        audit["application"]["diagnostics"] = serde_json::json!(self.frontend.diagnostics);
        audit["application"]["options_open"] = serde_json::json!(self.frontend.options_open);
        Ok(audit)
    }
    #[cfg(test)]
    pub(super) fn current_generation_for_test(&self) -> u64 {
        self.generation()
    }
    #[cfg(test)]
    pub(super) fn publication_for_test(&self) -> &Arc<Published> {
        &self.frontend.data
    }
    #[cfg(test)]
    pub(super) fn player_shortcut_queue_for_test(&self) -> Vec<(u64, Message)> {
        self.queued.iter().map(|item| (item.generation, item.work.message.clone())).collect()
    }
    #[cfg(test)]
    pub(super) fn install_synthetic_publication_for_test(&mut self, publication: Published) {
        assert!(self.frontend_idle_for_test());
        self.frontend.data = Arc::new(publication);
    }
    #[cfg(test)]
    pub(super) fn frontend_for_test(&self) -> Result<&Frontend> {
        ensure!(
            self.error.is_none()
                && !self.replacing
                && self.closing.is_none()
                && self.application_confirmation.is_none()
                && !self.frontend.diagnostics
                && !self.frontend.options_open,
            "Go Here requires the current unobscured play frontend"
        );
        Ok(&self.frontend)
    }
    #[cfg(test)]
    pub(super) fn frontend_idle_for_test(&self) -> bool {
        self.queued.is_empty() && self.worker.as_ref().is_some_and(|worker| !worker.busy())
    }
    #[cfg(test)]
    pub(super) fn widget_epoch_for_test(&self) -> u64 {
        self.frontend.widget_epoch
    }
    #[cfg(test)]
    pub(super) fn ui_event_for_test(&self, publication: &Published, message: Message) -> Event {
        Event::Ui {
            generation: self.generation(),
            message,
            popup: PopupSource {
                owner: publication.native_popup.clone(),
                epoch: publication.native_popup_epoch,
            },
        }
    }
    #[cfg(test)]
    pub(super) fn gate_next_for_test(
        &mut self,
    ) -> (
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::SyncSender<()>,
    ) {
        assert!(self.next_gate.is_none());
        let (started, entered) = std::sync::mpsc::sync_channel(1);
        let (resume, release) = std::sync::mpsc::sync_channel(1);
        self.next_gate = Some(TestGate {
            started,
            resume: release,
        });
        (entered, resume)
    }
}

#[cfg(test)]
fn capture_for_test(v: &Viewer, epoch: Instant, millis: u32) -> Result<serde_json::Value> {
    let s = v.session.as_ref().context("audit Session absent")?;
    Ok(serde_json::json!({"schema":1,"session":s.save(),
    "dialog_client":v.client.as_ref().context("audit client absent")?.save(s)?,
    "camera_device":v.device.as_ref().map(|d|d.save(s)).transpose()?,
    "application":{
        "diagnostics":v.diagnostics,"options_open":v.options_open,"display_size":v.display_size,
        "tutorial_highlight":v.tutorial_highlight,"resident":v.resident,"selected":v.selected,
        "floor":v.floor,"zoom":v.zoom,"playing":v.playing,"speed":v.speed,
        "last_ms":v.last.map(|t|t.duration_since(epoch).as_millis() as u64),
        "clock_origin":v.clock_origin.map(|(t,n)|(t.duration_since(epoch).as_millis() as u64,n)),
        "accumulator":v.accumulator,"pan":v.pan,"shift":v.shift,
        "viewport_enter_requested":v.viewport_enter_requested,"move_inputs":v.move_inputs,
        "status":v.status,"error":v.error,"menu_error":v.menu_error,"millis":millis
    }}))
}

#[cfg(test)]
#[path = "worker_host/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "worker_host/wiring_tests.rs"]
mod wiring_tests;

#[cfg(test)]
#[path = "window_capture/tests.rs"]
mod window_capture_tests;
