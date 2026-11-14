# Decisions

Choices made while working toward `docs/roadmap.md`, newest first. Each entry
says what was decided, why, and what would reopen it. Entries marked
**unverified** wait on the comparison harness.

## 2026-11-14: Scripted input reaches the game through xdotool, and is confirmed in memory

The harness can now drive the original from a scenario file while it
captures (`capture.py --scenario`, `tools/harness/input.py`). This is how
captures with player input are made, for our simulation to replay.

What was decided:

- **Input goes through xdotool in the guest, not QEMU.** QEMU's input
  devices do not reach the game: the guest's X server is Xvfb, which reads
  no device. A small guest job (`guest/input-agent.sh`) runs the commands
  the host drops in the share, polling every 50 ms; the generic job runner
  polls once a second. Guest memory is never written.
- **What was clicked is read from the pie menu, not trusted.** The
  picker tool's hovered object (picker `+0x2e`) lags the pointer by tens of
  lot updates under TCG, because the game takes pointer moves late. Once it
  named the bench while the pointer was over the porch floor, and the click
  opened "Go Here". So the hover reading only says when to press, and the
  pie menu's actions say what was clicked: a menu whose actions aim at
  another object is closed with Escape and the next aim point tried.
- **A chosen interaction is confirmed twice.** capture.py stops at
  `0x48ddcb`, the call in the pie menu's select handler (`0x48dd67`) that
  pushes the action on the person (`0x45d20b`, edi the person, esi the
  action), and logs it with the update it falls before or in. Then
  `expect-queued` waits for the person to hold that action, queued
  (`+0x820`) or running (`+0xa48`).
- **Dialogs are answered on the dialog stops.** A modal dialog holds the
  lot update it shows in, so a step that waits for update returns would
  never see it. `answer-dialog` presses once, as `answer.sh` does, and
  waits for the dialog's result stop.

**Checked against the original.** In a capture from a fresh boot
(2026-11-14), the scenario answered the five welcome notices and the Help
System dialog, aimed at bench 173 by projecting its position (hover
confirmed on the second aim point), opened a pie menu with the one entry
Sit (caller 21, target 173, routine 8213, priority 50), and chose it. The
push came before lot update 178. At the return of update 180 the person's
queue was already empty, and the full state after update 200 holds Sit on
173 as its active action, with uid 1. (That run's check read only the
queue; reading the running action too was added after it.) A second
scenario on the same running lot hit the bench at the same aim point, got
the same menu, recorded the push before update 168, and found Sit on 173
in person 21's queue (uid 7) at the return of update 168, behind its
running action. Screenshots showed the bench in the
person's queue. The memory layout used (picker, pie menu, people panel,
house view, queue) is listed in `input.py`; each part was read in the
running game.

**Unverified.**

- The projection was checked only in the starting view (zoom 1, no
  rotation), and it ignores the floor's height: the porch, on a
  foundation, is hit about 30 pixels above its projected point, which the
  aim points cover.
- Hovering is not free of side effects: `0x48c834` sets and clears
  highlight bits in the hovered object's flags (`+0x114`, through
  `0x4320da`). Whether those bits reach anything our simulation compares
  is not checked. A replay may need them, or a check that they are only
  drawn.
- Submenus, build and buy mode, and a missed dialog press (the step then
  waits for the dialog to be answered by hand) are not handled.
- Dialog button places are written in the scenario, not read from the
  dialog.
## 2026-11-14: The HIT player runs real sound events into PCM (track 3, sound)

`sims_audio::player::Player` is the HIT virtual machine, with a mixer.

- It takes `SoundEvent`s: play the event named N for object O, stop
  object O's sounds, or stop everything.
- It renders mono 16-bit PCM at 22,050 Hz, the rate of nearly every
  sample in the install.
- Each event starts a thread that runs its track's HIT routine. Threads
  run at the start of every 64-frame block; notes are mixed in between.
- Random choices (hit lists, `rand`) use the player's own seeded
  generator, so a seed always replays the same sound.

What was checked:

- **Phone ring.** `Phone_ring` renders exactly the decoded
  `Phone_ring.xa`, bit for bit, then silence to the end of the block.
- **Footstep.** `footstep_soft` with `SimSpeed` at 1 renders exactly one
  of the five soft footstep samples. Eight seeds pick soft1, soft3 and
  soft4. At `SimSpeed` 2 it renders nothing, as the routine says.
- Both digests are pinned in a gated test.
- **Every event.** All 4,494 events in the install were run for 20
  blocks with stub samples, and no instruction failed. Two kinds of
  fault remain:
  - The files' own loose ends (14 faults).
  - 190 events of kinds this player does not handle yet: music modes,
    television, sequenced hit lists and others.
- Synthetic tests cover calls, branches, waits, loops, hit lists,
  resampling, clamping, questions to the simulation, and faults.

How the instructions behave comes from the HSM register names, public
notes (FreeSO, the Niotso wiki) and what the install's programs need:

- **`end` returns from a call.** The install has 2,504 calls and no
  `return`.
- **`loop` without `set_loop` goes back to the routine's start.** The
  looping generic routine relies on this.
- **`loop` waits for the next block.** Otherwise a loop that starts no
  note would spin forever.
- **`arg1` holds the object** when a track's arguments are
  `kArgsIdVolPan` (4,135 of 4,316 tracks). The routines pass `arg1` to
  `getsrcdatafield`, the one way a script asks the simulation something.

All of this is **unverified** against the original.

Left out, also **unverified**:

- Volume, pan, pitch, ducking, priority and 3D position: every note
  plays at full volume, centred, at its own speed.
- The `seqgroup_*` instructions and `test1` to `test4` do nothing.
- `setll`, `setlt` and `settl` copy like `set`.
- When modules define the same event, patch or hit list, the module
  whose name sorts first wins.
- Which value of `SimSpeed` the game uses at normal speed.

The simulation does not emit these events yet, and `sims-vm` and
`sims-sim` were not changed. What it would need:

1. **Play Sound Event (primitive 23).** It names a sound by an FWAV
   string ID. The event name is that string: looked up in the code
   owner's file, then in `Global.iff`. The simulation sends
   `SoundEvent::Play { object, name }` with the object's ID, plus the
   primitive's flags (loop, volume) once the player uses them.
2. **Stop All Sounds (primitive 48).** It sends
   `SoundEvent::StopObject` for the object.
3. **Animation clips.** Their `sound` and `footstep` events send
   `Play` too. The footstep event names are the HOT names, such as
   `footstep_soft`.
4. **`getsrcdatafield`.** It needs a read-only view of the object's data
   fields, through `sims_audio::player::Sources`. What each field
   number means is unknown. The FM routines suggest field 0 picks
   between male, female and child voices.

## 2026-11-14: The sound tables read, and an event leads to its samples (track 3, sound)

`sims-audio` now reads the sound scripts' files. A *module* is a HIT
program (bytecode) with its HSM file (symbol names) and HOT file
(tables). The install has 27 modules: two in the base game, one per
expansion pack, and one per downloaded object that has its own sounds.
The HOT tables map event names such as `Phone_ring` to tracks, tracks to
HIT entry points and patches (sample files), and list the hit lists
that scripts pick samples from. FSC files are the outdoor ambient
scores, and all four parse.

How the HIT bytecode is laid out:

