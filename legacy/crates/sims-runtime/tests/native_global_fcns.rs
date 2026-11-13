//! Ordinary asset-backed fresh Session import and independent managed restore.
//! Explicit observed process input; no gameplay tick or original parity claim.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use sims_content::Index;
use sims_runtime::original_session::{Options, Session};
use std::{fs, path::PathBuf};
#[test]
#[ignore = "private original install and empty immutable-only install; source-bound process provider"]
fn standard_fresh_newbie_and_goth_keep_one_provider_across_managed_restore() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("FCNS fresh import panicked")
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn run() -> Result<()> {
    let assets =
        PathBuf::from(std::env::var_os("SIMS_FCNS_ASSETS").context("SIMS_FCNS_ASSETS required")?);
    let immutable = PathBuf::from(
        std::env::var_os("SIMS_FCNS_IMMUTABLE").context("SIMS_FCNS_IMMUTABLE required")?,
    );
    let base = PathBuf::from(std::env::var_os("SIMS_FCNS_OUT").context("SIMS_FCNS_OUT required")?);
    fs::create_dir_all(&base)?;
    let out = base.join("standard");
    fs::create_dir(&out)?;
    let index = Index::scan(&assets)?;
    let cold_index = Index::scan(&immutable)?;
    ensure!(
        cold_index
            .resources
            .iter()
            .all(|r| !r.locator.to_ascii_lowercase().starts_with("userdata")),
        "cold index contains starting saves"
    );
    let mut report = serde_json::json!({"scope":"Private explicit observed process input, ordinary loading and managed cold equality; no original tick credit","cases":[]});
    let result = (|| -> Result<()> {
        for (name, source) in [
            ("newbie", "UserData/Houses/House07.iff"),
            ("goth", "UserData/Houses/House05.iff"),
        ] {
            let options = Options {
                routing: sims_sim::lot_route::path::Policy::InferredRectangles,
                ..Default::default()
            };
            let s = Session::load(&index, source, options)
                .with_context(|| format!("ordinary fresh {name} import"))?;
            let owner = s
                .simulation
                .vm
                .world
                .autonomy_constants
                .as_ref()
                .context("process owner lost")?;
            ensure!(
                owner.process.loaded == Some(1)
                    && owner.friendship_threshold().map_err(anyhow::Error::msg)? == 50,
                "process producer not run"
            );
            let provider = s
                .host
                .autonomy_resources
                .as_ref()
                .context("provider lost Reader->Host")?;
            ensure!(
                provider.events.len() == 4 && provider.events[3].return_value == -99,
                "resource service ordering/error differs"
            );
            let value = serde_json::to_value(provider)?;
            let iff = &value["components"]["entries"]["1"]["iff"];
            let header = serde_json::json!({"aggregate_error":value["aggregate"]["error"],"child_error":iff["error"],
                "writable":value["components"]["entries"]["1"]["stream"]["writable"],
                "map_offset":iff["map_offset"],"unused":iff["unused"],"write_cursor":iff["write_cursor"],
                "modified":iff["modified"],"type_count":iff["table"]["len"]});
            ensure!(
                header
                    == serde_json::json!({"aggregate_error":-99,"child_error":0,"writable":false,
                "map_offset":64,"unused":10378,"write_cursor":4294967295u32,"modified":false,"type_count":9}),
                "modeled provider header differs from exposed original post-Add fields: {header}"
            );
            let before = serde_json::to_vec(&s.save())?;
            let managed = serde_json::to_vec(&s.save_managed()?)?;
            let restored = match Session::restore(&cold_index, serde_json::from_slice(&managed)?) {
                Ok(s) => s,
                Err(e) => {
                    fs::write(
                        out.join(format!("{name}-first-failure-managed.json")),
                        &managed,
                    )?;
                    return Err(e.context("independent immutable-only restore"));
                }
            };
            let after = serde_json::to_vec(&restored.save())?;
            if before != after {
                fs::write(out.join(format!("{name}-first-failure-warm.json")), &before)?;
                fs::write(out.join(format!("{name}-first-failure-cold.json")), &after)?;
                anyhow::bail!("{name} complete state/RNG differs after cold restore");
            }
            ensure!(
                serde_json::to_vec(&restored.save_managed()?)? == managed,
                "{name} managed bytes changed on resave"
            );
            ensure!(
                serde_json::to_vec(&s.save())? == before,
                "passive save mutated {name}"
            );
            report["cases"].as_array_mut().unwrap().push(serde_json::json!({"name":name,"source":source,"state_sha256":hash(&before),"managed_sha256":hash(&managed),"managed_bytes":managed.len(),"tick":s.simulation.vm.tick,"global_rng":s.simulation.vm.rng_state,"lot_rng":s.simulation.vm.lot_rng_state,"crt_rng":s.simulation.vm.crt_rng_state,"resource_events":provider.events,"provider_header":header,"full_cold_equal":true}));
        }
        Ok(())
    })();
    report["outcome"] = serde_json::json!(if result.is_ok() { "pass" } else { "failed" });
    if let Err(e) = &result {
        report["first_error"] = serde_json::json!(format!("{e:#}"));
    }
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    result
}

