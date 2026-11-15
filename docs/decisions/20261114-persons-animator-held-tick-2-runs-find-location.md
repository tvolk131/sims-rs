# A person's animator is held, and tick 2 runs to Find Location For

*Decided 2026-11-14.*

Each person now has an animator in the simulation state. Animate Sim
and the animator's tick are ported as far as the Goth house reaches
them. Tick 2 gets past Animate Sim on object 16 and Stop All Sounds, and
stops at Find Location For (primitive 16) on object 364. K stays 1.

Some terms. The *animator* is the object at person +0xadc (vtable
0x671ee4) that plays a person's animations. A *clip* is one named
animation from the game's animation files. A *layer* is one clip
playing on the person's skin, with its own clock. The animator has
three *slots* for layers: the *foreground* (+0xc), the clip a script
plays; the *base* (+0x14), the clip of the person's posture; and the
*carry* (+0x13c), the clip of something carried. An *event* is a number
a clip sends to the script that plays it. *Presentation* is what only
the picture or the sound uses.

What was decided:

- **What is simulation state.** `World::animators`
  (`sims_sim::animator`) keeps, for each person whose animator has
  ticked or played: the animator's clock, its speed and the speed it
  moves towards, its state, the result of its last route, its position,
  direction and destination, the head's clock, the posture and carry
  bookkeeping, whether the base slot holds a layer, Animate Sim's
  progress through a clip's events, the event queue, and the foreground
  layer's clock, speed, phase, duration and event times. Scripts read
  these through Animate Sim (whether a clip is done, which event came),
  and the animator's tick reads them to decide what it writes to the
  person. Field offsets are the original's and are listed in the doc
  comments.
- **What is presentation**, and left out: the posed bones and the skin
  (0x5994c3, 0x5a15bc, 0x5a0ef5), the head's turn past the countdown
  it keeps in person words 42 and 45 (0x590768), the draw state
  (0x434101), the face (0x58faf6), where a carried object is drawn
  (0x590b27 and the tail of 0x58dc72), the base layer's own clock, and
  sound. The evidence is the original's code: none of these routines
  writes anything a script or the scheduler reads, as far as they were
  followed. The base layer's clip sends no events, so its clock decides
  nothing. That nothing the simulation reads depends on these is
  **unverified** beyond what the capture shows below.
- **The animator before the first tick** (`animator::LOADED`) is
  inferred. The capture's first look at an animator is after tick 1.
  Each value is the one tick 1 leaves alone, or the one that tick 1
  turns into what the capture holds (the doc comment gives the reason
  for each). For example, the speed starts at 1.0, because tick 1 raises
  it by at most 0.2 and the capture holds 1.2.
- **Animate Sim** (opcode 44, 0x58950b, through the person's dispatcher
  0x45cbb3): starting a clip on the foreground slot, the event queue,
  numbered events in order, an interrupt ending the clip, and the events
  a finished clip did not send. The clip's name comes from the global
  file's string table 156 (adult) or 157 (age 17 or less), and its
  duration and frame count from the animation files
  (`sims_content::Animations`).
- **The animator's tick** (0x58dc72), run from the person's tick after
  the decay: the clock (50 per tick, 42 when slowed), the carry choice,
  the base layer's bookkeeping, the speed (towards 0.2 times a base
  plus 1.2, up by at most 0.2 per tick), placing a standing animator on
  its person's tile, the foreground layer's clock (0x597c4c), and the
  head's countdown. All arithmetic is in single precision, as the
  original's.
- **Stop All Sounds** (opcode 48, 0x45253a) takes the true branch and
  changes nothing. Its only effect is a command to the sound mixer.
- **Resetting a person whose animator has ticked fails loudly.** The
  reset also resets the animator (0x589220), which is not ported. The
  resets seen so far are all at the load, before any animator ticks.
- **Snapshot format 11.** `World` gained the animators. The pinned
  states in `tests/resize.rs` and the synthetic world's digest changed.
  The tests strip the animators (`format_10_bytes`) and check that the
  rest of the encoding still gives the format-10 pins. The load's state
  has no animators. After tick 1 the rest of the state is the same as
  before, so the tick-1 pins for formats 9 and 10 still hold.

How it was checked. The private capture of the original running ticks 2
to 5 (the entries below) includes memory pages that hold the three
people's animators and their foreground layers. The scratchpad tool
`tick_verify.py` now reads them and compares each field we keep, as
bits for the floats.

Equal to the original:

- After tick 1: every animator field that the capture's pages hold, 58
  of 58 for people 16, 18 and 21. This includes the speed (1.2, bits
  0x3f99999a), the target speed (bits 0x3fb33334), the state, the
  position and the clock. 23 field readings fall on pages the capture
  did not save. The rest of our state after tick 1 is unchanged and
  still equals the capture, as the entry below lists.
- Where tick 2 stops (at object 364, after every lower ID has run):
  72 of 72 readable animator fields, including object 16's foreground
  layer (priority, clock, duration, speed, phase, event times, frame
  count, end mode, and whether it may be interrupted). 354 of the 355
  objects below 364 equal the capture after tick 2 in data words and
  frames, and the three people also in person data and motives. These
  are compared with the capture after the whole tick, so something later
  in the tick could still change them; the capture shows it does not.

Consistent only with the old prototype's isolated comparisons: nothing.
The old prototype's animation clock is marked approximate, so it was
not used as a guide.

Did not match:

- Object 262, which object 364 makes again just before the stop,
  differs in data word 29: ours -5, the capture's 0. Object 364 is about
  to place it with Find Location For, which is where we stop. That the
  placement writes the word is likely and **unverified**.
- Object 364's own frames, since it has not finished its turn.
- Everything else the entries below list as not matching still does
  not.

**Unverified**, with no evidence either way:

- That every motion of a clip binds to a bone of the person's skeleton.
  A clip with motions is taken to bind at least one.
- Which animation files the original's library loads, in what order,
  and whether it ignores case in a clip's name. A clip that two files
  define differently, or that only a name in another case matches,
  fails loudly.
- The base layer's clock, which is not kept.
- The other skin routine (0x5a0ffd). Either routine updates the layers
  once per tick; which one runs is taken to change only the pose.
- Removing a finished layer from the skin (+0x64), which nothing ported
  reaches.
- The head's paths when bit 0 of person word 42 is set; they fail
  loudly.

What fails loudly: Animate Sim on an object that is not a person, on
the base or carry slot, with clip 0, with a name from anywhere but the
global tables, with a missing clip or one with timed events, replacing a
foreground layer, and at the end of a clip (0x592904); the animator's
tick in any state but 0 and 3, when it has to turn or walk, when it
starts a carry or posture clip, and on a layer end mode other than 0 or
a fading layer; and resetting a person whose animator has ticked.

Found wrong in existing code or docs: removing a person does not drop
its entries in `World::person_ticks` or `World::animators`. Nothing
reads them for a removed person, and no person is removed in the
ticks run, so it is noted and left.

What tick 2 needs next: Find Location For (0x44b3c8). Mode 0 searches
for a free place near the reference object (0x4347f0, which draws
random numbers through 0x46f457), tests each place (0x432679,
0x4329c1) and puts the object there. In the capture, object 262 ends
tick 2 at (632, 344, level 1). After that, tick 3 has people 18 and 21
start a clip, and an Animate Sim clip's end (0x592904) will be needed.
