//! The date and star bobs are bobs to the game as the standard one is,
//! and the pet picker's turntable loads pets' bodies. Needs
//! `SIMS_INSTALL`; skipped when it is unset. Nothing from the game is
//! kept: the bob and pet types are found by their definitions.

use crate::common::goth_options as options;
use sims_content::{Catalog, Index, Install};
use sims_game::Game;
use sims_people::bob::{self, Mode};
use sims_sim::World;
use sims_vm::boot::save::Saved;
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!("SIMS_INSTALL is not set; skipping the bob and pet tests");
            None
        }
    }
}

/// The Goth house's world with `change` made to it, restored into
/// `game` as a save would be (the world leads the snapshot's body; the
/// rest is kept).
fn restore_changed(game: &mut Game, change: impl FnOnce(&mut World)) {
    let saved = game.save().expect("the lot saves");
    let encoded = saved.encode();
    let tail = encoded[8..]
        .strip_prefix(saved.world().encode().as_slice())
        .expect("the world leads the snapshot")
        .to_vec();
    let mut world = saved.world().clone();
    change(&mut world);
    let mut bytes = encoded[..8].to_vec();
    bytes.extend(world.encode());
    bytes.extend(tail);
    game.restore(Saved::decode(&bytes).expect("the changed snapshot decodes"))
        .expect("the changed snapshot restores");
}

/// The Goth house's bob made a date bob, then a star bob (types with
/// their own 3D graphic whose suit picks that mode, found in the
/// install), then an object that is no bob. As the standard bob is, the
/// date and star bobs are spun by the game's clock (`spin_due`), redrawn
/// with no tick, and found by a click on their pixels (`Game::under`).
/// What is no bob is not spun. Only the head-slot object can be a bob
/// that is drawn, so the lot's other bob-type objects do not count.
#[test]
fn date_and_star_bobs_spin_and_are_picked_in_the_game() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install.clone())).expect("the Goth house opens");
    assert_eq!(game.run(100), 100);
    game.resize([800, 600]);
    let cast = sims_people::Cast::open(&install, &install.join("UserData")).expect("the cast");
    let mode_of = |guid: u32| {
        cast.animated_suit(guid)
            .and_then(|suit| Mode::of_suit(&suit))
    };
    let of_mode = |mode: Mode| {
        game.lot()
            .catalog()
            .objects
            .iter()
            .map(|o| o.guid)
            .find(|&guid| mode_of(guid) == Some(mode))
    };
    let (Some(date), Some(star)) = (of_mode(Mode::Date), of_mode(Mode::Star)) else {
        eprintln!("this install has no date or star bob; skipping");
        return;
    };
    let world = game.lot().world();
    let held = world
        .selected()
        .and_then(|person| bob::held_by(world, person))
        .expect("the selected Sim holds the standard bob");
    let plain = world
        .objects
        .values()
        .map(|o| o.guid)
        .find(|&guid| guid != bob::STANDARD && mode_of(guid).is_none())
        .expect("an object that is no bob");
    for (guid, is_bob) in [(date, true), (star, true), (plain, false)] {
        restore_changed(&mut game, |world| {
            world.objects.get_mut(&held).expect("the bob").guid = guid;
        });
        let (before, before_bob) = {
            let canvas = game.render();
            let on_bob: Vec<usize> = (0..canvas.width * canvas.height)
                .filter(|&i| {
                    canvas.id_at((i % canvas.width) as i32, (i / canvas.width) as i32) == Some(held)
                })
                .collect();
            (canvas.rgb565(), on_bob)
        };
        assert!(!game.spin_due());
        let standing = game.lot().world().hash();
        for _ in 0..12 {
            game.pace.restart();
            assert_eq!(game.advance(40_000), 0);
        }
        assert_eq!(game.lot().world().hash(), standing);
        assert_eq!(game.spin_due(), is_bob, "{:?}", mode_of(guid));
        if !is_bob {
            continue;
        }
        let canvas = game.render();
        let after = canvas.rgb565();
        let on_bob: Vec<[i32; 2]> = (0..600)
            .flat_map(|y| (0..800).map(move |x| [x, y]))
            .filter(|&[x, y]| canvas.id_at(x, y) == Some(held))
            .collect();
        assert!(
            !on_bob.is_empty(),
            "the {:?} bob is not drawn",
            mode_of(guid)
        );
        // The bob itself turned: a pixel it covers in either frame changed.
        let spun = before_bob
            .iter()
            .copied()
            .chain(on_bob.iter().map(|&[x, y]| y as usize * 800 + x as usize))
            .any(|i| before[i] != after[i]);
        assert!(spun, "the {:?} bob did not spin", mode_of(guid));
        assert!(!game.spin_due());
        for &point in &on_bob {
            assert_eq!(game.under(point), Some(held), "at {point:?}");
        }
    }
}

/// The pet picker's turntable (`render_pet_try_on_image`) loads a pet
/// with [`sims_people::Body::load`]: its skeleton is in an underscored
/// file, and it wears the picked body row alone. Every pet skeleton has a
/// type whose body row loads and draws. An install without the pack in
/// bit 32 of `Expansions::ALL` has no pets and is skipped.
#[test]
fn the_pet_pickers_turntable_loads_pets_bodies() {
    let Some(root) = install() else {
        return;
    };
    let found = Install::detect(&root).unwrap();
    if !found.expansions.contains(32) {
        eprintln!("this install has no pets; skipping");
        return;
    }
    let index = Index::build(&found).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let assets = sims_people::Assets::scan(&root).unwrap();
    let cast = sims_people::Cast::open(&root, &root.join("UserData")).unwrap();
    let mut drawn = std::collections::BTreeMap::<String, usize>::new();
    for object in catalog.objects.iter().filter(|o| o.kind == 2) {
        let Ok(rows) = cast.body_strings(object.guid) else {
            continue;
        };
        let (Some(skeleton), Some(row)) = (rows.first(), rows.get(1)) else {
            continue;
        };
        if assets.contains(&format!("{skeleton}-skeleton.cmx.bcf"))
            || !assets.contains(&format!("{skeleton}_skeleton.cmx.bcf"))
        {
            continue;
        }
        let count = drawn.entry(skeleton.clone()).or_default();
        let (app, tex) = sims_people::appearance::body_string(row);
        if app.is_empty() {
            continue;
        }
        let outfit = sims_people::Outfit {
            parts: vec![(app, tex)],
        };
        let Ok(body) = sims_people::Body::load(&assets, skeleton, &outfit) else {
            continue;
        };
        let canvas = sims_people::caf_preview::render_turntable(
            &body,
            None,
            &sims_people::caf_preview::TurntableParams::default(),
        )
        .unwrap_or_else(|e| panic!("type {:#010x}: {e}", object.guid));
        if canvas.rgba.chunks(4).any(|p| p[3] != 0) {
            *count += 1;
        }
    }
    assert_eq!(drawn.len(), 2, "{drawn:?}");
    assert!(drawn.values().all(|&n| n > 0), "{drawn:?}");
}
