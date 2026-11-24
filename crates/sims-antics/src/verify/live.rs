//! Symbolic live-output equivalence checker for SimAntics `BHAV` trees.
//!
//! The lockstep checker ([`super::check_bhav_equivalent`]) walks both trees node for node,
//! so two trees that do the same thing in a different number of steps fail it: Edith's
//! all-zero "dummy" nodes such as `my.attr[0] > my.attr[0] -> true / true`, a self
//! assignment `temp[0] = temp[0]`, a temp written twice, or a local that nothing reads.
//!
//! This checker instead runs each tree on its own over symbolic inputs and records, for
//! every path, a *summary* of what the rest of the game can observe:
//!
//! - how the tree exits (`true`, `false`, error, a modeled VM trap);
//! - the ordered trace of observable events: primitives and subroutine calls with their
//!   canonical operands, reads of memory whose read can trap or has side effects, and
//!   writes to memory the checker does not track. Each event carries a snapshot of the
//!   state it may read: the tracked cells below, the scratch pointer, and the frame
//!   slots the primitive reads (`super::primitive_frame_uses`);
//! - the final values of the *tracked* cells: the running object's eight temps and its
//!   attributes `0..my_attribute_count` (every object has at least 8,
//!   `sims-vm/src/factory.rs:314`), whose reads and writes never trap and have no side
//!   effects (`sims-vm/src/memory.rs:364`);
//! - the scratch-pointer state every `Expression` sets (`expression.rs:51`), which a later
//!   read of Global 34 observes;
//! - unless turned off, the frame's final parameters and locals, which stay in the
//!   frame storage after the tree returns and seed a later frame (`interpreter.rs:1216`).
//!
//! Two trees are equivalent when every pair of paths whose path conditions can hold
//! together has the same summary. Pure reads of tracked cells, self assignments, and
//! overwritten writes leave no trace, so they drop out of the comparison; writes to
//! dead frame slots do too when frame residue is not observed.
//!
//! Verdicts are conservative: `Equivalent` must be sound, a counterexample must be a
//! concrete run, and everything else is `Inconclusive`.
//!
//! - **Counterexamples come from `sims-vm`.** A symbolic difference is only a candidate.
//!   The checker picks concrete inputs that satisfy the combined path condition, runs
//!   both trees in `sims-vm` from the same world and frame (`super::replay`), and
//!   reports a counterexample only when the two runs observably differ. A candidate it
//!   cannot replay (a reachable primitive or call, an inexact path condition, more than
//!   `MAX_REPLAYS` (32) candidates) or whose runs agree is `Inconclusive`.
//! - **Faults are never a difference.** A fault aborts the tick and leaves the faulting
//!   frame at its node, with whatever the node had done to the frame and the last
//!   branch result by then (`interpreter.rs:809`). Any node may fault: a modeled trap,
//!   a branch to the error exit or a missing node, and any primitive or call (each gets
//!   a faulting path of its own). Two faulting paths are equal only when they fault at
//!   the same node, with the same opcode and operands, from the same state before it
//!   (see `FaultPoint`); any other pairing is `Inconclusive`, never a counterexample (the
//!   replay fails on a fault).
//! - **The instruction budget is a model assumption.** `Equivalent` means equivalent
//!   when neither run exhausts the VM's per-tick instruction budget. Dummy nodes change
//!   the instruction count by design, so requiring equal counts would reject the very
//!   trees this checker exists for. Likewise, faults the world can raise on its own
//!   (an object deleted under the tree, a stack overflow from deep calls) are outside
//!   the model.
//!
//! Soundness: values are named by what determines them (initial inputs, or an event
//! together with the whole event history before it), so equal names always mean equal
//! values. Branches are pruned only when the path condition provably rules them out.
//! The checker never reports `Equivalent` for trees it cannot fully explore within its
//! bounds.

use super::domains::ParamDomains;
use super::rename::local_operands;
use super::replay::{self, Witness};
use super::text::{TableLocals, TextLocals};
use super::{
    EquivResult, FrameUse, TraceStep, canonicalize_primitive_operands,
    collect_inline_call_arg_bytes, is_always_true_instruction, primitive_frame_uses, word_i16,
};
use crate::ir::BhavChunk;
use crate::lower::{TARGET_ERROR, TARGET_FALSE, TARGET_TRUE};
use sims_formats::behavior::Instruction;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

/// Attributes every object has, whatever its definition says (`sims-vm/src/factory.rs:314`,
/// `sims-vm/src/objects.rs:530`): reading `my.attr[0..8]` never traps.
pub const GUARANTEED_ATTRIBUTES: usize = 8;

/// Exit code for a square root of a negative number, which needs the original's x87
/// exception policy (`expression.rs:206`); outside the native error-code range.
const NEGATIVE_SQRT: u16 = 0xFFFF;

/// Native error for a branch target or instruction index past node 250
/// (`interpreter.rs:766`, `interpreter.rs:1695`).
const INVALID_NODE: u16 = 0x44c;

/// The last node index a tree can run (`interpreter.rs:29`).
const LAST_NODE: usize = 250;

/// Temps every object has (`sims-vm/src/memory.rs:438`).
const TEMP_COUNT: i16 = 8;

/// Most concrete replays one comparison runs before leaving the rest undecided.
const MAX_REPLAYS: usize = 32;

/// What the checker may assume about where one side's tree runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveContext {
    /// `(tree_id -> argument count)` for subroutines the tree calls, used to compare
    /// calls whose inline arguments spill into the following instructions.
    pub callee_arities: Option<BTreeMap<u16, usize>>,
    /// How many attributes whatever runs as `me` is known to have. That need not be the
    /// object whose file holds the tree (`call_as` runs a tree as another object), so it
    /// is the smallest count of anything that may run it. Reads of `my.attr[i]` below
    /// this never trap. Defaults to [`GUARANTEED_ATTRIBUTES`].
    pub my_attribute_count: usize,
    /// The tree's own file's `BCON` tables (`chunk id -> values`). When set, and the tree
    /// is a private tree (`4096..=8191`), reads of the owner's tuning constants
    /// (scope 26, tables `0..64`, `sims-vm/src/interpreter.rs:713`) fold to literals.
    pub local_bcons: Option<BTreeMap<u16, Vec<i16>>>,
    /// Trees a call can name that take no arguments and are not indexed (kind 4). A
    /// call to one reads none of its operand bytes (`interpreter.rs:1539`: no argument
    /// words, and `entry_node` ignores the entry for other kinds), so the checker
    /// compares any spelling of them as `[0xFF; 8]`, when `callee_arities` agrees
    /// ([`LiveContext::call_reads_no_operands`]).
    pub argless_callees: Option<BTreeSet<u16>>,
    /// Values tree parameters are assumed to hold on entry (`tree -> param -> domain`;
    /// `verify-equiv --param-domain`). Empty by default, which assumes nothing. With
    /// one, `Equivalent` holds only for runs that enter each constrained tree inside its
    /// domain. Ignored for kind-4 trees, whose first parameter is their entry node.
    pub param_domains: ParamDomains,
    /// The locals the tree's own file's string tables may make `Dialog` and
    /// `Add/Change Action String` read ([`TextLocals`]). Used for private trees
    /// (`4096..=8191`), whose strings come from their own file (`dialog.rs`,
    /// `string_table`). Without it those primitives may read any local.
    pub text_locals: Option<TextLocals>,
    /// Private trees (`4096..=8191`) of the tree's own file that are not indexed (kind 4)
    /// and whose first node sets the last branch result before anything can read the
    /// value the caller left ([`overwrites_last_result_first`]). Used only under
    /// `LiveOptions::observe_fault_last_result == false`.
    pub last_result_overwriters: Option<BTreeSet<u16>>,
    /// For private trees (`4096..=8191`) of the tree's own file that are not indexed
    /// (kind 4): the parameters the tree overwrites before anything can see what its
    /// caller passed ([`params_written_first`]). A call from a private tree to one of
    /// them, with its arguments in its operands (at most 4), compares only the other
    /// arguments ([`FrameVal::CallArgs`]).
    pub dead_args: Option<BTreeMap<u16, BTreeSet<usize>>>,
}

impl LiveContext {
    /// Whether node `ins` sets the running object's last branch result before anything
    /// can read the value it had, unless it faults ([`overwrites_last_result_first`],
    /// [`LiveContext::last_result_overwriters`]).
    /// A call names this file's tree only from a private tree (`tree`).
    pub(crate) fn overwrites_last_result(&self, tree: u16, ins: &Instruction) -> bool {
        if ins.opcode >= 256 {
            (4096..=8191).contains(&tree)
                && (4096..=8191).contains(&ins.opcode)
                && self
                    .last_result_overwriters
                    .as_ref()
                    .is_some_and(|set| set.contains(&ins.opcode))
        } else {
            overwrites_last_result_first(ins.opcode, &ins.operands)
        }
    }

    /// The locals primitive `opcode` of private tree `tree` reads through its text, when
    /// it formats text from a table [`LiveContext::text_locals`] knows every string of.
    /// `None` when it may read any local, or formats no text.
    pub(crate) fn text_reads(
        &self,
        tree: u16,
        opcode: u16,
        ops: &[u8; 8],
    ) -> Option<BTreeSet<i16>> {
        if !(4096..=8191).contains(&tree) {
            return None;
        }
        match self.text_locals.as_ref()?.primitive(opcode, ops)? {
            TableLocals::Known(slots) => Some(slots),
            TableLocals::Unknown => None,
        }
    }

    /// The arguments a call from private tree `tree` to `callee` passes that the callee
    /// overwrites unseen ([`LiveContext::dead_args`]), when the call's arguments are its
    /// operand words or temps (at most 4 of them, `interpreter.rs:1539`).
    pub(crate) fn dead_args(&self, tree: u16, callee: u16) -> Option<&BTreeSet<usize>> {
        if !(4096..=8191).contains(&tree) || !(4096..=8191).contains(&callee) {
            return None;
        }
        let dead = self.dead_args.as_ref()?.get(&callee)?;
        let arity = *self.callee_arities.as_ref()?.get(&callee)?;
        (arity <= 4 && dead.iter().all(|&i| i < arity)).then_some(dead)
    }

    /// Whether a call to tree `id` reads none of its operand bytes: `argless_callees`
    /// lists it and `callee_arities` agrees it takes no arguments. A context that lists
    /// a tree with no count, or with a count above 0, gets its operands compared.
    pub(crate) fn call_reads_no_operands(&self, id: u16) -> bool {
        id >= 256
            && self
                .argless_callees
                .as_ref()
                .is_some_and(|set| set.contains(&id))
            && self
                .callee_arities
                .as_ref()
                .and_then(|arities| arities.get(&id))
                == Some(&0)
    }
}

/// Whether node `opcode` with `ops` sets the running object's last branch result before
/// anything can read the value it had, unless it faults.
///
/// A node sets it only when its primitive returns a branch (`interpreter.rs:1479`,
/// `:1493`); a yield or a fault leaves it (`interpreter.rs:1426`, `:1452`). Then the old
/// value is read by a resumed primitive, by recovery (`tick.rs` `recover`,
/// `people.rs:1297`, `visits.rs:280`), by a person letting go of an object
/// (`forget.rs:142`), or by a tree some primitive runs on the running object's own
/// stack and pops (`interpreter.rs:1246`). So a node qualifies when it never yields,
/// runs no tree, and cannot make sims-vm run one on the running object:
///
/// - an `Expression` (`expression.rs`) that compares (operators 0, 1, 2, 8, 14, 15, 16),
///   or writes a temporary, parameter or local (scopes 8, 9, 25). Another destination
///   may be global 3, which in co-op mode selects a person (`memory.rs:846`), and
///   moving the selection indicator runs PLACED on either person (`selection.rs:346`,
///   `:413`, `placement.rs:1506`);
/// - a primitive that returns a bool (`primitives.rs`, the `.into()` arms) and touches
///   only data: Change Suit (6), Refresh (7), Own Events (10), Get Distance To (11),
///   Breakpoint (15), Show String (21), Play Sound Event (23), Old Relationship (24),
///   Alter Budget (25), Set Motive Change (29), Test Object Type (32), Find 5 Worst
///   Motives (33), Test Sim Interacting With (37), Set Balloon/Headline (41), Stop All
///   Sounds (48), Notify Out of Idle (49), Add/Change Action String (50), Manage
///   Inventory (51);
/// - Generic Sims Call (1) except routines 19 and 20, which make a person or object and
///   run its Init (`generic.rs:758`, `:762`, `factory.rs:189`);
/// - Random Number (8), Get Direction To (12), Relationship (26) and Set To Next (31),
///   when their destination is a temporary, parameter or local (or, for Set To Next,
///   the stack object), for the reason `Expression`'s must be.
///
/// Every other primitive may yield, run a tree, or run callbacks on the running object
/// (Grab, Drop, Burn, Push Interaction, Find Best Object For, Find Location For, Create
/// New Object Instance, Drop Onto, Snap To Slot), or build a temporary person (Make New
/// Character, UI Effect).
pub(crate) fn overwrites_last_result_first(opcode: u16, ops: &[u8; 8]) -> bool {
    let plain = |scope: u16| matches!(scope, 8 | 9 | 25);
    match opcode {
        2 => matches!(ops[5], 0 | 1 | 2 | 8 | 14 | 15 | 16) || plain(u16::from(ops[6])),
        1 => !matches!(u16::from_le_bytes([ops[0], ops[1]]), 19 | 20),
        6 | 7 | 10 | 11 | 15 | 21 | 23 | 24 | 25 | 29 | 32 | 33 | 37 | 41 | 48 | 49 | 50 | 51 => {
            true
        }
        8 | 12 => plain(u16::from_le_bytes([ops[2], ops[3]])),
        26 => plain(u16::from(ops[4])),
        31 => ops[4] & 0x80 == 0 || plain(u16::from(ops[5])),
        _ => false,
    }
}

impl Default for LiveContext {
    fn default() -> Self {
        Self {
            callee_arities: None,
            my_attribute_count: GUARANTEED_ATTRIBUTES,
            local_bcons: None,
            argless_callees: None,
            dead_args: None,
            param_domains: ParamDomains::new(),
            text_locals: None,
            last_result_overwriters: None,
        }
    }
}

/// Bounds and observation policy for [`check_bhav_live_equivalent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveOptions {
    /// Most paths explored per tree and entry point before giving up.
    pub max_paths: usize,
    /// Most instructions one path may execute before giving up (loops).
    pub max_steps: usize,
    /// Most pairs of paths with different summaries checked for joint feasibility. A
    /// pair whose conditions contain one branch with opposite outcomes is ruled out
    /// without a check and does not count.
    pub max_pair_checks: usize,
    /// Compare the scratch-pointer state at events and at exit. A caller that reads
    /// Global 34 right after the call observes it (`expression.rs:31`).
    pub observe_scratch_pointer: bool,
    /// Compare the frame's final parameters and locals (on by default). They stay in
    /// the frame storage after the tree returns and become the initial locals of a
    /// later frame at the same depth (`interpreter.rs:1216`), which can read them before
    /// writing them. A difference here is only ever a possible one: it shows only if
    /// such a reader runs.
    pub observe_frame_residue: bool,
    /// Compare the thread's last branch result where something reads it: at exit
    /// (popping the frame of the tree an action started records it in the person's
    /// last action, `object_uses.rs:246`) and before primitives, which may start a tree
    /// that reads it (`found_action.rs:102`, `idle.rs:187`). Two trees that return the
    /// same value from differently-worded final tests differ here.
    pub observe_last_result: bool,
    /// Compare under the behavioral relation (`verify-equiv --behavioral`,
    /// `docs/decisions/20261123-verifier-behavioral-mode.md`): node numbers and frame
    /// storage are not observed. Primitives, calls, reads and writes of untracked
    /// memory, exits, tracked cells, and whether and how a path faults still are. Implies
    /// that frame residue is not observed.
    pub behavioral: bool,
    /// Compare the last branch result on paths that fault (on by default, whatever
    /// `observe_last_result` says). A fault keeps it: play-mode recovery rebuilds an
    /// object's frame with the result its last primitive took (`visits.rs:280`), and
    /// resets a person after copying it into the person's last-action flags
    /// (`people.rs:1297`). Turning this off (`verify-equiv --ignore-fault-last-result`)
    /// is an assumption that nothing reads it after a fault, which the CLI prints.
    pub observe_fault_last_result: bool,
    /// Compare the scratch-pointer state on paths that fault (on by default, whatever
    /// `observe_scratch_pointer` says). A fault leaves it in the object's state
    /// (`World::scratch_ptrs`, cleared only when the object goes away), where the
    /// object's next `Expression` or `UI Effect` reading Global 34 observes it
    /// (`expression.rs:34`, `ui_effect.rs:371`). Turning this off
    /// (`verify-equiv --ignore-fault-scratch-pointer`) is an assumption that nothing
    /// reads it after a fault, which the CLI prints.
    pub observe_fault_scratch_pointer: bool,
}

impl LiveOptions {
    /// Whether final parameters and locals are compared.
    fn observes_residue(&self) -> bool {
        self.observe_frame_residue && !self.behavioral
    }
}

impl Default for LiveOptions {
    fn default() -> Self {
        Self {
            max_paths: 4096,
            max_steps: 2048,
            max_pair_checks: 250_000,
            observe_scratch_pointer: true,
            observe_frame_residue: true,
            observe_last_result: true,
            behavioral: false,
            observe_fault_last_result: true,
            observe_fault_scratch_pointer: true,
        }
    }
}

// ============================================================================
// Symbolic values
// ============================================================================

/// A 16-bit value: a literal, or an interned symbol (see [`SymKey`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Val {
    Const(i16),
    Sym(u32),
}

/// A tracked shared cell of the running object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Cell {
    Temp(i16),
    Attr(i16),
}

/// What a symbol stands for. Every key names a value that is fully determined by the
/// key, so the same key in tree A and tree B is the same value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum SymKey {
    /// The tree's parameter `i` on entry.
    InitParam(u16),
    /// The frame-storage word a local starts from on entry (`args + i`).
    InitLocal(u16),
    /// A tracked cell's value at the start of epoch `prefix` (entry, or the last event
    /// that may have written shared memory).
    Cell(u32, Cell),
    /// The value an opaque read returned; `prefix` is the read event.
    EventValue(u32),
    /// The running frame's stack object at the start of epoch `prefix`. Only an event
    /// changes it: an `Expression` writing scope 10 is an opaque write, and every
    /// primitive and call ends its epoch.
    StackObject(u32),
    /// A frame slot after event `prefix` that may have written it; it was `old` before.
    FrameMaybe(u32, u8, i16, Val),
    /// The number Random Number event `prefix` drew and stored in its destination.
    Drawn(u32),
    /// The running object's data word `idx` after the event that ends at `prefix`
    /// ([`MY_DATA_WORDS`]).
    MyData(u32, i16),
    /// An arithmetic `Expression` operator applied to two values.
    Op(u8, Val, Val),
    /// Equivalence class `k` of the product state that cut `prefix` starts
    /// ([`PrefixNode::Cut`]): a frame slot or tracked cell both runs hold some value in,
    /// the same value wherever the class is the same.
    CutClass(u32, u16),
}

impl SymKey {
    fn is_leaf(&self) -> bool {
        !matches!(self, Self::Op(..))
    }
}

/// The scratch-pointer state an `Expression` leaves (`expression.rs:51`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Scratch {
    /// Whatever it was at the start of the current epoch.
    Inherited,
    /// `SCRATCH_PTR_SET | SCRATCH_PTR_EXPRESSION`, set by an `Expression`.
    Expression,
    /// `SCRATCH_PTR_SET | SCRATCH_PTR_ARG_SLOT`, set by Random Number or Get Direction
    /// To (`random.rs`, `direction.rs`).
    ArgSlot,
    /// `SCRATCH_PTR_SET_TO_NEXT`, set by Set To Next (`set_to_next.rs:117`).
    SetToNext,
}

