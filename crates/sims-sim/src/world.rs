use crate::rooms::Rooms;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_content::{Catalog, House};
use sims_core::{CrtRng, GameRng};
use sims_formats::lot::{Array, Simulator};
use sims_formats::saved_objects::{Frame, Instance, Multipart, Person, Relationship, Slot};
use std::collections::{BTreeMap, BTreeSet};

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
    /// The C runtime's random stream. It belongs to the running game, not
    /// to the lot: no save holds it.
    pub crt_rng: CrtRng,
    /// The side of the lot's active square, in tiles. The arrays stay 64 by
    /// 64 whatever this is (0x4778fc).
    pub size: i16,
    pub budget: Budget,
    /// Object IDs in registration order, the order the scheduler visits.
    pub order: Vec<u16>,
    /// The lot's object type table (objt): type ID to GUID. Script frames
    /// name the object whose code they run by type ID.
    pub types: BTreeMap<u16, u32>,
    pub objects: BTreeMap<u16, Object>,
    /// Lot arrays (floors, walls, terrain and more) by Arry chunk ID.
    pub arrays: BTreeMap<u16, Array>,
    /// Objects waiting to be removed, in the order they asked (0x442d70).
    /// They stay registered until the removals are carried out.
    pub removals: Vec<u16>,
    /// Type attributes: words every object of a type shares, which the
    /// original keeps with its catalog and saves in the neighborhood file
    /// (TATT). One record per definition that asks for any, in catalog
    /// order. A type is looked up by GUID and the first record wins.
    pub type_attributes: Vec<TypeAttributes>,
    /// The instances of each type a script has asked for the instances of,
    /// by GUID. The original makes a type's list on first use, from the
    /// objects of that type in ID order, and from then on appends each new
    /// instance (0x445857).
    pub instances: BTreeMap<u32, Vec<u16>>,
    /// The object that owns interface events, or 0 for none (0x44d906).
    pub event_owner: u16,
    /// The magic history of the family the game has selected, or None
    /// while it has selected none (0x410af6). It belongs to the running
    /// game, not to the lot. [`World::select_family`] sets it.
    pub magic_history: Option<MagicHistory>,
    /// The two indicator objects the game keeps on the lot, or 0 for none:
    /// the marker over the selected Sim, then the one over a date
    /// (0x445e4a). Clearing the lot removes them first (0x445f84).
    pub indicators: [u16; 2],
    /// The GUIDs of the object types whose scripts changed since a house
    /// that listed them was saved (0x43a516). The original keeps this as a
    /// flag on each definition, so it belongs to the running game and
    /// outlives a load: reading a house's type table sets or clears the
    /// flag of each type the table lists and leaves the others alone.
    pub changed_types: BTreeSet<u32>,
    /// For each tile of a multi-tile object that has a following tile, the
    /// ID of that tile: the list the original keeps from a group's lead
    /// tile through its other tiles (0x4228a9). Reading saved objects
    /// appends each tile to its lead's list in the file's order.
    pub next_parts: BTreeMap<u16, u16>,
    /// The skeleton files the running game has read, by name. Reading one
    /// for the first time draws from the C runtime's random stream
    /// (0x598af7), so whether a file was read before is state. It belongs
    /// to the running game, not to the lot.
    pub skeleton_files: BTreeSet<String>,
    /// The lot's rooms, worked out from its walls ([`World::rebuild_rooms`],
    /// 0x479d9a). Empty on a lot whose rooms have not been worked out: the
    /// rooms of the lot the game holds before a load are not modeled.
    pub rooms: Rooms,
    /// The objects on the lot's list of burning fires, in the order they
    /// joined it (Generic Sims Call routines 10 and 11).
    pub fires: Vec<u16>,
    /// Each object's frame storage, for the objects whose thread has
    /// pushed or popped a frame since the object was made or read. See
    /// [`FrameStorage`].
    pub frame_storage: BTreeMap<u16, FrameStorage>,
}

/// The memory an object's script frames live in.
///
/// The original keeps a thread's frames in one block, each frame after
/// the one that called it, and never clears the block. A frame is eight
/// header words (tree, node, stack object, the local and parameter
/// counts, two words of saved primitive state, and a pointer to the
/// frame's code), then its parameters and its locals, padded to a
/// multiple of four words. A new frame's locals are not zeroed: they hold
/// whatever an earlier frame left at that place. So what popped frames
/// left behind is state.
///
/// [`Object::stack`] holds the live frames. This holds the block as it
/// was when a frame was last pushed or popped, and where each live frame
/// starts in it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameStorage {
    /// The block, in words. None is a word that holds half of a pointer,
    /// whose value belongs to the running game's memory. A block that was
    /// never written this far reads as zero.
    pub words: Vec<Option<i16>>,
    /// Where each live frame starts, in words, outermost first.
    pub offsets: Vec<u32>,
    /// The locals of live frames that started on half of a pointer and
    /// have not been written since, as the frame's place in the stack and
    /// the local's number. Reading one fails.
    pub unknown: BTreeSet<(u32, u32)>,
}

/// One type's shared words. See [`World::type_attributes`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeAttributes {
    pub guid: u32,
    pub values: Vec<i16>,
}

