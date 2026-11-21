# Sound Volume Sliders (Kinds 37–39) and Window Focus (Kind 25)

## Context

Two open items remained on the Complete Collection 1.91 sound fidelity checklist:

1. **Volume sliders (kinds 37, 38, 39)** in the sound manager dispatcher (`0x573617`), especially kind 37 (effects volume slider) re-running the day and night loop update (`0x574978`).
2. **Window focus wiring in `sims-game`** sending kind 25 (`SET_ACTIVE`, `0x573c64`) and matching the original's `WM_ACTIVATEAPP` handler (`0x4f7608`..`0x4f76dc`).

## Disassembly Findings (Complete Collection 1.91)

- **Slider table (`0x67156c`) and defaults (`0x5713ee`..`0x571409`):**
  The sound manager initializes three slider step fields (`+0xf0` effects, `+0xf4` music, `+0xf8` voice) to `10`. Kinds 37, 38, and 39 index the 11-entry table at `0x67156c`:
  `[0, 300, 390, 490, 590, 690, 790, 840, 895, 960, 1024]`.
  Kinds 40, 41, and 42 (`0x574137`..`0x574152`) are C++ pointer getters returning `+0xf0`, `+0xf4`, and `+0xf8`.
- **Kind 37 (`0x57408f`, effects volume slider):**
  Stores step `[ebp + 0xc]` at `+0xf0`, looks up `vol = [step * 4 + 0x67156c]`, sets group 1 (`kGroupSfx`) volume via `0x57bde7(1)` → `vtable[+0x40]` (`0x57c0ea`), sets every loaded ambience score's volume (`+0x104`..`+0x110`, `vtable[+0x24]` → `0x5785f6`), and jumps to `0x5741b7` to call `0x574978` (`update_day_night`).
- **Kind 38 (`0x5740f9`, music) and Kind 39 (`0x57410d`, voice):**
  Store step at `+0xf4` / `+0xf8`, look up `0x67156c`, and set group 2 (`kGroupMusic`) / group 3 (`kGroupVox`) volume via `0x57c0ea`. Neither touches ambience scores or calls `0x574978`.
- **Voice level refresh (`0x57c0ea` → `0x57c197` → `0x57f759`, `0x57d31e` → `0x57cc74`, `0x57def1`..`0x57e057`):**
  Group volume and `main_duckpri` updates walk the running list (which `0x57dc5a` removes held tracks from, and which excludes idle kind-53 voices) and call `0x57f759`. `0x57f759` reads `EXTVOL` (19), `VOL` (18), `Is3d` (26), and `DUCKPRI` (25) through `target` (`vtable[+0x88]` → `0x57de16`), while reading `PAN` locally (`[esi + 0x44]`, `0x57f85f`) and `PITCH` locally (`[esi + 0x48]`, `0x57f734`). `0x57d31e` refreshes unheld running voices on every `MAIN_DUCKPRI` request even when the maximum is unchanged, whereas direct writes to `DUCKPRI` (25) and `Is3d` (26) leave the voice latched until the next refresh (`0x57def1`..`0x57e057`).
- **Day and night loop target volume & muted-start writes (`0x574a07`..`0x574a68`):**
  Inside `0x574978`, the fade target reads `VOLUME_STEPS[[edi + 0xf0]]` (`0x574a07`..`0x574a10`) independently of raw group 1 volume (`0x57c102`). After calling `start([0, 0, 0])` (`vtable[+0x30]`), it unconditionally writes `EXTVOL = 0`, `FADE_DEST = target_vol`, `FADE_VAR = EXTVOL`, `FADE_SPEED = 5000`, `FADE_ON = 1` (forced to `0` on a stopped track at `0x57e059`) and calls `let_go` (`vtable[+0x5c]`), even when `start` returned early on group 1 volume `0`.
- **Window focus & Live-mode auto-pause (`0x4eee14`, `0x4f7608`..`0x4f76dc`, `0x4cee4a`, `0x4ce5b0`):**
  The app constructor initializes activation (`+0x105` / `+0xbd`) to `1`, and `0x4f7610` returns when unchanged. With `byte [0x6c9fa1] == 1` (the captured default, Run in Background off), focus loss in Live mode saves the current pause state (`0x4939c0` → `+0xbc`) and requests pause (`0x4cee4a(1)`), and focus regain restores `+0xbc` (`0x4cee4a`). `0x4f76d5` then dispatches kind 25 (`SET_ACTIVE`) immediately, while the pending pause flag reaches sound on the next update as kind 21/22 (`0x4ce5c5` / `0x4ce5d9`).
