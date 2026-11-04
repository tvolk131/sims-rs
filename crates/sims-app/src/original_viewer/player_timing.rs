use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub(super) type Id = u64;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Scope {
    ConstructionToSettled,
    LoaderExecution,
    JourneyOperation,
    Play,
    UiDispatch,
    NativeProof,
    EvidenceValidation,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    FreshLoad,
    InitialPlay,
    PostReloadPlay,
    CancelContinuation,
    ResidentSelection,
    BuyLive,
    Buy,
    Live,
    InteractionQueueCancel,
    OfferedInteraction,
    Cancellation,
    ManagedSaveLoad,
    ProductionSave,
    ManagedLoad,
    OtherUi,
    TutorialCamera,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Role {
    PlayerPath,
    PrimarySave,
    ControlSave,
    ManagedRestore,
    ValidationResave,
    PrimaryEvidenceCopy,
    ControlEvidenceCopy,
    InputReselection,
    ResaveEvidenceCopy,
    HashAndFullStateValidation,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Outcome {
    Completed,
    Error,
    Unsupported,
    Superseded,
    Censored,
}

#[derive(Clone, Default, Serialize)]
pub(super) struct Observation {
    pub generation: u64,
    pub session_present: bool,
    pub pending: bool,
    pub modal: bool,
    pub playing: bool,
    pub application_error: bool,
    pub menu_error: bool,
    pub owner_drained: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock_minute: Option<i64>,
    pub commands: usize,
    pub frames: u32,
    pub service_polls: usize,
    pub effect_polls: usize,
    pub task_streams: usize,
}

impl Observation {
    fn ready(&self, expected: u64) -> bool {
        self.generation == expected
            && self.session_present
            && !self.pending
            && !self.modal
            && !self.application_error
            && !self.menu_error
            && self.owner_drained
    }
}

#[derive(Serialize)]
struct Deltas {
    #[serde(skip_serializing_if = "Option::is_none")]
    tick: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clock_minute: Option<i64>,
    commands: usize,
    frames: u32,
    service_polls: usize,
    effect_polls: usize,
    task_streams: usize,
}

impl Deltas {
    fn between(before: &Observation, after: &Observation) -> Self {
        Self {
            tick: before
                .tick
                .zip(after.tick)
                .and_then(|(before, after)| after.checked_sub(before)),
            clock_minute: before
                .clock_minute
                .zip(after.clock_minute)
                .and_then(|(before, after)| after.checked_sub(before)),
            commands: after.commands.saturating_sub(before.commands),
            frames: after.frames.saturating_sub(before.frames),
            service_polls: after.service_polls.saturating_sub(before.service_polls),
            effect_polls: after.effect_polls.saturating_sub(before.effect_polls),
            task_streams: after.task_streams.saturating_sub(before.task_streams),
        }
    }
}

#[derive(Serialize)]
struct Row {
    id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<Id>,
    scope: Scope,
    operation: Operation,
    role: Role,
    start_ns: u64,
    end_ns: Option<u64>,
    elapsed_ns: Option<u64>,
    shell_end_ns: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    drive_return_ns: Option<u64>,
    drained_ns: Option<u64>,
    settled_observed_ns: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    loader_request_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    before: Option<Observation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    after: Option<Observation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    settled_observation: Option<Observation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deltas: Option<Deltas>,
    outcome: Outcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip)]
    needs_settled: bool,
    #[serde(skip)]
    loader: Option<LoaderPayload>,
}

struct Buffer {
    rows: Vec<Row>,
    capacity: usize,
    truncated: bool,
    loader_failure: Option<&'static str>,
}

#[derive(Clone)]
pub(super) struct Recorder {
    epoch: Instant,
    buffer: Arc<Mutex<Buffer>>,
    loader_clock: Option<Arc<dyn Fn() -> LoaderResult<u64> + Send + Sync>>,
}

