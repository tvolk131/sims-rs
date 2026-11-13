//! Retained Buy -> Live selection/publication boundary, without GUI or ticks.
use super::*;
use clap::Parser;
use serde_json::json;
use std::io::Write;

fn observation(v: &Viewer) -> Result<serde_json::Value> {
    let s = v.session.as_ref().context("session absent")?;
    let p = &s.host.presentation;
    Ok(json!({
        "tick":s.simulation.vm.tick,
        "selected":s.simulation.native_selected_person().map_err(anyhow::Error::msg)?,
        "selected_index":p.transport.as_ref().and_then(|t|t.controls.selected_person_index),
        "dirty":p.transport.as_ref().map(|t|t.controls.buy.dirty),
        "publication":p.publication,
        "current_tool":p.current_tool.as_ref().map(|t|json!({"current":t.current(),"request":t.request()})),
        "selection":s.simulation.native_selection,
        "client_person":v.client.as_ref().and_then(|c|c.live_panel()).and_then(|p|p.person()).map(|p|p.object),
    }))
}

#[test]
#[ignore = "private retained picker selection checkpoint; no original guest"]
fn retained_picker_selection_publication_trace() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("selection trace panicked")
}

fn run() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_SELECTION_TRACE_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let input = repo.join("cache/controller-live-newbie-20261022-reentry/picker-selection.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        repo.join("game-data/cabinet-extracted").to_str().unwrap(),
        "--load",
        input.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    let start = observation(&v)?;
    ensure!(start["selected"] == 105 && v.session.as_ref().unwrap().publication_pending());
    let tick = v.session.as_ref().unwrap().simulation.vm.tick;
    let mut ledger = std::fs::File::create(output.join("trace.jsonl"))?;
    let mut previous = start;
    writeln!(ledger, "{}", json!({"slice":0,"state":previous}))?;
    let mut clearing = None;
    for slice in 1..10000 {
        use sims_runtime::original_session::publication::Progress as P;
        let s = v.session.as_mut().unwrap();
        if !s.publication_pending() {
            break;
        }
        let device = v.device.as_mut().unwrap();
        match device.advance_publication(s, 1)? {
            P::Complete | P::Pending => (),
            P::Panels(_) if device.publish_replacement_people(s, v.client.as_mut().unwrap())? => (),
            other => anyhow::bail!("selection trace publication service: {other:?}"),
        }
        let current = observation(&v)?;
        ensure!(
            v.session.as_ref().unwrap().simulation.vm.tick == tick,
            "presentation advanced gameplay"
        );
        if current != previous {
            writeln!(ledger, "{}", json!({"slice":slice,"state":current}))?;
            if previous["selected"] == 105 && current["selected"].is_null() && clearing.is_none() {
                clearing = Some(slice);
                std::fs::write(
                    output.join("first-clear-before.json"),
                    serde_json::to_vec_pretty(&previous)?,
                )?;
                std::fs::write(
                    output.join("first-clear-after.json"),
                    serde_json::to_vec_pretty(&current)?,
                )?;
            }
            previous = current;
        }
    }
    ensure!(
        !v.session.as_ref().unwrap().publication_pending(),
        "publication did not finish"
    );
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "input":input,"tick":tick,"final":previous,"first_clear_slice":clearing,
            "completed_gameplay_ticks":0,"original_parity_ticks_added":0
        }))?,
    )?;
    Ok(())
}
