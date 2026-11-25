//! Dialog (opcode 36): ask the player something and read back the answer.
//!
//! Read from the original's 0x451552. The instruction runs again every
//! tick until it has an answer:
//!
//! 1. It looks up the first dialog record the object owns (0x44435c). A
//!    record that was shown and not answered (status 1) is closed
//!    (0x437058 sets status 5), and a closed one counts as none.
//! 2. With operand 7 bit 0 set the dialog only informs: a record is made
//!    (0x4443ab) and the true branch taken at once.
//! 3. Otherwise, with no record, a record is made and the instruction
//!    yields. The registry shows the record at the end of the lot update
//!    (`tick.rs`), and the player's answer goes into its status.
//! 4. With a record, the record is acknowledged (0x43718f: status 2 to 4
//!    becomes 5) and the status read: not answered yet (0 or 1) yields
//!    again; the first button (2) takes the true branch; the second (3)
//!    takes the false branch, clears the temp operand 7 names in a
//!    three-button dialog, and sets registry word 0x100 to -1; the third
//!    (4) takes the false branch and sets that temp to 1.
//!
//! Operand 5 (bits 0 to 6) is the kind of dialog:
//! - Kinds 0, 3, and 4 have one button (kind 3 also asks for text and copies
//!   it to `[0x6e4ac0]` at `0x451a6b..0x451a85`), kind 1 two, and kind 2 three.
//! - Kinds 5 to 15 are the game's special panels (`0x4515ae..0x4519ad` in
//!   `Dialog`, paired with `0x4413e1..0x442320` in `end_of_update`):
//!   - 5 (`reg+0xea`, `0x4515b3`, `0x4414cd`): Downtown lot picker (`21..=30`);
//!   - 6 (`reg+0xf4`/`+0xf6`/`+0xf8`, `0x451657`, `0x4417b3`): Wardrobe / shop;
//!   - 7 (`reg+0xec`, `0x4515f0`, `0x441df2`): Vacation Island lot picker (`40..=48`);
//!   - 8 (`reg+0xf2`, `0x4515df`, `0x4415fd`): Old Town / Community lot picker (`zoning == 2`);
//!   - 9 (`reg+0x100`/`+0x104`/`+0x108`, `0x45181c`, `0x441b8b`): Pet Adoption picker;
//!   - 10 (`reg+0xe8`, `0x45179a`, `0x4413e1`): Phone / Call Neighbor picker;
//!   - 11 (`reg+0xee`/`+0x10c`, `0x451601`, `0x441f72`): Studio Town lot picker (`81..=89`);
//!   - 12 (`0x4518e2` -> `0x5107dc(person, 0)`): Magic Spellbook (inline modal);
//!   - 13 (`reg+0xf0`, `0x451612`, `0x44216e`): Magic Town lot picker (`90..=98`);
//!   - 14 (`reg+0xf4`/`+0xf6`/`+0xfc`, `0x4516f9`, `0x4419a6`): Magic Charm Transform picker;
//!   - 15 (`0x451946` -> `0x5107dc(person, 1)`): Cookbook (inline modal).
//!
//! What an ordinary dialog says and shows (its text, pictures and buttons' labels) is
//! presentation and is not modeled, except where showing a person's
//! portrait builds it ([`Machine::dialog_portrait`]): building one for a
//! person type with nobody on the lot makes a person and removes it at
//! once. Making a record is otherwise assumed not to run any object's
//! code or change any simulation state: **unverified**.
//!
//! In the original, step 3 makes no record while the interface has a
//! modal dialog open (interface word 0x7c, 0x451a8e). Here modal dialogs
//! are shown and answered inside the end of the lot update, so none is
//! open while scripts run.
//!
//! The evidence (`docs/decisions/`): on the Goth house five controllers
//! reach a one-button dialog in the first tick and yield; the run of the
//! original answered each, and in the next tick each took the true
//! branch. Object 15 then made a two-button dialog, answered with the
//! second button, and took the false branch the tick after. The rest of
//! the protocol follows the original's code and the old prototype's
//! `Protocol::step`.

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine};
use sims_sim::Dialog;

/// A definition's type word (+0x12), and its value for a person.
const TYPE: usize = 9;
const PERSON_TYPE: i16 = 2;
/// Definition word 41 (+0x52): the catalog string table (0x457d5f).
const CATALOG_STRINGS: usize = 41;

/// The tokens of the formatter's passes between `$Local`'s and
/// `$NeighborLocal`'s (0x44bf1a), in the order they run: `$Attrib`
/// (0x44c280), `$NameAttrib` (0x44c330), `$NameLocal` (0x44c3ff) and
/// `$TokenNameLocal` (0x44c490). None is ported, and a text that holds one
/// when they run fails loudly: they read the running tree's locals and
/// the stack object's attributes, and fault or throw for some (0x44c2dd,
/// 0x44c381, 0x44c4f6), and `$TokenNameLocal`'s may add an inventory
/// (0x42a8a1).
const BETWEEN: [&str; 4] = ["$Attrib", "$NameAttrib", "$NameLocal", "$TokenNameLocal"];

/// The tokens of the formatter's passes after `$Neighbor`'s and before
/// `$TimeLocal`'s, in the order they run: `$CatalogLocal` (0x6e4fa0, from
/// 0x6c40f0; 0x44c6ea), `$FameTitleLocal` (0x6e4f90, from 0x6c4100;
/// 0x44c797) and `$Grade` (0x6e5000; 0x44c861). None is ported, and a text
/// that holds one when they run fails loudly: the text they write is not
/// known to end their passes or to make no later token. `$CatalogLocal`
/// writes a catalog name (0x4573de, 0x44c771), which may hold its token
/// again, and `$FameTitleLocal` (0x405241, 0x44c83d) and `$Grade`
/// (0x4034ce; empty for some numbers, 0x44c8ae) may join the text around
/// them into a career token, which faults for an object that is not a
/// person (0x44ca27).
const AFTER: [&str; 3] = ["$CatalogLocal", "$FameTitleLocal", "$Grade"];

/// The career tokens, in the order their loop (0x44c966) runs them:
/// `$JobOffer` (0x6e5010), `$JobDesc` (0x6e4ff0), `$Job` (0x6e5040).
const CAREER: [&str; 3] = ["$JobOffer", "$JobDesc", "$Job"];

/// Whether `text`, written for a career token, may hold one past its
/// first character, or end in the start of one: a pass searches again
/// from one past the place it wrote (0x44ca9f to 0x44cad3), so it would
/// find the token again and might never end. [`crate::Program::career_texts_end`]
/// is whether no text the career passes may write does.
pub(crate) fn may_hold_career_token(text: &str) -> bool {
    text.char_indices().skip(1).any(|(i, _)| {
        let rest = &text[i..];
        rest.starts_with('$')
            && CAREER
                .iter()
                .any(|token| rest.starts_with(token) || token.starts_with(rest))
    })
}

/// `text` with each `token` replaced by `name` as the formatter's passes
/// do it (0x4558a0, 0x455750): from the start, then from one past each
/// place replaced. Never called with a name for which [`never_ends`].
fn replace_each(text: &str, token: &str, name: &str) -> String {
    let mut text = text.to_owned();
    let mut from = 0;
    while let Some(at) = text
        .get(from..)
        .and_then(|t| t.find(token))
        .map(|i| i + from)
    {
        text.replace_range(at..at + token.len(), name);
        from = at + text[at..].chars().next().map_or(1, char::len_utf8);
    }
    text
}

/// Whether a name holds `token` past its first character. Replacing the
/// token with it ([`replace_each`]) never ends in the original: each
/// search goes on from one past the place replaced (0x4558a0, 0x455750),
/// and finds the token the name put back. A token made with the text
/// after the name uses some of that text up, so it ends.
fn never_ends(token: &str, name: &str) -> bool {
    name.char_indices()
        .skip(1)
        .any(|(i, _)| name[i..].starts_with(token))
}

/// Outcome of formatting one string with `0x44bf1a`.
pub(crate) struct Formatted {
    /// The formatted text left by the passes.
    pub(crate) text: String,
    /// The portrait type written to the formatter's output slot
    /// (`[ebp+0x14]`): `None` keeps what a previous string wrote;
    /// `Some(guid)` replaces it.
    pub(crate) portrait: Option<u32>,
}

/// Replaces each `token` in `text` with `name` (`$Me`'s, `$Object`'s,
/// `$Neighbor`'s, at `at`), from the start and then from one past each
/// place replaced ([`replace_each`]). Fails loudly when the name is not
/// known (`None`), and when it holds the token past its first character,
/// so the pass would never end ([`never_ends`]).
fn name_pass(
    text: String,
    token: &str,
    name: Option<&str>,
    what: &str,
    at: &str,
) -> Result<String, Trap> {
    if !text.contains(token) {
        return Ok(text);
    }
    let Some(name) = name else {
        return Err(Trap::Unsupported(format!(
            "{what} `{token}` is replaced by a name that is not known (0x44bf1a, 0x4573d9, \
             {at}), and a name that holds `{token}` past its first character would never end \
             (0x4558a0, 0x455750): not ported"
        )));
    };
    if never_ends(token, name) {
        return Err(Trap::Unsupported(format!(
            "{what} `{token}` is replaced by the name {name:?}, which holds it past its first \
             character, so the pass never ends (0x44bf1a, {at}, 0x4558a0, 0x455750)"
        )));
    }
    Ok(replace_each(&text, token, name))
}

/// Fails loudly, naming `token` and its pass, when `text` holds one of
/// `tokens`, whose passes are not ported.
fn not_ported(text: &str, tokens: &[&str], what: &str, passes: &str) -> Result<(), Trap> {
    for token in tokens {
        if text.contains(*token) {
            return Err(Trap::Unsupported(format!(
                "{what} text holds `{token}` when its pass runs (0x44bf1a, {passes}): not ported \
                 ({text:?})"
            )));
        }
    }
    Ok(())
}