impl Recorder {
    pub fn enabled(enabled: bool, capacity: usize) -> Option<Self> {
        enabled.then(|| Self {
            epoch: Instant::now(),
            buffer: Arc::new(Mutex::new(Buffer {
                rows: Vec::new(),
                capacity,
                truncated: false,
                loader_failure: None,
            })),
            loader_clock: None,
        })
    }

    fn now(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    pub fn timestamp(&self) -> u64 {
        self.now()
    }

    pub fn start(
        &self,
        parent_id: Option<Id>,
        scope: Scope,
        operation: Operation,
        role: Role,
        before: Option<Observation>,
        needs_settled: bool,
    ) -> Option<Id> {
        let mut buffer = self.buffer.lock().unwrap();
        if buffer.rows.len() >= buffer.capacity {
            buffer.truncated = true;
            return None;
        }
        let id = buffer.rows.len() as Id;
        let generation = before.as_ref().map(|before| before.generation);
        let expected_generation =
            if scope == Scope::JourneyOperation && operation == Operation::ManagedSaveLoad {
                generation.and_then(|generation| generation.checked_add(1))
            } else {
                generation
            };
        buffer.rows.push(Row {
            id,
            parent_id,
            scope,
            operation,
            role,
            start_ns: self.now(),
            end_ns: None,
            elapsed_ns: None,
            shell_end_ns: None,
            drive_return_ns: None,
            drained_ns: None,
            settled_observed_ns: None,
            request_generation: generation,
            expected_generation,
            observed_generation: None,
            loader_request_id: None,
            before,
            after: None,
            settled_observation: None,
            deltas: None,
            outcome: Outcome::Censored,
            error: None,
            needs_settled,
            loader: None,
        });
        Some(id)
    }

    fn update(&self, id: Option<Id>, update: impl FnOnce(&mut Row, u64)) {
        if let Some(id) = id {
            let now = self.now();
            if let Some(row) = self.buffer.lock().unwrap().rows.get_mut(id as usize) {
                update(row, now);
            }
        }
    }

    pub fn expect_generation(&self, id: Option<Id>, request: u64, expected: Option<u64>) {
        self.update(id, |row, _| {
            row.request_generation = Some(request);
            row.expected_generation = expected;
        });
    }

    pub fn dispatch<Reply>(&self, id: Option<Id>, dispatch: impl FnOnce() -> Reply) -> Reply {
        let start = self.now();
        let reply = dispatch();
        let shell_end = self.now();
        self.update(id, |row, _| {
            row.start_ns = start;
            row.shell_end_ns = Some(shell_end);
        });
        let mut buffer = self.buffer.lock().unwrap();
        let parent = id
            .and_then(|id| buffer.rows.get(id as usize))
            .and_then(|row| row.parent_id);
        if let Some(parent) = parent.and_then(|parent| buffer.rows.get_mut(parent as usize)) {
            if parent.scope == Scope::NativeProof && parent.shell_end_ns.is_none() {
                parent.start_ns = start;
                parent.shell_end_ns = Some(shell_end);
            }
        }
        reply
    }

    pub fn drive_return(&self, id: Option<Id>, drained: bool) {
        self.drive_return_at(id, self.now(), drained);
    }

    pub fn drive_return_at(&self, id: Option<Id>, returned: u64, drained: bool) {
        self.update(id, |row, _| {
            row.drive_return_ns = Some(returned);
            if drained {
                row.drained_ns = Some(returned);
            }
        });
        let mut buffer = self.buffer.lock().unwrap();
        let parent = id
            .and_then(|id| buffer.rows.get(id as usize))
            .and_then(|row| row.parent_id);
        if let Some(parent) = parent.and_then(|parent| buffer.rows.get_mut(parent as usize)) {
            if parent.scope == Scope::NativeProof && parent.drive_return_ns.is_none() {
                parent.drive_return_ns = Some(returned);
                parent.drained_ns = drained.then_some(returned);
            }
        }
    }

    pub fn finish(
        &self,
        id: Option<Id>,
        outcome: Outcome,
        after: Option<Observation>,
        error: Option<&str>,
    ) {
        self.finish_at(id, self.now(), outcome, after, error);
    }

    pub fn finish_at(
        &self,
        id: Option<Id>,
        end: u64,
        outcome: Outcome,
        after: Option<Observation>,
        error: Option<&str>,
    ) {
        self.update(id, |row, _| {
            if row.end_ns.is_some() {
                return;
            }
            row.end_ns = Some(end);
            row.elapsed_ns = Some(end.saturating_sub(row.start_ns));
            row.observed_generation = after.as_ref().map(|after| after.generation);
            row.deltas = row
                .before
                .as_ref()
                .zip(after.as_ref())
                .map(|(before, after)| Deltas::between(before, after));
            row.after = after;
            row.outcome = outcome;
            row.error = error.map(|error| error.chars().take(512).collect());
        });
    }

    pub fn finish_checkpoint(&self, id: Option<Id>, end: u64, after: Observation) {
        self.finish_at(id, end, Outcome::Completed, Some(after.clone()), None);
        let parent = {
            let buffer = self.buffer.lock().unwrap();
            id.and_then(|id| buffer.rows.get(id as usize))
                .and_then(|row| row.parent_id)
                .filter(|parent| buffer.rows[*parent as usize].scope == Scope::NativeProof)
        };
        self.finish_at(parent, end, Outcome::Completed, Some(after), None);
    }

    pub fn observe_settled(&self, observation: Observation) {
        let mut buffer = self.buffer.lock().unwrap();
        let mut checkpoint = None;
        for row in &mut buffer.rows {
            if !row.needs_settled
                || row.settled_observed_ns.is_some()
                || row.drained_ns.is_none()
                || matches!(row.outcome, Outcome::Error | Outcome::Superseded)
            {
                continue;
            }
            if row
                .expected_generation
                .is_some_and(|expected| observation.generation > expected)
            {
                row.outcome = Outcome::Superseded;
            } else if row
                .expected_generation
                .is_some_and(|expected| observation.ready(expected))
            {
                let now = *checkpoint.get_or_insert_with(|| self.now());
                row.settled_observed_ns = Some(now);
                row.settled_observation = Some(observation.clone());
                if row.scope == Scope::ConstructionToSettled {
                    row.end_ns = Some(now);
                    row.elapsed_ns = Some(now.saturating_sub(row.start_ns));
                    row.outcome = Outcome::Completed;
                    row.after = Some(observation.clone());
                    row.observed_generation = Some(observation.generation);
                }
            }
        }
    }

    pub fn loader_probe(&self, parent_id: Option<Id>) -> LoaderProbe {
        if let Err(error) = self.prepare_loader_request(parent_id) {
            self.deny_loader(error);
        }
        LoaderProbe {
            recorder: self.clone(),
            parent_id,
        }
    }

    fn deny_loader(&self, error: &'static str) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.loader_failure.get_or_insert(error);
        }
    }