- **20 ms (50 Hz) volume-nudge countdown (`0x572fc3` → `0x572fd6` → `0x573019`):**
  Kind 25 on arms `+0x114 = 100` (`0x573c79`). The sound-manager timer fires every `20 ms` (`0x14`, exactly `441` frames at `22,050` Hz), decrementing `+0x114` once per 20 ms tick (~2.00 s total).

## Decisions

1. **Port kinds 37, 38, and 39 and target-following `0x57f759` in `sims-audio` (`Player`):**
   - Export `VOLUME_STEPS`, `TIMER_FRAMES` (`441`), `kind::SET_FX_VOLUME` (37), `kind::SET_MUSIC_VOLUME` (38), `kind::SET_VOX_VOLUME` (39), and remove 37..=39 from `kind::NOT_PORTED`.
   - Track `[u8; 3]` slider steps initialized to `[10, 10, 10]` with getters `fx_volume_step`, `music_volume_step`, and `vox_volume_step`, and setters `set_fx_volume_step`, `set_music_volume_step`, `set_vox_volume_step` (clamping `step` to `0..=10`; HOT events report a fault on steps outside `0..=10` rather than reading past `0x67156c`).
   - Read `VOLUME_STEPS[self.volume_steps[0]]` in `update_day_night` (`0x574a07`..`0x574a10`) and write the stopped-track fade setup when `start` returns early on muted group 1 (`0x574a1e`..`0x574a5e`).
   - Make `Player::levels` follow `target` for `VOL`, `EXTVOL`, `DUCKPRI`, and `THREE_D` while keeping `PAN` and `PITCH` local (`0x57f759`, `0x57f71d`), refresh voices immediately on level/pan/pitch writes (`0x57def1`..`0x57e057`, leaving `DUCKPRI` and `THREE_D` latched), refresh unheld running voices (`!held`, `0x57dc5a`) on every `MAIN_DUCKPRI` write (`0x57d31e`) and matching-group volume change (`0x57c197`), and remove the unconditional end-of-block overwrite.
   - Step the kind-25 nudge countdown (`0x573019`) at exact 20 ms (`TIMER_FRAMES = 441`) boundaries inside `render_stereo` (keeping `render_until_idle` aligned to `BLOCK = 64` boundaries) so focus-regain nudging lasts 2.00 s as in 1.91; moving track/fade execution off `BLOCK = 64` remains a separate change because existing pinned PCM digests and tests expect block-rate stepping.
2. **Wire `WindowEvent::Focused` to auto-pause + kind 25 in `sims-game`:**
   - Add `Input::Active(bool)` handled by `Mixer::input` via `Player::set_active(active, &self.sources)`.
   - On `Feed::set_focused`: deduplicate unchanged focus (`0x4f7610`); on focus loss flush pending `Sources`, clicks/sounds, and camera moves first (deferring all pending pause state — both manual toggles at `0x53a5bf` and earlier focus changes — to `Feed::update`, matching `0x4cee52`..`0x4cee5d` → `0x4ce59e`) so kind 25 cannot overtake earlier events or release held audio before pause settles, then save and set lot pause (`0x4f7635`..`0x4f765a`), mark pause dirty (`0x4cee52` bit `0x800000`, tested at `0x4ce59e`), and send `Input::Active(false)` immediately while deferring `SoundEvent::Pause` to the next `Feed::update` (`0x4ce5c5`); on focus regain refresh `Sources`, restore saved pause state (`0x4f765c`..`0x4f7666`), mark pause dirty so even a rapid blur-then-refocus dispatches the final pause state (`0x4ce59e` → `0x4ce5b0`), and send `Input::Active(true)` immediately while resetting `App::last` when pause changes so paused/stalled wall-clock time does not produce catch-up ticks.
   - Filter winit 0.30's pre-activation startup `WindowEvent::Focused(false)` on macOS (`window_delegate.rs:779`) via `FocusFilter` so ordinary window startup sends no spurious kind 25. Note: `sims-game` creates a single top-level window, so winit's per-window `Focused` event cannot fire for a same-app secondary window on Windows.
