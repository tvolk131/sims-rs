//! Compare explicitly aligned screenshot viewports, or measure original-capture variance.
#[path = "support/screenshot_compare.rs"]
mod comparison;
use anyhow::{Result, ensure};
use std::path::Path;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 4,
        "usage: screenshot_compare compare|variance JOB.json NEW_OUTPUT_DIRECTORY"
    );
    let input = Path::new(&args[2]);
    let base = input.parent().unwrap_or(Path::new("."));
    let out = Path::new(&args[3]);
    let bytes = std::fs::read(input)?;
    match args[1].as_str() {
        "compare" => {
            let job: comparison::Job = serde_json::from_slice(&bytes)?;
            let status = comparison::write_comparison(&job, base, out)?;
            println!("{status:?}: {}", out.join("report.json").display());
            match status {
                comparison::Status::Passed | comparison::Status::Illustrative => (),
                comparison::Status::Failed => std::process::exit(1),
                comparison::Status::NotComparable => std::process::exit(2),
            }
        }
        "variance" => {
            ensure!(!out.exists(), "output directory must be fresh");
            let job: comparison::VarianceJob = serde_json::from_slice(&bytes)?;
            let report = comparison::variance(&job, base)?;
            std::fs::create_dir_all(out)?;
            std::fs::write(
                out.join("variance.json"),
                serde_json::to_vec_pretty(&report)?,
            )?;
            println!("Observed variance: {}", out.join("variance.json").display());
        }
        _ => anyhow::bail!("expected compare or variance"),
    }
    Ok(())
}
