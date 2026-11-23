//! Ticks: one lot update after another.
//!
//! A tick is one call of the original's lot update (0x46d345):
//!
//! 1. The game's random stream is put aside and the lot's installed
//!    (0x46f447, 0x46f44d): scripts draw from [`World::lot_rng`]. The
//!    scheduler makes a pass over the objects (0x46d553,
//!    [`Interpreter::scheduler_pass`]). Then the lot's stream is saved and
//!    the game's restored: from here scripts draw from
//!    [`World::global_rng`].
//! 2. After a full pass, global 6 (seconds) goes up by 2 and the tick
//!    count by 1. At 60 seconds the seconds go back by 60 and global 5
//!    (minutes) goes up, and signal 0xdb is sent. At 60 minutes the
//!    minutes go back by 60 and global 0 (hours) goes up. At 24 hours the
//!    hours go back by 24 and the day turns (0x46d3e9, [`crate::daily`]:
//!    the date, the objects' daily update, the budget's days), and the
//!    signal is 0xdc instead of 0xdb.
//! 3. The registry's pass at the end of the update (0x4412d1,
//!    [`Interpreter::end_of_update`]): the dialogs, then the queued
//!    removals, then a check that a Sim is selected.
//! 4. The signal from step 2, if any, goes to the game's eight
//!    subscribers (0x40760d). Of those only the interface takes signal
//!    0xdb (0x4f65bf; a sound when the minutes are 0), so it changes
//!    nothing here. On 0xdc the lot moves the usage counts back a day and
//!    works out its statistics again ([`crate::daily`]).
//! 5. If the hour turned, the time of day for the new hour (0x46d525) is
//!    compared with global 4. If it differs, global 4 takes it, the
//!    interface is told (0x4c4648, which picks the day or night look of
//!    the panel of the current mode), and signals 0xf8 (with 5) and 0xdd
//!    are sent. Of the subscribers only the architecture takes 0xf8
//!    (0x47b22a): when global 36 is not 0 it grows the lawn
//!    ([`World::grow_grass`] by 5, 0x47b3c8), and then sends 0x86 and
//!    0x83: the interface's panels (0x4c3ea5) set a refresh flag on 0x86
//!    and the house viewer (0x4a82db) redraws on 0x83. The panels also
//!    take 0xdd (0x4c4189): the current mode's time-of-day look, then
//!    0x83 again. On 0xdd the lot
//!    marks every room as not counted and both levels as waiting
//!    ([`World::mark_all_rooms`]), after setting the brightest a lit
//!    tile may be, 1 by day and 0.8 otherwise (0x466ba3, at 0x6c741c,
//!    which only the drawing's light clamp reads, 0x4b4525). The next
//!    frame draw counts the rooms with the new time of day's daylight.
//!
//! **Not modeled**, being presentation as far as the original's code
//! shows: an object's headlines (the list at object 0x148, stepped after
//! each thread run; see `primitives/headline.rs` for why no headline
//! shows here), the parts of a person's animator
//! that only pose and draw the person (see [`crate::animator`]), the
//! graphics work every tenth tick (0x5a0932), and signal 0xf7, which a person's tick sends when the
//! selected Sim changes room and which only the house viewer takes
//! (0x4a82db, 0x4ab992). That none of these changes simulation state is
//! **unverified**. The lot's subscriber passes each signal to the object
//! at lot 0x10 (0x416393) before its own cases, but by a direct call to
//! the subscribers' empty handler (0x41639e calls 0x407607, which returns
//! 0), so that does nothing. Whether the panels' and the house viewer's
//! handling of these signals (taken as presentation) changes anything the
//! simulation keeps is **unverified**; the house viewer's handler was not
//! read in full. The
//! registry's pass also opens the game's special panels (registry words
//! 0xe8 to 0x108) when a script asked for one; no ported primitive asks,
//! so they never open here. When the pass showed a modal dialog the lot
//! update clears two simulator words (0xb8, 0xbc) that are not modeled.
//!
//! Read from the original's code; the old prototype
//! (legacy/crates/sims-vm/src/lib.rs, legacy/crates/sims-sim/src/clock.rs,
//! legacy/crates/sims-sim/src/removal.rs) was the guide. What a capture of
//! the original running these ticks confirms is in `docs/decisions/`.

use crate::daily;
use crate::fault::{Fault, Trap};
use crate::interpreter::{Interpreter, Status, Stream};
use crate::placement::Refreshing;
use crate::play::{Play, Recovery, ScriptError};
use crate::primitives::dialog::{
    self, CALL_NEIGHBOR_CANCELLED, DONE, InlineSpecialPanel, NEW, SHOWN, SPECIAL_PANEL_ACCEPTED,
    SPECIAL_PANEL_CANCELLED, SpecialPanelRequest, SpecialPanelResponse,
};
use crate::primitives::ui_effect::UiEffectEvent;
use crate::selection::{publish_waited_for, selected};
use crate::sounds::SoundEvent;
use crate::visits::function;
use sims_sim::motives::{self, Tuning};
use sims_sim::rooms::Room;
use sims_sim::{PersonTick, World};
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;

/// How many instructions a thread may run in one tick. The original
/// counts each thread's instructions from 0 at every run (object field 4,
/// 0x474ea1) and raises error 0x44d past the value at 0x6c4d70, which
/// was 458,605 in a memory image of the running game. Here a thread that
/// passes it fails; the original's handling of the error (the object's
/// virtual function 4) is not ported. Whether the value is the same in
/// every run is **unverified**.
pub const TICK_BUDGET: usize = 458_605;

/// Globals the clock uses.
const HOURS: usize = 0;
/// Global 4: the time of day (0 day, 1 dusk, 2 night, 3 dawn).
const TIME_OF_DAY: usize = 4;
const MINUTES: usize = 5;
/// The global that lets the lawn grow when the time of day changes
/// (0x47b2e8: simulator +0x58), and the amount it grows by: signal
/// 0xf8's parameter, which the clock sends as 5.
const GROW_LAWN: usize = 36;
const LAWN_GROWTH: i32 = 5;
const SECONDS: usize = 6;
/// Globals that make the scheduler's pass partial: 17, or 19 and 10 both.
const PAUSED: usize = 17;
const MODE: usize = 19;
const LOT: usize = 10;

/// The time of day for an hour (0x46d525): night before 6, dawn at 6, day
/// from 7 to 17, dusk at 18 and night from 19.
fn time_of_day(hours: i16) -> i16 {
    match hours {
        ..=5 => 2,
        6 => 3,
        7..=17 => 0,
        18 => 1,
        _ => 2,
    }
}

/// Scheduler flags (the registry word's high half).
const READY: u16 = 1;
const IN_PARTIAL: u16 = 2;
const CHECK_ERROR: u16 = 4;

/// Object data words.
const ROOM: usize = 29;
const HIDDEN: usize = 34;

/// The bounds of person word 1's kinds (0x4a69aa): from -20000 up to
/// -10000 the interface plays a sound as it shows the value; below or
/// above, it only shows it.
const STING: i16 = -20000;
const MONEY: i16 = -10000;

/// Person data words.
mod person {
    /// Something for the interface to show over the person (0x4a69aa),
    /// cleared after.
    pub const REQUEST: usize = 1;
    /// Counted up every tick.
    pub const TICKS: usize = 27;
    /// Nonzero stops the motives decaying.
    pub const NO_DECAY: usize = 29;
    /// 0 for a member of the household that lives on the lot.
    pub const VISITOR: usize = 32;
    /// The kind of person: above 1 the room's enter and leave are skipped.
    pub const KIND: usize = 65;
}

/// The motive that tells whether a person is awake (0 and above).
const ALERTNESS: usize = 11;

/// What the player does when the game asks. A dialog that stops the game
/// (a modal one) is shown during the registry's pass, and the game waits
/// there for the answer (0x436da0).
pub trait Player {
    /// The answer to a dialog, or None to give none.
    fn answer(&mut self, shown: &Shown) -> Option<Answer>;

    /// The response to a special panel (`Dialog` kinds `5..=15`, `0x451552` /
    /// `0x4413e1..0x442320`), or `None` to wait for the player.
    ///
    /// Dismiss-only reference modals (kind 12 Spellbook and kind 15 Cookbook)
    /// auto-dismiss by default so existing headless `Player` implementations
    /// continue uninterrupted; interactive pickers wait (`None`).
    fn special_panel(&mut self, shown: &ShownSpecialPanel) -> Option<SpecialPanelResponse> {
        if shown.request.is_dismiss_only() {
            Some(SpecialPanelResponse::Dismissed)
        } else {
            None
        }
    }
}

