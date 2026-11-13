//! Current route controls and suspended native walks; no search/start/poll work.
//! Supplemental source authenticity is the caller's responsibility. All native
//! portal columns remain in `source`, including columns outside active people.
use super::{
    completed_route_controls::Pending,
    completed_routes::{Person, Suspension},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sims_fixtures::readset::Readset;
use sims_sim::{
    Body, Point, contention,
    lot_route::{
        animator::{RetainedOrigin, Walk},
        portal::Control,
    },
    portals,
};
use sims_vm::{ObjectId, Status, Vm};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    pub controls: BTreeMap<u64, contention::State>,
    pub caches: BTreeMap<ObjectId, portals::Cache>,
    pub walks: BTreeMap<ObjectId, Walk>,
    pub source: Value,
}
pub struct Input<'a, 'm> {
    pub state: &'a Value,
    pub source: &'a Value,
    pub people: &'a BTreeMap<ObjectId, Person>,
    pub pending: &'a BTreeMap<ObjectId, Pending>,
    pub memory: &'a Readset<'m>,
    pub vm: &'a Vm,
    pub bodies: &'a BTreeMap<ObjectId, Body>,
    pub epoch: &'a str,
}
fn n(v: &Value) -> Result<u32> {
    Ok(v.as_u64().context("route native word absent")?.try_into()?)
}
fn id(v: &Value) -> Result<ObjectId> {
    Ok(v.as_i64().context("route object ID absent")?.try_into()?)
}
fn word(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn short(b: &[u8], at: usize) -> i16 {
    i16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn raw(v: &Value) -> Result<Vec<u8>> {
    super::completed_animations::bytes(v.as_str().context("route raw bytes absent")?)
}

pub fn import(input: Input<'_, '_>) -> Result<Imported> {
    let Input {
        state,
        source,
        people,
        pending,
        memory,
        vm,
        bodies,
        epoch,
    } = input;
    ensure!(
        source["schema"] == 1
            && source["contract"] == "completed-native-route-execution-v1"
            && source["tick"] == vm.tick
            && source["tick"] == state["tick"]
            && source["source_readset_sha256"] == memory.source_sha256()
            && source["source_observation_sha256"]
                == "5ec18620f4412251ffe0f78a29b68c64aff0f609893a998a67d2ba455a359299"
            && source["physical_compressed_sha256"]
                == state["physical_baseline"]["archive"]["compressed_sha256"]
            && source["physical_decoded_sha256"]
                == state["physical_baseline"]["archive"]["decoded_sha256"]
            && source["original_guest_executed"] == false
            && !epoch.trim().is_empty()
            && epoch.len() <= 4096
            && !epoch.chars().any(char::is_control),
        "retained route execution source/adapter epoch differs"
    );
    let expected = [
        (
            "route_handler",
            0x45ad63,
            0xcc5,
            "436802f4beca3f1ed2a6deb122c9f776314a5d1fc4fb23a5be535463020b750e",
        ),
        (
            "context_constructor",
            0x48163e,
            0x192,
            "63b66e828b4dd99a3b9f33a46a5c42d985083543184a8d2f3fea250dc1176739",
        ),
        (
            "portal_costs",
            0x4638b9,
            0x695,
            "6eb6a563966729686c6e955ff2e34ac5448a4a038590b9db54043fa7efd1a3b8",
        ),
        (
            "route_context_copy_pop",
            0x45f8aa,
            0x75,
            "1447765a6a37ed16e12cff2ab53f47a495c0411ad660a553194dd04379a49ccf",
        ),
    ];
    let code = source["code"]
        .as_array()
        .context("retained route code absent")?;
    ensure!(
        code.len() == expected.len(),
        "retained route code coverage differs"
    );
    let mut seen = BTreeSet::new();
    for c in code {
        let name = c["name"]
            .as_str()
            .context("retained route code name absent")?;
        let e = expected
            .iter()
            .find(|e| e.0 == name)
            .context("unknown retained route code")?;
        let b = raw(&c["raw"])?;
        ensure!(
            seen.insert(name)
                && n(&c["address"])? == e.1
                && b.len() == e.2
                && c["bytes"] == b.len()
                && c["sha256"] == e.3
                && sims_fixtures::digest(&b) == e.3,
            "retained route code witness differs"
        );
    }
    for r in source["reads"]
        .as_array()
        .context("retained route reads absent")?
    {
        let b = raw(&r["raw"])?;
        ensure!(
            r["bytes"] == b.len() && r["sha256"] == sims_fixtures::digest(&b),
            "retained route receipt differs"
        );
        super::completed_skin::captured::overlap(memory, n(&r["address"])?, &b)?;
    }
    ensure!(
        people.keys().eq(pending.keys()),
        "retained route person/request coverage differs"
    );
    let pointers: BTreeMap<_, _> = state["threads"]["objects"]
        .as_array()
        .context("route registry absent")?
        .iter()
        .map(|o| Ok((n(&o["pointer"])?, id(&o["id"])?)))
        .collect::<Result<_>>()?;
    let object = |p: u32| -> Result<Option<ObjectId>> {
        if p == 0 {
            Ok(None)
        } else {
            Ok(Some(
                *pointers.get(&p).context("route pointer outside registry")?,
            ))
        }
    };
    let mut controls = BTreeMap::new();
    let mut walks = BTreeMap::new();
    for (&actor, p) in people {
        ensure!(
            p.readset_sha256 == memory.source_sha256(),
            "route person source differs"
        );
        let tid = *vm
            .object_threads
            .get(&actor)
            .context("route thread absent")?;
        for c in &p.contexts {
            let b = &c.raw;
            ensure!(
                b.len() == 0x9c
                    && memory.read(c.pointer, b.len())? == *b
                    && word(b, 0x80) == 0
                    && b[0x84] == 0
                    && b[0x78] <= 1,
                "retained route needs a separately qualified active shoo/link profile"
            );
            let request = pending[&actor]
                .routes
                .get(&c.frame_depth)
                .context("route request depth absent")?;
            let frame = vm.threads[&tid]
                .stack
                .get(c.frame_depth - 1)
                .context("route VM frame absent")?;
            ensure!(
                frame.location == request.origin && frame.stack_object == c.target,
                "retained route request/frame differs"
            );
            let crossing = matches!(c.suspension, Suspension::Child { .. });
            ensure!(
                frame.continuation == if crossing { 2 } else { 3 },
                "retained route frame phase differs"
            );
            let state = contention::State {
                thread: tid,
                caller: actor,
                // Contention is inactive in native frame states 2/3. Retry is
                // the replacement's inactive phase, not a synthesized native
                // continuation 6; all persisted native controller cells stay.
                controller: contention::Controller {
                    phase: contention::Phase::Retry,
                    stage: (word(b, 0x58) as i32).try_into()?,
                    previous: Point::new(word(b, 0x64) as i32, word(b, 0x60) as i32),
                    started: word(b, 0x5c),
                    result: (word(b, 0x88) as i32).try_into()?,
                },
                link: None,
                blocker_success: false,
                callback: None,
                deciding: false,
                shoo_result: None,
                last_portal: object(word(b, 0x68))?,
                original: Some(Control {
                    cache_valid: b[0x78] != 0,
                    ignored: short(b, 0x94),
                    crossing,
                    blocked_object: short(b, 0x8c),
                    seated: None,
                    standing: None,
                }),
            };
            ensure!(
                controls.insert(request.id, state).is_none(),
                "duplicate retained route execution"
            );
            if !crossing {
                ensure!(
                    matches!(&vm.threads[&tid].status,Status::AwaitingRoute(r)if r==request),
                    "retained walk is not the current route leaf"
                );
                ensure!(
                    walks
                        .insert(
                            actor,
                            Walk {
                                thread: tid,
                                request: request.id,
                                target: request.target,
                                target_body: bodies
                                    .get(&request.target)
                                    .context("retained walk target body absent")?
                                    .clone(),
                                started_tick: vm.tick,
                                yielded_tick: vm.tick,
                                retained_origin: Some(RetainedOrigin {
                                    completed_tick: vm.tick,
                                    source_readset_sha256: memory.source_sha256().into(),
                                    adapter_epoch: epoch.into()
                                }),
                                poll: None,
                                physical: None,
                                stopping: None
                            }
                        )
                        .is_none(),
                    "multiple walking leaves per actor"
                );
            }
        }
    }
    let mut columns = BTreeMap::new();
    for a in source["actors"]
        .as_array()
        .context("retained portal actors absent")?
    {
        let actor = id(&a["object"])?;
        let p = n(&a["pointer"])?;
        let b = raw(&a["raw"])?;
        ensure!(
            people.contains_key(&actor)
                && object(p)? == Some(actor)
                && b.len() == 0x188
                && memory.read(p, b.len())? == b
                && a["ordinal"] == short(&b, 0x56)
                && vm.world.objects[&actor].data.get(&6) == Some(&short(&b, 0x56)),
            "portal actor ordinal differs"
        );
        ensure!(
            columns.insert(actor, short(&b, 0x56)).is_none(),
            "duplicate portal actor"
        );
    }
    ensure!(
        columns.keys().eq(people.keys()),
        "portal actor coverage differs"
    );
    let mut caches: BTreeMap<_, _> = columns
        .iter()
        .filter(|(_, n)| **n > 0)
        .map(|(&id, _)| {
            (
                id,
                portals::Cache {
                    costs: BTreeMap::new(),
                },
            )
        })
        .collect();
    let portal_rows = source["portals"]
        .as_array()
        .context("retained portals absent")?;
    let mut portal_ids = BTreeSet::new();
    let ordered = raw(&source["portal_vector"]["used_raw"])?;
    ensure!(
        ordered.len() == portal_rows.len() * 4,
        "portal vector coverage differs"
    );
    for (r, ptr) in portal_rows.iter().zip(ordered.chunks_exact(4)) {
        let portal = id(&r["object"])?;
        let p = n(&r["pointer"])?;
        let b = raw(&r["raw"])?;
        ensure!(
            word(ptr, 0) == p
                && object(p)? == Some(portal)
                && portal_ids.insert(portal)
                && memory.read(p, 0x188)? == b,
            "portal identity/order differs"
        );
        let c = &r["costs"];
        let h = raw(&c["raw"])?;
        let data = raw(&c["used_raw"])?;
        ensure!(
            n(&c["address"])? == p.checked_add(0x1ac).context("portal address overflow")?
                && h.len() == 12
                && c["begin"] == word(&h, 0)
                && c["end"] == word(&h, 4)
                && c["capacity"] == word(&h, 8)
                && word(&h, 0) <= word(&h, 4)
                && word(&h, 4) <= word(&h, 8)
                && word(&h, 4) - word(&h, 0) == data.len() as u32
                && data.len() % 4 == 0,
            "portal cost extent differs"
        );
        for (&actor, &ordinal) in &columns {
            if ordinal > 0 {
                let i = (ordinal as usize - 1) * 4;
                // Native unallocated cells read as UNKNOWN through the checked
                // vector accessor. Existing cells retain exact float bits.
                if i < data.len() {
                    let value = f32::from_bits(word(&data, i));
                    ensure!(value.is_finite(), "nonfinite native portal cost");
                    caches.get_mut(&actor).unwrap().costs.insert(portal, value);
                }
            }
        }
    }
    Ok(Imported {
        controls,
        caches,
        walks,
        source: source.clone(),
    })
}
