# UI and HUD parity polish: text fields, balloons, the job panel, the tutorial marker, mist and dialog corners

*Decided 2026-11-27.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-ui/src/{art,balloon,dialog,hud,options,tutorial,lib}.rs`
  - `crates/sims-ui/tests/integration/{dialog,ui_ratchet}.rs`
  - `crates/sims-game/src/hud.rs`, `crates/sims-game/src/game/{buy_mode,commands,dialogs,drawing,mod,opening,pointer,tests,ticking}.rs`, `crates/sims-game/src/main.rs`
  - `crates/sims-game/tests/integration/{non_modal_dialogs,options_and_dialogs,special_panels_camera_animations}.rs`
- **Track:** Wave parity, lane C (UI and HUD)
- **Supersedes:** the corner caveat in `20261115-help-system-dialog-matches-original.md`.

## Context

The roadmap listed several UI gaps:
- the dialog frame's corners;
- help balloons for the job panel, and the fade for every balloon;
- the job panel's blinking performance;
- caret, selection and paste in a kind-3 text field;
- the Magicland mist overlays drawn too bright;
- the tutorial marker, drawn as a yellow outline instead of the original's sprite.

Each item below was traced in Sims.exe 1.91 before it was ported.

## Decision 1: a text field edits as `cTSWinTextEdit2` does

The field is `cTSWinTextEdit2` (vtable `0x6755d0`). Its handlers:

| Address | What it does |
|---|---|
| `0x5f2895` | OnChar |
| `0x5f29d8` | OnKeyDown. Flags: bit 0 = Shift, bit 1 = Ctrl. Insert toggles overwrite (`[+0x13c]`). |
| `0x5f2ec7` | Caret move (delta, extend). With extend, the end of the selection at the caret moves and the other end, the anchor, stays. Without extend, the selection is dropped and the caret steps from where it is. |
| `0x5f3188` | Word move (dir, extend). Separators are space and `\n`. The ported loop: a separator sets a flag and steps; a non-separator after the flag ends the move. Leftward, it first skips separators before the caret. The result is clamped to `0..=len`. |
| `0x5f2c21` | Slot `0xa8`, a double click (medium confidence). It moves a word left without extend, then a word right with extend, so the word is selected. |
| `0x5f243c`, `0x5f2522` | Mouse-down. A Shift press on the same field extends the selection from its anchor, and a drag keeps the anchor. |
| `0x5f4699`, `0x5f4724`, `0x5f473f` | Copy, Cut and Paste. Paste replaces the selection through `0x5f32bc`, which truncates to the field's limit. |
| `0x5d7cf1` | Clipboard import. Drops `\r`, turns a tab into 4 spaces, and drops every other character below 0x20 except `\n`. The index still steps after a removal, so the character after one is not checked (ported as is). |

A single-line field also drops characters below 0x20, with the same skip quirk. Overwrite replaces the character at the caret even when the field is full.

Each text control keeps its own overwrite flag, so the two fields of a Save Card toggle separately. The cleaners run in one pass and lazily, so a paste reads only as much of the clipboard as fits in the field. A huge paste therefore cannot stall the window.

A double click runs the word move left from one past the caret when the caret sits on a letter. The original's routine, run from the caret itself, would select the word before when the click lands on a word's first letter. The port selects the clicked word instead. It acts only when the press that made the double click went to the field, so a press taken by a window in front (the co-op lobby) changes no selection behind it.

In sims-ui, `DialogState` gains:
- `overwrite`;
- the selection-extending forms of every caret move: `move_left` / `move_right` / `move_word` / `move_up` / `move_down`, and `text_home` / `text_end` / `home_in_layout` / `end_in_layout`, each taking `extend`;
- `select_word`, `select_all`, `selected_text`, `cut` and `paste`;
- `clipboard_text` and `single_line_text`.

In sims-game:
- The HUD tracks Shift for a Shift-press.
- `Game::dialog_paste`, `dialog_copy` and `dialog_cut` use the dialog clipboard:
  - On macOS, paste reads `pbpaste`, but only when `osascript`'s `clipboard info` lists a plain-text type (the original pastes `CF_TEXT` only; `pbpaste` would fall back to RTF or EPS). A clipboard without text pastes nothing; it never falls back to older text.
  - Elsewhere it is the process's own buffer (`coop::clipboard_*`).
  - Copy writes through `coop::clipboard_copy`. An error leaves a note, and a cut whose copy failed leaves the field as it was.
  - `Game::use_local_clipboard` keeps the clipboard inside the game, which tests use so they leave the system clipboard alone.
- The window binds:
  - Ctrl/Cmd+V, C and X;
  - Shift+Insert (paste), Ctrl+Insert (copy) and Shift+Delete (cut);
  - Insert to toggle overwrite;
  - Shift and Ctrl on the arrow keys, Home and End;
  - a double click in the field to select a word.
- While Ctrl is held, no character is typed, so Cmd+V on macOS no longer types `v`.

The original's OnKeyDown binds no key to copy, cut or paste. Its routines are reached from elsewhere, and which keys 1.91 sends them was not traced. The bindings are this port's own choice and are listed under *Not done*.

Tests:
- sims-ui dialog unit tests: `shift_moves_grow_and_shrink_the_selection_from_its_anchor`, `word_moves_stop_where_the_original_stops`, `a_paste_is_cleaned_and_cut_to_the_fields_limit`, `overwrite_replaces_the_character_at_the_caret_even_at_the_limit`, `a_huge_paste_reads_only_what_fits`, `each_save_card_field_keeps_its_own_overwrite_mode`.
- `non_modal_dialogs::a_non_modal_text_field_takes_the_keys_after_a_modal_dialog`, now with:
  - Shift+Left ×2;
  - a paste of `x\ty` that becomes `Ax    y`;
  - Ctrl+Shift+Left and a cut;
  - overwrite;
  - Shift+End, then Backspace;
  - a paste from the game's own clipboard;
  - a double click that selects only after a press in the field.

## Decision 2: job-panel skill balloons, and one fade for every balloon

**Job skill balloons.** The adult job panel's handler `0x53074c` (code 1) lays out six skill labels from the table at `0x6f4420`: at (93, 6), 62×16, 15 pixels apart. Each opens a help balloon:
- its text is Live.iff `STR#` 137, pairs 0–5;
- its pictures come from `0x6f4290`, the same table as the fame panel's skill pictures (`0x6f3cb8`).

