use crate::animator::{Animator, HeldPose, Layer};
use crate::characters::Characters;
use crate::cow::{CowBox, CowMap};
use crate::neighbors::{Family, Neighbors};
use crate::rooms::Rooms;
use crate::route::Routing;
use crate::{Error, Result};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize, Serializer};
use sha2::{Digest, Sha256};
use sims_content::{Catalog, House as HouseFile};
use sims_core::{CrtRng, GameRng};
use sims_formats::lot::{Array, House, Simulator};
use sims_formats::saved_objects::{Frame, Instance, Multipart, Person, Relationship, Slot};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Bit 0 of [`ScratchPtrs`]: `[object + 0x128]` has been populated by a safe
/// writer (`Expression`, `Random Number`, `Get Direction To`, `Relationship`).
pub const SCRATCH_PTR_SET: u8 = 1 << 0;
/// Bit 1 of [`ScratchPtrs`]: last `[object + 0x128]` writer was `Set To Next`.
pub const SCRATCH_PTR_SET_TO_NEXT: u8 = 1 << 1;
/// Bit 2 of [`ScratchPtrs`]: last `[object + 0x128]` writer was `Expression`.
pub const SCRATCH_PTR_EXPRESSION: u8 = 1 << 2;
/// Bit 3 of [`ScratchPtrs`]: last `[object + 0x128]` writer was `Random Number`
/// or `Get Direction To`.
pub const SCRATCH_PTR_ARG_SLOT: u8 = 1 << 3;

const SCRATCH_DENSE_LIMIT: usize = 4096;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ScratchPtrsInner {
    dense: Vec<u8>,
    overflow: BTreeMap<u16, u8>,
    counts: [usize; 4],
}

impl ScratchPtrsInner {
    #[inline]
    fn get(&self, id: u16) -> u8 {
        let idx = usize::from(id);
        if idx < SCRATCH_DENSE_LIMIT {
            self.dense.get(idx).copied().unwrap_or(0)
        } else {
            self.overflow.get(&id).copied().unwrap_or(0)
        }
    }

    fn set(&mut self, id: u16, mask: u8) {
        let mask = mask & 0x0f;
        let prev = self.get(id);
        if prev == mask {
            return;
        }
        let changed = prev ^ mask;
        for bit in 0..4u8 {
            let flag = 1u8 << bit;
            if changed & flag != 0 {
                if mask & flag != 0 {
                    self.counts[usize::from(bit)] += 1;
                } else {
                    self.counts[usize::from(bit)] -= 1;
                }
            }
        }
        let idx = usize::from(id);
        if idx < SCRATCH_DENSE_LIMIT {
            if mask == 0 {
                if idx < self.dense.len() {
                    self.dense[idx] = 0;
                    while self.dense.last() == Some(&0) {
                        self.dense.pop();
                    }
                }
            } else {
                if idx >= self.dense.len() {
                    self.dense.resize(idx + 1, 0);
                }
                self.dense[idx] = mask;
            }
        } else if mask == 0 {
            self.overflow.remove(&id);
        } else {
            self.overflow.insert(id, mask);
        }
    }
}

/// Compact copy-on-write per-object bitmask consolidating the four
/// `[object + 0x128]` scratch-pointer state flags (`SCRATCH_PTR_*`).
///
/// Cloning is `O(1)` via [`CowBox`]; unmodified flags skip CoW detachment.
#[derive(Debug, Clone, Default, Eq)]
pub struct ScratchPtrs(CowBox<ScratchPtrsInner>);

impl PartialEq for ScratchPtrs {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0) || *self.0 == *other.0
    }
}

impl ScratchPtrs {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn get(&self, id: u16) -> u8 {
        self.0.get(id)
    }

    #[inline]
    pub fn has(&self, id: u16, bit: u8) -> bool {
        self.0.get(id) & bit != 0
    }

    #[inline]
    pub fn set(&mut self, id: u16, mask: u8) {
        let mask = mask & 0x0f;
        if self.0.get(id) == mask {
            return;
        }
        self.0.set(id, mask);
    }

    #[inline]
    pub fn insert_bit(&mut self, id: u16, bit: u8) {
        let prev = self.0.get(id);
        let next = (prev | bit) & 0x0f;
        if prev != next {
            self.0.set(id, next);
        }
    }

    #[inline]
    pub fn remove_bit(&mut self, id: u16, bit: u8) {
        let prev = self.0.get(id);
        let next = prev & !bit;
        if prev != next {
            self.0.set(id, next);
        }
    }

    #[inline]
    pub fn clear_id(&mut self, id: u16) {
        if self.0.get(id) != 0 {
            self.0.set(id, 0);
        }
    }

    /// Reconstructs a `BTreeSet<u16>` of all object IDs having `bit` set.
    pub fn to_btreeset(&self, bit: u8) -> BTreeSet<u16> {
        self.iter_bit(bit).collect()
    }

    /// Builds [`ScratchPtrs`] from the four legacy `BTreeSet<u16>` sets
    /// (used when hydrating saved lots).
    pub fn from_btreesets(
        set: &BTreeSet<u16>,
        set_to_next: &BTreeSet<u16>,
        expression: &BTreeSet<u16>,
        arg_slot: &BTreeSet<u16>,
    ) -> Self {
        let mut inner = ScratchPtrsInner::default();
        for &id in set {
            inner.set(id, inner.get(id) | SCRATCH_PTR_SET);
        }
        for &id in set_to_next {
            inner.set(id, inner.get(id) | SCRATCH_PTR_SET_TO_NEXT);
        }
        for &id in expression {
            inner.set(id, inner.get(id) | SCRATCH_PTR_EXPRESSION);
        }
        for &id in arg_slot {
            inner.set(id, inner.get(id) | SCRATCH_PTR_ARG_SLOT);
        }
        Self(CowBox::new(inner))
    }

