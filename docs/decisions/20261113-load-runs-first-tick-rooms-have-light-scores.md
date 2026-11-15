# The load runs to the first tick, and rooms have light and scores

*Decided 2026-11-13.*

`sims boot` now runs all 22 load steps on the Goth house. The state it
ends in is the one the original's first tick starts from. Ticks are not
implemented.

Some terms. The *selected Sim* is the one the player controls; the
original marks it with a flag on the object and writes its ID to global
3. An *indicator* is one of two objects the game keeps on every lot; the
first is the marker over the selected Sim. A room is *marked* when it
waits to be counted. A level *waits* when the lot knows that one of its
rooms is marked. The *room update* counts the marked rooms.

What was decided:

- **Step 16 clears global 3** (`Interpreter::announce_house`, 0x44302e).
  When the house is announced and no Sim is selected, the registry asks
  for the indicators and then announces that nobody is selected, which
  clears global 3. The roadmap had this as part of step 17.
- **Which Sim is selected** (`Interpreter::select_resident`, the end of
  0x4f1ae9). A selected Sim that belongs to a family stays. Otherwise it
  is the family's first member: the first object of that member's type.
  Whether it may be selected depends on the lot's zoning and number and
  on the Sim's visitor word (0x4e989b). The roadmap had asked whether it
  is the Sim the house saved in global 3 or the family's first member. It
  is the first member: global 3 is cleared before the choice, and no
  save holds the Sim's mark.
- **Selecting a Sim** (`Interpreter::select`, 0x445449): the mark
  (`Object::selected`), the indicators (0x44664c), then global 3.
- **An indicator that leaves the lot is taken off twice.** When an
  object leaves the lot the registry is told (0x447d2b), and it takes an
  indicator off once more (0x4481eb). So the indicator is asked function
  10 twice. `placement.rs` used to say that telling the registry changes
  nothing the simulation holds.
- **Step 19 is `Interpreter::go_live`**: every room is marked, the
  lot's statistics are worked out, and global 19 becomes 0. Of the
  statistics (0x416085) only globals 23 and 37 are ported.
- **Steps 21 and 22 are `Interpreter::before_frame`**: the rooms that
  wait are counted, and global 12 becomes 0.
- **The room update is ported whole** (`World::update_rooms`, 0x466c3c):
  the objects standing in each room, the room's light and score, the
  lot's outdoor score, and the rule that a room whose light changed is
  marked again. `Rooms` now holds which levels wait.
- **Step 10a is two updates.** The original counts the rooms after step
  10, then marks them all when the house read ends (signal 0xdd) and
  counts what waits. Before, one count gave the same sizes. With light
  it does not give the same marks: the first count changes every room's
  light, which marks every room again.
- **The room arithmetic rounds every operation to single precision**
  (`Ext::single` in `sims-core`). See below.
- **The snapshot format is 8.** New: each object's selected mark, each
  room's counts, light, score and pool mark, and the rooms' waiting
  levels, outdoor sum and outdoor score.

How it was checked. The private trace of the original loading the Goth
house stops at the entry and return of the funds setter, of the
selection (twice), of the mode change (twice) and of the lot's part in
it, at the entry and return of the wait check, and at the entry of the
first tick. It holds every call of the function that runs an object's
callback (0x422d46). Our run was compared with it outside the repository
(`live_verify.py` in the private scratchpad, one line for each thing
compared).

*Equal to the trace:*

- **The whole state at seven stops**: the entry of step 11 (after the
  room count), the entry of the funds setter (after step 16), the return
  of the selection (after step 18), the return of the lot's part of the
  mode change and the entry of the second selection (inside and after
  step 19), the entry of the wait check (after step 21), and the entry
  of the first tick. At each: everything the entry below lists for the
  end of step 13, and each object's selected mark.
- **The rooms at each of those stops**, all 11: the twelve counts, the
  lights and people, the score and the light bit for bit, whether the
  room is outdoors, whether it is a pool, the mark, the levels that
  wait, and the lot's outdoor sum and score bit for bit.
- **The callbacks of step 18**, in order: 383:10, 383:10, 383:9, 21:9.
  No other callback is asked between the end of step 13 and the first
  tick, there or here.
- **Steps 13 to 15** as before; the comparison of the entry below was
  run again with the room fields added.

Why the six indoor rooms stay marked after step 21, which the roadmap
could not explain: their light changed. At step 10a every indoor room
was fully lit through its windows. Step 13 reset the windows, which
cleared the light each lets in. So at step 21 the indoor rooms come out
dark, or lit by their lamps only, and a room whose light changed is
marked again. The outdoor rooms have the daylight both times.

The original's FPU and single precision. One room's score in the trace
is 3 units in the last place away from what `sims-core`'s 64-bit
register model gives. It is exactly what the sum gives when every
addition is rounded to 24 bits. The other ten rooms are equal either
way. So the FPU's precision control is set to single precision when the
room update runs, as Direct3D sets it. The room code now rounds each
operation that way, and all 11 scores, both times, are equal bit for
bit. Motive values compared earlier were equal under the 64-bit model.
The sums that made them may not tell the two apart. That has to be
looked at again when motives change in a tick.

