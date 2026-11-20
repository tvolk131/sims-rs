//! Reach (opcode 47, 0x45c9ba): a person reaches for an object, plays a
//! reach clip, and at the clip's grab event takes the object into its
//! hand or puts what it holds into a slot.
//!
//! The person's own dispatcher (0x45cbb6) runs this opcode. For anything
//! that is not a person the general table (0x45246a) has no entry for 47:
//! its default throws native error 5 (0x452455).
//!
//! Operands: word 0 the mode, word 1 whether to move the object (nonzero
//! moves it), word 2 (mode 1) the index of the frame parameter that holds
//! the slot. The frame's primitive state (+8) is the step:
//!
//! - **0.** What to reach for ([`Machine::reach_target`], 0x45c848). If
//!   that fails, the false branch. Otherwise the reach clip starts
//!   ([`Machine::start_reach`], 0x589ef3): the step becomes 1 and the
//!   thread yields. If no clip starts, the true branch when word 1 is 0
//!   and the false branch otherwise (0x45ca7f).
//! - **1.** Every event waiting in the animator's queue is taken
//!   (virtual function 0x80, 0x5864ce). Unless one of them was event 0,
//!   or the foreground clip is done (0x58a000, 0x586d41; an empty slot is
//!   done), the thread yields. Then the target is worked out again; if
//!   that fails, the branch is as for a clip that did not start. With
//!   word 1 at 0 the step becomes 2. Otherwise, by mode:
//!   - 0: the target's place test for my slot 0, at my position and
//!     level (virtual function 0x54, [`crate::Interpreter::fits_in`]). If
//!     it fits, the target is put there with the lot told (virtual
//!     function 0x58, its last argument 1), the target's "CT - Grab" tree
//!     runs (0x452bed), and the step becomes 2. If not, the step becomes
//!     3.
//!   - 1: the same for what I hold in my slot 0, into the stack object's
//!     slot, at the stack object's position and level, with "CT - Put".
//!     Holding nothing makes the step 3.
//!   - any other mode leaves the step at 1.
//!
//!   The thread then yields.
//! - **2** and **3.** While the foreground clip is not done the thread
//!   yields. Then the reach ends (0x58a00d): the foreground slot is
//!   cleared (virtual function 0xfc, 0x586b63), the pickup flag (+0x1a4)
//!   cleared, the event queue emptied (0x407161), and the animator takes
//!   the state of the person's posture (0x592904). Step 2 takes the true
//!   branch, step 3 the false.
//!
//! The "CT - Grab" and "CT - Put" trees are looked up by name in the
//! moved object's file, then its semi-global and the global file
//! (0x4021f2), and run at once on my stack with the moved object's code
//! and as the stack object, and my temps as the arguments (0x4749b5 with
//! no arguments). What they return is not read; a missing tree runs
//! nothing.
//!
//! A put that fails partway (on what the port does not model) leaves the
//! lot as it was before it, as for Grab, and fails loudly.
//!
//! **Unverified**: no capture covers a Reach. The base game's trees use
//! it in food (getting food, picking plates up, the food slot test and
//! placement), the bills, the flowers, the bar and the thief.

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine, word};
use crate::people::read_appearance_if_stale;
use crate::placement::{Place, capture, on_lot};
use core::cmp::Ordering;
use sims_core::{Ext, F32};
use sims_sim::animator::{Animator, LOADED, Layer};

/// Person data words: the posture (+0x58c) and the age (+0x600).
const POSTURE: usize = 0;
const AGE: usize = 58;
/// The global file's tables of reach clips (0x59dcd4, the global clip
/// tables 4 and 5 from 0x6f7698): an adult's, and a child's (age 17 or
/// less).
const ADULT_REACHES: u16 = 154;
const CHILD_REACHES: u16 = 155;
/// The kind of a SLOT entry that is a container slot.
const CONTAINER: u16 = 0;
/// The trees the moved object runs (0x6c45a0, 0x6c4594).
const GRAB_TREE: &str = "CT - Grab";
const PUT_TREE: &str = "CT - Put";
/// The height mode 2 reaches at (0x666c18).
const NO_HEIGHT: F32 = F32::from_bits(0xbf80_0000);
/// The height of a slot in units (0x454f29 times 3, 0x667344).
const THREE: F32 = F32::from_bits(0x4040_0000);

