//! Dedicated background save & rotating autosave worker thread backed by
//! REDB [`sims_sim::AutosaveStore`], plus [`sims_ui::saves::SaveManagerSnapshot`]
//! projection helpers.
//!
//! Main-thread frames stay hitch-free: callers clone a cheap CoW [`World`] or
//! [`Saved`] snapshot on the UI thread and enqueue it onto [`AutosaveWorker`].
//! Serialization (`World::encode` / `Saved::encode`), payload validation,
//! SHA-256 checksumming, and atomic REDB commits all run off-thread.

use sims_net::manifest::sha256_bytes;
use sims_sim::World;
use sims_sim::autosave::{
    AutosaveStore, SaveError, SaveJob, SaveMetadata, SavePayload, SavePayloadKind, SaveSlot,
    SaveSlotEntry, SaveTarget,
};
use sims_ui::saves::{SaveHealth, SaveManagerSnapshot, SaveSlotKind, SaveSlotSummary};
use sims_vm::boot::save::Saved;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

const VALIDATION_CACHE_MAX: usize = 64;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ValidationKind {
    SavedLot,
    WorldSnapshot,
    Custom(String),
}

impl From<&SavePayloadKind> for ValidationKind {
    fn from(kind: &SavePayloadKind) -> Self {
        match kind {
            SavePayloadKind::SavedLot => Self::SavedLot,
            SavePayloadKind::WorldSnapshot => Self::WorldSnapshot,
            SavePayloadKind::Custom(tag) => Self::Custom(tag.clone()),
        }
    }
}

#[derive(Default)]
struct ValidationCache {
    order: VecDeque<(ValidationKind, [u8; 32])>,
    results: BTreeMap<(ValidationKind, [u8; 32]), Result<(), String>>,
}

impl ValidationCache {
    fn get(&self, key: &(ValidationKind, [u8; 32])) -> Option<Result<(), String>> {
        self.results.get(key).cloned()
    }

    fn insert(&mut self, key: (ValidationKind, [u8; 32]), value: Result<(), String>) {
        if let Some(existing) = self.results.get_mut(&key) {
            *existing = value;
            return;
        }
        while self.order.len() >= VALIDATION_CACHE_MAX {
            if let Some(oldest) = self.order.pop_front() {
                self.results.remove(&oldest);
            }
        }
        self.order.push_back(key.clone());
        self.results.insert(key, value);
    }
}

fn validation_cache() -> &'static Mutex<ValidationCache> {
    static CACHE: OnceLock<Mutex<ValidationCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(ValidationCache::default()))
}

/// Records that an already-hashed `Saved::encode()` payload produced inside the
/// worker thread is valid so subsequent lookups hit the validation cache in O(1)
/// without hashing the incoming payload twice.
fn note_validated_saved_lot_sha256(sha256: [u8; 32]) {
    let key = (ValidationKind::SavedLot, sha256);
    let mut guard = validation_cache().lock().unwrap_or_else(|e| e.into_inner());
    guard.insert(key, Ok(()));
}

/// Result of one background save or autosave job executed by [`AutosaveWorker`].
#[derive(Debug, Clone)]
pub struct SaveCompletion {
    pub neighborhood_id: String,
    pub lot_id: u32,
    pub target: SaveTarget,
    pub sim_tick: u64,
    pub result: Result<SaveMetadata, SaveError>,
}

enum WorkerMessage {
    Job(SaveJob),
    SavedLot {
        neighborhood_id: String,
        expected_instance_id: u128,
        lot_id: u32,
        target: SaveTarget,
        sim_tick: u64,
        saved_at_unix_secs: Option<u64>,
        label: String,
        saved: Box<Saved>,
    },
    Flush(Sender<()>),
    Shutdown,
}

/// Validates autosave/manual-save payload bytes in full (decoding `Saved` or
/// `World`) for slot loads, external imports, and corruption recovery.
pub fn validate_save_payload(kind: &SavePayloadKind, bytes: &[u8]) -> Result<(), String> {
    match kind {
        SavePayloadKind::SavedLot => Saved::decode(bytes)
            .map(|_| ())
            .map_err(|e| format!("invalid Saved lot payload: {e}")),
        SavePayloadKind::WorldSnapshot => World::decode(bytes)
            .map(|_| ())
            .map_err(|e| format!("invalid World snapshot payload: {e}")),
        SavePayloadKind::Custom(_) => Ok(()),
    }
}

