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
//! The handler is 0x44ba26. Byte 5 picks how the tree runs:
//!
//! - Mode 2: the tree is pushed on the running thread as a frame of the
//!   stack object's type, with the running object's temps as its
//!   arguments. When it returns, the instruction takes the branch of its
//!   result. Before the push the running object is woken (its scheduler
//!   countdown goes back into the Sleep it waits in). A person that runs a
//!   tree this way also starts using the stack object (the person's
//!   virtual function 0x34, 0x45e59b; [`Machine::count_use`]), which its
//!   return ends.
//! - Mode 1: the tree runs at once, to its end, on the stack object's own
//!   thread (0x44bb4b picks that thread; 0x44bc1b calls 0x4749b5). It is a
//!   tree of the stack object's file, the stack object is its own stack
//!   object, and the arguments pointer is null, so the arguments are the
//!   stack object's temps (0x47484a). True takes the true branch and false
//!   the false one; nobody is woken and no use is counted. A tree that
//!   would wait fails, as in the original (native error 0x44f). The
//!   original returns false at once if the stack object's thread is in an
//!   error state (0x4749cc); objects here are never in one.
//! - Mode 0 runs the tree the same way on the running object's own thread;
//!   it is not ported and fails loudly, as does mode 1 when the stack
//!   object is the running object, and a string table other than 303.

use crate::factory::TURN_BUDGET;
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
        let target = target as u16;
        if operands[5] == 1 && target != self.me {
            let temps = self.world.objects[&target].temps.clone();
            let result = self
                .interpreter()
                .call_as(
                    self.world,
                    target,
                    target,
                    tree,
                    target as i16,
                    &temps,
                    temps.len(),
                    TURN_BUDGET,
                    &mut None,
                )
                .map_err(|fault| {
                    Trap::Unsupported(format!(
                        "Run Tree By Name mode 1: tree {tree:#06x} on object {target}'s \
                         thread: {fault}"
                    ))
                })?;
            return Ok(Flow::Branch(result));
        }
        if operands[5] != 2 {
            return Err(Trap::Unsupported(format!(
                "Run Tree By Name mode {} on {} (0x44ba26)",
                operands[5],
                if target == self.me {
                    "the running object's own thread"
                } else {
                    "another object"
                }
            )));
        }
        let temps = self.me()?.temps.clone();
        self.wake(self.me)?;
        self.frame_mut()?.primitive_state = RAN_TREE;
        self.enter_on(target, tree, &temps)?;
        if self.me()?.person.is_some() {
            self.count_use(target)?;
        }
        Ok(Flow::Pushed)
    }
}