/// The two numbers of a `$Local`, `$NeighborLocal`, `$TimeLocal`, or `$Job*`
/// token and where it ends, read as 0x44caf6 reads it from `end`, just after
/// the token's name: a `:` and digits, and an optional second `:` and digits.
/// Each number is its first 15 digits (0x44cb52 makes a string of 16 bytes
/// for them, and 0x46f925 adds no more than fit, 0x46f78d to 0x46f79d), by
/// the C runtime's 32-bit conversion (0x636029), which wraps; -1 when there
/// is none (0x44cb34, 0x44cb3e).
///
/// The digit test (0x636993) is handed each byte sign-extended (0x44cb70,
/// 0x44cbc3), so a byte above 0x7f is looked up below the C runtime's table,
/// and some pass as digits that the conversion reads as none: where the scan
/// meets one it fails loudly, naming `token`.
fn local_token(
    bytes: &[u8],
    mut end: usize,
    what: &str,
    token: &str,
) -> Result<(i32, i32, usize), Trap> {
    let mut numbers = [-1i32; 2];
    for slot in &mut numbers {
        if bytes.get(end) != Some(&b':') {
            break;
        }
        end += 1;
        let digits = bytes[end..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if bytes.get(end + digits).is_some_and(|b| !b.is_ascii()) {
            return Err(Trap::Unsupported(format!(
                "{what} `{token}` is read with digits up to a byte above 0x7f, which the \
                 original's digit test (0x636993) reads sign-extended (0x44bf1a, 0x44cb70): not \
                 ported"
            )));
        }
        if digits > 0 {
            *slot = bytes[end..end + digits].iter().take(15).fold(0i32, |n, d| {
                n.wrapping_mul(10).wrapping_add(i32::from(d - b'0'))
            });
        }
        end += digits;
    }
    Ok((numbers[0], numbers[1], end))
}

/// Formats an hour in `[0, 24)` as `0x4a6ab1` -> `0x5fe51e` does in US
/// English (language 1): `sprintf("%2u:%.2u %s", display_hour, 0, suffix)`
/// (`0x6d8498`, `0x6d84b8`, `0x6d84bc`, `0x5d4f07`).
fn format_hour(hour: i16) -> String {
    let suffix = if hour < 12 { "am" } else { "pm" };
    let display = if hour == 0 {
        12
    } else if hour > 12 {
        hour - 12
    } else {
        hour
    };
    format!("{display:2}:00 {suffix}")
}

/// Record status: made and not shown yet.
pub const NEW: u8 = 0;
/// Shown, not answered.
pub const SHOWN: u8 = 1;
/// Answered with the first, second or third button.
pub const FIRST: u8 = 2;
pub const SECOND: u8 = 3;
pub const THIRD: u8 = 4;
/// Done: the registry destroys it at the end of the lot update.
pub const DONE: u8 = 5;

/// Sentinel written to `reg+0xe8` (`SpecialPanels::call_neighbor`) when the
/// player cancels the Call Neighbor picker (`0x4414b4`: `0xd8f1` = `-9999`).
pub const CALL_NEIGHBOR_CANCELLED: i16 = -9999;
/// Low-word status returned by modal 3D preview pickers (kinds 6, 9, 14) when
/// accepted (`0x5098bd`, `0x522073`, `0x5373ee`: `0xfffe` = `-2`).
pub const SPECIAL_PANEL_ACCEPTED: i16 = -2;
/// Low-word status returned by modal 3D preview pickers (kinds 6, 9, 14) when
/// cancelled (`0x5098db`, `0x522091`, `0x53740b`: `0xffff` = `-1`).
pub const SPECIAL_PANEL_CANCELLED: i16 = -1;

/// Person data word 31 (`+0x5ca`): the person's neighbor ID (`0x4517e4`).
const PERSON_NEIGHBOR: usize = 31;

/// Registry special-panel fields (`cObjectRegistry` offsets `+0xe8..=+0x10c`,
/// initialized in `0x43f628..0x43f72d`).
///
/// Note: registry word `+0x100` (pet picker result / declined sentinel) lives
/// in [`sims_sim::Dialogs::declined`] so `Make New Character` (`0x44b064`)
/// shares it directly with `Dialog` kind 9 (`0x451899`, `0x441d95`).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SpecialPanels {
    /// `+0xe8`: kind 10 (Phone / Call Neighbor). `0` idle; `< 0` (except
    /// `-9999`) pending (`-caller_neighbor`); `> 0` chosen neighbor ID;
    /// `-9999` (`0xd8f1`) cancelled.
    pub call_neighbor: i16,
    /// `+0xea`: kind 5 (Downtown lot picker). `0` idle; `-1` pending; `21..=30` picked lot.
    pub downtown_lot: i16,
    /// `+0xec`: kind 7 (Vacation Island lot picker). `0` idle; `-1` pending; `40..=48` picked lot.
    pub vacation_lot: i16,
    /// `+0xee`: kind 11 (Studio Town lot picker). `0` idle; `-1` pending; `81..=89` picked lot.
    pub studio_lot: i16,
    /// `+0xf0`: kind 13 (Magic Town lot picker). `0` idle; `-1` pending; `90..=98` picked lot.
    pub magic_lot: i16,
    /// `+0xf2`: kind 8 (Old Town / Community lot picker). `0` idle; `-1` pending; community lot ID.
    pub old_town_lot: i16,
    /// `+0xf4`: kind 6 input wardrobe mode / kind 6 & 14 modal status (`-2` accepted).
    pub wardrobe_status: i16,
    /// `+0xf6`: kind 14 input `temp 0` / kind 6 & 14 modal result index.
    pub wardrobe_index: i16,
    /// `+0xf8`: kind 6 pending person object ID.
    pub wardrobe_caller: Option<u16>,
    /// `+0xfc`: kind 14 pending person object ID.
    pub charm_caller: Option<u16>,
    /// `+0x104`: kind 9 pending pet person object ID (from `stack_object`).
    pub pet_target: Option<u16>,
    /// `+0x108`: kind 9 modal status (`-2` accepted).
    pub pet_status: i16,
    /// `+0x10c`: kind 11 Studio Town star-limo subject person ID (set by
    /// Generic Sims Call routine 35 at `0x451197`, cleared on valid pick at `0x442152`).
    pub studio_caller: Option<u16>,
    /// UI transition/panel busy flag `[0x6f2a58] + 0xf8` checked by Dialog kind 13
    /// (`0x451628..0x451634`). While `true`, kind 13 yields without arming
    /// `magic_lot = -1` when `magic_lot == 0`.
    pub magic_picker_blocked: bool,
}

/// Typed special panel request surfaced by `Dialog` kinds `5..=15` (`0x451552`
/// and `0x4413e1..0x442320`) so `sims-game` can display the appropriate native
/// panel or trigger travel (`sims_vm::travel::depart_to_destination`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpecialPanelRequest {
    /// Kind 5 (`reg+0xea`, `0x4414cd` -> `0x549dae`): Hot Date Downtown lot picker (`21..=30`).
    DowntownLotPicker,
    /// Kind 6 (`reg+0xf4`/`+0xf6`/`+0xf8`, `0x4417b3` -> `0x508f22`): Wardrobe /
    /// clothing shop picker. `mode` is the input `temp 0` (`0` everyday, `1` formal,
    /// `2` swimwear, `3` pajamas, `4` winter, `5` high-fashion at `0x5097c9`).
    WardrobeShop { person: u16, mode: i16 },
    /// Kind 7 (`reg+0xec`, `0x441df2` -> `0x5686e9`): Vacation Island lot picker (`40..=48`).
    /// `on_residential` mirrors `0x40745f` (`0x441f34`, loading transition 2 vs 1).
    VacationLotPicker { on_residential: bool },
    /// Kind 8 (`reg+0xf2`, `0x4415fd` -> `0x556721`/`0x4425a8`): Old Town / Community lot
    /// picker (any lot with `LotZoning == 2` per `0x42b9e3`).
    OldTownLotPicker { unleashed: bool },
    /// Kind 9 (`reg+0x100`/`+0x104`/`+0x108`, `0x441b8b` -> `0x521788`): Unleashed Pet
    /// Adoption / Buy Pet picker for stack-object person `pet`.
    PetPicker { pet: u16 },
    /// Kind 10 (`reg+0xe8`, `0x4413e1` -> `0x5221d0`): Phone / Call Neighbor picker.
    /// `caller_neighbor` is the calling Sim's `person.data[31]` (`PERSON_NEIGHBOR`).
    CallNeighbor { caller_neighbor: i16 },
    /// Kind 11 (`reg+0xee`/`+0x10c`, `0x441f72` -> `0x5644b5`): Superstar Studio Town lot
    /// picker (`81..=89`). `star_limo` reflects `reg+0x10c` (`person.data[80] > 0 &&
    /// person.data[56] == 0` at `0x441f94..0x441ffa`).
    StudioTownLotPicker { star_limo: bool },
    /// Kind 12 (`0x4518e2` -> `0x5107dc(person, 0)`): Makin' Magic Spellbook reference
    /// modal, opened synchronously inside `Dialog` and always taking the True branch.
    /// `caller` is `Some(me)` when `me` casts to `cXPerson` (`0x635abe`), or `None`
    /// (passing `NULL` to `0x5107dc`) for a non-person caller.
    SpellBook { caller: Option<u16> },
    /// Kind 13 (`reg+0xf0`, `0x44216e` -> `0x551285`): Makin' Magic Town lot picker
    /// (`90..=98`). `via_blimp` reflects `world.globals[34] != 0` (`0x4421e3..0x442219`,
    /// written to `[dialog + 0xec]` and read at `0x5545ee`: `false` for the Magic Hole
    /// `ClownPortal.iff` node 64, `true` for the Magic Blimp `phonepluginmagiclots.iff`
    /// node 98).
    MagicTownLotPicker { via_blimp: bool },
    /// Kind 14 (`reg+0xf4`/`+0xf6`/`+0xfc`, `0x4419a6` -> `0x536a6b`): Makin' Magic
    /// Charm Transform picker for `person`.
    CharmTransform { person: u16 },
    /// Kind 15 (`0x451946` -> `0x5107dc(person, 1)`): Makin' Magic Cookbook reference
    /// modal, opened synchronously inside `Dialog` and always taking the True branch.
    /// `caller` is `Some(me)` when `me` casts to `cXPerson` (`0x635abe`), or `None`
    /// (passing `NULL` to `0x5107dc`) for a non-person caller.
    CookBook { caller: Option<u16> },
}