`Help::JobSkill(n)` is balloon id `32 + n`. It reuses the fame skill texts, and it is open only while the Job panel shows an adult's job.

**Dismissal** was already ported and is unchanged:
- leaving the balloon's area with the pointer (`0x51f1f2..0x51f3e5`);
- a press outside it (`0x51f3c5..0x51f3e5`);
- the focus moving to another control (`0x51d312`, `0x51d354`).

**Fade.** Options help balloons are the same `cWinBalloon` (`0x516615` / `0x51679a`, `0x5165ac` / `0x516734`). They now take the 250 ms fade the Live ones had:
- sims-ui gains `balloon::composite` and `options::draw_balloon_with_alpha`;
- the HUD keeps one `FadingBalloon`, either Live or Options.

`0x50fad3..0x50fb2f`: a balloon opened while the last one is still fading out turns the ramp around without `UI_whoosh`, from wherever the ramp is, even on the millisecond the fade-out started.
- A close starts the fade-out at once, so a close and a reopen with no frame drawn between are one whoosh and a turned ramp. Every close does: a press, Escape in Options, and F10.
- The ramp is shared only within a kind. An Options balloon is the Options panel's child: when the panel goes, or collapses, the balloon goes without a fade. A Live balloon opened then starts its own ramp, with its whoosh.

A help press is checked against the selection the panel was laid out for. In co-op the panel is always laid out for the player's own selection, even when the lot's selection is the same Sim, so a label that is shown is never refused.

Tests:
- `an_adults_job_skill_labels_open_the_skill_balloons`.
- `temporal_ui_animations_advance_when_clock_positive_and_snap_at_clock_zero`, now with:
  - the silent reopen 100 ms into a fade-out;
  - an Options balloon's fade-in and fade-out;
  - a close and reopen with no frame between (one whoosh);
  - Escape fading an Options balloon out, and an Escape and reopen with no frame between (one whoosh);
  - F10 collapsing the panel under an Options balloon (no fade);
  - an Options balloon left open when the panel goes (no fade in Live, and a Live balloon whooshes).