/// Reading the running object's data words `0..MY_DATA_WORDS` (scope 3) cannot fail
/// and changes nothing, except the lockout word ([`MY_DATA_LOCKOUT`]), so it is not an
/// event. `sims-vm` resolves scope 3 to `Cell::Data(me, index)` after checking
/// `index < 72` and `index < data.len()` (`memory.rs:448`), and the running object
/// always exists. Every object has at least 72 data words: the factory makes 72
/// (`factory.rs:325`), a saved object is loaded with 72 (`saved_objects.rs:385`,
/// `objects.rs:579`), co-op and placement make 128 (`coop.rs:3467`,
/// `placement.rs:2864`); only test fixtures make fewer. Reading a `Cell::Data` is a
/// plain array read (`memory.rs:742`).
const MY_DATA_WORDS: i16 = 72;

/// The data word whose resolution wakes the object (`memory.rs:22`, `LOCKOUT`, and
/// `memory.rs:137`): reading it stays an event.
const MY_DATA_LOCKOUT: i16 = 25;

/// The parameters tree `bhav` overwrites before anything can see the value its caller
/// passed ([`LiveContext::dead_args`]). Empty for an indexed (kind 4) tree, whose entry
/// node comes from the call.
///
/// The VM enters any other tree at node 0 (`entry_node`). From there the tree runs a
/// chain of `Expression` assignments (operator 5, which writes without using the old
/// value and returns true, `expression.rs:75`), each to a temp or a parameter in range
/// and from a literal, a temp or a parameter in range. None of those can fault
/// (`memory.rs:472`, `memory.rs:752`, `memory.rs:969`). A parameter the chain assigns
/// before it reads it is overwritten before any node can read it, fault with it in the
/// frame, or return with it in frame storage. The chain ends at the first other node.
///
/// Under the behavioral relation (`behavioral`), which observes no frame at a fault and
/// no frame storage, the chain may also read memory the frame does not address: the
/// running or stack object's attributes or data words (scopes 0, 1, 3, 4), the stack
/// object (scope 10, index 0), a tuning constant (26), or a global other than 34. Such
/// a read may fault or be an event, but neither depends on a parameter's value, and a
/// fault there leaves nothing the relation compares that does.
#[must_use]
pub fn params_written_first(bhav: &BhavChunk, behavioral: bool) -> BTreeSet<usize> {
    let mut written = BTreeSet::new();
    if bhav.kind == 4 {
        return written;
    }
    let args = usize::from(bhav.args);
    let param = |idx: i16| usize::try_from(idx).ok().filter(|&i| i < args);
    let mut read = BTreeSet::new();
    let mut pc = 0usize;
    for _ in 0..bhav.instructions.len() {
        let Some(ins) = bhav.instructions.get(pc) else {
            break;
        };
        let ops = &ins.operands;
        if ins.opcode != 2 || ops[5] != 5 {
            break;
        }
        let (lhs_idx, rhs_idx, lhs_scope, rhs_scope) =
            (word_i16(ops, 0), word_i16(ops, 2), ops[6], ops[7]);
        match rhs_scope {
            7 => {}
            8 if (0..8).contains(&rhs_idx) => {}
            0 | 1 | 3 | 4 | 26 if behavioral => {}
            10 if behavioral && rhs_idx == 0 => {}
            6 if behavioral && rhs_idx != 34 => {}
            9 => match param(rhs_idx) {
                Some(i) => {
                    if !written.contains(&i) {
                        read.insert(i);
                    }
                }
                None => break,
            },
            _ => break,
        }
        match lhs_scope {
            8 if (0..8).contains(&lhs_idx) => {}
            9 => match param(lhs_idx) {
                Some(i) => {
                    if !read.contains(&i) {
                        written.insert(i);
                    }
                }
                None => break,
            },
            _ => break,
        }
        pc = usize::from(ins.on_true);
    }
    if behavioral {
        written.extend(params_never_read(bhav));
    }
    written
}

/// The parameters no node of `bhav` reads (the behavioral part of
/// [`params_written_first`]). An `Expression` reads a parameter it names on its right, on
/// its left unless it assigns (operators 5 and 20, `expression.rs:75`), and through
/// scope 22 on either side (`memory.rs:511`). A primitive reads the frame slots
/// [`primitive_frame_uses`] lists, scope 22 included, and every one when it lists the
/// whole frame. A call reads none: its arguments are operand bytes or temps
/// (`interpreter.rs:1539`). The behavioral relation observes no frame storage and no
/// frame at a fault, so the value of a parameter nothing reads is never seen.
fn params_never_read(bhav: &BhavChunk) -> BTreeSet<usize> {
    let args = usize::from(bhav.args);
    let mut read = vec![false; args];
    let mut mark = |scope: u8, idx: i16| {
        if matches!(scope, 9 | 22)
            && let Some(slot) = usize::try_from(idx).ok().and_then(|i| read.get_mut(i))
        {
            *slot = true;
        }
    };
    for ins in &bhav.instructions {
        let ops = &ins.operands;
        match ins.opcode {
            2 => {
                let (op, lhs_scope, rhs_scope) = (ops[5], ops[6], ops[7]);
                mark(rhs_scope, word_i16(ops, 2));
                if !matches!(op, 5 | 20) || lhs_scope == 22 {
                    mark(lhs_scope, word_i16(ops, 0));
                }
            }
            0..=255 => {
                for use_ in primitive_frame_uses(ins.opcode, ops) {
                    match use_ {
                        FrameUse::Read(scope, idx, _) | FrameUse::Dst(scope, idx, _) => {
                            mark(scope, idx);
                        }
                        FrameUse::WholeFrame => return BTreeSet::new(),
                        FrameUse::AllLocals(_) => {}
                    }
                }
            }
            _ => {}
        }
    }
    (0..args).filter(|&i| !read[i]).collect()
}

/// What a built-in primitive that cannot observe the scratch-pointer state does to it,
/// or `None` for one that may observe it.
///
/// In `sims-vm` only two readers look at the state (`ScratchPtrs::get`/`has`):
/// `Expression` with a Global 34 right operand (`expression.rs:34`) and `UI Effect`
/// reading Global 34 (`ui_effect.rs:371`). A primitive therefore observes the state its
/// node starts with only if it is one of those, or if code reads it while the primitive
/// runs: a tree it runs (Push Interaction runs an at-once interaction's trees,
/// `in_way.rs:674`; Find Best Object, Create, Reach, Run Tree, Idle, ...), or anything
/// that runs while it yields. The primitives listed here return `bool` from their
/// handler (`primitives.rs:126`), so they never yield, and none of them runs a tree,
/// removes an object or calls a routine that does:
///
/// - these leave the state as it was (`Some(None)`): Refresh 7, Own Events 10, Get
///   Distance To 11, Breakpoint 15, Show String 21, Play Sound 23, Old Relationship 24
///   (`relationship.rs:171`), Alter Budget 25, Set Motive Change 29, Test Object Type
///   32, Find Worst Motives 33, Sim Interacting With 37, Stop All Sounds 48;
/// - these set it before anything else, so before any fault too (`Some(Some(_))`):
///   Random Number 8 (`random.rs:22`; its scope check comes after), Set To Next 31
///   (`set_to_next.rs:116`), and Get Direction To 12 (`direction.rs:49`) when its
///   destination scope word fits a byte: it checks that word first (`direction.rs:45`),
///   and a fault there leaves the state as it was, so then it counts as observing.
///
/// For these, the state before the node is observed only through what follows: the
/// state they leave (compared at the next event or exit) and, when they fault, the state
/// at the fault (compared at that exit).
fn scratch_effect(opcode: u16, ops: &[u8; 8]) -> Option<Option<Scratch>> {
    match opcode {
        7 | 10 | 11 | 15 | 21 | 23 | 24 | 25 | 29 | 32 | 33 | 37 | 48 => Some(None),
        8 => Some(Some(Scratch::ArgSlot)),
        12 if u16::from_le_bytes([ops[2], ops[3]]) <= 255 => Some(Some(Scratch::ArgSlot)),
        31 => Some(Some(Scratch::SetToNext)),
        _ => None,
    }
}

// ============================================================================
// Events and trace prefixes
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum EventKind {
    /// A primitive or subroutine call.
    Primitive {
        /// The node it runs at: a primitive that yields (or a tree it runs) keeps this
        /// frame on the stack at that node.
        node: u8,
        opcode: u16,
        operands: [u8; 8],
        /// Extended inline call arguments read from the following instructions.
        inline_args: Vec<u8>,
        /// The whole instruction suffix after a call whose callee arity is unknown.
        unknown_arity_suffix: Option<Vec<u8>>,
        /// The called tree's argument count in this side's file, for subroutine calls.
        callee_arity: Option<usize>,
        /// Whether this side's file says the called tree reads no operand bytes, so
        /// `operands` is the normalized `[0xFF; 8]` ([`LiveContext::argless_callees`]).
        argless: bool,
        /// `(params, locals)` of the running frame. Frame storage is one word stack,
        /// so a tree the primitive runs (a call, or a callback such as a routing door
        /// tree or Idle For Input's interaction) starts after this frame and its
        /// initial locals are whatever an earlier frame left there
        /// (`interpreter.rs:1195`).
        caller_frame: (usize, usize),
    },
    /// An `Expression` read of an untracked operand.
    Read {
        scope: u8,
        idx: i16,
        indirect: Option<Val>,
    },
    /// An `Expression` that writes an untracked operand (or a mutating compare on one).
    Write {
        op: u8,
        scope: u8,
        idx: i16,
        indirect: Option<Val>,
        rhs: Val,
    },
}

/// A frame slot a primitive reads or writes, with what the checker knows about it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum FrameVal {
    /// A read of `param`/`local` `idx`: its value, or where it falls outside the frame
    /// ([`Outside`]), and whether it is a local the tree may not have written yet.
    Read(u8, i16, Result<Val, Outside>, bool),
    /// A destination `param`/`local` `idx`: inside the frame, or where outside it.
    Dst(u8, i16, Result<(), Outside>),
    /// Every local, and which may be unwritten (text formatting `$Local` tokens).
    AllLocals(Vec<Val>, Vec<bool>),
    /// The locals text formatting may read, from the file's own strings
    /// ([`LiveContext::text_reads`]): each slot, its value or where it falls outside the
    /// frame, and whether it may be unwritten.
    TextLocals(Vec<(i16, Result<Val, Outside>, bool)>),
    /// Every parameter and local, and which locals may be unwritten (unmodeled primitive).
    WholeFrame(Vec<Val>, Vec<Val>, Vec<bool>),
    /// The arguments a call passes, whether from its operand words or the temps; `None`
    /// for one the callee overwrites unseen ([`LiveContext::dead_args`]). The call's
    /// operands are then compared as zeros: with at most 4 arguments and a callee that is
    /// not indexed, the VM reads nothing else of them (`interpreter.rs:1539`,
    /// `entry_node`).
    CallArgs(Vec<Option<Val>>),
}

/// The local number a [`FrameVal::Read`] shows for a local a primitive acts on by value
/// only, under the behavioral relation (`Explorer::value_read_locals`).
const BY_VALUE: i16 = i16::MIN;

/// How far past the end of its scope a frame operand falls (`idx - count`; negative for
/// an index below zero).
///
/// Primitives check frame bounds themselves and fail differently one slot past the end
/// than further out (Push Interaction, `push.rs`). Strict mode already observes the frame
/// size through `caller_frame`; the behavioral relation does not, so a primitive's frame
/// operand keeps this instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Outside(i32);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LiveEvent {
    kind: EventKind,
    frame: Vec<FrameVal>,
    /// The epoch the store snapshot is relative to (fixed by the history before it).
    epoch: u32,
    store: Vec<(Cell, Val)>,
    scratch: Option<Scratch>,
    /// The thread's last branch result before a primitive (`None` inside: entry value).
    last_result: Option<Option<bool>>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum PrefixNode {
    Root,
    Event(u32, LiveEvent),
    Branch(u32, bool),
    /// Names the atom "the event at this prefix faulted".
    Fault(u32),
    /// The start of a segment of the cut-point bisimulation ([`bisimulate`]): whatever
    /// equal history both runs had when they reached that product state. Unique per
    /// product state.
    Cut(u32),
}

/// Shared by both trees so identical keys get identical ids.
struct Interner {
    syms: BTreeMap<SymKey, u32>,
    sym_keys: Vec<SymKey>,
    prefixes: BTreeMap<PrefixNode, u32>,
    prefix_nodes: Vec<PrefixNode>,
    /// Cut roots handed out so far ([`Interner::fresh_cut`]).
    cuts: u32,
}

impl Interner {
    fn new() -> Self {
        let mut prefixes = BTreeMap::new();
        prefixes.insert(PrefixNode::Root, 0);
        Self {
            syms: BTreeMap::new(),
            sym_keys: Vec::new(),
            prefixes,
            prefix_nodes: vec![PrefixNode::Root],
            cuts: 0,
        }
    }

    /// A new segment root, distinct from every prefix handed out before.
    fn fresh_cut(&mut self) -> u32 {
        self.cuts += 1;
        self.prefix(PrefixNode::Cut(self.cuts))
    }

    fn sym(&mut self, key: SymKey) -> Val {
        // One search, hit or miss: keys can be long to compare.
        match self.syms.entry(key) {
            Entry::Occupied(e) => Val::Sym(*e.get()),
            Entry::Vacant(e) => {
                let id = self.sym_keys.len() as u32;
                self.sym_keys.push(e.key().clone());
                Val::Sym(*e.insert(id))
            }
        }
    }

    fn lookup(&self, key: &SymKey) -> Option<Val> {
        self.syms.get(key).map(|&id| Val::Sym(id))
    }

    fn key(&self, id: u32) -> &SymKey {
        &self.sym_keys[id as usize]
    }

    /// Whether symbol `id` is an input the checker may pick freely: a value the tree
    /// starts from. Values a primitive produced or may have changed can be constrained
    /// by what the primitive does (a `Random` result is below its bound), and a read of
    /// unmodeled memory may only ever return some values (`memory.rs:542`), so a
    /// difference in them is only a possible one.
    fn is_input(&self, id: u32) -> bool {
        matches!(
            self.key(id),
            SymKey::InitParam(_) | SymKey::InitLocal(_) | SymKey::Cell(0, _)
        )
    }

    fn is_input_val(&self, v: Val) -> bool {
        matches!(v, Val::Sym(id) if self.is_input(id))
    }

    fn prefix(&mut self, node: PrefixNode) -> u32 {
        match self.prefixes.entry(node) {
            Entry::Occupied(e) => *e.get(),
            Entry::Vacant(e) => {
                let id = self.prefix_nodes.len() as u32;
                self.prefix_nodes.push(e.key().clone());
                *e.insert(id)
            }
        }
    }

    /// The events (with branch outcomes) leading to `prefix`, oldest first, back to the
    /// entry or to the cut the segment started at.
    fn history(&self, mut prefix: u32) -> Vec<&PrefixNode> {
        let mut out = Vec::new();
        while prefix != 0 {
            let node = &self.prefix_nodes[prefix as usize];
            prefix = match node {
                PrefixNode::Root | PrefixNode::Cut(_) => break,
                PrefixNode::Event(parent, _)
                | PrefixNode::Branch(parent, _)
                | PrefixNode::Fault(parent) => *parent,
            };
            out.push(node);
        }
        out.reverse();
        out
    }

    fn describe(&self, v: Val) -> String {
        self.describe_depth(v, 0)
    }

    /// Renders `v`, cutting operator terms off below a few levels: terms share
    /// subterms, so a loop that doubles a value would otherwise print exponentially.
    fn describe_depth(&self, v: Val, depth: usize) -> String {
        match v {
            Val::Const(c) => c.to_string(),
            Val::Sym(id) => match self.key(id) {
                SymKey::InitParam(i) => format!("param[{i}]@entry"),
                SymKey::InitLocal(w) => format!("frame_word[{w}]@entry"),
                SymKey::Cell(p, Cell::Temp(i)) => format!("temp[{i}]@epoch{p}"),
                SymKey::Cell(p, Cell::Attr(i)) => format!("my.attr[{i}]@epoch{p}"),
                SymKey::EventValue(p) => format!("read#{p}"),
                SymKey::StackObject(p) => format!("stack_obj@epoch{p}"),
                SymKey::FrameMaybe(p, s, i, _) => format!("frame({s},{i})@event{p}"),
                SymKey::Drawn(p) => format!("random@event{p}"),
                SymKey::MyData(p, i) => format!("my.data[{i}]@event{p}"),
                SymKey::CutClass(p, k) => format!("class{k}@cut{p}"),
                SymKey::Op(..) if depth >= 3 => format!("t{id}"),
                SymKey::Op(op, a, b) => format!(
                    "({} op{op} {})",
                    self.describe_depth(*a, depth + 1),
                    self.describe_depth(*b, depth + 1)
                ),
            },
        }
    }
}

// ============================================================================
// Path conditions
// ============================================================================

/// A canonical branch predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Atom {
    /// `l < r`.
    Lt(Val, Val),
    /// `l == r` (operands ordered).
    Eq(Val, Val),
    /// `l & bit(r) != 0` (`expression.rs:147`).
    Flag(Val, Val),
    /// Whether the event at this prefix took its `true` branch.
    Outcome(u32),
}

fn eq_atom(a: Val, b: Val) -> Atom {
    if b < a {
        Atom::Eq(b, a)
    } else {
        Atom::Eq(a, b)
    }
}

fn flag_bit(r: i32) -> i32 {
    // Flags count from 1; x86 masks the shift count, so flag 0 tests the sign bit.
    1i32.wrapping_shl((r - 1) as u32)
}

/// Evaluates an atom whose operands make it constant.
fn eval_atom(atom: Atom) -> Option<bool> {
    match atom {
        Atom::Lt(Val::Const(l), Val::Const(r)) => Some(l < r),
        Atom::Lt(l, r) if l == r => Some(false),
        Atom::Eq(Val::Const(l), Val::Const(r)) => Some(l == r),
        Atom::Eq(l, r) if l == r => Some(true),
        Atom::Flag(Val::Const(l), Val::Const(r)) if (0..=16).contains(&r) => {
            Some(i32::from(l) & flag_bit(i32::from(r)) != 0)
        }
        _ => None,
    }
}

/// Interval `[lo, hi]` minus `excluded` that a symbol may take under a path condition.
#[derive(Debug, Clone)]
struct Range {
    lo: i32,
    hi: i32,
    excluded: BTreeSet<i32>,
}

impl Range {
    fn full() -> Self {
        Self {
            lo: i32::from(i16::MIN),
            hi: i32::from(i16::MAX),
            excluded: BTreeSet::new(),
        }
    }

    fn apply(&mut self, atom: Atom, value: bool, sym: u32) {
        let is = |v: Val| v == Val::Sym(sym);
        match atom {
            Atom::Lt(t, Val::Const(c)) if is(t) => {
                if value {
                    self.hi = self.hi.min(i32::from(c) - 1);
                } else {
                    self.lo = self.lo.max(i32::from(c));
                }
            }
            Atom::Lt(Val::Const(c), t) if is(t) => {
                if value {
                    self.lo = self.lo.max(i32::from(c) + 1);
                } else {
                    self.hi = self.hi.min(i32::from(c));
                }
            }
            Atom::Eq(Val::Const(c), t) | Atom::Eq(t, Val::Const(c)) if is(t) => {
                if value {
                    self.lo = self.lo.max(i32::from(c));
                    self.hi = self.hi.min(i32::from(c));
                } else {
                    self.excluded.insert(i32::from(c));
                }
            }
            _ => {}
        }
    }

