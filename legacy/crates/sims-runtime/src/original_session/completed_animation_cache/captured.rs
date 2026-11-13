//! Current effective names, with separately proved private150 lookup misses.
//! This adds scoped Program views, not whole-file versions or native tick credit.
use super::*;
use sims_fixtures::readset::Readset;
use std::collections::BTreeSet;

const LOOKUP_CODE: &[(u32, usize, &str)] = &[
    (
        0x456840,
        213,
        "10c391281b9a7426c2e7d009c64ffad45ae0c8344a7de9c8c2c415520907cdf5",
    ),
    (
        0x457b84,
        36,
        "9ec68be2968173df7bc5c6bfe1a6e7b91c800f30240fe8bc3d632ff3f3ce38d4",
    ),
    (
        0x401039,
        267,
        "338d2d97fe85cf369d062f4d5752fc436d79d0e179a22e78ab8ad73f744eb72a",
    ),
    (
        0x4702f9,
        86,
        "2e9b8962ed2d403647be0f76cb6df1737d96cd8280704dafa7ef92f019e1acb0",
    ),
    (
        0x46feac,
        136,
        "6d9493341c03ef36dd8565a3b390770197a18ea186b8f006be41a22be227e4bf",
    ),
];

fn at(p: u32, n: u32) -> Result<u32> {
    p.checked_add(n).context("posture owner overflow")
}

/// Complete bucket traversal of the selected native map. All node counts and
/// links come from the frozen bytes; JSON's selected/complete flags grant no miss.
fn nodes(memory: &Memory, p: u32, inner: bool) -> Result<Vec<(u32, Vec<u8>)>> {
    let raw = memory.read(p, if inner { 20 } else { 24 })?;
    let (a, b, c) = (word(&raw, 4), word(&raw, 8), word(&raw, 12));
    ensure!(
        (a >= 0x10000 || a == 0 && b == 0 && c == 0)
            && a <= b
            && b <= c
            && (b - a) % 4 == 0
            && (c - a) % 4 == 0
            && (c - a) / 4 <= 65536,
        "posture resource bucket extent differs"
    );
    let heads = memory.read(a, (b - a) as usize)?;
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for head in heads.chunks_exact(4) {
        let mut p = word(head, 0);
        while p != 0 {
            ensure!(
                p >= 0x10000 && seen.len() < 65536 && seen.insert(p),
                "posture resource bucket cycle/shared node/budget"
            );
            let raw = memory.read(p, if inner { 100 } else { 28 })?;
            result.push((p, raw.clone()));
            p = word(&raw, 0);
        }
    }
    ensure!(
        result.len() == word(&raw, 16) as usize,
        "posture resource map count differs"
    );
    Ok(result)
}

fn private_miss(memory: &Memory, provider: u32) -> Result<Vec<u32>> {
    let raw = memory.read(provider, 172)?;
    ensure!(word(&raw, 0) == 0x6672b0, "posture aggregate class differs");
    let mut components = Vec::new();
    let mut seen = BTreeSet::new();
    for i in 0..8 {
        let component = word(&raw, 12 + i * 20);
        if component == 0 {
            break;
        }
        ensure!(seen.insert(component), "duplicate posture component");
        let raw = memory.read(component, 44)?;
        ensure!(word(&raw, 0) == 0x666944, "posture component class differs");
        let mut kinds = BTreeSet::new();
        for (p, raw) in nodes(memory, word(&raw, 24), false)? {
            let tag = word(&raw, 4);
            ensure!(kinds.insert(tag), "duplicate posture resource type");
            if tag != 0x53545223 && tag != 0x435354 {
                continue;
            }
            let mut keys = BTreeSet::new();
            for (_, raw) in nodes(memory, at(p, 8)?, true)? {
                let id = u16::from_le_bytes(raw[4..6].try_into()?);
                ensure!(keys.insert((id, raw[6])), "duplicate posture resource key");
                ensure!(
                    !(id == 150 && raw[6] == 0),
                    "private posture150 has a resource record; its load/cache semantics remain required"
                );
            }
        }
        components.push(component);
    }
    ensure!(!components.is_empty(), "posture provider components absent");
    Ok(components)
}

pub struct Joined {
    pub program: sims_vm::Program,
    pub vm: sims_vm::Snapshot,
    pub source: Value,
}

