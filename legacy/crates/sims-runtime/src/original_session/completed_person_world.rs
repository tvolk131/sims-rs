//! Constructorless current neighborhood/person-world component. External
//! Family/history and original relationship row order retain separate owners;
//! this is not a complete World/Session or source-continuation admission.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sims_vm::{ObjectId, person::WorldState};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    pub epoch: String,
    pub events_since_binding: Vec<Value>,
    pub native_pending_consumer_closure: String,
}
pub struct Imported {
    pub state: WorldState,
    pub relationship_orders: BTreeMap<i16, Vec<i16>>,
    pub external_families: Value,
    pub delivery: Delivery,
}

pub fn import(
    fields: Value,
    orders: BTreeMap<i16, Vec<i16>>,
    external: Value,
    delivery: Delivery,
) -> Result<Imported> {
    let state: WorldState = serde_json::from_value(fields.clone())?;
    ensure!(
        serde_json::to_value(&state)? == fields,
        "captured person fields missing, defaulted, or discarded"
    );
    ensure!(
        !delivery.epoch.is_empty()
            && delivery.epoch.len() <= 200
            && delivery.events_since_binding.is_empty()
            && delivery.native_pending_consumer_closure == "separate admission demand",
        "fresh delivery domain/source-closure policy differs"
    );
    ensure!(
        external["schema"] == 1
            && external["status"] == "captured"
            && external["families"].is_array(),
        "external Family sidecar absent"
    );
    let neighbor_ids: BTreeSet<_> = state.neighbors.keys().copied().collect();
    ensure!(
        neighbor_ids.len() <= 32767
            && neighbor_ids.iter().all(|v| *v > 0)
            && orders.keys().copied().collect::<BTreeSet<_>>() == neighbor_ids,
        "captured neighbor/order coverage differs"
    );
    let slots = state
        .neighbor_slots
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("native neighbor slots unknown"))?;
    ensure!(
        slots.len() <= 32767
            && slots.iter().flatten().copied().collect::<BTreeSet<_>>() == neighbor_ids
            && slots.iter().flatten().count() == neighbor_ids.len(),
        "native neighbor slot mapping differs"
    );
    for (index, id) in slots.iter().enumerate() {
        ensure!(
            id.is_none_or(|id| id as usize == index + 1),
            "native neighbor identity is not its retained slot"
        );
    }
    for (&id, n) in &state.neighbors {
        let order = &orders[&id];
        let keys: BTreeSet<_> = n.relationships.keys().copied().collect();
        ensure!(
            order.len() == keys.len() && order.iter().copied().collect::<BTreeSet<_>>() == keys,
            "native relationship order lost/duplicated"
        );
        ensure!(
            n.person.len() == 256
                && (0..256).all(|v| n.person.contains_key(&v))
                && n.friend_count.is_some(),
            "native neighbor data/cache incomplete"
        );
        ensure!(
            n.relationships.values().all(|r| r.len() <= 256),
            "native relationship column bound exceeded"
        );
    }
    let family_ids: BTreeSet<_> = state.families.keys().copied().collect();
    let order = state
        .family_order
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("native Family order unknown"))?;
    ensure!(
        family_ids.len() <= 32767
            && order.len() == family_ids.len()
            && order.iter().copied().collect::<BTreeSet<_>>() == family_ids
            && state
                .family_metadata
                .keys()
                .copied()
                .collect::<BTreeSet<_>>()
                == family_ids
            && state.family_homes.keys().copied().collect::<BTreeSet<_>>() == family_ids
            && family_ids.contains(&state.controlled_family),
        "native Family ownership coverage differs"
    );
    for (&id, members) in &state.families {
        let m = &state.family_metadata[&id];
        let guids = m
            .member_guids
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("native Family GUID vector unknown"))?;
        ensure!(
            members.len() == guids.len()
                && m.friend_count.is_some()
                && members
                    .iter()
                    .zip(guids)
                    .all(|(n, g)| state.neighbors.get(n).is_some_and(|v| v.guid == *g)),
            "native Family membership/cache differs"
        );
    }
    ensure!(
        state.family_changes.is_empty() && state.nearby_houses.is_some(),
        "family delivery/nearby ownership differs"
    );
    let objects: BTreeSet<ObjectId> = state.object_order.iter().copied().collect();
    ensure!(
        objects.len() == state.object_order.len()
            && objects.len() <= 32767
            && objects.iter().all(|id| *id > 0),
        "native live object order differs"
    );
    let people: BTreeSet<_> = state.person_order.iter().copied().collect();
    ensure!(
        people.len() == state.person_order.len() && people.is_subset(&objects),
        "native person order differs"
    );
    ensure!(
        state
            .resource_instances
            .values()
            .all(|ids| ids.iter().all(|id| objects.contains(id))
                && ids.iter().collect::<BTreeSet<_>>().len() == ids.len()),
        "native resource-instance cache differs"
    );
    let mut tiled = BTreeSet::new();
    for chain in &state.tile_chains {
        ensure!(
            !chain.is_empty()
                && chain
                    .iter()
                    .all(|id| objects.contains(id) && tiled.insert(*id)),
            "native tile traversal lost/duplicated an owner"
        );
    }
    let a = state
        .autonomy
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("native lot autonomy capability unknown"))?;
    ensure!(
        a.residential
            && a.full_scan.is_none()
            && a.advertised.is_none()
            && a.selected_person.is_none_or(|id| people.contains(&id))
            && a.placements.keys().copied().collect::<BTreeSet<_>>() == objects,
        "captured residential autonomy/selection/placement policy differs"
    );
    Ok(Imported {
        state,
        relationship_orders: orders,
        external_families: external,
        delivery,
    })
}
