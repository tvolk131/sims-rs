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

use crate::play::Play;
use crate::{Answers, Callback, Interpreter, Program, Stream, TickError, Update};
use sims_content::{Animations, Catalog, House, Index, Install, Neighborhood, People, Surfaces};
use sims_sim::World;
use sims_sim::load::{Context, Household};
use sims_sim::motives::Tuning;
use std::cell::RefCell;
use std::collections::BTreeSet;
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
    /// Repeating frames, as `(from, step)`: a frame after tick `from` and
    /// after every `step`th tick from it. `step` is at least 1.
    pub every: Vec<(u32, u32)>,
}

impl Default for Frames {
    fn default() -> Self {
        Self {
            after: vec![0, 1, 2, 3],
            every: vec![(33, 30)],
        }
    }
}

impl Frames {
    /// Reads a list such as `0,33+30`: a tick number draws a frame after
    /// that tick (0: before the first), `N+` after every tick from N on,
    /// and `N+S` after N and every S-th tick from there. A list may hold
    /// several of each. `none` draws no frame.
    pub fn parse(text: &str) -> Option<Self> {
        let mut frames = Self {
            after: Vec::new(),
            every: Vec::new(),
        };
        if text == "none" {
            return Some(frames);
        }
        for part in text.split(',') {
            match part.split_once('+') {
                Some((from, step)) => {
                    let step = if step.is_empty() {
                        1
                    } else {
                        step.parse().ok().filter(|&s| s > 0)?
                    };
                    frames.every.push((from.parse().ok()?, step));
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
                .every
                .iter()
                .any(|&(from, step)| tick >= from && (tick - from).is_multiple_of(step))
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
    /// The motive constants, worked out at the first step: a load that
    /// runs no tick does not need them.
    tuning: Option<Tuning>,
    player: Answers,
    frames: Frames,
    /// Ticks run, and the lot's tick count when the first one started.
    ticks: u32,
    start: u32,
    load: LoadReport,
    /// Set when a tick or a frame failed. The lot is then as the failure
    /// left it, partway through a tick or a frame, so no later step runs.
    halted: bool,
    /// Play mode's own state ([`crate::play`]), used by
    /// [`Lot::step_play`] only.
    play: Play,
    /// Whether a dialog that offers a choice, with no answer left in the
    /// list given at load, waits for the player ([`Lot::wait_for_answers`])
    /// rather than halting the lot.
    asks: bool,
    /// The dialog the lot waits on, while it waits.
    asking: Option<Box<Asking>>,
    /// The dialog is answered and the tick that showed it has not run
    /// again yet: the lot is as it was before that tick, so it takes no
    /// input until the next step.
    answered: bool,

    /// The house view's rotation, 0 to 3 ([`Lot::set_view_rotation`]).
    view_rotation: u8,
}

/// A dialog the lot waits on ([`Lot::asking`]). The tick that showed it
/// has not happened: the lot is as it was before that tick, and
/// [`Lot::answer`] then a step runs the tick again, which gives the
/// answer where the original waits for it (0x436da0).
#[derive(Debug, Clone)]
pub struct Asking {
    pub shown: crate::Shown,
    /// The lot partway through the tick, as it was when the dialog was
    /// shown: its record ([`sims_sim::World::dialogs`]) and the object
    /// that asked, for drawing the dialog. Not the lot's state.
    pub world: World,
}

/// How [`Lot::advance`] stopped: a failure, or a dialog that waits for
/// the player.
enum Stop {
    Step(StepError),
    Asks { tick: u32, shown: crate::Shown },
}

impl From<StepError> for Stop {
    fn from(error: StepError) -> Self {
        Self::Step(error)
    }
}

impl Stop {
    /// As a step's error, for a lot that does not wait for answers.
    fn into_step(self) -> StepError {
        match self {
            Self::Step(error) => error,
            Self::Asks { tick, .. } => StepError::Asking { tick },
        }
    }
}

/// Why an answer was not taken ([`Lot::answer`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerError {
    /// No dialog waits for an answer.
    NotAsked,
    /// The dialog does not offer that answer.
    NotOffered { answer: u8, offered: &'static [u8] },
}

impl std::fmt::Display for AnswerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAsked => write!(f, "no dialog waits for an answer"),
            Self::NotOffered { answer, offered } => {
                write!(f, "the dialog offers {offered:?}, not {answer}")
            }
        }
    }
}

impl std::error::Error for AnswerError {}

/// Why a step could not run, or stopped partway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepError {
    /// The load did not complete, so no tick can run.
    Incomplete,
    /// The install lacks the motive constants a tick needs.
    Tuning(String),
    /// An earlier step failed, leaving the lot partway through a tick or a
    /// frame; the lot is unchanged since.
    Halted,
    /// Tick `tick` shows a dialog that waits for the player
    /// ([`Lot::wait_for_answers`], [`Lot::asking`]). The lot is as it was
    /// before the tick, and is not halted: answer ([`Lot::answer`]) and
    /// step again.
    Asking { tick: u32 },
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
            Self::Tuning(error) => write!(f, "{error}"),
            Self::Halted => write!(f, "an earlier step failed, so no more ticks run"),
            Self::Asking { tick } => {
                write!(
                    f,
                    "tick {tick} shows a dialog that waits for the player's answer"
                )
            }
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

/// What [`Lot::pie_menu`] lists: the entries offered, in the menu's
/// order, and the requests to the sound system the check trees made.
/// Picking an entry queues its action ([`Lot::pick`]).
#[derive(Debug, Clone)]
pub struct Menu {
    pub entries: Vec<crate::orders::MenuEntry>,
    pub sounds: Vec<crate::SoundEvent>,
}

/// What [`Lot::highlight`] did: the entry's action as the check left it,
/// and the check tree's requests to the sound system.
#[derive(Debug, Clone)]
pub struct Highlight {
    pub action: sims_formats::saved_objects::Interaction,
    pub sounds: Vec<crate::SoundEvent>,
}

/// What [`Lot::cancel`] did: whether the press counts, and the trees'
/// requests to the sound system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cancelled {
    pub counts: bool,
    pub sounds: Vec<crate::SoundEvent>,
}

