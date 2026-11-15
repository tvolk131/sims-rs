//! Play Sound Event (opcode 23) and Stop All Sounds (opcode 48).
//!
//! Play Sound Event (0x452822, from the general table at 0x45246a):
//!
//! 1. The object is the frame's stack object when operand byte 4 is not
//!    0xff and has bit 1 set, and me (object field 0xf0) otherwise. Its ID
//!    is taken as it is, without checking that it names an object.
//! 2. Operand word 0 names an FWAV chunk, looked up by ID (0x582c57, type
//!    0x46574156) first in the running code's file and then in the global
//!    file: the frame's file record (frame +0xc) at +8 and then at +4.
//!    Change Suit/Accessory reads the same record for the same two files
//!    (0x45da81 for the running code's, 0x45da8e for the global one).
//! 3. A name that is found is played for the object (0x57a54c); one that
//!    is not plays nothing. Either way the debug log notes it when byte
//!    0x6e509c is set, and with byte 0x6f2a44 set a missing name is
//!    reported. Both bytes are 0 in a memory image of the running game.
//! 4. It returns 1: the true branch, whether or not anything played.
//!
//! The playing is noted as a [`SoundEvent::Play`] ([`crate::sounds`]).
//! When the interface's byte [0x6e48bc]+0x5b is set and I am a person,
//! the original also hands the name to the interface (0x4a698d); that is
//! not modeled.
//!
//! Stop All Sounds (0x45253a, from the same table) takes me (operand word
//! 0 is 0, object field 0xf0) or the stack object, hands its ID to the
//! sound manager's stop (0x57a6fb, which queues a mixer command,
//! 0x573617), notes it in the debug log when that is on (byte 0x6e509c),
//! and returns 1: the true branch. It reads no other state and raises no
//! error, even for an ID that names no object. The stop is noted as a
//! [`SoundEvent::Stop`].
//!
//! The old prototype (legacy/crates/sims-vm/src/lib.rs:1175-1240) reads
//! both primitives the same way.

use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::sounds::SoundEvent;

impl Machine<'_, '_> {
    pub(crate) fn play_sound(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let id = u16::from_le_bytes([operands[0], operands[1]]);
        let frame = self.frame()?;
        let object = if operands[4] != 0xff && operands[4] & 2 != 0 {
            frame.stack_object
        } else {
            self.me as i16
        };
        let owner = self.owner(frame)?;
        let name = [owner, self.program.global].into_iter().find_map(|file| {
            self.program
                .libraries
                .get(&file)
                .and_then(|library| library.sounds.get(&id))
        });
        if let Some(name) = name {
            self.sound(SoundEvent::Play {
                object,
                name: name.clone(),
            });
        }
        Ok(true)
    }

    pub(crate) fn stop_all_sounds(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let object = if i16::from_le_bytes([operands[0], operands[1]]) != 0 {
            self.frame()?.stack_object
        } else {
            self.me as i16
        };
        self.sound(SoundEvent::Stop { object });
        Ok(true)
    }
}