/// A special panel (`Dialog` kinds `5..=15`) waiting for the player's response.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ShownSpecialPanel {
    /// The tick count when it is shown.
    pub tick: u32,
    /// Which special panel was requested, with its decoded parameters.
    pub request: SpecialPanelRequest,
}

/// A [`Player`] pairing ordinary dialog [`Answers`] with a scripted queue of
/// [`SpecialPanelResponse`]s (for headless tests and replays).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptedPlayer {
    pub answers: Answers,
    pub special_panels: VecDeque<SpecialPanelResponse>,
    /// When `false`, dismiss-only reference books (kinds 12 & 15) auto-dismiss
    /// without consuming `special_panels`; when `true`, they pop from
    /// `special_panels` (waiting if empty).
    pub wait_for_books: bool,
}

impl ScriptedPlayer {
    pub fn new(answers: Answers, special_panels: &[SpecialPanelResponse]) -> Self {
        Self {
            answers,
            special_panels: special_panels.iter().cloned().collect(),
            wait_for_books: false,
        }
    }
}

impl Player for ScriptedPlayer {
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

/// The result of a modal dialog and the UI sample taken while it was open.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Answer {
    /// The status the button gives (0x4370be): 2 for the first button,
    /// 3 for the second, 4 for the third.
    pub status: u8,
    /// What the wait check (0x476548) last published to global 12 while
    /// the dialog was open: the activated button's bitmap resource ID, or
    /// 0 from an idle poll (0x5dac77, 0x47653d) after an earlier button
    /// was destroyed (0x5e39cf). None means no poll changed it, so global
    /// 12 is left alone; it does not mean the dialog closed itself. The
    /// window supplies the ID and a headless replay records what was
    /// observed.
    pub bitmap_id: Option<i16>,
    /// The text entered into a kind 3 text-prompt dialog (`record + 0x44`,
    /// copied to global `[0x6e4ac0]` at `0x451a6b..0x451a85`), or `None` for
    /// non-text dialogs.
    pub text: Option<String>,
}

impl From<u8> for Answer {
    fn from(status: u8) -> Self {
        Self {
            status,
            bitmap_id: None,
            text: None,
        }
    }
}

/// A dialog the registry shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// The tick count when it is shown.
    pub tick: u32,
    /// The object that asked for it.
    pub object: u16,
    /// The answers it offers.
    pub answers: &'static [u8],
    /// Whether this is a kind 3 text-prompt dialog (`cTSWinTextEdit2`,
    /// `0x451a6b..0x451a85`), which requires an explicit player answer rather
    /// than auto-accepting as a single-button notice.
    pub text_entry: bool,
}

/// A player who answers from a list, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Answers {
    pub(crate) queue: VecDeque<Answer>,
    include_single_button: bool,
    /// Choice-only answers (`include_single_button = false`) that take over
    /// once `queue` is drained (for example when reopening a saved
    /// [`Answers::replay`] lot with caller-supplied choices).
    pub(crate) choices_after: Option<VecDeque<Answer>>,
}

impl Answers {
    /// A list of statuses for dialogs that offer a choice. A one-button
    /// dialog gives its only answer without consuming a list entry.
    pub fn choices(list: &[u8]) -> Self {
        Self {
            queue: list.iter().copied().map(Answer::from).collect(),
            include_single_button: false,
            choices_after: None,
        }
    }

    /// One answer for each dialog shown, including one-button notices.
    pub fn replay(list: &[Answer]) -> Self {
        Self {
            queue: list.iter().cloned().collect(),
            include_single_button: true,
            choices_after: None,
        }
    }

    /// Appends choice-only answers to take over once any existing queued
    /// answers have been consumed, switching `include_single_button` to `false`
    /// at the boundary so later one-button notices never consume a choice.
    pub(crate) fn extend_choices_after(&mut self, list: &[u8]) {
        if list.is_empty() {
            return;
        }
        let extra = self.choices_after.get_or_insert_with(VecDeque::new);
        extra.extend(list.iter().copied().map(Answer::from));
    }

    /// Appends an interactive answer at the end of the pending answer stream
    /// (onto `choices_after` when choice-only follow-up answers are staged
    /// behind an explicit replay prefix, otherwise onto `queue`).
    pub(crate) fn push_back_answer(&mut self, answer: Answer) {
        if let Some(extra) = self.choices_after.as_mut() {
            extra.push_back(answer);
        } else {
            self.queue.push_back(answer);
        }
    }
}

impl Player for Answers {
    fn answer(&mut self, shown: &Shown) -> Option<Answer> {
        if self.queue.is_empty()
            && let Some(next) = self.choices_after.take()
        {
            self.queue = next;
            self.include_single_button = false;
        }
        match shown.answers {
            [only] if !self.include_single_button && !shown.text_entry => Some((*only).into()),
            _ => self.queue.pop_front(),
        }
    }
}

/// Why a tick could not be run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickError {
    /// A script or a service on an object failed.
    Fault(Fault),
    /// A dialog was shown and the player gave no answer, or one it does
    /// not offer.
    Unanswered { shown: Shown, given: Option<u8> },
    /// A special panel (`Dialog` kinds `5..=15`) was shown and the player
    /// gave no response, or a response variant that does not match the panel.
    UnansweredSpecialPanel {
        shown: ShownSpecialPanel,
        given: Option<SpecialPanelResponse>,
    },
    /// Something the lot update does that is not ported.
    Unsupported(String),
}

impl fmt::Display for TickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fault(fault) => fault.fmt(f),
            Self::Unanswered { shown, given } => {
                write!(
                    f,
                    "object {} shows a dialog that waits for an answer ({}); ",
                    shown.object,
                    shown
                        .answers
                        .iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )?;
                match given {
                    None => f.write_str("none was given"),
                    Some(answer) => write!(f, "{answer} was given"),
                }
            }
            Self::UnansweredSpecialPanel { shown, given } => {
                write!(
                    f,
                    "special panel kind {} ({:?}) waits for a response; ",
                    shown.request.kind(),
                    shown.request
                )?;
                match given {
                    None => f.write_str("none was given"),
                    Some(resp) => write!(f, "{resp:?} was given"),
                }
            }
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
        }
    }
}

impl std::error::Error for TickError {}

impl From<Fault> for TickError {
    fn from(fault: Fault) -> Self {
        Self::Fault(fault)
    }
}

/// What one lot update did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Update {
    /// Whether the scheduler's pass was a full one, which advances the
    /// clock.
    pub full: bool,
    /// The objects whose countdown was counted down.
    pub counted: Vec<u16>,
    /// The objects ticked, in order.
    pub ticked: Vec<u16>,
    /// The dialogs shown and how each was answered.
    pub answered: Vec<(u16, u8)>,
    /// The special panels (`Dialog` kinds `5..=15`) shown and how each was
    /// answered during this update.
    pub special_panels_answered: Vec<(SpecialPanelRequest, SpecialPanelResponse)>,
    /// The text returned by the last kind 3 text-entry dialog answered during
    /// this update (`0x451a6b..0x451a85` -> global `[0x6e4ac0]`), if any.
    pub dialog_text: Option<String>,
    /// The objects the registry removed.
    pub removed: Vec<u16>,
    /// In play mode, the error caught in an object's tick, if any, and
    /// what was done about it ([`crate::play`]). Always empty in strict
    /// mode.
    pub errors: Vec<ScriptError>,
    /// In play mode, the frames of the house view drawn with this step
    /// that failed, and were skipped ([`crate::boot::Lot::step_play`]), in
    /// the order they were drawn. Always empty in strict mode.
    pub frame_errors: Vec<SkippedFrame>,
    /// The requests to the sound system made during the update, in order
    /// ([`crate::sounds`]). Empty when the interpreter carries its own
    /// sink ([`Interpreter::sounds`]), which then has them.
    pub sounds: Vec<SoundEvent>,
    /// The HUD highlight requests emitted by UI Effect (opcode 34, `0x44acff`)
    /// during the update, in order ([`UiEffectEvent`]). Empty when the
    /// interpreter carries its own sink ([`Interpreter::ui_effects`]), which
    /// then has them.
    pub ui_effects: Vec<UiEffectEvent>,
    /// The levels whose rooms' cutaway masks the original built again
    /// during the update, as bits (1 for level 1, 2 for level 2): those a
    /// room update counted, which builds them at its end at the zoom of
    /// the moment (0x466c3c, at 0x466ef6). A view that keeps the zoom its
    /// masks were built at reads it. Filled by [`crate::boot::Lot`]'s
    /// steps only ([`Interpreter::masks_rebuilt`]).
    pub masks_rebuilt: u8,
}

