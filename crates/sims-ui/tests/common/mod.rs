//! Opens a saved house for the interface's tests and examples.

#![allow(dead_code)]

use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_content::{Index, Install, Neighborhood, Surfaces};
use sims_ui::art::Art;
use sims_ui::house;
use sims_ui::household::{Acquaintances, Careers, Faces, Snapshot};
use sims_vm::boot::{Frames, Lot};
use std::path::{Path, PathBuf};

/// The Goth family's house in the install's first neighborhood.
pub const GOTH_HOUSE: &str = "UserData/Houses/House05.iff";

pub struct Opened {
    pub careers: Careers,
    pub house: house::Tuning,
    pub kinds: house::Kinds,
    pub snapshot: Snapshot,
    pub art: Art,
    pub faces: Faces,
    pub install: Install,
    /// The lot's world after the load, and the index and catalog it was
    /// loaded from.
    pub world: sims_sim::World,
    pub index: Index,
    pub catalog: sims_content::Catalog,
}

/// The install `SIMS_INSTALL` names, or None to skip.
pub fn install_root() -> Option<PathBuf> {
    std::env::var_os("SIMS_INSTALL").map(PathBuf::from)
}

/// Opens `house` below the install at `root` and reads what the control
/// panel shows.
///
/// The house is loaded as the game loads it (`sims_vm::boot::Lot`), up to
/// the frame before the first tick: steps 17 and 18 set the funds to the
/// family's and select its first member, and the neighborhood's people
/// are read for the relationship panel. The original showed $2,542 and
/// Mortimer selected after loading the Goths' house, and the house file
/// alone says 2,515 and nobody.
pub fn open(root: &Path, house: &str) -> Opened {
    let install = Install::detect(root).unwrap();
    let mut index = Index::build(&install).unwrap();
    let house = root.join(house);
    let neighborhood = Neighborhood::open(house.parent().unwrap().parent().unwrap()).unwrap();
    if let Some(characters) = neighborhood.characters().unwrap() {
        index.add_characters(&characters).unwrap();
    }
    let lot = Lot::load(
        root,
        &house,
        &[],
        Frames {
            after: Vec::new(),
            every: Vec::new(),
        },
    )
    .unwrap();
    let report = lot.load_report();
    assert!(
        report.finished.is_some(),
        "the load stopped: {:?}",
        report.stopped
    );
    let (world, catalog) = (lot.world(), lot.catalog());
    let number = neighborhood
        .houses()
        .unwrap()
        .into_iter()
        .find(|(_, path)| path.file_name() == house.file_name())
        .map(|(n, _)| n)
        .unwrap();
    let (_, family) = neighborhood.family_in(number).unwrap();
    let careers = Careers::load(&index).unwrap();
    // The friendship threshold the original's tuning gives (0x6c4398).
    let known = Acquaintances::read(&index, catalog, install.expansions, 50);
    let snapshot = Snapshot::read(world, &family.members)
        .with_careers(&careers)
        .with_relations(world, &family.members, &known);
    let mut house = house::Tuning::load(&index, catalog).unwrap();
    house.prices = house::Prices::load(&install, &Surfaces::build(&install).unwrap());
    let mut kinds = house::Kinds::default();
    kinds.learn(world, &index, catalog);
    let snapshot = snapshot.with_house(world, &house, &kinds);
    let ui = UiGraphics::open(&install).unwrap();
    let mut art = Art::load(&ui, US_ENGLISH, &sims_ui::hud::RESOURCES).unwrap();
    art.load_optional(&ui, US_ENGLISH, sims_ui::balloon::BACKGROUND);
    let faces = Faces::load(&index, catalog, &family.members).unwrap();
    Opened {
        careers,
        house,
        kinds,
        snapshot,
        art,
        faces,
        install,
        world: world.clone(),
        catalog: catalog.clone(),
        index,
    }
}
