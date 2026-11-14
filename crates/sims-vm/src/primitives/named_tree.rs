//! Run Tree By Name (opcode 28): run a tree of the stack object's type,
//! found by its name.
//!
//! Ported from the old prototype's `run_named_tree`, `named_tree_binding`
//! and `push_target_tree` (legacy/crates/sims-vm/src/person.rs), recovered
//! from the original's 0x44e9d2 and 0x4021f2.
//!
//! Operands: the string table that holds the name (word 0), whether that
//! table is the global file's or the running code's (byte 2 bit 0), the
//! name's number in the table, from 1 (byte 4), and how to run the tree
//! (byte 5).
//!
//! The name is looked up among the trees of the stack object's file, its
//! semi-global file and the global file ([`crate::Program::named_tree`]).
//! No name, or no tree with it, is the false branch.
//!
//! Only one way of running is ported, mode 2: the tree is pushed on the
//! running thread as a frame of the stack object's type, with the running
//! object's temps as its arguments. When it returns, the instruction takes
//! the branch of its result. Before the push the running object is woken
//! (its scheduler countdown goes back into the Sleep it waits in). A
//! person that runs a tree this way also starts using the stack object
//! (the person's virtual function 0x34, 0x45e59b;
//! [`Machine::count_use`]), which its return ends. Modes 0 and 1, which run the tree
//! synchronously or on the stack object's own thread, fail loudly, as does
//! a string table other than 303.

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine, RAN_TREE, word};
use crate::program::TREE_NAMES;

impl Machine<'_, '_> {
    pub(crate) fn run_named_tree(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        let frame = self.frame()?;
        let target = frame.stack_object;
        let table = word(operands, 0) as u16;
        if table != TREE_NAMES {
            return Err(Trap::Unsupported(format!(
                "Run Tree By Name from string table {table}"
            )));
        }
        let strings = if operands[2] & 1 != 0 {
            self.program.global
        } else {
            self.owner(frame)?
        };
        let name = self
            .program
            .libraries
            .get(&strings)
            .and_then(|library| {
                usize::from(operands[4])
                    .checked_sub(1)
                    .and_then(|i| library.tree_names.get(i))
            })
            .filter(|name| !name.is_empty());
        let Some(name) = name else {
            return Ok(Flow::Branch(false));
        };
        if operands[5] > 2 {
            return Err(Trap::native(0x2c, "invalid Run Tree By Name mode"));
        }
        let object = u16::try_from(target)
            .ok()
            .and_then(|id| self.world.objects.get(&id))
            .ok_or(Trap::native(0x15, "Run Tree By Name target is missing"))?;
        let owner = self.definition(object.guid)?.resource;
        let Some(tree) = self
            .program
            .named_tree(owner, name)
            .map_err(Trap::Unsupported)?
        else {
            return Ok(Flow::Branch(false));
        };
        if operands[5] != 2 {
            return Err(Trap::Unsupported(format!(
                "Run Tree By Name mode {}",
                operands[5]
            )));
        }
        let temps = self.me()?.temps.clone();
        self.wake(self.me)?;
        self.frame_mut()?.primitive_state = RAN_TREE;
        self.enter_on(target as u16, tree, &temps)?;
        if self.me()?.person.is_some() {
            self.count_use(target as u16)?;
        }
        Ok(Flow::Pushed)
    }
}