    fn prepare_loader_request(&self, parent_id: Option<Id>) -> LoaderResult<()> {
        let mut buffer = self.buffer.lock().map_err(|_| "loader recorder poisoned")?;
        let parent_id = parent_id.ok_or("loader request parent missing")?;
        let parent = buffer.rows.iter_mut().find(|row| row.id == parent_id)
            .ok_or("loader request parent missing")?;
        if parent.scope != Scope::ConstructionToSettled || parent.operation != Operation::FreshLoad {
            return Err("loader request is not initial construction");
        }
        if parent.loader_request_id.is_some_and(|request| request != parent_id) {
            return Err("loader request identity changed");
        }
        parent.loader_request_id = Some(parent_id);
        if parent.request_generation.is_none() && parent.expected_generation.is_none() {
            parent.request_generation = Some(0);
            parent.expected_generation = Some(0);
        }
        if parent.request_generation.is_none() || parent.expected_generation.is_none() {
            return Err("loader request generation missing");
        }
        Ok(())
    }

    fn try_loader_now(&self) -> LoaderResult<u64> {
        {
            let buffer = self.buffer.lock().map_err(|_| "loader recorder poisoned")?;
            if let Some(error) = buffer.loader_failure {
                return Err(error);
            }
        }
        let sampled = match &self.loader_clock {
            Some(clock) => match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| clock())) {
                Ok(sampled) => sampled,
                Err(payload) => {
                    std::mem::forget(payload);
                    Err("loader diagnostic clock panicked")
                }
            },
            None => u64::try_from(self.epoch.elapsed().as_nanos())
                .map_err(|_| "loader timestamp overflow"),
        };
        if let Err(error) = sampled {
            self.deny_loader(error);
        }
        sampled
    }

    fn start_loader(&self, parent_id: Option<Id>, start: u64) -> LoaderResult<Id> {
        let mut buffer = self.buffer.lock().map_err(|_| "loader recorder poisoned")?;
        if let Some(error) = buffer.loader_failure {
            return Err(error);
        }
        let parent_id = parent_id.ok_or("loader request parent missing")?;
        let parent = buffer.rows.iter().find(|row| row.id == parent_id)
            .ok_or("loader request parent missing")?;
        if parent.loader_request_id != Some(parent_id)
            || parent.request_generation.is_none() || parent.expected_generation.is_none()
        {
            return Err("loader request identity unavailable");
        }
        let request_generation = parent.request_generation;
        let expected_generation = parent.expected_generation;
        let observed_generation = parent.observed_generation;
        if buffer.rows.len() >= buffer.capacity {
            buffer.truncated = true;
            return Err("loader recorder capacity exhausted");
        }
        let id = Id::try_from(buffer.rows.len()).map_err(|_| "loader id overflow")?;
        buffer.rows.push(Row {
            id,
            parent_id: Some(parent_id),
            scope: Scope::LoaderExecution,
            operation: Operation::FreshLoad,
            role: Role::PlayerPath,
            start_ns: start,
            end_ns: None,
            elapsed_ns: None,
            shell_end_ns: None,
            drive_return_ns: None,
            drained_ns: None,
            settled_observed_ns: None,
            request_generation,
            expected_generation,
            observed_generation,
            loader_request_id: Some(parent_id),
            before: None,
            after: None,
            settled_observation: None,
            deltas: None,
            outcome: Outcome::Censored,
            error: None,
            needs_settled: false,
            loader: None,
        });
        Ok(id)
    }

    fn finish_loader(
        &self,
        id: Id,
        end: u64,
        succeeded: bool,
        payload: Option<LoaderPayload>,
    ) -> LoaderResult<()> {
        let mut buffer = self.buffer.lock().map_err(|_| "loader recorder poisoned")?;
        let row = buffer.rows.iter_mut().find(|row| row.id == id)
            .ok_or("loader aggregate missing")?;
        if row.scope != Scope::LoaderExecution || row.end_ns.is_some() {
            return Err("loader aggregate invalid or already closed");
        }
        let elapsed = end.checked_sub(row.start_ns).ok_or("loader aggregate time reversed")?;
        row.end_ns = Some(end);
        row.elapsed_ns = Some(elapsed);
        row.outcome = if succeeded { Outcome::Completed } else { Outcome::Error };
        row.error = (!succeeded).then(|| "Ready::load returned an error; see actual_loaded_effect".to_owned());
        row.loader = payload;
        let row = buffer.rows.iter().find(|row| row.id == id).ok_or("loader aggregate missing")?;
        let parent = buffer.rows.iter().find(|parent| Some(parent.id) == row.parent_id)
            .ok_or("loader request parent missing")?;
        if let Some(payload) = &row.loader {
            payload.view(row, parent)?;
        }
        Ok(())
    }

    fn interrupt_loader(&self, id: Id, stop: u64, payload: LoaderPayload) -> LoaderResult<()> {
        let mut buffer = self.buffer.lock().map_err(|_| "loader recorder poisoned")?;
        let row = buffer.rows.iter_mut().find(|row| row.id == id)
            .ok_or("interrupted loader aggregate missing")?;
        if row.scope != Scope::LoaderExecution || row.end_ns.is_some() || stop < row.start_ns {
            return Err("interrupted loader aggregate invalid");
        }
        if payload.len > LOADER_STAGE_CAPACITY
            || payload.spans[..payload.len].iter().any(|span| span.start_ns > stop || span.end_ns.is_some_and(|end| end > stop))
        {
            return Err("interrupted loader prefix outside observed stop");
        }
        row.outcome = Outcome::Censored;
        row.loader = Some(payload);
        let row = buffer.rows.iter().find(|row| row.id == id).ok_or("interrupted loader aggregate missing")?;
        let parent = buffer.rows.iter().find(|parent| Some(parent.id) == row.parent_id)
            .ok_or("interrupted loader request parent missing")?;
        if let Some(payload) = &row.loader {
            payload.view(row, parent)?;
        }
        Ok(())
    }

    pub fn loaded_effect(&self, generation: u64, current: u64) {
        let mut buffer = self.buffer.lock().unwrap();
        for row in &mut buffer.rows {
            if row.loader_request_id.is_some() && row.observed_generation.is_none() {
                row.observed_generation = Some(current);
                if generation != current || row.expected_generation != Some(generation) {
                    row.outcome = Outcome::Superseded;
                }
            }
        }
    }

    pub fn artifact(&self, journey_completed: bool) -> serde_json::Value {
        self.try_final_artifact(journey_completed, true).unwrap()
    }

    pub fn final_artifact(
        &self,
        journey_completed: bool,
        terminal_complete: bool,
    ) -> serde_json::Value {
        self.artifact(journey_completed && terminal_complete)
    }

    pub fn try_final_artifact(
        &self,
        journey_completed: bool,
        terminal_complete: bool,
    ) -> anyhow::Result<serde_json::Value> {
        let buffer = self.buffer.lock().map_err(|_| anyhow::anyhow!("timing recorder poisoned"))?;
        if let Some(error) = buffer.loader_failure {
            anyhow::bail!("loader timing recording failed: {error}");
        }
        let mut rows = serde_json::to_value(&buffer.rows)?;
        let values = rows.as_array_mut().ok_or_else(|| anyhow::anyhow!("timing rows are not an array"))?;
        let mut complete = journey_completed && terminal_complete && !buffer.truncated && !buffer.rows.is_empty();
        for (row, value) in buffer.rows.iter().zip(values) {
            let object = value.as_object_mut().ok_or_else(|| anyhow::anyhow!("timing row is not an object"))?;
            if row.outcome == Outcome::Censored
                || (row.outcome == Outcome::Completed && row.needs_settled && row.settled_observed_ns.is_none())
            {
                object.insert("outcome".to_owned(), serde_json::to_value(Outcome::Censored)?);
                complete = false;
            } else if row.outcome != Outcome::Completed {
                complete = false;
            }
            if let Some(payload) = &row.loader {
                let parent = buffer.rows.iter().find(|parent| Some(parent.id) == row.parent_id)
                    .ok_or_else(|| anyhow::anyhow!("loader request parent missing during export"))?;
                let view = payload.view(row, parent).map_err(|error| anyhow::anyhow!(error))?;
                complete &= view.complete;
                object.insert("loader".to_owned(), serde_json::to_value(view)?);
            }
        }
        #[derive(Serialize)]
        struct Artifact {
            schema: &'static str,
            clock: &'static str,
            capacity: usize,
            truncated: bool,
            complete: bool,
            cold_process_startup: &'static str,
            engine_only: &'static str,
            notification_only: &'static str,
            pure_io: &'static str,
            rows: serde_json::Value,
        }
        Ok(serde_json::to_value(Artifact {
            schema: "sims-host-journey-timing-v1",
            clock: "process-local-std-instant",
            capacity: buffer.capacity,
            truncated: buffer.truncated,
            complete,
            cold_process_startup: "unavailable",
            engine_only: "unavailable",
            notification_only: "unavailable",
            pure_io: "unavailable",
            rows,
        })?)
    }
}