- Opcode names and numbers come from the HSM files. Operand layouts come
  from public notes (the Niotso wiki, FreeSO's HIT player).
- The install confirms them for every opcode it uses. All 27 programs
  decode from their first byte to their last, and every one of the 4,363
  HOT entry points lands on an instruction.
- Branch, call and jump operands are not addresses. They are track data
  IDs that the HOT file maps to addresses; every such operand in the
  install resolves within its own module.
- `jump` has two forms: a 4-byte ID, or a 1-byte register holding the
  target. The opcode does not say which. We take the ID form when the 4
  bytes are an ID of the program. That covers 399 of the 401 jumps; the
  other two only decode as register jumps. **Unverified** how the
  original decides. Opcodes the install never uses keep the public
  layouts, also **unverified**.

What the files themselves leave dangling:

- 155 patches name sample files the install lacks. One path names a
  build machine's `c:\` drive.
- 14 events give an empty track.
- 10 track entry points use a symbol that their own module's HSM file
  does not define (`tkd_Generic`, `tkd_GenericInterrupt`). Other modules
  do define them, so the original may look across modules.
  **Unverified.**
- 4 tracks name a patch that does not exist.
- 6 hit list items are neither a number nor a range: four names and two
  dotted pairs such as `2370.2376`. These are kept apart, unread.
- The FSC scores mostly use samples in their own directory. The two big
  scores also use 56 samples from the other ambient directories, and
  six samples exist nowhere. Where the original looks for them is
  **unverified**.

The gated test pins all of these counts, so any change to the rules
shows up in it.

Traced by hand, then pinned in a test:

- **Phone ring.** `Phone_ring` plays track 3479. That track runs the
  generic routine (`note_on v1; wait_samp; end`) with the patch
  `sounddata\phone\Phone_ring.xa`.
- **Footstep.** `footstep_soft` runs a routine that sets `interrupt`,
  compares 2 with the game speed (global register 100), and branches to
  code that picks one of five soft footstep samples (hit list 217).

When two modules define the same event, the module whose name sorts
first wins (both events above are in the base game's generated module).
The original's order is **unverified**.

## 2026-11-14: Sound files decode (track 3, sound)

A new crate, `sims-audio`, reads the game's sound files and decodes them
to PCM (16-bit signed samples). The simulation does not depend on it.
Every sound file in the Complete Collection install decodes.

Terms used here. *XA* is Maxis's 4-bit ADPCM (adaptive differential PCM)
sound format. A *FAR* archive is the game's file bundle.

What was decided:

- **Three formats, no others.** The install holds 18,155 XA files, 285
  WAV files (all 16-bit PCM at 22,050 Hz) and 141 MP3 files (the radio
  stations, MPEG-1 Layer III at 44,100 Hz, stereo). No file is UTK (The
  Sims Online's codec) and no file starts with its magic number. Outside
  those extensions, only the animation `.cfp` files start with bytes that
  look like an MPEG frame header, by chance.
- **XA is decoded from public notes.** The layout follows FFmpeg's Maxis
  XA demuxer and decoder and the Niotso wiki: a 24-byte header, then
  frames of 28 samples per channel in `15 * channels` bytes. The header's
  bytes 2 and 3 hold 0, `J` or 0x12 and are not read. One stereo file
  (`sting_crystalball.xa`) ends half way through a frame and needs that
  half's samples to reach its stated size, so a partial last frame
  decodes as far as it goes. Output is cut to the size in the header.
- **MP3 uses the `symphonia` decoder** (pure Rust, MPL-2.0). Its output
  is floating point converted to 16-bit, so its digest is pinned on one
  machine only. Music is presentation; nothing in the simulation reads it.
- **Sound files are found by install-relative path.** The sound tables
  name files such as `sounddata\phone\Phone_ring.xa`; that file is the
  member `Phone\Phone_Ring.XA` of `SoundData/SoundData.far`. An
  archive's members count as files of the archive's directory, paths
  ignore case, a loose file replaces a member, and the first of two
  members with one path is kept, as in `sims_content::Index`.
  **Unverified** that the sound manager resolves paths this way.

How it was checked:

- Synthetic tests: hand-computed XA samples (each predictor, clamping,
  stereo order, a partial frame), an XA encoder written apart from the
  decoder whose output decodes to exactly what it predicted, WAV chunk
  walking, and silent MP3 frames.
- A gated test (`SIMS_INSTALL`) decodes every sound file, shadowed
  duplicates included, and pins counts per format and variant, total
  frames, and an FNV-1a digest per format. It takes about 40 seconds in a
  debug build. An ignored test decodes all eight hours of MP3 in full;
  run once in a release build (24 seconds), every MP3 file decoded and
  gave as many frames as its frame headers count.
- **Against FFmpeg.** FFmpeg's `adpcm_ea_maxis_xa` and PCM decoders
  (FFmpeg through PyAV 15.1.0, in a private virtual environment) decoded
  all 17,915 distinct XA files and 285 WAV files. After cutting to the
  size each XA header states, every file's PCM equals ours, bit for bit.
  FFmpeg does not cut: for 17,129 files it returns the rest of the last
  frame too.

Not checked against the original. The game's own XA decoder was not
traced in the executable, and the harness runs with `-nosound`.

## 2026-11-14: Drawing the lot starts in its own crate, on the CPU

The lot is drawn by a new crate, `sims-render`. It reads the world
(`sims_sim::World`) and never changes it, and nothing in the simulation
depends on it. It draws on the CPU, so tests need no window or GPU.

Terms used here. A *sprite* is a pre-drawn picture from the game's files.
An object's sprites (SPR2) carry a *depth byte* per pixel that says how
far that pixel is from the camera. A *draw group* (DGRP) lists the sprites
that make up one view of an object. A wall's *shape* is a mask that says
which pixels of a wall exist (full wall, cut-down wall, fence); its
*pattern* is the wallpaper. *Cutaway* is the original's middle wall mode,
in which walls that would hide a room's inside are cut down.

What was decided:

- **The original's projection.** A tile's top corner is at
  `((x - y) * (8 << zoom), (x + y) * (4 << zoom))` plus the camera's
  origin, raised 58, 116 or 232 pixels per storey at zooms 1 to 3 and
  `2 << zoom` pixels per terrain step. The four rotations use the
  original's 64-tile rotation table. This is the old prototype's reading
  of the original's code (`legacy/crates/sims-sim/src/lot_presenter/`).
- **Depth is a number per pixel, smaller nearer, and a tie goes to the
  later draw.** Its scale was fitted from the game's sprites, not read
  from the original's code. For the lowest pixel of each sprite column,
  which mostly touches the floor, the depth byte falls 1.5 per screen
  pixel below the object's anchor at the near zoom, 3 at medium and 6
  at far, and is about 200 level with the anchor. So one byte is a fixed
  distance in the world. The renderer gives floors, walls and the
  anchors of objects a depth on the same scale. Whether this matches the
  original's depth tables is **unverified**.
- **Where a sprite goes.** An object's anchor is the screen point of its
  position on the floor. Each sprite of its draw group is drawn with its
  left edge and bottom at the offsets the draw group gives (checked on
  every near-zoom draw group of the Goth house's objects: the anchor sits
  at the same point of each sprite's frame). A draw group sprite with
  flag 1 is mirrored. Dynamic sprites hide when their word is zero
  (0x437f99, from the old prototype).
- **Floors** are `GameData/floors.iff` sprites `id`, `id + 256` and
  `id + 512` for the three zooms, drawn with their middle column on the
  tile's top corner. A tile a diagonal wall splits shows two floors from
  its wall cell; which half takes which is **unverified**. Grass is a
  brown-to-green mix by each tile's grass value, scattered in two
  shades. The original draws grass blades from a seeded pattern, so
  grass will not match pixel for pixel.
- **Walls** are a pattern from `GameData/walls.iff` (`id` plus 1536,
  1792 or 2048) masked by a shape (`style * 2 + cut` plus 0, 512 or
  1024), one frame per wall direction. Each wall cell owns its low-x and
  low-y walls and its diagonals; which side's wallpaper shows follows
  the old prototype's `faces`, which was compared with the original's
  wall records. A wall of style 22 or 23 takes its shape from the door
  or window standing in it (the object's OBJD byte 0xa2), as the old
  prototype did. Where a wall sprite sits on its edge was worked out
  from the shapes' outlines and is **unverified** until compared with a
  screenshot.
- **Cutaway is simplified.** A wall stays up only if the room on the
  camera's side is indoors and the other side is outdoors: the back
  walls of the house. Doors and windows in a cut wall are hidden. The
  original's cutaway (0x54059a and 0x4aa446) also depends on a mask it
  builds per view; that is not ported, so this is **unverified**.
- **Not drawn yet:** the Sims (the people track draws them), roofs,
  shadows, lighting by time of day and room, objects in other objects'
  slots raised to the slot's height, and terrain shading.
- **A small loader** (`open_house`) builds a world from a house file,
  widens its floor and wall arrays as the load does and works out the
  rooms, so a house can be drawn without a full `sims boot`.

How it was checked:

- The Goth house renders at all three zooms and four rotations, with
  every wall shape found. The renders were looked at: floors, walls,
  fences, doors, windows and furniture sit where they should, and the
  back walls stay up in cutaway. They are pinned by digest of the
  16-bit-color image (`tests/goth_house.rs`, gated on `SIMS_INSTALL`).
- Nothing has been compared with a screenshot of the original yet.

What would reopen it: a screenshot comparison that disagrees, or a
reading of the original's depth tables or cutaway mask.

## 2026-11-14: People play the simulation's clips; placement and facing

A person is drawn from the simulation's animator (`sims_sim::animator`),
read only:

- **Clips.** Every animation file (a `.bcf` that starts with eight zero
  bytes) is read once, and its clips are known by name, ignoring case. A
  clip's samples come from `<skill>.cfp`. A clip is posed at a phase from
  0 to 1: the frame is `phase × frames`, blended with the next frame,
  which wraps to the first only for a layer that repeats (end mode 1).
  This is the old prototype's sampling (`legacy/crates/sims-sim/src/pose/sampling.rs`)
  in ordinary `f32`. It is **unverified** against the original's frames.
- **Layers.** The rest pose, then the base layer's clip, then the
  foreground layer at its phase and weight. The simulation keeps only
  whether a base layer exists, not its clip or phase. Until it exposes
  them, the clip comes from the saved person record (the base descriptor,
  for the Goths `a2o-standing-loop` and `c2o-standing-loop`) and the phase
  from the animator's clock. Carry and blend layers are not drawn yet.
- **Root travel.** The walking clip's root sways forward and back by
  about 0.27 units, and returns; it does not travel. The simulation moves
  the person. So the clip's root is kept as it is, unlike the old
  prototype, which removed it.
- **Place.** Sim x and y (sixteenths of a tile) become model X and Z, at
  3 model units per tile. That figure comes from the walk step, which
  moves a person by a clip's distance times 16/3 sixteenths. Floors above
  the ground are not handled yet.
- **Facing.** Checked against the simulation, not the original: person
  259 walking for 82 ticks of `sims boot` on House05 moves along
  (sin θ, −cos θ) in sim x and y, where θ is its direction (in eighths of
  a turn) times 45 degrees. The mean difference is 0.2 degrees. Every
  clip leaves the toes pointing at model +Z, so a person is turned by
  π − θ about the vertical, as the old prototype did. The turn is computed
  with additions, multiplications and square roots only, so pinned renders
  hold on every machine.

Whether the drawing is the original's image or its mirror image is still
**unverified**, and so is which screen direction sim +x takes. Both wait
on a screenshot of the original.

## 2026-11-14: Drawing people starts in its own crate, on the CPU

The Sims are drawn by a new crate, `sims-people`. It reads simulation
state and never changes it, and nothing in the simulation depends on it.
Its math is ordinary `f32`, and it may use the platform's `sin` and `cos`
(it allows the workspace's float lint at its crate root).

Terms used here. An *appearance* is a named set of meshes bound to bones
(`b004fafit_01`, Bella Goth's body). A *texture string* is what a saved
person keeps next to each appearance, such as `BODY=b004fafitmed_fatale01`.

What was decided:

- **Files are found by their own name.** Every `.bcf`, `.cmx`, `.bmf`,
  `.skn`, `.bmp` and `.cfp` file, loose or in a FAR archive, is known by
  its name without directory, ignoring case. The base game is searched
  first, then the expansions in the order the original mounts their
  object folders, then `Downloads`. The original's own avatar registry
  is not ported, so this order is **unverified**. Every file the Goth
  family is drawn from has one source or identical copies, so the order
  does not change what they look like.
- **What a person wears comes from its saved record.** The body, the
  two hands, the head and the accessories each name an appearance and a
  texture string. An appearance is read from `<name>.cmx.bcf`, or from
  the text form `<name>.cmx`. A mesh is read from `<name>.bmf`, or from
  `<name>.skn`.
- **A texture string `KEY=texture` gives its texture to every mesh whose
  name contains `KEY`.** Other meshes keep the texture their own file
  names (Cassandra's glasses, `nerdspecs`). This is the old prototype's
  reading of the original's code (0x599e87). It explains every part of
  the three Goths.
- **The pose is computed in a mirrored frame.** Positions and rotations
  from the files have their first coordinate negated, as the old
  prototype's renderer did. Whether this gives the original's image or
  its mirror image is **unverified** until it is compared with a
  screenshot of the original.
- **Skinning follows the old prototype:** each vertex moves with its
  bone, and a blended vertex moves part of the way towards its position
  on a second bone. How close this is to the original's blend is
  **unverified**.
- **Rendering is a small software rasterizer for now.** Tests draw
  offscreen with no window and no GPU. The camera keeps sines and
  cosines, and the house view is built from square roots, so a render
  gives the same bits on every machine and can be pinned by digest.
- **The texture decoder (`.bmp`) lives in `sims-people`**, not in
  `sims-formats`, to avoid conflicts with the rendering track. It can
  move later.

How it was checked:

- The three Goths (Mortimer, Bella and Cassandra) load with the meshes
  and textures listed in `tests/goth_family.rs`, and draw in their rest
  pose. The renders were looked at: clothes, skin, hands and heads sit
  where they should, and Cassandra wears her glasses. They are pinned by
  digest, gated on `SIMS_INSTALL`.
- Nothing is compared with the original yet.

## 2026-11-14: The end of a path on a timer, the walk's strides, and K = 333

In tick 262 of the Goth house, person 21 comes to the end of a path
with object data word 9 clear. That case is now ported, with what it
needs: the walk's strides, worked out by posing the person's skeleton.
Ticks 262 to 333 then reach a few more mechanics, which are ported too.
Our state after every one of ticks 1 to 333 equals the fourth capture.
Tick 334 stops loudly. K goes from 261 to 333.

Terms used here. A *stride* is how far apart a person's feet are, along
one axis, at a footfall of the walk clip; the animator keeps two
(+0x17c and +0x178). A *footfall* is a phase of the walk clip marked by
an `anchor` event. A *pose* is where every bone of a skeleton is when a
clip is played at a phase. A *sample* is a bone's position or rotation
at one frame of a clip, stored compressed in a CFP file.

What was decided:

- **The end of a path with data word 9 clear is ported** (0x58e8be to
  0x58eb03). The phase of the walk clip picks the next footfall and the
  stride that ends there. Within that stride of the end, the animator
  sets a timer (+0x1c0) and fades the clip's weight to the share of the
  stride left (0x597a3e). A clip that may not be interrupted and has not
  finished only sets the near-end flag. When the timer runs out, the
  person stands on the end point, facing along the path's last segment.
- **A layer's fade is ported** (0x597e3c to 0x597e9b). The old test that
  expected a fading layer to fail now checks the fade instead.
- **The strides are worked out when needed, not kept.** The original
  works them out once, in the person's constructor (0x58831d), and keeps
  them. We work them out from the same walk clip and skeleton each time
  (`Interpreter::strides`). The result is the same while the clip and
  skeleton stay the same, and the snapshot does not grow. The pose code
  is new (`sims_sim::pose`). It follows the old prototype's 24-bit
  variants (`Selection::at_position` and `motion_with_control` in
  legacy/crates/sims-sim/src/pose/sampling.rs;
  `blend_quaternion_with_control` and `compose_bone_with_control` in
  legacy/crates/sims-sim/src/pose.rs). Content now reads each
  skeleton's bones and each clip's channels and raw samples.
- **The sample decoder uses the original's table.** The original turns
  each compressed sample byte into a change through a table of 253
  floats that it builds at 0x5975d4 (stored at 0x6f7188). Each entry is
  worked out in steps, each rounded to single precision. Our decoder
  used a closed formula in double precision, which differed in the last
  bits of most entries. That moved the strides by 70 to 400 units in the
  last place. `sample_deltas` now builds the table as the original does.
- **The turn on the spot takes every turn clip** (0x58f4ee to 0x58f688),
  not only the half turn. A clip already in the foreground is cleared
  first (0x592878).
- **Animate Sim names a clip from an object's own tables** (source 0,
  0x59dd6d): string tables 129 to 132, by the ages of the person and of
  the person its interaction is with.
- **Look Towards mode 0 is ported** (primitive 22, 0x45a98e, 0x58a035):
  person data words 41 to 45 hold the head's target. Modes 1 to 3 fail
  loudly.
- **Snapshot format 17** keeps the motion's timer (+0x1c0) and each
  layer's fade (+0x48 to +0x54). The pins taken under format 16 still
  hold with these left out (`format_16_bytes`).
- **The harness reads a stale spatial-order cache without failing.** The
  fourth capture stopped at update 609 with `unknown retained
  spatial-order object identity`. The registry keeps a cache of objects
  in drawing order (registry +0x4c). The only reader (0x440349) rebuilds
  it first (0x440397) when its dirty byte (+0x44) is set or its rotation
  is not the camera's. Removing an object (0x44383c) sets the dirty byte
  (0x443b65) and leaves the cache as it was. So a dirty cache can hold
  the address of an object that is gone. The harness now keeps a stale
  cache's addresses as read, with no identities, and still fails on an
  unknown address in a current cache. Its output schema is now 2.

How it was checked:

- After every one of ticks 1 to 333, our state equals the fourth
  capture's, floats bit for bit, in the same fields as before, now with
  the timer and the fade, and the `updates.jsonl` line.
- The second capture is equal at every tick it holds (to 240). The third
  is equal to 300 but for the two known lines (global 12 after ticks 1
  and 2; one local of object 18 after ticks 210 and 220).
- Our strides equal the stored ones bit for bit, for all three people
  (child 16: 0x40d893f8 and 0x40bcad30; adults 18 and 21: 0x412c5b86
  and 0x412a0544). The capture holds the same values after updates 1,
  300 and 600.
- Our table of 253 changes equals the one in a memory image of the
  original at 0x6f7188, entry for entry. In double precision, 225
  entries differ.
- `the_goth_house_first_tick_matches_the_original` runs to tick 333,
  checks the end of the paths in ticks 262 to 322 and the strides, pins
  the state, and checks that tick 334 stops at 0x5904d2.
- The load comparison still gives its three known lines.
- New synthetic tests: the frame choice, sampling, blending and
  composing of a pose; the fade; the start of a stop on a timer; the
  table's digest; Animate Sim's own tables; and Look Towards mode 0.
- The harness change was checked only on made-up memory. No capture
  has been run with it.

What did not match, and why:

- Nothing new. At the stop in tick 334, person 16's person word 27 is
  12 in ours and 13 in the capture after the tick. The steps after the
  stop presumably write it, as at the stop in tick 262.
- The first capture still differs from tick 233 on, as before.

Unverified:

- To pose the skeleton for the strides, the layer's time since its last
  pose is taken as 0 (so the next frame is always blended in) and the
  clip as looping. Neither is read from the original. The strides this
  gives equal the stored ones.
- That a removed object was what stopped the fourth capture at update
  609. The code says a dirty cache can hold one, and the dirty byte was
  set after every update the capture saved; the memory at update 609
  was not saved.

## 2026-11-14: A person walks through a door, and K = 261

In tick 233 of the Goth house, person 21 routes to an object in another
room. Routing between rooms through doors is now ported, with what the
walk reaches next: slots searched by distance, the end of a path, and a
room noting who is in it. Our state after every one of ticks 1 to 261
equals a new fourth capture of the original, which holds the full state
after every update to 608. Tick 262 stops loudly. K goes from 232 to
261. Why the first capture chose a tick later than the others is not
known, and cannot be told from what can be recorded: this is a stop
condition for modeling it.

Terms used here. A *portal* is a door seen as a way from one room to
another. A *capture* is a run of the original under the harness that
saved its state. A *frame* is one drawing of the house view, as far as
it writes the simulation's state.

What was decided:

- **Routing between rooms is ported** (`routing/portals.rs`). People and
  doors take numbers from one pool (object data word 6, 0x46381a; reset
  at 0x4638b9). Each door holds a vector of costs (object +0x1ac), one
  column per number. A door's column holds its distance to every other
  door through rooms and doors (0x46390f). A person's column holds the
  cost of reaching the target through each door of its room (0x463c83:
  10001 for no way, 10000 for a door the route may not use, 1101 for a
  door that failed, 1001 to 1100 for one tried before, the distance
  otherwise). Step 2 of the route (0x45afd0) takes the cheapest door,
  notes the try (0x463e43) and pushes the door's tree on the person's
  thread. A second try, or a door tree that returns false, marks the
  door failed (0x463e09).
- **Slots are searched by distance** (0x483598). Each place is
  classified (0x481439): off the lot, another room, closed off by walls
  or an object (0x4359ea), uneven ground, or free. A place is turned by
  the slot's direction (0x483d4b). The last step of a route to a slot
  takes the slot's facing (0x45c516 to 0x45c5fb).
- **The planner plans indoors and upstairs** (a free rectangle costs 0
  there, 0x4824d0 to 0x48253e), and without the width limit in a small
  or crowded room. The ratio of objects to half tiles is rounded to
  single precision before it is compared with 0.2 in double precision
  (0x468103, 0x46810a), so exactly a fifth counts as crowded.
- **The end of a path is ported for a person with object data word 9
  set** (0x58e740). It walks on until it is near the end point, then
  stands. Standing with the route's end noted clears the walking flag
  (0x58f117). The route then ends (route_result 0x589d47, end_route
  0x45b6ea; a route under way does not stop, 0x589e2d). The person is
  put on its tile and turns along the last segment.
- **Rooms note who is in them** (0x468802). A person whose room changes
  tells both rooms. A dark room with no lamp then shows the light of a
  lit room (0x4688bf), so it waits to be counted again. A person's visit
  in the load notes its room too (0x45e43d, called at 0x459c22).
- **Each frame counts the rooms that wait** before it draws (0x4a8895,
  called at 0x4a8b03). So the frame schedule changes the rooms' state.
- **The default frames are `0,1,2,3,33+30`**: before tick 1, after
  ticks 1, 2 and 3, after tick 33, and after every 30th tick from there.
  The three captures that watched the draw guard all show this. The
  earlier default, a frame after every tick from 33 (`0,1,2,3,33+`),
  counted the rooms sooner. The `N+S` form is new in `sims boot
  --frames`.
- **Snapshot format 16** keeps each room's mark (Room +0x73) and the
  animator's last step (+0x294, +0x298), its near-end flag (+0x288) and
  its finishing flag (+0x28c). The pins taken under format 15 still
  hold with these left out (`format_15_bytes`). The states after ticks
  5, 13, 33 and 232 were pinned again, because the rooms now hold other
  values from the frame after tick 2. Everything the captures hold is
  the same before and after.
- **A third and a fourth capture were made.** The third holds 300
  updates (full state after 1 to 100 and every tenth, 26 MB). The fourth
  holds the full state after every update to 608 (130 MB). Both watched
  the draw guard. The harness now saves each door's portal costs as
  bit patterns (`portal_bits`). The fourth stopped at update 609: the
  harness found an object in a retained list that the registry did not
  hold (`ValueError` from `qemu_retained_owners.py`), and it refuses to
  guess. The captures stay private under the old checkout's `cache/`.

How it was checked:

- After every one of ticks 1 to 261, our state equals the fourth
  capture's, floats bit for bit: every field listed in
  `the_goth_house_first_tick_matches_the_original`, the people's
  animators (with the new fields), the route contexts and paths, all 71
  doors' portal costs, and the `updates.jsonl` line (tick count, both
  random streams, the C runtime stream, the scheduler cursor).
- The second capture is equal after every tick it holds (to 240). The
  third is equal but for the two lines below.
- `the_goth_house_first_tick_matches_the_original` runs to tick 261,
  pins the state there, and checks that tick 262 stops at 0x58e8be.
- The load comparison still gives its three known lines. Ticks 1 to 232
  still match.
- The new mechanics have synthetic tests: the pool and columns, the
  distance between doors, choosing and trying doors, closed directions,
  the turn, the slot order of refusals, the planner's two settings, the
  end of a path, and rooms noting people in a tick, a visit and a frame.

What did not match, and why:

- Global 12 after ticks 1 and 2 is 17 in the third capture (and the
  first), and 0 in ours, the second and the fourth. It is the
  interface's wait check, which we do not run.
- One local of object 18's seventh frame after ticks 210 and 220 is
  16104 in the third capture, and 0 in ours and in the fourth.
- At the stop in tick 262, person 21's person word 27 is 40 in ours and
  41 in the capture after the tick. The steps after the stop presumably
  write it.
- The first capture does not choose in tick 233 (below).

**The first capture's tick 234: not modeled, likely the harness's pause.** Its state after
tick 230 equals ours in all compared fields, and its streams are equal
to 232. After tick 240, person 21 runs tree 0x208a node 10 with no
running action: it chose a tick later. Ruled out:

- The frames. Fifteen schedules in our model all choose in tick 233.
- The dialogs. Notices can close without a press (one run showed it),
  so the presses are not the input.
- The cursor. It differs between runs, but runs with different cursors
  both chose in tick 233.
- The state. The original's code, run in an emulator from the first
  capture's own memory at update 47, chooses in tick 233.

What differs: only the first capture was stopped and taken over at
update 45 (its events: `attach resume=True updates=45`,
`took_over_at_return`), so the game stood still for a while. Object
+0x154 and +0x158 look like timestamps in milliseconds, and they differ
between runs. Find Best Interaction checks interface state before it
chooses (0x45ddc0 to 0x45dde2): the selected person (0x4e97dc),
[0x6e48bc]+0x10c, and byte 0x6f2a54 (written at 0x4ef21f). Any of these
may depend on timing. The captures do not hold what would tell, and
timing cannot be recorded as an input with this harness. So it is not
modeled; we follow the other three captures, which agree with each
other and with ours. This is not a roadmap stop condition: the original
is observable, and three runs that were not paused for long behave the
same. The likely cause is the long pause at update 45, which a normal
run does not have; that is **unverified**. An experiment that pauses a
run on purpose would test it. If the original's choices turn out to
depend on wall-clock time in normal play too, a fixed time input will
be needed, as for the frames.

Found wrong along the way:

- `draw_frame` did not count the waiting rooms, though the doc in
  `rooms.rs` said frames did. Without the count, motive 13 of a person
  differed from the fourth capture after tick 260.
- The planner's doc said "at most a fifth"; the single-precision
  rounding makes exactly a fifth too many.
- The slot's three proximity words were named minimum, optimal and
  maximum. The search (0x483598) reads them as the least, greatest and
  best distance, in that order.
- The walking flag was not cleared when a person stood at the end of a
  route, and a visit did not note its room.
- Route context word +0x68 had no meaning in our docs. It holds the
  door the route goes through.
- A doc comment about cells in `placement/find.rs` sat on the wrong
  function.
- The last entry's claim that a frame after every tick from 33 gives
  the same state after tick 232 no longer holds for the rooms (it still
  holds for what the captures hold). Its default `0,1,2,3,33+` is
  replaced.

**Unverified:** that route context word +0x74, which the original leaves
as the heap held it, is never read; which of the candidates above made
the first capture choose later; that nothing outside the compared fields
depends on the frames within updates 1 and 2; that the draw's other
writes do not matter; that the steps after the 262 stop write person
word 27.

## 2026-11-14: A person chooses an interaction by itself, and K = 232

In tick 233 of the Goth house, person 21 chooses what to do by itself.
Find Best Interaction (primitive 3) and Gosub Found Action (primitive 30)
are now ported, and so is the first part of Go To Relative Position
(primitive 27). The tick then stops loudly, because the person's route
goes through a door, and routing between rooms is not ported. A second
capture of the original holds the full state after tick 232, and ours
equals it. K goes from 230 to 232.

What was decided:

- **Find Best Interaction is ported** (0x45dd39, `autonomy.rs`). It
  scores each allowed interaction table entry against the person's
  motives, with the person's score curves, in single precision. It sorts
  the candidates with the C runtime's `qsort` (0x6356ba), because the
  order of equal candidates decides the choice. Then it draws once from
  the lot's random stream, unless the best entry or the person skips the
  draw. Its constants come from the global file's tuning table 2, with
  the original's defaults (`Program::autonomy_constants`). The check
  trees it runs can read and change their entry's advertisements (memory
  scopes 12, 28 and 29). What the house does not reach fails loudly:
  free will off for a pet or a visitor, a lot that is not zoned or is a
  community lot, joining another person's interaction, and a visitor's
  entry that asks the relationship.
- **Three primitives that the check trees use are ported**: Alter Budget
  (25), only its test; Test Sim Interacting With (37); and Add/Change
  Action String (50), only inside a check tree, where it does nothing.
- **Gosub Found Action is ported** (0x45a859, `found_action.rs`). It
  pushes the chosen entry's action tree on the person's thread, and the
  person starts using the object. The action becomes the person's
  running action (0x45e6cd, `Machine::set_action`). A new action with a
  target and no number takes the next number from a counter in the
  running game (0x6e4904). No save holds that counter, so it is kept in
  `World::action_serial` and starts at 0. Snapshot format 15.
- **Go To Relative Position builds its slot** (0x45c634,
  `relative_slot`) and then takes the same route steps as Go To Routing
  Slot. A target in another room fails loudly at 0x45b140: the route
  then goes through a door that 0x45c7c2 and 0x46390f choose, and that
  is not ported. A smaller exact K is better than a larger one that is
  not exact.
- **A second capture was made**, to see tick 233. It holds 240 lot
  updates, the full state after updates 1 to 5, 30 to 35 and 225 to 240,
  and a memory image at the entry of update 233. It also watched the
  house view's draw guard, the candidate list, the choice and the action
  counter. The view drew a frame before update 1 and after updates 1, 2,
  3, 33, 63 and every 30th to 213. Our run uses those frames when it is
  compared with this capture. With a frame after every tick from 33
  instead, our state after tick 232 is the same.

How it was checked:

- Our state after each of ticks 1 to 5, 30 to 35 and 225 to 232 equals
  the second capture's, field by field, floats bit for bit, in the same
  fields as for the first capture. The `updates.jsonl` line is equal
  after every tick from 1 to 232.
- The first capture still matches as before: every tick to 100, every
  tenth to 230, and the `updates.jsonl` line to 232.
- In tick 233 the watchpoints saw six candidates, the choice of menu 6
  (0x45e28b), and the action counter going from 0 to 1 (0x41f15e). Ours
  does the same. Find Best Interaction was also compared step by step
  with the original's code, run on a memory image at the entry of
  update 233.
- At our stop in tick 233, these equal the capture's state after the
  tick: the lot's random stream, the action counter, person 21's running
  and last actions, its frames, its use of object 132, object 132's user
  count, and every object that ran before person 21.
- `the_goth_house_first_tick_matches_the_original` runs to tick 232,
  pins the state, and checks where tick 233 stops and what it holds
  there. The pins taken under format 14 still hold with the new counter
  left out (`format_14_bytes`).
- The new primitives have synthetic tests.

What did not match, and why:

- At our stop, person 21's data differs from the capture's. The capture
  went on past the stop: it numbered the person for the doors (object
  data word 6), pushed two frames of the door's tree, and put the door
  in use. Object data word 9 and person data words 27 and 37 also
  differ there. We do not run those steps.
- The first capture does not choose in tick 233. Its one draw of ticks
  233 and 234 comes in tick 234. Its state after tick 232 was equal to
  ours as far as we compared. The second capture, and the original's
  code run on the first capture's state from update 47, choose in tick
  233. So the original does not always do the same here. We follow the
  second capture.
- Global 12 after tick 1 is 17 in the first capture and 0 in the second.
  Ours is 0.

Found wrong along the way: an entry with no check tree failed loudly in
our Find Best Interaction; it is now allowed, as in the original. The
house did not reach that case. The comment on Alter Budget named the
wrong scopes; the code was right (0x44eb02: a literal, an argument, a
local).

**Unverified:** why the first capture chose a tick later; the end of an
action and of a use (no use ends in the ticks that run); whether the
steps the capture took past our stop wrote object data word 9 and person
data words 27 and 37; Alter Budget's spending, and Add/Change Action
String outside a check tree, which fail loudly.

## 2026-11-14: The house view's frames are modeled, a person walks, and K = 230

Our state now equals the 1000-tick capture of the Goth house after each
of ticks 1 to 100 and after every tenth tick from 110 to 230, in
everything compared, but for global 12 after tick 1 (already known). The
capture has no full state after ticks 231 and 232, and tick 233 stops at
Find Best Interaction (primitive 3), which is not ported. K goes from 33
to 230.

Some terms. A *frame* is one drawing of the game's window; the *house
view* is the part that draws the lot. A *write watchpoint* is a hardware
breakpoint that stops the game when it writes given bytes; it gives the
instruction that wrote (its address, EIP) and, through the saved frame
pointers, the return addresses of the calls that led there (the *call
chain*). The *animator* and its *state* are as in the entry below.

What was decided:

- **The first frame that draws a person writes its animator, and that
  is modeled** (`crates/sims-vm/src/frame.rs`,
  `Interpreter::draw_frame`). After tick 34 the capture had person
  259's direction and destination direction at 0. A run of the original
  with a write watchpoint on the direction (`capture.py --watch`)
  stopped at 0x584895, outside the lot update, between updates 33 and
  34. Call chain: 0x45e36b (the person's draw), 0x4ab67d, 0x4a945d,
  0x4a8deb (the house view's draw), 0x4d1bbb, 0x4d0dea
  (`DDDSimsView::Draw()`), 0x5dcffe (the main loop). It is the
  animator's draw (0x590c05). Its byte +0xdb is set when the animator
  is made and cleared by the first draw, which also unsets the position
  (0x589361) and places the animator on its person's tile, facing the
  person's direction word (0x592132). Drawing is presentation, but this
  writes state the next tick reads, so it is modeled. The rest of the
  draw (the pose, bounds, screen rectangle, the skin's censor update) is
  not; that it writes nothing the simulation reads is **unverified**.
  Where the draw would need more than this (a skin waiting for its
  update, a head turn, a body that follows person word 8, a person in
  the house view's second list), it fails loudly. Byte +0xdb is kept as
  `Animator::undrawn`; snapshot format 14.
- **When frames are drawn is an input.** A second run watched the house
  view's draw guard (written at 0x4d1b60 and 0x4d1be7). The view drew
  before the first update, after updates 1, 2 and 3, and after update
  33, and not after updates 4 to 32 or 34 to 45. People 16, 18 and 21
  were drawn by the first frame (their +0xd8 to +0xdb were already 0
  before update 1). That timing comes from the main loop and the window
  system, not the simulation, so `sims boot --frames` takes it, and the
  default is `0,1,2,3,33+` (0: before tick 1; 33+: after every tick from
  33 on). Checked: with that default, and with `0,33+` or
  `0,1,2,3,33`, the state after tick 232 is the same and every compared
  tick is equal; with only `0`, 82 lines differ from tick 34 on. Why the
  original drew nothing after updates 4 to 32 is **unverified**; it
  depends on how the harness paces the game.
- **Registry byte 0x12 is set at the end of the object read**
  (`World::finish_object_read`). A watchpoint on it stopped at 0x43fc8d
  (in 0x43fc02, after load steps 8 and 9), going from 0 to 1. It stopped
  again at 0x442521, in update 36, going to 0: the end of a full lot
  update clears it when the tick count is a multiple of 40 (0x4424cb),
  now ported too. Both are simulation. The load's state, and
  the state after tick 1, now match the capture on this byte; before,
  ours set it in tick 2.
- **The walk is ported** for what the house reaches (0x58eb29 to
  0x58f089, `crates/sims-vm/src/animator.rs`). Facing the point ahead,
  the animator starts the walk clip repeating (layer end mode 1,
  0x597dca), keeps the old clip's phase if that clip moves the person
  (clip +0x34, `Clip::moving`), moves each tick by the clip's distance
  times 16/3 over its duration at its speed in ticks of 42 milliseconds,
  moves the step onto the path where that is nearer (0x48313e,
  0x45ed5f), puts the person on the step's tile with the place test,
  and notes its room (0x45e43d, moved out of the person's tick as
  `note_room`). Byte +0x19c, whether it walks, is kept
  (`Animator::walking`). A walk that would stop short (0x58ed52) or is
  blocked (0x58f682) fails loudly.
- **Timed events are ported** (0x5912fd, `clip_events.rs`): `xevt`,
  `interruptible`, `lefthand`, `righthand` and `censor`. Sounds are not
  modeled. `anchor` is not kept: it feeds only code behind byte
  0x6f6d70, which no code writes and which is 0 in a memory capture of
  the running game. That nothing else reads a layer's anchor fields is
  **unverified**. `dress` and `undress` fail. Animate Sim's clip 0
  stops the foreground clip (0x5897aa).

How it was checked. The capture's full state after each of ticks 1 to
100 and every tenth tick to 230, and its `updates.jsonl` line after
every tick to 232, were compared with ours field by field, floats bit
for bit, by a script outside the repository: the fields listed for tick
1 in `the_goth_house_first_tick_matches_the_original`
(`crates/sims-vm/tests/resize.rs`), the four people's animators with the
walking flag, and each person's number of route contexts. That test runs
to tick 232, pins the state, and checks that tick 233 fails. The pins
taken before this change still hold once the new fields and byte 0x12
are put back as they were (`format_13_bytes`, `with_read_flag_clear`).
The walk, the path snap, the repeat mode and clip 0 have synthetic tests.

What did not match: global 12 after tick 1, as before.

**Unverified:** the state after the ticks the capture skips (101 to 109
and so on); the draw's other writes; the frame schedule outside the
45 updates the second run watched.

## 2026-11-14: A person routes to a slot, and ticks 14 to 33 match (K = 33)

In tick 14 of the Goth house, person 259 goes to a routing slot of
object 51 (Go To Routing Slot, primitive 45). That route is now ported:
the route context, the places to stand, the planner and the animator's
start of the route, its following of the path and its turn on the spot.
The end of a clip in Animate Sim is ported too (tick 24). Our state
after each of ticks 1 to 33 equals the capture. K goes from 13 to 33.

Some terms. A *route context* is the record a routing primitive makes
for a person and pushes on the person's list (person +0xaf0); it holds
the slot, the places the person could stand (*candidates*) and the one
chosen. The *path* is the list of points the planner gives (person
+0xae0). The *animator* is the person's object that plays clips and
moves the person along the path; its *state* says what it is doing (3
standing, 4 turning on the spot, 9 following the path). A *clip* is one
animation; the *foreground slot* holds the clip a script started.

What was decided:

- **Go To Routing Slot is ported for the route the house takes**
  (`crates/sims-vm/src/primitives/go_to_slot.rs`). Its steps run from
  the frame's primitive state, as in 0x45ad63: make the context
  (0x48163e), find the candidates (0x48235a), plan (0x45c38e), start
  the route (0x589c85), then wait while the route is under way. Every
  other step and result fails loudly. The slot's index is the operand
  word at byte 0. The word at byte 2 says where the index comes from;
  only 1, a literal, is ported.
- **The routing state is part of the lot** (`sims_sim::route::Routing`,
  `World::routing`): each person's contexts and path, the slot flags
  that differ from the type's, and the person words the original wrote
  from memory it never set.
- **Person word 37 is held as unknown.** The planner copies a
  candidate's word +0xe into it (0x45c418), and the slot search never
  writes that word. The original's value is whatever its memory held
  (3287 in the capture). We store 0 and mark the word unknown; a script
  that reads it fails loudly, and a write makes it known.
- **The planner is ported** (`routing/planner.rs`, `regions.rs`,
  `rooms.rs`, `search.rs`), from the original's code and the old
  prototype's search (legacy/crates/sims-sim/src/lot_route/search.rs,
  legacy/crates/sims-sim/src/region_path.rs,
  legacy/crates/sims-sim/src/regions.rs). An object's obstacle keeps its
  footprint's words in their own order. An obstacle that costs 8 or
  more, or is a person, grows by 3. Only an outdoor room on the ground
  of at least 100 tiles is ported.
- **The animator follows a path and turns on the spot**
  (`crates/sims-vm/src/animator.rs`): state 3 (0x58ddbf), the start of a
  turn of more than 3 eighths the negative way (0x58df01), state 4
  (0x58e045) and state 9 (0x58e4e0, 0x58eb29). A new animator and a reset
  one have segment -1 (0x587d2c, 0x589220). Angles use the FPU's
  `FPATAN`, which `sims-core` gives as `Ext::atan2` through libm.
- **A clip's end is ported** (`primitives/animate.rs`, 0x589c25): the
  animator stops playing, takes its state from the posture (0x592904:
  0 gives 3, 1 gives 1, 2 gives 2), and brings its base layer up to
  date (0x590430). The pickup and person word 19 are cleared, and the
  true branch is taken. Starting the next clip clears the finished one
  from the foreground slot at once (0x586b63). The original fades it out
  instead when byte 0x6f6bd0 is set; that byte is 0 in a dump of the
  running game, and the fade is not ported.
- **Snapshot format 13** adds the route fields of each animator and the
  routing state. Every older pin in `tests/resize.rs` and
  `sims-sim/tests/world.rs` still holds once the new fields are taken
  out (`format_12_bytes`), so the pinned states are the same.
- **The harness can dump the game's memory** (`tools/harness/qmp.py`,
  `capture.py --dump`). QEMU's `memsave` reads the stopped guest; nothing
  writes it. The dumps feed an emulator of the original's code that is
  outside the repository.

How it was checked:

- Our state after each of ticks 1 to 33 was compared with the capture's
  after the same lot update, field by field, floats bit for bit. The
  fields are those of the previous entry, plus the animators' route
  fields and each person's number of route contexts. The list is in the
  doc comment of `the_goth_house_first_tick_matches_the_original`.
- The capture does not hold the context or the path. A new run of the
  harness dumped the original's memory at the entry of updates 14, 15,
  47 and 580. The original's update was run in an emulator on the dump
  of update 14. Its context, the planner's obstacles, room rectangles
  and goals, both searches, and the 11-point path equal ours.
- Ticks 1 to 13 still match, and the load comparison with its trace
  gives the same 3 lines as before.

What did not match, and why:

- After tick 1, global 12 and registry byte 0x12. These are the known
  differences of the previous entries.
- After tick 34, person 259's animator. The capture has 0 as its
  direction and its destination's direction; ours has the turn's (0.127
  and 0.210). The emulated update writes the same direction as ours, at
  0x58488f, so the 0 is written outside the lot update. The dump of
  update 47 also has 0.
- From tick 36, registry byte 0x12 (ours 1, the capture 0). Its cause
  was not looked into.
- Tick 46 does not finish. Person 18 starts a clip with timed events
  (0x5912fd), which is not ported.

What is **unverified**:

- What writes 0 into person 259's animator direction after tick 34. A
  guess is the interface: an animator's position can be unset (virtual
  function 0x20, 0x589361) and placed again from the person (virtual
  function 0x60, 0x592132). Nothing ported runs the interface.
- That `Ext::atan2` gives the FPU's value. libm is correct to within one
  unit in a double's last place; the game rounds to 24 bits, so only an
  angle within about 2^-52 of a rounding midpoint could differ.
- That the sound events of a footstep change nothing the simulation
  keeps. They are not modeled.
- The branches of the routing the house does not reach. They fail
  loudly. The synthetic tests check the obstacle rule, the following of
  a path and the turn against our reading of the code.

What would reopen this: a difference found in ticks 14 to 33, or the
writer of the animator's direction after tick 34.

## 2026-11-14: A script can make a person, and ticks 6 to 13 match (K = 13)

In tick 6 of the Goth house, object 382 makes a new person (Create New
Object Instance mode 8). That is now ported, and so is what the next
seven ticks reach. Our state after each of ticks 1 to 13 equals the
capture of the original. K, the number of ticks after each of which our
state equals the capture's, goes from 5 to 13. Tick 14 stops loudly at
routing.

Some terms. A *capture* is the private record the harness took of the
original running the Goth house: its full state after each of lot
updates 1 to 100, and one line per lot update in `updates.jsonl` with
the tick count, the three random streams and the scheduler cursor. An
*object use* is an entry in a person's list of the objects it is using
(person +0xb00). The *usage counts* are six pairs of numbers in the
house file's model (HOUS 1): for each of the last five days and today,
how often a household member was counted, and how often while routing.

What was decided:

- **The person factory is ported** (`crates/sims-vm/src/person_factory.rs`,
  `Interpreter::create_person`), following the person path of 0x440b2d
  from 0x440d63: a guard against pets, the lowest unused ID, the
  storage, the skeleton (which draws from the C runtime's stream), the
  neighborhood record, default traits if the interests are low, the
  construction reset (0x4598e8: Init twice, the first update once, the
  animator reset), and the price and ready flag. Ported from the old
  prototype's `PostConstruction`, `ConstructionReset`, `InitialReset`,
  `default_traits` and `attach_person_neighbor`
  (legacy/crates/sims-vm/src/person_factory.rs).
- **The pet guard fails loudly.** A person type whose skeleton is `dog`
  or `kat` is made only if a field of the running game (+0x66 of the
  object at 0x6e48bc) has bit 0x20 or 0x80 (0x440d8b to 0x440dab). That
  field is not modeled.
- **Mode 8 makes a person** (`primitives/create.rs`): the factory, then
  the record's interests and personality (0x425930), then, with flag
  0x20, the stack object's facing whole (0x44e684). A new person whose
  person word 84 or 68 has bit 0 set, or word 74 bit 0 or 1, goes
  through 0x45810c, which fails loudly.
- **The place test handles a person** (`placement/find.rs`): a person
  skips the wall rules (0x432ac5); one with person word 68 bit 0 collides
  with nothing; another object is in its way as 0x433975 says (when not
  solid, with data word 8 flag 2; when solid, without flag 0x10). Person
  word 84 bit 0 fails loudly (0x43383f). What the interface does when a
  person comes onto the lot (0x44752e) fails loudly in three cases (the
  selected Sim, person word 77 above 0, person word 81 of 10 or more)
  and otherwise does nothing.
- **The house's usage counts are state** (`World::usage`,
  `HouseUsage`). They are read from the house file's model with the
  simulator state (0x416568; `World::read_house_model`). Every tenth
  tick, each awake household member adds one to today's total, and one
  to today's routing count if it is in the middle of a route (0x45ce14
  to 0x45ce5c, 0x4165e3). The roadmap said before that the person asks
  the lot something there; that was wrong.
- **Object uses are counted** (`crates/sims-vm/src/object_uses.rs`).
  When a person runs a tree of another object with Run Tree By Name in
  mode 2, the person's virtual function 0x34 (0x45e59b) adds an entry
  and the object's data word 62 goes up. After each frame a person's
  thread pops, its virtual function 8 (0x45e5fd) removes the last entry
  if its frame is gone. When the list empties, the running action ends
  (0x45e6cd). The result of a return is written after that (0x474da0),
  so the ended action's flag 0x20 takes the result before the return.
  Ported from the prototype's `push_target_tree_args`,
  `pop_person_reset_frame` and `clear_person_action`.
- **The first clip reads the appearance again.** A new animator has its
  flag +0xd8 set (`Animator::profile_stale`); looking up the first clip
  reads the person's appearance and clears it (0x5893b3).
- **Snapshot format 12** adds that flag at the end of each animator and
  the usage counts at the end of the encoding. Every older pin in
  `tests/resize.rs` and `sims-sim/tests/world.rs` still holds once the
  new fields are taken out (`format_11_bytes`, `without_usage`), so the
  pinned states are the same.

How it was checked: our state after each of ticks 1 to 13 was compared
with the capture's after the same lot update, field by field, floats
bit for bit. The fields are listed in the doc comment of
`the_goth_house_first_tick_matches_the_original`: the `updates.jsonl`
line; the tick count, streams, cursor, funds, globals, removal queue,
registration order and set of objects; the 463 neighborhood records,
the inventories and registry byte 0x12; every object's GUID, data
words, temps, attributes, scheduler word, last result, frames,
relationships, slots, position and marks; every person's data, motives,
actions, queue and objects in use; and the people's animators. 617
lines are equal. Ticks 1 to 5 were also compared again with the first
capture, and the load with its trace; both give the same differences as
before.

What did not match, and why:

- After tick 1, global 12 (17 in the original, 0 here) and registry byte
  0x12 (set in the original from its load, here from tick 2). These are
  the known differences of the previous entry.
- Tick 14 does not finish. Person 259 reaches Go To Routing Slot
  (primitive 45, 0x45abce), the start of routing, which is not ported.
  Where it stops, person 259's frames and objects in use and the 258
  objects before it in the tick equal the capture's after tick 14.

What is **unverified**:

- The usage counts. The capture does not hold them, and nothing ported
  reads them.
- The end of an object use and of a running action. No use ends in the
  ticks that run.
- One local of person 259's third frame at the tick 14 stop. The
  original left half of a pointer there (2623); we hold it as unknown,
  and it is not compared.
- The parts of the person factory, mode 8 and the place test that the
  Goth house does not reach. The synthetic tests check them against our
  reading of the code.

What would reopen this: a difference found after tick 13, or a capture
that holds the usage counts.

## 2026-11-14: The comparison harness runs again, and a capture holds 1000 ticks

The original runs again in the emulator and can be stopped and read at
every lot update. `tools/harness/` holds the scripts. A new private
capture of the Goth house holds the state at the entry of tick 1, after
each of the first 100 lot updates, and after every tenth up to 1000.

What was decided:

- **The harness is the old prototype's, cut down.** Debian bookworm
  with `qemu-system-i386` in Docker. An i386 guest with full emulation
  (TCG) runs `Sims.exe -w -nosound` under Wine. Hardware breakpoints
  stop the game at the entry of the lot update (0x46d345) and at the
  end of its body (0x46d51d). State is read through QEMU's GDB stub.
  Nothing writes guest memory or registers. The only inputs are the
  button presses the README lists.
- **The guest reads the install's own UserData.** The guest's UserData
  folders are replaced with copies of the install's, and their hashes
  are checked before the game starts. With that, the C runtime stream
  and all 463 neighborhood records match ours at the end of the load.
  The first capture's 52-draw offset and its five differing records
  came from that guest's own files, not from the code.
- **Dialogs are answered as in the first capture.** OK to the five
  notices in tick 1 (read with status 2), and No to the Help System in
  tick 2 (status 3). The game closed two later dialogs itself, in
  updates 901 and 902.
- **Captures stay private.** They live in the old checkout's ignored
  `cache/`. The guest disk, kernel, initramfs and ISO are inputs that
  the repository does not hold.

How it was checked: our state at the entry of tick 1 and after each of
ticks 1 to 5 was compared with the new capture field by field. 248
lines are equal and 3 differ:

- Registry byte 0x12 is already 1 when the original's load ends. Ours
  sets it in tick 2. This is two of the lines (load and tick 1).
- After tick 1, global 12 is 17 in the original and 0 in ours. Global
  12 is the interface's wait check, which we do not run. The first
  capture was read at the entry of the next lot update, not at the end
  of tick 1, and had 0 there. **Unverified:** that the open dialog sets
  it and something outside the lot update clears it.

Known gaps in the capture: the animator pointers were not recorded for
updates 1 to 45. For ticks 1 to 5 the comparison uses the pointers
recorded at update 46, on the same pages.

What would reopen this: a capture whose load does not match ours from
the install's files, or a need to observe something the GDB stub
cannot reach.

## 2026-11-14: Find Location For is ported, and ticks 2 to 5 match (K = 5)

Find Location For (primitive 16) is ported for the modes the Goth house
reaches. With it, tick 2 runs to its end, and ticks 2 to 5 run. Our
state after each of ticks 1 to 5 equals a private capture of the
original after the same tick, in everything compared, the C runtime's
random stream aside. K is now 5. Tick 6 is beyond the capture and stops
loudly.

Some terms. The *place test* is the check whether an object may stand
at a position (virtual function 0x54, 0x433311). The *search* is the
lot's walk over tiles in a square spiral out from a start tile, looking
for one that passes its tests and the place test (0x4347f0). The *C
runtime stream* is the random number generator of the C library
(`rand`), apart from the game's own two streams.

What was decided:

- **Find Location For** (0x44b3c8, `primitives/find_location.rs`).
  Mode 0, and modes 6 and up, which the original treats the same: the
  search starts from tile (1, 1) on level 1, or, with flag 1, from the
  object a local names, and looks first in the start tile's
  room, then anywhere. Flag 2 allows a tile that already holds an
  object; flag 4 refuses tiles with ground flag 0x20. The tile found is
  tested once more, the object is put there with the lot told (0x433340)
  and the true branch is taken. Modes 1 to 5 fail loudly; mode 5 is the
  one that draws random numbers. Native errors 0x15, 0x33 and 0x17 are
  kept.
- **The search and the place test** (`placement/find.rs`), written from
  the original's code: the spiral (0x434ba9), the search's own tile tests
  (a diagonal wall, an empty grid cell, the same room, a pool tile, the
  ground flag), and the place test through 0x4329c1: the border and
  level, the walls the object needs or cannot stand against (0x476f5a,
  only for an object with no wall rules), water, pool, slope, floor,
  tile flag 4, the objects on the tile and the eight around it (0x4337f6)
  and the walls the footprint would cross (0x476dbc). A person, a tile of
  a group, a container, the upper level's floor test (0x4330b7), wall
  rules and two objects that are both not solid fail loudly. The old
  prototype's ports (legacy/crates/sims-sim/src/placement.rs `Spiral`,
  `search_tile_allowed`; legacy/crates/sims-sim/src/lot_acceptance.rs)
  were read alongside; they were checked only against the original's
  code run alone, which is weaker evidence than the capture.
- **Clip names ignore case when nothing matches exactly.** The script of
  people 18 and 21 names the clip "...-Rstand"; the animation library
  has only "...-rstand". The library's lookup (0x5a27f2) compares bytes
  exactly, but the capture shows the original playing "...-rstand" in
  tick 3, with the duration and frame count of that clip. Where the
  original ignores case was not found in its code. `Animations::clip`
  takes an exact match first, then the single name that differs only in
  case, and fails when there are two.
- **Sample names lose their directory.** Members of the animation
  archives whose names carry a directory are now stored under the bare
  file name, which is how scripts and the skeleton files name them. Tick
  3 needed this to find its clip's samples.
- **Create New Object Instance mode 8** (0x44e0c8, the jump table at
  0x44e92e, `primitives/create.rs`) is ported for an ordinary object,
  with no flag or only flag 0x20: the object is made where the object a
  local names stands, tested with the place test, removed again if it
  does not fit, and put there. Turning it (flag 0x20, when its facing
  differs) is not ported and fails loudly, as do a new person and an
  object of a group. This is only reached in tick 6, so it is
  **unverified**.
- **Object 15's dialog** in tick 2 is answered with 3, the second
  button, as the capture's events show. `sims boot` needs `--answers 3`
  to get past it.

How it was checked:

- Each of ticks 2 to 5 was compared with the capture's state at the
  return of the same lot update, outside the repository, with a script
  that builds our state from the install and the house file alone. One
  line per thing compared. Equal, after each tick: the tick count; the
  lot's and the game's random streams; the scheduler cursor; the funds;
  all 64 globals; the removal queue; the registration order; the set of
  objects; per object its GUID, data words, temps, attributes, scheduler
  word, last result, frames with their saved primitive state,
  relationships, slot count, position and two marks; per person the 256
  person data words, motives and previous motives bit for bit, the
  running and last actions, the queue and the objects in use; the
  people's animators, in every field we keep that the capture's memory
  pages hold (100 readings after ticks 3 to 5, 72 after tick 2; 23 fall
  on pages the capture lacks); and, read from the neighborhood file the
  capture's game read, the 463 neighborhood records, the inventories and
  registry byte 0x12.
- The tick-1 comparison is unchanged: equal except the C runtime stream
  and registry byte 0x12, which our tick 2 sets and the capture already
  had set after tick 1.
- The comparison of the load with its trace still gives the three known
  differences, all in object 19's frames.
- Synthetic tests cover the spiral's order, wall crossing, the search
  staying in a room, flag 2, a refusal, the native errors, and mode 8's
  success, refusal and loud failures. The gated test runs ticks 1 to 5
  on the Goth house, pins the state after tick 5 and asserts the tick 6
  stop.

What did not match, and why:

- The C runtime stream is 52 draws further on in the capture after every
  tick. Neither side draws from it in ticks 1 to 5: ours stays at the
  same value and so does the capture's. The likely cause is that the
  capture's game drew 52 more numbers before its load; that is
  **unverified**.
- Read from the install's neighborhood file, records 47, 48, 301, 344
  and 351 and record 301's inventory differ after every tick, because
  the install's file differs from the one the capture's game read.
- Registry byte 0x12 differs after tick 1 only (ours 0, the capture 1).
  What sets it before tick 2 in the original is not known.

What is **unverified**:

- Global 0x6e4ad4, which would make the search refuse more tiles, is
  taken to be 0; the capture does not hold it.
- Everything after tick 5, including Create New Object Instance mode 8.
- The paths of Find Location For and the place test that the Goth house
  does not reach. They fail loudly where they are not ported; the
  ported ones beyond what ticks 2 to 5 use are checked only by the
  synthetic tests against our reading of the code.

Tick 6 starts at tick count 88,449. Object 382 asks for a new object of
type 0x44A86A6C, a person, where the pedestrian portal (object 6)
stands. Making and placing a new person is not ported, so the tick
fails there, before anything is made. What would reopen this: a capture
past tick 5, or a difference found in a later tick.

## 2026-11-14: A person's animator is held, and tick 2 runs to Find Location For

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

## 2026-11-13: The neighborhood's people are held, and tick 2 runs to Animate Sim

The simulation now holds the neighborhood's people the way the original
does while a lot runs. Tick 2 of the Goth house gets past the scripts
that read and write them, and stops at Animate Sim on object 16, which
needs a person's animator. K stays 1.

Some terms. A *record* is the original's 628-byte entry for one person
of the neighborhood, kept in a list in the neighborhood object (+0x148):
an ID, the GUID of the person's type, a revision, 256 words of *person
data*, *relationship rows* (one per other person, a list of numbers
each), a cached friend count and a mark that says the count is out of
date (the *stale* mark). A record's *slot* is its place in that list. A
neighbor's *inventory* is a list of tokens (kind, GUID, count). The
*frame storage* is the block of words in which the original keeps an
object's script frames; a new frame's locals start with whatever an
earlier frame left there.

What was decided:

- **The records live in `World::neighbors`** (`sims_sim::neighbors`):
  every slot, empty ones included, with the inventories and registry
  byte 0x12, which a relationship write sets. `World::read_neighbors`
  builds them before the load, as the reader (0x42feb2) and the
  neighborhood's reload (0x424806) do: the file's records as read, then
  person data word 61 (the family) rebuilt from the families in their
  sorted order (0x425209, 0x425244). Ported from the old prototype's
  `prepare_family_reload` and `rebuild_family_links`
  (legacy/crates/sims-vm/src/neighborhood.rs). A person type with no
  record fails loudly: the reload would make one, which is not ported.
  `sims_content::Neighborhood` now keeps the empty slots, since a script
  names a record by its slot plus one (0x4256f4), and reads the
  inventories from the NGBH chunk (0x4254f6). Constructing a person and
  attaching it to its record now read these records, not a copy in
  `People`.
- **What reads and writes them**, each ported from the original's code
  named in its doc comment: Set To Next modes 5 and 7 (0x425b15,
  0x44ed07); expression scope 32, a record's person data (0x4537e3);
  Relationship with flag 2, and between two people on the lot, which
  keep their rows in the records (0x44d17c; the row's key is the target
  record's own ID; a write marks both records stale and sets the
  registry byte, 0x44485b); Manage Inventory operation 3 (0x4552f8).
- **Idle For Input** (opcode 17, 0x45db94), through the person's own
  dispatcher (0x45cbb3), with the test of whether something interrupts
  the wait (0x45d7c0: a queued action of higher priority or pushed to the
  front, or a very low motive on a residential lot). Checked against the
  old prototype's `idle_for_input` and `should_interrupt`
  (legacy/crates/sims-vm/src/action.rs). Starting the queued action is
  not ported and fails loudly.
- **Get Distance To** (opcode 11, 0x451f27, distance 0x434148): the sum
  of squares in 32-bit integers, then single precision for the rest, and
  truncation toward zero. The prototype has no version of it.
- **A returning frame leaves its return target in its node word.** The
  interpreter wrote the target of every branch into the frame's node
  word, except a return. The original writes it for a return too (0xfe or
  0xff, at 0x474cea and 0x474d1b) before it pops the frame, so the frame
  storage keeps it. This was wrong in the existing code. The evidence: in
  the capture's tick 2, a person's new frame finds 254 in a local that
  lands on such a word; ours found the old node, 15.
- **Snapshot format 10.** `World` gained the records. The pinned states
  in `tests/resize.rs` and the synthetic world's digest changed, for two
  reasons. The records: the tests strip them (`format_9_bytes`) and check
  the remaining encoding. The return targets: before the pins changed,
  every pinned state was dumped under the old and the new interpreter
  and compared outside the repository. Everything but the frame storage
  was equal. In the frame storage, every word that differed sits at a
  frame's node word and now holds 0xfe or 0xff. The tick-1 gated test
  still equals the capture.

How it was checked. The private capture of the original running ticks 2
to 5 (the one in the entry below) holds the 463 records, the
inventories and the registry byte after every tick. The scratchpad tool
`tick_verify.py` now compares them too, one line per field.

The capture's game read another neighborhood file. Its records equal
ours read from the emulator guests' `Neighborhood.iff` (private, outside
the repository). The
install's file differs from that one in five records: 47 and 48
(revision, person data, relationship rows of three numbers instead of
two), 301 (revision, person data, and an inventory), 344 and 351
(person data). So the comparison is run both ways: with the guests' file
every record field and the inventories are equal after tick 1; with the
install's file only those five records and the inventory differ.
Everything else in our state, after tick 1 and where tick 2 stops, is
the same from either file. The gated tests use the install's file and
say so.

Equal to the original:

- After tick 1, all 463 records (ID, GUID, revision, person data,
  relationship rows, friend count, stale mark) and the inventories, read
  from the same file. Before this, the load trace had shown family word
  61 of every record equal to ours.
- Where tick 2 stops: object 16's frames (trees, nodes, stack objects,
  arguments, locals) equal the capture's after tick 2. The capture has
  object 16 at that same node after ticks 3, 4 and 5 as well.
- Part way through tick 2, at that stop, records 32, 33 and 34 equal the
  capture's after tick 2 except two rows (33 to 34, and 34 to 33), which
  in the capture gain a third number later in the tick. That is a
  spike's comparison of a state taken part way through the tick, not a
  like-for-like one, so it is not counted.

Consistent only with the old prototype's isolated comparisons: the
family rebuild and Idle For Input's paths, which the prototype has in
the same form.

Did not match: registry byte 0x12 is 1 in the capture after tick 1 and 0
here. A load trace of another run shows it 0 at the resize and 1 after
the object read; what sets it there is not known, and nothing ported
reads it. The C runtime's random stream is still 52 draws further on in
the capture (the entry below); ours does not move in tick 2 up to the
stop.
The six type attribute words no file holds (the entry two below) play no
part in these results: the comparison is the same with and without them.

**Unverified**, with no evidence either way:

- Get Distance To's value. It is ported from the code alone; the
  comparisons that cover it are whole ticks, and tick 2 does not finish.
- That the original keeps nothing else in the records that a lot
  changes. The capture's records change only in the rows named above
  through tick 5.
- The family friend count a relationship write also marks (0x4246b9,
  family +0x124). The families are not kept; nothing ported reads it.
- Manage Inventory's token layout (12 bytes: kind, GUID, count), read
  from the code; every inventory in the capture is empty.
- The interrupt test's paths: the capture does not show which way it
  went for the Goth house's people.

What fails loudly: Animate Sim (opcode 44), whose animator tick
(0x58dc72) moves the person and runs the animation clock; Idle For
Input starting a queued action (0x45dc14); Manage Inventory operations
other than 3; a person type with no record; a record whose GUID has no
definition, in Set To Next mode 7 and Manage Inventory; Set To Next mode
11; an empty slot in the file's inventory list; and everything the entry
below lists.

What tick 2 needs next: Animate Sim and enough of the person's animator
to say, each tick, which events an animation reached and whether it is
done, from the original's code (0x58950b, 0x58dc72). The old prototype's
animation clock is marked approximate there, so it is no guide. With it
tick 2 should reach what the capture shows next: object 15's two-button
dialog, answered with the second button, and object 262 made again.

## 2026-11-13: Ticks run, and the first one matches the original

`sims boot <install> <house> --ticks N` now runs N ticks after the load
and prints the hash. On the Goth house tick 1 runs and our state after
it equals the original's. Tick 2 stops at something not ported, and the
boot fails and names it and the tick.

Some terms. A *tick* is one lot update, the original's 0x46d345. The
*scheduler* is the part of it that goes over the objects. An object's
*countdown* is the low half of its scheduler word: while it is above 0
the scheduler counts it down instead of running the object. A *dialog
record* is what the Dialog primitive leaves for the game to show; its
*status* says whether it was shown and which button answered it. A
*modal* dialog stops the game until it is answered.

What was decided:

- **The lot update, the scheduler pass, the object tick and the person
  tick are ported** as `Interpreter::lot_update`, `scheduler_pass`,
  `tick_object` and `tick_person` (`crates/sims-vm/src/tick.rs`), read
  from 0x46d345, 0x46d553, 0x4521f7 and 0x45cc8c.
- **The end of the lot update is more than the removals.** The roadmap
  said 0x4412d1 carries out the queued removals. It first goes over the
  dialog records, showing new ones and destroying finished ones, then the
  game's special panels, and only then, after a full pass, the removals;
  it ends with a check that a household Sim is selected. Ported as
  `Interpreter::end_of_update`, without the panels, which no ported
  primitive opens.
- **Dialog is ported in the form the simulation needs**
  (`primitives/dialog.rs`, 0x451552): records in `World::dialogs`, the
  statuses 0 to 5, the true branch for the first button and the false
  branch for the second and third. What a dialog says and shows is not
  modeled. Kinds 3 (text) and 5 to 15 (the special panels) fail loudly.
- **The player answers modal dialogs inside the end of the update.** The
  original waits there, in the dialog's own loop, until the dialog is
  answered. `sims_vm::Player` gives the answer. `sims boot` answers a
  one-button dialog with its only button, and a dialog with a choice
  with the next value of `--answers`; with none left the tick fails. A
  dialog that does not stop the game fails loudly.
- **Motives** (`sims_sim::motives`): the decay (0x420fb6), the mood
  (0x45d165) and the motive changes (0x45cdb3), all in single precision.
  The tuning comes from the global file (`Program::motive_tuning`).
- **Snapshot format 9.** `World` gained the dialog records with the
  registry's word 0x100, and each person's tick state: the tick of the
  last decay (person field 0x810) and the room last seen in (0xafc).
  Before a tick these encode as three zero bytes at the end. The gated
  tests check every state pinned under format 8 by dropping those three
  bytes (`format_8_bytes` in `tests/resize.rs`), and the synthetic
  world's old digest the same way.

How it was checked. A second run of the original on the same install and
house is private and outside the repository. Its observer attached while
the game sat in the dialog loop of its first tick. It holds the full
state at the entry of the next lot update, which is the state after tick
1, and the states after ticks 2 to 5, with a log of the dialog functions
it passed. The comparison is the scratchpad tool `tick_verify.py`.

Equal to the original after tick 1: the tick count, the lot's and the
game's random streams, the scheduler cursor, the funds, all 64 globals,
the removal queue (empty), the registration order, the set of objects
(the 7 IDs the load queued and 2 the tick queued are gone, in both), and
for each of the 377 objects its GUID, 72 data words, temps, attributes,
scheduler word, last result, frames with their saved primitive state,
relationships, slot count, position, and the removal and selected marks;
for each of the three people its 256 person data words, its 16 motives
and 16 previous motives bit for bit, its running and last actions, its
queue and its number of objects in use. Of the 31 lines compared, 30 are
equal. The original's log agrees with the dialogs: in tick 2 the five
controllers acknowledge an answered record and go on, and in tick 3 object
15 acknowledges a record answered with the second button.

Did not match: the C runtime's random stream. The capture's state is 52
draws further on than ours, after tick 1 and after every later tick (it
never moves in that run's ticks 2 to 5). Our tick 1 draws none. The trace
of the load, from another run, equals ours at the start of tick 1. The
likely cause is that this run's game drew 52 more numbers before its
load; that is **unverified**, and so is that the original's tick 1 draws
none.

Consistent only with the old prototype's isolated comparisons: the mood
curves' parsing (legacy `Curve::parse`, compared there with 0x461423 and
0x46137b run alone). The decay and the mood themselves are equal to the
original (above), for the Goth house's people at this one decay.

**Unverified**, with no evidence either way:

- What an object's animated sprites, a person's animator and the graphics
  work every tenth tick do; they are taken as presentation and not
  modeled. The same for signal 0xdb (only the interface takes it) and
  signal 0xf7 (only the house viewer); the lot's second subscriber, at lot
  0x10, which sees every signal first, is not known.
- The instruction limit of one tick, 458,605 (read from a memory image).
- That making a dialog record runs no object code.
- Kind 2 (three-button) dialogs, an informing dialog, and the second and
  third buttons' effect on temps and registry word 0x100: read from the
  code, not exercised by the capture.
- The scheduler pass stops at the highest ID held; the original's table
  can be longer. It shows only for an object made during a pass with a
  higher ID.

What fails loudly: the hour's rollover; a scheduler tick that returns
false; a thread that returns from its outermost frame or runs past the
limit; a person entering or leaving an indoor room; the lot service every
tenth tick (0x4165e3; first at tick count 88,450 on the Goth house);
person word 1; removing a tile of a multi-tile object or an object that
owns a dialog at the end of the update; no selected Sim; dialog kinds not
ported; a choice with no answer.

What tick 2 needs: Set To Next mode 5, over the neighborhood's people.
Object 16, a person, reaches it in a tree of the people's semi-global
file that also reads those people's relationships. Porting it means
holding the neighborhood's people (their records, person data and
relationships) in the simulation. That would reopen this entry's K = 1.

## 2026-11-13: The load runs to the first tick, and rooms have light and scores

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

## 2026-11-13: People are visited like the original visits them, and the load runs until the interface takes over

Load step 13 visits every object on the lot. It used to stop at the first
person. `sims boot` now visits people too, finishes the step, and runs
steps 14 to 16. On the Goth house all 382 visits run. The boot stops
after step 16, before the original's interface selects a Sim.

Some terms. A person's *neighborhood record* is its entry in the
neighborhood file: an ID, the person's type, and 256 person data words.
A record has a *revision*, which says how many of those words it keeps
for the person. A type has *changed* when the scripts installed now
differ from the ones the house was saved with. Function 3 of an object is
its Reset tree and function 0 its Init tree.

What was decided:

- **A person's visit is `Interpreter::visit` too** (0x459bf6). It differs
  from an object's in three ways. The person first takes the fields its
  neighborhood record keeps (`attach_neighbor`, 0x4258a3). It is not
  asked its Load function. And if its type changed it resets in its own
  way (`reset_person`, 0x45e791).
- **The record's fields.** The record's ID goes into person data word 31.
  Then the words `persistent_fields` lists for the record's revision are
  copied from the record: 29 at revision 1, 48 at revision 10. The record
  is the first of the person's type in the file.
- **A person's reset**, in this order: person data word 30 is cleared;
  the gender, age and skin tone are read again from the body strings of
  the person's type (`reload_profile`); the thread restarts at Main and
  then the Reset tree runs on top of it; person data words 0, 40, 59, 33,
  71, 72 and 73 and data words 34 and 9 are cleared, with any motive
  changes in progress; and each queued action is dropped, its target
  being asked function 4. An object's reset runs the Reset tree first and
  restarts the thread afterwards. A person's does it the other way round.
- **Only a person that is doing nothing can be reset.** A person that is
  using an object, has a running action with a target, is in the middle
  of a route, is inside another object or off the lot, or has person data
  word 73 set fails. The original does more for each. No person of the
  Goth house is in any of those states.
- **The cached room and the motive curves are not kept.** After the
  visit the original notes the person's room, which it uses to light the
  room, and loads the curves that turn each motive into its share of the
  mood. The lighting is not modeled. Nothing reads the curves until a
  person's motives are updated in a tick. `curve_table` says which string
  table the curves come from. So `World` did not change, and the snapshot
  format is still 7.
- **`People` now carries each person type's profile**: three of its body
  strings (gender, age, skin tone) and the skin tone names from its
  semi-global file.
- **Step 14 is `Interpreter::ensure_indicators`** (0x445e4a): create the
  two indicators unless the lot has them. Step 16 calls it again.
- **Step 15 is `World::finish_selection`.** After the visits the function
  that selected the household may delete people who do not belong, create
  a tutorial controller, reset the lot, and work out who is a visitor.
  None of that is ported. The step checks that none of it could do
  anything, and then changes nothing: every person is of the selected
  family and not a visitor, global 26 is 0, and the family's record does
  not ask for a reset (flag 4). Otherwise it fails and says which.
- **The roadmap's step list was wrong at its end and is corrected.** It
  had one last step, "create the indicator objects and finish
  selection". The trace shows nine more stops after the visits. Steps 14
  to 16 belong to the loader and are ported. Steps 17 to 22 are the
  interface's: it clears and sets the selected Sim, moves an indicator
  into that Sim's slot, goes to live mode, and updates the rooms. They
  are written down in the roadmap from the trace and are not ported.

How it was checked. The private trace of the original loading the Goth
house holds a full state at the return of step 13, and at three stops
after it: the return of the indicators' creation after the last visit,
the return of the lighting update that ends the household selection, and
the return of the indicators' creation called when the house is
announced. For every visit it holds each call of the function that runs
an object's callback (0x422d46). Our run was compared with it outside
the repository (`people_verify.py` in the private scratchpad, which
prints one line for each thing compared).

*Equal to the trace:*

- **Every call of every visit.** For each of the 382 visits: the object
  visited, and the calls it makes, as object and function, in order.
  1,781 calls. The three people are visits 202 to 204, objects 21, 18
  and 16, and each is asked function 3 and then function 0.
- **The whole state at the return of step 13.** All 386 objects: GUID,
  the 72 data words, temps, attributes, scheduler word, last result,
  every frame's tree, node, stack object, parameters, locals and saved
  primitive state, the relationships, the number of slots, the position
  and the removal-without-resets mark. For the three people also the 256
  person data words, the 16 motives and the 16 previous motives bit for
  bit, the running action and the last action (ten fields each), the
  queued actions and the number of objects in use. The tick, the 64
  globals, both game random streams and the C runtime's, the funds, the
  removal queue, the indicator references, the registration order, the
  instance lists, which rooms wait to be counted (8 of 11), and the
  arrays inside the lot's size, the object grids included.
- **The same at the three later stops**, after our steps 14, 15 and 16.
  The trace changes nothing compared between them, and neither do we.
- **The two stops in the middle of visit 140**, as in the entry below,
  now with the people's motives and actions compared as well.

This is the first check of what is not an object field after visit 140.
The entry below called that unverified: the global random stream, the
globals, the arrays and object grids, the removal queue and the rooms
waiting to be counted. All are equal at the step's return. The global
random stream moves during the step, so its being equal is not empty.

What the trace shows of a person's visit, and what it cannot show:

- It shows the record's fields being taken: person data words 46 to 55
  change, to the record's values. The other words copied already hold
  the record's values.
- It shows the thread restarting: three frames become one Main frame at
  node 0 with no stack object.
- It shows what the Reset and Init trees do: the last result becomes
  true and seven motives change. Ours are equal bit for bit.
- It cannot show the words the reset clears or reloads being written.
  In this house they already hold the values written: the cleared words
  are 0, and the gender, age and skin tone read from the body strings
  are the saved ones. So the trace shows them unchanged, and so do we.
- It cannot show whether Main is restarted before or after the Reset
  tree runs. The result is the same here.

*Consistent with the old prototype's isolated comparisons only:*

- The order inside a person's reset, and which words it clears. The
  prototype ran the original's reset alone on 16 cases.
- The list of fields a record keeps, by revision. The prototype compared
  126 cases. The Goth records have revision 10.
- The body strings read by the reset and how they become numbers. The
  prototype compared 216 cases.
- The conditions of step 15, which the prototype compared on 192 cases.
- A queued action being dropped and its target asked function 4. No
  person of this house has a queued action.

What did not match, and why it does not matter. Object 19's second frame
has tree ID 0x8118 in the trace and 0x118 here and in the house file, at
every stop before its visit. Its visit is number 206. The visit makes
the same three calls here as in the trace (functions 2, 3 and 0), and at
the step's return object 19 is equal in every field compared: its reset
replaced the thread. So the bit does not change what the visit does.
What sets it is still not known. Six type attribute words belong to the
game that was running; with those copied from the trace's start they are
equal.

What changed in earlier work, and why:

- **The gated test of step 13 now runs the whole step.** It still shows
  the state pinned after visit 201: it makes the first 201 visits one at
  a time on a copy and gets the old digest. No other pinned digest
  changed, because `World`'s encoding did not.
- **`sims boot` no longer says it stops before the indicators.** It
  reports steps 14 to 16 and stops before step 17.
- **The roadmap's "Progress" said the boot runs through step 12.** It had
  been running into step 13 since the entry below.

What is **unverified**:

- **A person that is doing something**, as listed above. It fails.
- **A neighborhood record of a revision below 1**, which holds no person
  data, and a person with no record. Both fail.
- **The motive changes in progress being cleared.** The trace does not
  hold them, and the Goths have none.
- **Which motive curves a person gets** (`curve_table`). Nothing loads
  them yet. The trace holds the two global tables at the end of the
  load, not which person uses which.
- **Step 15 on any other lot**: a visitor, a person of no family, the
  tutorial house, a family that asks for a reset. All fail.
- **Steps 17 to 22**, which are not ported. What the trace shows of them
  is in the roadmap. Two things about them are not known from the
  trace: how the game picks the Sim it selects, and what makes the six
  indoor rooms still wait to be counted after the room update.

What is not modeled: a person's animations, graphics, action icons and
accessories during a reset; the guard against a reset inside a reset;
the cached room and the lighting it drives; the lighting update at the
end of step 15; and the signal that announces the house, beyond the
indicators it asks for.

What fails loudly: resetting a person in any of the states above; a
person type with no profile or no neighborhood record; a record with no
person data; a body string age that overflows 32 bits; and each
condition of step 15.

What would reopen this: a house whose people are in the middle of
something when saved; a house with visitors; a capture that holds a
person's motive changes or shows which curves it uses; anything that
shows a reset writing a value that differs from what the trees or the
save left.

## 2026-11-13: Objects are visited up to the first person, and frames keep what earlier frames left

After the household is selected the original visits every object on the
lot, in registration order (load step 13). `sims boot` now makes those
visits for objects that are not people, and stops at the first person. On
the Goth house that is 201 of 382 visits.

Some terms. An object's *callback table* lists the trees the game runs on
set occasions; "function 2" is the second entry counting from 0. Function
0 is Init, 2 is Load, 3 is Reset, 9 is run when the object has been placed
and 10 before it leaves. A type has *changed* when the scripts installed
now differ from the ones the house was saved with (step 7). To *rebuild*
an object is to take it off the lot, reset its storage, run its Init and
give it a first turn, and put it back.

What was decided:

- **A visit is `Interpreter::visit`** (0x431e2e). The object's position is
  written again, with its footprint and room, for it and everything it
  holds. Its Load function runs. If its type changed, it resets, and then
  Init runs. Two scheduler flags are refreshed. A reset runs the Reset
  function if the object has one, and otherwise rebuilds the object.
- **The step stops at a visit it cannot make.**
  `Interpreter::visit_objects` makes each visit on a copy of the lot and
  keeps the copy only if the visit finished. So the lot is always in a
  state the original is in between two visits, and the result says which
  visit failed and why. A person's visit fails: it needs the person's
  fields from the neighborhood record, the person's own reset, its room
  and its motive curves, none of which is ported.
- **Placement is `placement.rs`.** Taking an object off the lot and
  putting it on a tile or into another object keeps three things in step:
  the object grid, the slots, and each object's link words (data words 14
  and 26, and 2 and 3). The callbacks run in the original's order. A
  multi-tile object moves as a group.