    fn count(&self) -> i64 {
        if self.lo > self.hi {
            return 0;
        }
        let span = i64::from(self.hi) - i64::from(self.lo) + 1;
        let holes = self.excluded.range(self.lo..=self.hi).count() as i64;
        span - holes
    }

    /// The allowed value closest to `target`, if any.
    fn pick_near(&self, target: i16) -> Option<i16> {
        if self.count() <= 0 {
            return None;
        }
        let t = i32::from(target).clamp(self.lo, self.hi);
        let ok = |v: i32| (self.lo..=self.hi).contains(&v) && !self.excluded.contains(&v);
        (0..=self.excluded.len() as i32 + 1)
            .flat_map(|d| [t + d, t - d])
            .find(|&v| ok(v))
            .map(|v| v as i16)
    }

    fn single(&self) -> Option<i16> {
        if self.count() != 1 {
            return None;
        }
        (self.lo..=self.hi)
            .find(|v| !self.excluded.contains(v))
            .map(|v| v as i16)
    }
}

/// The symbol an atom constrains against a literal, if it has that shape.
fn atom_sym_vs_const(atom: Atom) -> Option<u32> {
    match atom {
        Atom::Lt(Val::Sym(s), Val::Const(_))
        | Atom::Lt(Val::Const(_), Val::Sym(s))
        | Atom::Eq(Val::Const(_), Val::Sym(s))
        | Atom::Eq(Val::Sym(s), Val::Const(_)) => Some(s),
        _ => None,
    }
}

fn range_of(cond: &[(Atom, bool)], sym: u32) -> Range {
    let mut range = Range::full();
    for &(atom, value) in cond {
        if atom_sym_vs_const(atom) == Some(sym) {
            range.apply(atom, value, sym);
        }
    }
    range
}

/// Whether `cond` already decides `atom`.
fn decide(cond: &[(Atom, bool)], atom: Atom) -> Option<bool> {
    if let Some(v) = eval_atom(atom) {
        return Some(v);
    }
    if let Some(&(_, v)) = cond.iter().find(|(a, _)| *a == atom) {
        return Some(v);
    }
    let sym = atom_sym_vs_const(atom)?;
    let range = range_of(cond, sym);
    let mut if_true = range.clone();
    if_true.apply(atom, true, sym);
    let mut if_false = range;
    if_false.apply(atom, false, sym);
    match (if_true.count() > 0, if_false.count() > 0) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

/// Joint satisfiability of path conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Feasibility {
    Unsat,
    /// Satisfiable in the checker's model; `exact` when every constraint is a literal
    /// bound on an independent input (so the satisfying assignment really exists).
    Sat {
        exact: bool,
    },
}

fn feasibility(conds: &[&[(Atom, bool)]], interner: &Interner) -> Feasibility {
    // Sorted, an atom taken both ways is two neighbours with the same atom.
    let mut literals: Vec<(Atom, bool)> = conds.iter().flat_map(|c| c.iter().copied()).collect();
    literals.sort_unstable();
    literals.dedup();
    if literals.windows(2).any(|w| w[0].0 == w[1].0) {
        return Feasibility::Unsat;
    }
    let mut ranges: BTreeMap<u32, Range> = BTreeMap::new();
    let mut exact = true;
    // Ranges only narrow, so the order literals are applied in does not matter.
    for &(atom, value) in &literals {
        if let Some(v) = eval_atom(atom)
            && v != value
        {
            return Feasibility::Unsat;
        }
        match atom {
            Atom::Outcome(_) => {}
            _ => match atom_sym_vs_const(atom) {
                Some(sym) => {
                    ranges
                        .entry(sym)
                        .or_insert_with(Range::full)
                        .apply(atom, value, sym);
                    if !interner.is_input(sym) {
                        exact = false;
                    }
                }
                None => exact = false,
            },
        }
    }
    if ranges.values().any(|r| r.count() <= 0) {
        return Feasibility::Unsat;
    }
    Feasibility::Sat { exact }
}

// ============================================================================
// Paths
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Exit {
    True,
    False,
    Error,
    /// A VM trap the checker models (`sims-vm` native error code).
    Trap(u16),
    /// A jump to a node past the end of the tree: the target and the tree's length,
    /// which are both in the VM's error (`Trap::InvalidBranch`, `interpreter.rs:1484`).
    BadNode(u8, usize),
    /// A primitive or call that failed (its error is not modeled).
    PrimitiveFault,
    /// An operand the checker does not track, or a local the tree may not have
    /// written, that the VM failed to read or write (its error is not modeled).
    OperandFault,
}

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::True => write!(f, "return true"),
            Self::False => write!(f, "return false"),
            Self::Error => write!(f, "error"),
            Self::Trap(NEGATIVE_SQRT) => write!(f, "square root of a negative number"),
            Self::Trap(code) => write!(f, "VM trap {code:#x}"),
            Self::BadNode(n, count) => {
                write!(f, "jump to missing node {n} of a {count}-node tree")
            }
            Self::PrimitiveFault => write!(f, "a failing primitive or call"),
            Self::OperandFault => write!(f, "a failing operand"),
        }
    }
}

/// Everything about a path the rest of the game can observe.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Outputs {
    exit: Exit,
    /// For a path that faults (a trap, the error target, a bad branch, a primitive or
    /// call that fails): where, and from what state (see [`FaultPoint`]).
    fault: Option<FaultPoint>,
    /// The event history (interned), which fixes the ordered trace.
    prefix: u32,
    /// The epoch `store` is relative to (fixed by `prefix`).
    epoch: u32,
    store: Vec<(Cell, Val)>,
    scratch: Option<Scratch>,
    /// The last branch result at exit (`Some(None)`: the caller's, nothing ran).
    last_result: Option<Option<bool>>,
    /// Final parameters, locals, and which locals may be unwritten.
    residue: Option<(Vec<Val>, Vec<Val>, Vec<bool>)>,
}

/// Where a path faults: the node it faults at, that node's instruction, and the whole
/// state before the node ran. A fault leaves the faulting frame on the stack at that
/// node, with whatever the node had done to the frame words, their unknown markers and
/// the last branch result by then (`interpreter.rs:809`, `expression.rs:69`). The
/// checker does not model that partial state; it only relies on it being a function of
/// the state before the node and the node's bytes, so two faults are equal only when
/// all of those are.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FaultPoint {
    /// The tree's ID: a fault names the tree it happened in (`fault.rs`, `Location`).
    /// Trees with different IDs are already Inconclusive on entry; this keeps the
    /// fault point a whole description of the VM's error location on its own.
    tree: u16,
    /// `u8::MAX` for an entry past the last node, which faults before any node runs.
    node: u8,
    /// Opcode and operands. The branch targets are left out: a node that faults
    /// inside never reaches them, and one that faults by branching (to the error exit
    /// or a missing node) shows the target in its exit.
    instruction: Option<(u16, [u8; 8])>,
    prefix: u32,
    /// For a primitive or call, the prefix after its event (which also fixes the inline
    /// arguments it reads from the following nodes).
    event: Option<u32>,
    epoch: u32,
    store: Vec<(Cell, Val)>,
    scratch: Scratch,
    last_result: Option<bool>,
    params: Vec<Val>,
    locals: Vec<Val>,
    unwritten: Vec<bool>,
}

impl FaultPoint {
    /// A fault as the behavioral relation sees it: only the tree it happened in. The
    /// rest of what a fault leaves that the world can see is in the path's [`Outputs`]
    /// at the moment it faulted: the kind of fault (`exit`), the event history up to the
    /// failing event (which names a failing primitive, call or operand, and what a
    /// primitive did before failing), the tracked cells, the scratch pointer, and the
    /// last result. That relies on the checker applying a node's effects on those in the
    /// VM's order: an `Expression` sets the scratch pointer after reading its right
    /// operand and before resolving its left one (`expression.rs:43`, `expression.rs:51`),
    /// writes its destination last, and a node sets the last result only once it
    /// succeeds (`interpreter.rs:1479`). The node number and the frame are not observed.
    fn behavioral(tree: u16) -> Self {
        Self {
            tree,
            node: 0,
            instruction: None,
            prefix: 0,
            event: None,
            epoch: 0,
            store: Vec::new(),
            scratch: Scratch::Inherited,
            last_result: None,
            params: Vec::new(),
            locals: Vec::new(),
            unwritten: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct PathSummary {
    out: Outputs,
    cond: Vec<(Atom, bool)>,
    nodes: Vec<u8>,
    entry: usize,
}

#[derive(Debug, Clone)]
struct PathState {
    pc: u8,
    /// The trace so far.
    prefix: u32,
    /// The last point that may have written shared memory; untouched tracked cells hold
    /// their value from then.
    epoch: u32,
    params: Vec<Val>,
    locals: Vec<Val>,
    store: BTreeMap<Cell, Val>,
    scratch: Scratch,
    /// The thread's last branch result (`None`: still the caller's).
    last_result: Option<bool>,
    /// Locals the tree may not have written yet. Reading one can fail in the VM when
    /// it starts as half of a pointer an earlier frame left (`interpreter.rs:1277`).
    unwritten: Vec<bool>,
    cond: Vec<(Atom, bool)>,
    steps: usize,
    nodes: Vec<u8>,
    /// The node running now and the state before it ran (`None` before the first).
    at: Option<FaultPoint>,
}

impl PathState {
    /// A copy for one side of a split. Unlike `clone`, it leaves room for the next few
    /// conditions and nodes, so the push that follows does not copy them again.
    fn fork(&self) -> Self {
        let mut cond = Vec::with_capacity(self.cond.len() + 8);
        cond.extend_from_slice(&self.cond);
        let mut nodes = Vec::with_capacity(self.nodes.len() + 16);
        nodes.extend_from_slice(&self.nodes);
        Self {
            pc: self.pc,
            prefix: self.prefix,
            epoch: self.epoch,
            params: self.params.clone(),
            locals: self.locals.clone(),
            store: self.store.clone(),
            scratch: self.scratch,
            last_result: self.last_result,
            unwritten: self.unwritten.clone(),
            cond,
            steps: self.steps,
            nodes,
            at: self.at.clone(),
        }
    }
}

/// Where an `Expression` operand lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Loc {
    Literal(i16),
    Param(usize),
    Local(usize),
    Tracked(Cell),
    Trap(u16),
    /// Memory the checker does not track; `indirect` is `param[idx]` for scope 22.
    Opaque(Option<Val>),
}

enum Step {
    Continue(PathState),
    Done(PathSummary),
}

struct Explorer<'a> {
    bhav: &'a BhavChunk,
    ctx: &'a LiveContext,
    opts: &'a LiveOptions,
    args: usize,
    locals: usize,
    entry: usize,
    /// For a kind-4 tree, the raw entry the call passed, which is also `param[0]`
    /// (`interpreter.rs:964`): exactly this value, or (`false`) at least it, for the
    /// last node, which every larger entry is clamped to.
    entry_param: Option<(i16, bool)>,
    /// Bounds the entry assumes on parameters, `(index, lo, hi)` (a domain region).
    region: &'a [(usize, i16, i16)],
    /// End every path right after its first primitive or call event
    /// ([`decisive_difference`]).
    stop_at_primitive: bool,
}

/// `(params, locals)` the VM gives a frame running `bhav` (`sims_vm::interpreter::dimensions`).
pub(super) fn frame_dims(bhav: &BhavChunk) -> (usize, usize) {
    let args = match bhav.format {
        0x8000 => 4,
        0x8001 => usize::from(bhav.args).min(4),
        _ => usize::from(bhav.args),
    };
    // `sims_vm::interpreter::dimensions` truncates the local count to a byte.
    let locals = match bhav.format {
        0x8000 => 0,
        0x8001 => usize::from(bhav.locals as u8).min(4),
        _ => usize::from(bhav.locals as u8),
    };
    (args, locals)
}

impl<'a> Explorer<'a> {
    fn new(
        bhav: &'a BhavChunk,
        ctx: &'a LiveContext,
        opts: &'a LiveOptions,
        entry: usize,
        entry_param: Option<(i16, bool)>,
        region: &'a [(usize, i16, i16)],
    ) -> Self {
        let (args, locals) = frame_dims(bhav);
        Self {
            bhav,
            ctx,
            opts,
            args,
            locals,
            entry,
            entry_param,
            region,
            stop_at_primitive: false,
        }
    }

    fn initial(&self, interner: &mut Interner) -> PathState {
        let params = (0..self.args)
            .map(|i| interner.sym(SymKey::InitParam(i as u16)))
            .collect();
        let locals = (0..self.locals)
            .map(|i| interner.sym(SymKey::InitLocal((self.args + i) as u16)))
            .collect();
        let mut params: Vec<Val> = params;
        let mut cond = Vec::new();
        if let (Some((raw, exact)), Some(first)) = (self.entry_param, params.first_mut()) {
            if exact {
                *first = Val::Const(raw);
            } else if raw > 0 {
                cond.push((Atom::Lt(Val::Const(raw - 1), *first), true));
            }
        }
        for &(i, lo, hi) in self.region {
            let Some(&p) = params.get(i) else { continue };
            if lo > i16::MIN {
                cond.push((Atom::Lt(p, Val::Const(lo)), false));
            }
            if hi < i16::MAX {
                cond.push((Atom::Lt(Val::Const(hi), p), false));
            }
        }
        PathState {
            // An entry past the last node index fails before running anything; 251 is
            // the invalid target `step` treats that way.
            pc: if self.entry > LAST_NODE {
                251
            } else {
                self.entry as u8
            },
            prefix: 0,
            epoch: 0,
            params,
            locals,
            store: BTreeMap::new(),
            scratch: Scratch::Inherited,
            last_result: None,
            unwritten: vec![true; self.locals],
            cond,
            steps: 0,
            nodes: Vec::new(),
            at: None,
        }
    }

    /// The state before node `node` (`ins`) runs; see [`FaultPoint`].
    fn fault_point(
        &self,
        state: &PathState,
        node: u8,
        ins: Option<&Instruction>,
        interner: &Interner,
    ) -> FaultPoint {
        FaultPoint {
            tree: self.bhav.id,
            node,
            instruction: ins.map(|i| (i.opcode, self.operands(i))),
            prefix: state.prefix,
            event: None,
            epoch: state.epoch,
            store: self.normalized_store(state, interner),
            scratch: state.scratch,
            last_result: state.last_result,
            params: state.params.clone(),
            locals: state.locals.clone(),
            unwritten: state.unwritten.clone(),
        }
    }

    /// `ins`'s operands as the VM uses them: `[0xFF; 8]` for a call that reads none
    /// ([`LiveContext::call_reads_no_operands`]).
    fn operands(&self, ins: &Instruction) -> [u8; 8] {
        if self.ctx.call_reads_no_operands(ins.opcode) {
            [0xFF; 8]
        } else {
            ins.operands
        }
    }

    fn explore(&self, interner: &mut Interner) -> Result<(Vec<PathSummary>, usize), String> {
        let mut done = Vec::new();
        let mut stack = vec![self.initial(interner)];
        let mut steps = 0usize;
        while let Some(state) = stack.pop() {
            if done.len() + stack.len() > self.opts.max_paths {
                return Err(format!(
                    "tree {} has more than {} paths from entry node {}",
                    self.bhav.id, self.opts.max_paths, self.entry
                ));
            }
            steps += 1;
            for next in self.step(state, interner)? {
                match next {
                    Step::Continue(s) => stack.push(s),
                    Step::Done(summary) => done.push(summary),
                }
            }
        }
        Ok((done, steps))
    }

    fn finish(&self, mut state: PathState, exit: Exit, interner: &mut Interner) -> Step {
        let cond = std::mem::take(&mut state.cond);
        let nodes = std::mem::take(&mut state.nodes);
        let at = state.at.take();
        Step::Done(self.summarize(&state, at, cond, nodes, exit, interner))
    }

    /// Finishes a copy of `state` that also assumes `literal`, without copying the
    /// state: a faulting branch off a path that goes on. `event`, when given, is the
    /// event the fault point records.
    fn finish_branch(
        &self,
        state: &PathState,
        literal: (Atom, bool),
        event: Option<u32>,
        exit: Exit,
        interner: &mut Interner,
    ) -> Step {
        let mut cond = Vec::with_capacity(state.cond.len() + 1);
        cond.extend_from_slice(&state.cond);
        cond.push(literal);
        let mut at = state.at.clone();
        if let (Some(at), Some(event)) = (at.as_mut(), event) {
            at.event = Some(event);
        }
        Step::Done(self.summarize(state, at, cond, state.nodes.clone(), exit, interner))
    }

    fn summarize(
        &self,
        state: &PathState,
        at: Option<FaultPoint>,
        cond: Vec<(Atom, bool)>,
        nodes: Vec<u8>,
        exit: Exit,
        interner: &mut Interner,
    ) -> PathSummary {
        let store = self.normalized_store(state, interner);
        let faults = !matches!(exit, Exit::True | Exit::False);
        // A fault leaves the faulting frame on the stack and the thread's last result
        // in the object (`interpreter.rs:809`), so a faulting path compares the whole
        // state at the faulting node, whatever the observation policy says.
        let fault = faults.then(|| {
            if self.opts.behavioral {
                return FaultPoint::behavioral(self.bhav.id);
            }
            // An entry past the last node faults before any node runs; it is never
            // compared (`check_bhav_live_equivalent`).
            let mut at = at.unwrap_or_else(|| self.fault_point(state, u8::MAX, None, interner));
            if !self.opts.observe_fault_last_result {
                at.last_result = None;
            }
            if !self.opts.observe_fault_scratch_pointer {
                at.scratch = Scratch::Inherited;
            }
            at
        });
        // The behavioral relation does not observe node numbers: a jump to a missing
        // node is the same fault whatever its target and the tree's length.
        let exit = match exit {
            Exit::BadNode(..) if self.opts.behavioral => Exit::BadNode(0, 0),
            other => other,
        };
        let residue =
            (self.opts.observes_residue() || (faults && !self.opts.behavioral)).then(|| {
                (
                    state.params.clone(),
                    state.locals.clone(),
                    state.unwritten.clone(),
                )
            });
        let observed = if faults {
            self.opts.observe_fault_last_result
        } else {
            self.opts.observe_last_result
        };
        let last_result = observed.then_some(state.last_result);
        PathSummary {
            out: Outputs {
                exit,
                fault,
                prefix: state.prefix,
                epoch: state.epoch,
                store,
                scratch: if faults {
                    self.opts.observe_fault_scratch_pointer
                } else {
                    self.opts.observe_scratch_pointer
                }
                .then_some(state.scratch),
                last_result,
                residue,
            },
            cond,
            nodes,
            entry: self.entry,
        }
    }

    /// The tracked cells that differ from their value at the start of the epoch.
    fn normalized_store(&self, state: &PathState, interner: &Interner) -> Vec<(Cell, Val)> {
        state
            .store
            .iter()
            .filter(|&(&cell, &val)| interner.lookup(&SymKey::Cell(state.epoch, cell)) != Some(val))
            .map(|(&c, &v)| (c, v))
            .collect()
    }