pub(super) struct LoaderProbe {
    recorder: Recorder,
    parent_id: Option<Id>,
}

impl LoaderProbe {
    pub fn measure<Loaded, Error>(
        self,
        load: impl FnOnce() -> Result<Loaded, Error>,
    ) -> Result<Loaded, Error> {
        self.measure_inner(false, |_| load())
    }

    pub fn measure_staged<Loaded, Error>(
        self,
        load: impl FnOnce(&mut LoaderStages) -> Result<Loaded, Error>,
    ) -> Result<Loaded, Error> {
        self.measure_inner(true, load)
    }

    fn measure_inner<Loaded, Error>(
        self,
        collect: bool,
        load: impl FnOnce(&mut LoaderStages) -> Result<Loaded, Error>,
    ) -> Result<Loaded, Error> {
        let started = self.recorder.try_loader_now()
            .and_then(|start| self.recorder.start_loader(self.parent_id, start));
        let mut stages = LoaderStages::new(self.recorder.clone());
        stages.aggregate_id = started.ok();
        stages.publish_on_drop = collect;
        if let Err(error) = started {
            self.recorder.deny_loader(error);
            stages.failure = Some(error);
        }
        let result = load(&mut stages);
        stages.publish_on_drop = false;
        let end = match stages.failure {
            Some(error) => {
                self.recorder.deny_loader(error);
                Err(error)
            }
            None => self.recorder.try_loader_now(),
        };
        match (started, end) {
            (Ok(id), Ok(end)) => {
                let payload = collect.then_some(LoaderPayload {
                    spans: stages.spans,
                    len: stages.len,
                    finished: stages.finished,
                });
                if let Err(error) = self.recorder.finish_loader(id, end, result.is_ok(), payload) {
                    self.recorder.deny_loader(error);
                }
            }
            (_, Err(error)) | (Err(error), _) => self.recorder.deny_loader(error),
        }
        result
    }
}