- **The frame storage is state** (`World::frame_storage`). The original
  keeps a thread's frames in one block of memory, one after another, and
  does not clear the block. A new frame's locals are not zeroed: they hold
  what an earlier frame left at that place. We now keep that block for
  every object whose thread has pushed or popped a frame, write the live
  frames into it at each push and pop, and take a new frame's locals from
  it. Before the original runs a callback it writes an eight-word mark
  above the top frame; we write it too, because it moves where the
  callback's frames lie. Two words of each frame's header hold a pointer.
  A local that lands on one cannot be read until it is written; reading it
  fails and says so.
- **The lot's indicators are made in step 13** when the lot has none and
  an object comes onto it from outside (0x44752e calls 0x445e4a). In a
  load that is the first rebuilt object that stands on the lot.
- **An object with graphics of its own draws 32 numbers** from the C
  runtime's random stream when the factory makes it. The indicators are
  such objects. Only the draws are kept. This also settles the stream's
  state at the start of a load, which earlier entries called state of the
  running game that no file holds. In the trace the stream starts the
  load at its initial state 1 advanced by exactly 64 draws: the two
  indicators the game made when it started. `sims boot` makes those two
  on its starting lot, so the stream now equals the trace's at every stop
  from the files alone.
- **New primitives**, each for the cases the visits reach: Notify Stack
  Object out of Idle (49), Relationship (26) and Old Relationship (24),
  Create New Object Instance (42), Run Tree By Name (28), Set To Next
  modes 8 and 9, Refresh modes 1 and 2, and Generic Sims Call 10, 11 and
  43. What each leaves out fails with an error that names it.
