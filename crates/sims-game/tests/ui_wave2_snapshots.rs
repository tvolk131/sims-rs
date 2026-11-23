//! Headless snapshots of the UI Honesty Audit Wave 2 panels on the Goth
//! house (Lot 5) with the real Complete Collection art: the native phone
//! book (kind 10), the filtered wardrobe (kind 6), charm (kind 14) and pet
//! (kind 9) pickers, the Spellbook, the Photo Album, the Travel dialog and
//! the Import Family picker.
//!
//! Each panel must open and draw. `SIMS_UI_SNAPSHOT_OUT=<dir>` also writes
//! one PNG per panel for visual review. Needs `SIMS_INSTALL`; skips cleanly
//! without it.

use sims_content::{Catalog, Index, Install};
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use sims_game::game::goth_house;
use sims_game::neighborhood::NeighborhoodShell;
use sims_game::{Frames, Game, Options};
use sims_render::{Canvas, png};
use sims_sim::neighbors::{FAMILY, KIND, Row};
use sims_ui::art::Art;
use sims_ui::neighborhood::{BarButton, BarLayout};
use sims_vm::SpecialPanelRequest;
use std::path::{Path, PathBuf};

const SIZE: [i32; 2] = [800, 600];

fn env_dir(name: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os(name)?);
    dir.is_dir().then_some(dir)
}

fn snapshot(name: &str, canvas: &Canvas) {
    let Some(out) = std::env::var_os("SIMS_UI_SNAPSHOT_OUT").map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&out).expect("create snapshot dir");
    std::fs::write(out.join(format!("{name}.png")), png::encode(canvas)).expect("write PNG");
}

fn open_goth(root: &Path) -> Game {
    let mut game = Game::open(&Options {
        house: goth_house(root),
        install: root.to_path_buf(),
        answers: vec![3],
        play: false,
        frames: Frames::default(),
    })
    .expect("open Goth house");
    game.resize(SIZE);
    game.render();
    game
}

/// The GUID of a person type whose skeleton (`STR# 200` entry 0) is a dog's.
fn dog_guid(install: &Install) -> Option<u32> {
    let index = Index::build(install).ok()?;
    let catalog = Catalog::build(&index).ok()?;
    catalog
        .objects
        .iter()
        .filter(|o| o.kind == 2)
        .find_map(|o| {
            let bytes = index.resources.get(o.resource)?.read().ok()?;
            let iff = Iff::parse(bytes).ok()?;
            let chunk = iff.get(b"STR#", 200)?;
            let table = StringTable::parse(iff.data(chunk)).ok()?;
            let skeleton = table.get(0, 1)?.to_ascii_lowercase();
            skeleton.contains("dog").then_some(o.guid)
        })
}

/// Opens `request` over `world` (closing the previous panel), checks a
/// modal is up and snapshots it.
fn preview(game: &mut Game, name: &str, request: SpecialPanelRequest, world: &sims_sim::World) {
    game.preview_special_panel(request.clone(), world);
    game.render();
    assert!(game.has_dialog(), "{name} must open a modal");
    assert_eq!(game.active_special_panel_request(), Some(&request));
    snapshot(name, game.canvas());
}

