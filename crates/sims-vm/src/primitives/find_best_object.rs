//! Find Best Object for Function (opcode 14, 0x44f2bf): the object on the
//! lot that serves a function best, weighing its score against its
//! distance, into the stack object.
//!
//! Operands: the function (word 0); flags (byte 6); and, with flag bit 8,
//! the index (word 1) and scope (word 2) of the object to measure from.
//!
//! The function picks a callback (a place in the objects' function tables)
//! and an attribute, an object data word that holds each object's score
//! for it (the table at 0x44f641):
//!
//! | function | callback | score          |
//! |----------|----------|----------------|
//! | 0        | 0x12     | data word 31   |
//! | 1        | 0x13     | data word 32   |
//! | 2        | 0x14     | data word 33   |
//! | 3        | 0x15     | data word 36   |
//! | 6        | 0x18     | data word 37   |
//! | 7        | 0x19     | data word 38   |
//! | 8        | 0x1a     | 50             |
//! | 10       | 0x0e     | data word 61   |
//! | 11       | 0x1c     | data word 39   |
//! | 12       | 0x10     | data word 64   |
//! | 13       | 0x11     | data word 65   |
//! | 14       | 0x1d     | data word 15   |
//!
//! Function 15 takes no callback: it looks for objects of category 4 (data
//! word 59), each scoring 50. Functions 4, 5 and 9, and anything past 15,
//! give false at once. So does function 8 when the running object is not a
//! person (the original's `dynamic_cast` to its person class), or when
//! the person's data word 65 (+0x60e) is above 1.
//!
//! With flag bit 8 the object the variable names is the origin; if no
//! object has that ID, native error 0x17. Without it the origin is the
//! running object. Then every object on the lot, in registration order
//! ([`sims_sim::World::order`], the list at `[lot+0x30]` that Set To Next
//! mode 2 also walks), is skipped if:
//!
//! - it is in use (data word 25 above 0) or has flag 0x200 (data word 8);
//! - with flag bit 1, it is in a room that is not outdoors (room -5, and
//!   a room the lot does not have, count as outdoors); else with flag
//!   bit 2, it has no room (-5) or is in a room that is outdoors; else
//!   with flag bit 4, it is not in the origin's room (data word 29);
//! - its function table has no action for the callback; or its score is
//!   0. The score goes into the running object's data word 66 (+0xce),
//!   even when it is 0;
//! - the entry has a check tree, and the tree gives false, or leaves the
//!   running object's data word 66 at 0. The check tree is the object
//!   type's, run at once on the running object's stack (0x4749b5) with
//!   the object as its stack object and, the arguments pointer being null,
//!   the running object's temps as its arguments. A tree the type lacks
//!   gives false after the mark above the top frame is written.
//!
//! Each object left scores data word 66 less the attenuation ("functional
//! distance attenuation", 3 by default) times its distance from the origin
//! ([`super::distance::distance`], halved past 1 for function 11), and at
//! least 1 (a score that compares unordered, a NaN, counts as 1). The
//! first object with the highest score becomes the stack
//! object and the branch is true; with none the branch is false. The
//! arithmetic is the FPU's in single precision, as Get Distance To's.
//!
//! Ported from the original's code (above). **Unverified** against a
//! capture of the primitive itself; tick 417 of the Goth house is the
//! first that runs it.

use crate::factory::TURN_BUDGET;
use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use crate::primitives::distance::distance;
use crate::program::Tree;
use sims_core::{Ext, F32};
use sims_sim::rooms::NO_ROOM;
use std::cmp::Ordering;

/// Object data words the walk reads.
mod data {
    /// Flags; 0x200 takes the object out of the walk.
    pub const FLAGS: usize = 8;
    pub const DISABLED: i16 = 0x200;
    /// How many are using the object.
    pub const IN_USE: usize = 25;
    pub const ROOM: usize = 29;
    pub const CATEGORY: usize = 59;
    /// The running object's word that holds the score of the object being
    /// weighed (+0xce).
    pub const SCORE: usize = 66;
}

/// The person data word function 8 needs at most 1 (+0x60e).
const PERSON_WORD_65: usize = 65;

/// The score of function 8, and of function 15's objects.
const FIXED_SCORE: i16 = 50;

/// Function 11, whose distances past 1 count half.
const HALVED: i16 = 11;