- **The snapshot format is 7.** `World` has two new last fields: the list
  of fires and the frame storage.

How it was checked. The private trace of the original loading the Goth
house holds a full state at four moments of this step: its entry; the
entry and the return of the function that creates the indicators, which
runs in the middle of visit 140; and the step's return. For every visit
it also holds each call of the function that runs an object's callback
(0x422d46), with the object and the function's number. Our run was
compared with it outside the repository
(`visits_verify.py` in the private scratchpad).

*Equal to the trace:*

- **Every call of every visit that runs.** For each of the 201 visits:
  the object visited, and the calls it makes, as object and function, in
  order. 525 calls.
- **The whole state in the middle of visit 140**, at the entry of the
  indicators' creation. All 382 objects: GUID, the 72 data words, temps,
  attributes, scheduler word, last result, every frame's tree, node,
  stack object, parameters, locals and saved primitive state, the
  relationships, the number of slots, the position, the
  removal-without-resets mark, and the three people's data. Also the
  tick, the 64 globals, both game random streams, the funds, the removal
  queue (seven controllers asked to be removed in their Load trees), the
  registration order, the instance lists, which rooms wait to be counted
  (none), and the arrays inside the lot's size, the object grids
  included. By then 139 visits are over, 137 that only run Load and two
  that rebuild a controller off the lot, and visit 140 has taken the
  first object off the lot, rebuilt it and put it back.
