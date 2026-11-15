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
//! - Mode 0 runs the tree the same way, but on the running object's own
//!   thread (0x44bb75 picks it): I stay me, the stack object is still its
//!   stack object, and the arguments are my temps. 0x4749b5 treats the
//!   running thread like any other: it pushes a stop frame (tree -1) and
//!   the tree's frame above the frames already there, runs until the stop
//!   frame is reached, and puts back the thread's +4 word it cleared. So
//!   the tree runs above my frames and they are untouched. Mode 1 when the
//!   stack object is me is the same run.
//!
//! In modes 0 and 1, the advertisements a check tree may change (the
//! global record at 0x6c437c) stay the same record through the run.
//!
//! A string table other than 303 fails loudly.

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
        if operands[5] != 2 {
            // Mode 0 runs on my thread and mode 1 on the stack object's.
            let thread = if operands[5] == 0 { self.me } else { target };
            let temps = self.world.objects[&thread].temps.clone();
            // The advertisements a check tree changes are one global record
            // (0x6c437c), which the run keeps.
            let mut advertisements = self.advertisements.take();
            let result = self.interpreter().call_as(
                self.world,
                thread,
                target,
                tree,
                target as i16,
                &temps,
                temps.len(),
                TURN_BUDGET,
                &mut advertisements,
            );
            self.advertisements = advertisements;
            let result = result.map_err(|fault| {
                Trap::Unsupported(format!(
                    "Run Tree By Name mode {}: tree {tree:#06x} of object {target} on \
                         object {thread}'s thread: {fault}",
                    operands[5]
                ))
            })?;
            return Ok(Flow::Branch(result));
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
