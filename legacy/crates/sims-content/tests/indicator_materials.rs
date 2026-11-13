//! Actual source material publication with explicit backend settings. Dynamic
//! marker control producers and native device construction remain separate.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::{
    Index,
    character_geometry::CharacterGeometry,
    graphic_materials::{Backend, GraphicMaterials},
    saved_avatar::BoundAvatar,
};
use sims_sim::pose::model_material::{Request, Update};

fn report() -> Result<Value> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let sources: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-indicator-initialized-frames.json"),
    )?)?;
    let mut cases = vec![];
    for source in sources.as_array().context("marker frames absent")? {
        let kind = source["state"]["kind"]
            .as_u64()
            .context("marker kind absent")?;
        let geometry: CharacterGeometry = serde_json::from_value(source["geometry"].clone())?;
        let binding = serde_json::from_value(source["asset_state"].clone())?;
        let avatar =
            BoundAvatar::restore(&index, source["skeleton"].as_str().unwrap(), kind, &binding)?;
        ensure!(avatar.snapshot() == binding);
        for mode in [0, 1, 2] {
            let mut owner = GraphicMaterials::from_builtin_geometry(
                &geometry,
                &avatar,
                [13, 17, 19],
                Backend {
                    mode,
                    flags: 0x1234567f,
                    references: 1,
                    ramp_size: 32,
                },
            )?;
            for flags in [
                [0, 0, 0],
                [1, 0, 0],
                [0, 1, 0],
                [0, 0, 1],
                [1, 1, 0],
                [1, 0, 1],
                [0, 1, 1],
                [1, 1, 1],
                [0, 0, 0],
            ] {
                let input = json!({"requested":flags,"normal_mode":mode,"publication_result":0,"mutations":[],"state":owner.state});
                let mut update = Update::new(flags);
                let mut events = vec![];
                loop {
                    let before = (owner.clone(), update.clone());
                    (owner, update) = serde_json::from_slice(&serde_json::to_vec(&before)?)?;
                    ensure!((owner.clone(), update.clone()) == before);
                    let Some(request) = update
                        .advance(&mut owner.state, mode)
                        .map_err(anyhow::Error::msg)?
                    else {
                        break;
                    };
                    let kind = match request {
                        Request::Cast { .. } => "Cast",
                        Request::Acquire { .. } => "Acquire",
                        Request::Publish { .. } => "Publish",
                        Request::Release { .. } => "Release",
                    };
                    events.push(json!({"kind":kind,"request":request,"state":owner.state,"normal_mode":mode}));
                    let pending = (owner.clone(), update.clone());
                    (owner, update) = serde_json::from_slice(&serde_json::to_vec(&pending)?)?;
                    ensure!((owner.clone(), update.clone()) == pending);
                    owner.advance(&mut update, mode)?;
                }
                ensure!(update.complete());
                for (&binding, &id) in &owner.bindings {
                    ensure!(
                        owner.published_for_binding(binding)? == &owner.state.materials[&id].words
                    );
                }
                cases.push(json!({"name":source["name"],"mode":mode,"input":input,
                    "rust":{"state":owner.state,"events":events,"normal_mode":mode},"owner":owner}));
            }
        }
    }
    ensure!(cases.len() == 81);
    Ok(
        json!({"cases":cases,"scope":"original marker materials; explicit retained CPU backend modes, flags, references and device ramp; original effect update; no live control producer or native device construction"}),
    )
}

#[test]
#[ignore = "requires private original marker assets and initialized frame report"]
fn original_marker_material_inputs_and_service_restores() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::write(
        root.join("cache/original-indicator-materials.json"),
        serde_json::to_vec(&report()?)?,
    )?;
    Ok(())
}

#[test]
#[ignore = "requires private original assets and freshly generated native material evidence"]
fn original_marker_materials_match_native_execution() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let actual = report()?;
    let expected: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-indicator-materials-native.json"),
    )?)?;
    ensure!(
        actual["cases"].as_array().unwrap().len() == expected["cases"].as_array().unwrap().len()
    );
    for (a, e) in actual["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(expected["cases"].as_array().unwrap())
    {
        ensure!(
            a["input"] == e["input"],
            "native material source inputs changed"
        );
        ensure!(
            a["rust"] == e["original"],
            "native source material mismatch: {}",
            a["name"]
        );
    }
    Ok(())
}