- **The same when the indicators' creation returns**, with the two new
  indicators, and the C runtime's random stream, which is 64 draws
  further on. Object 246 is left out there, because the trace stops in
  the middle of its visit, and the global random stream, because the
  rest of that visit draws from it.
- **The 205 objects visited or created by visit 201**, against the
  trace's state at the step's return: every object field listed above.
  17 of them differ from the trace in data words 14 and 26 and in nothing
  else. In the trace each of the 17 is linked behind an object that is
  visited after visit 201. That object's rebuild takes it off the tile
  and puts it back in front, which rewrites those two words of the object
  behind it. So they are what they should be after visit 201.
- **The frame storage's effect.** The locals of restarted Main frames
  hold in the trace what our frame storage gives them: for example object
  12's one local is 280, the tree ID of the frame that used to lie where
  the local now does.

The comparison is not empty: between the step's entry and visit 140 the
trace changes 10 scheduler words, 10 attribute sets, the frames of 9
objects, global 36, the global random stream, the removal queue and an
instance list, and all of those are equal here.

*Consistent with the old prototype's isolated comparisons only.* These
run on the Goth house too, but the trace cannot show them wrong or right
in detail, or they do not happen there:

- Placement beyond what the objects rebuilt in visits 138 to 201 do:
  an object put into a container whose parent is asked function 9, the
  seat flag, the door mark on a tile, the wall bits of data word 5 (four
  objects are asked function 6 in the trace, and here), and a group
  placed off the lot.