/// Why a player's order was not queued or the pie menu not listed
/// ([`Lot::queue_interaction`], [`Lot::pie_menu`], [`Lot::pick`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderError {
    /// The load did not complete.
    Incomplete,
    /// An earlier step failed and halted the lot.
    Halted,
    /// The lot waits for the player to answer a dialog ([`Lot::asking`]);
    /// as in the original, nothing else is taken until it is answered.
    Asking,
    /// The order names no such person or interaction, or the queue fails
    /// on something it does not port.
    Refused(crate::Fault),
}

impl std::fmt::Display for OrderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Incomplete => write!(f, "the load did not complete"),
            Self::Halted => write!(f, "an earlier step failed, so the lot takes no orders"),
            Self::Asking => write!(f, "the lot waits for the player to answer a dialog"),
            Self::Refused(fault) => write!(f, "the order was refused: {fault}"),
        }
    }
}

impl std::error::Error for OrderError {}

/// Whether the lot is residential, as the idle loop asks (`0x40745f`):
/// its zoning (`0x42b9e3`, by the lot number in global 10; lot 0 is 1)
/// is 1.
fn residential(world: &World, program: &Program) -> bool {
    let lot = world.globals.get(10).copied().unwrap_or(0);
    let zoning = if lot == 0 {
        1
    } else {
        program.zoning.get(&lot).copied().unwrap_or(0)
    };
    zoning == 1
}

