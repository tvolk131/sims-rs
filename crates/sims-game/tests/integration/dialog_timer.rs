//! A simulation dialog's 40 s auto-answer timer (`0x436c89`), on the Goth
//! house's Help System dialog.

use crate::common::{goth_options, install_root};
use sims_game::{Game, Options};

/// Tick 2 shows the Help System's dialog (Yes, No), with no answers given.
fn waiting() -> Option<Game> {
    let install = install_root()?;
    let mut game = Game::open(&Options {
        answers: Vec::new(),
        ..goth_options(install)
    })
    .expect("the Goth house opens");
    assert_eq!(game.advance(149_999), 1);
    assert!(game.asking().is_some());
    game.resize([800, 600]);
    game.render();
    Some(game)
}

#[test]
fn the_help_dialog_answers_itself_with_its_last_button_after_40_s() {
    let Some(mut game) = waiting() else {
        return;
    };
    assert!(game.auto_answer_running());
    // Not before the whole 40 s, paused or not.
    game.advance(39_999_999);
    assert!(game.asking().is_some());
    game.advance(1);
    assert!(game.asking().is_none(), "{:?}", game.stopped());
    // As a press of No, the last button, does (`0x5248a8` presses it).
    let mut pressed = waiting().unwrap();
    let no = pressed.dialog().expect("the dialog is drawn").buttons[1];
    let point = [no.x + no.width / 2, no.y + no.height / 2];
    pressed.press(point);
    pressed.release(point);
    assert!(pressed.asking().is_none());
    assert_eq!(game.lot().ticks(), 2);
    assert_eq!(game.lot().world().hash(), pressed.lot().world().hash());
    assert!(!game.auto_answer_running());
}

#[test]
fn a_press_on_the_dialog_holds_the_timer_and_its_release_restarts_it() {
    let Some(mut game) = waiting() else {
        return;
    };
    let layout = game.dialog().expect("the dialog is drawn");
    // On the dialog, off its buttons and text field: its title area.
    let point = [layout.rect.x + layout.rect.width / 2, layout.rect.y + 4];
    assert!(
        layout
            .buttons
            .iter()
            .all(|b| !b.contains(point[0], point[1]))
    );
    game.advance(30_000_000);
    game.press(point);
    assert!(!game.auto_answer_running());
    // Held, it never runs out.
    game.advance(120_000_000);
    assert!(game.asking().is_some());
    // The release starts the whole 40 s again.
    game.release(point);
    assert!(
        game.asking().is_some(),
        "a release off a button answers nothing"
    );
    assert!(game.auto_answer_running());
    game.advance(39_999_999);
    assert!(game.asking().is_some());
    game.advance(1);
    assert!(game.asking().is_none(), "{:?}", game.stopped());
}

#[test]
fn the_time_spent_waiting_owes_no_ticks_once_the_timer_answers() {
    let Some(mut game) = waiting() else {
        return;
    };
    // One call past the deadline: the answered tick runs again, and the
    // 45 s the dialog waited are not run as ticks after it.
    assert_eq!(game.advance(45_000_000), 0);
    assert!(game.asking().is_none(), "{:?}", game.stopped());
    assert_eq!(game.lot().ticks(), 2);
    // From there time is owed as before.
    assert_eq!(game.advance(50_000), 1);
}

#[test]
fn the_timer_starts_when_the_tick_shows_the_dialog_not_at_the_first_frame() {
    let Some(install) = install_root() else {
        return;
    };
    let mut game = Game::open(&Options {
        answers: Vec::new(),
        ..goth_options(install)
    })
    .expect("the Goth house opens");
    assert_eq!(game.advance(149_999), 1);
    assert!(game.asking().is_some());
    // No frame drawn yet: the widget, and its timer, came with the tick.
    assert!(game.auto_answer_running());
    game.advance(40_000_000);
    assert!(game.asking().is_none(), "{:?}", game.stopped());
}

#[test]
fn the_co_op_lobby_holds_the_timer_without_waking_the_host() {
    let Some(mut game) = waiting() else {
        return;
    };
    game.open_coop_lobby(None);
    game.advance(60_000_000);
    assert!(
        game.asking().is_some(),
        "the lobby in front holds the timer"
    );
    // Run out but held: nothing to wake for (no 1 us spin).
    assert!(
        game.next_wake_us(false, 16_000).is_none_or(|us| us > 1_000),
        "{:?}",
        game.next_wake_us(false, 16_000)
    );
    // Closed, the timer that ran out presses its button.
    game.close_coop_lobby();
    assert_eq!(game.next_wake_us(false, 16_000), Some(1));
    game.advance(1);
    assert!(game.asking().is_none(), "{:?}", game.stopped());
}