- The footprint. The trace's objects hold it, but it was not compared.
- Set To Next modes 8 and 9, and the Relationship primitives beyond the
  rows they write here.
- That a tile inside the border of the 64 by 64 arrays counts as on the
  lot. The prototype tests against the arrays, not the lot's size, and no
  object of this house stands between the two.

What did not match. Object 19's second frame has tree ID 0x8118 in the
trace and 0x118 here and in the house file, as before. Its visit comes
after visit 201. In the trace that frame is gone when the step returns:
the object's reset replaces its thread. Nothing here explains the bit.
Six type attribute words belong to the game that was running; with those
copied from the trace's start they are equal. The C runtime's random
stream is now equal without copying anything.

What changed in earlier work, and why:

- **The pinned digests of steps 3 to 12 changed** because of the two new
  fields and the indicators' 64 draws. The gated tests show each state is
  the one pinned under format 6: with the fires and the frame storage
  emptied and the stream put back to its old state, the encoding hashes
  to the old digest. The comparison of steps 10 to 12 with the trace was
  run again and is equal as before.
- **The people's constructors draw 7,084 numbers, not 7,074.** They draw
  until a value fits, and they now start 64 draws later in the stream.
  Nothing they draw is kept. The stream's state after them is the
  trace's. Before, it was only once the stream's state at the start of
  the load had been copied from the trace.
- **An earlier note said function 2 is the only thing asked of 191
  objects.** It is 198: seven more run a Load tree that only wakes the
  object.
- **Refresh modes 1 and 2 no longer fail.** They mark the object's room
  as not counted.

What is **unverified**:

- **Everything after visit 140 that is not an object field**: the global
  random stream, the globals, the arrays and object grids, the removal
  queue and the rooms waiting to be counted after visits 141 to 201. The
  trace has no state there. The object fields at the end of those visits
  are equal, which the random stream and the grids feed into.
- **The contents of the slots and of the frame storage.** The trace holds
  neither. The link words and the object grids, which mirror the slots,
  are compared, and so are the locals that come out of the frame
  storage.
- **That frame storage never written reads as zero**: beyond the frames
  a save holds, and in a new object.
- **A person that refers to an object that resets.** The original makes
  the person drop it. Here that fails. No person of this house refers to
  an object reset in visits 1 to 201.
- **Notify on an object that is inside a callback further up the chain
  of calls.** We look at its top frame.
- **Create New Object Instance for a GUID with no definition**, which
  takes the false branch here.
- **The lot before a load**, as before, but for one thing the trace now
  confirms: its two indicators' 64 draws.

What is not modeled: the redrawing, shadows and lighting an object asks
for as it moves; the interface being told an object left; mode 1 of
Refresh asking for a redraw; and the old-version conversions the original
applies to an object between restoring its position and running Load.

## 2026-11-13: The lot's rooms are worked out from its walls, and the load runs through the household selection

The original works out which room each tile is in from the lot's walls.
It does that after it has read a house's objects, and then gives every
object its room (load step 10), empties the removal queue (step 11) and
selects the household (step 12). `sims boot` now does all of that and
stops before step 13.

What was decided:

- **The rooms are state of the lot** (`World::rooms`), worked out by
  `World::rebuild_rooms`, which is the original's 0x479d9a. A room is a
  set of tiles that connect without crossing a wall. The map holds one
  room ID for each tile of each level. A tile that a diagonal wall cuts in
  two has the mark -5 there, and a record that names its two triangles and
  the room of each. The ground level's rooms are numbered from 0 and the
  upper level's from 1024, in the order a column-by-column scan finds
  them.
- **A room's tile list keeps its repeats.** A room lists its tiles in the
  order it reached them, and lists a tile again each time a neighbour
  reaches it while it waits. The original counts a room's size from that
  list, so a room of 27 tiles can list 46 and count as 46. We keep the
  list as it is.
- **The same function rebuilds the tile flags**, as in the original: which
  tiles are outside every enclosure, and which can be built on. The
  original has a branch for a flood that stands on a tile with a diagonal
  wall. A flood can only get there by starting there, at the lot's corner,
  and we refuse a diagonal wall on the border. So the branch is not
  ported.
- **Whether a room is outdoors is worked out with the rooms.** The
  original works it out the first time something asks and keeps the
  answer. Nothing between the two moments changes what it is worked out
  from, so the result is the same.
- **Step 10 is `Interpreter::assign_rooms`.** The first pass writes each
  object's room (`World::room_at`, 0x433bc5) into its data word 29. The
  second runs each object's function 7 tree as a callback. Scripts can now
  read three things about a room (scope 23): whether it is outdoors, its
  level, and its area. Any other room field fails and names itself.
- **The room count is `World::update_rooms`**, the part of the original's
  room update (0x466c3c) that needs only walls and floors: each room's
  half tiles, covered half tiles, walls and covered walls, and from them
  global 27, the lot's indoor area. The original runs its update after
  step 10 and before step 11, and so does `sims boot`.
- **Steps 11 and 12 are called.** `World::select_family` now also takes
  the family's six magic words and sets `World::magic_history`, which
  nothing set before. The family's ID is the ID of its record in the
  neighborhood file, not the number inside the record.
- **Refresh modes 1 and 2 still fail loudly.** They mark a room as
  changed. No tree in steps 10 to 12 reaches them, so nothing shows what
  else the original does there. Step 13 reaches mode 2.
- **The snapshot format is 6.** `World` has a new last field, the rooms. A
  lot with no rooms encodes them as four zero bytes. The gated tests show
  that each state pinned under format 5 is the same state without those
  bytes.
- **A lot with no wall array gets a bare lot's cells outside its size.**
  See "What was wrong" below.

How it was checked. The old prototype kept, privately, a trace of the
original loading the Goth house. It stops at the entry and the return of
step 10 (0x442659), at the entry and the return of step 11 (0x428679),
and when step 12's funds setter (0x46d8dc) returns. At each stop it holds
the objects, the globals, the random streams, the arrays inside the lot's
size, the 672 bytes of each room, and some pages of the game's memory. Our
state at each of the five moments was compared with it outside the
repository.

*Equal to the trace, the rooms (entry of step 10):*

- The room IDs: 0 to 4 and 1024 to 1029.
- How many tiles each of the 11 rooms lists.
- The listed tiles, in order, of the 8 rooms whose tile memory the trace
  holds: 1, 2, 4, 1025, 1026, 1028 and 1029 at this stop, and 1027 at the
  end of the load.
- The room of every tile of the upper level, in the 32 columns of 64 the
  trace holds at this stop.
- The number of split tiles, 7.
- The tile flags of both levels, every cell. The rebuild changes none: the
  flags it works out are the flags the house saved.
- For an empty lot of 52 tiles, which the dump tool makes by blanking the
  walls and floors: two rooms that list 5,307 tiles each, as the two rooms
  the trace has before the rebuild do.

*Equal to the trace, but at a later stop.* The room of every tile of both
levels, all 64 columns, equals the trace's memory at the end of the load.
The trace does not hold the ground level's map at the entry of step 10.
Its rooms have the same tile lists at both stops, so the maps are taken to
be the same.

*Equal to the trace, step 10 (return of step 10):*

- The trace asks function 7 of all 382 objects, in registration order.
- The 62 objects whose frame storage it moves are the 62 that have a
  function 7 tree here, in the same order. They are also the 62 whose last
  result changes.
- No data word changes in the trace, and the first pass changes none here.
  So the room worked out for each of the 382 objects is the room the
  house saved. 234 of them stand on the lot.
- All the fields compared after step 9 (see the entry below), for all 382
  objects, with the last result. The globals, both game random streams,
  the removal queue and the arrays.
- Whether a room is outdoors, for the 8 rooms the trace has worked it out
  for by then. Those are the rooms the 62 trees asked about.

*Equal to the trace, the room count (entry of step 11):*

- For all 11 rooms: the half tiles, the covered half tiles, the walls
  that can take a covering, the walls that have one, and whether the room
  is outdoors.
- Global 27, which goes from 0 to 204.
- Everything compared at the return of step 10.

*Equal to the trace, steps 11 and 12:* the same fields after each. The
removal queue goes from four IDs to none. In step 12 global 9 goes from 0
to 5, global 13 and the funds from 2,515 to 2,542; the selected family's
ID is 5, its magic history is marked as read and its six words are zero.
The trace's next stop, the entry of step 13, differs from that one in
nothing compared here.

The comparison is not empty. Between the stops the trace changes exactly
what we change: the rooms, 62 last results, global 27, the queue, and
globals 9 and 13 with the funds.

*Consistent with the old prototype's isolated comparisons only.* The
prototype ran the original's code alone, outside a running game, on
made-up inputs. These rest on that and on nothing in the trace:

- The room of an object on a split tile. No object of the Goth house
  stands on one. The prototype compared 32,780 positions.
- Which room each triangle of a split tile has. The trace's memory does
  not hold the records. The prototype compared 80 made-up layouts and
  six real floors.
- The edge of a pool as a room boundary. The Goth house has no pool.
- The parts of the flag rules this house does not reach. It does have
  sloped ground, fences, a fence on a diagonal, and an upper level with
  supported and unsupported tiles, and its flags are equal in every
  cell. The prototype compared 96 made-up cases.
- The refused cases: crossing diagonals, unmirrored walls, a diagonal on
  the border.

