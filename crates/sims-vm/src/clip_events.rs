//! A clip's timed events: the layer's update hands each due event to the
//! animator (the callback at 0x6f7690, 0x587c56, which finds the animator
//! through the bone's skeleton), and the animator acts on its properties
//! (0x5912fd).
//!
//! 0x5912fd goes through the event's properties in order. Each value is
//! read as a number (atoi, 0x636029); each key is compared exactly
//! (0x634d50) with:
//!
//! - "xevt": the number goes to the end of the animator's event queue
//!   (virtual function 0x7c, 0x5917eb; +0x2c), where Animate Sim takes it.
//! - "interruptable" or "interruptible": the layer's +0x68
//!   ([`Layer::interruptible`]).
//! - "lefthand", "righthand", "censor": person words 22, 23 and 30
//!   (person +0x5b8, +0x5ba, +0x5c8), as 16 bits.
//! - "discontinuity": a blend layer (+0x10) is removed and the next
//!   foreground layer's priority raised (+0x1a0). No blend layer is made
//!   here (`animator.rs`), so this does nothing.
//! - "sound", "selectedsound", "deselectedsound" and "footstep": a sound
//!   is picked (from the person's view, its selection, or the floor under
//!   it, 0x432595, 0x45e50e, 0x58900f) and played through the sound
//!   events (0x57a54c). Neither the pick nor the play is ported yet
//!   ([`crate::sounds`]), except that in a person's animator refresh at
//!   its taking itself off the lot such an event fails (`timed_event`).
//!   None of 0x57a54c, 0x58900f,
//!   0x432595 and 0x45e50e calls the C runtime's random function
//!   (0x636c74) directly or through direct calls five deep; that nothing
//!   else the simulation keeps changes is **unverified**. "sound" also
//!   writes a line to the game's log when its byte 0x5b is set.
//! - "anchor" (0x5913a6): the event's bone becomes, or stops being, the
//!   anchor of the layer (+0x6c, +0x70 cleared) and, for the foreground
//!   layer, of the animator (+0x28, with +0x24 cleared when it changes).
//!   The animator's tick reads its anchor only behind byte 0x6f6d70
//!   (0x58f6eb, through a local copied from it at 0x58dc9b and
//!   0x58e826), which no code writes and which holds 0 in a memory
//!   capture of the running game; the code there (0x58f739), which moves
//!   the position with the anchor bone, never runs. So the anchor is not
//!   kept. That nothing reads the layer's fields but the pose is
//!   **unverified**.
//! - "dress" and "undress": the value, as written, is put on as an
//!   accessory through the animator's virtual function 0xd8 (0x58d88f)
//!   with an empty binding (0x6de5b0), or taken off through 0xdc
//!   (0x58d919): what Change Suit does
//!   ([`crate::primitives::change_suit`]), but without lowering the name
//!   first. Every such value in the shipped animations is lowercase. The
//!   registry's keys are lowercase (0x5950f5), and whether its lookup
//!   (0x5a044d, 0x5a25dc) ignores case is not worked out, so a name with
//!   an ASCII capital fails.
//!
//! Any other key does nothing.

use crate::fault::Trap;
use crate::interpreter::Interpreter;
use crate::primitives::change_suit::{put_on, undress};
use sims_content::People;
use sims_sim::World;
use sims_sim::animator::{Advance, Animator, Layer};

/// Person data words a timed event writes.
const LEFT_HAND: usize = 22;
const RIGHT_HAND: usize = 23;
const CENSOR: usize = 30;

impl Interpreter<'_> {
    /// Updates `layer`, a layer of person `id`'s animator `a`, at clock
    /// `now` (0x597c4c), and handles the timed events it reaches
    /// ([`Layer::due`]): for each motion of the clip with events, in
    /// order, each due event in turn. Then the event time is committed
    /// ([`Layer::commit`]).
    ///
    /// The clip is looked up again by the layer's name, as it was when the
    /// layer started.
    pub(crate) fn update_layer(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        layer: &mut Layer,
        now: i32,
    ) -> Result<(), Trap> {
        match layer.update(now).map_err(Trap::Unsupported)? {
            Advance::Unbound => return Ok(()),
            Advance::Silent => {}
            Advance::Events if layer.cursors.is_empty() => {}
            Advance::Events => {
                let clip = self
                    .program
                    .animations
                    .clip(&layer.clip)
                    .map_err(|e| Trap::Unsupported(format!("a layer's clip: {e}")))?;
                if clip.tracks.len() != layer.cursors.len() {
                    return Err(Trap::Invalid(format!(
                        "the layer of {:?} has {} cursors for {} motions with events",
                        layer.clip,
                        layer.cursors.len(),
                        clip.tracks.len()
                    )));
                }
                for (track, motion) in clip.tracks.iter().enumerate() {
                    let times: Vec<(i32, bool)> = motion
                        .events
                        .iter()
                        .map(|e| (e.time, !e.properties.is_empty()))
                        .collect();
                    for due in layer.due(track, &times).map_err(Trap::Unsupported)? {
                        timed_event(
                            world,
                            &self.program.people,
                            id,
                            a,
                            layer,
                            &motion.events[due].properties,
                            self.take_off_refresh,
                        )?;
                    }
                }
            }
        }
        layer.commit();
        Ok(())
    }
}

