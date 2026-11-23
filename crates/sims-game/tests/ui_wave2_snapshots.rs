//! Headless snapshots of the UI Honesty Audit Wave 2 panels on the Goth
//! house (Lot 5) with the real Complete Collection art: the native phone
//! book (kind 10), the filtered wardrobe (kind 6), charm (kind 14) and pet
//! (kind 9) pickers, the Spellbook, the Photo Album, the Travel dialog and
//! the Import Family picker.
//!
//! Each panel must open and draw. `SIMS_UI_SNAPSHOT_OUT=<dir>` also writes
//! one PNG per panel for visual review. Needs `SIMS_INSTALL`; skips cleanly
//! without it.

use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_content::{Catalog, Index, Install};
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use sims_game::game::goth_house;
use sims_game::hud::DialogKey;
use sims_game::neighborhood::NeighborhoodShell;
use sims_game::{Frames, Game, Options};
use sims_render::{Canvas, png};
use sims_sim::neighbors::{FAMILY, KIND, Row};
use sims_ui::art::Art;
use sims_ui::neighborhood::{BarButton, BarLayout};
use sims_vm::SpecialPanelRequest;
use std::path::{Path, PathBuf};

const SIZE: [i32; 2] = [800, 600];

/// The folder `name` names, or `None` when it is unset. A set variable
/// that is not a folder fails the test instead of skipping it, so a typo or
/// an unmounted fixture cannot pass with nothing checked.
fn env_dir(name: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os(name)?);
    assert!(dir.is_dir(), "{name}={} is not a folder", dir.display());
    Some(dir)
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

/// Asserts the panel actually painted: at least a third of the central
/// 400×300 window differs from `before`, the same view with no panel.
fn assert_drawn(name: &str, before: &[u32], after: &Canvas) {
    assert_eq!(
        before.len(),
        after.color.len(),
        "{name}: frame size changed"
    );
    let (w, h) = (after.width, after.color.len() / after.width);
    let (x0, y0) = (w / 2 - 200, h / 2 - 150);
    let changed = (y0..y0 + 300)
        .flat_map(|y| (x0..x0 + 400).map(move |x| y * w + x))
        .filter(|&i| before[i] != after.color[i])
        .count();
    assert!(
        changed * 3 >= 400 * 300,
        "{name} must draw over the view: only {changed} of 120000 central pixels changed"
    );
}

/// Asserts the album drew the native `scrapback.bmp` (1200, 700×560,
/// centered): its left bevel (`x` 2..16, `y` 100..400, covered by nothing
/// else) matches the bitmap. The 9-slice fallback fails this.
fn assert_scrapback(install: &Install, canvas: &Canvas) {
    let ui = UiGraphics::open(install).expect("open UIGraphics");
    let art = sims_ui::album::load_art(&ui, US_ENGLISH).expect("load album art");
    let scrap = art
        .image(sims_ui::album::SCRAP_BACK)
        .expect("scrapback.bmp loads");
    assert_eq!([scrap.width, scrap.height], [700, 560]);
    let h = canvas.color.len() / canvas.width;
    let (ox, oy) = ((canvas.width - 700) / 2, (h - 560) / 2);
    let (mut opaque, mut matched) = (0, 0);
    for y in 100..400 {
        for x in 2..16 {
            let i = (y * 700 + x) * 4;
            let px = &scrap.rgba[i..i + 4];
            if px[3] != 255 {
                continue;
            }
            opaque += 1;
            let ours = canvas.color[(oy + y) * canvas.width + ox + x];
            let ours = [(ours >> 16) as u8, (ours >> 8) as u8, ours as u8];
            if ours.iter().zip(px).all(|(a, b)| a.abs_diff(*b) <= 8) {
                matched += 1;
            }
        }
    }
    assert!(
        opaque >= 1000,
        "scrapback's left bevel is opaque ({opaque})"
    );
    assert!(
        matched * 100 >= opaque * 95,
        "the album draws native scrapback.bmp: {matched} of {opaque} bevel pixels match"
    );
}

