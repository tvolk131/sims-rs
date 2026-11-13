//! Existing skeleton prototypes and cache noise, separate from animated clones.
//! Catalogs are immutable byte owners; native constructors/RNG are never run.
use super::{
    completed_geometry::{self, bytes, field, floats, i, rows, u, word},
    completed_residents,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sims_content::{
    Index, Resource,
    skeleton_resources::{Cache, RetainedCatalog},
};
use sims_formats::avatar::{Bone, Skeleton};
use sims_sim::pose::skeleton::{Resource as SkeletonResource, WiggleState};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub pointer: u32,
    pub prototype: u32,
    pub raw: Vec<u8>,
    pub name: String,
    pub directory: String,
    pub catalog: String,
    pub locator: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    pub pointer: u32,
    pub raw: Vec<u8>,
    pub buckets: Vec<u32>,
    pub chains: Vec<Vec<u32>>,
    pub records: Vec<Record>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prototype {
    pub pointer: u32,
    pub raw: Vec<u8>,
    pub bone_pointers: Vec<u32>,
    pub bone_raw: Vec<Vec<u8>>,
    pub locator: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    pub characters: completed_residents::Imported,
    pub skeletons: Cache,
    pub registries: Vec<Registry>,
    pub prototypes: Vec<Prototype>,
    pub adapter_epoch: String,
}
fn text(v: &Value) -> Result<String> {
    let b = bytes(v, "used_raw")?;
    narrow(b)
}
fn narrow(b: Vec<u8>) -> Result<String> {
    ensure!(
        b.is_ascii() && !b.contains(&0),
        "unsupported prototype narrow string"
    );
    Ok(String::from_utf8(b)?)
}
fn vector(v: &Value, raw: &[u8], offset: usize, stride: usize) -> Result<Vec<u8>> {
    let begin = u(raw, offset)?;
    let end = u(raw, offset + 4)?;
    let cap = u(raw, offset + 8)?;
    let b = bytes(v, "used_raw")?;
    ensure!(
        begin <= end
            && end <= cap
            && (end - begin) as usize == b.len()
            && b.len() % stride == 0
            && word(v, "stride")? as usize == stride,
        "prototype vector extent differs"
    );
    for (j, k) in ["begin", "end", "capacity"].into_iter().enumerate() {
        ensure!(
            word(v, k)? == u(raw, offset + 4 * j)?,
            "prototype vector alias differs"
        );
    }
    Ok(b)
}
fn indexed<'a>(g: &'a Value, key: &str) -> Result<BTreeMap<u32, &'a Value>> {
    let mut m = BTreeMap::new();
    for r in rows(g, key)? {
        ensure!(
            m.insert(word(r, "pointer")?, r).is_none(),
            "duplicate prototype graph owner"
        );
    }
    Ok(m)
}
fn canonical(s: &str) -> String {
    s.replace('/', "\\")
        .trim_start_matches(".\\")
        .to_ascii_lowercase()
}
fn asset_path(locator: &str) -> Result<String> {
    let p = if let Some((archive, entry)) = locator.split_once("::") {
        let parent = archive
            .rsplit_once('/')
            .context("prototype archive parent absent")?
            .0;
        format!("{parent}/{entry}")
    } else {
        locator.into()
    };
    let p = p
        .strip_prefix("GameData_ALL_Recursive/")
        .map(|x| format!("GameData/{x}"))
        .unwrap_or(p);
    ensure!(
        p.starts_with("GameData/") || p.starts_with("ExpansionShared/"),
        "prototype provider outside qualified immutable roots"
    );
    Ok(canonical(&p))
}
fn validate_source_bone(
    source: &Bone,
    raw: &[u8],
    row: &Value,
    properties: &BTreeMap<u32, &Value>,
) -> Result<()> {
    ensure!(
        raw.len() == 0x16c && text(field(row, "name")?)? == source.name,
        "prototype bone name differs"
    );
    ensure!(
        floats::<3>(raw, 0x30)?.map(f32::to_bits) == source.translation.map(f32::to_bits)
            && floats::<4>(raw, 0x3c)?.map(f32::to_bits) == source.rotation.map(f32::to_bits),
        "native prototype rest pose differs from source catalog"
    );
    ensure!(
        [u(raw, 0x6c)?, u(raw, 0x70)?, u(raw, 0x74)?] == source.flags
            && i(raw, 0x78)? == source.wiggle.gradient_scale
            && u(raw, 0x7c)? == source.wiggle.amplitude.to_bits(),
        "native prototype flags/noise parameters differ from source"
    );
    let p = u(raw, 0x2c)?;
    if p == 0 {
        ensure!(
            source.properties.is_empty(),
            "source prototype properties lost"
        );
    } else {
        let r = properties
            .get(&word(row, "pointer")?)
            .context("prototype property owner absent")?;
        let h = bytes(r, "raw")?;
        ensure!(
            word(r, "pointer")? == p && h.len() == 16,
            "prototype property alias differs"
        );
        let v = vector(field(r, "vector")?, &h, 4, 8)?;
        let pairs = rows(r, "pairs")?;
        ensure!(
            pairs.len() * 8 == v.len() && source.properties.len() == 1,
            "prototype property grouping differs"
        );
        let mut actual = vec![];
        for (raw, pair) in v.chunks_exact(8).zip(pairs) {
            ensure!(
                word(pair, "key")? == u(raw, 0)? && word(pair, "value")? == u(raw, 4)?,
                "prototype property string aliases differ"
            );
            actual.push((
                narrow(bytes(pair, "key_raw")?)?,
                narrow(bytes(pair, "value_raw")?)?,
            ));
        }
        ensure!(
            actual == source.properties[0],
            "prototype properties differ from source catalog"
        );
    }
    Ok(())
}
pub fn import(
    graph: &Value,
    catalog_input: &Value,
    characters: completed_residents::Imported,
    epoch: &str,
) -> Result<Imported> {
    ensure!(
        graph["schema"] == 1
            && graph["tick"] == 88448
            && graph["executes_target_code"] == false
            && !epoch.is_empty(),
        "prototype source epoch/profile differs"
    );
    import_fields(graph,catalog_input,characters,epoch)
}