    /// Iterates all object IDs with `bit` set in strictly ascending order.
    pub fn iter_bit(&self, bit: u8) -> impl Iterator<Item = u16> + '_ {
        let dense = self
            .0
            .dense
            .iter()
            .enumerate()
            .filter(move |&(_, &m)| m & bit != 0)
            .map(|(i, _)| i as u16);
        let overflow = self
            .0
            .overflow
            .iter()
            .filter(move |&(_, &m)| m & bit != 0)
            .map(|(&id, _)| id);
        dense.chain(overflow)
    }

    /// Number of object IDs having `bit` (a single `SCRATCH_PTR_*` flag) set.
    #[inline]
    pub fn count_bit(&self, bit: u8) -> usize {
        debug_assert!(bit.is_power_of_two() && bit <= 0x08);
        self.0.counts[bit.trailing_zeros() as usize]
    }

    /// Zero-allocation [`Serialize`] adapter that emits the exact `postcard`
    /// wire representation of `BTreeSet<u16>` for `bit`.
    #[inline]
    pub fn serialize_bit(&self, bit: u8) -> ScratchPtrBitSerialize<'_> {
        ScratchPtrBitSerialize { ptrs: self, bit }
    }
}

/// Zero-allocation `Serialize` adapter emitting a single bit plane of
/// [`ScratchPtrs`] identically to `BTreeSet<u16>`.
pub struct ScratchPtrBitSerialize<'a> {
    ptrs: &'a ScratchPtrs,
    bit: u8,
}

impl Serialize for ScratchPtrBitSerialize<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let len = self.ptrs.count_bit(self.bit);
        let mut seq = serializer.serialize_seq(Some(len))?;
        for id in self.ptrs.iter_bit(self.bit) {
            seq.serialize_element(&id)?;
        }
        seq.end()
    }
}

/// Opaque cached reverse index from GUID to lowest type ID in [`World::types`].
#[derive(Debug, Clone, Default)]
pub struct TypesByGuidCache(Option<Arc<TypesByGuid>>);

/// Cached reverse index from GUID to lowest type ID in [`World::types`].
#[derive(Debug, Clone)]
struct TypesByGuid {
    len: usize,
    first: Option<(u16, u32)>,
    last: Option<(u16, u32)>,
    map: BTreeMap<u32, u16>,
}

impl TypesByGuid {
    fn build(types: &BTreeMap<u16, u32>) -> Self {
        let mut map = BTreeMap::new();
        for (&id, &guid) in types {
            map.entry(guid).or_insert(id);
        }
        Self {
            len: types.len(),
            first: types.first_key_value().map(|(&k, &v)| (k, v)),
            last: types.last_key_value().map(|(&k, &v)| (k, v)),
            map,
        }
    }

    #[inline]
    fn matches(&self, types: &BTreeMap<u16, u32>) -> bool {
        self.len == types.len()
            && self.first == types.first_key_value().map(|(&k, &v)| (k, v))
            && self.last == types.last_key_value().map(|(&k, &v)| (k, v))
    }
}

