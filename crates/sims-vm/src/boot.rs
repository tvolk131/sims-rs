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

pub mod save;

use crate::play::Play;
use crate::selection::publish_waited_for;
use crate::{
    Answer, Answers, BuyCursor, Callback, Interpreter, PanelMode, Player, Program, Shown,
    ShownSpecialPanel, SkippedFrame, SpecialPanelResponse, SpecialPanels, Stream, TickError,
    Update,
};
use sims_content::{
    Animations, ArchivePool, AvatarRegistry, Catalog, House, Index, Install, Neighborhood, People,
    Surfaces,
};
use sims_core::CrtRng;
use sims_sim::World;
use sims_sim::load::{Context, Household};
use sims_sim::motives::Tuning;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::sync::Arc;

/// Anything that stops a boot, with a message that says where.
pub type BootError = Box<dyn std::error::Error + Send + Sync>;

/// Loads the localized default baby/character name from
/// `GameData/UIText.iff STR# 142` entry 5 (`0x44b0bf..0x44b0d6` -> `0x435f4e`).
pub fn default_character_name(install: &Install) -> String {
    read_default_character_name(install)
}

/// Loads the localized default baby/character name from
/// `GameData/UIText.iff STR# 142` entry 5 (`0x44b0bf..0x44b0d6` -> `0x435f4e`).
pub(crate) fn read_default_character_name(install: &Install) -> String {
    let Ok(path) = install.file("GameData/UIText.iff") else {
        return String::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return String::new();
    };
    let Ok(iff) = sims_formats::iff::Iff::parse(bytes) else {
        return String::new();
    };
    let Some(chunk) = iff.get(b"STR#", 142) else {
        return String::new();
    };
    let Ok(table) = sims_formats::strings::StringTable::parse(iff.data(chunk)) else {
        return String::new();
    };
    table.get(4, 1).unwrap_or_default().to_owned()
}

/// Loads initial `(lot_name, lot_description)` for `lot_id` from the
/// neighborhood's `Houses/*Desc.iff` `STR# (2000 + lot_id)` entries 1 and 2
/// (`cWinEditLotInfo` at `0x50e14b..0x50e253`):
/// - `21..=30` (`0x15..=0x1e`): `DTDesc.iff` (Downtown)
/// - `40..=48` (`0x28..=0x30`): `VIDesc.iff` (Vacation Island)
/// - `81..=89` (`0x51..=0x59`): `STDesc.iff` (Studio Town)
/// - `90..=99` (`0x5a..=0x63`): `MTDesc.iff` (Magic Town)
/// - otherwise, only when `0x42b9e3` zoning is nonzero (lot 0 returns 1):
///   `NeighborhoodDesc.iff` (`0x50e19b..0x50e1b2`)
fn read_lot_info(house: &Path, lot_id: u32, zoning: Option<i32>) -> (String, String) {
    let Some(houses_dir) = house.parent() else {
        return (String::new(), String::new());
    };
    let desc_file = match lot_id {
        21..=30 => "DTDesc.iff",
        40..=48 => "VIDesc.iff",
        81..=89 => "STDesc.iff",
        90..=99 => "MTDesc.iff",
        _ if lot_id == 0 || zoning.unwrap_or(0) != 0 => "NeighborhoodDesc.iff",
        _ => return (String::new(), String::new()),
    };
    let path = houses_dir.join(desc_file);
    let Ok(bytes) = std::fs::read(&path) else {
        return (String::new(), String::new());
    };
    let Ok(iff) = sims_formats::iff::Iff::parse(bytes) else {
        return (String::new(), String::new());
    };
    let Ok(table_id) = u16::try_from(2000u32.saturating_add(lot_id)) else {
        return (String::new(), String::new());
    };
    let Some(chunk) = iff.get(b"STR#", table_id) else {
        return (String::new(), String::new());
    };
    let Ok(table) = sims_formats::strings::StringTable::parse(iff.data(chunk)) else {
        return (String::new(), String::new());
    };
    let name = table.get(0, 1).unwrap_or_default().to_owned();
    let desc = table.get(1, 1).unwrap_or_default().to_owned();
    (name, desc)
}

/// The level the house view shows: the ground.
/// Global 11: set by Special Effect's first state (0x44aad6), cleared
/// by the idle loop before the next update (0x46dacc).
const EFFECT_SHOWN: usize = 11;

const VIEW_LEVEL: i8 = 1;

/// When the house view draws a frame, as far as drawing writes the
/// simulation's state (`frame.rs`). The game's main loop and the window
/// system decide that, not the simulation, so it is an input here. Tick 0
/// means before the first tick.
///
/// The default, `0,1,2,3,33+30`, is what three captures of the original
/// running the Goth house recorded, with a hardware watchpoint on the
/// house view's draw guard (`docs/decisions/`): a frame before the
/// first lot update, after updates 1, 2 and 3 (many times while a dialog
/// waited), after update 33, and after every 30th update from there (63,
/// 93, ...). Frames write two things: the first frame that draws a
/// person places its animator (on this house, the frame after tick 33
/// draws person 259, made in tick 6), and every frame counts the rooms
/// that wait to be counted (0x4a8895). So the schedule changes the
/// state: with a frame after every tick, the rooms are counted sooner.
/// On this house that changes no state the captures hold through tick
/// 261, but it changes the rooms' state in between.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// What the game holds when a lot loads straight after an open's pools
/// ([`Lot::load_after_pools`]).
#[derive(Debug, Clone, Default)]
pub struct Handoff {
    /// The skeletons, by name, whose files the game has already read, in exact
    /// open order so duplicate entries across already-opened catalogs preserve
    /// native first-registration order (`0x59f18c`, `0x59f34c`).
    pub skeletons_read: Vec<String>,
    /// Whether the two indicators' graphics roots already exist.
    pub indicators_made: bool,
}

/// Preloaded catalog person types: `(resolved, unresolved_by_guid)`.
pub type PreloadedPeople = (People, BTreeMap<u32, String>);

/// Loads the native-ordered wardrobe for Make New Character (`0x44b151`) using
/// `pool`, converting lenient archive/catalog errors into the `Result` stored
/// in `Program::ordered_wardrobe`.
pub fn load_ordered_wardrobe(
    install: &Install,
    pool: &ArchivePool,
) -> Result<sims_content::ImportWardrobe, String> {
    match sims_content::ImportWardrobe::load_lenient_with_pool(install, pool) {
        Ok((ordered, errors)) if errors.is_empty() => Ok(ordered),
        Ok((_, errors)) => {
            let (file, error) = errors.into_iter().next().unwrap();
            Err(format!("{file} ({error})"))
        }
        Err(error) => Err(error.to_string()),
    }
}

/// Pre-loaded content shared by a host (such as `sims-game`) so [`Lot::load_preloaded`]
/// reuses the same [`Install`], [`Neighborhood`], [`Index`], [`Catalog`],
/// [`Program`], [`Animations`], [`People`], [`Surfaces`], [`sims_content::Wardrobe`],
/// and [`sims_content::ImportWardrobe`] without scanning or parsing game archives twice.
///
/// Parallel stage results are passed as `Result`s so [`Lot::load_preloaded`]
/// checks every error at the exact original sequential point (`House::load` ->
/// `Program::link` -> `Neighborhood::zoning` -> `Animations::load` ->
/// `People::load_each` -> `icon_frames` -> `Surfaces::build` at step 6b).
pub struct PreloadedBoot<'a> {
    pub install: &'a Install,
    pub neighborhood: &'a Neighborhood,
    pub characters: Option<&'a Path>,
    pub index: &'a Index,
    pub catalog: Arc<Catalog>,
    pub pool: &'a ArchivePool,
    pub file: Option<House>,
    pub program: crate::Result<Program>,
    pub animations: sims_content::Result<Animations>,
    pub people: sims_content::Result<PreloadedPeople>,
    pub icon_frames: crate::Result<Option<BTreeMap<u16, i32>>>,
    pub wardrobe: sims_content::Wardrobe,
    pub ordered_wardrobe: Result<sims_content::ImportWardrobe, String>,
    pub surfaces: sims_content::Result<Surfaces>,
    pub default_character_name: String,
    pub view_centre: Option<[i8; 2]>,
}

/// A booted house: its linked code, its catalog and its lot.
pub struct Lot {
    pub(crate) program: Program,
    pub(crate) fast: crate::program::ProgramFast,
    pub(crate) catalog: Arc<Catalog>,
    /// The floor and wall catalogs the load renumbered the lot's patterns
    /// by (load step 6b).
    pub(crate) surfaces: Surfaces,
    pub(crate) world: World,
    /// The motive constants, worked out at the first step: a load that
    /// runs no tick does not need them.
    pub(crate) tuning: Option<Tuning>,
    pub(crate) player: Answers,
    pub(crate) frames: Frames,
    /// Ticks run, and the lot's tick count when the first one started.
    pub(crate) ticks: u32,
    pub(crate) start: u32,
    pub(crate) load: LoadReport,
    /// Set when a tick or a frame failed. The lot is then as the failure
    /// left it, partway through a tick or a frame, so no later step runs.
    pub(crate) halted: bool,
    /// Play mode's own state ([`crate::play`]), used by
    /// [`Lot::step_play`] only.
    pub(crate) play: Play,
    /// Whether a dialog that offers a choice, with no answer left in the
    /// list given at load, waits for the player ([`Lot::wait_for_answers`])
    /// rather than halting the lot.
    pub(crate) asks: bool,
    /// The dialog the lot waits on, while it waits.
    pub(crate) asking: Option<Box<Asking>>,
    /// The special panel (`Dialog` kinds `5..=15`) the lot waits on, while it waits.
    pub(crate) asking_special_panel: Option<Box<AskingSpecialPanel>>,
    /// Registry special-panel slots (`cObjectRegistry +0xe8..=+0x10c`) carried
    /// across ticks (`0x451552`, `0x4413e1..0x442320`).
    pub(crate) special_panels: SpecialPanels,
    /// Queued player responses for special panels (`Dialog` kinds `5..=15`).
    pub(crate) special_panel_responses: VecDeque<SpecialPanelResponse>,
    /// The dialog is answered and the tick that showed it has not run
    /// again yet: the lot is as it was before that tick, so it takes no
    /// input until the next step.
    pub(crate) answered: bool,

    /// The house view's rotation, 0 to 3 ([`Lot::set_view_rotation`]).
    pub(crate) view_rotation: u8,
    /// The saved house's opening camera direction, 0 to 3
    /// ([`opening_rotation`]): the rotation that level 1's road-dash floor
    /// tiles (`10` and `11`) in `world` are oriented for (`0x4a8042`,
    /// `0x4a861f`, `0x4a87b1`–`0x4a87dc`).
    pub(crate) floor_rotation: u8,
    /// The tile at the house view's centre, if known
    /// ([`Lot::set_view_centre`]).
    pub(crate) view_centre: Option<[i8; 2]>,
    /// The view rotation and centre frozen when a tick paused on an
    /// unanswered dialog (`Stop::Asks`), so moving the camera while a
    /// dialog is showing does not alter the tick's replay once answered.
    pub(crate) replay_view: Option<(u8, Option<[i8; 2]>)>,
    /// The text entered in the last kind 3 text-prompt dialog
    /// (`0x451a6b..0x451a85` -> global `[0x6e4ac0]`).
    pub(crate) last_dialog_text: String,
    /// Catalog names assigned to characters created by Make New Character
    /// (`0x44b0a8..0x44b0fc`), keyed by new character GUID.
    pub(crate) character_names: BTreeMap<u32, String>,
    /// Localized fallback name from `GameData/UIText.iff STR# 142[5]` for
    /// empty kind-3 Make New Character submissions (`0x44b0bf..0x44b0d6`).
    pub(crate) default_character_name: String,
    /// Editable lot name (`cWinEditLotInfo`, `0x50e1f8..0x50e42c`), loaded
    /// from the neighborhood's `*Desc.iff` `STR# (2000 + lot_id)` and persisted
    /// in [`save::Saved`].
    pub(crate) lot_name: String,
    /// Editable lot description (`cWinEditLotInfo`, `0x50e1f8..0x50e42c`),
    /// loaded from `*Desc.iff` `STR# (2000 + lot_id)` and persisted in
    /// [`save::Saved`].
    pub(crate) lot_description: String,
    /// The world's hash as the load left it, which a save carries so it
    /// goes back only into a lot loaded from the same files
    /// ([`save`]).
    pub(crate) loaded: [u8; 32],
    /// Committed Buy Mode purchases available for Undo (`0x493259`, `0x4932fa`).
    /// Cleared when leaving a non-Live mode (`0x4cea5d` → `0x4933dc`) or
    /// restoring a save.
    pub(crate) buy_undo: Vec<crate::BuyPurchase>,
    /// Undone Buy Mode purchases available for Redo (`0x493338`).
    pub(crate) buy_redo: Vec<crate::BuyPurchase>,
    /// Detached Buy Mode objects whose history cleanup (`0x48c20e`) faulted,
    /// tracked so [`Lot::save`] and mode switches never leak them silently.
    pub(crate) buy_orphans: Vec<crate::BuyPurchase>,
    /// The single live Buy Mode cursor currently owned by this lot, enforcing
    /// native single-cursor ownership and preventing retired/cross-lot
    /// [`BuyCursor`] handles from touching objects owned by this lot.
    pub(crate) active_buy_cursor: Option<ActiveBuyCursor>,
    /// Committed Build Mode operations available for Undo.
    pub(crate) build_undo: Vec<crate::BuildHistory>,
    /// Undone Build Mode operations available for Redo.
    pub(crate) build_redo: Vec<crate::BuildHistory>,
    /// Chronological order of Buy vs Build entries on the Undo/Redo stacks.
    pub(crate) undo_order: Vec<HistoryKind>,
    pub(crate) redo_order: Vec<HistoryKind>,
    /// The single live Build Mode `"Wall Cursor"` (`0x00000439`) owned by this lot.
    pub(crate) active_wall_cursor: Option<crate::WallCursor>,
    /// When opened via [`crate::lifecycle::enter_lot`], records the store,
    /// neighborhood ID, neighborhood instance identity, and revision this lot
    /// was loaded from so Save & Leave rejects cross-store saves,
    /// cross-neighborhood saves, re-imported same-ID overwrites, and
    /// stale-revision overwrites. Cleared by [`Lot::restore`].
    pub(crate) neighborhood_origin: Option<NeighborhoodOrigin>,
    /// Optional co-op multiplayer VM context (`crate::coop`). `None` in
    /// single-player, preserving 100% bit-for-bit identical behavior.
    pub(crate) coop: Option<crate::coop::CoopVmContext>,
    /// Buy Mode `move_objects` cheat override (`false` by default). Does not
    /// affect normal simulation ticks.
    pub(crate) move_objects: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoryKind {
    Buy,
    Build,
}

/// Store-backed origin of a live [`Lot`] opened via [`crate::lifecycle::enter_lot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NeighborhoodOrigin {
    pub store_id: u128,
    pub neighborhood_id: String,
    pub instance_id: u128,
    pub revision: u64,
}

/// Process-wide monotonic token allocator for [`BuyCursor`] handles so no two
/// cursors across any [`Lot`] instances ever share a token.
static NEXT_BUY_CURSOR_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn alloc_buy_cursor_token() -> u64 {
    loop {
        let token = NEXT_BUY_CURSOR_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if token != 0 {
            return token;
        }
    }
}

/// Lifecycle state tracked by [`Lot`] for the active [`BuyCursor`] handle.
#[derive(Debug, Clone)]
pub(crate) struct ActiveBuyCursor {
    pub(super) token: u64,
    pub(super) cursor_lead: u16,
    pub(super) held_lead: u16,
    pub(super) existing: bool,
    pub(super) owned_ids: BTreeSet<u16>,
    pub(super) cursor: BuyCursor,
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

/// A special panel (`Dialog` kinds `5..=15`) the lot waits on
/// ([`Lot::asking_special_panel`]). The tick that showed it has not happened:
/// the lot is as it was before that tick, and [`Lot::answer_special_panel`]
/// then a step runs the tick again with the supplied [`SpecialPanelResponse`].
#[derive(Debug, Clone)]
pub struct AskingSpecialPanel {
    pub shown: ShownSpecialPanel,
    /// The lot partway through the tick when the special panel was requested,
    /// for rendering picker state (portraits, outfits, neighbors, lots). Not
    /// the lot's committed state.
    pub world: World,
}

/// How [`Lot::advance`] stopped: a failure, or a dialog / special panel that
/// waits for the player.
enum Stop {
    Step(StepError),
    Asks { tick: u32, shown: crate::Shown },
    AsksSpecialPanel { tick: u32, shown: ShownSpecialPanel },
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
            Self::Asks { tick, .. } | Self::AsksSpecialPanel { tick, .. } => {
                StepError::Asking { tick }
            }
        }
    }
}

/// Why a special panel response was not taken ([`Lot::answer_special_panel`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecialPanelAnswerError {
    /// No special panel waits for a response.
    NotAsked,
    /// The response variant does not match the requested special panel kind.
    WrongResponse {
        expected_kind: u8,
        given: SpecialPanelResponse,
    },
}

impl std::fmt::Display for SpecialPanelAnswerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAsked => write!(f, "no special panel waits for a response"),
            Self::WrongResponse {
                expected_kind,
                given,
            } => write!(
                f,
                "special panel kind {expected_kind} does not accept response {given:?}"
            ),
        }
    }
}