/// Full payload validator backed by a bounded `(SavePayloadKind, SHA-256)`
/// cache using an already-computed `sha256` digest so `AutosaveStore`
/// commits and loads avoid hashing immutable payloads a second time while
/// agreeing 100% with [`validate_save_payload`].
pub fn validate_save_payload_cached_with_sha256(
    kind: &SavePayloadKind,
    sha256: &[u8; 32],
    bytes: &[u8],
) -> Result<(), String> {
    let key = (ValidationKind::from(kind), *sha256);
    {
        let guard = validation_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = guard.get(&key) {
            return cached;
        }
    }
    let result = validate_save_payload(kind, bytes);
    {
        let mut guard = validation_cache().lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(key, result.clone());
    }
    result
}

/// Full payload validator backed by a bounded `(SavePayloadKind, SHA-256)`
/// cache so Save Manager listings, overwrite/pruning safeguards, and
/// self-encoded commits agree 100% with [`validate_save_payload`] without
/// repeatedly re-decoding identical immutable payloads.
pub fn validate_save_payload_cached(kind: &SavePayloadKind, bytes: &[u8]) -> Result<(), String> {
    let sha256 = sha256_bytes(bytes);
    validate_save_payload_cached_with_sha256(kind, &sha256, bytes)
}

/// Fast header-only check verifying the 8-byte `Saved` format and
/// `SNAPSHOT_FORMAT` header. Callers requiring full body integrity (such as
/// Save Manager listing, commit safeguards, and loads) should use
/// [`validate_save_payload_cached`] or [`validate_save_payload`].
pub fn validate_save_payload_header(kind: &SavePayloadKind, bytes: &[u8]) -> Result<(), String> {
    match kind {
        SavePayloadKind::SavedLot => {
            if bytes.len() <= 8 {
                return Err(format!(
                    "invalid Saved lot payload: shorter than 8-byte header ({} bytes)",
                    bytes.len()
                ));
            }
            let format = u32::from_le_bytes(bytes[0..4].try_into().expect("4 bytes"));
            if !(sims_vm::boot::save::MIN_FORMAT..=sims_vm::boot::save::FORMAT).contains(&format) {
                return Err(format!(
                    "invalid Saved lot payload: unsupported save format {format} (expected {}..={})",
                    sims_vm::boot::save::MIN_FORMAT,
                    sims_vm::boot::save::FORMAT
                ));
            }
            let world_format = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes"));
            if world_format != sims_sim::SNAPSHOT_FORMAT {
                return Err(format!(
                    "invalid Saved lot payload: snapshot format {world_format} (expected {})",
                    sims_sim::SNAPSHOT_FORMAT
                ));
            }
            Ok(())
        }
        // `AutosaveStore` already runs built-in envelope + `World::decode` checks
        // for `WorldSnapshot`, so avoid decoding `World` a second time here.
        SavePayloadKind::WorldSnapshot | SavePayloadKind::Custom(_) => Ok(()),
    }
}

/// Returns the current Unix epoch timestamp in seconds, if the system clock is
/// after 1970-01-01.
pub fn unix_timestamp_secs() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Formats a stable, human-readable label carrying simulation day, hour,
/// minute, and optional household name so [`build_save_manager_snapshot`] can
/// recover exact [`SaveSlotSummary`] fields from REDB metadata.
pub fn format_save_label(
    household_name: Option<&str>,
    sim_day: u32,
    sim_hour: u8,
    sim_minute: u8,
) -> String {
    let day = sim_day.max(1);
    let hour = sim_hour.min(23);
    let minute = sim_minute.min(59);
    match household_name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => format!("Day {day} {hour:02}:{minute:02} — {name}"),
        None => format!("Day {day} {hour:02}:{minute:02}"),
    }
}