#[test]
fn wave2_panels_open_and_draw_with_native_art() {
    let Some(root) = env_dir("SIMS_INSTALL") else {
        eprintln!("skipping: set SIMS_INSTALL to snapshot the Wave 2 panels");
        return;
    };
    let install = Install::detect(&root).expect("open install");

    let mut game = open_goth(&root);
    let world = game.lot().world().clone();
    let sim = world.selected().expect("Mortimer selected");
    let neighbor = world.objects[&sim]
        .person
        .as_ref()
        .and_then(|p| p.data.get(31).copied())
        .expect("Mortimer's neighbor id");

    // A fresh Mortimer knows only his family, whom the phone book leaves
    // out ("doesn't know anyone"). Give him ten townies to page through.
    preview(
        &mut game,
        "phonebook-empty",
        SpecialPanelRequest::CallNeighbor {
            caller_neighbor: neighbor,
        },
        &world,
    );
    assert!(
        game.phonebook_snapshot()
            .expect("phone book")
            .entries
            .is_empty()
    );
    let mut social_world = world.clone();
    let goth_family = social_world
        .neighbors
        .slots
        .iter()
        .flatten()
        .find(|r| r.id == neighbor)
        .map(|r| r.data[FAMILY])
        .expect("Mortimer's record");
    let townies: Vec<i16> = social_world
        .neighbors
        .slots
        .iter()
        .flatten()
        .filter(|r| r.id > 0 && r.data[KIND] <= 1 && r.data[FAMILY] > 0)
        .filter(|r| r.data[FAMILY] != goth_family)
        .map(|r| r.id)
        .take(10)
        .collect();
    assert!(!townies.is_empty(), "the neighborhood has other families");
    let mortimer = social_world
        .neighbors
        .slots
        .iter_mut()
        .flatten()
        .find(|r| r.id == neighbor)
        .expect("Mortimer's record");
    for (i, &id) in townies.iter().enumerate() {
        let i = i as i32;
        mortimer.relationships.push(Row {
            target: i32::from(id),
            values: vec![i * 22 - 60, i * 15 - 30],
        });
    }
    preview(
        &mut game,
        "phonebook",
        SpecialPanelRequest::CallNeighbor {
            caller_neighbor: neighbor,
        },
        &social_world,
    );
    let listed = game.phonebook_snapshot().expect("phone book").entries.len();
    assert_eq!(
        listed,
        townies.len(),
        "every townie Mortimer knows is listed"
    );

    preview(
        &mut game,
        "wardrobe-everyday",
        SpecialPanelRequest::WardrobeShop {
            person: sim,
            mode: 0,
        },
        &world,
    );
    preview(
        &mut game,
        "charm-transform",
        SpecialPanelRequest::CharmTransform { person: sim },
        &world,
    );

    // A dog in Mortimer's place, so the pet picker has a skeleton to turn.
    if let Some(guid) = dog_guid(&install) {
        let mut dog_world = world.clone();
        let dog = dog_world.objects.get_mut(&sim).expect("Mortimer");
        dog.guid = guid;
        dog.person.as_mut().expect("person").data[KIND] = 2;
        preview(
            &mut game,
            "pet-picker",
            SpecialPanelRequest::PetPicker { pet: sim },
            &dog_world,
        );
        // Try On turns the breed on the 3D turntable.
        game.shop_try_on_selected();
        game.shop_rotate_preview(45);
        game.render();
        let snap = game.shop_snapshot().expect("pet picker");
        assert!(
            snap.preview_image.is_some(),
            "the pet picker previews the breed in 3D: {:?}",
            snap.preview_label
        );
        snapshot("pet-picker-preview", game.canvas());
    } else {
        panic!("Complete Collection must define a dog person type");
    }

    preview(
        &mut game,
        "spellbook",
        SpecialPanelRequest::SpellBook { caller: Some(sim) },
        &world,
    );
    assert!(game.book_open(), "Spellbook opens");
    let title = &game.book_snapshot().expect("Spellbook").title;
    assert_eq!(title, "Goth Family Spellbook");

    // Shell panels: Photo Album, Travel dialog, Import Family.
    let tmp = std::env::temp_dir().join(format!("sims-ui-wave2-snapshots-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("temp dir");
    let mut shell = NeighborhoodShell::open(
        root.clone(),
        tmp.join("saves.redb"),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("open NeighborhoodShell");
    shell.resize(SIZE);
    shell.enter_lot(5).expect("enter Goth house");
    shell.render();

    shell.open_photo_album().expect("open Photo Album");
    shell.render();
    assert!(shell.album_open());
    snapshot("photo-album", shell.canvas());
    shell.close_photo_album();

    shell
        .open_default_travel_dialog()
        .expect("open the Travel dialog");
    shell.render();
    assert!(shell.travel_dialog_open());
    snapshot("travel-dialog", shell.canvas());
    shell.close_travel_dialog();

    shell.leave_lot(false).expect("back to the neighborhood");
    shell.render();
    // Export the Goths as a `.FAM` so the Import picker has a family to list.
    let stored = shell
        .store()
        .load_neighborhood("UserData")
        .expect("load UserData")
        .expect("UserData exists");
    let family = stored.family_in(5).expect("Goths on lot 5").0;
    let chars = shell
        .store()
        .load_character_files("UserData")
        .expect("load characters");
    let fam = sims_vm::caf::export_family_fam(&stored, &chars, family, "Snapshot", 1, true)
        .expect("export .FAM");
    let import_dir = tmp.join("Import");
    std::fs::create_dir_all(&import_dir).expect("Import dir");
    std::fs::write(import_dir.join("Snapshot_99.FAM"), fam).expect("write .FAM");
    shell.add_import_search_dir(import_dir);
    let layout = BarLayout::compute(800, 600, shell.neighborhood_state(), &Art::default());
    let rect = layout
        .button(BarButton::ImportFamily)
        .expect("Import Family button")
        .hit_rect;
    let point = [rect.x + rect.width / 2, rect.y + rect.height / 2];
    shell.press(point);
    shell.release(point);
    shell.render();
    assert!(shell.import_picker_open(), "Import Family opens its picker");
    snapshot("import-family", shell.canvas());
}
