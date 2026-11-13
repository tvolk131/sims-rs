//! `sims boot`: load a house into simulation state and print its hash.

use sims_content::{Catalog, House, Index, Install, Neighborhood};
use sims_sim::{Snapshots, World};
use std::error::Error;
use std::fmt::Write as _;
use std::path::Path;

pub struct Options<'a> {
    pub root: &'a Path,
    /// A house file inside a neighborhood's `Houses` directory.
    pub house: &'a Path,
    pub ticks: u32,
    pub snapshot: Option<&'a Path>,
}

pub fn run(options: &Options<'_>) -> Result<String, Box<dyn Error>> {
    if options.ticks > 0 {
        return Err("running ticks needs the script interpreter (roadmap step 3)".into());
    }
    let install = Install::detect(options.root)?;
    let directory = options
        .house
        .parent()
        .and_then(Path::parent)
        .ok_or("the house file must be inside a neighborhood's Houses directory")?;
    let neighborhood = Neighborhood::open(directory)?;
    let mut index = Index::build(&install)?;
    if let Some(characters) = neighborhood.characters()? {
        index.add_directory("Characters", &characters, false)?;
    }
    let catalog = Catalog::build(&index)?;
    let house = House::load(options.house, &catalog)?;
    let world = World::from_house(&house, &catalog)?;
    if let Some(path) = options.snapshot {
        Snapshots::open(path)?.save("boot", &world)?;
    }
    let people = world
        .objects
        .values()
        .filter(|o| o.person.is_some())
        .count();
    let mut out = String::new();
    let _ = writeln!(out, "House:   {}", options.house.display());
    let _ = writeln!(
        out,
        "Objects: {} ({people} people), tick {}",
        world.objects.len(),
        world.ticks
    );
    let _ = writeln!(out, "Hash:    {}", world.hash_hex());
    Ok(out)
}
