use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use sims_content::{Index, avatar};
use std::{path::PathBuf, time::Instant};

#[test]
#[ignore = "private installed registry reconstruction diagnostic; not native qualification"]
fn installed_skeleton_registry_reconstruction() -> Result<()> {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted")
        });
    let repetitions: usize = std::env::var("SIMS_REGISTRY_PROBE_REPETITIONS")
        .unwrap_or_else(|_| "5".into())
        .parse()?;
    ensure!(
        (3..=20).contains(&repetitions),
        "expected 3..=20 repetitions"
    );
    let label = std::env::var("SIMS_REGISTRY_PROBE_LABEL").unwrap_or_else(|_| "current".into());
    let index = Index::scan(&root)?;
    let (expected, source) = avatar::load_skeleton(&index, "adult")?;
    let source_sha256 = format!("{:x}", Sha256::digest(index.resources[source].read()?));
    let skeleton_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&expected)?));
    for repetition in 0..repetitions {
        let started = Instant::now();
        let (actual, actual_source) = avatar::load_skeleton(&index, "adult")?;
        let elapsed_us = started.elapsed().as_micros();
        assert_eq!(actual, expected);
        assert_eq!(actual_source, source);
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(index.resources[actual_source].read()?)
            ),
            source_sha256
        );
        println!(
            "registry_probe {}",
            serde_json::json!({
                "label": label,
                "repetition": repetition,
                "elapsed_us": elapsed_us,
                "indexed_resources": index.resources.len(),
                "source_sha256": source_sha256,
                "skeleton_sha256": skeleton_sha256,
                "scope": "one warm-process production load_skeleton call, including fresh installation/provider/registry reconstruction and source validation; excludes initial indexing, one warmup, output checks and returned-value teardown; not native gameplay, CPU utilization or qualification"
            })
        );
    }
    Ok(())
}