/// See [`World::magic_history`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MagicHistory {
    pub family: i32,
    pub values: [i16; 6],
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
    /// The branch the object's last primitive took. No save holds it.
    pub last_result: bool,
    /// Set when the object is queued for removal with the request not to
    /// reset the objects that depend on it (the original's object flag
    /// 0x1000). No save holds it.
    pub removal_skips_resets: bool,
}

/// The side of a lot's arrays, and of a lot nothing has resized.
pub const FULL_SIZE: i16 = 64;

impl World {
    /// The lot the game holds before it has loaded any house: no objects,
    /// all 64 globals zero, the full 64 tiles.
    ///
    /// **Unverified**: nothing here was compared with the original at
    /// startup. The original has a clock and two indicator objects by then,
    /// and its C runtime stream need not still be at its initial state 1.
    pub fn empty() -> Self {
        Self {
            ticks: 0,
            globals: vec![0; 64],
            scheduler_cursor: 0,
            lot_rng: GameRng::from_state(0),
            global_rng: GameRng::from_state(0),
            crt_rng: CrtRng::from_state(1),
            size: FULL_SIZE,
            budget: Budget {
                funds: 0,
                pending_funds: 0,
                lot_value: 0,
                objects_value: 0,
                architecture_value: 0,
                days: Vec::new(),
            },
            order: Vec::new(),
            types: BTreeMap::new(),
            objects: BTreeMap::new(),
            arrays: BTreeMap::new(),
            removals: Vec::new(),
            type_attributes: Vec::new(),
            instances: BTreeMap::new(),
            event_owner: 0,
            magic_history: None,
            indicators: [0; 2],
            changed_types: BTreeSet::new(),
            next_parts: BTreeMap::new(),
            skeleton_files: BTreeSet::new(),
            rooms: Rooms::default(),
            fires: Vec::new(),
            frame_storage: BTreeMap::new(),
        }
    }

    /// Reads a house's simulator state (SIMI) into the lot the game is
    /// holding (0x46cfb3): the clock, the globals, the scheduler cursor,
    /// both game random streams and the budget. The objects on the lot, its
    /// size and the C runtime stream are not in SIMI and stay as they are.
    ///
    /// The globals are the saved 32 or 64 words as they are;
    /// [`World::apply_context`] turns them into the ones the lot runs with.
    pub fn read_simulator(&mut self, simulator: &Simulator) {
        self.ticks = simulator.ticks;
        self.globals = simulator.globals.clone();
        self.scheduler_cursor = simulator.scheduler_cursor;
        self.lot_rng = GameRng::from_state(simulator.lot_rng);
        self.global_rng = GameRng::from_state(simulator.global_rng);
        self.budget = Budget {
            funds: simulator.funds,
            pending_funds: simulator.pending_funds,
            lot_value: simulator.lot_value,
            objects_value: simulator.objects_value,
            architecture_value: simulator.architecture_value,
            days: simulator.budget_days.clone(),
        };
    }

    /// The state a saved house describes, before any script runs: the
    /// whole file read onto an empty lot in one go. Every object must have
    /// a definition in `catalog`.
    ///
    /// The original never holds this state. It reads the simulator state
    /// first, creates and removes the lot-wide controllers, and only then
    /// reads the arrays, the type table and the objects, each onto the lot
    /// it is holding (`docs/roadmap.md`, load sequence). It also constructs
    /// each object before it reads the saved fields into it, which this
    /// does not. This is for looking at a house file, not for loading one.
    pub fn from_house(house: &House, catalog: &Catalog) -> Result<Self> {
        let mut world = Self::empty();
        world.read_simulator(&house.simulator);
        world.arrays = house.arrays.clone();
        world.types = house.types.entries.iter().map(|t| (t.id, t.guid)).collect();
        // Every object the file describes, whether or not its registration
        // list names it.
        for (&id, instance) in &house.objects.instances {
            if catalog.object(instance.guid).is_none() {
                return Err(Error::MissingDefinition {
                    object: id,
                    guid: instance.guid,
                });
            }
            world.objects.insert(id, Object::from_instance(instance));
        }
        world.order = house.objects.order.clone();
        Ok(world)
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
    /// The scheduler countdown, the low half of the registry word: while it
    /// is above zero the scheduler counts it down instead of running the
    /// object. None when the object has no registry word.
    pub fn countdown(&self) -> Option<i16> {
        self.registry_word.map(|word| word as i16)
    }

    /// Replaces the countdown and keeps the flags. Does nothing without a
    /// registry word.
    pub fn set_countdown(&mut self, countdown: i16) {
        if let Some(word) = &mut self.registry_word {
            *word = (*word & !0xffff) | i32::from(countdown as u16);
        }
    }

    /// The scheduler flags, the high half of the registry word.
    pub fn flags(&self) -> u16 {
        self.registry_word.map_or(0, |word| (word >> 16) as u16)
    }

    /// Sets or clears the flags in `mask` and keeps the rest (0x4457ef).
    /// Does nothing without a registry word.
    pub fn set_flags(&mut self, mask: u16, on: bool) {
        if let Some(word) = &mut self.registry_word {
            let flags = ((*word >> 16) as u16 & !mask) | if on { mask } else { 0 };
            *word = ((u32::from(flags) << 16) | (*word as u32 & 0xffff)) as i32;
        }
    }

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
            last_result: false,
            removal_skips_resets: false,
        }
    }
}
