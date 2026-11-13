use serde_json::{Value, json};
use sims_sim::pose::{
    geometry::{Binding, Drawable, State},
    mesh::{MeshBounds, Pose},
    model::{Bound, Bounds},
};

fn bounds(v: &Value) -> Bounds {
    let values: Vec<f32> = serde_json::from_value(v.clone()).unwrap();
    Bounds {
        minimum: values[..3].try_into().unwrap(),
        maximum: values[3..].try_into().unwrap(),
    }
}
fn compare(bound: &Bound, state: &State, meshes: &[MeshBounds], expected: &Value) {
    let model = &expected["model"];
    assert_eq!(
        bound.model,
        serde_json::from_value::<sims_sim::pose::model::Model>(model["model"].clone()).unwrap()
    );
    assert_eq!(bound.local_bounds, bounds(&model["bounds"]));
    assert_eq!(bound.render_bounds, bounds(&model["model_bounds"]));
    let skin = &bound.skeleton;
    assert_eq!(
        skin.hierarchy.absolute,
        serde_json::from_value::<Vec<sims_sim::head_look::rig::AbsoluteBone>>(
            model["propagated"]["absolute"].clone()
        )
        .unwrap()
    );
    assert_eq!(
        json!(skin.hierarchy.updates),
        model["propagated"]["updates"]
    );
    let animator = json!(skin.skin.animator);
    for (got, key) in [
        (json!(u32::from(skin.geometry.dirty)), "0x98"),
        (json!(skin.geometry.evaluated_generation), "0xa8"),
        (json!(skin.geometry.generation), "0xac"),
        (json!(skin.geometry.bounds_generation), "0xb0"),
        (animator["generation"].clone(), "0xb4"),
        (animator["pose_generation"].clone(), "0xa4"),
    ] {
        assert_eq!(got, model["skin"][key], "{key}");
    }
    for (i, &absolute) in skin.hierarchy.absolute.iter().enumerate() {
        assert_eq!(
            Pose::from_absolute(absolute, skin.hierarchy.updates[i])
                .unwrap()
                .rotation,
            serde_json::from_value::<[f32; 9]>(expected["rotations"][i].clone()).unwrap()
        );
        assert_eq!(
            json!(skin.geometry.bone_generations[i]),
            model["bones"][i]["geometry_stamp"]
        );
        assert_eq!(
            json!(bound.bone_bounds_generations[i]),
            model["bone_geometry"][i]["stamp"]
        );
        assert_eq!(
            state.bone_vertex_bounds[i],
            bounds(&expected["bone_vertex_bounds"][i])
        );
    }
    for (i, (mesh, binding)) in meshes.iter().zip(&state.bindings).enumerate() {
        let expected = &expected["meshes"][i];
        assert_eq!(json!(mesh.stamps), expected["stamps"]);
        assert_eq!(mesh.bounds, bounds(&expected["bounds"]));
        assert_eq!(binding.bounds, bounds(&expected["binding_bounds"]));
        assert_eq!(
            binding.drawable.bounds,
            bounds(&expected["drawable_bounds"])
        );
        assert_eq!(json!(binding.drawable.flags), expected["drawable_flags"]);
    }
}

fn setup(c: &Value) -> (Bound, State, Vec<MeshBounds>) {
    let source = serde_json::from_value(c["skeleton"].clone()).unwrap();
    let mut bound = Bound::new(&source, &mut (c["crt_state"].as_u64().unwrap() as u32)).unwrap();
    bound
        .skeleton
        .hierarchy
        .update(&mut bound.skeleton.skin, false)
        .unwrap();
    bound.skeleton.geometry.dirty = c["dirty"].as_bool().unwrap();
    let mut state = State::empty(bound.skeleton.hierarchy.absolute.len());
    state.appearances = serde_json::from_value(c["appearances"].clone()).unwrap();
    let mut meshes = vec![];
    for (i, m) in c["meshes"].as_array().unwrap().iter().enumerate() {
        meshes.push(MeshBounds {
            local: m["local"].as_array().unwrap().iter().map(bounds).collect(),
            stamps: serde_json::from_value(m["stamps"].clone()).unwrap(),
            bounds: bounds(&m["bounds"]),
        });
        state.bindings.push(Binding {
            mesh: i,
            bones: serde_json::from_value(m["bones"].clone()).unwrap(),
            bounds: bounds(&m["binding_bounds"]),
            drawable: Drawable {
                bounds: bounds(&m["drawable_bounds"]),
                flags: m["drawable_flags"].as_u64().unwrap() as u32,
            },
        });
    }
    (bound, state, meshes)
}

#[cfg(feature = "private-fixtures")]
#[test]
fn populated_model_geometry_matches_native_cache_and_drawable_publication() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-populated-geometry.json")).unwrap();
    compare_sequences(report, 576);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn head_geometry_preserves_native_classification_and_drawable_publication() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-head-geometry.json")).unwrap();
    compare_sequences(report, 480);
}

