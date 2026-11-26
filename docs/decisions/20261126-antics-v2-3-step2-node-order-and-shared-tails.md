# `.antics` v2.3 step 2: node-order pinning, shared tails, staging review, `light`

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-antics/src/node_order.rs` (new)
  - `src/{ast,parser,lower,lib}.rs` (the `.air` tree header and compiler)
  - `src/v2/{ast,attrs,parser}.rs`
  - `src/v2/lower/{callable,mod,object,schema,tree,trees}.rs`
  - no edits to `src/v2/lint.rs` or `src/v2/decompile.rs`
- **Track:** Track 8 (compiler), `.antics` v2.3, step 2 (strict-mode enablers for Track 9)
- **Builds on:**
  - [`20261125-antics-v2-2-temps-and-flags.md`](20261125-antics-v2-2-temps-and-flags.md) (#480, in-place staging)
  - [`20261126-antics-v2-3-step1-if-let-and-469-fixes.md`](20261126-antics-v2-3-step1-if-let-and-469-fixes.md)
  - [`20261123-verifier-behavioral-mode.md`](20261123-verifier-behavioral-mode.md)

Track 9's strict mode wants a lifted tree to compile to the original's bytes. Two gaps were
left after v2.2. The first is node numbers: a structured lift emits the right instructions in
another order. The second is shared tails: the original jumps into one copy of a tail from
several places, which a structured body writes once per arm. Shared tails also produce the 46
behavioral `frame_headroom` lints, which flag lifted trees of more than 200 nodes.

## Decision 1: `node_order` renumbers a compiled tree

`#[bhav(.., node_order = [0, 5, 1, ..])]` sets the node order, and so does the `.air` header
field `node_order: [..]`.

- **Meaning.** Entry `i` names the compiled node that becomes node `i`. Every branch follows
  its node. "Compiled" means after `share_tails` (Decision 2), when both are given.
- **Checks.** Each one fails with an error that says what is wrong:
  - the list names every node exactly once, so its length is the compiled node count;
  - it starts with `0`. A tree runs from node 0, so the entry cannot move.
- **Indexed trees (kind 4) keep every node in place.** Such a tree is entered at the node its
  first argument names. Only the identity order is accepted.
- **What it changes.** Only node numbers change; each run executes the same instructions in
  the same order. Node numbers are observable in two ways:
  - **a save game.** It holds the node each running frame stopped at. A save from one build
    resumes another build at the wrong node, which is the behavioral mode's
    "start and end under one build" caveat. Pinning to the original's numbers removes it.
  - **a call that reads the nodes after it.** See Decision 3.
  - **a fault report.** The VM reports a fault at its node number, so a fault keeps its
    position only under the order that numbers it. Strict comparison already treats fault
    positions as node numbers (the verifier does not equate faults at different nodes).
- **The tree must fit a branch byte.** A rewritten tree holds at most 251 nodes, whatever
  option and however the tree was written: a verbatim tree may assemble more, and renumbering
  one would turn jumps into return bytes.
- **`match_node_order(compiled, original)`** (`sims_antics::node_order`) finds the order that
  turns one tree into another. Track 9 uses it to write the pin.
  - It walks both trees from node 0 together. Each pair of nodes must hold the same opcode
    and operands, and each pair of branches must return the same way or lead to a pair of
    nodes that is consistent with the pairs found so far.
  - It returns `None` when the trees differ in length, or when a node of either tree is not
    reached from node 0. An unreached node has no position the walk can fix.
  - Unit and integration tests round-trip it through `node_order` on if/else, `match`,
    `while`, `if let`, raw blocks, lifecycle trees and `.air`.

## Decision 2: `share_tails` merges nodes that run alike

`#[bhav(.., share_tails)]` merges nodes that run alike, and so does the `.air` header field
`share_tails: true`.

