//!4352e8/422fc9 pickup and435440/423122 sale permissions. No script callbacks.
use crate::{Simulation, lot_acceptance::person::Diagnostics};
use sims_vm::{ObjectId, World};
use std::collections::BTreeSet;

///The cheat is a process setting (006e4ad4), not a saved object or lot word.
pub fn check(s: &mut Simulation, object: ObjectId, cheat: bool) -> Result<bool, String> {
    let native = s
        .native_lot
        .as_mut()
        .ok_or("pickup permission needs native lot")?;
    let size = native.placement.links.size;
    let ground = native
        .placement
        .flags
        .first()
        .ok_or("pickup ground flags absent")?;
    if ground.len() != size * size {
        return Err("pickup ground flag extent differs".into());
    }
    virtual_check(
        &s.vm.world,
        object,
        cheat,
        size,
        ground,
        &mut native.diagnostics,
        &mut BTreeSet::new(),
    )
}

///Virtual+70 is distinct from pickup's virtual+68 and the Move refund flag.
///435440 calls the *base* pickup check, then tests sale bit8 and rejects any
///live contained object.423122 visits the multipart head followed by each part.
///The process cheat bypasses all base checks, including diagnostic clearing.
pub fn check_sale(s: &mut Simulation, object: ObjectId, cheat: bool) -> Result<bool, String> {
    let w = &s.vm.world;
    let o = w
        .objects
        .get(&object)
        .ok_or("sale permission object absent")?;
    let parts = if o.multitile.is_some() {
        sims_vm::multitile::chain(w, object)?
    } else {
        vec![object]
    };
    if cheat {
        return Ok(true);
    }
    let native = s
        .native_lot
        .as_mut()
        .ok_or("sale permission needs native lot")?;
    let size = native.placement.links.size;
    let ground = native
        .placement
        .flags
        .first()
        .ok_or("sale ground flags absent")?;
    if ground.len() != size * size {
        return Err("sale ground flag extent differs".into());
    }
    for member in parts {
        let mut active = BTreeSet::from([member]);
        if !base(
            w,
            member,
            false,
            size,
            ground,
            &mut native.diagnostics,
            &mut active,
        )? {
            return Ok(false);
        }
        let o = &w.objects[&member];
        if o.data
            .get(&43)
            .copied()
            .ok_or("sale permission flags absent")?
            & 8
            == 0
        {
            return Ok(false);
        }
        //Native signed registry lookups ignore empty, missing and nonpositive
        //IDs. A live child forbids sale even when that child is movable.
        if o.slots
            .iter()
            .rev()
            .flatten()
            .any(|id| *id > 0 && w.objects.contains_key(id))
        {
            return Ok(false);
        }
    }
    Ok(true)
}
fn virtual_check(
    w: &World,
    id: ObjectId,
    cheat: bool,
    size: usize,
    ground: &[u8],
    d: &mut Diagnostics,
    active: &mut BTreeSet<ObjectId>,
) -> Result<bool, String> {
    if !active.insert(id) || active.len() > sims_vm::MAX_STACK {
        return Err("cyclic pickup permission containment".into());
    }
    let o = w
        .objects
        .get(&id)
        .ok_or("pickup permission object absent")?;
    let result = if let Some(part) = &o.multitile {
        let mut current = Some(part.head.unwrap_or(id));
        let mut seen = BTreeSet::new();
        let mut allowed = true;
        while let Some(member) = current {
            if !seen.insert(member) {
                return Err("cyclic pickup permission group".into());
            }
            if !base(w, member, cheat, size, ground, d, active)? {
                allowed = false;
                break;
            }
            current = w
                .objects
                .get(&member)
                .and_then(|o| o.multitile.as_ref())
                .ok_or("pickup permission group member absent")?
                .next;
        }
        allowed
    } else {
        base(w, id, cheat, size, ground, d, active)?
    };
    active.remove(&id);
    Ok(result)
}
fn base(
    w: &World,
    id: ObjectId,
    cheat: bool,
    size: usize,
    ground: &[u8],
    d: &mut Diagnostics,
    active: &mut BTreeSet<ObjectId>,
) -> Result<bool, String> {
    *d = Diagnostics {
        error: 0,
        blocker: None,
    };
    if cheat {
        return Ok(true);
    }
    let o = w
        .objects
        .get(&id)
        .ok_or("pickup permission member absent")?;
    let word = |n| {
        o.data
            .get(&n)
            .copied()
            .ok_or("pickup permission word absent")
    };
    if word(43)? & 2 == 0 {
        d.error = 0x25;
        return Ok(false);
    }
    if word(62)? >= 1 || word(8)? & 0x20 != 0 {
        d.error = 0x26;
        return Ok(false);
    }
    //Native skips missing/nonpositive signed registry IDs, then dispatches
    //virtual+68 on each child, from the last logical slot toward the first.
    for child in o.slots.iter().rev().flatten().copied() {
        if child > 0
            && w.objects.contains_key(&child)
            && !virtual_check(w, child, cheat, size, ground, d, active)?
        {
            return Ok(false);
        }
    }
    let p = w
        .positions
        .get(&id)
        .ok_or("pickup permission position absent")?;
    let column = (p.y >> 4) as i8;
    let row = (p.x >> 4) as i8;
    //As in the presenter, the cached signed bytes reverse physical XY. This
    //specific native test always reads the ground-floor array, even upstairs.
    if column >= 0
        && row >= 0
        && (column as usize) < size
        && (row as usize) < size
        && ground[column as usize * size + row as usize] & 0x20 != 0
    {
        d.error = 0x27;
        return Ok(false);
    }
    Ok(true)
}
