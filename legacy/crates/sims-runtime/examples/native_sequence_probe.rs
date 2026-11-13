//! Bounded retained-caller diagnostic. It cannot admit more original ticks.
use anyhow::{Context, Result, ensure};
use sims_fixtures::Store;
use sims_runtime::conformance::native_household::{Prepared, sequence::RoutineInput};
use std::{fs, path::Path};
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceInputs {
    routines: Vec<RoutineInput>,
    animation_cache: AnimationInput,
    #[serde(default)]
    tuning_cache: Option<AnimationInput>,
    #[serde(default)]
    score_strings: Option<AnimationInput>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AnimationInput {
    path: String,
    sha256: String,
    after_tick: u64,
}
fn read_cache(input: &AnimationInput) -> Result<serde_json::Value> {
    ensure!(
        fs::metadata(&input.path)?.len() <= 4 << 20,
        "cache input exceeds bound"
    );
    let raw = fs::read(&input.path)?;
    ensure!(
        sims_fixtures::digest(&raw) == input.sha256,
        "cache input hash differs"
    );
    Ok(serde_json::from_slice(&raw)?)
}
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    if a.len() == 4 && a[0] == "export-trace" {
        let report: serde_json::Value = serde_json::from_slice(&fs::read(&a[2])?)?;
        let reference = serde_json::from_value(report["trace"].clone())?;
        let trace: Vec<sims_vm::Trace> = Store::open(&a[1])?.get_value(&reference)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&a[3])?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec(&trace)?)?;
        return Ok(());
    }
    ensure!(
        a.len() == 5 || a.len() == 6,
        "usage: native_sequence_probe STORE CASE IMMUTABLE NEW_DIR TICKS [ROUTINE_INPUT_JSON]"
    );
    let ticks: u64 = a[4].parse()?;
    ensure!(
        (1..=32).contains(&ticks),
        "sequence probe bound must be1..32 ticks"
    );
    let store = Store::open(&a[0])?;
    let case = store.load_case(&a[1])?;
    let immutable = Path::new(&a[2]).canonicalize()?;
    let output = Path::new(&a[3]);
    let parent = output
        .parent()
        .context("output parent absent")?
        .canonicalize()?;
    ensure!(
        !parent.starts_with(store.root().canonicalize()?) && !parent.starts_with(&immutable),
        "output overlaps fixture or immutable source"
    );
    ensure!(!output.exists(), "probe output must be new");
    fs::metadata(&parent)?;
    let prepared = Prepared::load(&store, &case, &immutable, "UserData/Houses/House05.iff")?
        .context("qualified baseline unavailable")?;
    let inputs: serde_json::Value = a
        .get(5)
        .map(|p| -> Result<_> { Ok(serde_json::from_slice(&fs::read(p)?)?) })
        .transpose()?
        .unwrap_or_else(|| serde_json::json!([]));
    let report = if inputs.is_array() {
        prepared.probe_sequence_with_inputs(
            ticks,
            &serde_json::from_value::<Vec<RoutineInput>>(inputs)?,
            output,
        )?
    } else {
        let input: ResourceInputs = serde_json::from_value(inputs)?;
        let animation = read_cache(&input.animation_cache)?;
        let tuning = input.tuning_cache.as_ref().map(read_cache).transpose()?;
        let scores = input.score_strings.as_ref().map(read_cache).transpose()?;
        ensure!(
            input
                .tuning_cache
                .as_ref()
                .is_none_or(|t| t.after_tick == input.animation_cache.after_tick),
            "Global cache inputs have different installation cuts"
        );
        ensure!(
            input
                .score_strings
                .as_ref()
                .is_none_or(|s| s.after_tick == input.animation_cache.after_tick),
            "score strings have different installation cut"
        );
        prepared.probe_sequence_with_global_inputs(
            ticks,
            &input.routines,
            sims_runtime::conformance::native_household::sequence::GlobalInputs {
                after_tick: input.animation_cache.after_tick,
                animation_cache: Some(&animation),
                tuning_cache: tuning.as_ref(),
                score_strings: scores.as_ref(),
            },
            output,
        )?
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
