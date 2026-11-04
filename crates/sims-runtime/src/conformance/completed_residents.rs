//! Whole character storage component; explicitly not whole-session admission.
use super::{AdapterResult, Scope, Status};
use crate::original_session::completed_residents::{self, Imported};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sims_fixtures::{ArtifactFormat, CaseManifest, Coverage, Store};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};
pub const CONTRACT: &str = "completed-native-residents-component-v1";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u32,
    contract: String,
    tick: u64,
    native_parity_ticks: u64,
    skin_input: Value,
    geometry_input: Value,
    resident_input: Value,
    provenance: Value,
    scope: String,
}
/// Import retained character owners without executing their updates. This is
/// a component import; its unresolved asset/Session demands remain explicit.
pub fn import_parts(v: &Value) -> Result<Imported> {
    let i: Input = serde_json::from_value(v.clone())?;
    ensure!(
        i.schema == 1
            && i.contract == CONTRACT
            && i.tick == 88448
            && i.native_parity_ticks == 0
            && !i.scope.is_empty(),
        "native resident component profile differs"
    );
    ensure!(
        i.provenance["image_sha256"] == i.skin_input["provenance"]["image_sha256"]
            && i.provenance["observation_sha256"]
                == i.skin_input["provenance"]["observation_sha256"],
        "native resident source epoch differs"
    );
    let proof = i.skin_input["proof"]["actors"]
        .as_array()
        .context("resident skin source actors absent")?;
    for actor in i.resident_input["actors"]
        .as_array()
        .context("resident actors absent")?
    {
        let parent = proof
            .iter()
            .find(|p| p["object"] == actor["object"])
            .context("resident parent actor absent")?;
        ensure!(
            actor["animator_raw"] == parent["animator_raw"],
            "resident/qualified skin animator epoch differs"
        );
        let geometry = i.geometry_input["characters"]
            .as_array()
            .context("resident geometry absent")?
            .iter()
            .find(|p| p["object"] == actor["object"])
            .context("resident geometry actor absent")?;
        ensure!(
            geometry["model_raw"] == parent["model_raw"]
                && geometry["skin_raw"] == parent["skin_raw"],
            "resident/qualified skin model epoch differs"
        );
    }
    let skins = super::completed_skin::import_parts(&i.skin_input)?;
    completed_residents::import(
        &i.resident_input,
        skins,
        &i.geometry_input,
        i.provenance["image_sha256"]
            .as_str()
            .context("resident epoch absent")?,
        i.tick,
    )
}
fn evaluate(v: &Value) -> Result<Value> {
    let imported = import_parts(v)?;
    ensure!(
        imported.residents.keys().copied().collect::<Vec<_>>() == [16, 18, 21]
            && imported.geometry.meshes.meshes.len() == 13
            && imported
                .geometry
                .characters
                .values()
                .map(|c| c.geometry.state.appearances.len())
                .sum::<usize>()
                == 12,
        "native resident graph coverage differs"
    );
    let binary = postcard::to_allocvec(&imported)?;
    let cold: Imported = postcard::from_bytes(&binary)?;
    ensure!(
        imported == cold && postcard::to_allocvec(&cold)? == binary,
        "native resident binary cold restore differs"
    );
    let mut program = sims_vm::Program::default();
    let animations = super::completed_animations::import_parts(
        &v["skin_input"]["skeleton_input"]["animation_input"],
    )?;
    program.animations = animations.assets;
    for (&id, resident) in &cold.residents {
        resident
            .character
            .validate(&program)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            resident.evaluated_tick == 88447
                && resident.character.clock == 4422350
                && resident
                    .controls
                    .movement
                    .path
                    .as_ref()
                    .is_some_and(|p| p.points.is_empty()),
            "native resident current clocks/path ownership differs for {id}"
        );
    }
    let vertices = imported
        .geometry
        .characters
        .values()
        .flat_map(|c| c.geometry.vertices.values())
        .map(|v| v.vertices.len())
        .sum::<usize>();
    Ok(
        json!({"schema":1,"contract":CONTRACT,"status":"passed","scope":"original_component","tick":88448,"native_parity_ticks":0,"residents":3,"models":3,"appearance_instances":12,"mesh_bindings":13,"shared_meshes":13,"cpu_vertices":vertices,"current_transforms_head_movement_and_clocks_restored":true,"no_constructor_pose_or_geometry_refresh":true,"binary_cold_roundtrip_equal":true,"saved_sha256":sims_fixtures::digest(&binary),"resumable_world_baseline":false,"immutable_skeleton_source_qualified":false,"unqualified":imported.demands,"provenance":v["provenance"]}),
    )
}
pub fn run(store: &Store, m: &CaseManifest, output: &Path) -> Result<AdapterResult> {
    ensure!(
        m.source.format == ArtifactFormat::Json
            && m.source.schema == 1
            && matches!(&m.coverage,Coverage::Component{scope,..}if scope==CONTRACT),
        "native residents fixture contract differs"
    );
    let report = evaluate(&serde_json::from_slice(
        &store.read_bytes(&m.source.blob, 16 << 20)?,
    )?)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("completed-residents.json"))?;
    serde_json::to_writer_pretty(&mut f, &report)?;
    f.write_all(b"\n")?;
    Ok(AdapterResult{status:Status::Passed,scope:Scope::OriginalComponent,contract:CONTRACT.into(),observations:3,completed_ticks:0,matched_completed_ticks:0,difference:None,detail:Some("Three actual character Residents/models,12appearance instances,13shared mesh/cache/CPU drawable owners restore cold without update or RNG; immutable source skeletons, asset-manager owners and World/session admission remain explicit demands".into()),artifacts:vec![PathBuf::from("completed-residents.json")]})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn private_input() -> Value {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../fixtures/private/goth-character-residents-20261027-third/residents-input.json",
        );
        serde_json::from_slice(
            &std::fs::read(p).expect("private frozen Goth resident fixture required"),
        )
        .unwrap()
    }
    #[test]
    #[ignore = "requires private captured original source graph"]
    fn private_actual_residents_cold_roundtrip() {
        let report = evaluate(&private_input()).unwrap();
        assert_eq!(report["residents"], 3);
        assert_eq!(report["native_parity_ticks"], 0);
        assert_eq!(report["resumable_world_baseline"], false);
    }
    fn overwrite(v: &mut Value, key: &str, offset: usize, n: u32) {
        let mut b = super::super::super::original_session::completed_animations::bytes(
            v[key].as_str().unwrap(),
        )
        .unwrap();
        b[offset..offset + 4].copy_from_slice(&n.to_le_bytes());
        v[key] = json!(b.iter().map(|x| format!("{x:02x}")).collect::<String>());
    }
    #[test]
    #[ignore = "requires private captured original source graph"]
    fn private_actual_residents_reject_wrong_owners_and_source() {
        let original = private_input();
        for kind in 0..12 {
            let mut v = original.clone();
            match kind {
                0 => v["tick"] = json!(88449),
                1 => v["resident_input"]["actors"][0]["person"] = json!(1),
                2 => overwrite(
                    &mut v["resident_input"]["actors"][0],
                    "animator_raw",
                    0xb4,
                    4422400,
                ),
                3 => overwrite(
                    &mut v["resident_input"]["actors"][0],
                    "animator_raw",
                    0x188,
                    1,
                ),
                4 => v["resident_input"]["actors"][0]["property_context"]["selected"] = json!(true),
                5 => v["geometry_input"]["bindings"][0]["drawable"] = json!(1),
                6 => v["geometry_input"]["meshes"][0]["bone_ids"]["used_raw"] = json!("ffffffff"),
                7 => v["geometry_input"]["code"][0]["sha256"] = json!("bad"),
                8 => v["resident_input"]["actors"][0]["path"] = Value::Null,
                9 => v["skin_input"]["proof"]["callback"]["raw"] = json!("0000000000000000"),
                10 => {
                    v["resident_input"]["actors"][0]["property_context"]["unowned_future_state"] =
                        json!(1)
                }
                _ => {
                    let vector = &mut v["geometry_input"]["bindings"][0]["bones"];
                    for k in ["begin", "end", "capacity"] {
                        vector[k] = json!(vector[k].as_u64().unwrap() + 4);
                    }
                }
            }
            assert!(
                import_parts(&v).is_err(),
                "mutation {kind} improperly accepted"
            );
        }
    }
    #[test]
    #[ignore = "requires private captured original source graph"]
    fn private_actual_residents_keep_stale_caches_without_refresh() {
        let mut v = private_input();
        // Synthetic perturbations of an observed graph, never original evidence.
        // A constructor/refresh would overwrite these independently owned words.
        overwrite(
            &mut v["geometry_input"]["meshes"][0]["bindings"],
            "used_raw",
            20,
            0x12345678,
        );
        overwrite(
            &mut v["geometry_input"]["drawables"][0],
            "vertices_raw",
            0,
            321.125f32.to_bits(),
        );
        let restored = import_parts(&v).unwrap();
        assert_eq!(restored.geometry.meshes.meshes[0].stamps[0], 0x12345678);
        assert_eq!(
            restored.geometry.characters[&16].geometry.vertices[&0].vertices[0].position[0]
                .to_bits(),
            321.125f32.to_bits()
        );
        let bytes = postcard::to_allocvec(&restored).unwrap();
        let cold: Imported = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(postcard::to_allocvec(&cold).unwrap(), bytes);
        let original = private_input();
        let skins = super::super::completed_skin::import_parts(&original["skin_input"]).unwrap();
        for (id, skin) in skins {
            assert_eq!(cold.residents[&id].character.skin, skin.instance.skin);
            assert_eq!(
                cold.residents[&id].character.hierarchy,
                skin.instance.hierarchy
            );
            assert_eq!(cold.residents[&id].character.rig, skin.rig);
        }
    }
}
