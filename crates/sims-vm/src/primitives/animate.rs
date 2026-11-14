//! Animate Sim (opcode 44, 0x58950b): a person plays a clip and the
//! script follows its events.
//!
//! The person's own dispatcher runs this opcode on the person's animator
//! (person +0xadc, virtual function 0x28); anything that is not a person
//! fails loudly here, since the general table's version is not ported.
//!
//! Operands: word 0 the clip's number, byte 4 where its name is listed,
//! byte 5 flags, byte 6 how many numbered events the clip sends (signed),
//! byte 2 a local. Flag bits: 0x01 and 0x10 pick the slot (neither the
//! foreground, 0x01 the base, 0x10 the carry, both the foreground with a
//! pickup); 0x02 plays backward; 0x04 takes the number from the parameter
//! word 0 names; 0x08 lets an interrupt end the clip; 0x20 stores events
//! in the local instead of parameter 0; 0x40 makes the clip follow the
//! animator's speed; 0x80 slows the animator's clock.
//!
//! Returns 2 (the thread yields, to run this again next tick) while the
//! animator walks or turns (state above 3) and while the clip plays with
//! nothing to report. Otherwise, in order:
//!
//! 1. If no clip is playing (animator +0xa0), the clip is looked up
//!    (0x5893b3, which first reads the person's appearance again if the
//!    animator says so: [`sims_sim::animator::Animator::profile_stale`]),
//!    the event queue emptied, the foreground slot cleared (0x586b63: a
//!    finished clip left there is removed at once) and the clip started
//!    there (0x592726) at the animator's clock. The
//!    animator's state becomes 0, Animate Sim's count of events seen 0.
//! 2. An event waiting in the queue (virtual function 0x80) is taken. A
//!    numbered event (below 100) must be the next one expected, counting
//!    down when backward; any other is dropped and the thread yields. The
//!    event is stored and the false branch taken.
//! 3. With no event, a clip that has not finished (0x586d41) yields,
//!    unless it may be interrupted and the person wants to be (0x45d7c0,
//!    [`Machine::wants_interrupt`]).
//! 4. A finished or interrupted clip first sends, one per run, the
//!    numbered events it did not send, each stored with the false branch.
//!    Then the clip is done (0x589c25), with the true branch: nothing
//!    plays, the animator takes the state of the person's posture
//!    (0x592904: 3 standing, 1 or 2 for postures 1 and 2), the base layer
//!    is brought up to date (virtual function 0x58, 0x590430, as the
//!    animator's tick does), and the pickup flag (+0x1a4) and person word
//!    19 are cleared. The finished layer stays in the foreground slot
//!    until the next clip clears it.
//!
//! Clip number 0 stops the foreground clip instead (0x5897aa): the
//! foreground slot is cleared as in step 1, the animator takes the state
//! of the person's posture, the pickup flag and person word 19 are
//! cleared, and the true branch is taken. It does not touch whether a
//! clip plays (+0xa0), nor the base layer.
//!
//! **Not ported**, failing loudly: the base and carry slots (flag bits
//! 0x01 and 0x10 alone), names
//! listed anywhere but the global animation tables (byte 4 other than 3),
//! a clip that is missing, the timed events `clip_events.rs` does not
//! port, and a person whose posture
//! is 1 or 2 when its clip is done. The original also notes an unexpected
//! event in its log when logging is on (0x6f2a44); that is not modeled.

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine, word};
use crate::people::read_appearance_if_stale;
use sims_sim::animator::{Animator, LOADED, Layer};

/// Person data words: the posture, and word 19 (person +0x5b2), which
/// the end of a clip clears with the animator's pickup flag.
const POSTURE: usize = 0;
const PICKUP_WORD: usize = 19;

/// The global file's tables of clip names (0x59dd16): an adult's, and a
/// child's (person word 58 at most 17). The original indexes its table of
/// string tables from 0x6f7698 with 8 and 9, which hold these.
const ADULT_CLIPS: u16 = 156;
const CHILD_CLIPS: u16 = 157;

/// Person word 58: the person's age.
const AGE: usize = 58;

impl Machine<'_, '_> {
    pub(crate) fn animate_sim(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        if self.me()?.person.is_none() {
            return Err(Trap::Unsupported(
                "Animate Sim on an object that is not a person: the general primitive table \
                 handles it, which is not ported"
                    .into(),
            ));
        }
        let mut animator = self
            .world
            .animators
            .get(&self.me)
            .cloned()
            .unwrap_or(LOADED);
        let flow = self.animate(&mut animator, operands)?;
        self.world.animators.insert(self.me, animator);
        Ok(flow)
    }

