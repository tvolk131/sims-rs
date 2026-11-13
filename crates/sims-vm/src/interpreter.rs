//! Running an object's script stack: stepping instructions, calling and
//! returning from trees, and branching.
//!
//! An object's thread is its stack of saved frames in [`World`]; the
//! interpreter keeps no state of its own. Ported from the old prototype's
//! `call`, `branch`, `current`, `dimensions` and `entry_pc`
//! (legacy/crates/sims-vm/src/lib.rs:1550-1800), without its observation,
//! native-suspension and retained-frame machinery.

use crate::fault::{Fault, Location, Trap};
use crate::program::{Program, Tree};
use sims_content::Catalog;
use sims_formats::behavior::{Behavior, Instruction};
use sims_formats::saved_objects::Frame;
use sims_sim::{Object, World};

/// The deepest an object's stack may grow, counting saved frames.
pub const MAX_STACK: usize = 128;

/// Branch targets with special meaning. Targets above 250 other than these
/// are invalid (native error 0x44c).
const ERROR: u8 = 0xfd;
const RETURN_TRUE: u8 = 0xfe;
const RETURN_FALSE: u8 = 0xff;
const LAST_NODE: u8 = 0xfa;

/// Where Random Number draws from.
///
/// The original keeps two streams and draws from whichever is installed at
/// 0x6e54ac. Outside the lot update that is the global stream; around the
/// lot update (0x46d553, called from 0x46d345) the lot stream is swapped in
/// and swapped back out afterwards (legacy/crates/sims-vm/src/rng.rs). The
/// interpreter does not track that swap, so the caller says which stream is
/// active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// [`World::lot_rng`], active while the lot updates.
    Lot,
    /// [`World::global_rng`], active everywhere else.
    Global,
}

/// The result of one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The thread has more to run.
    Running,
    /// The outermost frame returned this value and was popped.
    Returned(bool),
}

/// Runs object scripts against a lot. It holds only what doesn't change
/// while scripts run: the linked code, the catalog, and the active random
/// stream.
#[derive(Debug, Clone, Copy)]
pub struct Interpreter<'a> {
    pub program: &'a Program,
    pub catalog: &'a Catalog,
    pub stream: Stream,
}

impl<'a> Interpreter<'a> {
    pub fn new(program: &'a Program, catalog: &'a Catalog, stream: Stream) -> Self {
        Self {
            program,
            catalog,
            stream,
        }
    }

    /// Executes the instruction at the top of `object`'s stack. Returning
    /// from the bottom frame empties the stack and gives
    /// [`Status::Returned`]. On a fault the stack is left as it was at the
    /// faulting instruction.
    pub fn step(&self, world: &mut World, object: u16) -> Result<Status, Fault> {
        let mut machine = self.machine(world, object);
        machine.step(0).map_err(|trap| machine.fault(trap))
    }

    /// Pushes a frame for `tree` on `object`'s stack and runs until that
    /// frame returns, executing at most `budget` instructions. The frame runs
    /// the object's own code, with `stack_object` as its stack object and
    /// `args` as its parameters (cut or zero-padded to the tree's count). For
    /// an indexed tree (kind 4) the first argument picks the entry node.
    ///
    /// Frames already on the stack are left alone. On a fault, the stack is
    /// left as it was at the faulting instruction.
    pub fn run_tree(
        &self,
        world: &mut World,
        object: u16,
        tree: u16,
        stack_object: i16,
        args: &[i16],
        budget: usize,
    ) -> Result<bool, Fault> {
        let mut machine = self.machine(world, object);
        let base = machine
            .enter(tree, stack_object, args)
            .map_err(|trap| Fault {
                object,
                at: Some(Location { tree, node: 0 }),
                trap,
            })?;
        for _ in 0..budget {
            match machine.step(base) {
                Ok(Status::Running) => {}
                Ok(Status::Returned(value)) => return Ok(value),
                Err(trap) => return Err(machine.fault(trap)),
            }
        }
        Err(machine.fault(Trap::BudgetExhausted { budget }))
    }