/// One lot's simulation state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct World {
    /// Ticks the lot has run.
    pub ticks: u32,
    /// Simulator globals (32 words before SIMI 0x41, 64 from it).
    pub globals: Vec<i16>,
    /// The scheduler's position in a pass over the objects: the ID of the
    /// object it looks at next, or zero for a new pass (0x46d553).
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
    /// Object IDs in registration order: the order of the original's
    /// list of objects, which the load's passes walk. The scheduler does
    /// not use it: it goes by ID (0x46d553).
    pub order: Vec<u16>,
    /// The original's person vector, which Set To Next walks for people
    /// (mode 1, 0x44efb4; mode 12, 0x44f0d4): the people a load reads,
    /// in registration order, then each person made since, at the end.
    /// A person removed leaves it.
    pub people: Vec<u16>,
    /// The lot's object type table (objt): type ID to GUID. Script frames
    /// name the object whose code they run by type ID.
    pub types: BTreeMap<u16, u32>,
    pub objects: CowMap<u16, Object>,
    /// Lot arrays (floors, walls, terrain and more) by Arry chunk ID.
    pub arrays: CowMap<u16, Array>,
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
    pub frame_storage: CowMap<u16, FrameStorage>,
    /// The dialogs objects have asked for and the registry still holds.
    /// See [`Dialogs`].
    pub dialogs: Dialogs,
    /// For each person whose tick has run, what the tick keeps that no
    /// save holds. A person with no entry has the values its constructor
    /// gives (0x459327); see [`PersonTick`].
    pub person_ticks: BTreeMap<u16, PersonTick>,
    /// The neighborhood's people. See [`Neighbors`].
    pub neighbors: CowBox<Neighbors>,
    /// For each person whose animator has run, what it keeps. A person
    /// with no entry has the animator the load leaves; see
    /// [`crate::animator::LOADED`].
    pub animators: BTreeMap<u16, Animator>,
    /// How often the household was counted on the lot. See [`HouseUsage`].
    pub usage: HouseUsage,
    /// The people's routes and what routing changed elsewhere. See
    /// [`Routing`].
    pub routing: Routing,
    /// The last unique number given to a person's running action
    /// (0x6e4904). A new action with a target and no number takes the
    /// next one (0x41f158). It belongs to the running game, not to the
    /// lot: no save holds it. It is 0 in the game's memory before the
    /// house is entered and at the start of the Goth house's tick 233,
    /// the first that gives one, as captures of the original show.
    pub action_serial: i32,
    /// The advertisements of the interaction entry the original keeps
    /// one copy of (the record at 0x6e5100, which 0x6c437c points to).
    /// Find Best Interaction copies each entry it may choose over it
    /// (0x4763c5, from the check 0x458319), the entry's check tree reads
    /// and writes it (scopes 12, 28 and 29), and the check then adjusts
    /// it for the room. Nothing else copies over it, so a check tree the
    /// pie menu runs finds what the last one left. The record always has
    /// sixteen. None until something first writes it, which stands for
    /// what the game's start leaves there: sixteen advertisements of 0
    /// (the record's constructor 0x475e23, run by the static initializer
    /// 0x458271 from the start-up table at 0x6c0154, zeroes each one). It
    /// belongs to the running game, not to the lot.
    pub advertisement_record: Option<[Advertisement; ADVERTISEMENTS]>,
    /// The neighborhood's families, in the order the neighborhood's
    /// reload sorts them. See [`Family`]. Like the neighborhood's
    /// people, they belong to the running game, not to the lot.
    pub families: Vec<Family>,
    /// The first headline of each object that shows one. See
    /// [`Headline`]. An object with no entry shows none: its first
    /// headline, if it has any, has a duration of 0.
    pub headlines: BTreeMap<u16, Headline>,
    /// The layer in the base slot (animator +0x14) of each person's
    /// animator that has one: the clip's playback, whose phase the
    /// skin's update advances (0x5992b3, 0x597c4c) and from which the
    /// head's measurement poses the bones (0x5904d2). What kind of clip
    /// it is stays in [`Animator::base`], and a person has an entry here
    /// exactly when that slot is filled. Kept apart from the animator so
    /// that the encodings of the states before it was kept stay a prefix
    /// of the new ones.
    pub base_layers: BTreeMap<u16, Layer>,
    /// The bones of each person's skin whose translation a layer has
    /// written since the skin was made, by name. The skin's update keeps
    /// a bone's translation when no layer it samples moves that bone
    /// (0x598226, 0x598450), so a bone in this set that no layer playing
    /// now moves holds a translation that is not kept; one not in it is
    /// still at rest. Every layer the skin's update could sample is
    /// counted, so this may hold more than the original moved. Kept apart
    /// from the animator so that the encodings of the states before it
    /// was kept stay a prefix of the new ones.
    pub translated_bones: BTreeMap<u16, BTreeSet<String>>,
    /// The neighborhood's sixteen data words (the neighborhood's +0x128),
    /// as the neighborhood file's NGBH chunk gives them. Scripts read
    /// words 1 to 15 through scope 34 (0x4531bf) and cannot write them;
    /// a Generic Sims Call (0x429c7d, from 0x44f7eb) writes words 1 and
    /// 2. Like the neighborhood's people, they belong to the running
    /// game, not to the lot.
    pub neighborhood_data: [i16; 16],
    /// The pose each person's skin holds from the load, for each person
    /// whose skin has sampled no layer since (see [`HeldPose`]). The load
    /// restores the saved layers, poses the skin with them and places it
    /// (0x5918ba), and the person's reset then takes the layers away
    /// (0x589220) but leaves the bones; the skin's next update that
    /// samples a layer ends it. Kept apart from the animator so that the
    /// encodings of the states before it was kept stay a prefix of the new
    /// ones.
    pub held_poses: BTreeMap<u16, HeldPose>,
    /// The people whose animator was asked to stop its route while the
    /// route was under way (animator +0x284 set by 0x589e2d), since the
    /// route started: a route's start clears it (0x589d38), and the walk
    /// stops the route at its next step when it is set (0x58eeeb). A new
    /// animator's constructor sets it too (0x5887b7), and the reset can
    /// (0x5892dd); neither is kept, as the walk reads it only on a route
    /// under way (+0x188 1), which only a route's start makes. Kept apart
    /// from the animator so that the encodings of the states before it
    /// was kept stay a prefix of the new ones.
    pub stops_asked: BTreeSet<u16>,
    /// The person types made while the game ran (Make New Character,
    /// 0x4260e1), and the neighborhood's counter for their files. Kept
    /// apart so that the encodings of the states before it was kept stay a
    /// prefix of the new ones.
    pub characters: Characters,
    /// The person types whose portrait the interface has built this
    /// session, by GUID: those whose entry in the interface's table of
    /// person types (0x6f2698, one 0x44-byte entry for each person
    /// definition the catalog holds, 0x4e8b3d) has its first image (entry
    /// +0x14, checked by 0x4e92a8). Building one when the type has no
    /// person on the lot makes a person and removes it at once
    /// (0x4e8c5d, 0x4e9652), which draws from the C runtime's stream, so
    /// what the interface has built is kept. Only the dialog's builds are
    /// counted (see `sims_vm`'s Dialog primitive). Kept apart so that the
    /// encodings of the states before it was kept stay a prefix of the new
    /// ones.
    pub portraits: BTreeSet<u32>,
    /// The person types whose portrait slot 1 (entry +0x14) has its dirty
    /// byte (+0x18) set (`0x4eb0eb` → `0x4e895e`), so `0x4e92a8` calls
    /// `0x4e8c5d(index, 1, 0)` at `0x4e92e5` before falling back to `BMP_`
    /// 2002 (`0x4e9338`). Always disjoint from [`World::portraits`]: every
    /// `0x4eb0eb` releases all seven slots before marking them dirty, and
    /// storing a non-null image (`0x4e899e`) clears the dirty byte. Kept
    /// apart so that the encodings of the states before it was kept stay a
    /// prefix of the new ones.
    pub dirty_portraits: BTreeSet<u32>,
    /// The person types whose `BMP_` 2002 portrait was written to a writable
    /// character file during this run (`0x4e938c` → `0x4e8d2e` → `0x4e9000`),
    /// so a later invalidation (`dirty_portraits`) and no-tile fallback
    /// (`0x4e9706` → `0x4e9338`) can load the saved bitmap even when the file
    /// had none at load. Kept apart so that the encodings of the states before
    /// it was kept stay a prefix of the new ones.
    pub saved_portraits: BTreeSet<u32>,
    /// Which IFF files had `BMP_` 2002 (`0x7d2`) generated and persisted by
    /// `0x4e938c` → `0x4e8c5d(index, 1, 1)` → `0x4e9000` during this session.
    /// Keyed by IFF file ([`PortraitFile`]) rather than GUID because `0x4e9000`
    /// writes resource `BMP_` 2002 into the definition's IFF file, so any other
    /// person definition in the same file sees it at `0x4e9338` → `0x4e9260`.
    /// Kept out of [`World::encode`] so snapshot format 32's wire bytes stay
    /// unchanged; `sims_vm::boot::save::Saved` preserves it across lot
    /// save/restore.
    #[serde(skip)]
    pub generated_portrait_2002: BTreeSet<PortraitFile>,
    /// Whether each runtime-made character (`Make New Character`, opcode 19)
    /// has an exact `"HEAD"` bone binding (`0x4e7824`): `Some(true)` when
    /// every candidate wardrobe pick binds `"HEAD"`, `Some(false)` when none
    /// does, or `None` when the wardrobe mixes both (depends on unmodeled
    /// wardrobe order). Kept out of [`World::encode`] so snapshot format 32's
    /// wire bytes stay unchanged; `sims_vm::boot::save::Saved` preserves it
    /// across lot save/restore.
    #[serde(skip)]
    pub made_portrait_head: BTreeMap<u32, Option<bool>>,
    /// Runtime-made pet GUIDs whose immediate `0x44b126` (`0x4e8c5d`) render
    /// placed the pet and reached `0x4e7824` while [`World::made_portrait_head`]
    /// was `None`, leaving whether `0x44b126` cached the portrait or wrote
    /// `BMP_` 2002 unknown until a later `UI Effect` or `Dialog` queries it.
    /// Kept out of [`World::encode`] so snapshot format 32's wire bytes stay
    /// unchanged; `sims_vm::boot::save::Saved` preserves it across save/restore.
    #[serde(skip)]
    pub ambiguous_portraits: BTreeSet<u32>,
    /// Lazy `.ndx`-indexed skeletons populated at runtime (`0x5a0384` →
    /// `0x59f18c`–`0x59f19f` / `0x5a389b`): maps skeleton name to
    /// `Some(catalog_location)` when loaded directly or filled as a sibling by
    /// another catalog's load, or `None` when a lazy lookup failed so later
    /// catalog loads cannot overwrite it. Kept out of [`World::encode`] to
    /// preserve snapshot format 32; `sims_vm::boot::save::Saved` preserves it.
    #[serde(skip)]
    pub loaded_indexed_skeletons: BTreeMap<String, Option<String>>,
    /// Lazy `.ndx`-indexed appearances populated at runtime (`0x5a044d` →
    /// `0x5a38da` → `0x59f35f`): maps lowercased appearance name to
    /// `Some(has_head)` when loaded directly or filled as a sibling by another
    /// catalog's load, or `None` when a lazy lookup failed so later catalog
    /// loads cannot overwrite it. Kept out of [`World::encode`] to preserve
    /// snapshot format 32; `sims_vm::boot::save::Saved` preserves it.
    #[serde(skip)]
    pub loaded_indexed_suits: BTreeMap<String, Option<bool>>,
    /// Per-suit required `"HEAD"` mesh bone lists from the first `.ndx` `.bcf`
    /// catalog that registered each runtime-loaded suit (`0x59f34c..0x59f35f`),
    /// so duplicate suit names across catalogs preserve the winning catalog's
    /// mesh requirements (`0x599f8a..0x59a01c`). Kept out of [`World::encode`]
    /// to preserve snapshot format 32; `sims_vm::boot::save::Saved` preserves it.
    #[serde(skip)]
    pub loaded_indexed_suit_bones: BTreeMap<String, Vec<Vec<String>>>,
    /// `STR# 200` outfit rows rewritten by `Make New Character` (`0x40d183`,
    /// `0x40d4e7`, `0x40a5e2` rows 2 and 30–34, `0x40a667` rows 30 and 34,
    /// `0x40ac57` rows 1, 2, and 15–18) for runtime-made characters, kept out
    /// of [`World::characters`] / [`World::encode`] to preserve snapshot format
    /// 32; `sims_vm::boot::save::Saved` preserves it.
    #[serde(skip)]
    pub made_outfits: BTreeMap<u32, BTreeMap<usize, String>>,
    /// Per-person live `cTSPerson` attachment slots (`[ebx + 0x258]` head,
    /// `[ebx + 0x25c]` body & body_alt, `[ebx + 0x268]` left & right extras,
    /// `[ebx + 0x274]` accessories), cached dressing guard (`[ebx + 0x1b0..0x1bc]`),
    /// and cached career workwear (`[esi + 0xb2c..0xbc8]`).
    /// Kept out of [`World::encode`] to preserve snapshot format 32;
    /// `sims_vm::boot::save::Saved` preserves it across save/restore.
    #[serde(skip)]
    pub person_outfits: BTreeMap<u16, PersonOutfit>,
    /// Compact bitmask consolidating the four `[object + 0x128]` scratch
    /// destination pointer states (`SCRATCH_PTR_SET`, `SCRATCH_PTR_SET_TO_NEXT`,
    /// `SCRATCH_PTR_EXPRESSION`, `SCRATCH_PTR_ARG_SLOT`). Kept out of
    /// [`World::encode`] to preserve snapshot format 32; `sims_vm::boot::save::Saved`
    /// preserves it across save/restore.
    #[serde(skip)]
    pub scratch_ptrs: ScratchPtrs,
    /// Appearance names newly inserted into `AvatarRegistry`'s hash table by
    /// runtime `.bcf` catalog loads (`0x59f320` → `0x5a1fb1`) that were not
    /// already present in startup eager or `.ndx` registrations, in exact
    /// insertion order. Kept out of [`World::encode`] to preserve snapshot
    /// format 32; `sims_vm::boot::save::Saved` preserves it across save/restore.
    #[serde(skip)]
    pub runtime_added_suits: Vec<String>,
    /// First-call snapshot of `AvatarRegistry`'s appearance hash-table walk
    /// (`0x409255` → `0x6e48a0`, skipping `"suit-*"`), cached for the rest of
    /// the session once any generated-outfit matcher runs. Kept out of
    /// [`World::encode`] to preserve snapshot format 32;
    /// `sims_vm::boot::save::Saved` preserves it across save/restore.
    #[serde(skip)]
    pub suit_scan_cache: Option<Vec<String>>,
    /// Lazily maintained reverse index from GUID to lowest type ID in
    /// [`World::types`], stored behind [`Arc`] so [`World::clone`] is `O(1)`.
    #[doc(hidden)]
    #[serde(skip)]
    pub types_by_guid: TypesByGuidCache,
    /// Per-object bitmask of lower stack frame depths modified outside
    /// top-frame execution (`idle`, `go_to_slot`, `notify`, `wake`, `create`)
    /// so `sync_top_frame` flushes them at the exact same point full
    /// `sync_frames` would have.
    #[doc(hidden)]
    #[serde(skip)]
    pub dirty_frames: CowBox<BTreeMap<u16, u64>>,
    /// Per-family `FAMI` flags (`+0x138`, keyed by family ID) read by scope 24
    /// fields 7 (`0x425c5e`, bit 0) and 8 (`0x425cb4`, bit 1). Kept out of
    /// [`World::encode`] to preserve snapshot format 32.
    #[serde(skip)]
    pub family_flags: BTreeMap<i32, u32>,
    /// The interface's indicator refresh queues ([`IndicatorQueues`]).
    /// The house file does not hold them. Kept out of [`World::encode`] to
    /// preserve snapshot format 32; `sims_vm::boot::save::Saved` keeps
    /// them across lot save/restore.
    #[serde(skip)]
    pub indicator_queues: IndicatorQueues,
}

