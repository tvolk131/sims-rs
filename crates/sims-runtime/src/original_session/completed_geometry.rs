//! Import already-published mesh caches and CPU drawables at a completed cut.
//! No attachment, bounds refresh, vertex pass or resource constructor runs here.
//! BMF below is a transport of resident mesh fields, not a claimed source file.
use super::{completed_skeletons::NativeBinding, completed_skin::Qualified};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_content::character_geometry::{CharacterGeometry, MeshPool, PublishedVertices};
use sims_sim::pose::{
    geometry::{Binding, Drawable, State},
    mesh::MeshBounds,
    model::{Bounds, Matrix, Model},
    vertices::Vertex,
};
use sims_vm::ObjectId;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Character {
    pub geometry: CharacterGeometry,
    pub model: Model,
    pub matrix: Matrix,
    pub local_bounds: Bounds,
    pub render_bounds: Bounds,
    /// Native address -> fresh, unpublished appearance identity.
    pub native_appearances: BTreeMap<u32, u64>,
    pub native_bindings: Vec<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    pub characters: BTreeMap<ObjectId, Character>,
    pub meshes: MeshPool,
    pub mesh_bmf: Vec<Vec<u8>>,
    pub native_meshes: Vec<u32>,
    pub adapter_epoch: String,
}
pub(super) fn field<'a>(v: &'a Value, k: &str) -> Result<&'a Value> {
    v.get(k)
        .with_context(|| format!("missing completed geometry field {k}"))
}
pub(super) fn word(v: &Value, k: &str) -> Result<u32> {
    u32::try_from(field(v, k)?.as_u64().context("geometry integer absent")?)
        .context("geometry integer overflow")
}
pub(super) fn bytes(v: &Value, k: &str) -> Result<Vec<u8>> {
    super::completed_animations::bytes(field(v, k)?.as_str().context("geometry hex absent")?)
}
pub(super) fn u(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4)
            .context("short native geometry storage")?
            .try_into()?,
    ))
}
pub(super) fn i(b: &[u8], o: usize) -> Result<i32> {
    Ok(u(b, o)? as i32)
}
pub(super) fn f(b: &[u8], o: usize) -> Result<f32> {
    let x = f32::from_bits(u(b, o)?);
    ensure!(x.is_finite(), "nonfinite native geometry scalar");
    Ok(x)
}
pub(super) fn floats<const N: usize>(b: &[u8], o: usize) -> Result<[f32; N]> {
    let mut a = [0.; N];
    for (j, x) in a.iter_mut().enumerate() {
        *x = f(b, o + 4 * j)?;
    }
    Ok(a)
}
fn bounds(b: &[u8], o: usize) -> Result<Bounds> {
    Ok(Bounds {
        minimum: floats(b, o)?,
        maximum: floats(b, o + 12)?,
    })
}
fn matrix(b: &[u8], o: usize) -> Result<Matrix> {
    Ok(Matrix {
        values: floats(b, o)?,
        kind: u(b, o + 64)?,
    })
}
pub(super) fn rows<'a>(v: &'a Value, k: &str) -> Result<&'a Vec<Value>> {
    field(v, k)?
        .as_array()
        .context("native geometry rows absent")
}
fn indexed<'a>(v: &'a Value, k: &str) -> Result<BTreeMap<u32, &'a Value>> {
    let mut m = BTreeMap::new();
    for x in rows(v, k)? {
        ensure!(
            m.insert(word(x, "pointer")?, x).is_none(),
            "duplicate native geometry owner"
        );
    }
    Ok(m)
}
fn vector(v: &Value, k: &str, stride: usize) -> Result<Vec<u8>> {
    let v = field(v, k)?;
    let b = bytes(v, "used_raw")?;
    let begin = word(v, "begin")?;
    let end = word(v, "end")?;
    let cap = word(v, "capacity")?;
    ensure!(
        word(v, "stride")? as usize == stride
            && begin <= end
            && end <= cap
            && (end - begin) as usize == b.len()
            && b.len() % stride == 0
            && b.len() <= 1 << 20,
        "invalid native geometry vector"
    );
    Ok(b)
}
fn owned_vector(v: &Value, k: &str, owner: &[u8], offset: usize, stride: usize) -> Result<Vec<u8>> {
    let b = vector(v, k, stride)?;
    let header = field(v, k)?;
    for (j, key) in ["begin", "end", "capacity"].into_iter().enumerate() {
        ensure!(
            word(header, key)? == u(owner, offset + 4 * j)?,
            "native geometry vector aliases differ"
        );
    }
    Ok(b)
}
fn string(out: &mut Vec<u8>, b: &[u8]) -> Result<()> {
    ensure!(
        b.len() < 255 && b.is_ascii() && !b.contains(&0),
        "unsupported resident mesh string"
    );
    out.push(b.len() as u8);
    out.extend(b);
    Ok(())
}
fn block(out: &mut Vec<u8>, b: &[u8], stride: usize) {
    out.extend(((b.len() / stride) as u32).to_le_bytes());
    out.extend(b);
}
fn at<'a>(m: &BTreeMap<u32, &'a Value>, p: u32) -> Result<&'a Value> {
    m.get(&p).copied().context("uncaptured geometry owner")
}
pub(super) fn check_code(input: &Value, expected: &[(&str, u32, usize, &str)]) -> Result<()> {
    let code = rows(input, "code")?;
    ensure!(
        code.len() == expected.len(),
        "completed geometry code witness count differs"
    );
    for (name, address, count, sha) in expected {
        let row = code
            .iter()
            .find(|x| x["name"] == *name)
            .context("completed geometry witness absent")?;
        let raw = bytes(row, "raw")?;
        ensure!(
            word(row, "address")? == *address
                && raw.len() == *count
                && format!("{:x}", Sha256::digest(&raw)) == *sha
                && row["sha256"] == *sha,
            "completed geometry code differs"
        );
    }
    Ok(())
}
pub fn import(
    input: &Value,
    skins: &BTreeMap<ObjectId, Qualified>,
    fresh_epoch: &str,
) -> Result<Imported> {
    let bindings = skins
        .iter()
        .map(|(&id, skin)| {
            (
                id,
                Skin {
                    native: &skin.retained.native,
                    bone_vertex_bounds: &skin.retained.bone_vertex_bounds,
                },
            )
        })
        .collect();
    import_with_bindings(input, &bindings, fresh_epoch)
}

