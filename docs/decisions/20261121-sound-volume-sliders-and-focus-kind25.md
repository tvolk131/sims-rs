# Sound Volume Sliders (Kinds 37–39) and Window Focus (Kind 25)

## Context

Two open items remained on the Complete Collection 1.91 sound fidelity checklist:

1. **Volume sliders (kinds 37, 38, 39)** in the sound manager dispatcher (`0x573617`), especially kind 37 (effects volume slider) re-running the day and night loop update (`0x574978`).
2. **Window focus wiring in `sims-game`** sending kind 25 (`SET_ACTIVE`, `0x573c64`) on focus loss and focus gain (`WM_ACTIVATEAPP` → `0x4f7608` → `0x4f76d5`).

## Disassembly Findings (Complete Collection 1.91)

- **Slider table (`0x67156c`) and defaults (`0x5713ee`..`0x571409`):**
  The sound manager initializes three slider step fields (`+0xf0` effects, `+0xf4` music, `+0xf8` voice) to `10`. Kinds 37, 38, and 39 index the 11-entry table at `0x67156c`:
  `[0, 300, 390, 490, 590, 690, 790, 840, 895, 960, 1024]`.
  Kinds 40, 41, and 42 (`0x574137`..`0x574152`) are C++ pointer getters returning `+0xf0`, `+0xf4`, and `+0xf8`.
- **Kind 37 (`0x57408f`, effects volume slider):**
  Stores step `[ebp + 0xc]` at `+0xf0`, looks up `vol = [step * 4 + 0x67156c]`, sets group 1 (`kGroupSfx`) volume via `0x57bde7(1)` → `vtable[+0x40]` (`0x57c0ea`), sets every loaded ambience score's volume (`+0x104`..`+0x110`, `vtable[+0x24]` → `0x5785f6`), and jumps to `0x5741b7` to call `0x574978` (`update_day_night`).
- **Kind 38 (`0x5740f9`, music) and Kind 39 (`0x57410d`, voice):**
  Store step at `+0xf4` / `+0xf8`, look up `0x67156c`, and set group 2 (`kGroupMusic`) / group 3 (`kGroupVox`) volume via `0x57c0ea`. Neither touches ambience scores or calls `0x574978`.
- **Day and night loop muted-start writes (`0x574a1b`..`0x574a68`):**
  Inside `0x574978`, after calling `start([0, 0, 0])` (`vtable[+0x30]`), the original unconditionally writes `EXTVOL = 0`, `FADE_DEST = target_vol`, `FADE_VAR = EXTVOL`, `FADE_SPEED = 5000`, `FADE_ON = 1` (forced to `0` on a stopped track at `0x57e059`) and calls `let_go` (`vtable[+0x5c]`), even when `start` returned early because group 1 volume was `0`. When kind 37 later raises group 1 volume from `0` to nonzero, `0x574978` sees `day_night_playing == false` and starts the loop fading up to the new slider volume.
- **Window focus (`0x4eee14`, `0x4f7608`..`0x4f76dc`):**
  The app constructor initializes its active flag (`+0x105`, offset `+0xbd` on the `0x4f7608` subobject) to `1`. `0x4f7610` returns immediately when the incoming focus boolean matches the stored flag (so startup while already focused sends no kind 25 and does not arm the 100-update volume-nudge countdown). On real focus changes, `0x4f76b0`..`0x4f76d5` calls `0x573617(25, focused, 0, 0, 0)`. (When `[0x6c9fa1] != 0` — i.e., "Run in Background" off — `0x4f7635`..`0x4f766a` also saves and sets the lot's pause flag via `0x4cee4a`.)

## Decisions

1. **Port kinds 37, 38, and 39 in `sims-audio` (`Player`):**
   - Export `VOLUME_STEPS`, `kind::SET_FX_VOLUME` (37), `kind::SET_MUSIC_VOLUME` (38), `kind::SET_VOX_VOLUME` (39), and remove 37..=39 from `kind::NOT_PORTED`.
   - Track `[u8; 3]` slider steps initialized to `[10, 10, 10]` with getters `fx_volume_step`, `music_volume_step`, and `vox_volume_step`.
   - Expose `set_fx_volume_step(step, sources)`, `set_music_volume_step(step)`, and `set_vox_volume_step(step)` (clamping `step` to `0..=10`), and dispatch HOT events of kinds 37–39 (reporting a fault on out-of-range steps rather than reading past `VOLUME_STEPS`).
   - Preserve `set_group_volume(group, volume)` for callers passing arbitrary `0..=1024` volumes, and write the stopped-track fade setup in `update_day_night` when `start` returns early on muted group 1 (`0x574a1e`..`0x574a5e`).
2. **Wire `WindowEvent::Focused` to kind 25 in `sims-game`:**
   - Add `Input::Active(bool)` handled by `Mixer::input` via `Player::set_active(active, &self.sources)`.
   - Track `focused` / `active_sent` (both initialized to `true`, matching `0x4eee14` + `0x4f7610`) on `sound::Feed`, with `Feed::set_focused(focused, game, send)` deduplicating unchanged states and flushing `Input::Sources`, `enter_live` (if first), pause state, and `Input::Active(focused)` immediately on `WindowEvent::Focused`.
   - Leave simulation pause state untouched on focus changes (matching the `[0x6c9fa1] == 0` "Run in Background" path of `0x4f7608`) until the Options UI exposes the background-run toggle.
3. **Defer 20 ms (50 Hz) periodic update timer (`0x572fc3` → `0x57cf37`):**
   - Changing the player's step interval from `BLOCK = 64` frames (2.9 ms) to 20 ms (441 frames) affects `wait`, fade cadence, `nudge` duration, and pinned PCM digests across the existing `sims-audio` test suite, so it belongs in its own focused change.
