//! Compare exposed native route contexts with existing Rust suspension owners.
//! This does not resolve missing resource catalogs or manufacture source closure.
use super::*;
use sims_fixtures::readset::Readset;
use sims_vm::{ObjectId, RouteRequest, Status};

fn u(v: &Value) -> Result<u32> {
    Ok(v.as_u64().context("route word absent")?.try_into()?)
}
fn word(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn short(b: &[u8], at: usize) -> i16 {
    i16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    crate::original_session::completed_animations::bytes(v.as_str().context("route bytes absent")?)
}
fn vector(v: &Value, m: &Readset<'_>, stride: usize) -> Result<(u32, usize)> {
    let b = bytes(&v["raw"])?;
    let p = u(&v["address"])?;
    ensure!(
        b.len() == 12 && m.read(p, 12)? == b,
        "route vector header differs"
    );
    let (begin, end, cap) = (word(&b, 0), word(&b, 4), word(&b, 8));
    ensure!(
        begin <= end
            && end <= cap
            && (begin != 0 || cap == 0)
            && (end - begin) as usize % stride == 0
            && (cap - begin) as usize % stride == 0
            && (end - begin) as usize / stride <= 16384
            && v["begin"] == begin
            && v["end"] == end
            && v["capacity"] == cap,
        "route vector extent differs"
    );
    Ok((begin, (end - begin) as usize))
}
pub fn compare(state: &Value, m: &Readset<'_>, sim: &sims_sim::Simulation) -> Result<Value> {
    ensure!(
        state["tick"] == sim.vm.tick && state["readset"]["sha256"] == m.source_sha256(),
        "route comparison source epoch differs"
    );
    let source = &state["domains"]["person_routes"];
    crate::original_session::completed_skeletons::captured::reads(source, m)?;
    let rows = state["threads"]["objects"]
        .as_array()
        .context("native thread rows absent")?;
    let mut objects = BTreeMap::new();
    let mut pointers = BTreeMap::new();
    for o in rows {
        let id: ObjectId = o["id"]
            .as_i64()
            .context("native route object ID absent")?
            .try_into()?;
        ensure!(
            objects.insert(id, o).is_none() && pointers.insert(u(&o["pointer"])?, id).is_none(),
            "duplicate native route identity"
        );
    }
    ensure!(
        objects.keys().eq(sim.vm.world.objects.keys()),
        "native/Rust route object domains differ"
    );
    let identity = |p: u32| -> Result<Option<ObjectId>> {
        if p == 0 {
            Ok(None)
        } else {
            Ok(Some(
                *pointers.get(&p).context("route pointer outside registry")?,
            ))
        }
    };
    let mut checks = 0;
    let mut total = 0;
    let mut first = Vec::new();
    let mut controls = BTreeSet::new();
    let mut actors = BTreeSet::new();
    let mut candidates = Vec::new();
    let mut check = |path: String, a: Value, b: Value| {
        super::pose::differences(&path, &a, &b, &mut checks, &mut total, &mut first)
    };
    for a in source["actors"]
        .as_array()
        .context("native route actors absent")?
    {
        let actor: ObjectId = a["object"]
            .as_i64()
            .context("native route actor absent")?
            .try_into()?;
        ensure!(
            actors.insert(actor)
                && sim.characters.contains_key(&actor)
                && objects[&actor]["pointer"] == a["pointer"],
            "route actor coverage differs"
        );
        let native = objects[&actor];
        let tid = sim.vm.object_threads[&actor];
        let thread = &sim.vm.threads[&tid];
        ensure!(
            thread.callback.is_none(),
            "route callback comparison requires another profile"
        );
        let mut pending: BTreeMap<usize, &RouteRequest> = BTreeMap::new();
        if let Status::AwaitingRoute(r) = &thread.status {
            pending.insert(thread.stack.len(), r);
        }
        for service in thread.service.iter().flat_map(|s| s.chain()) {
            if let Status::AwaitingRoute(r) = &service.resume_status {
                ensure!(
                    pending.insert(service.base_depth, r).is_none(),
                    "duplicate suspended route depth"
                );
            }
        }
        let (begin, n) = vector(&a["context_vector"], m, 0x9c)?;
        let contexts = a["contexts"]
            .as_array()
            .context("native route contexts absent")?;
        ensure!(
            contexts.len() == n / 0x9c && contexts.len() == pending.len(),
            "native/Rust suspended route coverage differs"
        );
        for (ordinal, (c, (&depth, r))) in contexts.iter().zip(&pending).enumerate() {
            let raw = bytes(&c["raw"])?;
            let cp = u(&c["pointer"])?;
            ensure!(
                raw.len() == 0x9c
                    && cp == begin + ordinal as u32 * 0x9c
                    && m.read(cp, raw.len())? == raw
                    && word(&raw, 0x4c) == u(&a["pointer"])?
                    && word(&raw, 0x80) == 0
                    && raw[0x84] == 0
                    && raw[0x78] <= 1,
                "native route context identity/active-obstruction profile differs"
            );
            let frame = thread
                .stack
                .get(depth - 1)
                .context("Rust route frame absent")?;
            let nf = native["frames"]
                .as_array()
                .context("native route frames absent")?
                .get(depth - 1)
                .context("native route frame absent")?;
            let fb = bytes(&nf["storage"]["raw"])?;
            ensure!(
                m.read(u(&nf["pointer"])?, fb.len())? == fb,
                "native route frame source differs"
            );
            let continuation = u(&nf["continuation_word_raw"])?;
            ensure!(
                matches!(continuation, 2 | 3)
                    && frame.location == r.origin
                    && controls.insert(r.id),
                "route frame phase/request ownership differs"
            );
            let control = sim
                .route_control
                .get(&r.id)
                .context("Rust route control absent")?;
            let original = control
                .original
                .as_ref()
                .context("Rust native route control absent")?;
            let sims_vm::routing::Destination::Relative(descriptor) = &r.destination else {
                anyhow::bail!("non-relative route comparison requires another profile")
            };
            let target = identity(word(&raw, 0x48))?.context("native route target null")?;
            let owner = m.read(u(&nf["code_owner_context"])?, 24)?;
            ensure!(
                word(&owner, 0) == 0x666514 && word(&owner, 8) != 0,
                "native route resource context differs"
            );
            let expected = json!({"actor":actor,"target":target,"tree":nf["tree_raw"],"node":nf["node_raw"],
                "frame_target":nf["stack_object"],"continuation":continuation,"code_owner":format!("native-provider:{:08x}",word(&owner,8)),
                "descriptor":(0..15).map(|i|word(&raw,0xc+i*4)).collect::<Vec<_>>(),
                "stage":word(&raw,0x58)as i32,"previous":[word(&raw,0x64)as i32,word(&raw,0x60)as i32],
                "started":word(&raw,0x5c),"result":word(&raw,0x88)as i32,"last_portal":identity(word(&raw,0x68))?,
                "cache_valid":raw[0x78]!=0,"ignored":short(&raw,0x94),"crossing":continuation==2,"blocked_object":short(&raw,0x8c)});
            let actual = json!({"actor":r.caller,"target":r.target,"tree":frame.location.bhav,"node":frame.location.instruction,
                "frame_target":frame.stack_object,"continuation":frame.continuation,"code_owner":frame.code_owner,
                "descriptor":descriptor.words(),"stage":control.controller.stage,"previous":[control.controller.previous.x,control.controller.previous.y],
                "started":control.controller.started,"result":control.controller.result,"last_portal":control.last_portal,
                "cache_valid":original.cache_valid,"ignored":original.ignored,"crossing":original.crossing,"blocked_object":original.blocked_object});
            check(format!("/routes/{actor}/{depth}"), expected, actual);
            ensure!(
                control.thread == tid
                    && control.caller == actor
                    && control.controller.phase == sims_sim::contention::Phase::Retry
                    && control.link.is_none()
                    && !control.blocker_success
                    && control.callback.is_none()
                    && !control.deciding
                    && control.shoo_result.is_none()
                    && original.seated.is_none()
                    && original.standing.is_none(),
                "Rust inactive route owner profile differs"
            );
            if continuation == 2 {
                let service = thread
                    .service
                    .iter()
                    .flat_map(|s| s.chain())
                    .find(|s| s.base_depth == depth)
                    .context("Rust portal suspension owner absent")?;
                let child = &native["frames"][depth];
                check(
                    format!("/routes/{actor}/{depth}/child"),
                    json!({"callee":identity(word(&raw,0x68))?,"tree":child["tree_raw"]}),
                    json!({"callee":service.callee,"tree":service.bhav}),
                );
            } else {
                let walk = sim.characters[&actor]
                    .native_walk
                    .as_ref()
                    .context("Rust native walk owner absent")?;
                ensure!(
                    walk.thread == tid
                        && walk.request == r.id
                        && walk.target == target
                        && walk.poll.is_none()
                        && walk.physical.is_none()
                        && walk.stopping.is_none(),
                    "Rust native walk suspension differs"
                );
            }
            let (cb, cn) = vector(&c["candidate_vector"], m, 16)?;
            let cr = bytes(&c["candidates_raw"])?;
            ensure!(
                cn == cr.len() && m.read(cb, cn)? == cr,
                "native route candidate source differs"
            );
            candidates.push(json!({"actor":actor,"depth":depth,"count":cn/16,"index":word(&raw,0x50)as i32,"maximum":word(&raw,0x54)as i32}));
        }
    }
    ensure!(
        actors.iter().eq(sim.characters.keys()) && controls.iter().eq(sim.route_control.keys()),
        "uncompared Rust route/actor owners"
    );
    Ok(
        json!({"schema":1,"kind":"observed_route_ownership_diagnostic","native_parity_credit":0,
        "checks":checks,"differences":total,"first_differences":first,"route_contexts":controls.len(),
        "uncompared_candidate_caches":candidates,
        "qualification":"Existing suspension, frame/context association, descriptor, controller and child/walk owners only. Native candidate vectors/index/maximum are observed but not yet retained by Rust. Does not establish complete route-state or missing resource/source closure."}),
    )
}
