//! Read-only renderer audit of a managed household checkpoint.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{Camera, original_house::OriginalHouse, software};
use sims_runtime::original_session::Session;
use std::path::Path;
fn main() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("geometry audit panicked"))?
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 3 || args.len() == 4,
        "geometry_audit ROOT MANAGED_SAVE NEW_OUTPUT_DIR [REFERENCE_JSON]"
    );
    let output = Path::new(&args[2]);
    ensure!(
        output
            .parent()
            .context("output parent")?
            .canonicalize()?
            .starts_with(Path::new("cache").canonicalize()?),
        "private output required"
    );
    std::fs::create_dir(output)?;
    let value: Value = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let index = Index::scan(Path::new(&args[0]))?;
    let session = Session::restore(&index, serde_json::from_value(value["session"].clone())?)?;
    let before = serde_json::to_vec(&session.save())?;
    let reference = args
        .get(3)
        .map(|path| -> Result<Value> { Ok(serde_json::from_slice(&std::fs::read(path)?)?) })
        .transpose()?;
    let input = if let Some(reference) = &reference {
        serde_json::from_value(reference["camera"].clone())?
    } else {
        serde_json::from_value(value["camera_device"]["published_frame"]["input"].clone())?
    };
    let floor = value["floor"].as_i64().context("floor absent")? as i32;
    let camera = Camera::from_native(input, floor)?;
    let mut renderer = OriginalHouse::default();
    let mut compared = 0;
    let scene = if let Some(reference) = &reference {
        let presenter = session
            .host
            .presentation
            .presenter
            .as_ref()
            .context("presenter")?;
        let mut graph = presenter
            .camera
            .as_ref()
            .context("camera graph")?
            .graph
            .clone();
        graph
            .rotate(u8::try_from(input.rotation)?)
            .map_err(anyhow::Error::msg)?;
        let mut view = presenter.view.floor.clone();
        view.level = reference["level"].as_i64().context("reference floor")? as i32;
        ensure!(floor == view.level, "reference and managed floor differ");
        view.all_walls = reference["all_walls"].as_u64().context("wall mode")? as u8;
        view.visibility.enabled = reference["cutaway"].as_u64().context("cutaway mode")? as u8;
        view.visibility.dirty = 1;
        view.visibility.mask.rows = serde_json::from_value(reference["cutaway_mask"].clone())?;
        let storage = reference["view_storage"]
            .as_str()
            .context("reference view bytes")?;
        let roof = u8::from_str_radix(&storage[0x51 * 2..0x52 * 2], 16)?;
        let plan = sims_render::wall_view::Plan::prepare(&graph, &view, roof, floor)?;
        let directions = [
            [0, -1],
            [0, 1],
            [-1, 0],
            [1, 0],
            [-1, -1],
            [1, 1],
            [1, -1],
            [-1, 1],
        ];
        // Captured reference is the same unrotated house geometry. This is a
        // presentation-input experiment, not a post-load simulation alignment.
        ensure!(
            input.rotation == 0,
            "reference comparison currently requires rotation zero"
        );
        for f in reference["floors"].as_array().context("reference floors")? {
            for e in f["edges"].as_array().context("reference edges")? {
                let d = directions[e["kind"].as_u64().context("edge kind")? as usize];
                let mut a: [i32; 3] = serde_json::from_value(e["ends"][0].clone())?;
                for s in e["segments"].as_array().context("segments")? {
                    let b = [a[0] + d[0], a[1] + d[1], a[2]];
                    let actual = plan.segment(a[2], [a[0], a[1]], [b[0], b[1]])?;
                    ensure!(
                        u64::from(actual.style) == s["style"].as_u64().unwrap()
                            && u64::from(actual.flags) == s["flags"].as_u64().unwrap(),
                        "native wall mismatch at {a:?} -> {b:?}: {actual:?} vs {s}"
                    );
                    compared += 1;
                    a = b;
                }
            }
        }
        renderer.scene_with_wall_plan(&session, camera, Some(&plan))?
    } else {
        renderer.scene(&session, camera)?
    };
    software::render(&scene).save(output.join("software.png"))?;
    ensure!(
        before == serde_json::to_vec(&session.save())?,
        "renderer mutated retained session"
    );
    let mut range = [f32::INFINITY, f32::NEG_INFINITY];
    let mut outside = [0usize; 2];
    for v in &scene.vertices {
        range[0] = range[0].min(v.position[2]);
        range[1] = range[1].max(v.position[2] + v.depth_scale);
        outside[0] += usize::from(v.position[2] < 0.);
        outside[1] += usize::from(v.position[2] > 1.);
    }
    let report = json!({"scope":"Read-only presentation audit, no simulation tick/input replay", "source":session.house.source,
        "tick":session.simulation.vm.tick,"camera":input,"floor":floor,"vertices":scene.vertices.len(),"draws":scene.draws.len(),
        "depth_range":range,"vertices_outside_near_far":outside,"retained_state_unchanged":true,
        "native_wall_segments_compared":compared});
    std::fs::write(
        output.join("audit.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    // Vertex-only evidence can be analyzed without retaining/re-exporting original textures.
    std::fs::write(
        output.join("vertices.json"),
        serde_json::to_vec(
            &scene
                .vertices
                .iter()
                .map(|v| (v.position, v.depth_scale))
                .collect::<Vec<_>>(),
        )?,
    )?;
    println!("{report}");
    Ok(())
}
