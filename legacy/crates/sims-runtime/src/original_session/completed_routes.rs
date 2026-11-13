//! Retained native RouteTo contexts and shared movement controller at completed
//! boundaries. Never prepare/restart a route, allocate a request or service a
//! child. Pending Thread/service/route-execution ownership is a separate import.
//! Original 45ad63 pushes contexts, 45c349 selects the last, and 45f8e8 pops.
use super::{completed_actions::Identity, completed_invocations};
use anyhow::{Context, Result, ensure};
use sims_fixtures::readset::Readset;
use sims_sim::{
    pose::movement::{self, path},
    routing::Candidate,
};
use sims_vm::{ObjectId, routing::Descriptor};
use std::collections::BTreeMap;

fn word(raw: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(raw[off..off + 4].try_into().unwrap())
}
fn short(raw: &[u8], off: usize) -> i16 {
    i16::from_le_bytes(raw[off..off + 2].try_into().unwrap())
}
fn at(pointer: u32, offset: u32) -> Result<u32> {
    pointer
        .checked_add(offset)
        .context("native route address overflow")
}
fn floats<const N: usize>(raw: &[u8], off: usize) -> Result<[f32; N]> {
    let values = std::array::from_fn(|i| f32::from_bits(word(raw, off + 4 * i)));
    ensure!(
        values.iter().all(|v| v.is_finite()),
        "nonfinite retained native movement field"
    );
    Ok(values)
}
fn vector(m: &Readset<'_>, pointer: u32, stride: usize, maximum: usize) -> Result<(u32, Vec<u8>)> {
    let h = m.read(pointer, 12)?;
    let (begin, end, capacity) = (word(&h, 0), word(&h, 4), word(&h, 8));
    ensure!(
        begin <= end
            && end <= capacity
            && (begin != 0 || capacity == 0)
            && (end - begin) as usize % stride == 0
            && (capacity - begin) as usize % stride == 0
            && (end - begin) as usize / stride <= maximum,
        "retained native route vector extent differs"
    );
    Ok((begin, m.read(begin, (end - begin) as usize)?))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Suspension {
    /// Native state2 waits for this already entered target tree. This is an
    /// observed association, not a constructed lifecycle::ServiceCall.
    Child {
        depth: usize,
        callee: ObjectId,
        tree: u16,
    },
    Walking,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ContextOwner {
    pub pointer: u32,
    pub raw: Vec<u8>,
    pub frame_depth: usize,
    pub frame_pointer: u32,
    pub target: ObjectId,
    pub descriptor: Descriptor,
    pub candidates: Vec<Candidate>,
    pub candidates_raw: Vec<u8>,
    pub candidate_index: i32,
    pub maximum: i32,
    pub result: u32,
    pub suspension: Suspension,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub readset_sha256: String,
    pub contexts: Vec<ContextOwner>,
    pub context_vector_raw: Vec<u8>,
    pub points: Vec<[i32; 2]>,
    pub path_raw: Vec<u8>,
    pub animator_pointer: u32,
    pub animator_raw: Vec<u8>,
    pub movement: movement::Movement,
}

pub(super) fn movement(
    m: &Readset<'_>,
    person: u32,
    ar: &[u8],
    points: &[[i32; 2]],
) -> Result<movement::Movement> {
    let route_pointer = word(ar, 0x184);
    let path = if route_pointer == 0 {
        None
    } else {
        ensure!(
            route_pointer == at(person, 0xae0)?,
            "native animator/person route vector alias differs"
        );
        let tuning = m.read(0x6d07a8, 0x38)?;
        Some(path::Path {
            points: points.to_vec(),
            settings: path::Settings {
                sample_step: floats::<1>(&tuning, 0)?[0],
                turn_threshold: floats::<1>(&tuning, 12)?[0],
                turn_step: floats::<1>(&tuning, 16)?[0],
                minimum_brake_time: word(&tuning, 0x2c) as i32,
                endpoint_distance: floats(&tuning, 0x30)?,
            },
            index: word(ar, 0x180) as i32,
            progress: floats::<1>(ar, 0x1ec)?[0],
            deadline: word(ar, 0x1c0) as i32,
            phase_window: [floats::<1>(ar, 0x174)?[0], floats::<1>(ar, 0x170)?[0]],
            stop_distance: floats(ar, 0x178)?,
            start_distance: [floats::<1>(ar, 0x1f8)?[0], floats::<1>(ar, 0x204)?[0]],
            lookahead: [floats::<1>(ar, 0x200)?[0], floats::<1>(ar, 0x1fc)?[0]],
            interrupt: word(ar, 0x284) as i32,
            guard: word(ar, 0x28c) as i32,
            delta: floats(ar, 0x294)?,
            stop_mode: word(ar, 0x18c) as i32,
        })
    };
    let state = movement::State {
        status: word(ar, 0x188) as i32,
        flags: [word(ar, 0x19c) as i32, word(ar, 0x290) as i32],
        start: floats(ar, 0x1d0)?,
        end: floats(ar, 0x1dc)?,
        turn_delta: floats::<1>(ar, 0x1f0)?[0],
        nominal_turn: floats::<1>(ar, 0x1f4)?[0],
        reason: word(ar, 0x288) as i32,
        apply_rate: ar[0x29c],
    };
    // All fields are explicit. No native synchronous animator C stack is live
    // at the separately qualified completed boundary; no Rust pass was started.
    let fields = serde_json::json!({"state":state,"path":path,"scalar_7c":floats::<1>(ar,0x7c)?[0],"pass":null});
    let result: movement::Movement = serde_json::from_value(fields.clone())?;
    ensure!(
        serde_json::to_value(&result)? == fields,
        "native movement fields defaulted or discarded"
    );
    Ok(result)
}

pub fn import(
    objects: &[Identity],
    invocations: &completed_invocations::Imported,
    memory: &Readset<'_>,
) -> Result<BTreeMap<ObjectId, Person>> {
    ensure!(
        invocations.readset_sha256 == memory.source_sha256(),
        "native route/stack observed memory epoch differs"
    );
    for (address, count, hash) in [
        (
            0x45adef,
            30,
            "f7a25ae2cef5bb9275dcb108f778966b362209d9ed60a97b3dee751da245a7bc",
        ),
        (
            0x45f8e8,
            55,
            "0b3359de2bfa75453c13a944f90c23b50dca1735f99099a2db5b5dbd88b20eea",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&memory.read(address, count)?) == hash,
            "native route context witness differs"
        );
    }
    let ids: BTreeMap<_, _> = objects.iter().map(|o| (o.pointer, o.object)).collect();
    ensure!(
        ids.len() == objects.len() && invocations.objects.len() == objects.len(),
        "native route inventory differs"
    );
    let mut result = BTreeMap::new();
    for o in objects.iter().filter(|o| o.person) {
        let stack = invocations
            .objects
            .get(&o.object)
            .context("native route person stack owner absent")?;
        let frames = stack.stack.as_ref().map_or(&[][..], |s| s.frames());
        let mut routes = Vec::new();
        for (i, f) in frames.iter().enumerate() {
            let (_, routine) = invocations
                .program
                .resolve(&f.code_owner, f.location.bhav)
                .map_err(anyhow::Error::msg)?;
            let opcode = routine.body.instructions[f.location.instruction].opcode;
            if matches!(opcode, 27 | 45) && f.continuation != 0 {
                routes.push((i, f));
            }
        }
        let (_, path_raw) = vector(memory, at(o.pointer, 0xae0)?, 8, 16384)?;
        let points = path_raw
            .chunks_exact(8)
            .map(|b| [word(b, 0) as i32, word(b, 4) as i32])
            .collect::<Vec<_>>();
        let (begin, context_vector_raw) = vector(memory, at(o.pointer, 0xaf0)?, 0x9c, 64)?;
        ensure!(
            routes.len() == context_vector_raw.len() / 0x9c,
            "native RouteTo frame/context coverage differs"
        );
        let animator_pointer = word(&memory.read(at(o.pointer, 0xadc)?, 4)?, 0);
        let animator_raw = memory.read(animator_pointer, 0x2a0)?;
        ensure!(
            word(&animator_raw, 0) == 0x671ee4 && word(&animator_raw, 0x138) == o.pointer,
            "native route animator/person ownership differs"
        );
        let movement = movement(memory, o.pointer, &animator_raw, &points)?;
        let mut contexts = Vec::new();
        for (ordinal, ((index, frame), raw)) in routes
            .into_iter()
            .zip(context_vector_raw.chunks_exact(0x9c))
            .enumerate()
        {
            let pointer = at(begin, (ordinal * 0x9c) as u32)?;
            ensure!(
                word(raw, 0x4c) == o.pointer,
                "native route context/person reverse owner differs"
            );
            let target = *ids
                .get(&word(raw, 0x48))
                .context("native route target not registered")?;
            ensure!(
                target == frame.stack_object,
                "native route target differs from retained frame stack object"
            );
            let descriptor = Descriptor(std::array::from_fn(|i| word(raw, 0xc + 4 * i)));
            descriptor.validate().map_err(anyhow::Error::msg)?;
            let (_, candidates_raw) = vector(memory, pointer, 16, 16384)?;
            let candidates = candidates_raw
                .chunks_exact(16)
                .map(|c| Candidate {
                    position: [word(c, 0) as i32, word(c, 4) as i32],
                    score: word(c, 8) as i32,
                    seat: short(c, 12),
                    flags: short(c, 14) as u16,
                })
                .collect();
            let suspension = match frame.continuation {
                2 => {
                    let child = frames
                        .get(index + 1)
                        .context("native state2 has no entered child")?;
                    let selected = *ids
                        .get(&word(raw, 0x68))
                        .context("native state2 child target not registered")?;
                    ensure!(
                        child.action_target == Some(selected) && child.callee == selected,
                        "native state2 child/use/context ownership differs"
                    );
                    Suspension::Child {
                        depth: index + 2,
                        callee: selected,
                        tree: child.location.bhav,
                    }
                }
                3 => {
                    ensure!(
                        index + 1 == frames.len()
                            && movement.path.as_ref().is_some_and(|p| p.points.len() >= 3),
                        "native walking continuation lacks leaf/path ownership"
                    );
                    Suspension::Walking
                }
                _ => anyhow::bail!(
                    "native route continuation requires a separate retained-control profile"
                ),
            };
            contexts.push(ContextOwner {
                pointer,
                raw: raw.to_vec(),
                frame_depth: index + 1,
                frame_pointer: stack.frame_pointers[index],
                target,
                descriptor,
                candidates,
                candidates_raw,
                candidate_index: word(raw, 0x50) as i32,
                maximum: word(raw, 0x54) as i32,
                result: word(raw, 0x88),
                suspension,
            });
        }
        result.insert(
            o.object,
            Person {
                readset_sha256: memory.source_sha256().into(),
                contexts,
                context_vector_raw,
                points,
                path_raw,
                animator_pointer,
                animator_raw,
                movement,
            },
        );
    }
    Ok(result)
}
