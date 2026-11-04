//! Read-only whole shared-pool identity and ordinary instance ownership checks.
use super::*;
use sims_content::graphic_retirement::observed::frame;
use sims_content::{
    Index,
    character_geometry::{CharacterGeometry, MeshPool},
    saved_avatar::{BindingState, BoundAvatar},
};
use sims_formats::avatar::Mesh;
use sims_sim::pose::{
    appearance::{Binding, Record, Records},
    mesh::MeshBounds,
    model::{Bound, Bounds},
};

fn bounds(b: &[u8], o: usize) -> Result<Bounds> {
    Ok(Bounds {
        minimum: floats(b, o)?,
        maximum: floats(b, o + 12)?,
    })
}
fn signature(m: &Mesh) -> Value {
    json!({"name":m.name,"texture":m.texture,"bones":m.bones,"bindings":m.bindings,"triangles":m.triangles,
        "vertices":m.vertices.iter().map(|v|(v.position.map(f32::to_bits),v.normal.map(f32::to_bits),v.uv.map(f32::to_bits),v.bone)).collect::<Vec<_>>(),
        "blends":m.blends.iter().map(|v|(v.vertex,v.weight.to_bits(),v.position.map(f32::to_bits),v.normal.map(f32::to_bits),v.bone)).collect::<Vec<_>>()})
}
pub(super) fn ordinary(source: &Value) -> Result<completed_geometry::Imported> {
    let mut native = BTreeMap::new();
    let mut vertex = BTreeMap::new();
    for a in source["ordinary"]["actors"]
        .as_array()
        .context("native ordinary actors absent")?
    {
        let id: i16 = serde_json::from_value(a["object"].clone())?;
        let input: frame::Input = serde_json::from_value(a["input"].clone())?;
        let r = &input.retained;
        ensure!(
            r.skin.bones.len() == 1
                && r.skin.root == r.skin.bones[0].id
                && r.skin.layers.is_empty()
                && r.layers == [None; 3],
            "ordinary ROOT active-layer profile differs"
        );
        let bone = &r.skin.bones[0];
        let br = raw(&a["bones_raw"][bone.id.to_string()])?;
        vertex.insert(id, vec![bounds(&br, 0x154)?]);
        native.insert(
            id,
            completed_skeletons::NativeBinding {
                person: r.actor.try_into()?,
                animator: r.animator.try_into()?,
                model: r.model.id.try_into()?,
                skin: r.skin.id.try_into()?,
                bone_pointers: vec![bone.id.try_into()?],
                bone_names: vec![bone.name.bytes.clone()],
                parent_names: vec![bone.parent_name.bytes.clone()],
                lookup: r
                    .skin
                    .lookup
                    .iter()
                    .map(|p| {
                        ensure!(
                            p.is_none() || *p == Some(bone.id),
                            "ordinary ROOT lookup alias differs"
                        );
                        Ok(p.map(|_| 0))
                    })
                    .collect::<Result<_>>()?,
                root: 0,
            },
        );
    }
    let skins = native
        .iter()
        .map(|(&id, n)| {
            (
                id,
                completed_geometry::Skin {
                    native: n,
                    bone_vertex_bounds: &vertex[&id],
                },
            )
        })
        .collect();
    completed_geometry::import_with_bindings(
        &source["ordinary"]["geometry"],
        &skins,
        "new-unpublished-ordinary-live-storage",
    )
}
pub(super) fn ordinary_binding(
    a: &Value,
    g: &completed_geometry::Character,
    id: i16,
) -> Result<BindingState> {
    let input: frame::Input = serde_json::from_value(a["input"].clone())?;
    let r = &input.retained;
    ensure!(
        r.generic.len() == 1 && r.auxiliary.is_empty(),
        "ordinary record/auxiliary profile differs"
    );
    let record = &r.generic[0];
    let binding = record
        .binding
        .map(|p| -> Result<Binding> {
            Ok(Binding {
                id: *g
                    .native_appearances
                    .get(&u32::try_from(p)?)
                    .context("ordinary record/geometry alias absent")?,
                owner: id as u64,
            })
        })
        .transpose()?;
    ensure!(
        r.skin.appearances.len() == usize::from(binding.is_some())
            && g.geometry.state.appearances.len() == usize::from(binding.is_some()),
        "ordinary unclaimed appearance"
    );
    for t in [&record.name.bytes, &record.texture.bytes] {
        ensure!(
            t.is_ascii() && !t.contains(&0),
            "ordinary narrow record is invalid"
        );
    }
    let order: Vec<_> = binding.iter().map(|b| b.id).collect();
    Ok(BindingState {
        records: Records {
            entries: vec![Record {
                binding,
                name: String::from_utf8(record.name.bytes.clone())?,
                texture: String::from_utf8(record.texture.bytes.clone())?,
            }],
            generic: 0,
            head: 0,
            hands: [0; 2],
            body: [0; 2],
            accessories: vec![],
        },
        next_binding: order.iter().max().copied().unwrap_or(0) + 1,
        order,
        auxiliary: BTreeMap::new(),
    })
}
pub(super) fn bound(a: &Value, g: &completed_geometry::Character, fp: &Value) -> Result<Bound> {
    let input: frame::Input = serde_json::from_value(a["input"].clone())?;
    let r = &input.retained;
    let ar = raw(&a["animator_raw"])?;
    let sr = raw(&a["skin_raw"])?;
    let bone = &r.skin.bones[0];
    let br = raw(&a["bones_raw"][bone.id.to_string()])?;
    ensure!(
        word(&ar, 0x10)? == 0
            && word(&ar, 0x24)? == 0
            && word(&sr, 0x94)? <= 1
            && word(&sr, 0x98)? <= 1,
        "ordinary events/anchors/dirty flags need separate source ownership"
    );
    let local = floats::<7>(&br, 0x30)?;
    let absolute = floats::<7>(&br, 0x4c)?;
    let value = json!({"skeleton":{"skin":{"animator":{"fp_control":fp,"pending_vertex_updates":{},"layers":[],"slots":[null,null,null],"blend":null,"next_id":1,
        "dirty":word(&sr,0x94)?!=0,"generation":word(&sr,0xb4)?,"pose_generation":word(&sr,0xa4)?,"last_clock":int(&sr,0xb8)?,"promote_next_foreground":false,
        "selection_cache":int(&ar,0x98)?,"anchor_valid":false,"anchor_bone":null,"anchor_point":floats::<3>(&ar,0x18)?,"numbered_events":[],"pending_bone_updates":{},"pass":null},
        "bones":[{"local":{"translation":&local[..3],"rotation":&local[3..]},"priority":int(&br,0x68)?,"blend_marker":word(&br,0x74)?}],"handler":null},
        "hierarchy":{"parents":[null],"absolute":[{"translation":&absolute[..3],"rotation":&absolute[3..]}],"updates":[word(&br,0x130)?],"evaluated_generation":word(&sr,0xa0)?},
        "geometry":{"dirty":word(&sr,0x98)?!=0,"generation":word(&sr,0xac)?,"evaluated_generation":word(&sr,0xa8)?,"bounds_generation":word(&sr,0xb0)?,"bone_generations":[word(&br,0x150)?]},
        "wiggle":[{"gradient_scale":int(&br,0xe8)?,"amplitude":floats::<1>(&br,0x7c)?[0],"records":bone.wiggle_records.map(|r|r.map(|w|w as i32))}],"flags":[bone.flags],"named_bone_flags":[bone.named!=0]},
        "model":g.model,"matrix":g.matrix,"local_bounds":g.local_bounds,"render_bounds":g.render_bounds,"bone_bounds":[bounds(&br,0x138)?],"bone_bounds_generations":[word(&br,0x134)?]});
    let typed: Bound = serde_json::from_value(value.clone())?;
    ensure!(
        serde_json::to_value(&typed)? == value,
        "ordinary bound projection defaulted/discarded native fields"
    );
    Ok(typed)
}
struct Shared {
    pointers: BTreeMap<u32, usize>,
    meshes: Vec<MeshBounds>,
    bmf: Vec<Vec<u8>>,
    aliases: BTreeMap<String, usize>,
}
impl Shared {
    fn merge(&mut self, g: &completed_geometry::Imported) -> Result<Vec<usize>> {
        ensure!(
            g.native_meshes.len() == g.meshes.meshes.len()
                && g.native_meshes.len() == g.mesh_bmf.len(),
            "native mesh identity/cache/BMF cohort differs"
        );
        let mut map = vec![];
        for (i, &p) in g.native_meshes.iter().enumerate() {
            let index = if let Some(&old) = self.pointers.get(&p) {
                ensure!(
                    postcard::to_allocvec(&self.meshes[old])?
                        == postcard::to_allocvec(&g.meshes.meshes[i])?
                        && self.bmf[old] == g.mesh_bmf[i],
                    "same native mesh has conflicting current values"
                );
                old
            } else {
                let n = self.meshes.len();
                self.pointers.insert(p, n);
                self.meshes.push(g.meshes.meshes[i].clone());
                self.bmf.push(g.mesh_bmf[i].clone());
                n
            };
            map.push(index);
        }
        let value = serde_json::to_value(&g.meshes)?;
        for (k, v) in value["prototypes"]
            .as_object()
            .context("native retained mesh aliases absent")?
        {
            let old = usize::try_from(v.as_u64().context("native prototype index absent")?)?;
            let target = *map.get(old).context("native alias outside its pool")?;
            ensure!(
                self.aliases
                    .insert(k.clone(), target)
                    .is_none_or(|n| n == target),
                "conflicting retained prototype alias"
            );
        }
        Ok(map)
    }
    fn live_aliases(
        &mut self,
        index: &Index,
        skeleton: &str,
        id: i16,
        binding: &BindingState,
        geometry: &CharacterGeometry,
    ) -> Result<()> {
        let (avatar, keys) =
            BoundAvatar::restore_with_mesh_keys(index, skeleton, id as u64, binding)?;
        ensure!(
            keys.len() == geometry.appearances.len(),
            "native/live resource key cohort differs"
        );
        for (&identity, keys) in &keys {
            let ordinal = *geometry
                .appearances
                .get(&identity)
                .context("native appearance identity absent")?;
            let bindings = &geometry.state.appearances[ordinal];
            let range = avatar
                .part_ranges
                .get(&identity)
                .context("immutable part identity absent")?;
            ensure!(
                bindings.len() == keys.len() && range.len() == keys.len(),
                "native immutable mesh ordinal differs"
            );
            for ((&b, key), part) in bindings
                .iter()
                .zip(keys)
                .zip(&avatar.avatar.parts[range.clone()])
            {
                let mesh = geometry.state.bindings[b].mesh;
                let native = sims_formats::avatar::read_bmf(&self.bmf[mesh])?;
                ensure!(
                    signature(&native) == signature(&part.mesh),
                    "native mesh/source resource differs"
                );
                ensure!(
                    self.aliases
                        .insert(key.clone(), mesh)
                        .is_none_or(|old| old == mesh),
                    "two native mesh owners claim one source prototype key"
                );
            }
        }
        Ok(())
    }
}
pub(super) fn evaluate(
    source: &Value,
    saved: &Saved,
    prototypes: &Value,
    index: &Index,
) -> Result<Vec<Value>> {
    let native_sources = sims_runtime::conformance::completed_prototypes::import_parts(prototypes)?;
    let epoch = prototypes["provenance"]["image_sha256"]
        .as_str()
        .context("native resource epoch absent")?;
    let (_, people) = observed(source, epoch)?;
    let ordinary = ordinary(source)?;
    let mut shared = Shared {
        pointers: BTreeMap::new(),
        meshes: vec![],
        bmf: vec![],
        aliases: BTreeMap::new(),
    };
    let person_map = shared.merge(&people)?;
    ensure!(
        person_map.iter().copied().eq(0..person_map.len()),
        "native person pool changed mesh ordinal"
    );
    let ordinary_map = shared.merge(&ordinary)?;
    for (&id, g) in &people.characters {
        let a = source["resident"]["actors"]
            .as_array()
            .context("native people absent")?
            .iter()
            .find(|a| a["object"] == id)
            .context("native person missing")?;
        let b = binding(a, &raw(&a["animator_raw"])?, id, g)?;
        let skeleton = &native_sources.characters.residents[&id]
            .source
            .as_ref()
            .context("native source skeleton absent")?
            .name;
        shared.live_aliases(index, skeleton, id, &b, &g.geometry)?;
    }
    let actual = serde_json::to_value(&saved.assets.ordinary)?;
    let actors = source["ordinary"]["actors"]
        .as_array()
        .context("native ordinary actors absent")?;
    ensure!(
        actors.len() == saved.assets.ordinary.graphics.len(),
        "ordinary native cohort differs"
    );
    let mut rows = vec![];
    for (i, a) in actors.iter().enumerate() {
        let id: i16 = serde_json::from_value(a["object"].clone())?;
        let g = &ordinary.characters[&id];
        let observed = &actual["graphics"][id.to_string()];
        let b = ordinary_binding(a, g, id)?;
        let mut geometry = g.geometry.clone();
        for binding in &mut geometry.state.bindings {
            binding.mesh = *ordinary_map
                .get(binding.mesh)
                .context("ordinary binding outside native mesh pool")?;
        }
        shared.live_aliases(index, "root", id, &b, &geometry)?;
        let value: CharacterGeometry = serde_json::from_value(observed["geometry"].clone())?;
        same(
            &mut rows,
            id,
            "ordinary_complete_geometry_vertices_and_appearance_map",
            &geometry,
            &value,
        )?;
        same(
            &mut rows,
            id,
            "ordinary_complete_record_bindings_and_order",
            &b,
            &serde_json::from_value::<BindingState>(observed["binding"].clone())?,
        )?;
        same(
            &mut rows,
            id,
            "ordinary_complete_bound_root_skin_and_model",
            &bound(a, g, &source["fp_control"])?,
            &serde_json::from_value::<Bound>(observed["bound"].clone())?,
        )?;
        let def = raw(&a["definition_raw"])?;
        let owner = sims_sim::pose::graphic_owner::State {
            definition_flags: *def.get(0xcc).context("ordinary definition flags absent")?,
            resource: (i + 1) as u64,
            graphic: Some(id as u64),
            ready: serde_json::from_value(a["ready"].clone())?,
        };
        same(
            &mut rows,
            id,
            "ordinary_complete_resource_and_graphic_owner",
            &owner,
            &serde_json::from_value::<sims_sim::pose::graphic_owner::State>(
                observed["owner"].clone(),
            )?,
        )?;
        same(
            &mut rows,
            id,
            "ordinary_root_skeleton_resource_name",
            &String::from("root"),
            &serde_json::from_value::<String>(observed["skeleton"].clone())?,
        )?;
    }
    let expected: MeshPool =
        serde_json::from_value(json!({"meshes":shared.meshes,"prototypes":shared.aliases}))?;
    same(
        &mut rows,
        0,
        "complete_shared_mesh_pool_values_and_all_prototype_aliases",
        &expected,
        &saved.assets.mesh_pool,
    )?;
    Ok(rows)
}