    fn step(&self, mut state: PathState, interner: &mut Interner) -> Result<Vec<Step>, String> {
        let pc = state.pc;
        match pc {
            TARGET_TRUE => return Ok(vec![self.finish(state, Exit::True, interner)]),
            TARGET_FALSE => return Ok(vec![self.finish(state, Exit::False, interner)]),
            TARGET_ERROR => return Ok(vec![self.finish(state, Exit::Error, interner)]),
            // Targets past the last node index (250) other than the three exits are
            // invalid, whatever the tree's length (`interpreter.rs:1695`).
            251..=252 => {
                return Ok(vec![self.finish(state, Exit::Trap(INVALID_NODE), interner)]);
            }
            _ => {}
        }
        let Some(ins) = self.bhav.instructions.get(usize::from(pc)) else {
            let count = self.bhav.instructions.len();
            return Ok(vec![self.finish(state, Exit::BadNode(pc, count), interner)]);
        };
        state.steps += 1;
        if state.steps > self.opts.max_steps {
            return Err(format!(
                "a path through tree {} runs more than {} instructions (loop not bounded by literals)",
                self.bhav.id, self.opts.max_steps
            ));
        }
        state.nodes.push(pc);
        state.at = Some(self.fault_point(&state, pc, Some(ins), interner));
        if ins.opcode == 2 {
            self.expression(state, ins, interner)
        } else {
            self.primitive(state, ins, interner)
        }
    }

    // ------------------------------------------------------------------------
    // Operands
    // ------------------------------------------------------------------------

    fn locate(&self, state: &PathState, scope: u8, idx: i16) -> Result<Loc, String> {
        let in_range = |count: usize| idx >= 0 && (idx as usize) < count;
        Ok(match scope {
            7 => Loc::Literal(idx),
            9 if in_range(self.args) => Loc::Param(idx as usize),
            9 => Loc::Trap(8),
            25 if in_range(self.locals) => Loc::Local(idx as usize),
            25 => Loc::Trap(8),
            8 if (0..TEMP_COUNT).contains(&idx) => Loc::Tracked(Cell::Temp(idx)),
            8 => Loc::Trap(2),
            0 if idx < 0 => Loc::Trap(6),
            0 if in_range(self.ctx.my_attribute_count) => Loc::Tracked(Cell::Attr(idx)),
            6 if idx == 34 => {
                return Err(
                    "Global 34 operand (scratch-pointer redirection, expression.rs:31) is not modeled"
                        .to_owned(),
                );
            }
            10 if idx != 0 => Loc::Trap(0xf),
            22 if in_range(self.args) => Loc::Opaque(Some(state.params[idx as usize])),
            22 => Loc::Trap(8),
            26 => match self.bcon(idx) {
                Some(v) => Loc::Literal(v),
                None => Loc::Opaque(None),
            },
            2 | 5 | 39..=255 => Loc::Trap(9),
            _ => Loc::Opaque(None),
        })
    }

    /// The locals primitive `opcode` with `ops` only reads, under the behavioral
    /// relation: those a renamable operand names ([`local_operands`]) that the primitive
    /// does not also write ([`primitive_frame_uses`]' destinations,
    /// [`primitive_frame_writes`]). The relation does not observe frame storage, and a
    /// primitive acts on such a local's value only, which nothing but the running tree
    /// writes, also across a yield (Notify Out of Idle writes only parameters). So the
    /// event records each one's value under [`BY_VALUE`], and its operand bytes as 0.
    fn value_read_locals(&self, opcode: u16, ops: &[u8; 8]) -> BTreeSet<i16> {
        if !self.opts.behavioral || opcode >= 256 {
            return BTreeSet::new();
        }
        let mut written: BTreeSet<i16> = primitive_frame_writes(opcode, ops)
            .into_iter()
            .filter(|&(scope, _)| scope == 25)
            .map(|(_, idx)| idx)
            .collect();
        for frame_use in primitive_frame_uses(opcode, ops) {
            match frame_use {
                FrameUse::Dst(25, idx, _) => {
                    written.insert(idx);
                }
                FrameUse::Read(..) | FrameUse::Dst(..) => {}
                // Never with a renamable operand; nothing is read by value.
                FrameUse::AllLocals(_) | FrameUse::WholeFrame => return BTreeSet::new(),
            }
        }
        local_operands(opcode, ops)
            .into_iter()
            .filter(|&(_, writes)| !writes)
            .map(|(pos, _)| pos.get(ops))
            .filter(|idx| !written.contains(idx))
            .collect()
    }

    fn bcon(&self, idx: i16) -> Option<i16> {
        let bcons = self.ctx.local_bcons.as_ref()?;
        if !(4096..=8191).contains(&self.bhav.id) {
            return None;
        }
        let address = idx as u16;
        let table = address >> 7;
        if table > 63 {
            return None;
        }
        bcons
            .get(&(table + 4096))?
            .get(usize::from(address & 127))
            .copied()
    }

    fn tracked(&self, state: &PathState, loc: Loc, interner: &mut Interner) -> Option<Val> {
        Some(match loc {
            Loc::Literal(v) => Val::Const(v),
            Loc::Param(i) => state.params[i],
            Loc::Local(i) => state.locals[i],
            Loc::Tracked(cell) => match state.store.get(&cell) {
                Some(&v) => v,
                None => interner.sym(SymKey::Cell(state.epoch, cell)),
            },
            Loc::Trap(_) | Loc::Opaque(_) => return None,
        })
    }

    fn snapshot(
        &self,
        state: &PathState,
        interner: &Interner,
    ) -> (Vec<(Cell, Val)>, Option<Scratch>) {
        (
            self.normalized_store(state, interner),
            self.opts.observe_scratch_pointer.then_some(state.scratch),
        )
    }

    /// Records an event and returns the prefix after it.
    fn record(
        &self,
        state: &PathState,
        kind: EventKind,
        frame: Vec<FrameVal>,
        last_result: Option<Option<bool>>,
        interner: &mut Interner,
    ) -> u32 {
        self.record_with(state, kind, frame, last_result, true, interner)
    }

    /// [`Self::record`], leaving the scratch-pointer state out unless `scratch_observed`.
    fn record_with(
        &self,
        state: &PathState,
        kind: EventKind,
        frame: Vec<FrameVal>,
        last_result: Option<Option<bool>>,
        scratch_observed: bool,
        interner: &mut Interner,
    ) -> u32 {
        let (store, scratch) = self.snapshot(state, interner);
        let scratch = scratch.filter(|_| scratch_observed);
        let event = LiveEvent {
            kind,
            frame,
            epoch: state.epoch,
            store,
            scratch,
            last_result,
        };
        interner.prefix(PrefixNode::Event(state.prefix, event))
    }

    /// Reads an operand: a value, or the trap reading it raises. A read the VM may fail
    /// in a way the checker does not model (memory it does not track, a local the tree
    /// may not have written) also leaves a faulting path in `out` ([`Self::may_fault`]).
    fn read(
        &self,
        state: &mut PathState,
        scope: u8,
        idx: i16,
        out: &mut Vec<Step>,
        interner: &mut Interner,
    ) -> Result<Result<Val, u16>, String> {
        if scope == 10 && idx == 0 {
            // The stack object is a field of the running frame (`memory.rs:474`,
            // `memory.rs:777`): reading it cannot fail and changes nothing, so it is
            // not an event.
            return Ok(Ok(interner.sym(SymKey::StackObject(state.epoch))));
        }
        if scope == 3 && (0..MY_DATA_WORDS).contains(&idx) && idx != MY_DATA_LOCKOUT {
            // See `MY_DATA_WORDS`. Only events change the running object's data words
            // (writes to scope 3 are opaque write events), so the value is keyed by the
            // last event, whatever kind: a read event of the lockout word wakes the
            // object (`memory.rs:137`) without ending the epoch.
            return Ok(Ok(interner.sym(SymKey::MyData(state.prefix, idx))));
        }
        let loc = self.locate(state, scope, idx)?;
        if let Loc::Trap(code) = loc {
            return Ok(Err(code));
        }
        self.note_unwritten_read(state, loc, scope, idx, out, interner);
        if let Some(v) = self.tracked(state, loc, interner) {
            return Ok(Ok(v));
        }
        let Loc::Opaque(indirect) = loc else {
            unreachable!("tracked locations return above")
        };
        // A read observes no scratch-pointer state: only an `Expression` or `UI Effect`
        // reading Global 34 does (`scratch_effect`), and Global 34 is refused. When it
        // faults, the state at the fault is compared at that exit.
        let prefix = self.record_with(
            state,
            EventKind::Read {
                scope,
                idx,
                indirect,
            },
            Vec::new(),
            None,
            false,
            interner,
        );
        state.prefix = prefix;
        // Any untracked operand may fail to resolve or read (`memory.rs:364`): an index
        // out of range (`global[64]`), a missing object, a scope the VM does not hold.
        self.may_fault(state, prefix, out, interner);
        Ok(Ok(interner.sym(SymKey::EventValue(prefix))))
    }

    /// Splits off a path on which the running node faults, after the event that ends at
    /// `prefix`, for a failure the checker does not model. As for primitives, the
    /// fault's condition is a fresh atom of the event: two trees agree on it only for
    /// the same event from the same history, and their faults are equal only from the
    /// same state at the same node ([`FaultPoint`]).
    fn may_fault(
        &self,
        state: &mut PathState,
        prefix: u32,
        out: &mut Vec<Step>,
        interner: &mut Interner,
    ) {
        let faults = interner.prefix(PrefixNode::Fault(prefix));
        out.push(self.finish_branch(
            state,
            (Atom::Outcome(faults), true),
            Some(prefix),
            Exit::OperandFault,
            interner,
        ));
        state.cond.push((Atom::Outcome(faults), false));
    }

    /// Records the read of a local the tree may not have written yet as an event: the
    /// VM can fail reading it (`interpreter.rs:1277`), so it is not a pure read, and
    /// leaves a faulting path in `out`.
    fn note_unwritten_read(
        &self,
        state: &mut PathState,
        loc: Loc,
        scope: u8,
        idx: i16,
        out: &mut Vec<Step>,
        interner: &mut Interner,
    ) {
        if let Loc::Local(i) = loc
            && state.unwritten[i]
        {
            let prefix = self.record_with(
                state,
                EventKind::Read {
                    scope,
                    idx,
                    indirect: None,
                },
                Vec::new(),
                None,
                false,
                interner,
            );
            state.prefix = prefix;
            self.may_fault(state, prefix, out, interner);
        }
    }

    fn write(&self, state: &mut PathState, loc: Loc, val: Val) {
        match loc {
            Loc::Param(i) => state.params[i] = val,
            Loc::Local(i) => {
                state.locals[i] = val;
                state.unwritten[i] = false;
            }
            Loc::Tracked(cell) => {
                state.store.insert(cell, val);
            }
            _ => unreachable!("only writable tracked locations are written"),
        }
    }

    /// After an event that may write shared memory: tracked cells and the scratch
    /// pointer are whatever the event left.
    fn havoc_shared(state: &mut PathState) {
        state.epoch = state.prefix;
        state.store.clear();
        state.scratch = Scratch::Inherited;
    }

    /// Splits `state` on `atom`, appending to `out` the branch that traps (`trap_when`)
    /// and returning the state that continues, if that branch is feasible.
    fn guard(
        &self,
        mut state: PathState,
        atom: Atom,
        trap_when: bool,
        code: u16,
        out: &mut Vec<Step>,
        interner: &mut Interner,
    ) -> Option<PathState> {
        match decide(&state.cond, atom) {
            Some(v) if v == trap_when => {
                out.push(self.finish(state, Exit::Trap(code), interner));
                None
            }
            Some(_) => Some(state),
            None => {
                out.push(self.finish_branch(
                    &state,
                    (atom, trap_when),
                    None,
                    Exit::Trap(code),
                    interner,
                ));
                state.cond.push((atom, !trap_when));
                Some(state)
            }
        }
    }

    /// Continues at `on_true` where `atom` holds and `on_false` where it does not. The
    /// instruction's own result (`last_result`) is the atom's value, negated for the
    /// `>=`, `<=` and `!=` tests, which take their false branch when the atom holds.
    fn branch(
        &self,
        state: PathState,
        atom: Atom,
        on_true: u8,
        on_false: u8,
        negated: bool,
        out: &mut Vec<Step>,
    ) {
        let go = |mut s: PathState, value: bool| {
            s.pc = if value { on_true } else { on_false };
            s.last_result = Some(value != negated);
            Step::Continue(s)
        };
        match decide(&state.cond, atom) {
            Some(value) => out.push(go(state, value)),
            None => {
                let mut t = state.fork();
                t.cond.push((atom, true));
                out.push(go(t, true));
                let mut f = state;
                f.cond.push((atom, false));
                out.push(go(f, false));
            }
        }
    }

    // ------------------------------------------------------------------------
    // Expression (opcode 2, `sims-vm/src/primitives/expression.rs`)
    // ------------------------------------------------------------------------

    fn expression(
        &self,
        mut state: PathState,
        ins: &Instruction,
        interner: &mut Interner,
    ) -> Result<Vec<Step>, String> {
        let ops = &ins.operands;
        let (lhs_idx, rhs_idx) = (word_i16(ops, 0), word_i16(ops, 2));
        let (op, lhs_scope, rhs_scope) = (ops[5], ops[6], ops[7]);
        if op > 20 {
            return Err(format!("unknown expression operator {op}"));
        }
        let mut out = Vec::new();
        // The right operand is read first (`expression.rs:43`).
        let rhs = match self.read(&mut state, rhs_scope, rhs_idx, &mut out, interner)? {
            Ok(v) => v,
            Err(code) => {
                out.push(self.finish(state, Exit::Trap(code), interner));
                return Ok(out);
            }
        };
        state.scratch = Scratch::Expression;
        let lhs_loc = self.locate(&state, lhs_scope, lhs_idx)?;
        if let Loc::Trap(code) = lhs_loc {
            {
                out.push(self.finish(state, Exit::Trap(code), interner));
                return Ok(out);
            };
        }

        // Comparisons read the left operand without writing it.
        if matches!(op, 0 | 1 | 2 | 8 | 14 | 15 | 16) {
            let lhs = match self.read(&mut state, lhs_scope, lhs_idx, &mut out, interner)? {
                Ok(v) => v,
                Err(code) => {
                    out.push(self.finish(state, Exit::Trap(code), interner));
                    return Ok(out);
                }
            };
            let (atom, t, f, negated) = match op {
                0 => (Atom::Lt(rhs, lhs), ins.on_true, ins.on_false, false),
                1 => (Atom::Lt(lhs, rhs), ins.on_true, ins.on_false, false),
                2 => (eq_atom(lhs, rhs), ins.on_true, ins.on_false, false),
                14 => (Atom::Lt(lhs, rhs), ins.on_false, ins.on_true, true),
                15 => (Atom::Lt(rhs, lhs), ins.on_false, ins.on_true, true),
                16 => (eq_atom(lhs, rhs), ins.on_false, ins.on_true, true),
                _ => {
                    // Flag test: the flag index must be 0..=16 (`expression.rs:139`).
                    let Some(s) = self.guard(
                        state,
                        Atom::Lt(rhs, Val::Const(0)),
                        true,
                        0x0d,
                        &mut out,
                        interner,
                    ) else {
                        return Ok(out);
                    };
                    let Some(s) = self.guard(
                        s,
                        Atom::Lt(Val::Const(16), rhs),
                        true,
                        0x0d,
                        &mut out,
                        interner,
                    ) else {
                        return Ok(out);
                    };
                    state = s;
                    (Atom::Flag(lhs, rhs), ins.on_true, ins.on_false, false)
                }
            };
            self.branch(state, atom, t, f, negated, &mut out);
            return Ok(out);
        }

        // Writes to memory the checker does not track are events.
        if let Loc::Opaque(indirect) = lhs_loc {
            let prefix = self.record(
                &state,
                EventKind::Write {
                    op,
                    scope: lhs_scope,
                    idx: lhs_idx,
                    indirect,
                    rhs,
                },
                Vec::new(),
                // A global's store may run code on this object: the only writable one
                // the checker does not refuse is global 3, which in co-op mode selects a
                // person and runs PLACED on them (`memory.rs:846`,
                // `overwrites_last_result_first`). So it records the last result.
                (self.opts.observe_last_result && lhs_scope == 6).then_some(state.last_result),
                interner,
            );
            state.prefix = prefix;
            // Resolving, reading or storing an untracked destination may fail
            // (`expression.rs:63`, `memory.rs:797`).
            self.may_fault(&mut state, prefix, &mut out, interner);
            // The `Expression` set the scratch pointer before the store, but the store
            // can run code (selecting a Sim runs its placement callback, `memory.rs:846`)
            // that leaves it in another state. So it is whatever the write event left
            // (`havoc_shared`), not `Scratch::Expression`.
            if matches!(op, 11 | 17) {
                let taken = state.fork();
                for (value, mut s) in [(true, taken), (false, state)] {
                    s.prefix = interner.prefix(PrefixNode::Branch(prefix, value));
                    s.cond.push((Atom::Outcome(prefix), value));
                    Self::havoc_shared(&mut s);
                    s.last_result = Some(value);
                    s.pc = if value { ins.on_true } else { ins.on_false };
                    out.push(Step::Continue(s));
                }
            } else {
                Self::havoc_shared(&mut state);
                state.last_result = Some(true);
                state.pc = ins.on_true;
                out.push(Step::Continue(state));
            }
            return Ok(out);
        }

        let writable = !matches!(lhs_loc, Loc::Literal(_));
        // `=` and square root mark a local written before reading it (`expression.rs:75`);
        // the other operators read it first.
        if !matches!(op, 5 | 20) {
            self.note_unwritten_read(&mut state, lhs_loc, lhs_scope, lhs_idx, &mut out, interner);
        }
        let lhs = self
            .tracked(&state, lhs_loc, interner)
            .expect("tracked left operand");

        if matches!(op, 11 | 17) {
            let delta = if op == 11 { 1 } else { -1 };
            let value = mk_op(3, lhs, Val::Const(delta), interner);
            if !writable {
                {
                    out.push(self.finish(state, Exit::Trap(0x13), interner));
                    return Ok(out);
                };
            }
            self.write(&mut state, lhs_loc, value);
            let atom = if op == 11 {
                Atom::Lt(value, rhs)
            } else {
                Atom::Lt(rhs, value)
            };
            self.branch(state, atom, ins.on_true, ins.on_false, false, &mut out);
            return Ok(out);
        }

        // Operator traps, in the order `expression.rs` checks them.
        let mut state = Some(state);
        match op {
            7 => {
                state = self.guard(
                    state.take().unwrap(),
                    eq_atom(rhs, Val::Const(0)),
                    true,
                    0x26,
                    &mut out,
                    interner,
                );
            }
            9 | 10 => {
                state = self.guard(
                    state.take().unwrap(),
                    Atom::Lt(rhs, Val::Const(0)),
                    true,
                    0x0d,
                    &mut out,
                    interner,
                );
                if let Some(s) = state.take() {
                    state = self.guard(
                        s,
                        Atom::Lt(Val::Const(16), rhs),
                        true,
                        0x0d,
                        &mut out,
                        interner,
                    );
                }
            }
            12 => {
                if !writable {
                    let s = state.take().unwrap();
                    out.push(self.finish(s, Exit::Trap(0x13), interner));
                } else {
                    state = self.guard(
                        state.take().unwrap(),
                        eq_atom(rhs, Val::Const(0)),
                        true,
                        0x26,
                        &mut out,
                        interner,
                    );
                }
            }
            20 => {
                // A negative square root needs the original's x87 policy (`expression.rs:206`).
                state = self.guard(
                    state.take().unwrap(),
                    Atom::Lt(rhs, Val::Const(0)),
                    true,
                    NEGATIVE_SQRT,
                    &mut out,
                    interner,
                );
            }
            _ => {}
        }
        let Some(mut state) = state else {
            return Ok(out);
        };
        if !writable {
            out.push(self.finish(state, Exit::Trap(0x13), interner));
            return Ok(out);
        }
        let value = mk_op(op, lhs, rhs, interner);
        self.write(&mut state, lhs_loc, value);
        state.last_result = Some(true);
        state.pc = ins.on_true;
        out.push(Step::Continue(state));
        Ok(out)
    }