impl SpecialPanelRequest {
    /// The `Dialog` operand 5 kind (`5..=15`) corresponding to this panel.
    #[must_use]
    pub const fn kind(&self) -> u8 {
        match self {
            Self::DowntownLotPicker => 5,
            Self::WardrobeShop { .. } => 6,
            Self::VacationLotPicker { .. } => 7,
            Self::OldTownLotPicker { .. } => 8,
            Self::PetPicker { .. } => 9,
            Self::CallNeighbor { .. } => 10,
            Self::StudioTownLotPicker { .. } => 11,
            Self::SpellBook { .. } => 12,
            Self::MagicTownLotPicker { .. } => 13,
            Self::CharmTransform { .. } => 14,
            Self::CookBook { .. } => 15,
        }
    }

    /// Subject/caller object ID associated with this panel request, if any.
    #[must_use]
    pub const fn caller_object(&self) -> Option<u16> {
        match *self {
            Self::WardrobeShop { person, .. } | Self::CharmTransform { person } => Some(person),
            Self::PetPicker { pet } => Some(pet),
            Self::SpellBook { caller } | Self::CookBook { caller } => caller,
            Self::DowntownLotPicker
            | Self::VacationLotPicker { .. }
            | Self::OldTownLotPicker { .. }
            | Self::CallNeighbor { .. }
            | Self::StudioTownLotPicker { .. }
            | Self::MagicTownLotPicker { .. } => None,
        }
    }

    /// Whether this panel is a dismiss-only reference modal (Spellbook / Cookbook).
    #[must_use]
    pub const fn is_dismiss_only(&self) -> bool {
        matches!(self, Self::SpellBook { .. } | Self::CookBook { .. })
    }

    /// Whether `response` is the matching response variant for this panel request.
    #[must_use]
    pub const fn accepts(&self, response: &SpecialPanelResponse) -> bool {
        match self {
            Self::DowntownLotPicker
            | Self::VacationLotPicker { .. }
            | Self::OldTownLotPicker { .. }
            | Self::StudioTownLotPicker { .. }
            | Self::MagicTownLotPicker { .. } => matches!(response, SpecialPanelResponse::Lot(_)),
            Self::CallNeighbor { .. } => matches!(response, SpecialPanelResponse::Neighbor(_)),
            Self::WardrobeShop { .. } | Self::PetPicker { .. } | Self::CharmTransform { .. } => {
                matches!(response, SpecialPanelResponse::Selection { .. })
            }
            Self::SpellBook { .. } | Self::CookBook { .. } => {
                matches!(response, SpecialPanelResponse::Dismissed)
            }
        }
    }
}

/// The player's response to a [`SpecialPanelRequest`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpecialPanelResponse {
    /// Lot chosen in a neighborhood destination picker (kinds 5, 7, 8, 11, 13),
    /// or `None` if cancelled / closed without picking a valid lot.
    Lot(Option<i16>),
    /// Neighbor ID chosen in the Phone / Call Neighbor picker (kind 10), or
    /// `None` (or `<= 0`) if cancelled.
    Neighbor(Option<i16>),
    /// Outfit, charm costume, or pet body index chosen in a 3D preview picker
    /// (kinds 6, 9, 14). `accepted == true` packs `-2` (`0xfffe`, True branch);
    /// `accepted == false` packs `-1` (`0xffff`, False branch).
    Selection { accepted: bool, index: i16 },
    /// Reference book modal closed (kinds 12 Spellbook and 15 Cookbook).
    Dismissed,
}

/// Covariant sink interface allowing inline reference modals (kinds 12 & 15,
/// `0x4518e2`, `0x451946`) inside `Machine::dialog` to invoke
/// `Player::special_panel` synchronously at any call depth (top-level threads
/// and synchronous `Run Tree By Name` callbacks) while keeping `Interpreter<'a>`
/// covariant in `'a`.
pub(crate) trait InlinePanelSink: std::fmt::Debug {
    /// Invokes the player synchronously for an inline book request. Returns
    /// `true` when accepted (`Dismissed`) and `false` when unanswered/rejected
    /// (recording the [`crate::tick::TickError`] to unwind the tick).
    fn ask_inline(&self, request: SpecialPanelRequest) -> bool;

    /// Whether an inline book modal suspended this pass awaiting player input.
    fn has_unanswered(&self) -> bool;

    /// Takes any recorded unanswered inline modal error.
    fn take_unanswered(&self) -> Option<crate::tick::TickError>;
}

struct InlineSpecialPanelState<'p> {
    tick: u32,
    player: &'p mut dyn crate::tick::Player,
    answered: Vec<(SpecialPanelRequest, SpecialPanelResponse)>,
    unanswered: Option<crate::tick::TickError>,
}

/// Concrete per-pass [`InlinePanelSink`] wrapping a [`crate::tick::Player`].
pub(crate) struct InlineSpecialPanel<'p> {
    state: std::cell::RefCell<InlineSpecialPanelState<'p>>,
}

impl std::fmt::Debug for InlineSpecialPanel<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.borrow();
        f.debug_struct("InlineSpecialPanel")
            .field("tick", &state.tick)
            .field("answered", &state.answered.len())
            .field("has_unanswered", &state.unanswered.is_some())
            .finish()
    }
}

impl<'p> InlineSpecialPanel<'p> {
    pub(crate) fn new(tick: u32, player: &'p mut dyn crate::tick::Player) -> Self {
        Self {
            state: std::cell::RefCell::new(InlineSpecialPanelState {
                tick,
                player,
                answered: Vec::new(),
                unanswered: None,
            }),
        }
    }

    pub(crate) fn drain_answered(
        &self,
        out: &mut Vec<(SpecialPanelRequest, SpecialPanelResponse)>,
    ) {
        out.append(&mut self.state.borrow_mut().answered);
    }
}

impl InlinePanelSink for InlineSpecialPanel<'_> {
    fn ask_inline(&self, request: SpecialPanelRequest) -> bool {
        let mut state = self.state.borrow_mut();
        let shown = crate::tick::ShownSpecialPanel {
            tick: state.tick,
            request: request.clone(),
        };
        match state.player.special_panel(&shown) {
            Some(resp) if request.accepts(&resp) => {
                state.answered.push((request, resp));
                true
            }
            other => {
                state.unanswered = Some(crate::tick::TickError::UnansweredSpecialPanel {
                    shown,
                    given: other,
                });
                false
            }
        }
    }

    fn has_unanswered(&self) -> bool {
        self.state.borrow().unanswered.is_some()
    }

    fn take_unanswered(&self) -> Option<crate::tick::TickError> {
        self.state.borrow_mut().unanswered.take()
    }
}

/// The answers an ordinary dialog of this kind (operand 5) offers, as statuses,
/// or None for special panels (kinds 5..=15) and unknown kinds. From the
/// original's button setup (0x43601c, 0x436a52), as the old prototype read it
/// (legacy/crates/sims-vm/src/dialog/presentation.rs, `buttons`).
pub fn answers(kind: u8) -> Option<&'static [u8]> {
    match kind & 0x7f {
        // Kind 3 asks for text (`0x436a52`, `0x436c43..0x436c76`): one button
        // (FIRST / OK) plus a single-line text entry field; accepting copies
        // the text to global `[0x6e4ac0]` (`0x451a6b..0x451a85`) and takes
        // the true branch.
        0 | 3 | 4 => Some(&[FIRST]),
        1 => Some(&[FIRST, SECOND]),
        2 => Some(&[FIRST, SECOND, THIRD]),
        _ => None,
    }
}

/// Whether a record's dialog stops the game until it is answered (operand
/// 7 bit 7 clear; record byte 0x69).
pub fn modal(operands: &[u8; 8]) -> bool {
    operands[7] & 0x80 == 0
}

/// Whether a record only informs: it is answered once and is done (operand
/// 7 bit 0).
pub fn informs(operands: &[u8; 8]) -> bool {
    operands[7] & 1 != 0
}

