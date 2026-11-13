//! Current completed caller admission, separate from the pinned idle v1 cut.
//! Capture authenticity and immutable physical-source admission remain caller
//! responsibilities. This binds those admitted observations to current parts;
//! absence covers inline C++ passes only, never persistent work or future calls.
use super::*;
use sims_fixtures::readset::Readset;

pub(crate) fn boundary(
    proof: &Input,
    state: &Value,
    memory: &Readset<'_>,
) -> Result<BTreeSet<ObjectId>> {
    let p = &proof.parent_closure;
    ensure!(
        proof.schema == 2
            && proof.contract == "completed-native-skin-closure-v2"
            && Some(proof.tick) == state["tick"].as_u64()
            && state["tick"] == state["snapshot"]["tick"]
            && proof.image_sha256 == state["physical_baseline"]["archive"]["compressed_sha256"]
            && proof.parent_closure_sha256 == digest(&serde_json::to_vec(p)?)
            && p["contract"] == "sims191/completed-caller-registry-controls-v2"
            && p["tick"] == proof.tick
            && p["source_observation_sha256"] == proof.observation_sha256
            && p["source_readset_sha256"] == memory.source_sha256()
            && p["physical_compressed_sha256"] == proof.image_sha256
            && p["physical_decoded_sha256"]
                == state["physical_baseline"]["archive"]["decoded_sha256"]
            && p["native_globals"]["active_menu_descriptor"] == 0,
        "current skin caller/source epoch differs"
    );
    let ids: Vec<ObjectId> = state["threads"]["objects"]
        .as_array()
        .context("current skin registry absent")?
        .iter()
        .map(|r| {
            Ok(r["id"]
                .as_i64()
                .context("current skin registry ID absent")?
                .try_into()?)
        })
        .collect::<Result<_>>()?;
    let set: BTreeSet<_> = ids.iter().copied().collect();
    ensure!(
        !set.is_empty()
            && set.len() == ids.len()
            && serde_json::to_value(&set)? == p["construction_absent_objects"]
            && p["construction_absent_objects"] == p["native_consumer_absent_objects"]
            && p["boundary"]["tick"] == proof.tick,
        "current skin caller coverage differs"
    );
    // Every complete actor header was independently recovered from physical
    // RAM. Its available sparse bytes must agree, including active movement.
    for a in &proof.actors {
        for (address, bytes) in [
            (a.animator, raw(&a.animator_raw)?),
            (a.model, raw(&a.model_raw)?),
            (a.skin, raw(&a.skin_raw)?),
            (a.tick_slot_address, raw(&a.tick_slot_raw)?),
        ] {
            overlap(memory, address, &bytes)?;
        }
    }
    for w in &proof.code {
        overlap(memory, w.address, &raw(&w.raw)?)?;
    }
    overlap(memory, proof.callback.address, &raw(&proof.callback.raw)?)?;
    for r in &proof.rig_labels {
        overlap(memory, r.address, &raw(&r.raw)?)?;
    }
    witnesses(&proof.code)?;
    Ok(set)
}

pub(crate) fn overlap(memory: &Readset<'_>, address: u32, bytes: &[u8]) -> Result<()> {
    let end = u64::from(address)
        .checked_add(bytes.len().try_into()?)
        .context("skin range overflow")?;
    ensure!(end <= 1u64 << 32, "skin range outside native address space");
    for r in memory.coverage() {
        let a = u64::from(address).max(u64::from(r.address));
        let z = end.min(u64::from(r.address) + r.bytes.len() as u64);
        if a < z {
            ensure!(
                memory.read(a.try_into()?, (z - a).try_into()?)?
                    == &bytes[(a - u64::from(address)) as usize..(z - u64::from(address)) as usize],
                "current skin sparse/physical overlap differs"
            );
        }
    }
    Ok(())
}

pub fn import(
    parts: BTreeMap<ObjectId, Parts>,
    proof: &Input,
    state: &Value,
    memory: &Readset<'_>,
    fresh_epoch: &str,
) -> Result<BTreeMap<ObjectId, Qualified>> {
    let inherited = boundary(proof, state, memory)?;
    super::import_fields(parts, proof, fresh_epoch, inherited)
}
