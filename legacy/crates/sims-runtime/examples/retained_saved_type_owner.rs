//! Retain the observed catalog's saved-type metadata and stop at missing source.
//! This is an entry-epoch component proof, not an execution of the house suffix.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sims_fixtures::{Pin, Store};
use sims_sim::{
    object_catalog::{Key, definition},
    saved_types::{self, Declaration, Function, Gtab, Record},
};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MissingSource {
    events: Vec<String>,
}
impl saved_types::Host for MissingSource {
    fn retire_declaration(&mut self, _: &Declaration) -> Result<(), String> {
        Err("native declaration retirement owner is not supplied".into())
    }
    fn gtab(&mut self) -> Result<Gtab, String> {
        self.events.push("observed Gtab#1 absent".into());
        Ok(Gtab::Absent)
    }
    fn begin_objt(&mut self) -> Result<Option<i32>, String> {
        Err("current objt#0 payload/source is unqualified".into())
    }
    fn next_objt(&mut self, _: i32) -> Result<Option<Record>, String> {
        Err("no objt source was opened".into())
    }
    fn end_objt(&mut self) -> Result<(), String> {
        Err("no objt source was opened".into())
    }
    fn signature(&mut self, _: Key<u32>, _: Function) -> Result<u32, String> {
        Err("no current signature source/getter was supplied".into())
    }
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 3,
        "usage: retained_saved_type_owner STATE.json DEFINITIONS.json NEW_OUTPUT"
    );
    let input_raw = fs::read(&args[0])?;
    let definitions_raw = fs::read(&args[1])?;
    let input: serde_json::Value = serde_json::from_slice(&input_raw)?;
    ensure!(
        input["definitions_input_sha256"] == sims_fixtures::digest(&definitions_raw)
            && input["evidence"]["row"]["sha256"]
                == "e5002772820e16fc1ee15628ab08c598676d61b31a1d8e0df80a05d8938dde7a",
        "catalog definition/source epoch binding differs"
    );
    let mut catalog: definition::Catalog = serde_json::from_slice(&definitions_raw)?;
    let keys: Vec<Key<u32>> = serde_json::from_value(input["catalog_keys"].clone())?;
    ensure!(
        input["schema"] == 1 && catalog.entries.iter().map(|e| e.key).eq(keys),
        "foreign catalog/key projection"
    );
    ensure!(
        input["gtab_absence"]["observed_absent"] == true
            && input["gtab_absence"]["elf_sha256"] == input["evidence"]["elf_sha256"]
            && input["gtab_absence"]["source_row"]["sha256"]
                == "e5002772820e16fc1ee15628ab08c598676d61b31a1d8e0df80a05d8938dde7a",
        "Gtab absence source binding missing"
    );
    let state = saved_types::State::from_parts(
        &catalog,
        serde_json::from_value(input["saved_indices"].clone())?,
        serde_json::from_value(input["declarations"].clone())?,
        serde_json::from_value(input["type_guids"].clone())?,
    )
    .map_err(anyhow::Error::msg)?;
    let before_catalog = catalog.clone();
    let before_state = state.clone();
    let output = PathBuf::from(&args[2]);
    fs::create_dir(&output).context("output must be new")?;
    let store = Store::create(output.join("store"))?;
    let mut state = state;
    let mut execution = saved_types::Execution::new(65536);
    let mut host = MissingSource { events: vec![] };
    let mut stop = None;
    for _ in 0..16 {
        match execution.step(&mut state, &mut catalog, &mut host) {
            Ok(false) => {}
            Ok(true) => bail!("unqualified source unexpectedly completed"),
            Err(error) => {
                stop = Some(error);
                break;
            }
        }
    }
    ensure!(
        stop.as_deref() == Some("current objt#0 payload/source is unqualified"),
        "unexpected metadata stop:{stop:?}"
    );
    ensure!(
        catalog == before_catalog,
        "source gate changed canonical definitions"
    );
    let bundle = (
        state.clone(),
        execution.clone(),
        host.clone(),
        catalog.clone(),
    );
    let blob = store.put_value(&bundle)?;
    let cold: (
        saved_types::State,
        saved_types::Execution,
        MissingSource,
        definition::Catalog,
    ) = store.get_value(&blob)?;
    ensure!(cold == bundle, "metadata cold roundtrip differs");
    let (mut cold_state, mut cold_execution, mut cold_host, mut cold_catalog) = cold;
    ensure!(
        cold_execution
            .step(&mut cold_state, &mut cold_catalog, &mut cold_host)
            .unwrap_err()
            == stop.clone().unwrap(),
        "source gate bypassed on restore"
    );
    ensure!(
        (cold_state, cold_execution, cold_host, cold_catalog) == bundle,
        "repeated pending source changed state"
    );
    let blobs = vec![
        store.put_value(&input)?,
        store.put_value(&(before_catalog, before_state))?,
        blob,
    ];
    store.pin(&Pin {
        schema: 1,
        id: "retained-saved-type-owner".into(),
        reason:
            "Observed entry-epoch catalog metadata only; current objt source remains unqualified"
                .into(),
        blobs: blobs.clone(),
    })?;
    let report = serde_json::json!({"schema":1,"input_sha256":sims_fixtures::digest(&input_raw),"definitions_sha256":sims_fixtures::digest(&definitions_raw),
        "definitions":catalog.entries.len(),"saved_indices":state.saved_indices.len(),"prior_declarations":state.declarations.len(),"prior_type_guids":state.type_guids,
        "typed_metadata_imported":true,"cold_equal":true,"pending_source_retry_unchanged":true,"catalog_unchanged":true,"stop":stop,"events":host.events,"blobs":blobs,
        "entry_epoch_component_only":true,"full_resource_owner_imported":false,"post_clear_metadata_equality_claimed":false,"house_suffix_executed":false,"original_gameplay_ticks_added":0});
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
