//! What the sound scripts may ask about an object (`getsrcdatafield`),
//! as the original answers it.
//!
//! The instruction's handler (0x57f2e8) reads its two source registers,
//! the object's ID and the field, and hands both to the game's callback
//! (0x571127, which the sound manager registers at 0x571651). A field
//! of 21 or more is refused: the handler logs "Illegal source parameter
//! request" and the destination register keeps its value. Any other
//! field goes to the object query (0x570e61), which looks the ID up in
//! the object registry (0x40743b: an ID that is not above 0, or names no
//! object, is none) and casts it to a person:
//!
//! | Field | Answer |
//! | --- | --- |
//! | 0 | 1000 if person data word 74 has bit 2 or 3 set; else 2 if word 58 is below 12; else word 65 |
//! | 1 | person data word 58 |
//! | 2, 3 | person data words 10, 9, divided by 10 |
//! | 4 | global 0, the hour (`[[0x6e48bc]+0x1c]+0x10`, the globals block), for any ID |
//! | 5, 6, 7 | person data words 15, 11, 12, divided by 10 |
//! | 8, 9, 10 | 0 |
//! | 11, 12 | person data words 17, 18, divided by 10 |
//! | 13 | for any object: the size in tiles (half tiles / 2, 0x46810a) of the room its data word 29 names (0x466fcc); 100 for no object or no such room |
//! | 14 | motive 3 (0x45d117) times the single 0.1, truncated (`__ftol`) |
//! | 15 to 20 | person data words 2 to 7, divided by 10 |
//!
//! A field the person cast is needed for answers 0 for an object that is
//! not a person, and so does a negative field (the query's jump table
//! takes it as unsigned and falls to its default). Divisions truncate
//! toward zero (`idiv`).
//!
//! The original asks the live simulation when the script runs. Here the
//! simulation's host takes a [`Snapshot`] of every answer when it has run
//! the lot, and the player reads the last one it was sent while it
//! renders. A host that runs several ticks at once (as `sims-game` does
//! after a slow frame) snapshots after the last, so a sound from an
//! earlier tick of the batch hears a later state, and the mixer's own
//! timing adds more. Nothing the sound does is known to depend on it.

use crate::player::Sources;
use std::collections::BTreeMap;

/// How many fields the query answers (0x571127 refuses 21 and above).
pub const FIELDS: usize = 21;

/// The query's answers for every object, taken between ticks. The
/// default holds no object, with the hour 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Each object's answers, by object ID.
    pub objects: BTreeMap<i32, [i32; FIELDS]>,
    /// The answers for an ID that names no object: field 4 is still the
    /// hour and field 13 is 100; the rest are 0.
    pub absent: [i32; FIELDS],
}

/// Field 13's answer for no object or no room (0x570fba).
pub const NO_ROOM: i32 = 100;

impl Default for Snapshot {
    fn default() -> Self {
        let mut absent = [0; FIELDS];
        absent[13] = NO_ROOM;
        Self {
            objects: BTreeMap::new(),
            absent,
        }
    }
}

impl Sources for Snapshot {
    fn field(&self, source: i32, field: i32) -> Option<i32> {
        if field >= FIELDS as i32 {
            return None;
        }
        let Ok(field) = usize::try_from(field) else {
            return Some(0);
        };
        let answers = self.objects.get(&source).unwrap_or(&self.absent);
        Some(answers[field])
    }
}