    // ------------------------------------------------------------------------
    // Primitives and subroutine calls
    // ------------------------------------------------------------------------

    fn primitive(
        &self,
        mut state: PathState,
        ins: &Instruction,
        interner: &mut Interner,
    ) -> Result<Vec<Step>, String> {
        let argless = self.ctx.call_reads_no_operands(ins.opcode);
        let ops = &self.operands(ins);
        let pc = usize::from(state.pc);
        match ins.opcode {
            // Random / GetDirectionTo redirect their store to Global 34 after some
            // primitives (`random.rs:38`, `direction.rs:65`).
            8 => {
                let dst_scope = u16::from_le_bytes([ops[2], ops[3]]);
                let rng_scope = u16::from_le_bytes([ops[6], ops[7]]);
                if (rng_scope == 6 && word_i16(ops, 4) == 34) || dst_scope == 22 {
                    return Err(
                        "Random with Global 34 / scope 22 aliasing is not modeled".to_owned()
                    );
                }
            }
            12 => {
                let dst_scope = u16::from_le_bytes([ops[2], ops[3]]);
                let from_g34 = ops[4] & 1 != 0 && ops[5] == 6 && word_i16(ops, 6) == 34;
                if from_g34 || dst_scope == 22 {
                    return Err(
                        "GetDirectionTo with Global 34 / scope 22 aliasing is not modeled"
                            .to_owned(),
                    );
                }
            }
            _ => {}
        }

        let mut inline_args = Vec::new();
        let mut unknown_arity_suffix = None;
        let callee_arity = (ins.opcode >= 256)
            .then(|| {
                self.ctx
                    .callee_arities
                    .as_ref()
                    .and_then(|m| m.get(&ins.opcode).copied())
            })
            .flatten();
        if ins.opcode >= 256 && *ops != [0xFF; 8] {
            match callee_arity {
                Some(arity) if arity <= 4 => {}
                Some(arity) => {
                    inline_args = collect_inline_call_arg_bytes(self.bhav, pc, arity * 2)
                        .ok_or_else(|| {
                            format!(
                                "call #{} at node {pc} needs {arity} inline arguments past the end of tree {}",
                                ins.opcode, self.bhav.id
                            )
                        })?;
                }
                None => {
                    let mut suffix = Vec::new();
                    for next in self.bhav.instructions.iter().skip(pc + 1) {
                        suffix.extend_from_slice(&next.opcode.to_le_bytes());
                        suffix.push(next.on_true);
                        suffix.push(next.on_false);
                        suffix.extend_from_slice(&next.operands);
                    }
                    unknown_arity_suffix = Some(suffix);
                }
            }
        }

        // What the primitive reads from the frame, and which frame slots it may write.
        let mut frame = Vec::new();
        // A call to a tree that overwrites some parameters unseen passes the others by
        // value, from its operand words or the temps (`interpreter.rs:1539`).
        let dead_args = self.ctx.dead_args(self.bhav.id, ins.opcode);
        if let (Some(dead), Some(arity)) = (dead_args, callee_arity) {
            let from_temps = *ops == [0xFF; 8];
            let mut args = Vec::with_capacity(arity);
            for i in 0..arity {
                args.push(if dead.contains(&i) {
                    None
                } else if from_temps {
                    let temp = Loc::Tracked(Cell::Temp(i as i16));
                    Some(
                        self.tracked(&state, temp, interner)
                            .expect("temps are tracked"),
                    )
                } else {
                    Some(Val::Const(word_i16(ops, 2 * i)))
                });
            }
            frame.push(FrameVal::CallArgs(args));
        }
        let mut written: BTreeSet<(u8, i16)> = BTreeSet::new();
        let in_frame = |scope: u8, idx: i16| match scope {
            9 => idx >= 0 && (idx as usize) < self.args,
            25 => idx >= 0 && (idx as usize) < self.locals,
            _ => false,
        };
        let fit = |scope: u8, idx: i16| {
            if in_frame(scope, idx) {
                return Ok(());
            }
            let count = if scope == 9 { self.args } else { self.locals };
            Err(Outside(i32::from(idx) - count as i32))
        };
        let slot_val = |state: &PathState, scope: u8, idx: i16| match scope {
            9 => state.params[idx as usize],
            _ => state.locals[idx as usize],
        };
        let by_value = self.value_read_locals(ins.opcode, ops);
        for frame_use in primitive_frame_uses(ins.opcode, ops) {
            match frame_use {
                FrameUse::Read(scope, idx, _) | FrameUse::Dst(scope @ 22, idx, _) => {
                    let scope = if scope == 22 { 9 } else { scope };
                    if matches!(scope, 9 | 25) {
                        let val = fit(scope, idx).map(|()| slot_val(&state, scope, idx));
                        let unwritten = val.is_ok() && scope == 25 && state.unwritten[idx as usize];
                        let shown = if scope == 25 && by_value.contains(&idx) {
                            BY_VALUE
                        } else {
                            idx
                        };
                        frame.push(FrameVal::Read(scope, shown, val, unwritten));
                    }
                }
                FrameUse::Dst(scope, idx, _) => {
                    if matches!(scope, 9 | 25) {
                        frame.push(FrameVal::Dst(scope, idx, fit(scope, idx)));
                        written.insert((scope, idx));
                    }
                }
                FrameUse::AllLocals(_) => {
                    frame.push(match self.ctx.text_reads(self.bhav.id, ins.opcode, ops) {
                        Some(slots) => FrameVal::TextLocals(
                            slots
                                .into_iter()
                                .map(|idx| {
                                    let val = fit(25, idx).map(|()| slot_val(&state, 25, idx));
                                    let unwritten = val.is_ok() && state.unwritten[idx as usize];
                                    (idx, val, unwritten)
                                })
                                .collect(),
                        ),
                        None => FrameVal::AllLocals(state.locals.clone(), state.unwritten.clone()),
                    });
                }
                FrameUse::WholeFrame => {
                    frame.push(FrameVal::WholeFrame(
                        state.params.clone(),
                        state.locals.clone(),
                        state.unwritten.clone(),
                    ));
                    written.extend((0..self.args).map(|i| (9, i as i16)));
                    written.extend((0..self.locals).map(|i| (25, i as i16)));
                }
            }
        }
        written.extend(primitive_frame_writes(ins.opcode, ops));
        written.retain(|&(scope, idx)| in_frame(scope, idx));

        // A primitive that cannot observe the scratch-pointer state does not record it
        // (`scratch_effect`); one that sets it first leaves its value at a fault too.
        let scratch_effect = if ins.opcode < 256 {
            scratch_effect(ins.opcode, ops)
        } else {
            None
        };
        if let Some(Some(set)) = scratch_effect {
            state.scratch = set;
        }

        let prefix = self.record_with(
            &state,
            EventKind::Primitive {
                // The behavioral relation observes neither the node a yield leaves the
                // frame at nor where in frame storage a callee's frame starts.
                node: if self.opts.behavioral { 0 } else { state.pc },
                opcode: ins.opcode,
                operands: {
                    let mut operands = event_operands(ins.opcode, *ops);
                    if dead_args.is_some() {
                        operands = [0; 8];
                    }
                    // Reached through `Read(25, BY_VALUE, ..)` instead (`value_read_locals`).
                    if !by_value.is_empty() {
                        for (pos, _) in local_operands(ins.opcode, ops) {
                            if by_value.contains(&pos.get(ops)) {
                                let _ = pos.set(&mut operands, 0);
                            }
                        }
                    }
                    operands
                },
                inline_args,
                unknown_arity_suffix,
                callee_arity,
                argless,
                caller_frame: if self.opts.behavioral {
                    (0, 0)
                } else {
                    (self.args, self.locals)
                },
            },
            frame,
            // When faults are assumed not to expose it, a node that sets the last
            // result before anything reads it hides the value it had. Each side
            // records the value unless its own node hides it, so two values are
            // equated only when both sides hide them.
            (self.opts.observe_last_result
                && (self.opts.observe_fault_last_result
                    || !self.ctx.overwrites_last_result(self.bhav.id, ins)))
            .then_some(state.last_result),
            scratch_effect.is_none(),
            interner,
        );
        state.prefix = prefix;
        if let Some(at) = state.at.as_mut() {
            at.event = Some(prefix);
        }
        if self.stop_at_primitive {
            // Only the history up to this event and the condition for reaching it matter.
            return Ok(vec![self.finish(state, Exit::PrimitiveFault, interner)]);
        }

        let mut out = Vec::new();
        // Any primitive or call may fault: Random with a zero range (`random.rs`), a
        // bad operand, a missing object or tree, a trap in a tree it runs. The checker
        // does not model when, so each one gets a faulting path whose condition is a
        // fresh atom of the event: two trees agree on it only for the same event from
        // the same history, and their faults are equal only from the same state at the
        // same node ([`FaultPoint`]).
        let faults = interner.prefix(PrefixNode::Fault(prefix));
        out.push(self.finish_branch(
            &state,
            (Atom::Outcome(faults), true),
            None,
            Exit::PrimitiveFault,
            interner,
        ));
        state.cond.push((Atom::Outcome(faults), false));

        // Random never takes its false branch: it succeeds or faults (`random.rs`).
        let outcomes: Vec<(Option<bool>, u8)> =
            if is_always_true_instruction(ins) || ins.opcode == 8 {
                vec![(None, ins.on_true)]
            } else {
                vec![(Some(true), ins.on_true), (Some(false), ins.on_false)]
            };
        let last = outcomes.len() - 1;
        let mut state = Some(state);
        for (i, (outcome, target)) in outcomes.into_iter().enumerate() {
            // The last outcome takes the state itself; the others take a fork.
            let mut s = match (i == last, &state) {
                (false, Some(state)) => state.fork(),
                _ => state.take().expect("one state per outcome"),
            };
            if let Some(value) = outcome {
                s.prefix = interner.prefix(PrefixNode::Branch(prefix, value));
                s.cond.push((Atom::Outcome(prefix), value));
            }
            for &(scope, idx) in &written {
                // Random Number's only write is its destination, which it always stores
                // its draw in when it takes its one branch (`random.rs`; the global-34
                // redirect is refused above). The draw depends on the world and the
                // range, which the event records, so not on the slot.
                if ins.opcode == 8 {
                    let new = interner.sym(SymKey::Drawn(s.prefix));
                    match scope {
                        9 => s.params[idx as usize] = new,
                        _ => {
                            s.locals[idx as usize] = new;
                            s.unwritten[idx as usize] = false;
                        }
                    }
                    continue;
                }
                let old = slot_val(&s, scope, idx);
                let new = interner.sym(SymKey::FrameMaybe(s.prefix, scope, idx, old));
                match scope {
                    9 => s.params[idx as usize] = new,
                    _ => s.locals[idx as usize] = new,
                }
            }
            let scratch = s.scratch;
            Self::havoc_shared(&mut s);
            if scratch_effect.is_some() {
                s.scratch = scratch;
            }
            s.last_result = Some(outcome.unwrap_or(true));
            s.pc = target;
            out.push(Step::Continue(s));
        }
        Ok(out)
    }
}

/// The operand bytes a primitive event compares: [`canonicalize_primitive_operands`],
/// and also Animate's event local (byte 2), which the VM reads only with flag 0x20
/// (`animate.rs:271`).
///
/// This is narrower than the canonical operands on purpose. `pad` clauses and lifting use
/// those, and a call before the node may read byte 2 as an inline argument. The call's
/// event records that read itself (`inline_args`).
fn event_operands(opcode: u16, ops: [u8; 8]) -> [u8; 8] {
    let mut out = canonicalize_primitive_operands(opcode, ops);
    if opcode == 44 && ops[5] & 0x20 == 0 {
        out[2] = 0;
    }
    out
}

/// Frame slots primitives write beyond their listed destinations (mirrors the lockstep
/// checker's per-opcode frame updates).
pub(super) fn primitive_frame_writes(opcode: u16, ops: &[u8; 8]) -> Vec<(u8, i16)> {
    match opcode {
        // Sleep / IdleForInput write their parameter back (`sleep.rs`, `idle.rs`).
        0 | 17 => vec![(9, word_i16(ops, 0))],
        8 | 12 => {
            let dst_scope = u16::from_le_bytes([ops[2], ops[3]]);
            match u8::try_from(dst_scope) {
                Ok(scope @ (9 | 25)) => vec![(scope, word_i16(ops, 0))],
                _ => Vec::new(),
            }
        }
        24 => vec![(9, i16::from(ops[2] as i8))],
        26 => vec![(ops[4], word_i16(ops, 6))],
        31 if ops[4] & 0x80 != 0 => vec![(ops[5], i16::from(ops[7]))],
        // Animate stores its event in `local[ops[2]]` (flag 0x20) or `param[0]`
        // (`animate.rs:269`).
        44 if ops[5] & 0x20 != 0 => vec![(25, i16::from(ops[2]))],
        44 => vec![(9, 0)],
        _ => Vec::new(),
    }
}

/// `lhs op rhs` for an assignment operator, folded when both are literals
/// (`expression.rs:181`).
fn mk_op(op: u8, lhs: Val, rhs: Val, interner: &mut Interner) -> Val {
    if op == 5 {
        return rhs;
    }
    if let (Val::Const(l), Val::Const(r)) = (lhs, rhs) {
        let (l32, r32) = (i32::from(l), i32::from(r));
        let folded = match op {
            3 => Some(l32 + r32),
            4 => Some(l32 - r32),
            6 => Some(l32.wrapping_mul(r32)),
            7 if r32 != 0 => Some(l32 / r32),
            9 if (0..=16).contains(&r32) => Some(l32 | flag_bit(r32)),
            10 if (0..=16).contains(&r32) => Some(l32 & !flag_bit(r32)),
            12 if r32 != 0 => Some(i32::from(l.wrapping_abs()) % i32::from(r.wrapping_abs())),
            13 => Some(l32 & r32),
            18 => Some(l32 | r32),
            19 => Some(l32 ^ r32),
            20 if r32 >= 0 => Some((r32 as u32).isqrt() as i32),
            _ => None,
        };
        if let Some(v) = folded {
            return Val::Const(v as i16);
        }
    }
    // Identities that hold for every 16-bit value.
    match (op, lhs, rhs) {
        (3 | 4 | 18 | 19, x, Val::Const(0)) => return x,
        (3 | 18 | 19, Val::Const(0), x) => return x,
        (6, x, Val::Const(1)) | (6, Val::Const(1), x) => return x,
        (13 | 18, x, y) if x == y => return x,
        (4 | 19, x, y) if x == y => return Val::Const(0),
        _ => {}
    }
    let (a, b) = if matches!(op, 3 | 6 | 13 | 18 | 19) && rhs < lhs {
        (rhs, lhs)
    } else {
        (lhs, rhs)
    };
    interner.sym(SymKey::Op(op, a, b))
}

// ============================================================================
// Cut-point bisimulation
// ============================================================================

/// Most product states one cut-point bisimulation explores before giving up.
const MAX_PRODUCT_STATES: usize = 4096;

/// Visits to a pair of nodes after which literals that changed between visits are
/// widened into classes, so a counter that never bounds a loop does not unroll it.
const WIDEN_AFTER: usize = 4;

/// Where one side's segment ends ([`Explorer::segment`]).
enum SegmentEnd {
    Done(PathSummary),
    /// Right after a cut node, ready to start the next segment.
    Cut(PathState),
}

impl SegmentEnd {
    fn cond(&self) -> &[(Atom, bool)] {
        match self {
            Self::Done(p) => &p.cond,
            Self::Cut(s) => &s.cond,
        }
    }
}

/// A frame slot or tracked cell of a product state: a literal, or an equivalence class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Abs {
    Const(i16),
    Class(u16),
}

/// Both runs' state at a cut, up to which of their values are equal. Shared memory needs
/// nothing more: both runs have the same history, so it is the same on both sides.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ProductState {
    pc: (u8, u8),
    /// The tracked cells either side may hold a value in that differs from the epoch's.
    cells: Vec<Cell>,
    /// A's parameters, locals and `cells`, then B's.
    slots: Vec<Abs>,
    unwritten: (Vec<bool>, Vec<bool>),
    scratch: (Scratch, Scratch),
    last_result: (Option<bool>, Option<bool>),
}

/// Per pair of nodes: visits so far, and each slot's literal if every visit had the same.
type WidenTable = BTreeMap<((u8, u8), Vec<Cell>), (usize, Vec<Option<i16>>)>;

/// Where [`bisimulate`] cuts the runs.
#[derive(Clone, Copy)]
enum Cuts<'h> {
    /// After each primitive or call.
    Primitives,
    /// After any node that recorded an event.
    EveryEvent,
    /// After each primitive or call, and on reaching a loop head of this side's tree
    /// (`.0` for A, `.1` for B; [`Explorer::loop_heads`]), so a loop that records no
    /// event is cut too.
    LoopHeads(&'h BTreeSet<u8>, &'h BTreeSet<u8>),
}

impl Explorer<'_> {
    /// The targets of the back edges of a depth-first walk from the entry, following
    /// the true branch first: every cycle the walk reaches passes through one. The
    /// walk follows the graph, not the node numbers, so a tree laid out differently
    /// has the same loop heads.
    fn loop_heads(&self) -> BTreeSet<u8> {
        let len = self.bhav.instructions.len().min(LAST_NODE + 1);
        let next = |n: usize| -> Vec<usize> {
            let ins = &self.bhav.instructions[n];
            let mut out = Vec::new();
            for t in [ins.on_true, ins.on_false] {
                let t = usize::from(t);
                if t < len && !out.contains(&t) {
                    out.push(t);
                }
            }
            out
        };
        let mut heads = BTreeSet::new();
        if self.entry >= len {
            return heads;
        }
        // 0 unvisited, 1 on the walk's stack, 2 finished.
        let mut color = vec![0u8; len];
        let mut stack: Vec<(usize, Vec<usize>)> = vec![(self.entry, next(self.entry))];
        color[self.entry] = 1;
        while let Some((node, succ)) = stack.last_mut() {
            if succ.is_empty() {
                color[*node] = 2;
                stack.pop();
                continue;
            }
            let t = succ.remove(0);
            match color[t] {
                0 => {
                    color[t] = 1;
                    stack.push((t, next(t)));
                }
                1 => {
                    heads.insert(t as u8);
                }
                _ => {}
            }
        }
        heads
    }

    /// Runs `start` until each path has run a cut node (see [`Cuts`]) or has ended.
    fn segment(
        &self,
        start: PathState,
        cuts: Cuts,
        side_b: bool,
        interner: &mut Interner,
    ) -> Result<Vec<SegmentEnd>, String> {
        let every_event = matches!(cuts, Cuts::EveryEvent);
        let heads = match cuts {
            Cuts::LoopHeads(a, b) => Some(if side_b { b } else { a }),
            _ => None,
        };
        let mut ends = Vec::new();
        let mut stack = vec![start];
        while let Some(state) = stack.pop() {
            if ends.len() + stack.len() > self.opts.max_paths {
                return Err(format!(
                    "tree {} has more than {} paths between two cuts",
                    self.bhav.id, self.opts.max_paths
                ));
            }
            let primitive = usize::from(state.pc) <= LAST_NODE
                && self
                    .bhav
                    .instructions
                    .get(usize::from(state.pc))
                    .is_some_and(|ins| ins.opcode != 2);
            let before = state.prefix;
            for next in self.step(state, interner)? {
                // A branch to an exit or a missing node finishes in this segment, where
                // strict mode still knows the node it faults at (`PathState::at`).
                let to_node = |s: &PathState| {
                    usize::from(s.pc) <= LAST_NODE
                        && usize::from(s.pc) < self.bhav.instructions.len()
                };
                match next {
                    Step::Done(summary) => ends.push(SegmentEnd::Done(summary)),
                    Step::Continue(s)
                        if s.prefix != before && (primitive || every_event) && to_node(&s) =>
                    {
                        ends.push(SegmentEnd::Cut(s));
                    }
                    Step::Continue(s)
                        if heads.is_some_and(|h| h.contains(&s.pc)) && to_node(&s) =>
                    {
                        ends.push(SegmentEnd::Cut(s));
                    }
                    Step::Continue(s) => stack.push(s),
                }
            }
        }
        Ok(ends)
    }

    /// This side's state at the start of product state `ps`'s segment: values of class
    /// `k` become the symbol `CutClass(root, k)`, and shared memory is named by `root`.
    fn instantiate(
        &self,
        ps: &ProductState,
        side_b: bool,
        root: u32,
        interner: &mut Interner,
    ) -> PathState {
        let width = |x: &Explorer| x.args + x.locals + ps.cells.len();
        let start = if side_b {
            ps.slots.len() - width(self)
        } else {
            0
        };
        let mut vals = ps.slots[start..start + width(self)]
            .iter()
            .map(|&v| match v {
                Abs::Const(c) => Val::Const(c),
                Abs::Class(k) => interner.sym(SymKey::CutClass(root, k)),
            });
        let params = vals.by_ref().take(self.args).collect();
        let locals = vals.by_ref().take(self.locals).collect();
        let store = ps.cells.iter().copied().zip(vals).collect();
        PathState {
            pc: if side_b { ps.pc.1 } else { ps.pc.0 },
            prefix: root,
            epoch: root,
            params,
            locals,
            store,
            scratch: if side_b { ps.scratch.1 } else { ps.scratch.0 },
            last_result: if side_b {
                ps.last_result.1
            } else {
                ps.last_result.0
            },
            unwritten: if side_b {
                ps.unwritten.1.clone()
            } else {
                ps.unwritten.0.clone()
            },
            cond: Vec::new(),
            steps: 0,
            nodes: Vec::new(),
            at: None,
        }
    }
}

