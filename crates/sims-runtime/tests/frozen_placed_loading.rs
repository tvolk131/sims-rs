//! Native-derived factory ID order and exact managed cold restoration. No
//! gameplay ticks, fabricated starting owners, GUI or original guest execution.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_content::Index;
use sims_runtime::original_session::{Options, Session};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_pinned(repo: &Path, path: &str, expected: &str) -> Result<Vec<u8>> {
    let data = fs::read(repo.join(path))?;
    ensure!(digest(&data) == expected, "private input changed: {path}");
    Ok(data)
}
fn state(s: &Session) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&s.save())?)
}
fn clock_rng(s: &Session) -> Value {
    json!([
        s.simulation.vm.tick,
        s.simulation.vm.rng_state,
        s.simulation.vm.lot_rng_state,
        s.simulation.vm.crt_rng_state
    ])
}
fn cold(s: &Session, index: &Index, out: &Path, label: &str) -> Result<Value> {
    let before = state(s)?;
    let streams = clock_rng(s);
    let encoded = serde_json::to_vec(&s.save_managed()?)?;
    let restored = match Session::restore(index, serde_json::from_slice(&encoded)?) {
        Ok(s) => s,
        Err(e) => {
            fs::write(
                out.join(format!("{label}-first-failure-managed.json")),
                &encoded,
            )?;
            return Err(e.context(format!("{label} independent managed cold restore")));
        }
    };
    let after = state(&restored)?;
    if before != after {
        fs::write(
            out.join(format!("{label}-first-failure-before.json")),
            &before,
        )?;
        fs::write(
            out.join(format!("{label}-first-failure-after.json")),
            &after,
        )?;
        anyhow::bail!("{label} complete retained state differs after cold restore");
    }
    ensure!(streams == clock_rng(&restored), "{label} tick/RNG differs");
    ensure!(before == state(s)?, "{label} passive save advanced state");
    Ok(
        json!({"label":label,"research_sha256":digest(&before),"managed_sha256":digest(&encoded),"managed_bytes":encoded.len(),"exact_state_equal":true,"clock_rng":streams}),
    )
}

