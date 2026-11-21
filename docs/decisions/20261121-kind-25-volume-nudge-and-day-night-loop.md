# Kind 25, the volume-nudge countdown, and the day and night loop

*Decided 2026-11-21.*

Kind 25 (sound active), the sound manager's `+0x114` volume-nudge
countdown, and the day and night background loop (`0x574978`) are now
ported in `crates/sims-audio`, together with `0x575155` (`MainMusicLoop`
let-go on unpause and kind 25 on). This supersedes the "A quirk not
ported" note in `20261115-camera-move-places-every-track-again.md` and
the "Not ported" note in
`20261115-pausing-holds-sounds-playing-originals-sound.md`.

## The original

- **Startup and kind 25's sender.** The sound manager constructor sets
  `active` (`+0xc`) to `1` (`0x5712e9`) and `nudge` (`+0x114`) to `0`
  (`0x571433`). The only caller of kind 25 (`0x19`) in the binary is
  `0x4f7608` (at `0x4f76d5`), vtable slot `+0x20` of the application
  subobject at `+0x48` (`0x669810`), called by `WM_ACTIVATEAPP` (`0x1c`,
  `0x5d6ccb` → `0x5d6e2e`). The application constructor initializes that
  subobject's active byte (`+0xbd`, application `+0x105`) to `1`
  (`0x4eee14`), and `0x4f7610` returns immediately when the incoming
  flag equals it. Startup therefore sends **no** kind 25; only focus
  loss (`0`) and regain (`1`) do.
- **Kind 25 (`0x573c64`).** Stores `arg0 != 0` into `active` (`+0xc`):
  - When turning sound **on** (`0x573c71`..`0x573cc5`):
    1. sets `+0x114 = 100`, calls `0x57447f` (place every track with
       recorded instances), clears `+0x114 = 0`, calls `0x57447f` again,
       and sets `+0x114 = 100`;
    2. if not paused (`+0xd == 0`), calls `0x574904` (unpause) and
       unpauses the current ambience score (`+0x100`, vtable `+0x7c(0)`);
    3. if `music_mode == 3` (`+0xe8`), calls `0x574904` (even if paused,
       clearing `+0xd`, without unpausing `+0x100` when paused);
    4. calls `0x575155` (lets go of `MainMusicLoop`, track `515`, vtable
       `+0x5c`).
  - When turning sound **off** (`0x573cca`..`0x573d0f`):
    - in live music mode (`music_mode == 0`), pauses all four ambience
      scores (`+0x104`..`+0x110`, vtable `+0x7c(1)`). It leaves `+0x114`
      and running tracks alone (on focus loss, `0x4f7608` requests
      auto-pause only when in mode 2 with `[0x6c9fa1] != 0`,
      `0x4f7630`..`0x4f763c`; `0x4cee4a(1)` marks the pause pending at
      `0x4cee52`, and `0x4ce5c5` sends kind 21 later when that pending
      change is processed).
- **Other checks of `active` (`+0xc`).** Every read of `+0xc` in the
  sound manager (`0x5712d0`..`0x575600`):
  - kind 1 (`0x5736ee`): returns immediately when inactive;
  - kind 22 (`0x573c44`): clears `paused` (`+0xd`), then returns without
    resuming tracks, day/night loop, `MainMusicLoop` or ambience when
    inactive (`0x573c48`);
  - kind 30 (`0x573d7b`): skips `let_go` when inactive (as when paused),
    leaving a freshly started track held;
  - `0x574978` (`0x5749a1`): stops the day and night loop when inactive.