/// The people whose indicators the interface will refresh when it next
/// draws the house view (the interface object's vectors +0xb0 and +0xbc
/// and its flag +0xac).
///
/// Resolving a person's data word with write intent, before the word is
/// written, queues the person (0x4538e3, the resolver's person-data case):
///
/// - word 77, the date: 0x4448a6 queues the person unless its word 77 is
///   0 before the write, or the person is already queued;
/// - word 81, fame: 0x444962 queues the person every time.
///
/// Either sets `dirty`, which only the drain (0x4464db) clears.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndicatorQueues {
    /// People whose date indicator to refresh (+0xb0), each once.
    pub dates: Vec<u16>,
    /// People whose star marker to refresh (+0xbc), repeats kept.
    pub stars: Vec<u16>,
    /// +0xac: a refresh is waiting.
    pub dirty: bool,
}

impl IndicatorQueues {
    /// 0x4448a6: queues `person` for a date refresh, given its word 77
    /// before the write. A date of 0 or a person already queued (compared
    /// by object ID, object +0xf0) changes nothing.
    pub fn queue_date(&mut self, person: u16, date: i16) {
        if date == 0 || self.dates.contains(&person) {
            return;
        }
        self.dates.push(person);
        self.dirty = true;
    }

    /// 0x444962: queues `person` for a star refresh.
    pub fn queue_star(&mut self, person: u16) {
        self.stars.push(person);
        self.dirty = true;
    }
}