- **Rule.** The pass computes the coarsest bisimulation by Moore partition refinement. Two
  nodes merge when they hold the same opcode and operands, and each pair of branches returns
  the same way or leads to nodes that merge. Loops are included.
  - Each merged class keeps its first node's position. Node 0 stays the entry, and an
    unmerged tree comes out unchanged.
- **Why it is sound.** A node's behaviour is its instruction plus where each outcome goes.
  Merging bisimilar nodes changes no run's sequence of instructions or exits; only node
  numbers change, as in Decision 1.
- **Indexed trees are refused** when anything would merge, because a merge renumbers nodes.
- **Opt-in.** Open-pack and every existing source compile byte for byte as before, and the
  gate checks this (`pack_cmp`). The original game shares some tails but not all, so maximal
  sharing is not always what a strict lift wants. Track 9 asks for it per tree.
- **Size limit.** A tree with `share_tails` may assemble more than 251 nodes. The limit is
  checked after merging. Refinement only splits classes, so it stops as soon as the classes
  pass 251, and the error gives that lower bound. Without `share_tails` the limit is unchanged
  ("exceeds maximum of 251").
- **Branches must be resolvable.** Under either option, a branch to a node the tree does not
  have is refused, and so is a raw branch byte that names no node (`0xFB`, `0xFC`). The VM
  faults on both, and renumbering or merging could make either name a real node.
  `match_node_order` likewise answers `None` for a tree with such a byte.

## Decision 3: a rewritten tree may call with inline arguments only into narrow frames

A subroutine call carries up to four argument words in its operands; this is the inline
form, as opposed to `(temps)`. A callee whose frame holds more arguments reads the rest from
the instruction records after the call (`sims-vm` interpreter; `verify.rs` checks this as
"unknown callee arity and differing following instruction layout").

Merging or renumbering moves those records. So a tree that `share_tails` or `node_order`
rewrites may make an inline call only to a tree listed in its header's
`narrow_callees: [..]`, a tree whose frame holds at most four arguments.

- **`.air`.** `lower_tree` refuses an unlisted inline call. The error names the node, the
  callee, and the fix.
- **A tree of the same module, in the caller's own library range, is checked by the frame it
  compiles to.** This happens after every tree is lowered, whether or not the tree is listed.
  - A tree's frame can be wider than its signature: `.air` infers the argument count from
    the `param[N]` its body reads.
  - The frame is the one the VM reads (`Behavior::frame_dimensions`): format `0x8000` has
    four arguments, `0x8001` at most four, and `0x8002` the header's count.
  - The VM looks a callee up in the library its ID range names: global `256..=4095`, private
    `4096..=8191`, semi-global `8192..`. So a module tree in another range, such as a global
    ID in an object's file, is not what the caller runs. It must be listed like any other
    callee.
- **v2.** The lowerer lists an inline call when every declaration of that tree id the object
  can see, in any module or in the object, gives it a frame of at most four arguments. A
  declaration is an `extern`'s positional count, or a tree's `args =` frame field.
  - Scope is ignored: one tree has one frame. An inlined helper sees only its own module's
    declarations, but the object's may disagree.
  - Two declarations that disagree grant nothing.
  - A semi-global ID (`8192..`) is never listed. Which library holds it depends on the
    `GLOB` link of the object that runs the call, so a declaration made for one library says
    nothing about another's tree. A typed call to one is refused with that reason. A raw one
    fails closed at `.air`.
- **`narrow_callees` in hand-written `.air` is an assertion the compiler trusts**, as it
  trusts the rest of an `.air` header. A wrong entry is the author's error. An example is a
  semi-global ID listed for an object whose `GLOB` links back to its own file, where the
  private and semi-global ranges resolve to the same file. The v2 front end lists only what
  its rules prove. That is the
  same knowledge the optimizer's `keep_layout` rule already trusts. An inline call to a wider
  frame is refused at the call: "fn 't' has 'share_tails', which moves nodes, but this call
  passes its arguments inline to …, whose frame holds 6 arguments …".
