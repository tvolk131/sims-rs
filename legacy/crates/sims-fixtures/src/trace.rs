//! Bounded typed MessagePack chunks with explicit keyframes and lossless deltas.
//! A trace is evidence, never a resumable VM or a completed-tick qualification.
use crate::{BlobRef, Store, digest, model::Boundary};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const TRACE_INDEX_SCHEMA: u32 = 1;
pub const TRACE_SCHEMA: u32 = TRACE_INDEX_SCHEMA;
const CHUNK_SCHEMA: u32 = 1;
const HEADER_RESERVE: u64 = 1024;
const MAX_BLOCK_BYTES: u64 = 512 * 1024 * 1024;
const ZERO_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TraceOptions {
    pub max_records_per_chunk: usize,
    pub max_chunk_bytes: u64,
    pub max_record_bytes: u64,
    pub keyframe_interval: u64,
}
impl Default for TraceOptions {
    fn default() -> Self {
        Self {
            max_records_per_chunk: 64,
            max_chunk_bytes: 16 * 1024 * 1024,
            max_record_bytes: 8 * 1024 * 1024,
            keyframe_interval: 128,
        }
    }
}
impl TraceOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=65_536).contains(&self.max_records_per_chunk),
            "invalid trace chunk record limit"
        );
        ensure!(
            self.max_record_bytes > 0
                && self.max_chunk_bytes <= MAX_BLOCK_BYTES
                && self
                    .max_record_bytes
                    .checked_add(HEADER_RESERVE)
                    .is_some_and(|n| n <= self.max_chunk_bytes),
            "invalid trace byte limits"
        );
        ensure!(
            self.keyframe_interval > 0,
            "keyframe interval must be positive"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Observation,
    Event,
    Checkpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceRecord {
    pub line: u64,
    pub byte_offset: u64,
    pub byte_length: u64,
    pub actual_tick: Option<u64>,
    /// Explicit source data only. Missing/zero never becomes a simulated update.
    pub advanced: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TraceRecordMeta {
    pub kind: RecordKind,
    pub boundary: Boundary,
    pub source: Option<SourceRecord>,
}
impl TraceRecordMeta {
    pub fn validate(&self) -> Result<()> {
        self.boundary.validate()?;
        if let Some(source) = &self.source {
            ensure!(
                source.line > 0
                    && source.byte_length > 0
                    && source.byte_offset.checked_add(source.byte_length).is_some(),
                "invalid source record span"
            );
            let boundary_tick = match &self.boundary {
                Boundary::CompletedTick { tick } => Some(*tick),
                Boundary::Phase { tick, .. } | Boundary::Event { tick, .. } => *tick,
                Boundary::Unspecified => None,
            };
            if let (Some(a), Some(b)) = (boundary_tick, source.actual_tick) {
                ensure!(a == b, "boundary/source tick disagree");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Change {
    Set { path: Vec<String>, value: Value },
    Remove { path: Vec<String> },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Payload {
    Keyframe { value: Value },
    Delta { changes: Vec<Change> },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Record {
    ordinal: u64,
    meta: TraceRecordMeta,
    previous_digest: String,
    payload: Payload,
    digest: String,
}
#[derive(Serialize)]
struct RecordDigest<'a> {
    ordinal: u64,
    meta: &'a TraceRecordMeta,
    previous_digest: &'a str,
    payload: &'a Payload,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Chunk {
    schema: u32,
    first_record: u64,
    previous_chunk_sha256: Option<String>,
    records: Vec<Record>,
}
#[derive(Serialize)]
struct BorrowedChunk<'a> {
    schema: u32,
    first_record: u64,
    previous_chunk_sha256: &'a Option<String>,
    records: &'a [Record],
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TraceChunkRef {
    pub blob: BlobRef,
    pub first_record: u64,
    pub record_count: u64,
    pub previous_chunk_sha256: Option<String>,
    pub first_record_digest: String,
    pub last_record_digest: String,
    pub keyframe_count: u64,
    pub delta_count: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TraceIndex {
    pub schema: u32,
    pub options: TraceOptions,
    pub record_count: u64,
    pub keyframe_count: u64,
    pub delta_count: u64,
    pub observation_count: u64,
    pub event_count: u64,
    pub checkpoint_count: u64,
    pub final_record_digest: String,
    pub chunks: Vec<TraceChunkRef>,
}
impl TraceIndex {
    /// Every chunk is declared, including keyframe-only chunks, for GC closure.
    pub fn blob_refs(&self) -> Vec<BlobRef> {
        self.chunks.iter().map(|c| c.blob.clone()).collect()
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == TRACE_INDEX_SCHEMA,
            "unsupported trace index schema {}",
            self.schema
        );
        self.options.validate()?;
        valid_digest(&self.final_record_digest)?;
        ensure!(self.chunks.len() <= 1_000_000, "too many trace chunks");
        let mut next = 0u64;
        let (mut keys, mut deltas) = (0u64, 0u64);
        let mut previous = None;
        let mut last = ZERO_DIGEST.to_string();
        let mut identities = BTreeSet::new();
        for chunk in &self.chunks {
            chunk.blob.validate()?;
            ensure!(
                chunk.blob.bytes <= self.options.max_chunk_bytes,
                "chunk exceeds declared byte bound"
            );
            ensure!(
                chunk.first_record == next
                    && chunk.record_count > 0
                    && chunk.record_count <= self.options.max_records_per_chunk as u64,
                "missing/reordered trace chunk range"
            );
            ensure!(
                chunk.previous_chunk_sha256 == previous,
                "trace chunk chain differs"
            );
            ensure!(
                identities.insert(chunk.blob.sha256.clone()),
                "reused/reordered chunk identity"
            );
            valid_digest(&chunk.first_record_digest)?;
            valid_digest(&chunk.last_record_digest)?;
            ensure!(
                chunk.keyframe_count > 0
                    && chunk.keyframe_count.checked_add(chunk.delta_count)
                        == Some(chunk.record_count),
                "chunk payload counts differ"
            );
            next = next
                .checked_add(chunk.record_count)
                .context("trace count overflow")?;
            keys = keys
                .checked_add(chunk.keyframe_count)
                .context("keyframe count overflow")?;
            deltas = deltas
                .checked_add(chunk.delta_count)
                .context("delta count overflow")?;
            previous = Some(chunk.blob.sha256.clone());
            last = chunk.last_record_digest.clone();
        }
        ensure!(
            next == self.record_count
                && keys == self.keyframe_count
                && deltas == self.delta_count
                && last == self.final_record_digest,
            "trace totals/final digest differ"
        );
        ensure!(
            self.observation_count
                .checked_add(self.event_count)
                .and_then(|n| n.checked_add(self.checkpoint_count))
                == Some(next),
            "trace kind totals differ"
        );
        Ok(())
    }
    pub fn verify(&self, store: &Store) -> Result<()> {
        self.reconstruct(store, |_, _| Ok(()))
    }
    /// Emits only verified prefix records; an error means the whole trace did not
    /// verify. Callers needing atomic output should stage it before publication.
    pub fn reconstruct(
        &self,
        store: &Store,
        mut visit: impl FnMut(&TraceRecordMeta, &Value) -> Result<()>,
    ) -> Result<()> {
        self.validate()?;
        let mut current = None;
        let mut previous = ZERO_DIGEST.to_string();
        let mut ordinal = 0u64;
        let (mut keys, mut deltas, mut observations, mut events, mut checkpoints) =
            (0u64, 0u64, 0u64, 0u64, 0u64);
        for reference in &self.chunks {
            let chunk: Chunk = store.get_value(&reference.blob)?;
            ensure!(
                chunk.schema == CHUNK_SCHEMA,
                "unsupported trace chunk schema"
            );
            ensure!(
                chunk.first_record == reference.first_record
                    && chunk.previous_chunk_sha256 == reference.previous_chunk_sha256
                    && chunk.records.len() as u64 == reference.record_count,
                "chunk contents/index differ"
            );
            ensure!(
                matches!(
                    chunk.records.first().map(|r| &r.payload),
                    Some(Payload::Keyframe { .. })
                ),
                "chunk must start with explicit keyframe"
            );
            let (mut chunk_keys, mut chunk_deltas) = (0u64, 0u64);
            for (i, record) in chunk.records.iter().enumerate() {
                record.meta.validate()?;
                ensure!(
                    record.ordinal == ordinal && record.previous_digest == previous,
                    "record missing/reordered or previous digest differs"
                );
                ensure!(
                    record.digest == record_digest(record)?,
                    "record digest differs"
                );
                ensure!(
                    encoded_len(record)? <= self.options.max_record_bytes,
                    "record exceeds declared byte bound"
                );
                let require_keyframe = ordinal.is_multiple_of(self.options.keyframe_interval)
                    || record.meta.kind == RecordKind::Checkpoint
                    || i == 0;
                ensure!(
                    !require_keyframe || matches!(&record.payload, Payload::Keyframe { .. }),
                    "required explicit keyframe absent"
                );
                match &record.payload {
                    Payload::Keyframe { value } => {
                        validate_value_shape(value)?;
                        current = Some(value.clone());
                        keys += 1;
                        chunk_keys += 1;
                    }
                    Payload::Delta { changes } => {
                        let value = current.as_mut().context("delta before keyframe")?;
                        let before = value.clone();
                        apply(value, changes)?;
                        validate_value_shape(value)?;
                        let expected = diff(&before, value);
                        ensure!(
                            canonical(&serde_json::to_value(&expected)?)?
                                == canonical(&serde_json::to_value(changes)?)?,
                            "noncanonical or contradictory delta operations"
                        );
                        deltas += 1;
                        chunk_deltas += 1;
                    }
                }
                ensure!(
                    encoded_len(current.as_ref().context("record has no value")?)?
                        <= self.options.max_record_bytes,
                    "reconstructed value exceeds trace record byte bound"
                );
                match record.meta.kind {
                    RecordKind::Observation => observations += 1,
                    RecordKind::Event => events += 1,
                    RecordKind::Checkpoint => checkpoints += 1,
                }
                if i == 0 {
                    ensure!(
                        record.digest == reference.first_record_digest,
                        "first record anchor differs"
                    );
                }
                previous = record.digest.clone();
                ordinal += 1;
                visit(
                    &record.meta,
                    current.as_ref().context("record has no value")?,
                )?;
            }
            ensure!(
                previous == reference.last_record_digest
                    && chunk_keys == reference.keyframe_count
                    && chunk_deltas == reference.delta_count,
                "chunk final anchor/counts differ"
            );
        }
        ensure!(
            ordinal == self.record_count
                && previous == self.final_record_digest
                && keys == self.keyframe_count
                && deltas == self.delta_count
                && observations == self.observation_count
                && events == self.event_count
                && checkpoints == self.checkpoint_count,
            "reconstructed trace totals differ"
        );
        Ok(())
    }
}

pub struct TraceWriter<'a> {
    store: &'a Store,
    index: TraceIndex,
    current: Option<Value>,
    pending: Vec<Record>,
    pending_bytes: u64,
    previous_chunk: Option<String>,
}
impl<'a> TraceWriter<'a> {
    pub fn new(store: &'a Store, options: TraceOptions) -> Result<Self> {
        options.validate()?;
        Ok(Self {
            store,
            index: TraceIndex {
                schema: TRACE_INDEX_SCHEMA,
                options,
                record_count: 0,
                keyframe_count: 0,
                delta_count: 0,
                observation_count: 0,
                event_count: 0,
                checkpoint_count: 0,
                final_record_digest: ZERO_DIGEST.into(),
                chunks: vec![],
            },
            current: None,
            pending: vec![],
            pending_bytes: 0,
            previous_chunk: None,
        })
    }
    pub fn append(&mut self, value: &Value, meta: TraceRecordMeta) -> Result<()> {
        meta.validate()?;
        validate_value_shape(value)?;
        ensure!(
            self.index.record_count < u64::MAX,
            "trace record count overflow"
        );
        // Refuse an oversized value even if a small delta could encode it. This
        // keeps both writer and reconstruction current-value memory bounded.
        ensure!(
            encoded_len(value)? <= self.index.options.max_record_bytes,
            "value exceeds trace record byte bound"
        );
        if self.pending.len() == self.index.options.max_records_per_chunk {
            self.flush()?;
        }
        let mut record = self.record(value, meta.clone())?;
        let mut length = encoded_len(&record)?;
        if !self.pending.is_empty()
            && self
                .pending_bytes
                .checked_add(length)
                .and_then(|n| n.checked_add(HEADER_RESERVE))
                .is_none_or(|n| n > self.index.options.max_chunk_bytes)
        {
            self.flush()?;
            record = self.record(value, meta)?;
            length = encoded_len(&record)?;
        }
        ensure!(
            length <= self.index.options.max_record_bytes
                && length
                    .checked_add(HEADER_RESERVE)
                    .is_some_and(|n| n <= self.index.options.max_chunk_bytes),
            "encoded record exceeds trace bounds"
        );
        self.pending_bytes += length;
        self.index.record_count += 1;
        match record.meta.kind {
            RecordKind::Observation => self.index.observation_count += 1,
            RecordKind::Event => self.index.event_count += 1,
            RecordKind::Checkpoint => self.index.checkpoint_count += 1,
        }
        match &record.payload {
            Payload::Keyframe { .. } => self.index.keyframe_count += 1,
            Payload::Delta { .. } => self.index.delta_count += 1,
        }
        self.index.final_record_digest = record.digest.clone();
        self.pending.push(record);
        self.current = Some(value.clone());
        Ok(())
    }
    fn record(&self, value: &Value, meta: TraceRecordMeta) -> Result<Record> {
        let key = self.current.is_none()
            || self.pending.is_empty()
            || self
                .index
                .record_count
                .is_multiple_of(self.index.options.keyframe_interval)
            || meta.kind == RecordKind::Checkpoint;
        let payload = if key {
            Payload::Keyframe {
                value: value.clone(),
            }
        } else {
            Payload::Delta {
                changes: diff(self.current.as_ref().unwrap(), value),
            }
        };
        let mut record = Record {
            ordinal: self.index.record_count,
            meta,
            previous_digest: self.index.final_record_digest.clone(),
            payload,
            digest: String::new(),
        };
        record.digest = record_digest(&record)?;
        Ok(record)
    }
    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let payload = BorrowedChunk {
            schema: CHUNK_SCHEMA,
            first_record: self.pending[0].ordinal,
            previous_chunk_sha256: &self.previous_chunk,
            records: &self.pending,
        };
        ensure!(
            encoded_len(&payload)? <= self.index.options.max_chunk_bytes,
            "encoded chunk exceeds byte bound"
        );
        // Pending records/anchors remain owned if storage fails, allowing retry.
        let blob = self.store.put_value(&payload)?;
        ensure!(
            blob.bytes <= self.index.options.max_chunk_bytes,
            "stored chunk exceeds byte bound"
        );
        let keys = self
            .pending
            .iter()
            .filter(|r| matches!(&r.payload, Payload::Keyframe { .. }))
            .count() as u64;
        self.index.chunks.push(TraceChunkRef {
            blob: blob.clone(),
            first_record: self.pending[0].ordinal,
            record_count: self.pending.len() as u64,
            previous_chunk_sha256: self.previous_chunk.clone(),
            first_record_digest: self.pending[0].digest.clone(),
            last_record_digest: self.pending.last().unwrap().digest.clone(),
            keyframe_count: keys,
            delta_count: self.pending.len() as u64 - keys,
        });
        self.previous_chunk = Some(blob.sha256);
        self.pending.clear();
        self.pending_bytes = 0;
        Ok(())
    }
    pub fn finish(mut self) -> Result<TraceIndex> {
        self.flush()?;
        self.index.validate()?;
        Ok(self.index)
    }
}

fn encoded_len<T: Serialize>(value: &T) -> Result<u64> {
    struct Counter(u64);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len() as u64)
                .filter(|n| *n <= MAX_BLOCK_BYTES)
                .ok_or_else(|| {
                    std::io::Error::other("trace encoding exceeds maximum block bound")
                })?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    value.serialize(&mut rmp_serde::Serializer::new(&mut counter).with_struct_map())?;
    Ok(counter.0)
}
fn valid_digest(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid trace digest"
    );
    Ok(())
}
fn record_digest(record: &Record) -> Result<String> {
    let body = serde_json::to_value(RecordDigest {
        ordinal: record.ordinal,
        meta: &record.meta,
        previous_digest: &record.previous_digest,
        payload: &record.payload,
    })?;
    let mut bytes = b"sims-fixtures/trace-record/v1\0".to_vec();
    bytes.extend(canonical(&body)?);
    Ok(digest(&bytes))
}
/// JSON object key order is canonical, while number domain and f64 sign survive.
fn canonical(value: &Value) -> Result<Vec<u8>> {
    fn write(value: &Value, out: &mut Vec<u8>) -> Result<()> {
        match value {
            Value::Object(map) => {
                out.push(b'{');
                let keys: BTreeSet<_> = map.keys().collect();
                for (i, key) in keys.into_iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    serde_json::to_writer(&mut *out, key)?;
                    out.push(b':');
                    write(&map[key], out)?;
                }
                out.push(b'}');
            }
            Value::Array(values) => {
                out.push(b'[');
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    write(value, out)?;
                }
                out.push(b']');
            }
            _ => serde_json::to_writer(out, value)?,
        }
        Ok(())
    }
    let mut bytes = vec![];
    write(value, &mut bytes)?;
    Ok(bytes)
}
fn validate_value_shape(value: &Value) -> Result<()> {
    let mut pending = vec![(value, 0usize)];
    let mut nodes = 0u64;
    while let Some((value, depth)) = pending.pop() {
        ensure!(depth <= 128, "trace value nesting exceeds bound");
        nodes += 1;
        ensure!(nodes <= 16_000_000, "trace value node count exceeds bound");
        match value {
            Value::Object(map) => pending.extend(map.values().map(|v| (v, depth + 1))),
            Value::Array(values) => pending.extend(values.iter().map(|v| (v, depth + 1))),
            _ => (),
        }
    }
    Ok(())
}
fn exact_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) if a.is_f64() || b.is_f64() => {
            a.is_f64()
                && b.is_f64()
                && a.as_f64().unwrap().to_bits() == b.as_f64().unwrap().to_bits()
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).is_some_and(|w| exact_equal(v, w)))
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| exact_equal(a, b))
        }
        _ => a == b,
    }
}
fn diff(old: &Value, new: &Value) -> Vec<Change> {
    fn walk(old: &Value, new: &Value, path: &mut Vec<String>, changes: &mut Vec<Change>) {
        if exact_equal(old, new) {
            return;
        }
        if let (Value::Object(a), Value::Object(b)) = (old, new) {
            let keys: BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for key in keys {
                path.push(key.clone());
                match (a.get(key), b.get(key)) {
                    (Some(old), Some(new)) => walk(old, new, path, changes),
                    (None, Some(value)) => changes.push(Change::Set {
                        path: path.clone(),
                        value: value.clone(),
                    }),
                    (Some(_), None) => changes.push(Change::Remove { path: path.clone() }),
                    _ => unreachable!(),
                }
                path.pop();
            }
        } else {
            changes.push(Change::Set {
                path: path.clone(),
                value: new.clone(),
            });
        }
    }
    let mut changes = vec![];
    walk(old, new, &mut vec![], &mut changes);
    changes
}
fn apply(value: &mut Value, changes: &[Change]) -> Result<()> {
    for change in changes {
        let path = match change {
            Change::Set { path, .. } | Change::Remove { path } => path,
        };
        ensure!(path.len() <= 512, "delta path exceeds bound");
        if path.is_empty() {
            match change {
                Change::Set { value: next, .. } => *value = next.clone(),
                Change::Remove { .. } => bail!("cannot remove value root"),
            };
            continue;
        }
        let mut parent = &mut *value;
        for key in &path[..path.len() - 1] {
            parent = parent
                .as_object_mut()
                .and_then(|o| o.get_mut(key))
                .context("delta parent absent/nonobject")?;
        }
        let object = parent
            .as_object_mut()
            .context("delta parent is not object")?;
        let key = path.last().unwrap();
        match change {
            Change::Set { value, .. } => {
                object.insert(key.clone(), value.clone());
            }
            Change::Remove { .. } => {
                ensure!(object.remove(key).is_some(), "delta removal key absent");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn meta(kind: RecordKind, boundary: Boundary) -> TraceRecordMeta {
        TraceRecordMeta {
            kind,
            boundary,
            source: None,
        }
    }
    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::create(d.path().join("store")).unwrap();
        (d, s)
    }
    fn options() -> TraceOptions {
        TraceOptions {
            max_records_per_chunk: 3,
            max_chunk_bytes: 64 * 1024,
            max_record_bytes: 32 * 1024,
            keyframe_interval: 5,
        }
    }
    fn write(store: &Store, values: &[Value]) -> TraceIndex {
        let mut w = TraceWriter::new(store, options()).unwrap();
        for value in values {
            w.append(value, meta(RecordKind::Observation, Boundary::Unspecified))
                .unwrap();
        }
        w.finish().unwrap()
    }
    fn read(store: &Store, index: &TraceIndex) -> Vec<Value> {
        let mut values = vec![];
        index
            .reconstruct(store, |_, v| {
                values.push(v.clone());
                Ok(())
            })
            .unwrap();
        values
    }
    #[test]
    fn lossless_delete_null_arrays_root_types_and_keyframes() {
        let (_d, s) = store();
        let values = vec![
            json!({"a":null,"b":1,"items":[1,2]}),
            json!({"b":null,"items":[2,1]}),
            json!({"a":null,"b":null,"items":[2,1]}),
            json!([true, null]),
            json!(false),
            json!({"x":{"y":3}}),
            json!({}),
        ];
        let index = write(&s, &values);
        assert_eq!(read(&s, &index), values);
        assert_eq!(index.record_count, 7);
        assert!(index.keyframe_count >= 3);
        assert!(index.delta_count > 0);
        assert_eq!(index.blob_refs().len(), 3);
        index.verify(&s).unwrap();
    }
    #[test]
    fn negative_zero_and_numeric_domain_changes_are_not_equality_noops() {
        let (_d, s) = store();
        let values = vec![
            json!({"n":-0.0}),
            json!({"n":0.0}),
            json!({"n":-0.0}),
            json!({"n":u64::MAX}),
            json!({"n":i64::MIN}),
            json!({"n":1.0}),
            json!({"n":1}),
        ];
        let index = write(&s, &values);
        let back = read(&s, &index);
        for (a, b) in values.iter().zip(&back) {
            assert_eq!(canonical(a).unwrap(), canonical(b).unwrap());
        }
        assert!(!diff(&values[0], &values[1]).is_empty());
        assert!(!diff(&values[5], &values[6]).is_empty());
    }
    #[test]
    fn completed_tick_gaps_and_non_tick_events_remain_explicit() {
        let (_d, s) = store();
        let mut w = TraceWriter::new(&s, options()).unwrap();
        let metas = [
            meta(RecordKind::Checkpoint, Boundary::CompletedTick { tick: 10 }),
            meta(
                RecordKind::Event,
                Boundary::Event {
                    name: "menu_open".into(),
                    tick: Some(10),
                },
            ),
            meta(
                RecordKind::Observation,
                Boundary::CompletedTick { tick: 14 },
            ),
        ];
        for (i, m) in metas.iter().enumerate() {
            w.append(&json!({"n":i}), m.clone()).unwrap();
        }
        let index = w.finish().unwrap();
        let mut got = vec![];
        index
            .reconstruct(&s, |m, _| {
                got.push(m.clone());
                Ok(())
            })
            .unwrap();
        assert_eq!(got, metas);
        assert_eq!(
            (
                index.record_count,
                index.checkpoint_count,
                index.event_count,
                index.observation_count
            ),
            (3, 1, 1, 1)
        );
    }
    #[test]
    fn missing_reordered_and_changed_blocks_are_rejected() {
        let (_d, s) = store();
        let values: Vec<_> = (0..8).map(|n| json!({"n":n})).collect();
        let index = write(&s, &values);
        let mut bad = index.clone();
        bad.chunks.remove(1);
        assert!(bad.verify(&s).is_err());
        let mut bad = index.clone();
        bad.chunks.swap(0, 1);
        assert!(bad.verify(&s).is_err());
        let mut bad = index.clone();
        let mut chunk: Chunk = s.get_value(&bad.chunks[1].blob).unwrap();
        chunk.records[0].payload = Payload::Keyframe {
            value: json!("corrupt"),
        };
        bad.chunks[1].blob = s.put_value(&chunk).unwrap();
        assert!(bad.verify(&s).is_err());
        let mut bad = index.clone();
        bad.chunks[0].last_record_digest = "a".repeat(64);
        assert!(bad.verify(&s).is_err());
    }
    #[test]
    fn unknown_schema_invalid_limits_and_source_metadata_fail_closed() {
        let (_d, s) = store();
        let mut index = write(&s, &[Value::Null]);
        index.schema = 2;
        assert!(index.verify(&s).is_err());
        let mut bad = options();
        bad.keyframe_interval = 0;
        assert!(TraceWriter::new(&s, bad).is_err());
        let mut w = TraceWriter::new(&s, options()).unwrap();
        let mut m = meta(RecordKind::Observation, Boundary::CompletedTick { tick: 1 });
        m.source = Some(SourceRecord {
            line: 1,
            byte_offset: u64::MAX,
            byte_length: 1,
            actual_tick: Some(1),
            advanced: Some(0),
        });
        assert!(w.append(&Value::Null, m).is_err());
        assert_eq!(w.finish().unwrap().record_count, 0);
    }
    #[test]
    fn individually_bounded_rehashed_deltas_cannot_accumulate_unbounded_state() {
        let (_d, s) = store();
        let options = TraceOptions {
            max_records_per_chunk: 16,
            max_chunk_bytes: 8192,
            max_record_bytes: 2048,
            keyframe_interval: 128,
        };
        let mut records = vec![];
        let mut previous = ZERO_DIGEST.to_string();
        for (ordinal, payload) in [
            Payload::Keyframe { value: json!({}) },
            Payload::Delta {
                changes: vec![Change::Set {
                    path: vec!["a".into()],
                    value: Value::String("x".repeat(1200)),
                }],
            },
            Payload::Delta {
                changes: vec![Change::Set {
                    path: vec!["b".into()],
                    value: Value::String("y".repeat(1200)),
                }],
            },
        ]
        .into_iter()
        .enumerate()
        {
            let mut record = Record {
                ordinal: ordinal as u64,
                meta: meta(RecordKind::Observation, Boundary::Unspecified),
                previous_digest: previous,
                payload,
                digest: String::new(),
            };
            record.digest = record_digest(&record).unwrap();
            assert!(encoded_len(&record).unwrap() < options.max_record_bytes);
            previous = record.digest.clone();
            records.push(record);
        }
        let first = records[0].digest.clone();
        let chunk = Chunk {
            schema: 1,
            first_record: 0,
            previous_chunk_sha256: None,
            records,
        };
        let blob = s.put_value(&chunk).unwrap();
        let index = TraceIndex {
            schema: TRACE_INDEX_SCHEMA,
            options,
            record_count: 3,
            keyframe_count: 1,
            delta_count: 2,
            observation_count: 3,
            event_count: 0,
            checkpoint_count: 0,
            final_record_digest: previous.clone(),
            chunks: vec![TraceChunkRef {
                blob,
                first_record: 0,
                record_count: 3,
                previous_chunk_sha256: None,
                first_record_digest: first,
                last_record_digest: previous,
                keyframe_count: 1,
                delta_count: 2,
            }],
        };
        index.validate().unwrap();
        let mut emitted = 0;
        let error = index
            .reconstruct(&s, |_, _| {
                emitted += 1;
                Ok(())
            })
            .unwrap_err();
        assert!(error.to_string().contains("reconstructed value exceeds"));
        assert_eq!(emitted, 2);
    }
    #[test]
    fn deterministic_diff_does_not_depend_on_insertion_order() {
        let old = json!({"z":1,"a":{"x":2},"remove":null});
        let new = json!({"z":2,"a":{"x":3,"new":[2]},"add":null});
        let mut actual = old.clone();
        let changes = diff(&old, &new);
        apply(&mut actual, &changes).unwrap();
        assert_eq!(canonical(&actual).unwrap(), canonical(&new).unwrap());
        assert!(
            matches!(&changes[0],Change::Set{path,..}if path==&vec!["a".to_string(),"new".to_string()])
        );
        assert!(
            apply(
                &mut actual,
                &[Change::Remove {
                    path: vec!["absent".into()]
                }]
            )
            .is_err()
        );
    }
}