/// Extracts `(sim_day, sim_hour, sim_minute, household_name)` from a label
/// formatted by [`format_save_label`], falling back to `(1, 0, 0, Some(label))`
/// for arbitrary caller-supplied labels.
pub fn parse_save_label(label: &str) -> (u32, u8, u8, Option<String>) {
    let trimmed = label.trim();
    if let Some(rest) = trimmed.strip_prefix("Day ") {
        let mut parts = rest.splitn(3, ' ');
        if let (Some(day_str), Some(time_str)) = (parts.next(), parts.next())
            && let Ok(day) = day_str.parse::<u32>()
            && let Some((h_str, m_str)) = time_str.split_once(':')
            && let (Ok(h), Ok(m)) = (h_str.parse::<u8>(), m_str.parse::<u8>())
            && h < 24
            && m < 60
        {
            let household = parts
                .next()
                .map(str::trim)
                .and_then(|tail| {
                    tail.strip_prefix('—')
                        .or_else(|| tail.strip_prefix('-'))
                        .map(str::trim)
                        .or(Some(tail))
                })
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            return (day.max(1), h, m, household);
        }
    }
    let household = (!trimmed.is_empty()).then(|| trimmed.to_owned());
    (1, 0, 0, household)
}

/// Encodes a [`SaveSlot`] into the opaque `slot_id` string used by
/// [`sims_ui::saves::SaveManagerDialog`].
pub fn slot_id_for(slot: SaveSlot) -> String {
    match slot {
        SaveSlot::Manual => "manual".to_owned(),
        SaveSlot::Autosave(seq) => format!("autosave:{seq}"),
    }
}

/// Parses an opaque `slot_id` emitted by [`sims_ui::saves::SaveManagerCommand`]
/// back into a [`SaveSlot`].
pub fn parse_slot_id(slot_id: &str) -> Option<SaveSlot> {
    if slot_id == "manual" {
        return Some(SaveSlot::Manual);
    }
    slot_id
        .strip_prefix("autosave:")
        .and_then(|s| s.parse::<u64>().ok())
        .map(SaveSlot::Autosave)
}

/// Projects all REDB manual and rotating autosave slots for
/// `(neighborhood_id, lot_id)` into a read-only [`SaveManagerSnapshot`] for
/// [`sims_ui::saves::SaveManagerDialog`].
///
/// Uses [`AutosaveStore::list_saves_metadata`] so listing verifies each slot's
/// outer SHA-256 envelope, format version, and key-bound metadata header
/// without decompressing V2 payloads or deserializing every slot's full
/// `World`/`Saved` on the UI thread. Full [`Saved::decode`] / [`World::decode`]
/// validation still runs whenever a slot is actually loaded or committed.
pub fn build_save_manager_snapshot(
    store: &AutosaveStore,
    neighborhood_id: &str,
    neighborhood_slot: u8,
    lot_id: u32,
    lot_label: impl Into<String>,
    active_error: Option<String>,
) -> SaveManagerSnapshot {
    let lot_u16 = u16::try_from(lot_id).unwrap_or(u16::MAX);
    let lot_label = lot_label.into();
    let entries = match store.list_saves_metadata(neighborhood_id, lot_id) {
        Ok(entries) => entries,
        Err(err) => {
            let merged_err = match active_error {
                Some(prev) if !prev.trim().is_empty() => format!("{prev} ({err})"),
                _ => err.to_string(),
            };
            return SaveManagerSnapshot {
                neighborhood: neighborhood_slot,
                lot: lot_u16,
                lot_label,
                slots: Vec::new(),
                active_error: Some(merged_err),
            };
        }
    };

    let mut slots = Vec::with_capacity(entries.len());
    for entry in entries {
        match entry {
            SaveSlotEntry::Intact(meta) => {
                let (sim_day, sim_hour, sim_minute, household_name) = parse_save_label(&meta.label);
                let (kind, label) = match meta.slot {
                    SaveSlot::Manual => (SaveSlotKind::Manual, "Manual Save".to_owned()),
                    SaveSlot::Autosave(seq) => (
                        SaveSlotKind::Autosave {
                            rotation_index: u32::try_from(seq).unwrap_or(u32::MAX),
                        },
                        format!("Autosave #{seq}"),
                    ),
                };
                slots.push(SaveSlotSummary {
                    slot_id: slot_id_for(meta.slot),
                    neighborhood: neighborhood_slot,
                    lot: lot_u16,
                    label,
                    kind,
                    sim_day,
                    sim_hour,
                    sim_minute,
                    tick: meta.sim_tick,
                    saved_at_epoch_secs: meta.saved_at_unix_secs,
                    household_name,
                    health: SaveHealth::Intact,
                });
            }
            SaveSlotEntry::Corrupt { slot, error } => {
                let (kind, label) = match slot {
                    SaveSlot::Manual => (SaveSlotKind::Manual, "Manual Save".to_owned()),
                    SaveSlot::Autosave(seq) => (
                        SaveSlotKind::Autosave {
                            rotation_index: u32::try_from(seq).unwrap_or(u32::MAX),
                        },
                        format!("Autosave #{seq}"),
                    ),
                };
                let health = match &error {
                    SaveError::UnsupportedVersion { found, .. } => SaveHealth::Unsupported {
                        schema_version: *found,
                        reason: error.to_string(),
                    },
                    _ => SaveHealth::Corrupt {
                        reason: error.to_string(),
                    },
                };
                slots.push(SaveSlotSummary {
                    slot_id: slot_id_for(slot),
                    neighborhood: neighborhood_slot,
                    lot: lot_u16,
                    label,
                    kind,
                    sim_day: 1,
                    sim_hour: 0,
                    sim_minute: 0,
                    tick: 0,
                    saved_at_epoch_secs: None,
                    household_name: None,
                    health,
                });
            }
        }
    }

    SaveManagerSnapshot {
        neighborhood: neighborhood_slot,
        lot: lot_u16,
        lot_label,
        slots,
        active_error,
    }
}

