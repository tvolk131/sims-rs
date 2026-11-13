use serde_json::Value;
use sims_formats::avatar::{Blend, Mesh, MeshBinding, Vertex};
use sims_sim::pose::{
    mesh::Pose,
    model::Bounds,
    vertices::{Buffer, Vertex as Published},
};

#[cfg(feature = "private-fixtures")]
#[test]
fn loaded_mesh_vertices_match_native_transforms_blends_and_drawable_writes() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-mesh-vertices.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 50);
    check(&report);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn loaded_mesh_vertices_preserve_original_precision_controls() {
    let report: Value = serde_json::from_str(include_str!("fixtures/native-mesh-vertices-precision.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 100);
    check(&report);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn loaded_mesh_vertex_normalization_retains_x87_exponent_range() {
    let report: Value = serde_json::from_str(include_str!("fixtures/native-mesh-vertices-precision-range.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 4);
    check(&report);
}

fn check(report: &Value) {
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let samples: Vec<[f32; 6]> = serde_json::from_value(c["samples"].clone()).unwrap();
        let blends: Vec<[i32; 2]> = serde_json::from_value(c["blends"].clone()).unwrap();
        let rows: Vec<[usize; 6]> = serde_json::from_value(c["bindings"].clone()).unwrap();
        let primary = samples.len() - blends.len();
        let mesh = Mesh {
            name: "native vertex fixture".into(),
            texture: String::new(),
            triangles: vec![],
            bones: (0..rows.len()).map(|i| format!("bone{i}")).collect(),
            bindings: rows
                .iter()
                .map(|r| MeshBinding {
                    bone: r[0],
                    first_vertex: r[1],
                    vertex_count: r[2],
                    first_blend: r[3],
                    blend_count: r[4],
                })
                .collect(),
            vertices: samples[..primary]
                .iter()
                .map(|v| Vertex {
                    position: v[..3].try_into().unwrap(),
                    normal: v[3..].try_into().unwrap(),
                    uv: [0.125, 0.875],
                    bone: 0,
                })
                .collect(),
            blends: blends
                .iter()
                .zip(&samples[primary..])
                .map(|(b, v)| Blend {
                    vertex: b[1] as usize,
                    weight: b[0] as f32 / 32768.,
                    position: v[..3].try_into().unwrap(),
                    normal: v[3..].try_into().unwrap(),
                    bone: 0,
                })
                .collect(),
        };
        let poses: Vec<_> = c["bones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                if b.is_null() {
                    None
                } else {
                    Some(Pose {
                        position: serde_json::from_value(b["position"].clone()).unwrap(),
                        rotation: serde_json::from_value(b["rotation"].clone()).unwrap(),
                        stamp: 0,
                    })
                }
            })
            .collect();
        let mut buffer = Buffer {
            vertices: vec![
                Published {
                    position: [91., 92., 93.],
                    normal: [94., 95., 96.],
                    uv: [0.125, 0.875]
                };
                samples.len()
            ],
            flags: c["flags"].as_u64().unwrap() as u32,
            buffer_flags: c["buffer_flags"].as_u64().unwrap() as u32,
            range: [0, 0],
            bounds: Bounds::EMPTY,
        };
        let notification = buffer
            .update_with_control(
                &mesh,
                &poses,
                c["rest"].as_bool().unwrap(),
                c["cached_drawable"].as_bool().unwrap(),
                Bounds {
                    minimum: [-4., -5., -6.],
                    maximum: [7., 8., 9.],
                },
                c["control"].as_u64().unwrap_or(0x037f) as u16,
            )
            .unwrap();
        let expected: Vec<[f32; 8]> =
            serde_json::from_value(row["original"]["vertices"].clone()).unwrap();
        for (vertex, (got, want)) in buffer.vertices.iter().zip(expected).enumerate() {
            let got: Vec<_> = got
                .position
                .iter()
                .chain(&got.normal)
                .chain(&got.uv)
                .copied()
                .collect();
            assert_eq!(got.into_iter().map(f32::to_bits).collect::<Vec<_>>(), want.map(f32::to_bits), "case {case}, vertex {vertex}");
        }
        let original = &row["original"];
        assert_eq!(
            buffer.flags,
            original["flags"].as_u64().unwrap() as u32,
            "case {case}"
        );
        assert_eq!(
            buffer.buffer_flags,
            original["buffer_flags"].as_u64().unwrap() as u32,
            "case {case}"
        );
        let range: [u32; 2] = serde_json::from_value(original["range"].clone()).unwrap();
        assert_eq!(buffer.range.map(|v| v as u32), range, "case {case}");
        let bounds: [f32; 6] = serde_json::from_value(original["bounds"].clone()).unwrap();
        assert_eq!(buffer.bounds.minimum, bounds[..3]);
        assert_eq!(buffer.bounds.maximum, bounds[3..]);
        assert_eq!(
            notification,
            !original["events"].as_array().unwrap().is_empty()
        );
        assert_eq!(original["returned"], false);
        let stamps: Vec<usize> = serde_json::from_value(original["stamps"].clone()).unwrap();
        assert_eq!(stamps, rows.iter().map(|r| r[5]).collect::<Vec<_>>());
        let restored: Buffer =
            serde_json::from_slice(&serde_json::to_vec(&buffer).unwrap()).unwrap();
        assert_eq!(restored, buffer);
    }
}