type LoaderResult<Reply> = Result<Reply, &'static str>;
const LOADER_STAGE_CAPACITY: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum LoaderPhase {
    IndexScanRoute,
    SessionLoad,
    CameraOpenCompletion,
    NeighborhoodBinding,
    DialogsResidentOpening,
    ClockTransportBuyPreview,
    IconsReadyWrapCleanup,
}

impl LoaderPhase {
    const ALL: [Self; LOADER_STAGE_CAPACITY] = [
        Self::IndexScanRoute,
        Self::SessionLoad,
        Self::CameraOpenCompletion,
        Self::NeighborhoodBinding,
        Self::DialogsResidentOpening,
        Self::ClockTransportBuyPreview,
        Self::IconsReadyWrapCleanup,
    ];

    fn follows(self, previous: Self) -> bool {
        matches!((previous, self),
            (Self::IndexScanRoute, Self::SessionLoad | Self::IconsReadyWrapCleanup)
                | (Self::SessionLoad, Self::CameraOpenCompletion)
                | (Self::CameraOpenCompletion, Self::NeighborhoodBinding)
                | (Self::NeighborhoodBinding, Self::DialogsResidentOpening)
                | (Self::DialogsResidentOpening, Self::ClockTransportBuyPreview)
                | (Self::ClockTransportBuyPreview, Self::IconsReadyWrapCleanup))
    }
}

