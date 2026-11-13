//! `sims boot`: load a house into simulation state and print its hash.

use sims_content::{Catalog, House, Index, Install, Neighborhood};
use sims_sim::load::Context;
use sims_sim::{Snapshots, World};
use sims_vm::{Callback, Program};
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
        index.add_characters(&characters)?;
    }
    let catalog = Catalog::build(&index)?;
    let house = House::load(options.house, &catalog)?;
    let mut world = World::from_house(&house, &catalog)?;
    // The load steps, in the original's order (docs/roadmap.md, load sequence). The
    // boot stops at the first one not implemented yet.
    world.apply_context(Context::COMPLETE_COLLECTION)?;
    let types = &house.types.entries;
    let program = Program::link(&index, &catalog, types.iter().map(|t| t.guid))?;
    let mut mismatches = Vec::new();
    for entry in types {
        let object = catalog.object(entry.guid).expect("linked above");
        let current = [Callback::Init, Callback::Main]
            .map(|c| program.signature(object.resource, program.callback(object, c)));
        if current != [entry.init_signature, entry.main_signature] {
            mismatches.push(format!(
                "  type {} {:?} (0x{:08X}): saved {:08x}/{:08x}, linked {:08x}/{:08x}",
                entry.id,
                entry.name,
                entry.guid,
                entry.init_signature,
                entry.main_signature,
                current[0],
                current[1]
            ));
        }
    }
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
    let _ = writeln!(
        out,
        "Linked:  {} files; {} of {} object types match their saved signatures",
        program.libraries.len(),
        types.len() - mismatches.len(),
        types.len()
    );
    for line in &mismatches {
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(
        out,
        "Load:    globals set; stopped before the global controllers (not implemented yet)"
    );
    let _ = writeln!(out, "Hash:    {}", world.hash_hex());
    Ok(out)
}
