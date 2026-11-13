//! Constructorless completed VM assembly with explicit fresh host identities.
//! Source authenticity/whole Character and Session admission are caller duties.
use super::callbacks::Main;
use crate::original_session::{completed_invocations, completed_route_controls::Pending};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sims_fixtures::readset::Readset;
use sims_vm::{
    ObjectId, Program, Status, ThreadId, Vm, World,
    lifecycle::{Lifecycle, Phase},
    native_registry_reentry::{Absence, AdapterOwnership, Evidence, GroupMember, Leaf, Topology},
    registry_suspension::{Boundary, FrameShape, SourceBinding},
    retained_completed::{CompletedAdapterEpoch, CompletedThreadInput, ObjectThreadBinding},
    retained_frames::{RetainedStack, ThreadControls},
};
use std::collections::{BTreeMap, BTreeSet};

pub struct Input<'a, 'm> {
    pub state: &'a Value,
    pub caller: &'a Value,
    pub memory: &'a Readset<'m>,
    pub invocations: &'a completed_invocations::Imported,
    pub program: &'a Program,
    pub world: World,
    pub cached: &'a BTreeMap<ObjectId, Main>,
    pub stacks: BTreeMap<ObjectId, RetainedStack>,
    pub pending: &'a BTreeMap<ObjectId, Pending>,
    pub thread_ids: BTreeMap<ObjectId, ThreadId>,
    pub epoch: CompletedAdapterEpoch,
}
fn number(v: &Value) -> Result<u32> {
    Ok(v.as_u64()
        .context("retained VM native word absent")?
        .try_into()?)
}
fn word(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn source_key(epoch: &str, id: ObjectId) -> String {
    format!("{epoch}/native-object-{id}")
}

pub fn import(input: Input<'_, '_>) -> Result<Vm> {
    let Input {
        state,
        caller,
        memory,
        invocations,
        program,
        world,
        cached,
        stacks,
        pending,
        thread_ids,
        epoch,
    } = input;
    ensure!(
        caller["contract"] == "sims191/completed-caller-registry-controls-v2"
            && caller["tick"] == state["tick"]
            && caller["source_readset_sha256"] == memory.source_sha256()
            && invocations.readset_sha256 == memory.source_sha256()
            && caller["physical_compressed_sha256"]
                == state["physical_baseline"]["archive"]["compressed_sha256"],
        "retained VM caller/source epoch differs"
    );
    let ids: Vec<_> = world.objects.keys().copied().collect();
    ensure!(
        stacks.keys().eq(world.objects.keys())
            && cached.keys().eq(world.objects.keys())
            && thread_ids.keys().eq(world.objects.keys())
            && serde_json::to_value(&ids)? == caller["construction_absent_objects"]
            && serde_json::to_value(&ids)? == caller["native_consumer_absent_objects"],
        "retained VM owner/absence coverage differs"
    );
    let boundary: Boundary = serde_json::from_value(caller["boundary"].clone())?;
    ensure!(
        boundary.tick == state["tick"].as_u64().context("retained VM tick absent")?
            && caller["native_globals"]["active_menu_descriptor"] == 0,
        "retained VM completed caller/menu differs"
    );
    let raw_capture_sha256 = sims_fixtures::digest(&serde_json::to_vec(&state["threads"])?);
    let closure = format!(
        "native-completed-caller:{}",
        sims_fixtures::digest(&serde_json::to_vec(caller)?)
    );
    let dispatch: BTreeMap<ObjectId, _> = caller["dispatches"]
        .as_array()
        .context("retained VM dispatches absent")?
        .iter()
        .map(|r| {
            Ok((
                r["object"]
                    .as_i64()
                    .context("retained VM dispatch object absent")?
                    .try_into()?,
                r,
            ))
        })
        .collect::<Result<_>>()?;
    ensure!(
        dispatch.len() == ids.len(),
        "retained VM dispatch coverage differs"
    );
    let raw_objects: BTreeMap<ObjectId, _> = state["threads"]["objects"]
        .as_array()
        .context("retained VM raw objects absent")?
        .iter()
        .map(|r| {
            Ok((
                r["id"]
                    .as_i64()
                    .context("retained VM raw ID absent")?
                    .try_into()?,
                r,
            ))
        })
        .collect::<Result<_>>()?;
    let mut entries = Vec::new();
    let mut bindings = Vec::new();
    let mut consumed = BTreeSet::new();
    for (id, stack) in stacks {
        let tid = thread_ids[&id];
        let main = &cached[&id];
        let raw = raw_objects
            .get(&id)
            .context("retained VM raw object missing")?;
        let pointer = number(&raw["pointer"])?;
        let h = memory.read(pointer, 0x2c)?;
        ensure!(
            h[0x20] <= 1
                && raw["last_result_raw"] == h[0x20]
                && i16::from_le_bytes(h[0x28..0x2a].try_into().unwrap()) == 0,
            "retained VM last result/native error differs"
        );
        let mut service = None;
        let status = if let Some(p) = pending.get(&id) {
            consumed.insert(id);
            service = p.service.clone();
            p.leaf.clone()
        } else {
            let leaf = stack
                .frames()
                .last()
                .context("retained VM Sleep leaf absent")?;
            let (_, routine) = program
                .resolve(&leaf.code_owner, leaf.location.bhav)
                .map_err(anyhow::Error::msg)?;
            let ins = routine
                .body
                .instructions
                .get(leaf.location.instruction)
                .context("retained VM Sleep instruction absent")?;
            let arg = usize::try_from(i16::from_le_bytes([ins.operands[0], ins.operands[1]]))?;
            ensure!(
                ins.opcode == 0 && world.objects[&id].person.is_none(),
                "retained VM non-Sleep needs explicit pending owner"
            );
            let binding = &invocations.objects[&id].binding;
            let object_raw = memory.read(pointer, 0x188)?;
            let topology = if world.objects[&id].multitile.is_some() {
                let members = sims_vm::multitile::chain(&world, id)
                    .map_err(anyhow::Error::msg)?
                    .into_iter()
                    .map(|member| {
                        Ok(GroupMember {
                            object: member,
                            registry_lifetime: world
                                .scheduler_storage
                                .as_ref()
                                .unwrap()
                                .handle(member)
                                .context("retained group lifetime absent")?,
                            definition: world.objects[&member]
                                .definition
                                .clone()
                                .context("retained group definition absent")?,
                            part: world.objects[&member]
                                .multitile
                                .clone()
                                .context("retained group part absent")?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Topology::LiveGroup {
                    source: format!("{closure}/retained-group"),
                    members,
                }
            } else {
                Topology::NonGroup {
                    source: format!("{closure}/non-group"),
                }
            };
            let e = Evidence {
                schema: 1,
                boundary: boundary.clone(),
                object: id,
                native_pointer: pointer,
                source_binding: SourceBinding {
                    resource: word(&object_raw, 0x11c),
                    context: binding.native_context,
                    definition: word(&object_raw, 0x118),
                    guid: binding.definition.guid,
                    objd: binding.definition.object,
                },
                cached_init: main.init,
                cached_main: main.main,
                frame_shapes: stack
                    .frames()
                    .iter()
                    .map(|f| FrameShape {
                        offset_words: f.storage_offset,
                        tree: f.location.bhav,
                        node: f.location.instruction,
                        context: invocations.objects[&f.callee].binding.native_context,
                        argc: f.args.len(),
                        locals: f.locals.len(),
                    })
                    .collect(),
                registry_word: world.objects[&id]
                    .idle
                    .as_ref()
                    .context("retained VM idle owner absent")?
                    .registry_word(),
                tick_dispatch: serde_json::from_value({
                    let mut v = dispatch
                        .get(&id)
                        .context("retained Tick dispatch missing")?
                        .as_object()
                        .unwrap()
                        .clone();
                    v.remove("object");
                    Value::Object(v)
                })?,
                leaf: Leaf::BaseTickSleep {
                    argument_index: arg,
                    argument_value: *leaf
                        .args
                        .get(arg)
                        .context("retained Sleep argument absent")?,
                },
                reference_image_sha256: state["threads"]["reference_image_sha256"]
                    .as_str()
                    .context("retained VM image absent")?
                    .into(),
                source_witness_sha256: caller["source_witness_sha256"]
                    .as_str()
                    .context("retained VM code witness absent")?
                    .into(),
                raw_capture_sha256: raw_capture_sha256.clone(),
                adapter_epoch_source: epoch.fresh.source_key.clone(),
            };
            let adapter = AdapterOwnership {
                fresh: epoch.fresh.clone(),
                construction_absent: Absence {
                    objects: ids.clone(),
                    source: format!("{closure}/construction"),
                },
                native_consumer_absent: Absence {
                    objects: ids.clone(),
                    source: format!("{closure}/consumers"),
                },
                topology,
            };
            Status::NativeRegistryReentry(
                stack
                    .native_registry_reentry(program, &world, boundary.tick, e, adapter)
                    .map_err(anyhow::Error::msg)
                    .with_context(|| format!("retained native Sleep object {id}"))?,
            )
        };
        let controls = ThreadControls {
            service,
            dispatch: None,
            autonomy: None,
            find_best: Vec::new(),
            completed_actions: Vec::new(),
            last_result: h[0x20] != 0,
            callback: None,
            callback_result: None,
            check: false,
            interrupted: false,
            status,
        };
        entries.push(CompletedThreadInput {
            source_owner: source_key(&epoch.fresh.source_key, id),
            adapter_thread: tid,
            object: Some(id),
            stack,
            controls,
        });
        bindings.push(ObjectThreadBinding {
            object: id,
            thread: Some(tid),
            lifecycle: Some(Lifecycle {
                owner: main.owner.clone(),
                definition: main.definition,
                init: main.init,
                main: main.main,
                phase: Phase::Main,
                thread: tid,
                run_init: world.objects[&id].multitile.is_some(),
            }),
        });
    }
    ensure!(
        consumed.iter().eq(pending.keys()),
        "unclaimed retained pending thread owner"
    );
    let projection = serde_json::from_value(state["domains"]["systems"]["projection"].clone())?;
    Vm::from_retained_completed_epoch(program, world, projection, entries, bindings, epoch)
        .map_err(anyhow::Error::msg)
}
