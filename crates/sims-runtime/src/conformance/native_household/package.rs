//! One-time migration of the archived accepted proof to a small typed replay
//! fixture. Archive membership is checked; historical captures are never edited.
use super::*;
use sims_fixtures::archive::ArchiveInventory;
use sims_fixtures::{Artifact, Boundary, Compression, Coverage, Origin, Reference, ReferenceKind};

fn inventory(store: &Store, id: &str, expected: &str) -> Result<(ArchiveInventory, BlobRef)> {
    let case = store.load_case(id)?;
    ensure!(
        case.source.format == ArtifactFormat::ArchiveInventory
            && case.source.blob.sha256 == expected,
        "qualified archive inventory differs"
    );
    let inv: ArchiveInventory = store.get_value(&case.source.blob)?;
    inv.validate_artifact(&case.source)?;
    Ok((inv, case.source.blob))
}
fn file(store: &Store, inventory: &ArchiveInventory, path: &str) -> Result<(Vec<u8>, BlobRef)> {
    let row = inventory
        .files
        .iter()
        .find(|r| r.relative_path == path)
        .with_context(|| format!("qualified source file absent: {path}"))?;
    Ok((store.read_bytes(&row.blob, 32 << 20)?, row.blob.clone()))
}
fn copy_file(
    store: &Store,
    target: &Store,
    inventory: &ArchiveInventory,
    path: &str,
) -> Result<BlobRef> {
    let (data, reference) = file(store, inventory, path)?;
    ensure!(
        target.put_bytes(&data, Compression::Gzip)? == reference,
        "fixture copy changed source identity"
    );
    Ok(reference)
}
fn source_saved(
    source: &Store,
    inventory: &ArchiveInventory,
    reader: &Store,
    target: &Store,
    reference: &BlobRef,
) -> Result<BlobRef> {
    let relative = format!(
        "fixtures/private/goth-label-owners-20261029-first/session-first/store/blobs/sha256/{}/{}.blob",
        &reference.sha256[..2],
        &reference.sha256[2..]
    );
    let (raw, transport) = file(source, inventory, &relative)?;
    ensure!(
        sims_fixtures::digest(&raw) == transport.sha256 && raw.len() as u64 == transport.bytes,
        "restored checkpoint transport differs from qualified archive"
    );
    let destination = reader.blob_path(reference)?;
    fs::create_dir_all(destination.parent().context("transport parent absent")?)?;
    fs::write(&destination, raw)?;
    let saved: Saved = reader.get_value(reference)?;
    ensure!(
        target.put_value(&saved)? == *reference,
        "native snapshot migration changed bytes"
    );
    Ok(reference.clone())
}
pub fn build(source: &Store, result: &Store, target: &Store, id: &str) -> Result<Value> {
    // Read archived transports in a new owned Store, without modifying the archive.
    let scratch = target.root().join("tmp").join(format!(
        "native-reader-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let reader = Store::create(&scratch)?;
    let (inv, source_inventory) = inventory(
        source,
        "native-goth-completed-one-tick-20261029-v1",
        SOURCE_INVENTORY,
    )?;
    let (result_inv, _) = inventory(
        result,
        "native-goth-one-tick-admission-result-20261029-v1",
        "50e86777f0c5879a078e9d0492b09cd73680f4f3fe31c37ec55f866fa2d27fa2",
    )?;
    let proof_path = "fixtures/private/goth-one-tick-admission-20261029-first/independent-first/verification/admission.json";
    let (proof_bytes, qualification) = file(result, &result_inv, proof_path)?;
    ensure!(
        qualification.sha256 == QUALIFICATION,
        "original admission receipt differs"
    );
    let proof: Value = serde_json::from_slice(&proof_bytes)?;
    target.put_bytes(&proof_bytes, Compression::Gzip)?;
    target.put_bytes(
        &source.read_bytes(&source_inventory, 16 << 20)?,
        Compression::Gzip,
    )?;
    let config_path = "fixtures/private/goth-label-owners-20261029-first/session-input.json";
    let (config_bytes, _) = file(source, &inv, config_path)?;
    let config: Value = serde_json::from_slice(&config_bytes)?;
    let mut program_files = BTreeMap::new();
    program_files.insert(
        config_path.into(),
        copy_file(source, target, &inv, config_path)?,
    );
    for key in [
        "reentry",
        "systems",
        "appearances",
        "bindings",
        "qualification",
        "catalog_bindings",
        "corrected_animation_input",
        "sample_buffer_proof",
        "resident_global_capture",
    ] {
        if let Some(name) = config[key].as_str() {
            program_files.insert(name.into(), copy_file(source, target, &inv, name)?);
        }
    }
    let mut qualification_witnesses = BTreeMap::new();
    for (name, row) in proof["witnesses"]
        .as_object()
        .context("qualification witnesses absent")?
    {
        let path = format!(
            "fixtures/private/goth-one-tick-admission-20261029-first/independent-first/verification/{}",
            row["path"].as_str().context("witness locator absent")?
        );
        let reference = copy_file(result, target, &result_inv, &path)?;
        ensure!(
            row["sha256"] == reference.sha256,
            "qualification witness source hash differs"
        );
        qualification_witnesses.insert(name.clone(), reference);
    }
    let checkpoint = "fixtures/private/goth-label-owners-20261029-first/session-first";
    let reference = |name: &str| -> Result<BlobRef> {
        let (bytes, _) = file(source, &inv, &format!("{checkpoint}/{name}"))?;
        let value: Value = serde_json::from_slice(&bytes)?;
        let r = serde_json::from_value(value["session"].clone())?;
        source_saved(source, &inv, &reader, target, &r)
    };
    let baseline = reference("refs.json")?;
    let endpoint = reference("managed-endpoint.json")?;
    let native_trace = copy_file(
        source,
        target,
        &inv,
        "fixtures/private/goth-original-replay-20261028-first/exact-cold-second/native-bytecode.json",
    )?;
    let reference_trace = copy_file(
        source,
        target,
        &inv,
        &format!("{checkpoint}/rust-trace.json"),
    )?;
    let mut cold_cuts = vec![];
    for row in proof["independent_suspended_resumes"]
        .as_array()
        .context("qualified suspended states absent")?
    {
        let state: BlobRef = serde_json::from_value(row["checkpoint"].clone())?;
        cold_cuts.push(ColdCut {
            state: source_saved(source, &inv, &reader, target, &state)?,
            suffix_events: row["trace_suffix_events"]
                .as_u64()
                .context("qualified trace suffix absent")? as usize,
            boundary: row["cut"].clone(),
        });
    }
    let request = Request {
        schema: 1,
        contract: CONTRACT.into(),
        qualification: qualification.clone(),
        source_inventory,
        qualification_witnesses,
        program_config: config_path.into(),
        program_files,
        baseline: baseline.clone(),
        frames: vec![Frame {
            tick: 88449,
            state: endpoint.clone(),
            native_trace,
            reference_trace,
        }],
        cold_cuts,
    };
    validate_qualification(&request, &proof)?;
    let request_blob = target.put_value(&request)?;
    let artifact = Artifact::declared(
        request_blob.clone(),
        ArtifactFormat::Other(FORMAT.into()),
        1,
        request.dependencies(),
    );
    let mut case=CaseManifest::minimal_import(id,"Qualified original Goth completed88448 to88449 replay",artifact.clone(),vec!["Compact native-mapped replay references derived from the accepted79-check proof; source requalification remains a separate acceptance path. No simulation executes during migration.".into()]);
    case.artifacts.push(artifact);
    case.provenance.origin = Origin::Authored;
    case.provenance.source_locator = Some(format!(
        "native-goth-completed-one-tick-20261029-v1@{SOURCE_INVENTORY}"
    ));
    case.coverage=Coverage::ClaimedParity{contract:CONTRACT.into(),report:Artifact::self_contained(qualification,ArtifactFormat::Json,1),limitations:vec!["Exactly completed88448->88449 with the observed empty gameplay-command schedule and original exclusions. Qualification is reused by hash, not rerun by the fast adapter.".into(),"Complete mapped Snapshot wire schema is part of this contract. Representation changes require requalification.".into()]};
    case.schedule = vec![ScheduleStep::WaitCompletedTicks { count: 1 }];
    for (label, kind, blob, tick) in [
        ("baseline", ReferenceKind::Resumable, baseline, 88448),
        ("endpoint", ReferenceKind::Observed, endpoint, 88449),
    ] {
        case.references.push(Reference {
            label: label.into(),
            kind: kind.clone(),
            artifact: Artifact::self_contained(blob, ArtifactFormat::MessagePack, 1),
            resume_contract: if kind == ReferenceKind::Resumable {
                Some(CONTRACT.into())
            } else {
                None
            },
            boundary: Boundary::CompletedTick { tick },
        });
    }
    target.publish_case(&case)?;
    fs::remove_dir_all(scratch)?;
    Ok(
        json!({"case":id,"request":request_blob,"declared_blobs":request.dependencies().len(),"baseline":request.baseline,"frames":request.frames.len(),"suspended_states":request.cold_cuts.len(),"qualification_reused":true,"runtime_executed":false}),
    )
}