pub fn join(
    state: &Value,
    sparse: &Readset<'_>,
    source: &Value,
    observation_sha256: &str,
    mut program: sims_vm::Program,
    mut vm: sims_vm::Snapshot,
) -> Result<Joined> {
    ensure!(
        source["schema"] == 1
            && source["contract"] == "completed-native-private-posture-sources-v1"
            && source["tick"] == state["tick"]
            && state["tick"] == vm.state.tick
            && source["source_observation_sha256"] == observation_sha256
            && source["source_readset_sha256"] == sparse.source_sha256()
            && source["physical_compressed_sha256"]
                == state["physical_baseline"]["archive"]["compressed_sha256"]
            && source["physical_decoded_sha256"]
                == state["physical_baseline"]["archive"]["decoded_sha256"]
            && source["executes_target_code"] == false,
        "current posture source epoch differs"
    );
    vm.clone().restore(&program).map_err(anyhow::Error::msg)?;
    let before = serde_json::to_vec(&vm.state)?;
    let input = &state["animation_caches"];
    super::super::completed_skeletons::captured::reads(input, sparse)?;
    let global = GlobalTables::read(input)?;
    ensure!(
        global.tick == vm.state.tick && global.source == program.globals,
        "current effective animation Global owner differs"
    );
    let memory = Memory::new(source)?;
    for row in source["reads"]
        .as_array()
        .context("posture source receipts absent")?
    {
        super::super::completed_skin::captured::overlap(
            sparse,
            number(&row["address"])?,
            &bytes(&row["raw"])?,
        )?;
    }
    ensure!(
        source["code"]
            .as_array()
            .context("posture source code absent")?
            .len()
            == LOOKUP_CODE.len(),
        "posture source method coverage differs"
    );
    for &(p, n, hash) in LOOKUP_CODE {
        ensure!(
            sims_fixtures::digest(&memory.read(p, n)?) == hash,
            "posture lookup method differs"
        );
    }
    let caches = input["private_caches"]
        .as_array()
        .context("private posture caches absent")?;
    let rows = source["private"]
        .as_array()
        .context("private posture source owners absent")?;
    ensure!(
        rows.len() == caches.len(),
        "private posture source coverage differs"
    );
    let mut admitted = BTreeSet::new();
    let mut misses = Vec::new();
    let mut resources = BTreeSet::new();
    for cache in caches {
        let resource = number(&cache["resource"])?;
        ensure!(
            resources.insert(resource),
            "duplicate private posture owner"
        );
        let matches: Vec<_> = rows.iter().filter(|r| r["resource"] == resource).collect();
        ensure!(
            matches.len() == 1,
            "private posture source missing/duplicated"
        );
        let row = matches[0];
        let provider = number(&cache["provider"])?;
        let native = bytes(&cache["resource_raw"])?;
        ensure!(
            native.len() == 0x90
                && sparse.read(resource, 0x90)? == native
                && word(&native, 0) == provider
                && word(&native, 0x78) == 0
                && cache["cache_pointer"] == 0
                && cache["status"] == "unmaterialized"
                && cache["effective"].is_null()
                && row["provider"] == provider
                && row["cache_pointer"] == 0
                && row["objects"] == cache["objects"],
            "private posture owner/cached pointer differs"
        );
        let source_name = format!("native-provider:{provider:08x}");
        ensure!(
            cache["native_source"] == source_name,
            "private posture provider alias differs"
        );
        let vector = at(provider, 0x25c)?;
        let raw = memory.read(vector, 12)?;
        ensure!(
            row["vector"]["pointer"] == vector && bytes(&row["vector"]["raw"])? == raw,
            "private posture generic cache vector differs"
        );
        let (a, b, c) = (word(&raw, 0), word(&raw, 4), word(&raw, 8));
        ensure!(
            (a >= 0x10000 || a == 0 && b == 0 && c == 0)
                && a <= b
                && b <= c
                && (b - a) % 12 == 0
                && (c - a) % 12 == 0
                && (c - a) / 12 <= 4096,
            "private posture generic cache extent differs"
        );
        let records = memory.read(a, (b - a) as usize)?;
        ensure!(
            bytes(&row["records"])? == records,
            "private posture generic cache data differs"
        );
        for record in records.chunks_exact(12) {
            ensure!(
                !(word(record, 4) != 0 && word(record, 8) == 150),
                "shared successful posture150 cache needs its retained StringSet/reference owner"
            );
        }
        ensure!(
            row["matches"].as_array().is_some_and(Vec::is_empty),
            "supplied successful private150 cache differs"
        );
        let components = private_miss(&memory, provider)?;
        ensure!(
            serde_json::to_value(&components)? == row["components"],
            "posture component order differs"
        );
        for id in cache["objects"]
            .as_array()
            .context("private posture objects absent")?
        {
            let id: sims_vm::ObjectId = id
                .as_i64()
                .context("private posture object id absent")?
                .try_into()?;
            let object = vm
                .state
                .world
                .objects
                .get(&id)
                .context("private posture VM object absent")?;
            ensure!(
                object.person.is_some()
                    && object
                        .definition
                        .as_ref()
                        .is_some_and(|d| d.source == source_name)
                    && admitted.insert(id),
                "private posture VM source/coverage differs"
            );
        }
        ensure!(
            !program
                .modules
                .get(&source_name)
                .context("private posture Program source absent")?
                .strings
                .contains_key(&150),
            "proved private posture miss contradicts Program table"
        );
        misses.push(
            json!({"source":source_name,"objects":cache["objects"],"components":components,
            "shared150_absent":true,"STR150_absent":true,"CST150_absent":true}),
        );
    }
    ensure!(
        admitted.iter().copied().eq(vm
            .state
            .world
            .objects
            .iter()
            .filter_map(|(&id, o)| o.person.is_some().then_some(id))),
        "private posture person coverage differs"
    );
    let module = program
        .modules
        .get_mut(&global.source)
        .context("effective Global Program source absent")?;
    ensure!(
        global
            .tables
            .keys()
            .all(|id| !module.strings.contains_key(id)),
        "current effective names would overwrite Program tables"
    );
    // These are actual current effective vectors, not STR# file-version claims.
    // Missing private150 stays missing; never manufacture an empty table.
    module.strings.extend(global.tables.clone());
    vm.program_hash = program.fingerprint();
    let restored = vm.clone().restore(&program).map_err(anyhow::Error::msg)?;
    ensure!(
        serde_json::to_vec(&restored)? == before,
        "effective name join changed VM/RNG"
    );
    Ok(Joined {
        source: json!({"kind":"current_effective_posture_names","tick":global.tick,
        "readset_sha256":sparse.source_sha256(),"private_proof_sha256":sims_fixtures::digest(&serde_json::to_vec(source)?),
        "global_cache_sha256":global.canonical_input_sha256,"languages":global.languages,
        "tables":global.tables.keys().collect::<Vec<_>>(),"private_misses":misses,
        "whole_file_version_admitted":false,"complete_Session_admitted":false,"native_parity_credit":0}),
        program,
        vm,
    })
}
