//! Running an object's script stack: stepping instructions, calling and
//! returning from trees, and branching.
//!
//! An object's thread is its stack of saved frames in [`World`]; the
//! interpreter keeps no state of its own. Ported from the old prototype's
//! `call`, `branch`, `current`, `dimensions` and `entry_pc`
//! (legacy/crates/sims-vm/src/lib.rs:1550-1800), without its observation,
//! native-suspension and retained-frame machinery.

use crate::fault::{Fault, Location, Trap};
use crate::program::{DEFINITION_WORDS, Program, Tree};
use sims_content::Catalog;
use sims_formats::behavior::{Behavior, Instruction};
use sims_formats::saved_objects::Frame;
use sims_sim::{Object, World};
use std::cell::RefCell;

/// The deepest an object's stack may grow, counting saved frames.
pub const MAX_STACK: usize = 128;

/// Branch targets with special meaning. Targets above 250 other than these
/// are invalid (native error 0x44c).
const ERROR: u8 = 0xfd;
const RETURN_TRUE: u8 = 0xfe;
const RETURN_FALSE: u8 = 0xff;
const LAST_NODE: u8 = 0xfa;

/// The saved primitive state of a frame whose Run Tree By Name
/// instruction pushed a tree and waits for it to return.
pub(crate) const RAN_TREE: i32 = 1;

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
    /// The thread gave up the rest of its turn (Sleep, or an object removing
    /// itself). Its top frame stays on the instruction that yielded, which
    /// runs again on the object's next turn.
    Yielded,
}

/// What a primitive tells the interpreter to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    /// Take the true or the false branch.
    Branch(bool),
    /// Stop here and run this instruction again next turn.
    Yield,
    /// The primitive pushed a frame, which runs next.
    Pushed,
}

impl From<bool> for Flow {
    fn from(result: bool) -> Self {
        Self::Branch(result)
    }
}

/// Runs object scripts against a lot. It holds only what doesn't change
/// while scripts run: the linked code, the catalog, and the active random
/// stream.
#[derive(Debug, Clone, Copy)]
pub struct Interpreter<'a> {
    pub program: &'a Program,
    pub catalog: &'a Catalog,
    pub stream: Stream,
    /// Where to note each callback the lot asks of an object, or None to
    /// note nothing. See [`Asked`].
    pub asked: Option<&'a RefCell<Vec<Asked>>>,
}

/// One call of the original's "run this object's function" (0x422d46):
/// the object and the function's place in its callback table. The call is
/// noted whether or not the object has a tree for the function. Tests
/// compare the list with the calls a trace of the original shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asked {
    pub object: u16,
    pub function: usize,
}

impl<'a> Interpreter<'a> {
    pub fn new(program: &'a Program, catalog: &'a Catalog, stream: Stream) -> Self {
        Self {
            program,
            catalog,
            stream,
            asked: None,
        }
    }

    /// Executes the instruction at the top of `object`'s stack. Returning
    /// from the bottom frame empties the stack and gives
    /// [`Status::Returned`]. On a fault the stack is left as it was at the
    /// faulting instruction.
    pub fn step(&self, world: &mut World, object: u16) -> Result<Status, Fault> {
        let mut machine = self.machine(world, object, 0, false);
        machine.step().map_err(|trap| machine.fault(trap))
    }

    /// Runs `object`'s thread for one turn: until it yields or its
    /// outermost frame returns, executing at most `budget` instructions.
    /// The budget is a guard against scripts that never yield; the
    /// original's own limit is not modeled.
    pub fn turn(&self, world: &mut World, object: u16, budget: usize) -> Result<Status, Fault> {
        let mut machine = self.machine(world, object, 0, false);
        for _ in 0..budget {
            match machine.step() {
                Ok(Status::Running) => {}
                Ok(status) => return Ok(status),
                Err(trap) => return Err(machine.fault(trap)),
            }
        }
        Err(machine.fault(Trap::BudgetExhausted { budget }))
    }