- `balloon_fade_state_ramps_over_250ms_and_reverses_smoothly`, now with a close and reopen on the same millisecond (no flash to 0).
- Mutations: playing the whoosh on a reopen, closing the fade only at the next draw, carrying the Options ramp into Live, Escape without the fade, and keeping the Options ramp over a collapsed panel each fail the test.

## Decision 3: the performance blinks green under zero

`0x5306df(333)` starts a 333 ms timer when the Sim has a job (`0x4ea722`) and the job performance (`0x4ea88c`) is below zero. `0x53070d` stops it. The timer `0x52c659` flips `[+0x1a4]`, and while that is set the performance text is drawn in `[0x6f2ab0]` = `0x0647` (RGB 0, 202, 57, packed at `0x4ff43a`).

The port:
- `hud::performance_blinks` decides when it blinks;
- `PERFORMANCE_BLINK_US` = 333,000;
- `PERFORMANCE_LIT` is the colour;
- the sims-game HUD keeps a `PerformanceBlink` beside the pause button's blink and wakes the window for it. Its timer starts the frame a blinking job panel is first laid out.

The pause button's 500 ms blink was already right.

Tests: `the_performance_blinks_only_for_a_job_below_zero` (sims-ui), `the_performance_blinks_green_every_333_ms_and_stops_unlit` (sims-game).

## Decision 4: one tutorial marker, placed by `0x5de13f`

The UI Effect primitive (`0x44acff`) reaches a control through three routines:
- `0x476485` for a button;
- `0x47690d` for a portrait;
- `0x4769c3` for the first relationship control, whoever it shows (`0x4769ca`).

Each calls the control's slot `+0x160`, which is `0x5de13f`. That routine works on the one `cTSWinTutorialHighlight` window, at `[[0x6fe944] + 0x1f4]`:
- That window has vtable `0x6747b8` and is created at `0x4d089d`.
- **A highlight on a visible control** (vtable `+0x8c`) moves the window and shows it (`+0x94`):
  - `x = target.x + (target.w − marker.w) / 2`, truncating toward zero (`cdq; sub; sar`);
  - `y = target.y − marker.h − 3`.
- **A clear, or a highlight on a hidden control**, hides the window (`+0x98`) whatever control it was over (`0x5de1d1`).

The bitmap is UI graphic 4980 (150×50), keyed on `[0x6f2ab4]` = `0xf81f`. `0x5e999a(bitmap, 3, 6.0f)` cuts it into three 50×50 cells played six a second. The port plays them in the order 0, 1, 2, 1.

The window is also hidden:
- by `0x5dab61`, while a modal loop runs. The loop hides it if it is visible and shows it again afterwards. The port starts the animation over then.
- by Escape on the lot view. Key 27 at `0x4d133b` goes to `0x4d14a5`, which hides it at `0x4d14b2`.

The port:
- **New `sims_ui::tutorial`:** `MARKER`, `origin`, `cell_at`, `next_cell_us` and `draw`.
- **The yellow outline is gone.** It is replaced by one `Option<TutorialMarker>`:
  - its target is a `MarkerTarget`: button, portrait or relationship;
  - it records when it was shown;
  - it is hidden while a modal dialog is up (`Hud::has_dialog`).
- **Wakes:** the marker wakes the window for its next cell, but only while it is on screen. A marker over a control that is not laid out, or with no bitmap (the open pack), is not drawn. It then sets no wakes, and Escape is not taken.
- **Modals:** besides the HUD's modal dialogs, the Game-drawn modals (shop, book, phone book, special panels, destination picker), the co-op lobby and the shell's windows over the lot (Save Manager, Photo Album, travel dialog; the window tells the game through `Game::set_shell_modal` before each frame) hide it too.
- **Steady frames:** a marker that is up rules out the steady-frame fast path.
- **Escape:** both of the window's Escape paths hide it. A hide counts as handling the Escape, so that press does not close the window. What counts is the marker drawn on the last frame: one moved to a control not drawn yet still takes the Escape, since the old one is on screen.
- **`highlighted_buttons()`** and the other highlight sets are kept for callers and tests.

