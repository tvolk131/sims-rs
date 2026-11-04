//! Original mesh geometry through actual base visibility and native household
//! camera math. Clean/dirty status and camera scroll are explicit fixture inputs.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sims_content::{
    Index,
    character_geometry::CharacterGeometry,
    saved_avatar::{BindingState, BoundAvatar},
};
use sims_sim::pose::{
    graphic_frame::State,
    graphic_transform as placement, graphic_visibility as visibility,
    model::{Bound, Matrix},
    model_visibility,
    portrait::view,
    transform,
};

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// Compare the actual native parameter widths. JSON's f64 representation of
// an f32 can change its last decimal bit during parsing without changing the
// bytes passed to either implementation.
#[derive(Debug, Deserialize, PartialEq)]
struct NativeInput {
    state: NativeState,
    enabled: bool,
    camera: Matrix,
    viewport: [i32; 4],
    mutation: std::collections::BTreeMap<String, Value>,
    geometry: NativeGeometry,
}
#[derive(Debug, Deserialize, PartialEq)]
struct NativeState {
    model: Option<u64>,
    dirty: u8,
    visible: u8,
    matrix: Matrix,
    base: [f32; 3],
    offset: [f32; 3],
}
#[derive(Debug, Deserialize, PartialEq)]
struct NativeGeometry {
    bounds: [f32; 6],
    scale: Matrix,
}

fn source_report() -> Result<Value> {
    let root = root();
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-indicator-initialized-frames.json"),
    )?)?;
    let mut cases = vec![];
    let mut sources = vec![];
    for row in rows {
        let name = row["name"].as_str().context("marker name absent")?;
        let owner = row["state"]["kind"]
            .as_u64()
            .context("marker owner absent")?;
        let bindings: BindingState = serde_json::from_value(row["asset_state"].clone())?;
        let avatar =
            BoundAvatar::restore(&index, row["skeleton"].as_str().unwrap(), owner, &bindings)?;
        let mut bound: Bound = serde_json::from_value(row["bound"].clone())?;
        let geometry: CharacterGeometry = serde_json::from_value(row["geometry"].clone())?;
        ensure!(!geometry.vertices.is_empty() && avatar.snapshot() == bindings);
        let mut state: State = serde_json::from_value(row["frame"].clone())?;
        state.placement = placement::Placement::constructed();
        let mut update = placement::Update::default();
        let request = update
            .advance(
                &mut state.placement,
                state.matrix,
                placement::Source {
                    position: [320, 320],
                    facing: 0,
                    level: 1,
                },
                transform::View {
                    rotation: 0,
                    floor_vector: [0.; 3],
                },
                transform::Terrain {
                    size: 2,
                    corners: &[[0; 4]; 4],
                },
            )
            .map_err(anyhow::Error::msg)?
            .context("placement absent")?;
        let placement::Request::Publish(matrix) = request else {
            anyhow::bail!("ordinary placement expected")
        };
        state.matrix = matrix;
        bound.publish(matrix).map_err(anyhow::Error::msg)?;
        update
            .acknowledge(&mut state.placement, &request)
            .map_err(anyhow::Error::msg)?;
        state.placement.dirty = false;
        state.visible = 71;
        let before_bound = bound.clone();
        for zoom in 1..=3 {
            for rotation in 0..4 {
                // Choose a diagnostic scroll that centers this source mesh, then
                // move the same real camera far away in both directions.
                let camera0 = view::Input {
                    viewport: [0, 0, 640, 480],
                    center: [0, 0],
                    zoom,
                    rotation,
                }
                .build()
                .map_err(anyhow::Error::msg)?
                .screen;
                let midpoint = std::array::from_fn(|i| {
                    (bound.local_bounds.minimum[i] + bound.local_bounds.maximum[i]) * 0.5
                });
                let center_pixel = bound
                    .model
                    .scale
                    .multiply(state.matrix)
                    .and_then(|m| m.multiply(camera0))
                    .and_then(|m| m.point(midpoint))
                    .map_err(anyhow::Error::msg)?;
                let center = [
                    (320. - center_pixel[0]).round() as i32,
                    (240. - center_pixel[1]).round() as i32,
                ];
                for center in [center, center.map(|v| v + 1000), center.map(|v| v - 3000)] {
                    let camera_input = view::Input {
                        viewport: [0, 0, 640, 480],
                        center,
                        zoom,
                        rotation,
                    };
                    let camera = camera_input.build().map_err(anyhow::Error::msg)?.screen;
                    let viewport = [0, 0, 640, 480];
                    let mut direct = state.clone();
                    let visible =
                        visibility::evaluate(&mut direct, 0, true, Some(&bound), viewport, || {
                            Ok(camera)
                        })
                        .map_err(anyhow::Error::msg)?;
                    let mut resumed = state.clone();
                    let mut pass = visibility::Update::new(viewport);
                    let mut projection = None;
                    loop {
                        (pass, resumed) =
                            serde_json::from_slice(&serde_json::to_vec(&(pass, resumed))?)?;
                        let Some(request) = pass
                            .advance(&mut resumed, 0, true, Some(1))
                            .map_err(anyhow::Error::msg)?
                        else {
                            break;
                        };
                        let reply = match &request {
                            visibility::Request::Camera => visibility::Reply::Camera(camera),
                            visibility::Request::Project {
                                matrix,
                                camera,
                                viewport,
                                ..
                            } => {
                                let result = model_visibility::project(
                                    bound.local_bounds,
                                    bound.model.scale,
                                    *matrix,
                                    *camera,
                                    *viewport,
                                )
                                .map_err(anyhow::Error::msg)?;
                                let visible = result.visible;
                                projection = Some(result);
                                visibility::Reply::Visible(u8::from(visible))
                            }
                        };
                        (pass, resumed) =
                            serde_json::from_slice(&serde_json::to_vec(&(pass, resumed))?)?;
                        pass.acknowledge(&mut resumed, &request, reply)
                            .map_err(anyhow::Error::msg)?;
                    }
                    ensure!(direct == resumed && bound == before_bound && pass.complete());
                    cases.push(json!({"name":name,"camera_input":camera_input,
                "input":{"state":{"model":1,"dirty":0,"visible":state.visible,"matrix":state.matrix,"base":state.placement.base,"offset":state.placement.offset},"enabled":true,"camera":camera,"viewport":viewport,"mutation":{},
                    "geometry":{"bounds":bound.local_bounds.minimum.into_iter().chain(bound.local_bounds.maximum).collect::<Vec<_>>(),"scale":bound.model.scale}},
                "rust":{"visible":visible,"matrix":resumed.matrix,"projection":projection}}));
                }
            }
        }
        sources.push(json!({"name":name,"owner":owner,"skeleton":row["skeleton"],"asset_state":bindings,"geometry":geometry,"bound":bound,"frame":state}));
    }
    let visible = cases.iter().filter(|c| c["rust"]["visible"] == 1).count();
    ensure!(
        visible > 0 && visible < cases.len(),
        "source camera cases did not exercise both culling results"
    );
    println!(
        "{} source marker visibility cases: {visible} visible",
        cases.len()
    );
    Ok(json!({"sources":sources,"cases":cases}))
}