/// What Reach reaches for (0x45c848).
struct Target {
    /// The object it moves: in mode 0 the stack object, in mode 1 what I
    /// hold in my slot 0.
    object: Option<u16>,
    /// The container it reaches into: in mode 0 the stack object's, in
    /// mode 1 the stack object.
    container: Option<u16>,
    /// That container's slot, as scripts count it.
    slot: i32,
    /// How high the reach goes.
    height: F32,
}

impl Machine<'_, '_> {
    pub(crate) fn reach(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        if self.me()?.person.is_none() {
            return Err(Trap::native(
                5,
                "Reach on an object that is not a person: the general primitive table has no \
                 entry for it",
            ));
        }
        let mode = word(operands, 0);
        let moves = word(operands, 2) != 0;
        let fail = Flow::Branch(!moves);
        match self.frame()?.primitive_state {
            0 => {
                let Some(target) = self.reach_target(operands)? else {
                    return Ok(Flow::Branch(false));
                };
                if !self.start_reach(target.height)? {
                    return Ok(fail);
                }
                self.frame_mut()?.primitive_state = 1;
                self.reach_yield()
            }
            1 => {
                let mut a = self.reach_animator();
                let mut grab = false;
                for event in std::mem::take(&mut a.events) {
                    grab |= event == 0;
                }
                grab |= done(&a);
                self.world.animators.insert(self.me, a);
                if !grab {
                    return self.reach_yield();
                }
                let Some(target) = self.reach_target(operands)? else {
                    return Ok(fail);
                };
                let step = if !moves {
                    2
                } else {
                    match mode {
                        0 => self.reach_move(target.object, self.me, 0, GRAB_TREE)?,
                        1 => match target.container {
                            Some(container) => {
                                self.reach_move(target.object, container, target.slot, PUT_TREE)?
                            }
                            None => 3,
                        },
                        _ => 1,
                    }
                };
                self.frame_mut()?.primitive_state = step;
                self.reach_yield()
            }
            step @ (2 | 3) => {
                let mut a = self.reach_animator();
                if !done(&a) {
                    return self.reach_yield();
                }
                self.end_reach(&mut a)?;
                self.world.animators.insert(self.me, a);
                Ok(Flow::Branch(step == 2))
            }
            step => Err(Trap::Unsupported(format!(
                "Reach resumed at step {step}, which it never sets (0x45cba9)"
            ))),
        }
    }

    fn reach_animator(&self) -> Animator {
        self.world
            .animators
            .get(&self.me)
            .cloned()
            .unwrap_or(LOADED)
    }

    /// Returns 2: the thread yields. A tree that must finish without
    /// yielding fails as Sleep's does.
    fn reach_yield(&self) -> Result<Flow, Trap> {
        if self.synchronous {
            return Err(Trap::native(0x44f, "a synchronous tree yielded in Reach"));
        }
        Ok(Flow::Yield)
    }

