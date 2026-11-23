//! A family that arrives in Magic Town is placed by the middle ped marker
//! once, and then left alone.
//!
//! Magic Town's middle ped marker (GUID 0x12e84823, tree 0x100e "main -
//! middle ped marker") has attribute 3 set by its init tree. Its main loop
//! then puts each member of the arriving family beside marker 0x13076784
//! (Find Location For), runs Generic Sims Call 21 for that person, and
//! clears attribute 3 when the family is done. Routine 21 used to be
//! missing: its fault restarted the tree with attribute 3 still set, so
//! every 21 ticks the marker put the first traveler beside it again,
//! whatever the Sim was doing.
//!
//! Needs an install (`SIMS_INSTALL`); skipped otherwise.

use sims_game::Frames;
use sims_game::Game;
use sims_game::neighborhood::NeighborhoodShell;
use std::collections::BTreeMap;
use std::path::PathBuf;

const MAGIC_PORTAL_GUID: u32 = 0x2baa_0029;
const MIDDLE_PED_MARKER_GUID: u32 = 0x12e8_4823;
/// Ticks the marker's arrival pass may take: it sleeps 20 ticks before
/// placing anyone.
const ARRIVAL: u32 = 60;
/// Ticks watched after that. The old loop moved a traveler every 21.
const WATCHED: u32 = 600;

fn install_root() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    dir.is_dir().then_some(dir)
}

fn place_and_queue_magic_hole(game: &mut Game) {
    let jumper = game.lot().world().selected().expect("selected Goth Sim");
    let sim_pos = game.lot().world().objects[&jumper].position;
    let lot = game.lot_mut();
    lot.set_mode(sims_vm::PanelMode::Buy).expect("enter Buy");
    let mut cursor = lot
        .buy_start(MAGIC_PORTAL_GUID, 0, 0)
        .expect("buy_start Magic Hole")
        .expect("Magic Hole cursor");
    let hole = cursor.held;
    let [x, y] = [sim_pos[0] / 16, sim_pos[1] / 16];
    let placed = (1..=14i32).any(|r| {
        (-r..=r).any(|dy| {
            (-r..=r).any(|dx| {
                let [tx, ty] = [x + dx, y + dy];
                (4..=60).contains(&tx)
                    && (4..=60).contains(&ty)
                    && lot
                        .buy_move(&mut cursor, [tx * 16 + 8, ty * 16 + 8], 1)
                        .unwrap_or(false)
                    && cursor.valid
                    && lot.buy_commit(&cursor).is_ok()
            })
        })
    });
    assert!(placed, "no tile for the Magic Hole");
    lot.set_mode(sims_vm::PanelMode::Live)
        .expect("return to Live");
    lot.select(jumper).expect("select the jumper");
    let menu = lot.pie_menu(jumper, hole).expect("Magic Hole pie menu");
    let jump_in = menu
        .entries
        .iter()
        .find(|e| e.action.action == 0x1001)
        .expect("Jump In")
        .action
        .clone();
    lot.pick(jumper, jump_in).expect("queue Jump In");
}

#[test]
fn the_middle_ped_marker_places_an_arriving_family_once() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to run the Magic Town arrival test");
        return;
    };
    let dir = std::env::temp_dir().join(format!("sims-magic-town-arrival-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut shell = NeighborhoodShell::open(
        root,
        dir.join("saves.redb"),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("open the neighborhood");
    shell.resize([1024, 768]);
    shell.enter_lot(5).expect("enter the Goth house");
    let game = shell.game_mut().expect("active lot");
    game.run(30);
    place_and_queue_magic_hole(game);
    let mut picker = false;
    for _ in 0..800 {
        game.run(1);
        assert!(game.stopped().is_none(), "lot halted: {:?}", game.stopped());
        if let Some(asking) = game.asking() {
            let caller = asking.shown.object;
            game.answer(caller, 0);
            continue;
        }
        if game.asking_special_panel().is_some() {
            picker = true;
            break;
        }
    }
    assert!(picker, "the Magic Hole never asked for a Magic Town lot");
    shell.render();
    let point = shell
        .game()
        .and_then(|g| g.destination_picker())
        .and_then(|p| p.hit_point_for_lot([1024, 768], 93))
        .expect("a point on lot 93");
    let _ = shell.press(point);
    let _ = shell.release(point);
    assert_eq!(shell.active_lot_house(), Some(93));

    let game = shell.game_mut().expect("lot 93");
    let travelers: Vec<u16> = {
        let world = game.lot().world();
        world
            .objects
            .values()
            .filter(|o| o.person.is_some() && o.position[0] >= 0)
            .map(|o| o.id)
            .collect()
    };
    assert!(!travelers.is_empty(), "nobody arrived on lot 93");
    let marker = game
        .lot()
        .world()
        .objects
        .values()
        .find(|o| o.guid == MIDDLE_PED_MARKER_GUID)
        .map(|o| o.id)
        .expect("lot 93 has a middle ped marker");

    assert_eq!(game.run(ARRIVAL), ARRIVAL, "lot 93 stopped advancing");
    let mut last: BTreeMap<u16, [i32; 3]> = travelers
        .iter()
        .map(|&id| (id, game.lot().world().objects[&id].position))
        .collect();
    let mut jumps = Vec::new();
    for _ in 0..WATCHED {
        // An unanswered dialog would stop the ticks without stopping the
        // lot, and the positions would stand still.
        assert_eq!(game.run(1), 1, "lot 93 stopped advancing");
        assert!(game.stopped().is_none(), "lot halted: {:?}", game.stopped());
        let tick = game.lot().ticks();
        for &id in &travelers {
            let Some(object) = game.lot().world().objects.get(&id) else {
                continue;
            };
            let now = object.position;
            let before = last.insert(id, now).expect("traveler watched");
            // A walking Sim moves about a sixteenth of a tile a tick; more
            // than a tile and a half at once is a jump.
            let step = (now[0] - before[0]).abs().max((now[1] - before[1]).abs());
            if step > 24 || now[2] != before[2] {
                jumps.push(format!("tick {tick}: Sim {id} {before:?} -> {now:?}"));
            }
        }
    }
    let world = game.lot().world();
    assert_eq!(
        world.objects[&marker].attributes[3], 0,
        "the marker's arrival pass never finished"
    );
    let generic: Vec<&String> = game
        .errors()
        .iter()
        .filter(|e| e.contains("Generic Sims Call"))
        .collect();
    assert!(generic.is_empty(), "caught: {generic:?}");
    assert!(
        jumps.is_empty(),
        "travelers jumped after arrival:\n{}",
        jumps.join("\n")
    );
}