/// The rotation a house opens at: the camera direction its file saved
/// (the HOUS chunk), as the render track opens the view
/// (`sims_render::open_house`). That the original's load turns the scene
/// (0x6e5528, +0x84) to it is not traced; the comparison harness saw the
/// Goth house (`House05.iff`, which saved 0) open at rotation 0.
pub fn opening_rotation(file: &House) -> u8 {
    file.view
        .as_ref()
        .map_or(0, |v| (v.camera_direction & 3) as u8)
}

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
        let view_rotation = opening_rotation(&file);
        let context = Context::COMPLETE_COLLECTION;
        // The original has every definition registered, and the resize
        // step walks them all, so the whole catalog is linked.
        let mut program = Program::link(&index, &catalog, catalog.objects.iter().map(|o| o.guid))?;
        program.zoning = neighborhood.zoning(context.expansions)?;
        program.expansions = context.expansions;
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
        program.icon_frames = crate::program::icon_frames(&install)?;
        let mut world = World::empty();
        let mut load = LoadReport::default();
        {
            let interpreter = Interpreter {
                view_rotation,
                ..Interpreter::new(&program, &catalog, Stream::Global)
            };
            // The lot the game holds before the load: empty, with the
            // neighborhood's people, the type attributes the neighborhood
            // saved and the two indicator objects.
            world.read_neighbors(
                &neighborhood,
                &sims_sim::neighbors::registry(&index, &catalog),
            )?;
            world.drop_empty_families()?;
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
            // The floor and wall catalogs, which the original builds at
            // start-up.
            let surfaces = Surfaces::build(&install)?;
            load.renumbered = world.renumber_surfaces(
                file.floor_map.as_deref(),
                file.wall_map.as_deref(),
                &surfaces,
            )?;
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
            // 0x42821a: the people who are not visitors, noted before the
            // family is selected.
            let residents = world.residents();
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
            // end of the household selection, which works out again who is
            // a visitor (`World::finish_selection`). 16: the
            // loaded house is announced, which asks for the indicators once
            // more and clears the selected Sim. 17 and 18: the funds are set
            // again and the family's first member is selected. 19: the game
            // goes to live mode. 20 changes nothing here. 21 and 22: the
            // waiting rooms are counted and the interface's wait check
            // clears global 12.
            if load.stopped.is_none() {
                let made = interpreter.ensure_indicators(&mut world)?.is_some();
                // The types registered from the neighborhood's
                // Characters folder, which the original registers with
                // kind 1 (0x424a8b, 0x43c228).
                let characters: BTreeSet<u32> = catalog
                    .objects
                    .iter()
                    .map(|o| o.guid)
                    .filter(|&guid| {
                        catalog.object(guid).is_some_and(|o| {
                            index.mounts[index.resources[o.resource].mount].path
                                == Index::CHARACTERS
                        })
                    })
                    .collect();
                world.finish_selection(&Household {
                    house: i32::try_from(number)?,
                    flags: record.flags,
                    members: &record.members,
                    characters: &characters,
                    residents: &residents,
                    zoning: &program.zoning,
                })?;
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
        let start = world.ticks;
        Ok(Self {
            program,
            catalog,
            world,
            tuning: None,
            player: Answers {
                queue: answers.iter().copied().collect(),
            },
            frames,
            ticks: 0,
            start,
            load,
            halted: false,
            play: Play::default(),
            asks: false,
            answered: false,
            asking: None,

            view_rotation,
        })
    }

    /// Runs the next tick: the lot update, then the house view's frame if
    /// one is drawn after it. The first step first draws the frame before
    /// the first tick, if one is drawn there.
    ///
    /// After a step fails in its tick or a frame, every later step fails
    /// with [`StepError::Halted`] and changes nothing. A step that fails
    /// before it starts ([`StepError::Incomplete`], [`StepError::Tuning`])
    /// changes nothing either.
    ///
    /// The update's [`Update::sounds`] holds the requests to the sound
    /// system the step made, the frames' included ([`crate::sounds`]).
    pub fn step(&mut self) -> Result<Update, StepError> {
        self.step_in(false)
    }

    /// Runs the next tick in play mode ([`crate::play`]): an error in an
    /// object's tick, such as an unported primitive, resets the object as
    /// the original does on a SimAntics error, and the lot goes on. The
    /// errors caught are in [`Update::errors`]. Any other failure halts
    /// the lot as [`Lot::step`] does.
    ///
    /// Play mode is not the original's behaviour wherever a caught error
    /// comes from something not ported, so a lot stepped this way is not
    /// one to compare with the original. [`Lot::step`] is unchanged by it.
    pub fn step_play(&mut self) -> Result<Update, StepError> {
        self.step_in(true)
    }

    fn step_in(&mut self, play: bool) -> Result<Update, StepError> {
        if self.halted {
            return Err(StepError::Halted);
        }
        if self.load.finished.is_none() {
            return Err(StepError::Incomplete);
        }
        if self.tuning.is_none() {
            let tuning = self
                .program
                .motive_tuning()
                .map_err(|error| StepError::Tuning(error.to_string()))?;
            self.tuning = Some(tuning);
        }
        if !self.asks {
            let result = self.advance(play).map_err(Stop::into_step);
            if result.is_err() {
                self.halted = true;
            }
            return result;
        }
        if self.asking.is_some() {
            return Err(StepError::Asking {
                tick: self.ticks + 1,
            });
        }
        // The tick may stop at a dialog the player has not answered; it
        // then runs again from here once they have.
        let before = (
            self.world.clone(),
            self.play.clone(),
            self.ticks,
            self.player.clone(),
        );
        self.answered = false;
        match self.advance(play) {
            Err(Stop::Asks { tick, shown }) => {
                let (world, play, ticks, player) = before;
                let partway = std::mem::replace(&mut self.world, world);
                self.play = play;
                self.ticks = ticks;
                self.player = player;
                self.asking = Some(Box::new(Asking {
                    shown,
                    world: partway,
                }));
                Err(StepError::Asking { tick })
            }
            result => {
                let result = result.map_err(Stop::into_step);
                if result.is_err() {
                    self.halted = true;
                }
                result
            }
        }
    }

    /// The tick and its frames, for a lot that can step.
    fn advance(&mut self, play: bool) -> Result<Update, Stop> {
        let tuning = self.tuning.as_ref().ok_or(StepError::Incomplete)?;
        let asks = self.asks;
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut first_frame = None;
        if self.ticks == 0
            && self.frames.after(0)
            && let Err(error) = interpreter.draw_frame(&mut self.world, VIEW_LEVEL)
        {
            if !play {
                return Err(StepError::Frame {
                    tick: 0,
                    error: error.to_string(),
                    update: None,
                }
                .into());
            }
            first_frame = Some(error.to_string());
        }
        let tick = self.ticks + 1;
        // The idle loop sets a speed of its own before it runs the update
        // (0x46d9d6, `World::idle_speed`).
        let residential = residential(&self.world, &self.program);
        self.world.idle_speed(residential);
        let update = interpreter
            .lot_update_with(
                &mut self.world,
                tuning,
                &mut self.player,
                play.then_some(&mut self.play),
            )
            .map_err(|error| match error {
                // Nobody has answered: the player is asked.
                TickError::Unanswered { shown, given: None } if asks => Stop::Asks { tick, shown },
                error => StepError::Tick {
                    tick,
                    start: self.start.wrapping_add(self.ticks),
                    error: error.to_string(),
                }
                .into(),
            })?;
        let mut update = Update {
            frame_errors: first_frame.into_iter().collect(),
            ..update
        };
        self.ticks = tick;
        // Between lot updates the game handles its messages, which
        // redraws the family panel.
        interpreter.refresh_family_panel(&mut self.world);
        let frame = if self.frames.after(tick) {
            interpreter.draw_frame(&mut self.world, VIEW_LEVEL).err()
        } else {
            None
        };
        // The step's sounds go with its update whether or not the frame
        // failed.
        update.sounds = sounds.into_inner();
        if let Some(error) = frame {
            if play {
                update.frame_errors.push(error.to_string());
                return Ok(update);
            }
            return Err(StepError::Frame {
                tick,
                error: error.to_string(),
                update: Some(Box::new(update)),
            }
            .into());
        }
        Ok(update)
    }

    /// Queues on `person` the interaction of `object` with menu number
    /// `menu`, as picking it from the pie menu does ([`crate::orders`]).
    /// Returns the action's number, or None when the person's queue took
    /// nothing. Call it between steps, as the original takes the player's
    /// input between lot updates.
    ///
    /// An order that fails changes nothing, and does not halt the lot. One
    /// the full queue does not take still changes it: the action took its
    /// number before the queue was checked (0x45d30d, before 0x45d4c8's
    /// count), as in the original.
    pub fn queue_interaction(
        &mut self,
        person: u16,
        object: u16,
        menu: u32,
    ) -> Result<Option<i32>, OrderError> {
        self.ready()?;
        let interpreter = Interpreter {
            view_rotation: self.view_rotation,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let queued = interpreter
            .order(&mut world, person, object, menu)
            .map_err(OrderError::Refused)?;
        self.world = world;
        Ok(queued)
    }

    /// Lists the pie menu `person` is offered on `object`
    /// ([`Interpreter::pie_menu`]). Call it between steps, at the moments
    /// the original lists it: when the cursor first reaches an object
    /// (0x48c834), not on the click. Listing runs the entries' check trees,
    /// which change the lot as the original's do, so a recording of play
    /// must note each call.
    ///
    /// A listing that fails on something not ported changes nothing (the
    /// original has no such failure), and does not halt the lot.
    pub fn pie_menu(&mut self, person: u16, object: u16) -> Result<Menu, OrderError> {
        self.ready()?;
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let entries = interpreter
            .pie_menu(&mut world, person, object)
            .map_err(OrderError::Refused)?;
        self.world = world;
        Ok(Menu {
            entries,
            sounds: sounds.into_inner(),
        })
    }

    /// Checks again the action of the pie menu's entry the pointer has
    /// reached ([`Interpreter::highlight`], 0x48df28). Call it between
    /// steps when the pointer reaches an entry of an open menu: the check
    /// tree changes the lot, so a recording of play must note each call.
    /// Returns the action as the check left it, and the requests to the
    /// sound system its tree made. A check that fails on something not
    /// ported changes nothing, as for [`Lot::pie_menu`].
    pub fn highlight(
        &mut self,
        person: u16,
        action: sims_formats::saved_objects::Interaction,
    ) -> Result<Highlight, OrderError> {
        self.ready()?;
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let action = interpreter
            .highlight(&mut world, person, action)
            .map_err(OrderError::Refused)?;
        self.world = world;
        Ok(Highlight {
            action,
            sounds: sounds.into_inner(),
        })
    }

    /// Queues on `person` the action of an entry [`Lot::pie_menu`] listed,
    /// as picking it does ([`Interpreter::pick`]). Returns the action's number, or None
    /// when the queue took nothing (it is full, or the action's entry is
    /// gone). An order that fails changes nothing, as for
    /// [`Lot::queue_interaction`].
    pub fn pick(
        &mut self,
        person: u16,
        action: sims_formats::saved_objects::Interaction,
    ) -> Result<Option<i32>, OrderError> {
        self.ready()?;
        let interpreter = Interpreter {
            view_rotation: self.view_rotation,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let queued = interpreter
            .pick(&mut world, person, action)
            .map_err(OrderError::Refused)?;
        self.world = world;
        Ok(queued)
    }

    /// Cancels `person`'s action numbered `uid`, as a press on its icon
    /// in the queue does ([`Interpreter::cancel`], 0x45d5f8), between
    /// steps. Returns whether the press counts (the original then plays
    /// `UI_queue_delete`, else `UI_error`), and the requests to the sound
    /// system the trees it ran made. A cancel that fails changes nothing,
    /// as for [`Lot::queue_interaction`].
    pub fn cancel(&mut self, person: u16, uid: i32) -> Result<Cancelled, OrderError> {
        self.ready()?;
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let counts = interpreter
            .cancel(&mut world, person, uid)
            .map_err(OrderError::Refused)?;
        self.world = world;
        Ok(Cancelled {
            counts,
            sounds: sounds.into_inner(),
        })
    }

    /// Selects the Sim `person` as the player does from a portrait,
    /// between steps ([`Interpreter::select`], 0x445449): the Sim
    /// selected before is deselected, and the indicators move. Returns
    /// the requests to the sound system the indicators' trees made.
    ///
    /// Deselecting refreshes the deselected Sim's animator at the lot's
    /// tick count (0x58da12 with 0), which can move its head and person
    /// words 42 and 45.
    ///
    /// A selection that fails changes nothing, as for
    /// [`Lot::queue_interaction`]. Only a person can be selected; which
    /// people the interface lets the player select (the portraits of the
    /// household) is the caller's to decide.
    pub fn select(&mut self, person: u16) -> Result<Vec<crate::SoundEvent>, OrderError> {
        self.ready()?;
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        interpreter
            .select(&mut world, Some(person))
            .map_err(OrderError::Refused)?;
        self.world = world;
        Ok(sounds.into_inner())
    }

    /// Sets the idle loop's own speed between ticks (`0x46d9d6`,
    /// [`World::idle_speed`]), as the original's idle loop does on each
    /// pass, whether an update is due or not and paused or not. Each step
    /// does it too, before its update, so a host that only steps gets the
    /// same ticks; a host with a clock calls this on each pass of its own,
    /// so that a speed set between ticks meets the idle loop's as the
    /// original's does. Returns whether it called the setter, which
    /// restarts the original's clocks, even when the speed stays the same.
    pub fn idle_speed(&mut self) -> Result<bool, OrderError> {
        self.ready()?;
        let residential = residential(&self.world, &self.program);
        Ok(self.world.idle_speed(residential))
    }

    /// Sets the game's speed between ticks, as the panel's speed buttons
    /// do ([`sims_sim::speed`], `0x46d697`): globals 16 and 18. Returns
    /// the milliseconds the original waits between two lot updates at
    /// that speed; when to step is the caller's.
    pub fn set_speed(&mut self, speed: i32) -> Result<u32, OrderError> {
        self.ready()?;
        Ok(self.world.set_speed(speed))
    }

    /// Pauses the game between ticks (`0x46d813`): global 17 is set, and
    /// each step from now on is a partial pass in which the clock stands.
    /// The original goes on updating the lot while paused, so the caller
    /// should go on stepping it. Returns whether the game was running.
    pub fn pause(&mut self) -> Result<bool, OrderError> {
        self.ready()?;
        Ok(self.world.pause())
    }

    /// Runs the game again between ticks (`0x46d83a`): global 17 is
    /// cleared. Returns whether the game was paused.
    pub fn unpause(&mut self) -> Result<bool, OrderError> {
        self.ready()?;
        Ok(self.world.unpause())
    }

    /// From now on, a dialog that offers a choice when no answer is left
    /// in the list given at load waits for the player, as the original
    /// waits (0x436da0): the step returns [`StepError::Asking`] and the
    /// lot stays as it was before that tick, not halted. The window turns
    /// this on; `sims boot` and the comparisons do not, and halt there.
    ///
    /// The tick runs again once [`Lot::answer`] has the answer. A tick
    /// runs the same from the same lot, so it reaches the same dialog,
    /// and the lot then goes on exactly as if the answer had been in the
    /// list at load.
    pub fn wait_for_answers(&mut self) {
        self.asks = true;
    }

    /// The dialog the lot waits on, if it waits.
    pub fn asking(&self) -> Option<&Asking> {
        self.asking.as_deref()
    }

    /// Answers the dialog the lot waits on with `answer` (the status its
    /// button gives: 2 the first, 3 the second, 4 the third). The next
    /// step runs the tick again and gives it this answer; until then the
    /// lot is as it was before that tick and takes no orders.
    pub fn answer(&mut self, answer: u8) -> Result<(), AnswerError> {
        let asking = self.asking.as_ref().ok_or(AnswerError::NotAsked)?;
        let offered = asking.shown.answers;
        if !offered.contains(&answer) {
            return Err(AnswerError::NotOffered { answer, offered });
        }
        self.player.queue.push_back(answer);
        self.asking = None;
        self.answered = true;
        Ok(())
    }

    /// Whether the lot takes a player's input.
    fn ready(&self) -> Result<(), OrderError> {
        if self.halted {
            return Err(OrderError::Halted);
        }
        if self.asking.is_some() || self.answered {
            return Err(OrderError::Asking);
        }
        if self.load.finished.is_none() {
            return Err(OrderError::Incomplete);
        }
        Ok(())
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

    /// The house view's rotation, 0 to 3 (the scene at 0x6e5528, +0x84),
    /// which Generic Sims Call routine 12 reads (0x44fdb8, 0x434f58).
    pub fn view_rotation(&self) -> u8 {
        self.view_rotation
    }

    /// Turns the house view to `rotation` (taken modulo 4). It is the
    /// player's choice, not the lot's, but the simulation reads it, so it
    /// is an input like an order: the game window sets it whenever its
    /// camera turns, between steps, and a recording of play must note
    /// each change. The load starts it at the house's saved camera
    /// direction ([`opening_rotation`]).
    pub fn set_view_rotation(&mut self, rotation: u8) {
        self.view_rotation = rotation & 3;
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
    use super::{Answers, Finished, Frames, LoadReport, Lot, OrderError, StepError};
    use crate::Program;
    use crate::play::Play;
    use sims_content::Catalog;
    use sims_sim::World;
    use sims_sim::motives::{Curve, Rates, Tuning};
    use std::collections::BTreeMap;

    /// A lot with no code, whose load finished (or not), drawing frames
    /// as `frames` says.
    fn lot(world: World, finished: bool, frames: Frames) -> Lot {
        let load = LoadReport {
            finished: finished.then_some(Finished {
                indicators_made: false,
                sim: None,
                counted: 0,
            }),
            ..LoadReport::default()
        };
        let start = world.ticks;
        Lot {
            program: Program {
                global: 0,
                libraries: BTreeMap::new(),
                semi_globals: BTreeMap::new(),
                careers: None,
                zoning: BTreeMap::new(),
                animations: Default::default(),
                people: Default::default(),
                icon_frames: None,
                expansions: 0,
            },
            catalog: Catalog {
                global: 0,
                objects: Vec::new(),
                by_guid: BTreeMap::new(),
                semi_globals: BTreeMap::new(),
                diagnostics: Vec::new(),
            },
            world,
            tuning: None,
            player: Answers::default(),
            frames,
            ticks: 0,
            start,
            load,
            halted: false,
            play: Play::default(),
            asks: false,
            answered: false,
            asking: None,

            view_rotation: 0,
        }
    }

    fn tuning() -> Tuning {
        let rates = || Rates::from_lookup(0, |_| None);
        let curve = |motive| Curve::parse(motive, "(-100;1)(100;1)", 8);
        Tuning {
            rates: [rates(), rates(), rates()],
            adult_mood: vec![curve(7)],
            child_mood: vec![curve(7)],
        }
    }

    #[test]
    fn the_idle_loop_runs_a_residential_lot_nobody_lives_on_at_1000() {
        let mut world = World::empty();
        world.globals[16] = 760;
        world.globals[18] = -1;
        let mut lot = lot(world, true, Frames::default());
        lot.tuning = Some(tuning());
        assert!(super::residential(&lot.world, &lot.program));
        // The update itself fails on an empty lot, after the idle loop
        // set its speed.
        assert!(matches!(lot.step(), Err(StepError::Tick { tick: 1, .. })));
        assert_eq!(lot.world.speed(), 1000);
        assert_eq!(lot.world.globals[18], 760);
        // A lot with no zoning is not residential: the idle loop leaves
        // its speed alone.
        lot.world.globals[10] = 7;
        assert!(!super::residential(&lot.world, &lot.program));
    }

    #[test]
    fn the_idle_loops_speed_is_set_between_ticks_and_a_speed_set_after_stands() {
        let mut world = World::empty();
        world.globals[16] = 760;
        world.globals[18] = -1;
        let mut lot = lot(world, true, Frames::default());
        // Between ticks, before any is due: 1000, once.
        assert_eq!(lot.idle_speed(), Ok(true));
        assert_eq!((lot.world.speed(), lot.world.globals[18]), (1000, 760));
        assert_eq!(lot.idle_speed(), Ok(false));
        // The player's speed set after it stands.
        assert_eq!(lot.set_speed(924), Ok(14));
        assert_eq!(lot.idle_speed(), Ok(false));
        assert_eq!((lot.world.speed(), lot.world.globals[18]), (924, 924));
        // A halted lot takes none.
        lot.halted = true;
        assert_eq!(lot.idle_speed(), Err(OrderError::Halted));
    }

    #[test]
    fn a_failed_tick_halts_the_lot() {
        // An empty lot has no selected Sim at the end of its update, which
        // stops the tick partway.
        let mut lot = lot(World::empty(), true, Frames::default());
        lot.tuning = Some(tuning());
        let Err(StepError::Tick { tick: 1, .. }) = lot.step() else {
            panic!("the tick should fail");
        };
        let failed = lot.world().hash_hex();
        for _ in 0..3 {
            assert_eq!(lot.step().unwrap_err(), StepError::Halted);
        }
        assert_eq!(lot.world().hash_hex(), failed);
        assert_eq!(lot.ticks(), 0);
    }

    #[test]
    fn the_view_rotation_is_an_input_taken_modulo_4() {
        let mut lot = lot(World::empty(), true, Frames::default());
        assert_eq!(lot.view_rotation(), 0);
        lot.set_view_rotation(3);
        assert_eq!(lot.view_rotation(), 3);
        lot.set_view_rotation(6);
        assert_eq!(lot.view_rotation(), 2);
        // The lot's state does not hold it.
        assert_eq!(lot.world().hash_hex(), World::empty().hash_hex());
    }

    #[test]
    fn play_mode_still_halts_on_a_failure_outside_an_objects_tick() {
        let mut lot = lot(World::empty(), true, Frames::default());
        lot.tuning = Some(tuning());
        let Err(StepError::Tick { tick: 1, .. }) = lot.step_play() else {
            panic!("the tick should fail");
        };
        assert_eq!(lot.step_play().unwrap_err(), StepError::Halted);
        assert_eq!(lot.play, Play::default());
    }

    #[test]
    fn an_order_that_fails_changes_nothing_and_a_halted_lot_takes_none() {
        let mut lot = lot(World::empty(), true, Frames::default());
        let before = lot.world().hash_hex();
        assert!(matches!(
            lot.queue_interaction(1, 2, 3),
            Err(OrderError::Refused(_))
        ));
        assert_eq!(lot.world().hash_hex(), before);
        assert!(!lot.halted);
        lot.halted = true;
        assert_eq!(lot.queue_interaction(1, 2, 3), Err(OrderError::Halted));
        let mut unloaded = super::tests::lot(World::empty(), false, Frames::default());
        assert_eq!(
            unloaded.queue_interaction(1, 2, 3),
            Err(OrderError::Incomplete)
        );
    }

    #[test]
    fn missing_motive_constants_fail_only_a_step() {
        // The load does not need them; the step says so, changes nothing,
        // and does not halt (a later step fails the same way).
        let mut lot = lot(World::empty(), true, Frames::default());
        let before = lot.world().hash_hex();
        for _ in 0..2 {
            let Err(StepError::Tuning(error)) = lot.step() else {
                panic!("the step should want the motive constants");
            };
            assert!(error.contains("tuning table"), "{error}");
        }
        assert_eq!(lot.world().hash_hex(), before);
        assert!(!lot.halted);
    }

    #[test]
    fn an_incomplete_load_runs_no_tick() {
        let mut lot = lot(World::empty(), false, Frames::default());
        lot.tuning = Some(tuning());
        let before = lot.world().hash_hex();
        assert_eq!(lot.step().unwrap_err(), StepError::Incomplete);
        assert_eq!(lot.world().hash_hex(), before);
    }

    #[test]
    fn a_frame_list_reads_ticks_and_repeats() {
        let frames = Frames::parse("0,2,10+5").unwrap();
        let drawn: Vec<u32> = (0..=21).filter(|&t| frames.after(t)).collect();
        assert_eq!(drawn, [0, 2, 10, 15, 20]);
        let every = Frames::parse("3+").unwrap();
        let drawn: Vec<u32> = (0..=6).filter(|&t| every.after(t)).collect();
        assert_eq!(drawn, [3, 4, 5, 6]);
        assert_eq!(Frames::parse("none").unwrap().every, []);
        assert_eq!(Frames::parse("4+0"), None);
        assert_eq!(Frames::parse("x"), None);
        let default = Frames::default();
        let drawn: Vec<u32> = (0..=100).filter(|&t| default.after(t)).collect();
        assert_eq!(drawn, [0, 1, 2, 3, 33, 63, 93]);
        assert_eq!(Frames::parse("0,1,2,3,33+30"), Some(default));
    }

    #[test]
    fn a_frame_list_keeps_every_repeating_schedule() {
        let frames = Frames::parse("0+2,1+3").unwrap();
        let drawn: Vec<u32> = (0..=10).filter(|&t| frames.after(t)).collect();
        assert_eq!(drawn, [0, 1, 2, 4, 6, 7, 8, 10]);
    }
}