    fn machine<'w>(&self, world: &'w mut World, me: u16) -> Machine<'a, 'w> {
        Machine {
            program: self.program,
            catalog: self.catalog,
            world,
            me,
            stream: self.stream,
        }
    }
}

/// One object's view of the world while its thread runs. "Me" is the object
/// whose stack runs; the frame at the top of that stack is the context for
/// operand resolution.
pub(crate) struct Machine<'a, 'w> {
    pub program: &'a Program,
    pub catalog: &'a Catalog,
    pub world: &'w mut World,
    pub me: u16,
    pub stream: Stream,
}

impl<'a> Machine<'a, '_> {
    pub fn fault(&self, trap: Trap) -> Fault {
        let at = self
            .world
            .objects
            .get(&self.me)
            .and_then(|o| o.stack.last())
            .map(|f| Location {
                tree: f.tree,
                node: f.node,
            });
        Fault {
            object: self.me,
            at,
            trap,
        }
    }

    pub fn me(&self) -> Result<&Object, Trap> {
        self.world
            .objects
            .get(&self.me)
            .ok_or(Trap::native(10, "missing object"))
    }

    fn me_mut(&mut self) -> Result<&mut Object, Trap> {
        self.world
            .objects
            .get_mut(&self.me)
            .ok_or(Trap::native(10, "missing object"))
    }

    /// The frame operands resolve against.
    pub fn frame(&self) -> Result<&Frame, Trap> {
        self.me()?
            .stack
            .last()
            .ok_or_else(|| Trap::Invalid("empty stack".into()))
    }

    pub fn frame_mut(&mut self) -> Result<&mut Frame, Trap> {
        self.me_mut()?
            .stack
            .last_mut()
            .ok_or_else(|| Trap::Invalid("empty stack".into()))
    }

    /// The resource whose private and semi-global trees a frame resolves:
    /// the file of the object its code owner type names.
    pub fn owner(&self, frame: &Frame) -> Result<usize, Trap> {
        let id = u16::try_from(frame.code_owner_type).map_err(|_| {
            Trap::Invalid(format!(
                "negative code owner type {}",
                frame.code_owner_type
            ))
        })?;
        let guid = *self.world.types.get(&id).ok_or_else(|| {
            Trap::Invalid(format!(
                "code owner type {id} is not in the lot's type table"
            ))
        })?;
        self.definition(guid).map(|d| d.resource)
    }

    pub fn definition(&self, guid: u32) -> Result<&'a sims_content::Object, Trap> {
        self.catalog
            .object(guid)
            .ok_or_else(|| Trap::Invalid(format!("no definition for GUID 0x{guid:08X}")))
    }

    fn behavior(&self, owner: usize, id: u16) -> Result<&'a Behavior, Trap> {
        match self.program.tree(owner, id) {
            Tree::Behavior(_, behavior) => Ok(behavior),
            Tree::Missing => Err(Trap::MissingTree { id }),
            Tree::Primitive(_) => Err(Trap::Invalid(format!(
                "tree ID {id} names a primitive, not a BHAV"
            ))),
        }
    }

    /// The top frame's current instruction (legacy lib.rs:1563).
    fn current(&self) -> Result<&'a Instruction, Trap> {
        let frame = self.frame()?;
        let behavior = self.behavior(self.owner(frame)?, frame.tree)?;
        if frame.node > u16::from(LAST_NODE) {
            return Err(Trap::native(0x44c, "invalid instruction index"));
        }
        behavior
            .instructions
            .get(usize::from(frame.node))
            .ok_or_else(|| {
                Trap::Invalid(format!(
                    "node {} is outside the tree's {} instructions",
                    frame.node,
                    behavior.instructions.len()
                ))
            })
    }

    /// Pushes the entry frame for [`Interpreter::run_tree`] and returns the
    /// stack depth below it. Mirrors the old prototype's `spawn`
    /// (legacy lib.rs:498).
    fn enter(&mut self, tree: u16, stack_object: i16, args: &[i16]) -> Result<usize, Trap> {
        let guid = self.me()?.guid;
        let owner_type = self
            .world
            .types
            .iter()
            .find(|&(_, &g)| g == guid)
            .map(|(&id, _)| id)
            .ok_or_else(|| {
                Trap::Invalid(format!("GUID 0x{guid:08X} is not in the lot's type table"))
            })?;
        let code_owner_type = i16::try_from(owner_type)
            .map_err(|_| Trap::Invalid(format!("type ID {owner_type} does not fit a frame")))?;
        let owner = self.definition(guid)?.resource;
        let behavior = self.behavior(owner, tree)?;
        let (argc, locals) = dimensions(behavior)?;
        if args.len() > 255 {
            return Err(Trap::Invalid(
                "entry arguments exceed the native limit of 255".into(),
            ));
        }
        let node = entry_node(behavior, args.first().copied().unwrap_or(0))?;
        let mut parameters = args.to_vec();
        parameters.resize(argc, 0);
        let stack = &mut self.me_mut()?.stack;
        if stack.len() >= MAX_STACK {
            return Err(Trap::StackOverflow);
        }
        let base = stack.len();
        stack.push(Frame {
            stack_object,
            tree,
            node,
            parameters,
            locals: vec![0; locals],
            primitive_state: 0,
            code_owner_type,
        });
        Ok(base)
    }

    /// Executes one instruction. `base` is the stack depth below the frames
    /// this run owns: when a return pops the stack down to it, the run is
    /// over and the frame below is not branched.
    pub fn step(&mut self, base: usize) -> Result<Status, Trap> {
        let instruction = self.current()?;
        if instruction.opcode >= 256 {
            if instruction.opcode & 0x8000 != 0 {
                // Program::signature masks this bit off when following
                // calls; what the original's dispatch does with it is not
                // known (legacy lib.rs:993).
                return Err(Trap::Unsupported(format!(
                    "flagged tree call {:#06x}",
                    instruction.opcode
                )));
            }
            self.call(instruction)?;
            return Ok(Status::Running);
        }
        let state = self.frame()?.primitive_state;
        if state != 0 {
            return Err(Trap::Unsupported(format!(
                "resuming primitive {} with saved state {state:#x}",
                instruction.opcode
            )));
        }
        let result = self.primitive(instruction)?;
        self.frame_mut()?.primitive_state = 0;
        self.branch(
            if result {
                instruction.on_true
            } else {
                instruction.on_false
            },
            base,
        )
    }

    /// Calls the tree an instruction names (legacy lib.rs:1578).
    fn call(&mut self, instruction: &Instruction) -> Result<(), Trap> {
        let me = self.me()?;
        if me.stack.len() >= MAX_STACK {
            return Err(Trap::StackOverflow);
        }
        let parent = self.frame()?;
        let owner = self.owner(parent)?;
        let callee = self.behavior(owner, instruction.opcode)?;
        let (argc, locals) = dimensions(callee)?;
        let from_temps = instruction.operands == [255; 8];
        let args: Vec<i16> = if from_temps {
            // All-0xFF operands pass the caller's temps as arguments.
            me.temps
                .get(..argc)
                .ok_or_else(|| {
                    Trap::Unsupported("native argument read beyond eight temporaries".into())
                })?
                .to_vec()
        } else {
            // Native copies a contiguous argc-word span beginning at the
            // instruction's operands, running on into the instructions that
            // follow it in memory (12 bytes each: opcode, branches, operands).
            let tree = self.behavior(owner, parent.tree)?;
            let mut bytes = instruction.operands.to_vec();
            for next in tree.instructions.iter().skip(usize::from(parent.node) + 1) {
                if bytes.len() >= argc * 2 {
                    break;
                }
                bytes.extend_from_slice(&next.opcode.to_le_bytes());
                bytes.extend_from_slice(&[next.on_true, next.on_false]);
                bytes.extend_from_slice(&next.operands);
            }
            bytes
                .get(..argc * 2)
                .ok_or_else(|| {
                    Trap::Unsupported(
                        "native call argument read past the end of the BHAV resource".into(),
                    )
                })?
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&b| i16::from_le_bytes(b))
                .collect()
        };
        let entry = if from_temps {
            *me.temps
                .first()
                .ok_or_else(|| Trap::Invalid("object has no temps".into()))?
        } else {
            word(&instruction.operands, 0)
        };
        let node = entry_node(callee, entry)?;
        // The original reuses a frame arena, so a new frame's locals start
        // with whatever words an earlier, popped frame left at that offset
        // (legacy lib.rs:1640-1650). The saved state has no arena, so locals
        // start at zero here. Unverified.
        let frame = Frame {
            stack_object: parent.stack_object,
            tree: instruction.opcode,
            node,
            parameters: args,
            locals: vec![0; locals],
            primitive_state: 0,
            code_owner_type: parent.code_owner_type,
        };
        self.me_mut()?.stack.push(frame);
        Ok(())
    }

    /// Follows a branch target from the top frame (legacy lib.rs:1665).
    /// Returns pop the frame and branch the caller on the returned value,
    /// which may return again.
    fn branch(&mut self, mut target: u8, base: usize) -> Result<Status, Trap> {
        loop {
            match target {
                ERROR => return Err(Trap::ErrorBranch),
                RETURN_TRUE | RETURN_FALSE => {
                    let value = target == RETURN_TRUE;
                    let stack = &mut self.me_mut()?.stack;
                    stack.pop();
                    if stack.len() <= base {
                        return Ok(Status::Returned(value));
                    }
                    let state = stack.last().map_or(0, |f| f.primitive_state);
                    if state != 0 {
                        // A primitive that ran a tree (Idle For Input, Run
                        // Functional Tree and others) resumes instead of
                        // branching (legacy lib.rs:1737).
                        return Err(Trap::Unsupported(format!(
                            "return into a primitive with saved state {state:#x}"
                        )));
                    }
                    let call = self.current()?;
                    target = if value { call.on_true } else { call.on_false };
                }
                _ if target > LAST_NODE => {
                    return Err(Trap::native(0x44c, "invalid branch target"));
                }
                _ => {
                    let frame = self.frame()?;
                    let count = self
                        .behavior(self.owner(frame)?, frame.tree)?
                        .instructions
                        .len();
                    if usize::from(target) >= count {
                        return Err(Trap::InvalidBranch { target, count });
                    }
                    self.frame_mut()?.node = u16::from(target);
                    return Ok(Status::Running);
                }
            }
        }
    }
}