impl Machine<'_, '_> {
    pub(crate) fn dialog(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        let kind = operands[5] & 0x7f;
        let me = self.me;
        // `0x451562..0x45158d` runs unconditionally before the `informs` check
        // (`0x451593`) and the `kind` dispatch (`0x4515ae..0x4515b1`).
        let records = &mut self.world.dialogs.records;
        let mut found = records.iter().position(|record| record.caller == me);
        if let Some(index) = found {
            match records[index].status {
                SHOWN => {
                    records[index].status = DONE;
                    found = None;
                }
                DONE => found = None,
                _ => {}
            }
        }
        // `0x451593..0x4515a9`: when `informs` (operand 7 bit 0) is set, call
        // `0x4443ab` and take the true branch immediately. `0x4443ab` has
        // dedicated neighborhood-picker constructors for kinds 5, 7, 8, 11, 13
        // (skipping `0x43601c` / portrait formatting) and routes ordinary kinds
        // `0..=4` through `0x435d8a` + `0x43601c`.
        if informs(operands) {
            let record = Dialog {
                caller: me,
                operands: *operands,
                status: NEW,
            };
            match kind {
                0..=4 => {
                    self.dialog_portrait(operands)?;
                    self.world.dialogs.records.push(record);
                    return Ok(Flow::Branch(true));
                }
                5 | 7 | 8 | 11 | 13 => {
                    self.world.dialogs.records.push(record);
                    return Ok(Flow::Branch(true));
                }
                _ => {
                    return Err(Trap::Unsupported(format!(
                        "Dialog kind {kind} (operand 5) with informs flag (operand 7 bit 0) is \
                         not ported (0x4443ab)"
                    )));
                }
            }
        }
        if kind > 15 {
            return Err(Trap::Unsupported(format!(
                "Dialog kind {kind} (operand 5) is unknown, which is not ported (0x451552)"
            )));
        }
        match kind {
            5 => return self.dialog_lot_picker(5, false, |sp| &mut sp.downtown_lot),
            7 => return self.dialog_lot_picker(7, false, |sp| &mut sp.vacation_lot),
            8 => return self.dialog_lot_picker(8, false, |sp| &mut sp.old_town_lot),
            11 => return self.dialog_lot_picker(11, false, |sp| &mut sp.studio_lot),
            13 => return self.dialog_lot_picker(13, true, |sp| &mut sp.magic_lot),
            6 => return self.dialog_wardrobe(),
            14 => return self.dialog_charm_transform(),
            10 => return self.dialog_call_neighbor(),
            9 => return self.dialog_pet_picker(),
            12 => {
                let caller = self.me()?.person.is_some().then_some(me);
                return self.dialog_reference_book(SpecialPanelRequest::SpellBook { caller });
            }
            15 => {
                let caller = self.me()?.person.is_some().then_some(me);
                return self.dialog_reference_book(SpecialPanelRequest::CookBook { caller });
            }
            _ => {}
        }
        let record = Dialog {
            caller: me,
            operands: *operands,
            status: NEW,
        };
        let Some(index) = found else {
            self.dialog_portrait(operands)?;
            self.world.dialogs.records.push(record);
            return Ok(Flow::Yield);
        };
        let records = &mut self.world.dialogs.records;
        let status = records[index].status;
        if (FIRST..=THIRD).contains(&status) {
            records[index].status = DONE;
        }
        let temp = usize::from((operands[7] >> 4) & 7);
        Ok(match status {
            NEW | SHOWN => Flow::Yield,
            FIRST => Flow::Branch(true),
            SECOND => {
                if kind == 2 {
                    self.set_temp(temp, 0)?;
                }
                self.world.dialogs.declined = -1;
                Flow::Branch(false)
            }
            THIRD => {
                if kind == 2 {
                    self.set_temp(temp, 1)?;
                }
                Flow::Branch(false)
            }
            _ => Flow::Branch(true),
        })
    }

    /// Kinds 5 (`reg+0xea`), 7 (`reg+0xec`), 8 (`reg+0xf2`), 11 (`reg+0xee`),
    /// and 13 (`reg+0xf0`) at `0x4515b3..0x451656`: if the slot is 0, arm it
    /// to -1 and yield (except kind 13 when `magic_picker_blocked` / `[0x6f2a58]+0xf8`
    /// is set at `0x451628..0x451634`, which yields without arming `-1`);
    /// otherwise copy the picked lot into `temp 0`, clear the slot to 0, and
    /// take the true branch.
    fn dialog_lot_picker(
        &mut self,
        kind: u8,
        check_magic_blocked: bool,
        slot: impl FnOnce(&mut SpecialPanels) -> &mut i16,
    ) -> Result<Flow, Trap> {
        let Some(panels) = self.special_panels else {
            return Err(Trap::Unsupported(format!(
                "Dialog kind {kind} (operand 5) requires Interpreter::special_panels (0x451552)"
            )));
        };
        let value = {
            let mut panels = panels.borrow_mut();
            let blocked = check_magic_blocked && panels.magic_picker_blocked;
            let field = slot(&mut panels);
            let value = *field;
            if value == 0 {
                if !blocked {
                    *field = -1;
                }
                return Ok(Flow::Yield);
            }
            *field = 0;
            value
        };
        self.set_temp(0, value)?;
        Ok(Flow::Branch(true))
    }

    /// Kind 6 (`0x451657..0x4516f8`): Wardrobe / Clothing Shop picker
    /// (`reg+0xf4`, `+0xf6`, `+0xf8`).
    fn dialog_wardrobe(&mut self) -> Result<Flow, Trap> {
        if self.me()?.person.is_none() {
            return Err(Trap::native(0x1c, "object is not a person"));
        }
        let Some(panels) = self.special_panels else {
            return Err(Trap::Unsupported(
                "Dialog kind 6 (operand 5) requires Interpreter::special_panels (0x451657)".into(),
            ));
        };
        let me = self.me;
        let mode = *self
            .me()?
            .temps
            .first()
            .ok_or(Trap::native(2, "temp out of range"))?;
        let outcome = {
            let mut sp = panels.borrow_mut();
            match sp.wardrobe_caller {
                None => {
                    sp.wardrobe_status = mode;
                    sp.wardrobe_caller = Some(me);
                    None
                }
                Some(owner) if owner == me => {
                    let index = sp.wardrobe_index;
                    let accepted = sp.wardrobe_status == SPECIAL_PANEL_ACCEPTED;
                    sp.wardrobe_status = 0;
                    sp.wardrobe_caller = None;
                    Some((index, accepted))
                }
                Some(_) => {
                    sp.wardrobe_status = 0;
                    sp.wardrobe_caller = None;
                    Some((0, false))
                }
            }
        };
        match outcome {
            None => Ok(Flow::Yield),
            Some((index, accepted)) => {
                self.set_temp(0, index)?;
                Ok(Flow::Branch(accepted))
            }
        }
    }

    /// Kind 14 (`0x4516f9..0x451799`): Magic Charm Transform picker
    /// (`reg+0xf4`, `+0xf6`, `+0xfc`).
    fn dialog_charm_transform(&mut self) -> Result<Flow, Trap> {
        if self.me()?.person.is_none() {
            return Err(Trap::native(0x1c, "object is not a person"));
        }
        let Some(panels) = self.special_panels else {
            return Err(Trap::Unsupported(
                "Dialog kind 14 (operand 5) requires Interpreter::special_panels (0x4516f9)".into(),
            ));
        };
        let me = self.me;
        let input = *self
            .me()?
            .temps
            .first()
            .ok_or(Trap::native(2, "temp out of range"))?;
        let outcome = {
            let mut sp = panels.borrow_mut();
            match sp.charm_caller {
                None => {
                    sp.wardrobe_index = input;
                    sp.charm_caller = Some(me);
                    None
                }
                Some(owner) if owner == me => {
                    let index = sp.wardrobe_index;
                    let accepted = sp.wardrobe_status == SPECIAL_PANEL_ACCEPTED;
                    sp.wardrobe_index = 0;
                    sp.charm_caller = None;
                    Some((index, accepted))
                }
                Some(_) => {
                    sp.wardrobe_index = 0;
                    sp.charm_caller = None;
                    Some((0, false))
                }
            }
        };
        match outcome {
            None => Ok(Flow::Yield),
            Some((index, accepted)) => {
                self.set_temp(0, index)?;
                Ok(Flow::Branch(accepted))
            }
        }
    }

    /// Kind 10 (`0x45179a..0x45181b`): Phone / Call Neighbor picker (`reg+0xe8`).
    fn dialog_call_neighbor(&mut self) -> Result<Flow, Trap> {
        let neighbor_id = self
            .me()?
            .person
            .as_ref()
            .ok_or(Trap::native(0x1c, "object is not a person"))?
            .data
            .get(PERSON_NEIGHBOR)
            .copied()
            .unwrap_or(0);
        let Some(panels) = self.special_panels else {
            return Err(Trap::Unsupported(
                "Dialog kind 10 (operand 5) requires Interpreter::special_panels (0x45179a)".into(),
            ));
        };
        let value = {
            let mut sp = panels.borrow_mut();
            let v = sp.call_neighbor;
            if v == 0 {
                sp.call_neighbor = neighbor_id.wrapping_neg();
                return Ok(Flow::Yield);
            }
            sp.call_neighbor = 0;
            v
        };
        if value > 0 {
            self.set_temp(0, value)?;
            Ok(Flow::Branch(true))
        } else {
            Ok(Flow::Branch(false))
        }
    }

    /// Kind 9 (`0x45181c..0x4518e1`): Unleashed Pet Adoption / Buy Pet picker
    /// (`reg+0x100` in [`sims_sim::Dialogs::declined`], `reg+0x104`, `reg+0x108`).
    fn dialog_pet_picker(&mut self) -> Result<Flow, Trap> {
        let stack_object = self.frame()?.stack_object;
        let pet_id = u16::try_from(stack_object)
            .ok()
            .filter(|&id| {
                id > 0
                    && self
                        .world
                        .objects
                        .get(&id)
                        .is_some_and(|o| o.person.is_some())
            })
            .ok_or(Trap::native(0x1c, "stack object is not a person"))?;
        let Some(panels) = self.special_panels else {
            return Err(Trap::Unsupported(
                "Dialog kind 9 (operand 5) requires Interpreter::special_panels (0x45181c)".into(),
            ));
        };
        let outcome = {
            let mut sp = panels.borrow_mut();
            match sp.pet_target {
                None => {
                    sp.pet_target = Some(pet_id);
                    None
                }
                Some(owner) if owner == pet_id => {
                    let index = self.world.dialogs.declined;
                    let accepted = sp.pet_status == SPECIAL_PANEL_ACCEPTED;
                    sp.pet_target = None;
                    if !accepted {
                        self.world.dialogs.declined = -1;
                    }
                    Some((index, accepted))
                }
                Some(_) => {
                    sp.pet_target = None;
                    self.world.dialogs.declined = -1;
                    Some((0, false))
                }
            }
        };
        match outcome {
            None => Ok(Flow::Yield),
            Some((index, accepted)) => {
                self.set_temp(0, index)?;
                Ok(Flow::Branch(accepted))
            }
        }
    }

    /// Kinds 12 (Spellbook, `0x4518e2`) and 15 (Cookbook, `0x451946`): open
    /// modal `0x5107dc(person, mode)` synchronously inside `Dialog` and always
    /// take the true branch on the same tick.
    fn dialog_reference_book(&mut self, request: SpecialPanelRequest) -> Result<Flow, Trap> {
        if self.in_both_ways {
            return Err(Trap::Unsupported(format!(
                "Dialog kind {} (inline reference book) while an animator refresh is pending \
                 cannot run speculatively in both-ways mode (0x4518e2, 0x451946)",
                request.kind()
            )));
        }
        if let Some(inline) = self.inline_special_panel {
            self.interpreter().refresh_family_panel(self.world);
            if inline.ask_inline(request) {
                return Ok(Flow::Branch(true));
            }
            return Ok(Flow::Yield);
        }
        Ok(Flow::Branch(true))
    }