    fn animate(&mut self, a: &mut Animator, operands: &[u8; 8]) -> Result<Flow, Trap> {
        let flags = operands[5];
        let reverse = flags & 0x02 != 0;
        let count = i32::from(operands[6] as i8);
        if !(0..=3).contains(&a.state) {
            return self.yield_here();
        }
        let mut id = word(operands, 0);
        if flags & 0x04 != 0 {
            let parameters = &self.frame()?.parameters;
            id = usize::try_from(id)
                .ok()
                .and_then(|i| parameters.get(i).copied())
                .ok_or(Trap::native(8, "invalid Animate Sim parameter"))?;
        }
        let slot = (flags & 0x01) | if flags & 0x10 != 0 { 2 } else { 0 };
        if slot == 1 || slot == 2 {
            return Err(Trap::Unsupported(format!(
                "Animate Sim on the {} slot (0x589571), which is not ported",
                if slot == 1 { "base" } else { "carry" }
            )));
        }
        if id == 0 {
            // 0x5897aa: the foreground slot is cleared as below (0x586b63),
            // the animator takes the state of the person's posture
            // (0x592904), and the pickup flag and person word 19 are
            // cleared. Whether a clip plays (+0xa0) stays as it is.
            if a.foreground.take().is_some() {
                a.skin_dirty = true;
            }
            a.selection = -1;
            self.posture_state(a)?;
            a.pickup = 0;
            self.clear_pickup_word()?;
            return Ok(Flow::Branch(true));
        }
        if !a.playing {
            // 0x5893b3 first reads the person's appearance again if the
            // animator says so.
            read_appearance_if_stale(self.world, self.program, self.me, &mut a.profile_stale)?;
            let (name, duration, frames, bound, tracks) = self.clip(operands, id)?;
            a.events.clear();
            // 0x586b63: the foreground slot is cleared (0x592878) and the
            // selection set to -1. A layer there is removed from the skin
            // at once (0x5a0a7b), which marks the skin's layers changed.
            // The original fades it out through the blend slot instead
            // when its byte 0x6f6bd0 is set; that byte holds 0 in the
            // memory image of a running game, and the capture shows the
            // blend slot empty after person 18 starts its second clip in
            // tick 24. The fade is not ported.
            if a.foreground.take().is_some() {
                a.skin_dirty = true;
            }
            a.selection = -1;
            a.slow = flags & 0x80 != 0;
            let now = a.clock_at(self.world.ticks as i32);
            let mut layer = Layer::start(&name, duration, frames, bound, tracks, reverse, now);
            // 0x592726: the foreground slot's layer takes the raised
            // priority once. The blend slot (+0x10) is never filled here.
            if a.promote {
                a.promote = false;
                layer.priority += 5;
            }
            a.skin_dirty = true;
            layer.interruptible = i32::from(flags & 0x08 != 0);
            a.foreground = Some(layer);
            a.slow = flags & 0x80 != 0;
            a.follow_rate = flags & 0x40 != 0;
            if slot == 3 {
                // The carry slot is never filled here, so clearing it does
                // nothing.
                a.carry_selection = -1;
                a.pickup = 1;
            }
            a.seen = 0;
            a.state = 0;
            a.selection = -1;
            a.playing = true;
        }
        if !a.events.is_empty() {
            let event = a.events.remove(0);
            if event < 100 {
                let mut expected = if reverse { count - a.seen - 1 } else { a.seen };
                if expected >= count || expected < 0 {
                    expected = -1;
                }
                if event != expected || event < 0 {
                    return self.yield_here();
                }
                a.seen += 1;
            }
            self.store_event(operands, event)?;
            return Ok(Flow::Branch(false));
        }
        let finished = a.foreground.as_ref().is_none_or(Layer::finished);
        if !finished {
            let interruptible = a.foreground.as_ref().is_some_and(|l| l.interruptible != 0);
            if !interruptible || !self.wants_interrupt()? {
                return self.yield_here();
            }
        }
        if a.seen < count {
            let expected = if reverse { count - a.seen - 1 } else { a.seen };
            self.store_event(operands, expected)?;
            a.seen += 1;
            return Ok(Flow::Branch(false));
        }
        // 0x589c25: the clip is done. Nothing plays; the foreground layer
        // stays in its slot.
        a.playing = false;
        self.posture_state(a)?;
        // Virtual function 0x58 (0x590430): the base layer.
        let interpreter = self.interpreter();
        let flags = interpreter.base_flags(self.world, self.me)?;
        interpreter.update_base(self.world, self.me, a, flags)?;
        a.pickup = 0;
        self.clear_pickup_word()?;
        Ok(Flow::Branch(true))
    }

