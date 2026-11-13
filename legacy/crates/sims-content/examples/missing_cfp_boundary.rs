//! Read-only comparison of missing sample streams at the current content boundary.
//! This records loader errors, retries and state preservation; it does not supply
//! fallback samples or alter the runtime's interpretation of those errors.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_content::{Index, assets::load_named_native_animations};
use sims_vm::Program;
use std::{collections::BTreeSet, fs, path::PathBuf};

#[derive(Deserialize)]
struct Inventory {
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    name: String,
    cfp: String,
}

#[derive(Serialize)]
struct Attempt {
    error: String,
    program_unchanged: bool,
    program_sha256: String,
}

#[derive(Serialize)]
struct Case {
    name: String,
    cfp: String,
    attempts: Vec<Attempt>,
}

#[derive(Serialize)]
struct Report {
    root: PathBuf,
    inventory: PathBuf,
    inventory_sha256: String,
    control: String,
    control_source: String,
    control_samples_source: String,
    entries: Vec<Case>,
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 3,
        "usage: missing_cfp_boundary ROOT INVENTORY OUT"
    );
    let root = PathBuf::from(&args[0]);
    let inventory = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    ensure!(!out.exists(), "refusing to overwrite {}", out.display());
    let input = fs::read(&inventory)?;
    let source: Inventory = serde_json::from_slice(&input)?;
    let index = Index::scan(&root)?;
    ensure!(
        index.diagnostics.is_empty(),
        "index diagnostics: {:?}",
        index.diagnostics
    );
    let mut program = Program::default();
    let control = "a2o-bored1".to_owned();
    let loaded =
        load_named_native_animations(&index, &mut program, &BTreeSet::from([control.clone()]))?;
    ensure!(loaded.loaded_clips == 1 && loaded.unresolved.is_empty());
    let control_asset = program
        .animations
        .get(&control)
        .context("control missing")?;
    let control_source = control_asset.source.clone();
    let control_samples_source = control_asset.samples_source.clone();
    let before = serde_json::to_value(&program)?;
    let mut entries = Vec::new();
    for entry in source.entries {
        let mut attempts = Vec::new();
        for _ in 0..2 {
            let result = load_named_native_animations(
                &index,
                &mut program,
                &BTreeSet::from([entry.name.clone()]),
            );
            let error = format!("{:#}", result.expect_err("expected absent CFP to fail"));
            let after = serde_json::to_value(&program)?;
            let program_unchanged = after == before;
            ensure!(
                program_unchanged,
                "program changed after loading {}",
                entry.name
            );
            attempts.push(Attempt {
                error,
                program_unchanged,
                program_sha256: format!("{:x}", Sha256::digest(serde_json::to_vec(&after)?)),
            });
        }
        entries.push(Case {
            name: entry.name,
            cfp: entry.cfp,
            attempts,
        });
    }
    let report = Report {
        root,
        inventory,
        inventory_sha256: format!("{:x}", Sha256::digest(&input)),
        control,
        control_source,
        control_samples_source,
        entries,
    };
    fs::write(&out, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{} missing sample names, {} attempts, program preserved; {}",
        report.entries.len(),
        report.entries.len() * 2,
        out.display()
    );
    Ok(())
}