- **A raw block's inline calls** (`call N(..)` other than `(temps)`, and `op N (..)` for
  N ≥ 256) are listed when every signature visible to the tree that names `N` gives it a narrow
  frame: a sibling tree, or an `extern` fn such as `std::globals`. Otherwise the `.air`
  compiler refuses the tree, so the rule fails closed. The `.air` check is what makes it
  sound; the v2 list only grants permission, and each entry is a fact about its callee.
  Other raw call spellings that `.air` accepts (`subroutine(N, ..)`, `call name(..)`) are not
  scanned. They fail closed too.

The equivalence runs below found this rule. Without `--global`, the verifier proved only 2 of
the 46 trees outright, because their global callees' arities were unknown. With `--global`, it
proved 43.

## Decision 4: in-place staging is already as wide as is sound

The step asked whether #480's in-place call staging can cover more argument shapes. It cannot
usefully:

- **A local or parameter beside temps** (`f(temp[1], k)`, `f(temp[1], x)`) already compiles
  to `temp[0] := temp[1]; temp[1] := <local>;` with no copy. The copying form snapshots
  only the temp. The optimizer's copy propagation then forwards that snapshot, because
  nothing between can change or observe it.
  - Adding locals and parameters to `stages_in_place` was tried. It left open-pack and every
    test byte-identical, so it was reverted.
  - The test `locals_and_parameters_beside_temps_already_need_no_copy` pins the current
    output.
- **An attribute of another object** (`stack_obj.attr[2]`) is a single read that runs no
  code. The copying form already reads it where it stores it, as on main.
- **The copies that remain are required:**
  - **Arithmetic on a temp** (`temp[0] + 1`) is worked out in a local before any store, so
    the temp it reads still holds the caller's value.
    - Folding it into the destination would change which node's Expression sets the last
      result and the scratch pointer, both strict-mode observables.
    - The corpus has about 237 such arguments, mostly `expr()`.
  - **An argument a later argument's code could change** (`later_impure`). A call or a
    foreign read may run a tree that writes the temp.
  - **A runtime `bcon[..]` read.** It can fault on a missing chunk, and the temps a fault
    leaves are observable.
  - **Every staging under `copy_staging`**: global 34 read, `#[temp]` loops, a fixed `kind`,
    or keep-layout objects. There node identity is observable, as #480 recorded.

Soundness is unchanged: no staging rule changed in this step.

## Decision 5: `light` names object data word 51

- **`self.light`** (and `obj.light` / `stack_obj.light`) is data word 51, `i16`. It is the
  light the object gives.
  - When `flags2` holds `LightSource` and the object stands on the lot, the room count adds
    it to its room's lamp light. A window's goes to the window light.
  - A lamp that gives light is drawn lit.
  - Sources: sims-sim `rooms.rs` `count_object`; sims-render `scene.rs` `LIGHT`, `lot.rs`
    `gives_light`.
- A state of the same name still wins on `self`, as for every engine word.
- The `room_impact` (word 7) comment said the word lights the room. It is the room score
  impact, and the comment is fixed.

## Decision 6: review round 1

The Sol review found three bugs, two risks and a nit. All are fixed, and each fix has a
regression test (`review_round_1_rewrite_soundness`, plus `node_order` unit tests):

- **Bug: an inferred sibling frame.** A sibling tree whose body reads `param[4]` passed as
  narrow by its zero-argument signature. Same-module callees are now checked by their compiled
  frame (Decision 3).
- **Bug: reserved branch bytes.** A raw `0xFB`/`0xFC` branch could become a real jump. It is
  now refused, and the matcher refuses it too (Decision 2).
- **Bug: oversized `node_order` trees.** A `node_order`-only rewrite of a verbatim tree with
  more than 251 nodes could emit return bytes for jumps. The size limit now applies to every
  rewrite (Decision 1).
- **Risk: conflicting declarations.** A typed call could grant an id that another visible
  declaration declares wider. Typed calls now ask the same all-agree question as raw calls.