/// One timed event's properties (0x5912fd; the module documentation).
///
/// In a person's animator refresh at its taking itself off the lot
/// (`refresh`; `Interpreter::refresh_at_take_off`), a sound key fails: the
/// refresh is made only if the house view drew the person, which is not
/// kept, and the sound it would play (for a selected person
/// "selectedsound" reaches 0x57a54c through 0x59158c and 0x59175e) is not
/// one the animator's tick alone plays. When the original plays nothing
/// there is not worked out.
fn timed_event(
    world: &mut World,
    people: &People,
    id: u16,
    a: &mut Animator,
    layer: &mut Layer,
    properties: &[(String, String)],
    refresh: bool,
) -> Result<(), Trap> {
    for (key, value) in properties {
        let number = atoi(value);
        let word = match key.as_str() {
            "xevt" => {
                a.events.push(number);
                continue;
            }
            "interruptable" | "interruptible" => {
                layer.interruptible = number;
                continue;
            }
            "lefthand" => LEFT_HAND,
            "righthand" => RIGHT_HAND,
            "censor" => CENSOR,
            "sound" | "selectedsound" | "deselectedsound" | "footstep" if refresh => {
                return Err(Trap::Unsupported(format!(
                    "the clip {:?} has a timed event {key:?} (0x5912fd) due in person {id}'s \
                     animator refresh at its taking itself off the lot (0x58da12), which plays \
                     a sound only if the house view drew the person; that is not kept",
                    layer.clip
                )));
            }
            "dress" | "undress" => {
                if value.bytes().any(|b| b.is_ascii_uppercase()) {
                    return Err(Trap::Unsupported(format!(
                        "the clip {:?} has a timed event {key:?} (0x5912fd) naming {value:?}, \
                         with a capital: whether the suit lookup (0x5a044d) ignores case is \
                         not worked out",
                        layer.clip
                    )));
                }
                let accessories = &mut world
                    .objects
                    .get_mut(&id)
                    .and_then(|o| o.person.as_mut())
                    .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?
                    .accessories;
                if key == "dress" {
                    put_on(people, accessories, value.clone())?;
                } else {
                    undress(accessories, value);
                }
                continue;
            }
            // "discontinuity" finds no blend layer; the anchor is not
            // kept; the sounds are not modeled; other keys do nothing.
            _ => continue,
        };
        let data = &mut world
            .objects
            .get_mut(&id)
            .and_then(|o| o.person.as_mut())
            .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?
            .data;
        *data
            .get_mut(word)
            .ok_or_else(|| Trap::Invalid(format!("person data has no word {word}")))? =
            number as i16;
        world.routing.unknown_words.remove(&(id, word));
    }
    Ok(())
}

/// The C runtime's atoi (0x636029, through atol): spaces and tabs and the
/// other C-locale white space are skipped, then an optional sign, then
/// decimal digits up to the first other character. Overflow wraps, as the
/// runtime's 32-bit arithmetic does.
pub(crate) fn atoi(text: &str) -> i32 {
    let bytes = text.as_bytes();
    let mut i = 0;
    while bytes
        .get(i)
        .is_some_and(|&b| b == b' ' || (9..=13).contains(&b))
    {
        i += 1;
    }
    let negative = bytes.get(i) == Some(&b'-');
    if matches!(bytes.get(i), Some(b'-' | b'+')) {
        i += 1;
    }
    let mut total: i32 = 0;
    while let Some(&b) = bytes.get(i).filter(|b| b.is_ascii_digit()) {
        total = total.wrapping_mul(10).wrapping_add(i32::from(b - b'0'));
        i += 1;
    }
    if negative {
        total.wrapping_neg()
    } else {
        total
    }
}
