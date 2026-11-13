//! Constructorless ordinary storage from the completed native ROOT owners.
//! Source/raw lifetime and renderer owners remain explicit sidecars. This is
//! structural Host storage, not full process/source-version admission.
use super::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use sims_content::graphic_profiles::Cached;
use sims_formats::{iff::Iff, strings::StringTable};
use sims_sim::pose::{model, skeleton};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportedLive {
    pub ordinary: Imported,
    pub registry: graphic_loading::Saved,
    pub profile_sources: Value,
    pub complete_native_host_admitted: bool,
    pub remaining: Vec<String>,
}

fn exact<T: serde::de::DeserializeOwned + Serialize>(value: Value) -> Result<T> {
    let typed: T = serde_json::from_value(value.clone())?;
    ensure!(
        serde_json::to_value(&typed)? == value,
        "ordinary live fields defaulted/discarded"
    );
    Ok(typed)
}
fn signed(raw: &[u8], offset: usize) -> Result<i32> {
    Ok(u(raw, offset)? as i32)
}
fn floats<const N: usize>(raw: &[u8], offset: usize) -> Result<[f32; N]> {
    completed_geometry::floats(raw, offset)
}
fn matrix(raw: &sims_content::graphic_retirement::observed::Matrix) -> model::Matrix {
    model::Matrix {
        values: raw.bits.map(f32::from_bits),
        kind: raw.flags,
    }
}

const PROFILE_CODE: &[(u32, usize, &str)] = &[
    (
        0x457eec,
        116,
        "cc3dace163075bdbc73f6bb03ae41cdc040276bc0b3c057661e5d3012eb7579c",
    ),
    (
        0x455e3f,
        88,
        "892e206c5497251c997eca873668413b43e84249c1f9cbd4a052633d64894869",
    ),
    (
        0x455e97,
        114,
        "3260679ec02675845520dc3452366dff1dd3a97c43367bf1634d265941fced37",
    ),
    (
        0x470256,
        53,
        "2997d167787dd30537741ce1b62982c5d56d3a17f076c225d8d1dbbd68d0bc83",
    ),
    (
        0x46fae6,
        28,
        "701b24000696d094fa6fd69036456cfdee9e7dd58b7324dc4c3958afdc857fae",
    ),
];
fn profile_memory(profiles: &Value) -> Result<Memory> {
    let m = Memory::from(&json!({"reads":profiles["reads"],"geometry":{"reads":[]}}))?;
    let code = rows(profiles, "code")?;
    ensure!(
        code.len() == PROFILE_CODE.len(),
        "ordinary profile code coverage differs"
    );
    let mut seen = BTreeSet::new();
    for c in code {
        let address = word(c, "address")?;
        let (_, count, hash) = PROFILE_CODE
            .iter()
            .find(|(a, _, _)| *a == address)
            .context("unknown ordinary profile method")?;
        let raw = bytes(c, "raw")?;
        ensure!(
            seen.insert(address)
                && raw.len() == *count
                && word(c, "bytes")? as usize == *count
                && c["sha256"] == *hash
                && sims_fixtures::digest(&raw) == *hash
                && m.read(address, *count)? == raw,
            "ordinary profile method witness differs"
        );
    }
    Ok(m)
}

fn verify_strings(m: &Memory, p: &Value, raw: &[u8], strings: &[String]) -> Result<()> {
    let cache = word(p, "cache")?;
    ensure!(
        m.read(cache, 0x6c)? == raw
            && u(raw, 4)? == word(p, "provider")?
            && u16::from_le_bytes(raw[0x54..0x56].try_into()?) as u32 == word(p, "bank")?
            && u(raw, 0x58)? == 0x53545223
            && raw[0x5c..0x5e] == [0, 0],
        "ordinary profile metadata/readset differs"
    );
    let (begin, end, capacity) = (u(raw, 0x60)?, u(raw, 0x64)?, u(raw, 0x68)?);
    ensure!(
        begin >= 0x10000
            && begin <= end
            && end <= capacity
            && end - begin == 4 * strings.len() as u32
            && capacity - begin <= 4096 * 4,
        "ordinary current string vector differs"
    );
    let pointers = m.read(begin, (end - begin) as usize)?;
    for (i, value) in strings.iter().enumerate() {
        ensure!(
            value.len() <= 4096 && value.is_ascii() && !value.as_bytes().contains(&0),
            "unsupported ordinary cached string"
        );
        let pointer = u(&pointers, 4 * i)?;
        let mut expected = value.as_bytes().to_vec();
        expected.push(0);
        ensure!(
            m.read(pointer, expected.len())? == expected,
            "ordinary current string/readset differs"
        );
    }
    Ok(())
}