ZoomIn (26, 522), 27×27, puts the marker at (15, 469).

Tests:
- `tutorial::tests` (sims-ui): placement, the cell order, and drawing with the key.
- `the_tutorial_marker_finds_buttons_portraits_and_the_first_relationship_control`.
- `the_tutorial_marker_follows_highlights_and_hides_for_clears_modals_and_escape`. It runs on the open pack and covers:
  - a retarget;
  - a repeat highlight that keeps the animation going;
  - a clear of another control, which hides the marker;
  - the hide under a modal notice and the restart after it;
  - the wake;
  - Escape, also after a retarget that no frame has drawn;
  - with no bitmap, no draw, no wake and an Escape that is not taken. With a synthetic bitmap, cell 0 is drawn at Zoom In's marker origin.
- `ui_ratchet::the_tutorial_marker_bitmap_is_three_keyed_50_pixel_cells` (needs `SIMS_INSTALL`).

## Decision 5: the mist overlays keep their stored colour

`0x54228e` clears `[0x6c8c3c]` (at `0x54228f`), loads 0x1655, 0x1656 and 0x1657, and sets the flag back at `0x5422d4`. The TGA loader un-premultiplies (`0x4deb3d..0x4deb9d`) only while that flag is set, so these three Magicland mist overlays are the only UI TGAs drawn with their stored colour.

`art::MIST` is the range, and `art::prepare` skips un-premultiplying for it at every load site.

Test: `the_mist_overlays_keep_their_stored_color`.

## Decision 6: the dialog corners already match; pinned against the capture

`cTSWinMsgBox::OnDraw` (`0x524603`) draws the frame with the 9-slice `0x5df45d`:
- the corners are a plain copy;
- PopupInfoTiles (3008) has no partly transparent pixel.

`dialog::draw_nine_slice` already does the same. The 2026-11-15 caveat ("mitred here, blended in the original") describes an older drawing.

The 278 pixels that differed then came from comparing colours widened in two ways:
- the capture widens 5-6-5 by `v * 255 / 31`;
- `Frame::quantized` repeats the high bits.

Compared as RGB565 codes, every pixel of the 12-pixel ring matches the Help System capture: 0 of 16,656 differ. The Magic Town and Studio Town shots also differ in 0 ring pixels.

Tests:
- `ui_ratchet::the_dialog_frame_ring_matches_the_191_help_dialog`. It needs `SIMS_INSTALL` and the private capture (`SIMS_HELP_DIALOG_ORACLE`). Moving the rect by one pixel fails it, with 2,766 pixels different.
- `dialog::tests::the_nine_slice_copies_its_corners_exactly`, a synthetic test.

## Unchanged

- The screenshot ratchet: every class is at or above its floor. Panel scores: 99.99 / 99.99 / 99.88 ×5 / 99.19%.
- The simulation, strict mode and the 43,200 pin are unchanged.
- No sims-vm change.

## Not done

- **kHelpBtn** (`0x53aaad` → `0x50e8d3`). The click builds a modal help window (0x188 bytes) on background 3008:
  - it lists UIText `STR#` 166 titles, and topic *i* shows string 2*i*+2;
  - it remembers the topic at `[cp+0x1f0]`;
  - it closes on Enter or Esc (`0x50f013`);
  - it pauses the view (`0x4d3d98`).

  Its geometry was not traced, so the button still only notes that it is not ported.
- **The job panel's gadget balloons** (career, salary, performance, friends; `0x530af1`). Their string indices were not resolved.
- **The family-portrait balloon.**
- **The neighborhood lot button's override of slot `+0x160`** (`0x54f192`) moves the marker a further 50 pixels. No lot button takes a UI Effect in the port.
- **Marker visibility.** The port asks whether the control is visible when it draws, not when the highlight arrives. A highlight on a control that is laid out later still shows the marker; the original hides it.
- **Clipboard keys.** Which keys 1.91 binds to copy, cut and paste needs checking on the harness.
- **Double click.** The medium-confidence reading of slot `0xa8` (`0x5f2c21`) as a double click needs checking on the harness.