    /// The portrait an ordinary dialog's record shows, built as the
    /// record is made (0x4443ab, 0x435d8a, then 0x43601c), before it joins
    /// the registry. The icon mode is operand 7's bits 1 to 3 (0x436279):
    ///
    /// - mode 0 (0x436308): the type the dialog's text found, or else the
    ///   type the definition of the object the stack object names stands
    ///   for, when it is on the lot (a tile's group master's, 0x45728d).
    ///   The record's strings are formatted (0x44bf1a, from 0x436219) with
    ///   a place for a type (`[ebp-0x24]`), which mode 0 reads first
    ///   ([`Machine::dialog_text_type`]);
    /// - mode 2 (0x4362f4): the type of the neighbor record the stack
    ///   object names (0x425afe: 0x4256f4, record +8);
    /// - modes 1 and 4 to 7 show no person (the type stays 0 from
    ///   0x436271), and mode 3 shows a picture of the type's
    ///   (0x4362ca, 0x436362).
    ///
    /// A type that is a person (definition +0x12 is 2, 0x4575d2) has its
    /// entry in the interface's table of person types looked up by GUID
    /// (0x4572af, 0x4e9753), and 0x4e92a8 builds its first image when the
    /// entry has none: 0x4e8c5d draws the first person of that type on
    /// the lot (0x4e9608 walks the lot's people, +0x84). With none, it
    /// makes one (0x4e9652: the factory 0x440b2d, [`crate::Interpreter::create_person`]),
    /// finds it a tile from the house view's centre (0x4347f0 from
    /// 0x4a677c) and puts it there (virtual function 0x88), draws it,
    /// and removes it at once (0x44123f, [`crate::Interpreter::remove_now`]).
    /// The table holds every person type the catalog had when the game
    /// started (0x4e8b3d) plus every person type made while the game ran
    /// (`0x4265fd` → `0x4e8a3a`).
    ///
    /// Which entries have their image is kept in [`sims_sim::World::portraits`],
    /// which entries have slot 1's dirty byte (`+0x18`) set is kept in
    /// [`sims_sim::World::dirty_portraits`], and which writable files have had
    /// `BMP_` 2002 saved during this run (`0x4e938c` → `0x4e8d2e` → `0x4e9000`)
    /// is kept in [`sims_sim::World::saved_portraits`]. Only the dialog's
    /// builds are counted. The household's panel draws the family's portraits
    /// too, so a family member's type with nobody on the lot fails loudly
    /// whenever the dialog would otherwise create a temporary person.
    ///
    /// The put is [`crate::Interpreter::portrait_put_plan`]: the search from the
    /// house view's centre pick (an input, [`crate::Interpreter::view_centre`],
    /// bounds-checked and un-rotated as in `0x434891` and `0x43491c`) followed
    /// by [`crate::Interpreter::put_person`] when the tile is uniquely known or
    /// all reachable candidate tiles produce identical post-removal state.
    /// When the winning definition's file holds an embedded or persisted
    /// `BMP_` 2002 (`0x4e9338`, [`crate::program::Library::has_portrait_bitmap`]
    /// / [`sims_sim::World::generated_portrait_2002`] /
    /// [`sims_sim::World::saved_portraits`]) or the character was made during
    /// the session (`Make New Character`), `dialog_portrait` delegates directly
    /// to the full `0x4e92a8` path ([`Machine::ui_effect_portrait_for_guid`]).
    /// When no tile is found from the given view centre (or from every interior
    /// start when no view centre is given), the temporary person is removed
    /// unput (`0x4e9706`) with its Init/RNG side effects kept and the portrait
    /// left uncached; it fails loudly only when placement depends on an
    /// unmodeled detail or an unknown view centre where some starts find a tile
    /// and others do not. The rest of the put (the rooms, `0x440385`, and the
    /// tiles, `0x4330d7`) is taken to be undone by the removal: **unverified**.
    pub(crate) fn dialog_portrait(&mut self, operands: &[u8; 8]) -> Result<(), Trap> {
        let stack_object = self.frame()?.stack_object;
        let mode = (operands[7] >> 1) & 7;
        // The record's strings are formatted in every mode, before the
        // mode is read (0x436219 to 0x436265; mode 4 one more, 0x4362c0).
        let text_type = self.dialog_text_type(operands, mode)?;
        let guid = match mode {
            0 => match text_type {
                Some(guid) => guid,
                None => return Ok(()),
            },
            2 => match self.world.neighbors.get(stack_object) {
                Some(record) => record.guid,
                None => return Ok(()),
            },
            _ => return Ok(()),
        };
        if self.world.portraits.contains(&guid) {
            return Ok(());
        }
        // 0x43639e -> 0x4575d2: Dialog verifies the winning definition is a
        // person before calling 0x4e9753 / 0x4e92a8. When this GUID was made
        // during the session via Make New Character (registered in 0x6f2698
        // via 0xe0 -> 0x4e8add) OR its definition's IFF already holds an
        // embedded or persisted BMP_ 2002 (0x4e9338), run the full 0x4e92a8
        // portrait path (avoiding factory/CRT RNG draws on a clean slot with
        // embedded BMP_ 2002, and handling dirty-rebuild fallback to 0x4e9338).
        let is_person = crate::interpreter::object_definition(self.catalog, self.world, guid)
            .is_some()
            && self.definition_words(guid)?[TYPE] == PERSON_TYPE;
        if is_person
            && self.in_portrait_table(guid)?
            && (self.world.made_portrait_head.contains_key(&guid) || self.has_portrait_2002(guid))
        {
            return self.ui_effect_portrait_for_guid(guid, "a dialog's portrait");
        }
        if !self.listed_person_type(guid)? {
            return Ok(());
        }
        // 0x4e92e5 -> 0x4e92f2: a dirty slot runs 0x4e8c5d(index, 1, 0)
        // (save flag 0) before falling through to 0x4e9338 / 0x4e938c.
        if self.world.dirty_portraits.contains(&guid) {
            self.dialog_render_catalog_portrait(guid, false)?;
        }
        if self.world.portraits.contains(&guid) {
            return Ok(());
        }
        if self.has_portrait_2002(guid) {
            self.world.portraits.insert(guid);
            self.world.dirty_portraits.remove(&guid);
            return Ok(());
        }
        self.dialog_render_catalog_portrait(guid, true)?;
        Ok(())
    }