/// `index` must explicitly mount every definition's supplied IFF under its
/// current source name. The source verifier, not this structural composer,
/// authenticates those file versions against the captured provider epoch.
pub fn import(
    ordinary: Imported,
    definitions: &BTreeMap<ObjectId, Definition>,
    profiles: &Value,
    proof: &super::super::completed_skin::Input,
    index: &Index,
) -> Result<ImportedLive> {
    let closed = super::super::completed_skin::synchronous_boundary(proof)?;
    ensure!(
        profiles["provenance"]["image_sha256"] == proof.image_sha256
            && profiles["provenance"]["observation_sha256"] == proof.observation_sha256
            && ordinary.source["provenance"]["image_sha256"] == proof.image_sha256
            && ordinary.source["provenance"]["observation_sha256"] == proof.observation_sha256,
        "ordinary live source epoch differs"
    );
    import_fields(ordinary, definitions, profiles, &closed, index)
}

/// Current completed-cut variant; the old idle-cut gate remains unchanged.
pub fn import_captured(
    ordinary: Imported,
    definitions: &BTreeMap<ObjectId, Definition>,
    source: &Value,
    proof: &super::super::completed_skin::Input,
    state: &Value,
    memory: &sims_fixtures::readset::Readset<'_>,
    index: &Index,
) -> Result<ImportedLive> {
    let closed = super::super::completed_skin::captured::boundary(proof, state, memory)?;
    ensure!(
        source["schema"] == 1
            && source["contract"] == "completed-native-ordinary-profiles-v1"
            && source["tick"] == proof.tick
            && ordinary.source["tick"] == proof.tick
            && source["source_observation_sha256"] == proof.observation_sha256
            && source["source_readset_sha256"] == memory.source_sha256()
            && source["physical_compressed_sha256"] == proof.image_sha256
            && source["physical_decoded_sha256"]
                == state["physical_baseline"]["archive"]["decoded_sha256"]
            && source["executes_target_code"] == false,
        "current ordinary profile/caller epoch differs"
    );
    for row in rows(&source["profiles"], "reads")? {
        let raw = bytes(row, "raw")?;
        ensure!(
            raw.len() == word(row, "bytes")? as usize
                && row["sha256"] == sims_fixtures::digest(&raw),
            "current ordinary profile receipt differs"
        );
        super::super::completed_skin::captured::overlap(memory, word(row, "address")?, &raw)?;
    }
    super::super::completed_skeletons::captured::reads(&ordinary.source, memory)?;
    super::super::completed_skeletons::captured::reads(&ordinary.source["geometry"], memory)?;
    let mut imported = import_fields(ordinary, definitions, &source["profiles"], &closed, index)?;
    let controls: Vec<_> = state["identity"]["hardware_registers"]
        .as_str()
        .context("current ordinary CPU control absent")?
        .split_whitespace()
        .filter_map(|s| s.strip_prefix("FCW="))
        .collect();
    ensure!(
        controls.len() == 1,
        "current ordinary CPU control missing/ambiguous"
    );
    let control = u16::from_str_radix(controls[0], 16)?;
    let marker_values: BTreeMap<_, _> = rows(&imported.ordinary.source, "actors")?
        .iter()
        .map(|actor| {
            Ok((
                word(actor, "object")? as ObjectId,
                signed(&bytes(actor, "animator_raw")?, 0x100)?,
            ))
        })
        .collect::<Result<_>>()?;
    imported
        .registry
        .bind_observed_process(control, &marker_values)?;
    Ok(imported)
}

