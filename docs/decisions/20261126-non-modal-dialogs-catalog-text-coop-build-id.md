# Non-modal dialogs and the 40 s timer, catalog text, the co-op build identity, and parked picks

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-vm/src/{tick,dialog_non_modal,coop,lib}.rs`, `crates/sims-vm/src/primitives/dialog.rs`
  - `crates/sims-ui/src/{dialog_timer,buy}.rs`
  - `crates/sims-game/src/hud.rs`, `crates/sims-game/src/game/{ticking,pointer,dialogs}.rs`, `crates/sims-game/src/coop.rs`
  - `crates/sims-content/src/buy_catalog.rs`
  - `crates/sims-net/{build.rs,src/protocol.rs,src/session/lockstep.rs}`
- **Track:** Wave 2B

## Context

#484 drew non-modal dialogs and routed their input, but sims-vm still answered them inside the end of the lot update, so the player never saw one. The 40 s auto-answer timer was not ported. The Buy popup wrote its price as `${}`. Whether the catalog text lookup (CATS/CTSS) searches the semi-global was open. A co-op peer on another build could join and diverge. #458 left open whether the co-op executor should refuse a pick on an object parked off the lot.

## Decision 1: play mode leaves a non-modal dialog for the player

The original, at the end of the lot update (`0x436da0`), sets the record's status to 1 (shown). For a non-modal dialog (widget byte `0x69` clear) it keeps the widget, and the game goes on. A press of a button answers between updates (`0x4370be`): the button's status, or done for a dialog that only informs. Until then, the caller's next `Dialog` (`0x451552`) closes the shown record (`0x437058`, status 5, widget destroyed) and makes another. So:
- a choice is shown again every tick its caller runs;
- an informing dialog stays until it is answered or its caller shows another.

`Lot::present_non_modal_dialogs(true)` makes play mode do this:
- `end_of_update` leaves such a record shown and lists its caller in `Update::non_modal_shown`.
- `Lot::non_modal_dialogs` lists the shown records.
- `Lot::answer_non_modal(caller, status, text)` answers one between steps, through `ready()`, so not while a modal dialog or special panel waits. It is refused while co-op is attached, because an answer on one peer alone would part the worlds (a record can survive into co-op from single play). For kind 3, `text` goes to the global the caller's next `Dialog` reads.

The setting lives in `SpecialPanels::present_non_modal`, marked `#[serde(skip)]`. `Lot`, `Play` and the trip and lifecycle constructors are other tracks' files, and every constructor of `SpecialPanels` uses `default()` or moves a saved value. No save holds the setting and the save format is unchanged. A restore or a trip clears it, so `Game` sets it before every step (on exactly when it steps in play mode).

Unchanged:
- Strict mode still stops at a non-modal dialog with `TickError::Unsupported`, with or without the setting.
- Play mode with the setting off still answers at once.
- Co-op answers at once even with the setting on, since no command carries such an answer.
- Modal dialogs, scripted answers and `TICK_43200` are untouched.

Tests:
- `interpreter::tick::a_presented_non_modal_choice_waits_for_the_player_and_is_shown_again_each_tick`
- `interpreter::tick::a_presented_informing_non_modal_dialog_stays_until_closed`
- `interpreter::tick::without_presenting_a_non_modal_dialog_is_handled_as_before`
- the `dialog_non_modal` unit tests

In sims-game:
- `Game::answer` sends a press on a non-modal dialog (its dismiss box answers with the first button) to `answer_non_modal`, with the field's text. It answers nothing when the window still shows an older record of that caller's, replaced before the next draw. The answer marks the lot dirty.
- After a step that shows one, `Hud::read_non_modal_shown` reads each shown record at once, so an informing caller that went on has its text read from that tick's frame. The original makes the widget at the end of the update, at the same moment. The HUD keeps what it read per record (caller and operands) while the record is shown, so one queued behind another keeps its text.
- When a caller shows its choice again with other text (the same instruction, other `$Local` values), the HUD replaces the shown one with fresh input state, so a press held on the old text answers nothing.
- A modal record comes before a non-modal one still shown (`sims_ui::dialog::pending` and `Hud::read_dialog`), so a modal prompt the lot waits on is never hidden behind one. Among non-modal records the HUD shows the first whose text was or can be read, so one that cannot be read hides no other.
- `Game::has_dialog` already left non-modal dialogs out, because no modal `Lot::asking` comes from one.

## Decision 2: the 40 s auto-answer timer

When `0x436c89` makes the widget, it sets a 40000 ms timer through `0x523a76` and stores the button to press at widget +0x11c/+0x120. That is the last button: status 3 of two, 4 of three, 2 otherwise. There is no timer for kinds 3 and 4, or for a dialog anchored at the top right (record +0x6c). The clock is `timeGetTime` (`0x5daf06`, import slot `0x666400`), so the timer runs paused or not, modal or not.

The widget class (vtable `0x666ddc`) handles it in three slots:
- +0x118 (`0x5248a8`) presses the stored button on the timer message (0x1a).
- +0xe4 (`0x5249f9`) kills the timer on mouse-down (message 7).
- +0xf4 (`0x524a13`) restarts the full 40 s on mouse-up (message 11).

