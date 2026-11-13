//! Bounded legacy JSON/JSONL ingestion. Exact selected source bytes are kept
//! independently of normalized observations. Imported observations are never
//! declared resumable states or certified simulation ticks.
use crate::{
    Artifact, ArtifactFormat, BlobRef, Boundary, CaseManifest, Compression, Coverage,
    HashVerification, ProvenanceSource, Store,
    trace::{
        RecordKind, SourceRecord, TRACE_INDEX_SCHEMA, TraceOptions, TraceRecordMeta, TraceWriter,
    },
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;
use std::{
    io::{BufRead, Read},
    path::Path,
};

pub const DEFAULT_RECORD_LIMIT: u64 = 33;
pub const DEFAULT_MAX_LINE_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_MAX_PREFIX_BYTES: u64 = 256 * 1024 * 1024;
pub const DEFAULT_MAX_TRACE_RECORD_BYTES: u64 = 128 * 1024 * 1024;
pub const DEFAULT_MAX_TRACE_CHUNK_BYTES: u64 = DEFAULT_MAX_TRACE_RECORD_BYTES + 1024;
pub const MAX_RECORDS: u64 = 1_000_000;
const MAX_DECLARATION_BYTES: usize = 65_000;

#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub id: String,
    pub title: String,
    pub compression: Compression,
    /// Inert logical locator, not a file to open during restore/replay.
    pub source_locator: Option<String>,
    /// Explicit caller declaration about the complete original source, which
    /// may be larger than the selected prefix. Never computed from that prefix.
    pub origin_sha256: Option<String>,
    pub source_image_sha256: Option<String>,
    pub provenance: Option<Value>,
    pub coverage: Option<Value>,
    /// Exact inert invocation vectors supplied by the calling program.
    pub commands: Vec<Vec<String>>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct JsonlOptions {
    pub record_limit: u64,
    pub max_records: u64,
    pub max_line_bytes: u64,
    pub max_prefix_bytes: u64,
    /// MessagePack limits are independent of raw line size: numbers and
    /// metadata can make a normalized record larger than its JSON source.
    pub trace_options: TraceOptions,
    /// Optional RFC6901 source fields. Missing on a row means unobserved;
    /// present values with the wrong type are rejected, never normalized.
    pub tick_pointer: Option<String>,
    pub advanced_pointer: Option<String>,
}
impl Default for JsonlOptions {
    fn default() -> Self {
        Self {
            record_limit: DEFAULT_RECORD_LIMIT,
            max_records: MAX_RECORDS,
            max_line_bytes: DEFAULT_MAX_LINE_BYTES,
            max_prefix_bytes: DEFAULT_MAX_PREFIX_BYTES,
            trace_options: TraceOptions {
                max_record_bytes: DEFAULT_MAX_TRACE_RECORD_BYTES,
                max_chunk_bytes: DEFAULT_MAX_TRACE_CHUNK_BYTES,
                ..TraceOptions::default()
            },
            tick_pointer: None,
            advanced_pointer: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub case_id: String,
    pub source: BlobRef,
    pub normalized: BlobRef,
    pub selected_records: u64,
    pub selected_bytes: u64,
    /// Only true if EOF was actually read, including an unterminated final row.
    pub eof_observed: bool,
    pub explicit_advanced_true_records: u64,
    pub first_observed_tick: Option<u64>,
    pub last_observed_tick: Option<u64>,
    pub trace_options: Option<TraceOptions>,
    pub coverage: &'static str,
}

fn validate_sha(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "declared SHA-256 must be exactly 64 lowercase hex characters"
    );
    Ok(())
}
fn validate_pointer(pointer: &str) -> Result<()> {
    ensure!(
        pointer.is_empty() || pointer.starts_with('/'),
        "source field must be a JSON pointer"
    );
    ensure!(
        !pointer.contains('\0') && pointer.len() <= 4096,
        "source JSON pointer exceeds limits"
    );
    let mut bytes = pointer.bytes();
    while let Some(b) = bytes.next() {
        if b == b'~' {
            ensure!(
                matches!(bytes.next(), Some(b'0' | b'1')),
                "invalid JSON pointer escape"
            );
        }
    }
    Ok(())
}
impl JsonlOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=MAX_RECORDS).contains(&self.max_records),
            "max-records must be 1..={MAX_RECORDS}"
        );
        ensure!(
            (1..=self.max_records).contains(&self.record_limit),
            "record-limit exceeds max-records"
        );
        ensure!(
            (1..=1024 * 1024 * 1024).contains(&self.max_line_bytes),
            "max-line-bytes must be 1..=1GiB"
        );
        ensure!(
            (1..=16 * 1024 * 1024 * 1024).contains(&self.max_prefix_bytes),
            "max-prefix-bytes must be 1..=16GiB"
        );
        self.trace_options.validate()?;
        for pointer in [&self.tick_pointer, &self.advanced_pointer]
            .into_iter()
            .flatten()
        {
            validate_pointer(pointer)?;
        }
        Ok(())
    }
}
fn validate_import(options: &ImportOptions) -> Result<()> {
    crate::model::validate_id(&options.id)?;
    ensure!(
        !options.title.trim().is_empty() && options.title.len() <= 1024,
        "invalid import title"
    );
    for sha in [&options.origin_sha256, &options.source_image_sha256]
        .into_iter()
        .flatten()
    {
        validate_sha(sha)?;
    }
    for declaration in [&options.provenance, &options.coverage]
        .into_iter()
        .flatten()
    {
        ensure!(
            serde_json::to_vec(declaration)?.len() <= MAX_DECLARATION_BYTES,
            "import declaration too large"
        );
    }
    Ok(())
}
fn manifest(
    options: &ImportOptions,
    source: Artifact,
    normalized: Artifact,
    summary: String,
) -> Result<CaseManifest> {
    let mut notes = options.notes.clone();
    notes.push(summary);
    notes.push("Imported observations only; source records/advanced flags are not independently qualified gameplay ticks or resumable VM states".into());
    let mut case = CaseManifest::minimal_import(&options.id, &options.title, source, notes);
    case.artifacts.push(normalized);
    case.provenance.source_locator = options.source_locator.clone();
    case.provenance.commands = options.commands.clone();
    if let Some(sha) = &options.origin_sha256 {
        case.provenance.sources.push(ProvenanceSource {
            label: "Complete original source (caller declaration)".into(),
            sha256: sha.clone(),
            verification: HashVerification::Provided,
        });
        case.provenance.notes.push(format!("Caller-declared complete original source SHA-256 (not computed from selected prefix): {sha}"));
    }
    if let Some(sha) = &options.source_image_sha256 {
        case.provenance.sources.push(ProvenanceSource {
            label: "Source image (caller declaration)".into(),
            sha256: sha.clone(),
            verification: HashVerification::Provided,
        });
        case.provenance.notes.push(format!(
            "Caller-declared source image SHA-256 (image not opened by importer): {sha}"
        ));
    }
    if let Some(value) = &options.provenance {
        case.provenance.notes.push(format!(
            "Unverified caller provenance declaration: {}",
            serde_json::to_string(value)?
        ));
    }
    if let Some(value) = &options.coverage {
        case.notes.push(format!(
            "Unverified caller coverage declaration; does not override import qualification: {}",
            serde_json::to_string(value)?
        ));
    }
    case.coverage = Coverage::Unqualified { reason: "Legacy source bytes and normalized observations preserved; no semantic, parity or resume qualification performed".into() };
    case.validate()?;
    Ok(case)
}