    /// What to reach for, and how high (0x45c848), or None when there is
    /// nothing to reach for:
    ///
    /// - Mode 0: the stack object (looked up in the registry, 0x40743b),
    ///   and the container and slot it is in (0x43265e).
    /// - Mode 1: the stack object as the container, the slot my frame's
    ///   parameter word 2 names (native error 8 for no such parameter),
    ///   and what my slot 0 holds (none when it holds nothing).
    /// - Mode 2: nothing, at height -1.
    /// - Any other mode: None.
    ///
    /// In modes 0 and 1 a missing stack object gives None. With no
    /// container the height is 0. Otherwise a slot that is not one of the
    /// container's container slots gives None, and the height is that of
    /// the slot ([`stacked_height`]) in units: times 1/(2√6) (0x454f29)
    /// and 3.
    fn reach_target(&mut self, operands: &[u8; 8]) -> Result<Option<Target>, Trap> {
        let stack_object = self.registered(self.frame()?.stack_object);
        let (object, container, slot) = match word(operands, 0) {
            0 => {
                let Some(object) = stack_object else {
                    return Ok(None);
                };
                let place = capture(self.world, object)?;
                (Some(object), place.container, place.slot)
            }
            1 => {
                let Some(container) = stack_object else {
                    return Ok(None);
                };
                let index = word(operands, 4);
                let slot = usize::try_from(index)
                    .ok()
                    .and_then(|i| self.frame().ok()?.parameters.get(i).copied())
                    .ok_or(Trap::native(8, "Reach names no parameter"))?;
                let held = self.me()?.slots.get(1).map_or(0, |s| s.object);
                (self.registered(held), Some(container), i32::from(slot))
            }
            2 => {
                return Ok(Some(Target {
                    object: None,
                    container: None,
                    slot: 0,
                    height: NO_HEIGHT,
                }));
            }
            _ => return Ok(None),
        };
        let height = match container {
            None => F32::ZERO,
            Some(container) => {
                let count = self.world.objects[&container].slots.len() as i32 - 1;
                if slot < 0 || slot >= count {
                    return Ok(None);
                }
                let height = self.stacked_height(container, slot)?;
                ((unit().ext() * height.ext()).single() * THREE.ext()).to_f32()
            }
        };
        Ok(Some(Target {
            object,
            container,
            slot,
            height,
        }))
    }

    /// An object ID as the registry looks it up (0x40743b): one above 0
    /// that names an object.
    fn registered(&self, id: i16) -> Option<u16> {
        u16::try_from(id)
            .ok()
            .filter(|id| *id > 0 && self.world.objects.contains_key(id))
    }

    /// The height of `container`'s slot `slot` (0x43404d): the slot's
    /// height ([`Machine::slot_height`]), plus that of the slot the
    /// container is in, and so on up the containers (0x4325b5), each sum
    /// stored as a float. A slot that is not one of its container's
    /// container slots adds nothing.
    fn stacked_height(&mut self, container: u16, slot: i32) -> Result<F32, Trap> {
        let mut height = F32::ZERO;
        let (mut container, mut slot) = (container, slot);
        let mut seen = std::collections::BTreeSet::new();
        loop {
            if !seen.insert(container) {
                return Err(Trap::Invalid(format!(
                    "the containers above object {container} form a loop"
                )));
            }
            let count = self.world.objects[&container].slots.len() as i32 - 1;
            if (0..count).contains(&slot) {
                height = (height.ext() + self.slot_height(container, slot)?.ext()).to_f32();
            }
            let place = capture(self.world, container)?;
            match place.container {
                Some(next) => (container, slot) = (next, place.slot),
                None => return Ok(height),
            }
        }
    }

    /// The height of `container`'s container slot `slot` (record +8): for
    /// one of its definition's SLOT container entries, as the record's
    /// height code sets it (0x46def8): 0 for code 1, 2.5 for 2, 4 for 3
    /// and 8, 4.05 for 4, and the entry's third offset for any other. A
    /// person's own five records are at 0 (0x46de93).
    fn slot_height(&self, container: u16, slot: i32) -> Result<F32, Trap> {
        let object = &self.world.objects[&container];
        let definition = self.definition(object.guid)?;
        let entries: Vec<_> = self
            .program
            .slots(definition.resource, definition.chunk)
            .iter()
            .filter(|entry| entry.kind == CONTAINER)
            .collect();
        let own = if object.person.is_some() {
            crate::objects::PERSON_SLOTS
        } else {
            0
        };
        if entries.len() + own + 1 != object.slots.len() {
            return Err(Trap::Invalid(format!(
                "object {container} keeps {} slots, but its SLOT has {} containers and it adds \
                 {own} of its own",
                object.slots.len(),
                entries.len(),
            )));
        }
        let Some(entry) = usize::try_from(slot).ok().and_then(|s| entries.get(s)) else {
            return Ok(F32::ZERO);
        };
        Ok(match entry.height {
            1 => F32::ZERO,
            2 => F32::from_bits(0x4020_0000),
            3 | 8 => F32::from_bits(0x4080_0000),
            4 => F32::from_bits(0x4081_999a),
            _ => F32::from_f32(entry.offset[2]),
        })
    }