/// A tree's argument and local counts, as its format limits them
/// (legacy lib.rs:1550). Format 0x8000 always has four arguments and no
/// locals, and 0x8001 caps both at four. The local count is a byte.
fn dimensions(behavior: &Behavior) -> Result<(usize, usize), Trap> {
    if behavior.instructions.is_empty() {
        return Err(Trap::Invalid("BHAV has no instructions".into()));
    }
    let locals = behavior.locals as u8;
    let (args, locals) = match behavior.format {
        0x8000 => (4, 0),
        0x8001 => (behavior.args.min(4), locals.min(4)),
        0x8002 => (behavior.args, locals),
        format => {
            return Err(Trap::Unsupported(format!("BHAV format {format:#06x}")));
        }
    };
    Ok((usize::from(args), usize::from(locals)))
}

/// The node a call enters at (legacy lib.rs:1783). Only indexed trees
/// (kind 4) use the entry argument, clamped to the last node.
fn entry_node(behavior: &Behavior, entry: i16) -> Result<u16, Trap> {
    if behavior.kind != 4 {
        return Ok(0);
    }
    let entry = u16::try_from(entry)
        .map_err(|_| Trap::Unsupported("negative indexed BHAV entry".into()))?;
    let last = behavior.instructions.len().saturating_sub(1);
    Ok(entry.min(u16::try_from(last).unwrap_or(u16::MAX)))
}

/// The little-endian word at `offset` in an instruction's operands.
pub(crate) fn word(operands: &[u8; 8], offset: usize) -> i16 {
    i16::from_le_bytes([operands[offset], operands[offset + 1]])
}
