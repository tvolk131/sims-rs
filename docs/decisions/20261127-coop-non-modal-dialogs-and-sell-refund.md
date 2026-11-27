# Co-op non-modal dialogs, typing into a non-modal field, and the sell prompt's refund

*Decided 2026-11-27.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-net/src/{protocol.rs,session/state.rs}`
  - `crates/sims-vm/src/{tick,dialog_non_modal,coop}.rs`, `crates/sims-vm/tests/{integration/fuzz,interpreter/tick}.rs`
  - `crates/sims-ui/src/buy.rs`
  - `crates/sims-game/src/hud.rs`, `crates/sims-game/src/game/{coop_shell,dialogs,pointer,ticking}.rs`, `crates/sims-game/src/main.rs`
  - `crates/sims-game/tests/integration/non_modal_dialogs.rs`
- **Track:** Wave 2B follow-up

## Context

Wave 2B (#504, `20261126-non-modal-dialogs-catalog-text-coop-build-id.md`) left five things undone:
- In co-op a non-modal dialog was still answered at once, because no command carried the answer, and its timer did not run.
- The window sent keys only to a modal dialog's field, so a non-modal kind 3's field took none.
- How the sell prompt writes its refund had not been traced.
- No sims-game test showed a non-modal dialog, so the HUD's per-record text and its replacement had no end-to-end test.
- A record that could not be read had no end-to-end test either.

## Decision 1: co-op shows non-modal dialogs and answers them by command

In play mode co-op always shows a non-modal dialog, whatever `present_non_modal` says, because every peer must do the same. The setting is the host's and lives in no save. Strict co-op is unchanged.

`sims_net::Command::AnswerNonModalDialog { caller, operands, answer }` carries the answer. It is appended after `HostChanged`, so the earlier variants keep their postcard tags. `PROTOCOL_VERSION` is now 4. The lenient `Hello` decode compares against the current version, not a fixed one, so a protocol-3 joiner still gets a clear `ProtocolVersionMismatch`. As with a pick, the session applies it if the issuer is a member.

Every peer runs the command between the same two ticks (`Lot::answer_non_modal_in_coop`), in tick order:

| Case | Result |
|---|---|
| Record of `caller` is shown with the same operands, and the dialog offers the answer | `Applied`. The record takes the status (done for one that only informs), and a kind 3's text goes to the global the caller's next `Dialog` reads. |
| No record shown for `caller` (answered or closed), or one with other operands (`NonModalAnswerError::Replaced`: it showed another since) | `IgnoredBySession(DialogAlreadyAnswered)`. The first answer in tick order wins. |
| An answer the dialog does not offer | `Refused`. |
| A modal dialog is pending or frozen, or one was answered this tick | `IgnoredWhileDialogPending`. |

`Lot::answer_non_modal` is still refused while co-op is attached, so no answer reaches one peer alone.

`Lot::takes_non_modal_answer` is the same gate seen from outside: the lot is ready, and no co-op tick frozen by a modal dialog waits to run again (a dialog answered while paused).

In sims-game:
- `Game::answer_non_modal` sends the command for the record shown (its operands, with the field's text) instead of answering the lot. While `takes_non_modal_answer` is false it sends nothing and leaves a note, since every peer would ignore the command.
- `apply_coop_batch_side_effects` reads `Update::non_modal_shown` as the single-player step does. That includes the host's prime tick at the start: `CoopController::take_prime_report` hands its report over after `start_host_with_transport`, and after a poll that finished an `iroh` start.
- When an answer is applied (`Applied` in the batch's results), the HUD drops that record's widget and the text it read (`Hud::close_non_modal`), as `0x4370be` destroys the widget. The caller's next identical prompt is then a new widget with fresh input state, and does not inherit the old one's typed text. The next dialog shown is taken up at once (`Hud::read_dialog`), with its timer, without waiting for a frame to be drawn.
- If sending the command fails, the timer runs again (`Hud::restart_auto_answer`) instead of staying spent.
- The 40 s timer runs in co-op for a non-modal dialog only, and only while the lot takes the answer, so the timer does not run out into an ignored command. When it runs out it sends the command, as a press would. A modal co-op dialog still has no timer, since one peer's timer would answer for everyone; that is left for later.

Tests:
- `coop::tests::a_non_modal_dialog_is_shown_in_co_op_and_answered_once_by_command` (sims-vm). It covers stale operands (ignored), an answer not offered (refused), the first answer (applied), a later one (ignored), and peers that agree.
- `interpreter::tick::without_presenting_a_non_modal_dialog_is_handled_as_before`. Its co-op half now expects the record shown with the setting off.
- The fuzzer (`integration::fuzz`) sends `AnswerNonModalDialog` for each record shown when no modal dialog or panel is open.
- `non_modal_dialogs::a_co_op_answer_closes_a_non_modal_dialog_on_every_peer` (sims-game). On a memory mesh:
  - the first lockstep tick shows the records, with no prime tick;
  - the host's HUD reads them from that batch, so its timer runs before any frame is drawn;
  - a peer's press answers on both, and the next record shows on both;
  - with no frame drawn since the answer, the host's timer answers that record on both.
- `non_modal_dialogs::a_host_reads_the_non_modal_dialogs_its_prime_tick_showed`: with the default prime tick, the host's timer runs before any frame is drawn.
- Mutation checks: dropping the prime report fails the second test; dropping the read after an answer fails the first.
- `lockstep_memory::a_hello_of_an_older_shape_is_rejected_with_a_clear_message` now expects version 4. A protocol-3 `Hello`, which has this protocol's shape, decodes whole, and the version check refuses it.

## Decision 2: a non-modal text field takes the keys

`Game::has_non_modal_dialog_entry` is true when the dialog shown is a non-modal kind 3 with no modal dialog, no Game-drawn modal and no pie menu in front. An open pie menu keeps the keys, so Escape still closes it. The window then sends it characters, Backspace, the caret keys, Enter and Escape, as it does for a modal dialog's field. `Hud::confirm_key` accepts that case too:
- Enter answers with the first button and the field's text.
- Escape on a one-button field does nothing, as on a modal one.

A modal dialog comes first: the HUD reads a modal record before a non-modal one, so the keys go to the modal one.

When the caller shows its dialog again with the same text, the HUD keeps the widget and what was typed. `same_dialog` compares the messages without their fields. Only different text replaces the widget. A press held on a kept widget also keeps the timer held: the reshow restarts the timer (`AutoAnswer::restart`) only when no press holds it, and the release starts the whole 40 s. Before, a reshow re-armed the timer under the held press, so it could answer while the button was down (`dialog_timer::tests::a_reshow_restarts_the_timer_but_not_through_a_held_press`).

`crates/sims-game/src/main.rs` is the only file outside this track's list. It changes one condition in the window's key routing.

Test: `non_modal_dialogs::a_non_modal_text_field_takes_the_keys_after_a_modal_dialog`. A modal Yes/No in front takes Enter; then typing, Backspace, a reshow that keeps the text, Escape doing nothing, and Enter answering with the text (`Game::last_dialog_text`).

## Decision 3: the sell prompt writes its refund as money

`0x50a343` builds the prompt and `0x50cb3d` substitutes it:
- `%` (`0x6cc78c`) becomes the object's name, or `<Unknown` (`0x6cc780`).
- `$` (`0x6c10a0`) becomes the refund, which is computed only when bytes `[esi+0x110]` and `[esi+0x111]` are set: `ftol(0x43219a(obj) * float[esi+0x114] + 0.5)`, with `0x636128` as `ftol`.

At `0x50cec3..0x50ced4` the refund is formatted through the application object (`[0x6fe90c]`, set at `0x5d5f67`, vtable `0x676c60`):
- Its vtable +4 (`0x5d5bee`) returns the locale at app+0x188.
- That locale's vtable +0x7c is a thunk (`0x5fdc5c`) to the cTSLocale currency formatter: `(this+0x14)` vtable `0x676868` +0x34 = `0x6007df`. This is the routine `sims_ui::currency` ports.

`SellLabels::format` now writes `currency::simoleons(refund)`, for example `$6,500`. Before it wrote `$6500`. The Goth sell test's `$250` reads the same either way.

Test: `buy::tests::the_sell_prompt_writes_its_refund_as_money`, which uses synthetic templates.

## Decision 4: end-to-end tests on the open pack with patched records

No installed house shows a non-modal dialog while idle. The new `non_modal_dialogs` integration module runs on the open pack instead, so it needs no install and runs in CI.

Each test puts the record into the world as a caller's `Dialog` would, through a save, a patch and a restore:
- a new record, which the next tick shows;
- or a shown one set back to new, as the caller's next `Dialog` does.

The callers are idle open-pack objects, whose frames do not run during these ticks. A caller's text comes from its frame's code owner's file, so a test points that frame at:
- the phone's file, for its strings;
- another file, for other text from the same operands, as other `$Local` values would give;
- a type the lot does not have, for a record that cannot be read.

The tests compare texts with each other and never with literal words.

Tests:
- `the_hud_keeps_each_records_text_and_passes_over_one_it_cannot_read`:
  - an unreadable first record hides nothing;
  - a record read when shown keeps its text after its frame becomes unreadable, including after a reshow;
  - nothing is drawn when only an unread record is left;
  - that record is drawn once it can be read.
- `a_reshown_dialog_with_other_text_replaces_the_one_shown_and_its_press`:
  - a press held across a reshow with the same text answers on release;
  - one held across a reshow with other text answers nothing.
- The text-field and co-op tests from Decisions 1 and 2.

Mutation checks: reverting `same_dialog` fails the text-field test, and disabling the co-op timer fails the co-op test.

## Unchanged

- Strict mode, single-player play mode with the setting off, modal dialogs and scripted answers behave as before.
- `TICK_43200` and the strict pins are unchanged.

## Not done

- When the HUD cannot read a shown choice (its caller's file is broken, such as a custom bitmap that does not parse), it draws nothing and sets no timer, and a yielding caller asks again each tick. That is already so in single-player play mode. A caller that runs `Dialog` always has a frame, so only broken content gets here. A fallback widget built from the record's buttons is left for later.
- In co-op the timer does not run out while the lot takes no answer (a modal dialog in front, or its tick not yet run again), but its deadline is not pushed back. An informing dialog that is not shown again can therefore answer as soon as the lot takes answers again. This matches the hold for special panels in single-player, and the original's timer runs on the wall clock whatever is in front, so it would have run out sooner.
- A co-op answer is matched by caller and operands. Suppose two players answer the same prompt in different ticks, and the caller asks the identical prompt again in between. The later answer then answers the new prompt. Every peer does the same, so the peers do not part. Telling a new prompt from a reshow needs a prompt identity carried in the co-op snapshot, which is left for later.
- Kind-3 answers share one text global (`last_dialog_text`), as the single-player answer already does. Two text answers taken before either caller runs leave the second text for both. Every peer agrees. Keeping the text per record needs a field on the world's dialog record, which belongs to another track's crate. No known object shows a non-modal text dialog.
- A peer that joins after a record was shown reads its text from the snapshot's world when it draws. If that record's caller has since gone on (an informing notice), the joiner cannot read it and draws nothing, while the host shows the text it read when the record was shown.
- A modal (blocking) co-op dialog runs no 40 s timer.
- Escape on a one-button non-modal text field does nothing, as on a modal one. The original's dismiss box is the only other way to close it.
- When a modal dialog is answered in front of a non-modal text field, the field is shown again with fresh input state, so text typed before the modal dialog is lost.
