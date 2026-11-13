use super::*;
use anyhow::{Context, ensure};
use sha2::{Digest, Sha256};
use sims_formats::avatar::{Skeleton, catalog::Asset, index::Kind};
use std::{path::PathBuf, time::Instant};

pub(super) fn pinned_catalog(
    memo: &mut Memo,
    index: &Arc<Index>,
    reference: &Tape,
) -> Result<(Loaded, Directories)> {
    ensure!(
        Arc::ptr_eq(&memo.index, index),
        "screen static index changed"
    );
    memo.installed_catalog_using(
        index,
        |_| anyhow::bail!("screen startup namespace changed or reference entry absent"),
        |tape, limit| {
            ensure!(
                tape.same_reference(reference),
                "screen startup reference replaced"
            );
            match tape.validate(limit)? {
                Validation::Unchanged => Ok(Validation::Unchanged),
                Validation::Changed(_) => anyhow::bail!("screen startup inputs changed"),
            }
        },
    )
}

fn selected(memo: &mut Memo, index: &Arc<Index>, reference: &Tape) -> Result<(Skeleton, usize)> {
    let (mut installed, directories) = pinned_catalog(memo, index, reference)?;
    let record = installed
        .catalog
        .resolve(index, &directories, Kind::Skeleton, b"adult")?
        .context("adult absent from memoized startup registry")?;
    let Asset::Skeleton(skeleton) = &record.value else {
        anyhow::bail!("memoized registry returned another asset kind");
    };
    Ok((skeleton.clone(), record.source))
}

#[test]
#[ignore = "private startup-result reuse screen; not native gameplay or qualification"]
fn installed_skeleton_validated_startup_memo_screen() -> Result<()> {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted")
        });
    let index = Arc::new(Index::scan(&root)?);
    let expected = crate::avatar::load_skeleton(&index, "adult")?;
    let source_sha256 = format!("{:x}", Sha256::digest(index.resources[expected.1].read()?));
    let skeleton_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&expected.0)?));
    let baseline = crate::clothing_catalog::installed_catalog(&index)?;
    let baseline_state_sha256 = format!("{:x}", Sha256::digest(format!("{baseline:?}")));
    drop(baseline);
    let mut memo = Memo::new(index.clone());
    let started = Instant::now();
    let cold = memo.installed_catalog(&index)?;
    let cold_us = started.elapsed().as_micros();
    ensure!(
        format!("{:x}", Sha256::digest(format!("{cold:?}"))) == baseline_state_sha256,
        "cold memo changed complete startup state"
    );
    drop(cold);
    let entry = memo
        .entry
        .as_ref()
        .context("startup transcript exceeded its private retention bounds")?;
    let reference = entry.tape.clone();
    let namespace = entry.namespace.clone();
    let (events, retained_bytes) = reference.statistics();
    ensure!(
        selected(&mut memo, &index, &reference)? == expected,
        "memo warmup changed selected skeleton/source"
    );
    ensure!(
        crate::avatar::load_skeleton(&index, "adult")? == expected,
        "baseline warmup changed selection"
    );
    let mut records = Vec::with_capacity(24);
    for (block, order) in ["ABBA", "BAAB"].into_iter().enumerate() {
        for repetition in 0..3 {
            for (position, variant) in order.chars().enumerate() {
                let started = Instant::now();
                let actual = if variant == 'A' {
                    crate::avatar::load_skeleton(&index, "adult")?
                } else {
                    selected(&mut memo, &index, &reference)?
                };
                let elapsed_us = started.elapsed().as_micros();
                ensure!(
                    actual == expected,
                    "timed selection changed skeleton/source"
                );
                records.push(serde_json::json!({
                    "block": block, "order": order, "repetition": repetition,
                    "position": position, "variant": variant.to_string(), "elapsed_us": elapsed_us,
                }));
            }
        }
    }
    let entry = memo
        .entry
        .as_ref()
        .context("memo lost eligibility during unchanged corpus screen")?;
    ensure!(
        entry.tape.same_reference(&reference),
        "screen startup reference replaced"
    );
    ensure!(
        matches!(reference.validate(MAX_BYTES)?, Validation::Unchanged),
        "startup sources changed during screen"
    );
    let installation = Installation::new(&index, complete_collection_asset_bindings(&index.root))?;
    ensure!(
        installation.namespace() == namespace,
        "namespace changed during screen"
    );
    ensure!(
        format!("{:x}", Sha256::digest(index.resources[expected.1].read()?)) == source_sha256,
        "selected source changed during screen"
    );
    println!(
        "startup_memo_probe {}",
        serde_json::json!({
            "schema": 1, "scope": "Isolated release operation screen; complete warm-process load_skeleton calls including fresh namespace, every ordered source read, exact byte comparison, template cloning, lazy selection and temporary teardown. Not native gameplay, CPU utilization or qualification.",
            "variants": {"A": "ordinary source-backed reconstruction", "B": "validated startup-result memo"},
            "cold_us": cold_us, "indexed_resources": index.resources.len(), "transcript_events": events,
            "transcript_retention_charge_bytes": retained_bytes, "retention_limit_bytes": MAX_BYTES,
            "source_sha256": source_sha256, "skeleton_sha256": skeleton_sha256,
            "complete_startup_state_sha256": baseline_state_sha256, "records": records,
        })
    );
    Ok(())
}
