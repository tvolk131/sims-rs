use anyhow::Context;
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, overlay, renderer, widget};
use iced::{Element, Event, Length, Rectangle, Size, Theme, mouse};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_JOURNAL_BYTES: usize = 64 * 1024 * 1024;
const CANDIDATE_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(target_os = "macos")]
#[path = "window_session/presentation.rs"]
mod presentation;

#[path = "window_session/loaded_input.rs"]
pub(crate) mod loaded_input;

struct Recorder {
    loaded_input: loaded_input::Account,
    start: Instant,
    sender: mpsc::SyncSender<Record>,
    writer_result: Mutex<mpsc::Receiver<anyhow::Result<()>>>,
    dropped: AtomicU64,
    frames: AtomicU64,
    drawn_state: AtomicU64,
    state: Mutex<ObservedState>,
    scene: Mutex<SceneIdentity>,
    previous_draw: Mutex<Option<Instant>>,
    observer_fence: Arc<FenceMonitor>,
    fence: AtomicU64,
    inputs: InputSequence,
    spans: AtomicU64,
}

#[derive(Default)]
struct InputSequence {
    value: Mutex<u64>,
}

impl InputSequence {
    fn record(&self, publish: impl FnOnce(u64)) -> bool {
        let Ok(mut sequence) = self.value.lock() else {
            return false;
        };
        *sequence += 1;
        publish(*sequence);
        true
    }

    fn snapshot(&self, publish: impl FnOnce(u64)) -> bool {
        let Ok(sequence) = self.value.lock() else {
            return false;
        };
        publish(*sequence);
        true
    }
}

struct FenceMonitor {
    value: AtomicU64,
    running: AtomicBool,
}

impl FenceMonitor {
    fn new() -> Self {
        Self {
            value: AtomicU64::new(0),
            running: AtomicBool::new(true),
        }
    }

    fn start(path: PathBuf) -> std::io::Result<Arc<Self>> {
        let monitor = Arc::new(Self::new());
        let worker = Arc::clone(&monitor);
        std::thread::Builder::new()
            .name("window-session-fence".into())
            .spawn(move || {
                while worker.running.load(Ordering::Acquire) {
                    worker.refresh(|| {
                        std::fs::read(&path)
                            .ok()
                            .and_then(|bytes| fence_token(&bytes))
                    });
                    std::thread::sleep(Duration::from_millis(5));
                }
            })?;
        Ok(monitor)
    }

    fn refresh(&self, read: impl FnOnce() -> Option<u64>) {
        let token = read();
        if self.running.load(Ordering::Acquire)
            && let Some(token) = token
        {
            self.value.store(token, Ordering::Release);
        }
    }

    fn snapshot(&self) -> u64 {
        self.value.load(Ordering::Acquire)
    }

    fn stop(&self) {
        self.running.store(false, Ordering::Release);
    }
}

fn fence_token(bytes: &[u8]) -> Option<u64> {
    <[u8; 8]>::try_from(bytes).ok().map(u64::from_le_bytes)
}

enum Record {
    Event(Value),
    WindowCandidate {
        value: Value,
        done: mpsc::SyncSender<()>,
    },
    #[cfg(target_os = "macos")]
    Presentation(presentation::Packet),
    Finalize {
        end: Value,
        done: mpsc::SyncSender<()>,
    },
}

static RECORDER: OnceLock<Recorder> = OnceLock::new();

trait DurableWriter: Write {
    fn sync_data(&self) -> std::io::Result<()>;
}

impl DurableWriter for std::io::BufWriter<std::fs::File> {
    fn sync_data(&self) -> std::io::Result<()> {
        self.get_ref().sync_data()
    }
}

/// Enforce the reader's journal profile at the producer. An exhausted budget
/// fails the writer and cannot produce a successful session_end or candidate
/// acknowledgement. Presentation packets use this same bounded writer.
struct BoundedWriter<W> {
    inner: W,
    remaining: usize,
}

impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(std::io::Error::other("window session journal byte budget exhausted"));
        }
        let written = self.inner.write(bytes)?;
        self.remaining -= written;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl<W: DurableWriter> DurableWriter for BoundedWriter<W> {
    fn sync_data(&self) -> std::io::Result<()> {
        self.inner.sync_data()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicationStatus {
    Pending,
    DurablyRecorded,
    Unavailable,
}

/// This exact queue item's private one-shot acknowledgement. Enqueue success
/// alone is not publication. The UI polls without blocking and must separately
/// reject an obsolete window lifetime or household generation.
#[derive(Debug)]
pub(crate) struct CandidatePublication {
    acknowledgement: mpsc::Receiver<()>,
    deadline: Instant,
    status: PublicationStatus,
}

impl CandidatePublication {
    pub(crate) fn poll(&mut self, now: Instant) -> PublicationStatus {
        if self.status == PublicationStatus::Pending {
            self.status = if now >= self.deadline {
                PublicationStatus::Unavailable
            } else {
                match self.acknowledgement.try_recv() {
                    Ok(()) => PublicationStatus::DurablyRecorded,
                    Err(mpsc::TryRecvError::Empty) => PublicationStatus::Pending,
                    Err(mpsc::TryRecvError::Disconnected) => PublicationStatus::Unavailable,
                }
            };
        }
        self.status
    }
}

fn write_record(writer: &mut impl Write, value: &impl serde::Serialize) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *writer, value).context("serialize/write window session record")?;
    writer
        .write_all(b"\n")
        .context("write window session record separator")
}

fn write_records(
    mut writer: impl DurableWriter,
    receiver: mpsc::Receiver<Record>,
) -> anyhow::Result<()> {
    loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(Record::Event(value)) => write_record(&mut writer, &value)?,
            Ok(Record::WindowCandidate { value, done }) => {
                write_record(&mut writer, &value).context("write owned window candidate")?;
                writer.flush().context("flush owned window candidate")?;
                writer.sync_data().context("sync owned window candidate")?;
                let _ = done.try_send(());
            }
            #[cfg(target_os = "macos")]
            Ok(Record::Presentation(value)) => write_record(&mut writer, &value)?,
            Ok(Record::Finalize { end, done }) => {
                write_record(&mut writer, &end).context("write window session_end")?;
                writer
                    .flush()
                    .context("flush final window session records")?;
                writer
                    .sync_data()
                    .context("sync final window session records")?;
                let _ = done.send(());
                return Ok(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("window session queue disconnected before session_end");
            }
        }
        writer.flush().context("flush window session records")?;
    }
}

fn spawn_writer(
    writer: impl DurableWriter + Send + 'static,
    capacity: usize,
) -> anyhow::Result<(mpsc::SyncSender<Record>, mpsc::Receiver<anyhow::Result<()>>)> {
    let (sender, receiver) = mpsc::sync_channel(capacity);
    let (done, result) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("window-session-writer".into())
        .spawn(move || {
            let _ = done.send(write_records(writer, receiver));
        })?;
    Ok((sender, result))
}