/// The function whose objects are those of category 4.
const CATEGORY_FUNCTION: i16 = 15;
const CATEGORY: i16 = 4;

/// The function that only a person whose word 65 is at most 1 asks.
const PERSON_FUNCTION: i16 = 8;

/// Definition word 9, the object type; type 7 is never any distance
/// away.
const OBJECT_TYPE: usize = 9;
const NO_DISTANCE_TYPE: i16 = 7;

/// The floats at 0x666748 and 0x666578.
const ONE: f32 = 1.0;
const HALF: f32 = 0.5;

/// What a function looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wanted {
    /// Objects whose table has an action for the callback, scoring the
    /// attribute (data word), or [`FIXED_SCORE`] for None.
    Callback {
        callback: usize,
        attribute: Option<usize>,
    },
    /// Objects of category 4.
    Category,
}

/// The table at 0x44f641; None for the functions that give false.
fn wanted(function: i16) -> Option<Wanted> {
    let (callback, attribute) = match function {
        0 => (0x12, Some(31)),
        1 => (0x13, Some(32)),
        2 => (0x14, Some(33)),
        3 => (0x15, Some(36)),
        6 => (0x18, Some(37)),
        7 => (0x19, Some(38)),
        8 => (0x1a, None),
        10 => (0x0e, Some(61)),
        11 => (0x1c, Some(39)),
        12 => (0x10, Some(64)),
        13 => (0x11, Some(65)),
        14 => (0x1d, Some(15)),
        CATEGORY_FUNCTION => return Some(Wanted::Category),
        _ => return None,
    };
    Some(Wanted::Callback {
        callback,
        attribute,
    })
}

impl Machine<'_, '_> {
    pub(crate) fn find_best_object(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let function = word(operands, 0);
        let Some(wanted) = wanted(function) else {
            return Ok(false);
        };
        if function == PERSON_FUNCTION && !self.may_ask_function_8()? {
            return Ok(false);
        }
        let flags = operands[6];
        let origin = if flags & 8 != 0 {
            let scope = u8::try_from(word(operands, 4)).map_err(|_| {
                Trap::Unsupported(format!(
                    "Find Best Object for Function scope {}",
                    word(operands, 4)
                ))
            })?;
            let id = self.read(scope, word(operands, 2))?;
            u16::try_from(id)
                .ok()
                .filter(|id| self.world.objects.contains_key(id))
                .ok_or(Trap::native(
                    0x17,
                    "Find Best Object for Function object is missing",
                ))?
        } else {
            self.me
        };
        let room = if flags & 4 != 0 {
            Some(object_data(self, origin, data::ROOM)?)
        } else {
            None
        };
        let attenuation = self.program.autonomy_constants().distance_attenuation;
        let one = F32::from_f32(ONE).ext();
        let order = self.world.order.clone();
        let mut best = (Ext::ZERO, None);
        for &id in &order {
            if object_data(self, id, data::IN_USE)? > 0
                || object_data(self, id, data::FLAGS)? & data::DISABLED != 0
            {
                continue;
            }
            let at = object_data(self, id, data::ROOM)?;
            let outdoors = self.world.rooms.rooms.get(&at).map(|room| room.outdoors);
            let in_room = if flags & 1 != 0 {
                at == NO_ROOM || outdoors != Some(false)
            } else if flags & 2 != 0 {
                at != NO_ROOM && outdoors != Some(true)
            } else {
                room.is_none_or(|room| room == at)
            };
            if !in_room {
                continue;
            }
            match wanted {
                Wanted::Category => {
                    if object_data(self, id, data::CATEGORY)? != CATEGORY {
                        continue;
                    }
                    self.set_score(FIXED_SCORE)?;
                }
                Wanted::Callback {
                    callback,
                    attribute,
                } => {
                    let guid = self.world.objects[&id].guid;
                    let definition = self.definition(guid)?;
                    let entry = self
                        .program
                        .libraries
                        .get(&definition.resource)
                        .and_then(|library| library.callbacks.get(&definition.chunk))
                        .and_then(|table| table.get(callback))
                        .filter(|entry| entry.action != 0)
                        .cloned();
                    let Some(entry) = entry else {
                        continue;
                    };
                    let score = match attribute {
                        Some(attribute) => object_data(self, id, attribute)?,
                        None => FIXED_SCORE,
                    };
                    self.set_score(score)?;
                    if score == 0 {
                        continue;
                    }
                    if entry.condition != 0 {
                        let passed = self.check(id, definition.resource, entry.condition)?;
                        if self.world.order != order {
                            return Err(Trap::Unsupported(
                                "a check tree changed the object list Find Best Object for \
                                 Function walks"
                                    .into(),
                            ));
                        }
                        if !passed || object_data(self, self.me, data::SCORE)? == 0 {
                            continue;
                        }
                    }
                }
            }
            let score = Ext::from_i32(i32::from(object_data(self, self.me, data::SCORE)?));
            let from = self
                .world
                .objects
                .get(&origin)
                .ok_or(Trap::native(10, "missing object"))?
                .position;
            let to = &self.world.objects[&id];
            let zero = self.definition_words(to.guid)?[OBJECT_TYPE] == NO_DISTANCE_TYPE;
            let mut span = distance(from, to.position, zero);
            if span.partial_cmp(&one) == Some(Ordering::Greater) && function == HALVED {
                span = (span * F32::from_f32(HALF).ext()).single();
            }
            let mut value = (score - (attenuation.ext() * span).single()).single();
            // `fcom` then `jae` (0x44f5ef): a score that is not at least 1,
            // unordered included, becomes 1. The best is replaced only on
            // `fcom` then not `jbe`, so an unordered score never is.
            if !matches!(
                value.partial_cmp(&one),
                Some(Ordering::Greater | Ordering::Equal)
            ) {
                value = one;
            }
            if value.partial_cmp(&best.0) == Some(Ordering::Greater) {
                best = (value, Some(id));
            }
        }
        let Some(id) = best.1 else {
            return Ok(false);
        };
        self.frame_mut()?.stack_object = id as i16;
        Ok(true)
    }