- **The `+0x114` volume-nudge countdown (`0x573019`, `0x574701`).**
  Earlier notes assumed `+0x114` stayed `100` forever after kind 25 on.
  It does not: in the sound manager's update (`0x572fd6`, at `0x573019`,
  right before `0x57cf37` steps the HIT tracks), whenever `+0x114` is
  nonzero it calls `0x57447f` (with `+0x114` nonzero), zeroes `+0x114`,
  calls `0x57447f` again (with `+0x114 == 0`), and stores `old - 1` back
  into `+0x114`. While `+0x114` is nonzero, every call to `0x5746d9`
  (kind 33 camera moves, kind 23 `stop_object` re-placement, kind 105
  `kill_all_objectsounds` re-placement at `0x574647`, and the first of
  each pair in `0x573c7f` / `0x573028`) writes `vol - 1`, or `1` when
  `vol - 1 < 0` (`0x57470c`..`0x574717`).
- **The day and night loop (`0x57492a`, `0x574978`).**
  - `0x57492a` picks the loop track:
    - if the current ambience (`+0x100`) is Beach (`+0x108`, mode 1),
      `BeachLoop` (`68259`, `[0x6ceb10]`);
    - else if the lot number (`0x571093`: global 10, `[[0x6e48bc]+0x1c]+0x24`,
      0 when no lot is loaded) is in `20..=29` (Downtown), none;
    - else if the hour (`0x570e61(0, 4)`) is `< 6` or `> 18`,
      `nite_loop` (`2985`, `[0x6ceb08]`);
    - else none.
  - `0x574978` is called from sound init (`0x5722dc`), pause (`0x574844`,
    before holding tracks), unpause (`0x574918`, after letting tracks go
    and before `0x575155`), and the shared tail `0x5741b7` of kind 33
    (camera move), kind 36 (music mode) and kind 37 (effects volume
    slider). Kind 52 (ambience mode, `0x573fdd`) does not call it.
  - When paused, inactive, or not in live music mode (`music_mode != 0`),
    `0x574978` fades out and ends (`vtable +0x34`) the remembered loop
    (`0x6f6424`) if it is running (`vtable +0x70`), leaving `0x6f6424`
    unchanged.
  - Otherwise it picks the new ID, stores it in `0x6f6424`, fades out
    and ends the previous loop if the ID changed, and if the new ID is a
    registered track that is not already running (`vtable +0x70` is
    false; a patch object's `+0x70` returns true at `0x57bfa6`), starts
    it with `[0, 0, 0]` (`vtable +0x30`), sets `extvol` to `0` (`+0x40`),
    sets `fade_dest` to the effects slider volume (`0x67156c[[+0xf0]]`,
    default `1024`), `fade_var` to `extvol` (`19`), `fade_speed` to
    `5000` (`0x1388`), `fade_on` to `1`, and lets it go (`+0x5c`).
- **Hour change sends kind 33.** The simulation clock emits signal
  `0xdb` at `0x46d4d3`; at minute zero (`[globals + 0x1a] == 0`),
  `0x4f66bc`..`0x4f66c6` calls the kind 33 wrapper at `0x57aac5`, which
  updates the ambience cursor and runs `0x574978` even while the camera
  is still.
- **Group-off start rejection (`0x57d969`..`0x57d9a5`).** At the start
  of `cHitTrack::start` (`0x57d960`), when the track's group ID is
  nonzero and that group's volume (`+0xc0`, looked up via `0x57bde7`) is
  `0`, the start returns immediately without starting, refreshing or
  restarting the track. Thus `BeachLoop` or `nite_loop` does not start
  at `extvol = 0` while effects are muted and can start fresh once
  effects are unmuted.
- **Music unpause (`0x575155`)** clears `[esi + 0xec]` (never set
  elsewhere) and calls `let_go` (`vtable +0x5c`) on `MainMusicLoop`
  (`515`, `0x203`).
- **Ambience pause propagation:** kind 52 (`0x573fd4`) and kind 36 mode
  0 (`0x57400d`) apply `paused` (`+0xd`) to the started ambience score;
  kind 36 mode 3 (`0x574072`) unpauses the backyard score.

## What changes here

- `Sources::lot(&self) -> i32` (default `0`) and `sources::Snapshot::lot`
  (filled from `world.globals[10]` in `sims_game::sound::sources`) carry
  the lot number for `0x57492a`.
- `Player::set_active(&mut self, active: bool, sources: &dyn Sources)`,
  `Player::is_active(&self) -> bool`, `Player::score(&self, mode: u32)`,
  and `kind::SET_ACTIVE` (`25`, removed from `kind::NOT_PORTED`) port
  kind 25.
- `Player::unpause` now takes `sources: &dyn Sources` so `0x574904` can
  run `0x574978`. `kind::KILL_OBJECT_SOUNDS` (105) now re-places
  remaining `<= 0` instances (`0x574647`).
- `Player::place_track` applies the `+0x114` nudge when `nudge != 0`,
  and `Player::run_at_block_start` runs the `0x573019` countdown pair
  before `Player::run`. `Player::place`, `place_track` and
  `place_through` immediately update the written thread's playing notes'
  gains (`0x57df13` → `0x57f759`).
- `Player::start` rejects starts whose nonzero group has volume `0`
  (`0x57d969`..`0x57d9a5`), returning any already-running thread so
  callers still apply placement (following `target`, and covering ended
  or idle notes when no script thread runs) and `let_go_track`
  unconditionally clears the track's hold (`0x57dc68`). `apply_levels`
  (also run on `set_group_volume` and `set_master_volume`) updates
  playing notes of ended and idle threads too so unmuting restores them.
- `Player::update_day_night` (`0x574978`, counting ended notes as still
  playing until gone) and `Player::unpause_music` (`0x575155`) run from
  the exact call sites above (`camera_moved`, `set_music_mode`, `pause`,
  `unpause`, `set_active`).
- `sims_game::sound::Feed` (used by `sims-game`'s window loop) sends
  `enter_live()` (`bkground_nhood1`, `setambience_backyard`,
  `bkground_live`, `0x4ce6d2`) when opening a residential lot directly
  into live mode, and forwards `SoundEvent::Pause` / `SoundEvent::Unpause`
  (`0x4ce5c5` / `0x4ce5d9`) whenever the lot's pause state changes, so
  backyard ambience and the day and night loop run in normal launches
  and pause/resume with the lot.
- `sims_game::sound::Mixer::input(Input::Hour)` runs `camera_moved` when
  the hour changes after its initial value (`0x46d4d3` →
  `0x4f66bc`..`0x4f66c6`), and `sims-game`'s main loop sends
  `Input::View` before `Input::Hour` so the view is current when it
  fires.
- Kind 52 and kind 36 propagate pause state to started ambience scores
  as `0x573fd4`, `0x57400d` and `0x574072` do.

## Unverified, and not ported

- **Unverified:** none of this has been compared with audio from the
  running original; all behavior is read from the memory image.
- **Update rate:** the 100-update `+0x114` countdown steps once per
  64-frame block (~2.9 ms, ~290 ms total) here, matching `Player::run`,
  whereas the original's sound timer (`0x572fc3` passing `0x14` = 20 ms
  to `0x5edc57`) fires nominally every 20 ms (~2 seconds for 100
  callbacks).
- **Not ported:** top-level `end` while a note plays (`0x57edcc`,
  tracked separately): `day_night_playing` prevents overlapping or
  leaking a replacement `note_on; end` loop script's ended note, but
  such a note still plays at its last gain until it ends or is cut on
  switch rather than fading out at `end` as the original does.
- **Not ported:** `Player::set_group_volume` does not run `0x574978`
  (kind 37 in the original does, at `0x5740f4`; kind 37 remains in
  `kind::NOT_PORTED`, and `set_group_volume` takes no `Sources`). Once
  effects are unmuted, the next `camera_moved`, hour change, unpause or
  music mode call starts the loop cleanly thanks to `0x57d969`.
- **Not ported:** `sims-game` does not yet send kind 25 on window focus
  changes.

## What would reopen this

A capture of the running original with sound enabled, stepping tracks
once a millisecond (`0x57cf37`), or porting kind 37.