    /// Starts the reach clip for `height` (0x589ef3), and returns whether
    /// it started:
    ///
    /// 1. The clip's number by height ([`band`], 0x59265c); none above
    ///    the highest band, and no clip starts.
    /// 2. The animator reads the person's appearance again if it says so
    ///    (+0xd8, virtual function 0x14).
    /// 3. Its name in the global reach table (154, or 155 for a person of
    ///    age 17 or less, 0x59dcd4) and the clip (0x59dc32).
    /// 4. The event queue is emptied (0x407161) and the clip starts in
    ///    the foreground slot, forward (virtual function 0x100, 0x592726).
    /// 5. The selection (+0x98) becomes -1, the state 0 and the pickup
    ///    flag 1.
    ///
    /// The original also works out whether the person's hand holds
    /// something (0x589fce) and hands it to 0x59265c, which does not read
    /// it. A name that is not listed, or is empty, is no clip (0x59dd09
    /// hands 0x59dc32 a null name, 0x59dc45 skips the load), so the start
    /// fails (0x589f45) after the appearance re-read, with no other write.
    /// A listed clip our content lacks fails loudly (the original reports
    /// a missing animation and plays a stand-in, as for Animate Sim).
    fn start_reach(&mut self, height: F32) -> Result<bool, Trap> {
        let me = self.me;
        let Some(number) = band(height, self.person_word(POSTURE)? == 1) else {
            return Ok(false);
        };
        let mut a = self.reach_animator();
        read_appearance_if_stale(self.world, self.program, me, &mut a.profile_stale)?;
        let table = if self.person_word(AGE)? <= 17 {
            CHILD_REACHES
        } else {
            ADULT_REACHES
        };
        let Some(name) = self
            .program
            .libraries
            .get(&self.program.global)
            .and_then(|g| g.strings.get(&table))
            .and_then(|s| s.get(number))
            .filter(|name| !name.is_empty())
            .cloned()
        else {
            self.world.animators.insert(me, a);
            return Ok(false);
        };
        let clip = self
            .program
            .animations
            .clip(&name)
            .map_err(|e| Trap::Unsupported(format!("Reach: {e}")))?;
        a.events.clear();
        let now = a.clock_at(self.world.ticks as i32);
        let layer = Layer::start(
            &name,
            F32::from_bits(clip.duration),
            clip.frames,
            clip.motions > 0,
            clip.tracks.len(),
            false,
            now,
        );
        start_foreground(&mut a, layer);
        a.selection = -1;
        a.state = 0;
        a.pickup = 1;
        self.world.animators.insert(me, a);
        Ok(true)
    }

    /// The reach's end (0x58a00d): the foreground slot cleared (0x586b63:
    /// the layer removed from the skin, the selection -1), the pickup
    /// flag cleared, the event queue emptied, and the state of the
    /// person's posture.
    fn end_reach(&self, a: &mut Animator) -> Result<(), Trap> {
        a.clear_foreground();
        a.pickup = 0;
        a.events.clear();
        self.posture_state(a)
    }

    /// Moves `object` into `container`'s slot `slot` and runs its `tree`,
    /// and returns the next step: 2 when it moved, 3 when there is no
    /// object or the place test refuses (0x45ca9e to 0x45cb60).
    fn reach_move(
        &mut self,
        object: Option<u16>,
        container: u16,
        slot: i32,
        tree: &str,
    ) -> Result<i32, Trap> {
        let Some(object) = object else {
            return Ok(3);
        };
        let me = self.me;
        let [p0, p1, level] = self
            .world
            .objects
            .get(&container)
            .ok_or(Trap::native(10, "missing object"))?
            .position;
        let position = [p0, p1];
        let interpreter = self.interpreter();
        let unsupported = |fault| {
            Trap::Unsupported(format!(
                "Reach by object {me} moving object {object} into object {container}: {fault}"
            ))
        };
        if !interpreter
            .fits_in(self.world, object, container, slot, position, level)
            .map_err(unsupported)?
        {
            return Ok(3);
        }
        let before = self.world.clone();
        let put = interpreter.put(
            self.world,
            object,
            Place {
                on_lot: on_lot(position),
                position,
                container: Some(container),
                slot,
                level,
            },
            true,
        );
        if let Err(fault) = put {
            *self.world = before;
            return Err(unsupported(fault));
        }
        self.reach_tree(object, tree)?;
        Ok(2)
    }

