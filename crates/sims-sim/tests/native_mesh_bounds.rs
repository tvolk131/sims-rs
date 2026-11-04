use serde_json::{Value, json};
use sims_sim::pose::{
    mesh::{LocalBounds, MeshBounds, Pose},
    model::Bounds,
};
fn bounds(value: &Value) -> Bounds {
    let b: Vec<f32> = serde_json::from_value(value.clone()).unwrap();
    Bounds {
        minimum: b[..3].try_into().unwrap(),
        maximum: b[3..].try_into().unwrap(),
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn fresh_local_boxes_match_original_mesh_reader_construction() {
    use sims_formats::avatar::{Blend, Mesh, MeshBinding, Vertex};
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-mesh-local-bounds.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let input = &row["input"];
        let samples: Vec<[f32; 3]> = serde_json::from_value(input["samples"].clone()).unwrap();
        let normals: Vec<[f32; 3]> = serde_json::from_value(input["normals"].clone()).unwrap();
        let primary_count = input["primary_count"].as_u64().unwrap() as usize;
        let bindings: Vec<[usize; 5]> = serde_json::from_value(input["bindings"].clone()).unwrap();
        let mesh = Mesh {
            name: "native local-box fixture".into(),
            texture: String::new(),
            bones: (0..bindings.len()).map(|i| format!("bone{i}")).collect(),
            bindings: bindings
                .iter()
                .map(|b| MeshBinding {
                    bone: b[0],
                    first_vertex: b[1],
                    vertex_count: b[2],
                    first_blend: b[3],
                    blend_count: b[4],
                })
                .collect(),
            triangles: vec![],
            vertices: samples[..primary_count]
                .iter()
                .zip(&normals)
                .map(|(&position, &normal)| Vertex {
                    position,
                    normal,
                    uv: [0.; 2],
                    bone: 0,
                })
                .collect(),
            blends: samples[primary_count..]
                .iter()
                .map(|&position| Blend {
                    vertex: 0,
                    weight: 0.5,
                    position,
                    normal: [0., 0., 1.],
                    bone: 0,
                })
                .collect(),
        };
        let got = LocalBounds::from_mesh(&mesh).unwrap();
        assert_eq!(
            got.local,
            row["original"]["local"]
                .as_array()
                .unwrap()
                .iter()
                .map(bounds)
                .collect::<Vec<_>>(),
            "{input}"
        );
        assert_eq!(got.bounds, bounds(&row["original"]["bounds"]), "{input}");
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 33);
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_mesh_bounds_match_native_cache_gate_and_batch_transform() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-mesh-bounds.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let bones = c["bones"].as_array().unwrap();
        let poses: Vec<Pose> = bones
            .iter()
            .map(|b| Pose {
                position: serde_json::from_value(b["position"].clone()).unwrap(),
                rotation: serde_json::from_value(b["rotation"].clone()).unwrap(),
                stamp: b["stamp"].as_u64().unwrap() as u32,
            })
            .collect();
        let mut mesh = MeshBounds {
            local: bones.iter().map(|b| bounds(&b["bounds"])).collect(),
            stamps: bones
                .iter()
                .map(|b| b["cached"].as_u64().unwrap() as u32)
                .collect(),
            bounds: bounds(&c["old_bounds"]),
        };
        let restored =
            serde_json::from_slice::<MeshBounds>(&serde_json::to_vec(&mesh).unwrap()).unwrap();
        assert_eq!(mesh, restored);
        let changed = mesh.update(&poses, c["rest"].as_bool().unwrap()).unwrap();
        assert_eq!(json!(changed), row["original"]["changed"], "{c}");
        assert_eq!(mesh.bounds, bounds(&row["original"]["bounds"]), "{c}");
        assert_eq!(json!(mesh.stamps), row["original"]["stamps"], "{c}");
        let before = mesh.clone();
        assert!(!mesh.update(&poses, c["rest"].as_bool().unwrap()).unwrap());
        assert_eq!(mesh, before);
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 64);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn retained_goth_mesh_matrices_use_actual_process_precision() {
    let v: Value = serde_json::from_str(include_str!(
        "fixtures/retained-goth-mesh-matrices-003f.json"
    ))
    .unwrap();
    assert_eq!(v["fp_control"], 0x003f);
    let rows = v["rows"].as_array().unwrap();
    assert!(!rows.is_empty());
    for (index, row) in rows.iter().enumerate() {
        let bits: [u32; 7] = serde_json::from_value(row["pose_bits"].clone()).unwrap();
        let a = bits.map(f32::from_bits);
        let bone = sims_sim::head_look::rig::AbsoluteBone {
            translation: a[..3].try_into().unwrap(),
            rotation: a[3..].try_into().unwrap(),
        };
        let p =
            Pose::from_absolute_with_control(bone, row["stamp"].as_u64().unwrap() as u32, 0x003f)
                .unwrap();
        let expected: [u32; 9] = serde_json::from_value(row["rotation_bits"].clone()).unwrap();
        assert_eq!(p.rotation.map(f32::to_bits), expected, "native row {index}");
    }
}