/// Dedicated background worker thread committing CoW world and lot snapshots
/// into REDB via [`AutosaveStore`] without stalling the main UI/render thread.
pub struct AutosaveWorker {
    tx: Option<Sender<WorkerMessage>>,
    rx: Receiver<SaveCompletion>,
    pending: Arc<AtomicUsize>,
    unread: Arc<AtomicUsize>,
    handle: Option<JoinHandle<()>>,
}

impl AutosaveWorker {
    /// Spawns a background worker thread owning `store`.
    pub fn spawn(store: AutosaveStore) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<WorkerMessage>();
        let (done_tx, done_rx) = mpsc::channel::<SaveCompletion>();
        let pending = Arc::new(AtomicUsize::new(0));
        let unread = Arc::new(AtomicUsize::new(0));
        let pending_worker = Arc::clone(&pending);
        let unread_worker = Arc::clone(&unread);

        let builder = thread::Builder::new().name("sims-autosave-worker".into());
        let handle = builder
            .spawn(move || {
                worker_loop(store, job_rx, done_tx, pending_worker, unread_worker);
            })
            .expect("failed to spawn sims-autosave-worker thread");

        Self {
            tx: Some(job_tx),
            rx: done_rx,
            pending,
            unread,
            handle: Some(handle),
        }
    }

    /// Enqueues an already-constructed [`SaveJob`] (such as a CoW
    /// `SavePayload::World(Box<World>)` snapshot with `expected_instance_id`
    /// fencing) for asynchronous REDB commit.
    pub fn enqueue_job(&self, job: SaveJob) -> Result<(), String> {
        let Some(tx) = &self.tx else {
            return Err("autosave worker is shut down".into());
        };
        self.pending.fetch_add(1, Ordering::AcqRel);
        if tx.send(WorkerMessage::Job(job)).is_err() {
            self.pending.fetch_sub(1, Ordering::AcqRel);
            return Err("autosave worker channel closed".into());
        }
        Ok(())
    }

    /// Enqueues a cheap CoW [`World`] snapshot (`SavePayload::World`) fenced to
    /// `expected_instance_id`.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_world(
        &self,
        neighborhood_id: impl Into<String>,
        expected_instance_id: u128,
        lot_id: u32,
        target: SaveTarget,
        sim_tick: u64,
        label: impl Into<String>,
        world: World,
    ) -> Result<(), String> {
        let payload = SavePayload::from_world(world);
        let mut job = match target {
            SaveTarget::Manual => SaveJob::manual_for_instance(
                neighborhood_id,
                expected_instance_id,
                lot_id,
                sim_tick,
                payload,
            ),
            SaveTarget::ManualForceReplace => SaveJob::manual_force_replace_for_instance(
                neighborhood_id,
                expected_instance_id,
                lot_id,
                sim_tick,
                payload,
            ),
            SaveTarget::Autosave => SaveJob::autosave_for_instance(
                neighborhood_id,
                expected_instance_id,
                lot_id,
                sim_tick,
                payload,
            ),
        }
        .with_label(label);
        if let Some(ts) = unix_timestamp_secs() {
            job = job.with_saved_at_unix_secs(ts);
        }
        self.enqueue_job(job)
    }

    /// Enqueues a cheap CoW [`Saved`] lot snapshot whose `Saved::encode()` runs
    /// off-thread on the worker before committing into REDB.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_saved_lot(
        &self,
        neighborhood_id: impl Into<String>,
        expected_instance_id: u128,
        lot_id: u32,
        target: SaveTarget,
        sim_tick: u64,
        label: impl Into<String>,
        saved: Saved,
    ) -> Result<(), String> {
        let Some(tx) = &self.tx else {
            return Err("autosave worker is shut down".into());
        };
        let msg = WorkerMessage::SavedLot {
            neighborhood_id: neighborhood_id.into(),
            expected_instance_id,
            lot_id,
            target,
            sim_tick,
            saved_at_unix_secs: unix_timestamp_secs(),
            label: label.into(),
            saved: Box::new(saved),
        };
        self.pending.fetch_add(1, Ordering::AcqRel);
        if tx.send(msg).is_err() {
            self.pending.fetch_sub(1, Ordering::AcqRel);
            return Err("autosave worker channel closed".into());
        }
        Ok(())
    }

    /// Enqueues already-encoded [`Saved`] lot bytes onto the background worker
    /// so the Save Manager manual slot records the exact same payload committed
    /// to `SAVED_LOTS`.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_saved_lot_bytes(
        &self,
        neighborhood_id: impl Into<String>,
        expected_instance_id: u128,
        lot_id: u32,
        target: SaveTarget,
        sim_tick: u64,
        label: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        let payload = SavePayload::saved_lot(bytes);
        let mut job = match target {
            SaveTarget::Manual => SaveJob::manual_for_instance(
                neighborhood_id,
                expected_instance_id,
                lot_id,
                sim_tick,
                payload,
            ),
            SaveTarget::ManualForceReplace => SaveJob::manual_force_replace_for_instance(
                neighborhood_id,
                expected_instance_id,
                lot_id,
                sim_tick,
                payload,
            ),
            SaveTarget::Autosave => SaveJob::autosave_for_instance(
                neighborhood_id,
                expected_instance_id,
                lot_id,
                sim_tick,
                payload,
            ),
        }
        .with_label(label);
        if let Some(ts) = unix_timestamp_secs() {
            job = job.with_saved_at_unix_secs(ts);
        }
        self.enqueue_job(job)
    }

    /// Non-blocking drain of all completed background save jobs.
    pub fn poll_completions(&self) -> Vec<SaveCompletion> {
        let mut out = Vec::new();
        while let Ok(item) = self.rx.try_recv() {
            self.unread.fetch_sub(1, Ordering::AcqRel);
            out.push(item);
        }
        out
    }

    /// Number of save jobs currently queued or actively committing on the
    /// worker thread.
    pub fn pending_count(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }

    /// Whether any background save job is either still in flight on the worker
    /// thread OR has published a completion not yet drained by
    /// [`AutosaveWorker::poll_completions`].
    pub fn has_pending_or_unread(&self) -> bool {
        self.pending.load(Ordering::Acquire) > 0 || self.unread.load(Ordering::Acquire) > 0
    }

    /// Blocks until every previously enqueued save job has finished committing
    /// in REDB (leaving the worker running for future jobs).
    pub fn flush(&self) {
        let Some(tx) = &self.tx else {
            return;
        };
        let (ack_tx, ack_rx) = mpsc::channel();
        if tx.send(WorkerMessage::Flush(ack_tx)).is_ok() {
            let _ = ack_rx.recv();
        }
    }
}