#[derive(Clone, Copy, Serialize)]
struct LoaderSpan {
    phase_id: LoaderPhase,
    start_ns: u64,
    end_ns: Option<u64>,
    elapsed_ns: Option<u64>,
    outcome: Outcome,
}

impl LoaderSpan {
    const EMPTY: Self = Self {
        phase_id: LoaderPhase::IndexScanRoute,
        start_ns: 0,
        end_ns: None,
        elapsed_ns: None,
        outcome: Outcome::Censored,
    };
}

pub(super) struct LoaderStages {
    recorder: Recorder,
    spans: [LoaderSpan; LOADER_STAGE_CAPACITY],
    len: usize,
    finished: bool,
    failure: Option<&'static str>,
    aggregate_id: Option<Id>,
    publish_on_drop: bool,
}

impl LoaderStages {
    fn new(recorder: Recorder) -> Self {
        Self {
            recorder,
            spans: [LoaderSpan::EMPTY; LOADER_STAGE_CAPACITY],
            len: 0,
            finished: false,
            failure: None,
            aggregate_id: None,
            publish_on_drop: false,
        }
    }

    pub fn enter_optional(collector: &mut Option<&mut Self>, phase: LoaderPhase) {
        if let Some(collector) = collector.as_deref_mut() {
            collector.enter(phase);
        }
    }

