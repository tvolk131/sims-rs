//! Read-only ordinary ROOT endpoint fields; no initializer/private pass claims.
use super::*;
use sims_sim::pose::{
    graphic_appearance, graphic_frame,
    model::{Bounds, Matrix, Model},
    skeleton,
};
fn bounds(raw: &[u8], offset: usize) -> Result<Bounds> {
    Ok(Bounds {
        minimum: floats(raw, offset)?,
        maximum: floats(raw, offset + 12)?,
    })
}
fn matrix(raw: &[u8], offset: usize) -> Result<Matrix> {
    Ok(Matrix {
        values: floats(raw, offset)?,
        kind: word(raw, offset + 64)?,
    })
}
fn value<T: DeserializeOwned>(v: &Value) -> Result<T> {
    Ok(serde_json::from_value(v.clone())?)
}
pub fn compare(source: &Value, saved: &Saved, rows: &mut Vec<Value>) -> Result<()> {
    if !source["ordinary"].is_object() {
        return Ok(());
    }
    let original = &source["ordinary"];
    let actual = serde_json::to_value(&saved.assets.ordinary)?;
    let actors = original["actors"]
        .as_array()
        .context("ordinary owners absent")?;
    ensure!(
        actors.len()
            == actual["graphics"]
                .as_object()
                .context("ordinary Rust owners absent")?
                .len(),
        "ordinary owner coverage differs"
    );
    let mut natives = BTreeMap::new();
    let mut vertex_bounds = BTreeMap::new();
    for a in actors {
        let r = &a["input"]["retained"];
        let skin = &r["skin"];
        let bones = skin["bones"].as_array().context("ordinary bones absent")?;
        ensure!(
            bones.len() == 1 && skin["root"] == bones[0]["id"],
            "ordinary ROOT diagnostic profile differs"
        );
        let id: i16 = value(&a["object"])?;
        let ptr: u32 = value(&skin["root"])?;
        let br = raw(&a["bones_raw"][ptr.to_string()])?;
        vertex_bounds.insert(id, vec![bounds(&br, 0x154)?]);
        natives.insert(
            id,
            completed_skeletons::NativeBinding {
                person: value(&a["native_actor"])?,
                animator: value(&r["animator"])?,
                model: value(&r["model"]["id"])?,
                skin: value(&skin["id"])?,
                bone_pointers: vec![ptr],
                bone_names: vec![value(&bones[0]["name"]["bytes"])?],
                parent_names: vec![value(&bones[0]["parent_name"]["bytes"])?],
                lookup: skin["lookup"]
                    .as_array()
                    .context("ordinary native name lookup absent")?
                    .iter()
                    .map(|p| {
                        if p.is_null() {
                            Ok(None)
                        } else {
                            ensure!(
                                p.as_u64() == Some(u64::from(ptr)),
                                "ordinary lookup aliases another bone"
                            );
                            Ok(Some(0))
                        }
                    })
                    .collect::<Result<_>>()?,
                root: 0,
            },
        );
    }
    let bindings = natives
        .iter()
        .map(|(&id, n)| {
            (
                id,
                completed_geometry::Skin {
                    native: n,
                    bone_vertex_bounds: &vertex_bounds[&id],
                },
            )
        })
        .collect();
    let geometry = completed_geometry::import_with_bindings(
        &original["geometry"],
        &bindings,
        "observed-ordinary-endpoint",
    )?;
    let mut shared = vec![];
    for g in [&source["geometry"], &original["geometry"]] {
        for m in g["meshes"]
            .as_array()
            .context("shared native mesh inventory absent")?
        {
            let p: u32 = value(&m["pointer"])?;
            if !shared.contains(&p) {
                shared.push(p);
            }
        }
    }
    for a in actors {
        let id: i16 = value(&a["object"])?;
        let g = &actual["graphics"][id.to_string()];
        let ar = raw(&a["animator_raw"])?;
        let sr = raw(&a["skin_raw"])?;
        let r = &a["input"]["retained"];
        let bone = &r["skin"]["bones"][0];
        let ptr: u32 = value(&bone["id"])?;
        let br = raw(&a["bones_raw"][ptr.to_string()])?;
        let label = |s: &str| format!("ordinary/{id}/{s}");
        let frame: graphic_frame::State = value(&g["frame"])?;
        ensure!(
            r["layers"] == json!([null, null, null]) && r["skin"]["layers"] == json!([]),
            "ordinary active layers require another source profile"
        );
        projected(
            &label("frame"),
            json!({"clock_mode":a["input"]["fields"]["clock_mode"],"now":a["input"]["fields"]["now"],"visible":a["input"]["fields"]["visible"],"blend":null,
            "placement":{"base":floats::<3>(&ar,0x80)?,"offset":floats::<3>(&ar,0x8c)?,"dirty":ar[0xd9]!=0,"explicit":ar[0x108]!=0},"matrix":matrix(&ar,0x38)?}),
            &frame,
            rows,
        )?;
        let appearance: graphic_appearance::State = value(&g["appearance"])?;
        projected(
            &label("appearance"),
            json!({"cached":int(&ar,0x9c)?,"dirty":ar[0xd9]!=0,"marker":ar[0xf8]!=0,"kind":word(&ar,0xfc)?,"ordinary_material":ar[0xf7]!=0,"tint":&ar[0xf4..0xf7]}),
            &appearance,
            rows,
        )?;
        same(
            &label("marker_value"),
            &Some(int(&ar, 0x100)?),
            &value::<Option<i32>>(&g["marker_value"])?,
            rows,
        )?;
        let observed = &g["bound"];
        let native = &geometry.characters[&id];
        same(
            &label("model"),
            &native.model,
            &value::<Model>(&observed["model"])?,
            rows,
        )?;
        same(
            &label("matrix"),
            &native.matrix,
            &value::<Matrix>(&observed["matrix"])?,
            rows,
        )?;
        same(
            &label("local_bounds"),
            &bounds(&sr, 0xbc)?,
            &value::<Bounds>(&observed["local_bounds"])?,
            rows,
        )?;
        same(
            &label("render_bounds"),
            &native.render_bounds,
            &value::<Bounds>(&observed["render_bounds"])?,
            rows,
        )?;
        let cache: skeleton::Geometry = value(&observed["skeleton"]["geometry"])?;
        projected(
            &label("geometry_stamps"),
            json!({"dirty":word(&sr,0x98)?!=0,"generation":word(&sr,0xac)?,"evaluated_generation":word(&sr,0xa8)?,"bounds_generation":word(&sr,0xb0)?,"bone_generations":[word(&br,0x150)?]}),
            &cache,
            rows,
        )?;
        let hierarchy: sims_sim::pose::refresh::Hierarchy =
            value(&observed["skeleton"]["hierarchy"])?;
        let pose = floats::<7>(&br, 0x4c)?;
        projected(
            &label("hierarchy"),
            json!({"parents":[null],"absolute":[{"translation":&pose[..3],"rotation":&pose[3..]}],"updates":[word(&br,0x130)?],"evaluated_generation":word(&sr,0xa0)?}),
            &hierarchy,
            rows,
        )?;
        let animator = &observed["skeleton"]["skin"]["animator"];
        for (field, offset) in [
            ("generation", 0xb4),
            ("pose_generation", 0xa4),
            ("last_clock", 0xb8),
        ] {
            if field == "last_clock" {
                same(
                    &label(field),
                    &int(&sr, offset)?,
                    &value::<i32>(&animator[field])?,
                    rows,
                )?;
            } else {
                same(
                    &label(field),
                    &word(&sr, offset)?,
                    &value::<u32>(&animator[field])?,
                    rows,
                )?;
            }
        }
        let local = floats::<7>(&br, 0x30)?;
        let bones: Vec<sims_sim::pose::sampling::BoneInput> =
            value(&observed["skeleton"]["skin"]["bones"])?;
        projected(
            &label("local_bones"),
            json!([{"local":{"translation":&local[..3],"rotation":&local[3..]},"priority":int(&br,0x68)?,"blend_marker":word(&br,0x74)?}]),
            &bones,
            rows,
        )?;
        same(
            &label("bone_bounds"),
            &vec![bounds(&br, 0x138)?],
            &value::<Vec<Bounds>>(&observed["bone_bounds"])?,
            rows,
        )?;
        same(
            &label("bone_bounds_stamps"),
            &vec![word(&br, 0x134)?],
            &value::<Vec<u32>>(&observed["bone_bounds_generations"])?,
            rows,
        )?;
        let mut state = native.geometry.state.clone();
        for binding in &mut state.bindings {
            let pointer = geometry.native_meshes[binding.mesh];
            binding.mesh = shared
                .iter()
                .position(|p| *p == pointer)
                .context("ordinary shared mesh alias absent")?;
        }
        same(
            &label("appearance_geometry"),
            &state,
            &value::<sims_sim::pose::geometry::State>(&g["geometry"]["state"])?,
            rows,
        )?;
    }
    Ok(())
}
