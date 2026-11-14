//! What a person's routing keeps: its route contexts and its path.
//!
//! The Go To Routing Slot primitive (0x45abce) and Go To Relative Position
//! (0x45c634) build a route context (0x48163e) for the person and push it
//! onto the person's list of contexts (person +0xaf0). The planner
//! (0x4823fa) fills the person's path (person +0xae0) and the animator
//! walks it. The person's tick reads the last context (0x45c349) to tell
//! whether the person is routing.
//!
//! Field offsets are the original's. The bytes of a context the original
//! never writes (it leaves what its memory held) are not kept.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The lot's routing state. See [`crate::World::routing`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routing {
    /// The route of each person that has one, by the person's ID.
    pub routes: BTreeMap<u16, Route>,
    /// Slot flags an object's slot table holds that differ from its
    /// type's, by object ID and slot index. Go To Routing Slot sets and
    /// clears flag 0x4000 in the object's own copy of the table
    /// (0x45ac5c).
    pub slot_flags: BTreeMap<(u16, u16), i32>,
    /// Person data words whose value is not known, by person ID and word.
    /// The original wrote them from memory it never initialized: the
    /// planner copies a candidate's word +0xe, which the slot search
    /// never sets, into person word 37 (0x45c418). They hold 0 here; a
    /// script that reads one fails, and a write makes it known again.
    pub unknown_words: BTreeSet<(u16, usize)>,
}

/// A person's route: its contexts and its path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    /// Person +0xaf0: the route contexts, the newest last.
    pub contexts: Vec<Context>,
    /// Person +0xae0: the points of the path, each the second coordinate
    /// (object +0xf4) first and the first (object +0xf8) second.
    pub path: Vec<[i32; 2]>,
}

/// A place the person could stand to use a slot, as the slot search
/// found it (16 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// +0 and +4: the position, the +0xf4 coordinate first.
    pub position: [i32; 2],
    /// +8: how good it is; only candidates above 0 become goals.
    pub score: i32,
    /// Word +0xc: nonzero sends the end of planning down a path that is
    /// not ported.
    pub word_c: i16,
    /// Word +0xe, `None` where the original left it uninitialized.
    pub word_e: Option<i16>,
}

/// A route context (156 bytes, 0x48163e and 0x481693).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// +0: the places found for the slot.
    pub candidates: Vec<Candidate>,
    /// +0xc to +0x48: a copy of the slot's record (fifteen words; see
    /// [`RouteSlot`]).
    pub slot: RouteSlot,
    /// +0x48: the object the slot belongs to.
    pub target: u16,
    /// +0x4c: the person.
    pub person: u16,
    /// +0x50: the index of the candidate the planner chose, or -1.
    pub chosen: i32,
    /// +0x54: the best candidate score, or -1.
    pub best: i32,
    /// +0x58: saved before each step of the primitive and put back,
    /// sign-extended from its low word, after it.
    pub word_58: i32,
    /// +0x5c.
    pub time: i32,
    /// +0x60, +0x64: the person's object +0xf4 and +0xf8 when the context
    /// was made; also saved and put back around each step.
    pub saved: [i32; 2],
    /// +0x68: the door the route goes through, 0 for none. The original
    /// holds a pointer to the door's object; this holds the object's ID.
    pub word_68: i32,
    /// +0x6c, +0x70, +0x74: where the person starts (+0xf4, +0xf8) and
    /// a third word set to 0 (0x4832d4). The constructor (0x48163e) sets
    /// the first two to 0 and leaves the third as the heap held it,
    /// which is 0 here.
    pub start: [i32; 3],
    /// Byte +0x78.
    pub byte_78: bool,
    /// +0x7c, +0x80: two objects the context names, 0 for none.
    pub objects: [u16; 2],
    /// Byte +0x84.
    pub byte_84: bool,
    /// +0x88.
    pub result: i32,
    /// Word +0x8c.
    pub word_8c: i16,
    /// Byte +0x8e.
    pub byte_8e: bool,
    /// Word +0x94: the person's word 72.
    pub word_94: i16,
    /// +0x98: 0x30, or 8 in a room of more than 250 tiles.
    pub limit: i32,
}

/// A slot record as an object's slot table holds it in memory (sixty
/// bytes; the table at object +0x13c). The resource's SLOT entry has
/// three more fields that the record does not keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteSlot {
    /// +0, +4, +8: the offset from the object, as floats' bits.
    pub offset: [u32; 3],
    /// +0xc: the slot's index plus one.
    pub number: i32,
    /// +0x10, +0x14, +0x18.
    pub standing: i32,
    pub sitting: i32,
    pub ground: i32,
    /// +0x1c.
    pub flags: i32,
    /// +0x20.
    pub snap_target: i32,
    /// +0x24, +0x28, +0x2c: the least, greatest and best distance from
    /// the object to stand at (the slot search, 0x483598, reads them in
    /// that order).
    pub proximity: [i32; 3],
    /// +0x30, as a float's bits.
    pub gradient: u32,
    /// +0x34: the facing.
    pub facing: i32,
    /// +0x38.
    pub resolution: i32,
}
