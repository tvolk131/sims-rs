//! Set Balloon/Headline (opcode 41, 0x44b724): show an icon over an
//! object, or clear the one it shows.
//!
//! Operands: byte 0 bit 0 picks the object (the stack object when set,
//! native error 0x15 if it names none; me otherwise), byte 2 is the icon's
//! index and byte 3 its kind, word 2 (bytes 4 and 5) the duration, and
//! word 3 (bytes 6 and 7) options.
//!
//! An object's headlines are a list (object +0x148, records of 0x3c
//! bytes) its construction fills: one for each SLOT entry of kind 1 of
//! its definition (the base constructor, 0x431aaf, parsing the SLOT at
//! 0x46e2d1, which sends kind 0 to the containers and kind 3 to the
//! routing slots), and one more for a person (its post-constructor,
//! 0x459630). Nothing in a house file restores one. The primitive only
//! ever touches the first:
//!
//! 1. With an empty list it does nothing.
//! 2. The priority is 2 for kind 4 and 1 for any other. A headline still
//!    showing (its duration, record +0x10, not 0) with a higher priority
//!    stays, and nothing else happens; otherwise the record takes the new
//!    priority (+0x34).
//! 3. A duration of 0 clears the headline: its duration and its tick
//!    count (+0x28) become 0 (0x46e042).
//! 4. Any other duration shows an icon (from 0x44b7ed). The index is
//!    byte 2, plus my temp 0 (object +0x3a) when option bit 12 is set.
//!    Kind 7 shows a catalog picture (0x44b811), picked by byte 2 as it
//!    is (without the temp): 0, the stack object's; 2, the object a local
//!    names, the local being operand word 0 shifted right by one, signed
//!    (native error 0x33 when it is not below the frame's local count,
//!    frame byte 6; a local still holding half of an old pointer fails
//!    loudly); 1, the neighbor record the stack object names (0x4256f4:
//!    slot `id - 1`), whose type (record +8) is the picture. The reader
//!    sets that type from the catalog by the record's GUID (0x423ef2,
//!    0x43ac10), and a record made in play takes the definition it is
//!    made from (0x4237a1); the reload (0x424806) only reads it. A
//!    missing record or a GUID the catalog lacks ends the call. An
//!    object ID that names nothing on the lot (not positive, or a null
//!    entry of the lot's table), or any other byte 2, ends the call. The picture is the object's
//!    definition's (+0x11c), loaded into its cache (+0x8c, 0x45728d) when
//!    first asked for, and the record's frame count becomes 1 (0x46dfe9);
//!    the backdrop and what follows are as below. Any other kind names a
//!    sprite ([`icon_sprite`], 0x416a0c and 0x416a6f); with none (0) the
//!    call ends here. The sprite's frame count, from the headlines'
//!    sprite cache ([`crate::Program::icon_frames`], 0x4c8deb), divided by
//!    three, is the record's frame count (+0x1c, 0x46dfb5); if that is 0
//!    the call ends here too. Then the backdrop (option byte 0, 0x416bc5),
//!    the overlay (option bit 9) and the flag (option bit 8) are kept, and
//!    the duration set: -1 as it is, otherwise the duration times the
//!    frame count times 12 when option bit 11 is set (0x46e050), or the
//!    duration as it is (0x46e042). Either clears the tick count. The
//!    object is redrawn (0x434101) and woken (0x44574b,
//!    [`Machine::wake`]).
//!
//! The branch is always true. A call that ends early has still written
//! the priority, so a headline already showing goes on showing with it.
//!
//! What the simulation reads of the record is kept in
//! [`sims_sim::World::headlines`]: the priority and the duration, while
//! the duration is not 0. The rest (the sprite and its animation, the
//! backdrop, the overlay and the flag, written by 0x46dfb5 and from
//! 0x44b956 to 0x44b9b9) is drawn, and is left out. The record is read by
//! Sleep's test that the first headline is not showing (0x44aa57), by the
//! tail of an object's tick (0x4521f7, which counts a positive duration
//! down and moves the animation, 0x46e004, redrawing the object when the
//! frame changes), by a person's reset, which clears them all (0x45e8af),
//! and by the drawn bounds of an object (0x437ce8) and a person
//! (0x45a07b).
//!
//! Ported from the original's code (above); the old prototype's port
//! (legacy/crates/sims-vm/src/headline.rs) agrees. The comparisons that
//! cover it are whole ticks of the Goth house.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use crate::program::ICON_FILE;
use sims_sim::Headline;

/// The SLOT entry kind that makes a headline.
const HEADLINE: u16 = 1;

/// The kind whose icon is a catalog picture (0x44b80b).
const PICTURE: u8 = 7;

/// The first sprite of each kind (the table at 0x6c1f98: kind, then the
/// sprite ID at +4), or none (0xffff) for kinds 0 and 7.
const KIND_SPRITES: [u16; 11] = [0xffff, 100, 400, 200, 300, 500, 600, 0xffff, 700, 800, 900];

/// Kind 0's sprites, by index (the table at 0x666758).
const KIND_0_SPRITES: [i16; 19] = [
    200, 201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213, 400, 401, 402, 403, 404,
];

/// The sprite an icon of kind `kind` (not 7) and index `index` shows, or 0
/// for none (0x416a0c, 0x416a6f): the kind's first sprite plus the index,
/// in 16 bits; for kind 0, the index's entry of its own table when the
/// index is from 0 to 18; none for any other kind.
pub(crate) fn icon_sprite(kind: u8, index: i32) -> i16 {
    match KIND_SPRITES.get(usize::from(kind)) {
        Some(&first) if first != 0xffff => first.wrapping_add(index as u16) as i16,
        _ if kind == 0 => usize::try_from(index)
            .ok()
            .and_then(|i| KIND_0_SPRITES.get(i))
            .copied()
            .unwrap_or(0),
        _ => 0,
    }
}

