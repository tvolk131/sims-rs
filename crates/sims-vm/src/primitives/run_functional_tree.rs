//! Run Functional Tree (opcode 20, 0x44f681): the running object runs the
//! stack object's action for a function, as a call.
//!
//! Operands: the function (word 0) and a flag (byte 2, bit 0).
//!
//! The stack object must exist; if not, native error 0x15 (thrown before
//! the function is looked at). The function picks a callback, a place in
//! the stack object's function table (the jump table at 0x44f76a):
//!
//! | function | callback |
//! |----------|----------|
//! | 0 to 7   | 0x12 to 0x19 |
//! | 8        | 0x1a     |
//! | 9        | 0x1b     |
//! | 10       | 0x0e     |
//! | 11       | 0x1c     |
//! | 12       | 0x10     |
//! | 13       | 0x11     |
//! | 14       | 0x1d     |
//!
//! Any other function (compared unsigned, so below 0 too) gives false. So
//! does function 8 unless the running object is a person whose data word
//! 65 is at most 1, as for Find Best Object for Function. The callback's
//! action (the first word of its entry, 0x455fa9; 0 for a callback past
//! the table's end) must not be 0, or the result is false.
//!
//! The action tree is then pushed on the running object's thread (its
//! virtual function 0x34, the same push Run Tree By Name's mode 2 makes):
//! the running object is woken (0x452b98, 0x44574b), and the tree, of the
//! stack object's type, runs with the stack object as its stack object
//! and, the arguments pointer being null, the running object's temps as
//! its arguments (0x47484a). A person also starts using the stack object
//! (0x45e59b), noting the flag with the use; a set flag also notes the
//! object at person +0xa54, which is not modeled, so it fails loudly.
//!
//! Unlike Run Tree By Name, the primitive leaves its frame's state alone
//! (0x44f75c returns 3 without writing frame +8), so when the tree returns
//! the instruction takes the branch of its result, as for a call. A tree
//! the type lacks fails loudly here; the original's push gives false.

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine, word};

/// Function 8, which only some people may ask.
const PERSON_FUNCTION: i16 = 8;

/// The callback each function names (0x44f76a); None gives false.
fn callback(function: i16) -> Option<usize> {
    Some(match function {
        0..=7 => 0x12 + function as usize,
        PERSON_FUNCTION => 0x1a,
        9 => 0x1b,
        10 => 0x0e,
        11 => 0x1c,
        12 => 0x10,
        13 => 0x11,
        14 => 0x1d,
        _ => return None,
    })
}

impl Machine<'_, '_> {
    pub(crate) fn run_functional_tree(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        let target = self.frame()?.stack_object;
        let object = u16::try_from(target)
            .ok()
            .and_then(|id| self.world.objects.get(&id))
            .ok_or(Trap::native(
                0x15,
                "Run Functional Tree stack object is missing",
            ))?;
        let guid = object.guid;
        let target = target as u16;
        let function = word(operands, 0);
        let Some(callback) = callback(function) else {
            return Ok(Flow::Branch(false));
        };
        if function == PERSON_FUNCTION && !self.may_ask_function_8()? {
            return Ok(Flow::Branch(false));
        }
        let definition = self.definition(guid)?;
        let tree = self
            .program
            .libraries
            .get(&definition.resource)
            .and_then(|library| library.callbacks.get(&definition.chunk))
            .and_then(|table| table.get(callback))
            .map_or(0, |entry| entry.action);
        if tree == 0 {
            return Ok(Flow::Branch(false));
        }
        let flag = operands[2] & 1 != 0;
        let person = self.me()?.person.is_some();
        if person && flag {
            return Err(Trap::Unsupported(
                "Run Functional Tree with flag 1 notes the object at person +0xa54 \
                 (0x45e5d4), which is not modeled"
                    .into(),
            ));
        }
        let temps = self.me()?.temps.clone();
        self.wake(self.me)?;
        self.enter_on(target, tree, &temps)?;
        if person {
            self.count_use(target)?;
        }
        Ok(Flow::Pushed)
    }
}
