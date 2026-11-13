//! Existing original-derived component corpora, independent of household replay.
//! All host replies are explicit inputs of the pinned component contracts.
use super::{Adapter, AdapterResult, Scope, Status};
use crate::fixture_replay::first_difference;
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sims_fixtures::{ArtifactFormat, CaseManifest, Coverage, Store};
use sims_vm::scheduler::clock::{Fields, Frame, Inputs, Request};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const SAVED_RNG_CONTRACT: &str = "native-saved-rng-v1";
pub const SCHEDULER_CLOCK_CONTRACT: &str = "native-scheduler-clock-gameplay-v1";
const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CALLBACKS_PER_ROW: usize = 1000;
const EXE: &str = "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58";
const VM_CAPTURE: &str = "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb";
const GAME_CAPTURE: &str = "a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedInput {
    lot: u32,
    global: u32,
    residential: bool,
    timers: [u32; 2],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedOriginal {
    lot: u32,
    global_: u32,
    next: u32,
    after: u32,
    /// Preserved in the raw source, outside the existing Rust function contract.
    #[serde(rename = "timer_calls")]
    _timer_calls: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedRow {
    input: SavedInput,
    original: SavedOriginal,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedReport {
    schema: u32,
    source_sha256: String,
    capture_sha256: String,
    boundaries: String,
    cases: Vec<SavedRow>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    returned: Option<u32>,
    mutation: Option<Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClockInput {
    kind: String,
    fields: Value,
    inputs: Value,
    actions: BTreeMap<String, Vec<Action>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClockRow {
    input: ClockInput,
    original: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClockReport {
    scope: String,
    source_sha256: String,
    capture_hashes: BTreeMap<String, String>,
    original_execution: bool,
    supplied_instructions: bool,
    peek_message_calls: u32,
    cases: Vec<ClockRow>,
}

struct Evaluation {
    expected: Value,
    actual: Value,
    observations: u64,
    detail: &'static str,
}

fn saved_rng(bytes: &[u8]) -> Result<Evaluation> {
    let report: SavedReport = serde_json::from_slice(bytes).context("saved RNG corpus JSON")?;
    ensure!(report.schema == 1, "unsupported saved RNG corpus schema");
    ensure!(report.source_sha256 == EXE && report.capture_sha256 == VM_CAPTURE,
        "saved RNG source profile differs");
    ensure!(!report.boundaries.is_empty() && report.cases.len() == 162,
        "saved RNG corpus must declare its 162 original component cases");
    let mut expected = Vec::with_capacity(report.cases.len());
    let mut actual = Vec::with_capacity(report.cases.len());
    for (index, row) in report.cases.iter().enumerate() {
        let c = &row.input;
        let [lot, mut global] = sims_vm::rng::load_saved(
            c.lot, c.global, c.residential, Some(c.timers),
        ).map_err(anyhow::Error::msg).with_context(|| format!("saved RNG row {index}"))?;
        let loaded_global = global;
        let next = sims_vm::rng::next(&mut global);
        let n = &row.original;
        expected.push(json!({"original": {"lot":n.lot,"global_":n.global_,"next":n.next,"after":n.after}}));
        actual.push(json!({"original": {"lot":lot,"global_":loaded_global,"next":next,"after":global}}));
    }
    Ok(Evaluation {
        expected: json!({"cases":expected}), actual: json!({"cases":actual}),
        observations: report.cases.len() as u64,
        detail: "Original-derived SIMI reseeding and first draw only. Explicit serialized seeds, residential reply and timer readings are supplied inputs. Compares all four fields of the existing contract: lot/global_/next/after. Raw timer_calls is retained but not compared: the production function consumes supplied readings and does not expose the native host-producer call trace. No CRT/factory/household execution or completed ticks are claimed.",
    })
}

fn exact_keys(value: &Value, keys: &[&str], label: &str) -> Result<()> {
    let object = value.as_object().with_context(|| format!("{label} must be an object"))?;
    ensure!(object.len() == keys.len() && keys.iter().all(|k| object.contains_key(*k)),
        "unknown or missing {label} fields");
    Ok(())
}

fn clock_state(value: &Value) -> Result<(Fields, Inputs)> {
    exact_keys(value, &["fields", "inputs"], "clock state")?;
    exact_keys(&value["fields"], &["timing", "period", "timer", "budget_d8", "budget_dc", "trap", "publish"], "clock fields")?;
    exact_keys(&value["fields"]["timing"], &["last", "elapsed"], "clock timing")?;
    exact_keys(&value["fields"]["timer"], &["elapsed", "interval", "last", "limit"], "clock timer")?;
    exact_keys(&value["inputs"], &["now", "stopping", "step_word"], "clock inputs")?;
    Ok((serde_json::from_value(value["fields"].clone())?,
        serde_json::from_value(value["inputs"].clone())?))
}

/// The original producer merges supplied callback writes into existing fields.
/// Unknown paths are not silently ignored by Fields' permissive serde decoder.
fn merge_existing(value: &mut Value, mutation: &Value) -> Result<()> {
    let changes = mutation.as_object().context("clock mutation must be an object")?;
    let target = value.as_object_mut().context("clock mutation path is not an object")?;
    for (key, update) in changes {
        let old = target.get_mut(key).with_context(|| format!("unknown clock mutation path {key}"))?;
        if update.is_object() {
            merge_existing(old, update)?;
        } else {
            *old = update.clone();
        }
    }
    Ok(())
}

fn scheduler_clock(bytes: &[u8]) -> Result<Evaluation> {
    let report: ClockReport = serde_json::from_slice(bytes).context("clock corpus JSON")?;
    ensure!(report.source_sha256 == EXE && report.original_execution && !report.supplied_instructions,
        "clock corpus is not the recovered original-instruction profile");
    let hashes = [
        ("capture-000000ec-00010000.bin", GAME_CAPTURE),
        ("capture-000000ec-68880000.bin", "c4390b357f860db7a7ec371178e1f600a34380555a8249f9e40b00f52850aa8a"),
        ("capture-000000ec-7b600000.bin", "6116f5327d845c0076f915b7a8b95a6662425cffdc8e578c1c3713f461b854d6"),
    ];
    ensure!(report.capture_hashes.len() == hashes.len()
        && hashes.iter().all(|(k,v)| report.capture_hashes.get(*k).is_some_and(|s| s == v)),
        "clock captured code/export sources differ");
    ensure!(!report.scope.is_empty() && report.cases.len() == 239 && report.peek_message_calls == 89,
        "clock corpus must declare the existing 239 cases/89 PeekMessage calls");
    let mut expected = Vec::with_capacity(report.cases.len());
    let mut actual = Vec::with_capacity(report.cases.len());
    let mut peek_count = 0u32;
    for (index, row) in report.cases.iter().enumerate() {
        let c = &row.input;
        ensure!(c.kind == "frame", "unsupported clock row kind at {index}");
        for key in c.actions.keys() {
            let request: Request = serde_json::from_value(json!(key))?;
            ensure!(request != Request::DebugBreak, "protected clock boundary is unsupported");
        }
        let (mut fields, mut inputs) = clock_state(&json!({"fields":c.fields,"inputs":c.inputs}))?;
        let mut call = Frame::default();
        let mut events = Vec::new();
        let mut counts = BTreeMap::<String, usize>::new();
        while let Some(request) = call.advance(&mut fields, &mut inputs).map_err(anyhow::Error::msg)? {
            ensure!(events.len() < MAX_CALLBACKS_PER_ROW, "clock callback bound at row {index}");
            let mut state = json!({"fields":fields,"inputs":inputs});
            events.push(json!({"request":request,"state":state}));
            // The pinned existing contract also verifies every pending callback
            // through a real serialization retry, without rerunning the child.
            call = serde_json::from_slice(&serde_json::to_vec(&call)?)?;
            ensure!(call.advance(&mut fields, &mut inputs).map_err(anyhow::Error::msg)? == Some(request),
                "clock cold retry lost pending callback at row {index}");
            ensure!(state == json!({"fields":fields,"inputs":inputs}),
                "clock cold retry mutated pending state at row {index}");
            let key = serde_json::to_value(request)?.as_str().context("clock request is not a string")?.to_owned();
            let count = counts.entry(key.clone()).or_default();
            let action = c.actions.get(&key).and_then(|a| a.get(*count));
            *count += 1;
            if let Some(mutation) = action.and_then(|a| a.mutation.as_ref()) {
                merge_existing(&mut state, mutation)?;
                (fields, inputs) = clock_state(&state)?;
            }
            // Exactly the original component producer's supplied-service rule:
            // absent action/returned means reply0, with no mutation. This is not
            // a default for a real household or an unknown engine host.
            let returned = action.and_then(|a| a.returned).unwrap_or(0);
            call.acknowledge(request, returned).map_err(anyhow::Error::msg)?;
            if request == Request::PeekMessage {
                peek_count += 1;
            }
        }
        ensure!(call.complete(), "clock row {index} did not complete");
        expected.push(json!({"original":row.original}));
        actual.push(json!({"original": {"events":events,"state":{"fields":fields,"inputs":inputs},"trapped":call.trapped()}}));
    }
    Ok(Evaluation {
        expected: json!({"cases":expected,"peek_message_calls":report.peek_message_calls}),
        actual: json!({"cases":actual,"peek_message_calls":peek_count}),
        observations: report.cases.len() as u64,
        detail: "Original-derived recovered elapsed-clock/message-poll component, with exact ordered callbacks, all event/final state fields and pending-callback serialization retries. Application/registry predicates, speed changes, UpdateLot, requests, publication and OS replies are supplied child services. The pinned producer explicitly supplies reply0/no mutation for absent callback actions. Historical wire field trap means poll_messages; protected DebugBreak profiles are rejected. No uninterrupted household execution or completed ticks are claimed.",
    })
}

fn write_new(path: &Path, value: &Value) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}

/// Run against immutable fixture bytes; write only this worker's existing new
/// output directory. No publication/checkpoint/log is added to the source Store.
pub fn run(store: &Store, case: &CaseManifest, adapter: &Adapter, output: &Path) -> Result<AdapterResult> {
    case.validate()?;
    let contract = match adapter {
        Adapter::SavedRng => SAVED_RNG_CONTRACT,
        Adapter::SchedulerClock => SCHEDULER_CLOCK_CONTRACT,
        _ => bail!("not a component adapter"),
    };
    ensure!(matches!(&case.coverage, Coverage::Component { scope, .. } if scope == contract),
        "unknown or mismatched declared component contract; expected {contract}");
    ensure!(case.source.format == ArtifactFormat::Json && case.source.schema == 1,
        "component source requires Json artifact schema1");
    ensure!(case.schedule.is_empty() && case.references.is_empty() && case.screenshots.is_empty(),
        "component source is self-describing; external schedule/reference/UI inputs are unsupported");
    let output_root = fs::canonicalize(output).context("existing worker output directory")?;
    ensure!(output_root.is_dir() && !output_root.starts_with(fs::canonicalize(store.root())?),
        "component worker output cannot be inside source Store");
    let bytes = store.read_bytes(&case.source.blob, MAX_SOURCE_BYTES)?;
    let evaluation = match adapter {
        Adapter::SavedRng => saved_rng(&bytes)?,
        Adapter::SchedulerClock => scheduler_clock(&bytes)?,
        _ => unreachable!("adapter checked before source access"),
    };
    let difference = first_difference(&evaluation.expected, &evaluation.actual);
    let status = if difference.is_some() { Status::Diverged } else { Status::Passed };
    write_new(&output_root.join("component-actual.json"), &json!({
        "schema":1,"contract":contract,"source":case.source.blob,
        "observations":evaluation.observations,"actual":evaluation.actual,
    }))?;
    write_new(&output_root.join("component-comparison.json"), &json!({
        "schema":1,"status":status,"scope":Scope::OriginalComponent,"contract":contract,
        "observations":evaluation.observations,"completed_ticks":0,"matched_completed_ticks":0,
        "difference":difference,"limitations":evaluation.detail,
    }))?;
    Ok(AdapterResult {
        status, scope: Scope::OriginalComponent, contract: contract.into(),
        observations: evaluation.observations, completed_ticks: 0, matched_completed_ticks: 0,
        difference, detail: Some(evaluation.detail.into()),
        artifacts: vec![PathBuf::from("component-actual.json"), PathBuf::from("component-comparison.json")],
    })
}

#[cfg(feature = "private-fixtures")]
#[cfg(test)]
mod tests;