/// Read one entire selected line with strict allocation bounds. The returned
/// bytes include its exact CRLF/LF, or lack of final newline. Empty/invalid JSON
/// lines remain errors rather than being silently skipped or renumbered.
struct ReadLine {
    bytes: Vec<u8>,
    eof_observed: bool,
}
fn line(
    reader: &mut impl BufRead,
    max_line: u64,
    remaining_prefix: u64,
) -> Result<Option<ReadLine>> {
    let mut output = Vec::new();
    loop {
        let available = reader.fill_buf().context("read legacy JSONL source")?;
        if available.is_empty() {
            return Ok((!output.is_empty()).then_some(ReadLine {
                bytes: output,
                eof_observed: true,
            }));
        }
        let count = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |p| p + 1);
        let next = (output.len() as u64)
            .checked_add(count as u64)
            .context("JSONL line length overflow")?;
        ensure!(next <= max_line, "JSONL line exceeds max-line-bytes");
        ensure!(
            next <= remaining_prefix,
            "selected source prefix exceeds max-prefix-bytes"
        );
        let complete = available[count - 1] == b'\n';
        output.extend_from_slice(&available[..count]);
        reader.consume(count);
        if complete {
            return Ok(Some(ReadLine {
                bytes: output,
                eof_observed: false,
            }));
        }
    }
}
fn metadata(
    value: &Value,
    options: &JsonlOptions,
    line: u64,
    offset: u64,
    length: u64,
) -> Result<TraceRecordMeta> {
    let actual_tick = options
        .tick_pointer
        .as_ref()
        .and_then(|p| value.pointer(p))
        .map(|v| {
            v.as_u64()
                .context("explicit tick field must be an unsigned integer")
        })
        .transpose()?;
    let advanced = options
        .advanced_pointer
        .as_ref()
        .and_then(|p| value.pointer(p))
        .map(|v| {
            v.as_bool()
                .map(u64::from)
                .context("explicit advanced field must be a Boolean")
        })
        .transpose()?;
    Ok(TraceRecordMeta {
        kind: RecordKind::Observation,
        // Even a supplied `advanced:true` is a source observation, not a
        // storage-issued CompletedTick or semantic qualification.
        boundary: Boundary::Unspecified,
        source: Some(SourceRecord {
            line,
            byte_offset: offset,
            byte_length: length,
            actual_tick,
            advanced,
        }),
    })
}

