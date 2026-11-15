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
//! 4. Any other duration shows an icon: a sprite from the kind and index
//!    (0x416a6f), sized by the global sprite's frame count, or for kind 7
//!    the catalog picture of an object, a neighbor or a local; then the
//!    backdrop, the overlay, the flag and the duration from the options,
//!    and the object is redrawn (0x434101) and the lot told
//!    (0x44574b, which looks at the scheduler when the object is in a
//!    Sleep). That fails loudly.
//!
//! The branch is always true.
//!
//! Because showing an icon is not ported, no headline here ever has a
//! duration other than 0, so step 2 always takes the new priority, and a
//! clear changes nothing that is kept: the priority is read only while a
//! duration is not 0, and every call that sets one writes the priority
//! first. So no headline state is kept yet, only whether the object has
//! a headline at all. The other code that reads the list is unaffected
//! for the same reason: Sleep's test (0x44aa57) that the first headline
//! is not showing, the step after each run of an object's thread
//! (0x4521f7, which counts a showing headline down and moves its
//! animation, 0x46e004), and a reset that clears them all (0x45e8af).
//!
//! Ported from the original's code (above); the old prototype's port
//! (legacy/crates/sims-vm/src/headline.rs) agrees. The comparisons that
//! cover it are whole ticks of the Goth house.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};

/// The SLOT entry kind that makes a headline.
const HEADLINE: u16 = 1;

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
        if word(operands, 4) == 0 {
            return Ok(true);
        }
        Err(Trap::Unsupported(format!(
            "Set Balloon/Headline showing an icon of kind {} on object {target} (0x44b7ed), \
             which is not ported",
            operands[3]
        )))
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