/// Opens `request` over `world` (closing the previous panel), checks a
/// modal is up and painted, and snapshots it.
fn preview(
    game: &mut Game,
    name: &str,
    request: SpecialPanelRequest,
    world: &sims_sim::World,
    before: &[u32],
) {
    game.preview_special_panel(request.clone(), world);
    game.render();
    assert!(game.has_dialog(), "{name} must open a modal");
    assert_eq!(game.active_special_panel_request(), Some(&request));
    assert_drawn(name, before, game.canvas());
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
    let before = game.canvas().color.clone();
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
        &before,
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
        &before,
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
        &before,
    );
    let wardrobe = game.shop_snapshot().expect("wardrobe").items.len();
    assert!(wardrobe > 0, "the everyday wardrobe lists outfits");
    preview(
        &mut game,
        "charm-transform",
        SpecialPanelRequest::CharmTransform { person: sim },
        &world,
        &before,
    );
    let charms = game.shop_snapshot().expect("charm").items.len();
    assert!(charms > 0, "the charm transform lists outfits");

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
            &before,
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
        &before,
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
    let lot_view = shell.canvas().color.clone();

    shell.open_photo_album().expect("open Photo Album");
    shell.render();
    assert!(shell.album_open());
    assert_drawn("photo-album", &lot_view, shell.canvas());
    assert_scrapback(&install, shell.canvas());
    snapshot("photo-album", shell.canvas());
    shell.close_photo_album();

    shell
        .open_default_travel_dialog()
        .expect("open the Travel dialog");
    shell.render();
    assert!(shell.travel_dialog_open());
    assert_drawn("travel-dialog", &lot_view, shell.canvas());
    snapshot("travel-dialog", shell.canvas());
    shell.close_travel_dialog();

    shell.leave_lot(false).expect("back to the neighborhood");
    shell.render();
    let neighborhood_view = shell.canvas().color.clone();
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
    assert_drawn("import-family", &neighborhood_view, shell.canvas());
    snapshot("import-family", shell.canvas());
}

/// Every point on the lot (on a 6 px grid, off the panel) with something
/// under it.
fn lot_points(game: &Game) -> Vec<[i32; 2]> {
    (0..SIZE[1])
        .step_by(6)
        .flat_map(|y| (0..SIZE[0]).step_by(6).map(move |x| [x, y]))
        .filter(|&p| !game.over_panel(p) && game.under(p).is_some())
        .collect()
}

/// The Goth house `ticks` in, Mortimer selected, with no modal up.
fn goth_at(root: &Path, ticks: u32) -> Game {
    let mut game = open_goth(root);
    game.run(ticks);
    game.render();
    game
}

/// A point on the lot whose pie-menu listing changes the world at `ticks`
/// (its check trees commit), found on a throwaway copy of the same lot.
fn mutating_point(root: &Path, ticks: u32) -> [i32; 2] {
    let mut probe = goth_at(root, ticks);
    for point in lot_points(&probe) {
        let before = probe.lot().world().hash_hex();
        probe.hover(point);
        if probe.lot().world().hash_hex() != before {
            return point;
        }
    }
    panic!("no listing on the Goth lot changes the world at tick {ticks}");
}

const TICKS: u32 = 93;