/// One appearance attachment slot on `cTSPerson` (`0x583559`: suit name at
/// `[slot + 4]`, texture string at `[slot + 0x10]`, and whether the bound
/// appearance defines a valid `"HEAD"` mesh binding).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BoundSlot {
    pub suit: String,
    pub texture: String,
    pub has_head: Option<bool>,
}

/// Cached career workwear descriptor on `cXPerson` (`0x45ee5f`: `[esi + 0xb2c]`
/// job type, `[esi + 0xb2e]` job level, `[esi + 0xb30]` body suit,
/// `[esi + 0xb7c]` body texture, `[esi + 0xbc8]` body_alt suit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkwearCache {
    pub job_type: i16,
    pub job_level: i16,
    pub body_suit: String,
    pub body_tex: String,
    pub body_alt_suit: String,
}

/// Live outfit attachment slots and cached dressing guard on `cTSPerson`
/// (`0x587dfa`, `0x58a178`, `0x591a30`, `0x45ee5f`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonOutfit {
    /// `[ebx + 0x258]` (head slot).
    pub head: Option<BoundSlot>,
    /// `[ebx + 0x25c][0]` (primary body slot).
    pub body: Option<BoundSlot>,
    /// `[ebx + 0x25c][1]` (secondary body slot, e.g. workwear hat/prop).
    pub body_alt: Option<BoundSlot>,
    /// `[ebx + 0x268][0]` (left hand / extra slot).
    pub extra_left: Option<BoundSlot>,
    /// `[ebx + 0x268][1]` (right hand / extra slot).
    pub extra_right: Option<BoundSlot>,
    /// `[ebx + 0x274]` (actually bound accessory attachments).
    pub accessories: Vec<BoundSlot>,
    /// `[ebx + 0x1b0..0x1bc]` (`data[8], data[21], data[22], data[23]`),
    /// initialized to `[-1, -1, -1, -1]` in `cTSPerson::cTSPerson` (`0x587dfa`).
    pub last_dressed: [i32; 4],
    /// `[esi + 0xb2c..0xbc8]` cached workwear descriptors (`0x45ee5f`).
    pub workwear_cache: Option<WorkwearCache>,
}

impl PersonOutfit {
    /// Freshly constructed `cTSPerson` attachment state (`0x587dfa`): all slots
    /// unbound and cached suit/variant/hand words at `-1`.
    pub const fn new() -> Self {
        Self {
            head: None,
            body: None,
            body_alt: None,
            extra_left: None,
            extra_right: None,
            accessories: Vec::new(),
            last_dressed: [-1; 4],
            workwear_cache: None,
        }
    }
}

impl Default for PersonOutfit {
    fn default() -> Self {
        Self::new()
    }
}

/// Which IFF file holds a portrait bitmap persisted by `0x4e9000`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PortraitFile {
    /// A catalog resource file (index into `sims_content::Index::resources`).
    Resource(usize),
    /// A runtime-made character file (`Characters/User%05d.iff`,
    /// [`crate::characters::Character::file`]).
    Made(i32),
}

/// How many advertisements the record of [`World::advertisement_record`]
/// holds: its count (+4) is 16 in a memory image of the running game, and
/// the copy (0x4763c5) never changes it.
pub const ADVERTISEMENTS: usize = 16;

/// One advertisement as the original holds it, in six bytes: a
/// personality number, a minimum and an amount scaled by the personality.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Advertisement {
    pub personality: i16,
    pub minimum: i16,
    pub delta: i16,
}

impl Advertisement {
    /// A field by its place in the original's six bytes: 0 the
    /// personality number, 1 the minimum, 2 the delta.
    pub fn field(&self, field: usize) -> i16 {
        *[self.personality, self.minimum, self.delta]
            .get(field)
            .unwrap_or(&self.delta)
    }

    /// A field to write, by its place as for [`Advertisement::field`].
    pub fn field_mut(&mut self, field: usize) -> &mut i16 {
        match field {
            0 => &mut self.personality,
            1 => &mut self.minimum,
            _ => &mut self.delta,
        }
    }
}

/// The house's usage counts: five earlier days and today, each a count of
/// samples and of the samples in which the person was following a route.
///
/// The original keeps them in the house model (HOUS 1), which the house
/// file holds and the load reads (0x416568; read with
/// [`World::read_house_model`]). In memory they are the six pairs at +0x38
/// of the object the application's first field points to, today's at
/// +0x60. Every tenth tick, each awake member of the household who is not
/// hidden adds a sample (0x4165e3, from the person's tick). The day's
/// rollover moves today into the history (0x41637f), which is not ported.
///
/// Nothing that is ported reads them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HouseUsage {
    /// The five earlier days, the most recent first.
    pub history: [Usage; 5],
    /// Today's.
    pub today: Usage,
}

/// One day's usage counts. Both wrap at 32 bits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Samples (the pair's word at +0).
    pub total: i32,
    /// Samples in which the person had a route context (+4).
    pub routing: i32,
}

