//! A house booted for play: the lot load run in the original's order, then
//! one tick at a time.
//!
//! [`Lot::load`] runs every load step (docs/roadmap.md, load sequence)
//! and leaves the lot as the original's first tick finds it, or as the
//! first step that is not ported left it ([`LoadReport::stopped`]).
//! [`Lot::step`] runs one lot update ([`Interpreter::lot_update`]) and
//! then the house view's frame when [`Frames`] draws one after it.
//! Between steps, [`Lot::world`] reads the state; nothing outside the
//! simulation writes it.
//!
//! `sims boot` prints what this does; presentation crates use it to
//! watch a lot run.

use crate::{Answers, Callback, Interpreter, Program, Stream, Update};
use sims_content::{Animations, Catalog, House, Index, Install, Neighborhood, People};
use sims_sim::World;
use sims_sim::load::Context;
use sims_sim::motives::Tuning;
use std::path::Path;

/// Anything that stops a boot, with a message that says where.
pub type BootError = Box<dyn std::error::Error + Send + Sync>;

/// The level the house view shows: the ground.
const VIEW_LEVEL: i8 = 1;

/// When the house view draws a frame, as far as drawing writes the
/// simulation's state (`frame.rs`). The game's main loop and the window
/// system decide that, not the simulation, so it is an input here. Tick 0
/// means before the first tick.
///
/// The default, `0,1,2,3,33+30`, is what three captures of the original
/// running the Goth house recorded, with a hardware watchpoint on the
/// house view's draw guard (`docs/decisions.md`): a frame before the
/// first lot update, after updates 1, 2 and 3 (many times while a dialog
/// waited), after update 33, and after every 30th update from there (63,
/// 93, ...). Frames write two things: the first frame that draws a
/// person places its animator (on this house, the frame after tick 33
/// draws person 259, made in tick 6), and every frame counts the rooms
/// that wait to be counted (0x4a8895). So the schedule changes the
/// state: with a frame after every tick, the rooms are counted sooner.
/// On this house that changes no state the captures hold through tick
/// 261, but it changes the rooms' state in between.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frames {
    /// Ticks after which a frame is drawn.
    pub after: Vec<u32>,
    /// A frame after this tick and after every `step`th tick from it.
    pub from: Option<u32>,
    /// The ticks between two of those frames, at least 1.
    pub step: u32,
}

impl Default for Frames {
    fn default() -> Self {
        Self {
            after: vec![0, 1, 2, 3],
            from: Some(33),
            step: 30,
        }
    }
}

impl Frames {
    /// Reads a list such as `0,33+30`: a tick number draws a frame after
    /// that tick (0: before the first), `N+` after every tick from N on,
    /// and `N+S` after N and every S-th tick from there. `none` draws no
    /// frame.
    pub fn parse(text: &str) -> Option<Self> {
        let mut frames = Self {
            after: Vec::new(),
            from: None,
            step: 1,
        };
        if text == "none" {
            return Some(frames);
        }
        for part in text.split(',') {
            match part.split_once('+') {
                Some((from, step)) => {
                    frames.from = Some(from.parse().ok()?);
                    frames.step = if step.is_empty() {
                        1
                    } else {
                        step.parse().ok().filter(|&s| s > 0)?
                    };
                }
                None => frames.after.push(part.parse().ok()?),
            }
        }
        Some(frames)
    }

    /// Whether a frame is drawn after tick `tick`.
    pub fn after(&self, tick: u32) -> bool {
        self.after.contains(&tick)
            || self
                .from
                .is_some_and(|from| tick >= from && (tick - from).is_multiple_of(self.step))
    }
}