#[test]
#[ignore = "private captured retained Goth input; exact process/provider ownership and immutable-only restore"]
fn retained_goth_preserves_existing_provider_and_never_bootstraps_unknown_process() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(retained)?
        .join()
        .expect("retained FCNS test panicked")
}
fn retained() -> Result<()> {
    use sims_content::{autonomy_resources::InitialProfile, house_startup::Incoming};
    use sims_vm::autonomy_constants::{Reply, Request};
    let repo =
        PathBuf::from(std::env::var_os("SIMS_FCNS_REPO").context("SIMS_FCNS_REPO required")?);
    let base = PathBuf::from(std::env::var_os("SIMS_FCNS_OUT").context("SIMS_FCNS_OUT required")?);
    fs::create_dir_all(&base)?;
    let out = base.join("retained");
    fs::create_dir(&out)?;
    let index = Index::scan(&PathBuf::from(
        std::env::var_os("SIMS_FCNS_ASSETS").context("SIMS_FCNS_ASSETS required")?,
    ))?;
    let immutable = Index::scan(&PathBuf::from(
        std::env::var_os("SIMS_FCNS_IMMUTABLE").context("SIMS_FCNS_IMMUTABLE required")?,
    ))?;
    ensure!(
        immutable
            .resources
            .iter()
            .all(|r| !r.locator.to_ascii_lowercase().starts_with("userdata"))
    );
    let mut report = serde_json::json!({"scope":"Actual retained lot with explicit authored nonzero process marker; ownership and managed cold proof only, no original baseline/tick parity","strict_original_ticks_added":0});
    let result = (|| -> Result<()> {
        let read = |path: &str, expected: &str| -> Result<Vec<u8>> {
            let bytes = fs::read(repo.join(path))?;
            ensure!(hash(&bytes) == expected, "retained input changed: {path}");
            Ok(bytes)
        };
        let bytes = read(
            "cache/native-parity-guest-20261024-sixth/retained-startup-first.json",
            "67ff0c2e1659cf906034c7c8259f7ca3a34dad3739951e1c043ef8cd130aa3c2",
        )?;
        let envelope: serde_json::Value = serde_json::from_slice(&bytes)?;
        let mut incoming: Incoming = serde_json::from_value(envelope["incoming"].clone())?;
        ensure!(
            incoming.vm.world.autonomy_constants.is_none(),
            "historical retained owner unexpectedly present"
        );
        let inputs: serde_json::Value = serde_json::from_slice(&read(
            "cache/native-parity-guest-20261024-sixth/zero-inputs-first.json",
            "0f0dec84a6333ff86fab05b0eae8641453d59cf113eb30da09569cf7bf86d976",
        )?)?;
        let skeletons: sims_content::skeleton_resources::Cache = serde_json::from_slice(&read(
            "cache/native-parity-guest-20261023-second/empty-skeleton-cache.json",
            "5eec93bc5b8eb8aabb94448c95b54b6733b9e004670190f91485fba112bf1728",
        )?)?;
        let options = Options {
            application_context: Some(serde_json::from_value(
                inputs["application_context"].clone(),
            )?),
            native_postload: true,
            native_load_completion: true,
            ..Default::default()
        };
        let input = InitialProfile::CompleteCollection191ObservedInitial20261024.prepare(&index)?;
        let error = Session::load_with_retained_process_resources(
            &index,
            "UserData/Houses/House05.iff",
            options.clone(),
            true,
            skeletons.clone(),
            incoming.clone(),
            input.provider.clone(),
        )
        .err()
        .context("historical unknown incoming was silently bootstrapped")?;
        ensure!(
            error
                .to_string()
                .contains("retained process owner is unknown"),
            "wrong unknown-owner rejection: {error:#}"
        );
        report["unknown_owner_rejection"] = serde_json::json!(format!("{error:#}"));
        let mut owner = input.owner;
        owner.process.loaded = Some(2);
        owner.configuration = Default::default();
        // A deliberately nonzero caller-owned marker tests the verified skip;
        // it is not a claim that this raw byte was captured in the Goth process.
        incoming.vm.world.autonomy_constants = Some(owner.clone());
        let mut provider = input.provider;
        let binding = provider.binding();
        let Reply::Loaded { table, .. } =
            provider.service(&Request::Load(binding.clone()), None)?
        else {
            unreachable!()
        };
        let Reply::Flushed { return_value, .. } =
            provider.service(&Request::Flush(binding), Some(&table))?
        else {
            unreachable!()
        };
        ensure!(return_value == -99);
        let retained_provider = serde_json::to_vec(&provider)?;
        let session = Session::load_with_retained_process_resources(
            &index,
            "UserData/Houses/House05.iff",
            options,
            true,
            skeletons,
            incoming,
            provider,
        )?;
        ensure!(
            session.simulation.vm.world.autonomy_constants.as_ref() == Some(&owner),
            "retained process owner changed"
        );
        ensure!(
            serde_json::to_vec(
                session
                    .host
                    .autonomy_resources
                    .as_ref()
                    .context("retained provider lost")?
            )? == retained_provider,
            "Reader->Host reopened/rebuilt retained provider"
        );
        let before = serde_json::to_vec(&session.save())?;
        let managed = serde_json::to_vec(&session.save_managed()?)?;
        let restored = match Session::restore(&immutable, serde_json::from_slice(&managed)?) {
            Ok(s) => s,
            Err(e) => {
                fs::write(out.join("first-failure-managed.json"), &managed)?;
                return Err(e);
            }
        };
        let after = serde_json::to_vec(&restored.save())?;
        if before != after {
            fs::write(out.join("first-failure-warm.json"), &before)?;
            fs::write(out.join("first-failure-cold.json"), &after)?;
            anyhow::bail!("retained full state/RNG cold mismatch");
        }
        ensure!(
            serde_json::to_vec(restored.host.autonomy_resources.as_ref().unwrap())?
                == retained_provider
        );
        ensure!(restored.simulation.vm.world.autonomy_constants == Some(owner));
        report["state_sha256"] = serde_json::json!(hash(&before));
        report["managed_sha256"] = serde_json::json!(hash(&managed));
        report["provider_sha256"] = serde_json::json!(hash(&retained_provider));
        report["full_cold_equal"] = true.into();
        report["tick"] = session.simulation.vm.tick.into();
        Ok(())
    })();
    report["outcome"] = serde_json::json!(if result.is_ok() { "pass" } else { "failed" });
    if let Err(e) = &result {
        report["first_error"] = serde_json::json!(format!("{e:#}"));
    }
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    result
}