impl HouseUsage {
    /// One sample (0x4165e3): `routing` adds to that count first, then the
    /// total goes up.
    pub fn sample(&mut self, routing: bool) {
        if routing {
            self.today.routing = self.today.routing.wrapping_add(1);
        }
        self.today.total = self.today.total.wrapping_add(1);
    }
}

/// The registry's list of dialogs (registry words 0x64 to 0x68), and the
/// one registry word the ordinary dialogs write.
///
/// The Dialog primitive (0x451552) adds a record and later reads back how
/// it was answered. The registry's pass at the end of a lot update
/// (0x4412d1) shows each new record and destroys the ones that are done.
/// What a dialog says and shows is presentation and is not kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dialogs {
    /// The records in the order they were made.
    pub records: Vec<Dialog>,
    /// Registry word 0x100. `cObjectRegistry` initializes it to -1 (0x43f72d),
    /// an ordinary dialog sets it to -1 when its second button is pressed
    /// (0x451a49), a pet-picker dialog writes the chosen pet body index
    /// (0x441d95), and Make New Character's pet branch reads it and resets it
    /// to -1 (0x44b064, 0x44b09b).
    pub declined: i16,
}

impl Default for Dialogs {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            declined: -1,
        }
    }
}

/// One dialog record (0x4443ab makes it; the original's record is far
/// larger, the rest being its text and pictures).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dialog {
    /// The object that asked for it.
    pub caller: u16,
    /// The Dialog instruction's operands, which say what kind of dialog it
    /// is and how it is answered.
    pub operands: [u8; 8],
    /// Record word 0x74: 0 new, 1 shown, 2 to 4 answered with the first,
    /// second or third button, 5 done.
    pub status: u8,
}

/// What the simulation reads of an object's first headline (object
/// +0x148, a record of 0x3c bytes) while it shows: Set Balloon/Headline
/// sets it (0x44b724), the tail of the object's tick counts it down
/// (0x4521f7, 0x46e004), Sleep keeps a showing object out of the
/// scheduler's hands (0x44aa57), and a person's reset clears it
/// (0x45e8af). No save holds it.
///
/// The record's other fields are presentation: the sprite, its frame
/// count and the frame shown, with the step and the tick count that move
/// it (+0x14 to +0x28, which only decide when the object is redrawn,
/// 0x434101), the backdrop and the overlay (+0x2c, +0x30) and the flag
/// (+0x38, which the person's drawn bounds read, 0x45a092). So does the
/// priority while the duration is 0: every call that reads it while a
/// headline shows, and every call that shows one writes it first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Headline {
    /// The priority (+0x34): 2 for an icon of kind 4, 1 for any other. A
    /// headline of a higher priority than a new one keeps showing.
    pub priority: i32,
    /// The duration (+0x10), never 0: the ticks left, or a negative
    /// number for a headline that shows until it is cleared (the tick
    /// counts down only a positive duration).
    pub duration: i32,
}

/// What a person's tick keeps that no save holds (person fields 0x810 and
/// 0xafc).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonTick {
    /// The tick at which the motives last decayed.
    pub last_decay: i32,
    /// The room the person was last seen in, or -5 for none yet.
    pub room: i16,
}

impl PersonTick {
    /// The values the person constructor gives (0x459327 writes 0 to
    /// 0x810 at 0x45942a and -5 to 0xafc at 0x45948b). Nothing else
    /// writes either field but the person's tick (0x45cd32) and the room
    /// step (0x45e492, in 0x45e43d), which the person's tick runs, and so
    /// do the person's visit in a load (0x459c22) and the placement of a
    /// walking person (0x45e4e9).
    pub const CONSTRUCTED: Self = Self {
        last_decay: 0,
        room: -5,
    };
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
    /// How many times the object's thread has been restarted (every frame
    /// dropped) since the lot was loaded. It is not part of the game's
    /// state and is not encoded: the interpreter compares it across one
    /// instruction, to see that a primitive restarted the thread running
    /// it.
    #[serde(skip)]
    pub restarts: u32,
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
    /// The daily budget entries, eight categories each, in the save's
    /// order, which the load keeps (0x46d1e4): today's first (simulator
    /// +0x184, which spending adds to, 0x46d91c), then yesterday and the
    /// four days before it (+0xe4 on). None is a day the save marked with
    /// flag 0; the original then reads seven words for it (0x46d270),
    /// which the parser does not. Empty until a save is read.
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
    /// Set on the Sim the player has selected (the original's object flag
    /// 2, which 0x4456e4 looks for among the people). No save holds it:
    /// after a load nobody is selected until the game selects a Sim
    /// (0x445449).
    pub selected: bool,
}

/// Object data word 29: the room the object was last put in.
const ROOM: usize = 29;

