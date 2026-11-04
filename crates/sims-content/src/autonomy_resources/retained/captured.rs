//! Private original-derived checks. Required original observations live in the
//! local evidence capsule; authored unit fixtures are in the parent module.
use super::super::*;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn cold<T: Serialize + serde::de::DeserializeOwned>(v: &T) -> Result<T> {
    Ok(serde_json::from_slice(&serde_json::to_vec(v)?)?)
}

#[test]
#[ignore = "private fifth native resize provider, immutable assets and initializer observations"]
fn fifth_observed_provider_continues_initializer_with_every_cold_cut() -> Result<()> {
    let repo =
        PathBuf::from(std::env::var_os("SIMS_RETAINED_GLOBAL_REPO").context("repo required")?);
    let assets = PathBuf::from(std::env::var_os("SIMS_FCNS_ASSETS").context("assets required")?);
    let out =
        PathBuf::from(std::env::var_os("SIMS_FCNS_OUT").context("new private output required")?);
    ensure!(
        out.parent()
            .context("output parent")?
            .canonicalize()?
            .starts_with(repo.join("cache").canonicalize()?),
        "private output required"
    );
    fs::create_dir(&out)?;
    let mut report = json!({"scope":"Same fifth resize Global provider projection and first autonomy initializer; no complete household import or completed gameplay tick claim", "strict_parity_ticks_added":0, "completed_gameplay_ticks":0, "native_resource_postgraph_equality_claimed":false, "scope_limit":"Intervening full-load calls are not replayed here; native equality covers the captured FCNS table/process stores/error, not the whole later resource graph"});
    let result = (|| -> Result<()> {
        let read = |path: &str, expected: &str| -> Result<Vec<u8>> {
            let bytes = fs::read(repo.join(path))?;
            ensure!(hash(&bytes) == expected, "captured input changed: {path}");
            Ok(bytes)
        };
        let retained_bytes = read(
            "cache/native-fifth-resize-provider-audit-20261025-first/retained-state-first.json",
            "1c8f4e07430c5d2060615ba05074efc5c133413a69f3472d63bd5ff924b0c3e8",
        )?;
        let expected_bytes = read(
            "cache/native-fifth-retained-global-20261025-first/native-expected-second.json",
            "8c2d33147e3a947f8726e3877dc0cdc84a0a8eaaf1d3cf72d924aaba4a8c50ce",
        )?;
        report["retained_input_sha256"] = json!(hash(&retained_bytes));
        report["native_expected_sha256"] = json!(hash(&expected_bytes));
        let input: Retained = serde_json::from_slice(&retained_bytes)?;
        let expected: Value = serde_json::from_slice(&expected_bytes)?;
        let index = Index::scan(&assets)?;
        let matches: Vec<_> = index
            .resources
            .iter()
            .filter(|r| r.name == "Global.iff")
            .collect();
        ensure!(matches.len() == 1, "Global provider missing or ambiguous");
        let locator = &matches[0].locator;
        let provider = Provider::from_retained(&index, locator, input.clone())?;
        let component = &provider.components.entries[&1];
        ensure!(
            component.iff == input.iff
                && provider.aggregate == input.aggregate
                && component.stream.position == input.stream_position
                && component.stream.open == input.stream_open
                && component.stream.writable == input.stream_writable
                && provider.components.handles
                    == input.handles.iter().map(|(&h, &l)| (h, (1, l))).collect()
                && provider.components.next_handle == input.next_handle
                && provider.components.buffers == input.buffers
                && provider.events.is_empty(),
            "import changed the observed modeled owner"
        );
        ensure!(
            component.iff.records().count() == 406
                && input.handles.len() == 13
                && input.stream_position == 129431,
            "wrong observed provider profile"
        );
        ensure!(
            component
                .iff
                .records()
                .find(|r| r.tag == TAG && r.id == 2)
                .context("FCNS2 absent from map")?
                .payload
                .is_none(),
            "FCNS2 was already loaded"
        );
        report["initial_provider_sha256"] = json!(hash(&serde_json::to_vec(&provider)?));
        fs::write(
            out.join("imported-provider.json"),
            serde_json::to_vec(&provider)?,
        )?;

        // Public construction must reject changed ownership/capability rather
        // than reopen a fresh resource graph as a repair.
        for (name, bad) in [
            (
                "closed",
                Retained {
                    stream_open: false,
                    ..input.clone()
                },
            ),
            (
                "writable",
                Retained {
                    stream_writable: true,
                    ..input.clone()
                },
            ),
            (
                "lost_handles",
                Retained {
                    handles: BTreeMap::new(),
                    ..input.clone()
                },
            ),
        ] {
            ensure!(
                Provider::from_retained(&index, locator, bad).is_err(),
                "accepted {name}"
            );
        }

        let owner: Owner = serde_json::from_value(expected["initial_owner"].clone())?;
        owner.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            owner.configuration.source.as_ref() == Some(&provider.binding()),
            "source identity differs"
        );
        let mut warm_provider = provider.clone();
        let mut cold_provider = provider;
        let mut warm_process = owner.process.clone();
        let mut cold_process = warm_process.clone();
        let mut warm_call = owner.begin();
        let mut cold_call = warm_call.clone();
        let mut stores = vec![];
        let mut cuts = 0;
        for _ in 0..64 {
            // A complete representation cut includes the suspended child,
            // process words, resources, payloads and stream cursor together.
            (cold_provider, cold_process, cold_call) =
                cold(&(cold_provider, cold_process, cold_call))?;
            cold_provider.validate(&index)?;
            ensure!(
                serde_json::to_vec(&(&cold_provider, &cold_process, &cold_call))?
                    == serde_json::to_vec(&(&warm_provider, &warm_process, &warm_call))?,
                "cold state differs before cut{cuts}"
            );
            let warm_progress = warm_call
                .advance(&mut warm_process)
                .map_err(anyhow::Error::msg)?;
            let cold_progress = cold_call
                .advance(&mut cold_process)
                .map_err(anyhow::Error::msg)?;
            ensure!(warm_progress == cold_progress, "cold progress differs");
            match warm_progress {
                constants::Progress::Service(request) => {
                    let reply = warm_provider.service(&request, warm_call.table())?;
                    let cold_reply = cold_provider.service(&request, cold_call.table())?;
                    ensure!(
                        serde_json::to_value(&reply)? == serde_json::to_value(&cold_reply)?,
                        "cold reply differs"
                    );
                    match &reply {
                        Reply::Loaded { table, .. } => ensure!(
                            serde_json::to_value(table)? == expected["loaded_table"],
                            "loaded native table differs"
                        ),
                        Reply::Flushed { return_value, .. } => ensure!(
                            Some(i64::from(*return_value))
                                == expected["flush_return_value"].as_i64(),
                            "native readonly error differs"
                        ),
                    }
                    warm_call.reply(reply).map_err(anyhow::Error::msg)?;
                    cold_call.reply(cold_reply).map_err(anyhow::Error::msg)?;
                }
                constants::Progress::Store(store) => {
                    match &store {
                        constants::Store::Word(field, value) => {
                            let native = &expected["stores"][stores.len()];
                            ensure!(
                                native["address"].as_u64() == Some(u64::from(field.address()))
                                    && native["value"].as_u64() == Some(u64::from(*value)),
                                "native store differs at{}",
                                stores.len()
                            );
                            ensure!(
                                serde_json::to_value(&warm_process)? == native["process"],
                                "native full process differs at{}",
                                stores.len()
                            );
                        }
                        constants::Store::Loaded(value) => ensure!(
                            *value == 1
                                && serde_json::to_value(&warm_process)?
                                    == expected["final_process"],
                            "native loaded store differs"
                        ),
                    }
                    stores.push(store);
                }
                constants::Progress::Complete => break,
            }
            cuts += 1;
            ensure!(
                serde_json::to_vec(&(&cold_provider, &cold_process, &cold_call))?
                    == serde_json::to_vec(&(&warm_provider, &warm_process, &warm_call))?,
                "cold state differs after cut{cuts}"
            );
        }
        ensure!(
            warm_call.complete() && cold_call.complete() && stores.len() == 13,
            "initializer incomplete"
        );
        ensure!(
            serde_json::to_value(&warm_process)? == expected["final_process"],
            "final native process differs"
        );
        warm_provider.validate(&index)?;
        let final_component = &warm_provider.components.entries[&1];
        ensure!(
            warm_provider.aggregate.error == -99
                && final_component.iff.error == 0
                && final_component
                    .iff
                    .records()
                    .filter(|r| r.payload.is_some())
                    .count()
                    == 14
                && warm_provider.components.buffers.is_empty(),
            "readonly provider poststate differs"
        );
        // The original13 resident resources and exact bucket topology survive.
        let before: Vec<_> = input
            .iff
            .records()
            .filter(|r| r.payload.is_some())
            .collect();
        for record in before {
            ensure!(
                final_component
                    .iff
                    .records()
                    .find(|r| r.tag == record.tag && r.id == record.id && r.flags == record.flags)
                    == Some(record),
                "original resident changed"
            );
        }
        let mut projected = serde_json::to_value(&final_component.iff)?;
        let before = serde_json::to_value(&input.iff)?;
        let fcns2 = projected["storage"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|r| r["tag"].as_u64() == Some(u64::from(TAG)) && r["id"] == 2)
            .context("final FCNS2 absent")?;
        fcns2["payload"] = Value::Null;
        projected["next_handle"] = before["next_handle"].clone();
        ensure!(projected == before, "IFF metadata or topology changed");
        fs::write(
            out.join("final-provider.json"),
            serde_json::to_vec(&warm_provider)?,
        )?;
        report["stores"] = serde_json::to_value(stores)?;
        report["cold_cuts"] = json!(cuts);
        report["final_process"] = serde_json::to_value(&warm_process)?;
        report["resource_events"] = serde_json::to_value(&warm_provider.events)?;
        report["final_provider_sha256"] = json!(hash(&serde_json::to_vec(&warm_provider)?));
        report["full_cold_state_equal_at_every_cut"] = json!(true);
        report["all_native_process_stores_equal"] = json!(true);
        Ok(())
    })();
    report["outcome"] = json!(if result.is_ok() { "pass" } else { "failed" });
    if let Err(e) = &result {
        report["first_error"] = json!(format!("{e:#}"));
    }
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    result
}