fn compare_sequences(report: Value, expected_steps: usize) {
    let mut steps = 0;
    for row in report["sequences"].as_array().unwrap() {
        let c = &row["input"];
        let (mut bound, mut state, mut meshes) = setup(c);
        compare(&bound, &state, &meshes, &row["initial"]);
        for step in row["steps"].as_array().unwrap() {
            let restored: (Bound, State, Vec<MeshBounds>) =
                serde_json::from_slice(&serde_json::to_vec(&(&bound, &state, &meshes)).unwrap())
                    .unwrap();
            assert_eq!(
                (&bound, &state, &meshes),
                (&restored.0, &restored.1, &restored.2)
            );
            (bound, state, meshes) = restored;
            let a = &step["action"];
            match a["op"].as_str().unwrap() {
                "refresh" => state
                    .refresh(&mut bound, &mut meshes, a["force"].as_bool().unwrap())
                    .unwrap(),
                "head" => state
                    .refresh_head(&mut bound, &mut meshes, a["force"].as_bool().unwrap())
                    .unwrap(),
                "hierarchy" => bound
                    .skeleton
                    .hierarchy
                    .update(&mut bound.skeleton.skin, a["force"].as_bool().unwrap())
                    .unwrap(),
                "dirty" => bound.skeleton.geometry.dirty = true,
                "clear" => state.appearances.clear(),
                _ => unreachable!(),
            }
            compare(&bound, &state, &meshes, &step["original"]);
            steps += 1;
        }
    }
    assert_eq!(steps, expected_steps);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn appearance_attachment_invalidates_bones_and_skin_in_native_order() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-geometry-attachment.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let (mut bound, mut state, mut meshes) = setup(&row["input"]);
        state.appearances.clear();
        compare(&bound, &state, &meshes, &row["initial"]);
        let bindings = std::mem::take(&mut state.bindings)
            .into_iter()
            .map(|mut b| {
                b.bounds = Bounds::EMPTY;
                b.drawable = Drawable {
                    bounds: Bounds {
                        minimum: [-1., -2., -3.],
                        maximum: [4., 5., 6.],
                    },
                    flags: 6,
                };
                b
            })
            .collect();
        assert_eq!(state.attach(&mut bound, bindings, &meshes).unwrap(), 0);
        compare(&bound, &state, &meshes, &row["attached"]);
        state.refresh(&mut bound, &mut meshes, false).unwrap();
        compare(&bound, &state, &meshes, &row["refreshed"]);
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 25);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn appearance_removal_preserves_native_invalidation_and_remaining_geometry() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-geometry-destruction.json")).unwrap();
    let mut steps = 0;
    for row in report["sequences"].as_array().unwrap() {
        let (mut bound, mut state, mut meshes) = setup(&row["input"]);
        let mut identities: Vec<_> = (0..state.appearances.len()).collect();
        compare(&bound, &state, &meshes, &row["initial"]);
        for step in row["steps"].as_array().unwrap() {
            (bound, state, meshes) =
                serde_json::from_slice(&serde_json::to_vec(&(&bound, &state, &meshes)).unwrap())
                    .unwrap();
            let action = &step["action"];
            if action["op"] == "remove" {
                let id = action["id"].as_u64().unwrap() as usize;
                if let Some(index) = identities.iter().position(|&x| x == id) {
                    let removed = state.appearances[index].clone();
                    let expected_order: Vec<_> = removed
                        .iter()
                        .rev()
                        .flat_map(|&binding| {
                            std::iter::once(json!(["binding", binding])).chain(
                                state.bindings[binding]
                                    .bones
                                    .iter()
                                    .rev()
                                    .map(|&bone| json!(["bone", bone])),
                            )
                        })
                        .collect();
                    let actual_order: Vec<_> = step["events"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|e| e["op"] == "binding" || e["op"] == "bone")
                        .map(|e| {
                            json!([
                                e["op"],
                                e[if e["op"] == "binding" {
                                    "binding"
                                } else {
                                    "bone"
                                }]
                            ])
                        })
                        .collect();
                    assert_eq!(expected_order, actual_order);
                    state.detach(&mut bound, index).unwrap();
                    identities.remove(index);
                } else {
                    assert!(step["events"].as_array().unwrap().is_empty());
                }
            } else {
                state
                    .refresh(&mut bound, &mut meshes, action["force"].as_bool().unwrap())
                    .unwrap();
            }
            assert_eq!(json!(identities), step["active"]);
            assert_eq!(json!(identities.len() + 1), step["prototype_references"]);
            compare(&bound, &state, &meshes, &step["original"]);
            steps += 1;
        }
    }
    assert_eq!(steps, 867);
}

#[test]
#[ignore = "requires private original resident geometry evidence"]
fn original_resident_initializer_geometry_matches_native() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/native-original-populated-geometry.json")).unwrap(),
    )
    .unwrap();
    for row in report["cases"].as_array().unwrap() {
        let mut bound: Bound = serde_json::from_value(row["input"]["bound"].clone()).unwrap();
        let mut state: State = serde_json::from_value(row["input"]["state"].clone()).unwrap();
        let mut meshes: Vec<MeshBounds> =
            serde_json::from_value(row["input"]["pool"]["meshes"].clone()).unwrap();
        state
            .refresh(
                &mut bound,
                &mut meshes,
                row["input"]["force"].as_bool().unwrap(),
            )
            .unwrap();
        assert_eq!(
            bound,
            serde_json::from_value::<Bound>(row["rust"]["bound"].clone()).unwrap()
        );
        assert_eq!(
            state,
            serde_json::from_value::<State>(row["rust"]["state"].clone()).unwrap()
        );
        assert_eq!(
            meshes,
            serde_json::from_value::<Vec<MeshBounds>>(row["rust"]["pool"]["meshes"].clone())
                .unwrap()
        );
        compare(&bound, &state, &meshes, &row["original"]);
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 14);
}
