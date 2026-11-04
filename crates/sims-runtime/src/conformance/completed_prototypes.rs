//! Loaded skeleton resource ownership, independent of current animated poses.
use super::{AdapterResult, Scope, Status};
use crate::original_session::completed_prototypes::{self, Imported};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sims_fixtures::{ArtifactFormat, CaseManifest, Coverage, Store};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};
pub const CONTRACT: &str = "completed-native-prototypes-component-v1";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u32,
    contract: String,
    tick: u64,
    native_parity_ticks: u64,
    resident_input: Value,
    graph: Value,
    catalogs: Value,
    provenance: Value,
    scope: String,
}
pub fn import_parts(v: &Value) -> Result<Imported> {
    let i: Input = serde_json::from_value(v.clone())?;
    ensure!(
        i.schema == 1
            && i.contract == CONTRACT
            && i.tick == 88448
            && i.native_parity_ticks == 0
            && !i.scope.is_empty(),
        "prototype component profile differs"
    );
    ensure!(
        i.provenance == i.resident_input["provenance"],
        "prototype/resident provenance differs"
    );
    let characters = super::completed_residents::import_parts(&i.resident_input)?;
    completed_prototypes::import(
        &i.graph,
        &i.catalogs,
        characters,
        i.provenance["image_sha256"]
            .as_str()
            .context("prototype epoch absent")?,
    )
}
fn evaluate(v: &Value) -> Result<Value> {
    let imported = import_parts(v)?;
    ensure!(
        imported.prototypes.len() == 5
            && imported
                .prototypes
                .iter()
                .map(|p| p.bone_raw.len())
                .sum::<usize>()
                == 173,
        "prototype resource coverage differs"
    );
    let saved = postcard::to_allocvec(&imported)?;
    let cold: Imported = postcard::from_bytes(&saved)?;
    ensure!(
        cold == imported && postcard::to_allocvec(&cold)? == saved,
        "prototype binary cold restore differs"
    );
    for (&id, resident) in &cold.characters.residents {
        let name = if id == 16 { "child" } else { "adult" };
        ensure!(
            resident.source.as_ref().is_some_and(|s| s.name == name),
            "captured resident source absent"
        );
    }
    Ok(
        json!({"schema":1,"contract":CONTRACT,"status":"passed","scope":"original_component","tick":88448,"native_parity_ticks":0,"prototype_catalogs":5,"prototype_bones":173,"source_bound_residents":3,"original_noise_retained":true,"constructors_or_rng_executed":false,"binary_cold_roundtrip_equal":true,"saved_sha256":sims_fixtures::digest(&saved),"resumable_world_baseline":false,"unqualified":cold.characters.demands,"provenance":v["provenance"]}),
    )
}
pub fn run(store: &Store, m: &CaseManifest, output: &Path) -> Result<AdapterResult> {
    ensure!(
        m.source.format == ArtifactFormat::Json
            && m.source.schema == 1
            && matches!(&m.coverage,Coverage::Component{scope,..}if scope==CONTRACT),
        "prototype fixture contract differs"
    );
    let report = evaluate(&serde_json::from_slice(
        &store.read_bytes(&m.source.blob, 16 << 20)?,
    )?)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("completed-prototypes.json"))?;
    serde_json::to_writer_pretty(&mut f, &report)?;
    f.write_all(b"\n")?;
    Ok(AdapterResult{status:Status::Passed,scope:Scope::OriginalComponent,contract:CONTRACT.into(),observations:173,completed_ticks:0,matched_completed_ticks:0,difference:None,detail:Some("5 actual loaded skeleton catalogs/173 prototype bones and original noise records bind3Residents to immutable source metadata without constructors/RNG; asset manager and whole-World admission remain explicit demands".into()),artifacts:vec![PathBuf::from("completed-prototypes.json")]})
}
#[cfg(test)]
mod tests {
    use super::*;
    fn actual() -> Value {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../fixtures/private/goth-resource-closure-20261028-first/prototypes-input.json",
        );
        serde_json::from_slice(
            &std::fs::read(p).expect("private frozen Goth prototype input required"),
        )
        .unwrap()
    }
    #[test]
    #[ignore = "requires private frozen original prototype graph and catalog payloads"]
    fn actual_prototypes_cold_restore_without_live_pose_changes() {
        let v = actual();
        let before = super::super::completed_residents::import_parts(&v["resident_input"]).unwrap();
        let report = evaluate(&v).unwrap();
        assert_eq!(report["source_bound_residents"], 3);
        let after = import_parts(&v).unwrap();
        for (id, r) in before.residents {
            let mut expected = r;
            expected.source = after.characters.residents[&id].source.clone();
            assert_eq!(
                postcard::to_allocvec(&expected).unwrap(),
                postcard::to_allocvec(&after.characters.residents[&id]).unwrap()
            );
        }
    }
    fn overwrite(v: &mut Value, key: &str, offset: usize, value: u32) {
        let mut b =
            crate::original_session::completed_animations::bytes(v[key].as_str().unwrap()).unwrap();
        b[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        v[key] = json!(b.iter().map(|b| format!("{b:02x}")).collect::<String>());
    }
    #[test]
    #[ignore = "requires private frozen original prototype graph and catalog payloads"]
    fn actual_prototypes_reject_source_noise_property_and_registry_mismatches() {
        let original = actual();
        for kind in 0..12 {
            let mut v = original.clone();
            match kind {
                0 => v["graph"]["code"][0]["sha256"] = json!("bad"),
                1 => v["graph"]["actors"][0]["prototype"] = json!(1),
                2 => overwrite(&mut v["graph"]["records"][0], "raw", 0x34, 0),
                3 => v["graph"]["properties"][0]["pairs"][0]["value_raw"] = json!("626164"),
                4 => overwrite(&mut v["graph"]["bones"][0], "raw", 0x30, 12.5f32.to_bits()),
                5 => overwrite(&mut v["graph"]["bones"][0], "raw", 0xe8, 123),
                6 => v["graph"]["managers"][0]["chains"][0] = json!([1]),
                7 => v["graph"]["records"][0]["bucket"] = json!(0),
                8 => {
                    v["catalogs"]["catalogs"]["child-skeleton.cmx.bcf"][0]["sha256"] = json!("bad")
                }
                9 => {
                    v["catalogs"]["catalogs"]["child-skeleton.cmx.bcf"][0]["locator"] =
                        json!("UserData/skeleton.bcf")
                }
                10 => {
                    v["graph"]["bones"].as_array_mut().unwrap().pop();
                }
                _ => {
                    let c = v["catalogs"]["catalogs"]["child-skeleton.cmx.bcf"][0].clone();
                    v["catalogs"]["catalogs"]["child-skeleton.cmx.bcf"]
                        .as_array_mut()
                        .unwrap()
                        .push(c);
                }
            }
            assert!(
                import_parts(&v).is_err(),
                "synthetic mutation {kind} was accepted"
            );
        }
    }
    #[test]
    #[ignore = "requires private frozen original prototype graph and catalog payloads"]
    fn actual_noise_records_survive_and_sources_bind_future_layers() {
        let mut v = actual();
        overwrite(&mut v["graph"]["bones"][0], "raw", 0xec, 0x12345678);
        let imported = import_parts(&v).unwrap();
        let saved = serde_json::to_value(&imported.skeletons).unwrap();
        let child = &saved["catalogs"]["GameData_ALL_Recursive/Animation/Animation.far::child-skeleton.cmx.bcf"]
            ["resources"][0];
        assert_eq!(child["wiggle"][0]["records"][0][0], 0x12345678);
        let animations = super::super::completed_animations::import_parts(
            &v["resident_input"]["skin_input"]["skeleton_input"]["animation_input"],
        )
        .unwrap();
        let bytes = postcard::to_allocvec(&imported).unwrap();
        let cold: Imported = postcard::from_bytes(&bytes).unwrap();
        for r in cold.characters.residents.values() {
            for layer in r.character.skin.animator.layers() {
                let clip = &animations.assets[&layer.layer.name].clip;
                let next = sims_sim::pose::layer::Layer::bind(
                    r.source.as_ref().unwrap(),
                    clip,
                    sims_sim::pose::layer::Start {
                        priority: layer.layer.priority,
                        speed: 1.,
                        weight: 1.,
                        now: r.character.clock,
                    },
                )
                .unwrap();
                assert_eq!(
                    next.bindings
                        .iter()
                        .map(|b| (b.bone, b.motion))
                        .collect::<Vec<_>>(),
                    layer
                        .layer
                        .bindings
                        .iter()
                        .map(|b| (b.bone, b.motion))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
    #[test]
    #[ignore = "requires private captured graph and immutable game asset installation"]
    fn actual_appearance_host_reconstructs_without_starting_saves() {
        let v = actual();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted");
        let mut index = sims_content::Index::scan(&root).unwrap();
        index
            .resources
            .retain(|r| !r.locator.split('/').any(|s| s.starts_with("UserData")));
        let before = import_parts(&v).unwrap();
        let saved = postcard::to_allocvec(&before).unwrap();
        let proof:Value=serde_json::from_slice(&std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-resource-closure-20261028-first/appearance-code-first.json")).unwrap()).unwrap();
        let restored = crate::original_session::completed_appearances::import(
            &v["resident_input"]["resident_input"],
            &proof,
            before,
            &index,
        )
        .unwrap();
        assert_eq!(restored.saved.len(), 3);
        assert_eq!(postcard::to_allocvec(&restored.characters).unwrap(), saved);
        let binary = postcard::to_allocvec(&restored).unwrap();
        let cold: crate::original_session::completed_appearances::Imported =
            postcard::from_bytes(&binary).unwrap();
        assert_eq!(postcard::to_allocvec(&cold).unwrap(), binary);
    }
}