Where it lives:
- `sims_ui::dialog_timer` holds the rule (`auto_answer_status`) and the state (`AutoAnswer`).
- `Hud` arms it when it reads a new dialog, stops it on a press on the dialog, restarts it on the release, and restarts it when `Update::non_modal_shown` names the caller (a new widget). A modal dialog is read when the step stops on it, so its timer starts then, not at the next frame. `Hud::forget` stops it.
- `Game::advance_while` presses the button when it runs out, outside co-op, unless a special panel, a shell modal or the co-op lobby is in front, which holds it. After a modal answer it restarts the pace, so the time spent waiting runs no ticks, as after a press.
- `Game::next_wake_us_with_window` wakes for the deadline whether or not the window can draw, but not while the timer is held, so a timer that ran out behind a panel does not wake the host every microsecond. Because a non-modal choice is made again each tick its caller runs, its timer only runs out while that caller does not run (for example, paused), as in the original.

Tests on the Goth house (`dialog_timer::`):
- `the_help_dialog_answers_itself_with_its_last_button_after_40_s` (same world as pressing No)
- `a_press_on_the_dialog_holds_the_timer_and_its_release_restarts_it`
- `the_time_spent_waiting_owes_no_ticks_once_the_timer_answers`
- `the_timer_starts_when_the_tick_shows_the_dialog_not_at_the_first_frame`
- `the_co_op_lobby_holds_the_timer_without_waking_the_host`

## Decision 3: the Buy popup writes money with the currency formatter

`sims_ui::buy::card_header` writes the price through `currency::simoleons`, giving `$6,500`. The sell prompt's `$` substitution (`0x50a343`) is left as it was, since how it formats the refund was not traced.

## Decision 4: catalog text is read from the object's own file only

`0x457d3b` passes the definition's own file (`[def+0]`) to `0x4061e3`, which reads CATS through that file's vtable +0x38 (`0x406222`). The CTSS fallback reads the same file (`0x470256` → `0x470ca7`, vtable +0x38 at `0x470cfc`). The semi-global (`[def+0xc]`) is read only by the TTAB lookup (`0x457cb0`, through `0x416920`/`0x416924`).

The port already did this. A comment at the lookup cites the addresses, and `buy_catalog::catalog_text_is_never_read_from_the_semi_global` pins it: a semi-global CATS/CTSS of the same number leaves the definition named by its label.

## Decision 5: `Hello` carries a build identity

`sims_net::BUILD_ID` is generated by `crates/sims-net/build.rs`:
- It is the first 16 bytes of a SHA-256 over `Cargo.toml` and every file under `src/` of sims-core, sims-formats, sims-content, sims-sim, sims-vm and sims-net, which decide a tick.
- Each file contributes its path relative to `crates/` (path order), a NUL, its length, and its bytes with carriage returns dropped.
- Hidden files and editor backups are skipped.
- The workspace `Cargo.toml` and `Cargo.lock` are covered, since they choose the dependencies' versions and patches (float maths included).
- The interface crates are left out.

`WireMessage::Hello` ends with `build: BuildId`. A host refuses a different one with `RejectReason::BuildMismatch { expected, actual }`, after the version check and before the manifest check. `build_mismatch_message` gives the player's text, which sims-game shows.

`PROTOCOL_VERSION` is now 3, since postcard has no default for a missing field. `decode_payload` falls back to decoding a `Hello`'s leading `protocol_version`. When that version is not this one, it yields a `Hello` the version check refuses, so a protocol-2 joiner gets a clear `ProtocolVersionMismatch` instead of a silent drop. A protocol-3 joiner to an old host still fails as the old host decides.

Tests (`lockstep_memory::`):
- `a_hello_from_another_build_is_rejected_with_a_clear_message`
- `a_hello_of_an_older_shape_is_rejected_with_a_clear_message`

## Decision 6: the co-op executor refuses a pick on a parked object

#458 left this open (`20261125-lot22-go-home-fuzzer-targets-and-famh-export.md`, Decision 3). The UI never draws an object parked at [-16, -16], so no player can click it. Some of those menus fault when run (a stack object of 0, native error 0x34), which halts a strict co-op lot for every peer.

`execute_coop_pick` now returns `CoopCommandStatus::Refused` before listing the menu when the Sim, the target or a join's origin Sim is parked. The check is deterministic, so every peer refuses alike.

`coop::coop_refuses_a_peer_pick_on_an_object_parked_off_the_lot` does two things:
- It reproduces the fault with a plain `Lot::pick` (a strict step halts with native error 0x34).
- It sends the same entry as a co-op `Pick` and checks the refusal, then runs 60 more ticks.

## Not done

- No command carries a non-modal answer in co-op, so it still answers at once there and runs no timer.
- No sims-game test shows a real non-modal dialog. None of the installed houses shows one in 3,000 idle ticks, since they come from interactions. The HUD's per-record reading and replacement are untested for that reason.
- The sell prompt's refund formatting is not traced.
- The window routes typing only to a modal dialog's field, so a non-modal kind 3's field takes no keys yet (its text is sent as shown).
- A non-modal record whose text cannot be read (its caller has no frame) shows nothing and stays until its caller's next `Dialog`.
