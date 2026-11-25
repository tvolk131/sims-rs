# `.antics` v2: PR I-2 Pilot Fixes

*Decided 2026-11-25.*

**Branch:** `t3/antics-v2-pr-i2-pilot-fixes`  
**Track:** Track 8 (`.antics` v2)

## Context

Round 4 (the Rust-like syntax of PR I-1, #453) and the Tier B expansion pilots found real bugs and
rough edges in the v2 compiler:

- functional kind names off by one against the engine's table;
- a local `extern mod globals` clashing with `use std::globals::*;`;
- `stack_obj.run_functional(N)` clobbering a found cursor;
- `me` not reading like the Sim handle in `#[context(inherit)]` trees;
- `sim.debit_funds` / `credit_funds` refused as bool expressions;
- bare `1 => break,` match arms refused;
- `$Me` text forcing an unused entry save;
- some smaller cleanups.

Every regression test is synthetic and lives in `tests/integration/v2_rust_syntax.rs`.

## Decision

The code is the definition of each rule. This section says what changed and why.

### 1. Functional kinds follow the engine's table (`parser.rs` `FUNCTION_KINDS`)

`Run Functional Tree` maps function N to an `OBJf` slot, and Find Best Object searches some
functions by an object data word. Both tables come from `sims-vm` (`run_functional_tree.rs`,
`find_best_object.rs`). One table now holds, for each function:

- the v2 name;
- the function number;
- the callback;
- the search: `Score(word)`, `Fixed`, or `None` (never searched).

Both the `.air` parser and v2 lowering read this table.

The names were shifted by one from function 10 on. The names are now:

`prep`, `cook`, `surface`, `dispose`, `eat`, `pickup_from_slot`, `wash_dish`, `eating_surface`,
`sit`, `stand`, `serving_surface`, `clean`, `gardening` (new, function 12), `wash_hands` and
`repair`. `category4` (function 15) is unchanged.

The retired spellings fail with a hint naming the kind that now gives their function:

- `serve` → `gardening`;
- `rest` → `repair`;
- `pick_up` → `eating_surface`.

Three names stay but now mean the engine's function:

- `eat` (function 4);
- `clean` (function 11);
- `repair` (function 14).

`find_best_object(kind)` is refused for a kind the engine never searches, because it would always
fail. The untyped search also takes `category4`, as raw `.air` does. A `functional` block's
`score:` is accepted only for a scored kind. A typed search's fallback scan takes a candidate only
when the kind's own score word is nonzero, for every scored kind. This matches the engine, which
skips only a zero score (a negative score is eligible). Before this, the scan checked only `prep`
and `cook`, and required a positive score.

### 2. A local `extern mod` merges with an imported one (`loader.rs` `merge_extern_mods`)

`use std::globals::*;` together with an object's or module's own `extern mod globals { .. }` now
gives one module holding both sets of functions.

A declaration's module types (an enum, bitflags, an object, `Self`) resolve in the module that
owns it. Built-in types (`Height`, `Placement`, ..) are the same everywhere and do not count. The merged block
has one owner, so only one side may name module types: that side owns the merge. The merge is
refused, with an error naming both owners, when:

- both sides, from two modules, name module types (two `Mode`s spelled alike may differ);
- one name is declared twice with a different signature (parameter types or ABI slots, or
  return);
- a function calls its tree differently from any function of the other side with that tree id
  (every alias is checked, not only the first, and before an exact duplicate is skipped);
- an object's own block would take a module's declaration that names a module type: the object's
  block resolves types in the object's scope, where a local type may shadow it.

The same declaration in both is fine. Neither the result nor the refusal depends on import
order. This applies to object-member `extern mod` blocks
(merged with the module scope's block) and to glob and explicit imports into a scope that
already binds the name.

### 3. `stack_obj.run_functional(N)` restores nothing (`StackRestore`)

`sims-vm` runs the functional tree in a pushed frame, and the stack object belongs to each frame.
So a call whose receiver is already `stack_obj` leaves the caller's cursor where it was.
`prepare_run_functional` therefore returns `StackRestore::Unmoved` for that receiver and
`Moved(saved)` for any other. The restore code emits nothing for `Unmoved`.

Before this, the call saved and restored the cursor through a local that never held the found
object, so the cursor that a `find_best_object` had just found was clobbered.

The receiver stays in place when the call has no argument code. Argument code may move the
cursor (reading another object's cells moves it and puts back the entry object; a step such as
`set_to_next(.., dest: stack_obj)` moves it on). So when there is any, the cursor is saved before
the argument code and selected again for the call, and nothing is restored after it.

### 4. `me` is the Sim handle in inherit trees (`TreeLowerer::is_my_ident`)

In a `#[context(inherit)]` tree, `me` (the frame's own object) reads typed fields and calls
methods the same way the `sim` handle does in a Sim tree. For example:

- `me.skill.body`;
- `me.motive.mood = ..`;
- `me.id`;
- `play_sound` and `refresh(target: me)`.

`is_my_ident` (`is_sim_ident || is_frame_me`) replaces `is_sim_ident` in lowering, lvalues,
const evaluation, callables, builtins and the stack-target checks. Lint runs on the lowered
result, so lint and lowering agree.

The Sim fields apply only where `me` may be a Sim: an inherit tree and a Sim tree. In an
object's own tree (Init, Main, a plain `#[bhav] fn`), `me` is the object, so only its raw cells
apply there, as on main. A `bcon me` table keeps its name. A local named `me` shadows the
handle, and a cursor loop's body may not bind `me` (the loop checks its body against the names as
they stand outside it). A field nothing in scope has fails with "unknown field 'me.f' (nothing in
scope has it)".

### 5. Budget methods are bool expressions

`sim.debit_funds(n)` and `sim.credit_funds(n)` are accepted as statements, conditions (negated
too), values and helper tails. They give the same nodes as the `debit_funds(n)` call form, with no
extra local or node.

### 6. Bare exit arms

A match arm's short form is, in order:

1. a bare exit ended by the arm's `,` or `}` (`1 => break,`, `_ => return flag_set,`);
2. otherwise one expression, then an optional assignment operator and value (also to a binding
   named `break` or `return`: `_ => break += 1,`, `_ => return.person[3] += 1,`). Without the
   operator, the expression is the arm's value.

An ordinary arm's text is parsed once, so nested `match` arms no longer reparse at every level.
Only an arm starting with `break`, `continue` or `return` that is not a bare exit is parsed twice.

So `1 => break,` lowers as the statement, and `fmt` keeps it.

### 7. `$Me` text and the entry save (`mir::text_words_unmoved`)

#### The R3 rule (unchanged)

An unread entry save (`let __self_obj = stack_obj;`) is dropped only when every frame word the
object's text may read keeps its occupant. The save's own word always changes hands. `$Me` made
the text's reads `Unknown`, so the save stayed in any tree that formats text.

#### The `$Me` exception

`$Me` writes the running object's name into the text, and later formatter passes read it as
tokens. A name holding `$Local:N` therefore reads word N, which can be any word. The objects
carry a second fact, `text_reads_besides_me` (`mir::strings_read_besides_me`): the strings read
with each `$Me` replaced by a space.

A name without `$` can still complete a token that the source starts right before the `$Me`:
- `$Local:$Me` with the name `0` reads word 0;
- `$Loc$Me` with the name `al:0` reads word 0 too;
- in `$TimeLocal:$Me$Local:0`, the name's digits and local 0's value become the hour token's
  number.

So a `$Me` right after a letter, digit, `_`, `-`, `:` or `$` leaves this fact `Unknown`.

When the reads are `Unknown` only because of `$Me`:

- the words the strings name explicitly are checked by the strict rule above, so the save's own
  word counts;
- every other word must also keep its named occupant, apart from the save's own word.

#### When the save stays

The save stays when:

- the text reads are `Unknown` apart from `$Me` too (for example `$Object` or `$NameLocal`);
- the frame is a bottom frame;
- the tree runs under main's rules (global 34).

#### The accepted gap

A Sim or catalog name that itself contains `$` can form a token naming the save's own word (a name
holding `$Local:1`, or ending in `$Local:` before source digits). With the save kept, that token
shows the entry object's id. With the save dropped, it shows the token itself or an earlier
frame's leftovers. No source local lives in that word, so no source value is shown wrong.

