//! Opens a saved house for the interface's tests and examples.

#![allow(dead_code)]

use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_content::{Catalog, House, Index, Install, Neighborhood};
use sims_ui::art::Art;
use sims_ui::household::{Faces, Snapshot};
use std::path::{Path, PathBuf};

/// The Goth family's house in the install's first neighborhood.
pub const GOTH_HOUSE: &str = "UserData/Houses/House05.iff";

pub struct Opened {
    pub snapshot: Snapshot,
    pub art: Art,
    pub faces: Faces,
    pub install: Install,
}

/// The install `SIMS_INSTALL` names, or None to skip.
pub fn install_root() -> Option<PathBuf> {
    std::env::var_os("SIMS_INSTALL").map(PathBuf::from)
}

/// Opens `house` below the install at `root` and reads what the control
/// panel shows.
///
/// The full load (`sims boot`, crates/sims-cli/src/boot.rs) lives in the
/// command-line tool, not in a library, so this stands in for its two
/// steps the panel shows: step 17 sets the funds to the family's, and
/// step 18 selects the family's first member. The original showed
/// $2,542 and Mortimer selected after loading this house, and the house
/// file alone says 2,515 and nobody.
pub fn open(root: &Path, house: &str) -> Opened {
    let install = Install::detect(root).unwrap();
    let mut index = Index::build(&install).unwrap();
    let house = root.join(house);
    let neighborhood = Neighborhood::open(house.parent().unwrap().parent().unwrap()).unwrap();
    if let Some(characters) = neighborhood.characters().unwrap() {
        index.add_characters(&characters).unwrap();
    }
    let catalog = Catalog::build(&index).unwrap();
    let file = House::load(&house, &catalog).unwrap();
    let mut world = sims_sim::World::from_house(&file, &catalog).unwrap();
    let number = neighborhood
        .houses()
        .unwrap()
        .into_iter()
        .find(|(_, path)| path.file_name() == house.file_name())
        .map(|(n, _)| n)
        .unwrap();
    let (_, family) = neighborhood.family_in(number).unwrap();
    world.budget.funds = family.budget;
    if let Some(first) = family.members.first()
        && let Some(object) = world
            .objects
            .values_mut()
            .find(|o| o.guid == *first && o.person.is_some())
    {
        object.selected = true;
    }
    let snapshot = Snapshot::read(&world, &family.members);
    let ui = UiGraphics::open(&install).unwrap();
    let art = Art::load(&ui, US_ENGLISH, &sims_ui::hud::RESOURCES).unwrap();
    let faces = Faces::load(&index, &catalog, &family.members).unwrap();
    Opened {
        snapshot,
        art,
        faces,
        install,
    }
}
