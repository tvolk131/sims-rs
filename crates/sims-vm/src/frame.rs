//! What drawing a frame of the house view does to the simulation's state.
//!
//! Between two lot updates the game may draw the lot. `DDDSimsView::Draw()`
//! (0x4d1b37) first brings the action queue on the control panel up to
//! date (0x4d28df, at 0x4d1b8e), then draws the house view.
//!
//! The queue (0x4d28df), while the panel's mode (word +8 of the object at
//! +0x11c) is 2, lists the selected Sim's actions again (0x4ea31c on
//! 0x4e986d's Sim): the running action, then each queued one in order,
//! each only if its icon object is set (0x4ea203; for an icon that is not
//! a person 0x45728d must also find the type's lead definition, which
//! 0x43ae0a always does for an object on the lot). Each action listed is
//! copied into the list (0x4eb1a2, 0x4e87b7), and the copy works out its
//! name (0x41f167), which clears the action's flag 0x80 ([`NAMED`], set
//! when Add/Change Action String or a put-at-the-front names it). The
//! name itself and the queue's pictures are presentation. Which mode 2
//! is, is **unverified**; it is taken to be live mode, the only mode in
//! which the lot runs, so the queue is brought up to date at every frame.
//! In the recording of a snack order, Mortimer's running action from the
//! fridge keeps flag 0x80 from update 872, when he took it, to the frame
//! after update 891.
//!
//! The house view's draw (vtable function 0x3c, 0x4a8935) then counts the
//! rooms that wait (0x4a8895 at 0x4a8b03,
//! which calls 0x467094; [`Interpreter::update_pending_rooms`]). Every
//! path through the draw reaches that call. The rest of 0x4a8895 updates
//! the view's levels and its graphics, which is presentation. The draw
//! then rebuilds its list of objects to draw (0x4aacb9: every object
//! in registration order whose word +0x8e is 0, which is not kept here;
//! the capture holds 0 there for every person) and then draws each
//! (0x4a9262, 0x4ab56d). An object is drawn if its tile is inside the lot
//! but for the border, that is, both tile coordinates from 1 to the lot's
//! size minus 2, and its level is at most the level the view shows. A
//! person one level above the shown one is drawn too if its person word
//! 40 is not 0. A person's draw (virtual function 0x20, 0x45e33e) draws
//! its animator (virtual function 0x78, 0x590c05). Drawing is
//! presentation, but the animator's draw writes state the simulation
//! reads, so it is modeled here, in order:
//!
//! 1. Byte +0xd8 ([`Animator::profile_stale`]): if set, it is cleared and
//!    the person's appearance read again (virtual function 0x14,
//!    0x5888d3), as looking up a clip does.
//! 2. The body (0x58a178 with 0). If the type's definition word 102 has
//!    bit 3 set, the body follows person word 8, which is not ported when
//!    word 8 is above 0. Otherwise, if person word 65 has bit 1 or 2 set,
//!    a word 8 that is not 0 becomes 0 (0x58a2e5). The rest rebuilds the
//!    body's meshes when person words 8, 21, 22 or 23 changed since the
//!    last draw, which is taken to be presentation except that it sets
//!    byte +0xd9 (step 4); that it writes nothing else the simulation
//!    keeps is **unverified**.
//! 3. Byte +0xdb ([`Animator::undrawn`]), the first draw since the
//!    animator was made: it is cleared, and the position unset (virtual
//!    function 0x24, 0x5893a9, calling 0x589361, which also sets +0xd9).
//! 4. Byte +0xd9: if set, it is cleared, the skin updated at the
//!    animator's clock (0x5992b3), and byte +0xda set.
//! 5. Byte +0xda: if set, it is cleared and the animator placed on its
//!    person's tile (virtual function 0x60, 0x592132): each part of the
//!    position that is unset takes the person's, and so does the
//!    destination's.
//! 6. Each draw then runs the head's step, measurement and restoration
//!    (0x590768, 0x5904d2, 0x590b27) as the animator's tick does
//!    (`crate::head`). After the tick the head's clock is the animator's,
//!    so the step is skipped; a person whose head follows something
//!    measures both headings again, against the target as it is now.
//!
//! Bytes +0xd9 and +0xda are not kept: besides the first draw, many
//! places set +0xd9 (the appearance read among them), and none of them is
//! ported as such. Their effects are what matter, and here they are
//! checked instead: the skin update of step 4 changes nothing when the
//! skin is up to date at the animator's clock, and placing changes
//! nothing when no part of the position is unset. A frame that would
//! find either otherwise fails, since whether the original's bytes are
//! set then is not known. After the animator's tick both hold, so in
//! practice only the first draw changes anything.
//!
//! When the game draws a frame is up to the game's main loop and the
//! window system, not the simulation. A capture of the original stops
//! between lot updates, so the frames it draws come at a time the
//! simulation does not decide; the caller says when ([`Interpreter::draw_frame`]).
//!
//! Not modeled, as presentation: the pose (0x5994c3, 0x5a15bc), the draw
//! bounds (virtual function 0x6c), what a person's out-of-view draw
//! writes (virtual function 0x24 of the person, 0x45e414: the animator's
//! screen rectangle, 0x5929cb), and the skin's censor update (0x5a1092).
//! That they change nothing the simulation reads is **unverified**.

