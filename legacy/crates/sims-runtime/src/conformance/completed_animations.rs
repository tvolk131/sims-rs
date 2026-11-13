//! Compare the captured resident animation component with actual Rust owners.
//! This does not admit a complete baseline or execute a gameplay tick.
use super::{AdapterResult, Scope, Status};
use crate::original_session::completed_animations::{self, AssetInput, CharacterInput};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sims_fixtures::{ArtifactFormat, CaseManifest, Coverage, Store};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};
pub const CONTRACT: &str = "completed-native-animation-component-v1";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u32,
    contract: String,
    tick: u64,
    native_parity_ticks: u64,
    payload: Payload,
    characters: Vec<CharacterInput>,
    unqualified: Vec<String>,
    adapter_policy: String,
    provenance: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    schema: u32,
    assets: Vec<AssetInput>,
    reads: Value,
    code: Value,
    observed_bytes: u64,
    executes_target_code: bool,
    resumable_baseline_qualified: bool,
    unresolved: Vec<String>,
}
pub fn import_parts(value: &Value) -> Result<completed_animations::Imported> {
    // Reuse every original profile/provenance/clock/binding check before moving
    // these owners into another separately scoped native component.
    evaluate(serde_json::from_value(value.clone())?)?;
    let i: Input = serde_json::from_value(value.clone())?;
    completed_animations::import(
        &i.payload.assets,
        &i.characters,
        i.provenance["image_sha256"].as_str().unwrap(),
    )
}
fn evaluate(input: Input) -> Result<Value> {
    ensure!(
        input.schema == 1
            && input.contract == CONTRACT
            && input.tick == 88448
            && input.native_parity_ticks == 0,
        "animation component schema/credit differs"
    );
    ensure!(
        input.payload.schema == 1
            && !input.payload.executes_target_code
            && !input.payload.resumable_baseline_qualified
            && input.payload.unresolved.len() >= 3
            && input.unqualified.len() >= 5
            && input
                .adapter_policy
                .starts_with("Fresh unpublished LayerIds"),
        "animation component exclusions/policy absent"
    );
    ensure!(
        input.provenance["image_sha256"]
            == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && input.provenance["observation_sha256"]
                == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c",
        "original animation source profile differs"
    );
    ensure!(
        input.payload.assets.len() == 4
            && input.characters.len() == 3
            && input
                .characters
                .iter()
                .map(|c| c.object)
                .collect::<Vec<_>>()
                == [16, 18, 21],
        "original animation coverage differs"
    );
    ensure!(
        input.payload.reads.is_array()
            && input.payload.code.as_array().is_some_and(|v| v.len() == 3)
            && input.payload.observed_bytes > 0,
        "animation read/code provenance absent"
    );
    let epoch = input.provenance["image_sha256"].as_str().unwrap();
    let imported = completed_animations::import(&input.payload.assets, &input.characters, epoch)?;
    let mut observations = vec![];
    let mut layers = 0;
    for (id, c) in &imported.characters {
        let native = input.characters.iter().find(|v| v.object == *id).unwrap();
        ensure!(
            native.bone_pointers.len() == 29
                && c.clock == 4422350
                && c.engaged == 1
                && c.animation_events == 0,
            "captured person timing/control profile differs"
        );
        let saved = postcard::to_allocvec(&c.animator)?;
        let restored: sims_sim::pose::animator::Animator = postcard::from_bytes(&saved)?;
        ensure!(
            restored == c.animator && postcard::to_allocvec(&restored)? == saved,
            "cold animator component changes state"
        );
        layers += c.animator.layers().len();
        observations.push(json!({"object":id,"layers":c.animator.layers().len(),"clock":c.clock,"last_evaluated_tick":88447,"native_clock_equal":true,"cold_binary_roundtrip_equal":true,"saved_sha256":sims_fixtures::digest(&saved)}));
    }
    ensure!(layers == 6, "captured live layer coverage differs");
    Ok(
        json!({"schema":1,"contract":CONTRACT,"status":"passed","scope":"original_component","tick":input.tick,"native_parity_ticks":0,"assets":imported.assets.len(),"layers":layers,"characters":observations,"adapter_policy":input.adapter_policy,"unqualified":input.unqualified,"provenance":input.provenance}),
    )
}
pub fn run(store: &Store, manifest: &CaseManifest, output: &Path) -> Result<AdapterResult> {
    ensure!(
        manifest.source.format == ArtifactFormat::Json
            && manifest.source.schema == 1
            && matches!(&manifest.coverage,Coverage::Component{scope,..}if scope==CONTRACT),
        "animation fixture contract differs"
    );
    let input: Input = serde_json::from_slice(&store.read_bytes(&manifest.source.blob, 16 << 20)?)?;
    let report = evaluate(input)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("completed-animations.json"))?;
    serde_json::to_writer_pretty(&mut f, &report)?;
    f.write_all(b"\n")?;
    Ok(AdapterResult{status:Status::Passed,scope:Scope::OriginalComponent,contract:CONTRACT.into(),observations:3,completed_ticks:0,matched_completed_ticks:0,difference:None,
        detail:Some("4 resident clips, 6 live layers and 3 animator clocks match actual Rust animation owners, including cold binary roundtrip; full Character/VM/World/Session and gameplay progression remain unqualified".into()),artifacts:vec![PathBuf::from("completed-animations.json")]})
}
