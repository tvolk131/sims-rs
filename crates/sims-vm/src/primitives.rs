//! Primitives: the instructions below opcode 256, built into the
//! interpreter. Each returns which branch to take. A primitive with no
//! implementation fails with [`Trap::Unimplemented`], which names it.

mod create;
pub(crate) mod dialog;
mod distance;
mod events;
mod expression;
mod generic;
mod idle;
mod inventory;
mod named_tree;
mod notify;
mod object_type;
mod random;
mod refresh;
mod relationship;
mod remove;
mod set_to_next;
mod sleep;
mod sound;

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine};
use sims_formats::behavior::Instruction;

/// Primitive names, by opcode. Names the old prototype dispatched on are
/// confirmed by its behavior (legacy/crates/sims-vm/src/lib.rs:1130-1350,
/// 1528); the rest follow the community's SimAntics references and only
/// label messages.
const NAMES: [&str; 52] = [
    "Sleep",
    "Generic Sims Call",
    "Expression",
    "Find Best Interaction",
    "Grab",
    "Drop",
    "Change Suit/Accessory",
    "Refresh",
    "Random Number",
    "Burn",
    "Tutorial",
    "Get Distance To",
    "Get Direction To",
    "Push Interaction",
    "Find Best Object for Function",
    "Breakpoint",
    "Find Location For",
    "Idle For Input",
    "Remove Object Instance",
    "Make New Character",
    "Run Functional Tree",
    "Show String",
    "Look Towards",
    "Play Sound Event",
    "Old Relationship",
    "Alter Budget",
    "Relationship",
    "Go To Relative Position",
    "Run Tree By Name",
    "Set Motive Change",
    "Gosub Found Action",
    "Set To Next",
    "Test Object Type",
    "Find 5 Worst Motives",
    "UI Effect",
    "Special Effect",
    "Dialog",
    "Test Sim Interacting With",
    "unknown",
    "unknown",
    "unknown",
    "Set Balloon/Headline",
    "Create New Object Instance",
    "Drop Onto",
    "Animate Sim",
    "Go To Routing Slot",
    "Snap",
    "Reach",
    "Stop All Sounds",
    "Notify Stack Object out of Idle",
    "Add/Change Action String",
    "Manage Inventory",
];

/// A primitive's name, for messages.
pub fn primitive_name(opcode: u16) -> &'static str {
    NAMES
        .get(usize::from(opcode))
        .copied()
        .unwrap_or("unnamed primitive")
}

impl Machine<'_, '_> {
    /// Runs a primitive and returns its branch, or that it yields.
    pub(crate) fn primitive(&mut self, instruction: &Instruction) -> Result<Flow, Trap> {
        let operands = &instruction.operands;
        Ok(match instruction.opcode {
            0 => self.sleep(operands)?,
            1 => self.generic_call(operands)?.into(),
            2 => self.expression(operands)?.into(),
            7 => self.refresh(operands)?.into(),
            8 => self.random(operands)?.into(),
            10 => self.own_events(operands)?.into(),
            11 => self.get_distance_to(operands)?.into(),
            17 => self.idle_for_input(operands)?,
            18 => self.remove(operands)?,
            23 => self.play_sound(operands)?.into(),
            24 => self.old_relationship(operands)?.into(),
            26 => self.relationship(operands)?.into(),
            28 => self.run_named_tree(operands)?,
            31 => self.set_to_next(operands)?.into(),
            42 => self.create_object(operands)?.into(),
            32 => self.test_object_type(operands)?.into(),
            36 => self.dialog(operands)?,
            49 => self.notify()?.into(),
            51 => self.manage_inventory(operands)?.into(),
            opcode => return Err(Trap::Unimplemented { opcode }),
        })
    }
}