/// The product state covering the pair of states `sa` and `sb` reach after the same
/// history. Each value becomes a literal or a class, with two values in the same class
/// only when they are the same symbol (or, once widened, the same literal). The path
/// condition is dropped. So the product state stands for every pair of states with those
/// literals and equalities, which includes this one.
fn abstraction(
    (xa, sa): (&Explorer, &PathState),
    (xb, sb): (&Explorer, &PathState),
    widen: &mut WidenTable,
    interner: &mut Interner,
) -> ProductState {
    let (store_a, store_b) = (
        xa.normalized_store(sa, interner),
        xb.normalized_store(sb, interner),
    );
    let cells: Vec<Cell> = store_a
        .iter()
        .chain(&store_b)
        .map(|&(c, _)| c)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut raw: Vec<Val> = Vec::new();
    for (state, store) in [(sa, &store_a), (sb, &store_b)] {
        raw.extend(&state.params);
        raw.extend(&state.locals);
        for &cell in &cells {
            let v = match store.iter().find(|&&(c, _)| c == cell) {
                Some(&(_, v)) => v,
                None => interner.sym(SymKey::Cell(state.epoch, cell)),
            };
            raw.push(v);
        }
    }
    let pc = (sa.pc, sb.pc);
    let literals: Vec<Option<i16>> = raw
        .iter()
        .map(|v| match *v {
            Val::Const(c) => Some(c),
            Val::Sym(_) => None,
        })
        .collect();
    let (visits, stable) = widen
        .entry((pc, cells.clone()))
        .or_insert_with(|| (0, literals.clone()));
    *visits += 1;
    for (kept, now) in stable.iter_mut().zip(&literals) {
        if kept != now {
            *kept = None;
        }
    }
    let widened = *visits > WIDEN_AFTER;
    let mut classes: BTreeMap<Val, u16> = BTreeMap::new();
    let slots = raw
        .iter()
        .zip(stable.iter())
        .map(|(&v, kept)| match v {
            Val::Const(c) if !widened || kept.is_some() => Abs::Const(c),
            v => {
                let next = classes.len() as u16;
                Abs::Class(*classes.entry(v).or_insert(next))
            }
        })
        .collect();
    ProductState {
        pc,
        cells,
        slots,
        unwritten: (sa.unwritten.clone(), sb.unwritten.clone()),
        scratch: (sa.scratch, sb.scratch),
        last_result: (sa.last_result, sb.last_result),
    }
}

/// Where two histories first differ.
fn divergence(a: u32, b: u32, interner: &Interner) -> String {
    let (ha, hb) = (interner.history(a), interner.history(b));
    let k = ha.iter().zip(&hb).take_while(|(x, y)| x == y).count();
    let show = |n: Option<&&PrefixNode>| n.map_or("nothing".to_owned(), |n| describe_node(n));
    format!(
        "event {k} after the cut: A {} vs B {}",
        show(ha.get(k)),
        show(hb.get(k))
    )
}

/// Proves two runs equivalent by cutting both at primitive events
/// (`docs/decisions/20261123-verifier-behavioral-mode.md`, Decision 3).
///
/// From each product state, both sides run one segment. Every pair of segment ends
/// whose path conditions can hold together must either end with the same outputs or run
/// the same events to the same outcome, after which the pair of states is abstracted
/// into a product state of its own. Product states are explored once, which closes
/// loops and merges diamonds. Induction on the number of cuts: two runs from the same
/// inputs reach every cut with the same history and frames that the product state
/// covers, so they end the same.
///
/// Returns the segments and product states explored, or why the proof failed. A
/// failure is never a counterexample: the abstraction drops path conditions.
fn bisimulate(
    xa: &Explorer,
    xb: &Explorer,
    cuts: Cuts,
    interner: &mut Interner,
) -> Result<(usize, usize), String> {
    let opts = xa.opts;
    let mut seen: BTreeSet<ProductState> = BTreeSet::new();
    let mut widen = WidenTable::new();
    let mut work = vec![(xa.initial(interner), xb.initial(interner))];
    let mut segments = 0usize;
    let mut checks = 0usize;
    while let Some((sa, sb)) = work.pop() {
        segments += 1;
        let ends_a = xa
            .segment(sa, cuts, false, interner)
            .map_err(|e| format!("A: {e}"))?;
        let ends_b = xb
            .segment(sb, cuts, true, interner)
            .map_err(|e| format!("B: {e}"))?;
        let index = ConflictIndex::new(ends_b.iter().map(SegmentEnd::cond));
        for ea in &ends_a {
            let conflicts = index.conflicts(ea.cond());
            for (j, eb) in ends_b.iter().enumerate() {
                if let (SegmentEnd::Done(pa), SegmentEnd::Done(pb)) = (ea, eb)
                    && pa.out == pb.out
                {
                    continue;
                }
                // Infeasible together (see `compare_paths`).
                if ConflictIndex::has(&conflicts, j) {
                    continue;
                }
                checks += 1;
                if checks > opts.max_pair_checks {
                    return Err(format!(
                        "more than {} pairs of segment ends to check",
                        opts.max_pair_checks
                    ));
                }
                if feasibility(&[ea.cond(), eb.cond()], interner) == Feasibility::Unsat {
                    continue;
                }
                match (ea, eb) {
                    (SegmentEnd::Done(pa), SegmentEnd::Done(pb)) => {
                        let mut cond = pa.cond.clone();
                        cond.extend_from_slice(&pb.cond);
                        let what = match first_difference(&pa.out, &pb.out, &cond, interner) {
                            Difference::Definite(w) | Difference::Possible(w) => w,
                        };
                        return Err(format!(
                            "segment ends may differ: {what} (A path {}, B path {})",
                            format_nodes(pa),
                            format_nodes(pb)
                        ));
                    }
                    (SegmentEnd::Cut(sa), SegmentEnd::Cut(sb)) if sa.prefix == sb.prefix => {
                        let ps = abstraction((xa, sa), (xb, sb), &mut widen, interner);
                        if !seen.contains(&ps) {
                            if seen.len() >= MAX_PRODUCT_STATES {
                                return Err(format!(
                                    "more than {MAX_PRODUCT_STATES} product states"
                                ));
                            }
                            let root = interner.fresh_cut();
                            work.push((
                                xa.instantiate(&ps, false, root, interner),
                                xb.instantiate(&ps, true, root, interner),
                            ));
                            seen.insert(ps);
                        }
                    }
                    (SegmentEnd::Cut(sa), SegmentEnd::Cut(sb)) => {
                        return Err(format!(
                            "segments may run different events: {}",
                            divergence(sa.prefix, sb.prefix, interner)
                        ));
                    }
                    (SegmentEnd::Done(p), SegmentEnd::Cut(s)) => {
                        return Err(format!(
                            "A may end ({}, path {}) where B runs on: {}",
                            p.out.exit,
                            format_nodes(p),
                            divergence(p.out.prefix, s.prefix, interner)
                        ));
                    }
                    (SegmentEnd::Cut(s), SegmentEnd::Done(p)) => {
                        return Err(format!(
                            "B may end ({}, path {}) where A runs on: {}",
                            p.out.exit,
                            format_nodes(p),
                            divergence(s.prefix, p.out.prefix, interner)
                        ));
                    }
                }
            }
        }
    }
    Ok((segments, seen.len()))
}

// ============================================================================
// Comparison
// ============================================================================

/// Checks whether two `BHAV` trees have the same observable behavior by comparing the
/// summaries of all their paths (see the module docs).
///
/// `ctx_a` / `ctx_b` describe where each tree runs (its file's callee arities and
/// tuning constants). Returns [`EquivResult::Inconclusive`] when a tree exceeds the
/// bounds in `opts`, uses an unmodeled construct (Global 34), or the trees differ only
/// in ways the checker cannot decide.
#[must_use]
pub fn check_bhav_live_equivalent(
    a: &BhavChunk,
    ctx_a: &LiveContext,
    b: &BhavChunk,
    ctx_b: &LiveContext,
    opts: &LiveOptions,
) -> EquivResult {
    let result = check_as_written(a, ctx_a, b, ctx_b, opts);
    // The behavioral relation does not observe frame storage, so locals that hold the
    // same values under different slot numbers may be renamed first (`super::rename`).
    // Only a proof counts: a renaming that does not prove the trees equivalent says
    // nothing about them.
    if !opts.behavioral || !matches!(result, EquivResult::Inconclusive { .. }) {
        return result;
    }
    for (renamed_a, renamed_b) in super::rename::candidates(a, ctx_a, b, ctx_b) {
        let renamed = check_as_written(&renamed_a, ctx_a, &renamed_b, ctx_b, opts);
        if matches!(renamed, EquivResult::Equivalent { .. }) {
            return renamed;
        }
    }
    result
}

/// [`check_bhav_live_equivalent`] on the trees as they are, without renaming locals.
fn check_as_written(
    a: &BhavChunk,
    ctx_a: &LiveContext,
    b: &BhavChunk,
    ctx_b: &LiveContext,
    opts: &LiveOptions,
) -> EquivResult {
    if let Some(reason) = super::context_mismatch(a, ctx_a, b, ctx_b) {
        return EquivResult::Inconclusive { reason };
    }
    // A fault names its tree (`fault.rs`, `Location`), and every tree can fault on
    // entry, before any node runs, on inputs both runs share: more than 255 arguments,
    // a negative kind-4 entry, a full stack (`interpreter.rs:931`). So two trees with
    // different IDs never fault alike.
    if a.id != b.id {
        return EquivResult::Inconclusive {
            reason: format!(
                "the trees have different IDs (#{} vs #{}), and a fault names its tree; any \
                 tree can fault on entry",
                a.id, b.id
            ),
        };
    }
    // Only a kind-4 tree is entered at a node other than 0 (`interpreter.rs:1792`), but
    // whether other kinds differ anywhere else is not modeled.
    if a.kind != b.kind {
        return EquivResult::Inconclusive {
            reason: format!(
                "the trees have different kinds ({} vs {}), which the checker does not model",
                a.kind, b.kind
            ),
        };
    }
    // How the VM enters a tree depends on its frame's shape, whatever the relation:
    // - a caller supplies at most so many arguments, and entry faults when the tree
    //   wants more (`factory.rs:494`);
    // - Create New Object Instance (flag 0x02) writes into the new object's bottom frame:
    //   its first argument, else its first local, and fails when the frame has neither
    //   (`create.rs:537`). With an argument it never looks at the locals, so whether
    //   there is a local matters only to a tree that takes no argument.
    // The frame-residue and fault checks observe both in strict mode only when the frame
    // is compared, so check them up front, on the counts the VM uses (the format caps
    // them and the local count is a byte), not the header's.
    let ((args_a, locals_a), (args_b, locals_b)) = (frame_dims(a), frame_dims(b));
    if args_a != args_b || (args_a == 0 && (locals_a == 0) != (locals_b == 0)) {
        return EquivResult::Inconclusive {
            reason: format!(
                "the trees' frames differ in shape ({args_a} arguments and {locals_a} locals \
                 vs {args_b} and {locals_b}): a caller or Create New Object Instance may \
                 write or supply a frame word only one of them has"
            ),
        };
    }
    // The VM enters only formats 0x8000 to 0x8002 (`interpreter.rs:1768`).
    if let Some(format) = [a.format, b.format]
        .into_iter()
        .find(|f| !(0x8000..=0x8002).contains(f))
    {
        return EquivResult::Inconclusive {
            reason: format!("BHAV format {format:#06x} faults when entered"),
        };
    }
    // An empty tree faults on entry (`interpreter.rs:1786`).
    if a.instructions.is_empty() || b.instructions.is_empty() {
        return EquivResult::Inconclusive {
            reason: format!(
                "an empty tree faults when entered (A has {} nodes, B has {})",
                a.instructions.len(),
                b.instructions.len()
            ),
        };
    }
    // Kind-4 trees can be entered at any node; the VM clamps the entry to the last
    // node (`interpreter.rs:1790`), and a node past 250 fails (`interpreter.rs:766`).
    // Entries from the longer tree's length on all map to the same pair of nodes.
    let (len_a, len_b) = (a.instructions.len(), b.instructions.len());
    let entries = if a.kind == 4 { len_a.max(len_b) } else { 1 };

    // Assumed parameter domains (never for kind 4, whose first parameter is the entry):
    // the entry is split into regions that cover the domain exactly, each explored from
    // a path condition bounding the parameters to it. A replay witness outside the
    // domain is not a run the assumption covers, so it is never replayed.
    let domains = (a.kind != 4)
        .then(|| ctx_a.param_domains.get(&a.id))
        .flatten();
    let regions = super::domains::entry_regions(domains, frame_dims(a).0);
    let in_domain = |w: &Witness| {
        domains.is_none_or(|d| {
            d.iter().all(|(&i, domain)| {
                w.params
                    .get(usize::from(i))
                    .is_none_or(|&v| domain.contains(v))
            })
        })
    };

    let mut interner = Interner::new();
    let mut steps = 0usize;
    let mut paths = 0usize;
    let mut undecided: Option<String> = None;
    let mut replays = 0usize;
    for entry in 0..entries {
        let (entry_a, entry_b) = (entry.min(len_a - 1), entry.min(len_b - 1));
        if entry_a > LAST_NODE || entry_b > LAST_NODE {
            // The frame is pushed at the clamped entry node, which then faults with
            // 0x44c (`interpreter.rs:766`), leaving the frame at that node. Two such
            // faults are the same only at the same node with the same frame.
            if a.id == b.id && entry_a == entry_b && frame_dims(a) == frame_dims(b) {
                continue;
            }
            return EquivResult::Inconclusive {
                reason: format!(
                    "entry {entry} faults past node {LAST_NODE} on {} (entered at node {entry_a} on A, \
                     {entry_b} on B)",
                    if entry_a > LAST_NODE && entry_b > LAST_NODE {
                        "both sides, at different nodes or with different frames"
                    } else {
                        "one side only"
                    }
                ),
            };
        }
        // Entries below the last pair are passed exactly; the last pair stands for every
        // larger entry too. A negative entry fails alike on both sides.
        let entry_param = (a.kind == 4).then(|| {
            (
                i16::try_from(entry).unwrap_or(i16::MAX),
                entry + 1 < entries,
            )
        });
        for region in &regions {
            let explore =
                |bhav: &BhavChunk, ctx: &LiveContext, entry: usize, interner: &mut Interner| {
                    Explorer::new(bhav, ctx, opts, entry, entry_param, region).explore(interner)
                };
            // Where the comparison cannot decide, a wrong operand on the first primitive
            // both trees run is still a counterexample (`decisive_difference`).
            let first_primitive = |interner: &mut Interner| {
                // Only for a tree that runs as itself with a witness that says how it is
                // entered. A frame's tree word with bit 15 set selects the dispatch hook
                // and is looked up without it (`saved_objects.rs:29`), so tree 0x9000
                // runs 0x1000's nodes. A kind-4 tree's entry picks its first node, which
                // the witness's parameters do not pin down.
                if a.id & sims_formats::saved_objects::Frame::TREE_FLAG != 0
                    || entry_param.is_some()
                {
                    return None;
                }
                let mut xa = Explorer::new(a, ctx_a, opts, entry_a, entry_param, region);
                let mut xb = Explorer::new(b, ctx_b, opts, entry_b, entry_param, region);
                xa.stop_at_primitive = true;
                xb.stop_at_primitive = true;
                decisive_difference(
                    &xa,
                    &xb,
                    interner,
                    entry_param,
                    frame_dims(a).0.max(frame_dims(b).0),
                    &in_domain,
                )
            };
            let explored = explore(a, ctx_a, entry_a, &mut interner)
                .map_err(|e| format!("A: {e}"))
                .and_then(|pa| {
                    explore(b, ctx_b, entry_b, &mut interner)
                        .map(|pb| (pa, pb))
                        .map_err(|e| format!("B: {e}"))
                });
            let ((paths_a, steps_a), (paths_b, steps_b)) = match explored {
                Ok(r) => r,
                Err(reason) => {
                    // Too many paths, or a loop the literals do not bound: cut both runs at
                    // their events instead (`bisimulate`).
                    let (xa, xb) = (
                        Explorer::new(a, ctx_a, opts, entry_a, entry_param, region),
                        Explorer::new(b, ctx_b, opts, entry_b, entry_param, region),
                    );
                    let proof = bisimulate(&xa, &xb, Cuts::Primitives, &mut interner)
                        .or_else(|first| {
                            bisimulate(&xa, &xb, Cuts::EveryEvent, &mut interner)
                                .map_err(|every| (first, every))
                        })
                        .or_else(|(first, every)| {
                            let (heads_a, heads_b) = (xa.loop_heads(), xb.loop_heads());
                            bisimulate(&xa, &xb, Cuts::LoopHeads(&heads_a, &heads_b), &mut interner)
                                .map_err(|heads| {
                                    format!(
                                        "cut at primitives: {first}; cut at every event: {every}; \
                                     cut at loop heads: {heads}"
                                    )
                                })
                        });
                    match proof {
                        Ok((segments, states)) => {
                            steps += segments;
                            paths += states;
                            continue;
                        }
                        Err(why) => {
                            if let Some((path, divergence)) = first_primitive(&mut interner) {
                                return EquivResult::Counterexample { path, divergence };
                            }
                            return EquivResult::Inconclusive {
                                reason: format!("{reason}; cut-point bisimulation failed: {why}"),
                            };
                        }
                    }
                }
            };
            steps += steps_a + steps_b;
            paths += paths_a.len() + paths_b.len();
            // A difference counts only if both trees, run in `sims-vm` from the same world
            // built from a witness, end differently.
            let mut confirm = |w: &Witness| -> Option<String> {
                if replays >= MAX_REPLAYS
                    || !in_domain(w)
                    || !replay::replayable(a, entry_a)
                    || !replay::replayable(b, entry_b)
                {
                    return None;
                }
                replays += 1;
                replay::replay(a, ctx_a, b, ctx_b, opts, w)
            };
            match compare_paths(
                &paths_a,
                &paths_b,
                &interner,
                opts,
                entry_param,
                frame_dims(a).0.max(frame_dims(b).0),
                &mut confirm,
            ) {
                PairVerdict::Same => {}
                PairVerdict::Differ(path, divergence) => {
                    return EquivResult::Counterexample { path, divergence };
                }
                PairVerdict::Undecided(reason) => {
                    if let Some((path, divergence)) = first_primitive(&mut interner) {
                        return EquivResult::Counterexample { path, divergence };
                    }
                    undecided.get_or_insert(reason);
                }
            }
        }
    }
    match undecided {
        Some(reason) => EquivResult::Inconclusive { reason },
        None => EquivResult::Equivalent {
            states_checked: steps,
            paths_verified: paths,
        },
    }
}