/// A frame of the house view that failed in play mode and was skipped
/// ([`Update::frame_errors`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFrame {
    /// The tick the frame was drawn after, counted as
    /// [`crate::boot::Lot::ticks`] counts them, from the load; 0 for the
    /// frame drawn before the first tick.
    pub after: u32,
    /// Why it failed.
    pub error: String,
}

impl std::fmt::Display for SkippedFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.after {
            0 => write!(f, "the frame before tick 1: {}", self.error),
            tick => write!(f, "the frame after tick {tick}: {}", self.error),
        }
    }
}

impl Interpreter<'_> {
    /// One lot update (0x46d345). See the module's documentation.
    ///
    /// On an error the world is left as the failing step left it.
    pub fn lot_update(
        &self,
        world: &mut World,
        tuning: &Tuning,
        player: &mut dyn Player,
    ) -> Result<Update, TickError> {
        self.lot_update_with(world, tuning, player, None)
    }

    /// One lot update, in play mode when `play` is given ([`crate::play`]):
    /// an error in an object's tick is caught and the object reset, and
    /// the update goes on.
    pub fn lot_update_with(
        &self,
        world: &mut World,
        tuning: &Tuning,
        player: &mut dyn Player,
        play: Option<&mut Play>,
    ) -> Result<Update, TickError> {
        let (update, result) = self.lot_update_kept(world, tuning, player, play);
        result.map(|()| update)
    }

    /// One lot update as [`Interpreter::lot_update_with`] runs it, with
    /// what it did whether or not it failed: on an error the update holds
    /// what came before it, such as, in play mode, the errors caught in
    /// the objects' ticks, and the sounds.
    pub fn lot_update_kept(
        &self,
        world: &mut World,
        tuning: &Tuning,
        player: &mut dyn Player,
        play: Option<&mut Play>,
    ) -> (Update, Result<(), TickError>) {
        let mut update = Update::default();
        // In play mode the deletions are noted, so that an object reset
        // and a later one given its ID are told apart (Interpreter::recover).
        let deleted = RefCell::new(Vec::new());
        let sink = RefCell::new(Vec::new());
        let ui_sink = RefCell::new(Vec::new());
        let this = Interpreter {
            sounds: Some(self.sounds.unwrap_or(&sink)),
            ui_effects: Some(self.ui_effects.unwrap_or(&ui_sink)),
            deleted: match play {
                Some(_) => Some(&deleted),
                None => self.deleted,
            },
            ..*self
        };
        let result = this.run_update(world, tuning, player, play, &deleted, &mut update);
        update.sounds = sink.into_inner();
        update.ui_effects = ui_sink.into_inner();
        (update, result)
    }

    /// The lot update's work, for [`Interpreter::lot_update_kept`].
    fn run_update(
        &self,
        world: &mut World,
        tuning: &Tuning,
        player: &mut dyn Player,
        play: Option<&mut Play>,
        deleted: &RefCell<Vec<u16>>,
        update: &mut Update,
    ) -> Result<(), TickError> {
        let mut play = play;
        let lot = Interpreter {
            stream: Stream::Lot,
            ..*self
        };
        update.full = lot.pass(world, tuning, update, play.as_deref_mut(), Some(player))?;
        let game = Interpreter {
            stream: Stream::Global,
            ..*self
        };
        let mut signal = None;
        let mut hour_turned = false;
        if update.full {
            world.ticks = world.ticks.wrapping_add(1);
            let seconds = global(world, SECONDS)?.wrapping_add(2);
            set_global(world, SECONDS, seconds)?;
            if seconds >= 60 {
                set_global(world, SECONDS, seconds.wrapping_sub(60))?;
                let minutes = global(world, MINUTES)?.wrapping_add(1);
                set_global(world, MINUTES, minutes)?;
                signal = Some(0xdb);
                if minutes >= 60 {
                    set_global(world, MINUTES, minutes.wrapping_sub(60))?;
                    let hours = global(world, HOURS)?.wrapping_add(1);
                    set_global(world, HOURS, hours)?;
                    hour_turned = true;
                    if hours >= 24 {
                        set_global(world, HOURS, hours.wrapping_sub(24))?;
                        signal = Some(0xdc);
                        if play.is_some() {
                            daily::turn_day_play(&game, world, &mut update.errors);
                        } else {
                            daily::turn_day(&game, world).map_err(TickError::Fault)?;
                        }
                    }
                }
            }
        }
        game.end_of_update_with(world, !update.full, player, play.as_deref_mut(), update)?;
        if let Some(play) = play.as_deref_mut() {
            for gone in deleted.take() {
                play.fresh.remove(&gone);
            }
        }
        // Signal 0xdb reaches only the interface among the subscribers
        // (see the module's documentation). Of 0xdc the lot's subscriber
        // takes its part (see `daily`).
        if signal == Some(0xdc) {
            daily::shift_usage(world);
            if play.is_some() {
                game.day_statistics_play(world, &mut update.errors);
            } else {
                game.day_statistics(world).map_err(TickError::Fault)?;
            }
        }
        if hour_turned {
            let period = time_of_day(global(world, HOURS)?);
            if period != global(world, TIME_OF_DAY)? {
                // The interface's call (0x4c4648) changes nothing kept
                // here. Signal 0xf8 grows the lawn when global 36 is set
                // (0x47b2e0), and signal 0xdd makes the lot mark every
                // room (0x416446, 0x466ba3). See the module's
                // documentation. The original sets global 4 first; neither
                // the growth nor the marking reads it, so growing first,
                // which leaves the lot unchanged when it fails, is the same.
                if global(world, GROW_LAWN)? != 0
                    && let Err(msg) = world.grow_grass(LAWN_GROWTH)
                {
                    if play.is_some() {
                        update.errors.push(ScriptError {
                            tick: world.ticks,
                            object: 0,
                            fault: Fault {
                                object: 0,
                                at: None,
                                trap: Trap::Unsupported(msg),
                            },
                            in_thread: false,
                            then: None,
                            recovery: Recovery::Reset,
                        });
                    } else {
                        return Err(TickError::Unsupported(msg));
                    }
                }
                set_global(world, TIME_OF_DAY, period)?;
                world.mark_all_rooms();
            }
        }
        Ok(())
    }

    /// The scheduler's pass over the objects (0x46d553). Returns whether
    /// it was a full one.
    ///
    /// The pass is partial if global 17 is not 0, or if globals 19 and 10
    /// both are not. It goes by ID from the cursor (or 1) up to the
    /// highest ID it saw when it started; an object made during the pass
    /// with a higher ID waits for the next pass. For each object:
    ///
    /// 1. If its scheduler flags have bit 2 (value 4), the pass checks for
    ///    a deferred thread error (`0x46d5da` → `0x439a6f`), represented here
    ///    by a ready object whose stack was cleared when `0x474aae` caught an
    ///    error on another thread (stopping the pass and recovering in play
    ///    mode, or failing loudly in strict mode); otherwise the flag is
    ///    cleared.
    /// 2. Without flag 1 it is skipped, and in a partial pass so is one
    ///    without flag 2.
    /// 3. A countdown above 0 is counted down by 1. Otherwise the object
    ///    is ticked ([`Interpreter::tick_object`], or
    ///    [`Interpreter::tick_person`] for a person) with the tick count.
    ///    A tick that returns false stops the pass with the cursor on the
    ///    object.
    ///
    /// After the last object the cursor goes back to 0.
    ///
    /// The original's table of objects can hold IDs past the highest
    /// object it holds (an ID freed at the end and not reused); here the
    /// pass stops at the highest ID held. The difference shows only for
    /// an object made during a pass with such an ID: **unverified**.
    pub fn scheduler_pass(
        &self,
        world: &mut World,
        tuning: &Tuning,
        update: &mut Update,
    ) -> Result<bool, TickError> {
        self.pass(world, tuning, update, None, None)
    }

    /// The scheduler's pass, in play mode when `play` is given: a fault in
    /// an object's tick is dealt with as the original deals with a
    /// SimAntics error ([`crate::play`]), and the pass stops partial with
    /// the cursor on the object.
    fn pass(
        &self,
        world: &mut World,
        tuning: &Tuning,
        update: &mut Update,
        mut play: Option<&mut Play>,
        player: Option<&mut dyn Player>,
    ) -> Result<bool, TickError> {
        let partial =
            global(world, PAUSED)? != 0 || (global(world, MODE)? != 0 && global(world, LOT)? != 0);
        let end = world.objects.keys().next_back().copied().unwrap_or(0);
        if world.scheduler_cursor == 0 {
            world.scheduler_cursor = 1;
        }
        let start = u16::try_from(world.scheduler_cursor).map_err(|_| {
            TickError::Unsupported(format!("a scheduler cursor of {}", world.scheduler_cursor))
        })?;
        let taken_off = RefCell::new(BTreeMap::new());
        let inline_tick = world.ticks.wrapping_add(u32::from(!partial));
        let inline_panel = player.map(|p| InlineSpecialPanel::new(inline_tick, p));
        let inline_sink: Option<&(dyn crate::primitives::dialog::InlinePanelSink + '_)> =
            inline_panel
                .as_ref()
                .map(|p| p as &dyn crate::primitives::dialog::InlinePanelSink);
        let this = Interpreter {
            taken_off: Some(&taken_off),
            inline_special_panel: inline_sink,
            ..*self
        };
        let mut cursor = start;
        while cursor <= end {
            let Some(id) = world.objects.next_key_from(cursor) else {
                break;
            };
            if id > end {
                break;
            }
            cursor = id.saturating_add(1);
            world.scheduler_cursor = id as i16;
            let Some(object) = world.objects.get(&id) else {
                continue;
            };
            let mut flags = object.flags();
            let mut countdown = object.countdown().unwrap_or(0);
            let mut is_person = object.person.is_some();
            if flags & CHECK_ERROR != 0 {
                if flags & READY != 0 && object.stack.is_empty() {
                    // 0x46d5da -> 0x46d68c -> 0x439a6f: a deferred thread
                    // error recorded by 0x474aae (such as a faulted "init
                    // visitor") stops the pass and resets the object instead
                    // of running its tick.
                    let fault = Fault {
                        object: id,
                        at: None,
                        trap: Trap::Unsupported(format!(
                            "object {id} has a deferred thread error (0x46d5da -> 0x439a6f)"
                        )),
                    };
                    if let Some(play) = play.as_deref_mut() {
                        let recovery = self.recover(world, id, play);
                        update.errors.push(ScriptError {
                            tick: world.ticks,
                            object: id,
                            fault,
                            in_thread: true,
                            then: None,
                            recovery,
                        });
                        if let Some(panel) = &inline_panel {
                            panel.drain_answered(&mut update.special_panels_answered);
                        }
                        return Ok(false);
                    }
                    return Err(fault.into());
                }
                if let Some(object) = world.objects.get_mut(&id) {
                    object.set_flags(CHECK_ERROR, false);
                    flags = object.flags();
                    countdown = object.countdown().unwrap_or(0);
                    is_person = object.person.is_some();
                }
            }
            if flags & READY == 0 || (partial && flags & IN_PARTIAL == 0) {
                if id == u16::MAX {
                    break;
                }
                continue;
            }
            if countdown > 0 {
                if let Some(object) = world.objects.get_mut(&id) {
                    object.set_countdown(countdown - 1);
                }
                update.counted.push(id);
                if id == u16::MAX {
                    break;
                }
                continue;
            }
            let tick = world.ticks as i32;
            update.ticked.push(id);
            let mut caught = None;
            let catch = play.is_some().then_some(&mut caught);
            let mut inline_err = None;
            let result = if is_person {
                this.person_tick_with_player(world, id, tick, tuning, catch, &mut inline_err)
            } else {
                this.object_tick_with_player(world, id, catch, &mut inline_err)
            };
            if let Some(panel) = &inline_panel {
                panel.drain_answered(&mut update.special_panels_answered);
            }
            if let Some(err) = inline_err {
                return Err(err);
            }
            // The fault caught in the thread, whether it was, and a fault
            // in the rest of the tick after it.
            let (carried_on, error) = match result {
                Ok(carried_on) => (carried_on, caught.map(|fault| (fault, true, None))),
                Err(fault) if play.is_some() => match caught {
                    Some(first) => (true, Some((first, true, Some(fault)))),
                    None => (true, Some((fault, false, None))),
                },
                Err(fault) => return Err(fault.into()),
            };
            if let Some(play) = play.as_deref_mut() {
                if let Some((fault, in_thread, then)) = error {
                    let recovery = self.recover(world, id, play);
                    update.errors.push(ScriptError {
                        tick: world.ticks,
                        object: id,
                        fault,
                        in_thread,
                        then,
                        recovery,
                    });
                    return Ok(false);
                }
                play.fresh.remove(&id);
            }
            if !carried_on {
                return Err(TickError::Unsupported(format!(
                    "object {id}'s tick returned false, which stops the scheduler's pass with \
                     the cursor kept; carrying on in the next update is not ported"
                )));
            }
            if id == u16::MAX {
                break;
            }
        }
        world.scheduler_cursor = 0;
        Ok(!partial)
    }

    /// An object's tick (0x4521f7): its thread runs (0x474e88) until it
    /// yields. If that returns true, data word 25 is counted down when it
    /// is not 0, and the object's headlines step (0x46e004): the first
    /// one's duration counts down ([`sims_sim::World::headlines`]); its
    /// animation, which only decides when the object is redrawn, is not
    /// modeled.
    ///
    /// The thread returns true when it yields. Returning from its
    /// outermost frame is error 0x3e9 in the original, which the object's
    /// error handler deals with; that fails here. No ported primitive
    /// returns the codes that make a thread return false.
    pub fn tick_object(&self, world: &mut World, id: u16) -> Result<bool, Fault> {
        let mut inline_err = None;
        self.object_tick_with_player(world, id, None, &mut inline_err)
    }

    /// An object's tick. With `catch`, a fault in the thread's run is put
    /// there and the rest of the tick runs, as the original's catch
    /// (0x474ec1) lets it ([`crate::play`]); without, it is returned.
    fn object_tick_with_player(
        &self,
        world: &mut World,
        id: u16,
        catch: Option<&mut Option<Fault>>,
        inline_err: &mut Option<TickError>,
    ) -> Result<bool, Fault> {
        let fail = |code, message| Fault {
            object: id,
            at: None,
            trap: Trap::native(code, message),
        };
        let turn_result = self.turn(world, id, TICK_BUDGET);
        if let Some(err) = self
            .inline_special_panel
            .and_then(|inline| inline.take_unanswered())
        {
            *inline_err = Some(err);
            return Ok(true);
        }
        let error = match turn_result {
            Ok(Status::Yielded) => None,
            Ok(Status::Returned(_)) => Some(fail(
                0x3e9,
                "the thread returned from its outermost frame; the original's error handling \
                 (the object's virtual function 4) is not ported",
            )),
            Ok(Status::Running) => Some(fail(
                0x44d,
                "the thread ran past the instruction limit of one tick; the original's error \
                 handling (the object's virtual function 4) is not ported",
            )),
            Err(fault) => Some(fault),
        };
        if let Some(fault) = error {
            let Some(slot) = catch else {
                return Err(fault);
            };
            // The original's virtual function 4 (0x439a4e) sets flag 4.
            if let Some(object) = world.objects.get_mut(&id) {
                object.set_flags(CHECK_ERROR, true);
            }
            *slot = Some(fault);
        }
        // The tail of 0x4521f7, as the factory runs it after an object's
        // first turn.
        let mut machine = self.machine(world, id, 0, false);
        machine.end_turn().map_err(|trap| machine.fault(trap))?;
        Ok(true)
    }

    /// A person's tick (0x45cc8c):
    ///
    /// 1. The object's tick ([`Interpreter::tick_object`]). If it returns
    ///    false, so does this.
    /// 2. Once 60 ticks have passed since the last decay (person field
    ///    0x810, 0 when the person is made), the decay tick is noted and,
    ///    if person data word 29 and data word 34 are 0, the motives decay
    ///    ([`Tuning::decay`], 0x45d145).
    /// 3. The person's animator ticks (its virtual function 0x50, 0x58dc72;
    ///    see [`crate::animator`]).
    /// 4. The room the person was last seen in (person field 0xafc) is
    ///    brought up to date from data word 29 (0x45e43d,
    ///    [`Interpreter::note_room`]; a walking animator has already done
    ///    so when it put the person down). When it changes
    ///    a person whose kind (person word 65) is at most 1 leaves the old
    ///    room, if no other person was last seen there (0x46886c), and
    ///    enters the new one ([`World::note_occupant`], 0x468802). The
    ///    original then sends signal 0xf7 for the selected Sim (not
    ///    modeled).
    /// 5. Unless the room is still unknown (-5), motive 13 takes the
    ///    room's score (0x45cef4): the lot's outdoor score for an outdoor
    ///    room, the room's own score for an indoor one, 0 for a room that
    ///    does not exist. In room 0 on a lot that is not residential a
    ///    score below 0 counts as 0.
    /// 6. Person word 27 goes up by 1.
    /// 7. The motive changes the person's actions set run once
    ///    ([`motives::apply_changes`], 0x45cdb3).
    /// 8. Every tenth tick, for an awake member of the household who is
    ///    not hidden, the house's usage counts take a sample (0x4165e3,
    ///    [`sims_sim::HouseUsage::sample`]), which counts whether the
    ///    person has a route context (0x45c349).
    /// 9. A nonzero person word 1 goes to the interface (0x4a69aa) and is
    ///    cleared (0x45ce7e). Below -20000 the interface shows -20000 less
    ///    the value; from -20000 up to -10000 it plays the sound
    ///    "sting_char_winner" for the person (0x57a54c) and shows -10000
    ///    less the value; from -10000 up it shows the value as money.
    ///    Each is shown only while the interface's object
    ///    (\[0x6f2a58\]+0x84) is there. The showing is presentation (text
    ///    drawn over the person, 0x4d322c), so only the clearing is
    ///    modeled; the sound is not ported and fails.
    pub fn tick_person(
        &self,
        world: &mut World,
        id: u16,
        tick: i32,
        tuning: &Tuning,
    ) -> Result<bool, Fault> {
        let mut inline_err = None;
        if self.taken_off.is_some() {
            return self.person_tick_with_player(world, id, tick, tuning, None, &mut inline_err);
        }
        // Outside the scheduler's pass, the tick has a pass of its own for
        // the refreshes of anyone it queues for removal (thread or
        // animator). No other person's tick follows in it for them to wait
        // for, so one left there fails: dropping it would hide it from a
        // step run both ways (`crate::both_ways`).
        let taken_off = RefCell::new(BTreeMap::new());
        let ticked = Interpreter {
            taken_off: Some(&taken_off),
            ..*self
        }
        .person_tick_with_player(world, id, tick, tuning, None, &mut inline_err)?;
        if let Some(other) = taken_off.into_inner().into_keys().next() {
            return Err(Fault {
                object: id,
                at: None,
                trap: Trap::Unsupported(format!(
                    "person {other} queued for removal in person {id}'s tick outside the \
                     scheduler's pass: its animator's refresh (0x58da12) waits for its own \
                     tick, which no pass here runs"
                )),
            });
        }
        Ok(ticked)
    }

    /// A person's tick, catching a fault in its thread's run as
    /// [`Interpreter::object_tick_with_player`] does.
    fn person_tick_with_player(
        &self,
        world: &mut World,
        id: u16,
        tick: i32,
        tuning: &Tuning,
        catch: Option<&mut Option<Fault>>,
        inline_err: &mut Option<TickError>,
    ) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        // A person queued before its tick was queued by another's removal
        // (one that removes itself queues during its tick). Its animator's
        // refresh as it left the lot, worked out then
        // (`Interpreter::refresh_before_removal`), is pending until its
        // animator's tick, as for a person that takes itself off.
        let pending = if world.removals.contains(&id) {
            let pending = self
                .taken_off
                .and_then(|taken_off| taken_off.borrow_mut().remove(&id));
            if pending.is_none() && catch.is_none() {
                return Err(fail(Trap::Unsupported(format!(
                    "person {id} starts its tick while queued for removal, with no animator's \
                     refresh worked out as it left the lot: that refresh (0x58da12) depends on \
                     whether the view drew it (animator byte +0x109), which is not kept"
                ))));
            }
            pending
        } else {
            None
        };
        let refreshing = RefCell::new(Refreshing {
            person: id,
            pending,
            both_ways: false,
        });
        let ticked = Interpreter {
            refreshing: Some(&refreshing),
            ..*self
        }
        .object_tick_with_player(world, id, catch, inline_err)?;
        if inline_err.is_some() {
            return Ok(true);
        }
        let pending = refreshing.into_inner().pending;
        if !ticked {
            if pending.is_some() {
                return Err(fail(Trap::Unsupported(
                    "a person that left the lot ends its tick before its \
                     animator's: its animator's refresh at the take-off (0x58da12) is then not \
                     redone at the same clock"
                        .into(),
                )));
            }
            return Ok(false);
        }
        let hour = world.globals.get(HOURS).copied().unwrap_or(0);
        let lot = world.globals.get(LOT).copied().unwrap_or(0);
        let residential = self.program.zoning.get(&lot) == Some(&1);
        let mut state = world
            .person_ticks
            .get(&id)
            .copied()
            .unwrap_or(PersonTick::CONSTRUCTED);
        {
            let object = world
                .objects
                .get_mut(&id)
                .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
            let hidden = object.data.get(HIDDEN).copied().unwrap_or(0);
            let person = object
                .person
                .as_mut()
                .ok_or_else(|| fail(Trap::native(0x1c, "object is not a person")))?;
            if tick >= state.last_decay.wrapping_add(60) {
                state.last_decay = tick;
                if person.data.get(person::NO_DECAY).copied().unwrap_or(0) == 0 && hidden == 0 {
                    tuning.decay(person, hour);
                }
            }
        }
        // The animator's walk notes the room too (0x45e4e9), so the state
        // goes back before it ticks.
        world.person_ticks.insert(id, state);
        match pending {
            None => self.tick_animator(world, id),
            Some(pending) => self.tick_animator_after_take_off(world, id, pending),
        }
        .map_err(fail)?;
        self.note_room(world, id).map_err(fail)?;
        let state = world
            .person_ticks
            .get(&id)
            .copied()
            .unwrap_or(PersonTick::CONSTRUCTED);
        let outdoor_score = world.rooms.outdoor_score;
        // The score of a room that exists (0x45cef4).
        let score = |room: &Room| {
            if room.outdoors {
                outdoor_score
            } else {
                room.score
            }
        };
        let object = world
            .objects
            .get_mut(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        let word = |data: &[i16], index: usize| data.get(index).copied().unwrap_or(0);
        let hidden = word(&object.data, HIDDEN);
        let person = object
            .person
            .as_mut()
            .ok_or_else(|| fail(Trap::native(0x1c, "object is not a person")))?;
        if state.room != -5 {
            let mut value =
                world_room(&world.rooms.rooms, state.room).map_or(sims_core::F32::ZERO, score);
            // `jae` past the zeroing (0x45cd9c): a value that is not a
            // number is zeroed too.
            if state.room == 0 && !residential && below_zero(value) {
                value = sims_core::F32::ZERO;
            }
            person.motives[motives::ROOM] = value.to_f32();
        }
        if let Some(count) = person.data.get_mut(person::TICKS) {
            *count = count.wrapping_add(1);
        }
        motives::apply_changes(person);
        // `jb` past the call (0x45ce41): a value that is not a number
        // counts as asleep.
        let awake = !below_zero(sims_core::F32::from_f32(person.motives[ALERTNESS]));
        if tick % 10 == 0 && word(&person.data, person::VISITOR) == 0 && hidden == 0 && awake {
            // 0x45c349: the person's last route context, if it has any.
            // The routing primitives push and pop them (Go To Routing
            // Slot, 0x45abce, pushes); the load makes as many as the save
            // counts (0x460443).
            let routing = world
                .routing
                .routes
                .get(&id)
                .is_some_and(|r| !r.contexts.is_empty());
            world.usage.sample(routing);
        }
        // 0x45ce61: a nonzero person word 1 goes to the interface
        // (0x4a69aa) and is cleared.
        let request = word(&person.data, person::REQUEST);
        if request != 0 {
            if (STING..MONEY).contains(&request) {
                return Err(fail(Trap::Unsupported(format!(
                    "person word 1 is {request}, which 0x4a69aa shows with the sound \
                     \"sting_char_winner\" (0x4a6a56); not ported"
                ))));
            }
            person.data[person::REQUEST] = 0;
        }
        world.person_ticks.insert(id, state);
        Ok(true)
    }

    /// The room a person was last seen in (person field 0xafc) brought up
    /// to date from its data word 29 (0x45e43d; [`Interpreter::tick_person`],
    /// step 4). The person's tick runs this, and so does the placement of
    /// a walking person (virtual function 0x88, 0x45e4e9).
    pub(crate) fn note_room(&self, world: &mut World, id: u16) -> Result<(), Trap> {
        let mut state = world
            .person_ticks
            .get(&id)
            .copied()
            .unwrap_or(PersonTick::CONSTRUCTED);
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::native(10, "missing object"))?;
        let word = |data: &[i16], index: usize| data.get(index).copied().unwrap_or(0);
        let room = word(&object.data, ROOM);
        if room == state.room {
            return Ok(());
        }
        let kind = word(
            &object
                .person
                .as_ref()
                .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?
                .data,
            person::KIND,
        );
        // 0x46886c: how many other people were last seen in the old
        // room, leaving out those with bit 0 of person word 84 or 68
        // set (0x4351ef). A person not ticked yet has the
        // constructor's -5.
        let others = world
            .objects
            .iter()
            .filter(|&(&other, _)| other != id)
            .filter_map(|(other, o)| Some((other, o.person.as_ref()?)))
            .filter(|(_, p)| {
                let bit = |index: usize| p.data.get(index).is_some_and(|w| w & 1 != 0);
                !bit(84) && !bit(68)
            })
            .filter(|(other, _)| {
                world
                    .person_ticks
                    .get(other)
                    .map_or(PersonTick::CONSTRUCTED.room, |t| t.room)
                    == state.room
            })
            .count();
        // A room word from -5 to -1 names no room and is not looked up
        // (0x45e463, an unsigned compare with 0xfffb).
        let named = |room: i16| !(-5..=-1).contains(&room);
        let old = state.room;
        state.room = room;
        world.person_ticks.insert(id, state);
        if kind <= 1 {
            if named(old) && others == 0 {
                world.note_occupant(old, false);
            }
            if named(room) {
                world.note_occupant(room, true);
            }
        }
        Ok(())
    }

    /// What play mode does with an object after its error
    /// ([`crate::play`]): the original's handling (0x439a6f), or where
    /// that fails, a restart of its thread, or where that fails too or the
    /// object faulted on its first tick after a reset, parking it.
    fn recover(&self, world: &mut World, id: u16, play: &mut Play) -> Recovery {
        let park = |world: &mut World| {
            if let Some(object) = world.objects.get_mut(&id) {
                object.set_flags(READY, false);
            }
        };
        // An object deleted since it was reset is not the one that is
        // fresh, even if a later one has its ID.
        if let Some(deleted) = self.deleted {
            for gone in deleted.take() {
                play.fresh.remove(&gone);
            }
        }
        if !play.fresh.insert(id) {
            play.fresh.remove(&id);
            park(world);
            return Recovery::Parked(None);
        }
        let reset = self.drop_references(world, id).and_then(|()| {
            if world.objects.get(&id).is_some_and(|o| o.person.is_some()) {
                self.reset_person(world, id, &self.program.people)
            } else {
                self.reset(world, id, false)
            }
        });
        let Err(failed) = reset else {
            return Recovery::Reset;
        };
        // A person's animator is reset too where it can be, as a person's
        // own reset does; one it cannot reset is left as it is.
        let _ = world.reset_animator(id);
        let mut machine = self.machine(world, id, 0, false);
        match machine.install_main() {
            Ok(()) => Recovery::Restarted(failed),
            Err(trap) => {
                let fault = machine.fault(trap);
                play.fresh.remove(&id);
                park(world);
                Recovery::Parked(Some(fault))
            }
        }
    }

    /// The registry's pass at the end of a lot update (0x4412d1):
    ///
    /// 1. The dialogs, from the first record, starting over after each
    ///    one it acts on: a new record is shown (0x436da0: status 1) and,
    ///    if it stops the game, answered by the player then and there; a
    ///    finished one (status 5) is destroyed (0x448663). While a dialog
    ///    waits, the family panel is redrawn and the modal message loop
    ///    (0x5dab4b) lets the application poll the wait check (0x4d1a8b).
    ///    The answer can carry the bitmap ID that poll writes to global 12.
    /// 2. After a full pass, the queued removals, oldest first: each ID is
    ///    taken off the queue and, if it still names an object, the
    ///    object is removed at once ([`Interpreter::remove_now`],
    ///    0x44123f), a tile of a group with its whole group. A tile whose
    ///    type has the dynamic group function (8, 0x42344e) would first
    ///    leave its group (0x42346a), which is not ported. A partial
    ///    pass leaves the queue as it is; one that leaves a person queued
    ///    fails, since that person's animator would live on past the
    ///    refresh its take-off skipped
    ///    ([`Interpreter::take_off_to_remove`]).
    /// 3. Also after a full pass, on a tick count (already counted for this
    ///    update) that is a multiple of 40, registry byte 0x12
    ///    ([`sims_sim::neighbors::Neighbors::relationships_changed`]) is cleared
    ///    (0x4424cb).
    /// 4. If no Sim is selected, or the selected one is a visitor on a
    ///    residential lot, the game selects another household member
    ///    (`0x445501`, [`Interpreter::select_next_resident`]).
    pub fn end_of_update(
        &self,
        world: &mut World,
        partial: bool,
        player: &mut dyn Player,
        update: &mut Update,
    ) -> Result<(), TickError> {
        self.end_of_update_with(world, partial, player, None, update)
    }

    /// Registry end-of-update pass (`0x4412d1`), with Play Mode recovery when
    /// `play` is `Some`: removal, dynamic-group, dialog-owner, `+0x109`
    /// partial-pass person deferral, and resident-reselection faults are caught
    /// and cleaned up instead of halting the lot. Strict Mode (`play: None`)
    /// is unchanged.
    fn end_of_update_with(
        &self,
        world: &mut World,
        partial: bool,
        player: &mut dyn Player,
        mut play: Option<&mut Play>,
        update: &mut Update,
    ) -> Result<(), TickError> {
        loop {
            let records = &mut world.dialogs.records;
            let Some(index) = records
                .iter()
                .position(|record| record.status == NEW || record.status == DONE)
            else {
                break;
            };
            if records[index].status == DONE {
                records.remove(index);
                continue;
            }
            let record = &mut records[index];
            record.status = SHOWN;
            let kind = record.operands[5] & 0x7f;
            if dialog::informs(&record.operands) && matches!(kind, 5 | 7 | 8 | 11 | 13) {
                // `0x4443ab` constructed a dedicated neighborhood-picker dialog
                // with literal `0` (ignoring operand 7 bit 7 and portrait
                // formatting). For kind 13, `0x551285` initializes
                // `[dialog + 0xec] = 1` (`via_blimp: true`) without the
                // blocking path's global-34 override (`0x4421e3..0x442219`).
                let req = match kind {
                    5 => SpecialPanelRequest::DowntownLotPicker,
                    7 => {
                        let current_lot = world.globals.get(LOT).copied().unwrap_or(0);
                        let on_residential =
                            current_lot == 0 || self.program.zoning.get(&current_lot) == Some(&1);
                        SpecialPanelRequest::VacationLotPicker { on_residential }
                    }
                    8 => SpecialPanelRequest::OldTownLotPicker {
                        unleashed: self.program.expansions & 0x20 != 0,
                    },
                    11 => SpecialPanelRequest::StudioTownLotPicker { star_limo: false },
                    13 => SpecialPanelRequest::MagicTownLotPicker { via_blimp: true },
                    _ => unreachable!(),
                };
                let shown = ShownSpecialPanel {
                    tick: world.ticks,
                    request: req.clone(),
                };
                self.refresh_family_panel(world);
                match player.special_panel(&shown) {
                    Some(resp) if req.accepts(&resp) => {
                        world.dialogs.records[index].status = DONE;
                        update.special_panels_answered.push((req, resp));
                        continue;
                    }
                    other => {
                        return Err(TickError::UnansweredSpecialPanel {
                            shown,
                            given: other,
                        });
                    }
                }
            }
            let Some(answers) = dialog::answers(record.operands[5]) else {
                let caller = record.caller;
                let msg =
                    format!("object {caller}'s dialog is of a kind that is not ported (0x436da0)");
                if play.is_some() {
                    record.status = DONE;
                    update.errors.push(ScriptError {
                        tick: world.ticks,
                        object: caller,
                        fault: Fault {
                            object: caller,
                            at: None,
                            trap: Trap::Unsupported(msg),
                        },
                        in_thread: false,
                        then: None,
                        recovery: Recovery::Reset,
                    });
                    continue;
                }
                return Err(TickError::Unsupported(msg));
            };
            if !dialog::modal(&record.operands) {
                let caller = record.caller;
                let msg = format!(
                    "object {caller} shows a dialog that does not stop the game; answering one \
                     while the game runs is not ported (0x436da0)"
                );
                if play.is_some() {
                    record.status = DONE;
                    update.errors.push(ScriptError {
                        tick: world.ticks,
                        object: caller,
                        fault: Fault {
                            object: caller,
                            at: None,
                            trap: Trap::Unsupported(msg),
                        },
                        in_thread: false,
                        then: None,
                        recovery: Recovery::Reset,
                    });
                    continue;
                }
                return Err(TickError::Unsupported(msg));
            }
            let shown = Shown {
                tick: world.ticks,
                object: record.caller,
                answers,
                text_entry: record.operands[5] & 0x7f == 3,
            };
            // While the dialog waits, the game handles its messages, which
            // redraws the family panel.
            self.refresh_family_panel(world);
            let given = player.answer(&shown);
            let answer = match given {
                Some(answer) if answers.contains(&answer.status) => answer,
                other => {
                    return Err(TickError::Unanswered {
                        shown,
                        given: other.map(|answer| answer.status),
                    });
                }
            };
            if let Some(bitmap_id) = answer.bitmap_id {
                publish_waited_for(world, bitmap_id)?;
            }
            let record = &mut world.dialogs.records[index];
            // 0x4370be: the button's result, or done at once for a dialog
            // that only informs.
            record.status = if dialog::informs(&record.operands) {
                DONE
            } else {
                answer.status
            };
            if record.operands[5] & 0x7f == 3 {
                let text = answer.text.unwrap_or_default();
                if let Some(slot) = self.dialog_text {
                    *slot.borrow_mut() = text.clone();
                }
                update.dialog_text = Some(text);
            }
            update.answered.push((record.caller, answer.status));
        }
        self.end_of_update_special_panels(world, player, update)?;
        if partial
            && play.is_none()
            && let Some(&id) = world.removals.iter().find(|id| {
                world
                    .objects
                    .get(id)
                    .is_some_and(|object| object.person.is_some())
            })
        {
            return Err(TickError::Unsupported(format!(
                "person {id} stays queued for removal after a partial pass; its animator's \
                 refresh as it left the lot (0x58da12) depends on whether the view drew it \
                 (animator byte +0x109), which is not kept"
            )));
        }
        if !partial {
            while !world.removals.is_empty() {
                let id = world.removals.remove(0);
                let Some(object) = world.objects.get(&id) else {
                    continue;
                };
                let guid = object.guid;
                let has_multipart = object.multipart.is_some();
                let master = crate::interpreter::object_definition(self.catalog, world, guid)
                    .and_then(|d| self.program.definition(d.resource, d.chunk))
                    .map(|words| words[10]);
                let master = match master {
                    Some(master) => master,
                    None => {
                        let fault = Fault {
                            object: id,
                            at: None,
                            trap: Trap::Invalid(format!("no definition for GUID 0x{guid:08X}")),
                        };
                        if let Some(play) = play.as_deref_mut() {
                            update.errors.push(ScriptError {
                                tick: world.ticks,
                                object: id,
                                fault,
                                in_thread: false,
                                then: None,
                                recovery: Recovery::Parked(None),
                            });
                            for gone in self.force_remove_object(world, id) {
                                play.fresh.remove(&gone);
                            }
                            update.removed.push(id);
                            continue;
                        }
                        return Err(fault.into());
                    }
                };
                // 0x442450 to 0x4424b0: a tile of a group whose type has
                // the dynamic group function leaves its group first.
                if master != 0
                    && has_multipart
                    && self.has_function(world, id, function::DYNAMIC_GROUP)
                    && let Err(fault) = self.leave_group(world, id)
                {
                    if play.is_some() {
                        update.errors.push(ScriptError {
                            tick: world.ticks,
                            object: id,
                            fault,
                            in_thread: false,
                            then: None,
                            recovery: Recovery::Parked(None),
                        });
                        crate::placement::unlink_tile_force(world, id);
                    } else {
                        return Err(fault.into());
                    }
                }
                if !world.objects.contains_key(&id) {
                    if let Some(play) = play.as_deref_mut() {
                        play.fresh.remove(&id);
                    }
                    update.removed.push(id);
                    continue;
                }
                if world.dialogs.records.iter().any(|r| r.caller == id) {
                    if play.is_some() {
                        world.dialogs.records.retain(|r| r.caller != id);
                    } else {
                        return Err(TickError::Unsupported(format!(
                            "removing object {id}, which owns a dialog, is not ported"
                        )));
                    }
                }
                if let Some(play) = play.as_deref_mut() {
                    let group_parts = self.parts(world, id).unwrap_or_else(|_| vec![id]);
                    if let Err(fault) = self.remove_now(world, id) {
                        update.errors.push(ScriptError {
                            tick: world.ticks,
                            object: id,
                            fault,
                            in_thread: false,
                            then: None,
                            recovery: Recovery::Parked(None),
                        });
                        for part in group_parts {
                            for gone in self.force_remove_object(world, part) {
                                play.fresh.remove(&gone);
                            }
                        }
                        if world.objects.contains_key(&id) {
                            for gone in self.force_remove_object(world, id) {
                                play.fresh.remove(&gone);
                            }
                        }
                        play.fresh.remove(&id);
                    }
                } else {
                    self.remove_now(world, id)?;
                }
                update.removed.push(id);
            }
            // 0x4424cb: on a tick count that is a multiple of 40, registry
            // byte 0x12 is cleared, after a call that only returns
            // (0x46fb64).
            if world.neighbors.relationships_changed && (world.ticks as i32) % 40 == 0 {
                world.neighbors.relationships_changed = false;
            }
        }
        let lot = global(world, LOT)?;
        let reselect = match selected(world) {
            // `0x445501` (`0x4455e8..0x4456e3`): when the active family
            // (`game + 0x10`) has zero members (an unoccupied lot), scanning
            // for a replacement Sim returns NULL without changing selection.
            None => self.active_family != Some(&[]),
            Some(sim) => {
                let visitor = world
                    .objects
                    .get(&sim)
                    .and_then(|o| o.person.as_ref())
                    .and_then(|p| p.data.get(person::VISITOR))
                    .copied()
                    .unwrap_or(0);
                // The original also asks that a byte at 0x6e4f64 be 0,
                // which it is not known to set: **unverified**.
                self.program.zoning.get(&lot) == Some(&1) && visitor != 0
            }
        };
        if reselect {
            let before = selected(world);
            let chosen = match self.select_next_resident(world) {
                Ok(chosen) => chosen,
                Err(fault) if play.is_some() => {
                    update.errors.push(ScriptError {
                        tick: world.ticks,
                        object: fault.object,
                        fault,
                        in_thread: false,
                        then: None,
                        recovery: Recovery::Reset,
                    });
                    if selected(world).is_none()
                        && let Some(slot) = world.globals.get_mut(3)
                    {
                        *slot = 0;
                    }
                    None
                }
                Err(fault) => return Err(fault.into()),
            };
            let after = selected(world);
            if let Some(coop) = self.coop
                && (chosen.is_some() || after != before)
            {
                let mut ctx = coop.borrow_mut();
                let host_id = ctx.session.host_id;
                if let Some(host) = ctx.session.players.get_mut(&host_id) {
                    host.selected_sim = after;
                }
            }
        }
        Ok(())
    }

    /// Special panel modal dispatch in `end_of_update` (`0x4413e1..0x442320`):
    ///
    /// 1. Kind 10 (`reg+0xe8`, Call Neighbor, `0x4413e1..0x4414cd`) — falls through;
    /// 2. Kind 5 (`reg+0xea`, Downtown lot picker, `0x4414cd..0x4415fd`) — falls through;
    /// 3. Mutually exclusive chain (`jmp 0x442320` after the first one that fires):
    ///    - Kind 8 (`reg+0xf2`, Old Town / Community lot picker, `0x4415fd..0x4417ae`);
    ///    - Kind 6 (`reg+0xf8`, Wardrobe / Clothing Shop, `0x4417b3..0x4419a1`);
    ///    - Kind 14 (`reg+0xfc`, Magic Charm Transform, `0x4419a6..0x441b86`);
    ///    - Kind 9 (`reg+0x104`, Pet Adoption picker, `0x441b8b..0x441df2`);
    ///    - Kind 7 (`reg+0xec`, Vacation Island lot picker, `0x441df2..0x441f6d`);
    ///    - Kind 11 (`reg+0xee`, Studio Town lot picker, `0x441f72..0x442169`);
    ///    - Kind 13 (`reg+0xf0`, Magic Town lot picker, `0x44216e..0x442320`).
    fn end_of_update_special_panels(
        &self,
        world: &mut World,
        player: &mut dyn Player,
        update: &mut Update,
    ) -> Result<(), TickError> {
        let Some(panels) = self.special_panels else {
            return Ok(());
        };
        let ask = |world: &mut World,
                   player: &mut dyn Player,
                   update: &mut Update,
                   request: SpecialPanelRequest|
         -> Result<SpecialPanelResponse, TickError> {
            let shown = ShownSpecialPanel {
                tick: world.ticks,
                request: request.clone(),
            };
            self.refresh_family_panel(world);
            match player.special_panel(&shown) {
                Some(resp) if request.accepts(&resp) => {
                    update.special_panels_answered.push((request, resp.clone()));
                    Ok(resp)
                }
                other => Err(TickError::UnansweredSpecialPanel {
                    shown,
                    given: other,
                }),
            }
        };

        // 1. Kind 10: Call Neighbor (`0x4413e1..0x4414cd`, falls through).
        let call_neighbor = panels.borrow().call_neighbor;
        if call_neighbor < 0 && call_neighbor != CALL_NEIGHBOR_CANCELLED {
            let req = SpecialPanelRequest::CallNeighbor {
                caller_neighbor: call_neighbor.wrapping_neg(),
            };
            let resp = ask(world, player, update, req)?;
            let mut sp = panels.borrow_mut();
            sp.call_neighbor = match resp {
                SpecialPanelResponse::Neighbor(Some(id)) if id > 0 => id,
                _ => CALL_NEIGHBOR_CANCELLED,
            };
        }

        // 2. Kind 5: Downtown lot picker (`0x4414cd..0x4415fd`, falls through).
        if panels.borrow().downtown_lot < 0 {
            let resp = ask(
                world,
                player,
                update,
                SpecialPanelRequest::DowntownLotPicker,
            )?;
            let mut sp = panels.borrow_mut();
            sp.downtown_lot = match resp {
                SpecialPanelResponse::Lot(Some(lot)) if (21..=30).contains(&lot) => lot,
                _ => 0,
            };
        }

        // 3. Mutually exclusive chain (`0x4415fd..0x442320`).
        let snapshot = panels.borrow().clone();
        if snapshot.old_town_lot < 0 {
            // Kind 8: Old Town / Community lot picker (`0x4415fd..0x4417ae`).
            let req = SpecialPanelRequest::OldTownLotPicker {
                unleashed: self.program.expansions & 0x20 != 0,
            };
            let resp = ask(world, player, update, req)?;
            let mut sp = panels.borrow_mut();
            sp.old_town_lot = match resp {
                SpecialPanelResponse::Lot(Some(lot))
                    if lot > 0 && self.program.zoning.get(&lot) == Some(&2) =>
                {
                    lot
                }
                _ => 0,
            };
        } else if let Some(person) = snapshot.wardrobe_caller {
            // Kind 6: Wardrobe / Clothing Shop picker (`0x4417b3..0x4419a1`).
            let req = SpecialPanelRequest::WardrobeShop {
                person,
                mode: snapshot.wardrobe_status,
            };
            panels.borrow_mut().wardrobe_index = 0;
            let resp = ask(world, player, update, req)?;
            if let SpecialPanelResponse::Selection { accepted, index } = resp {
                let mut sp = panels.borrow_mut();
                sp.wardrobe_index = index;
                sp.wardrobe_status = if accepted {
                    SPECIAL_PANEL_ACCEPTED
                } else {
                    SPECIAL_PANEL_CANCELLED
                };
            }
        } else if let Some(person) = snapshot.charm_caller {
            // Kind 14: Magic Charm Transform picker (`0x4419a6..0x441b86`).
            let req = SpecialPanelRequest::CharmTransform { person };
            panels.borrow_mut().wardrobe_index = 0;
            let resp = ask(world, player, update, req)?;
            if let SpecialPanelResponse::Selection { accepted, index } = resp {
                let mut sp = panels.borrow_mut();
                sp.wardrobe_index = index;
                sp.wardrobe_status = if accepted {
                    SPECIAL_PANEL_ACCEPTED
                } else {
                    SPECIAL_PANEL_CANCELLED
                };
            }
        } else if let Some(pet) = snapshot.pet_target {
            // Kind 9: Pet Adoption / Buy Pet picker (`0x441b8b..0x441df2`).
            // `0x441bb2..0x441bd7`: saves current speed (`0x46d78b`), sets speed
            // to `760` (`0x4425a2` -> `0x46d697`), then `0x441de8` restores the
            // saved speed through `0x46d697` (updating `return_speed` if idle
            // speed was active).
            let saved_speed = i32::from(world.speed());
            world.set_speed(760);
            let req = SpecialPanelRequest::PetPicker { pet };
            // `0x441d52`: clears reg+0xf6 before opening the modal.
            panels.borrow_mut().wardrobe_index = 0;
            let resp = ask(world, player, update, req)?;
            if let SpecialPanelResponse::Selection { accepted, index } = resp {
                world.dialogs.declined = index;
                panels.borrow_mut().pet_status = if accepted {
                    SPECIAL_PANEL_ACCEPTED
                } else {
                    SPECIAL_PANEL_CANCELLED
                };
            }
            world.set_speed(saved_speed);
        } else if snapshot.vacation_lot < 0 {
            // Kind 7: Vacation Island lot picker (`0x441df2..0x441f6d`).
            let current_lot = world.globals.get(LOT).copied().unwrap_or(0);
            let on_residential =
                current_lot == 0 || self.program.zoning.get(&current_lot) == Some(&1);
            let req = SpecialPanelRequest::VacationLotPicker { on_residential };
            let resp = ask(world, player, update, req)?;
            let mut sp = panels.borrow_mut();
            sp.vacation_lot = match resp {
                SpecialPanelResponse::Lot(Some(lot)) if (40..=48).contains(&lot) => lot,
                _ => 0,
            };
        } else if snapshot.studio_lot < 0 {
            // Kind 11: Studio Town lot picker (`0x441f72..0x442169`).
            let star_limo = snapshot
                .studio_caller
                .and_then(|id| world.objects.get(&id))
                .and_then(|o| o.person.as_ref())
                .is_some_and(|p| {
                    p.data.get(80).copied().unwrap_or(0) > 0
                        && p.data.get(56).copied().unwrap_or(0) == 0
                });
            let req = SpecialPanelRequest::StudioTownLotPicker { star_limo };
            let resp = ask(world, player, update, req)?;
            let mut sp = panels.borrow_mut();
            match resp {
                SpecialPanelResponse::Lot(Some(lot)) if (81..=89).contains(&lot) => {
                    sp.studio_lot = lot;
                    sp.studio_caller = None;
                }
                _ => {
                    sp.studio_lot = 0;
                }
            }
        } else if snapshot.magic_lot < 0 {
            // Kind 13: Magic Town lot picker (`0x44216e..0x442320`).
            // `0x4421e3..0x442219`: writes `world.globals[34] != 0` to `[dialog + 0xec]`
            // (`false` for Magic Hole `ClownPortal.iff` node 64, `true` for Magic Blimp
            // `phonepluginmagiclots.iff` node 98).
            let via_blimp = world.globals.get(34).copied().unwrap_or(0) != 0;
            let req = SpecialPanelRequest::MagicTownLotPicker { via_blimp };
            let resp = ask(world, player, update, req)?;
            let mut sp = panels.borrow_mut();
            sp.magic_lot = match resp {
                SpecialPanelResponse::Lot(Some(lot)) if (90..=98).contains(&lot) => lot,
                _ => 0,
            };
        }
        Ok(())
    }
}

/// Below 0 or not a number: the carry flag an x87 compare with 0 sets.
fn below_zero(value: sims_core::F32) -> bool {
    !matches!(
        value.ext().partial_cmp(&sims_core::Ext::ZERO),
        Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
    )
}

/// A room by ID (0x466fcc).
fn world_room(rooms: &BTreeMap<i16, Room>, id: i16) -> Option<&Room> {
    rooms.get(&id)
}

fn global(world: &World, index: usize) -> Result<i16, TickError> {
    world
        .globals
        .get(index)
        .copied()
        .ok_or_else(|| TickError::Unsupported(format!("the lot has no global {index}")))
}

fn set_global(world: &mut World, index: usize, value: i16) -> Result<(), TickError> {
    let slot = world
        .globals
        .get_mut(index)
        .ok_or_else(|| TickError::Unsupported(format!("the lot has no global {index}")))?;
    *slot = value;
    Ok(())
}