    /// The state of the person's posture (0x592904): 3 standing, 1 or 2
    /// for postures 1 and 2; any other posture leaves the state.
    fn posture_state(&self, a: &mut Animator) -> Result<(), Trap> {
        match self.person_word(POSTURE)? {
            0 => a.state = 3,
            1 => a.state = 1,
            2 => a.state = 2,
            _ => {}
        }
        Ok(())
    }

    /// Person word 19 cleared (person +0x5b2), as the end of a clip does.
    fn clear_pickup_word(&mut self) -> Result<(), Trap> {
        let me = self.me;
        let words = &mut self
            .world
            .objects
            .get_mut(&me)
            .and_then(|o| o.person.as_mut())
            .ok_or(Trap::native(0x1c, "object is not a person"))?
            .data;
        *words
            .get_mut(PICKUP_WORD)
            .ok_or_else(|| Trap::Invalid(format!("person data has no word {PICKUP_WORD}")))? = 0;
        self.world.routing.unknown_words.remove(&(me, PICKUP_WORD));
        Ok(())
    }

    /// Returns 2: the thread yields. A tree that must finish without
    /// yielding fails as Sleep's does.
    fn yield_here(&self) -> Result<Flow, Trap> {
        if self.synchronous {
            return Err(Trap::native(
                0x44f,
                "a synchronous tree yielded in Animate Sim",
            ));
        }
        Ok(Flow::Yield)
    }

    /// Stores an event in the local operand byte 2 names (flag 0x20) or in
    /// parameter 0. Either one missing is native error 8.
    fn store_event(&mut self, operands: &[u8; 8], event: i32) -> Result<(), Trap> {
        let value = event as i16;
        if operands[5] & 0x20 != 0 {
            let local = usize::from(operands[2]);
            let frame = self.frame_mut()?;
            let slot = frame
                .locals
                .get_mut(local)
                .ok_or(Trap::native(8, "invalid Animate Sim local"))?;
            *slot = value;
            self.wrote_local(local);
        } else {
            let frame = self.frame_mut()?;
            let slot = frame
                .parameters
                .first_mut()
                .ok_or(Trap::native(8, "Animate Sim has no parameter to store in"))?;
            *slot = value;
        }
        Ok(())
    }

    /// The clip a number names (0x5893b3): its name, its duration, the
    /// frame count of its first motion, whether a motion binds to a bone,
    /// and how many motions have timed events.
    ///
    /// Only names listed in the global animation tables (byte 4 is 3,
    /// 0x59dd16) are ported: the number counts from 1 into string table
    /// 156, or 157 for a person of age 17 or less. That every motion of the
    /// clip binds to a bone of the person's skeleton is **unverified**; a
    /// clip with motions is taken to bind at least one.
    fn clip(
        &self,
        operands: &[u8; 8],
        id: i16,
    ) -> Result<(String, sims_core::F32, u32, bool, usize), Trap> {
        let source = operands[4] as i8;
        if source != 3 {
            return Err(Trap::Unsupported(format!(
                "Animate Sim naming its clip from source {source} (0x58944b), which is not \
                 ported"
            )));
        }
        let age = self
            .me()?
            .person
            .as_ref()
            .and_then(|p| p.data.get(AGE).copied())
            .unwrap_or(0);
        let table = if age <= 17 { CHILD_CLIPS } else { ADULT_CLIPS };
        let name = usize::try_from(id)
            .ok()
            .and_then(|i| {
                self.program
                    .libraries
                    .get(&self.program.global)
                    .and_then(|g| g.strings.get(&table))
                    .and_then(|s| s.get(i))
            })
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                Trap::Unsupported(format!(
                    "clip {id} is not listed in the global file's string table {table}: the \
                     original reports a missing animation and plays its stand-in (0x5898b6), \
                     which is not ported"
                ))
            })?;
        let clip = self
            .program
            .animations
            .clip(name)
            .map_err(|e| Trap::Unsupported(format!("Animate Sim: {e}")))?;
        Ok((
            name.clone(),
            sims_core::F32::from_bits(clip.duration),
            clip.frames,
            clip.motions > 0,
            clip.tracks.len(),
        ))
    }
}
