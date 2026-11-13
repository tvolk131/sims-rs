//! Publish a complete caller-authored manifest/run through the existing Store
//! validators. Blobs must already exist; case and run IDs remain immutable.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use sims_fixtures::{CaseManifest, Compression, RunReport, Store};
use std::io::Read;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    store: PathBuf,
    #[arg(long, required_unless_present = "run")]
    manifest: Option<PathBuf>,
    #[arg(long)]
    run: Option<PathBuf>,
    /// Optional new artifact files for the manifest/run's predeclared blob refs.
    #[arg(long)]
    blob: Vec<PathBuf>,
}

fn bounded_json<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Result<T> {
    let file = std::fs::File::open(path).with_context(|| path.display().to_string())?;
    let mut bytes = Vec::new();
    file.take((4 << 20) + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 4 << 20, "registration JSON exceeds 4 MiB");
    Ok(serde_json::from_slice(&bytes)?)
}

fn main() -> Result<()> {
    let args = Args::parse();
    let store = Store::open(&args.store)?;
    for path in args.blob {
        let blob = store.put_file(path, Compression::Gzip)?;
        println!("stored artifact {} {}", blob.sha256, blob.bytes);
    }
    if let Some(path) = args.manifest {
        let case: CaseManifest = bounded_json(&path)?;
        store.publish_case(&case)?;
        println!("published case {}", case.id);
    }
    if let Some(path) = args.run {
        let report: RunReport = bounded_json(&path)?;
        store.record_run(&report)?;
        println!("recorded run {}", report.id);
    }
    Ok(())
}