/// Where a Sim is: what [`World::selected_whereabouts`] reads. Only read
/// from the state; nothing here is worked out anew.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Whereabouts {
    /// The Sim's object ID.
    pub id: u16,
    /// Its position: the two coordinates in sixteenths of a tile, and the
    /// level (1 the ground floor, 2 upstairs).
    pub position: [i32; 3],
    /// The tile and level the position is on, as [`World::tile`] gives
    /// them: each coordinate divided by 16, rounding down, and cut to a
    /// signed byte. Nothing is clamped: an object taken off the lot is at
    /// -1, -1 (position -16, -16), but a position outside the lot any other
    /// way gives whatever tile the division does.
    pub tile: [i8; 3],
    /// The level, `position[2]`.
    pub level: i32,
    /// Its room word (object data word 29) as the state holds it: the
    /// room map's number for its tile ([`World::room_at`]), which the
    /// placement writes when the Sim is put somewhere and the room
    /// assignment rewrites when the rooms are worked out again. -5 means no
    /// room: a new object, or one taken off the lot. None only if the object
    /// has no word 29.
    pub room: Option<i16>,
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
            people: Vec::new(),
            types: BTreeMap::new(),
            objects: CowMap::new(),
            arrays: CowMap::new(),
            removals: Vec::new(),
            type_attributes: Vec::new(),
            instances: BTreeMap::new(),
            event_owner: 0,
            magic_history: None,
            indicators: [0; 2],
            indicator_queues: IndicatorQueues::default(),
            changed_types: BTreeSet::new(),
            next_parts: BTreeMap::new(),
            skeleton_files: BTreeSet::new(),
            rooms: Rooms::default(),
            fires: Vec::new(),
            frame_storage: CowMap::new(),
            dialogs: Dialogs::default(),
            person_ticks: BTreeMap::new(),
            neighbors: CowBox::default(),
            animators: BTreeMap::new(),
            usage: HouseUsage::default(),
            routing: Routing::default(),
            action_serial: 0,
            advertisement_record: None,
            families: Vec::new(),
            headlines: BTreeMap::new(),
            base_layers: BTreeMap::new(),
            translated_bones: BTreeMap::new(),
            neighborhood_data: [0; 16],
            held_poses: BTreeMap::new(),
            stops_asked: BTreeSet::new(),
            characters: Characters::default(),
            portraits: BTreeSet::new(),
            dirty_portraits: BTreeSet::new(),
            saved_portraits: BTreeSet::new(),
            generated_portrait_2002: BTreeSet::new(),
            made_portrait_head: BTreeMap::new(),
            ambiguous_portraits: BTreeSet::new(),
            loaded_indexed_skeletons: BTreeMap::new(),
            loaded_indexed_suits: BTreeMap::new(),
            loaded_indexed_suit_bones: BTreeMap::new(),
            made_outfits: BTreeMap::new(),
            person_outfits: BTreeMap::new(),
            scratch_ptrs: ScratchPtrs::new(),
            runtime_added_suits: Vec::new(),
            suit_scan_cache: None,
            types_by_guid: TypesByGuidCache(None),
            dirty_frames: CowBox::default(),
            family_flags: BTreeMap::new(),
        }
    }

    /// Inserts `(id, guid)` into [`World::types`] and updates the cached
    /// reverse GUID index incrementally when present.
    pub fn insert_type(&mut self, id: u16, guid: u32) {
        let prev = self.types.insert(id, guid);
        if prev.is_some() {
            // Overwriting an existing type ID invalidates the reverse map.
            self.types_by_guid.0 = None;
            return;
        }
        if let Some(cache) = self.types_by_guid.0.as_mut() {
            if cache.len + 1 == self.types.len() {
                let inner = Arc::make_mut(cache);
                inner.len = self.types.len();
                inner.first = self.types.first_key_value().map(|(&k, &v)| (k, v));
                inner.last = self.types.last_key_value().map(|(&k, &v)| (k, v));
                inner
                    .map
                    .entry(guid)
                    .and_modify(|existing| {
                        if id < *existing {
                            *existing = id;
                        }
                    })
                    .or_insert(id);
            } else {
                self.types_by_guid.0 = None;
            }
        }
    }

    /// Returns the smallest type ID in [`World::types`] whose GUID equals
    /// `guid` (matching `self.types.iter().find(|&(_, &g)| g == guid)`),
    /// using the lazily-built reverse index.
    pub fn type_id_for_guid(&mut self, guid: u32) -> Option<u16> {
        let valid = self
            .types_by_guid
            .0
            .as_ref()
            .is_some_and(|c| c.matches(&self.types));
        if !valid {
            self.types_by_guid.0 = Some(Arc::new(TypesByGuid::build(&self.types)));
        }
        let cache = self.types_by_guid.0.as_ref().expect("just initialized");
        let id = cache.map.get(&guid).copied()?;
        if self.types.get(&id) == Some(&guid) {
            return Some(id);
        }
        // Fallback rebuild if `world.types` was overwritten in place externally.
        let rebuilt = Arc::new(TypesByGuid::build(&self.types));
        let ans = rebuilt.map.get(&guid).copied();
        self.types_by_guid.0 = Some(rebuilt);
        ans
    }

    /// What the simulator read (0x46cfb3) does next on a lot not zoned
    /// residential (`0x40745f` false, asked with the saved global 10): it
    /// drops both saved streams and seeds each from a reading of the
    /// host's millisecond clock (`timeGetTime`, import slot `0x666400`),
    /// the lot's stream first (`0x46d0d3`, written to lot `+0xac`), then
    /// the global stream (`0x46d0db`, set through `0x46f44d`). A
    /// residential lot instead sets the global stream to its saved state
    /// (`0x46d0ce`). Every SIMI version the port reads is at least 14, the
    /// first to hold the streams (`0x46d096`).
    ///
    /// `readings` are the two clock readings, in that order. The port has
    /// no host clock in the simulation, so a load passes them in.
    pub fn seed_from_host_clock(&mut self, readings: [u32; 2]) {
        self.lot_rng = GameRng::from_state(readings[0]);
        self.global_rng = GameRng::from_state(readings[1]);
    }

    /// Reads a house's usage counts from its house model (HOUS 1;
    /// 0x416568). The file stores each pair as the routing count, then
    /// the total; the five earlier days first, today last. A model of a
    /// version below 0x26 has none, and the original then clears them
    /// (0x4165f3); [`sims_formats::lot::House::parse`] refuses such a
    /// model.
    pub fn read_house_model(&mut self, model: &House) {
        let usage = model.pairs.map(|[routing, total]| Usage { total, routing });
        self.usage = HouseUsage {
            history: [usage[0], usage[1], usage[2], usage[3], usage[4]],
            today: usage[5],
        };
    }

    /// Reads a house's simulator state (SIMI) into the lot the game is
    /// holding (0x46cfb3): the clock, the globals, the scheduler cursor,
    /// both game random streams and the budget. The objects on the lot, its
    /// size and the C runtime stream are not in SIMI and stay as they are.
    ///
    /// The globals are the saved 32 or 64 words as they are;
    /// [`World::apply_context`] turns them into the ones the lot runs with.
    ///
    /// Both streams take their saved states, as on a residential lot. A lot
    /// zoned otherwise reseeds them straight after
    /// ([`World::seed_from_host_clock`]).
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
    pub fn from_house(house: &HouseFile, catalog: &Catalog) -> Result<Self> {
        let mut world = Self::empty();
        world.read_simulator(&house.simulator);
        if let Some(model) = &house.model {
            world.read_house_model(model);
        }
        world.arrays = CowMap::from(house.arrays.clone());
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
        world.people = world.registered_people();
        Ok(world)
    }

    /// The people in registration order: the person vector a load
    /// leaves.
    pub fn registered_people(&self) -> Vec<u16> {
        self.order
            .iter()
            .copied()
            .filter(|id| self.objects.get(id).is_some_and(|o| o.person.is_some()))
            .collect()
    }

    /// Resets person `id`'s animator ([`Animator::reset`]), which empties
    /// its base slot, so the slot's layer ([`World::base_layers`]) goes
    /// with it. Answers whether the person has an animator; a reset that
    /// fails changes nothing.
    pub fn reset_animator(&mut self, id: u16) -> std::result::Result<bool, String> {
        let Some(animator) = self.animators.get_mut(&id) else {
            return Ok(false);
        };
        animator.reset()?;
        self.base_layers.remove(&id);
        Ok(true)
    }

    /// The selected Sim: the first person in registration order with
    /// [`Object::selected`] set (0x4456e4). None while nobody is selected,
    /// as after a load.
    ///
    /// The original walks its list of people. Here that is the registration
    /// order. Which order the original's list has is **unverified**; it
    /// matters only if two people are marked.
    pub fn selected(&self) -> Option<u16> {
        self.order.iter().copied().find(|id| {
            self.objects
                .get(id)
                .is_some_and(|object| object.person.is_some() && object.selected)
        })
    }

    /// Where the selected Sim is, for the presentation to follow: see
    /// [`Whereabouts`]. None while nobody is selected.
    pub fn selected_whereabouts(&self) -> Option<Whereabouts> {
        let id = self.selected()?;
        let object = &self.objects[&id];
        Some(Whereabouts {
            id,
            position: object.position,
            tile: World::tile(object.position),
            level: object.position[2],
            room: object.data.get(ROOM).copied(),
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
        let mut world: Self =
            postcard::from_bytes(bytes).map_err(|e| Error::Snapshot(e.to_string()))?;
        world.mark_all_frame_storage_dirty();
        Ok(world)
    }

    /// Marks every existing [`Self::frame_storage`] entry dirty so each object
    /// performs one full `sync_frames` on its first frame push/pop after a
    /// save/restore (preserving bit-exact parity for lower frames edited before
    /// serialization).
    #[doc(hidden)]
    pub fn mark_all_frame_storage_dirty(&mut self) {
        if self.frame_storage.is_empty() {
            return;
        }
        let dirty = self.dirty_frames.as_mut();
        for &id in self.frame_storage.keys() {
            dirty.insert(id, u64::MAX);
        }
    }

    /// SHA-256 of the canonical encoding, computed via zero-allocation
    /// streaming serialization.
    pub fn hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        crate::hash_flavor::hash_postcard(self, &mut hasher);
        hasher.finalize().into()
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
            selected: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_is_queued_once_and_only_while_set_and_fame_every_time() {
        // 0x4448a6 and 0x444962.
        let mut queues = IndicatorQueues::default();
        queues.queue_date(5, 0);
        assert_eq!(queues, IndicatorQueues::default());
        queues.queue_date(5, -3);
        queues.queue_date(5, 7);
        queues.queue_date(6, 1);
        queues.queue_star(5);
        queues.queue_star(5);
        assert_eq!(
            queues,
            IndicatorQueues {
                dates: vec![5, 6],
                stars: vec![5, 5],
                dirty: true,
            }
        );
    }

    #[test]
    fn scratch_ptrs_serialize_bit_matches_btreeset_postcard_bytes() {
        let mut ptrs = ScratchPtrs::new();
        let mut s_set = BTreeSet::new();
        let mut s_stn = BTreeSet::new();
        let mut s_expr = BTreeSet::new();
        let mut s_arg = BTreeSet::new();

        for (id, mask) in [
            (0u16, SCRATCH_PTR_SET | SCRATCH_PTR_EXPRESSION),
            (1, SCRATCH_PTR_SET_TO_NEXT),
            (42, SCRATCH_PTR_SET | SCRATCH_PTR_ARG_SLOT),
            (4095, SCRATCH_PTR_SET),
            (4096, SCRATCH_PTR_SET | SCRATCH_PTR_EXPRESSION),
            (60000, SCRATCH_PTR_SET_TO_NEXT),
        ] {
            ptrs.set(id, mask);
            if mask & SCRATCH_PTR_SET != 0 {
                s_set.insert(id);
            }
            if mask & SCRATCH_PTR_SET_TO_NEXT != 0 {
                s_stn.insert(id);
            }
            if mask & SCRATCH_PTR_EXPRESSION != 0 {
                s_expr.insert(id);
            }
            if mask & SCRATCH_PTR_ARG_SLOT != 0 {
                s_arg.insert(id);
            }
        }

        // Clearing an ID keeps counts & canonical representation intact.
        ptrs.clear_id(42);
        s_set.remove(&42);
        s_arg.remove(&42);

        for (bit, expected) in [
            (SCRATCH_PTR_SET, &s_set),
            (SCRATCH_PTR_SET_TO_NEXT, &s_stn),
            (SCRATCH_PTR_EXPRESSION, &s_expr),
            (SCRATCH_PTR_ARG_SLOT, &s_arg),
        ] {
            assert_eq!(ptrs.to_btreeset(bit), *expected);
            assert_eq!(ptrs.count_bit(bit), expected.len());
            let actual_bytes = postcard::to_allocvec(&ptrs.serialize_bit(bit)).unwrap();
            let expected_bytes = postcard::to_allocvec(expected).unwrap();
            assert_eq!(actual_bytes, expected_bytes, "bit {bit:#x}");
        }

        let roundtrip = ScratchPtrs::from_btreesets(&s_set, &s_stn, &s_expr, &s_arg);
        assert_eq!(roundtrip, ptrs);
    }

    #[test]
    fn world_streaming_hash_matches_encode_sha256() {
        let mut world = World::empty();
        world.ticks = 12345;
        world.globals[0] = 9;
        world.order = vec![1, 5, 5000];
        world.insert_type(1, 0x1111_2222);
        world.insert_type(5, 0x1111_2222);
        world.insert_type(2, 0x3333_4444);
        assert_eq!(world.type_id_for_guid(0x1111_2222), Some(1));
        assert_eq!(world.type_id_for_guid(0x3333_4444), Some(2));
        assert_eq!(world.type_id_for_guid(0x9999_9999), None);

        let expected: [u8; 32] = Sha256::digest(world.encode()).into();
        assert_eq!(world.hash(), expected);
    }
}
