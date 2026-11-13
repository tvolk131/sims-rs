use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, overlay, renderer, widget};
use iced::{Element, Event, Length, Rectangle, Size, Theme, mouse};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
#[path = "window_session/presentation.rs"]
mod presentation;

struct Recorder {
    start: Instant,
    sender: mpsc::SyncSender<Record>,
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
    #[cfg(target_os = "macos")]
    Presentation(presentation::Packet),
    Flush(mpsc::SyncSender<()>),
}

static RECORDER: OnceLock<Recorder> = OnceLock::new();

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
        if self.value != value {
            self.id = self.id.wrapping_add(1);
            self.value = value;
        }
        let mut diagnostics = json!({"clock":clock});
        if let Some(worker_busy) = worker_busy {
            diagnostics["worker_busy"] = worker_busy;
        }
        Arc::new(json!({"state_id":self.id,"value":self.value,"diagnostics":diagnostics}))
    }
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
    let (sender, receiver) = mpsc::sync_channel::<Record>(8192);
    std::thread::Builder::new()
        .name("window-session-writer".into())
        .spawn(move || {
            let mut writer = std::io::BufWriter::new(file);
            loop {
                match receiver.recv_timeout(Duration::from_millis(50)) {
                    Ok(Record::Event(value)) => {
                        if serde_json::to_writer(&mut writer, &value).is_err()
                            || writer.write_all(b"\n").is_err()
                        {
                            break;
                        }
                    }
                    #[cfg(target_os = "macos")]
                    Ok(Record::Presentation(value)) => {
                        if serde_json::to_writer(&mut writer, &value).is_err()
                            || writer.write_all(b"\n").is_err()
                        {
                            break;
                        }
                    }
                    Ok(Record::Flush(done)) => {
                        if writer.flush().is_ok() && writer.get_ref().sync_data().is_ok() {
                            let _ = done.send(());
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if writer.flush().is_err() {
                    break;
                }
            }
        })?;
    let observer_fence = FenceMonitor::start(output.join("observer-fence"))?;
    if RECORDER
        .set(Recorder {
            start: Instant::now(),
            sender,
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
        json!({"schema":1,"pid":std::process::id(),
        "unix_ns":SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos().to_string(),
        "scope":"Production window; CPU widget draw and scene preparation/encoding, not GPU duration, present completion or photon latency",
        "gpu_time_available":false,"present_time_available":false,
        "observer_fence":{"mode":"background_poll","poll_ms":5,
            "draw":"atomic snapshot","failed_read":"retain last complete token"}}),
    );
    Ok(())
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

pub(crate) fn finish() {
    if let Some(recorder) = RECORDER.get() {
        recorder.observer_fence.stop();
        #[cfg(target_os = "macos")]
        presentation::finish();
        emit(
            "session_end",
            json!({"dropped_records":recorder.dropped.load(Ordering::Relaxed)}),
        );
        let (sender, receiver) = mpsc::sync_channel(1);
        if recorder.sender.try_send(Record::Flush(sender)).is_ok() {
            let _ = receiver.recv_timeout(Duration::from_secs(2));
        }
    }
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