#### Idle scratch counts only when allocated

The idle scratch word is the word after the named locals, which `.air` uses around an `idle`. It
is now an occupant only when `.air` actually allocates it, meaning the tree reads `param[0]` and
idles (`AirLocalUsage::uses_idle_scratch`). Before this, the word was assumed for every tree.

### 8. Cleanups

- Not changed: a bool helper with a constant tail, returned through `return h();` in a tree whose
  raw block keeps the layout, gives the test node `if 1 == 0 { return true; } return false;`.
  That node sets the last result, and when the return ends an active action, `release_use`
  records the last result in the action's flag. The call before the node may have left the other
  result. So the node stays, and a test pins it.
  - The inline-branch path (no raw block) already returns the constant without the node, which can
    leave the call before it as the recorded result (pre-existing on main). It is flagged to the
    coordinator, not changed here.
- A lint `allow` comment on its own line above an item's attributes (`#[bhav(..)]`) covers the
  attributes and the item's line, as it does directly above the item. Before this, it covered
  only the attribute line, and a finding on the `fn` line was still reported.
- `frame_headroom` help now suggests a `#[bhav] fn` instead of `tree`.
- `std::globals::multi_tile_exit` was checked against tree 279. It reads its delay from
  `param[0]` (an explicit argument, like 273) and writes `param[1]` before reading it. The
  signature `fn multi_tile_exit(delay: i16) -> Result` is right, and a comment now says so.
- The `test_runner/harness.rs` docs point at `tests/integration/open_pack_behavioral.rs`.

## Consequences

- `open-pack` lint has 0 findings, and `fmt --check` is clean. `open-pack` builds byte-identical
  except `open_fridge.iff`, whose two typed searches (`prep`, `cook`) now test their fallback score
  with `!= 0` instead of `> 0` (section 1). That changes behaviour only for a negative score, which
  the engine treats as eligible.
- `.antics` sources using `serve`, `rest` or `pick_up` must be renamed. The error says to what.
- Not changed here: `global[CONST]` is still counted as a global-34 read by
  `mir::may_read_global_34` (Tier B bug 6). That is conservative, not unsound.