/// The geometry decoder needs identities and retained vertex bounds, not a
/// person skin or a running animator. Ordinary built-in ROOT owners use this
/// same decoder without inventing a Qualified person or executing a constructor.
pub struct Skin<'a> {
    pub native: &'a NativeBinding,
    pub bone_vertex_bounds: &'a [Bounds],
}

pub fn import_with_bindings(
    input: &Value,
    skins: &BTreeMap<ObjectId, Skin<'_>>,
    fresh_epoch: &str,
) -> Result<Imported> {
    ensure!(
        !fresh_epoch.is_empty() && input["schema"] == 1 && input["executes_target_code"] == false,
        "completed geometry profile/epoch differs"
    );
    check_code(input, CODE)?;
    let appearances = indexed(input, "appearances")?;
    let bindings = indexed(input, "bindings")?;
    let meshes = indexed(input, "meshes")?;
    let drawables = indexed(input, "drawables")?;
    let mut pool = vec![];
    let mut bmf = vec![];
    let mut native_meshes = vec![];
    let mut mesh_indices = BTreeMap::new();
    let mut chars = BTreeMap::new();
    let mut all_appearances = BTreeSet::new();
    let mut used_bindings = BTreeSet::new();
    let mut used_drawables = BTreeSet::new();
    let mut next_id = 1u64;
    for row in rows(input, "characters")? {
        let id =
            ObjectId::try_from(word(row, "object")?).context("native character id overflow")?;
        let skin = skins.get(&id).context("geometry skin absent")?;
        let native = skin.native;
        ensure!(
            word(row, "model")? == native.model && word(row, "skin")? == native.skin,
            "geometry model/skin epoch differs"
        );
        let model = bytes(row, "model_raw")?;
        let sh = bytes(row, "skin_raw")?;
        let rb = bytes(row, "render_bounds_raw")?;
        ensure!(
            model.len() == 0xb0
                && sh.len() == 0xd4
                && rb.len() == 0x80
                && u(&model, 4)? == native.skin
                && u(&model, 0x14)? == native.animator
                && u(&sh, 0x9c)? == native.model
                && u(&model, 0xc)? == word(row, "render_bounds")?,
            "geometry model aliases differ"
        );
        let mut state = State {
            bindings: vec![],
            appearances: vec![],
            bone_vertex_bounds: skin.bone_vertex_bounds.to_vec(),
        };
        let mut ids = BTreeMap::new();
        let mut id_indices = BTreeMap::new();
        let mut native_bindings = vec![];
        let mut published = BTreeMap::new();
        for a in owned_vector(row, "appearances", &sh, 0x88, 4)?.chunks_exact(4) {
            let p = u(a, 0)?;
            ensure!(all_appearances.insert(p), "appearance aliases two skins");
            let a = at(&appearances, p)?;
            let ar = bytes(a, "raw")?;
            ensure!(
                ar.len() == 28 && u(&ar, 0)? == native.skin,
                "appearance reverse owner differs"
            );
            let appearance_index = state.appearances.len();
            let identity = next_id;
            next_id = next_id
                .checked_add(1)
                .context("appearance identity overflow")?;
            ids.insert(p, identity);
            id_indices.insert(identity, appearance_index);
            let mut ordered = vec![];
            let records = owned_vector(a, "bindings", &ar, 8, 12)?;
            ensure!(
                records.len() / 12 == rows(a, "records")?.len(),
                "appearance descriptor count differs"
            );
            for (record, receipt) in records.chunks_exact(12).zip(rows(a, "records")?) {
                let binding_ptr = u(record, 8)?;
                ensure!(
                    word(receipt, "bone")? == u(record, 0)?
                        && word(receipt, "descriptor")? == u(record, 4)?
                        && word(receipt, "binding")? == binding_ptr
                        && native.bone_pointers.contains(&u(record, 0)?),
                    "appearance descriptor aliases differ"
                );
                ensure!(
                    used_bindings.insert(binding_ptr),
                    "multiply owned mesh binding"
                );
                let binding = at(&bindings, binding_ptr)?;
                let raw = bytes(binding, "raw")?;
                ensure!(
                    raw.len() == 0x34 && u(&raw, 4)? == native.skin,
                    "mesh binding skin differs"
                );
                let mp = u(&raw, 8)?;
                let dp = u(&raw, 12)?;
                ensure!(
                    word(binding, "mesh")? == mp
                        && word(binding, "drawable")? == dp
                        && used_drawables.insert(dp),
                    "mesh/drawable aliases differ"
                );
                let mesh = at(&meshes, mp)?;
                let mr = bytes(mesh, "raw")?;
                ensure!(mr.len() == 0xac, "mesh header size differs");
                let pointers = owned_vector(binding, "bones", &raw, 0x10, 4)?;
                let bone_ids = owned_vector(mesh, "bone_ids", &mr, 0x28, 4)?;
                ensure!(pointers.len() == bone_ids.len(), "mesh mapped bones differ");
                let mut mapped = vec![];
                for (ptr, name) in pointers.chunks_exact(4).zip(bone_ids.chunks_exact(4)) {
                    let bp = u(ptr, 0)?;
                    let index = native
                        .bone_pointers
                        .iter()
                        .position(|&p| p == bp)
                        .context("mapped mesh bone absent")?;
                    ensure!(
                        native.lookup.get(u(name, 0)? as usize) == Some(&Some(index)),
                        "mesh name lookup differs"
                    );
                    mapped.push(index);
                }
                let mut transport = vec![];
                string(&mut transport, &owned_vector(mesh, "name", &mr, 0, 1)?)?;
                string(&mut transport, &owned_vector(mesh, "texture", &mr, 12, 1)?)?;
                transport.extend((mapped.len() as u32).to_le_bytes());
                for &bone in &mapped {
                    string(&mut transport, &native.bone_names[bone])?;
                }
                block(
                    &mut transport,
                    &owned_vector(mesh, "triangles", &mr, 0x34, 12)?,
                    12,
                );
                let br = owned_vector(mesh, "bindings", &mr, 0x40, 24)?;
                transport.extend(((br.len() / 24) as u32).to_le_bytes());
                for b in br.chunks_exact(24) {
                    transport.extend(&b[..20]);
                }
                block(&mut transport, &owned_vector(mesh, "uvs", &mr, 0x4c, 8)?, 8);
                block(
                    &mut transport,
                    &owned_vector(mesh, "blends", &mr, 0x58, 8)?,
                    8,
                );
                block(
                    &mut transport,
                    &owned_vector(mesh, "samples", &mr, 0x64, 24)?,
                    24,
                );
                let parsed = sims_formats::avatar::read_bmf(&transport)?;
                ensure!(
                    parsed.bones.len() == mapped.len(),
                    "mesh transport bone count differs"
                );
                let local = owned_vector(mesh, "local_bounds", &mr, 0x70, 24)?
                    .chunks_exact(24)
                    .map(|b| bounds(b, 0))
                    .collect::<Result<Vec<_>>>()?;
                let stamps = br
                    .chunks_exact(24)
                    .map(|b| u(b, 20))
                    .collect::<Result<Vec<_>>>()?;
                ensure!(
                    local.len() == parsed.bindings.len(),
                    "mesh local bounds count differs"
                );
                let cache = MeshBounds {
                    local,
                    stamps,
                    bounds: bounds(&mr, 0x94)?,
                };
                let mesh_index = if let Some(&index) = mesh_indices.get(&mp) {
                    ensure!(
                        pool[index] == cache && bmf[index] == transport,
                        "shared mesh cache differs by binding"
                    );
                    index
                } else {
                    let index = pool.len();
                    pool.push(cache);
                    bmf.push(transport);
                    native_meshes.push(mp);
                    mesh_indices.insert(mp, index);
                    index
                };
                let drawable = at(&drawables, dp)?;
                let dr = bytes(drawable, "raw")?;
                let vr = bytes(drawable, "vertices_raw")?;
                ensure!(
                    dr.len() == 0x118
                        && u(&dr, 0xc8)? == word(drawable, "vertices_pointer")?
                        && vr.len() == 32 * (parsed.vertices.len() + parsed.blends.len()),
                    "drawable CPU buffer extent differs"
                );
                let vertices = vr
                    .chunks_exact(32)
                    .map(|v| {
                        Ok(Vertex {
                            position: floats(v, 0)?,
                            normal: floats(v, 12)?,
                            uv: floats(v, 24)?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let ranges = owned_vector(drawable, "ranges", &dr, 0xe0, 8)?;
                ensure!(ranges.len() == 8, "unsupported drawable range ownership");
                let index = state.bindings.len();
                ordered.push(index);
                native_bindings.push(binding_ptr);
                state.bindings.push(Binding {
                    mesh: mesh_index,
                    bones: mapped,
                    bounds: bounds(&raw, 0x1c)?,
                    drawable: Drawable {
                        bounds: bounds(&dr, 0xf8)?,
                        flags: u(&dr, 0x14)?,
                    },
                });
                published.insert(
                    index,
                    PublishedVertices {
                        vertices,
                        buffer_flags: u(&dr, 0xc4)?,
                        range: [i(&ranges, 0)?, i(&ranges, 4)?],
                    },
                );
            }
            ensure!(
                appearance_index == state.appearances.len(),
                "appearance order changed"
            );
            state.appearances.push(ordered);
        }
        let character = Character {
            geometry: CharacterGeometry {
                state,
                appearances: id_indices,
                vertices: published,
            },
            model: Model {
                placement: matrix(&model, 0x24)?,
                scale: matrix(&model, 0x68)?,
                bounds_transform: matrix(&rb, 0x18)?,
                bounds_flags: u(&rb, 0x14)?,
            },
            matrix: matrix(&sh, 0x1c)?,
            local_bounds: bounds(&sh, 0xbc)?,
            render_bounds: bounds(&rb, 0x68)?,
            native_appearances: ids,
            native_bindings,
        };
        ensure!(
            character.matrix.values.map(f32::to_bits)
                == character.model.bounds_transform.values.map(f32::to_bits),
            "model/skin published transform differs"
        );
        ensure!(
            chars.insert(id, character).is_none(),
            "duplicate geometry character"
        );
    }
    ensure!(
        chars.keys().eq(skins.keys())
            && all_appearances.len() == appearances.len()
            && used_bindings.len() == bindings.len()
            && used_drawables.len() == drawables.len()
            && mesh_indices.len() == meshes.len(),
        "uncaptured or unclaimed geometry graph owner"
    );
    let prototypes: BTreeMap<_, _> = native_meshes
        .iter()
        .enumerate()
        .map(|(i, p)| (format!("resident:{fresh_epoch}/{p:08x}"), i))
        .collect();
    let meshes = serde_json::from_value(json!({"meshes":pool,"prototypes":prototypes}))?;
    Ok(Imported {
        characters: chars,
        meshes,
        mesh_bmf: bmf,
        native_meshes,
        adapter_epoch: fresh_epoch.into(),
    })
}

const CODE: &[(&str, u32, usize, &str)] = &[
    (
        "FUN_00595629",
        0x595629,
        207,
        "2433158b7a8d9e938062bcbf9dd5974973e97b3171fd7967980fd6077ecad6cd",
    ),
    (
        "FUN_00599e87",
        0x599e87,
        409,
        "1ea53e7647ec3f023b56c268118991b525c58d87d2bdc4c2dc9a4048b278470e",
    ),
    (
        "FUN_00599955",
        0x599955,
        593,
        "e8add11699092e95f7e08092b9c1755f700979057a188430d4b7399b632778fe",
    ),
    (
        "FUN_0059a1c1",
        0x59a1c1,
        608,
        "328b518f2a2ca97eba26a4059a87bdaddd83e7f057d090003d2dbed51844577e",
    ),
    (
        "FUN_0059a020",
        0x59a020,
        399,
        "492538a187760fad9a4a852cf85785c916ab82504e7929e797d65157826cd4df",
    ),
    (
        "FUN_005a1103",
        0x5a1103,
        764,
        "e14fce021b50609fdba75e903f021a2ae6f23e21d4ee6982929508ab458f2ea8",
    ),
    (
        "FUN_0059934b",
        0x59934b,
        171,
        "2645c5ec61579b15e665c376cc7bf6c8121967bac334b11789af5c9936729cc0",
    ),
    (
        "FUN_0059a45d",
        0x59a45d,
        140,
        "54e29194872fea8d17a76ba841c1d06c3bb0b5fe851dcfe541197842b32f0dd7",
    ),
    (
        "FUN_005d13c7",
        0x5d13c7,
        85,
        "f0c172acbf9ca1f8cdcc99b8d4dd873b7cb839fe100d16672f2dda0f3d814bc0",
    ),
    (
        "FUN_005a0fcf",
        0x5a0fcf,
        46,
        "a0813bdbc6a5243bb04574681dc0df5859aa25e3ddde8164486db5fba245d87f",
    ),
    (
        "FUN_005a0e32",
        0x5a0e32,
        122,
        "b59113686af1485da09956926d3c5c9a1ca680bb50c2c6bf2f25d80a3c1c4f38",
    ),
    (
        "FUN_005d1590",
        0x5d1590,
        62,
        "4e166ad2890c6b1b2aa89b10d11ce2b1e4a941c3cfdcb52fe363c59b26a6223b",
    ),
];
