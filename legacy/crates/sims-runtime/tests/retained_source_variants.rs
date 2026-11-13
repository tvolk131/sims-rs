//! Current-engine differential checks; historical Rust cuts are not native goldens.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::{BlobRef, Store};
use sims_runtime::original_session::completed_assets::checkpoints::Retained;
use std::path::{Path, PathBuf};

fn retained(report: &Value, store: &Store, index: usize) -> Result<Retained> {
    let reference: BlobRef = serde_json::from_value(if index == 0 {
        report["baseline_retained_cut"].clone()
    } else {
        let row = &report["boundary_sequence"][index - 1];
        ensure!(
            row["boundary"] == index
                && row["fields"]["differences"] == 0
                && row["dispatches"].get("error").is_none(),
            "current source variant did not finish a matched boundary"
        );
        row["retained_cut"].clone()
    })?;
    let cut: Retained = store.get_value(&reference)?;
    ensure!(
        cut.simulation.state.vm.tick == 89073 + index as u64,
        "source-variant cut is not the requested completed boundary"
    );
    Ok(cut)
}
fn load(path: &Path) -> Result<(Value, Store, String)> {
    let encoded = std::fs::read(path)?;
    Ok((
        serde_json::from_slice(&encoded)?,
        Store::open(path.with_extension("store"))?,
        sims_fixtures::digest(&encoded),
    ))
}

#[test]
#[ignore = "private current-engine C1 replays with restricted versus expanded source closures"]
fn source_closure_extension_preserves_all_retained_owners_at_baseline_and_c1() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let directory = root.join("fixtures/private/goth-heading-20261031-first");
    let (full, full_store, full_sha) = load(&directory.join("sequence-heading-second.json"))?;
    let (restricted, restricted_store, restricted_sha) =
        load(&directory.join("sequence-source-control-first.json"))?;
    ensure!(
        !full["resident_routine_sources"].is_null()
            && !full["resident_constant_sources"].is_null()
            && restricted["resident_routine_sources"].is_null()
            && restricted["resident_constant_sources"].is_null(),
        "source-variant reports do not describe distinct closure configurations"
    );
    // Canonical copies deduplicate against the already published full cuts.
    // Preserve a differing blob as diagnostic evidence rather than replacing it.
    let canonical = &full_store;
    let mut rows = Vec::new();
    for index in [0, 1] {
        let a = retained(&full, &full_store, index)?;
        let mut b = retained(&restricted, &restricted_store, index)?;
        ensure!(
            a.simulation.version == b.simulation.version
                && a.simulation.program_hash != b.simulation.program_hash,
            "source extension did not change exactly the expected Program binding metadata"
        );
        let bindings =
            json!({"expanded":a.simulation.program_hash,"restricted":b.simulation.program_hash});
        // Program manifests differ by design. Everything else, including every
        // VM owner, RNG, suspended continuation, asset and float word, must agree.
        b.simulation.program_hash = a.simulation.program_hash.clone();
        ensure!(
            a == b,
            "source extension changed typed retained owners at boundary {index}"
        );
        let a_ref = canonical.put_value(&a)?;
        let b_ref = canonical.put_value(&b)?;
        ensure!(
            a_ref == b_ref,
            "source extension changed exact retained serialization at boundary {index}"
        );
        rows.push(json!({"boundary":index,"tick":a.simulation.state.vm.tick,"program_bindings":bindings,"normalized_retained":a_ref,"matched":true}));
    }
    let report = json!({"schema":1,"kind":"current_engine_source_extension_differential","native_parity_credit":0,
        "expanded_report_sha256":full_sha,"restricted_report_sha256":restricted_sha,"rows":rows,
        "qualification":"Current-engine source-closure invariant only. Native boundary comparisons and gameplay admission are separate."});
    if let Some(path) = std::env::var_os("SIMS_RETAINED_SOURCE_DIAG_OUTPUT") {
        use std::io::Write;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        output.write_all(&serde_json::to_vec_pretty(&report)?)?;
    }
    eprintln!(
        "source extension preserves all baseline/C1 retained owners; Program metadata differs"
    );
    Ok(())
}