impl Machine<'_, '_> {
    pub(crate) fn set_headline(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let target = if operands[0] & 1 != 0 {
            u16::try_from(self.frame()?.stack_object)
                .ok()
                .filter(|id| self.world.objects.contains_key(id))
                .ok_or(Trap::native(
                    0x15,
                    "Set Balloon/Headline has no stack object",
                ))?
        } else {
            self.me
        };
        if !self.has_headline(target)? {
            return Ok(true);
        }
        let kind = operands[3];
        let priority = if kind == 4 { 2 } else { 1 };
        if let Some(showing) = self.world.headlines.get_mut(&target) {
            if showing.priority > priority {
                return Ok(true);
            }
            showing.priority = priority;
        }
        let duration = i32::from(word(operands, 4));
        if duration == 0 {
            self.world.headlines.remove(&target);
            return Ok(true);
        }
        let options = word(operands, 6) as u16;
        let mut index = i32::from(operands[2]);
        if options & 0x1000 != 0 {
            let temp = *self
                .me()?
                .temps
                .first()
                .ok_or_else(|| Trap::Invalid("the running object has no temp 0".into()))?;
            index += i32::from(temp);
        }
        let frames = if kind == PICTURE {
            if !self.has_picture(operands)? {
                return Ok(true);
            }
            1
        } else {
            let sprite = icon_sprite(kind, index);
            if sprite == 0 {
                return Ok(true);
            }
            self.sprite_frames(sprite)?
        };
        if frames == 0 {
            return Ok(true);
        }
        let duration = if duration != -1 && options & 0x800 != 0 {
            frames.wrapping_mul(duration).wrapping_mul(12)
        } else {
            duration
        };
        if duration == 0 {
            self.world.headlines.remove(&target);
        } else {
            self.world
                .headlines
                .insert(target, Headline { priority, duration });
        }
        self.wake(target)?;
        Ok(true)
    }

    /// Whether kind 7 finds a catalog picture to show (0x44b811 to
    /// 0x44b8d8), by byte 2: the stack object's (0), the neighbor record
    /// the stack object names (1) or a local's object's (2).
    fn has_picture(&self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let id = match operands[2] {
            0 => i32::from(self.frame()?.stack_object),
            1 => {
                // The record the stack object names (0x4256f4) shows its
                // type, record +8: what the catalog held for the record's
                // GUID when the neighborhood was read (0x423ef2, 0x43ac10),
                // or the definition a record made in play was made from
                // (0x4237a1). Only whether there is one is simulation.
                let id = self.frame()?.stack_object;
                return Ok(self
                    .world
                    .neighbors
                    .get(id)
                    .is_some_and(|record| self.catalog.object(record.guid).is_some()));
            }
            2 => {
                let local = word(operands, 0) >> 1;
                let count = self.frame()?.locals.len();
                if usize::try_from(local).is_ok_and(|l| l >= count) {
                    return Err(Trap::native(0x33, "local out of range"));
                }
                let Ok(local) = usize::try_from(local) else {
                    return Err(Trap::Unsupported(format!(
                        "Set Balloon/Headline reading local {local} (0x44b84b): a negative \
                         local reads before the frame's locals"
                    )));
                };
                // The original reads the frame storage's word as it is,
                // which may be half of an old pointer.
                self.check_local(local)?;
                i32::from(self.frame()?.locals[local])
            }
            _ => return Ok(false),
        };
        Ok(u16::try_from(id).is_ok_and(|id| id != 0 && self.world.objects.contains_key(&id)))
    }

    /// The frame count of headline sprite `sprite`: its count in the
    /// headlines' sprite cache (0x4c8deb), divided by three.
    fn sprite_frames(&self, sprite: i16) -> Result<i32, Trap> {
        Ok(self
            .program
            .icon_frames
            .as_ref()
            .ok_or_else(|| {
                Trap::Unsupported(format!(
                    "Set Balloon/Headline showing sprite {sprite}, but the headlines' \
                     sprites ({ICON_FILE}) were not read"
                ))
            })?
            .get(&u16::try_from(sprite).map_err(|_| {
                Trap::Unsupported(format!(
                    "Set Balloon/Headline showing sprite {sprite}: looking up a negative \
                     sprite ID (0x4c8deb) is not ported"
                ))
            })?)
            .copied()
            .unwrap_or(0)
            / 3)
    }

    /// The tail of an object's tick (0x4521f7) for its first headline
    /// (0x46e004): a positive duration counts down, and at 0 the headline
    /// stops showing. Its animation moves too, which only decides when
    /// the object is redrawn.
    pub(crate) fn count_headline(&mut self) {
        if let Some(showing) = self.world.headlines.get_mut(&self.me)
            && showing.duration > 0
        {
            showing.duration -= 1;
            if showing.duration == 0 {
                self.world.headlines.remove(&self.me);
            }
        }
    }

    /// Whether object `id` has a headline: its definition's SLOT has an
    /// entry of kind 1, or it is a person.
    fn has_headline(&self, id: u16) -> Result<bool, Trap> {
        let object = &self.world.objects[&id];
        if object.person.is_some() {
            return Ok(true);
        }
        let definition = self.definition(object.guid)?;
        Ok(self
            .program
            .slots(definition.resource, definition.chunk)
            .iter()
            .any(|slot| slot.kind == HEADLINE))
    }
}
