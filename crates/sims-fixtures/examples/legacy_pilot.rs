//! Selective migration proof for an independently selected original capture.
//! The selection receipt describes each original source span; it is evidence,
//! not an executable scenario or proof of simulation parity.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_fixtures::{
    Artifact, ArtifactFormat, Boundary, Compression, Reference, ReferenceKind, Store,
    legacy::{ImportOptions, JsonlOptions, import_jsonl},
    trace::TraceIndex,
};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek},
    path::PathBuf,
    time::Instant,
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    store: PathBuf,
    #[arg(long)]
    source: PathBuf,
    #[arg(long)]
    selection: PathBuf,
    #[arg(long)]
    export: PathBuf,
    #[arg(long)]
    id: String,
}

fn u64_field(value: &Value, field: &str) -> Result<u64> {
    value[field]
        .as_u64()
        .with_context(|| format!("missing unsigned {field}"))
}
fn string_field<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value[field]
        .as_str()
        .with_context(|| format!("missing string {field}"))
}
fn hash_reader(mut reader: impl Read) -> Result<(String, u64)> {
    let mut hash = Sha256::new();
    let mut count = 0;
    let mut buffer = [0; 65536];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        count += n as u64;
    }
    Ok((format!("{:x}", hash.finalize()), count))
}

