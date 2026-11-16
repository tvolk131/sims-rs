//! Saving the Goths' house in the window and going back to the save.
//! Needs `SIMS_INSTALL`; skipped when it is unset.

use sims_game::game::goth_house;
use sims_game::{Game, Options};
use sims_ui::command::Speed;
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!("SIMS_INSTALL is not set; skipping the save tests");
            None
        }
    }
}

fn options(install: PathBuf) -> Options {
    Options {
        house: goth_house(&install),
        install,
        answers: vec![3],
        play: false,
    }
}

/// A REDB file in the temporary directory, removed when dropped.
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("sims-game-{name}-{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Opens a pie menu on the first thing on the lot that has one.
fn open_a_menu(game: &mut Game) {
    for y in (0..600).step_by(6) {
        for x in (0..800).step_by(6) {
            let point = [x, y];
            if game.over_panel(point) || game.under(point).is_none() {
                continue;
            }
            game.hover(point);
            game.press(point);
            game.render();
            if game.menu().is_some() {
                return;
            }
        }
    }
    panic!("nothing on the lot opens a pie menu");
}

#[test]
fn the_window_goes_back_to_its_save_and_plays_on_as_from_there() {
    let Some(install) = install() else {
        return;
    };
    let file = Temp::new("saves");
    let store = sims_sim::Snapshots::open(&file.0).unwrap();
    let mut game = Game::open(&options(install.clone())).expect("the Goth house opens");
    // Nothing is saved yet: the lot is as it was.
    assert!(!game.restore_from(&store, "goths"));
    assert_eq!(game.run(50), 50);
    game.resize([800, 600]);
    game.render();
    game.save_to(&store, "goths");

    // Then the player goes on: a pie menu open, ultra speed, more ticks.
    open_a_menu(&mut game);
    game.set_speed(Speed::Ultra);
    assert_eq!(game.run(100), 100);
    assert!(game.menu().is_some());

    assert!(game.restore_from(&store, "goths"));
    assert_eq!(game.lot().ticks(), 50);
    assert!(game.menu().is_none(), "the menu is the lot's before");
    assert_eq!(game.pace.speed, Speed::Normal);
    assert_eq!(game.stopped(), None);
    assert!(game.errors().is_empty());
    game.render();

    // From there it plays as a lot that ran straight on.
    let mut straight = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(straight.run(250), 250);
    assert_eq!(game.run(200), 200);
    assert_eq!(game.lot().world().hash(), straight.lot().world().hash());
}

#[test]
fn a_save_made_while_the_help_dialog_waits_shows_it_again() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        answers: Vec::new(),
        ..options(install)
    })
    .expect("the Goth house opens");
    game.resize([800, 600]);
    // Tick 2 shows the Help System's dialog.
    assert_eq!(game.run(5), 1);
    let shown = game.asking().expect("a dialog waits").shown.clone();
    game.render();
    assert!(game.dialog().is_some());
    let saved = game.save().unwrap();

    let no = game.dialog().unwrap().buttons[1];
    game.press([no.x + no.width / 2, no.y + no.height / 2]);
    assert!(game.asking().is_none());
    assert_eq!(game.run(20), 20);
    game.render();
    assert!(game.dialog().is_none());

    game.restore(saved).unwrap();
    assert_eq!(game.lot().ticks(), 1);
    // The tick that showed it runs again, and shows it again.
    assert_eq!(game.run(1), 0);
    assert_eq!(game.asking().expect("asked again").shown, shown);
    game.render();
    assert_eq!(game.dialog().expect("drawn again").caller, shown.object);
}