#[test]
#[ignore = "private captured Goth startup/immutable install; exact factory order and loader cold restoration"]
fn captured_goth_nested_placed_factory_order_and_newbie_load_survive_managed_cold() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("frozen Placed load test panicked")
}
fn run() -> Result<()> {
    let repo =
        PathBuf::from(std::env::var_os("SIMS_PLACED_REPO").context("SIMS_PLACED_REPO required")?)
            .canonicalize()?;
    let out = repo.join(std::env::var("SIMS_PLACED_OUT")?);
    ensure!(out.starts_with(repo.join("cache")));
    fs::create_dir(&out)?;
    let mut report = json!({"scope":"Native identity/order and internal managed cold equality only; no whole-engine or tick parity","strict_original_ticks_added":0,"branches":[]});
    let result = (|| -> Result<()> {
        let startup = read_pinned(
            &repo,
            "cache/native-parity-guest-20261024-sixth/retained-startup-first.json",
            "67ff0c2e1659cf906034c7c8259f7ca3a34dad3739951e1c043ef8cd130aa3c2",
        )?;
        let inputs = read_pinned(
            &repo,
            "cache/native-parity-guest-20261024-sixth/zero-inputs-first.json",
            "0f0dec84a6333ff86fab05b0eae8641453d59cf113eb30da09569cf7bf86d976",
        )?;
        let resources = read_pinned(
            &repo,
            "cache/native-parity-guest-20261023-second/empty-skeleton-cache.json",
            "5eec93bc5b8eb8aabb94448c95b54b6733b9e004670190f91485fba112bf1728",
        )?;
        let native = read_pinned(
            &repo,
            "cache/native-parity-guest-20261024-sixth/baseline-only-first.jsonl",
            "d476ccd23911f8ba3a9b9ea3b91376e18808f8a99b205dc061e509432aff6edd",
        )?;
        let startup: Value = serde_json::from_slice(&startup)?;
        let inputs: Value = serde_json::from_slice(&inputs)?;
        let incoming: sims_content::house_startup::Incoming =
            serde_json::from_value(startup["incoming"].clone())?;
        let crt = incoming
            .vm
            .crt_rng_state
            .context("captured CRT owner absent")?;
        let index = Index::scan(&repo.join("cache/native-parity-guest-20261023-second/assets"))?;
        let immutable =
            Index::scan(&repo.join("cache/native-parity-guest-20261023-second/assets-immutable"))?;
        let goth = Session::load_with_startup(
            &index,
            "UserData/Houses/House05.iff",
            Options {
                application_context: Some(serde_json::from_value(
                    inputs["application_context"].clone(),
                )?),
                native_postload: true,
                native_load_completion: true,
                crt_seed: crt,
                ..Default::default()
            },
            true,
            serde_json::from_slice(&resources)?,
            Some(incoming),
        )?;
        ensure!(goth.simulation.vm.tick == 88444);
        ensure!(
            goth.host.presentation.awaiting_house_open && goth.host.presentation.postload.is_none(),
            "application post-visit tail was run/moved early"
        );
        let indicators = &goth
            .simulation
            .native_lot
            .as_ref()
            .context("native lot absent")?
            .indicators;
        ensure!(
            indicators.primary == Some(383) && indicators.secondary == Some(384),
            "nested Placed factory IDs differ: {indicators:?}"
        );
        let actual: Vec<_> = [383, 384, 385, 386]
            .into_iter()
            .map(|id| {
                let object = &goth.simulation.vm.world.objects[&id];
                json!({"id":id,"guid":object.definition.as_ref().unwrap().guid})
            })
            .collect();
        let native: Value = serde_json::from_slice(
            native
                .split(|b| *b == b'\n')
                .filter(|line| !line.is_empty())
                .last()
                .context("native baseline absent")?,
        )?;
        let expected: Vec<_> = [383, 384, 385, 386]
            .into_iter()
            .map(|id| {
                let o = native["objects"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|o| o["id"] == id)
                    .unwrap();
                let hex = o["definition"].as_str().unwrap();
                let bytes: Vec<_> = (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                    .collect();
                json!({"id":id,"guid":u32::from_le_bytes(bytes[28..32].try_into().unwrap())})
            })
            .collect();
        ensure!(
            actual == expected,
            "factory identity/order differs from native"
        );
        for id in [383, 384] {
            ensure!(
                goth.simulation.vm.world.objects[&id].data[&8] as u16 & 0x4000 != 0,
                "indicator hide bit absent"
            );
            ensure!(
                goth.host.ordinary.graphics.contains_key(&id),
                "nested ordinary graphic was discarded during Reader→Host handoff"
            );
        }
        let cold_goth = cold(&goth, &immutable, &out, "goth-loaded")?;
        report["branches"].as_array_mut().unwrap().push(json!({"house":"Goth","native_ids":expected,"postvisit_tail_not_started":true,"ordinary_graphics_retained":[383,384],"cold":cold_goth}));
        drop(goth);
        // Adjacent stock-house smoke input; this fresh diagnostic context is not
        // presented as a captured Newbie native baseline or input history.
        let newbie = Session::load(&index, "UserData/Houses/House07.iff", Options::default())?;
        let indicators = &newbie
            .simulation
            .native_lot
            .as_ref()
            .context("Newbie native lot absent")?
            .indicators;
        let ids = [
            indicators.primary.context("Newbie primary absent")?,
            indicators.secondary.context("Newbie secondary absent")?,
        ];
        for (id, guid) in ids.into_iter().zip(sims_vm::load_indicators::GUIDS) {
            ensure!(
                newbie.simulation.vm.world.objects[&id]
                    .definition
                    .as_ref()
                    .unwrap()
                    .guid
                    == guid
            );
            ensure!(
                newbie.host.ordinary.graphics.contains_key(&id),
                "Newbie ordinary graphic absent"
            );
        }
        let cold_newbie = cold(&newbie, &immutable, &out, "newbie-loaded")?;
        report["branches"].as_array_mut().unwrap().push(json!({"house":"Newbie","scope":"Fresh diagnostic adjacent load, not native baseline parity","indicators":ids,"cold":cold_newbie}));
        Ok(())
    })();
    report["passed"] = json!(result.is_ok());
    if let Err(e) = &result {
        report["first_error"] = json!(format!("{e:#}"));
    }
    fs::write(out.join("result.json"), serde_json::to_vec_pretty(&report)?)?;
    result
}