fn finalize(
    sender: &mpsc::SyncSender<Record>,
    writer_result: &mpsc::Receiver<anyhow::Result<()>>,
    end: Value,
    timeout: Duration,
) -> anyhow::Result<()> {
    let started = Instant::now();
    let (done, acknowledgement) = mpsc::sync_channel(1);
    let acknowledgement_error = match sender.try_send(Record::Finalize { end, done }) {
        Ok(()) => match acknowledgement.recv_timeout(timeout.saturating_sub(started.elapsed())) {
            Ok(()) => return Ok(()),
            Err(error) => anyhow::Error::new(error).context(format!(
                "window session finalization: durable acknowledgement not received within {timeout:?}"
            )),
        },
        Err(mpsc::TrySendError::Full(_)) => {
            anyhow::bail!("window session finalization queue full; session_end was not queued");
        }
        Err(mpsc::TrySendError::Disconnected(_)) => anyhow::anyhow!(
            "window session finalization queue disconnected; session_end was not queued"
        ),
    };
    match writer_result.recv_timeout(timeout.saturating_sub(started.elapsed())) {
        Ok(Err(error)) => Err(error.context(format!("{acknowledgement_error:#}"))),
        Ok(Ok(())) => Err(acknowledgement_error),
        Err(error) => {
            let context = format!(
                "{acknowledgement_error}; window session writer result unavailable: {error}"
            );
            Err(acknowledgement_error.context(context))
        }
    }
}

#[derive(Default)]
struct ObservedState {
    id: u64,
    value: Value,
}

impl ObservedState {
    fn snapshot(&mut self, mut value: Value) -> Arc<Value> {
        let clock = value
            .as_object_mut()
            .and_then(|value| value.remove("clock"));
        let worker_busy = value
            .as_object_mut()
            .and_then(|value| value.remove("worker_busy"));
        // These counters describe this immutable capture's work, rather than
        // a change of loaded owner. Keep their exact copied values with the
        // frame without making every successful no-op a new state identity.
        let mut loaded_input_work = serde_json::Map::new();
        if let Some(input) = value.get_mut("loaded_input").and_then(Value::as_object_mut) {
            for name in ["work_ordinal", "active_work"] {
                if let Some(work) = input.remove(name) {
                    loaded_input_work.insert(name.into(), work);
                }
            }
        }
        if self.value != value {
            self.id = self.id.wrapping_add(1);
            self.value = value;
        }
        let mut diagnostics = json!({"clock":clock});
        if let Some(worker_busy) = worker_busy {
            diagnostics["worker_busy"] = worker_busy;
        }
        if !loaded_input_work.is_empty() {
            diagnostics["loaded_input_work"] = Value::Object(loaded_input_work);
        }
        Arc::new(json!({"state_id":self.id,"value":self.value,"diagnostics":diagnostics}))
    }
}

#[cfg(test)]
pub(crate) fn observed_state_for_test(value: Value) -> Arc<Value> {
    ObservedState::default().snapshot(value)
}

#[derive(Default)]
struct SceneIdentity {
    id: u64,
    previous: Option<Arc<sims_render::Scene>>,
}

impl SceneIdentity {
    fn observe(&mut self, scene: &Arc<sims_render::Scene>) -> u64 {
        let unchanged = self.previous.as_ref().is_some_and(|previous| {
            Arc::ptr_eq(previous, scene) || scene_content_equal(previous, scene)
        });
        if !unchanged {
            self.id = self.id.wrapping_add(1);
        }
        self.previous = Some(scene.clone());
        self.id
    }
}

fn scene_content_equal(previous: &sims_render::Scene, current: &sims_render::Scene) -> bool {
    previous.width == current.width
        && previous.height == current.height
        && previous.vertices.len() == current.vertices.len()
        && previous
            .vertices
            .iter()
            .zip(&current.vertices)
            .all(|(previous, current)| {
                previous.position.map(f32::to_bits) == current.position.map(f32::to_bits)
                    && previous.uv.map(f32::to_bits) == current.uv.map(f32::to_bits)
                    && previous.tint.map(f32::to_bits) == current.tint.map(f32::to_bits)
                    && previous.depth_scale.to_bits() == current.depth_scale.to_bits()
            })
        && previous.draws.len() == current.draws.len()
        && previous
            .draws
            .iter()
            .zip(&current.draws)
            .all(|(previous, current)| {
                previous.vertices == current.vertices
                    && previous.texture == current.texture
                    && previous.object == current.object
            })
        && previous.censors.len() == current.censors.len()
        && previous
            .censors
            .iter()
            .zip(&current.censors)
            .all(|(previous, current)| {
                previous.rect == current.rect
                    && previous.depth.to_bits() == current.depth.to_bits()
                    && previous.seed == current.seed
            })
        && (Arc::ptr_eq(&previous.textures, &current.textures)
            || previous.textures == current.textures)
}

fn cache_root(checkout: &Path) -> anyhow::Result<PathBuf> {
    let checkout = checkout.canonicalize()?;
    let cache = checkout.join("cache");
    anyhow::ensure!(
        !std::fs::symlink_metadata(&cache)?.file_type().is_symlink()
            && cache.canonicalize()? == cache,
        "the cache root must be a real directory inside the checkout"
    );
    Ok(cache)
}

pub(crate) fn start(output: &Path, game: &Path) -> anyhow::Result<()> {
    super::validate_output(game, output)?;
    let cache = cache_root(&std::env::current_dir()?)?;
    let parent = output
        .parent()
        .ok_or_else(|| anyhow::anyhow!("recording parent absent"))?;
    anyhow::ensure!(
        parent.canonicalize()?.starts_with(cache),
        "window recordings must remain inside the local cache directory"
    );
    std::fs::create_dir(output)?;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("events.jsonl"))?;
    let writer = BoundedWriter {
        inner: std::io::BufWriter::new(file),
        remaining: MAX_JOURNAL_BYTES,
    };
    let (sender, writer_result) = spawn_writer(writer, 8192)?;
    let observer_fence = FenceMonitor::start(output.join("observer-fence"))?;
    if RECORDER
        .set(Recorder {
            loaded_input: loaded_input::Account::default(),
            start: Instant::now(),
            sender,
            writer_result: Mutex::new(writer_result),
            dropped: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            drawn_state: AtomicU64::new(0),
            state: Mutex::new(ObservedState::default()),
            scene: Mutex::new(SceneIdentity::default()),
            previous_draw: Mutex::new(None),
            observer_fence: Arc::clone(&observer_fence),
            fence: AtomicU64::new(0),
            inputs: InputSequence::default(),
            spans: AtomicU64::new(0),
        })
        .is_err()
    {
        observer_fence.stop();
        anyhow::bail!("window recorder already started");
    }
    emit(
        "session",
        json!({"schema":1,"pid":std::process::id(),"extensions":[loaded_input::TAG],
        "unix_ns":SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos().to_string(),
        "scope":"Production window; CPU widget draw and scene preparation/encoding, not GPU duration, present completion or photon latency",
        "gpu_time_available":false,"present_time_available":false,
        "observer_fence":{"mode":"background_poll","poll_ms":5,
            "draw":"atomic snapshot","failed_read":"retain last complete token"}}),
    );
    Ok(())
}

pub(crate) fn register_viewer() -> Option<loaded_input::Registration> {
    Some(RECORDER.get()?.loaded_input.register())
}

pub(crate) fn enabled() -> bool {
    RECORDER.get().is_some()
}

pub(crate) fn start_presentation(path: &Path) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        presentation::load(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        anyhow::bail!(
            "native presentation recording requires macOS: {}",
            path.display()
        )
    }
}

pub(crate) fn bind_presentation(target: &wgpu::TextureView) {
    #[cfg(target_os = "macos")]
    if let Some(recorder) = RECORDER.get() {
        presentation::bind(
            target,
            recorder.frames.load(Ordering::Relaxed),
            recorder.drawn_state.load(Ordering::Relaxed),
        );
    }
    #[cfg(not(target_os = "macos"))]
    let _ = target;
}