#[test]
#[ignore = "requires private initialized original marker report and assets"]
fn original_marker_visibility_inputs_and_service_restores() -> Result<()> {
    std::fs::write(
        root().join("cache/original-indicator-visibility.json"),
        serde_json::to_vec(&source_report()?)?,
    )?;
    Ok(())
}

#[test]
#[ignore = "requires private source marker visibility reports and native comparison"]
fn original_marker_visibility_matches_composed_native_execution() -> Result<()> {
    let root = root();
    // Recompute with the current implementation; never compare two stale
    // cache reports or race the companion test's output file.
    let rust = source_report()?;
    let native: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-indicator-visibility-native.json"),
    )?)?;
    let rows = rust["cases"].as_array().context("source cases absent")?;
    let original = native["cases"].as_array().context("native cases absent")?;
    ensure!(rows.len() == 108 && rows.len() == original.len());
    for (r, n) in rows.iter().zip(original) {
        ensure!(
            serde_json::from_value::<NativeInput>(r["input"].clone())?
                == serde_json::from_value::<NativeInput>(n["input"].clone())?,
            "native source visibility input changed"
        );
        ensure!(
            r["rust"]["visible"] == n["original"]["result"],
            "native source visibility differs: {}",
            r["name"]
        );
        ensure!(
            serde_json::from_value::<Matrix>(r["rust"]["matrix"].clone())?
                == serde_json::from_value::<Matrix>(n["original"]["state"]["matrix"].clone())?
        );
    }
    Ok(())
}