impl Drop for AutosaveWorker {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(WorkerMessage::Shutdown);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn worker_loop(
    store: AutosaveStore,
    rx: Receiver<WorkerMessage>,
    done_tx: Sender<SaveCompletion>,
    pending: Arc<AtomicUsize>,
    unread: Arc<AtomicUsize>,
) {
    let publish = |completion: SaveCompletion| {
        // Increment `unread` BEFORE sending and BEFORE decrementing `pending`
        // so `has_pending_or_unread()` can never transiently observe both as 0
        // while a completion is waiting for the UI thread to drain it.
        unread.fetch_add(1, Ordering::AcqRel);
        if done_tx.send(completion).is_err() {
            unread.fetch_sub(1, Ordering::AcqRel);
        }
        pending.fetch_sub(1, Ordering::AcqRel);
    };
    while let Ok(msg) = rx.recv() {
        match msg {
            WorkerMessage::Job(job) => {
                let neighborhood_id = job.neighborhood_id.clone();
                let lot_id = job.lot_id;
                let target = job.target;
                let sim_tick = job.sim_tick;
                let result = store.commit_job_validated_with_sha256(
                    job,
                    validate_save_payload_cached_with_sha256,
                );
                publish(SaveCompletion {
                    neighborhood_id,
                    lot_id,
                    target,
                    sim_tick,
                    result,
                });
            }
            WorkerMessage::SavedLot {
                neighborhood_id,
                expected_instance_id,
                lot_id,
                target,
                sim_tick,
                saved_at_unix_secs,
                label,
                saved,
            } => {
                // Encode off the UI/render thread so manual & periodic saves never hitch.
                let encoded = saved.encode();
                let incoming_len = encoded.len();
                let payload = SavePayload::saved_lot(encoded);
                let mut job = match target {
                    SaveTarget::Manual => SaveJob::manual_for_instance(
                        neighborhood_id.clone(),
                        expected_instance_id,
                        lot_id,
                        sim_tick,
                        payload,
                    ),
                    SaveTarget::ManualForceReplace => SaveJob::manual_force_replace_for_instance(
                        neighborhood_id.clone(),
                        expected_instance_id,
                        lot_id,
                        sim_tick,
                        payload,
                    ),
                    SaveTarget::Autosave => SaveJob::autosave_for_instance(
                        neighborhood_id.clone(),
                        expected_instance_id,
                        lot_id,
                        sim_tick,
                        payload,
                    ),
                }
                .with_label(label);
                if let Some(ts) = saved_at_unix_secs {
                    job = job.with_saved_at_unix_secs(ts);
                }
                // `AutosaveStore::commit_job_validated_with_sha256` computes
                // `payload_sha256` once and invokes the validator first on the
                // freshly self-encoded incoming payload; seed the cache with
                // that digest on the first call so we never hash twice, while
                // existing manual/ring slots still run full `Saved::decode` on
                // first sight so corrupt slots are never overwritten/pruned.
                let incoming_pending = std::sync::atomic::AtomicBool::new(true);
                let result = store.commit_job_validated_with_sha256(job, |kind, sha256, bytes| {
                    if matches!(kind, SavePayloadKind::SavedLot)
                        && bytes.len() == incoming_len
                        && incoming_pending.swap(false, std::sync::atomic::Ordering::Relaxed)
                    {
                        note_validated_saved_lot_sha256(*sha256);
                        return Ok(());
                    }
                    validate_save_payload_cached_with_sha256(kind, sha256, bytes)
                });
                publish(SaveCompletion {
                    neighborhood_id,
                    lot_id,
                    target,
                    sim_tick,
                    result,
                });
            }
            WorkerMessage::Flush(ack) => {
                let _ = ack.send(());
            }
            WorkerMessage::Shutdown => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_label_and_slot_id_round_trip() {
        let label = format_save_label(Some("Goth"), 4, 17, 45);
        assert_eq!(label, "Day 4 17:45 — Goth");
        assert_eq!(parse_save_label(&label), (4, 17, 45, Some("Goth".into())));

        let no_fam = format_save_label(None, 1, 8, 5);
        assert_eq!(no_fam, "Day 1 08:05");
        assert_eq!(parse_save_label(&no_fam), (1, 8, 5, None));

        assert_eq!(
            parse_slot_id(&slot_id_for(SaveSlot::Manual)),
            Some(SaveSlot::Manual)
        );
        assert_eq!(
            parse_slot_id(&slot_id_for(SaveSlot::Autosave(7))),
            Some(SaveSlot::Autosave(7))
        );
        assert_eq!(parse_slot_id("bogus"), None);
    }
}