    fn enter(&mut self, phase: LoaderPhase) {
        if self.failure.is_some() {
            return;
        }
        let entered = (|| {
            if self.finished || self.len >= LOADER_STAGE_CAPACITY {
                return Err("loader local capacity exhausted or already finished");
            }
            if self.len == 0 {
                if phase != LoaderPhase::IndexScanRoute {
                    return Err("loader first phase is not scan");
                }
            } else if !phase.follows(self.spans[self.len - 1].phase_id) {
                return Err("loader phase order invalid");
            }
            let boundary = self.recorder.try_loader_now()?;
            if self.len != 0 {
                let previous = &mut self.spans[self.len - 1];
                let elapsed = boundary.checked_sub(previous.start_ns).ok_or("loader phase time reversed")?;
                previous.end_ns = Some(boundary);
                previous.elapsed_ns = Some(elapsed);
                previous.outcome = Outcome::Completed;
            }
            self.spans[self.len] = LoaderSpan {
                phase_id: phase,
                start_ns: boundary,
                ..LoaderSpan::EMPTY
            };
            self.len += 1;
            Ok(())
        })();
        if let Err(error) = entered {
            self.failure = Some(error);
            self.recorder.deny_loader(error);
        }
    }

    pub fn finish(&mut self, succeeded: bool) {
        if self.failure.is_some() {
            return;
        }
        let closed = (|| {
            if self.finished || self.len == 0 || self.len > LOADER_STAGE_CAPACITY {
                return Err("loader final phase missing or already closed");
            }
            let last = &mut self.spans[self.len - 1];
            if succeeded && last.phase_id != LoaderPhase::IconsReadyWrapCleanup {
                return Err("loader success lacks final wrapping phase");
            }
            let end = self.recorder.try_loader_now()?;
            last.elapsed_ns = Some(end.checked_sub(last.start_ns).ok_or("loader final time reversed")?);
            last.end_ns = Some(end);
            last.outcome = if succeeded { Outcome::Completed } else { Outcome::Error };
            self.finished = true;
            Ok(())
        })();
        if let Err(error) = closed {
            self.failure = Some(error);
            self.recorder.deny_loader(error);
        }
    }
}

impl Drop for LoaderStages {
    fn drop(&mut self) {
        if !std::mem::replace(&mut self.publish_on_drop, false) {
            return;
        }
        if let Some(error) = self.failure {
            self.recorder.deny_loader(error);
            return;
        }
        let Some(id) = self.aggregate_id else {
            return;
        };
        let stopped = self.recorder.try_loader_now();
        let payload = LoaderPayload {
            spans: self.spans,
            len: self.len,
            finished: self.finished,
        };
        if let Err(error) = stopped.and_then(|stop| self.recorder.interrupt_loader(id, stop, payload)) {
            self.recorder.deny_loader(error);
        }
    }
}

struct LoaderPayload {
    spans: [LoaderSpan; LOADER_STAGE_CAPACITY],
    len: usize,
    finished: bool,
}

#[derive(Serialize)]
struct LoaderView<'payload> {
    schema: &'static str,
    aggregate_parent_id: Id,
    request_id: Id,
    request_generation: Option<u64>,
    expected_generation: Option<u64>,
    observed_generation: Option<u64>,
    outcome: Outcome,
    complete: bool,
    stage_total_ns: u64,
    entry_residual_ns: Option<u64>,
    exit_residual_ns: Option<u64>,
    phases: &'payload [LoaderSpan],
}

fn checked_stage_total(spans: &[LoaderSpan]) -> LoaderResult<u64> {
    spans.iter().try_fold(0_u64, |total, span| {
        total.checked_add(span.elapsed_ns.unwrap_or(0)).ok_or("loader phase total overflow")
    })
}

