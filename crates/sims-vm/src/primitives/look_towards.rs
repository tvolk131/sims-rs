//! Look Towards (opcode 22, 0x45a98e): a person turns its head, or its
//! body, towards something.
//!
//! The person's own dispatcher runs this opcode; anything that is not a
//! person fails loudly here.
//!
//! Operand word 0 is the mode:
//!
//! - **0**: the head follows the stack object (0x58a035 on the person's
//!   animator): person data word 41 becomes the stack object's number,
//!   word 42 whether it is not 0, words 43 and 45 become 0 and word 44
//!   becomes 1. The branch is true.
//! - **1 to 3**: the person turns on the spot, through a path of its own
//!   tile twice and a point to face (1: one tile towards the camera; 2:
//!   the stack object; 3: as far the other way), which the animator
//!   follows as a route. On the first run (the frame's primitive state
//!   0) a person whose posture (person word 0) is not 0 takes the false
//!   branch; a missing stack object is native error 0x15. Otherwise the
//!   path (+0xae0) becomes those three points, the animator starts the
//!   route (0x589c85) and the primitive state becomes 1. Then, on that
//!   run and the later ones, the animator's route result (0x589d47):
//!   while it is under way (2) the thread yields; else the animator
//!   stops (0x589e2d) and the branch is true. Mode 1 reads no stack
//!   object: its point is one tile towards the camera
//!   ([`camera_point`]), and it clears object data word 9 (0x45ab66).
//! - Any other mode is native error 0x27.
//!
//! The old prototype's `request_turn` (legacy/crates/sims-vm/src/look.rs)
//! and `prepare` (legacy/crates/sims-sim/src/native_turn.rs) port modes 1
//! to 3; this port follows the original's code above.

use crate::animator::ask_stop;
use crate::fault::Trap;
use crate::interpreter::{Flow, Machine, word};
use crate::primitives::find_location::turn_in_square;

/// Person data words the head's target is kept in (person +0x5de to
/// +0x5e6, data at +0x58c).
const HEAD_TARGET: usize = 41;
/// The object data word mode 1 clears (+0x5c, 0x45ab66).
const CLEARED: usize = 9;

/// Mode 1's point to face, one tile towards the camera, as a path point
/// (the second coordinate first), for a person at `[x, y]` with the view
/// turned `rotation` quarter turns (the world's +0x84,
/// [`crate::Interpreter::view_rotation`]): the position is turned by the
/// rotation within the square of 0x400 (0x47bc1f, its first argument the
/// first coordinate), a tile (16, 16) is added (0x422022 with 1, 1), and
/// the sum is turned back by (-rotation) & 3.
fn camera_point([x, y]: [i32; 2], rotation: u8) -> [i32; 2] {
    let rotation = i32::from(rotation);
    let [x, y] = turn_in_square([x, y], rotation);
    let [x, y] = turn_in_square([x.wrapping_add(16), y.wrapping_add(16)], (-rotation) & 3);
    [y, x]
}

impl Machine<'_, '_> {
    pub(crate) fn look_towards(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        if self.me()?.person.is_none() {
            return Err(Trap::Unsupported(
                "Look Towards on an object that is not a person: the general primitive table \
                 handles it, which is not ported"
                    .into(),
            ));
        }
        match word(operands, 0) {
            0 => {
                let target = self.frame()?.stack_object;
                let words = [target, i16::from(target != 0), 0, 1, 0];
                let me = self.me;
                let data = &mut self
                    .me_mut()?
                    .person
                    .as_mut()
                    .ok_or(Trap::native(0x1c, "object is not a person"))?
                    .data;
                let slots = data
                    .get_mut(HEAD_TARGET..HEAD_TARGET + words.len())
                    .ok_or_else(|| Trap::Invalid("person data too short for the head".into()))?;
                slots.copy_from_slice(&words);
                for index in HEAD_TARGET..HEAD_TARGET + words.len() {
                    self.world.routing.unknown_words.remove(&(me, index));
                }
                Ok(Flow::Branch(true))
            }
            mode @ 1..=3 => self.turn_on_the_spot(mode),
            _ => Err(Trap::native(0x27, "invalid Look Towards mode")),
        }
    }

    /// Modes 1 to 3 (0x45a9c1, 0x45aa9d).
    fn turn_on_the_spot(&mut self, mode: i16) -> Result<Flow, Trap> {
        let me = self.me;
        let interpreter = self.interpreter();
        if self.frame()?.primitive_state == 0 {
            if self.person_word(0)? != 0 {
                return Ok(Flow::Branch(false));
            }
            if mode == 1 {
                // 0x45aab9 to 0x45ab66.
                let own = self.me()?.position;
                let at = [own[1], own[0]];
                let to = camera_point([own[0], own[1]], self.effective_view_rotation());
                self.world.routing.routes.entry(me).or_default().path = vec![at, at, to];
                self.set_datum(CLEARED, 0)?;
                interpreter.start_route(self.world, me)?;
                self.frame_mut()?.primitive_state = 1;
                return self.turn_result();
            }
            let target = self.frame()?.stack_object;
            let target = u16::try_from(target)
                .ok()
                .and_then(|id| self.world.objects.get(&id))
                .ok_or(Trap::native(0x15, "Look Towards has no stack object"))?
                .position;
            let own = self.me()?.position;
            // Path points hold the second coordinate first.
            let at = [own[1], own[0]];
            let mut to = [target[1], target[0]];
            if mode == 3 {
                for i in 0..2 {
                    to[i] = to[i].wrapping_sub(to[i].wrapping_sub(at[i]).wrapping_mul(2));
                }
            }
            self.world.routing.routes.entry(me).or_default().path = vec![at, at, to];
            interpreter.start_route(self.world, me)?;
            self.frame_mut()?.primitive_state = 1;
        }
        self.turn_result()
    }

    /// The animator's route result, on every run of modes 1 to 3
    /// (0x45ab80 to 0x45abad).
    fn turn_result(&mut self) -> Result<Flow, Trap> {
        let me = self.me;
        let interpreter = self.interpreter();
        let mut result = interpreter.route_result(self.world, me)?;
        if result == 0 {
            result = 1;
        }
        if result != 2 && !ask_stop(self.world, me)? {
            result = 2;
        }
        match result {
            1 => Ok(Flow::Branch(true)),
            2 => {
                if self.synchronous {
                    return Err(Trap::native(
                        0x44f,
                        "a synchronous tree yielded in Look Towards",
                    ));
                }
                Ok(Flow::Yield)
            }
            other => Err(Trap::Unsupported(format!(
                "Look Towards with the animator's route result {other}"
            ))),
        }
    }
}
