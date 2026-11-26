# Walk animator states 5, 7, 8 and 12, the sidestep, and clip sounds

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:** `crates/sims-vm/src/animator.rs`, new `crates/sims-vm/src/footsteps.rs`,
  `clip_events.rs`, `sounds.rs`, `interpreter.rs` (`floor_categories`), `lifecycle.rs`
  (`LotPrices` floor categories), `placement.rs` (read-only `container`); tests
  `tests/interpreter/walk_states.rs`, `tests/interpreter/clip_sounds.rs`, `animate.rs`,
  `tests/integration/boot.rs`; `sims-game/src/sound.rs` (the emitter's sign). No change to
  `sims-sim` or `sims-audio`.
- **Track:** Track B (walk animator states, footstep and clip sounds)

All addresses below are from the original executable, read from the memory image
(`cache/harness-runtime/sims-image-00400000.bin`).

## Decision 1: the animator's remaining states

The state jump table at 0x58faa2 has 13 entries. States 5, 7, 8 and 12 are now ported. Any
state above 12 (an unsigned compare, so negative states too) goes to the tick's tail
(0x58dd73), as before.

- **State 3, stand (0x58ddbf to 0x58e043).** A coordinate equal to -999 places the animator
  (0x58dd8f, `sahf; je`). An unordered compare sets ZF, so a NaN coordinate counts as unset
  too; the placing (0x592160, `jb`) then leaves it, and the tick ends. Otherwise state 3 now
  decides between a turn, a snap and a sidestep. The direction is wrapped first. Then:
  - Distance: `sqrt(dy² + dx²)` at single precision. It is 0 when both offsets are within
    epsilon.
  - Turn: `d` is wrapped into [-4, 4). A NaN `d` traps at 0x58e039.
  - Thresholds: by kind (person word 65). Above 1 (a pet), the turn step is `1/16 / rate`,
    the snap 0.5 × 0.2 (0x6d07e0, 0x66d72c) and the short 3.1 × 1/12 (0x6d07b0,
    0x6693ec). Otherwise they are `0.25 / rate`, 0.5 and 3.1.
  - A turn past the step turns (0x58f4ee).
  - A destination nearer than the snap is taken at once.
  - One at least the short distance away turns.
  - Anything else sidesteps.
- **The sidestep (0x58f321 to 0x58f4e7):**
  - **Clip:** the angle `atan2(dy, dx)` picks one of 8 sectors, by
    `floor(frac(angle / 2π + 1/16) × 8)`. Table 0x58fad6 gives clips 15, 13, 11, 12, 14,
    16, 18 and 17.
  - **Destination:** the clip's ROOT bone's last translation (vfunc 0x124, 0x588efb) is
    turned by the angle and taken off the destination.
  - **Layer weight:**
    - 0.5 if the distance is below 0.1 (a double);
    - `distance / |root|` below 1;
    - otherwise 1;
    - and at least 0.5.
  - **Sector 8.** A share just below a whole number has a fraction that rounds to 1, so
    sector 8, which is clip -1 (0x58f363). Clip -1 has no name (string 0 is null at
    0x4704fa), so there is no displacement and nothing starts (0x592733). The foreground
    already playing goes on, with the stop's end mode, the speed and the weight. With no
    foreground the update traps at 0x58f4d3, where the original reads through a null pointer.
  - **Errors:** a ROOT motion that is not the clip's first traps at 0x596b60. The original
    reads that motion's own frame count (+0x14, 0x588f51), and `sims_content::Clip` keeps
    only the first motion's.
- **State 5, sidestep under way (0x58e084).**
  - It eases the position as a turn does.
  - At phase 1 it stands at the turn's end, with no priority raise.
  - With no foreground it traps at 0x58e099.
- **State 7, the backward stop (0x58e251).**
  - The reversed stop clip eases from where the stop began (`turn_from`) to the destination.
  - At the end it raises the priority, stands, and sets the arrival from +0x288: 1 → -1,
    2 → 2, 3 → 3.
  - With no foreground at the end it traps at 0x58e2c2.
- **State 8:** sets walking and is done.
- **State 12:** an uninterruptible foreground sets the layer speed to the rate. Otherwise
  the state goes to 3.
- **The special stop (0x58ef8e).** It is entered from the end of the path (0x58e83b), the
  timed stop (`slow_down`), and a stop asked for under way (0x58ef2f).
  - Not walking: the person stands at once, arrival by +0x288.
  - Walking: state 7, with the stop clip (selection 0x13) reversed. A name no animation file
    defines is a null clip (0x58f036): nothing starts and the foreground goes on. A lookup that
    is ambiguous (names differing only in case) still traps.
  - **The walking flag:** the original tests +0x290, which is not kept. Adding a hashed
    field would move every pin. Entries (a) and (b) use `walking` and trap at 0x58ef8e for
    a kind above 1, the one case where the two can differ. At entry (c), +0x290 is always 1
    (0x58edec sets it).

## Decision 2: the clip sounds (0x5912fd, 0x591505 to 0x59175e)

The sounds are picked in the new `footsteps.rs`, and `clip_events.rs` notes them in the
interpreter's sound sink as `SoundEvent::Play`. The object is the person (+0xf0) unless
noted.

- **"sound":** plays for the person when it stands on the lot (0x432595: its tile inside the
  lot's border). Off the lot, it plays for object -4 when person word 65 is above 1, and
  otherwise not at all.
- **"selectedsound" / "deselectedsound":** play on the selected mark (0x45e50e: object +0x114
  bit 1, `Object::selected`) or its absence.
- **"footstep":**
  1. An animator at x = -999 (0x671edc) is passed over, using 1e-6 (0x67343c) after a
     single-precision subtraction.
  2. The surface comes from 0x58900f. Either coordinate below 0 (or NaN) gives -1, which
     plays nothing. Otherwise the tile is `ftol(coord + 0.5) >> 4` as signed bytes
     (0x666578 = 0.5), on the person's level (+0x110).
  3. The precedence (0x5890a8):
     1. Level-0 water would give 10, but 0x478f4a answers only for level 1, so it never
        does.
     2. Otherwise the highest-ranked uncontained object on the tile wins:
        - 6: 0x96e3398d, 0xa2daa08c, 0xc02b406a, 0x65274a4f
        - 7: 0xbf62f653, 0x7f907075
        - 8: 0x63416ba1
        - 9: 99999999
        - 10: 0x9be030b9, 0x3e7470f6, 0x5c67fc8f

        The tile is walked as 0x43f331 / 0x43f379 walk it.
     3. Otherwise, on level 1, the pool array (`Arry 9`): below 16 gives 11 (0x478f76),
        16 gives 1 (0x478fa3).
     4. Otherwise the floor pattern (0x4330b7): 0 gives 0 and 255 gives 2. Any other
        pattern goes by its category (0x4a1463): 0→3, 1→2, 2→4, 3→1. A key the catalog
        lacks counts as category 2.
  4. **Name** (jump table 0x5917bf):

     | Surface | Name |
     |---|---|
     | 1, 2 | `hard` |
     | 3, 5 | `medium` |
     | 4 | `soft` |
     | 6–11 | `plant`, `trash`, `ash`, `roach`, `puddle`, `pool_swim_stroke` |
     | anything else | the ground: `snow` on house 40–42 (0x4c471b); else `soft` on house 99; else `terrain` |

     `_noshoe` is added to hard, medium, soft and terrain when the suit last dressed
     (animator +0x1b0, `PersonOutfit::last_dressed[0]`) is 1, 2, 5 or 14. Every name starts
     with `footstep_`.
  5. **Silenced:** nothing plays when person word 84 bit 0, word 68 bit 0, or word 74 bit 3
     is set.
  6. **Direction:** the event's layer speed must agree with the step's sign. A speed of at
     least 0 needs a step of 1 or 2. A speed below 0, or NaN, needs -1 or -2.
- In a person's animator refresh at its take-off (0x58da12), a sound key plays nothing. The
  refresh is made only if the house view drew the person, which is not kept, and a sound
  writes nothing the lot reads, so both branches of the refresh stay alike. The refresh's
  update still takes the due event past, so the tick after it plays nothing either. The
  take-off's both-ways check therefore compares every cell but the sounds
  (`Cells::same_but_sounds`), and keeps the sounds of the tick alone. Whether the original
  plays a sound in the refresh is unverified. It confirms that a stop widens with its sign
  too (0x45255f, 0x4323c1).

Playback already worked: the game forwards `Update::sounds` to its sound thread, and
`sims_audio::Player::play` resolves any event name against the loaded modules. One fix was
needed: `sims_game::sound::event` now widens the 16-bit object with its sign, as the
original's play and stop do (0x57a60e, 0x45255f, 0x4323c1, `movsx`). So -4 reaches the player as -4, which places it at
the screen's centre at full volume, instead of as 65532, an unknown object that would keep
the previous placement.

## Decision 3: floor categories, the base game's file only

The original keeps each floor pattern's category in its catalog entry (+0x2c, low byte). It
comes from the first byte of the label of the pattern's `SPR2` (0x49fd95): 'C' 3, 'M' 1,
'S' 2, anything else 0.

- `sims_content::SurfaceCatalog` does not keep the label, and this track does not own
  sims-content.
- So `LotPrices::load` reads the categories of `GameData/floors.iff`, keys 1 to 29, which is
  one extra file at load. A key with no sprite is still in the original's catalog, made with category 0 (0x4a0722, inserted at 0x4a029a).
- A footstep on a plug-in pattern has no category here and plays nothing. Silence is the
  honest answer; a guessed sound would not be.
- Every interpreter a booted `Lot` makes carries the table (`floor_categories`, or through
  `architecture_prices`, as co-op's do), so a refresh outside the tick (selecting a Sim)
  hears the same floors. A `Machine` passes its table on to the nested interpreters its
  services make (the Global 3 selection among them). `Interpreter::floor_categories` also
  overrides the table for tests.

## Decision 4: sounds never touch the simulation

- Picking a sound reads the world and writes nothing. None of 0x57a54c, 0x58900f, 0x5890a8,
  0x432595 or 0x45e50e calls the C runtime's `rand` (0x636c74), directly or through direct
  calls five deep.
- The original's place lookup (0x4325b5, reached from 0x432595 and from 0x43265e in the
  object walk) rewrites the slot word of an object linked first on its tile with nothing
  before it. The footstep and sound paths use a read-only `placement::container` and leave
  that word alone. This keeps the sound sink out of the hash. It differs from the original
  only on a lot already in that inconsistent state.
- A lot without the pool array gets no pool surface. 0x478f76 tests for the array; 0x478fa3
  does not.
- A pick that cannot be made plays nothing and never fails the event. Examples are a tile
  off an array (x 1023.5 rounds to tile 64), where the original reads whatever memory lies
  there, or a level with no floor array. A trap there would reach play mode's reset and
  change the lot.

## Evidence

- **`tests/interpreter/walk_states.rs` (18 tests):** the snap; the sidestep with root motion
  (destination, weight, priority, end); all 8 sectors and the boundary; sector 8 (clip -1,
  with and without a foreground); the weight cases; a ROOT not first (0x596b60); a NaN
  coordinate in state 3; a stop clip no file defines; a far turn; the pet bounds (kinds 2 and 3) against kinds 0 and
  1; states 8, 12, 13, -1 and 100; the backward stop with its reason mapping; the
  special-stop entries (a), (b) and (c).
- **`tests/interpreter/clip_sounds.rs` (9 tests):**
  - every floor category, plus 0, 255 and an unread pattern;
  - `_noshoe` for each bare-footed suit;
  - snow, soft and terrain ground;
  - pool and pool edge before the floor;
  - object ranks, with a held object passed over;
  - direction and silencing words;
  - off the arrays;
  - "sound" on and off the lot, by kind;
  - selected and deselected;
  - world hash identical with or without the sink and categories, with the slot word
    unchanged.
- **Install-backed (release):** `the_goths_footsteps_sound_of_what_they_walk_on` steps
  House05 for 400 ticks, asserting `TICK_33` on the way and that footsteps play. The
  sims-vm integration suite passes with every strict pin unchanged: TICK_33, TICK_43200,
  NEWBIE_*, OLD_TOWN_69*, IMPORTED_69*, and the tick fuzzer.

## Follow-ups

- **Co-op sounds:** every co-op command (pick, mode, the host's selection) runs on its own
  sound buffer and drops it, as before this track. A selection's footsteps there are not
  heard. The co-op step needs an output for them.
- **Travel and household moves:** arrival, home reconciliation and eviction run interpreters
  with no sound sink and no floor table, so a refresh there plays nothing.
- **sims-content, per-motion frame counts:** these would lift the 0x596b60 trap for a ROOT
  motion that is not a clip's first.

- **sims-content:** keep each floor pattern's category (and colour, +0x2c) in
  `SurfaceCatalog`, so plug-in floors' footsteps play.
- **The +0x290 walking flag:** a kept field would lift the pet trap at 0x58ef8e. It needs a
  pin rebaseline.
