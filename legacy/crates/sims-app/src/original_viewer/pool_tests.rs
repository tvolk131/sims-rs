//! Independent research checkpoint: dive, pool locomotion and ladder exit.
//! No player save/GUI or original guest is accessed.
use super::*;
use clap::Parser;
use serde_json::{Value, json};
use std::io::Write;

fn observe(v: &Viewer) -> Value {
    let s = v.session.as_ref().unwrap();
    let o = &s.simulation.vm.world.objects[&196];
    let r = &s.simulation.characters[&196];
    let thread = s.simulation.vm.object_threads[&196];
    json!({"tick":s.simulation.vm.tick,
        "person":o.person.as_ref().map(|p|[p[&0],p[&8],p[&64],p[&65],p[&78]]),
        "position":s.simulation.vm.world.positions[&196],
        "ordinary":r.controls.ordinary,"movement":r.controls.movement.state,
        "animator":r.character.skin.animator,
        "placement":r.character.placement,"matrix":r.character.matrix,
        "root_local":r.character.skin.bones[0].local,
        "thread_id":thread,"thread":s.simulation.vm.threads.get(&thread),
        "rng":[s.simulation.vm.rng_state,s.simulation.vm.lot_rng_state,s.simulation.vm.crt_rng_state]})
}

#[test]
#[ignore = "private research Mashuga checkpoint; fresh SIMS_POOL_LOCOMOTION_OUTPUT"]
fn trace_original_dive_swim_and_ladder_exit() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("pool trace panicked")
}
fn retained(v: &Viewer) -> Result<Vec<u8>> {
    let s = v.session.as_ref().unwrap();
    Ok(serde_json::to_vec(&json!({"session":s.save(),
        "client":v.client.as_ref().unwrap().save(s)?,
        "device":v.device.as_ref().unwrap().save(s)?,
        "floor":v.floor,"zoom":v.zoom,"resident":v.resident,"selected":v.selected,
        "move_inputs":v.move_inputs,"enter":v.viewport_enter_requested,"tutorial":v.tutorial_highlight}))?)
}
fn equal(a: &Viewer, b: &Viewer, out: &Path, label: &str) -> Result<()> {
    let x = retained(a)?;
    let y = retained(b)?;
    if x != y {
        std::fs::write(out.join(format!("{label}-expected.json")), x)?;
        std::fs::write(out.join(format!("{label}-actual.json")), y)?;
        anyhow::bail!("pool continuation differs at{label}");
    }
    Ok(())
}
fn capture(a: &mut Viewer, b: &mut Option<Viewer>, out: &Path, label: &str) -> Result<()> {
    a.refresh()?;
    let before = retained(a)?;
    let scene = a.device.as_ref().unwrap().scene();
    let pixels = sims_render::software::render(scene);
    ensure!(
        pixels == sims_render::software::render(scene),
        "repeat pixels differ"
    );
    image::save_buffer(
        out.join(format!("{label}.png")),
        &pixels,
        scene.width,
        scene.height,
        image::ColorType::Rgba8,
    )?;
    ensure!(
        before == retained(a)?,
        "software draw changed full retained state"
    );
    if let Some(b) = b {
        b.refresh()?;
        equal(a, b, out, label)?;
        ensure!(
            pixels == sims_render::software::render(b.device.as_ref().unwrap().scene()),
            "cold pixels differ at{label}"
        );
    }
    std::fs::write(
        out.join(format!("{label}.json")),
        serde_json::to_vec(&a.session.as_ref().unwrap().save())?,
    )?;
    Ok(())
}
fn run() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = repo.join(std::env::var("SIMS_POOL_LOCOMOTION_OUTPUT")?);
    ensure!(out.starts_with(repo.join("cache")));
    std::fs::create_dir(&out)?;
    let root = repo.join("game-data/cabinet-extracted");
    let source = repo.join("cache/sale-20261022-seventh/mashuga/sold-returned-live.json");
    let raw: Value = serde_json::from_slice(&std::fs::read(&source)?)?;
    let old: Saved = serde_json::from_value(raw["session"].clone())?;
    let index = Index::scan(&root)?;
    let upgraded = Session::restore(&index, old.clone())?;
    ensure!(
        upgraded.simulation == old.simulation.state,
        "table hydration changed simulation/VM/RNG/continuations"
    );
    ensure!(
        upgraded.host.snapshot() == old.assets,
        "table hydration changed asset/binding state"
    );
    ensure!(
        upgraded.host.presentation == old.view,
        "table hydration changed presentation state"
    );
    ensure!(
        upgraded.program.fingerprint() != old.simulation.program_hash,
        "old incomplete dependency catalog was not upgraded"
    );
    let current = upgraded.save_managed()?;
    ensure!(
        serde_json::to_vec(&Session::restore(&index, current.clone())?.save_managed()?)?
            == serde_json::to_vec(&current)?,
        "current managed restore changed catalog/state"
    );
    // A version1 research checkpoint has no retained table list. Validate its
    // precise earlier whitelist fingerprint, then add fixed original tables.
    let mut legacy = old.clone();
    legacy.version = 1;
    legacy.managed = None;
    let legacy_upgraded = Session::restore(&index, legacy)?;
    ensure!(
        legacy_upgraded.simulation == old.simulation.state,
        "legacy hydration changed simulation"
    );
    ensure!(legacy_upgraded.program.fingerprint() == upgraded.program.fingerprint());
    let mut corrupt = old.clone();
    corrupt.simulation.program_hash = "untrusted".into();
    ensure!(
        Session::restore(&index, corrupt).is_err(),
        "catalog upgrade bypassed saved fingerprint"
    );
    let make = |load: &Path| -> Result<Viewer> {
        let args = Args::parse_from([
            "sims-room",
            "--original",
            "UserData2/Houses/House09.iff",
            "--paused",
            "--inferred-routing",
            "--root",
            root.to_str().unwrap(),
            "--load",
            load.to_str().unwrap(),
            "--save",
            out.join("research.json").to_str().unwrap(),
        ]);
        let ready = Arc::new(Ready::load(&args)?);
        let (mut v, _) = Viewer::new(args);
        v.install(ready)?;
        Ok(v)
    };
    let mut v = make(&source)?;
    let mut cold = None;
    let mut inputs = vec![];
    transport_tests::drain(&mut v, &mut inputs)?;
    v.prepare_and_publish()?;
    let s = v.session.as_ref().unwrap();
    let mut resources = serde_json::Map::new();
    for (source, module) in &s.program.modules {
        if source == &s.program.globals || source.to_ascii_lowercase().contains("pool") {
            resources.insert(source.clone(),json!({"source_hash":module.content_hash,"strings":module.strings,"routines":format!("{:#?}",module.routines)}));
        }
    }
    std::fs::write(
        out.join("resources.json"),
        serde_json::to_vec_pretty(&resources)?,
    )?;
    let start = s.simulation.vm.tick;
    let origin = s.host.presentation.clock.as_ref().unwrap().now;
    let target: u64 = std::env::var("SIMS_POOL_LOCOMOTION_TICKS")
        .unwrap_or("1000".into())
        .parse()?;
    ensure!(target >= 900, "workload must include ladder exit");
    let mut log = std::fs::File::create(out.join("ticks.jsonl"))?;
    let mut last = start;
    let mut previous = String::new();
    let mut stages = std::collections::BTreeMap::new();
    let mut compared_ticks = 0;
    let mut checkpoint_tick = None;
    capture(&mut v, &mut cold, &out, "initial")?;
    for frame in 0..target * 10 {
        for a in std::iter::once(&mut v).chain(cold.iter_mut()) {
            a.session
                .as_mut()
                .unwrap()
                .set_clock_time(origin.wrapping_add(frame as u32 * 10))?;
            if !a.presentation_pending() {
                a.poll_application_ui()?;
            }
            if !a.presentation_pending() {
                a.session.as_mut().unwrap().begin_clock_frame()?;
            }
        }
        for _ in 0..10000 {
            if let Some(b) = &cold {
                ensure!(
                    v.presentation_pending() == b.presentation_pending(),
                    "different caller return boundary"
                );
            }
            if !v.presentation_pending() {
                break;
            }
            let x = v.step();
            let y = if let Some(b) = &mut cold {
                Some(b.step())
            } else {
                None
            };
            if x.is_err() || y.as_ref().is_some_and(|r| r.is_err()) {
                std::fs::write(out.join("failed-session.json"), retained(&v)?)?;
                if let Some(b) = &cold {
                    std::fs::write(out.join("failed-restored.json"), retained(b)?)?;
                }
                anyhow::bail!("pool step failed: {x:?}/{y:?}");
            }
            if let Some(y) = y {
                ensure!(x? == y?, "different step result");
            }
            ensure!(v.error.is_none(), "{:?}", v.error);
            let tick = v.session.as_ref().unwrap().simulation.vm.tick;
            if tick != last {
                ensure!(tick == last + 1);
                v.refresh()?;
                if let Some(b) = &mut cold {
                    b.refresh()?;
                    equal(&v, b, &out, &format!("tick-{tick}"))?;
                    compared_ticks += 1;
                }
                let row = observe(&v);
                writeln!(log, "{}", row)?;
                log.flush()?;
                let layers = row["animator"]["layers"].as_array().unwrap();
                let names = layers
                    .iter()
                    .map(|l| l["layer"]["name"].as_str().unwrap())
                    .collect::<Vec<_>>();
                let has = |name: &str| names.contains(&name);
                let key = names.join("+");
                if key != previous {
                    eprintln!("pool tick{tick} {} {key}", row["person"]);
                    previous = key;
                }
                let label = if has("a2o-pool-dive-jumpin") && !stages.contains_key("dive") {
                    Some("dive")
                } else if has("a2o-pool-swim-start") && !stages.contains_key("swim-start") {
                    Some("swim-start")
                } else if has("a2o-pool-swim-loop") && !stages.contains_key("swim-loop") {
                    Some("swim-loop")
                } else if has("a2o-pool-ladder-getout") && !stages.contains_key("ladder") {
                    Some("ladder")
                } else if stages.contains_key("ladder")
                    && row["person"][2] == 0
                    && !has("a2o-pool-ladder-getout")
                    && !stages.contains_key("exited")
                {
                    Some("exited")
                } else {
                    None
                };
                if let Some(label) = label {
                    stages.insert(label, tick);
                    capture(&mut v, &mut cold, &out, label)?;
                }
                if row["person"][2] == 1 {
                    ensure!(
                        has("a2o-pool-standing-loop"),
                        "swimming profile selected an ordinary standing base"
                    );
                    ensure!(
                        !has("a2o-walking-start") && !has("a2o-walking-loop"),
                        "swimming profile selected walking locomotion"
                    );
                }
                // Cold restore the actual active elapsed caller at its first
                // swimming transition, preserving animation fade/clock/layers.
                if has("a2o-pool-swim-start") && cold.is_none() {
                    let checkpoint = out.join("mid-swim.json");
                    v.args.save = checkpoint.clone();
                    v.save()?;
                    let mut b = make(&checkpoint)?;
                    // install publishes one initial device generation. Restore
                    // at the saved caller boundary before comparing, then give
                    // both viewers exactly the same later publications.
                    b.load(&checkpoint)?;
                    equal(&v, &b, &out, "cold-active-swim")?;
                    cold = Some(b);
                    checkpoint_tick = Some(tick);
                    ensure!(
                        v.session.as_ref().unwrap().clock_pending(),
                        "test did not retain elapsed caller"
                    );
                }
                last = tick;
            }
        }
        v.prepare_and_publish()?;
        if let Some(b) = &mut cold {
            b.prepare_and_publish()?;
        }
        if last >= start + target {
            break;
        }
    }
    ensure!(last >= start + target, "clock stopped at{last}");
    ensure!(
        stages.len() == 5 && compared_ticks > 600,
        "incomplete dive/swim/ladder/cold coverage {stages:?}"
    );
    capture(&mut v, &mut cold, &out, "final")?;
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({
        "start":start,"end":last,"inputs":inputs,"stages":stages,"cold_checkpoint_tick":checkpoint_tick,"exact_compared_ticks":compared_ticks,
        "managed_and_legacy_hydration_preserve_full_simulation_assets_view":true,"saved_fingerprint_validated_before_upgrade":true,
        "corrupt_fingerprint_rejected":true,"full_warm_cold_state_equal_at_every_completed_tick":true,
        "stage_render_repeat_and_cold_pixels_equal":true,"render_preserves_full_state":true,"strict_original_household_parity_ticks":0,
        "scope":"Original-script Rust continuation with authored 10ms elapsed inputs and inferred route policy; unchanged original59db2f component checks separately recorded. Correct clip duration/root motion legitimately change subsequent gameplay timing."}))?,
    )?;
    Ok(())
}