/// For a list of path conditions, which of them take a branch the other way: a
/// condition that holds `(atom, v)` conflicts with every one holding `(atom, !v)`.
struct ConflictIndex {
    /// `(atom, value)` -> bitset of the conditions holding `(atom, !value)`.
    by_literal: BTreeMap<(Atom, bool), Vec<u64>>,
    words: usize,
}

impl ConflictIndex {
    fn new<'c>(conds: impl ExactSizeIterator<Item = &'c [(Atom, bool)]>) -> Self {
        let words = conds.len().div_ceil(64);
        let mut by_literal: BTreeMap<(Atom, bool), Vec<u64>> = BTreeMap::new();
        for (j, cond) in conds.enumerate() {
            for &(atom, value) in cond {
                by_literal
                    .entry((atom, !value))
                    .or_insert_with(|| vec![0; words])[j / 64] |= 1 << (j % 64);
            }
        }
        Self { by_literal, words }
    }

    /// The conditions that conflict with `cond`, as a bitset.
    fn conflicts(&self, cond: &[(Atom, bool)]) -> Vec<u64> {
        let mut out = vec![0u64; self.words];
        for literal in cond {
            if let Some(bits) = self.by_literal.get(literal) {
                for (o, b) in out.iter_mut().zip(bits) {
                    *o |= b;
                }
            }
        }
        out
    }

    fn has(bits: &[u64], j: usize) -> bool {
        bits[j / 64] & (1 << (j % 64)) != 0
    }
}

enum PairVerdict {
    Same,
    Differ(Vec<TraceStep>, String),
    Undecided(String),
}

/// Compares every pair of paths with different summaries. A pair that may differ is
/// undecided unless `confirm` (a concrete VM replay) shows it does for a witness built
/// from the pair's path condition.
fn compare_paths(
    paths_a: &[PathSummary],
    paths_b: &[PathSummary],
    interner: &Interner,
    opts: &LiveOptions,
    entry_param: Option<(i16, bool)>,
    args: usize,
    confirm: &mut dyn FnMut(&Witness) -> Option<String>,
) -> PairVerdict {
    fn group(paths: &[PathSummary]) -> BTreeMap<&Outputs, Vec<usize>> {
        let mut groups: BTreeMap<&Outputs, Vec<usize>> = BTreeMap::new();
        for (i, p) in paths.iter().enumerate() {
            groups.entry(&p.out).or_default().push(i);
        }
        groups
    }
    let groups_a = group(paths_a);
    let groups_b = group(paths_b);
    // Pairs that take one branch both ways are infeasible (the first test
    // `feasibility` makes), and need neither a check nor budget.
    let index = ConflictIndex::new(paths_b.iter().map(|p| p.cond.as_slice()));
    let conflicts: Vec<Vec<u64>> = paths_a.iter().map(|p| index.conflicts(&p.cond)).collect();
    let mut checks = 0usize;
    let mut undecided: Option<String> = None;
    for (out_a, idx_a) in &groups_a {
        for (out_b, idx_b) in &groups_b {
            if out_a == out_b {
                continue;
            }
            for &i in idx_a {
                for &j in idx_b {
                    if ConflictIndex::has(&conflicts[i], j) {
                        continue;
                    }
                    checks += 1;
                    if checks > opts.max_pair_checks {
                        return PairVerdict::Undecided(format!(
                            "more than {} pairs of paths with different outputs to check",
                            opts.max_pair_checks
                        ));
                    }
                    let (pa, pb) = (&paths_a[i], &paths_b[j]);
                    let exact = match feasibility(&[&pa.cond, &pb.cond], interner) {
                        Feasibility::Unsat => continue,
                        Feasibility::Sat { exact } => exact,
                    };
                    let mut cond = pa.cond.clone();
                    cond.extend_from_slice(&pb.cond);
                    match first_difference(&pa.out, &pb.out, &cond, interner) {
                        Difference::Definite(what) if exact => {
                            let shown = witnesses(&cond, interner, entry_param, args)
                                .iter()
                                .find_map(&mut *confirm);
                            if let Some(vm) = shown {
                                return PairVerdict::Differ(
                                    trace_steps(pa, pb),
                                    format!(
                                        "{what} (A path {}, B path {}{}); {vm}",
                                        format_nodes(pa),
                                        format_nodes(pb),
                                        format_cond(&cond, interner)
                                    ),
                                );
                            }
                            undecided.get_or_insert_with(|| {
                                format!(
                                    "paths may differ, but no sims-vm replay confirmed it: {what} (A path {}, B path {}{})",
                                    format_nodes(pa),
                                    format_nodes(pb),
                                    format_cond(&cond, interner)
                                )
                            });
                        }
                        Difference::Definite(what) | Difference::Possible(what) => {
                            undecided.get_or_insert_with(|| {
                                format!(
                                    "paths may differ but the checker cannot decide it: {what} (A path {}, B path {}{})",
                                    format_nodes(pa),
                                    format_nodes(pb),
                                    format_cond(&cond, interner)
                                )
                            });
                        }
                    }
                }
            }
        }
    }
    match undecided {
        Some(reason) => PairVerdict::Undecided(reason),
        None => PairVerdict::Same,
    }
}

/// Concrete inputs for a pair of paths: each input the path condition bounds gets a
/// value inside its bounds, and each other input a default. Two variants: unbounded
/// inputs all zero, and unbounded inputs pairwise distinct (so outputs that copy
/// different inputs differ). The replay decides whether a witness shows anything, so
/// these need not satisfy every constraint.
fn witnesses(
    cond: &[(Atom, bool)],
    interner: &Interner,
    entry_param: Option<(i16, bool)>,
    args: usize,
) -> Vec<Witness> {
    let mut out: Vec<Witness> = Vec::new();
    // Inputs near 0, then pairwise distinct, then at the bottom and at the top of what
    // the condition allows, then alternating bottom and top from one input to the next:
    // a difference that shows only at one end of a range (a parameter domain's bound,
    // say), or only when two inputs sit at different ends, still gets a witness.
    for seed in 0..6 {
        let mut nth_input = 0usize;
        // A kind-4 entry is passed as the first argument even to a tree that takes none.
        let mut w = Witness {
            params: vec![0; args.max(usize::from(entry_param.is_some()))],
            ..Witness::default()
        };
        let mut feasible = true;
        for (id, key) in interner.sym_keys.iter().enumerate() {
            let id = id as u32;
            if !interner.is_input(id) {
                continue;
            }
            let range = range_of(cond, id);
            let clamp = |v: i32| v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            let low_end = match seed {
                4 => nth_input.is_multiple_of(2),
                _ => !nth_input.is_multiple_of(2),
            };
            nth_input += 1;
            let base = match seed {
                0 => 0,
                1 => i16::try_from(3 * (id + 1)).unwrap_or(i16::MAX),
                2 => clamp(range.lo),
                3 => clamp(range.hi),
                _ if low_end => clamp(range.lo),
                _ => clamp(range.hi),
            };
            let Some(v) = range.pick_near(base) else {
                feasible = false;
                break;
            };
            match key {
                SymKey::InitParam(i) => {
                    if let Some(slot) = w.params.get_mut(usize::from(*i)) {
                        *slot = v;
                    }
                }
                SymKey::InitLocal(word) => {
                    w.frame_words.insert(*word, v);
                }
                SymKey::Cell(_, Cell::Temp(i)) => {
                    w.temps.insert(*i, v);
                }
                SymKey::Cell(_, Cell::Attr(i)) => {
                    w.attrs.insert(*i, v);
                }
                _ => {}
            }
        }
        // An exact kind-4 entry is a literal first parameter, not an input.
        if let (Some((raw, true)), Some(first)) = (entry_param, w.params.first_mut()) {
            *first = raw;
        }
        if feasible && !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

/// The value `v` takes under witness `w`, when `v` is a literal or an input.
fn concrete(v: Val, w: &Witness, interner: &Interner) -> Option<i16> {
    match v {
        Val::Const(c) => Some(c),
        Val::Sym(id) => match interner.key(id) {
            SymKey::InitParam(i) => w.params.get(usize::from(*i)).copied(),
            SymKey::InitLocal(word) => Some(w.frame_words.get(word).copied().unwrap_or(0)),
            SymKey::Cell(0, Cell::Temp(i)) => Some(w.temps.get(i).copied().unwrap_or(0)),
            SymKey::Cell(0, Cell::Attr(i)) => Some(w.attrs.get(i).copied().unwrap_or(0)),
            _ => None,
        },
    }
}

/// Whether witness `w` satisfies every literal of `cond` that is not a primitive's or
/// read's outcome. `None` when a literal is not over literals and inputs only.
fn satisfies(cond: &[(Atom, bool)], w: &Witness, interner: &Interner) -> Option<bool> {
    for &(atom, value) in cond {
        let holds = match atom {
            Atom::Outcome(_) => continue,
            Atom::Lt(l, r) => concrete(l, w, interner)? < concrete(r, w, interner)?,
            Atom::Eq(l, r) => concrete(l, w, interner)? == concrete(r, w, interner)?,
            Atom::Flag(l, r) => {
                let r = concrete(r, w, interner)?;
                if !(0..=16).contains(&r) {
                    return None;
                }
                i32::from(concrete(l, w, interner)?) & flag_bit(i32::from(r)) != 0
            }
        };
        if holds != value {
            return Some(false);
        }
    }
    Some(true)
}

/// The routing slot Go To Routing Slot (opcode 45) names, as far as it decides the
/// outcome (`go_to_slot.rs:333`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteSlot {
    /// Slot `index` of the stack object's own definition (sources 0 and 1).
    Own(i16),
    /// The lot's global routing slot `number` (source 2).
    Global(i16),
    /// Any other source: native error 0x25 once the slot is looked up.
    NoSlot,
}

impl std::fmt::Display for RouteSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Own(i) => write!(f, "the stack object's own slot {i}"),
            Self::Global(n) => write!(f, "global routing slot {n}"),
            Self::NoSlot => write!(f, "no slot (an invalid source, native error 0x25)"),
        }
    }
}

/// The slot a Go To Routing Slot event names under witness `w`: source 0 names it by
/// the parameter `number`, whose value the event's frame snapshot holds.
fn route_slot(event: &LiveEvent, w: &Witness, interner: &Interner) -> Option<RouteSlot> {
    let EventKind::Primitive {
        opcode: 45,
        operands,
        ..
    } = &event.kind
    else {
        return None;
    };
    let number = word_i16(operands, 0);
    Some(match u16::from_le_bytes([operands[2], operands[3]]) {
        0 => {
            let val = event.frame.iter().find_map(|f| match f {
                FrameVal::Read(9, idx, Ok(val), _) if *idx == number => Some(*val),
                _ => None,
            })?;
            RouteSlot::Own(concrete(val, w, interner)?)
        }
        1 => RouteSlot::Own(number),
        2 => RouteSlot::Global(number),
        _ => RouteSlot::NoSlot,
    })
}

/// Why two Go To Routing Slot runs from the same state behave differently in some
/// world, or `None` when the checker cannot tell. Both test the same things first (a
/// person runs it, a stack object exists and stands on the lot), then look the slot up
/// (`go_to_slot.rs:313`). A stack object's definition is part of the world, and its
/// slots are an array: index `i` exists exactly when `0 <= i < len`. The lot's global
/// slots are fixed game data the checker does not know.
/// - Two different own slots, one of them at least 0: a definition with both, placed
///   apart (or, when the other is negative, with the one) routes them differently.
/// - An own slot at least 0 against a global slot or an invalid source: a definition
///   with the own slot, when the global one is missing, or without it, when it exists,
///   makes exactly one of them fail with native error 0x25.
///
/// Two global slots, or a global slot against an invalid source, depend on the fixed
/// table, so they are not decisive.
fn route_slots_differ(a: RouteSlot, b: RouteSlot) -> bool {
    match (a, b) {
        (RouteSlot::Own(x), RouteSlot::Own(y)) => x != y && x.max(y) >= 0,
        (RouteSlot::Own(x), RouteSlot::Global(_) | RouteSlot::NoSlot)
        | (RouteSlot::Global(_) | RouteSlot::NoSlot, RouteSlot::Own(x)) => x >= 0,
        _ => false,
    }
}

/// Whether a path reached its last event with no event before it, from the entry (not a
/// cut), and with no condition on an event's outcome. Then the world (the running object,
/// its stack object, the stack object's definition and position) is whatever the entry
/// had, and the witness's literal bounds are all the path needs, so the world a decisive
/// difference picks is one the trees can meet. Any earlier event could rule that world
/// out: a write can set the stack object to an ID no object has (`stack_obj := -1`), and
/// an opaque read may fault in every world (`global[-1]`, `memory.rs:103`), which
/// [`satisfies`] cannot check because it skips outcome atoms.
fn reached_without_events(parent: u32, cond: &[(Atom, bool)]) -> bool {
    parent == 0
        && !cond
            .iter()
            .any(|(atom, _)| matches!(atom, Atom::Outcome(_)))
}

/// A difference both trees definitely reach: their first primitive (or call), after the
/// same history, from the same state, with operands that make the VM behave differently
/// in some world. Used where the full comparison cannot decide (a loop the checker
/// cannot close, a difference after a primitive), so a wrong operand on a primitive
/// both trees run is still reported.
///
/// Every path ends at its first primitive event, and only paths with no event before it
/// count ([`reached_without_events`]): nothing earlier might fail, never return the way
/// the path needs, or pin the world. The paths' joint condition must hold for a concrete
/// witness on its literal bounds. Only primitives whose operand
/// semantics are modeled here count ([`route_slots_differ`]); the VM replay never runs
/// primitives, so the counterexample says it is not replayed.
fn decisive_difference(
    xa: &Explorer,
    xb: &Explorer,
    interner: &mut Interner,
    entry_param: Option<(i16, bool)>,
    args: usize,
    in_domain: &dyn Fn(&Witness) -> bool,
) -> Option<(Vec<TraceStep>, String)> {
    let (paths_a, _) = xa.explore(interner).ok()?;
    let (paths_b, _) = xb.explore(interner).ok()?;
    let last_event = |p: &PathSummary| match interner.prefix_nodes[p.out.prefix as usize] {
        PrefixNode::Event(parent, ref event) => Some((parent, event.clone())),
        _ => None,
    };
    for pa in &paths_a {
        let Some((parent_a, ea)) = last_event(pa) else {
            continue;
        };
        for pb in &paths_b {
            let Some((parent_b, eb)) = last_event(pb) else {
                continue;
            };
            if parent_a != parent_b || ea == eb {
                continue;
            }
            let (
                EventKind::Primitive {
                    opcode: oa,
                    operands: _,
                    node: na,
                    inline_args: ia,
                    unknown_arity_suffix: ua,
                    callee_arity: ca,
                    argless: ga,
                    caller_frame: fa,
                },
                EventKind::Primitive {
                    opcode: ob,
                    operands: _,
                    node: nb,
                    inline_args: ib,
                    unknown_arity_suffix: ub,
                    callee_arity: cb,
                    argless: gb,
                    caller_frame: fb,
                },
            ) = (&ea.kind, &eb.kind)
            else {
                continue;
            };
            // The same primitive at the same point from the same state; the frame
            // snapshot is left out because it is what source 0 reads, which
            // `route_slot` resolves.
            if (oa, na, ia, ua, ca, ga, fa) != (ob, nb, ib, ub, cb, gb, fb)
                || *oa != 45
                || (ea.epoch, &ea.store, ea.scratch, ea.last_result)
                    != (eb.epoch, &eb.store, eb.scratch, eb.last_result)
            {
                continue;
            }
            if !matches!(
                feasibility(&[&pa.cond, &pb.cond], interner),
                Feasibility::Sat { exact: true }
            ) {
                continue;
            }
            let mut cond = pa.cond.clone();
            cond.extend_from_slice(&pb.cond);
            if !reached_without_events(parent_a, &cond) {
                continue;
            }
            for w in witnesses(&cond, interner, entry_param, args) {
                if !in_domain(&w) || satisfies(&cond, &w, interner) != Some(true) {
                    continue;
                }
                let (Some(sa), Some(sb)) =
                    (route_slot(&ea, &w, interner), route_slot(&eb, &w, interner))
                else {
                    continue;
                };
                if !route_slots_differ(sa, sb) {
                    continue;
                }
                return Some((
                    trace_steps(pa, pb),
                    format!(
                        "both trees reach Go To Routing Slot as their first primitive, after the \
                         same events and from the same state, but A routes to {sa} and B to {sb} \
                         (A path {}, B path {}; witness: {w}). Not replayed in sims-vm, which \
                         does not run primitives: a stack object whose definition has one slot \
                         and not the other, or both placed apart, tells them apart",
                        format_nodes(pa),
                        format_nodes(pb),
                    ),
                ));
            }
        }
    }
    None
}

enum Difference {
    /// The outputs differ for some input satisfying the path condition's literal bounds.
    Definite(String),
    /// The outputs differ syntactically but may be equal (arithmetic terms, unknown arity).
    Possible(String),
}

/// Whether `x` and `y` can take different values under `cond` (for inputs the checker
/// treats as independent).
fn can_differ(x: Val, y: Val, cond: &[(Atom, bool)], interner: &Interner) -> Option<bool> {
    let pin = |v: Val| match v {
        Val::Sym(s) if interner.key(s).is_leaf() => range_of(cond, s).single().map(Val::Const),
        _ => None,
    };
    let (x, y) = (pin(x).unwrap_or(x), pin(y).unwrap_or(y));
    if x == y {
        return Some(false);
    }
    match (x, y) {
        (Val::Const(_), Val::Const(_)) => Some(true),
        _ if (matches!(x, Val::Const(_)) || interner.is_input_val(x))
            && (matches!(y, Val::Const(_)) || interner.is_input_val(y)) =>
        {
            Some(true)
        }
        _ => None,
    }
}

fn first_difference(
    a: &Outputs,
    b: &Outputs,
    cond: &[(Atom, bool)],
    interner: &Interner,
) -> Difference {
    // A primitive both paths ran may never succeed, or never take the branch the
    // paths took after it (a `Random` with a zero bound always fails), so a difference
    // after one is only a possible one.
    let ha = interner.history(a.prefix);
    let hb = interner.history(b.prefix);
    let common = ha.iter().zip(&hb).take_while(|(x, y)| x == y).count();
    let after_primitive = ha[..common].iter().any(is_primitive);
    match first_difference_inner(a, b, &ha, &hb, cond, interner) {
        Difference::Definite(what) if after_primitive => Difference::Possible(format!(
            "{what}, after a primitive both trees run, which may not succeed or return on that branch"
        )),
        d => d,
    }
}