impl LoaderPayload {
    fn view<'payload>(&'payload self, row: &Row, parent: &Row) -> LoaderResult<LoaderView<'payload>> {
        if self.len == 0 || self.len > LOADER_STAGE_CAPACITY {
            return Err("loader phase capacity or prefix missing");
        }
        if row.scope != Scope::LoaderExecution || row.parent_id != Some(parent.id)
            || row.loader_request_id != Some(parent.id) || parent.loader_request_id != Some(parent.id)
            || row.request_generation != parent.request_generation
            || row.expected_generation != parent.expected_generation
            || row.observed_generation != parent.observed_generation
            || row.request_generation.is_none() || row.expected_generation.is_none()
        {
            return Err("loader parent identity or generation mismatch");
        }
        let phases = &self.spans[..self.len];
        if phases[0].phase_id != LoaderPhase::IndexScanRoute {
            return Err("loader prefix lacks scan");
        }
        if let Some(end) = row.end_ns {
            if row.elapsed_ns != Some(end.checked_sub(row.start_ns).ok_or("loader aggregate time reversed")?) {
                return Err("loader aggregate arithmetic invalid");
            }
        } else if row.elapsed_ns.is_some() {
            return Err("loader censored aggregate has elapsed endpoint");
        }
        for (position, span) in phases.iter().enumerate() {
            if span.start_ns < row.start_ns || row.end_ns.is_some_and(|end| span.start_ns > end) {
                return Err("loader phase outside aggregate");
            }
            if position != 0 {
                let previous = &phases[position - 1];
                if !span.phase_id.follows(previous.phase_id) || previous.end_ns != Some(span.start_ns)
                    || previous.outcome != Outcome::Completed
                {
                    return Err("loader phases not ordered disjoint siblings");
                }
            }
            match span.end_ns {
                Some(end) => {
                    if row.end_ns.is_some_and(|aggregate_end| end > aggregate_end)
                        || span.elapsed_ns != Some(end.checked_sub(span.start_ns).ok_or("loader phase time reversed")?)
                        || !matches!(span.outcome, Outcome::Completed | Outcome::Error)
                    {
                        return Err("loader phase endpoint invalid");
                    }
                }
                None => {
                    if position + 1 != phases.len() || span.elapsed_ns.is_some() || span.outcome != Outcome::Censored {
                        return Err("loader unclosed phase is not censored tail");
                    }
                }
            }
        }
        let first = &phases[0];
        let last = &phases[self.len - 1];
        if self.finished != last.end_ns.is_some() {
            return Err("loader finish marker inconsistent");
        }
        if row.outcome == Outcome::Completed && self.finished
            && (last.phase_id != LoaderPhase::IconsReadyWrapCleanup || !matches!(self.len, 2 | 7)
                || last.outcome != Outcome::Completed)
        {
            return Err("loader complete route missing final wrapping");
        }
        if row.outcome == Outcome::Error && self.finished && last.outcome != Outcome::Error {
            return Err("loader error lacks failed active phase");
        }
        Ok(LoaderView {
            schema: "sims-ready-loader-phases-v1",
            aggregate_parent_id: row.id,
            request_id: parent.id,
            request_generation: parent.request_generation,
            expected_generation: parent.expected_generation,
            observed_generation: parent.observed_generation,
            outcome: if self.finished { row.outcome } else { Outcome::Censored },
            complete: self.finished && row.outcome == Outcome::Completed && row.end_ns.is_some(),
            stage_total_ns: checked_stage_total(phases)?,
            entry_residual_ns: Some(first.start_ns.checked_sub(row.start_ns).ok_or("loader entry residual invalid")?),
            exit_residual_ns: row.end_ns.zip(last.end_ns)
                .map(|(end, last_end)| end.checked_sub(last_end).ok_or("loader exit residual invalid")).transpose()?,
            phases,
        })
    }
}

#[path = "player_timing/tests.rs"]
mod tests;
