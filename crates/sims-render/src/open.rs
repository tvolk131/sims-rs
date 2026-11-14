//! Opens a house for looking at: the lot as the game holds it once loaded,
//! and the art to draw it.
//!
//! This is not the full load `sims boot` runs (`crates/sims-cli/src/boot.rs`).
//! It reads the house file into a fresh world, widens the lot arrays the way
//! the load does (`World::read_arrays`) and works out the rooms
//! (`World::rebuild_rooms`), which cutaway walls need. The world it builds
//! belongs to the caller; nothing else shares it.

use crate::{Assets, Error, Result};
use sims_content::{Catalog, House, Index, Install, Neighborhood};
use sims_sim::World;
use std::path::Path;

/// A house ready to draw.
pub struct Opened {
    pub world: World,
    pub assets: Assets,
    /// The saved camera: rotation 0 to 3.
    pub rotation: u8,
}

/// Opens `house` (a `UserData*/Houses/House*.iff`) from the install at
/// `root`. The house's neighborhood is the folder above its `Houses`.
pub fn open_house(root: &Path, house: &Path) -> Result<Opened> {
    let install = Install::detect(root)?;
    let mut index = Index::build(&install)?;
    let neighborhood = house
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| Error::new("the house file is not in a Houses folder"))?;
    let neighborhood = Neighborhood::open(neighborhood)?;
    if let Some(characters) = neighborhood.characters()? {
        index.add_characters(&characters)?;
    }
    let catalog = Catalog::build(&index)?;
    let file = House::load(house, &catalog)?;
    let mut world = World::from_house(&file, &catalog).map_err(sim)?;
    let version = file
        .version()
        .ok_or_else(|| Error::new("the house file has no HOUS 1 chunk"))?;
    // The load shrinks the lot to the size the house asks for, in global
    // 23 (0 asks for the full 64), before it reads the arrays (load step
    // 3, `sims_vm::Interpreter::resize`). With no objects or arrays yet
    // there is nothing else to shrink.
    if let Some(&requested) = world.globals.get(23) {
        let requested = if requested == 0 {
            sims_sim::FULL_SIZE
        } else {
            requested
        };
        if (4..=sims_sim::FULL_SIZE).contains(&requested) {
            world.size = requested;
        }
    }
    // `from_house` keeps the floor and wall arrays as saved; the load
    // widens them into the lot's own, which a fresh lot does not have yet.
    for id in sims_sim::load::FLOORS
        .into_iter()
        .chain(sims_sim::load::WALLS)
    {
        world.arrays.remove(&id);
    }
    world.read_arrays(&file.arrays, version).map_err(sim)?;
    world.rebuild_rooms().map_err(sim)?;
    let rotation = file
        .view
        .as_ref()
        .map_or(0, |v| (v.camera_direction & 3) as u8);
    let assets = Assets::new(&install, index, catalog)?;
    Ok(Opened {
        world,
        assets,
        rotation,
    })
}

fn sim(e: impl std::fmt::Display) -> Error {
    Error::new(e.to_string())
}
