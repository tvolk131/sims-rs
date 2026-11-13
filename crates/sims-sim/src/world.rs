use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_content::{Catalog, House};
use sims_core::GameRng;
use sims_formats::lot::Array;
use sims_formats::saved_objects::{Frame, Instance, Multipart, Person, Relationship, Slot};
use std::collections::BTreeMap;

/// One lot's simulation state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct World {
    /// Ticks the lot has run.
    pub ticks: u32,
    /// Simulator globals (32 words before SIMI 0x41, 64 from it).
    pub globals: Vec<i16>,
    /// The scheduler's position in [`World::order`]: zero starts a new pass.
    pub scheduler_cursor: i16,
    /// The lot's random stream.
    pub lot_rng: GameRng,
    /// The global random stream.
    pub global_rng: GameRng,
    pub budget: Budget,
    /// Object IDs in registration order, the order the scheduler visits.
    pub order: Vec<u16>,
    pub objects: BTreeMap<u16, Object>,
    /// Lot arrays (floors, walls, terrain and more) by Arry chunk ID.
    pub arrays: BTreeMap<u16, Array>,
}

/// The household's money and the lot's value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub funds: i32,
    pub pending_funds: i32,
    pub lot_value: i32,
    pub objects_value: i32,
    pub architecture_value: i32,
    /// Recent days' budget entries; None marks a day with no entry.
    pub days: Vec<Option<[i32; 8]>>,
}

/// One object on the lot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Object {
    pub id: u16,
    pub guid: u32,
    /// Native rectangle order: minimum Y, minimum X, maximum Y, maximum X.
    pub footprint: [i32; 4],
    pub position: [i32; 3],
    pub unknown_short: i16,
    pub attributes: Vec<i16>,
    pub temps: Vec<i16>,
    pub data: Vec<i16>,
    pub obsolete_word: i16,
    /// Size of the native frame arena, in bytes.
    pub stack_capacity_bytes: i32,
    /// Script stack, outermost frame first.
    pub stack: Vec<Frame>,
    pub relationship_flag: i32,
    pub relationships: Vec<Relationship>,
    pub slots: Vec<Slot>,
    pub dynamic_sprites: Vec<i16>,
    pub multipart: Option<Multipart>,
    pub portal: Option<Vec<f32>>,
    pub person: Option<Person>,
    /// Scheduler countdown (low half) and flags (high half); None when the
    /// save carries none.
    pub registry_word: Option<i32>,
}

impl World {
    /// The state a saved house describes, before any script runs. Every
    /// object must have a definition in `catalog`.
    pub fn from_house(house: &House, catalog: &Catalog) -> Result<Self> {
        let simulator = &house.simulator;
        let mut objects = BTreeMap::new();
        for (&id, instance) in &house.objects.instances {
            if catalog.object(instance.guid).is_none() {
                return Err(Error::MissingDefinition {
                    object: id,
                    guid: instance.guid,
                });
            }
            objects.insert(id, Object::from_instance(instance));
        }
        Ok(Self {
            ticks: simulator.ticks,
            globals: simulator.globals.clone(),
            scheduler_cursor: simulator.scheduler_cursor,
            lot_rng: GameRng::from_state(simulator.lot_rng),
            global_rng: GameRng::from_state(simulator.global_rng),
            budget: Budget {
                funds: simulator.funds,
                pending_funds: simulator.pending_funds,
                lot_value: simulator.lot_value,
                objects_value: simulator.objects_value,
                architecture_value: simulator.architecture_value,
                days: simulator.budget_days.clone(),
            },
            order: house.objects.order.clone(),
            objects,
            arrays: house.arrays.clone(),
        })
    }

    /// The canonical encoding: postcard, which writes fields in declaration
    /// order, integers as little-endian varints, floats as their raw bits,
    /// and maps in key order. It is the same on every platform, and it is
    /// what snapshots store and what [`World::hash`] digests.
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("encoding to memory cannot fail")
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        postcard::from_bytes(bytes).map_err(|e| Error::Snapshot(e.to_string()))
    }

    /// SHA-256 of the canonical encoding.
    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.encode()).into()
    }

    /// [`World::hash`] as lowercase hex.
    pub fn hash_hex(&self) -> String {
        self.hash().iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl Object {
    fn from_instance(instance: &Instance) -> Self {
        Self {
            id: instance.id,
            guid: instance.guid,
            footprint: instance.footprint,
            position: instance.position,
            unknown_short: instance.unknown_short,
            attributes: instance.attributes.clone(),
            temps: instance.temps.clone(),
            data: instance.data.clone(),
            obsolete_word: instance.obsolete_word,
            stack_capacity_bytes: instance.stack_capacity_bytes,
            stack: instance.stack.clone(),
            relationship_flag: instance.relationship_flag,
            relationships: instance.relationships.clone(),
            slots: instance.slots.clone(),
            dynamic_sprites: instance.dynamic_sprites.clone(),
            multipart: instance.multipart.clone(),
            portal: instance.portal.clone(),
            person: instance.person.clone(),
            registry_word: instance.registry_word,
        }
    }
}