fn first_difference_inner(
    a: &Outputs,
    b: &Outputs,
    ha: &[&PrefixNode],
    hb: &[&PrefixNode],
    cond: &[(Atom, bool)],
    interner: &Interner,
) -> Difference {
    if a.prefix != b.prefix {
        for k in 0..ha.len().max(hb.len()) {
            match (ha.get(k), hb.get(k)) {
                (Some(x), Some(y)) if x == y => continue,
                // Two different events at the same point may still end the same way
                // (both primitives may fail before doing anything), and a difference in
                // what an opaque primitive is handed shows only if it reads it. Only
                // differences the checker models through to the end are definite.
                (Some(PrefixNode::Event(_, ea)), Some(PrefixNode::Event(_, eb))) => {
                    return match event_difference(k, ea, eb, cond, interner) {
                        Difference::Definite(what) => Difference::Possible(what),
                        d => d,
                    };
                }
                // A read that neither traps nor has side effects is unobservable, so an
                // extra or different read alone does not prove the trees differ.
                (Some(x), y) | (y @ Some(_), Some(x)) if is_read(x) || y.is_some_and(is_read) => {
                    return Difference::Possible(format!(
                        "traces differ in a memory read at event {k}: {}",
                        describe_node(x)
                    ));
                }
                (Some(x), None) | (None, Some(x)) if is_read(x) => {
                    return Difference::Possible(format!(
                        "one trace has an extra memory read at event {k}: {}",
                        describe_node(x)
                    ));
                }
                (Some(x), Some(y)) => {
                    return Difference::Possible(format!(
                        "trace diverges at event {k}: A {} vs B {}",
                        describe_node(x),
                        describe_node(y)
                    ));
                }
                (Some(x), None) => {
                    return Difference::Definite(format!(
                        "A has an extra event {k}: {}",
                        describe_node(x)
                    ));
                }
                (None, Some(y)) => {
                    return Difference::Definite(format!(
                        "B has an extra event {k}: {}",
                        describe_node(y)
                    ));
                }
                (None, None) => break,
            }
        }
    }
    if a.exit != b.exit {
        return Difference::Definite(format!("exit differs: A {} vs B {}", a.exit, b.exit));
    }
    if let (Some(fa), Some(fb)) = (&a.fault, &b.fault)
        && fa != fb
    {
        return Difference::Possible(if fa.node == fb.node {
            format!(
                "both fault with {} at node {}, but from different states or instructions",
                a.exit, fa.node
            )
        } else {
            format!("both fault with {}, but at different nodes", a.exit)
        });
    }
    if a.scratch != b.scratch {
        return Difference::Definite(format!(
            "scratch-pointer state at exit differs: A {:?} vs B {:?}",
            a.scratch, b.scratch
        ));
    }
    if let Some(d) = last_result_difference("at exit", a.last_result, b.last_result) {
        return d;
    }
    if let Some(d) = store_difference("final", &a.store, &b.store, a.epoch, cond, interner) {
        return d;
    }
    // What the frame leaves behind shows only if a later frame reads a local before
    // writing it, so a difference in it is only a possible one.
    if let (Some((pa, la, ua)), Some((pb, lb, ub))) = (&a.residue, &b.residue) {
        if la.len() != lb.len() || pa.len() != pb.len() {
            return Difference::Possible(
                "frame residue layout differs (a later frame may read what this one leaves)"
                    .to_owned(),
            );
        }
        let slots = pa
            .iter()
            .zip(pb)
            .enumerate()
            .map(|(i, p)| (format!("final param[{i}]"), p))
            .chain(
                la.iter()
                    .zip(lb)
                    .enumerate()
                    .map(|(i, p)| (format!("final local[{i}]"), p)),
            );
        for (what, (&x, &y)) in slots {
            if can_differ(x, y, cond, interner) != Some(false) {
                return Difference::Possible(format!(
                    "{what} left in the frame storage: A={} vs B={} (a later frame may read it)",
                    interner.describe(x),
                    interner.describe(y)
                ));
            }
        }
        if ua != ub {
            return Difference::Possible(
                "final locals differ in which may still hold what an earlier frame left".to_owned(),
            );
        }
    }
    Difference::Possible("outputs differ".to_owned())
}

/// Compares the thread's last branch result (`None` inside: the caller's, unknown).
fn last_result_difference(
    when: &str,
    a: Option<Option<bool>>,
    b: Option<Option<bool>>,
) -> Option<Difference> {
    if a == b {
        return None;
    }
    let show = |v: Option<Option<bool>>| match v {
        Some(Some(r)) => r.to_string(),
        _ => "the caller's".to_owned(),
    };
    let what = format!(
        "last branch result {when} differs: A {} vs B {}",
        show(a),
        show(b)
    );
    Some(match (a, b) {
        (Some(Some(_)), Some(Some(_))) => Difference::Definite(what),
        _ => Difference::Possible(what),
    })
}

/// Compares tracked-cell maps (absent cells hold their epoch value).
fn store_difference(
    when: &str,
    a: &[(Cell, Val)],
    b: &[(Cell, Val)],
    epoch: u32,
    cond: &[(Atom, bool)],
    interner: &Interner,
) -> Option<Difference> {
    let ma: BTreeMap<Cell, Val> = a.iter().copied().collect();
    let mb: BTreeMap<Cell, Val> = b.iter().copied().collect();
    let cells: BTreeSet<Cell> = ma.keys().chain(mb.keys()).copied().collect();
    for cell in cells {
        let (x, y) = (ma.get(&cell), mb.get(&cell));
        if x == y {
            continue;
        }
        let name = match cell {
            Cell::Temp(i) => format!("{when} temp[{i}]"),
            Cell::Attr(i) => format!("{when} my.attr[{i}]"),
        };
        let untouched = || format!("unchanged since epoch {epoch}");
        let (dx, dy) = (
            x.map_or_else(untouched, |&v| interner.describe(v)),
            y.map_or_else(untouched, |&v| interner.describe(v)),
        );
        // An untouched cell still holds its value from the start of the epoch. If no
        // path ever read that value it was never interned, and it is an unconstrained
        // input distinct from every other value.
        let decided = match (x, y) {
            (Some(&x), Some(&y)) => can_differ(x, y, cond, interner),
            (Some(&v), None) | (None, Some(&v)) => {
                match interner.lookup(&SymKey::Cell(epoch, cell)) {
                    Some(old) => can_differ(v, old, cond, interner),
                    None if epoch == 0
                        && (matches!(v, Val::Const(_)) || interner.is_input_val(v)) =>
                    {
                        Some(true)
                    }
                    None => None,
                }
            }
            (None, None) => Some(false),
        };
        match decided {
            Some(false) => continue,
            Some(true) => {
                return Some(Difference::Definite(format!("{name}: A={dx} vs B={dy}")));
            }
            None => return Some(Difference::Possible(format!("{name}: A={dx} vs B={dy}"))),
        }
    }
    None
}

fn event_difference(
    k: usize,
    a: &LiveEvent,
    b: &LiveEvent,
    cond: &[(Atom, bool)],
    interner: &Interner,
) -> Difference {
    let at = format!("event {k}");
    match (&a.kind, &b.kind) {
        (
            EventKind::Primitive {
                opcode: oa,
                callee_arity: ca,
                ..
            },
            EventKind::Primitive {
                opcode: ob,
                callee_arity: cb,
                ..
            },
        ) if oa == ob && ca != cb => {
            // Checked before the operands and inline arguments: the inline span follows
            // the arity, and the call may fail before reading either.
            return Difference::Possible(format!(
                "{at}: call #{oa} names trees with different argument counts in the two files \
                 ({ca:?} vs {cb:?})"
            ));
        }
        (
            EventKind::Primitive {
                opcode: oa,
                operands: xa,
                inline_args: ia,
                unknown_arity_suffix: sa,
                caller_frame: fa,
                ..
            },
            EventKind::Primitive {
                opcode: ob,
                operands: xb,
                inline_args: ib,
                unknown_arity_suffix: sb,
                caller_frame: fb,
                ..
            },
        ) if oa == ob && xa == xb && ia == ib && (sa != sb || fa != fb) => {
            if sa != sb {
                return Difference::Possible(format!(
                    "{at}: call #{oa} has unknown callee arity and the instructions after it differ"
                ));
            }
            return Difference::Possible(format!(
                "{at}: opcode {oa:#06x} may run a tree whose frame starts at a different \
                 frame-storage offset (caller params/locals A {fa:?} vs B {fb:?}), so its \
                 uninitialized locals may differ"
            ));
        }
        (
            EventKind::Primitive {
                node: na,
                opcode: oa,
                ..
            },
            EventKind::Primitive { node: nb, .. },
        ) if na != nb => {
            // A primitive that yields, or fails, leaves this frame at its node.
            return Difference::Possible(format!(
                "{at}: opcode {oa:#06x} runs at node {na} in A but node {nb} in B, which a \
                 fault or a yield leaves on the stack"
            ));
        }
        (x, y) if !same_shape(x, y) => {
            if matches!(x, EventKind::Read { .. }) || matches!(y, EventKind::Read { .. }) {
                return Difference::Possible(format!(
                    "traces differ in a memory read at {at}: A {} vs B {}",
                    describe_kind(x, interner),
                    describe_kind(y, interner)
                ));
            }
            return Difference::Definite(format!(
                "trace diverges at {at}: A {} vs B {}",
                describe_kind(x, interner),
                describe_kind(y, interner)
            ));
        }
        (x, y) => {
            let pair = match (x, y) {
                (EventKind::Read { indirect: va, .. }, EventKind::Read { indirect: vb, .. }) => {
                    va.zip(*vb).map(|(p, q)| ("indirect index", p, q))
                }
                (
                    EventKind::Write {
                        indirect: va,
                        rhs: ra,
                        ..
                    },
                    EventKind::Write {
                        indirect: vb,
                        rhs: rb,
                        ..
                    },
                ) => {
                    if ra != rb {
                        Some(("written value", *ra, *rb))
                    } else {
                        va.zip(*vb).map(|(p, q)| ("indirect index", p, q))
                    }
                }
                _ => None,
            };
            if let Some((what, p, q)) = pair
                && p != q
            {
                return match can_differ(p, q, cond, interner) {
                    Some(false) => {
                        Difference::Possible(format!("{at}: {what} spelled differently"))
                    }
                    Some(true) => Difference::Definite(format!(
                        "{at} {}: {what} A={} vs B={}",
                        describe_kind(x, interner),
                        interner.describe(p),
                        interner.describe(q)
                    )),
                    None => Difference::Possible(format!(
                        "{at} {}: {what} A={} vs B={}",
                        describe_kind(x, interner),
                        interner.describe(p),
                        interner.describe(q)
                    )),
                };
            }
        }
    }
    // A built-in primitive may overwrite the scratch pointer, the last branch result and
    // the tracked cells without reading them (`Stop All Sounds` sets its own result,
    // `sound.rs:64`), so a difference in them before one is only a possible one. Idle
    // and Gosub Found Action read the last result first (`idle.rs:187`,
    // `found_action.rs:102`). A called tree is handed all of them.
    let builtin = match a.kind {
        EventKind::Primitive { opcode, .. } if opcode < 256 => Some(opcode),
        _ => None,
    };
    let opaque = |d: Difference, reads_last_result: bool| match d {
        Difference::Definite(what)
            if builtin.is_some_and(|op| !(reads_last_result && matches!(op, 17 | 30))) =>
        {
            Difference::Possible(format!("{what}, but the primitive may overwrite it unread"))
        }
        d => d,
    };
    if a.scratch != b.scratch {
        let what = format!(
            "scratch-pointer state before {at} ({}) differs: A {:?} vs B {:?}",
            describe_kind(&a.kind, interner),
            a.scratch,
            b.scratch
        );
        return opaque(Difference::Definite(what), false);
    }
    if let Some(d) = last_result_difference(
        &format!("before {at} ({})", describe_kind(&a.kind, interner)),
        a.last_result,
        b.last_result,
    ) {
        return opaque(d, true);
    }
    if let Some(d) = store_difference(
        &format!("before {at} ({})", describe_kind(&a.kind, interner)),
        &a.store,
        &b.store,
        a.epoch,
        cond,
        interner,
    ) {
        return opaque(d, false);
    }
    if a.frame.len() != b.frame.len() {
        return Difference::Definite(format!("{at}: frame operands differ"));
    }
    for (fa, fb) in a.frame.iter().zip(&b.frame) {
        if fa == fb {
            continue;
        }
        return match (fa, fb) {
            (FrameVal::Read(s, i, _, ua), FrameVal::Read(t, j, _, ub))
                if s == t && i == j && ua != ub =>
            {
                Difference::Possible(format!(
                    "{at}: frame operand scope {s}[{i}] may still hold what an earlier frame left on one side"
                ))
            }
            (FrameVal::Read(s, i, Err(_), _), FrameVal::Read(t, j, Err(_), _))
            | (FrameVal::Dst(s, i, Err(_)), FrameVal::Dst(t, j, Err(_)))
                if s == t && i == j =>
            {
                Difference::Possible(format!(
                    "{at}: frame operand scope {s}[{i}] lies outside the frame by different amounts"
                ))
            }
            (FrameVal::Read(s, i, Ok(x), _), FrameVal::Read(t, j, Ok(y), _))
                if s == t && i == j =>
            {
                match can_differ(*x, *y, cond, interner) {
                    Some(true) => Difference::Definite(format!(
                        "{at} {}: frame operand scope {s}[{i}] A={} vs B={}",
                        describe_kind(&a.kind, interner),
                        interner.describe(*x),
                        interner.describe(*y)
                    )),
                    _ => Difference::Possible(format!(
                        "{at}: frame operand scope {s}[{i}] may differ"
                    )),
                }
            }
            (
                FrameVal::AllLocals(..) | FrameVal::TextLocals(..),
                FrameVal::AllLocals(..) | FrameVal::TextLocals(..),
            ) => Difference::Possible(format!(
                "{at}: text formatting may read frame locals via $Local tokens and local state differs"
            )),
            (FrameVal::WholeFrame(..), FrameVal::WholeFrame(..)) => Difference::Possible(format!(
                "{at}: an unmodeled primitive may read the frame and frame state differs"
            )),
            (FrameVal::CallArgs(x), FrameVal::CallArgs(y)) => Difference::Possible(format!(
                "{at} {}: call arguments may differ (A {} vs B {})",
                describe_kind(&a.kind, interner),
                describe_args(x, interner),
                describe_args(y, interner)
            )),
            _ => Difference::Definite(format!(
                "{at} {}: frame operands differ ({fa:?} vs {fb:?})",
                describe_kind(&a.kind, interner)
            )),
        };
    }
    Difference::Possible(format!("{at}: events differ"))
}

fn describe_args(args: &[Option<Val>], interner: &Interner) -> String {
    let words: Vec<String> = args
        .iter()
        .map(|a| a.map_or_else(|| "unread".to_owned(), |v| interner.describe(v)))
        .collect();
    format!("({})", words.join(", "))
}

fn is_primitive(node: &&PrefixNode) -> bool {
    matches!(
        node,
        PrefixNode::Event(
            _,
            LiveEvent {
                kind: EventKind::Primitive { .. },
                ..
            }
        )
    )
}

fn is_read(node: &&PrefixNode) -> bool {
    matches!(
        node,
        PrefixNode::Event(
            _,
            LiveEvent {
                kind: EventKind::Read { .. },
                ..
            }
        )
    )
}

fn same_shape(a: &EventKind, b: &EventKind) -> bool {
    match (a, b) {
        (
            EventKind::Primitive {
                node: na,
                opcode: oa,
                operands: xa,
                inline_args: ia,
                unknown_arity_suffix: sa,
                callee_arity: ca,
                argless: ga,
                caller_frame: fa,
            },
            EventKind::Primitive {
                node: nb,
                opcode: ob,
                operands: xb,
                inline_args: ib,
                unknown_arity_suffix: sb,
                callee_arity: cb,
                argless: gb,
                caller_frame: fb,
            },
        ) => {
            na == nb
                && oa == ob
                && xa == xb
                && ia == ib
                && sa == sb
                && ca == cb
                && ga == gb
                && fa == fb
        }
        (
            EventKind::Read {
                scope: sa, idx: ia, ..
            },
            EventKind::Read {
                scope: sb, idx: ib, ..
            },
        ) => sa == sb && ia == ib,
        (
            EventKind::Write {
                op: oa,
                scope: sa,
                idx: ia,
                ..
            },
            EventKind::Write {
                op: ob,
                scope: sb,
                idx: ib,
                ..
            },
        ) => oa == ob && sa == sb && ia == ib,
        _ => false,
    }
}

fn describe_kind(kind: &EventKind, interner: &Interner) -> String {
    match kind {
        EventKind::Primitive {
            opcode, operands, ..
        } if *opcode >= 256 => format!("call #{opcode} {operands:02x?}"),
        EventKind::Primitive {
            opcode, operands, ..
        } => format!("primitive {opcode:#06x} {operands:02x?}"),
        EventKind::Read {
            scope,
            idx,
            indirect,
        } => match indirect {
            Some(v) => format!("read scope {scope}[param {}]", interner.describe(*v)),
            None => format!("read scope {scope}[{idx}]"),
        },
        EventKind::Write { op, scope, idx, .. } => format!("write op{op} scope {scope}[{idx}]"),
    }
}

fn describe_node(node: &PrefixNode) -> String {
    match node {
        PrefixNode::Root => "entry".to_owned(),
        PrefixNode::Event(_, e) => format!("{:?}", e.kind),
        PrefixNode::Branch(_, v) => format!("branch {v}"),
        PrefixNode::Fault(_) => "fault".to_owned(),
        PrefixNode::Cut(_) => "cut".to_owned(),
    }
}

fn format_nodes(p: &PathSummary) -> String {
    let mut s = String::new();
    for n in &p.nodes {
        s.push_str(&format!("n{n}->"));
    }
    s.push_str(&p.out.exit.to_string());
    if p.entry != 0 {
        s = format!("[entry n{}] {s}", p.entry);
    }
    s
}

fn format_cond(cond: &[(Atom, bool)], interner: &Interner) -> String {
    let parts: Vec<String> = cond
        .iter()
        .filter(|(a, _)| !matches!(a, Atom::Outcome(_)))
        .map(|&(atom, v)| {
            let body = match atom {
                Atom::Lt(l, r) => format!("{} < {}", interner.describe(l), interner.describe(r)),
                Atom::Eq(l, r) => format!("{} == {}", interner.describe(l), interner.describe(r)),
                Atom::Flag(l, r) => {
                    format!("{} has flag {}", interner.describe(l), interner.describe(r))
                }
                Atom::Outcome(p) => format!("event {p} true"),
            };
            if v { body } else { format!("!({body})") }
        })
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("; when {}", parts.join(" && "))
    }
}

fn trace_steps(pa: &PathSummary, pb: &PathSummary) -> Vec<TraceStep> {
    let n = pa.nodes.len().max(pb.nodes.len());
    (0..n)
        .map(|i| TraceStep {
            pc_a: pa.nodes.get(i).copied().unwrap_or(0xFF),
            pc_b: pb.nodes.get(i).copied().unwrap_or(0xFF),
            event: "live-output path".to_owned(),
            branch: None,
        })
        .collect()
}