/// What the load did, step by step, for reports.
#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    /// Objects the house file saved, how many of them are people, and the
    /// tick count it saved.
    pub saved_objects: usize,
    pub saved_people: usize,
    pub saved_ticks: u32,
    /// Files linked.
    pub libraries: usize,
    /// Entries of the house's type table, and one line for each whose
    /// saved script signatures differ from the linked ones.
    pub types: usize,
    pub mismatches: Vec<String>,
    /// Step 4: controllers the resize created, and IDs it queued for
    /// removal.
    pub created: usize,
    pub queued: usize,
    /// Step 5: objects the first clear removed. Step 6: floor cells
    /// renumbered.
    pub removed: usize,
    pub renumbered: usize,
    /// Step 7: types whose scripts changed; then objects the second clear
    /// removed.
    pub changed: usize,
    pub cleared_again: usize,
    /// Step 6e: saved objects read, and the C runtime random numbers their
    /// constructors drew.
    pub read: usize,
    pub draws: u32,
    /// Step 8: controllers created among the saved objects, duplicates
    /// removed, and IDs then queued for removal.
    pub reconciled_created: usize,
    pub reconciled_removed: usize,
    pub queued_again: usize,
    /// Step 9: ground grid cells cleared.
    pub repaired: usize,
    /// Step 10: objects that changed room, and objects told the rooms
    /// changed.
    pub moved: usize,
    pub asked: usize,
    /// Step 11: queued removals dropped.
    pub dropped: usize,
    /// Step 12: the family selected and its house number.
    pub family: u16,
    pub house: u32,
    /// Step 13: objects to visit, objects visited and those reset.
    pub to_visit: usize,
    pub visited: usize,
    pub reset: usize,
    /// The step-13 visit that is not ported, if the load stopped there.
    pub stopped: Option<String>,
    /// Steps 14 to 22, when the load completed: whether the indicators
    /// were made, the Sim selected, and the rooms counted.
    pub finished: Option<Finished>,
}

/// The end of a completed load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Finished {
    pub indicators_made: bool,
    pub sim: Option<u16>,
    pub counted: usize,
}

/// A booted house: its linked code, its catalog and its lot.
pub struct Lot {
    program: Program,
    catalog: Catalog,
    world: World,
    tuning: Option<Tuning>,
    player: Answers,
    frames: Frames,
    /// Ticks run, and the lot's tick count when the first one started.
    ticks: u32,
    start: u32,
    load: LoadReport,
}

/// Why a step could not run, or stopped partway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepError {
    /// The load did not complete, so no tick can run.
    Incomplete,
    /// The lot update of tick `tick` failed; the lot is as the failure
    /// left it.
    Tick {
        tick: u32,
        start: u32,
        error: String,
    },
    /// The frame after tick `tick` (0: before the first) failed. After a
    /// tick, that tick's update ran first, and is here.
    Frame {
        tick: u32,
        error: String,
        update: Option<Box<Update>>,
    },
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Incomplete => write!(f, "ticks were asked for, but the load did not complete"),
            Self::Tick { tick, start, error } => write!(
                f,
                "tick {tick}, the lot update that starts at tick count {start}, failed: {error}"
            ),
            Self::Frame { tick: 0, error, .. } => {
                write!(f, "the frame before tick 1 failed: {error}")
            }
            Self::Frame { tick, error, .. } => {
                write!(f, "the frame after tick {tick} failed: {error}")
            }
        }
    }
}

impl std::error::Error for StepError {}