use crate::animator::place;
use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::people::read_appearance_if_stale;
use crate::primitives::action_string::NAMED;
use sims_core::F32;
use sims_sim::World;
use sims_sim::animator::{EPSILON, LOADED, UNSET};

/// Person data words the draw reads or writes.
const BODY: usize = 8;
const UPPER_LEVEL: usize = 40;
const KIND: usize = 65;
/// Object data word 1: the direction.
const DIRECTION: usize = 1;
/// The definition word whose bit 3 makes the body follow person word 8
/// (definition +0xcc), and the one whose bit 1 puts the type in the
/// view's other list (definition +0xca, 0x4aad5a).
const DEFINITION_FLAGS: usize = 102;
const DEFINITION_LISTS: usize = 101;

impl Interpreter<'_> {
    /// One frame of the house view, as far as it writes the simulation's
    /// state (the module documentation): the rooms that wait are counted,
    /// then each person the view draws, in registration order, has its
    /// animator drawn. `level` is the level the view shows (1 for the
    /// ground).
    pub fn draw_frame(&self, world: &mut World, level: i8) -> Result<(), Fault> {
        list_queue(world);
        self.update_pending_rooms(world)?;
        let people: Vec<u16> = world
            .order
            .iter()
            .copied()
            .filter(|id| world.objects.get(id).is_some_and(|o| o.person.is_some()))
            .collect();
        for id in people {
            self.draw_person(world, id, level).map_err(|trap| Fault {
                object: id,
                at: None,
                trap,
            })?;
        }
        Ok(())
    }

    /// Person `id`'s draw, if the view draws it (the module
    /// documentation).
    fn draw_person(&self, world: &mut World, id: u16, level: i8) -> Result<(), Trap> {
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::native(10, "missing object"))?;
        let person = object
            .person
            .as_ref()
            .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?;
        let word = |data: &[i16], index: usize| data.get(index).copied().unwrap_or(0);
        let [x, y, z] = World::tile(object.position);
        let size = i32::from(world.size);
        let inside = |t: i8| (1..size - 1).contains(&i32::from(t));
        let drawn = inside(x)
            && inside(y)
            && (z <= level || (z == level + 1 && word(&person.data, UPPER_LEVEL) != 0));
        if !drawn {
            return Ok(());
        }
        let position = object.position;
        let direction = word(&object.data, DIRECTION);
        let guid = object.guid;
        let definition = self.machine(world, id, 0, false).definition_words(guid)?;
        if definition[DEFINITION_LISTS] & 2 != 0 {
            return Err(Trap::Unsupported(
                "a person whose type puts it in the house view's second list (definition word \
                 101 bit 1, 0x4aad5a): how that list is drawn is not ported"
                    .into(),
            ));
        }
        let mut a = world.animators.get(&id).cloned().unwrap_or(LOADED);

        // 1. Byte +0xd8.
        read_appearance_if_stale(world, self.program, id, &mut a.profile_stale)?;

        // 2. The body (0x58a178).
        let data = &mut world
            .objects
            .get_mut(&id)
            .and_then(|o| o.person.as_mut())
            .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?
            .data;
        if definition[DEFINITION_FLAGS] & 8 != 0 {
            if word(data, BODY) > 0 {
                return Err(Trap::Unsupported(
                    "drawing a person whose body follows person word 8 (definition word 102 bit \
                     3, 0x58a1ed) is not ported"
                        .into(),
                ));
            }
        } else if word(data, KIND) & 6 != 0 && word(data, BODY) != 0 {
            data[BODY] = 0;
            world.routing.unknown_words.remove(&(id, BODY));
        }

        // 3. Byte +0xdb: the first draw.
        let first = a.undrawn;
        if first {
            a.undrawn = false;
            a.position = [UNSET; 3];
        }

        // 4. Byte +0xd9: the skin's update, which changes nothing when the
        // skin is up to date at the animator's clock.
        if a.skin_dirty || a.skin_clock != a.clock {
            return Err(Trap::Unsupported(format!(
                "a frame is drawn while person {id}'s skin waits for its update (0x590d26), \
                 which is not ported"
            )));
        }

        // 5. Byte +0xda: the placement (0x592132).
        let unset = |v: F32| (v.ext() - UNSET.ext()).single().abs() <= EPSILON.ext();
        if !first && a.position.iter().any(|&v| unset(v)) {
            return Err(Trap::Unsupported(format!(
                "a frame is drawn while person {id}'s animator is not placed and was drawn \
                 before: whether the draw places it (byte +0xda) is not known"
            )));
        }
        place(&mut a, position, direction);

        // 6. The head (0x590768, 0x5904d2, 0x590b27).
        self.look(world, id, &mut a)?;
        world.animators.insert(id, a);
        Ok(())
    }
}

/// The control panel's action queue listing the selected Sim's actions
/// again (0x4d28df, the module documentation): each action it lists, the
/// running one and the queued ones whose icon object is set, has its flag
/// [`NAMED`] cleared.
fn list_queue(world: &mut World) {
    let Some(sim) = world.selected() else {
        return;
    };
    let Some(person) = world.objects.get_mut(&sim).and_then(|o| o.person.as_mut()) else {
        return;
    };
    for action in std::iter::once(&mut person.active).chain(person.queue.iter_mut()) {
        if action.icon != 0 {
            action.flags &= !NAMED;
        }
    }
}