impl std::error::Error for SpecialPanelAnswerError {}

/// Player adapter used by [`Lot`] to feed both ordinary dialog [`Answers`] and
/// queued [`SpecialPanelResponse`]s into [`Interpreter::lot_update_kept`].
struct LotPlayer<'a> {
    answers: &'a mut Answers,
    special_panels: &'a mut VecDeque<SpecialPanelResponse>,
    wait_for_books: bool,
}

impl Player for LotPlayer<'_> {
    fn answer(&mut self, shown: &Shown) -> Option<Answer> {
        self.answers.answer(shown)
    }

    fn special_panel(&mut self, shown: &ShownSpecialPanel) -> Option<SpecialPanelResponse> {
        if shown.request.is_dismiss_only() && !self.wait_for_books {
            if matches!(
                self.special_panels.front(),
                Some(SpecialPanelResponse::Dismissed)
            ) {
                return self.special_panels.pop_front();
            }
            return Some(SpecialPanelResponse::Dismissed);
        }
        self.special_panels.pop_front()
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
    /// left it. In play mode `update` holds what the step did before it
    /// failed: the errors caught in the objects' ticks, the frame before
    /// the first tick if it was skipped, and the sounds. It is None in
    /// strict mode.
    Tick {
        tick: u32,
        start: u32,
        error: String,
        update: Option<Box<Update>>,
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
            Self::Tick {
                tick, start, error, ..
            } => write!(
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
/// (`HOUS 0`, `0x4a87b1`–`0x4a87dc`), as the render track opens the view
/// (`sims_render::open_house`). On load, the original calls `0x4a7dfe` at
/// `0x4a861f` with `force_no_swap = 1`, which turns the scene
/// (`[0x6e5528] + 0x84` via `0x4791a6`) to that direction without swapping
/// road-dash floors.
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
    ///
    /// The game is taken to have just started: nothing has drawn from the
    /// C runtime's generator, which is at state 1, as no recording has
    /// seen it seeded. [`Lot::load_with`] takes it as earlier play left it.
    pub fn load(
        root: &Path,
        house: &Path,
        answers: &[u8],
        frames: Frames,
    ) -> Result<Self, BootError> {
        Self::load_with(root, house, answers, frames, CrtRng::from_state(1))
    }

    /// [`Lot::load`] with the tile at the house view's centre known from
    /// the start ([`Lot::set_view_centre`]): the load's own steps (the
    /// objects' visits, the selection, the frame before the first tick)
    /// see it too.
    pub fn load_viewed(
        root: &Path,
        house: &Path,
        answers: &[u8],
        frames: Frames,
        view_centre: Option<[i8; 2]>,
    ) -> Result<Self, BootError> {
        let directory = house
            .parent()
            .and_then(Path::parent)
            .ok_or("the house file must be inside a neighborhood's Houses directory")?;
        let neighborhood = Neighborhood::open(directory)?;
        Self::boot(
            root,
            neighborhood,
            house,
            answers,
            frames,
            CrtRng::from_state(1),
            &BTreeSet::new(),
            None,
            view_centre,
        )
    }

    /// [`Lot::load`] with the C runtime's generator (`World::crt_rng`)
    /// starting at `crt`: after a neighborhood's first open, for one,
    /// whose import and pools draw from it.
    ///
    /// `crt` is the state from which this load's own draws reach the
    /// original's entry state. Those include the 64 draws of making the
    /// two indicator objects (`Interpreter::create_indicators`), which the
    /// original may already have made before its lot load starts. So
    /// `crt` need not be the original's state at any one point before its
    /// load, and passing that state instead would replay those draws.
    pub fn load_with(
        root: &Path,
        house: &Path,
        answers: &[u8],
        frames: Frames,
        crt: CrtRng,
    ) -> Result<Self, BootError> {
        Self::load_with_frame_tree_bits(root, house, answers, frames, crt, &BTreeSet::new())
    }

    /// [`Lot::load_with`] with the tree bits the original's frame arena
    /// held where the load reads each saved frame (0x4746e4), as
    /// (saved object ID, zero-based frame index). The house file does
    /// not carry them: see
    /// [`Interpreter::read_objects_with_frame_tree_bits`].
    pub fn load_with_frame_tree_bits(
        root: &Path,
        house: &Path,
        answers: &[u8],
        frames: Frames,
        crt: CrtRng,
        frame_tree_bits: &BTreeSet<(u16, usize)>,
    ) -> Result<Self, BootError> {
        let directory = house
            .parent()
            .and_then(Path::parent)
            .ok_or("the house file must be inside a neighborhood's Houses directory")?;
        let neighborhood = Neighborhood::open(directory)?;
        Self::boot(
            root,
            neighborhood,
            house,
            answers,
            frames,
            crt,
            frame_tree_bits,
            None,
            None,
        )
    }

    /// [`Lot::load`] with the neighborhood already open, as opening it
    /// left it ([`crate::neighborhood_open::open_neighborhood`]). Its
    /// records and families come from `neighborhood` and its people's
    /// files from that neighborhood's `Characters` directory.
    pub fn load_in(
        root: &Path,
        neighborhood: Neighborhood,
        house: &Path,
        answers: &[u8],
        frames: Frames,
    ) -> Result<Self, BootError> {
        Self::load_in_with(
            root,
            neighborhood,
            house,
            answers,
            frames,
            CrtRng::from_state(1),
        )
    }

    /// [`Lot::load_in`] with the C runtime's generator starting at `crt`,
    /// as [`Lot::load_with`] takes it.
    pub fn load_in_with(
        root: &Path,
        neighborhood: Neighborhood,
        house: &Path,
        answers: &[u8],
        frames: Frames,
        crt: CrtRng,
    ) -> Result<Self, BootError> {
        Self::boot(
            root,
            neighborhood,
            house,
            answers,
            frames,
            crt,
            &BTreeSet::new(),
            None,
            None,
        )
    }

    /// [`Lot::load_in_with`] straight after an open's pools
    /// ([`crate::neighborhood_open::open_neighborhood`]), from the C
    /// runtime state they leave. What the game already holds then is
    /// given as `handoff`, not assumed: which skeleton files it has read
    /// (0x598af7 skips a file already read; that none was cleared, as
    /// 0x5a38b3 can, is assumed), and whether the two indicators
    /// exist, whose graphics roots draw 64 numbers when made (0x445e4a,
    /// 0x595d65). The lot still makes its indicator objects; with
    /// `indicators_made`, their draws are not charged again.
    ///
    /// Both are inputs from a recording, not ported: which callback ran
    /// the Townies and tourists, and what it did before them, is not
    /// traced. Track 2's run 2 fits an adult skeleton read and the
    /// indicators made (`20261120-neighborhood-open-replays-five-pools.md`).
    pub fn load_after_pools(
        root: &Path,
        neighborhood: Neighborhood,
        house: &Path,
        answers: &[u8],
        frames: Frames,
        crt: CrtRng,
        handoff: &Handoff,
    ) -> Result<Self, BootError> {
        Self::boot(
            root,
            neighborhood,
            house,
            answers,
            frames,
            crt,
            &BTreeSet::new(),
            Some(handoff),
            None,
        )
    }

    /// Boots `house` from already-loaded content ([`PreloadedBoot`]), skipping
    /// duplicate `Install`, `Index`, `Catalog`, `Program`, `Animations`,
    /// `People`, `Wardrobe`, and `Surfaces` loading while running the exact
    /// original lot load sequence.
    pub fn load_preloaded(
        house: &Path,
        answers: &[u8],
        frames: Frames,
        preloaded: PreloadedBoot<'_>,
    ) -> Result<Self, BootError> {
        let view_centre = preloaded.view_centre;
        Self::boot_preloaded(
            house,
            answers,
            frames,
            CrtRng::from_state(1),
            &BTreeSet::new(),
            None,
            view_centre,
            preloaded,
        )
    }

    /// The load itself, with every input the loaders above take.
    #[allow(clippy::too_many_arguments)]
    fn boot(
        root: &Path,
        neighborhood: Neighborhood,
        house: &Path,
        answers: &[u8],
        frames: Frames,
        crt: CrtRng,
        frame_tree_bits: &BTreeSet<(u16, usize)>,
        handoff: Option<&Handoff>,
        view_centre: Option<[i8; 2]>,
    ) -> Result<Self, BootError> {
        let install = Install::detect(root)?;
        let pool = ArchivePool::new();

        // Stage A: build Index+Catalog alongside install-only loaders that do
        // not depend on Index or Catalog. Errors are deferred until their
        // original sequential check points in `boot_preloaded`.
        let (
            index_and_catalog,
            registry_res,
            surfaces_res,
            wardrobe,
            ordered_wardrobe,
            icon_frames_res,
            default_name,
        ) = std::thread::scope(|scope| {
            let idx_handle = scope.spawn(
                || -> Result<(Option<std::path::PathBuf>, Index, Catalog), BootError> {
                    let mut index = Index::build(&install)?;
                    let characters = neighborhood.characters()?;
                    if let Some(characters) = &characters {
                        index.add_characters(characters)?;
                    }
                    let catalog = Catalog::build_with_pool(&index, &pool)?;
                    Ok((characters, index, catalog))
                },
            );
            let reg_handle = scope.spawn(|| AvatarRegistry::load_with_pool(&install, &pool));
            let surf_handle = scope.spawn(|| Surfaces::build_with_pool(&install, &pool));
            let ward_handle = scope.spawn(|| match sims_content::Assets::scan(&install.root) {
                Ok(assets) => sims_content::Wardrobe::load_with_pool(&assets, &pool),
                Err(error) => sims_content::Wardrobe::unread(error),
            });
            let ordered_ward_handle = scope.spawn(|| load_ordered_wardrobe(&install, &pool));
            let icon_handle = scope.spawn(|| crate::program::icon_frames(&install));
            let name = read_default_character_name(&install);
            (
                idx_handle
                    .join()
                    .expect("boot index/catalog worker panicked"),
                reg_handle.join().expect("boot registry worker panicked"),
                surf_handle.join().expect("boot surfaces worker panicked"),
                ward_handle.join().expect("boot wardrobe worker panicked"),
                ordered_ward_handle
                    .join()
                    .expect("boot ordered_wardrobe worker panicked"),
                icon_handle
                    .join()
                    .expect("boot icon_frames worker panicked"),
                name,
            )
        });

        let (characters, index, catalog) = index_and_catalog?;
        let catalog = Arc::new(catalog);
        // Check `House::load` right after `Catalog::build`, matching the
        // original boot sequence before any Program/Animation/People/Surface errors.
        let file = House::load(house, &catalog)?;

        // Stage B: link Program, load Animations, and resolve person types in
        // parallel now that Index, Catalog, and AvatarRegistry are ready.
        let (program_res, animations_res, people_res) = std::thread::scope(|scope| {
            let prog_handle = scope.spawn(|| {
                // The original has every definition registered, and the resize
                // step walks them all, so the whole catalog is linked.
                Program::link_with_pool(
                    &index,
                    &catalog,
                    catalog.objects.iter().map(|o| o.guid),
                    &pool,
                )
            });
            let anim_handle = scope.spawn(|| Animations::load_with_pool(&install, &index, &pool));
            let person_types = catalog
                .objects
                .iter()
                .filter(|o| o.kind == 2)
                .map(|o| o.guid);
            // Every person type's skeleton and appearance, for the people a
            // script makes. A type whose skeleton cannot be chosen is left
            // out; making a person of it fails.
            let people_res = People::load_each_with_registry_result(
                &index,
                &catalog,
                person_types,
                registry_res,
                &pool,
            );
            (
                prog_handle.join().expect("boot program worker panicked"),
                anim_handle.join().expect("boot animations worker panicked"),
                people_res,
            )
        });

        Self::boot_preloaded(
            house,
            answers,
            frames,
            crt,
            frame_tree_bits,
            handoff,
            view_centre,
            PreloadedBoot {
                install: &install,
                neighborhood: &neighborhood,
                characters: characters.as_deref(),
                index: &index,
                catalog,
                pool: &pool,
                file: Some(file),
                program: program_res,
                animations: animations_res,
                people: people_res,
                icon_frames: icon_frames_res,
                wardrobe,
                ordered_wardrobe,
                surfaces: surfaces_res,
                default_character_name: default_name,
                view_centre,
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn boot_preloaded(
        house: &Path,
        answers: &[u8],
        frames: Frames,
        crt: CrtRng,
        frame_tree_bits: &BTreeSet<(u16, usize)>,
        handoff: Option<&Handoff>,
        view_centre: Option<[i8; 2]>,
        preloaded: PreloadedBoot<'_>,
    ) -> Result<Self, BootError> {
        let PreloadedBoot {
            install: _,
            neighborhood,
            characters,
            index,
            catalog,
            pool: _,
            file,
            program,
            animations,
            people,
            icon_frames,
            wardrobe,
            ordered_wardrobe,
            surfaces: surfaces_res,
            default_character_name,
            view_centre: _,
        } = preloaded;

        let file = match file {
            Some(file) => file,
            None => House::load(house, &catalog)?,
        };
        let view_rotation = opening_rotation(&file);
        let context = Context::COMPLETE_COLLECTION;
        let mut program = program?;
        program.zoning = neighborhood.zoning(context.expansions)?;
        program.expansions = context.expansions;
        // The types registered from the neighborhood's Characters folder,
        // which the original registers with kind 1 (0x424a8b, 0x43c228).
        program.characters = catalog
            .objects
            .iter()
            .map(|o| o.guid)
            .filter(|&guid| {
                catalog.object(guid).is_some_and(|o| {
                    index.mounts[index.resources[o.resource].mount].path == Index::CHARACTERS
                })
            })
            .collect();
        program.animations = animations?;
        let (catalog_people, unresolved_people) = people?;
        program.people = catalog_people;
        program.icon_frames = icon_frames?;
        program.wardrobe = Some(wardrobe);
        program.ordered_wardrobe = Some(ordered_wardrobe);
        if let Some(characters) = characters {
            program.character_files = crate::program::character_files(index, characters);
        }
        let mut world = World::empty();
        world.crt_rng = crt;
        let mut load = LoadReport::default();
        let surfaces;
        {
            let interpreter = Interpreter {
                view_rotation,
                view_centre,
                // `0x42a2b4..0x42a2e5`: during load after `clear_family()` and
                // on unoccupied lots, `game + 0x10` holds an empty family.
                active_family: Some(&[]),
                ..Interpreter::new(&program, &catalog, Stream::Global)
            };
            // The lot the game holds before the load: empty, with the
            // neighborhood's people, the type attributes the neighborhood
            // saved and the two indicator objects; and the skeleton files
            // the game has read by then.
            world
                .skeleton_files
                .extend(crate::objects::read_before_the_first_load(&program.people));
            for name in handoff.iter().flat_map(|handoff| &handoff.skeletons_read) {
                let files: BTreeSet<&String> = program
                    .people
                    .skeletons
                    .values()
                    .filter(|skeleton| skeleton.name.eq_ignore_ascii_case(name))
                    .map(|skeleton| &skeleton.file)
                    .collect();
                let [file] = files.into_iter().collect::<Vec<_>>()[..] else {
                    return Err(format!(
                        "0x598af7: the handoff's skeleton {name:?} is not in exactly one file"
                    )
                    .into());
                };
                world.skeleton_files.insert(file.clone());
                // The handoff's catalog was already opened through `0x59f087`
                // before the lot load, so its indexed skeletons and appearances
                // are already registered in the avatar registry (preventing a
                // duplicate lazy load and CRT RNG draw during house load).
                if let Some((items, _)) = program.people.indexed_catalogs.get(file) {
                    for item in items {
                        if !program.people.skeletons_by_name.contains_key(&item.name)
                            && !world.loaded_indexed_skeletons.contains_key(&item.name)
                        {
                            world
                                .loaded_indexed_skeletons
                                .insert(item.name.clone(), Some(file.clone()));
                        }
                    }
                }
                if let Some((suits, _)) = program.people.indexed_suit_catalogs.get(file) {
                    let catalog_bones = program.people.indexed_suit_head_bones.get(file);
                    for (app_name, has_head) in suits {
                        if !program.people.eager_suits.contains(app_name)
                            && !world.loaded_indexed_suits.contains_key(app_name)
                        {
                            world
                                .loaded_indexed_suits
                                .insert(app_name.clone(), Some(*has_head));
                            if let Some(bones) = catalog_bones.and_then(|m| m.get(app_name)) {
                                world
                                    .loaded_indexed_suit_bones
                                    .insert(app_name.clone(), bones.clone());
                            }
                            if !program.people.indexed_suits.contains_key(app_name)
                                && !world.runtime_added_suits.contains(app_name)
                            {
                                world.runtime_added_suits.push(app_name.clone());
                            }
                        }
                    }
                }
            }
            world.read_neighbors(
                neighborhood,
                &sims_sim::neighbors::registry(index, &catalog),
            )?;
            world.drop_empty_families()?;
            world.type_attributes = program.type_attributes(&catalog)?;
            if let Some(saved) = &neighborhood.type_attributes {
                world.read_type_attributes(saved);
            }
            let before_indicators = world.crt_rng;
            interpreter.create_indicators(&mut world)?;
            if handoff.is_some_and(|handoff| handoff.indicators_made) {
                // The factory called above builds both graphics roots
                // (0x595d65); the handoff says that happened before.
                world.crt_rng = before_indicators;
            }
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
            // Step 5 (`0x415e20..0x415e67`): shrink the bare lot to the house's
            // requested size, rebuild its bare-lot rooms and tile flags
            // (`0x415e48`), and reconcile lot-wide controllers before clearing.
            // Vacation lots (40..=48) run `Find Location For` inside controller
            // init here, which reads the bare-lot room map and architecture
            // arrays (`0, 1/101, 2/102, 3/103, 8/108`) that 1.91 holds from
            // startup.
            let requested = match world.globals.get(23).copied().unwrap_or(0) {
                0 => sims_sim::FULL_SIZE,
                size => size,
            };
            if requested >= 4 && requested < world.size {
                let full = usize::from(sims_sim::FULL_SIZE as u16);
                let bare = |stride, fill| sims_formats::lot::Array {
                    width: full,
                    height: full,
                    stride,
                    encoding: 0,
                    data: vec![fill; full * full * stride],
                };
                for (id, stride, fill) in [
                    (0u16, 4usize, 0u8),
                    (1, 2, 0),
                    (2, 14, 0),
                    (3, 2, 0),
                    (4, 1, 0),
                    (6, 1, 0),
                    (7, 1, 0),
                    (8, 1, 0),
                    (9, 1, 0xff),
                    (10, 1, 0xff),
                    (101, 2, 0),
                    (102, 14, 0),
                    (103, 2, 0),
                    (108, 1, 0),
                ] {
                    world.arrays.insert(id, bare(stride, fill));
                }
                world.size = requested;
                world.rebuild_rooms()?;
                world.globals[27] = 0;
                load.created = interpreter.reconcile_controllers(&mut world)?.created.len();
            } else {
                load.created = interpreter.resize(&mut world)?.created.len();
            }
            world.clear_family()?;
            load.queued = world.removals.len();
            load.removed = interpreter.clear(&mut world)?.len();
            world.arrays.clear();
            // 6a and 6b: the lot arrays, then the saved floor and wall IDs.
            let version = file
                .version()
                .ok_or("the house file has no HOUS 1 chunk, which holds its version")?;
            world.read_arrays(&file.arrays, version)?;
            surfaces = surfaces_res?;
            load.renumbered = world.renumber_surfaces(
                file.floor_map.as_deref(),
                file.wall_map.as_deref(),
                &surfaces,
            )?;
            // 6c and 7: the type table, which marks the types whose scripts
            // changed. Its GUIDs have been through the house's GUID table
            // (0x43a6fb, in `House::load`).
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
            // Reuse `program.people` directly instead of rebuilding
            // AvatarRegistry a second time: `People::load_each` resolves
            // skeletons immutably via `AvatarRegistry::export_skeletons`, so
            // checking `unresolved_people` over `saved_guids` matches
            // `People::load` over the saved objects' GUIDs identically.
            let saved_guids: BTreeSet<u32> =
                file.objects.instances.values().map(|o| o.guid).collect();
            for &guid in &saved_guids {
                if let Some(reason) = unresolved_people.get(&guid) {
                    return Err(sims_content::Error::Unsupported(reason.clone()).into());
                }
            }
            let read = interpreter.read_objects_with_frame_tree_bits(
                &mut world,
                &file.objects,
                &program.people,
                frame_tree_bits,
            )?;
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
            let occupied = neighborhood.family_in(number);
            let (family, budget, flags, members, magic_history) = match occupied {
                Some((family, record)) => (
                    family,
                    record.budget,
                    record.flags,
                    record.members.as_slice(),
                    record.magic_history()?,
                ),
                None => (0u16, 0i32, 0u32, &[][..], None),
            };
            load.family = family;
            load.house = number;
            // 0x42821a: the people who are not visitors, noted before the
            // family is selected.
            let residents = world.residents();
            world.select_family(
                i16::try_from(family)?,
                i16::try_from(number)?,
                budget,
                magic_history,
            )?;
            // 13: every object is visited in registration order. The step
            // stops at the first visit that is not ported, with the lot as
            // the visit before it left it.
            load.to_visit = world.order.len();
            let visited = interpreter.visit_objects(&mut world, &program.people);
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
                world.finish_selection(&Household {
                    house: i32::try_from(number)?,
                    flags,
                    members,
                    characters: &program.characters,
                    residents: &residents,
                    zoning: &program.zoning,
                })?;
                let made_again = interpreter.announce_house(&mut world)?.is_some();
                let sim = interpreter.select_resident(&mut world, budget, members)?;
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
        let loaded = world.hash();
        let zoning = if load.house == 0 {
            Some(1)
        } else {
            i16::try_from(load.house)
                .ok()
                .and_then(|id| program.zoning.get(&id).copied())
        };
        let (lot_name, lot_description) = read_lot_info(house, load.house, zoning);
        let fast = crate::program::ProgramFast::build(&program, &catalog);
        Ok(Self {
            program,
            fast,
            catalog,
            surfaces,
            world,
            tuning: None,
            player: Answers::choices(answers),
            frames,
            ticks: 0,
            start,
            load,
            halted: false,
            play: Play::default(),
            asks: false,
            answered: false,
            asking: None,
            asking_special_panel: None,
            special_panels: crate::SpecialPanels::default(),
            special_panel_responses: std::collections::VecDeque::new(),

            view_rotation,
            floor_rotation: view_rotation,
            view_centre,
            replay_view: None,
            last_dialog_text: String::new(),
            character_names: BTreeMap::new(),
            default_character_name,
            lot_name,
            lot_description,
            loaded,
            buy_undo: Vec::new(),
            buy_redo: Vec::new(),
            buy_orphans: Vec::new(),
            active_buy_cursor: None,
            build_undo: Vec::new(),
            build_redo: Vec::new(),
            undo_order: Vec::new(),
            redo_order: Vec::new(),
            active_wall_cursor: None,
            neighborhood_origin: None,
            coop: None,
            move_objects: false,
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
    /// errors caught are in [`Update::errors`]. A frame that fails is
    /// skipped and noted in [`Update::frame_errors`]. Any other failure
    /// ends the step as in [`Lot::step`]. A lot update that fails
    /// ([`StepError::Tick`]) halts the lot, and carries what the step did
    /// before it failed.
    ///
    /// Play mode is not the original's behaviour wherever a caught error
    /// comes from something not ported, so a lot stepped this way is not
    /// one to compare with the original. [`Lot::step`] is unchanged by it.
    pub fn step_play(&mut self) -> Result<Update, StepError> {
        self.step_in(true)
    }

    pub(crate) fn step_in(&mut self, play: bool) -> Result<Update, StepError> {
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
            // A lot restored from a save made once a dialog was answered
            // takes input again after this step, as the saved lot would.
            self.answered = false;
            let result = self.advance(play).map_err(Stop::into_step);
            self.replay_view = None;
            if result.is_err() {
                self.halted = true;
            }
            self.sync_active_buy_cursor();
            return result;
        }
        if self.asking.is_some() || self.asking_special_panel.is_some() {
            return Err(StepError::Asking {
                tick: self.ticks + 1,
            });
        }
        // The tick may stop at a dialog the player has not answered; it
        // then runs again from here once they have, using the original
        // camera view (`replay_view`) even if the player moved the camera
        // while the dialog was showing.
        let before = (
            self.world.clone(),
            self.play.clone(),
            self.ticks,
            self.player.clone(),
            self.special_panels.clone(),
            self.special_panel_responses.clone(),
            self.coop.clone(),
        );
        self.answered = false;
        match self.advance(play) {
            Err(Stop::Asks { tick, shown }) => {
                let (world, play, ticks, player, special_panels, special_panel_responses, coop) =
                    before;
                let partway = std::mem::replace(&mut self.world, world);
                self.play = play;
                self.ticks = ticks;
                self.player = player;
                self.special_panels = special_panels;
                self.special_panel_responses = special_panel_responses;
                self.coop = coop;
                self.replay_view
                    .get_or_insert((self.view_rotation, self.view_centre));
                self.asking = Some(Box::new(Asking {
                    shown,
                    world: partway,
                }));
                Err(StepError::Asking { tick })
            }
            Err(Stop::AsksSpecialPanel { tick, shown }) => {
                let (world, play, ticks, player, special_panels, special_panel_responses, coop) =
                    before;
                let partway = std::mem::replace(&mut self.world, world);
                self.play = play;
                self.ticks = ticks;
                self.player = player;
                self.special_panels = special_panels;
                self.special_panel_responses = special_panel_responses;
                self.coop = coop;
                self.replay_view
                    .get_or_insert((self.view_rotation, self.view_centre));
                self.asking_special_panel = Some(Box::new(AskingSpecialPanel {
                    shown,
                    world: partway,
                }));
                Err(StepError::Asking { tick })
            }
            result => {
                self.replay_view = None;
                let result = result.map_err(Stop::into_step);
                if result.is_err() {
                    self.halted = true;
                }
                self.sync_active_buy_cursor();
                result
            }
        }
    }

    /// The tick and its frames, for a lot that can step.
    fn advance(&mut self, play: bool) -> Result<Update, Stop> {
        let coop_cell = self.coop.take().map(RefCell::new);
        let result = self.advance_with_coop(play, coop_cell.as_ref());
        self.coop = coop_cell.map(RefCell::into_inner);
        result
    }

    /// Inner tick execution with an optional co-op context cell so `self.coop`
    /// is unconditionally restored on every exit path (including `Stop::Asks`
    /// and errors).
    fn advance_with_coop(
        &mut self,
        play: bool,
        coop: Option<&RefCell<crate::coop::CoopVmContext>>,
    ) -> Result<Update, Stop> {
        let tuning = self.tuning.as_ref().ok_or(StepError::Incomplete)?;
        let asks = self.asks;
        let sounds = RefCell::new(Vec::new());
        let ui_effects = RefCell::new(Vec::new());
        let masks_rebuilt = Cell::new(0);
        let dialog_text = RefCell::new(self.last_dialog_text.clone());
        let character_names = RefCell::new(self.character_names.clone());
        let special_panels = RefCell::new(self.special_panels.clone());
        let (view_rotation, view_centre) = if let Some(coop_cell) = coop {
            // In co-op mode, local camera panning (`self.view_centre`) is UI-only
            // and never feeds the shared simulation (`docs/vision.md`); portrait
            // search reads the authoritative lot-coordinate `portrait_anchor`
            // from `CoopVmContext`, and base rotation comes from the host's
            // stamped rotation in `CoopVmContext`.
            (coop_cell.borrow().host_camera_rotation(), None)
        } else {
            self.replay_view
                .unwrap_or((self.view_rotation, self.view_centre))
        };
        let family_zero_members: Vec<u32> = if self.load.house != 0 && self.load.family == 0 {
            self.world
                .families
                .iter()
                .find(|f| f.id == 0)
                .map(|f| f.members.clone())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            ui_effects: Some(&ui_effects),
            masks_rebuilt: Some(&masks_rebuilt),
            view_rotation,
            view_centre,
            dialog_text: Some(&dialog_text),
            character_names: Some(&character_names),
            default_character_name: Some(&self.default_character_name),
            active_family: (self.load.house != 0 && self.load.family == 0)
                .then_some(family_zero_members.as_slice()),
            coop,
            special_panels: Some(&special_panels),
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut frame_errors = Vec::new();
        let mode = crate::panel_mode(&self.world);
        if self.ticks == 0
            && self.frames.after(0)
            && let Err(error) = interpreter.draw_frame_in(&mut self.world, VIEW_LEVEL, mode)
        {
            if !play {
                return Err(StepError::Frame {
                    tick: 0,
                    error: error.to_string(),
                    update: None,
                }
                .into());
            }
            frame_errors.push(SkippedFrame {
                after: 0,
                error: error.to_string(),
            });
        }
        let tick = self.ticks + 1;
        // The idle loop sets a speed of its own before it runs the update
        // (0x46d9d6, `World::idle_speed`).
        let residential = residential(&self.world, &self.program);
        self.world.idle_speed(residential);
        // The application's poll (0x4d1a8b → 0x4764fe) before the next
        // update sees 0 once the clicked dialog's button is destroyed.
        // The poll runs once per pass of the application's loop, whose
        // idle call can run several updates (0x46da70 to 0x46dac7); here
        // every update is taken to follow a poll (unverified). Inside such
        // a batch, the original would still show a clicked bitmap.
        if let Err(error) = publish_waited_for(&mut self.world, 0) {
            return Err(StepError::Tick {
                tick,
                start: self.start.wrapping_add(self.ticks),
                error: TickError::from(error).to_string(),
                update: None,
            }
            .into());
        }
        let mut lot_player = LotPlayer {
            answers: &mut self.player,
            special_panels: &mut self.special_panel_responses,
            wait_for_books: asks,
        };
        let (update, result) = interpreter.lot_update_kept(
            &mut self.world,
            tuning,
            &mut lot_player,
            play.then_some(&mut self.play),
        );
        let mut update = Update {
            frame_errors,
            ..update
        };
        if let Err(error) = result {
            return Err(match error {
                // Nobody has answered: the player is asked.
                TickError::Unanswered { shown, given: None } if asks => Stop::Asks { tick, shown },
                TickError::UnansweredSpecialPanel { shown, given: None } if asks => {
                    Stop::AsksSpecialPanel { tick, shown }
                }
                error => {
                    // In play mode what came before the failure goes with
                    // it: the errors caught, a skipped first frame, and
                    // the sounds.
                    update.sounds = sounds.take();
                    update.ui_effects = ui_effects.take();
                    update.masks_rebuilt = masks_rebuilt.get();
                    StepError::Tick {
                        tick,
                        start: self.start.wrapping_add(self.ticks),
                        error: error.to_string(),
                        update: play.then(|| Box::new(update)),
                    }
                    .into()
                }
            });
        }
        self.ticks = tick;
        // After each update (0x46da70) the idle call looks at global 11,
        // which Special Effect sets (0x44aad6): when it is set, the call
        // clears it and returns (0x46da98, 0x46dacc), leaving the rest of
        // its time to the next idle call, which sets the idle speed again
        // (changing nothing more) before its first update. The call
        // returns without clearing it when 0x6e54a8 is set, or when the
        // simulator's byte +0xd4 is set and a message is waiting (0x46da75,
        // 0x46da86); neither is taken to happen here (unverified). Whether the message loop draws in between is
        // the draw timer's to say, which the input schedule's frames stand
        // for, so no draw is added (unverified). See
        // docs/decisions/20261115-snap-offset-order.md.
        if let Some(effect) = self.world.globals.get_mut(EFFECT_SHOWN) {
            *effect = 0;
        }
        // Between lot updates the game handles its messages, which
        // redraws the family panel.
        interpreter.refresh_family_panel(&mut self.world);
        let frame = if self.frames.after(tick) {
            let mode = crate::panel_mode(&self.world);
            interpreter
                .draw_frame_in(&mut self.world, VIEW_LEVEL, mode)
                .err()
        } else {
            None
        };
        // The step's sounds go with its update whether or not the frame
        // failed.
        update.sounds = sounds.into_inner();
        update.ui_effects = ui_effects.into_inner();
        update.masks_rebuilt = masks_rebuilt.get();
        self.last_dialog_text = dialog_text.into_inner();
        self.character_names = character_names.into_inner();
        self.special_panels = special_panels.into_inner();
        if let Some(error) = frame {
            if play {
                update.frame_errors.push(SkippedFrame {
                    after: tick,
                    error: error.to_string(),
                });
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
        let character_names = RefCell::new(self.character_names.clone());
        let interpreter = Interpreter {
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            character_names: Some(&character_names),
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
        let character_names = RefCell::new(self.character_names.clone());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            character_names: Some(&character_names),
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
        let character_names = RefCell::new(self.character_names.clone());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            character_names: Some(&character_names),
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

    /// Queues the action of an entry [`Lot::pie_menu`] listed for
    /// `person` on the action's caller, as picking it does
    /// ([`Interpreter::pick`]): `person`, but for the offer to join
    /// `person`'s running interaction, which is queued on the person the
    /// menu was listed on. Returns the action's number, or None when the
    /// queue took nothing (it is full, the action's entry is gone, or the
    /// action has no caller). An order that fails changes nothing, as for
    /// [`Lot::queue_interaction`].
    pub fn pick(
        &mut self,
        person: u16,
        action: sims_formats::saved_objects::Interaction,
    ) -> Result<Option<i32>, OrderError> {
        self.ready()?;
        let character_names = RefCell::new(self.character_names.clone());
        let interpreter = Interpreter {
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            character_names: Some(&character_names),
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
        let character_names = RefCell::new(self.character_names.clone());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            character_names: Some(&character_names),
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
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        interpreter
            .select(&mut world, Some(person))
            .map_err(OrderError::Refused)?;
        // Selection triggers a control-panel redraw (0x53a01f -> 0x4cfcdb),
        // refreshing the selected Sim's family-friends cache on non-residential
        // lots (and global 9's family on residential lots).
        interpreter.refresh_family_panel(&mut world);
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

    /// Enables or disables the Buy Mode `move_objects` cheat override on this
    /// live lot (default `false`). When enabled, Buy Mode Hand tool pickup,
    /// placement, rotation, and deletion bypass normal occupancy/in-use/locked
    /// guards while still enforcing lot coordinate bounds and protecting
    /// persons, reserved sentinels, global controllers, and surface children.
    /// Does not affect normal simulation ticks.
    pub fn set_move_objects(&mut self, enabled: bool) {
        self.move_objects = enabled;
    }

    /// Whether the Buy Mode `move_objects` cheat override is currently enabled.
    pub fn move_objects(&self) -> bool {
        self.move_objects
    }

    /// Adjusts household funds by signed `delta` directly on the live
    /// [`World`] without invoking [`Lot::restore`], preserving Buy/Build undo
    /// history and `neighborhood_origin`. Updates globals 13 and 14 via
    /// [`World::set_funds`] (respecting budget mode `global[32]`), clamps the
    /// resulting balance to the exact lossless `globals[13..14]` script range
    /// `[-327_689_999, 327_679_999]`, refuses mutation while a co-op session is
    /// attached, and returns the resulting `world.budget.funds`.
    pub fn add_funds(&mut self, delta: i32) -> Result<i32, OrderError> {
        const MIN_CHEAT_FUNDS: i32 = -327_689_999;
        const MAX_CHEAT_FUNDS: i32 = 327_679_999;
        self.ready()?;
        if self.coop.is_some() {
            return Err(OrderError::Refused(crate::Fault {
                object: 0,
                at: None,
                trap: crate::Trap::Invalid(
                    "cannot invoke local add_funds cheat while co-op session is attached".into(),
                ),
            }));
        }
        let next = self
            .world
            .budget
            .funds
            .saturating_add(delta)
            .clamp(MIN_CHEAT_FUNDS, MAX_CHEAT_FUNDS);
        self.world.set_funds(next).map_err(|error| {
            OrderError::Refused(crate::Fault {
                object: 0,
                at: None,
                trap: crate::Trap::Invalid(error.to_string()),
            })
        })?;
        Ok(self.world.budget.funds)
    }

    /// Sets the live lot clock to `hour:00:00` (`0..24`) directly on the live
    /// [`World`] without invoking [`Lot::restore`], preserving Buy/Build undo
    /// history and `neighborhood_origin`. Updates global 0 (hour), clears
    /// globals 5 and 6 (minute/second), updates global 4 (time-of-day period,
    /// marking all rooms for light recount when the period changes, matching
    /// `0x46d86f` / `0x46d525`), and refuses mutation while a co-op session is
    /// attached.
    pub fn set_hour(&mut self, hour: u8) -> Result<(), OrderError> {
        self.ready()?;
        if self.coop.is_some() {
            return Err(OrderError::Refused(crate::Fault {
                object: 0,
                at: None,
                trap: crate::Trap::Invalid(
                    "cannot invoke local set_hour cheat while co-op session is attached".into(),
                ),
            }));
        }
        if hour >= 24 {
            return Err(OrderError::Refused(crate::Fault {
                object: 0,
                at: None,
                trap: crate::Trap::Invalid(format!("hour {hour} out of range 0..24")),
            }));
        }
        if self.world.globals.len() <= 6 {
            return Err(OrderError::Refused(crate::Fault {
                object: 0,
                at: None,
                trap: crate::Trap::Invalid(format!(
                    "lot has only {} globals; cannot set hour",
                    self.world.globals.len()
                )),
            }));
        }
        self.world.globals[0] = i16::from(hour);
        self.world.globals[5] = 0;
        self.world.globals[6] = 0;
        let tod = match hour {
            0..=5 => 2,
            6 => 3,
            7..=17 => 0,
            18 => 1,
            _ => 2,
        };
        if self.world.globals[4] != tod {
            self.world.globals[4] = tod;
            self.world.mark_all_rooms();
        }
        Ok(())
    }

    /// The simulation's active panel mode from global 19 (`0x4ce44f`).
    pub fn mode(&self) -> PanelMode {
        crate::panel_mode(&self.world)
    }

    /// Switches the simulation's active panel mode between ticks
    /// (`0x4cea5d` + `0x4ce44f`). Leaving a non-Live mode back to
    /// [`PanelMode::Live`] marks all rooms uncounted, updates lot
    /// statistics, and clears global 19 (`0x416534`). Leaving any non-Live
    /// mode clears the Buy Mode Undo/Redo stacks and deletes any detached
    /// off-lot redo instances (`0x4cea5d` → `0x4933dc` → `0x48c20e`).
    pub fn set_mode(&mut self, mode: PanelMode) -> Result<(), OrderError> {
        self.set_mode_releasing_buy_cursor(mode, None)
    }

    /// Switches the simulation's active panel mode while tearing down an
    /// active Buy Mode `cursor` in exact native `0x4cea5d` order:
    /// 1. `0x4cea9e` → `0x416534` (`go_live` / `set_mode`),
    /// 2. `0x4ceac2` → `0x40726a` (`update_comeandsee`, while detached
    ///    history objects and the active Buy cursor/preview are still
    ///    registered so marker IDs and positions match native),
    /// 3. `0x4ceaf5` → `0x4933dc` → `0x48c20e` (`buy_purge_detached`,
    ///    keeping objects owned by `cursor` alive for step 4),
    /// 4. `0x4cec3a` → `0x4cef9b` → `0x487191` / `0x487105` (`buy_cancel` on
    ///    `cursor`, deleting unpaid previews or snapping back held existing
    ///    objects). If snapback is refused, the entire mode switch rolls back
    ///    atomically.
    pub fn set_mode_releasing_buy_cursor(
        &mut self,
        mode: PanelMode,
        cursor: Option<&BuyCursor>,
    ) -> Result<(), OrderError> {
        self.ready()?;
        let previous = self.mode();
        let clearing = previous != PanelMode::Live && mode != previous;
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let release_cursor = cursor
            .filter(|c| self.is_active_buy_cursor(c))
            .cloned()
            .or_else(|| {
                clearing
                    .then(|| self.active_buy_cursor.as_ref().map(|a| a.cursor.clone()))
                    .flatten()
            });
        let mut world = self.world.clone();
        interpreter
            .set_mode(&mut world, mode)
            .map_err(OrderError::Refused)?;
        if previous != PanelMode::Live && mode == PanelMode::Live {
            interpreter
                .update_comeandsee(&mut world)
                .map_err(OrderError::Refused)?;
        }
        if clearing {
            let mut keep = BTreeSet::new();
            if let Some(cur) = release_cursor.as_ref() {
                crate::buy::collect_cursor_ids(cur, &mut keep);
                keep.insert(interpreter.lead_object(&world, cur.cursor));
                keep.insert(interpreter.lead_object(&world, cur.held));
                // Protect any detached ancestor container holding `cur.held`
                // (e.g. an unpaid preview sitting in an undone table's slot)
                // until `buy_cancel` unlinks `cur.held` cleanly; a final pass
                // below purges any deferred off-lot container afterwards.
                let mut probe: Vec<u16> = keep.iter().copied().collect();
                while let Some(id) = probe.pop() {
                    if let Some(cid) = world
                        .objects
                        .get(&id)
                        .and_then(|o| o.data.get(2).copied())
                        .and_then(|c| u16::try_from(c).ok())
                        .filter(|&c| c != 0)
                    {
                        let lead = interpreter.lead_object(&world, cid);
                        if keep.insert(cid) {
                            probe.push(cid);
                        }
                        if keep.insert(lead) {
                            probe.push(lead);
                        }
                    }
                }
            }
            interpreter
                .buy_purge_detached(&mut world, &self.buy_redo, &keep)
                .map_err(OrderError::Refused)?;
            interpreter
                .buy_purge_detached(&mut world, &self.buy_undo, &keep)
                .map_err(OrderError::Refused)?;
            interpreter
                .buy_purge_detached(&mut world, &self.buy_orphans, &keep)
                .map_err(OrderError::Refused)?;
            let mut build_detached = Vec::new();
            for entry in self.build_undo.iter().chain(self.build_redo.iter()) {
                build_detached.extend_from_slice(entry.detached_objects());
            }
            interpreter
                .buy_purge_detached(&mut world, &build_detached, &keep)
                .map_err(OrderError::Refused)?;
        }
        if let Some(cur) = release_cursor.as_ref() {
            let live = self.strip_detached_origin_children(cur);
            interpreter
                .buy_cancel(&mut world, &live)
                .map_err(OrderError::Refused)?;
            if clearing {
                let empty = BTreeSet::new();
                interpreter
                    .buy_purge_detached(&mut world, &self.buy_redo, &empty)
                    .map_err(OrderError::Refused)?;
                interpreter
                    .buy_purge_detached(&mut world, &self.buy_undo, &empty)
                    .map_err(OrderError::Refused)?;
                interpreter
                    .buy_purge_detached(&mut world, &self.buy_orphans, &empty)
                    .map_err(OrderError::Refused)?;
                let mut build_detached = Vec::new();
                for entry in self.build_undo.iter().chain(self.build_redo.iter()) {
                    build_detached.extend_from_slice(entry.detached_objects());
                }
                interpreter
                    .buy_purge_detached(&mut world, &build_detached, &empty)
                    .map_err(OrderError::Refused)?;
            }
            self.active_buy_cursor = None;
        } else {
            let clear = self.reconcile_active_buy_cursor(&interpreter, &mut world, None);
            if clear {
                self.active_buy_cursor = None;
            }
        }
        if clearing && let Some(wall_cur) = self.active_wall_cursor.as_ref() {
            interpreter
                .build_cursor_close(&mut world, wall_cur)
                .map_err(OrderError::Refused)?;
        }
        self.world = world;
        if clearing {
            self.buy_undo.clear();
            self.buy_redo.clear();
            self.buy_orphans.clear();
            self.build_undo.clear();
            self.build_redo.clear();
            self.undo_order.clear();
            self.redo_order.clear();
            self.active_wall_cursor = None;
        }
        Ok(())
    }

    /// Starts placing a newly purchased catalog object of catalog master GUID
    /// `guid` (`0x486f5a` → `0x421986` → `0x4215e6` → `0x4226bc`), wrapped
    /// in a `"Tile Cursor"` (`0x00000437`). Returns `Ok(None)` if the
    /// household cannot afford it.
    pub fn buy_start(
        &mut self,
        guid: u32,
        level: i32,
        direction: i16,
    ) -> Result<Option<BuyCursor>, OrderError> {
        self.ready()?;
        if self.active_buy_cursor.is_some() {
            return Ok(None);
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let mut cursor = interpreter
            .buy_start(&mut world, guid, level, direction)
            .map_err(OrderError::Refused)?;
        if let Some(cur) = &mut cursor {
            let token = alloc_buy_cursor_token();
            cur.token = token;
            let cursor_lead = interpreter.lead_object(&world, cur.cursor);
            let held_lead = interpreter.lead_object(&world, cur.held);
            let mut owned_ids = BTreeSet::new();
            crate::buy::collect_cursor_ids(cur, &mut owned_ids);
            owned_ids.insert(held_lead);
            owned_ids.remove(&cur.cursor);
            owned_ids.remove(&cursor_lead);
            self.active_buy_cursor = Some(ActiveBuyCursor {
                token,
                cursor_lead,
                held_lead,
                existing: false,
                owned_ids,
                cursor: cur.clone(),
            });
        }
        self.world = world;
        Ok(cursor)
    }

    /// Whether existing lot object `id` (or its multi-tile group) can be
    /// picked up with the Buy Mode Hand Tool (`0x4352e8` / `0x422fdc`).
    pub fn can_pick_up(&self, id: u16) -> Result<bool, OrderError> {
        self.ready()?;
        let interpreter = Interpreter {
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        interpreter
            .can_pick_up(&self.world, id)
            .map_err(OrderError::Refused)
    }

    /// Whether existing lot object `id` (or its multi-tile group) can be sold
    /// or deleted in Buy Mode (`0x435440` / `0x423135`).
    pub fn can_delete(&self, id: u16) -> Result<bool, OrderError> {
        self.ready()?;
        let interpreter = Interpreter {
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        interpreter
            .can_delete(&self.world, id)
            .map_err(OrderError::Refused)
    }

    /// Resale/depreciation value of `id` (`0x43219a`: lead `data[41]`, halved
    /// if any part has `data[15] != 0`).
    pub fn resale_value(&self, id: u16) -> Result<i32, OrderError> {
        self.ready()?;
        let interpreter = Interpreter {
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        interpreter
            .resale_value(&self.world, id)
            .map_err(OrderError::Refused)
    }

    /// Picks up an existing lot object with the Buy Mode Hand Tool
    /// (`0x485f98` → `0x486960` → `0x4215e6` → `0x48ba19`). Returns `Ok(None)`
    /// without changing the lot if `can_pick_up` refuses `id`.
    pub fn buy_pick_up(&mut self, id: u16) -> Result<Option<BuyCursor>, OrderError> {
        self.ready()?;
        if self.active_buy_cursor.is_some() {
            return Ok(None);
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let lead = interpreter.lead_object(&self.world, id);
        let mut world = self.world.clone();
        let mut cursor = interpreter
            .buy_pick_up(&mut world, id)
            .map_err(OrderError::Refused)?;
        if let Some(cur) = &mut cursor {
            let token = alloc_buy_cursor_token();
            cur.token = token;
            let cursor_lead = interpreter.lead_object(&world, cur.cursor);
            let mut owned_ids = BTreeSet::new();
            crate::buy::collect_cursor_ids(cur, &mut owned_ids);
            owned_ids.insert(lead);
            owned_ids.remove(&cur.cursor);
            owned_ids.remove(&cursor_lead);
            self.active_buy_cursor = Some(ActiveBuyCursor {
                token,
                cursor_lead,
                held_lead: lead,
                existing: true,
                owned_ids,
                cursor: cur.clone(),
            });
            self.world = world;
        }
        Ok(cursor)
    }

    /// Reconciles the active [`BuyCursor`] against `world` (and optional
    /// history-mutated IDs `affected`): if the held object or any owned origin
    /// child was detached, deleted, or mutated by Undo/Redo, or if its `"Tile
    /// Cursor"` disappeared, immediately removes any orphaned `"Tile Cursor"`
    /// on `world` and returns `true` so the caller retires `active_buy_cursor`.
    fn reconcile_active_buy_cursor(
        &self,
        interpreter: &Interpreter<'_>,
        world: &mut World,
        affected: Option<&BTreeSet<u16>>,
    ) -> bool {
        let Some(active) = self.active_buy_cursor.as_ref() else {
            return false;
        };
        let cursor_missing = world
            .objects
            .get(&active.cursor_lead)
            .is_none_or(|o| o.guid != crate::TILE_CURSOR_GUID);
        let held_missing = !world.objects.contains_key(&active.held_lead);
        let affected_hit = affected.is_some_and(|ids| !ids.is_disjoint(&active.owned_ids));
        let detached_hit =
            active.existing && self.is_detached_history_object_in(world, active.held_lead);
        if cursor_missing || held_missing || affected_hit || detached_hit {
            if !cursor_missing {
                let _ = interpreter.buy_discard_tile_cursor_id(world, active.cursor_lead);
            }
            return true;
        }
        false
    }

    /// Reconciles `self.active_buy_cursor` against `self.world` directly.
    fn sync_active_buy_cursor(&mut self) {
        if self.active_buy_cursor.is_none() {
            return;
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let clear = self.reconcile_active_buy_cursor(&interpreter, &mut world, None);
        if clear {
            self.active_buy_cursor = None;
            self.world = world;
        }
    }

    /// Whether `cursor` matches the lot's single currently active [`BuyCursor`].
    pub(super) fn is_active_buy_cursor(&self, cursor: &BuyCursor) -> bool {
        cursor.token != 0
            && self.active_buy_cursor.as_ref().is_some_and(|active| {
                active.token == cursor.token
                    && active.cursor.cursor == cursor.cursor
                    && active.cursor.held == cursor.held
                    && active.existing == cursor.is_existing()
            })
    }

    /// Whether `cursor` is still an active [`BuyCursor`].
    fn is_live_buy_cursor(&self, cursor: &BuyCursor) -> bool {
        self.is_active_buy_cursor(cursor)
    }

    /// Whether `id` is currently detached/refunded (`!placed` and off-lot in
    /// `world`) in Buy Mode history (`buy_undo`, `buy_redo`, or `buy_orphans`).
    fn is_detached_history_object_in(&self, world: &World, id: u16) -> bool {
        self.buy_undo
            .iter()
            .chain(self.buy_redo.iter())
            .chain(self.buy_orphans.iter())
            .any(|p| {
                !p.placed
                    && p.object == id
                    && world.objects.get(&id).is_none_or(|o| o.position[0] < 0)
            })
    }

    /// Whether `id` is currently detached/refunded (`!placed` and off-lot) in
    /// Buy Mode history (`buy_undo`, `buy_redo`, or `buy_orphans`).
    fn is_detached_history_object(&self, id: u16) -> bool {
        self.is_detached_history_object_in(&self.world, id)
    }

    /// Returns a copy of `cursor` whose `origin.children` exclude any surface
    /// children that have already been detached/refunded by an intervening
    /// Undo/Sell so snapback never resurrects refunded children for free.
    pub(super) fn strip_detached_origin_children(&self, cursor: &BuyCursor) -> BuyCursor {
        fn prune(lot: &Lot, placement: &mut crate::BuyPlacement) {
            placement
                .children
                .retain(|c| !lot.is_detached_history_object(c.object));
            for child in &mut placement.children {
                prune(lot, child);
            }
        }
        let mut out = cursor.clone();
        if let Some(origin) = &mut out.origin {
            prune(self, origin);
        }
        out
    }

    /// Sells or deletes the held Buy Mode object (`0x487e18`):
    /// - Returns `Ok(false)` without mutating `world` if `can_delete` refuses
    ///   (or if `cursor` is a stale existing-object cursor whose held object
    ///   was already detached/refunded by Undo/Sell).
    /// - For an unpaid catalog preview, deletes both objects with `0` refund.
    /// - For a picked-up existing object, refunds `resale_value`, deletes only
    ///   `"Tile Cursor"`, leaves the sold object detached off-lot for Undo,
    ///   pushes a `BuyActionKind::Sell` undo record, and purges `buy_redo`.
    pub fn buy_sell(&mut self, cursor: &BuyCursor) -> Result<bool, OrderError> {
        self.ready()?;
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        if !self.is_live_buy_cursor(cursor) {
            return Ok(false);
        }
        let mut world = self.world.clone();
        let Some(record) = interpreter
            .buy_sell(&mut world, cursor)
            .map_err(OrderError::Refused)?
        else {
            return Ok(false);
        };
        if let Some(sell_record) = record {
            let mut keep = BTreeSet::new();
            crate::buy::collect_buy_ids(&self.buy_undo, &mut keep);
            crate::buy::collect_buy_ids(std::slice::from_ref(&sell_record), &mut keep);
            interpreter
                .buy_purge_detached(&mut world, &self.buy_redo, &keep)
                .map_err(OrderError::Refused)?;
            let mut build_redo_detached = Vec::new();
            for entry in &self.build_redo {
                build_redo_detached.extend_from_slice(entry.detached_objects());
            }
            interpreter
                .buy_purge_detached(&mut world, &build_redo_detached, &keep)
                .map_err(OrderError::Refused)?;
            self.world = world;
            self.buy_undo.push(sell_record);
            self.buy_redo.clear();
            self.build_redo.clear();
            self.undo_order.push(HistoryKind::Buy);
            self.redo_order.clear();
        } else {
            self.world = world;
        }
        self.active_buy_cursor = None;
        Ok(true)
    }

    /// Moves an active Buy Mode cursor and its held object to fine lot
    /// coordinates `fine` on `level` (`0x485a2e` → `0x421e72`).
    pub fn buy_move(
        &mut self,
        cursor: &mut BuyCursor,
        fine: [i32; 2],
        level: i32,
    ) -> Result<bool, OrderError> {
        self.ready()?;
        if !self.is_live_buy_cursor(cursor) {
            return Ok(false);
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let mut next = cursor.clone();
        let valid = interpreter
            .buy_move(&mut world, &mut next, fine, level)
            .map_err(OrderError::Refused)?;
        self.world = world;
        if let Some(active) = self.active_buy_cursor.as_mut() {
            active.cursor = next.clone();
        }
        *cursor = next;
        Ok(valid)
    }

    /// Rotates an active Buy Mode cursor's held object to `direction`
    /// (`0x4226bc` → `0x421be4`) and re-evaluates placement.
    /// Turns the held Buy Mode object by signed `steps` of its turn step in
    /// one `cTSTileCursor::Turn(steps)` call (`0x486cb8` → `0x421be4`).
    pub fn buy_turn(&mut self, cursor: &mut BuyCursor, steps: i32) -> Result<bool, OrderError> {
        self.ready()?;
        if !self.is_live_buy_cursor(cursor) {
            return Ok(false);
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let mut next = cursor.clone();
        let valid = interpreter
            .buy_turn(&mut world, &mut next, steps)
            .map_err(OrderError::Refused)?;
        self.world = world;
        if let Some(active) = self.active_buy_cursor.as_mut() {
            active.cursor = next.clone();
        }
        *cursor = next;
        Ok(valid)
    }

    pub fn buy_rotate(
        &mut self,
        cursor: &mut BuyCursor,
        direction: i16,
    ) -> Result<bool, OrderError> {
        self.ready()?;
        if !self.is_live_buy_cursor(cursor) {
            return Ok(false);
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let mut next = cursor.clone();
        let valid = interpreter
            .buy_rotate(&mut world, &mut next, direction)
            .map_err(OrderError::Refused)?;
        self.world = world;
        if let Some(active) = self.active_buy_cursor.as_mut() {
            active.cursor = next.clone();
        }
        *cursor = next;
        Ok(valid)
    }

    /// Hides an active Buy Mode cursor off the lot when the pointer leaves
    /// the house view (`0x487d4e`).
    pub fn buy_hide(&mut self, cursor: &mut BuyCursor) -> Result<(), OrderError> {
        self.ready()?;
        if !self.is_live_buy_cursor(cursor) {
            return Ok(());
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let mut next = cursor.clone();
        interpreter
            .buy_hide(&mut world, &mut next)
            .map_err(OrderError::Refused)?;
        self.world = world;
        if let Some(active) = self.active_buy_cursor.as_mut() {
            active.cursor = next.clone();
        }
        *cursor = next;
        Ok(())
    }

    /// Re-evaluates Buy Mode placement directly at the `"Tile Cursor"`'s
    /// physical `(x, y)` on `level` (`0x485b62..0x485bda` → `0x421e72`).
    pub fn buy_recheck(&mut self, cursor: &mut BuyCursor, level: i32) -> Result<bool, OrderError> {
        self.ready()?;
        if !self.is_live_buy_cursor(cursor) {
            return Ok(false);
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let mut next = cursor.clone();
        let valid = interpreter
            .buy_recheck(&mut world, &mut next, level)
            .map_err(OrderError::Refused)?;
        self.world = world;
        if let Some(active) = self.active_buy_cursor.as_mut() {
            active.cursor = next.clone();
        }
        *cursor = next;
        Ok(valid)
    }

    /// Checks `Tile Cursor` fit in place on `level` and, if it fits, places at
    /// the cursor's physical `(x, y)` without an extra take-off (`0x485e59` →
    /// `0x485e72`).
    pub fn buy_put_if_fits(
        &mut self,
        cursor: &mut BuyCursor,
        level: i32,
    ) -> Result<bool, OrderError> {
        self.ready()?;
        if !self.is_live_buy_cursor(cursor) {
            return Ok(false);
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let mut next = cursor.clone();
        let valid = interpreter
            .buy_put_if_fits(&mut world, &mut next, level)
            .map_err(OrderError::Refused)?;
        self.world = world;
        if let Some(active) = self.active_buy_cursor.as_mut() {
            active.cursor = next.clone();
        }
        *cursor = next;
        Ok(valid)
    }

    /// Commits a valid Buy Mode placement (`0x486ada` → `0x421a0e`, `0x46d91c`):
    /// deletes the `"Tile Cursor"`, places the held object permanently, runs
    /// OBJf function 11 (`USER_PLACEMENT`), deducts its price from household
    /// funds, and pushes an undo record (`0x486ada` → `0x493259`, clearing the
    /// redo stack and deleting any detached off-lot redo instances via
    /// `0x48c20e`). Returns the placed object's ID. On error, the lot is left
    /// unchanged so the caller still owns `cursor`.
    pub fn buy_commit(&mut self, cursor: &BuyCursor) -> Result<u16, OrderError> {
        self.ready()?;
        if !self.is_live_buy_cursor(cursor) {
            return Err(OrderError::Refused(crate::Fault {
                object: cursor.held,
                at: None,
                trap: crate::Trap::Invalid("cannot commit stale cursor for detached object".into()),
            }));
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let purchase = interpreter
            .buy_commit_record(&mut world, cursor)
            .map_err(OrderError::Refused)?;
        let mut keep = BTreeSet::new();
        crate::buy::collect_buy_ids(&self.buy_undo, &mut keep);
        crate::buy::collect_buy_ids(std::slice::from_ref(&purchase), &mut keep);
        interpreter
            .buy_purge_detached(&mut world, &self.buy_redo, &keep)
            .map_err(OrderError::Refused)?;
        let mut build_redo_detached = Vec::new();
        for entry in &self.build_redo {
            build_redo_detached.extend_from_slice(entry.detached_objects());
        }
        interpreter
            .buy_purge_detached(&mut world, &build_redo_detached, &keep)
            .map_err(OrderError::Refused)?;
        let id = purchase.object;
        self.world = world;
        self.buy_undo.push(purchase);
        self.buy_redo.clear();
        self.build_redo.clear();
        self.undo_order.push(HistoryKind::Buy);
        self.redo_order.clear();
        self.active_buy_cursor = None;
        Ok(id)
    }

    /// Whether there is a committed Buy Mode purchase to undo (`0x4cf074`).
    pub fn can_buy_undo(&self) -> bool {
        !self.buy_undo.is_empty()
    }

    /// Whether there is an undone Buy Mode purchase to redo (`0x4cf08c`).
    pub fn can_buy_redo(&self) -> bool {
        !self.buy_redo.is_empty()
    }

    /// Deletes any detached (off-lot) instances referenced by `dropped` from
    /// `self.world` one by one (`0x48c20e` / `0x48c258`). Any still-registered
    /// off-lot instance whose deletion faults is retained in `self.buy_orphans`
    /// so [`Lot::save`] never silently serializes a detached object.
    fn purge_buy_history(&mut self, dropped: &[crate::BuyPurchase], keep: &BTreeSet<u16>) {
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        for purchase in dropped {
            let mut next = self.world.clone();
            match interpreter.buy_purge_detached(&mut next, std::slice::from_ref(purchase), keep) {
                Ok(()) => self.world = next,
                Err(_) => {
                    if self
                        .world
                        .objects
                        .get(&purchase.object)
                        .is_some_and(|o| o.position[0] < 0)
                        && !self.buy_orphans.iter().any(|p| p.object == purchase.object)
                    {
                        self.buy_orphans.push(purchase.clone());
                    }
                }
            }
        }
    }

    /// Undoes the most recent committed Buy Mode purchase (`0x4cf2e0` / event
    /// `0xec` → `0x4932fa` → `0x48bd21`): runs OBJf 12 (`USER_PICKUP`), takes
    /// the purchased object off-lot while keeping it registered for Redo
    /// (`0x435751` / `0x422ff5`), refunds its price to household funds and
    /// today's budget category, and pushes it onto the redo stack. Returns
    /// `Ok(false)` when nothing can be undone (clearing the undo stack if the
    /// top record could not be applied, matching `0x4932fa` → `0x493376`).
    pub fn buy_undo(&mut self) -> Result<bool, OrderError> {
        self.ready()?;
        if self.buy_undo.is_empty() {
            return Ok(false);
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let after_snap =
            if let Some(active) = self.active_buy_cursor.as_ref().filter(|a| a.existing) {
                let live = self.strip_detached_origin_children(&active.cursor);
                interpreter
                    .buy_cancel(&mut world, &live)
                    .map_err(OrderError::Refused)?;
                Some(world.clone())
            } else {
                None
            };
        let snapped_existing = after_snap.is_some();
        let mut purchase = self.buy_undo.pop().expect("checked non-empty");
        match interpreter.buy_undo_purchase(&mut world, &mut purchase) {
            Ok(true) => {
                let mut affected = BTreeSet::new();
                crate::buy::collect_buy_ids(std::slice::from_ref(&purchase), &mut affected);
                let clear = snapped_existing
                    || self.reconcile_active_buy_cursor(&interpreter, &mut world, Some(&affected));
                self.buy_redo.push(purchase);
                if let Some(pos) = self.undo_order.iter().rposition(|&k| k == HistoryKind::Buy) {
                    self.undo_order.remove(pos);
                }
                self.redo_order.push(HistoryKind::Buy);
                if clear {
                    self.active_buy_cursor = None;
                }
                self.world = world;
                Ok(true)
            }
            Ok(false) => {
                if let Some(snapped) = after_snap {
                    self.world = snapped;
                    self.active_buy_cursor = None;
                }
                let mut dropped = std::mem::take(&mut self.buy_undo);
                dropped.push(purchase);
                self.undo_order.retain(|&k| k != HistoryKind::Buy);
                let mut keep = BTreeSet::new();
                crate::buy::collect_buy_ids(&self.buy_redo, &mut keep);
                for entry in self.build_undo.iter().chain(self.build_redo.iter()) {
                    crate::buy::collect_buy_ids(entry.detached_objects(), &mut keep);
                }
                self.purge_buy_history(&dropped, &keep);
                self.sync_active_buy_cursor();
                Ok(false)
            }
            Err(fault) => {
                if let Some(snapped) = after_snap {
                    self.world = snapped;
                    self.active_buy_cursor = None;
                }
                let mut dropped = std::mem::take(&mut self.buy_undo);
                dropped.push(purchase);
                self.undo_order.retain(|&k| k != HistoryKind::Buy);
                let mut keep = BTreeSet::new();
                crate::buy::collect_buy_ids(&self.buy_redo, &mut keep);
                for entry in self.build_undo.iter().chain(self.build_redo.iter()) {
                    crate::buy::collect_buy_ids(entry.detached_objects(), &mut keep);
                }
                self.purge_buy_history(&dropped, &keep);
                self.sync_active_buy_cursor();
                Err(OrderError::Refused(fault))
            }
        }
    }

    /// Redoes the most recently undone Buy Mode purchase (`0x4cf317` / event
    /// `0xed` → `0x493338` → `0x48bdfe`): reuses the detached registered
    /// instance (`0x48c07b..0x48c082`), runs `USER_PICKUP` (`0x48bfcb`),
    /// places it, runs `USER_PLACEMENT`, re-deducts its price, and pushes it
    /// back onto the undo stack. When placement is refused (`Ok(false)`),
    /// transfers the refused record to Undo while preserving remaining Redo
    /// entries (`0x48be72` → `0x493353`); on a VM fault (`Err`), clears the
    /// redo stack and deletes detached redo instances (`0x4933a9` → `0x48c20e`).
    pub fn buy_redo(&mut self) -> Result<bool, OrderError> {
        self.ready()?;
        if self.buy_redo.is_empty() {
            return Ok(false);
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let after_snap =
            if let Some(active) = self.active_buy_cursor.as_ref().filter(|a| a.existing) {
                let live = self.strip_detached_origin_children(&active.cursor);
                interpreter
                    .buy_cancel(&mut world, &live)
                    .map_err(OrderError::Refused)?;
                Some(world.clone())
            } else {
                None
            };
        let snapped_existing = after_snap.is_some();
        let mut purchase = self.buy_redo.pop().expect("checked non-empty");
        match interpreter.buy_redo_purchase(&mut world, &mut purchase) {
            Ok(true) => {
                let mut affected = BTreeSet::new();
                crate::buy::collect_buy_ids(std::slice::from_ref(&purchase), &mut affected);
                let clear = snapped_existing
                    || self.reconcile_active_buy_cursor(&interpreter, &mut world, Some(&affected));
                self.buy_undo.push(purchase);
                if let Some(pos) = self.redo_order.iter().rposition(|&k| k == HistoryKind::Buy) {
                    self.redo_order.remove(pos);
                }
                self.undo_order.push(HistoryKind::Buy);
                if clear {
                    self.active_buy_cursor = None;
                }
                self.world = world;
                Ok(true)
            }
            Ok(false) => {
                // For a refused `Move` Redo (destination blocked), do NOT
                // publish the detached intermediate world: keep the owned
                // object safely on-lot at `before` so leaving Buy Mode never
                // deletes paid furniture. For a refused `Buy` Redo (`0x48be72`
                // -> `0x493353`), publish the world (which ran `0x48c0ea`) and
                // transfer the record to Undo while leaving remaining Redo
                // entries intact.
                let is_move = purchase.kind == crate::BuyActionKind::Move;
                if !is_move {
                    let clear = snapped_existing
                        || self.reconcile_active_buy_cursor(&interpreter, &mut world, None);
                    self.buy_undo.push(purchase);
                    if clear {
                        self.active_buy_cursor = None;
                    }
                    self.world = world;
                } else {
                    if let Some(snapped) = after_snap {
                        self.world = snapped;
                        self.active_buy_cursor = None;
                    }
                    self.buy_undo.push(purchase);
                    self.sync_active_buy_cursor();
                }
                if let Some(pos) = self.redo_order.iter().rposition(|&k| k == HistoryKind::Buy) {
                    self.redo_order.remove(pos);
                }
                self.undo_order.push(HistoryKind::Buy);
                Ok(false)
            }
            Err(fault) => {
                if let Some(snapped) = after_snap {
                    self.world = snapped;
                    self.active_buy_cursor = None;
                }
                let mut dropped = std::mem::take(&mut self.buy_redo);
                dropped.push(purchase);
                self.redo_order.retain(|&k| k != HistoryKind::Buy);
                let mut keep = BTreeSet::new();
                crate::buy::collect_buy_ids(&self.buy_undo, &mut keep);
                for entry in self.build_undo.iter().chain(self.build_redo.iter()) {
                    crate::buy::collect_buy_ids(entry.detached_objects(), &mut keep);
                }
                self.purge_buy_history(&dropped, &keep);
                self.sync_active_buy_cursor();
                Err(OrderError::Refused(fault))
            }
        }
    }

    /// Cancels an active Buy Mode cursor (`0x487191` / `0x487105`): deletes
    /// both `"Tile Cursor"` and an unpaid catalog object, or deletes `"Tile
    /// Cursor"` and snaps a picked-up existing object back to its pre-pickup
    /// placement (clearing the redo stack per `0x487191` → `0x4933a9`). On
    /// error, the lot is left unchanged so the caller still owns `cursor`.
    pub fn buy_cancel(&mut self, cursor: &BuyCursor) -> Result<(), OrderError> {
        self.ready()?;
        if !self.is_active_buy_cursor(cursor) {
            return Ok(());
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let live = self.strip_detached_origin_children(cursor);
        interpreter
            .buy_cancel(&mut world, &live)
            .map_err(OrderError::Refused)?;
        if cursor.is_existing() {
            let mut keep = BTreeSet::new();
            crate::buy::collect_buy_ids(&self.buy_undo, &mut keep);
            keep.insert(interpreter.lead_object(&world, cursor.held));
            interpreter
                .buy_purge_detached(&mut world, &self.buy_redo, &keep)
                .map_err(OrderError::Refused)?;
            let mut build_redo_detached = Vec::new();
            for entry in &self.build_redo {
                build_redo_detached.extend_from_slice(entry.detached_objects());
            }
            interpreter
                .buy_purge_detached(&mut world, &build_redo_detached, &keep)
                .map_err(OrderError::Refused)?;
        }
        self.world = world;
        if cursor.is_existing() {
            self.buy_redo.clear();
            self.build_redo.clear();
            self.redo_order.clear();
        }
        self.active_buy_cursor = None;
        Ok(())
    }

    /// Snaps a picked-up existing Buy Mode object back to its pre-pickup
    /// placement and deletes `"Tile Cursor"` WITHOUT clearing `self.buy_redo`,
    /// used before executing Undo/Redo (`Game::apply_buy_history`) so the Hand
    /// Tool never shares ownership with an Undo/Redo record.
    pub fn buy_snap_back(&mut self, cursor: &BuyCursor) -> Result<(), OrderError> {
        self.ready()?;
        if !self.is_active_buy_cursor(cursor) {
            return Ok(());
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            move_objects: self.move_objects,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let live = self.strip_detached_origin_children(cursor);
        interpreter
            .buy_cancel(&mut world, &live)
            .map_err(OrderError::Refused)?;
        self.world = world;
        self.active_buy_cursor = None;
        Ok(())
    }

    /// Whether the Build Mode `"Wall Cursor"` handle `cursor` is currently live
    /// on this lot (matching both its monotonic token and its registered object ID).
    fn is_active_wall_cursor(&self, cursor: &crate::WallCursor) -> bool {
        cursor.token != 0
            && self
                .active_wall_cursor
                .as_ref()
                .is_some_and(|a| a.token == cursor.token && a.id == cursor.id)
    }

    /// Opens the Build Mode `"Wall Cursor"` (`0x00000439`, off-lot) with
    /// `data[0] = mode` (`0` = wall build, `1` = wall demolish, `2` = wallpaper,
    /// `3` = wall room-box), closing any previously active wall cursor first
    /// (`0x48b68b`, `0x48a66a`).
    pub fn build_cursor_open(&mut self, mode: i16) -> Result<crate::WallCursor, OrderError> {
        self.ready()?;
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        if let Some(old) = self.active_wall_cursor.as_ref() {
            interpreter
                .build_cursor_close(&mut world, old)
                .map_err(OrderError::Refused)?;
        }
        let mut cursor = interpreter
            .build_cursor_open(&mut world, mode)
            .map_err(OrderError::Refused)?;
        cursor.token = alloc_buy_cursor_token();
        self.world = world;
        self.active_wall_cursor = Some(cursor.clone());
        Ok(cursor)
    }

    /// Moves/updates the active Build Mode `"Wall Cursor"` on the lot
    /// (`0x48b797`, `0x48a7e9`).
    pub fn build_cursor_move(
        &mut self,
        cursor: &mut crate::WallCursor,
        fine: [i32; 3],
        facing: Option<i16>,
        mode: Option<i16>,
    ) -> Result<(), OrderError> {
        self.ready()?;
        if !self.is_active_wall_cursor(cursor) {
            return Ok(());
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let mut next = cursor.clone();
        interpreter
            .build_cursor_move(&mut world, &mut next, fine, facing, mode)
            .map_err(OrderError::Refused)?;
        self.world = world;
        self.active_wall_cursor = Some(next.clone());
        *cursor = next;
        Ok(())
    }

    /// Hides the active Build Mode `"Wall Cursor"` off-lot (`0x48b781`,
    /// `0x48a8ae`).
    pub fn build_cursor_hide(&mut self, cursor: &mut crate::WallCursor) -> Result<(), OrderError> {
        self.ready()?;
        if !self.is_active_wall_cursor(cursor) {
            return Ok(());
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let mut next = cursor.clone();
        interpreter
            .build_cursor_hide(&mut world, &mut next)
            .map_err(OrderError::Refused)?;
        self.world = world;
        self.active_wall_cursor = Some(next.clone());
        *cursor = next;
        Ok(())
    }

    /// Closes and deletes the active Build Mode `"Wall Cursor"` (`0x48a6e5`).
    pub fn build_cursor_close(&mut self, cursor: &crate::WallCursor) -> Result<(), OrderError> {
        self.ready()?;
        if !self.is_active_wall_cursor(cursor) {
            return Ok(());
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        interpreter
            .build_cursor_close(&mut world, cursor)
            .map_err(OrderError::Refused)?;
        self.world = world;
        self.active_wall_cursor = None;
        Ok(())
    }

    /// Closes any active Build Mode `"Wall Cursor"` owned by this lot.
    pub fn build_cursor_close_active(&mut self) -> Result<(), OrderError> {
        if let Some(cur) = self.active_wall_cursor.clone() {
            self.build_cursor_close(&cur)?;
        }
        Ok(())
    }

    /// Commits a planned wall build or demolition to `self.world`, updating
    /// rooms, Architecture funds, and the unified Undo/Redo stack. Refuses
    /// while any Buy Mode cursor is active so a held/hidden existing object
    /// cannot lose its snapback wall and an unpaid preview cannot be sold.
    pub fn build_commit_wall(&mut self, plan: &crate::WallPlan) -> Result<bool, OrderError> {
        self.ready()?;
        if self.active_buy_cursor.is_some() {
            return Ok(false);
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let Some(entry) = interpreter
            .build_commit_wall(&mut world, plan)
            .map_err(OrderError::Refused)?
        else {
            return Ok(false);
        };
        let mut keep = BTreeSet::new();
        crate::buy::collect_buy_ids(&self.buy_undo, &mut keep);
        for e in &self.build_undo {
            crate::buy::collect_buy_ids(e.detached_objects(), &mut keep);
        }
        crate::buy::collect_buy_ids(entry.detached_objects(), &mut keep);
        let mut build_redo_detached = Vec::new();
        for e in &self.build_redo {
            build_redo_detached.extend_from_slice(e.detached_objects());
        }
        interpreter
            .buy_purge_detached(&mut world, &self.buy_redo, &keep)
            .map_err(OrderError::Refused)?;
        interpreter
            .buy_purge_detached(&mut world, &build_redo_detached, &keep)
            .map_err(OrderError::Refused)?;
        let mut affected = BTreeSet::new();
        crate::buy::collect_buy_ids(entry.detached_objects(), &mut affected);
        let clear = self.reconcile_active_buy_cursor(&interpreter, &mut world, Some(&affected));
        if clear {
            self.active_buy_cursor = None;
        }
        self.world = world;
        self.build_undo.push(entry);
        self.build_redo.clear();
        self.buy_redo.clear();
        self.undo_order.push(HistoryKind::Build);
        self.redo_order.clear();
        Ok(true)
    }

    /// Commits a planned floor edit to `self.world`, updating Architecture
    /// funds and the unified Undo/Redo stack.
    pub fn build_commit_floor(&mut self, plan: &crate::FloorPlan) -> Result<bool, OrderError> {
        self.ready()?;
        if self.active_buy_cursor.is_some() {
            return Ok(false);
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let Some(entry) = interpreter
            .build_commit_floor(&mut world, plan)
            .map_err(OrderError::Refused)?
        else {
            return Ok(false);
        };
        let mut keep = BTreeSet::new();
        crate::buy::collect_buy_ids(&self.buy_undo, &mut keep);
        for e in &self.build_undo {
            crate::buy::collect_buy_ids(e.detached_objects(), &mut keep);
        }
        let mut build_redo_detached = Vec::new();
        for e in &self.build_redo {
            build_redo_detached.extend_from_slice(e.detached_objects());
        }
        interpreter
            .buy_purge_detached(&mut world, &self.buy_redo, &keep)
            .map_err(OrderError::Refused)?;
        interpreter
            .buy_purge_detached(&mut world, &build_redo_detached, &keep)
            .map_err(OrderError::Refused)?;
        self.world = world;
        self.build_undo.push(entry);
        self.build_redo.clear();
        self.buy_redo.clear();
        self.undo_order.push(HistoryKind::Build);
        self.redo_order.clear();
        Ok(true)
    }

    /// Commits a planned wallpaper edit to `self.world`, updating Architecture
    /// funds and the unified Undo/Redo stack.
    pub fn build_commit_wallpaper(
        &mut self,
        plan: &crate::WallpaperPlan,
    ) -> Result<bool, OrderError> {
        self.ready()?;
        if self.active_buy_cursor.is_some() {
            return Ok(false);
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            fast: Some(&self.fast),
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let Some(entry) = interpreter
            .build_commit_wallpaper(&mut world, plan)
            .map_err(OrderError::Refused)?
        else {
            return Ok(false);
        };
        let mut keep = BTreeSet::new();
        crate::buy::collect_buy_ids(&self.buy_undo, &mut keep);
        for e in &self.build_undo {
            crate::buy::collect_buy_ids(e.detached_objects(), &mut keep);
        }
        let mut build_redo_detached = Vec::new();
        for e in &self.build_redo {
            build_redo_detached.extend_from_slice(e.detached_objects());
        }
        interpreter
            .buy_purge_detached(&mut world, &self.buy_redo, &keep)
            .map_err(OrderError::Refused)?;
        interpreter
            .buy_purge_detached(&mut world, &build_redo_detached, &keep)
            .map_err(OrderError::Refused)?;
        self.world = world;
        self.build_undo.push(entry);
        self.build_redo.clear();
        self.buy_redo.clear();
        self.undo_order.push(HistoryKind::Build);
        self.redo_order.clear();
        Ok(true)
    }

    /// Whether there is any committed Buy or Build Mode action available to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo_order.is_empty() || !self.buy_undo.is_empty() || !self.build_undo.is_empty()
    }

    /// Whether there is any undone Buy or Build Mode action available to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo_order.is_empty() || !self.buy_redo.is_empty() || !self.build_redo.is_empty()
    }

    /// Undoes the most recent Buy or Build Mode action in chronological order.
    pub fn undo(&mut self) -> Result<bool, OrderError> {
        self.ready()?;
        let kind = match self.undo_order.last().copied() {
            Some(k) => k,
            None if !self.buy_undo.is_empty() => HistoryKind::Buy,
            None if !self.build_undo.is_empty() => HistoryKind::Build,
            None => return Ok(false),
        };
        match kind {
            HistoryKind::Buy => self.buy_undo(),
            HistoryKind::Build => {
                if self.build_undo.is_empty() {
                    self.undo_order.pop();
                    return Ok(false);
                }
                let sounds = RefCell::new(Vec::new());
                let interpreter = Interpreter {
                    sounds: Some(&sounds),
                    view_rotation: self.view_rotation,
                    view_centre: self.view_centre,
                    ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
                };
                let mut world = self.world.clone();
                let after_snap =
                    if let Some(active) = self.active_buy_cursor.as_ref().filter(|a| a.existing) {
                        let live = self.strip_detached_origin_children(&active.cursor);
                        interpreter
                            .buy_cancel(&mut world, &live)
                            .map_err(OrderError::Refused)?;
                        Some(world.clone())
                    } else {
                        None
                    };
                let snapped_existing = after_snap.is_some();
                let mut entry = self.build_undo.pop().expect("checked non-empty");
                match interpreter.build_undo_entry(&mut world, &mut entry) {
                    Ok(true) => {
                        let mut affected = BTreeSet::new();
                        crate::buy::collect_buy_ids(entry.detached_objects(), &mut affected);
                        let clear = snapped_existing
                            || self.reconcile_active_buy_cursor(
                                &interpreter,
                                &mut world,
                                Some(&affected),
                            );
                        if clear {
                            self.active_buy_cursor = None;
                        }
                        self.world = world;
                        self.build_redo.push(entry);
                        if let Some(pos) = self
                            .undo_order
                            .iter()
                            .rposition(|&k| k == HistoryKind::Build)
                        {
                            self.undo_order.remove(pos);
                        }
                        self.redo_order.push(HistoryKind::Build);
                        Ok(true)
                    }
                    Ok(false) => {
                        if let Some(snapped) = after_snap {
                            self.world = snapped;
                            self.active_buy_cursor = None;
                        }
                        self.build_undo.push(entry);
                        Ok(false)
                    }
                    Err(fault) => {
                        if let Some(snapped) = after_snap {
                            self.world = snapped;
                            self.active_buy_cursor = None;
                        }
                        self.build_undo.push(entry);
                        Err(OrderError::Refused(fault))
                    }
                }
            }
        }
    }

    /// Redoes the most recently undone Buy or Build Mode action in chronological
    /// order.
    pub fn redo(&mut self) -> Result<bool, OrderError> {
        self.ready()?;
        let kind = match self.redo_order.last().copied() {
            Some(k) => k,
            None if !self.buy_redo.is_empty() => HistoryKind::Buy,
            None if !self.build_redo.is_empty() => HistoryKind::Build,
            None => return Ok(false),
        };
        match kind {
            HistoryKind::Buy => self.buy_redo(),
            HistoryKind::Build => {
                if self.build_redo.is_empty() {
                    self.redo_order.pop();
                    return Ok(false);
                }
                let sounds = RefCell::new(Vec::new());
                let interpreter = Interpreter {
                    sounds: Some(&sounds),
                    view_rotation: self.view_rotation,
                    view_centre: self.view_centre,
                    ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
                };
                let mut world = self.world.clone();
                let after_snap =
                    if let Some(active) = self.active_buy_cursor.as_ref().filter(|a| a.existing) {
                        let live = self.strip_detached_origin_children(&active.cursor);
                        interpreter
                            .buy_cancel(&mut world, &live)
                            .map_err(OrderError::Refused)?;
                        Some(world.clone())
                    } else {
                        None
                    };
                let snapped_existing = after_snap.is_some();
                let mut entry = self.build_redo.pop().expect("checked non-empty");
                match interpreter.build_redo_entry(&mut world, &mut entry) {
                    Ok(true) => {
                        let mut affected = BTreeSet::new();
                        crate::buy::collect_buy_ids(entry.detached_objects(), &mut affected);
                        let clear = snapped_existing
                            || self.reconcile_active_buy_cursor(
                                &interpreter,
                                &mut world,
                                Some(&affected),
                            );
                        if clear {
                            self.active_buy_cursor = None;
                        }
                        self.world = world;
                        self.build_undo.push(entry);
                        if let Some(pos) = self
                            .redo_order
                            .iter()
                            .rposition(|&k| k == HistoryKind::Build)
                        {
                            self.redo_order.remove(pos);
                        }
                        self.undo_order.push(HistoryKind::Build);
                        Ok(true)
                    }
                    Ok(false) => {
                        if let Some(snapped) = after_snap {
                            self.world = snapped;
                            self.active_buy_cursor = None;
                        }
                        self.build_redo.push(entry);
                        Ok(false)
                    }
                    Err(fault) => {
                        if let Some(snapped) = after_snap {
                            self.world = snapped;
                            self.active_buy_cursor = None;
                        }
                        self.build_redo.push(entry);
                        Err(OrderError::Refused(fault))
                    }
                }
            }
        }
    }

    /// Replace the pending answers with an explicit replay, one entry
    /// per dialog shown, including one-button notices. Supply the bitmap
    /// ID only when the replay calls for a wait-check publication
    /// (0x4764fe). An exhausted replay leaves the next dialog unanswered.
    /// Call between steps, before the lot is waiting for an answer.
    /// The loaders' `&[u8]` lists still answer only dialogs with choices.
    pub fn replay_dialogs(&mut self, answers: &[Answer]) -> Result<(), OrderError> {
        self.ready()?;
        self.player = Answers::replay(answers);
        Ok(())
    }

    /// From now on, a dialog with no answer left in the list waits for
    /// the player (0x436da0). The step returns [`StepError::Asking`] and
    /// the lot stays ready to retry the tick. The window turns this on.
    ///
    /// The tick runs again once [`Lot::answer`] has the answer. A tick
    /// runs the same from the same lot, so it reaches the same dialog,
    /// and the lot then goes on as if the answer had been queued before
    /// the tick.
    pub fn wait_for_answers(&mut self) {
        self.asks = true;
    }

    /// The dialog the lot waits on, if it waits.
    pub fn asking(&self) -> Option<&Asking> {
        self.asking.as_deref()
    }

    /// The special panel (`Dialog` kinds `5..=15`) the lot waits on, if it waits.
    pub fn asking_special_panel(&self) -> Option<&AskingSpecialPanel> {
        self.asking_special_panel.as_deref()
    }

    /// Registry special-panel slots (`cObjectRegistry +0xe8..=+0x10c`).
    pub fn special_panels(&self) -> &SpecialPanels {
        &self.special_panels
    }

    /// Answers the dialog the lot waits on with `answer` (the status its
    /// button gives: 2 the first, 3 the second, 4 the third). The window
    /// can include the activated button's bitmap ID in an [`Answer`].
    /// The next step runs the tick again and gives it this answer; until
    /// then the lot is as it was before that tick and takes no orders.
    pub fn answer(&mut self, answer: impl Into<Answer>) -> Result<(), AnswerError> {
        let asking = self.asking.as_ref().ok_or(AnswerError::NotAsked)?;
        let offered = asking.shown.answers;
        let answer = answer.into();
        if !offered.contains(&answer.status) {
            return Err(AnswerError::NotOffered {
                answer: answer.status,
                offered,
            });
        }
        self.player.push_back_answer(answer);
        self.asking = None;
        self.answered = true;
        Ok(())
    }

    /// Answers the special panel (`Dialog` kinds `5..=15`) the lot waits on.
    /// The next step replays that tick and supplies `response` where the
    /// original modal panel waited (`0x4413e1..0x442320` or `0x4518e2`/`0x451946`).
    pub fn answer_special_panel(
        &mut self,
        response: SpecialPanelResponse,
    ) -> Result<(), SpecialPanelAnswerError> {
        let asking = self
            .asking_special_panel
            .as_ref()
            .ok_or(SpecialPanelAnswerError::NotAsked)?;
        if !asking.shown.request.accepts(&response) {
            return Err(SpecialPanelAnswerError::WrongResponse {
                expected_kind: asking.shown.request.kind(),
                given: response,
            });
        }
        self.special_panel_responses.push_back(response);
        self.asking_special_panel = None;
        self.answered = true;
        Ok(())
    }

    /// Appends scripted responses for upcoming special panels (`Dialog` kinds
    /// `5..=15`), for headless stepping and automated sweeps.
    pub fn queue_special_panel_responses(
        &mut self,
        responses: &[SpecialPanelResponse],
    ) -> Result<(), OrderError> {
        self.ready()?;
        self.special_panel_responses
            .extend(responses.iter().cloned());
        Ok(())
    }

    /// Whether the lot takes a player's input.
    pub(crate) fn ready(&self) -> Result<(), OrderError> {
        if self.halted {
            return Err(OrderError::Halted);
        }
        if self.asking.is_some() || self.asking_special_panel.is_some() || self.answered {
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

    /// The floor and wall catalogs the lot's patterns are keyed by: the
    /// ones the load built, so a reader prices or draws the patterns the
    /// load gave the keys to.
    pub fn surfaces(&self) -> &Surfaces {
        &self.surfaces
    }

    /// The linked code and the catalog, for reading what an object is.
    pub fn program(&self) -> &Program {
        &self.program
    }

    /// Whether the lot is residential (0x40745f), as the idle loop and
    /// the live panel's House button ask.
    pub fn residential(&self) -> bool {
        residential(&self.world, &self.program)
    }

    /// Editable lot name shown in the edit-mode Save Card (`cWinEditLotInfo`,
    /// `0x50e1f8..0x50e42c`), loaded from `*Desc.iff` `STR# (2000 + lot_id)`
    /// and persisted in [`save::Saved`].
    pub fn lot_name(&self) -> &str {
        &self.lot_name
    }

    /// Editable lot description shown in the edit-mode Save Card
    /// (`cWinEditLotInfo`, `0x50e1f8..0x50e42c`), loaded from `*Desc.iff`
    /// `STR# (2000 + lot_id)` and persisted in [`save::Saved`].
    pub fn lot_description(&self) -> &str {
        &self.lot_description
    }

    /// Updates the lot's name and description after accepting the edit-mode
    /// Save Card (`0x50e28f..0x50e42c`), so [`Lot::save`] persists both edits.
    pub fn set_lot_info(&mut self, name: String, description: String) {
        self.lot_name = name;
        self.lot_description = description;
    }

    /// The current lot number (`world.globals[10]`, house `+0x24`).
    pub fn lot_id(&self) -> i16 {
        self.world.globals.get(10).copied().unwrap_or(0)
    }

    /// The current lot's zoning code (`0x42b9e3`: 1 residential, 2 community/
    /// Studio Town, 0 unzoned; lot 0 returns 1).
    pub fn lot_zoning(&self) -> Option<i32> {
        let lot = self.lot_id();
        if lot == 0 {
            Some(1)
        } else {
            self.program.zoning.get(&lot).copied()
        }
    }

    /// Updates lot global 30 (`Free Will`, `0x432220..0x432242` ->
    /// `0x46d9c4(0x1e, al)`). Unlike gameplay orders, Free Will is a direct
    /// preference write that also applies while waiting on or replaying an
    /// answered dialog snapshot.
    pub fn set_free_will(&mut self, enabled: bool) -> Result<(), OrderError> {
        if self.halted {
            return Err(OrderError::Halted);
        }
        if self.load.finished.is_none() {
            return Err(OrderError::Incomplete);
        }
        let value = i16::from(enabled);
        if let Some(slot) = self.world.globals.get_mut(30) {
            *slot = value;
        }
        if let Some(asking) = &mut self.asking
            && let Some(slot) = asking.world.globals.get_mut(30)
        {
            *slot = value;
        }
        if let Some(asking) = &mut self.asking_special_panel
            && let Some(slot) = asking.world.globals.get_mut(30)
        {
            *slot = value;
        }
        Ok(())
    }

    /// Sets lot global 32 (`BUDGET_MODE` / `[lot + 0x50]`, `0x4f1bd9..0x4f1be5`)
    /// so edit-mode lots receive the native §999,999 budget (`0x46d8c9`).
    pub fn set_edit_budget(&mut self, enabled: bool) {
        let value = i16::from(enabled);
        if let Some(slot) = self.world.globals.get_mut(32) {
            *slot = value;
        }
        if let Some(asking) = &mut self.asking
            && let Some(slot) = asking.world.globals.get_mut(32)
        {
            *slot = value;
        }
        if let Some(asking) = &mut self.asking_special_panel
            && let Some(slot) = asking.world.globals.get_mut(32)
        {
            *slot = value;
        }
    }

    /// The text entered in the last answered kind 3 text-prompt dialog
    /// (`0x451a6b..0x451a85` -> global `[0x6e4ac0]`).
    pub fn last_dialog_text(&self) -> &str {
        &self.last_dialog_text
    }

    /// The catalog name assigned to a Make New Character GUID (`0x44b0a8..0x44b0fc`).
    pub fn character_name(&self, guid: u32) -> Option<&str> {
        self.character_names.get(&guid).map(String::as_str)
    }

    /// All runtime catalog names assigned to Make New Character GUIDs
    /// (`0x44b0a8..0x44b0fc`, `0x44b18a..0x44b1cc`), for HUD face/name sync.
    pub fn character_names(&self) -> &BTreeMap<u32, String> {
        &self.character_names
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// The house view's rotation, 0 to 3 (the scene at 0x6e5528, +0x84),
    /// which Generic Sims Call routine 12 reads (0x44fdb8, 0x434f58).
    pub fn view_rotation(&self) -> u8 {
        self.view_rotation
    }

    /// The house file's saved `HOUS 0` camera direction, 0 to 3
    /// ([`opening_rotation`]), which pairs with level 1's stored road-dash
    /// floor IDs (`10` and `11`, `0x4a861f`, `0x4a87b1`–`0x4a87dc`). Unlike
    /// [`Lot::view_rotation`], this does not change when the camera turns
    /// because the lot's floor array is not mutated on rotation.
    pub fn floor_rotation(&self) -> u8 {
        self.floor_rotation
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

    /// The tile at the house view's centre, if known
    /// ([`Lot::set_view_centre`]).
    pub fn view_centre(&self) -> Option<[i8; 2]> {
        self.view_centre
    }

    /// Sets the view-rotated tile pick at the house view's centre, `None`
    /// for not known: the pick a dialog's portrait resolves into its search
    /// start (`0x4e96dd`, `0x4347f0`, `0x4a677c`, `0x4a7b58`, bounds-checked
    /// at `0x434891` and un-rotated at `0x43491c`;
    /// [`crate::Interpreter::view_centre`]). Because the simulation reads it,
    /// it is an input like [`Lot::set_view_rotation`]: the view that shows
    /// the lot sets it whenever its camera moves, and a recording of play
    /// must note each change. Where it is not known and the tile searched
    /// from would change what the portrait does, the step fails as an
    /// unported branch does. [`Lot::load`] leaves it not known (which camera
    /// the original opens a house with is not traced); [`Lot::load_viewed`]
    /// takes it.
    pub fn set_view_centre(&mut self, centre: Option<[i8; 2]>) {
        self.view_centre = centre;
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
        let loaded = world.hash();
        let program = Program {
            global: 0,
            libraries: BTreeMap::new(),
            semi_globals: BTreeMap::new(),
            careers: None,
            career_texts_end: false,
            labels_hold_no_token: false,
            zoning: BTreeMap::new(),
            animations: Default::default(),
            people: Default::default(),
            icon_frames: None,
            wardrobe: None,
            ordered_wardrobe: None,
            character_files: Default::default(),
            expansions: 0,
            characters: Default::default(),
        };
        let catalog = std::sync::Arc::new(Catalog {
            global: 0,
            objects: Vec::new(),
            by_guid: BTreeMap::new(),
            semi_globals: BTreeMap::new(),
            diagnostics: Vec::new(),
        });
        let fast = crate::program::ProgramFast::build(&program, &catalog);
        Lot {
            program,
            fast,
            catalog,
            surfaces: sims_content::Surfaces::default(),
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
            asking_special_panel: None,
            special_panels: crate::SpecialPanels::default(),
            special_panel_responses: std::collections::VecDeque::new(),

            view_rotation: 0,
            floor_rotation: 0,
            view_centre: None,
            replay_view: None,
            last_dialog_text: String::new(),
            character_names: BTreeMap::new(),
            default_character_name: String::new(),
            lot_name: String::new(),
            lot_description: String::new(),
            loaded,
            buy_undo: Vec::new(),
            buy_redo: Vec::new(),
            buy_orphans: Vec::new(),
            active_buy_cursor: None,
            build_undo: Vec::new(),
            build_redo: Vec::new(),
            undo_order: Vec::new(),
            redo_order: Vec::new(),
            active_wall_cursor: None,
            neighborhood_origin: None,
            coop: None,
            move_objects: false,
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
        lot.step().expect("empty residential lot steps cleanly");
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
        // Truncated globals stop the lot update outside any object's tick.
        let mut world = World::empty();
        world.globals.truncate(15);
        let mut lot = lot(world, true, Frames::default());
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
        let mut world = World::empty();
        world.globals.truncate(15);
        let mut lot = lot(world, true, Frames::default());
        lot.tuning = Some(tuning());
        let Err(StepError::Tick {
            tick: 1,
            update: Some(update),
            ..
        }) = lot.step_play()
        else {
            panic!("the tick should fail, with what came before it");
        };
        // Nothing was caught or skipped before the clock failed.
        assert!(update.errors.is_empty() && update.frame_errors.is_empty());
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

    #[test]
    fn restore_rejects_a_snapshot_with_a_different_floor_rotation_baseline() {
        let mut source = lot(World::empty(), true, Frames::default());
        source.floor_rotation = 0;
        source.set_view_rotation(1);
        let saved = source.save().unwrap();
        let bytes = saved.encode();

        // A lot loaded with a different saved HOUS 0 floor rotation rejects
        // the snapshot even when the loaded world hash matches.
        let mut mismatched = lot(World::empty(), true, Frames::default());
        mismatched.floor_rotation = 1;
        assert_eq!(
            mismatched.restore(super::save::Saved::decode(&bytes).unwrap()),
            Err(super::save::SaveError::OtherLoad)
        );

        // A lot with the matching floor rotation restores cleanly, keeping
        // the snapshot's view_rotation and its own floor_rotation.
        let mut matching = lot(World::empty(), true, Frames::default());
        matching.floor_rotation = 0;
        matching
            .restore(super::save::Saved::decode(&bytes).unwrap())
            .unwrap();
        assert_eq!(matching.floor_rotation(), 0);
        assert_eq!(matching.view_rotation(), 1);
    }

    #[test]
    fn a_dialog_tick_replays_with_its_frozen_view_even_when_the_view_affects_the_tick() {
        use sims_formats::behavior::{Behavior, Instruction};
        use sims_formats::saved_objects::{Frame, Interaction, Person, Slot};
        use sims_sim::Object;

        const GUID: u32 = 0x1111;
        const SIM_GUID: u32 = 0x2222;
        let mut def = [0i16; crate::DEFINITION_WORDS];
        def[9] = 4; // non-person object
        def[14] = GUID as u16 as i16;
        def[15] = (GUID >> 16) as u16 as i16;

        // Node 0: Generic Sims Call 12 (tile_depth: reads view_rotation into temp 0).
        // Node 1: two-button Dialog (yields and asks the player on tick 1).
        // Node 2: Generic Sims Call 12 again on tick 2 (reads the moved view_rotation).
        let behavior = Behavior {
            format: 0x8002,
            kind: 0,
            args: 0,
            locals: 0,
            revision: 0,
            instructions: vec![
                Instruction {
                    opcode: 1,
                    on_true: 1,
                    on_false: 253,
                    operands: [12, 0, 0, 0, 0, 0, 0, 0],
                },
                Instruction {
                    opcode: 36,
                    on_true: 2,
                    on_false: 2,
                    operands: [0, 0, 0, 0, 0, 1, 0, 0],
                },
                Instruction {
                    opcode: 1,
                    on_true: 3,
                    on_false: 253,
                    operands: [12, 0, 0, 0, 0, 0, 0, 0],
                },
                Instruction {
                    opcode: 0,
                    on_true: 3,
                    on_false: 253,
                    operands: [0; 8],
                },
            ],
        };
        let mut library = crate::Library::default();
        library.definitions.insert(128, def);
        library.definition_order.push(128);
        library.behaviors.insert(4096, behavior);

        let interaction = Interaction {
            join: 0,
            uid: 0,
            caller: 0,
            target: 0,
            icon: 0,
            table_index: 0,
            args: [0; 4],
            priority: 0,
            action: 0,
            attenuation: 0.0,
            flags: 0,
        };
        let mut world = World::empty();
        world.size = 64;
        world.globals[3] = 2;
        world.order = vec![1, 2];
        world.people = vec![2];
        world.types.insert(1, GUID);
        world.types.insert(2, SIM_GUID);
        world.objects.insert(
            1,
            Object {
                id: 1,
                guid: GUID,
                footprint: [0; 4],
                position: [10 << 4, 20 << 4, 0],
                unknown_short: 0,
                attributes: Vec::new(),
                temps: vec![0; 8],
                data: vec![0; 72],
                obsolete_word: 0,
                stack_capacity_bytes: 0,
                stack: vec![Frame {
                    stack_object: 1,
                    tree: 4096,
                    node: 0,
                    parameters: vec![1, 0, 0, 0],
                    locals: Vec::new(),
                    primitive_state: 0,
                    code_owner_type: 1,
                }],
                relationship_flag: -1,
                relationships: Vec::new(),
                slots: vec![Slot {
                    unknown: 0,
                    object: 0,
                }],
                dynamic_sprites: Vec::new(),
                multipart: None,
                portal: None,
                person: None,
                registry_word: Some((1 << 16) | 0xffff),
                last_result: false,
                removal_skips_resets: false,
                selected: false,
            },
        );
        world.objects.insert(
            2,
            Object {
                id: 2,
                guid: SIM_GUID,
                footprint: [0; 4],
                position: [0; 3],
                unknown_short: 0,
                attributes: Vec::new(),
                temps: vec![0; 8],
                data: vec![0; 72],
                obsolete_word: 0,
                stack_capacity_bytes: 0,
                stack: Vec::new(),
                relationship_flag: -1,
                relationships: Vec::new(),
                slots: vec![Slot {
                    unknown: 0,
                    object: 0,
                }],
                dynamic_sprites: Vec::new(),
                multipart: None,
                portal: None,
                person: Some(Person {
                    animation_events: 0,
                    engaged: 0,
                    appearance: Default::default(),
                    accessories: Vec::new(),
                    animations: Default::default(),
                    routing_state: 0,
                    floats: [0.0; 9],
                    previous_motives: [0.0; 16],
                    motives: [0.0; 16],
                    data: vec![0; 256],
                    routing_frames: 0,
                    active: interaction.clone(),
                    last: interaction,
                    queue: Vec::new(),
                    object_uses: Vec::new(),
                    motive_deltas: Vec::new(),
                }),
                registry_word: None,
                last_result: false,
                removal_skips_resets: false,
                selected: true,
            },
        );
        let make_lot = || {
            let mut l = lot(world.clone(), true, Frames::parse("none").unwrap());
            l.tuning = Some(tuning());
            l.program.labels_hold_no_token = true;
            l.program.libraries.insert(0, crate::Library::default());
            l.program.libraries.insert(1, library.clone());
            l.program.semi_globals.insert(1, None);
            let cat = std::sync::Arc::make_mut(&mut l.catalog);
            cat.objects.push(sims_content::Object {
                guid: GUID,
                resource: 1,
                chunk: 128,
                kind: 4,
                master: 0,
            });
            cat.by_guid.insert(GUID, 0);
            cat.semi_globals.insert(1, None);
            l
        };

        let mut asked = make_lot();
        asked.wait_for_answers();
        asked.set_view_rotation(0);
        asked.set_view_centre(Some([10, 12]));
        // Tick 1 pauses on the dialog and freezes (0, Some([10, 12])).
        assert_eq!(asked.step().unwrap_err(), StepError::Asking { tick: 1 });
        // Moving the camera while waiting updates the live view, not the frozen replay view.
        asked.set_view_rotation(2);
        asked.set_view_centre(Some([40, 42]));
        asked.answer(2).unwrap();
        let bytes = asked.save().unwrap().encode();

        let mut restored = make_lot();
        restored.wait_for_answers();
        restored
            .restore(super::save::Saved::decode(&bytes).unwrap())
            .unwrap();
        assert_eq!(restored.view_rotation(), 2);
        assert_eq!(restored.view_centre(), Some([40, 42]));
        // Replaying tick 1 must use rotation 0: -(10 + 20) = -30, not rotation 2: -(53 + 43) = -96.
        restored.step().unwrap();
        assert_eq!(restored.world().objects[&1].temps[0], -30);
        // After the dialog tick finishes, tick 2 uses the moved rotation 2: -(53 + 43) = -96.
        restored.step().unwrap();
        assert_eq!(restored.world().objects[&1].temps[0], -96);
    }

    #[test]
    fn a_dialog_tick_replays_with_its_frozen_view_centre_across_save_and_restore() {
        use sims_content::{Profile, Skeleton};
        use sims_formats::behavior::{Behavior, Instruction};
        use sims_formats::execution::ObjectFunction;
        use sims_formats::lot::Array;
        use sims_formats::saved_objects::{Frame, Interaction, Person, Slot};
        use sims_formats::tuning::{Constant, Constants};
        use sims_sim::Object;
        use sims_sim::neighbors::Neighbor;

        const LAMP_GUID: u32 = 0x1111;
        const SIM_GUID: u32 = 0x2222;
        const PERSON_A: u32 = 0x3333_0001;
        const PERSON_B: u32 = 0x3333_0002;

        let array = |stride: usize, fill: u8| Array {
            width: 64,
            height: 64,
            stride,
            encoding: 0,
            data: vec![fill; 64 * 64 * stride],
        };
        let assign = |left_scope: u8, left: i16, right_scope: u8, right: i16, next: u8| {
            let [l0, l1] = left.to_le_bytes();
            let [r0, r1] = right.to_le_bytes();
            Instruction {
                opcode: 2,
                on_true: next,
                on_false: next,
                operands: [l0, l1, r0, r1, 0, 5, left_scope, right_scope],
            }
        };
        let person_def = |guid: u32| {
            let mut def = [0i16; crate::DEFINITION_WORDS];
            def[9] = 2;
            def[14] = guid as u16 as i16;
            def[15] = (guid >> 16) as u16 as i16;
            def
        };
        let mut lamp_def = [0i16; crate::DEFINITION_WORDS];
        lamp_def[9] = 4;
        lamp_def[14] = LAMP_GUID as u16 as i16;
        lamp_def[15] = (LAMP_GUID >> 16) as u16 as i16;

        // Object 1's script:
        // Node 0..1: show two-button dialog with neighbor 1's portrait (PERSON_A).
        // Node 2..3: on the next tick, show two-button dialog with neighbor 2's portrait (PERSON_B).
        let lamp_bhav = Behavior {
            format: 0x8002,
            kind: 0,
            args: 0,
            locals: 0,
            revision: 0,
            instructions: vec![
                assign(10, 0, 7, 1, 1), // Stack Object := 1 (neighbor 1 = PERSON_A)
                Instruction {
                    opcode: 36,
                    on_true: 2,
                    on_false: 2,
                    operands: [0, 0, 0, 0, 0, 1, 0, 2 << 1],
                },
                assign(10, 0, 7, 2, 3), // Stack Object := 2 (neighbor 2 = PERSON_B)
                Instruction {
                    opcode: 36,
                    on_true: 4,
                    on_false: 4,
                    operands: [0, 0, 0, 0, 0, 1, 0, 2 << 1],
                },
                Instruction {
                    opcode: 0,
                    on_true: 4,
                    on_false: 253,
                    operands: [0; 8],
                },
            ],
        };
        // Person Init: allow ground level (data[4] = 1) and no-floor placement (data[42] = 2).
        let person_init = Behavior {
            format: 0x8002,
            kind: 0,
            args: 0,
            locals: 0,
            revision: 0,
            instructions: vec![assign(3, 4, 7, 1, 1), assign(3, 42, 7, 2, 254)],
        };

        let mut global = crate::Library::default();
        for id in [1, 5, 6] {
            global.tuning.insert(
                id,
                Constants {
                    version: 0,
                    entries: Vec::<Constant>::new(),
                },
            );
        }
        for id in [502, 504] {
            global
                .strings
                .insert(id, vec!["(-100;1)(100;1)".to_owned(); 7]);
        }

        let mut lamp_lib = crate::Library::default();
        lamp_lib.definitions.insert(128, lamp_def);
        lamp_lib.definition_order.push(128);
        lamp_lib.behaviors.insert(4096, lamp_bhav);

        let make_person_lib = |guid: u32| {
            let mut lib = crate::Library {
                writable: true,
                ..Default::default()
            };
            lib.definitions.insert(128, person_def(guid));
            lib.definition_order.push(128);
            lib.behaviors.insert(4096, person_init.clone());
            lib.behaviors.insert(
                4097,
                Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 1,
                    locals: 0,
                    revision: 0,
                    instructions: vec![
                        assign(9, 0, 7, 10, 1),
                        Instruction {
                            opcode: 0,
                            on_true: 0,
                            on_false: 253,
                            operands: [0; 8],
                        },
                    ],
                },
            );
            lib.callbacks.insert(
                128,
                vec![
                    ObjectFunction {
                        condition: 0,
                        action: 4096,
                    },
                    ObjectFunction {
                        condition: 0,
                        action: 4097,
                    },
                ],
            );
            lib
        };

        let interaction = Interaction {
            join: 0,
            uid: 0,
            caller: 0,
            target: 0,
            icon: 0,
            table_index: 0,
            args: [0; 4],
            priority: 0,
            action: 0,
            attenuation: 0.0,
            flags: 0,
        };
        let mut world = World::empty();
        world.size = 6;
        world.globals[3] = 2;
        world.globals[9] = 1;
        world.families.push(sims_sim::neighbors::Family {
            id: 1,
            house: 0,
            members: vec![SIM_GUID],
            friends: 0,
            stale: false,
        });
        world.indicators = [1, 2];
        world.order = vec![1, 2];
        world.people = vec![2];
        world.types.insert(1, LAMP_GUID);
        world.types.insert(2, SIM_GUID);
        world.types.insert(3, PERSON_A);
        world.types.insert(4, PERSON_B);
        world.arrays.insert(0, array(4, 0));
        for (floors, walls, flags) in [(1, 2, 8), (101, 102, 108)] {
            world.arrays.insert(floors, array(2, 0));
            world.arrays.insert(walls, array(14, 0));
            world.arrays.insert(flags, array(1, 0));
        }
        world.arrays.insert(3, array(2, 0));
        world.arrays.insert(103, array(2, 0));
        world.arrays.insert(9, array(1, 0xff));
        world.arrays.insert(10, array(1, 0xff));
        // Room 1 everywhere, except tile (2, 2) is isolated in room 9 and occupied by object 1.
        // Searching from [1, 1] succeeds in room 1; searching from [2, 2] finds no tile in room 9.
        world.rooms.map[0] = vec![1; 64 * 64];
        world.rooms.map[1] = vec![1024; 64 * 64];
        world.rooms.map[0][2 * 64 + 2] = 9;
        let grid = &mut world.arrays.get_mut(&3).unwrap().data;
        grid[(2 * 64 + 2) * 2..(2 * 64 + 2) * 2 + 2].copy_from_slice(&1i16.to_le_bytes());

        world.neighbors.slots = vec![
            Some(Neighbor {
                id: 1,
                guid: PERSON_A,
                revision: 7,
                data: vec![12; 256],
                relationships: Vec::new(),
                friends: 0,
                stale: false,
            }),
            Some(Neighbor {
                id: 2,
                guid: PERSON_B,
                revision: 7,
                data: vec![12; 256],
                relationships: Vec::new(),
                friends: 0,
                stale: false,
            }),
        ];
        world.objects.insert(
            1,
            Object {
                id: 1,
                guid: LAMP_GUID,
                footprint: [0; 4],
                position: [2 << 4, 2 << 4, 0],
                unknown_short: 0,
                attributes: Vec::new(),
                temps: vec![0; 8],
                data: vec![0; 72],
                obsolete_word: 0,
                stack_capacity_bytes: 0,
                stack: vec![Frame {
                    stack_object: 1,
                    tree: 4096,
                    node: 0,
                    parameters: vec![1, 0, 0, 0],
                    locals: Vec::new(),
                    primitive_state: 0,
                    code_owner_type: 1,
                }],
                relationship_flag: -1,
                relationships: Vec::new(),
                slots: vec![Slot {
                    unknown: 0,
                    object: 0,
                }],
                dynamic_sprites: Vec::new(),
                multipart: None,
                portal: None,
                person: None,
                registry_word: Some((1 << 16) | 0xffff),
                last_result: false,
                removal_skips_resets: false,
                selected: false,
            },
        );
        world.objects.insert(
            2,
            Object {
                id: 2,
                guid: SIM_GUID,
                footprint: [0; 4],
                position: [0; 3],
                unknown_short: 0,
                attributes: Vec::new(),
                temps: vec![0; 8],
                data: vec![0; 72],
                obsolete_word: 0,
                stack_capacity_bytes: 0,
                stack: Vec::new(),
                relationship_flag: -1,
                relationships: Vec::new(),
                slots: vec![Slot {
                    unknown: 0,
                    object: 0,
                }],
                dynamic_sprites: Vec::new(),
                multipart: None,
                portal: None,
                person: Some(Person {
                    animation_events: 0,
                    engaged: 0,
                    appearance: Default::default(),
                    accessories: Vec::new(),
                    animations: Default::default(),
                    routing_state: 0,
                    floats: [0.0; 9],
                    previous_motives: [0.0; 16],
                    motives: [0.0; 16],
                    data: vec![0; 256],
                    routing_frames: 0,
                    active: interaction.clone(),
                    last: interaction,
                    queue: Vec::new(),
                    object_uses: Vec::new(),
                    motive_deltas: Vec::new(),
                }),
                registry_word: None,
                last_result: false,
                removal_skips_resets: false,
                selected: true,
            },
        );

        let make_lot = || {
            let mut l = lot(world.clone(), true, Frames::parse("none").unwrap());
            l.tuning = Some(tuning());
            l.program.labels_hold_no_token = true;
            l.program.libraries.insert(0, global.clone());
            l.program.libraries.insert(1, lamp_lib.clone());
            l.program.libraries.insert(2, make_person_lib(PERSON_A));
            l.program.libraries.insert(3, make_person_lib(PERSON_B));
            let cat = std::sync::Arc::make_mut(&mut l.catalog);
            for res in [1, 2, 3] {
                l.program.semi_globals.insert(res, None);
                cat.semi_globals.insert(res, None);
            }
            for (idx, (guid, res, kind)) in [(LAMP_GUID, 1, 4), (PERSON_A, 2, 2), (PERSON_B, 3, 2)]
                .into_iter()
                .enumerate()
            {
                cat.objects.push(sims_content::Object {
                    guid,
                    resource: res,
                    chunk: 128,
                    kind,
                    master: 0,
                });
                cat.by_guid.insert(guid, idx);
            }
            for (guid, name) in [(PERSON_A, "person_a"), (PERSON_B, "person_b")] {
                l.program.people.skeletons.insert(
                    guid,
                    Skeleton {
                        name: name.into(),
                        file: format!("{name}.bcf"),
                        place: 0,
                        scales: vec![vec![1]],
                        bones: Vec::new(),
                    },
                );
                l.program.people.profiles.insert(
                    guid,
                    Profile {
                        gender: None,
                        age: None,
                        skin: None,
                        tones: Vec::new(),
                        rig: Default::default(),
                        body: None,
                        head: None,
                        suit_rows: Default::default(),
                    },
                );
            }
            l
        };

        let mut asked = make_lot();
        asked.wait_for_answers();
        asked.set_view_rotation(0);
        // Start with [1, 1] (free room 1): PERSON_A's portrait search succeeds and
        // tick 1 pauses on the dialog, freezing replay_view = Some((0, Some([1, 1]))).
        asked.set_view_centre(Some([1, 1]));
        assert_eq!(asked.step().unwrap_err(), StepError::Asking { tick: 1 });

        // Move the camera to [2, 2] (blocked room 9) while the dialog waits, then
        // answer and round-trip through save/restore.
        asked.set_view_centre(Some([2, 2]));
        asked.answer(2).unwrap();
        let bytes = asked.save().unwrap().encode();

        let mut restored = make_lot();
        restored.wait_for_answers();
        restored
            .restore(super::save::Saved::decode(&bytes).unwrap())
            .unwrap();
        assert_eq!(restored.view_centre(), Some([2, 2]));
        // Tick 1 replay MUST use frozen [1, 1] so PERSON_A's portrait succeeds
        // (using the moved [2, 2] would hit the no-tile trap).
        restored.step().unwrap();
        assert!(restored.world().portraits.contains(&PERSON_A));
        assert!(restored.world().saved_portraits.contains(&PERSON_A));

        // Tick 2 runs after replay_view is cleared, so PERSON_B's portrait search
        // MUST use the moved live [2, 2], find no tile, remove unput (`0x4e9706`)
        // without caching or saving PERSON_B, and pause on tick 2's dialog.
        assert_eq!(restored.step().unwrap_err(), StepError::Asking { tick: 2 });
        restored.answer(2).unwrap();
        restored.step().unwrap();
        assert!(!restored.world().portraits.contains(&PERSON_B));
        assert!(!restored.world().saved_portraits.contains(&PERSON_B));
    }

    #[test]
    fn synthetic_cheats_add_funds_set_hour_and_move_objects_flag() {
        let mut world = World::empty();
        world.globals[13] = 1000;
        world.globals[14] = 0;
        world.budget.funds = 1000;
        world.globals[0] = 8;
        world.globals[4] = 0;
        world.globals[5] = 42;
        world.globals[6] = 17;
        world.rooms.pending = 0;
        let mut lot = lot(world, true, Frames::default());

        // move_objects defaults to false and toggles cleanly.
        assert!(!lot.move_objects());
        lot.set_move_objects(true);
        assert!(lot.move_objects());
        lot.set_move_objects(false);
        assert!(!lot.move_objects());

        // add_funds updates budget + globals 13/14 and clamps to the exact
        // lossless script-visible range [-327_689_999, 327_679_999].
        assert_eq!(lot.add_funds(500).unwrap(), 1500);
        assert_eq!(lot.world().budget.funds, 1500);
        assert_eq!(lot.world().globals[13], 1500);
        assert_eq!(lot.world().globals[14], 0);
        assert_eq!(lot.add_funds(-2000).unwrap(), -500);
        assert_eq!(lot.world().budget.funds, -500);
        assert_eq!(lot.add_funds(i32::MAX).unwrap(), 327_679_999);
        assert_eq!(
            i32::from(lot.world().globals[14]) * 10_000 + i32::from(lot.world().globals[13]),
            327_679_999
        );
        assert_eq!(lot.add_funds(i32::MIN).unwrap(), -327_689_999);
        assert_eq!(lot.world().globals[14], -32768);
        assert_eq!(lot.world().globals[13], -9999);
        assert_eq!(
            i32::from(lot.world().globals[14]) * 10_000 + i32::from(lot.world().globals[13]),
            -327_689_999
        );

        // Budget mode (global 32 == 1): World::set_funds is a no-op, preserving
        // both budget.funds and globals 13/14.
        lot.world.globals[32] = 1;
        lot.world.budget.funds = 250;
        let g13_before = lot.world().globals[13];
        let g14_before = lot.world().globals[14];
        assert_eq!(lot.add_funds(750).unwrap(), 250);
        assert_eq!(lot.world().budget.funds, 250);
        assert_eq!(lot.world().globals[13], g13_before);
        assert_eq!(lot.world().globals[14], g14_before);

        // set_hour rejects >= 24 and updates globals 0/4/5/6 + marks room lighting on period change.
        // Sims.exe 0x46d525 periods: 0..=5 => 2 (night), 6 => 3 (dawn),
        // 7..=17 => 0 (day), 18 => 1 (dusk), 19..=23 => 2 (night).
        assert!(matches!(lot.set_hour(24), Err(OrderError::Refused(_))));
        assert!(matches!(lot.set_hour(255), Err(OrderError::Refused(_))));
        lot.world.rooms.pending = 0;
        lot.set_hour(18).unwrap();
        assert_eq!(lot.world().globals[0], 18);
        assert_eq!(lot.world().globals[4], 1);
        assert_eq!(lot.world().globals[5], 0);
        assert_eq!(lot.world().globals[6], 0);
        assert_eq!(
            lot.world().rooms.pending,
            3,
            "period change must mark rooms for light recount"
        );
        lot.set_hour(22).unwrap();
        assert_eq!(lot.world().globals[4], 2);
        lot.set_hour(6).unwrap();
        assert_eq!(lot.world().globals[4], 3);
        lot.set_hour(12).unwrap();
        assert_eq!(lot.world().globals[4], 0);
        lot.set_hour(3).unwrap();
        assert_eq!(lot.world().globals[4], 2);

        // Halted lot refuses add_funds and set_hour.
        lot.halted = true;
        assert!(matches!(lot.add_funds(10), Err(OrderError::Halted)));
        assert!(matches!(lot.set_hour(10), Err(OrderError::Halted)));
    }
}