impl Lot {
    /// Loads the house file `house` (inside a neighborhood's `Houses`
    /// directory) from the install at `root`. `answers` answers, in order,
    /// the dialogs that offer a choice; a dialog with one button takes
    /// none. The frame before the first tick is not drawn yet; the first
    /// [`Lot::step`] draws it when `frames` asks for one.
    pub fn load(
        root: &Path,
        house: &Path,
        answers: &[u8],
        frames: Frames,
    ) -> Result<Self, BootError> {
        let install = Install::detect(root)?;
        let directory = house
            .parent()
            .and_then(Path::parent)
            .ok_or("the house file must be inside a neighborhood's Houses directory")?;
        let neighborhood = Neighborhood::open(directory)?;
        let mut index = Index::build(&install)?;
        if let Some(characters) = neighborhood.characters()? {
            index.add_characters(&characters)?;
        }
        let catalog = Catalog::build(&index)?;
        let file = House::load(house, &catalog)?;
        let context = Context::COMPLETE_COLLECTION;
        // The original has every definition registered, and the resize
        // step walks them all, so the whole catalog is linked.
        let mut program = Program::link(&index, &catalog, catalog.objects.iter().map(|o| o.guid))?;
        program.zoning = neighborhood.zoning(context.expansions)?;
        program.animations = Animations::load(&install, &index)?;
        let person_types = || {
            catalog
                .objects
                .iter()
                .filter(|o| o.kind == 2)
                .map(|o| o.guid)
        };
        // Every person type's skeleton and appearance, for the people a
        // script makes. A type whose skeleton cannot be chosen is left
        // out; making a person of it fails.
        program.people = People::load_each(&install, &index, &catalog, person_types())?.0;
        let mut world = World::empty();
        let mut load = LoadReport::default();
        {
            let interpreter = Interpreter::new(&program, &catalog, Stream::Global);
            // The lot the game holds before the load: empty, with the
            // neighborhood's people, the type attributes the neighborhood
            // saved and the two indicator objects.
            world.read_neighbors(&neighborhood, person_types())?;
            world.type_attributes = program.type_attributes(&catalog)?;
            if let Some(saved) = &neighborhood.type_attributes {
                world.read_type_attributes(saved);
            }
            interpreter.create_indicators(&mut world)?;
            // The load steps, in the original's order.
            world.read_simulator(&file.simulator);
            // With the simulator state, the house file's model gives the
            // house's usage counts (0x416568).
            world.read_house_model(
                file.model
                    .as_ref()
                    .ok_or("the house file has no HOUS 1 chunk, which holds its model")?,
            );
            world.apply_context(context)?;
            load.created = interpreter.resize(&mut world)?.created.len();
            world.clear_family()?;
            load.queued = world.removals.len();
            load.removed = interpreter.clear(&mut world)?.len();
            // 6a and 6b: the lot arrays, then the saved floor and wall IDs.
            let version = file
                .version()
                .ok_or("the house file has no HOUS 1 chunk, which holds its version")?;
            world.read_arrays(&file.arrays, version)?;
            load.renumbered =
                world.renumber_surfaces(file.floor_map.as_deref(), file.wall_map.as_deref())?;
            // 6c and 7: the type table, which marks the types whose scripts
            // changed.
            if file.guid_table {
                return Err(
                    "the house has a GUID translation table (Gtab), which is not ported".into(),
                );
            }
            let types = &file.types.entries;
            for entry in types {
                let object = catalog
                    .object(entry.guid)
                    .ok_or_else(|| format!("type {} has no definition", entry.id))?;
                if interpreter.type_changed(entry) == Some(true) {
                    let current = [Callback::Init, Callback::Main]
                        .map(|c| program.signature(object.resource, program.callback(object, c)));
                    load.mismatches.push(format!(
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
            load.types = types.len();
            load.changed = interpreter.read_types(&mut world, &file.types)?.len();
            if load.changed != load.mismatches.len() {
                return Err("the type table read and its listing disagree".into());
            }
            // 6d: the second clear, then 6e: the saved objects.
            load.cleared_again = interpreter.clear(&mut world)?.len();
            let people = People::load(
                &install,
                &index,
                &catalog,
                file.objects.instances.values().map(|o| o.guid),
            )?;
            let read = interpreter.read_objects(&mut world, &file.objects, &people)?;
            load.read = read.objects.len();
            load.draws = read.draws;
            // 8: the lot-wide controllers, now among the saved objects. 9:
            // the ground level's object grid.
            let reconciled = interpreter.reconcile_controllers(&mut world)?;
            load.reconciled_created = reconciled.created.len();
            load.reconciled_removed = reconciled.removed.len();
            load.repaired = world.repair_grid()?.len();
            world.finish_object_read();
            load.queued_again = world.removals.len();
            // The original now works out the lot's rooms from the walls it
            // has read. That makes it give every object its room and ask
            // each object's function 7 (step 10), and then count the
            // rooms, which sets the lot's indoor area (step 10a). The count
            // changes every room's light, so the rooms are marked again.
            // The end of the house read marks them all once more (signal
            // 0xdd) and counts whatever waits.
            world.rebuild_rooms()?;
            let roomed = interpreter.assign_rooms(&mut world)?;
            load.moved = roomed.moved.len();
            load.asked = roomed.asked.len();
            interpreter.update_rooms(&mut world, 0)?;
            world.mark_all_rooms();
            interpreter.update_pending_rooms(&mut world)?;
            // 11: the removal queue is emptied. 12: the household that
            // lives in the house is selected.
            load.dropped = world.discard_removals().len();
            let number = neighborhood
                .houses()?
                .into_iter()
                .find(|(_, path)| path.file_name() == house.file_name())
                .map(|(number, _)| number)
                .ok_or("the house file's name does not give a house number")?;
            let (family, record) = neighborhood.family_in(number).ok_or(
                "no family lives in the house; selecting none is not checked against the original",
            )?;
            load.family = family;
            load.house = number;
            world.select_family(
                i16::try_from(family)?,
                i16::try_from(number)?,
                record.budget,
                record.magic_history()?,
            )?;
            // 13: every object is visited in registration order. The step
            // stops at the first visit that is not ported, with the lot as
            // the visit before it left it.
            load.to_visit = world.order.len();
            let visited = interpreter.visit_objects(&mut world, &people);
            load.visited = visited.objects.len();
            load.reset = visited.reset.len();
            load.stopped = visited.stopped.map(|fault| fault.to_string());
            // 14: the indicators are created if no visit made them. 15: the
            // end of the household selection, which has nothing to do on a
            // lot whose people are all members of the household. 16: the
            // loaded house is announced, which asks for the indicators once
            // more and clears the selected Sim. 17 and 18: the funds are set
            // again and the family's first member is selected. 19: the game
            // goes to live mode. 20 changes nothing here. 21 and 22: the
            // waiting rooms are counted and the interface's wait check
            // clears global 12.
            if load.stopped.is_none() {
                let made = interpreter.ensure_indicators(&mut world)?.is_some();
                world.finish_selection(record.flags)?;
                let made_again = interpreter.announce_house(&mut world)?.is_some();
                let sim =
                    interpreter.select_resident(&mut world, record.budget, &record.members)?;
                interpreter.go_live(&mut world)?;
                let counted = interpreter.before_frame(&mut world)?.len();
                load.finished = Some(Finished {
                    indicators_made: made || made_again,
                    sim,
                    counted,
                });
            }
        }
        let saved = &file.objects.instances;
        load.saved_objects = saved.len();
        load.saved_people = saved.values().filter(|o| o.person.is_some()).count();
        load.saved_ticks = file.simulator.ticks;
        load.libraries = program.libraries.len();
        let tuning = match load.finished {
            Some(_) => Some(program.motive_tuning()?),
            None => None,
        };
        let start = world.ticks;
        Ok(Self {
            program,
            catalog,
            world,
            tuning,
            player: Answers {
                queue: answers.iter().copied().collect(),
            },
            frames,
            ticks: 0,
            start,
            load,
        })
    }

    /// Runs the next tick: the lot update, then the house view's frame if
    /// one is drawn after it. The first step first draws the frame before
    /// the first tick, if one is drawn there.
    pub fn step(&mut self) -> Result<Update, StepError> {
        let tuning = self.tuning.as_ref().ok_or(StepError::Incomplete)?;
        let interpreter = Interpreter::new(&self.program, &self.catalog, Stream::Global);
        if self.ticks == 0 && self.frames.after(0) {
            interpreter
                .draw_frame(&mut self.world, VIEW_LEVEL)
                .map_err(|error| StepError::Frame {
                    tick: 0,
                    error: error.to_string(),
                    update: None,
                })?;
        }
        let tick = self.ticks + 1;
        let update = interpreter
            .lot_update(&mut self.world, tuning, &mut self.player)
            .map_err(|error| StepError::Tick {
                tick,
                start: self.start.wrapping_add(self.ticks),
                error: error.to_string(),
            })?;
        self.ticks = tick;
        if self.frames.after(tick)
            && let Err(error) = interpreter.draw_frame(&mut self.world, VIEW_LEVEL)
        {
            return Err(StepError::Frame {
                tick,
                error: error.to_string(),
                update: Some(Box::new(update)),
            });
        }
        Ok(update)
    }

    /// The lot's state.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// The linked code and the catalog, for reading what an object is.
    pub fn program(&self) -> &Program {
        &self.program
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// What the load did.
    pub fn load_report(&self) -> &LoadReport {
        &self.load
    }

    /// Ticks run so far.
    pub fn ticks(&self) -> u32 {
        self.ticks
    }
}

#[cfg(test)]
mod tests {
    use super::Frames;

    #[test]
    fn a_frame_list_reads_ticks_and_repeats() {
        let frames = Frames::parse("0,2,10+5").unwrap();
        let drawn: Vec<u32> = (0..=21).filter(|&t| frames.after(t)).collect();
        assert_eq!(drawn, [0, 2, 10, 15, 20]);
        let every = Frames::parse("3+").unwrap();
        let drawn: Vec<u32> = (0..=6).filter(|&t| every.after(t)).collect();
        assert_eq!(drawn, [3, 4, 5, 6]);
        assert_eq!(Frames::parse("none").unwrap().from, None);
        assert_eq!(Frames::parse("4+0"), None);
        assert_eq!(Frames::parse("x"), None);
        let default = Frames::default();
        let drawn: Vec<u32> = (0..=100).filter(|&t| default.after(t)).collect();
        assert_eq!(drawn, [0, 1, 2, 3, 33, 63, 93]);
        assert_eq!(Frames::parse("0,1,2,3,33+30"), Some(default));
    }
}