    /// Runs `0x4e8c5d(index, 1, save_bitmap)` on the catalog-person Dialog
    /// fallback path (no embedded `BMP_` 2002 yet): honors `0x4e8cc6`
    /// writability, `0x4e8d02` → `0x4e725f` `"HEAD"` binding checks when
    /// appearance/bone metadata is supplied, and `0x4e8d2e` → `0x4e9000`
    /// `BMP_` 2002 persistence before `remove_now`, while preserving the
    /// household and no-tile safety checks for synthetic tests.
    fn dialog_render_catalog_portrait(&mut self, guid: u32, save_bitmap: bool) -> Result<(), Trap> {
        let on_lot_id = self.world.people.iter().copied().find(|id| {
            self.world
                .objects
                .get(id)
                .is_some_and(|object| object.guid == guid)
        });
        if let Some(person_id) = on_lot_id {
            if self.has_portrait_render_metadata(self.world, guid) {
                let can_render = crate::primitives::ui_effect::can_render_person_portrait(
                    self.catalog,
                    self.program,
                    self.world,
                    person_id,
                    guid,
                    false,
                    "a dialog's portrait",
                )?
                .unwrap_or(false);
                if !can_render {
                    return Ok(());
                }
            }
            self.world.portraits.insert(guid);
            self.world.dirty_portraits.remove(&guid);
            if save_bitmap
                && self.is_portrait_file_writable(guid)
                && let Some(file_key) = self.portrait_file(guid)
            {
                self.world.generated_portrait_2002.insert(file_key);
                self.world.saved_portraits.insert(guid);
            }
            return Ok(());
        }
        // 0x4e8cc6 -> 0x4067b3 -> 0x4174dc: off-lot rendering only calls
        // 0x4e9652 when the winning definition's file is writable.
        if !self.is_portrait_file_writable(guid) {
            return Ok(());
        }
        let interpreter = self.interpreter();
        let unsupported = |fault| {
            Trap::Unsupported(format!(
                "making a person of type 0x{guid:08X} for a dialog's portrait (0x4e9652): \
                 {fault}"
            ))
        };
        // On a copy of the lot and of the machine's cells (the sounds
        // the person's Init plays, the objects deleted), kept only when
        // the whole build is done: a build that fails leaves nothing
        // behind (the original removes the person on every path,
        // 0x4e8fdb, 0x4e9706).
        let mut copy = self.world.clone();
        let cells = crate::both_ways::Cells::of(&interpreter);
        let built = cells.on(&interpreter);
        let words = self.definition_words(guid)?;
        if words[crate::factory::word::TYPE] != 2 || words[crate::factory::word::GROUP] != 0 {
            // 0x440b6f checks OBJD group (`+0x14`) BEFORE person type (`+0x12`)
            // and before the pet expansion check (`0x440d63`): a grouped or
            // non-person definition runs the group/object factory, fails the
            // cXPerson cast at 0x4e96b7, and stays registered at 0x4e96c3.
            let _ = built.try_create(&mut copy, guid).map_err(unsupported)?;
            if !cells.same_refresh(&crate::both_ways::Cells::of(&interpreter)) {
                return Err(Trap::Unsupported(format!(
                    "making an object of type 0x{guid:08X} for a dialog's portrait (0x4e9652) \
                     changed the refresh pending, which is not ported"
                )));
            }
            cells.keep(&interpreter, self.world, copy);
            return Ok(());
        }
        // 0x440d63 -> 0x440dd5 -> 0x4e969d: a pet disallowed by the installed
        // expansions mask is refused before any construction.
        if self.interpreter().is_refused_pet(self.world, guid) {
            return Ok(());
        }
        let Some(id) = built
            .try_create_person(&mut copy, guid)
            .map_err(unsupported)?
        else {
            // 0x4e969d: factory returned 0 (e.g. refused pet). Keep any side
            // effects, leave portrait uncached, return.
            if !cells.same_refresh(&crate::both_ways::Cells::of(&interpreter)) {
                return Err(Trap::Unsupported(format!(
                    "making a person of type 0x{guid:08X} for a dialog's portrait (0x4e9652) \
                     changed the refresh pending, which is not ported"
                )));
            }
            cells.keep(&interpreter, self.world, copy);
            return Ok(());
        };
        // The put, its callbacks and the removal run the new person's
        // scripts after the factory returned, one creation deeper, as
        // Create New Object Instance's tail does
        // ([`crate::MAX_CREATION_DEPTH`]).
        let tail = crate::Interpreter {
            creation_depth: interpreter.creation_depth.saturating_add(1),
            ..interpreter
        };
        let plan = cells
            .on(&tail)
            .portrait_put_plan(&mut copy, id, "a dialog's portrait")
            .map_err(|fault| fault.trap)?;
        let finish_at = |tile: Option<[i32; 2]>,
                         placed: bool,
                         mut lot: sims_sim::World,
                         run_cells: crate::both_ways::Cells|
         -> Result<(sims_sim::World, crate::both_ways::Cells), Trap> {
            let run = run_cells.on(&tail);
            if let Some(pos) = tile {
                run.put_person(&mut lot, id, pos, 1)?;
            }
            let can_render = if !placed {
                false
            } else if self.has_portrait_render_metadata(&lot, guid) {
                crate::primitives::ui_effect::can_render_person_portrait(
                    self.catalog,
                    self.program,
                    &mut lot,
                    id,
                    guid,
                    false,
                    "a dialog's portrait",
                )?
                .unwrap_or(false)
            } else {
                true
            };
            if placed && can_render {
                // 0x4e8d14 caches the portrait and clears dirty, and 0x4e8d2e ->
                // 0x4e9000 stages BMP_ 2002 BEFORE removing the temporary person.
                lot.portraits.insert(guid);
                lot.dirty_portraits.remove(&guid);
                if save_bitmap && let Some(file_key) = self.portrait_file(guid) {
                    lot.generated_portrait_2002.insert(file_key);
                    lot.saved_portraits.insert(guid);
                }
            }
            // Both normal completion (0x4e8fdb) and no-tile failure (0x4e9706)
            // remove the temporary person immediately and keep its Init/RNG side
            // effects.
            run.remove_now(&mut lot, id).map_err(unsupported)?;
            // The refresh pending is the caller's to keep, and a build
            // that changed it is not ported.
            if !run_cells.same_refresh(&crate::both_ways::Cells::of(&interpreter)) {
                return Err(Trap::Unsupported(format!(
                    "making a person of type 0x{guid:08X} for a dialog's portrait (0x4e9652) \
                     changed the refresh pending, which is not ported"
                )));
            }
            Ok((lot, run_cells))
        };
        let (final_world, final_cells) = match plan {
            crate::placement::PortraitPutPlan::NoTile => finish_at(None, false, copy, cells)?,
            crate::placement::PortraitPutPlan::FastPlaced => finish_at(None, true, copy, cells)?,
            crate::placement::PortraitPutPlan::Exact(pos) => {
                finish_at(Some(pos), true, copy, cells)?
            }
            crate::placement::PortraitPutPlan::Candidates(positions) => {
                let first = finish_at(Some(positions[0]), true, copy.clone(), cells.copy())?;
                let first_lot = crate::both_ways::LotSnapshot::of(&first.0);
                for &pos in &positions[1..] {
                    let next = finish_at(Some(pos), true, copy.clone(), cells.copy())?;
                    if crate::both_ways::LotSnapshot::of(&next.0) != first_lot
                        || !first.1.same(&next.1)
                    {
                        return Err(Trap::Unsupported(
                            "a dialog's portrait puts a person on a tile searched for from the \
                             house view's centre (0x4e96dd, 0x4347f0, 0x4a677c; \
                             `Lot::set_view_centre`), and with no view centre given the placement \
                             outcome depends on which tile the search picks"
                                .into(),
                        ));
                    }
                }
                first
            }
            crate::placement::PortraitPutPlan::Starts { starts, reason } => {
                let run_start = |from: [i32; 2]| {
                    let mut lot = copy.clone();
                    let run_cells = cells.copy();
                    let tile = run_cells
                        .on(&tail)
                        .search(
                            &mut lot,
                            id,
                            from,
                            1,
                            crate::placement::PORTRAIT_SEARCH_RULES,
                        )
                        .map_err(|f| f.trap)?;
                    let (mut lot, run_cells) = finish_at(tile, tile.is_some(), lot, run_cells)?;
                    if !save_bitmap
                        && !lot.portraits.contains(&guid)
                        && self.has_portrait_2002_in(&lot, guid)
                    {
                        lot.portraits.insert(guid);
                        lot.dirty_portraits.remove(&guid);
                    }
                    Ok((lot, run_cells))
                };
                let first = run_start(starts[0])?;
                let first_lot = crate::both_ways::LotSnapshot::of(&first.0);
                for &from in &starts[1..] {
                    let Ok(next) = run_start(from) else {
                        return Err(Trap::Unsupported(reason));
                    };
                    if crate::both_ways::LotSnapshot::of(&next.0) != first_lot
                        || !first.1.same(&next.1)
                    {
                        return Err(Trap::Unsupported(reason));
                    }
                }
                first
            }
        };
        final_cells.keep(&interpreter, self.world, final_world);
        Ok(())
    }

    /// Whether a `BMP_` 2002 portrait was saved during play into the resource
    /// file holding `guid` (`0x4e904e..0x4e90ab`, `0x4e926a..0x4e9287`): each
    /// runtime-made character has its own dedicated `User%05d.iff`, while
    /// catalog definitions sharing a file share any `BMP_` 2002 written to it.
    pub(crate) fn file_has_saved_portrait(&self, guid: u32) -> bool {
        self.file_has_saved_portrait_in(self.world, guid)
    }

    pub(crate) fn file_has_saved_portrait_in(&self, world: &sims_sim::World, guid: u32) -> bool {
        if world.characters.made.contains_key(&guid) {
            return world.saved_portraits.contains(&guid);
        }
        self.catalog.object(guid).is_some_and(|entry| {
            world.saved_portraits.iter().any(|&saved| {
                !world.characters.made.contains_key(&saved)
                    && self
                        .catalog
                        .object(saved)
                        .is_some_and(|other| other.resource == entry.resource)
            })
        })
    }

    /// Whether type `guid` is in the interface's table of person types
    /// (0x4e9753, which gives its place, or -1): the table holds every
    /// person type the catalog had when the game started (0x4e8b3d, setting
    /// `[0x6f26a4] = 1` at `0x4e8a32`) plus every person type made while the
    /// game ran (`0x4265fd` calls `0x4e8a3a`, which appends new person types
    /// at `0x4e8add`).
    pub(crate) fn listed_person_type(&self, guid: u32) -> Result<bool, Trap> {
        Ok(
            crate::interpreter::object_definition(self.catalog, self.world, guid).is_some()
                && self.definition_words(guid)?[TYPE] == PERSON_TYPE,
        )
    }

    /// The strings a dialog's record loads, from STR# 301 of the running
    /// code's file (0x4360b8 then 0x416920, 0x47028b, then 0x4369de), in
    /// the order they are formatted (0x44bf1a, 0x436219 to 0x436265): the
    /// buttons' labels its kind (operand 5) loads (0x436175: kinds 0, 3
    /// and 4 operand 3, kind 1 operands 4 and 3, kind 2 operands 0, 4 and
    /// 3), formatted in the order operand 3, 4, 0; then the title
    /// (operand 6) and the message (operand 2). A string is named by its
    /// number from 1. In mode 4, operand 1's string comes last.
    ///
    /// A string that is number 0, that the file lacks or that is empty is
    /// left out: 0x4369de puts in its place the default the caller gives
    /// (0x436a21 to 0x436a35), a button's label from STR# 142 of
    /// `GameData/UIText.iff` (0x435fee, 0x43600a), or else "Missing
    /// string!!!" (0x6c29bc). The formatter reads them too, so they are
    /// left out only when the labels are known to hold no `$`
    /// ([`crate::Program::labels_hold_no_token`]); otherwise a dialog with
    /// one fails loudly.
    ///
    /// The table is read in the application's language: the table's
    /// language byte is the interface's +0x64 less one (0x4702b1 to
    /// 0x4702bb), and 0x4704fa reads that language's strings, with no
    /// other to fall back on. The port reads US English (language 1,
    /// [`crate::program::Library::strings`]), the recorded install's (its
    /// registry has `SIMS_LANGUAGE` USEnglish, which 0x4f2ddc to 0x4f2df8
    /// read after `SIMS_OTHERLANGUAGE`; the interface's +0x64 for it is
    /// **unverified**). An install in another language is not supported:
    /// its strings may hold other tokens.
    ///
    /// The table is the one the original's loader finds
    /// ([`crate::program::Library::loaded_strings`]): the resource of the
    /// ID, its entries in the loader's order. The loader's other ways are
    /// not ported, and fail loudly when the dialog names a string: a
    /// table it reads otherwise, such as one whose entries have a language
    /// and none is US English, which it reads again for English (0x47006b
    /// to 0x4700d0; [`crate::program::Library::unported_strings`]), and a
    /// file with no STR# 301 but a legacy CST 301, which it reads instead
    /// (0x4702c7; [`crate::program::Library::legacy_strings`]).
    /// Loads string table `table` of the running code's file as `0x47028b`
    /// with argument 3 zero loads it (`loaded_strings`, falling back to
    /// `strings`), failing loudly when the table uses an unported loader
    /// path (`unported_strings`) or falls back to a legacy CST resource
    /// (`legacy_strings`).
    pub(crate) fn string_table(&self, table: u16, what: &str) -> Result<Option<&[String]>, Trap> {
        let owner = self.owner(self.frame()?)?;
        let Some(library) = self.program.libraries.get(&owner) else {
            return Ok(None);
        };
        let strings = match library.loaded_strings.get(&table) {
            Some(loaded) => loaded.as_deref(),
            None => library.strings.get(&table).map(Vec::as_slice),
        };
        if let Some(why) = library.unported_strings.get(&table) {
            return Err(Trap::Unsupported(format!(
                "{what} STR# {table} {why}: not ported"
            )));
        }
        if strings.is_none() && library.legacy_strings.contains(&table) {
            return Err(Trap::Unsupported(format!(
                "{what} file has no STR# {table} but a legacy CST {table}, which the \
                 original's loader reads instead (0x4702c7, 0x47018e to 0x4701e6): not ported"
            )));
        }
        Ok(strings)
    }

