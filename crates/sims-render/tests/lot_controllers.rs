//! Real purchased furniture, native helper factory/placement and cold resume.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Session, controllers::Progress};
use std::{collections::BTreeSet, path::Path};

#[test]
#[ignore = "private original-house purchase checkpoints and immutable assets"]
fn original_controller_reconciliation_creates_and_places_helpers_across_cold_resume() -> Result<()>
{
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("controller test panicked")
}
fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = root.join(std::env::var("SIMS_CONTROLLERS_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let immutable =
        Index::scan(&root.join("cache/managed-independent-20261022-third/immutable-assets"))?;
    let mut reports = vec![];
    for (name, path) in [
        (
            "newbie",
            std::env::var("SIMS_CONTROLLER_NEWBIE")
                .unwrap_or_else(|_| "cache/controller-newbie-20261022/buy.json".into()),
        ),
        (
            "goth-purchase",
            std::env::var("SIMS_CONTROLLER_GOTH")
                .unwrap_or_else(|_| "cache/controller-goth-purchase-20261022/session.json".into()),
        ),
    ] {
        let source: Value = serde_json::from_slice(&std::fs::read(root.join(path))?)?;
        let mut initial =
            Session::restore(&index, serde_json::from_value(source["session"].clone())?)?;
        let initial_device = Device::restore(
            &initial,
            serde_json::from_value(source["camera_device"].clone())?,
        )?;
        let before_ids: BTreeSet<_> = initial
            .simulation
            .vm
            .world
            .objects
            .keys()
            .copied()
            .collect();
        let tick = initial.simulation.vm.tick;
        initial.begin_original_controllers()?;
        let baseline = initial.save_managed()?;
        let device_baseline = initial_device.save(&initial)?;
        let mut finals = vec![];
        for resumed in [false, true] {
            let mut s = Session::restore(&immutable, baseline.clone())?;
            let mut d = Device::restore(&s, device_baseline.clone())?;
            let mut checkpoints = BTreeSet::new();
            let mut complete = false;
            for n in 0..20000 {
                if resumed {
                    let e = serde_json::to_value(&s.simulation.controller_reconciliation)?;
                    let phase = e["phase"].as_str().unwrap_or("absent");
                    let callback = s
                        .simulation
                        .vm
                        .threads
                        .values()
                        .any(|t| t.callback.is_some());
                    let label = format!("{phase}{}", if callback { "-callback" } else { "" });
                    if ["Factory", "Placement", "Complete"].contains(&phase)
                        && checkpoints.insert(label.clone())
                    {
                        let saved = s.save_managed()?;
                        let dev = d.save(&s)?;
                        std::fs::write(
                            out.join(format!("{name}-{label}.json")),
                            serde_json::to_vec(&saved)?,
                        )?;
                        let mut invalid = serde_json::to_value(&saved)?;
                        invalid["simulation"]["state"]["controller_reconciliation"]["tick"] =
                            json!(tick + 1);
                        ensure!(
                            Session::restore(&immutable, serde_json::from_value(invalid)?).is_err(),
                            "future-tick controller checkpoint accepted"
                        );
                        s = Session::restore(&immutable, saved)?;
                        d = Device::restore(&s, dev)?;
                    }
                }
                match s.advance_original_controllers(if resumed { 1 } else { 500 })? {
                    Progress::Pending => (),
                    Progress::Service(_) => {
                        d.advance_notification(&mut s, 1)?;
                    }
                    Progress::Complete => {
                        s.finish_original_controllers()?;
                        complete = true;
                        break;
                    }
                }
                ensure!(
                    s.simulation.vm.tick == tick,
                    "controller maintenance advanced time"
                );
                ensure!(n < 19999, "controller reconciliation stalled");
            }
            ensure!(complete);
            let created: Vec<_> = s
                .simulation
                .vm
                .world
                .objects
                .keys()
                .filter(|id| !before_ids.contains(id))
                .copied()
                .collect();
            eprintln!("{name} resumed={resumed} created={created:?} checkpoints={checkpoints:?}");
            if name == "goth-purchase" {
                ensure!(
                    !created.is_empty(),
                    "purchased object did not create its helper"
                );
            }
            let saved = s.save_managed()?;
            let restored = Session::restore(&immutable, saved.clone())?;
            ensure!(serde_json::to_vec(&s.save())? == serde_json::to_vec(&restored.save())?);
            let ids = s
                .simulation
                .vm
                .world
                .objects
                .keys()
                .copied()
                .collect::<BTreeSet<_>>();
            s.begin_original_controllers()?;
            for _ in 0..20000 {
                match s.advance_original_controllers(500)? {
                    Progress::Complete => {
                        s.finish_original_controllers()?;
                        break;
                    }
                    Progress::Pending => (),
                    Progress::Service(_) => {
                        d.advance_notification(&mut s, 1)?;
                    }
                }
            }
            ensure!(s.simulation.controller_reconciliation.is_none());
            ensure!(
                s.simulation
                    .vm
                    .world
                    .objects
                    .keys()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    == ids,
                "second reconciliation duplicated helpers"
            );
            ensure!(s.simulation.vm.tick == tick);
            d.refresh(&s)?;
            let image = software::render(d.scene());
            if resumed {
                image.save(out.join(format!("{name}.png")))?;
            }
            finals.push((serde_json::to_vec(&s.save())?, d.save(&s)?, image));
            reports.push(json!({"name":name,"resumed":resumed,"tick":tick,"created":created,
                "checkpoints":checkpoints,"repeat_did_not_duplicate":true,"independent_restore":true}));
        }
        ensure!(
            finals[0] == finals[1],
            "controller result differs after cold restore"
        );
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "cases":reports,"equal":true,"original_gameplay_ticks_added":0,
            "scope":"Explicit original40726a service on paused real-house checkpoints. Native factory/placement and cold resume; not completed Live/Buy/Live or original gameplay parity."
        }))?,
    )?;
    Ok(())
}
