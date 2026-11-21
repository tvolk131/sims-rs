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

  So nothing else changes. An action whose caller is not a person on the
  lot fails as invalid. The original would read through a null pointer.
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
    differs, and an older build would refuse it.
- **The check reads exactly 1 at word +0, as the original does.** No
  other action has 1 there.
- **Autonomy's join entry** (0x458aee) is unchanged. Its third argument
  is 0: 0x41ee99 clears it and the scan writes none.

## Not verified

- No recording shows a join offer picked, so picking it is read from the
  code only.
- The offer's name: the original copies the running action's name and
  then gives it the entry's (0x41efc1). A `MenuEntry` without a name
  shows its entry's name, which is the same.
- Other readers of word +0 were not searched for.