- **Risk: refinement time.** On a long chain, refinement now stops once the tree cannot fit.
- **Risk: fault positions.** Fault reports carry node numbers. This is documented with the
  other node-number observables (Decision 1).
- **Nit: list entries.** A `node_order` list entry is an integer with at most a leading `-`
  (`-+0` was accepted).

## Decision 7: review round 2

The Sol re-check confirmed the round 1 fixes. It found two more bugs and a false rejection,
all now fixed, with tests in `review_round_2_callee_libraries_and_frames`:

- **Bug: a same-file copy of a global.** A module tree with the callee's ID counted as a
  sibling even when the VM takes that ID from another library. Siblings are now limited to
  the caller's own range (Decision 3).
- **Bug: an inlined helper's narrower declaration.** A module-level helper that calls a
  global through a one-argument `extern` passed, although the object declared the same ID
  with five arguments. The frame question now covers every declaration (Decision 3).
- **False rejection: capped formats.** A callee with `args: 5` in format `0x8000` or `0x8001`
  was refused, although the VM reads four arguments. The check now uses the VM's frame
  dimensions.

## Decision 8: review round 3

A narrow Sol re-check of the round 2 fixes confirmed all three. It found one more bug, now
fixed:

- **Bug: semi-global declarations.** Considering every declaration let an unused module's
  one-argument `extern` for a semi-global ID vouch for another library's five-argument tree.
  The reverse case could also block a valid rewrite. Semi-global IDs are no longer listed
  from declarations (Decision 3).
- **Left as is:**
  - an explicit, wrong `narrow_callees` entry in hand-written `.air`, which is documented as
    trusted (Decision 3);
  - an empty `0x8002` callee with `args: 5` is refused, although the VM faults on it before
    reading any argument. This is conservative.

## Measurements

These cover Track 9's behavioral lift of the base install (`lift-v2-2-final-beh`, 26,206
trees), with every tree compiled plain and with `share_tails`, and matched against the
original:

| | trees |
|---|---|
| byte-identical already | 2,831 |
| identical after `node_order` alone | 757 |
| identical only after `share_tails` + `node_order` | 280 |
| `share_tails` refused by Decision 3 (a raw inline call to an undeclared or semi-global callee) | 154 |

Of the 154 refused trees, one is already byte-identical, and one would match with
`node_order` alone but cannot be pinned. The other 152 match the original neither way.
Before raw calls were listed from their signatures, 841 trees were refused, among them 22 of
the 46 below.

The 46 behavioral `frame_headroom` trees (over 200 nodes) all fall below 200 with
`share_tails` (the largest to 196), for example 201 → 49, 244 → 62 and 243 → 161. None
reaches the original's exact node count, since maximal sharing merges more than the original did.

`verify-equiv --behavioral --all --global Global.iff` compared each object's plain build
with its `share_tails` build:

- 43 are EQUIVALENT;
- 3 are EQUIVALENT modulo unproven trees. Those trees call semi-global library trees whose
  arity was not given (`--semi-global`), or exceed the checker's state budget;
- 0 are DIFFERENT.

## Consequences

- **Track 9 (strict lift):**
  - write `#[bhav(.., node_order = [..])]` from `match_node_order(compiled, original)` on
    every tree that matches;
  - write `share_tails` (then `node_order`) where only the shared build matches, and on the
    46 `frame_headroom` trees;
  - leave both off kind-4 trees, and off trees whose raw blocks call undeclared callees
    inline (Decision 3).
- **Track 7 (lint):** `light` is a new engine word for the `OBJECT_WORDS`/`SELF_FIELDS`
  lists. `frame_headroom` needs no change: it reads the compiled tree, which is already
  shared.
- **Follow-ups:**
  - declare more global frames in `std` (for example IDs 334 and 425), and give the v2 front
    end each object's `GLOB` link, so that semi-global frames can be known. Either would let
    more of the 154 refused trees share tails;
  - add a partial-sharing mode, for trees where the original shares only some tails.