/// Current native prototype graph with independently pinned immutable catalog
/// bytes. Current live poses never substitute for these rest skeleton owners.
pub fn import_captured(
    graph:&Value,catalog_input:&Value,characters:completed_residents::Imported,
    epoch:&str,tick:u64,memory:&sims_fixtures::readset::Readset<'_>,
) -> Result<Imported> {
    ensure!(graph["schema"]==1 && graph["tick"]==tick
        && graph["executes_target_code"]==false && !epoch.is_empty(),
        "current prototype source epoch/profile differs");
    super::completed_skeletons::captured::reads(graph,memory)?;
    for key in ["managers","records","skins","bones","properties"] {
        for row in rows(graph,key)? {
            let p=word(row,"pointer")?;let b=bytes(row,"raw")?;
            ensure!(memory.read(p,b.len())?==b,"current prototype storage differs");
        }
    }
    for row in rows(graph,"actors")? {
        let b=bytes(row,"model_raw")?;
        ensure!(memory.read(word(row,"model")?,b.len())?==b,"current prototype actor model differs");
    }
    import_fields(graph,catalog_input,characters,epoch)
}
fn import_fields(
    graph:&Value,catalog_input:&Value,mut characters:completed_residents::Imported,epoch:&str,
) -> Result<Imported> {
    completed_geometry::check_code(graph, CODE)?;
    let skins = indexed(graph, "skins")?;
    let bones = indexed(graph, "bones")?;
    let mut properties = BTreeMap::new();
    for r in rows(graph, "properties")? {
        ensure!(
            properties.insert(word(r, "bone")?, r).is_none(),
            "duplicate prototype property owner"
        );
    }
    let mut index = Index {
        root: PathBuf::from("captured-immutable-catalogs"),
        resources: vec![],
        archives: 0,
        diagnostics: vec![],
    };
    let mut catalog_sources: BTreeMap<String, (String, Vec<Skeleton>)> = BTreeMap::new();
    for (name, candidates) in field(catalog_input, "catalogs")?
        .as_object()
        .context("prototype catalog payloads absent")?
    {
        let candidates = candidates
            .as_array()
            .context("prototype providers absent")?;
        ensure!(candidates.len() == 1, "prototype source provider ambiguous");
        let c = &candidates[0];
        let b = bytes(c, "raw")?;
        let sha = format!("{:x}", Sha256::digest(&b));
        ensure!(
            c["sha256"] == sha && c["bytes"].as_u64() == Some(b.len() as u64),
            "prototype catalog hash/length differs"
        );
        let parsed = sims_formats::avatar::read_bcf(&b)?;
        let locator = field(c, "locator")?
            .as_str()
            .context("prototype catalog locator absent")?
            .to_owned();
        let resource_name = field(c, "name")?
            .as_str()
            .context("prototype resource name absent")?
            .to_owned();
        ensure!(
            resource_name.rsplit(['\\', '/']).next() == Some(name.as_str()),
            "prototype source filename differs"
        );
        index.resources.push(Resource {
            locator: locator.clone(),
            name: resource_name,
            source: PathBuf::from("captured-immutable-catalogs").join(name),
            offset: 0,
            size: b.len() as u64,
            unpacked_size: b.len() as u64,
            archived: false,
            owned: Some(Arc::from(b)),
        });
        ensure!(
            catalog_sources
                .insert(name.clone(), (locator, parsed.skeletons))
                .is_none(),
            "duplicate prototype catalog payload"
        );
    }
    let mut records = indexed(graph, "records")?;
    let mut registries = vec![];
    let mut resources: BTreeMap<String, BTreeMap<String, SkeletonResource>> = BTreeMap::new();
    let mut prototypes = vec![];
    let mut used_bones = BTreeSet::new();
    let mut used_skins = BTreeSet::new();
    let mut used_properties = BTreeSet::new();
    let mut seen_names = BTreeSet::new();
    for manager in rows(graph, "managers")? {
        let pointer = word(manager, "pointer")?;
        let raw = bytes(manager, "raw")?;
        ensure!(raw.len() == 60, "prototype registry header extent differs");
        let bucket_raw = vector(field(manager, "buckets")?, &raw, 4, 4)?;
        ensure!(!bucket_raw.is_empty(), "empty prototype registry buckets");
        let buckets = bucket_raw
            .chunks_exact(4)
            .map(|x| u(x, 0))
            .collect::<Result<Vec<_>>>()?;
        let chains: Vec<Vec<u32>> = serde_json::from_value(field(manager, "chains")?.clone())?;
        ensure!(
            chains.len() == buckets.len(),
            "prototype bucket chain extent differs"
        );
        let mut ordered = vec![];
        for (bucket, (&head, chain)) in buckets.iter().zip(&chains).enumerate() {
            ensure!(
                chain.first().copied().unwrap_or(0) == head,
                "prototype bucket head differs"
            );
            for (j, &node) in chain.iter().enumerate() {
                let r = records
                    .remove(&node)
                    .context("prototype map node duplicate/absent")?;
                let b = bytes(r, "raw")?;
                ensure!(
                    b.len() == 0x38
                        && word(r, "manager")? == pointer
                        && word(r, "bucket")? as usize == bucket
                        && u(&b, 0)? == chain.get(j + 1).copied().unwrap_or(0),
                    "prototype map chain aliases differ"
                );
                let name = narrow(vector(field(r, "name")?, &b, 8, 1)?)?;
                ensure!(
                    u(&b, 4)? as usize == name.len()
                        && sims_sim::pose::clothing::name_map::hash(name.as_bytes()) as usize
                            % buckets.len()
                            == bucket
                        && seen_names.insert((pointer, name.clone())),
                    "prototype map key/hash differs"
                );
                let directory = narrow(vector(field(r, "directory")?, &b, 0x14, 1)?)?;
                let catalog = narrow(vector(field(r, "catalog")?, &b, 0x20, 1)?)?;
                let (locator, sources) = catalog_sources
                    .get(&catalog)
                    .context("prototype catalog source absent")?;
                ensure!(
                    asset_path(locator)? == canonical(&format!("{directory}{catalog}")),
                    "prototype virtual source path differs"
                );
                let source = sources
                    .iter()
                    .find(|s| s.name == name)
                    .context("prototype catalog skeleton absent")?;
                ensure!(
                    b[0x30] == 1 && word(r, "prototype")? == u(&b, 0x34)?,
                    "prototype loaded owner differs"
                );
                let p = u(&b, 0x34)?;
                ensure!(
                    used_skins.insert(p),
                    "two prototype nodes alias one mutable owner"
                );
                let skin = skins.get(&p).context("prototype skin absent")?;
                let sh = bytes(skin, "raw")?;
                ensure!(
                    sh.len() == 0xd4 && u(&sh, 0x9c)? == 0,
                    "prototype unexpectedly bound to a live model"
                );
                ensure!(
                    narrow(vector(field(skin, "name")?, &sh, 4, 1)?)? == name,
                    "prototype skin name differs"
                );
                let ordered_bones: Vec<u32> =
                    serde_json::from_value(field(skin, "ordered_bones")?.clone())?;
                ensure!(
                    source.bones.len() == ordered_bones.len(),
                    "prototype bone count differs"
                );
                let mut raw_bones = vec![];
                let mut wiggle = vec![];
                for (source_bone, &ptr) in source.bones.iter().zip(&ordered_bones) {
                    ensure!(used_bones.insert(ptr), "prototype bones alias two owners");
                    let bone = bones.get(&ptr).context("prototype bone absent")?;
                    let br = bytes(bone, "raw")?;
                    ensure!(u(&br, 0x1c)? == p, "prototype bone reverse owner differs");
                    validate_source_bone(source_bone, &br, bone, &properties)?;
                    ensure!(
                        u(&br, 0x24)? == source_bone.parent.map(|j| ordered_bones[j]).unwrap_or(0),
                        "prototype parent link differs"
                    );
                    let parent = source_bone
                        .parent
                        .map(|j| source.bones[j].name.as_str())
                        .unwrap_or("NULL");
                    let actual = text(field(bone, "parent_name")?)?;
                    ensure!(
                        actual == parent || source_bone.parent.is_none() && actual.is_empty(),
                        "prototype parent name differs"
                    );
                    if u(&br, 0x2c)? != 0 {
                        used_properties.insert(ptr);
                    }
                    let scale = i(&br, 0x78)?;
                    ensure!(
                        i(&br, 0xe8)? == scale,
                        "prototype noise scale cache differs"
                    );
                    let mut noise = [[0; 4]; 4];
                    for (j, n) in noise.iter_mut().enumerate() {
                        for (k, x) in n.iter_mut().enumerate() {
                            *x = i(&br, 0xec + 16 * j + 4 * k)?;
                        }
                    }
                    wiggle.push(WiggleState {
                        gradient_scale: scale,
                        amplitude: floats::<1>(&br, 0x7c)?[0],
                        records: noise,
                    });
                    raw_bones.push(br);
                }
                let retained = SkeletonResource {
                    source: source.clone(),
                    wiggle,
                };
                ensure!(
                    resources
                        .entry(locator.clone())
                        .or_default()
                        .insert(name.clone(), retained)
                        .is_none(),
                    "duplicate resource in retained catalog"
                );
                prototypes.push(Prototype {
                    pointer: p,
                    raw: sh,
                    bone_pointers: ordered_bones,
                    bone_raw: raw_bones,
                    locator: locator.clone(),
                });
                ordered.push(Record {
                    pointer: node,
                    prototype: p,
                    raw: b,
                    name,
                    directory,
                    catalog,
                    locator: locator.clone(),
                });
            }
        }
        ensure!(
            ordered.len() == u(&raw, 16)? as usize,
            "prototype registry entry count differs"
        );
        registries.push(Registry {
            pointer,
            raw,
            buckets,
            chains,
            records: ordered,
        });
    }
    ensure!(
        records.is_empty()
            && used_bones.len() == bones.len()
            && used_skins.len() == skins.len()
            && used_properties.len() == properties.len(),
        "unclaimed prototype graph owners"
    );
    let mut catalogs = vec![];
    for (locator, sources) in catalog_sources.values() {
        let mut retained = resources
            .remove(locator)
            .context("catalog has no retained prototypes")?;
        let ordered = sources
            .iter()
            .map(|s| {
                retained
                    .remove(&s.name)
                    .context("loaded catalog sibling prototype missing")
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(retained.is_empty(), "unclaimed catalog prototype");
        let r = &index.resources[index.find(locator)?];
        let sha = format!("{:x}", Sha256::digest(r.read()?));
        catalogs.push(RetainedCatalog {
            locator: locator.clone(),
            sha256: sha,
            resources: ordered,
        });
    }
    ensure!(resources.is_empty(), "unclaimed prototype catalogs");
    let skeletons = Cache::from_retained(&index, catalogs)?;
    let actors = rows(graph, "actors")?;
    ensure!(
        actors.len() == characters.residents.len(),
        "prototype actor coverage differs"
    );
    let mut seen = BTreeSet::new();
    for actor in actors {
        let id = i16::try_from(word(actor, "object")?)?;
        ensure!(seen.insert(id), "duplicate prototype actor");
        let native = &characters
            .retained
            .get(&id)
            .context("prototype actor resident absent")?
            .native;
        ensure!(
            word(actor, "model")? == native.model && word(actor, "live_skin")? == native.skin,
            "prototype actor epoch differs"
        );
        let model = bytes(actor, "model_raw")?;
        let manager = word(actor, "manager")?;
        ensure!(
            model.len() == 0xb0 && u(&model, 0)? == manager && u(&model, 4)? == native.skin,
            "prototype actor manager differs"
        );
        let registry = registries
            .iter()
            .find(|m| m.pointer == manager)
            .context("prototype actor registry absent")?;
        let record = registry
            .records
            .iter()
            .find(|r| r.pointer == word(actor, "node").unwrap_or(0))
            .context("prototype actor lookup absent")?;
        ensure!(
            record.prototype == word(actor, "prototype")?
                && record.name == text(field(actor, "live_name")?)?,
            "prototype actor lookup name differs"
        );
        let source = skeletons.retained_source(&record.locator, &record.name)?;
        ensure!(
            source.bones.len() == native.bone_names.len()
                && source
                    .bones
                    .iter()
                    .zip(&native.bone_names)
                    .all(|(b, n)| b.name.as_bytes() == n),
            "prototype/live bone correspondence differs"
        );
        let resident = characters
            .residents
            .get_mut(&id)
            .context("prototype resident absent")?;
        ensure!(
            resident.source.is_none(),
            "resident source already published"
        );
        resident.source = Some(source.clone());
    }
    characters
        .demands
        .retain(|d| !d.starts_with("Immutable source skeleton/catalog provenance"));
    Ok(Imported {
        characters,
        skeletons,
        registries,
        prototypes,
        adapter_epoch: epoch.into(),
    })
}
const CODE: &[(&str, u32, usize, &str)] = &[
    (
        "FUN_005a0384",
        0x5a0384,
        96,
        "4bdc2a2d11f1fd8c0e57588a6cac640900350b48d7b0f88cbc87270d71977695",
    ),
    (
        "FUN_005a25a3",
        0x5a25a3,
        57,
        "62e5de38651156ecbece91f39f7815d3e789465f8e3d814c7f8237765d5ae73d",
    ),
    (
        "FUN_005873d9",
        0x5873d9,
        43,
        "d14ddfd122da7c1b4ef2504642ecbd48406e893de2d2d9f9c27dc1b3015bfb8d",
    ),
    (
        "FUN_005b8ef9",
        0x5b8ef9,
        57,
        "ae33825af94905ac59e746c1fd0f4d978bf99121001413ef3a770194a398e80f",
    ),
    (
        "FUN_005a27f2",
        0x5a27f2,
        43,
        "154cf92a2461fba1dbbee4664efb5328c276cd535200709351c90a23af896da5",
    ),
    (
        "FUN_0059f087",
        0x59f087,
        1317,
        "0c45d86197e6e4d40ef04a382cdf560a894ddde7fecf8df5aab0d75ea72d6905",
    ),
    (
        "FUN_00598af7",
        0x598af7,
        247,
        "49eb5536b704c25a784d439ea50ff72aa745ce0229779a3fe9085f9197078abd",
    ),
    (
        "FUN_00595947",
        0x595947,
        444,
        "d2a1f3f4e4cc4e44da9dcb2f689bc707ba3197da977520c9a973f0d39115d76b",
    ),
    (
        "FUN_00595ee8",
        0x595ee8,
        38,
        "ffbdaa2d530408e17b9cec9d4c3a1c476f3281245a9afd7aaa88fb8a0eeb6ec6",
    ),
    (
        "FUN_00594105",
        0x594105,
        122,
        "76bff17a563edd7dcc27d4dabdf001a986cc2335cba88cc5816fd42b60f354c4",
    ),
    (
        "FUN_0059431a",
        0x59431a,
        118,
        "50ffffe95ca9b9605bcb186831a5dadbbcf4803030c835d679f5017eb37d7561",
    ),
    (
        "FUN_00598da3",
        0x598da3,
        345,
        "d54f57b4ad1962d52806139102235a1ac97e0505fcaddc20a5f7ffbd331916e9",
    ),
    (
        "FUN_00595bf4",
        0x595bf4,
        369,
        "518450c270ea26f06f39125e62f46a8bed260b79dd880f240e36fa6f80c062b6",
    ),
    (
        "FUN_00595d65",
        0x595d65,
        55,
        "ed0fde8dbfe259a4fd773e1252ffb5369ebb1bd21af0196ac7f5f81b94272462",
    ),
    (
        "FUN_005a06d4",
        0x5a06d4,
        466,
        "fb453306e731317f2f6281ab7936a3491cb6c27f2dbb3b90e6d002b7d5fa049a",
    ),
];
