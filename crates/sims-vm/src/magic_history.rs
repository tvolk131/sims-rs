//! Selected-family history consumed by Generic Sims Call43 (410af6).
//! This is not an object attribute default or data serialized in a house ObjM.
use crate::World;
use serde::{Deserialize, Serialize};

pub const CONTROLLER: u32 = 0xb6c90029;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Context {
    /// The application owner's +10 pointer was explicitly observed null.
    Absent,
    Retained {
        enabled: bool,
        /// Native +10c is a full signed integer, compared to signed global9.
        family: i32,
        /// Whether +128 binds this world's registry. Foreign owners must not
        /// be represented as true; the importer must establish that binding.
        registry_present: bool,
        values: [i16; 6],
    },
}

impl Context {
    /// 41003c: the six words and the read-valid flag start at zero. Family
    /// zero is constructed by neighborhood reload, independently of FAMI.
    pub fn constructed(family: i32, registry_present: bool) -> Self {
        Self::Retained {
            enabled: false,
            family,
            registry_present,
            values: [0; 6],
        }
    }
}

pub fn restore(world: &mut World) -> Result<(), String> {
    // A nonzero retained family must equal global9. When global9 is zero,
    // no possible retained context can pass both guards. In particular this
    // covers final load reconciliation without inventing absent process state.
    if world.globals.get(&9) == Some(&0) {
        return Ok(());
    }
    let context = world
        .magic_history
        .as_ref()
        .ok_or("native selected-family magic history unavailable")?;
    let Context::Retained {
        enabled,
        family,
        registry_present,
        values,
    } = context
    else {
        return Ok(());
    };
    if !enabled || *family == 0 {
        return Ok(());
    }
    if i32::from(*world.globals.get(&9).ok_or("selected family unavailable")?) != *family {
        return Ok(());
    }
    let lot = *world.globals.get(&10).ok_or("current lot unavailable")?;
    let registry = world
        .person
        .as_mut()
        .ok_or("native neighborhood unavailable")?;
    // 40745f calls the raw neighborhood zoning query. It does NOT use
    // Generic28's hard-coded vacation/community overrides for lots90..98.
    if (lot != 0 && registry.lot_zoning.get(&lot) != Some(&1)) || !registry_present {
        return Ok(());
    }
    // 4408d4 returns the first entry from445857. Its lazy vector begins in
    // numeric registry order; subsequent registrations preserve append order.
    let order = registry
        .resource_instances
        .entry(CONTROLLER)
        .or_insert_with(|| {
            world
                .objects
                .iter()
                .filter_map(|(&id, o)| {
                    o.definition
                        .as_ref()
                        .filter(|d| d.guid == CONTROLLER)
                        .map(|_| id)
                })
                .collect()
        });
    let Some(&id) = order.first() else {
        return Ok(());
    };
    let object = world
        .objects
        .get_mut(&id)
        .ok_or("stale magic-controller instance")?;
    if object
        .definition
        .as_ref()
        .is_none_or(|d| d.guid != CONTROLLER)
    {
        return Err("magic-controller instance binding mismatch".into());
    }
    let attrs = object
        .attributes
        .get_mut(..6)
        .ok_or("magic controller has fewer than six attributes")?;
    for (attribute, &retained) in attrs.iter_mut().zip(values) {
        *attribute = (*attribute).max(retained);
    }
    Ok(())
}