pub fn import_jsonl(
    store: &Store,
    reader: &mut impl BufRead,
    options: &ImportOptions,
    bounds: &JsonlOptions,
) -> Result<ImportReport> {
    validate_import(options)?;
    bounds.validate()?;
    let mut trace = TraceWriter::new(store, bounds.trace_options.clone())?;
    let (mut records, mut bytes, mut eof, mut advanced, mut first_tick, mut last_tick) =
        (0u64, 0u64, false, 0u64, None, None);
    let raw = store.put_with(options.compression, |output| {
        while records < bounds.record_limit {
            let Some(selected_line) = line(
                reader,
                bounds.max_line_bytes,
                bounds.max_prefix_bytes - bytes,
            )?
            else {
                eof = true;
                break;
            };
            let raw_line = selected_line.bytes;
            eof |= selected_line.eof_observed;
            let row = records.checked_add(1).context("record count overflow")?;
            let value: Value = serde_json::from_slice(&raw_line).with_context(|| {
                format!("invalid JSON at source line {row}, byte offset {bytes}")
            })?;
            let meta = metadata(&value, bounds, row, bytes, raw_line.len() as u64)?;
            let source = meta
                .source
                .as_ref()
                .expect("metadata explicitly contains source");
            advanced += u64::from(source.advanced == Some(1));
            if let Some(tick) = source.actual_tick {
                first_tick.get_or_insert(tick);
                last_tick = Some(tick);
            }
            trace.append(&value, meta)?;
            output
                .write_all(&raw_line)
                .context("store exact source prefix")?;
            bytes = bytes
                .checked_add(raw_line.len() as u64)
                .context("source byte offset overflow")?;
            records = row;
        }
        ensure!(records > 0, "JSONL import contains no selected records");
        Ok(())
    })?;
    let index = trace.finish()?;
    ensure!(
        index.record_count == records,
        "trace index count differs from selected source records"
    );
    let normalized = store.put_value(&index)?;
    let source = Artifact::self_contained(raw.clone(), ArtifactFormat::Other("jsonl".into()), 1);
    let normalized_artifact = Artifact::declared(
        normalized.clone(),
        ArtifactFormat::TraceIndex,
        TRACE_INDEX_SCHEMA,
        index.blob_refs(),
    );
    let case = manifest(
        options,
        source,
        normalized_artifact,
        format!(
            "Selected first {records} JSONL records, exact bytes [0,{bytes}); EOF observed={eof}; explicitly true advanced flags={advanced}; no record-count-to-tick conversion"
        ),
    )?;
    store.publish_case(&case)?;
    Ok(ImportReport {
        case_id: options.id.clone(),
        source: raw,
        normalized,
        selected_records: records,
        selected_bytes: bytes,
        eof_observed: eof,
        explicit_advanced_true_records: advanced,
        first_observed_tick: first_tick,
        last_observed_tick: last_tick,
        trace_options: Some(bounds.trace_options.clone()),
        coverage: "unqualified imported observations",
    })
}

pub fn import_json(
    store: &Store,
    mut reader: impl Read,
    options: &ImportOptions,
    max_bytes: u64,
) -> Result<ImportReport> {
    validate_import(options)?;
    ensure!(
        (1..=1024 * 1024 * 1024).contains(&max_bytes),
        "JSON max-input-bytes must be 1..=1GiB"
    );
    let mut raw = Vec::new();
    reader
        .by_ref()
        .take(max_bytes + 1)
        .read_to_end(&mut raw)
        .context("read bounded JSON source")?;
    ensure!(
        raw.len() as u64 <= max_bytes,
        "JSON source exceeds max-input-bytes"
    );
    let value: Value = serde_json::from_slice(&raw).context("parse legacy JSON source")?;
    let source = store.put_bytes(&raw, options.compression)?;
    let normalized = store.put_value(&value)?;
    let case = manifest(
        options,
        Artifact::self_contained(source.clone(), ArtifactFormat::Json, 1),
        Artifact::self_contained(normalized.clone(), ArtifactFormat::MessagePack, 1),
        format!(
            "One JSON document; exact {0} source bytes retained independently of normalized MessagePack",
            raw.len()
        ),
    )?;
    store.publish_case(&case)?;
    Ok(ImportReport {
        case_id: options.id.clone(),
        source,
        normalized,
        selected_records: 1,
        selected_bytes: raw.len() as u64,
        eof_observed: true,
        explicit_advanced_true_records: 0,
        first_observed_tick: None,
        last_observed_tick: None,
        trace_options: None,
        coverage: "unqualified imported observations",
    })
}

/// A default locator is deliberately portable. The caller can supply an inert
/// complete source path in provenance, but replay never requires that old path.
pub fn logical_source(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .context("source path has no UTF-8 basename; provide --source-id")
}