pub(crate) fn emit(kind: &str, body: Value) {
    if let Some(recorder) = RECORDER.get() {
        let value = json!({"kind":kind,"elapsed_us":recorder.start.elapsed().as_micros(),
            "dropped_records":recorder.dropped.load(Ordering::Relaxed),"data":body});
        if recorder.sender.try_send(Record::Event(value)).is_err() {
            recorder.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn enqueue_window_candidate(
    sender: &mpsc::SyncSender<Record>,
    dropped: &AtomicU64,
    start: Instant,
    now: Instant,
    window_id: u32,
    generation: u64,
) -> Option<CandidatePublication> {
    if window_id == 0 {
        return None;
    }
    let deadline = now.checked_add(CANDIDATE_PUBLICATION_TIMEOUT)?;
    // Fixed schema, fixed strings and bounded integer fields; no arbitrary
    // caller Value or native pointer can enlarge this candidate record.
    let value = json!({"kind":"owned_window_candidate", "elapsed_us":now.saturating_duration_since(start).as_micros(),
        "dropped_records":dropped.load(Ordering::Relaxed),
        "data":{"schema":"sims-owned-window-candidate-v1", "window_id":window_id,
            "generation":generation, "pid":std::process::id(),
            "scope":"observational candidate; independent retained-execution and exact-window verification required"}});
    let (done, acknowledgement) = mpsc::sync_channel(1);
    if sender.try_send(Record::WindowCandidate { value, done }).is_err() {
        dropped.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    Some(CandidatePublication { acknowledgement, deadline, status: PublicationStatus::Pending })
}

/// Queue an already accepted app-owned observation, still UNRECORDED until the
/// returned exact acknowledgement reports durability. This is never execution
/// or kernel authority, nor confirmation that the window remains current.
pub(crate) fn window_number_candidate(window_id: u32, generation: u64) -> Option<CandidatePublication> {
    let recorder = RECORDER.get()?;
    enqueue_window_candidate(&recorder.sender, &recorder.dropped, recorder.start,
        Instant::now(), window_id, generation)
}

pub(crate) struct Span {
    id: u64,
    name: &'static str,
    started: Instant,
}

pub(crate) fn span(name: &'static str, details: impl FnOnce() -> Value) -> Option<Span> {
    let recorder = RECORDER.get()?;
    let span = Span {
        id: recorder.spans.fetch_add(1, Ordering::Relaxed) + 1,
        name,
        started: Instant::now(),
    };
    let details = details();
    if !recorder.inputs.snapshot(|sequence| {
        emit(
            "span_begin",
            json!({"span_id":span.id,"name":name,"details":details,
            "input_sequence":sequence}),
        );
    }) {
        recorder.dropped.fetch_add(1, Ordering::Relaxed);
    }
    Some(span)
}

impl Drop for Span {
    fn drop(&mut self) {
        emit(
            "span_end",
            json!({"span_id":self.id,"name":self.name,
            "wall_us":self.started.elapsed().as_micros()}),
        );
    }
}

pub(crate) fn input(event: &Event) {
    let Some(recorder) = RECORDER.get() else {
        return;
    };
    if matches!(
        event,
        Event::Window(iced::window::Event::RedrawRequested(_))
    ) {
        return;
    }
    let received_unix_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let input = match event {
        Event::Keyboard(iced::keyboard::Event::KeyPressed { physical_key, .. }) => {
            json!({"kind":"key_pressed","physical_key":format!("{physical_key:?}")})
        }
        Event::Keyboard(iced::keyboard::Event::KeyReleased { physical_key, .. }) => {
            json!({"kind":"key_released","physical_key":format!("{physical_key:?}")})
        }
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            json!({"kind":"pointer_moved","point":[position.x,position.y]})
        }
        Event::Mouse(mouse::Event::ButtonPressed(button)) => {
            json!({"kind":"button_pressed","button":format!("{button:?}")})
        }
        Event::Mouse(mouse::Event::ButtonReleased(button)) => {
            json!({"kind":"button_released","button":format!("{button:?}")})
        }
        _ => Value::Null,
    };
    if !recorder.inputs.record(|sequence| {
        emit(
            "input",
            json!({"event":format!("{event:?}"),"input":input,
            "fence":recorder.fence.load(Ordering::Relaxed),"input_sequence":sequence,
            "received_unix_ns":received_unix_ns}),
        );
    }) {
        recorder.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn finish() -> anyhow::Result<()> {
    if let Some(recorder) = RECORDER.get() {
        recorder.observer_fence.stop();
        #[cfg(target_os = "macos")]
        presentation::finish();
        let dropped = recorder.dropped.load(Ordering::Relaxed);
        let end = json!({"kind":"session_end","elapsed_us":recorder.start.elapsed().as_micros(),
            "dropped_records":dropped,"data":{"dropped_records":dropped,
                "loaded_input":recorder.loaded_input.terminal()}});
        let writer_result = recorder
            .writer_result
            .try_lock()
            .map_err(|error| anyhow::anyhow!("window session finalization unavailable: {error}"))?;
        finalize(
            &recorder.sender,
            &writer_result,
            end,
            Duration::from_secs(2),
        )?;
    }
    Ok(())
}

pub(crate) fn state(value: Value) -> Option<Arc<Value>> {
    let recorder = RECORDER.get()?;
    let mut previous = recorder.state.lock().ok()?;
    Some(previous.snapshot(value))
}

pub(crate) fn scene_id(scene: Option<&Arc<sims_render::Scene>>) -> Option<u64> {
    Some(RECORDER.get()?.scene.lock().ok()?.observe(scene?))
}

pub(crate) fn stage(name: &str, started: Instant) {
    if let Some(recorder) = RECORDER.get() {
        emit(
            name,
            json!({"frame_id":recorder.frames.load(Ordering::Relaxed),
            "cpu_us":started.elapsed().as_micros()}),
        );
    }
}

pub(crate) fn prepared(
    started: Instant,
    preparation: sims_render::gpu::Preparation,
    scene: &sims_render::Scene,
) {
    if let Some(recorder) = RECORDER.get() {
        let cpu_us = started.elapsed().as_micros();
        let referenced_textures = (preparation.uploaded_textures > 0).then(|| {
            scene
                .draws
                .iter()
                .map(|draw| draw.texture)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        });
        emit(
            "scene_prepare",
            json!({
                "frame_id":recorder.frames.load(Ordering::Relaxed),"cpu_us":cpu_us,
                "uploaded_textures":preparation.uploaded_textures,
                "uploaded_bytes":preparation.uploaded_bytes,
                "software_composited":preparation.software_composited,
                "texture_slots":scene.textures.len(),"referenced_textures":referenced_textures,
                "vertices":scene.vertices.len(),"draws":scene.draws.len(),"censors":scene.censors.len()
            }),
        );
    }
}

pub(crate) fn observe<'a, Message: 'a>(
    content: Element<'a, Message>,
    state: Option<Arc<Value>>,
) -> Element<'a, Message> {
    if let Some(state) = state {
        Element::new(Observed { content, state })
    } else {
        content
    }
}

struct Observed<'a, Message> {
    content: Element<'a, Message>,
    state: Arc<Value>,
}

fn root_stage_wall_us(checkpoints: [Instant; 7]) -> Value {
    json!({
        "timestamp":checkpoints[1].duration_since(checkpoints[0]).as_micros(),
        "fence_snapshot":checkpoints[2].duration_since(checkpoints[1]).as_micros(),
        "observation_bookkeeping":checkpoints[3].duration_since(checkpoints[2]).as_micros(),
        "presentation_context":checkpoints[4].duration_since(checkpoints[3]).as_micros(),
        "draw_interval_bookkeeping":checkpoints[5].duration_since(checkpoints[4]).as_micros(),
        "content_draw":checkpoints[6].duration_since(checkpoints[5]).as_micros(),
    })
}

impl<Message> Widget<Message, Theme, iced::Renderer> for Observed<'_, Message> {
    fn tag(&self) -> widget::tree::Tag {
        self.content.as_widget().tag()
    }
    fn state(&self) -> widget::tree::State {
        self.content.as_widget().state()
    }
    fn children(&self) -> Vec<widget::Tree> {
        self.content.as_widget().children()
    }
    fn diff(&self, tree: &mut widget::Tree) {
        self.content.as_widget().diff(tree);
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }
    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }
    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }
    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let Some(recorder) = RECORDER.get() else {
            return;
        };
        let now = Instant::now();
        let started_unix_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|time| time.as_nanos());
        let timestamp_finished = Instant::now();
        let fence = recorder.observer_fence.snapshot();
        let fence_finished = Instant::now();
        recorder.fence.store(fence, Ordering::Relaxed);
        let frame = recorder.frames.fetch_add(1, Ordering::Relaxed) + 1;
        let state_id = self.state["state_id"].as_u64().unwrap_or(0);
        recorder.drawn_state.store(state_id, Ordering::Relaxed);
        let bookkeeping_finished = Instant::now();
        #[cfg(target_os = "macos")]
        presentation::draw_context(frame, state_id);
        let presentation_finished = Instant::now();
        let interval = recorder
            .previous_draw
            .lock()
            .ok()
            .and_then(|mut previous| previous.replace(now))
            .map(|previous| now.duration_since(previous).as_micros());
        let content_started = Instant::now();
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
        let content_finished = Instant::now();
        if !recorder.inputs.snapshot(|sequence| {
            emit(
                "frame",
                json!({"frame_id":frame,"state":self.state.as_ref(),
            "fence":fence,"draw_started_unix_ns":started_unix_ns,
            "input_sequence":sequence,
            "cursor":cursor.position().map(|point| [point.x,point.y]),
            "widget_draw_cpu_us":now.elapsed().as_micros(),"draw_interval_us":interval,
            "root_stage_wall_us":root_stage_wall_us([now,timestamp_finished,fence_finished,
                bookkeeping_finished,presentation_finished,content_started,content_finished])}),
            );
        }) {
            recorder.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut widget::Tree,
        layout: Layout<'a>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, iced::Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loaded_input_work_is_diagnostic_while_owner_and_failure_changes_keep_distinct_ids() {
        let account = loaded_input::Account::default();
        let registration = account.register();
        // Authored ledger input, not a restored Session or selected-save proof.
        let origin = sims_runtime::original_session::player_input::Observation {
            boundary: "player_application_envelope", sha256: "0".repeat(64), byte_count: 3,
            requested_path: "authored-ledger-input".into(), path_identity_observed: false,
            descriptor_before: None, descriptor_after: None, descriptor_status: "unavailable",
        };
        let setup = registration.work();
        registration.transition(false, true, Some(&origin));
        registration.preparation_result(true);
        setup.complete(true);
        let capture = || json!({"tick":7,"loaded_input":registration.capture(true, Some(&origin))});
        let mut observed = ObservedState::default();
        let first_work = registration.work();
        let first_capture = capture();
        let first = observed.snapshot(first_capture.clone());
        first_work.complete(true);
        let next_work = registration.work();
        let next = observed.snapshot(capture());
        assert_eq!(first["state_id"], next["state_id"]);
        assert_eq!(first["value"], next["value"]);
        assert_ne!(first["diagnostics"]["loaded_input_work"], next["diagnostics"]["loaded_input_work"]);
        assert!(first["value"]["loaded_input"].get("work_ordinal").is_none());
        assert!(first["value"]["loaded_input"].get("active_work").is_none());
        assert_eq!(first["diagnostics"]["loaded_input_work"]["work_ordinal"], first_capture["loaded_input"]["work_ordinal"]);
        next_work.complete(true);
        let replacement_work = registration.work();
        registration.transition(true, true, Some(&origin));
        registration.preparation_result(true);
        let replaced = observed.snapshot(capture());
        assert_ne!(next["state_id"], replaced["state_id"]);
        assert_eq!(replaced["value"]["loaded_input"]["transition_ordinal"], 2);
        replacement_work.complete(true);
        registration.work().complete(false);
        let failed = observed.snapshot(capture());
        assert_ne!(replaced["state_id"], failed["state_id"]);
        assert_eq!(failed["value"]["loaded_input"]["ownership_status"], "unknown_invalid");
        assert_eq!(failed["value"]["loaded_input"]["last_operation"], "failed");
        assert_eq!(first["value"]["loaded_input"]["transition_ordinal"], 1);
        assert_eq!(first_capture["loaded_input"]["active_work"].as_array().unwrap().len(), 1);
    }

    #[derive(Clone, Copy)]
    enum WriterFailure {
        Write,
        WriteAfter(usize),
        Separator,
        Flush,
        Sync,
    }

    #[derive(Default)]
    struct WriterOutput {
        bytes: Vec<u8>,
        durable: Vec<u8>,
        operations: Vec<&'static str>,
    }

    #[derive(Default)]
    struct TestWriter {
        output: Arc<Mutex<WriterOutput>>,
        failure: Option<WriterFailure>,
        sync_gate: Option<(mpsc::SyncSender<()>, mpsc::Receiver<()>)>,
    }

    impl Write for TestWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let mut output = self.output.lock().unwrap();
            if matches!(self.failure, Some(WriterFailure::Write))
                || matches!(self.failure, Some(WriterFailure::WriteAfter(limit)) if output.bytes.len() >= limit)
            {
                return Err(std::io::Error::other("injected write failure"));
            }
            if matches!(self.failure, Some(WriterFailure::Separator)) && bytes == b"\n" {
                return Err(std::io::Error::other("injected separator failure"));
            }
            output.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.output.lock().unwrap().operations.push("flush");
            if matches!(self.failure, Some(WriterFailure::Flush)) {
                return Err(std::io::Error::other("injected flush failure"));
            }
            Ok(())
        }
    }

    impl DurableWriter for TestWriter {
        fn sync_data(&self) -> std::io::Result<()> {
            self.output.lock().unwrap().operations.push("sync");
            if matches!(self.failure, Some(WriterFailure::Sync)) {
                return Err(std::io::Error::other("injected sync failure"));
            }
            if let Some((entered, resume)) = &self.sync_gate {
                entered
                    .send(())
                    .map_err(|error| std::io::Error::other(error.to_string()))?;
                resume
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|error| std::io::Error::other(error.to_string()))?;
            }
            let mut output = self.output.lock().unwrap();
            output.durable = output.bytes.clone();
            Ok(())
        }
    }

    #[test]
    fn window_candidate_is_not_recorded_until_the_exact_writer_sync_finishes() {
        let (entered, syncing) = mpsc::sync_channel(1);
        let (resume, waiting) = mpsc::sync_channel(1);
        let writer = TestWriter { sync_gate: Some((entered, waiting)), ..TestWriter::default() };
        let output = Arc::clone(&writer.output);
        let (sender, result) = spawn_writer(writer, 4).unwrap();
        let now = Instant::now();
        let mut pending = enqueue_window_candidate(&sender, &AtomicU64::new(0), now, now, 91, 3).unwrap();
        syncing.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(pending.poll(Instant::now()), PublicationStatus::Pending);
        assert!(output.lock().unwrap().durable.is_empty());
        resume.send(()).unwrap();
        loop {
            match pending.poll(Instant::now()) {
                PublicationStatus::Pending => std::thread::sleep(Duration::from_millis(1)),
                PublicationStatus::DurablyRecorded => break,
                other => panic!("candidate never became durable: {other:?}"),
            }
        }
        let durable = output.lock().unwrap().durable.clone();
        assert_eq!(durable.last(), Some(&b'\n'));
        let value: Value = serde_json::from_slice(&durable).unwrap();
        assert_eq!(value["kind"], "owned_window_candidate");
        assert_eq!(value["data"]["window_id"], 91);
        assert_eq!(value["data"]["generation"], 3);
        assert_eq!(value["data"]["pid"], std::process::id());
        assert!(durable.len() < 1024);
        // Finalization has its own sync; release that independent gate too.
        resume.send(()).unwrap();
        finalize(&sender, &result, session_end(), Duration::from_secs(2)).unwrap();
        assert!(result.recv_timeout(Duration::from_secs(2)).unwrap().is_ok());
    }

    #[test]
    fn window_candidate_write_separator_flush_and_sync_failures_never_acknowledge() {
        for failure in [WriterFailure::Write, WriterFailure::WriteAfter(1),
                        WriterFailure::Separator, WriterFailure::Flush, WriterFailure::Sync] {
            let writer = TestWriter { failure: Some(failure), ..TestWriter::default() };
            let output = Arc::clone(&writer.output);
            let (sender, result) = spawn_writer(writer, 1).unwrap();
            let now = Instant::now();
            let mut pending = enqueue_window_candidate(&sender, &AtomicU64::new(0), now, now, 91, 3).unwrap();
            assert!(result.recv_timeout(Duration::from_secs(2)).unwrap().is_err());
            assert_eq!(pending.poll(Instant::now()), PublicationStatus::Unavailable);
            assert!(output.lock().unwrap().durable.is_empty());
        }
    }

    #[test]
    fn window_candidate_full_disconnected_and_invalid_requests_remain_unrecorded() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let dropped = AtomicU64::new(0);
        let now = Instant::now();
        assert!(enqueue_window_candidate(&sender, &dropped, now, now, 0, 3).is_none());
        assert!(matches!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty)));
        sender.try_send(Record::Event(json!({"kind":"already_queued"}))).unwrap();
        assert!(enqueue_window_candidate(&sender, &dropped, now, now, 91, 3).is_none());
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
        assert!(matches!(receiver.try_recv().unwrap(), Record::Event(_)));
        drop(receiver);
        assert!(enqueue_window_candidate(&sender, &dropped, now, now, 91, 3).is_none());
        assert_eq!(dropped.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn window_candidate_expiration_is_terminal_even_with_a_late_exact_acknowledgement() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let now = Instant::now();
        let mut pending = enqueue_window_candidate(&sender, &AtomicU64::new(0), now, now, 91, 3).unwrap();
        let Record::WindowCandidate { done, .. } = receiver.recv().unwrap() else { panic!() };
        assert_eq!(pending.poll(now + CANDIDATE_PUBLICATION_TIMEOUT), PublicationStatus::Unavailable);
        done.try_send(()).unwrap();
        assert_eq!(pending.poll(now), PublicationStatus::Unavailable);
    }

    #[test]
    fn window_candidate_another_queue_item_cannot_satisfy_its_acknowledgement() {
        let (sender, receiver) = mpsc::sync_channel(2);
        let now = Instant::now();
        let dropped = AtomicU64::new(0);
        let mut old = enqueue_window_candidate(&sender, &dropped, now, now, 91, 3).unwrap();
        let mut current = enqueue_window_candidate(&sender, &dropped, now, now, 92, 4).unwrap();
        let Record::WindowCandidate { done: old_done, .. } = receiver.recv().unwrap() else { panic!() };
        let Record::WindowCandidate { done: current_done, .. } = receiver.recv().unwrap() else { panic!() };
        old_done.try_send(()).unwrap();
        assert_eq!(old.poll(now), PublicationStatus::DurablyRecorded);
        assert_eq!(current.poll(now), PublicationStatus::Pending);
        current_done.try_send(()).unwrap();
        assert_eq!(current.poll(now), PublicationStatus::DurablyRecorded);
    }

    #[test]
    fn journal_producer_budget_counts_encoded_bytes_and_preserves_the_exact_cap() {
        let inner = TestWriter::default();
        let output = Arc::clone(&inner.output);
        let mut writer = BoundedWriter { inner, remaining: 3 };
        writer.write_all("é".as_bytes()).unwrap();
        writer.write_all(b"\n").unwrap();
        assert!(writer.write_all(b"x").is_err());
        writer.flush().unwrap();
        writer.sync_data().unwrap();
        let output = output.lock().unwrap();
        assert_eq!(output.bytes, "é\n".as_bytes());
        assert_eq!(output.durable, output.bytes);
        assert_eq!(writer.remaining, 0);
    }

    #[test]
    fn journal_producer_budget_failure_cannot_acknowledge_a_partial_candidate() {
        let inner = TestWriter::default();
        let output = Arc::clone(&inner.output);
        let writer = BoundedWriter { inner, remaining: 1 };
        let (sender, result) = spawn_writer(writer, 1).unwrap();
        let now = Instant::now();
        let mut pending = enqueue_window_candidate(&sender, &AtomicU64::new(0), now, now, 91, 3).unwrap();
        assert!(format!("{:#}", result.recv_timeout(Duration::from_secs(2)).unwrap().unwrap_err())
            .contains("journal byte budget exhausted"));
        assert_eq!(pending.poll(Instant::now()), PublicationStatus::Unavailable);
        let output = output.lock().unwrap();
        assert!(output.bytes.len() <= 1);
        assert!(output.durable.is_empty());
    }


    #[test]
    fn window_candidate_real_file_is_durable_before_ack_and_finalizes_within_budget() {
        let path = std::env::temp_dir().join(format!("sims-window-candidate-{}-{}.jsonl",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
        let writer = BoundedWriter { inner: std::io::BufWriter::new(file), remaining: 2048 };
        let (sender, result) = spawn_writer(writer, 2).unwrap();
        let now = Instant::now();
        let mut pending = enqueue_window_candidate(&sender, &AtomicU64::new(0), now, now,
            u32::MAX, u64::MAX).unwrap();
        loop {
            match pending.poll(Instant::now()) {
                PublicationStatus::Pending => std::thread::sleep(Duration::from_millis(1)),
                PublicationStatus::DurablyRecorded => break,
                other => panic!("file candidate was not recorded: {other:?}"),
            }
        }
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.last(), Some(&b'\n'));
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["data"]["window_id"], u32::MAX);
        assert_eq!(value["data"]["generation"], u64::MAX);
        finalize(&sender, &result, session_end(), Duration::from_secs(2)).unwrap();
        assert!(result.recv_timeout(Duration::from_secs(2)).unwrap().is_ok());
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() <= 2048);
        assert_eq!(bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()).count(), 2);
        std::fs::remove_file(path).unwrap();
    }

    fn session_end() -> Value {
        json!({"kind":"session_end","elapsed_us":12,"dropped_records":0,
            "data":{"dropped_records":0}})
    }

    fn failing_finalization(failure: WriterFailure, ordinary_event: bool) -> String {
        let writer = TestWriter {
            failure: Some(failure),
            ..TestWriter::default()
        };
        let (sender, result) = spawn_writer(writer, 4).unwrap();
        if ordinary_event {
            sender.send(Record::Event(json!({"kind":"input"}))).unwrap();
        }
        let error = finalize(&sender, &result, session_end(), Duration::from_secs(2)).unwrap_err();
        format!("{error:#}")
    }

    #[test]
    fn finalization_rejects_full_queue_even_with_a_successful_old_acknowledgement() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (done, result) = mpsc::sync_channel(1);
        sender
            .try_send(Record::Event(json!({"kind":"input"})))
            .unwrap();
        done.send(Ok(())).unwrap();
        let error = finalize(&sender, &result, session_end(), Duration::ZERO).unwrap_err();
        assert!(error.to_string().contains("queue full"));
        assert!(matches!(receiver.try_recv().unwrap(), Record::Event(_)));
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert!(result.try_recv().unwrap().is_ok());
    }

    #[test]
    fn finalization_rejects_disconnected_queue_even_with_a_successful_old_acknowledgement() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (done, result) = mpsc::sync_channel(1);
        drop(receiver);
        done.send(Ok(())).unwrap();
        let error = finalize(&sender, &result, session_end(), Duration::ZERO).unwrap_err();
        assert!(error.to_string().contains("queue disconnected"));
        assert!(error.to_string().contains("session_end was not queued"));
    }

    #[test]
    fn disconnected_queue_preserves_the_writer_failure_diagnostic() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (done, result) = mpsc::sync_channel(1);
        drop(receiver);
        done.send(Err(anyhow::anyhow!("disk full").context("write failed")))
            .unwrap();
        let error = finalize(&sender, &result, session_end(), Duration::ZERO).unwrap_err();
        assert_eq!(error.root_cause().to_string(), "disk full");
        assert!(error.to_string().contains("queue disconnected"));
        assert!(format!("{error:#}").contains("write failed: disk full"));
    }

    #[test]
    fn finalization_times_out_without_a_durable_acknowledgement() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (_done, result) = mpsc::sync_channel(1);
        let error = finalize(&sender, &result, session_end(), Duration::ZERO).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("durable acknowledgement not received")
        );
        assert!(matches!(
            error.downcast_ref::<mpsc::RecvTimeoutError>(),
            Some(mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(matches!(
            receiver.try_recv().unwrap(),
            Record::Finalize { .. }
        ));
    }

    #[test]
    fn finalization_rejects_a_disconnected_acknowledgement_channel() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (done, result) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            drop(done);
            let request = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(matches!(request, Record::Finalize { .. }));
            drop(request);
        });
        let error = finalize(&sender, &result, session_end(), Duration::from_secs(2)).unwrap_err();
        worker.join().unwrap();
        assert!(matches!(
            error.downcast_ref::<mpsc::RecvTimeoutError>(),
            Some(mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn finalization_reports_an_ordinary_record_write_failure() {
        let error = failing_finalization(WriterFailure::Write, true);
        assert!(error.contains("serialize/write window session record"));
        assert!(error.contains("injected write failure"));
    }

    #[test]
    fn finalization_reports_session_end_write_failure() {
        let error = failing_finalization(WriterFailure::Write, false);
        assert!(error.contains("write window session_end"));
        assert!(error.contains("injected write failure"));
    }

    #[test]
    fn finalization_reports_session_end_separator_failure() {
        let error = failing_finalization(WriterFailure::Separator, false);
        assert!(error.contains("write window session record separator"));
        assert!(error.contains("injected separator failure"));
    }

    #[test]
    fn finalization_reports_an_ordinary_flush_failure() {
        let error = failing_finalization(WriterFailure::Flush, true);
        assert!(error.contains("flush window session records"));
        assert!(error.contains("injected flush failure"));
    }

    #[test]
    fn finalization_reports_the_final_flush_failure() {
        let error = failing_finalization(WriterFailure::Flush, false);
        assert!(error.contains("flush final window session records"));
        assert!(error.contains("injected flush failure"));
    }

    #[test]
    fn finalization_reports_sync_failure() {
        let error = failing_finalization(WriterFailure::Sync, false);
        assert!(error.contains("sync final window session records"));
        assert!(error.contains("injected sync failure"));
    }

    #[test]
    fn failed_finalization_preserves_previously_written_evidence() {
        let event = json!({"kind":"input"});
        let expected = format!("{event}\n").into_bytes();
        let writer = TestWriter {
            failure: Some(WriterFailure::WriteAfter(expected.len())),
            ..TestWriter::default()
        };
        let output = Arc::clone(&writer.output);
        let (sender, result) = spawn_writer(writer, 4).unwrap();
        sender.send(Record::Event(event)).unwrap();
        assert!(finalize(&sender, &result, session_end(), Duration::from_secs(2)).is_err());
        let output = output.lock().unwrap();
        assert_eq!(output.bytes, expected);
        assert!(output.durable.is_empty());
    }

    #[test]
    fn writer_cannot_acknowledge_success_without_session_end() {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(sender);
        let error = write_records(TestWriter::default(), receiver).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("disconnected before session_end")
        );
    }

    #[test]
    fn serialization_failure_preserves_its_diagnostic() {
        struct SerializationFailure;

        impl serde::Serialize for SerializationFailure {
            fn serialize<Serializer: serde::Serializer>(
                &self,
                _serializer: Serializer,
            ) -> Result<Serializer::Ok, Serializer::Error> {
                Err(serde::ser::Error::custom("injected serialization failure"))
            }
        }

        let error = write_record(&mut TestWriter::default(), &SerializationFailure).unwrap_err();
        assert!(format!("{error:#}").contains("injected serialization failure"));
    }

    #[test]
    fn finalization_succeeds_only_after_session_end_is_durable() {
        let writer = TestWriter::default();
        let output = Arc::clone(&writer.output);
        let (sender, result) = spawn_writer(writer, 4).unwrap();
        let end = session_end();
        finalize(&sender, &result, end.clone(), Duration::from_secs(2)).unwrap();
        let output = output.lock().unwrap();
        assert_eq!(output.bytes, format!("{end}\n").as_bytes());
        assert_eq!(output.durable, output.bytes);
        assert_eq!(output.operations, ["flush", "sync"]);
    }

    #[test]
    fn durable_acknowledgement_follows_exact_record_order_and_completed_sync() {
        let (entered, pending) = mpsc::sync_channel(1);
        let (release, resume) = mpsc::sync_channel(1);
        let writer = TestWriter {
            sync_gate: Some((entered, resume)),
            ..TestWriter::default()
        };
        let output = Arc::clone(&writer.output);
        let (sender, result) = spawn_writer(writer, 4).unwrap();
        let (done, acknowledgement) = mpsc::sync_channel(1);
        let first = json!({"kind":"input","data":{"sequence":1}});
        let second = json!({"kind":"frame","data":{"sequence":2}});
        let end = session_end();
        sender.send(Record::Event(first.clone())).unwrap();
        sender.send(Record::Event(second.clone())).unwrap();
        sender
            .send(Record::Finalize {
                end: end.clone(),
                done,
            })
            .unwrap();
        pending.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            acknowledgement.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
        {
            let output = output.lock().unwrap();
            assert_eq!(
                output.bytes,
                format!("{first}\n{second}\n{end}\n").as_bytes()
            );
            assert!(output.durable.is_empty());
            assert_eq!(output.operations, ["flush", "flush", "flush", "sync"]);
        }
        release.send(()).unwrap();
        acknowledgement
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        result
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        let output = output.lock().unwrap();
        assert_eq!(output.durable, output.bytes);
    }

    #[test]
    fn a_later_finalization_cannot_reuse_an_earlier_durable_acknowledgement() {
        let (entered, pending) = mpsc::sync_channel(1);
        let (release, resume) = mpsc::sync_channel(1);
        let writer = TestWriter {
            sync_gate: Some((entered, resume)),
            ..TestWriter::default()
        };
        let (sender, result) = spawn_writer(writer, 4).unwrap();
        let (first_done, first_acknowledgement) = mpsc::sync_channel(1);
        let (second_done, second_acknowledgement) = mpsc::sync_channel(1);
        sender
            .send(Record::Finalize {
                end: session_end(),
                done: first_done,
            })
            .unwrap();
        pending.recv_timeout(Duration::from_secs(2)).unwrap();
        sender
            .send(Record::Finalize {
                end: session_end(),
                done: second_done,
            })
            .unwrap();
        release.send(()).unwrap();
        first_acknowledgement
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(matches!(
            second_acknowledgement.recv_timeout(Duration::from_secs(2)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
        result
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
    }

    #[test]
    fn input_receipt_is_enqueued_before_its_draw_acknowledgment() {
        let sequence = Arc::new(InputSequence::default());
        let (sender, receiver) = mpsc::channel();
        let (entered, pending) = mpsc::sync_channel(1);
        let (release, resume) = mpsc::sync_channel(1);
        let input_sequence = Arc::clone(&sequence);
        let input_sender = sender.clone();
        let input = std::thread::spawn(move || {
            input_sequence.record(|sequence| {
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(2)).unwrap();
                input_sender.send(("input", sequence)).unwrap();
            })
        });
        pending.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(sequence.value.try_lock().is_err());
        let frame_sequence = Arc::clone(&sequence);
        let frame = std::thread::spawn(move || {
            frame_sequence.snapshot(|sequence| sender.send(("frame", sequence)).unwrap())
        });
        release.send(()).unwrap();
        assert!(input.join().unwrap());
        assert!(frame.join().unwrap());
        assert_eq!(
            receiver.into_iter().collect::<Vec<_>>(),
            [("input", 1), ("frame", 1)]
        );
    }

    #[test]
    fn draw_snapshot_cannot_be_overtaken_by_the_next_input_receipt() {
        let sequence = Arc::new(InputSequence::default());
        let (sender, receiver) = mpsc::channel();
        let (entered, pending) = mpsc::sync_channel(1);
        let (release, resume) = mpsc::sync_channel(1);
        let frame_sequence = Arc::clone(&sequence);
        let frame_sender = sender.clone();
        let frame = std::thread::spawn(move || {
            frame_sequence.snapshot(|sequence| {
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(2)).unwrap();
                frame_sender.send(("frame", sequence)).unwrap();
            })
        });
        pending.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(sequence.value.try_lock().is_err());
        let input_sequence = Arc::clone(&sequence);
        let input = std::thread::spawn(move || {
            input_sequence.record(|sequence| sender.send(("input", sequence)).unwrap())
        });
        release.send(()).unwrap();
        assert!(frame.join().unwrap());
        assert!(input.join().unwrap());
        assert_eq!(
            receiver.into_iter().collect::<Vec<_>>(),
            [("frame", 0), ("input", 1)]
        );
    }

    #[test]
    fn concurrent_receipts_and_draws_preserve_complete_queue_order() {
        let sequence = Arc::new(InputSequence::default());
        let (sender, receiver) = mpsc::channel();
        std::thread::scope(|scope| {
            for producer in 0..4 {
                let sequence = Arc::clone(&sequence);
                let sender = sender.clone();
                scope.spawn(move || {
                    for _ in 0..1024 {
                        let publish =
                            |sequence| sender.send((producer % 2 == 0, sequence)).unwrap();
                        assert!(if producer % 2 == 0 {
                            sequence.record(publish)
                        } else {
                            sequence.snapshot(publish)
                        });
                        std::thread::yield_now();
                    }
                });
            }
        });
        drop(sender);
        let mut observed_inputs = 0;
        let mut observed_frames = 0;
        for (input, sequence) in receiver {
            if input {
                observed_inputs += 1;
            } else {
                observed_frames += 1;
            }
            assert_eq!(sequence, observed_inputs);
        }
        assert_eq!(observed_inputs, 2048);
        assert_eq!(observed_frames, 2048);
    }

    #[test]
    fn poisoned_input_sequence_rejects_publication_without_a_callback() {
        let sequence = InputSequence::default();
        assert!(
            std::panic::catch_unwind(|| {
                sequence.record(|_| panic!("publication interrupted"));
            })
            .is_err()
        );
        assert!(!sequence.record(|_| panic!("poisoned input published")));
        assert!(!sequence.snapshot(|_| panic!("poisoned frame published")));
    }

    #[test]
    fn observer_fence_requires_one_complete_token() {
        for length in [0, 1, 7, 9, 16] {
            assert!(fence_token(&vec![0; length]).is_none());
        }
        assert_eq!(fence_token(&0_u64.to_le_bytes()), Some(0));
        assert_eq!(fence_token(&19_u64.to_le_bytes()), Some(19));
        assert_eq!(fence_token(&u64::MAX.to_le_bytes()), Some(u64::MAX));
    }

    #[test]
    fn observer_fence_failed_read_cannot_acknowledge_a_later_token() {
        let monitor = FenceMonitor::new();
        monitor.refresh(|| Some(7));
        monitor.refresh(|| None);
        assert_eq!(monitor.snapshot(), 7);
        monitor.refresh(|| Some(8));
        assert_eq!(monitor.snapshot(), 8);
    }

    #[test]
    fn observer_fence_snapshot_does_not_wait_for_pending_io() {
        let monitor = Arc::new(FenceMonitor::new());
        let worker = Arc::clone(&monitor);
        let (entered, pending) = mpsc::sync_channel(1);
        let (release, resume) = mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            worker.refresh(|| {
                entered.send(()).unwrap();
                resume.recv().unwrap();
                Some(9)
            });
        });
        pending.recv_timeout(Duration::from_secs(2)).unwrap();
        let observer = Arc::clone(&monitor);
        let (published, sampled) = mpsc::sync_channel(1);
        let sampler = std::thread::spawn(move || published.send(observer.snapshot()).unwrap());
        let snapshot = sampled.recv_timeout(Duration::from_secs(2));
        release.send(()).unwrap();
        reader.join().unwrap();
        sampler.join().unwrap();
        assert_eq!(snapshot, Ok(0));
        assert_eq!(monitor.snapshot(), 9);
    }

    #[test]
    fn stopped_observer_fence_does_not_publish_a_pending_read() {
        let monitor = Arc::new(FenceMonitor::new());
        monitor.refresh(|| Some(3));
        let worker = Arc::clone(&monitor);
        let (entered, pending) = mpsc::sync_channel(1);
        let (release, resume) = mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            worker.refresh(|| {
                entered.send(()).unwrap();
                resume.recv().unwrap();
                Some(9)
            });
        });
        pending.recv_timeout(Duration::from_secs(2)).unwrap();
        monitor.stop();
        release.send(()).unwrap();
        reader.join().unwrap();
        assert_eq!(monitor.snapshot(), 3);
    }

    #[test]
    fn root_draw_stage_timings_are_disjoint_wall_intervals() {
        let start = Instant::now();
        let checkpoints =
            [0, 3, 10, 21, 34, 51, 70].map(|micros| start + Duration::from_micros(micros));
        assert_eq!(
            root_stage_wall_us(checkpoints),
            json!({"timestamp":3,"fence_snapshot":7,"observation_bookkeeping":11,
                "presentation_context":13,"draw_interval_bookkeeping":17,"content_draw":19})
        );
    }

    #[test]
    fn inactive_recording_preserves_the_unwrapped_path() {
        assert!(!enabled());
        finish().unwrap();
        assert!(state(json!({"tick":3})).is_none());
        emit("unused", Value::Null);
        assert!(
            span("unused", || panic!(
                "disabled tracing evaluated its details"
            ))
            .is_none()
        );
    }

    #[test]
    fn state_ids_are_stable_without_mutating_earlier_observations() {
        let mut state = ObservedState::default();
        let initial = state.snapshot(json!({"tick":3}));
        assert_eq!(initial["state_id"], 1);
        assert_eq!(state.snapshot(json!({"tick":3}))["state_id"], 1);
        assert_eq!(state.snapshot(json!({"tick":4}))["state_id"], 2);
        assert_eq!(state.snapshot(json!({"tick":3}))["state_id"], 3);
        assert_eq!(initial["state_id"], 1);
        assert_eq!(initial["value"]["tick"], 3);
    }

    #[test]
    fn wall_clock_diagnostics_do_not_change_the_visible_state_identity() {
        let mut state = ObservedState::default();
        let first = state.snapshot(json!({"tick":3,"scene_id":1,"clock":{"now":10}}));
        let next = state.snapshot(json!({"tick":3,"scene_id":1,"clock":{"now":20}}));
        assert_eq!(first["state_id"], next["state_id"]);
        assert_eq!(first["diagnostics"]["clock"]["now"], 10);
        assert_eq!(next["diagnostics"]["clock"]["now"], 20);
        assert!(next["value"].get("clock").is_none());
        let changed = state.snapshot(json!({"tick":3,"scene_id":2,"clock":{"now":20}}));
        assert_ne!(next["state_id"], changed["state_id"]);
    }

    #[test]
    fn worker_busy_diagnostics_preserve_visible_state_and_prior_observations() {
        let mut state = ObservedState::default();
        let idle = state.snapshot(json!({"tick":3,"scene_id":1,"worker_busy":false}));
        let busy = state.snapshot(json!({"tick":3,"scene_id":1,"worker_busy":true}));
        assert_eq!(idle["state_id"], busy["state_id"]);
        assert_eq!(idle["value"], busy["value"]);
        assert_eq!(idle["diagnostics"]["worker_busy"], false);
        assert_eq!(busy["diagnostics"]["worker_busy"], true);
        assert!(busy["value"].get("worker_busy").is_none());
        let advanced = state.snapshot(json!({"tick":4,"scene_id":1,"worker_busy":true}));
        assert_ne!(busy["state_id"], advanced["state_id"]);
    }

    #[test]
    fn scene_identity_compares_immutable_content_not_allocation_addresses() {
        let scene = Arc::new(sims_render::Scene {
            vertices: vec![
                sims_render::Vertex {
                    position: [0., 0., 0.],
                    uv: [0., 0.],
                    tint: [1.; 4],
                    depth_scale: 1.,
                };
                3
            ],
            draws: vec![sims_render::Draw {
                vertices: 0..3,
                texture: 0,
                object: None,
            }],
            censors: vec![sims_render::censor::Region {
                rect: [0, 0, 1, 1],
                depth: 0.5,
                seed: 1,
            }],
            textures: Arc::new(vec![sims_render::Texture {
                width: 1,
                height: 1,
                rgba: vec![0; 4].into(),
                depth: vec![1].into(),
            }]),
            width: 10,
            height: 10,
        });
        let mut identity = SceneIdentity::default();
        assert_eq!(identity.observe(&scene), 1);
        assert_eq!(identity.observe(&Arc::new(scene.as_ref().clone())), 1);
        for field in 0..5 {
            let mut changed = scene.as_ref().clone();
            match field {
                0 => changed.width += 1,
                1 => changed.vertices[0].tint[0] = 0.5,
                2 => changed.draws[0].object = Some(3),
                3 => changed.censors[0].seed += 1,
                4 => Arc::make_mut(&mut Arc::make_mut(&mut changed.textures)[0].rgba)[0] = 2,
                _ => unreachable!(),
            }
            let before = identity.id;
            assert_eq!(identity.observe(&Arc::new(changed)), before + 1);
            assert_eq!(identity.observe(&scene), before + 2);
        }
    }

    #[test]
    fn native_window_recording_cannot_be_combined_with_offscreen_drivers() {
        use clap::Parser;
        assert!(
            crate::Args::try_parse_from(["sims-room", "--window-session-output", "cache/session"])
                .is_err()
        );
        for arguments in [
            ["--screenshot", "cache/image.png"],
            ["--exploration-plan", "cache/plan.json"],
        ] {
            assert!(
                crate::Args::try_parse_from([
                    "sims-room",
                    "--original",
                    "Synthetic.iff",
                    "--window-session-output",
                    "cache/session",
                    arguments[0],
                    arguments[1]
                ])
                .is_err()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn cache_root_rejects_symlinks_into_public_paths() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "sims-window-cache-test-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        std::fs::create_dir(&root)?;
        let public = root.join("tracked");
        std::fs::create_dir(&public)?;
        std::os::unix::fs::symlink(&public, root.join("cache"))?;
        let result = cache_root(&root);
        std::fs::remove_file(root.join("cache"))?;
        std::fs::create_dir(root.join("cache"))?;
        assert_eq!(cache_root(&root)?, root.canonicalize()?.join("cache"));
        std::fs::remove_dir_all(&root)?;
        assert!(result.is_err());
        Ok(())
    }
}

/// Bounded real-file fixture for the Host/leaf/publication join. No global
/// recorder, native handles, fake acknowledgements or production entry points.
/// Delaying finish() deliberately leaves genuine queue items unsynced.
#[cfg(test)]
pub(crate) mod candidate_journal_fixture {
    use super::*;

    pub(crate) struct Journal {
        sender: mpsc::SyncSender<Record>,
        receiver: Option<mpsc::Receiver<Record>>,
        writer: Option<BoundedWriter<std::io::BufWriter<std::fs::File>>>,
        dropped: AtomicU64,
        start: Instant,
    }

    impl Journal {
        pub(crate) fn new(path: &Path, capacity: usize, budget: usize) -> anyhow::Result<Self> {
            anyhow::ensure!(capacity > 0 && capacity <= 8 && budget <= MAX_JOURNAL_BYTES,
                "fixture must remain bounded");
            let file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            let (sender, receiver) = mpsc::sync_channel(capacity);
            Ok(Self {
                sender, receiver: Some(receiver),
                writer: Some(BoundedWriter { inner: std::io::BufWriter::new(file), remaining: budget }),
                dropped: AtomicU64::new(0), start: Instant::now(),
            })
        }

        pub(crate) fn enqueue(&self, number: u32, generation: u64, now: Instant) -> Option<CandidatePublication> {
            enqueue_window_candidate(&self.sender, &self.dropped, self.start, now, number, generation)
        }

        pub(crate) fn fill(&self) {
            assert!(self.sender.try_send(Record::Event(json!({"kind":"fixture_queue_occupant"}))).is_ok());
        }

        pub(crate) fn disconnect(&mut self) {
            self.receiver.take();
        }

        pub(crate) fn finish(&mut self) -> anyhow::Result<()> {
            let receiver = self.receiver.take().context("fixture receiver absent")?;
            let writer = self.writer.take().context("fixture writer absent")?;
            let (done, acknowledgement) = mpsc::sync_channel(1);
            self.sender.try_send(Record::Finalize { end: json!({"kind":"session_end",
                "elapsed_us":self.start.elapsed().as_micros(), "dropped_records":self.dropped.load(Ordering::Relaxed),
                "data":{"dropped_records":self.dropped.load(Ordering::Relaxed)}}), done })
                .map_err(|_| anyhow::anyhow!("fixture finalization queue unavailable"))?;
            // Identical bounded write/flush/sync/ack consumer as production.
            write_records(writer, receiver)?;
            acknowledgement.try_recv().context("fixture finalization was not acknowledged")?;
            Ok(())
        }
    }
}