    pub(crate) fn dialog_strings(&self, operands: &[u8; 8]) -> Result<Vec<String>, Trap> {
        const TABLE: u16 = 301;
        // Mode 4 formats operand 1's string too, after the others
        // (0x436297 to 0x4362c0, through 0x4369de as they are).
        let mode_4: &[usize] = if (operands[7] >> 1) & 7 == 4 {
            &[1]
        } else {
            &[]
        };
        let buttons: &[usize] = match operands[5] & 0x7f {
            0 | 3 | 4 => &[3],
            1 => &[3, 4],
            2 => &[3, 4, 0],
            _ => &[],
        };
        let named = buttons
            .iter()
            .chain(&[6, 2])
            .chain(mode_4)
            .any(|&operand| operands[operand] != 0);
        let strings = if named {
            self.string_table(TABLE, "a dialog's")?
        } else {
            None
        };
        let mut texts = Vec::new();
        for &operand in buttons.iter().chain(&[6, 2]).chain(mode_4) {
            let text = usize::from(operands[operand])
                .checked_sub(1)
                .and_then(|index| strings?.get(index))
                .filter(|text| !text.is_empty());
            match text {
                Some(text) => texts.push(text.clone()),
                None if !self.program.labels_hold_no_token => {
                    return Err(Trap::Unsupported(
                        "a dialog's string is its default (0x4369de, 0x436a21 to 0x436a35), \
                         a button's label from STR# 142 of `GameData/UIText.iff` (0x435fee, \
                         0x43600a), and the labels are not known to hold no token: not ported"
                            .into(),
                    ));
                }
                None => {}
            }
        }
        Ok(texts)
    }

    /// The type a dialog of icon mode 0 shows (None for the other modes):
    /// the type the formatter (0x44bf1a) finds in the record's strings
    /// ([`Machine::dialog_strings`]), each formatted in turn with the same
    /// place for it (`[ebp-0x24]`, cleared at 0x43620d), so the last one
    /// found is kept; or, with none found, the type the stack object's
    /// definition stands for ([`Machine::master_type`], 0x436308 to
    /// 0x43634d). The passes run in this order:
    ///
    /// 1. `$FamilyAssets` (0x44bf66) and `$Family` (0x44c034): not ported,
    ///    and a text that holds `$Family` fails loudly;
    /// 2. `$Me` (0x6e5060; 0x44c0fc): the running object's name (the
    ///    formatter's `this` is the record's object, 0x436200);
    /// 3. `$Object` (0x6e5050, from 0x6c407c; 0x44c155 to 0x44c1af): the
    ///    name and type of the object the stack object names (frame +4),
    ///    when there is one; otherwise the token stays (0x44c183 to
    ///    0x44c1a6);
    /// 4. `$Local` ([`Machine::locals_written`]);
    /// 5. the passes of [`BETWEEN`]: not ported, and a text that holds one
    ///    of their tokens fails loudly;
    /// 6. `$NeighborLocal` ([`Machine::neighbor_locals`]);
    /// 7. `$Neighbor` (0x6e5090, from 0x6c4054; 0x44c65f to 0x44c6e8): the
    ///    name and type of the neighbor record the stack object names
    ///    (0x425afe), when there is one; otherwise the token stays. It
    ///    comes after `$Object`'s, so it wins in a string that holds both;
    /// 8. the passes of [`AFTER`]: not ported, and a text that holds one
    ///    of their tokens fails loudly;
    /// 9. `$TimeLocal` (0x6e5020, 0x44c8c4): an hour local in [0, 24)
    ///    written in one of fixed formats (0x4a6ab1), which hold no token;
    /// 10. the career's: `$JobOffer`, `$JobDesc` and `$Job` ([`CAREER`];
    ///     0x44c966). Each token found casts the running object to a
    ///     person (0x44c9bc) and reads it (0x44ca27, also with the numbers
    ///     not locals), which faults for an object that is not one: that
    ///     fails loudly. For a person they write career texts, which end
    ///     the passes only when they hold no career token past their first
    ///     character: that fails loudly unless
    ///     [`crate::Program::career_texts_end`].
    ///
    /// The passes after `$Neighbor`'s write text and read the running
    /// object, the locals and the career tables, and are taken to change
    /// nothing else (**unverified**).
    ///
    /// The names are the type's catalog name (0x4573d9, 0x4061e3,
    /// [`Machine::catalog_name`]), replaced as [`name_pass`] does it.
    /// The checks run in every mode: the strings are formatted before
    /// the mode is read.
    ///
    /// Formats `text` through the original string formatter (`0x44bf1a`),
    /// returning the formatted text and the portrait type (`[ebp+0x14]`)
    /// written by `$Object` or `$Neighbor`.
    ///
    /// When `exact_text` is true (as for an Add/Change Action String entry
    /// added to a pie menu, whose formatted label is kept), tokens whose
    /// substitution is not ported (`$Job*` on a person) fail loudly instead
    /// of being treated as presentation.
    pub(crate) fn format_text(
        &self,
        mut text: String,
        what: &str,
        exact_text: bool,
    ) -> Result<Formatted, Trap> {
        let stack_object = self.frame()?.stack_object;
        let object = u16::try_from(stack_object)
            .ok()
            .and_then(|id| self.world.objects.get(&id))
            .map(|object| object.guid);
        let neighbor = self.world.neighbors.get(stack_object).map(|r| r.guid);
        if text.contains("$Family") {
            return Err(Trap::Unsupported(format!(
                "{what} text holds `$FamilyAssets` or `$Family`, whose passes (0x44bf1a, \
                 0x44bf66, 0x44c034) are not ported"
            )));
        }
        if text.contains("$Me") {
            let name = self.catalog_name(self.me()?.guid)?;
            text = name_pass(text, "$Me", name.as_deref(), what, "0x44c13e")?;
        }
        let mut portrait = None;
        if let Some(guid) = object.filter(|_| text.contains("$Object")) {
            portrait = Some(guid);
            let name = self.catalog_name(guid)?;
            text = name_pass(text, "$Object", name.as_deref(), what, "0x44c1ca")?;
        }
        text = self.locals_written(&text, what)?;
        not_ported(&text, &BETWEEN, what, "0x44c280 to 0x44c590")?;
        self.neighbor_locals(&text, what)?;
        if let Some(guid) = neighbor.filter(|_| text.contains("$Neighbor")) {
            portrait = Some(guid);
            let name = self.catalog_name(guid)?;
            text = name_pass(text, "$Neighbor", name.as_deref(), what, "0x44c6ba")?;
        }
        not_ported(&text, &AFTER, what, "0x44c6ea to 0x44c8c4")?;
        text = self.time_locals(&text, what)?;
        self.career_tokens(&text, what, exact_text)?;
        Ok(Formatted { text, portrait })
    }

    pub(crate) fn dialog_text_type(
        &self,
        operands: &[u8; 8],
        mode: u8,
    ) -> Result<Option<u32>, Trap> {
        let stack_object = self.frame()?.stack_object;
        let object = u16::try_from(stack_object)
            .ok()
            .and_then(|id| self.world.objects.get(&id))
            .map(|object| object.guid);
        // Mode 4's last string is formatted with no place for a type
        // (0x4362ba pushes 0), but only mode 0 reads the place.
        let mut found = None;
        for text in self.dialog_strings(operands)? {
            let formatted = self.format_text(text, "a dialog's", false)?;
            if formatted.portrait.is_some() {
                found = formatted.portrait;
            }
        }
        if mode != 0 {
            return Ok(None);
        }
        if found.is_some() {
            return Ok(found);
        }
        // Mode 0 shows, when nothing was found, the type the stack object's
        // definition stands for (0x436308 to 0x43634d).
        match object {
            Some(guid) => self.master_type(guid),
            None => Ok(None),
        }
    }

    /// The type of the definition type `guid`'s definition stands for
    /// ([`crate::Program::master_definition`], 0x45728d through 0x43ae0a):
    /// its own for a definition alone, its group's master's for a tile.
    /// None for a tile whose group has no master: mode 0 then has no type
    /// and shows no portrait (0x436345 to 0x43634d leave the place null).
    fn master_type(&self, guid: u32) -> Result<Option<u32>, Trap> {
        // A runtime-made character is a standalone person definition whose own
        // OBJD carries `guid` (not its template's GUID).
        if self.world.characters.made.contains_key(&guid) {
            return Ok(Some(guid));
        }
        let definition = self.definition(guid)?;
        let master = self
            .program
            .master_definition(definition.resource, definition.chunk)
            .map_err(Trap::Invalid)?;
        Ok(master.map(|words| {
            u32::from(words[crate::factory::word::GUID] as u16)
                | u32::from(words[crate::factory::word::GUID + 1] as u16) << 16
        }))
    }

