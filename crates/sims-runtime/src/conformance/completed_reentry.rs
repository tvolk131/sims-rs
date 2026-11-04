//! Constructorless source Object/Thread component, never executable World ingress.
use super::{AdapterResult, Scope, Status};
use crate::original_session::{completed_groups, completed_object_storage as objects};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sims_fixtures::{ArtifactFormat, CaseManifest, Coverage, Store};
use sims_vm::{
    ObjectId, Status as VmStatus, Thread, World,
    native_registry_reentry::{AdapterOwnership, Evidence},
    retained_frames::ThreadControls,
    scheduler::storage::Storage,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};
pub const CONTRACT: &str = "completed-native-reentry-component-v1";

/// Consumed, source-qualified owners for completed VM composition. This does
/// not return the validation carrier World or supply any of its unrelated
/// fields. The raw stack/controls move into a future complete epoch once the
/// remaining World and lifecycle owners have been independently supplied.
pub struct Parts {
    pub program: sims_vm::Program,
    pub objects: BTreeMap<ObjectId, sims_vm::Object>,
    pub registry: Storage,
    pub entries: Vec<sims_vm::retained_completed::CompletedThreadInput>,
    pub source_epoch: String,
    pub provenance: Value,
    pub report: Value,
}

pub fn import_parts(input: Value) -> Result<Parts> {
    import_input(serde_json::from_value(input)?)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u32,
    contract: String,
    tick: u64,
    native_parity_ticks: u64,
    stacks: Value,
    groups: Value,
    objects: Vec<objects::Input>,
    controls: Vec<Control>,
    function_caches: Vec<Value>,
    registry: Registry,
    source_closure: Value,
    scope: String,
    normalizations: Vec<String>,
    unqualified: Vec<String>,
    provenance: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    handles: Vec<Option<u32>>,
    words: Vec<u32>,
    last_handle: u32,
    native: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Control {
    object: ObjectId,
    evidence: Evidence,
    adapter: AdapterOwnership,
    controls_without_status: Value,
}
fn digest(v: &Value) -> Result<String> {
    Ok(sims_fixtures::digest(&serde_json::to_vec(v)?))
}
fn ids(v: &Value) -> Result<Vec<ObjectId>> {
    Ok(serde_json::from_value(v.clone())?)
}
fn span(v: &Value) -> Result<Vec<u8>> {
    let b = crate::original_session::completed_animations::bytes(
        v["raw"].as_str().context("native span raw absent")?,
    )?;
    ensure!(
        v["bytes"].as_u64() == Some(b.len() as u64) && v["sha256"] == sims_fixtures::digest(&b),
        "native span hash/extent differs"
    );
    Ok(b)
}
fn word(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
fn native_vector(v: &Value) -> Result<(u32, Vec<u8>, Vec<u8>)> {
    let header = span(&v["header"])?;
    ensure!(header.len() == 12, "native vector header length differs");
    let (begin, end, capacity) = (word(&header, 0), word(&header, 4), word(&header, 8));
    ensure!(
        (begin == 0 && end == 0 && capacity == 0)
            || (begin >= 0x10000
                && begin <= end
                && end <= capacity
                && begin % 4 == 0
                && (end - begin) % 4 == 0
                && (capacity - begin) % 4 == 0
                && capacity - begin <= 131072),
        "native vector extent differs"
    );
    ensure!(
        v["begin"] == begin && v["end"] == end && v["capacity"] == capacity,
        "native vector decoded/header aliases differ"
    );
    let body = span(&v["allocated"])?;
    ensure!(
        v["allocated"]["address"] == begin && body.len() == (capacity - begin) as usize,
        "native vector backing differs"
    );
    Ok((
        begin,
        body[..(end - begin) as usize].to_vec(),
        body[(end - begin) as usize..].to_vec(),
    ))
}

fn evaluate(i: Input) -> Result<Value> {
    Ok(import_input(i)?.report)
}

fn import_input(i: Input) -> Result<Parts> {
    ensure!(
        i.schema == 1
            && i.contract == CONTRACT
            && i.tick == 88448
            && i.native_parity_ticks == 0
            && !i.scope.is_empty()
            && i.unqualified.len() >= 6
            && i.normalizations.len() == 2,
        "reentry component profile differs"
    );
    ensure!(
        i.provenance["image_sha256"]
            == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && i.provenance["observation_sha256"]
                == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c",
        "reentry component source epoch differs"
    );
    let epoch = "goth88448-new-reentry-component-v1";
    let object_map = objects::import(&i.objects, epoch)?;
    ensure!(
        object_map.len() == 344 && i.controls.len() == 344,
        "incomplete original object/control coverage"
    );
    let expected_ids: Vec<_> = object_map.keys().copied().collect();
    ensure!(
        i.source_closure["status"] == "source_closure_observed"
            && i.source_closure["tick"] == i.tick
            && i.source_closure["native_parity_ticks"] == 0
            && i.source_closure["native_globals"]["active_menu_descriptor"] == 0,
        "source caller closure unavailable"
    );
    ensure!(
        ids(&i.source_closure["construction_absent_objects"])? == expected_ids
            && ids(&i.source_closure["native_consumer_absent_objects"])? == expected_ids,
        "source caller closure coverage differs"
    );
    ensure!(
        i.source_closure["image_sha256"] == i.provenance["image_sha256"]
            && i.source_closure["observation_sha256"] == i.provenance["observation_sha256"],
        "source caller closure epoch differs"
    );
    ensure!(
        digest(&i.source_closure)?
            == "bcbced9059a201f08ab04b68ea432d1adc2cbab3a0ef098646e8056f85944e50",
        "qualified frozen source caller proof differs"
    );
    for r in i.source_closure["reads"]
        .as_array()
        .context("closure reads absent")?
    {
        let b = crate::original_session::completed_animations::bytes(
            r["raw"].as_str().context("closure raw absent")?,
        )?;
        ensure!(
            r["bytes"].as_u64() == Some(b.len() as u64) && r["sha256"] == sims_fixtures::digest(&b),
            "source closure read hash/extent differs"
        );
    }
    // A read-only validation carrier supplies EXACTLY the captured Objects and
    // registry. Its unrelated default fields are never observed, saved, run or
    // claimed as native World state. No Session/VM is constructed here.
    let mut registry_view = World {
        objects: object_map,
        ..World::default()
    };
    let storage = Storage::from_retained(
        &registry_view,
        i.registry.handles.clone(),
        i.registry.words.clone(),
        i.registry.last_handle,
    )
    .map_err(anyhow::Error::msg)?;
    storage
        .require_complete_backing(&registry_view)
        .map_err(anyhow::Error::msg)?;
    registry_view.scheduler_storage = Some(storage.clone());
    let object_bytes = serde_json::to_vec(&registry_view.objects)?;
    let cold_objects: BTreeMap<ObjectId, sims_vm::Object> = serde_json::from_slice(&object_bytes)?;
    let storage_bytes = serde_json::to_vec(&storage)?;
    let cold_storage: Storage = serde_json::from_slice(&storage_bytes)?;
    ensure!(
        cold_objects == registry_view.objects
            && serde_json::to_vec(&cold_objects)? == object_bytes
            && cold_storage == storage
            && serde_json::to_vec(&cold_storage)? == storage_bytes,
        "captured object/registry named-field cold roundtrip differs"
    );
    // occupied_ids retains the entire used pointer vector, including42 holes.
    // Its length is not the live object count; do not discard vacant slots.
    let native_ids = ids(&i.registry.native["occupied_ids"])?;
    ensure!(
        native_ids.len() == i.registry.handles.len()
            && native_ids
                .iter()
                .zip(&i.registry.handles)
                .enumerate()
                .all(|(n, (&id, handle))| match handle {
                    Some(h) => id as usize == n + 1 && *h == id as u32,
                    None => id == 0,
                })
            && native_ids
                .iter()
                .copied()
                .filter(|id| *id > 0)
                .collect::<Vec<_>>()
                == expected_ids,
        "native registry membership differs"
    );
    let objects_by_pointer: BTreeMap<ObjectId, u32> = i
        .objects
        .iter()
        .map(|r| {
            Ok((
                serde_json::from_value(r.native["object"].clone())?,
                serde_json::from_value(r.native["pointer"].clone())?,
            ))
        })
        .collect::<Result<_>>()?;
    let (_, pointer_bytes, _) = native_vector(&i.registry.native["pointer_vector"])?;
    let (_, word_bytes, _) = native_vector(&i.registry.native["word_vector"])?;
    ensure!(
        pointer_bytes.len() == i.registry.handles.len() * 4
            && word_bytes.len() == i.registry.words.len() * 4
            && word_bytes
                .chunks_exact(4)
                .map(|b| word(b, 0))
                .eq(i.registry.words.iter().copied())
            && pointer_bytes
                .chunks_exact(4)
                .zip(&native_ids)
                .all(|(b, id)| word(b, 0) == if *id == 0 { 0 } else { objects_by_pointer[id] })
            && i.registry.last_handle
                == i.registry
                    .handles
                    .iter()
                    .copied()
                    .flatten()
                    .max()
                    .unwrap_or(0),
        "native registry backing/canonical mapping differs"
    );
    let group_rows: Vec<completed_groups::Input> =
        serde_json::from_value(i.groups["groups"].clone())?;
    let parts = completed_groups::import(&group_rows, &objects_by_pointer)?;
    ensure!(
        parts.len() == 142
            && parts
                .iter()
                .all(|(id, p)| registry_view.objects[id].multitile.as_ref() == Some(p)),
        "object/group owners differ"
    );
    let closure_source = format!(
        "goth-completed88448/source-closure@sha256:{}#",
        digest(&i.source_closure)?
    );
    let topology_source = format!("goth-completed88448/groups@sha256:{}", digest(&i.groups)?);
    let (mut program, mut stacks) = super::completed_stacks::import_storage(&i.stacks, &storage)?;
    let mut cached = BTreeSet::new();
    for cache in &i.function_caches {
        let id: ObjectId = serde_json::from_value(cache["object"].clone())?;
        ensure!(cached.insert(id), "duplicate native callback cache owner");
        let control = i
            .controls
            .iter()
            .find(|c| c.object == id)
            .context("callback cache Thread absent")?;
        ensure!(
            cache["source_binding"] == serde_json::to_value(&control.evidence.source_binding)?,
            "callback cache context/resource differs"
        );
        let resource = span(&cache["resource_table"])?;
        let header = span(&cache["vector"]["header"])?;
        ensure!(
            resource.len() == 4
                && header.len() == 12
                && cache["resource_table"]["address"].as_u64()
                    == Some(control.evidence.source_binding.resource as u64 + 0x90)
                && cache["vector"]["header"]["address"] == word(&resource, 0),
            "callback cache native owner/header differs"
        );
        let (begin, end, capacity) = (word(&header, 0), word(&header, 4), word(&header, 8));
        ensure!(
            begin >= 0x10000
                && begin <= end
                && end <= capacity
                && (end - begin) % 4 == 0
                && (capacity - begin) % 4 == 0
                && capacity - begin <= 128000,
            "callback cache extent differs"
        );
        let used = span(&cache["used"])?;
        let spare = span(&cache["spare"])?;
        ensure!(
            cache["used"]["address"] == begin
                && used.len() == (end - begin) as usize
                && cache["spare"]["address"] == end
                && spare.len() == (capacity - end) as usize
                && used.len() >= 8,
            "callback cache storage differs"
        );
        //455fa9 reads the low action word from each native4-byte record.
        // Preserve the other word too; no source-file OBJf reload/default fill.
        let callbacks: Vec<_> = used
            .chunks_exact(4)
            .map(|b| sims_formats::execution::ObjectFunction {
                action: u16::from_le_bytes([b[0], b[1]]),
                condition: u16::from_le_bytes([b[2], b[3]]),
            })
            .collect();
        ensure!(
            callbacks[0].action == control.evidence.cached_init
                && callbacks[1].action == control.evidence.cached_main,
            "native callback cache Init/Main differs"
        );
        let def = registry_view.objects[&id]
            .definition
            .as_ref()
            .context("callback cache definition absent")?;
        let module = program
            .modules
            .get_mut(&def.source)
            .context("callback cache module absent")?;
        if let Some(old) = module
            .object_functions
            .insert(def.object, callbacks.clone())
        {
            ensure!(
                old == callbacks,
                "shared native definition aliases different callback tables"
            );
        }
    }
    ensure!(
        cached.into_iter().collect::<Vec<_>>() == expected_ids,
        "native callback cache coverage differs"
    );
    let mut threads = BTreeMap::new();
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    let mut sleeping = 0;
    let mut polling = 0;
    for c in i.controls {
        ensure!(
            seen.insert(c.object)
                && c.adapter.fresh.source_key == epoch
                && c.evidence.adapter_epoch_source == epoch
                && c.adapter.construction_absent.source
                    == format!("{closure_source}construction_absent_objects")
                && c.adapter.native_consumer_absent.source
                    == format!("{closure_source}native_consumer_absent_objects"),
            "reentry adapter/closure binding differs"
        );
        ensure!(
            c.adapter.construction_absent.objects == expected_ids
                && c.adapter.native_consumer_absent.objects == expected_ids,
            "reentry scoped absence differs"
        );
        let source = match &c.adapter.topology {
            sims_vm::native_registry_reentry::Topology::NonGroup { source }
            | sims_vm::native_registry_reentry::Topology::LiveGroup { source, .. } => source,
        };
        ensure!(
            source == &topology_source,
            "reentry topology source differs"
        );
        ensure!(
            c.evidence.native_pointer == objects_by_pointer[&c.object],
            "reentry object pointer differs"
        );
        ensure!(
            c.evidence.raw_capture_sha256 == i.source_closure["raw_threads_sha256"]
                && i.registry.native["pointer_vector"]["header"]["address"].as_u64()
                    == Some(c.evidence.boundary.registry as u64 + 0x18)
                && i.registry.native["word_vector"]["header"]["address"].as_u64()
                    == Some(c.evidence.boundary.registry as u64 + 0x24),
            "native reentry source registry/raw epoch differs"
        );
        let raw = i
            .objects
            .iter()
            .find(|r| r.native["object"] == c.object)
            .context("reentry raw object absent")?;
        if matches!(
            &c.evidence.leaf,
            sims_vm::native_registry_reentry::Leaf::PersonAnimatePolling { .. }
        ) {
            // This profile imports direct Main frames with caller==callee and
            // no Frame action metadata. A captured native use record belongs
            // to an invocation depth and must never disappear behind those
            // adapter values. The actual captured three people have no uses.
            let uses = raw.legacy["person"]["uses"]
                .as_array()
                .context("native person action-use vector absent")?;
            ensure!(
                uses.is_empty(),
                "nonempty native action-use vector requires invocation-frame ownership import"
            );
        }
        let last = raw.legacy["last_result"]
            .as_u64()
            .context("native last result absent")?;
        ensure!(
            last <= 1
                && c.controls_without_status
                    == json!({"service":null,"dispatch":null,"autonomy":null,"find_best":[],"completed_actions":[],"last_result":last!=0,"callback":null,"callback_result":null,"check":false,"interrupted":false}),
            "reentry Thread controls differ from scoped source/fresh adapter"
        );
        let stack = stacks
            .remove(&c.object)
            .context("reentry retained stack absent")?;
        match &c.evidence.leaf {
            sims_vm::native_registry_reentry::Leaf::BaseTickSleep { .. } => sleeping += 1,
            _ => polling += 1,
        }
        let owner = stack
            .native_registry_reentry(&program, &registry_view, i.tick, c.evidence, c.adapter)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("native object {}", c.object))?;
        let mut controls = c.controls_without_status;
        controls
            .as_object_mut()
            .context("Thread controls are not an object")?
            .insert(
                "status".into(),
                serde_json::to_value(VmStatus::NativeRegistryReentry(owner))?,
            );
        let controls: ThreadControls = serde_json::from_value(controls)?;
        entries.push(sims_vm::retained_completed::CompletedThreadInput {
            source_owner: format!("{epoch}/native-object-{}", c.object),
            adapter_thread: c.object as u64,
            object: Some(c.object),
            stack: stack.clone(),
            controls: controls.clone(),
        });
        let t = stack
            .into_thread(&program, controls)
            .map_err(anyhow::Error::msg)?;
        let bytes = serde_json::to_vec(&t)?;
        let cold: Thread = serde_json::from_slice(&bytes)?;
        ensure!(
            t == cold && serde_json::to_vec(&cold)? == bytes,
            "retained Thread named-field cold roundtrip differs"
        );
        threads.insert(c.object, t);
    }
    ensure!(
        stacks.is_empty()
            && seen.into_iter().collect::<Vec<_>>() == expected_ids
            && sleeping == 341
            && polling == 3,
        "native Thread reentry coverage differs"
    );
    let encoded = serde_json::to_vec(&threads)?;
    let report = json!({"schema":1,"contract":CONTRACT,"scope":"original_component","status":"passed","tick":i.tick,"native_parity_ticks":0,"objects":344,"threads":344,"base_sleep":sleeping,"person_animation_polling":polling,"multipart_parts":parts.len(),"native_arenas_preserved":true,"named_field_cold_roundtrip_equal":true,"new_adapter_epoch":epoch,"normalized_thread_sha256":sims_fixtures::digest(&encoded),"normalizations":i.normalizations,"unqualified":i.unqualified,"resumable_baseline":false,"executes_vm":false});
    Ok(Parts {
        program,
        objects: registry_view.objects,
        registry: storage,
        entries,
        source_epoch: epoch.into(),
        provenance: i.provenance,
        report,
    })
}
pub fn run(store: &Store, m: &CaseManifest, output: &Path) -> Result<AdapterResult> {
    ensure!(
        m.source.format == ArtifactFormat::Json
            && m.source.schema == 1
            && matches!(&m.coverage,Coverage::Component{scope,..}if scope==CONTRACT),
        "reentry fixture contract differs"
    );
    let report = evaluate(serde_json::from_slice(
        &store.read_bytes(&m.source.blob, 32 << 20)?,
    )?)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("completed-reentry.json"))?;
    serde_json::to_writer_pretty(&mut f, &report)?;
    f.write_all(b"\n")?;
    Ok(AdapterResult{status:Status::Passed,scope:Scope::OriginalComponent,contract:CONTRACT.into(),observations:344,completed_ticks:0,matched_completed_ticks:0,difference:None,detail:Some("344 constructorless source Objects and retained Thread reentry owners pass scoped caller closure, actual Tick dispatch, arena preservation and named-field cold roundtrip; complete VM/World/Character/Session and original next-tick comparison remain unqualified".into()),artifacts:vec![PathBuf::from("completed-reentry.json")]})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private frozen Goth reentry input; no native or VM execution"]
    fn returned_parts_keep_qualified_stack_controls_for_composition() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let input:Value=serde_json::from_slice(&std::fs::read(root.join(
            "fixtures/private/goth-completed-stack-comparison-20261027-first/reentry-input-third.json"))?)?;
        let parts = import_parts(input)?;
        ensure!(
            parts.objects.len() == 344 && parts.entries.len() == 344,
            "composition owner coverage differs"
        );
        let mut threads = BTreeMap::new();
        for e in parts.entries {
            let id = e.object.context("retained object association missing")?;
            ensure!(
                e.adapter_thread == id as u64
                    && e.source_owner == format!("{}/native-object-{id}", parts.source_epoch),
                "new explicit adapter identity differs"
            );
            let thread = e
                .stack
                .into_thread(&parts.program, e.controls)
                .map_err(anyhow::Error::msg)?;
            threads.insert(id, thread);
        }
        ensure!(
            threads.keys().eq(parts.objects.keys())
                && parts.report["normalized_thread_sha256"]
                    == sims_fixtures::digest(&serde_json::to_vec(&threads)?),
            "composition handoff changed retained Threads"
        );
        let view = World {
            objects: parts.objects,
            ..World::default()
        };
        parts
            .registry
            .require_complete_backing(&view)
            .map_err(anyhow::Error::msg)?;
        Ok(())
    }
    fn observed(address: u32, b: &[u8]) -> Value {
        json!({"address":address,"bytes":b.len(),"raw":b.iter().map(|v|format!("{v:02x}")).collect::<String>(),"sha256":sims_fixtures::digest(b)})
    }
    #[test]
    fn captured_vector_keeps_unused_capacity_and_rejects_header_aliases() {
        let header: [u8; 12] = [0x10000u32, 0x10008, 0x10010]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let body = [0xff, 0xff, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8];
        let v = json!({"header":observed(0x900018,&header),"begin":0x10000,"end":0x10008,"capacity":0x10010,"allocated":observed(0x10000,&body)});
        let before = v.clone();
        let (_, used, spare) = native_vector(&v).unwrap();
        assert_eq!(used, &body[..8]);
        assert_eq!(spare, &body[8..]);
        assert_eq!(v, before);
        for kind in 0..4 {
            let mut bad = v.clone();
            match kind {
                0 => bad["end"] = json!(0x10004),
                1 => bad["allocated"]["bytes"] = json!(8),
                2 => bad["allocated"]["sha256"] = json!("0".repeat(64)),
                _ => bad["allocated"]["address"] = json!(0x10004),
            };
            assert!(native_vector(&bad).is_err());
        }
    }
    #[test]
    #[ignore = "requires local immutable completed-Goth component input; no native execution"]
    fn captured_reentry_rejects_mutated_closure_backing_and_ownership() {
        let p=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-completed-stack-comparison-20261027-first/reentry-input-third.json");
        let v: Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
        assert_eq!(
            evaluate(serde_json::from_value(v.clone()).unwrap()).unwrap()["threads"],
            344
        );
        let mut uses = v.clone();
        let person = uses["objects"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|o| o["native"]["object"] == 16)
            .unwrap();
        person["legacy"]["person"]["uses"] = json!([[1, 1, 0]]);
        let error = import_parts(uses).err().unwrap().to_string();
        assert!(
            error.contains("nonempty native action-use vector"),
            "action-use control failed elsewhere: {error}"
        );
        for kind in 0..6 {
            let mut bad = v.clone();
            match kind {
                0 => bad["source_closure"]["native_globals"]["active_menu_descriptor"] = json!(1),
                1 => {
                    let words = bad["registry"]["words"].as_array_mut().unwrap();
                    words[0] = json!(0x10000);
                }
                2 => {
                    bad["controls"][0]["controls_without_status"]["callback_result"] = json!(false)
                }
                3 => {
                    bad["controls"][0]["adapter"]["construction_absent"]["source"] =
                        json!("unsupported-fresh-queue-absence")
                }
                4 => {
                    bad["controls"].as_array_mut().unwrap().pop();
                }
                _ => bad["function_caches"][0]["used"]["sha256"] = json!("0".repeat(64)),
            }
            assert!(
                evaluate(serde_json::from_value(bad).unwrap()).is_err(),
                "mutation {kind}"
            );
        }
    }
}
