# The pie menu's offer to join a running interaction

*Decided 2026-11-20.*

## What the original does

Listing the pie menu of an object O for a person P (0x45859c) ends with
one more entry after the table's: the offer to join P's running
interaction (0x45866d). It is made when:

- the running interaction's entry, in its target's own table (0x41f0ff),
  has flag 2 (may be joined);
- O is a person other than P.

A target without a table offers nothing, so it gets no offer either
(0x4585ff). An empty object or self menu can still reach the offer:
when a menu offers nothing, the listing falls back to the other objects
on the tile (0x48d423), and one of those may be a person.

The action (0x41ee99) is a copy of P's running one (0x41edbf), changed
as follows:

- flags 2 and 4 are cleared;
- word +0 is 1 (it joins);
- O is the caller;
- the priority is 50.

It goes to the running target's interaction target (0x435286). Its menu
number is the joining index of that target's first entry with the
running menu number. Without any one of these, the action has no target
and keeps the running menu number:

- a table;
- such a first entry;
- a joining index other than -1;
- an entry with that joining index.

The rest is filled in like this:

- the arguments are cleared and the number (+0x3c) is 0;
- the tree, the attenuation (by whether O is a visitor) and flag bit 0
  come from its entry, or are 0 without one;
- the listing then writes P's object ID into its third argument
  (0x4586c7);
- it is checked and offered as the other entries are (0x458705).

The check (0x458319) reads the action's **caller**, here O:

- O's visitor word, age and kind;
- O's object data word 50;
- the check tree runs on O's stack.

For an action whose word +0 is 1, the check tree's second and third
arguments are 1 and the action's third argument (0x458372). So the
offer's tree runs with 0, 1, P's ID and 0. The context saves the temps
of the first person a check tree runs on (+0x1c, 0x4584c5) and puts
them back on that person (0x4582dd). Those are O's temps when the offer's
check tree is the listing's first.

Picking an entry queues its action on the action's caller (0x48ddbe,
0x45d20b). For the offer that is O: picking it has O join P.

## What changes here

- **The offer is ported** (`orders.rs`, `join_action`). It replaces the
  loud failure.
- **The check reads the action's caller.** `check_chosen` reads it for
  the person, its data word 50 and its temps, and runs the check tree on
  its stack. The pointer's check (`highlight`) does the same. Every
  action checked before this had the checking person as its caller:
  - the pie menu's other entries;
  - an order;
  - the queue's actions, which Push Interaction, the menu and Idle For
    Input all make with the queue's person as caller.

  So nothing else changes. Picking an action without a caller queues
  nothing (0x48ddc6), and pointing at one checks nothing (0x48dfa5), as
  in the original. Any other action whose caller is not a person on the
  lot fails as invalid.
- **The check of a queued or picked action runs as a check.** Its tree
  runs at once (0x4749b5), which clears 0x6c4d6c (0x474a37). So outside
  a listing, Add/Change Action String names nothing and takes the true
  branch, as it already did in autonomy's checks. Before, a picked entry
  that runs at once, or a queued action Idle For Input checks, failed
  loudly there. The same holds for an action's tree run at once, which
  goes through the same run (0x45d2d3, 0x474afb, 0x4749b5); it failed
  loudly before this branch too.
- **Picking and pointing clear flag 0x80.** The menu's window works out
  each shown entry's name when it is built (0x48d5ce, 0x41f167), and
  that always clears the action's flag 0x80 (0x41f1a4). The listing
  doesn't, so the offer keeps the running action's 0x80 there.
  `Interpreter::pick` and `Interpreter::highlight` clear it as the
  window would have.
- **The saved temps say whose they are** (`SavedTemps`), and go back to
  that person.
- **Picking queues on the caller** (`Interpreter::pick`).
- **Word +0 is kept: `Interaction::join`.**
  - The game saves the word first (0x41f1d8). The reader took it for a
    header that had to be 0, so a saved game with a queued join would
    not have loaded.
  - A picked offer keeps the word while it is queued and while it runs.
    Idle For Input's check of it reads the word again and passes the
    join's arguments.
  - The world's encoding folds the word into the high half of the flags,
    written as one 64-bit integer. Postcard writes an integer by its
    value whatever its width, so an action that does not join encodes
    exactly as before.
  - So every pinned hash stands and the snapshot format stays 30. A
    format-30 snapshot reads the same. Only a world holding a join
    differs.
  - **Why the format isn't bumped.** Nothing needs telling apart: every
    snapshot written before reads the same. An older build reading a
    world that holds a join gets a flags value too wide for its 32-bit
    field, so it fails to read the snapshot rather than misreading it.
    (The number is in the snapshot's header, which the world's hash does
    not cover, so a bump would not move the pinned hashes either.)
- **The word is kept as it is.** The check passes the join's arguments
  for exactly 1, as the original does (0x458372). The menu's other
  listing (mode 2 or 3, 0x48d4b7) makes actions with 2 there (0x41ee2f);
  that listing is not ported.
- **Autonomy's join entry** (0x458aee) is unchanged. Its third argument
  is 0: 0x41ee99 clears it and the scan writes none.

## Not verified

- No recording shows a join offer picked, so picking it is read from the
  code only.
- The offer's name: the original copies the running action's name and
  then gives it the entry's stored name (0x41efc1). A `MenuEntry`
  without a name shows its entry's name, which is the same. The stored
  name is the TTAs label when its first byte is not 0 (0x475bf6), else a
  name made from the entry's tree (0x475c77), so the offer carries one
  whenever the TTAs strings load, and the window's getter, which adds
  "*" for a debug entry only to an empty name (0x41f174), adds none. The
  game's menu follows that, unless a check tree named the offer, even
  with an empty name (0x4587d0). It shows the made name as the menu
  number, as it does for any entry without a label.
- Other readers of word +0 were not searched for.