    /// Runs `object`'s tree named `name` at once on my stack (0x452bed).
    fn reach_tree(&mut self, object: u16, name: &str) -> Result<(), Trap> {
        let guid = self.world.objects[&object].guid;
        let owner = self.definition(guid)?.resource;
        let Some(tree) = self
            .program
            .named_tree(owner, name)
            .map_err(Trap::Unsupported)?
        else {
            return Ok(());
        };
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
                crate::TURN_BUDGET,
                self.checking,
            )
            .map_err(|fault| {
                Trap::Unsupported(format!(
                    "Reach: the {name:?} tree {tree:#06x} of object {object}, run by person \
                     {me}: {fault}"
                ))
            })?;
        Ok(())
    }
}

/// Whether the foreground clip is done (0x58a000, 0x586d41): an empty
/// slot is.
fn done(a: &Animator) -> bool {
    a.foreground.as_ref().is_none_or(Layer::finished)
}

/// A clip started in the foreground slot (0x592726): the slot cleared
/// first (virtual function 0x104, 0x592878: a layer there is removed from
/// the skin at once, which marks the skin's layers changed), the layer
/// takes the raised priority once (+0x1a0), and it goes in before the
/// carry layer (priority 10). The blend slot (+0x10) is never filled here.
pub(super) fn start_foreground(a: &mut Animator, mut layer: Layer) {
    if a.foreground.take().is_some() {
        a.skin_dirty = true;
    }
    if a.promote {
        a.promote = false;
        layer.priority += 5;
    }
    a.skin_dirty = true;
    a.foreground = Some(layer);
    a.carry_first = false;
}

/// 1/(2√6) (0x454f29, worked out once into 0x703f2c): the square root
/// of 6, doubled, divided into 1, each step under the game's control word,
/// stored as a float.
fn unit() -> F32 {
    let root = Ext::from_i32(6).sqrt().single();
    let twice = (root + root).single();
    (Ext::from_i32(1) / twice).to_f32()
}

/// The number of the reach clip for a height (0x59265c): below 0, 9;
/// below 0.75, 1; below 2, 3; below 2 2/3, 5; below 4, 7; otherwise none.
/// One more for a sitting person (posture 1). Each test is `jae` after
/// `fcomp`, so a height that is not a number counts as below 0.
pub(crate) fn band(height: F32, sitting: bool) -> Option<usize> {
    let h = height.ext();
    let below = |bits: u32| {
        !matches!(
            h.partial_cmp(&F32::from_bits(bits).ext()),
            Some(Ordering::Greater | Ordering::Equal)
        )
    };
    let number = if below(0) {
        9
    } else if below(0x3f40_0000) {
        1
    } else if below(0x4000_0000) {
        3
    } else if below(0x402a_aaab) {
        5
    } else if below(0x4080_0000) {
        7
    } else {
        return None;
    };
    Some(number + usize::from(sitting))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_height_unit_is_one_over_twice_root_six() {
        assert_eq!(unit().to_bits(), (1.0f32 / (2.0 * 6.0f32.sqrt())).to_bits());
    }

    #[test]
    fn heights_pick_the_reach_clip() {
        let f = |x: f32| F32::from_f32(x);
        assert_eq!(band(f(-1.0), false), Some(9));
        assert_eq!(band(f(-1.0), true), Some(10));
        assert_eq!(band(F32::ZERO, false), Some(1));
        assert_eq!(band(f(0.749), true), Some(2));
        assert_eq!(band(f(0.75), false), Some(3));
        assert_eq!(band(f(2.0), false), Some(5));
        assert_eq!(band(F32::from_bits(0x402a_aaab), false), Some(7));
        assert_eq!(band(f(4.0), false), None);
        assert_eq!(band(F32::from_bits(0x7fc0_0000), false), Some(9));
    }
}
