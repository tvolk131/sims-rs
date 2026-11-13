//! Primitives: the instructions below opcode 256, built into the
//! interpreter. Each returns which branch to take. A primitive with no
//! implementation fails with [`Trap::Unimplemented`], which names it.

mod expression;
mod random;

use crate::fault::Trap;
use crate::interpreter::Machine;
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
    /// Runs a primitive and returns its branch.
    pub(crate) fn primitive(&mut self, instruction: &Instruction) -> Result<bool, Trap> {
        match instruction.opcode {
            2 => self.expression(&instruction.operands),
            8 => self.random(&instruction.operands),
            opcode => Err(Trap::Unimplemented { opcode }),
        }
    }
}
