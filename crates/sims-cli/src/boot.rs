//! `sims boot`: run the lot load for a house as far as it is implemented
//! and print the hash of the state it reaches.

use sims_content::{Catalog, House, Index, Install, Neighborhood};
use sims_sim::load::Context;
use sims_sim::{Snapshots, World};
use sims_vm::{Callback, Interpreter, Program, Stream};
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
    let context = Context::COMPLETE_COLLECTION;
    // The original has every definition registered, and the resize step
    // walks them all, so the whole catalog is linked.
    let mut program = Program::link(&index, &catalog, catalog.objects.iter().map(|o| o.guid))?;
    program.zoning = neighborhood.zoning(context.expansions)?;
    let interpreter = Interpreter::new(&program, &catalog, Stream::Global);
    // The lot the game holds before the load: empty, with the type
    // attributes the neighborhood saved and the two indicator objects.
    let mut world = World::empty();
    world.type_attributes = program.type_attributes(&catalog)?;
    if let Some(saved) = &neighborhood.type_attributes {
        world.read_type_attributes(saved);
    }
    interpreter.create_indicators(&mut world)?;
    // The load steps, in the original's order (docs/roadmap.md, load sequence). The
    // boot stops at the first one not implemented yet.
    world.read_simulator(&house.simulator);
    world.apply_context(context)?;
    let created = interpreter.resize(&mut world)?;
    world.clear_family()?;
    let queued = world.removals.len();
    let removed = interpreter.clear(&mut world)?;
    world.read_arrays(&house.arrays)?;
    // The load stops here. Next the original renumbers the saved floor and
    // wall IDs, which is not ported. After that it reads the type table,
    // which marks the types whose scripts changed; that is ported
    // (`Interpreter::read_types`), so say what it will mark.
    if house.guid_table {
        return Err("the house has a GUID translation table (Gtab), which is not ported".into());
    }
    let types = &house.types.entries;
    let mut mismatches = Vec::new();
    for entry in types {
        let object = catalog
            .object(entry.guid)
            .ok_or_else(|| format!("type {} has no definition", entry.id))?;
        if interpreter.type_changed(entry) == Some(true) {
            let current = [Callback::Init, Callback::Main]
                .map(|c| program.signature(object.resource, program.callback(object, c)));
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
    let saved = &house.objects.instances;
    let people = saved.values().filter(|o| o.person.is_some()).count();
    let mut out = String::new();
    let _ = writeln!(out, "House:   {}", options.house.display());
    let _ = writeln!(
        out,
        "Saved:   {} objects ({people} people), tick {}",
        saved.len(),
        house.simulator.ticks
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
        "Load:    lot resized to {} tiles, {} controllers created, {queued} queued for removal",
        world.size,
        created.len(),
    );
    let _ = writeln!(
        out,
        "         lot cleared, {} objects removed; {} lot arrays read",
        removed.len(),
        world.arrays.len()
    );
    let _ = writeln!(
        out,
        "         stopped before the saved floors and walls are renumbered (not implemented \
         yet)"
    );
    let _ = writeln!(
        out,
        "Lot:     {} objects, tick {}",
        world.objects.len(),
        world.ticks
    );
    let _ = writeln!(out, "Hash:    {}", world.hash_hex());
    Ok(out)
}
