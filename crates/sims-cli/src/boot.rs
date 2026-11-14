//! `sims boot`: run the lot load for a house as far as it is implemented
//! and print the hash of the state it reaches.

use sims_content::{Catalog, House, Index, Install, Neighborhood, People};
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
    let created = interpreter.resize(&mut world)?.created;
    world.clear_family()?;
    let queued = world.removals.len();
    let removed = interpreter.clear(&mut world)?;
    // 6a and 6b: the lot arrays, then the saved floor and wall IDs.
    let version = house
        .version()
        .ok_or("the house file has no HOUS 1 chunk, which holds its version")?;
    world.read_arrays(&house.arrays, version)?;
    let renumbered =
        world.renumber_surfaces(house.floor_map.as_deref(), house.wall_map.as_deref())?;
    // 6c and 7: the type table, which marks the types whose scripts
    // changed. The listing below says which and why.
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
    let changed = interpreter.read_types(&mut world, &house.types)?;
    if changed.len() != mismatches.len() {
        return Err("the type table read and its listing disagree".into());
    }
    // 6d: the second clear, then 6e: the saved objects.
    let cleared_again = interpreter.clear(&mut world)?;
    let people = People::load(
        &install,
        &index,
        &catalog,
        &neighborhood,
        house.objects.instances.values().map(|o| o.guid),
    )?;
    let read = interpreter.read_objects(&mut world, &house.objects, &people)?;
    // 8: the lot-wide controllers, now among the saved objects. 9: the
    // ground level's object grid.
    let reconciled = interpreter.reconcile_controllers(&mut world)?;
    let repaired = world.repair_grid()?;
    let queued_again = world.removals.len();
    // The original now works out the lot's rooms from the walls it has
    // read. That makes it give every object its room and ask each object's
    // function 7 (step 10), and then count the rooms, which sets the
    // lot's indoor area.
    world.rebuild_rooms()?;
    let roomed = interpreter.assign_rooms(&mut world)?;
    world.update_rooms()?;
    // 11: the removal queue is emptied. 12: the household that lives in
    // the house is selected.
    let dropped = world.discard_removals();
    let number = neighborhood
        .houses()?
        .into_iter()
        .find(|(_, path)| path.file_name() == options.house.file_name())
        .map(|(number, _)| number)
        .ok_or("the house file's name does not give a house number")?;
    let (family, record) = neighborhood.family_in(number).ok_or(
        "no family lives in the house; selecting none is not checked against the original",
    )?;
    world.select_family(
        i16::try_from(family)?,
        i16::try_from(number)?,
        record.budget,
        record.magic_history()?,
    )?;
    // 13: every object is visited in registration order. The step stops
    // at the first visit that is not ported, with the lot as the visit
    // before it left it.
    let to_visit = world.order.len();
    let visited = interpreter.visit_objects(&mut world, &people);
    // 14: the indicators are created if no visit made them. 15: the end
    // of the household selection, which has nothing to do on a lot whose
    // people are all members of the household. 16: the loaded house is
    // announced, which asks for the indicators once more.
    let mut finished = None;
    if visited.stopped.is_none() {
        let made = interpreter.ensure_indicators(&mut world)?.is_some();
        world.finish_selection(record.flags)?;
        let made_again = interpreter.ensure_indicators(&mut world)?.is_some();
        finished = Some(made || made_again);
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
        "         lot cleared, {} objects removed; {} lot arrays read, {renumbered} floor \
         cells renumbered",
        removed.len(),
        world.arrays.len()
    );
    let _ = writeln!(
        out,
        "         type table read, {} types changed; lot cleared again, {} objects removed",
        changed.len(),
        cleared_again.len()
    );
    let _ = writeln!(
        out,
        "         {} saved objects read; their constructors drew {} C runtime random numbers",
        read.objects.len(),
        read.draws
    );
    let _ = writeln!(
        out,
        "         {} controllers created among them, {} duplicates removed; {} IDs queued \
         for removal",
        reconciled.created.len(),
        reconciled.removed.len(),
        queued_again
    );
    let _ = writeln!(
        out,
        "         ground object grid checked, {} cells cleared",
        repaired.len()
    );
    let rooms = &world.rooms.rooms;
    let _ = writeln!(
        out,
        "         {} rooms worked out ({} indoors, {} tiles split by a diagonal wall); {} \
         objects changed room, {} told the rooms changed",
        rooms.len(),
        rooms.values().filter(|room| !room.outdoors).count(),
        world.rooms.halves.len(),
        roomed.moved.len(),
        roomed.asked.len()
    );
    let _ = writeln!(
        out,
        "         rooms counted, indoor area {} tiles; {} queued removals dropped",
        world.globals[27],
        dropped.len()
    );
    let _ = writeln!(
        out,
        "         family {family} selected for house {number}, funds {}",
        world.budget.funds
    );
    let _ = writeln!(
        out,
        "         {} of {to_visit} objects visited, {} of them reset; {} IDs queued for removal",
        visited.objects.len(),
        visited.reset.len(),
        world.removals.len()
    );
    match &visited.stopped {
        Some(fault) => {
            let _ = writeln!(
                out,
                "         stopped in step 13 at the next visit: {fault}"
            );
        }
        None => {
            let _ = writeln!(
                out,
                "         indicators {}; household selection finished; house announced",
                if finished == Some(true) {
                    "created"
                } else {
                    "already on the lot"
                }
            );
            let _ = writeln!(
                out,
                "         stopped before the game selects a Sim and goes to live mode (step 17, \
                 not implemented yet)"
            );
        }
    }
    let _ = writeln!(
        out,
        "Lot:     {} objects, tick {}",
        world.objects.len(),
        world.ticks
    );
    let _ = writeln!(out, "Hash:    {}", world.hash_hex());
    Ok(out)
}