/// Compare both original row bytes and normalized values, one observation at a
/// time. This is deliberately separate from the trace's internal hash checks.
fn compare(
    store: &Store,
    index: &TraceIndex,
    receipt: &Value,
    reader: &mut impl BufRead,
) -> Result<(u64, Value, Value)> {
    let rows = receipt["rows"].as_array().context("selection rows")?;
    let (mut ordinal, mut offset) = (0usize, 0u64);
    let (mut first, mut last) = (None, None);
    index.reconstruct(store, |meta, reconstructed| {
        let row = rows.get(ordinal).context("trace has extra observation")?;
        let mut raw = Vec::new();
        reader.read_until(b'\n', &mut raw)?;
        ensure!(!raw.is_empty(), "selected source ended early");
        ensure!(
            raw.len() as u64 == u64_field(row, "bytes")?,
            "row length changed"
        );
        ensure!(
            sims_fixtures::digest(&raw) == string_field(row, "sha256")?,
            "row bytes changed"
        );
        ensure!(
            offset == u64_field(row, "selected_byte_offset")?,
            "selection offset changed"
        );
        let original: Value = serde_json::from_slice(&raw)?;
        // Canonical JSON comparison also distinguishes -0.0 and integer/f64.
        ensure!(
            serde_json::to_vec(&original)? == serde_json::to_vec(reconstructed)?,
            "observation {ordinal} changed"
        );
        ensure!(
            original["kind"] == row["kind"]
                && original["tick"] == row["tick"]
                && original["advanced"] == row["advanced"],
            "selected original boundary changed"
        );
        let source = meta
            .source
            .as_ref()
            .context("missing trace source metadata")?;
        ensure!(
            source.line == ordinal as u64 + 1
                && source.byte_offset == offset
                && source.byte_length == raw.len() as u64,
            "trace source span changed"
        );
        ensure!(
            source.actual_tick == original["tick"].as_u64(),
            "observed tick changed"
        );
        ensure!(
            source.advanced.is_none(),
            "numeric original flag must not become a Boolean qualification"
        );
        first.get_or_insert_with(|| original.clone());
        last = Some(original);
        offset += raw.len() as u64;
        ordinal += 1;
        Ok(())
    })?;
    ensure!(ordinal == rows.len(), "missing observations");
    ensure!(reader.fill_buf()?.is_empty(), "uncompared source suffix");
    ensure!(
        offset == u64_field(receipt, "selected_bytes")?,
        "selected byte count changed"
    );
    Ok((
        ordinal as u64,
        first.context("empty trace")?,
        last.context("empty trace")?,
    ))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let argv = std::env::args_os()
        .map(|v| {
            v.into_string()
                .map_err(|_| anyhow::anyhow!("pilot provenance requires UTF8 arguments"))
        })
        .collect::<Result<Vec<_>>>()?;
    let receipt_bytes = fs::read(&args.selection)?;
    let receipt: Value = serde_json::from_slice(&receipt_bytes)?;
    let source_fingerprint = hash_reader(File::open(&args.source)?)?;
    ensure!(
        source_fingerprint.0 == string_field(&receipt, "selected_sha256")?
            && source_fingerprint.1 == u64_field(&receipt, "selected_bytes")?,
        "selected source differs from receipt"
    );
    ensure!(
        receipt["strict_original_parity_qualified"] == false,
        "pilot must not imply new parity qualification"
    );
    let rows = receipt["rows"].as_array().context("selection rows")?;
    let store = Store::create(&args.store)?;
    let start = Instant::now();
    let imported = import_jsonl(
        &store,
        &mut BufReader::new(File::open(&args.source)?),
        &ImportOptions {
            id: format!("{}-import", args.id),
            title: "Original-game selected observations (raw migration)".into(),
            compression: Compression::Gzip,
            source_locator: Some("selected-original-newbie-observations".into()),
            origin_sha256: Some(string_field(&receipt, "source_sha256")?.into()),
            source_image_sha256: None,
            provenance: None,
            coverage: None,
            commands: vec![argv],
            notes: vec!["Selected noncontiguous baseline/return records, not the contiguous prefix of the full capture. Original source spans are in the attached receipt. No replay input schedule or resumable VM is supplied.".into()],
        },
        &JsonlOptions {
            record_limit: rows.len() as u64,
            tick_pointer: Some("/tick".into()),
            ..JsonlOptions::default()
        },
    )?;
    let import_seconds = start.elapsed().as_secs_f64();
    ensure!(
        imported.source.sha256 == source_fingerprint.0
            && imported.source.bytes == source_fingerprint.1,
        "retained raw bytes differ"
    );
    let index: TraceIndex = store.get_value(&imported.normalized)?;
    let compare_start = Instant::now();
    let (count, first, last) = compare(
        &store,
        &index,
        &receipt,
        &mut BufReader::new(File::open(&args.source)?),
    )?;
    let compare_seconds = compare_start.elapsed().as_secs_f64();
    let mut case = store.load_case(&imported.case_id)?;
    case.id = args.id;
    case.title = "Newbie original baseline +32 recorded updates: storage migration pilot".into();
    case.artifacts.push(Artifact::self_contained(
        store.put_bytes(&receipt_bytes, Compression::Gzip)?,
        ArtifactFormat::Json,
        1,
    ));
    for (label, value) in [
        ("Original observed baseline", first),
        ("Original observed final state", last),
    ] {
        case.references.push(Reference {
            label: label.into(),
            kind: ReferenceKind::Observed,
            artifact: Artifact::self_contained(
                store.put_value(&value)?,
                ArtifactFormat::MessagePack,
                1,
            ),
            resume_contract: None,
            boundary: Boundary::Event {
                name: string_field(&value, "kind")?.into(),
                tick: value["tick"].as_u64(),
            },
        });
    }
    case.notes.push("The receipt reports a prior full-source SHA256 scan; this example rechecks the selected bytes and each selected observation. Complete strict semantic parity and cold resume remain unqualified. The schedule is absent, not assumed to contain no input.".into());
    store.publish_case(&case)?;
    let source_verification = store.verify_all()?;
    let source_raw_stored_bytes = fs::metadata(store.blob_path(&imported.source)?)?.len();
    let normalized_stored_bytes = fs::metadata(store.blob_path(&imported.normalized)?)?.len()
        + index
            .blob_refs()
            .iter()
            .map(|b| {
                fs::metadata(store.blob_path(b)?)
                    .map(|m| m.len())
                    .map_err(anyhow::Error::from)
            })
            .collect::<Result<Vec<_>>>()?
            .iter()
            .sum::<u64>();
    let export_start = Instant::now();
    store.export_case(&case.id, &args.export)?;
    let export = Store::open(&args.export)?;
    let export_verification = export.verify_all()?;
    let exported = export.load_case(&case.id)?;
    ensure!(case == exported, "export manifest changed");
    let exported_index: TraceIndex = export.get_value(&imported.normalized)?;
    let mut decoded = tempfile::tempfile()?;
    export.copy_blob(&exported.source.blob, &mut decoded)?;
    decoded.rewind()?;
    ensure!(
        hash_reader(&mut decoded)? == source_fingerprint,
        "exported raw source changed"
    );
    decoded.rewind()?;
    let (export_count, _, _) = compare(
        &export,
        &exported_index,
        &receipt,
        &mut BufReader::new(decoded),
    )?;
    ensure!(export_count == count, "export observation count changed");
    let report = json!({
        "case_id":case.id,"source":args.source,"store":args.store,"export":args.export,
        "selected_raw_bytes":source_fingerprint.1,"selected_raw_sha256":source_fingerprint.0,
        "observations_compared":count,"import_seconds":import_seconds,"compare_seconds":compare_seconds,
        "export_and_verify_seconds":export_start.elapsed().as_secs_f64(),
        "raw_source_stored_bytes":source_raw_stored_bytes,"normalized_trace_stored_bytes":normalized_stored_bytes,
        "trace_keyframes":index.keyframe_count,"trace_deltas":index.delta_count,"trace_chunks":index.chunks.len(),
        "source_store_verification":source_verification,"export_verification":export_verification,
        "coverage":"byte/observation preservation only; zero new gameplay parity ticks; not resumable",
        "old_capture_deleted":false
    });
    serde_json::to_writer_pretty(std::io::stdout().lock(), &report)?;
    println!();
    Ok(())
}
