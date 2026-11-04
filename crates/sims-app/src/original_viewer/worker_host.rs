//! Iced shell around the unchanged synchronous household owner. Only one
//! engine update runs at a time. Frames keep painting the last publication;
//! they do not enqueue extra simulation work while that update is executing.
use super::owned_worker::{Owner, SubmitError, Worker};
use super::ui_publication::{Published, view::Frontend};
use super::*;
use std::collections::VecDeque;

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
struct Engine {
    viewer: Box<Viewer>,
    popup: Option<sims_runtime::original_session::live_menu::State>,
    popup_epoch: u64,
    #[cfg(test)]
    audit_epoch: Option<(Instant, u32)>,
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
    fn new(viewer: Viewer) -> Self {
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
                self.object_progress_fixture.as_ref(), self.viewer.session.as_ref(),
            ) {
                (Some(fixture), _) => (&fixture.vm, None, None),
                (None, Some(session)) => {
                    let simulation = &session.simulation;
                    let route = simulation.vm.object_threads.get(&native.actor)
                        .and_then(|thread| simulation.routes.get(thread));
                    (&simulation.vm, route, simulation.visit)
                }
                (None, None) => return,
            };
            object.progress = Some(Box::new(super::ui_publication::JourneyObjectProgress::capture(
                vm, native.actor, route, visit,
            )));
        }
    }
}

impl Owner for Engine {
    type Command = Work;
    type Reply = Task<Message>;
    type Published = Published;
    fn apply(&mut self, work: Work) -> std::result::Result<Self::Reply, String> {
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
                return Ok(Task::none());
            }
        }
        let reply = self.viewer.update(work.message);
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
        Ok(reply)
    }
    fn publish(&self) -> std::result::Result<Published, String> {
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
        Message::Load | Message::Restart | Message::ConfiguredRestart(_)
    )
}
struct Queued {
    generation: u64,
    work: Work,
}
pub(super) struct Host {
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
}
impl Host {
    pub fn new(args: Args) -> (Self, Task<Event>) {
        let (owner, task) = Viewer::new(args);
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
    fn poll(&mut self) -> Task<Event> {
        let Some(worker) = &mut self.worker else {
            return Task::none();
        };
        match worker.poll() {
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
                    // Input created by the previous world cannot be replayed on
                    // reused native IDs after Load/Restart.
                    self.queued
                        .retain(|q| q.generation == completion.generation);
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
                    Ok(task) => {
                        let generation = completion.generation;
                        task.map(move |message| Event::Effect {
                            generation,
                            message,
                        })
                    }
                    Err(error) => {
                        self.error = Some(error);
                        Task::none()
                    }
                }
            }
            Ok(None) => Task::none(),
            Err(error) => {
                self.error = Some(format!("Household worker disconnected: {error}"));
                Task::none()
            }
        }
    }
    fn enqueue(&mut self, generation: u64, message: Message) {
        self.enqueue_origin(generation, message, None);
    }
    fn enqueue_origin(&mut self, generation: u64, message: Message, popup: Option<PopupSource>) {
        let shell = self.shell();
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
        if self.queued.len() == 256 {
            self.error =
                Some("Household input queue is full; no input was executed out of order.".into());
            return;
        }
        self.queued.push_back(Queued {
            generation,
            work: Work {
                message,
                shell,
                popup,
                #[cfg(test)]
                gate: self.next_gate.take(),
            },
        });
    }
    fn pump(&mut self) -> Task<Event> {
        if self.error.is_some() {
            return Task::none();
        }
        let Some(worker) = &mut self.worker else {
            return Task::none();
        };
        if worker.busy() {
            return Task::none();
        }
        while let Some(next) = self.queued.pop_front() {
            if next.generation != worker.generation() {
                continue;
            }
            let (sender, receiver) = iced::futures::channel::oneshot::channel();
            match worker.submit_notifying(
                next.work,
                Some(Box::new(move || {
                    let _ = sender.send(());
                })),
            ) {
                Ok(_) => {
                    return Task::perform(
                        async move {
                            let _ = receiver.await;
                        },
                        |()| Event::Completion,
                    );
                }
                Err(error) => match error {
                    SubmitError::Busy(work) => self.queued.push_front(Queued {
                        generation: next.generation,
                        work,
                    }),
                    SubmitError::Disconnected(_) => {
                        self.error = Some("Household worker disconnected.".into())
                    }
                    SubmitError::Quarantined(_) => {
                        self.error = Some("Household owner requires application restart.".into())
                    }
                    SubmitError::Exhausted(_) => {
                        self.error = Some("Household worker ticket space exhausted.".into())
                    }
                },
            }
            break;
        }
        Task::none()
    }
    pub fn update(&mut self, event: Event) -> Task<Event> {
        let task = self.poll();
        let (generation, message, effect, popup) = match event {
            Event::Close(id) => {
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
                    && self.queued.is_empty()
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
        if generation != self.generation() {
            return task;
        }
        if (self.closing.is_some() || self.close_sent) && !effect {
            return task;
        }
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
        if replacement_message(&message) {
            self.application_confirmation = None;
            self.replacing = true;
        }
        self.enqueue_origin(generation, message, popup);
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
            self.close_sent = true;
            window::close(id)
        } else {
            Task::none()
        }
    }
    pub fn view(&self) -> Element<'_, Event> {
        let generation = self.generation();
        let popup = PopupSource {
            owner: self.frontend.data.native_popup.clone(),
            epoch: self.frontend.data.native_popup_epoch,
        };
        let view = self
            .frontend
            .view_with_confirmation(
                self.application_confirmation,
                self.error.is_none() && !self.replacing && self.frontend.session_present,
            )
            .map(move |message| Event::Ui {
                generation,
                message,
                popup: popup.clone(),
            });
        if let Some(error) = self
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
        }
    }
    pub fn subscription(&self) -> iced::Subscription<Event> {
        iced::Subscription::batch([
            window::frames().map(Event::Frame),
            window::close_requests().map(Event::Close),
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
        let worker = match Worker::start(Engine::new(viewer)) {
            Ok(worker) => worker,
            Err(error) => panic!("test worker failed: {}", error.error),
        };
        Self {
            frontend: Frontend {
                data: worker.published().clone(),
                diagnostics,
                options_open,
                widget_epoch,
            },
            worker: Some(worker),
            queued: VecDeque::new(),
            replacing: false,
            error: None,
            closing: None,
            close_sent: false,
            shell_revision: 0,
            application_confirmation: None,
            next_gate: None,
        }
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
        owner
    }
    #[cfg(test)]
    pub(super) fn audited_for_test(viewer: Viewer, epoch: Instant, millis: u32) -> Self {
        let diagnostics = viewer.diagnostics;
        let options_open = viewer.options_open;
        let widget_epoch = viewer.widget_epoch;
        let mut engine = Engine::new(viewer);
        engine.audit_epoch = Some((epoch, millis));
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
            queued: VecDeque::new(),
            replacing: false,
            error: None,
            closing: None,
            close_sent: false,
            shell_revision: 0,
            application_confirmation: None,
            next_gate: None,
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
