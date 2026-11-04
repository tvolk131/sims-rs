//! Headless conformance front door; original capture is never launched here.
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use sims_runtime::conformance::runner;
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Parser)]
#[command(about = "Headless fixture conformance; evidence is not automatically parity")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Run {
        #[arg(long)]
        suite: PathBuf,
        #[arg(long = "case")]
        cases: Vec<String>,
        #[arg(long, default_value_t = 2)]
        jobs: usize,
        #[arg(long, default_value_t = 2048)]
        memory_mib: u32,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    #[command(hide = true)]
    Worker {
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Worker { request, output } => {
            runner::run_worker(runner::read_json(&request)?, &output)?;
        }
        Command::Run {
            mut suite,
            cases,
            jobs,
            memory_mib,
            output,
        } => {
            if !suite.exists() && suite.components().count() == 1 && suite.extension().is_none() {
                suite = PathBuf::from("fixtures/suites")
                    .join(suite)
                    .with_extension("json");
            }
            if output.is_none() {
                std::fs::create_dir_all("fixtures/private/conformance-runs")?;
            }
            let output = output.unwrap_or_else(|| {
                PathBuf::from("fixtures/private/conformance-runs").join(format!(
                    "conformance-{}-{}",
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos(),
                    std::process::id()
                ))
            });
            let report = runner::run_suite(&suite, &cases, &output, jobs, memory_mib)
                .with_context(|| format!("suite {}", suite.display()))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "suite": report.suite, "success": report.success, "counts": report.counts,
                    "report": output.join("suite-report.json"), "qualification": report.qualification,
                }))?
            );
            if !report.success {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}
