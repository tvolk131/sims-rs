//! Actual managed Buy checkpoint: both sides of the43dfc4 device boundary.
use anyhow::{Result, bail, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{
    Session,
    tool_selection::{Progress, Service},
};
use sims_vm::interface::mode_change::Request;
use std::path::Path;

#[test]
#[ignore = "private managed Buy checkpoint and immutable assets"]
fn mode_definition_refresh_rebuilds_geometry_without_running_or_resetting_the_vm() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("definition refresh test panicked")
}

fn snapshot(s: &Session, d: &Device) -> Result<Value> {
    Ok(json!({"session":s.save_managed()?, "camera_device":d.save(s)?}))
}

fn restore(index: &Index, value: &Value) -> Result<(Session, Device)> {
    let s = Session::restore(index, serde_json::from_value(value["session"].clone())?)?;
    let d = Device::restore(&s, serde_json::from_value(value["camera_device"].clone())?)?;
    ensure!(
        snapshot(&s, &d)? == *value,
        "managed device restore differs"
    );
    Ok((s, d))
}

fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = root.join(std::env::var("SIMS_DEFINITION_REFRESH_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir(&out)?;
    let index =
        Index::scan(&root.join("cache/managed-independent-20261022-third/immutable-assets"))?;
    let input = root.join("cache/live-house-refresh-iced-20261022-second/buy.json");
    let value: Value = serde_json::from_slice(&std::fs::read(&input)?)?;
    let s = Session::restore(&index, serde_json::from_value(value["session"].clone())?)?;
    let d = Device::restore(&s, serde_json::from_value(value["camera_device"].clone())?)?;
    let (mut s, mut d) = restore(&index, &snapshot(&s, &d)?)?;
    let before = snapshot(&s, &d)?;
    ensure!(s.acknowledge_mode_definition_refresh().is_err());
    ensure!(
        snapshot(&s, &d)? == before,
        "invalid acknowledgment changed state"
    );
    s.begin_mode_change(2, 0)?;
    for n in 0..100 {
        match s.advance_tool_selection(1)? {
            Progress::Pending => (),
            Progress::Service(Service::Mode(Request::RefreshObjects)) => break,
            other => bail!("unexpected mode boundary before resource refresh: {other:?}"),
        }
        ensure!(n < 99, "mode did not reach definition refresh");
    }
    let pending = snapshot(&s, &d)?;
    std::fs::write(out.join("pending.json"), serde_json::to_vec(&pending)?)?;
    let simulation = s.simulation.clone();
    ensure!(s.step().is_err(), "mode transition allowed gameplay");
    ensure!(snapshot(&s, &d)? == pending);
    let (mut cold_s, mut cold_d) = restore(&index, &pending)?;
    let pixels = software::render(d.scene());
    let generation = pending["camera_device"]["generation"].as_u64().unwrap();
    for (session, device) in [(&mut s, &mut d), (&mut cold_s, &mut cold_d)] {
        ensure!(!device.advance_tool_selection(session, 1)?);
        ensure!(
            session.simulation == simulation,
            "definition refresh changed simulation/RNG"
        );
        let saved = device.save(session)?;
        ensure!(serde_json::to_value(&saved)?["generation"].as_u64() == Some(generation + 1));
        ensure!(
            software::render(device.scene()) == pixels,
            "unpublished surface changed"
        );
        let after = snapshot(session, device)?;
        ensure!(session.acknowledge_mode_definition_refresh().is_err());
        ensure!(
            snapshot(session, device)? == after,
            "duplicate acknowledgment changed state"
        );
    }
    ensure!(snapshot(&s, &d)? == snapshot(&cold_s, &cold_d)?);
    let after = snapshot(&s, &d)?;
    restore(&index, &after)?;
    std::fs::write(out.join("after.json"), serde_json::to_vec(&after)?)?;
    let boundary = loop {
        match s.advance_tool_selection(1)? {
            Progress::Pending => (),
            Progress::Service(Service::Mode(Request::TimedResourceUpdates(clock))) => break clock,
            other => bail!("unexpected boundary after definition refresh: {other:?}"),
        }
    };
    let timed = snapshot(&s, &d)?;
    ensure!(
        s.acknowledge_mode_timed_resources(boundary.wrapping_add(1))
            .is_err()
    );
    ensure!(
        snapshot(&s, &d)? == timed,
        "wrong-clock reply changed state"
    );
    let simulation = s.simulation.clone();
    let pixels = software::render(d.scene());
    let (mut restored_s, mut restored_d) = restore(&index, &timed)?;
    for (session, device) in [(&mut s, &mut d), (&mut restored_s, &mut restored_d)] {
        ensure!(!device.advance_tool_selection(session, 1)?);
        ensure!(
            session.simulation == simulation,
            "cache reclamation changed simulation/RNG"
        );
        ensure!(
            serde_json::to_value(&session.save().assets)? == timed["session"]["assets"],
            "cache reclamation changed active asset, skeleton or geometry ownership"
        );
        ensure!(
            software::render(device.scene()) == pixels,
            "cache reclamation published a different surface"
        );
        let completed = snapshot(session, device)?;
        ensure!(session.acknowledge_mode_timed_resources(boundary).is_err());
        ensure!(snapshot(session, device)? == completed);
    }
    ensure!(snapshot(&s, &d)? == snapshot(&restored_s, &restored_d)?);
    restore(&index, &snapshot(&s, &d)?)?;
    std::fs::write(
        out.join("after-reclamation.json"),
        serde_json::to_vec(&snapshot(&s, &d)?)?,
    )?;
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "input":input, "tick":s.simulation.vm.tick, "application_clock":boundary,
            "independent_restore_before_and_after":true, "simulation_and_rng_unchanged":true,
            "geometry_rebuilt":true, "published_pixels_unchanged":true,
            "stale_acknowledgments_rejected":true, "original_gameplay_ticks_added":0,
            "qualifying_4096_scenarios":0,
        "runtime_cache_reclamation_preserves_simulation_and_rng":true,
        "next_boundary":"Mode(ReconcileLotControllers)",
        "scope":"Actual managed Newbie Buy->Live resource refresh and renderer-cache reclamation, with independent cold restore. Native cache allocation lifetimes are not reproduced. Controller reconciliation is tested separately."
        }))?,
    )?;
    Ok(())
}
