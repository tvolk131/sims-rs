//! Bounded no-input diagnostic continuation of an independently managed save.
//! It cannot supply an unobserved original input history or certify parity.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Progress, Session};
use std::{fs::File, io::Write, path::Path};

#[allow(dead_code)]
#[path = "newbie_parity.rs"]
mod observation;

fn main() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("retained-owner diagnostic panicked"))?
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 4,
        "CONTENT_ROOT MANAGED_SAVE NEW_OUTPUT_DIR THROUGH_TICK"
    );
    let out = Path::new(&args[2]);
    ensure!(
        out.parent()
            .context("output parent")?
            .canonicalize()?
            .starts_with(Path::new("cache").canonicalize()?),
        "private output required"
    );
    std::fs::create_dir(out).context("refusing to replace evidence")?;
    let index = Index::scan(Path::new(&args[0]))?;
    ensure!(
        index
            .resources
            .iter()
            .all(|r| !r.locator.to_ascii_lowercase().starts_with("userdata")),
        "immutable-only installation required"
    );
    let mut session = Session::restore(&index, serde_json::from_reader(File::open(&args[1])?)?)?;
    let start = session.simulation.vm.tick;
    let through: u64 = args[3].parse()?;
    ensure!(
        through >= start && through - start <= 4096,
        "invalid diagnostic tick extent"
    );
    let mut trace = std::io::BufWriter::new(File::create_new(out.join("ticks.jsonl"))?);
    let mut observations = 0;
    loop {
        let before = serde_json::to_value(session.save())?;
        let observed = observation::state(&session);
        ensure!(
            before == serde_json::to_value(session.save())?,
            "observation changed retained state"
        );
        serde_json::to_writer(&mut trace, &observed)?;
        writeln!(trace)?;
        trace.flush()?;
        observations += 1;
        if session.simulation.vm.tick == through {
            break;
        }
        let previous_tick = session.simulation.vm.tick;
        let mut slices = 0;
        loop {
            slices += 1;
            ensure!(slices <= 100_000, "diagnostic partial-pass bound exhausted");
            match session.step()? {
                Progress::Tick => break,
                Progress::PartialPass => (),
                progress => anyhow::bail!("unobserved external service: {progress:?}"),
            }
        }
        ensure!(
            session.simulation.vm.tick == previous_tick + 1,
            "diagnostic skipped a completed tick"
        );
        ensure!(
            session.simulation.vm.tick >= start && session.simulation.vm.tick <= through,
            "diagnostic skipped a tick boundary"
        );
    }
    serde_json::to_writer(
        File::create_new(out.join("final-state.json"))?,
        &observation::state(&session),
    )?;
    serde_json::to_writer(
        File::create_new(out.join("final-managed.json"))?,
        &session.save_managed()?,
    )?;
    let report = serde_json::json!({"scope":"No-input Rust diagnostic; original gap/input history remains unobserved",
        "source":args[1],"content_root":args[0],"start_tick":start,"through_tick":through,
        "observed_boundaries":observations,"completed_rust_ticks":through-start,
        "strict_original_parity_qualified":false,"qualified_original_ticks":0});
    serde_json::to_writer_pretty(File::create_new(out.join("report.json"))?, &report)?;
    println!("{report}");
    Ok(())
}