fn import_fields(
    ordinary: Imported,
    definitions: &BTreeMap<ObjectId, Definition>,
    profiles: &Value,
    closed: &BTreeSet<ObjectId>,
    index: &Index,
) -> Result<ImportedLive> {
    ensure!(
        profiles["schema"] == 1 && profiles["executes_target_code"] == false,
        "ordinary cache capture profile differs"
    );
    let profile_memory = profile_memory(profiles)?;
    ensure!(
        rows(profiles, "profiles")?.len() == rows(&ordinary.source, "actors")?.len(),
        "ordinary cache/actor ownership coverage differs"
    );
    let mut caches = vec![];
    let mut graphics = BTreeMap::new();
    for a in rows(&ordinary.source, "actors")? {
        let id = word(a, "object")? as ObjectId;
        ensure!(
            closed.contains(&id),
            "ordinary owner is outside completed caller boundary"
        );
        let input: frame::Input = serde_json::from_value(a["input"].clone())?;
        let r = &input.retained;
        let ar = bytes(a, "animator_raw")?;
        let sr = bytes(a, "skin_raw")?;
        let br = bytes(&a["bones_raw"], &r.skin.root.to_string())?;
        let objd = bytes(a, "definition_raw")?;
        let resource = bytes(a, "resource_raw")?;
        let definition = definitions
            .get(&id)
            .context("ordinary live definition absent")?
            .clone();
        ensure!(
            definition.source == format!("native-provider:{:08x}", u(&resource, 0)?)
                && definition.object == (u(&resource, 0xb4)? >> 16) as u16
                && definition.guid == u(&objd, 0x1c)?,
            "ordinary live definition alias differs"
        );
        let rows: Vec<_> = rows(profiles, "profiles")?
            .iter()
            .filter(|p| p["object"] == id)
            .collect();
        ensure!(rows.len() == 1, "ordinary current cache missing/duplicated");
        let p = rows[0];
        let raw = bytes(p, "raw")?;
        ensure!(
            p["resource"] == u(&bytes(a, "object_raw")?, 0x11c)?
                && p["cache"] == u(&resource, 0x88)?
                && p["provider"] == u(&resource, 0)?
                && p["vtable"] == 0x66728cu32
                && u(&raw, 0)? == 0x66728c
                && p["bank"] == u16::from_le_bytes(objd[0x26..0x28].try_into()?)
                && p["kind"] == i16::from_le_bytes(objd[0x12..0x14].try_into()?),
            "ordinary cache/resource alias differs"
        );
        let strings: Vec<String> = serde_json::from_value(p["strings"].clone())?;
        verify_strings(&profile_memory, p, &raw, &strings)?;
        ensure!(
            strings.len() >= 2 && strings[0].eq_ignore_ascii_case("root"),
            "ordinary ROOT profile differs"
        );
        let file = index.resources[index.find(&definition.source)?].read()?;
        let iff = Iff::parse(file)?;
        let bank = word(p, "bank")? as u16;
        let chunks: Vec<_> = iff
            .resource_order(b"STR#")?
            .into_iter()
            .filter(|c| c.id == bank)
            .collect();
        ensure!(
            chunks.len() == 1,
            "ordinary current profile bank missing/duplicated"
        );
        let decoded: Vec<_> = StringTable::parse(iff.data(chunks[0]))?
            .entries
            .into_iter()
            .filter(|e| e.language == 1)
            .map(|e| e.text)
            .collect();
        ensure!(
            decoded == strings,
            "ordinary cached/source strings differ; explicit overlay required"
        );
        let hash: [u8; 32] = Sha256::digest(&iff.bytes).into();
        let cache: Cached = exact(
            json!({"definition":definition,"state":{"kind":p["kind"],"id":bank,"file":1,"cached":1},
            "update":{"stage":"Done","constructed":null,"result":1,"pending":null},
            "profile":{"metadata":{"file":1,"id":bank,"tag":0x53545223u32,"flags":0,"language":0},"strings":strings},
            "source_hash":hash}),
        )?;
        let cache = Cached::restore(index, cache)?;
        let profile_id = caches.len() + 1;
        caches.push(cache);
        ensure!(
            r.layers == [None; 3]
                && r.skin.layers.is_empty()
                && u(&ar, 0x10)? == 0
                && u(&ar, 0x24)? == 0
                && u(&sr, 0x94)? <= 1
                && u(&sr, 0x98)? <= 1,
            "ordinary layer/anchor/dirty profile unsupported"
        );
        let local = floats::<7>(&br, 0x30)?;
        let absolute = floats::<7>(&br, 0x4c)?;
        // The ordinary base class has no person-only promote-next field. The
        // unused person hook is disabled in this explicit replacement domain.
        let skin = exact(
            json!({"animator":{"fp_control":null,"pending_vertex_updates":{},"layers":[],"slots":[null,null,null],"blend":null,"next_id":1,
            "dirty":u(&sr,0x94)?!=0,"generation":u(&sr,0xb4)?,"pose_generation":u(&sr,0xa4)?,"last_clock":signed(&sr,0xb8)?,
            "promote_next_foreground":false,"selection_cache":signed(&ar,0x98)?,"anchor_valid":false,"anchor_bone":null,
            "anchor_point":floats::<3>(&ar,0x18)?,"numbered_events":[],"pending_bone_updates":{},"pass":null},
            "bones":[{"local":{"translation":&local[..3],"rotation":&local[3..]},"priority":signed(&br,0x68)?,"blend_marker":u(&br,0x74)?}],"handler":null}),
        )?;
        let g = &ordinary.geometry.characters[&id];
        let instance = skeleton::Instance {
            skin,
            hierarchy: exact(
                json!({"parents":[null],"absolute":[{"translation":&absolute[..3],"rotation":&absolute[3..]}],"updates":[u(&br,0x130)?],"evaluated_generation":u(&sr,0xa0)?}),
            )?,
            geometry: skeleton::Geometry {
                dirty: u(&sr, 0x98)? != 0,
                generation: u(&sr, 0xac)?,
                evaluated_generation: u(&sr, 0xa8)?,
                bounds_generation: u(&sr, 0xb0)?,
                bone_generations: vec![u(&br, 0x150)?],
            },
            wiggle: vec![skeleton::WiggleState {
                gradient_scale: signed(&br, 0xe8)?,
                amplitude: floats::<1>(&br, 0x7c)?[0],
                records: r.skin.bones[0]
                    .wiggle_records
                    .map(|row| row.map(|word| word as i32)),
            }],
            flags: vec![r.skin.bones[0].flags],
            named_bone_flags: vec![r.skin.bones[0].named != 0],
        };
        let bound = model::Bound {
            skeleton: instance,
            model: g.model.clone(),
            matrix: g.matrix,
            local_bounds: g.local_bounds,
            render_bounds: g.render_bounds,
            bone_bounds: vec![bounds(input.fields.bones[&r.skin.root].bounds_bits)?],
            bone_bounds_generations: vec![u(&br, 0x134)?],
        };
        let saved: graphic_loading::SavedGraphic = exact(json!({
            "marker_value":signed(&ar,0x100)?,"owner":{"definition_flags":objd[0xcc],"resource":profile_id,"graphic":id,"ready":a["ready"]},"definition":definition,"profile":strings,
            "frame":{"clock_mode":input.fields.clock_mode,"now":input.fields.now,"visible":input.fields.visible,"blend":null,
                "placement":{"base":floats::<3>(&ar,0x80)?,"offset":floats::<3>(&ar,0x8c)?,"dirty":ar[0xd9]!=0,"explicit":input.fields.explicit!=0},
                "matrix":matrix(&r.reset.embedded_matrix)},
            "appearance":{"cached":signed(&ar,0x9c)?,"dirty":ar[0xd9]!=0,"marker":ar[0xf8]!=0,"kind":u(&ar,0xfc)?,"ordinary_material":ar[0xf7]!=0,"tint":&ar[0xf4..0xf7]},
            "bound":bound,"geometry":g.geometry,"skeleton":"root","binding":ordinary.bindings[&id],
            "materials":null,"update":null}))?;
        ensure!(
            graphics.insert(id, saved).is_none(),
            "duplicate ordinary live owner"
        );
    }
    // Empty wrappers belong to the new unpublished Rust asset epoch. Actual
    // source allocations/flags/reference counts remain intact in `ordinary`.
    let registry: graphic_loading::Saved =
        serde_json::from_value(json!({"profiles":caches,"jobs":{},"graphics":graphics,
        "retirements":{},"retained":{"graphics":{},"survivors":{},"refresh_context":null}}))?;
    let restored = graphic_loading::Registry::restore(index, registry.clone())?;
    ensure!(
        restored.snapshot() == registry,
        "ordinary constructorless restore changed state"
    );
    Ok(ImportedLive {ordinary,registry,profile_sources:profiles.clone(),complete_native_host_admitted:false,
        remaining:vec!["Complete shared Host/Simulation and source file-version epoch admission".into(),
            "Native allocation/reference-count and renderer sidecars are preserved; generic teardown must not replace them with fresh construction history".into(),
            "CPU material publication starts unpublished; native device trees are outside this headless renderer domain".into()]})
}

#[cfg(test)]
mod tests;