    /// Puts a score in my data word 66.
    /// Whether the running object may ask for function 8: it is a person
    /// (the original's `dynamic_cast` to its person class, 0x635abe) whose
    /// data word 65 (+0x60e) is at most 1. Find Best Object for Function
    /// and Run Functional Tree both ask.
    pub(super) fn may_ask_function_8(&self) -> Result<bool, Trap> {
        let Some(person) = self.me()?.person.as_ref() else {
            return Ok(false);
        };
        let value =
            person.data.get(PERSON_WORD_65).copied().ok_or_else(|| {
                Trap::Invalid(format!("person has no data word {PERSON_WORD_65}"))
            })?;
        Ok(value <= 1)
    }

    fn set_score(&mut self, score: i16) -> Result<(), Trap> {
        *self
            .me_mut()?
            .data
            .get_mut(data::SCORE)
            .ok_or_else(|| Trap::Invalid(format!("object has no data word {}", data::SCORE)))? =
            score;
        Ok(())
    }

    /// Runs `object`'s check tree `tree` (of the type file `resource`) at
    /// once on my stack, as the walk does.
    fn check(&mut self, object: u16, resource: usize, tree: u16) -> Result<bool, Trap> {
        match self.program.tree(resource, tree) {
            Tree::Missing => {
                // The original finds the tree missing only after it has
                // written its mark above the top frame.
                self.write_mark()?;
                return Ok(false);
            }
            Tree::Primitive(id) => {
                return Err(Trap::Unsupported(format!(
                    "check tree {id:#06x} of object {object}, which names a primitive"
                )));
            }
            Tree::Behavior(..) => {}
        }
        let me = self.me;
        let temps = self.me()?.temps.clone();
        self.interpreter()
            .call_as(
                self.world,
                me,
                object,
                tree,
                object as i16,
                &temps,
                temps.len(),
                TURN_BUDGET,
                false,
            )
            .map_err(|fault| Trap::CheckTree {
                routine: Some("Find Best Object for Function"),
                tree,
                object,
                fault: Box::new(fault),
            })
    }
}

/// Data word `index` of object `id`.
fn object_data(machine: &Machine<'_, '_>, id: u16, index: usize) -> Result<i16, Trap> {
    machine
        .world
        .objects
        .get(&id)
        .ok_or_else(|| Trap::Invalid(format!("registered object {id} does not exist")))?
        .data
        .get(index)
        .copied()
        .ok_or_else(|| Trap::Invalid(format!("object {id} has no data word {index}")))
}