    /// Pushes a frame for `tree` on `object`'s stack and runs until that
    /// frame returns, executing at most `budget` instructions. The frame runs
    /// the object's own code, with `stack_object` as its stack object and
    /// `args` as its parameters (cut or zero-padded to the tree's count). For
    /// an indexed tree (kind 4) the first argument picks the entry node.
    ///
    /// The tree runs synchronously, as the original runs callbacks such as
    /// Init (0x4749b5): it cannot yield, so a Sleep that would wait is
    /// native error 0x44f.
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
        self.call(world, object, tree, stack_object, args, usize::MAX, budget)
    }

    /// [`Interpreter::run_tree`] for a caller whose argument buffer holds
    /// `most` words: a tree that takes more is refused.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call(
        &self,
        world: &mut World,
        object: u16,
        tree: u16,
        stack_object: i16,
        args: &[i16],
        most: usize,
        budget: usize,
    ) -> Result<bool, Fault> {
        let base = world.objects.get(&object).map_or(0, |o| o.stack.len());
        let mut machine = self.machine(world, object, base, true);
        machine
            .enter(tree, stack_object, args, most, true)
            .map_err(|trap| Fault {
                object,
                at: Some(Location { tree, node: 0 }),
                trap,
            })?;
        for _ in 0..budget {
            match machine.step() {
                Ok(Status::Running) => {}
                Ok(Status::Returned(value)) => return Ok(value),
                Ok(Status::Yielded) => {
                    let trap = Trap::native(0x44f, "a synchronous tree yielded");
                    return Err(machine.fault(trap));
                }
                Err(trap) => return Err(machine.fault(trap)),
            }
        }
        Err(machine.fault(Trap::BudgetExhausted { budget }))
    }

    pub(crate) fn machine<'w>(
        &self,
        world: &'w mut World,
        me: u16,
        base: usize,
        synchronous: bool,
    ) -> Machine<'a, 'w> {
        Machine {
            program: self.program,
            catalog: self.catalog,
            world,
            me,
            stream: self.stream,
            asked: self.asked,
            base,
            synchronous,
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
    pub asked: Option<&'a RefCell<Vec<Asked>>>,
    /// The stack depth below the frames this run owns. When a return pops
    /// the stack down to it, the run is over and the frame below is not
    /// branched.
    pub base: usize,
    /// Whether the run must finish without yielding, as a callback does.
    pub synchronous: bool,
}

impl<'a> Machine<'a, '_> {
    /// An interpreter over the same code, for services that run other
    /// objects' trees.
    pub(crate) fn interpreter(&self) -> Interpreter<'a> {
        Interpreter {
            program: self.program,
            catalog: self.catalog,
            stream: self.stream,
            asked: self.asked,
        }
    }

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

    pub(crate) fn me_mut(&mut self) -> Result<&mut Object, Trap> {
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

    /// The words of the object definition a GUID resolves to.
    pub fn definition_words(&self, guid: u32) -> Result<&'a [i16; DEFINITION_WORDS], Trap> {
        let definition = self.definition(guid)?;
        self.program
            .definition(definition.resource, definition.chunk)
            .ok_or_else(|| {
                Trap::Invalid(format!(
                    "the program holds no definition for GUID 0x{guid:08X}"
                ))
            })
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

    /// Pushes an entry frame for `tree`, running the object's own code.
    /// Mirrors the old prototype's `spawn` (legacy lib.rs:498). A tree that
    /// takes more than `most` arguments is refused: the original would copy
    /// words from beyond the caller's argument buffer.
    pub(crate) fn enter(
        &mut self,
        tree: u16,
        stack_object: i16,
        args: &[i16],
        most: usize,
        callback: bool,
    ) -> Result<(), Trap> {
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
        if argc > most {
            return Err(Trap::Unsupported(format!(
                "tree {tree:#06x} takes {argc} arguments; its caller supplies {most}"
            )));
        }
        let node = entry_node(behavior, args.first().copied().unwrap_or(0))?;
        let mut parameters = args.to_vec();
        parameters.resize(argc, 0);
        if self.me()?.stack.len() >= MAX_STACK {
            return Err(Trap::StackOverflow);
        }
        self.push_frame(
            Frame {
                stack_object,
                tree,
                node,
                parameters,
                locals: vec![0; locals],
                primitive_state: 0,
                code_owner_type,
            },
            callback,
        )
    }

    /// Pushes a frame that runs a tree of another object's type, with
    /// that object as its stack object (Run Tree By Name, 0x44e9d2).
    pub(crate) fn enter_on(&mut self, target: u16, tree: u16, args: &[i16]) -> Result<(), Trap> {
        let guid = self
            .world
            .objects
            .get(&target)
            .ok_or(Trap::native(0x15, "missing object"))?
            .guid;
        let owner_type = self
            .world
            .types
            .iter()
            .find(|&(_, &g)| g == guid)
            .map(|(&id, _)| id)
            .ok_or_else(|| {
                Trap::Invalid(format!("GUID 0x{guid:08X} is not in the lot's type table"))
            })?;
        let owner = self.definition(guid)?.resource;
        let behavior = self.behavior(owner, tree)?;
        let (argc, locals) = dimensions(behavior)?;
        let parameters = args
            .get(..argc)
            .ok_or_else(|| Trap::Unsupported("a named tree with more than eight arguments".into()))?
            .to_vec();
        let node = entry_node(behavior, args.first().copied().unwrap_or(0))?;
        if self.me()?.stack.len() >= MAX_STACK {
            return Err(Trap::StackOverflow);
        }
        self.push_frame(
            Frame {
                stack_object: target as i16,
                tree,
                node,
                parameters,
                locals: vec![0; locals],
                primitive_state: 0,
                code_owner_type: owner_type as i16,
            },
            false,
        )
    }

    /// Brings the object's frame storage up to date with its live frames
    /// ([`sims_sim::FrameStorage`]). The storage of an object that has
    /// none yet is the live frames one after another, as the original
    /// lays out a saved thread when it reads it.
    ///
    /// **Unverified**: that the words of the block beyond the frames a
    /// save holds, and the block of a new object, start as zero. The old
    /// prototype assumed the same (legacy/crates/sims-vm/src/lib.rs,
    /// `save_frame_storage`).
    fn sync_frames(&mut self) -> Result<(), Trap> {
        let me = self.me;
        let object = self
            .world
            .objects
            .get(&me)
            .ok_or(Trap::native(10, "missing object"))?;
        let storage = self.world.frame_storage.entry(me).or_insert_with(|| {
            let mut storage = sims_sim::FrameStorage::default();
            let mut at = 0;
            for frame in &object.stack {
                storage.offsets.push(at as u32);
                at += frame_words(frame.parameters.len(), frame.locals.len());
            }
            storage
        });
        if storage.offsets.len() != object.stack.len() {
            return Err(Trap::Invalid(format!(
                "object {me} has {} frames and storage for {}",
                object.stack.len(),
                storage.offsets.len()
            )));
        }
        for (depth, (frame, &offset)) in object.stack.iter().zip(&storage.offsets).enumerate() {
            let start = offset as usize;
            let (args, locals) = (frame.parameters.len(), frame.locals.len());
            let end = start + frame_words(args, locals);
            if storage.words.len() < end {
                storage.words.resize(end, Some(0));
            }
            let state = frame.primitive_state;
            let header = [
                Some(frame.tree as i16),
                Some(frame.node as i16),
                Some(frame.stack_object),
                Some((locals | (args << 8)) as i16),
                Some(state as i16),
                Some((state >> 16) as i16),
                None,
                None,
            ];
            storage.words[start..start + 8].copy_from_slice(&header);
            for (i, &value) in frame.parameters.iter().enumerate() {
                storage.words[start + 8 + i] = Some(value);
            }
            for (i, &value) in frame.locals.iter().enumerate() {
                let unknown = storage.unknown.contains(&(depth as u32, i as u32));
                storage.words[start + 8 + args + i] = (!unknown).then_some(value);
            }
        }
        Ok(())
    }

    /// Where the next frame would start: after the top live frame.
    fn frames_end(&self) -> usize {
        let object = self.world.objects.get(&self.me);
        let storage = self.world.frame_storage.get(&self.me);
        match (
            object.and_then(|o| o.stack.last()),
            storage.and_then(|s| s.offsets.last()),
        ) {
            (Some(frame), Some(&offset)) => {
                offset as usize + frame_words(frame.parameters.len(), frame.locals.len())
            }
            _ => 0,
        }
    }

    /// Writes the mark the original puts above the top frame before it
    /// runs a tree synchronously (0x4749b5): eight words, -1, three
    /// zeros, 1 and three zeros. Returns where the tree's frame starts.
    /// The mark is written even when the tree turns out to be missing.
    pub(crate) fn write_mark(&mut self) -> Result<usize, Trap> {
        self.sync_frames()?;
        let start = self.frames_end();
        let storage = self
            .world
            .frame_storage
            .get_mut(&self.me)
            .ok_or_else(|| Trap::Invalid("no frame storage".into()))?;
        if storage.words.len() < start + 8 {
            storage.words.resize(start + 8, Some(0));
        }
        for (word, value) in storage.words[start..start + 8]
            .iter_mut()
            .zip([-1, 0, 0, 0, 1, 0, 0, 0])
        {
            *word = Some(value);
        }
        Ok(start + 8)
    }

    /// Pushes a frame. Its locals start as the words the frame storage
    /// holds where they land, not as zero. `callback` says the frame is the
    /// first of a tree run synchronously, which goes above a mark.
    fn push_frame(&mut self, mut frame: Frame, callback: bool) -> Result<(), Trap> {
        let start = if callback {
            self.write_mark()?
        } else {
            self.sync_frames()?;
            self.frames_end()
        };
        let me = self.me;
        let depth = self.me()?.stack.len() as u32;
        let storage = self
            .world
            .frame_storage
            .get_mut(&me)
            .ok_or_else(|| Trap::Invalid("no frame storage".into()))?;
        let (args, locals) = (frame.parameters.len(), frame.locals.len());
        let end = start + frame_words(args, locals);
        if storage.words.len() < end {
            storage.words.resize(end, Some(0));
        }
        for (i, local) in frame.locals.iter_mut().enumerate() {
            match storage.words[start + 8 + args + i] {
                Some(value) => *local = value,
                None => {
                    storage.unknown.insert((depth, i as u32));
                }
            }
        }
        storage.offsets.push(start as u32);
        self.me_mut()?.stack.push(frame);
        Ok(())
    }

    /// Pops the top frame (0x4752f6). What it held stays in the frame
    /// storage. Then, for a person, the objects it uses are looked at (the
    /// object's virtual function 8; [`Machine::release_use`]).
    fn pop_frame(&mut self) -> Result<(), Trap> {
        self.sync_frames()?;
        let me = self.me;
        self.me_mut()?.stack.pop();
        if let Some(storage) = self.world.frame_storage.get_mut(&me) {
            storage.offsets.pop();
            let depth = storage.offsets.len() as u32;
            storage.unknown.retain(|&(frame, _)| frame != depth);
        }
        let last_result = self.me()?.last_result;
        self.release_use(last_result)
    }

    /// Drops every frame. What they held stays in the frame storage. A
    /// person using an object fails: how the original ends those uses
    /// here is not known.
    pub(crate) fn clear_frames(&mut self) -> Result<(), Trap> {
        if self
            .me()?
            .person
            .as_ref()
            .is_some_and(|p| !p.object_uses.is_empty())
        {
            return Err(Trap::Unsupported(
                "dropping the frames of a person that is using an object".into(),
            ));
        }
        self.sync_frames()?;
        let me = self.me;
        self.me_mut()?.stack.clear();
        if let Some(storage) = self.world.frame_storage.get_mut(&me) {
            storage.offsets.clear();
            storage.unknown.clear();
        }
        Ok(())
    }

    /// Fails if a local of the top frame still holds half of a pointer.
    pub(crate) fn check_local(&self, local: usize) -> Result<(), Trap> {
        let depth = self.me()?.stack.len().saturating_sub(1) as u32;
        if self
            .world
            .frame_storage
            .get(&self.me)
            .is_some_and(|s| s.unknown.contains(&(depth, local as u32)))
        {
            return Err(Trap::Unsupported(format!(
                "reading local {local}, which starts as half of a pointer the original left \
                 in the frame storage"
            )));
        }
        Ok(())
    }

    /// Notes that a local of the top frame has been written.
    pub(crate) fn wrote_local(&mut self, local: usize) {
        let depth = self
            .world
            .objects
            .get(&self.me)
            .map_or(0, |o| o.stack.len().saturating_sub(1)) as u32;
        if let Some(storage) = self.world.frame_storage.get_mut(&self.me) {
            storage.unknown.remove(&(depth, local as u32));
        }
    }

    /// Executes one instruction.
    pub fn step(&mut self) -> Result<Status, Trap> {
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
        if state == RAN_TREE && instruction.opcode == 28 {
            // Run Tree By Name ran its tree: it takes the branch of the
            // tree's result.
            let result = self.me()?.last_result;
            self.frame_mut()?.primitive_state = 0;
            return self.branch(if result {
                instruction.on_true
            } else {
                instruction.on_false
            });
        }
        if state != 0 {
            return Err(Trap::Unsupported(format!(
                "resuming primitive {} with saved state {state:#x}",
                instruction.opcode
            )));
        }
        let result = match self.primitive(instruction)? {
            Flow::Branch(result) => result,
            Flow::Yield => return Ok(Status::Yielded),
            Flow::Pushed => return Ok(Status::Running),
        };
        self.frame_mut()?.primitive_state = 0;
        self.me_mut()?.last_result = result;
        self.branch(if result {
            instruction.on_true
        } else {
            instruction.on_false
        })
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
        // The frame's locals start as whatever an earlier frame left in
        // the frame storage ([`Machine::push_frame`]).
        let frame = Frame {
            stack_object: parent.stack_object,
            tree: instruction.opcode,
            node,
            parameters: args,
            locals: vec![0; locals],
            primitive_state: 0,
            code_owner_type: parent.code_owner_type,
        };
        self.push_frame(frame, false)
    }

    /// Follows a branch target from the top frame (legacy lib.rs:1665).
    /// Returns pop the frame and branch the caller on the returned value,
    /// which may return again.
    fn branch(&mut self, mut target: u8) -> Result<Status, Trap> {
        let base = self.base;
        loop {
            match target {
                ERROR => return Err(Trap::ErrorBranch),
                RETURN_TRUE | RETURN_FALSE => {
                    let value = target == RETURN_TRUE;
                    // The original writes every branch target, a return
                    // too, into the frame's node word before it acts on it
                    // (0x474cea, 0x474d1b), so a popped frame leaves 0xfe or
                    // 0xff there in the frame storage. The capture of the
                    // Goth house's second tick shows it: a person's new
                    // frame finds 254 in a local that lands on such a word.
                    self.frame_mut()?.node = u16::from(target);
                    self.pop_frame()?;
                    // A return counts as the thread's last result too
                    // (legacy lib.rs:1714), written once the frame is
                    // popped and a person's uses looked at (0x474da0).
                    self.me_mut()?.last_result = value;
                    let stack = &self.me()?.stack;
                    if stack.len() <= base {
                        return Ok(Status::Returned(value));
                    }
                    let state = stack.last().map_or(0, |f| f.primitive_state);
                    if state == RAN_TREE && self.current()?.opcode == 28 {
                        // The frame's next step finishes the primitive.
                        return Ok(Status::Running);
                    }
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

/// The words a frame takes in the frame storage: eight for its header,
/// then its parameters and locals, padded to a multiple of four.
fn frame_words(parameters: usize, locals: usize) -> usize {
    8 + ((parameters + locals + 3) & !3)
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