#[test]
fn a_panel_preview_never_changes_the_world() {
    let Some(root) = env_dir("SIMS_INSTALL") else {
        return;
    };
    let target = mutating_point(&root, TICKS);
    let mut game = goth_at(&root, TICKS);
    let world = game.lot().world().clone();
    let sim = world.selected().expect("Mortimer selected");
    let neighbor = world.objects[&sim]
        .person
        .as_ref()
        .and_then(|p| p.data.get(31).copied())
        .expect("Mortimer's neighbor id");
    let before = game.lot().world().hash_hex();

    game.preview_special_panel(
        SpecialPanelRequest::CallNeighbor {
            caller_neighbor: neighbor,
        },
        &world,
    );
    game.render();
    assert_eq!(game.lot().world().hash_hex(), before, "opening a preview");
    // Pointing at the lot behind the modal lists nothing, ending on the
    // target so the pointer is left over it.
    for point in lot_points(&game).into_iter().chain([target]) {
        game.hover(point);
        game.render();
        assert_eq!(game.listed(), None, "nothing is listed behind a modal");
    }
    assert_eq!(
        game.lot().world().hash_hex(),
        before,
        "hovering behind a preview"
    );
    // Dismissing leaves no stale pointer for the next frame to list.
    game.dialog_confirm_key(DialogKey::Escape);
    game.render();
    game.render();
    assert!(!game.has_dialog(), "Escape dismisses the preview");
    assert_eq!(
        game.listed(),
        None,
        "nothing is listed until the pointer moves"
    );
    assert_eq!(
        game.lot().world().hash_hex(),
        before,
        "dismissing a preview"
    );

    // Fresh pointer input over the target does list, and changes the world.
    game.hover(target);
    assert!(
        game.listed().is_some(),
        "with no modal, pointing lists a pie menu"
    );
    assert_ne!(
        game.lot().world().hash_hex(),
        before,
        "the target's listing changes the world"
    );
}

#[test]
fn the_community_shop_blocks_listing_behind_it() {
    let Some(root) = env_dir("SIMS_INSTALL") else {
        return;
    };
    let target = mutating_point(&root, TICKS);
    let mut game = goth_at(&root, TICKS);
    let before = game.lot().world().hash_hex();
    assert!(
        game.open_shop(sims_ui::shop::ShopKind::PetAdoption),
        "the shop opens for a live household"
    );
    game.render();
    for point in lot_points(&game).into_iter().chain([target]) {
        game.hover(point);
        game.render();
        assert_eq!(game.listed(), None, "nothing is listed behind the shop");
    }
    assert_eq!(
        game.lot().world().hash_hex(),
        before,
        "hovering behind the shop"
    );
}

#[test]
fn enter_behind_a_panel_preview_leaves_the_vm_prompt_pending() {
    let Some(root) = env_dir("SIMS_INSTALL") else {
        return;
    };
    // No scripted answers: the Goths' opening dialog waits for the player.
    let mut game = Game::open(&Options {
        house: goth_house(&root),
        install: root.to_path_buf(),
        answers: Vec::new(),
        play: false,
        frames: Frames::default(),
    })
    .expect("open Goth house");
    game.resize(SIZE);
    // Tick until the opening dialog stops the lot to wait for the player.
    game.run(50);
    game.render();
    assert!(game.asking().is_some(), "the opening dialog waits");
    let world = game.lot().world().clone();
    let sim = world.selected().expect("Mortimer selected");

    game.preview_special_panel(SpecialPanelRequest::CharmTransform { person: sim }, &world);
    game.render();
    game.dialog_submit();
    game.render();
    // Enter picks in the preview in front, which answers nothing and closes
    // it; the VM prompt behind stays pending.
    assert!(
        game.asking().is_some(),
        "Enter goes to the preview in front, not the VM prompt behind it"
    );
    assert!(
        game.active_special_panel_request().is_none(),
        "a pick closes the preview"
    );

    // Escape on a phone book preview closes only the preview.
    let neighbor = world.objects[&sim]
        .person
        .as_ref()
        .and_then(|p| p.data.get(31).copied())
        .expect("Mortimer's neighbor id");
    game.preview_special_panel(
        SpecialPanelRequest::CallNeighbor {
            caller_neighbor: neighbor,
        },
        &world,
    );
    game.render();
    game.dialog_confirm_key(DialogKey::Escape);
    game.render();
    assert!(game.asking().is_some(), "Escape closes only the preview");
    assert!(game.active_special_panel_request().is_none());

    // With no preview up, Enter answers the prompt.
    game.dialog_submit();
    game.render();
    assert!(
        game.asking().is_none(),
        "with no preview, Enter answers the prompt"
    );
}