What was wrong, and is fixed. `World::read_arrays` keeps the lot's own
cells outside the lot's size, and a lot with no array had zeros there.
That was marked unverified. The trace's memory at the entry of step 10
holds most cells of every array, also outside the size, and all of them
at some stop for the walls. Every held cell equals ours but one kind: the
ground level's wall cells outside the 52 tiles. In the original each is
`00 0f 01 00 01 00` and eight zero bytes: no segments, flags 0x0f, both
styles 1. The held lot has that in every ground wall cell when the load
starts (the trace's memory at the entry of the resize), and the house
file saved the same bytes there. A lot here has no arrays before the
load, so `read_arrays` now gives the ground level's walls that cell
outside the size (`BARE_GROUND_WALL`). After the fix every held cell of
every array is equal. The digests of the arrays and of the states after
steps 6e and 9 changed with it. The gated tests show that zeroing those
cells again gives the digests pinned before.

The same memory settles two things the entry below left unverified. The
eight grid cells outside the lot's size that name objects standing
elsewhere are still there in the trace after step 9, as they are here. And
the cells outside the size of the other arrays are what we have.

What did not match. Object 19's second frame has tree ID 0x8118 in the
trace and 0x118 here, at all five stops, as before. Nothing in this work
explains it. The C runtime's random stream and six type attribute words
belong to the game that was running; with those copied from the trace's
start of the load they are equal at every stop.

What is **unverified**:

- **The listed tiles of rooms 0, 3 and 1024.** The trace holds how many
  there are, not which.
- **The split tile records**, beyond their number.
- **An object on a split tile**, as above.
- **Room fields 2 and 3 as a script reads them** (level and area). The
  trees of step 10 read only field 1. The area a script reads before the
  rooms are counted is 0 here, because the count is; no tree shows that.
- **Global 10 in step 12.** The house saved its own number there.
- **Selecting no family**, and a family record older than version 9,
  which has no magic words. `sims boot` refuses a house no family lives
  in.
- **The level argument of step 10.** The original can run both passes for
  one level only. We always run both levels.
- **Arrays 4, 6 and 7**, as before.

What is not modeled, and not compared:

- The rest of the original's room update: what each object adds to its
  room, the rooms' light and scores, the lot's outdoor score, the lighting
  of the tiles. In the trace this update changes no object, random stream
  or global but 27, so the state we compare is not affected yet.
- Which rooms have people in them. The first pass of step 10 tells a
  person's room that it is occupied.
- The rooms of the bare lot before the load. The original has two after
  the resize (step 3). `World::rooms` is empty until step 9a. No script
  that runs in between reads a room.
- Two booleans about the upper level, the roofs, and the redrawing.

What fails loudly: two diagonal walls crossing in one tile; a wall its
neighbour does not mirror; an enclosing diagonal wall on the border of
the arrays; more than 1,024 rooms on a level; a missing or misshapen
array; asking for a room before the rooms exist; a room field other than
1 to 3; Refresh modes 1 and 2 on an object that is in a room; an object
that removes itself while it is told the rooms changed; and booting a
house no family lives in.

What the trace shows about step 13, noted here because it sets the next
piece of work. The step asks function 2 of all 379 objects that are not
people. 184 objects have a changed type, and 158 of them have no Reset
tree, so they are taken off the lot and put back: the trace asks
functions 10 and 9 of each, and of every tile of its group. 148 of those
stand on the lot. So step 13 needs placement: taking an object off its
tile and putting it back, with the grid, the slots, containers and
groups. A trial run of the Load trees here also stopped at Notify Stack
Object out of Idle, Generic Sims Calls 11 and 43, and Refresh mode 2.
`docs/roadmap.md` has the numbers.

What would reopen this: a house with crossing diagonals or with an object
on a split tile; a capture that holds the split records or the tile lists
of the big rooms; a script that reads a room's area before the rooms are
counted; anything that reads a room's light or score, which needs the
rest of the room update.

## 2026-11-13: The controllers are reconciled among the saved objects, and the grid repair clears only what is inside the lot

Load step 8 makes the lot hold one of every lot-wide controller, now that
the saved objects are on it. Load step 9 clears cells of the ground
level's object grid that name an object which is not there. `sims boot`
runs both and stops before step 10.

What was decided:

- **Step 8 is the reconciliation of step 3, called on the full lot**
  (`Interpreter::reconcile_controllers`). Creating the missing controllers
  needed no new code: the factory, the primitives and the services the
  141 controllers reached on a bare lot were enough for the 137 created
  here.
- **Later duplicates are removed.** The original lists the objects whose
  definition has a nonzero global word once, in registration order. For
  each controller definition it keeps the first listed instance and
  removes each later one at once (`Interpreter::remove_now`), scanning the
  list again from the start after each removal. The function now returns
  what it created and what it removed (`Reconciled`), and so does
  `Interpreter::resize`.
- **An object can be deleted on a lot with people, if no person refers to
  it.** The original tells every person about a deleted object. A person
  that does not refer to it changes nothing. A person refers to an object
  when it is using it, or when its running, last or a queued action
  targets it or shows its icon. Deleting an object a person refers to
  fails with an error that names the person, before anything changes.
- **Step 9 is a step of `World`** (`World::repair_grid`), because it runs
  no script. It looks only at the ground level's grid, and only at cells
  inside the lot's size. A cell is cleared if its ID is negative, names no
  object, or names an object whose tile and level are not that cell on
  level 1. The tile is the position divided by 16, rounding down
  (`World::tile`).
- **"Restore placement links" is not a step.** The roadmap listed it with
  step 9, after the old prototype, which built its own link tables at
  this point. The original's function for steps 8 and 9 (0x43fc02) ends
  after the grid repair. The links are state the object read already
  filled: the grid, and each object's slots. Nothing is rebuilt.
- **Step 11 has its state change** (`World::discard_removals`). It is not
  called, because step 10 is not ported.
- **The snapshot format stays 5.** `World` has no new field.

How it was checked. The old prototype kept, privately, a trace of the
original loading the Goth house, with the game stopped at several moments.
One is the return of 0x43fc02, after steps 8 and 9. Our state after both
steps was compared with it outside the repository.

Equal for all 382 objects: the ID and GUID, the 72 data words, the temps,
the attributes and their number, the scheduler word, the last result,
each frame's node, stack object, arguments, locals and saved primitive
state, the relationships, the number of slots, the position, the tile and
level bytes the grid repair reads, and the mark of an object that asked to
be removed without resets. Equal for the three people: the 256 words of
person data. Also equal: the registration order, the tick, the lot size,
the 64 globals, both game random streams, the four IDs queued for removal
(42, 44, 278, 280), the scheduler cursor, the funds, the indicator
references, the one instance list, and inside the lot's 52 tiles the
terrain heights, both levels' floors, walls, object grid and flags, the
pool and the water.

The comparison is not empty. Between the stop after the object read and
this one, the trace gains 137 objects and two queued IDs and its global
random stream moves. Nothing else it holds changes: no saved object, no
global, no type attribute, no grid cell. The same is true here.

*Slots.* The trace also saved pages of the game's memory at each stop.
They hold the slots of 198 objects at this stop and of 149 saved objects
at the stop before, 216 objects in all. For each, the object ID in every
slot equals ours. Those objects hold 30 of the 32 links between objects
that our state has.

*The grid.* In the trace every occupied cell of both grids names an
object whose tile bytes are that cell, before and after step 9, so the
original cleared nothing. We clear nothing either. The saved grid does
name eight objects that are elsewhere, all in cells outside the lot's 52
tiles; scanning the whole 64 would clear them. That the scan stops at the
lot's size is from the original's code, which was read for this port, and
from the old prototype, which ran that code alone on made-up grids with a
size smaller than the array.

What did not match. Object 19's second frame still has tree ID 0x8118 in
the trace and 0x118 here, as after the object read. Nothing in steps 8 or
9 touches it. In the trace the frame is gone after step 13, when the
object's thread holds only its Main frame. It is the only frame with that
bit at any stop looked at. It stays unexplained.

The C runtime's random stream and six type attribute words belong to the
game that was running, as before. With those copied from the trace's
state at the start of the load, they are equal at this stop too. Step 8
draws nothing from the C runtime's stream.

What is **unverified**:

- **Removing a duplicate controller.** The Goth house has none, so no
  capture shows it. The selection follows the old prototype, which
  compared its loop with the original's 0x442b9f run alone on 97 made-up
  catalogs, with the factory and the removal replaced by stubs.
- **That a person who does not refer to a deleted object changes
  nothing.** From the old prototype, which compared its port of the
  person's notification with the original's code run alone. No capture
  holds a deletion on a lot with people. Which of a person's fields count
  as a reference follows that port too; where it tests the frames of the
  person's thread that use an object, we test the saved list of uses.
- **Clearing a grid cell.** No capture shows one being cleared. The rule
  is from the original's code and the old prototype's isolated runs of it.
- **Cells outside the lot's size.** The trace does not hold them. We keep
  what the house saved there.
- **The slots of the other 166 objects**, and every slot field but the
  object ID.
- **`World::discard_removals` in its place.** The trace shows the queue
  going from four IDs to none with no object changing. Our state at that
  moment was not compared, because the load does not reach it.

What is not modeled and fails loudly instead: removing a duplicate that
is on the lot, holds or sits in something, is multi-tile, or has a
callback for leaving the lot; removing one a person refers to; a
controller that a script removed while the controllers were being
reconciled, where the original would use freed memory; and a ground grid
that is missing or not 64 by 64 two-byte cells.

What is not modeled and not compared: the animation tester's ID, which
step 8 notes for the interface; the log the second pass of step 9 writes;
and two flag bytes the object registry sets around these steps.

What the trace shows about the next steps, noted here because it changes
the plan. Step 10 needs the lot's rooms, which the original builds
between step 9 and step 10 (the trace has 2 rooms before and 11 after).
Working rooms out from the walls is the next piece to port. And the two
IDs step 3 left in the removal queue, 42 and 44, name saved objects after
step 6e. Step 11 empties the queue before anything acts on it, so it must
not be skipped.

What would reopen this: a house that saved a controller twice; a house
whose grid names a missing object inside the lot's size; a capture of
either; or finding what sets bit 15 of a frame's tree ID.

## 2026-11-13: Saved objects are constructed and then read, and the load runs through the object read

Load step 6 reads the rest of the house onto the cleared lot. This entry
covers its five parts: the lot arrays, the floor and wall renumbering, the
type table, the second clear and the saved objects. `sims boot` now runs
all of them and stops before step 8.

What was decided:

- **Saved floors and walls are widened into the lot's arrays**
  (`World::read_arrays`). A house of version 53 to 62 saves a floor cell
  as one byte and a wall cell as 8 bytes. The lot holds 2 and 14. The
  original widens only the cells inside the lot's size. Here the cells
  outside it are zero when the lot had no array. Other house versions
  fail.
- **The renumbering is ported for an empty map only**
  (`World::renumber_surfaces`). A floor ID the map should have listed
  becomes 65535. A wall cell that would change fails. A map with entries
  fails.
- **Reading the objects is a step of the interpreter**
  (`Interpreter::read_objects` in `crates/sims-vm/src/objects.rs`). It
  needs the object definitions, which `sims-sim` does not hold.
  `World::read_objects` is gone. `World::from_house` stays as a way to look
  at a house file; it copies saved fields and constructs nothing.
- **A saved object is constructed first.** It gets the attributes its
  definition asks for (8 at least), a slot for its tile and one for each
  object it can hold, and its definition's dynamic sprite flags. The saved
  fields are read over that and never shrink it. A record with no
  scheduler word keeps the word registration gave.
- **Multi-tile links are state** (`World::next_parts`). Each tile joins its
  lead tile's list in the file's order.
- **A person's constructor is ported as far as it changes state we hold.**
  It reads the skeleton's file if the game has not read it yet, builds the
  skeleton, attaches the person to its neighborhood record, and makes
  default interests and personality if the record gives it none. Each of
  those draws from the C runtime's random stream. The saved person data
  replaces the defaults. Person data has 256 words; an older save's 80
  are followed by zeros.
- **Which skeleton files the game has read is state**
  (`World::skeleton_files`). It belongs to the running game, like the C
  runtime's stream.
- **`sims_content::People` finds what the constructor needs.** The
  skeleton's name is the first of the person type's body strings. The
  file is the one skeleton file that defines that name, among the
  archives in `GameData/Animation` and the indexed object directories. A
  name that two files define is an error.
- **The snapshot format is 5.** The two new fields are at the end of the
  encoding.

How it was checked. The old prototype kept, privately, a trace of the
original loading the Goth house, with the game stopped at several moments.
Three of them were compared with our state outside the repository. A
"capture" below is one of these stops.

*After the arrays and the renumbering* (the capture at the entry of
0x43fc02). The capture holds the cells inside the lot's 52 tiles of the
terrain heights, the floors, walls, object grid and flags of both levels,
the pool and the water. All equal ours. The capture taken before the
arrays are read differs from the house, so the comparison is not empty.
The house's two maps are empty, and none of its cells holds an ID the
renumbering covers, so the renumbering changes nothing here; the capture
agrees.

*After the object read* (the capture at the return of 0x449c5e). Equal for
all 245 objects: the ID and GUID, the 72 data words, the temps, the
attributes and their number, the scheduler word, the last result, each
frame's node, stack object, arguments, locals and saved primitive state,
the relationships, the number of slots, and the position. Equal for the
three people: the 256 words of person data. Also equal: the registration
order, the tick, the lot size, the 64 globals, both game random streams,
the two IDs queued for removal, the scheduler cursor, the funds, the
indicator references, the one instance list, and the arrays again.

One object shows why the constructor matters: its definition asks for 16
attributes and the save holds 11. The capture has 16.

*The random draws.* The C runtime's stream starts in a state that belongs
to the game that was running. With that state, and the six type attribute
words that also belong to it, copied from the capture at the start of the
load, our stream after the object read equals the capture's. So the
constructors of the three Goths draw as many numbers here as in the
original. The stream's state only depends on how many numbers were drawn,
so this does not show what each draw was for. In the capture the stream
does not move anywhere else between the start of the load and the end of
the object read.

*Multi-tile links.* The trace does not hold them. Another capture, of the
same house four ticks after its load, does: all 142 tiles name the same
next tile, lead tile, origin and offset as ours.

What did not match. One frame of object 19 has tree ID 0x8118 in the
capture and 0x118 in the save and here. Object 17 has the same type and
the same thread, and matches. The capture still shows the bit after
step 8. What sets it is not known. It is **unverified** whether it is
state.

What is **unverified**:

- **Arrays 4, 6 and 7, and every cell outside the lot's size.** The
  captures do not hold them. That a lot without an array has zeros outside
  its size is a guess.
- **The rewrite to 65535.** The Goth house has no cell it applies to. It
  follows the old prototype, which compared its port with the original's
  code run alone on 24 made-up lots.
- **What the draws are for.** The split into skeleton files, skeletons
  and defaults follows the old prototype, which compared each part with
  the original's code run alone. Only the total was compared with the
  running game.
- **That the game has read no skeleton file before its first load.** The
  total fits that for this capture. Another session might have read one
  already.
- **Which files the skeleton lookup searches.** The original uses a
  registry of avatar files. Here the name must be defined by exactly one
  file.
- **Footprints, the short after the position, the frame arena size, slot
  contents, sprite flags, motives and queues.** They are the saved values.
  The captures do not hold them, or were not decoded for them.
- **The other 110 changed flags and the rebuilt type table**, as before.

What is not modeled and fails loudly instead: a house version outside 53
to 62; a floor or wall map with entries; a wall cell the renumbering
would change; an object with graphics of its own; a person with no
neighborhood record, for which the original makes one; a skeleton that no
file or two files define; saved attributes beyond what the constructor
allocates; and reading objects onto a lot that holds objects.

What is not modeled and not compared: the lot's lighting, which the array
read resets; object flags; a person's animator, saved animations and
built-in slot positions; and the game's count of queued interactions.

How the earlier states were kept. The states pinned for the resize and
the first clear are the old encodings plus the two bytes of the new,
empty fields. The gated tests check that: without those bytes each
hashes to its format 4 digest.

What would reopen this: a house with custom floors or walls; a house
whose people have no neighborhood record; a capture that holds the cells
outside the lot's size; or finding what sets bit 15 of a frame's tree ID.

## 2026-11-13: Clearing the lot deletes objects one at a time, and the type table is read before the objects

Load step 5 removes every object the lot holds before the house's own
objects are read. Removing is not dropping: each object leaves the lot,
resets one last time, and is then unlinked. The reset runs the object's
Reset tree, a script, so the clear can change globals, draw random numbers
and queue removals. This entry also corrects the order of steps 6 and 7 in
`docs/roadmap.md` and says where the load now stops.

What was decided:

- **Deletion is ported for the simple case** (`crates/sims-vm/src/removal.rs`).
  `Interpreter::remove_now` takes an object off the lot and deletes it.
  `Interpreter::delete` releases the ownership of interface events, runs
  the last reset, and unlinks the object from the registration list and
  from its type's instance list. `Interpreter::reset` runs the Reset tree
  on top of the object's thread and then restarts the thread at Main; an
  object with no Reset tree only leaves the lot. `Interpreter::clear` is
  step 5: the two indicators first, then the head of the registration list
  until it is empty.
- **The removal queue keeps IDs whose objects are gone.** The original's
  queue holds bare IDs. The clear does not touch it, and the load erases
  it later (step 11). So after the clear `World::removals` still names 42
  and 44.
- **An instance list outlives its instances.** A type's list in
  `World::instances` stays, empty, when its last object is deleted. An
  object read or created later is appended to it.
- **`World` holds the indicators' IDs** (`World::indicators`, 0 for none).
  The clear needs to know which objects they are, and it sets both to 0.
- **The changed flag is state of the running game.**
  `World::changed_types` holds the GUIDs of the types whose scripts
  changed since a house that listed them was saved. The original keeps
  the flag on each definition. Reading a type table sets or clears it for
  the types the table lists and leaves the others alone.
  `Interpreter::read_types` does that and rebuilds `World::types`.
- **Steps 6 and 7 run in another order than the roadmap said.** After the
  clear the original reads the lot arrays, renumbers the saved floors and
  walls, reads the type table and marks the changed types in the same
  pass, clears the lot a second time, and only then reads the objects. The
  roadmap had the changed types after the objects. The step table now
  lists 6a to 6e.
- **`World::from_house` is split.** `World::read_arrays` and
  `World::read_objects` read onto the lot the load holds and keep what the
  earlier steps left. `from_house` stays as a way to look at a house file.
- **`sims boot` stops after the lot arrays.** The next thing the original
  does, renumbering floors and walls, is not ported.
- **The snapshot format is 4.** The two new fields are at the end of the
  encoding.

How it was checked. The old prototype's harness captured the original at
the moment the clear returns, loading the Goth house. Our state was
compared with that capture outside the repository. Equal: no objects, an
empty registration list, the tick, the lot size, the 64 globals, both game
random streams, the queued removals (42 and 44), the scheduler cursor, the
funds, both indicator references (0), the one instance list (empty), and
the type attribute records' GUIDs, order and lengths. Eighteen controllers
ran a Reset tree on the way. The test
`the_goth_house_clear_removes_every_object` pins a digest of our state.

The state when the resize returns was not compared again. Its encoding is
the old one plus the three bytes of the new fields, and without those
bytes it hashes to the digest the earlier comparison pinned.

For the type table, a capture taken after the original had read it shows
the three Goths' own types: an Init signature of 20 for each, the changed
flag set, and a Main signature never worked out. We get 20 and the flag
for all three. That also confirms the rule that Main is only looked at
when Init matches.

What did not match, and why. The same things as when the resize returns,
and for the same reason: the C runtime's random stream and six type
attribute words belong to the game that was running. With those copied
from the capture into the lot before the resize, they match after the
clear too, so no Reset tree changes either one here.

What is **unverified**:

- **The lot arrays** (`World::read_arrays`). No capture covers the
  arrays after they are read. Which arrays are read, and in what order,
  comes from the old prototype's reading of the code. A missing array is
  an error here; the original stops reading arrays and carries on.
- **The other 110 changed flags and the rebuilt type table.** Ported from
  the old prototype, which compared its version with the original's code
  run alone on made-up records.
- **`World::read_objects`.** No capture covers the object read. It copies
  saved fields and does not construct anything, so it is not a load step
  yet and `sims boot` does not call it.
- **Deletion paths the 143 objects do not take.** A Reset entry whose
  tree is missing, a Reset tree that removes or creates objects, and an
  object that owns interface events are ported from the old prototype and
  covered by synthetic tests only.
- **The reset outside a deletion.** `Interpreter::reset` with a Reset
  tree is the same code path, but nothing calls it that way yet.
- **The leftover type table.** After the clear `World::types` still lists
  the controllers' types. That table is our bookkeeping, and the type
  table read replaces it.

What is not modeled and fails loudly instead: removing an object that is
on the lot, in a container, holding something, multi-tile, a person, or
has a callback for leaving the lot; deleting anything while a person is on
the lot, because people must drop their references to it; resetting an
object with no Reset tree outside a deletion, which rebuilds and replaces
it; reading a type table onto a lot that holds objects; a lot array that
is missing or not 64 by 64; and a house with a GUID translation table.

What is not modeled and not compared: the interface being switched off
during the clear, the objects' graphics, the pools of spare indicators,
the redraw of the room a removed light was in, the deletion mark on the
object (only a person's reset reads it), type ID 0 in the rebuilt table,
and type tables older than version 2.

What would reopen this: a capture taken after the arrays or the objects
are read; a lot whose objects are on tiles when it is cleared; or a house
with custom floors or walls, which makes the renumbering matter.

## 2026-11-13: A load starts from the lot the game holds, and resizing it creates the controllers

The original keeps one lot for as long as it runs and loads each house into
it. Before it reads a house's objects it reads the house's simulator state,
resizes the lot to the house's size, and that resize creates every lot-wide
controller the lot lacks. It then removes every object and only then reads
the saved ones. So 141 controllers run their Init and first turn on a
nearly empty lot, draw random numbers and write globals, and are thrown
away. The load sequence in `docs/roadmap.md` now starts with these steps.
This replaces the ten-step sequence of "The lot load follows a written
sequence" below, which read the whole house first, and that entry's mode 0.

What was decided:

- **`World` is the lot the game holds, not a house.** `World::empty` is the
  lot before any load, `World::read_simulator` reads a house's simulator
  state into it, and `Interpreter::resize` shrinks it. `World::from_house`
  still reads a whole file at once; it is no longer the load path and will
  be split when the saved objects are read onto the cleared lot.
- **The object factory is ported** (`Interpreter::create`): lowest unused
  ID, head of the registration list, the constructor's storage, the date
  stamp, the reset, Main installed, Init run synchronously above it, one
  first turn, then the scheduler flags and the price.
- **Sleep yields.** An object that is not a person and not locked out
  hands its count to the scheduler, which is the low half of the registry
  word. `Interpreter::turn` runs a thread until it yields or returns.
  Resolving data word 25 wakes an object, as in the original.
- **New state.** `World` gains the C runtime's random stream, the lot's
  size, the objects waiting for removal, the type attributes, the per-type
  instance lists, the owner of interface events and the selected family's
  magic history. `Object` gains its last result and a removal flag. The
  snapshot format is 3.
- **Frames of new objects need a type ID.** A frame names its code by the
  lot's type table, which the original only builds when it saves. A type
  the table lacks takes the lowest unused ID. This is our bookkeeping; no
  original state corresponds to it.
- **The whole catalog is linked.** The resize walks every definition, so
  `sims boot` links them all. `Program` also holds each file's interaction
  tables and slot counts, the career list from `work.iff`, and the
  neighborhood's lot zoning.
- **The context's mode is 1.** `apply_context` no longer clears the family
  in global 9; `World::clear_family` does, after the resize, because the
  controllers must still see the saved family.

How it was checked. The old prototype's harness captured the original as it
loaded the Goth house, at the moment the resize returns. Our state was
compared with that capture outside the repository. All 141 controllers
match: GUID, the 72 data words, temps, attributes, last result, every
frame, the registry word and the position. So do the 64 globals, both game
random streams, the tick, the lot size, the registration order, the two
controllers waiting for removal, the scheduler cursor, the funds, the
instance lists, and the type attribute records' GUIDs, order and lengths.
The test `the_goth_house_resize_creates_the_controllers` pins a digest of
our state and runs when `SIMS_INSTALL` is set. The capture is private.

The mode is 1 on the capture's evidence: at the start of the resize, before
any script has run, global 15 is 1 while the house saved 0.

What did not match, and why. Three things in the capture belong to the game
that was running, not to the house or the install:

- The two indicator objects. Ours are created on an empty lot whose clock
  is zero, and have just gone to sleep. The capture's carry another
  creation date and had been asleep for a while.
- The C runtime's random stream, which no file holds.
- Six type attribute words, one for each help system, which are set in
  the capture and clear in the neighborhood file.

With those copied from the capture's state at the start of the resize, the
two indicators match as well, which makes all 143 objects. The controllers
match either way.

What is **unverified**:

- **The starting lot.** The creation date and time the capture's
  indicators carry is exactly the clock saved in `Houses/House00.iff`, the
  empty lot every neighborhood ships with. So the original most likely
  loads House00 when it starts, and the indicators come from that load. We do not model it: `sims boot`
  starts from `World::empty`. This cannot change what a load leaves behind
  unless a controller reads the indicators' state, and none did here.
- **The six type attribute words.** Something in the running game set
  them before the load. We do not know what. The 141 controllers end in
  the same state with either value, but later ticks may not.
- **The C runtime stream's starting state.** It is 1, the runtime's own
  default. Nothing in this step draws from it.
- **Fields the capture does not hold.** A new object's footprint, the short
  after it, its frame arena size, its relationship flag and its slots are
  set to what the original's saves hold for such an object.
- **Paths the 141 controllers do not take.** Sleep, waking, Remove Object
  Instance, Generic Sims Call 28 and 43, event ownership, Set To Next
  modes 4 and 10 and the type attribute scopes are ported from the old
  prototype. The capture confirms the paths these controllers take through
  them, not the others. The original keeps an object with a headline
  showing out of the scheduler's hands while it sleeps; headlines are not
  modeled, so that test is missing.
- **The turn budget.** Init and the first turn may each run 100,000
  instructions, the old prototype's guard. The original's limit is not
  known.
- **Which zoning file the original reads.** We read the neighborhood's
  `LotZoning.iff`; the old prototype read the install's
  `MasterLotZoning.iff`. Both give the same table for the shipped
  neighborhoods.
- **Global 25 and free will in the context.** The capture agrees with
  them, but the house saved the same values, so it proves nothing.

What is not modeled and fails loudly instead: growing a lot; resizing a lot
that has arrays or objects standing on it; a second instance of a
controller, which needs deletion; removing an object that is on the lot,
holds or sits in something, is a person, or has a callback for leaving the
lot; the factories for people, cursors and multi-tile objects; Generic Sims
Call routines other than 28 and 43, and 43 once a family's magic history is
held; a type that wants its attributes cleared for each lot.

What is not modeled and not compared: the lot's rooms and their tile
counts, the interface's action counter, and the interface's lists of
indicators. The resize only writes the indoor area of a bare lot, which is
0, to global 27.

What would reopen this: a capture of the original at startup, which would
settle the starting lot and the type attribute words; a house whose
controllers take the unverified paths; or a load into a lot that is not
bare.

## 2026-11-13: The catalog follows the original's scan order and file tables

Objects register in the order the original scans for them (0x43c609):
`GameData/Objects`, `GameData/UserObjects`, the neighborhood's
`Characters`, `Downloads`, the packs in bit order up to Superstar, the
shared directories, and Makin' Magic last. `Index::add_characters` puts
the people in that place. This replaces "characters register after the
install" below.

Within a directory the original does not use the file system's order. It
keeps each directory's entries in a hash table keyed by lower-case name and
walks the table (0x5ba9bd). `sims-content`'s `file_table` reproduces the
hash, the bucket counts and the growth rule, ported from the old prototype.
An archive's members are entries of the directory the archive is in, and a
member whose name holds a path goes into that subdirectory, whether or not
it exists on disk. The first entry of a name wins, except that a file on
disk replaces an archive member. This replaces "directory order is NTFS
name order" below.

It is checked against the original. When the Complete Collection loads a
lot, it creates the lot-wide controllers the lot lacks by walking its
catalog. A capture from the old prototype's harness holds the 141 objects
that made, in order. Picking the same definitions from our catalog gives
the same 141 GUIDs in the same order. The test
`the_catalog_lists_controllers_in_the_originals_order` pins a digest of
that order and runs when `SIMS_INSTALL` is set. The capture itself is
private.

What that leaves **unverified**:

- The order of two names that share a bucket, and which of two archives
  supplies a member both hold. Both depend on the order Windows lists a
  directory in, for which `Index` uses NTFS name order. The 141 controllers
  show no such case.
- Where the people fall. No controller is a person, so their place rests on
  the old prototype's reading of the scan.
- Houses saved by the original list their object types in catalog order,
  and those lists disagree with the capture in one way: they show larger
  tables for some directories (389 buckets for the base game's objects,
  not 193). The studio's machines presumably had more files in those
  directories. The capture is from the install we support, so it wins.

## 2026-11-13: The lot load follows a written sequence and stops at the first gap

The load sequence in `docs/roadmap.md` lists the ten steps the original runs between
reading a house file and the first tick, with what `sims boot` does for
each. `sims boot` runs them in order and stops at the first step that is
not implemented, so the hash it prints is always of a state the original
passes through. The alternative, running whatever callbacks already work,
would give a state the original is never in, and its hash would mean
nothing to the comparison harness.

The first step added is the globals the running game supplies
(`World::apply_context`). The values for a Complete Collection install
(mode 0, global 25 zero, free will on, expansion mask 0xff, performance
class 2) are the old prototype's headless profile and are **unverified**.
The performance class comes from the machine's memory and CPU speed, so
the comparison harness must read all five from the running original.

Writing the sequence down showed that the catalog order was wrong for load
step 4, where new objects get IDs in catalog order. The entry above fixes
it.

## 2026-11-13: Init primitives stop at state the lot does not hold

Test Object Type, Set To Next, Refresh, Play Sound Event and the stack
object's definition words (scope 21) are ported from the old prototype.
`Program` now keeps every OBJD as the 108 words the original holds after
registration: versions 136 and 137 become 138, and every record is cut or
zero-padded to 216 bytes (0x431759), so every scope 21 index under 108
reads. Definitions are listed in the file's native resource order, as the
catalog registers them; the prototype used physical file order.

Refresh mode 0 only redraws, so it does nothing here. Modes 1 and 2 mark
the object's room dirty, which makes the next room update rebuild the
room's score and lighting. Rooms are not modeled, so those modes fail
loudly unless the object is in no room (data word 29 is -5). This is now
the most common init failure on the Goth house. Play Sound Event only
queues audio, so it takes the true branch and changes nothing.

Set To Next walks registration order (modes 0, 2 and 6), people (1),
multi-tile groups (3) and families (12). Mode 4 fails loudly: the
original keeps a per-GUID list of instances for the whole process. The
list is filled in ID order on first use and appended to as objects
register, even across lot loads, and `World` has no such list. Modes 5
and 7 to 11, and unknown modes, also fail loudly. Three choices are
**unverified**:

- The person vector is the people in registration order. That is how
  loading builds it; Sims joining or leaving may change it.
- A multi-tile group's chain is its head, then its tiles in registration
  order, which is how loading links them (0x423223). `World` keeps only
  each tile's head, so splitting or joining groups at run time would need
  the links themselves.
- Test Object Type on a tile whose group has no catalog master, and Set
  To Next mode 6 in a tree with no parameters, fail loudly. What the
  original does there is not known.

The three load callbacks that fail with "tree 0x100b is missing" are not a
linking bug. `FlowersOutdoor.iff` OBJD 16808 has no OBJf, so its callbacks
come from the OBJD itself, and its load field names 0x100b. The file has
no such tree and no semi-global file. The prototype found that the
original skips a callback whose tree is absent (0x4749b5). Whatever runs
callbacks will have to do the same.

## 2026-11-13: The interpreter runs on the saved frames

An object's thread is its stack of saved frames in `World`, so the
interpreter keeps no state of its own and every snapshot holds a resumable
stack. Frames name their code owner by objt type ID, so `World` now keeps
the lot's type table (`types`, type ID to GUID; snapshot format 2).
Expression, Random Number and the operand scopes are ported from the old
prototype with its native error codes. Scopes over state `World` lacks
(tree advertisements, definition words, rooms, neighbors, jobs,
neighborhood data, type attributes) and every other primitive fail loudly.
Four semantics are **unverified**:

- New frames start with zeroed locals. The original reuses a frame arena,
  so locals inherit words left by earlier frames; saves carry no arena.
- Random Number draws from a stream the caller names. The original swaps
  the lot stream in only around the lot update (0x46d553).
- Motive reads round half to even (the x87 default); the prototype rounded
  half away from zero.
- Motive arithmetic rounds once from the x87's 64-bit significand to 32
  bits. The FPU precision the original runs it at is not known.

## 2026-11-13: Linking is checked against saved behavior signatures

Each object type in a save records two signatures: for its init and main
trees, the root's revision word shifted left 16 plus the revisions of every
tree reachable from the root (0x4026ed, recovered by the old prototype).
`sims boot` recomputes them from the linked program. Across 296 shipped
houses, 37,655 of 47,359 saved types match. In the best houses nearly all
match, and the misses differ by a revision or two, as when a later pack
revised a tree after the house was saved. A linking mistake would give
unrelated numbers.

On the Goth house only 36 of 113 types match: it was saved by the base
game, and the expansions revised most of its objects' code. The original
resets a saved object's script state when its signatures differ
(0x43a516), so booting this house exercises that reset for 77 types. The
interpreter has to implement it before the first tick.

## 2026-11-13: The Complete Collection ISO is the supported install

Decided by the maintainer. The vision named the Legacy Collection as the
primary install; it now names The Sims Complete Collection, installed from
its ISO, and leaves other editions out of scope. That is the install the
emulator image already holds and every check so far ran against, so no
code changes. It replaces the entry below on recognizing only the CD
layout: that is no longer a gap.

## 2026-11-13: The state hash is SHA-256 of the postcard encoding

`World` is encoded with postcard: fields in declaration order, integers as
little-endian varints, floats as raw bits (NaN payloads and negative zero
kept), and maps in key order. Nothing in it depends on the platform, and a
pinned-hash test runs on both CI platforms. Snapshots store the same bytes
behind a 4-byte format number, in a REDB table keyed by name. Changing the
encoding means bumping `snapshot::FORMAT` and the pinned hash. Comparison
with the original goes field by field, not through this hash: the hash only
proves that our own runs repeat.

`World` keeps every field the readers decode for objects, the simulator
and the lot arrays. It leaves out bytes nobody understands yet (format
tails, raw ObjM records), the HOUS view and model, and the wall and floor
chunks. They join when the interpreter or the comparison needs them.

## 2026-11-13: The target house is the Goth house

The boot goal needs one house. It is `House05.iff` in the base game's
shipped neighborhood: the Goth family (Mortimer, Bella and Cassandra), 245
saved objects, SIMI and ObjM version 0x3e. The installed copy is byte for
byte the shipped one, so the emulator can load it without copying files in.
The Newbie house (`House07.iff`) is smaller but was played and saved by the
old harness, so the installed copy differs from the shipped one. Reopen if
the Goth house needs something the harness cannot capture.

## 2026-11-13: Placeholder objects carry no registry word

Every ObjM record ended with a field-encoded registry word, until the
expansion houses: the `*Placeholder` object (GUID 0x3C08965F) never has one.
Across 139,000 ordinary records in the installed and shipped houses, 24
to 31 bits remain after the dynamic sprites. Across 27,000 placeholder
records, 0 to 7 remain, so no field fits. The reader now reads the word only
when at least a byte of the record remains, and stores None otherwise.
Why the original writes no word is unknown; it may skip objects missing
from the scheduler's registry. Unverified until the harness compares a
placeholder.

## 2026-11-13: Neighborhood characters register after the install

Superseded by "The catalog follows the original's scan order and file
tables" above: characters register after the user objects.

A house's Sims are defined by files in the neighborhood's `Characters`
directory. `Index::add_directory` appends that directory as one more mount,
after Downloads, so the catalog registers characters last. Where the
original registers them is unverified. A family with house number 0 has no
house: in the shipped and installed neighborhoods every such family lacks
flag bit 0, and every family with it has a nonzero house.

## 2026-11-13: Some installed houses do not load yet, on purpose

`sims houses` loads 660 of the 718 house files in the install's eight
neighborhoods and the shipped one. The 58 failures:

- 18 are `House00` and `House11` in each neighborhood: base-game templates
  with ObjM versions 0x33 and 0x38, whose record layout is not known yet.
- 40 are houses 56, 63, 69, 76 and 77, which hold Sims whose character files
  are not in the neighborhood: Superstar's community templates, and houses
  the old harness saved. The original presumably repairs or drops such residents,
  which we have not reverse engineered.

Neither blocks the target house. Reopen when a goal needs those houses.

## 2026-11-13: Later GUID registrations win; semi-globals resolve to the first match

The catalog registers object definitions in load order, and a GUID resolves
to its last registration. The old prototype recovered that rule from the
original's lookup. Multi-tile masters are the exception: the first
registered master wins, and that is for the interpreter to apply. A GLOB
link resolves to the first file of that name in load order. That part is
unverified: the Complete Collection has no duplicate semi-global names, so
nothing tells the two rules apart yet.

## 2026-11-13: Directory order is NTFS name order (unverified)

Superseded by "The catalog follows the original's scan order and file
tables" above: NTFS name order now decides only the order of names that
share a bucket.

The original lists each mounted directory with FindFirstFile, and that order
decides the object catalog order. On Windows/NTFS it is name order with case
folded. `Index::build` uses that order and searches a subdirectory where its
name falls among the files. Neither detail has been confirmed against the
original's walker yet. The old harness ran the game under Wine on ext4,
whose directory order can differ from NTFS. The comparison harness must
either record the order the original saw and feed it to us, or put the
install on a file system that lists in NTFS order.

## 2026-11-13: Expansions come from the pack directories

The original reads the installed packs from the registry (`EPxInstalled`,
`SIMS_GAME_EDITION`). We have no registry, so a pack counts as installed
when its directory holds a FAR archive. On the Complete Collection install
this gives edition 0xFF, the same as its registry.

## 2026-11-13: Only the CD layout is recognized for now

The CD layout is checked against a real Complete Collection install. The
Legacy Collection's layout is not known here, and a guessed table would not
be tested by anything. It is added when someone can list a real Legacy
Collection install.

## 2026-11-13: Readers stay strict on unreferenced bad chunks

21 TTAB chunks in the original install are version 3 or zero bytes long.
Nothing references them. The reader keeps rejecting them, and loaders
decode only the chunks they use. Reopen if a referenced chunk ever fails.

## 2026-11-13: SIMI versions 0x33 and 0x38 use the later layout (unverified)

Only the template houses carry these versions. Their fields up to the budget
read plausibly with the 0x3e layout, and 0x33's unread tail is 24 bytes
shorter. Check the fields against the original when the harness loads one
of these houses.

## 2026-11-13: SPR# versions 503 and 505 share the 502/504 frame encoding

All 14 such sets in the install decode exactly with it. What the version
number itself selects is unknown.