    /// The catalog name the formatter writes for type `guid` (0x4573d9,
    /// 0x457d3b, 0x4061e3), or `None` when it is not known.
    ///
    /// `0x4061e3` checks for a legacy `CATS` resource of definition word 41's
    /// ID first, then loads the `CTSS` table in the app's language (US English,
    /// language 1, via `0x470ca7(0, 0)` and `0x471343`, matching
    /// [`crate::program::loaded_order`]) and reads slot 1 (`0x471411(1, -1)`)
    /// as the name (slot 2 is the description). When slot 1 is empty or absent,
    /// `0x4061e3` tries every language for a person (`0x470ca7(1, 0)`,
    /// `0x47145f(1, 0)`) and `0x457d3b` falls back to the definition's own
    /// name (`0x457db4`): none of those fallbacks nor `CATS` are ported, so
    /// they return `None` and fail loudly when a token needs the name.
    fn catalog_name(&self, guid: u32) -> Result<Option<String>, Trap> {
        // `0x44b0a8..0x44b0fc`: characters created by Make New Character take
        // their name from global `[0x6e4ac0]` (the last kind 3 text dialog).
        if let Some(names) = self.character_names
            && let Some(name) = names.borrow().get(&guid)
            && !name.is_empty()
        {
            return Ok(Some(name.clone()));
        }
        let table = self.definition_words(guid)?[CATALOG_STRINGS] as u16;
        let definition = self.definition(guid)?;
        let Some(library) = self.program.libraries.get(&definition.resource) else {
            return Ok(None);
        };
        if library.legacy_catalog_strings.contains(&table) {
            return Ok(None);
        }
        if let Some(why) = library.unported_catalog_strings.get(&table) {
            return Err(Trap::Unsupported(format!(
                "catalog string table {table} {why} (0x4061e3, 0x470ca7): not ported"
            )));
        }
        let Some(entries) = library.catalog_strings.get(&table) else {
            return Ok(None);
        };
        let mut loaded: Vec<&str> = Vec::new();
        let mut run = None;
        let mut place = 0;
        for (language, text) in entries {
            let lang = (*language).max(1);
            if run != Some(lang) {
                run = Some(lang);
                place = 0;
            }
            if lang == 1 {
                loaded.insert(place.min(loaded.len()), text.as_str());
                place += 1;
            }
        }
        Ok(loaded
            .first()
            .filter(|name| !name.is_empty())
            .map(|&name| name.to_owned()))
    }

    /// `text` after the formatter's `$Local` pass (0x44c1f2 to 0x44c27d).
    /// Each token (0x6e5030), searched for from one past the last
    /// (0x44c25f), is read with its `:` and number, and an optional second
    /// `:` and number ([`local_token`], 0x44caf6). When the number is one
    /// of the running tree's locals (from 0, below the count the tree has,
    /// frame +6, compared signed), the token and what was read with it are
    /// replaced (0x4557c0) by the local's value in decimal (0x46f941,
    /// `%d` of the word sign-extended). Otherwise the token stays.
    fn locals_written(&self, text: &str, what: &str) -> Result<String, Trap> {
        const TOKEN: &str = "$Local";
        let locals = &self.frame()?.locals;
        let mut text = text.to_owned();
        let mut from = 0;
        while let Some(at) = text
            .get(from..)
            .and_then(|t| t.find(TOKEN))
            .map(|i| i + from)
        {
            let (number, _, end) = local_token(text.as_bytes(), at + TOKEN.len(), what, TOKEN)?;
            if let Ok(n) = usize::try_from(number)
                && n < locals.len()
            {
                self.check_local(n).map_err(|trap| {
                    Trap::Unsupported(format!("{what} `$Local:{n}` (0x44bf1a, 0x44c240): {trap}"))
                })?;
                text.replace_range(at..end, &locals[n].to_string());
            }
            from = at + 1;
        }
        Ok(text)
    }

    /// The formatter's `$NeighborLocal` pass (0x44c590 to 0x44c65a) on
    /// `text`. Each token (0x6e4fc0), searched for from one past the last
    /// (0x44c643), is read with its `:` and number, and an optional second
    /// `:` and number ([`local_token`]). When the number is one of the
    /// running tree's locals (from 0, below the count the tree has, frame
    /// +6, compared signed), the local names a neighbor record (0x44c5f4,
    /// 0x4256f4) and the record's type is in the interface's table of
    /// person types (0x4572af, 0x4e9753), the token is replaced by the
    /// type's name from the interface's table (0x4e9c46, 0x4e893e, 0x455750),
    /// which is not ported, and fails loudly; a record whose type is not
    /// in the table ends the pass (0x44c616 to 0x44c653). Otherwise the
    /// token stays, and the `$Neighbor` pass finds it.
    pub(crate) fn neighbor_locals(&self, text: &str, what: &str) -> Result<(), Trap> {
        const TOKEN: &str = "$NeighborLocal";
        let locals = &self.frame()?.locals;
        let mut from = 0;
        while let Some(at) = text
            .get(from..)
            .and_then(|t| t.find(TOKEN))
            .map(|i| i + from)
        {
            let (number, _, _) = local_token(text.as_bytes(), at + TOKEN.len(), what, TOKEN)?;
            if let Ok(n) = usize::try_from(number)
                && n < locals.len()
            {
                self.check_local(n).map_err(|trap| {
                    Trap::Unsupported(format!(
                        "{what} `$NeighborLocal:{n}` (0x44bf1a, 0x44c5f4): {trap}"
                    ))
                })?;
                if let Some(record) = self.world.neighbors.get(locals[n]) {
                    if !self.listed_person_type(record.guid)? {
                        break;
                    }
                    return Err(Trap::Unsupported(format!(
                        "{what} `$NeighborLocal:{n}` is replaced by the name of type \
                         0x{:08X} from the interface's table (0x44bf1a, 0x4e9c46, 0x4e893e): \
                         not ported",
                        record.guid
                    )));
                }
            }
            from = at + 1;
        }
        Ok(())
    }

    /// `text` after the formatter's `$TimeLocal` pass (`0x44c8c4` to
    /// `0x44c965`). Each in-range local is read (`0x44c90b`); when its hour
    /// is in `[0, 24)` (`0x44c918`), the matched token (including any
    /// optional second `:digits`, `0x44caf6`) is replaced (`0x44c93e`) by
    /// the US English time string (`0x4a6ab1`, `0x5fe51e`) before the
    /// career pass runs, so `$Job*` sees the substituted text. Out-of-range
    /// indices and hours stay literal.
    fn time_locals(&self, text: &str, what: &str) -> Result<String, Trap> {
        const TOKEN: &str = "$TimeLocal";
        let locals = &self.frame()?.locals;
        let mut text = text.to_owned();
        let mut from = 0;
        while let Some(at) = text
            .get(from..)
            .and_then(|t| t.find(TOKEN))
            .map(|i| i + from)
        {
            let (number, _, end) = local_token(text.as_bytes(), at + TOKEN.len(), what, TOKEN)?;
            if let Ok(n) = usize::try_from(number)
                && n < locals.len()
            {
                self.check_local(n).map_err(|trap| {
                    Trap::Unsupported(format!(
                        "{what} `$TimeLocal:{n}` (0x44bf1a, 0x44c90b): {trap}"
                    ))
                })?;
                if (0..24).contains(&locals[n]) {
                    text.replace_range(at..end, &format_hour(locals[n]));
                }
            }
            from = at + 1;
        }
        Ok(text)
    }

    /// The formatter's career passes (`$JobOffer`, `$JobDesc`, `$Job`,
    /// `0x44c966` to `0x44cae0`). When both parsed numbers are in range of
    /// the frame's locals, both locals are read (`0x44c9fc`, `0x44ca0b`)
    /// before `0x44ca27` unconditionally dereferences the running object as a
    /// person (which faults on non-persons).
    fn career_tokens(&self, text: &str, what: &str, exact_text: bool) -> Result<(), Trap> {
        let locals = &self.frame()?.locals;
        let is_person = self.me()?.person.is_some();
        let mut matched = false;
        for token in CAREER {
            let mut from = 0;
            while let Some(at) = text
                .get(from..)
                .and_then(|t| t.find(token))
                .map(|i| i + from)
            {
                let (first, second, _) =
                    local_token(text.as_bytes(), at + token.len(), what, token)?;
                if let (Ok(n1), Ok(n2)) = (usize::try_from(first), usize::try_from(second))
                    && n1 < locals.len()
                    && n2 < locals.len()
                {
                    self.check_local(n1).map_err(|trap| {
                        Trap::Unsupported(format!(
                            "{what} `{token}:{n1}:{n2}` (0x44bf1a, 0x44c9fc): {trap}"
                        ))
                    })?;
                    self.check_local(n2).map_err(|trap| {
                        Trap::Unsupported(format!(
                            "{what} `{token}:{n1}:{n2}` (0x44bf1a, 0x44ca0b): {trap}"
                        ))
                    })?;
                }
                if !is_person {
                    return Err(Trap::Invalid(format!(
                        "{what} text holds a career token (`$JobOffer`, `$JobDesc`, `$Job`), \
                         whose pass casts the object running the tree to a person (0x44bf1a, \
                         0x44c9bc) and reads it (0x44ca27): it is not one, so the original \
                         faults ({text:?})"
                    )));
                }
                matched = true;
                from = at + 1;
            }
        }
        if matched {
            if exact_text {
                return Err(Trap::Unsupported(format!(
                    "{what} text holds a career token (`$JobOffer`, `$JobDesc`, `$Job`), whose \
                     text formatting (0x44bf1a, 0x44c966) is not ported ({text:?})"
                )));
            }
            if !self.program.career_texts_end {
                return Err(Trap::Unsupported(format!(
                    "{what} text holds a career token, and the career texts are not known to \
                     hold none past their first character, which would never end its pass \
                     (0x44bf1a, 0x44ca9f to 0x44cad3): not ported ({text:?})"
                )));
            }
        }
        Ok(())
    }

    fn set_temp(&mut self, temp: usize, value: i16) -> Result<(), Trap> {
        let object = self.me_mut()?;
        let slot = object
            .temps
            .get_mut(temp)
            .ok_or(Trap::native(2, "temp out of range"))?;
        *slot = value;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::may_hold_career_token;

    #[test]
    fn a_career_token_past_the_first_character_or_the_start_of_one_at_the_end() {
        for text in ["a $Job", "x$JobDesc:0", "Clerk $Jo", "B$", "$$Job"] {
            assert!(may_hold_career_token(text), "{text}");
        }
        for text in [
            "$Job",
            "B004MA$b_01",
            "business$g$c_01",
            "Clerk",
            "",
            "$JobOffer",
        ] {
            assert!(!may_hold_career_token(text), "{text}");
        }
    }
}