The second 383:10. The trace shows function 10 asked of the indicator
twice, and the code that moves the indicators asks once. To find the
second caller, the original's selection code (0x445449) was run in an
emulator on the memory the trace holds at its entry. It ran through both
calls and showed the chain: remove from lot (0x433125), told to the
registry (0x447d2b), which removes an indicator again (0x4481eb). The
emulation is weaker evidence than the trace, and only the caller rests
on it. The order of the four callbacks is equal to the trace.

*Consistent with the old prototype's isolated comparisons only:*

- The room update on a lot with a pool, and an object's share when it
  is hidden, has a negative room word, or is one of the two categories.
  The prototype compared the score arithmetic and the object visit with
  the original's code run alone. The Goth house has no pool.
- The names and defaults of the room constants. The Complete Collection
  has the constants the code defaults to.
- The zoning rules of 0x4e989b for other lots than a residential one.
  They were read from the code here; the prototype read the same.

What changed in earlier work, and why:

- **`World::order` and `World::scheduler_cursor` were documented
  wrongly.** The comments said the scheduler visits the registration
  list and that the cursor is a place in it. The scheduler's pass
  (0x46d553) goes through the registry's table of objects by ID, the
  table the ID lookup (0x40743b) reads, and the cursor is an ID. Only
  the comments changed; nothing uses either yet.
- **`World::update_rooms` takes the constants, the level and the
  objects on a tile.** `sims-vm` supplies them
  (`Interpreter::update_rooms`). The room tests in `sims-sim` count with
  no objects.
- **The gated tests pin new digests**, because the encoding changed.
  Each state that was pinned under format 7 is also encoded without the
  new fields (`format_7_bytes` in the test file), and that encoding
  still has the digest pinned before. So the states are the same ones.
  The chain back to formats 6, 5 and 4 now starts from that encoding.
- **The gated test of step 13 no longer says steps 14 to 16 change
  nothing.** Step 16 clears global 3. A new gated test takes the load
  from there to the first tick.
- **The roadmap's step 17 was "clear the selected Sim and set the funds
  again".** The clearing is step 16's. Step 17 is the funds.

What is **unverified**:

- **Selection on any lot but this one**: a family whose first member is
  not on the lot or may not be selected, a visitor, a community lot.
- **Three things the interface holds**, taken as the trace shows them:
  every family member is on the list of portraits; the switch that lets
  any Sim be selected (0x6e4f64) is off; a person's slot 1 takes an
  indicator. The trace's memory shows that slot flag on the two Goths
  whose slots it holds.
- **The order of the original's list of people**, which decides who is
  selected if two people have the mark. The registration order is used.
- **The statistics pass beyond globals 23 and 37**: global 22, the
  value of the lot's objects and architecture, two flags of the family's
  record. They are left as they are. In the trace they do not change,
  but they already hold what the house saved.
- **The bits of global 37 other than bit 0.** The Goth lot sets only
  that one. The others are read from the code.
- **The daylight's multiplier** (0x6f015c), which is 1.0 in the trace's
  memory and is taken to be 1.0 always.
- **The level the first room update of step 10a counts.** It is taken
  to be every level. The second update counts every level whatever the
  first did.
- **The time of day other than day**, and the lot's outdoor score at
  night, dawn and dusk. The trace is by day.
- **Single precision outside the room update.** See above.

What is not modeled: the indicator's and the Sim's graphics during the
selection; the star markers of the Superstar pack, which the original
handles only for a Sim that is not selected; the lights the room update
places in a room and the lighting of the tiles; the rooms' cached route
rectangles; what a person is told when the mode changes; the count of
the family's friends; and a field of the family's record that the end
of the load clears.

What fails loudly: deselecting a Sim; selecting a Sim with a date, or
one whose slot 1 holds something, or one of several tiles; a family
member's type that two definitions name; a star marker leaving the lot;
an indoor room that is dark, has no lights and has people in it, where
the light it shows depends on the room knowing it is occupied
(0x468802), which is not ported; and `sims boot --ticks` with a number
above 0.

About the first tick. It is not implemented, because one tick could not
be both built and checked in this piece of work. The roadmap now says
what a tick is in the original (0x46d345 and 0x46d553, read from the
code), what the first one needs, and what there is to check it against.
The main finding: an earlier capture of another run of the original, at
tick 88,445, differs from our baseline only in what one tick would
change, in every object field compared, and has the same two game
random streams. So the first five ticks can be checked against it.

What would reopen this: a house with a pool, or loaded at night; a lot
whose people stand indoors in the dark; a house where the selection is
not the family's first member; a trace that shows the statistics pass
writing a value; any float result that single precision does not give.
