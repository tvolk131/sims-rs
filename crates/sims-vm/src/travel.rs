//! Away-From-Home Travel Engine & Persistence (`Sims.exe` 1.91 travel
//! transitions for **Downtown**, **Vacation Island**, **Old Town**,
//! **Studio Town**, and **Magic Town**) over
//! [`sims_sim::neighborhood_store::NeighborhoodStore`].
//!
//! All transitions operate strictly against REDB without mutating
//! `SIMS_INSTALL` on disk:
//!
//! 1. **Classification & Eligibility ([`TravelDestination`], [`TravelParty`],
//!    [`TravelTransport`]):**
//!    - **Downtown** (`21..=30`, Hot Date `0x04`): `$50` cab, single adult or
//!      adult + adult date companion.
//!    - **Vacation Island** (`40..=48`, Vacation `0x08`): `$500` vacation wagon,
//!      whole household (adults, children, pets), optional hotel check-in
//!      (`Token - Hotel Key` `0x3ac022d6`, `kind = 0`) or campground stay.
//!    - **Old Town** (`50..=79` with `LotZoning == 2`, Unleashed `0x20`): `$0`
//!      trolley, whole household (including pets).
//!    - **Studio Town** (`81..=89`, Superstar `0x40`): `$50` studio shuttle
//!      (or `$0` limo for a Sim with fame `>= 10` / 1+ stars), single adult Sim.
//!    - **Magic Town** (`90..=98`, Makin' Magic `0x80`; `90..=92` residential,
//!      `93..=98` community): `$0` Magic Hole or `$50` Magic Blimp, single human
//!      Sim (adult or child) or household group/family.
//! 2. **Departure, `TRVL` Envelope & Destination Boot ([`depart_to_destination`]):**
//!    - Validates eligibility (using the install's detected expansion bitmask),
//!      live budget mode (`globals[32] == 0`), and fare; prepares the frozen
//!      home lot snapshot (deducting fare into `budget.days[0]`, flushing live
//!      persons to `NBRS`, refreshing lot values, setting
//!      `family.secondary_house = destination_house`, and preserving the home
//!      lot's exact clock and non-traveling Sims/pets); boots the destination
//!      lot from REDB in live spending mode (`globals[32] = 0`, `globals[9] =
//!      home_family`, `globals[10] = destination_house`); injects the traveling
//!      party with live motives/skills/career/fame/accessories/relationships/
//!      outfits/funds/MagiCoins/tokens near the arrival portal (establishing
//!      mutual `person.data[77]` links on Downtown Dates and using
//!      [`select_sim`]); and atomically persists a `TRVL` active-trip envelope
//!      in `NEIGHBORHOOD_LOTS[(nh, home_house)]` alongside `secondary_house`
//!      (blocking ordinary `enter_lot` on the frozen home lot while binding the
//!      live destination handle exclusively to its unique `trip_id` with
//!      `neighborhood_origin = None` so `save_and_leave_lot` cannot overwrite
//!      Build/Buy destination snapshots).
//! 3. **Return Home, Concurrent-Write Reconciliation & Crash Recovery
//!    ([`return_from_destination`], [`inspect_active_trip`],
//!    [`resume_active_trip`], [`abort_active_trip`]):**
//!    - Fences return on `(store_id, instance_id, trip_id)` rather than
//!      freezing `departure_revision`, so unrelated neighborhood writes made
//!      while away do not strand an active trip while retained sessions from
//!      older trips to the same lot are rejected.
//!    - Flushes all live destination persons to `NBRS`, captures updated
//!      motives, skills, career/fame, accessories, relationships, funds &
//!      MagiCoins delta, outfits, and inventory tokens (revoking temporary
//!      Vacation hotel keys granted on check-in while keeping souvenirs/gifts/
//!      ingredients/MagiCoins), and selectively merges destination updates onto
//!      the latest `StoredNeighborhood`.
//!    - Preserves any Build/Buy community-lot edits stored in REDB untouched
//!      (matching `Sims.exe` 1.91; community-lot Build/Buy edits use
//!      [`open_destination_for_edit`] + [`crate::lifecycle::save_and_leave_lot`]).
//!    - Reconciles and resumes the frozen home lot directly from the persisted
//!      `Saved` snapshot at its exact departure tick and clock with returning
//!      household travelers placed cleanly at the home curb/portal and zero
//!      duplicate or orphaned objects, unwrapping `TRVL` back to a normal saved
//!      lot in a single atomic checked REDB transaction.
//!    - Provides checked recovery after crashes, restarts, or halted
//!      destination lots via [`inspect_active_trip`], [`resume_active_trip`],
//!      and [`abort_active_trip`].

use crate::boot::save::Saved;
use crate::boot::{Finished, Frames, LoadReport, Lot, NeighborhoodOrigin, opening_rotation};
use crate::daily::value as object_value;
use crate::factory::word;
use crate::interpreter::{Interpreter, Stream};
use crate::people;
use crate::placement::{Place, Rules};
use crate::play::Play;
use crate::selection::selected;
use crate::{Answers, Program};
use serde::{Deserialize, Serialize};
use sims_content::{Animations, Catalog, House, Index, Install, People, SurfaceCatalog, Surfaces};
use sims_core::CrtRng;
use sims_formats::iff::Iff;
use sims_formats::saved_objects::Accessory;
use sims_formats::strings::StringTable;
use sims_sim::load::{Context, ExpandedWall, FLOORS, WALLS, visitor_again};
use sims_sim::neighborhood_store::{
    CheckedCommitError, DEFAULT_ARCH_VALUE_MULTIPLIER, NeighborhoodStore, StoredNeighborhood,
    lot_total_value,
};
use sims_sim::neighbors::{Inventory, Item, registry};
use sims_sim::{FULL_SIZE, PersonOutfit, World};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Magic prefix and format version for persisted active-trip envelopes stored
/// in `NEIGHBORHOOD_LOTS[(neighborhood_id, home_house)]` while a household is
/// away from home. Because `Saved::decode` expects `Saved::FORMAT` (`9u32`) LE, this
/// envelope simultaneously persists the authoritative trip record atomically
/// with `secondary_house` AND prevents `lifecycle::enter_lot` from opening or
/// overwriting the frozen home lot while the trip is active.
const TRAVEL_ENVELOPE_MAGIC: &[u8; 4] = b"TRVL";
const TRAVEL_ENVELOPE_VERSION: u32 = 1;

static NEXT_TRIP_SEQ: AtomicU64 = AtomicU64::new(1);

/// Installed-expansion bits (`[0x6e48bc] + 0x66`, `Program::expansions`).
pub const EXPANSION_HOT_DATE: u16 = 0x04;
pub const EXPANSION_VACATION: u16 = 0x08;
pub const EXPANSION_UNLEASHED: u16 = 0x20;
pub const EXPANSION_SUPERSTAR: u16 = 0x40;
pub const EXPANSION_MAKIN_MAGIC: u16 = 0x80;

/// Default Complete Collection 1.91 travel fares (matching `Phone` Call Cab
/// check trees `0x2074` in `House05.iff`).
pub const DOWNTOWN_FARE: i32 = 50;
pub const VACATION_FARE: i32 = 500;
pub const OLD_TOWN_FARE: i32 = 0;
pub const STUDIO_TOWN_FARE: i32 = 50;
pub const STUDIO_LIMO_FARE: i32 = 0;
pub const MAGIC_BLIMP_FARE: i32 = 50;
pub const MAGIC_HOLE_FARE: i32 = 0;

/// Minimum fame half-stars (`person.data[81]`) for a complimentary Studio Town
/// limo (1.0 star = 10 half-stars in `0x446c15` / `0x52d3e6`).
pub const STUDIO_LIMO_MIN_FAME: i16 = 10;

/// Vacation hotel-key inventory token (`ExpansionPack4.far::Tokens.iff`,
/// `OBJD #16810`, type `0xd`, inventory kind `0`, `0x43e21a`).
pub const HOTEL_KEY_TOKEN_GUID: u32 = 0x3ac0_22d6;
pub const HOTEL_KEY_TOKEN_KIND: i32 = 0;

/// Makin' Magic currency token (`ExpansionPack7.far::TokensMagic.iff`,
/// `"Magic Token - Currency"`, inventory kind `7`).
pub const MAGICOIN_TOKEN_GUID: u32 = 0x99e8_1bec;
pub const MAGICOIN_TOKEN_KIND: i32 = 7;

/// Standard curb arrival portal GUIDs (`Objects.far`).
pub const CAR_PORTAL_GUID: u32 = 0x865a_6812;
pub const PED_PORTAL_GUID: u32 = 0x81e6_bef9;
/// Magic Town `"Magic - Clown Portal"` (`ExpansionPack7.far::ClownPortal.iff`).
pub const MAGIC_PORTAL_GUID: u32 = 0x2baa_0029;
/// Magic Town `"Ped Marker Magic - Cab Stop"` (`ExpansionPack7.far::PedMarkersMagic.iff`).
pub const MAGIC_CAB_STOP_GUID: u32 = 0x1307_6784;

/// Global indices.
const GLOBAL_HOURS: usize = 0;
const GLOBAL_SELECTED: usize = 3;
const GLOBAL_MINUTES: usize = 5;
const GLOBAL_SECONDS: usize = 6;
const GLOBAL_PAUSED: usize = 17;
const GLOBAL_MODE: usize = 19;
const GLOBAL_BUILT: usize = 22;
const GLOBAL_LOT_SIZE: usize = 23;
const GLOBAL_INDOOR_AREA: usize = 27;
const GLOBAL_BUDGET_MODE: usize = 32;

/// Object data words.
const DATA_CATEGORY: usize = 59;
const DEF_TYPE_SKIPPED: i16 = 7;
const DEF_BUILT: [usize; 3] = [39, 40, 69];
const DEF_BUILD_KIND: usize = 69;

/// Person data words.
const PERSON_BODY_SUIT: usize = 8;
const PERSON_VARIANT: usize = 21;
const PERSON_HAND_LEFT: usize = 22;
const PERSON_HAND_RIGHT: usize = 23;
const PERSON_NEIGHBOR: usize = 31;
const PERSON_VISITOR: usize = 32;
const PERSON_AGE: usize = 58;
const PERSON_KIND: usize = 65;
const PERSON_HOME: usize = 67;
const PERSON_DATE: usize = 77;
const PERSON_FAME_LEVEL: usize = 81;

/// `Global.far` `FCNS` chunk IDs.
const NEIGHBORHOOD_TUNING: u16 = 4;
const AUTONOMY_TUNING: u16 = 2;
const DEFAULT_FRIENDSHIP_THRESHOLD: i32 = 50;

/// Lot array / pricing constants (`0x47b006`).
const TILE_FLAGS: [u16; 2] = [8, 108];
const POOL_ARRAY: u16 = 9;
const TILE_LOCKED: u8 = 0x20;
const SPLIT_FLOOR: u16 = 0xff;
const POOL_TILE_LIMIT: u8 = 0x10;
const PATTERNED_WALL_STYLES: [u16; 4] = [1, 7, 8, 9];
const WALL_CELL: usize = 14;
const STYLE_TABLE: u16 = 129;
const FLOOR_TABLE: u16 = 130;
const WALL_TABLE: u16 = 131;
const POOL_TABLE: u16 = 133;
const FLOOR_DEFAULT_TABLE: u16 = 134;
const WALL_DEFAULT_TABLE: u16 = 135;
const PRICED_STYLES: [u16; 5] = [1, 2, 13, 12, 14];
const SECOND_STYLE: u16 = 255;
const US_ENGLISH: u8 = 1;

/// Complete Collection 1.91 away-from-home travel destinations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TravelDestination {
    /// Hot Date Downtown lots (`21..=30`).
    Downtown,
    /// Vacation Island lots (`40..=48`).
    VacationIsland,
    /// Unleashed Old Town community lots (`50..=79` with `LotZoning == 2`).
    OldTown,
    /// Superstar Studio Town lots (`81..=89`).
    StudioTown,
    /// Makin' Magic Town lots (`90..=98`; `90..=92` residential, `93..=98` community).
    MagicTown,
}

impl TravelDestination {
    /// Classifies `house` using Complete Collection's lot-number ranges and
    /// `LotZoning.iff` table (`1` = residential, `2` = community).
    ///
    /// Returns `None` for standard residential home lots (`1..=10`, residential
    /// `50..=80`, lot `0`, etc.).
    #[must_use]
    pub fn classify(house: u32, zoning: &BTreeMap<i16, i32>) -> Option<Self> {
        match house {
            21..=30 => Some(Self::Downtown),
            40..=48 => Some(Self::VacationIsland),
            50..=79 => {
                let lot = i16::try_from(house).ok()?;
                (zoning.get(&lot).copied() == Some(2)).then_some(Self::OldTown)
            }
            81..=89 => Some(Self::StudioTown),
            90..=98 => Some(Self::MagicTown),
            _ => None,
        }
    }

    /// Expansion pack bit in `Program::expansions` (`[0x6e48bc]+0x66`) required
    /// to travel to this destination.
    #[must_use]
    pub const fn expansion_bit(self) -> u16 {
        match self {
            Self::Downtown => EXPANSION_HOT_DATE,
            Self::VacationIsland => EXPANSION_VACATION,
            Self::OldTown => EXPANSION_UNLEASHED,
            Self::StudioTown => EXPANSION_SUPERSTAR,
            Self::MagicTown => EXPANSION_MAKIN_MAGIC,
        }
    }

    /// Default transport mode for this destination.
    #[must_use]
    pub const fn default_transport(self) -> TravelTransport {
        match self {
            Self::Downtown => TravelTransport::DowntownCab,
            Self::VacationIsland => TravelTransport::VacationWagon,
            Self::OldTown => TravelTransport::OldTownTrolley,
            Self::StudioTown => TravelTransport::StudioShuttle,
            Self::MagicTown => TravelTransport::MagicBlimp,
        }
    }

    /// Whether `house` is one of the three Magic Town residential lots
    /// (`90..=92`, zoned `1` in `LotZoning.iff`).
    #[must_use]
    pub fn is_magic_residential(house: u32, zoning: &BTreeMap<i16, i32>) -> bool {
        (90..=92).contains(&house)
            && i16::try_from(house)
                .ok()
                .and_then(|lot| zoning.get(&lot).copied())
                .unwrap_or(1)
                == 1
    }

    /// Whether `house` is a non-residential community/destination lot according
    /// to `0x40745f` / `0x42b9e3` (`zoning != Some(1)`).
    #[must_use]
    pub fn is_community_mode(house: u32, zoning: &BTreeMap<i16, i32>) -> bool {
        i16::try_from(house)
            .ok()
            .and_then(|lot| zoning.get(&lot).copied())
            != Some(1)
    }

    /// `*Desc.iff` filename holding lot names & descriptions (`0x50e14b`).
    #[must_use]
    pub const fn desc_file(house: u32) -> &'static str {
        match house {
            21..=30 => "DTDesc.iff",
            40..=48 => "VIDesc.iff",
            81..=89 => "STDesc.iff",
            90..=99 => "MTDesc.iff",
            _ => "NeighborhoodDesc.iff",
        }
    }
}

/// Classifies a Complete Collection lot number & `LotZoning` table into its
/// [`TravelDestination`], if eligible for away-from-home travel.
#[must_use]
pub fn classify_destination(house: u32, zoning: &BTreeMap<i16, i32>) -> Option<TravelDestination> {
    TravelDestination::classify(house, zoning)
}

/// Transport vehicle or portal used for the away-from-home trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TravelTransport {
    /// Hot Date Yellow Cab (`Car - Taxi`, `$50`).
    DowntownCab,
    /// Vacation Island Station Wagon / Shuttle (`Car - Vacation Wagon`, `$500`).
    VacationWagon,
    /// Unleashed Old Town Trolley (`Car - Public Transit`, `$0`).
    OldTownTrolley,
    /// Superstar Studio Town Shuttle (`Car - Studio`, `$50`).
    StudioShuttle,
    /// Superstar Celebrity Limousine (`Car - Limo`, `$0`, requires fame >= 10).
    StudioLimo,
    /// Makin' Magic Hole in the Ground (`$0`).
    MagicHole,
    /// Makin' Magic Blimp / Magic Wagon (`Car - Magic Wagon`, `$50`).
    MagicBlimp,
}

impl TravelTransport {
    /// Destination associated with this transport mode.
    #[must_use]
    pub const fn destination(self) -> TravelDestination {
        match self {
            Self::DowntownCab => TravelDestination::Downtown,
            Self::VacationWagon => TravelDestination::VacationIsland,
            Self::OldTownTrolley => TravelDestination::OldTown,
            Self::StudioShuttle | Self::StudioLimo => TravelDestination::StudioTown,
            Self::MagicHole | Self::MagicBlimp => TravelDestination::MagicTown,
        }
    }

    /// Default fare in Simoleons deducted on departure.
    #[must_use]
    pub const fn default_fare(self) -> i32 {
        match self {
            Self::DowntownCab => DOWNTOWN_FARE,
            Self::VacationWagon => VACATION_FARE,
            Self::OldTownTrolley => OLD_TOWN_FARE,
            Self::StudioShuttle => STUDIO_TOWN_FARE,
            Self::StudioLimo => STUDIO_LIMO_FARE,
            Self::MagicHole => MAGIC_HOLE_FARE,
            Self::MagicBlimp => MAGIC_BLIMP_FARE,
        }
    }
}

/// Vacation Island lodging mode (`40..=48`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VacationAccommodation {
    /// Standard day visit (or non-Vacation destination).
    #[default]
    None,
    /// Vacation hotel check-in; when `grant_hotel_key` is true, issues
    /// `Token - Hotel Key` (`0x3ac022d6`, `kind = 0`) to traveling party
    /// members so category 4/7 hotel doors and beds unlock natively (`0x458319`),
    /// and revokes the temporary key on check-out/return.
    HotelCheckIn { grant_hotel_key: bool },
    /// Vacation campground (tent/igloo) stay.
    Campground,
}

/// Who is traveling away from home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TravelParty {
    /// A single household Sim (Downtown solo, Studio Town adult, Magic Town).
    Single { sim_guid: u32 },
    /// A Downtown date: a household adult leader + an adult companion
    /// (another household member or neighborhood townie/friend).
    Date {
        leader_guid: u32,
        companion_guid: u32,
    },
    /// The entire living household on the home lot (required for Vacation
    /// Island and Old Town; optional for Magic Town).
    WholeFamily { leader_guid: u32 },
    /// An explicit subset of household members (Magic Town family/group).
    HouseholdGroup {
        leader_guid: u32,
        member_guids: Vec<u32>,
    },
}

impl TravelParty {
    /// The GUID of the party leader (selected on arrival at the destination).
    #[must_use]
    pub const fn leader_guid(&self) -> u32 {
        match *self {
            Self::Single { sim_guid } => sim_guid,
            Self::Date { leader_guid, .. }
            | Self::WholeFamily { leader_guid }
            | Self::HouseholdGroup { leader_guid, .. } => leader_guid,
        }
    }
}

/// Parameters for [`depart_to_destination`].
#[derive(Debug, Clone)]
pub struct TravelRequest {
    /// Target destination house number (e.g. `21`, `40`, `58`, `81`, `90`, `93`).
    pub destination_house: u32,
    /// Traveling Sim or party composition.
    pub party: TravelParty,
    /// Transport mode; `None` selects [`TravelDestination::default_transport`].
    pub transport: Option<TravelTransport>,
    /// Vacation Island accommodation mode (must be [`VacationAccommodation::None`]
    /// for non-Vacation destinations).
    pub accommodation: VacationAccommodation,
    /// Optional non-negative fare override in Simoleons; `None` uses
    /// [`TravelTransport::default_fare`].
    pub fare_override: Option<i32>,
    /// Scripted dialog choice answers for the destination lot.
    pub answers: Vec<u8>,
    /// Frame draw schedule for the destination lot.
    pub frames: Frames,
    /// Optional initial camera tile centre on the destination lot.
    pub view_centre: Option<[i8; 2]>,
}

impl TravelRequest {
    /// Solo Downtown trip (`$50` cab).
    #[must_use]
    pub fn downtown(destination_house: u32, sim_guid: u32) -> Self {
        Self {
            destination_house,
            party: TravelParty::Single { sim_guid },
            transport: Some(TravelTransport::DowntownCab),
            accommodation: VacationAccommodation::None,
            fare_override: None,
            answers: vec![3],
            frames: Frames::default(),
            view_centre: None,
        }
    }

    /// Downtown date trip (`$50` cab) with a household adult and an adult companion.
    #[must_use]
    pub fn downtown_date(destination_house: u32, leader_guid: u32, companion_guid: u32) -> Self {
        Self {
            destination_house,
            party: TravelParty::Date {
                leader_guid,
                companion_guid,
            },
            transport: Some(TravelTransport::DowntownCab),
            accommodation: VacationAccommodation::None,
            fare_override: None,
            answers: vec![3],
            frames: Frames::default(),
            view_centre: None,
        }
    }

    /// Vacation Island whole-family trip (`$500` wagon).
    #[must_use]
    pub fn vacation(
        destination_house: u32,
        leader_guid: u32,
        accommodation: VacationAccommodation,
    ) -> Self {
        Self {
            destination_house,
            party: TravelParty::WholeFamily { leader_guid },
            transport: Some(TravelTransport::VacationWagon),
            accommodation,
            fare_override: None,
            answers: vec![3],
            frames: Frames::default(),
            view_centre: None,
        }
    }

    /// Unleashed Old Town community lot whole-family trip (`$0` trolley).
    #[must_use]
    pub fn old_town(destination_house: u32, leader_guid: u32) -> Self {
        Self {
            destination_house,
            party: TravelParty::WholeFamily { leader_guid },
            transport: Some(TravelTransport::OldTownTrolley),
            accommodation: VacationAccommodation::None,
            fare_override: None,
            answers: vec![3],
            frames: Frames::default(),
            view_centre: None,
        }
    }

    /// Superstar Studio Town adult trip (`$50` shuttle).
    #[must_use]
    pub fn studio_town(destination_house: u32, sim_guid: u32) -> Self {
        Self {
            destination_house,
            party: TravelParty::Single { sim_guid },
            transport: Some(TravelTransport::StudioShuttle),
            accommodation: VacationAccommodation::None,
            fare_override: None,
            answers: vec![3],
            frames: Frames::default(),
            view_centre: None,
        }
    }

    /// Makin' Magic Town trip via Magic Hole (`$0`) or Magic Blimp (`$50`).
    #[must_use]
    pub fn magic_town(
        destination_house: u32,
        party: TravelParty,
        transport: TravelTransport,
    ) -> Self {
        Self {
            destination_house,
            party,
            transport: Some(transport),
            accommodation: VacationAccommodation::None,
            fare_override: None,
            answers: vec![3],
            frames: Frames::default(),
            view_centre: None,
        }
    }
}

/// Errors returned by away-from-home travel operations.
#[derive(Debug)]
pub enum TravelError {
    Store(sims_sim::Error),
    NeighborhoodNotFound(String),
    HouseNotFound {
        neighborhood: String,
        house: u32,
    },
    FamilyNotFound {
        neighborhood: String,
        family: u16,
    },
    NotTravelDestination {
        house: u32,
        zoning: Option<i32>,
    },
    SameHouse {
        house: u32,
    },
    DestinationOccupied {
        house: u32,
        family: u16,
    },
    ExpansionNotInstalled {
        destination: TravelDestination,
        required_bit: u16,
        installed: u16,
    },
    IncompatibleTransport {
        destination: TravelDestination,
        transport: TravelTransport,
    },
    IncompatibleAccommodation {
        destination: TravelDestination,
        accommodation: VacationAccommodation,
    },
    InvalidParty(String),
    InvalidFare(i32),
    InsufficientFunds {
        family: u16,
        available: i32,
        required: i32,
    },
    NeighborhoodMismatch {
        expected: String,
        actual: String,
    },
    StaleNeighborhood {
        neighborhood: String,
        expected_revision: u64,
        current_revision: u64,
    },
    SessionMismatch(String),
    Lot(String),
}

impl fmt::Display for TravelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{error}"),
            Self::NeighborhoodNotFound(id) => write!(f, "neighborhood {id:?} not found in store"),
            Self::HouseNotFound {
                neighborhood,
                house,
            } => write!(
                f,
                "house {house} not found in neighborhood {neighborhood:?}"
            ),
            Self::FamilyNotFound {
                neighborhood,
                family,
            } => write!(
                f,
                "family {family} not found in neighborhood {neighborhood:?}"
            ),
            Self::NotTravelDestination { house, zoning } => write!(
                f,
                "house {house} is not a travel destination (zoning: {zoning:?})"
            ),
            Self::SameHouse { house } => {
                write!(f, "cannot travel from house {house} to itself")
            }
            Self::DestinationOccupied { house, family } => write!(
                f,
                "destination house {house} is occupied by residential family {family}"
            ),
            Self::ExpansionNotInstalled {
                destination,
                required_bit,
                installed,
            } => write!(
                f,
                "destination {destination:?} requires expansion bit 0x{required_bit:02x}, \
                 installed mask is 0x{installed:04x}"
            ),
            Self::IncompatibleTransport {
                destination,
                transport,
            } => write!(
                f,
                "transport {transport:?} cannot serve destination {destination:?}"
            ),
            Self::IncompatibleAccommodation {
                destination,
                accommodation,
            } => write!(
                f,
                "accommodation {accommodation:?} is only valid on Vacation Island, not \
                 {destination:?}"
            ),
            Self::InvalidParty(reason) => write!(f, "invalid travel party: {reason}"),
            Self::InvalidFare(fare) => write!(f, "invalid negative travel fare: ${fare}"),
            Self::InsufficientFunds {
                family,
                available,
                required,
            } => write!(
                f,
                "family {family} has ${available} but travel fare requires ${required}"
            ),
            Self::NeighborhoodMismatch { expected, actual } => write!(
                f,
                "neighborhood mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::StaleNeighborhood {
                neighborhood,
                expected_revision,
                current_revision,
            } => write!(
                f,
                "neighborhood {neighborhood:?} changed (expected revision {expected_revision}, \
                 store is at revision {current_revision})"
            ),
            Self::SessionMismatch(reason) => write!(f, "travel session mismatch: {reason}"),
            Self::Lot(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for TravelError {}

impl From<sims_sim::Error> for TravelError {
    fn from(error: sims_sim::Error) -> Self {
        Self::Store(error)
    }
}

/// Persistent identity and book-keeping for one Sim traveling away from home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TravelerRecord {
    pub guid: u32,
    pub neighbor_id: i16,
    pub is_household_member: bool,
    pub home_object_id: Option<u16>,
    pub destination_object_id: u16,
    /// Whether this Sim already held a `Token - Hotel Key` prior to departure
    /// (so temporary check-in keys granted on arrival are cleanly revoked on
    /// return without erasing a pre-existing token).
    pub had_hotel_key_before_trip: bool,
}

/// Active away-from-home travel session returned by [`depart_to_destination`]
/// (or recovered via [`resume_active_trip`] / [`inspect_active_trip`]) and
/// consumed by [`return_from_destination`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TravelSession {
    pub store_id: u128,
    pub neighborhood_id: String,
    /// Neighborhood instance lifetime ID at departure (fences against
    /// delete/re-import of the same neighborhood name).
    pub instance_id: u128,
    /// Unique non-zero departure identifier binding this session, the live
    /// destination handle, and the persisted REDB `TRVL` envelope together.
    pub trip_id: u128,
    pub home_house: u32,
    pub home_family: u16,
    pub destination_house: u32,
    pub destination: TravelDestination,
    pub transport: TravelTransport,
    pub accommodation: VacationAccommodation,
    pub fare_paid: i32,
    /// Home lot's `Lot::ticks()` frozen at departure.
    pub departure_ticks: u32,
    /// Home lot's `World::ticks` frozen at departure.
    pub departure_world_ticks: u32,
    /// Home lot's `(hours, minutes, seconds)` clock frozen at departure.
    pub departure_clock: (i16, i16, i16),
    /// Household funds prior to deducting `fare_paid`.
    pub funds_before_fare: i32,
    /// Household funds upon arriving at the destination lot (after fare deduction).
    pub funds_at_arrival: i32,
    /// Household MagiCoins (`kind = 7`, `0x99e81bec`) upon arriving at the destination.
    pub magic_coins_at_arrival: i32,
    /// Leader GUID selected on arrival and re-selected on return home.
    pub leader_guid: u32,
    /// Every Sim injected onto the destination lot for this trip, in party order.
    pub travelers: Vec<TravelerRecord>,
}

/// Output of [`depart_to_destination`].
pub struct DepartureResult {
    /// Live destination [`Lot`] ready to tick with the traveling party injected.
    pub destination_lot: Lot,
    /// Session metadata needed by [`return_from_destination`].
    pub session: TravelSession,
    /// Committed neighborhood revision after freezing the home lot.
    pub home_revision: u64,
}

impl fmt::Debug for DepartureResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepartureResult")
            .field("destination_house", &self.destination_lot.load.house)
            .field("session", &self.session)
            .field("home_revision", &self.home_revision)
            .finish()
    }
}

/// Summary of state reconciled by [`return_from_destination`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnReport {
    pub neighborhood: String,
    pub home_house: u32,
    pub destination_house: u32,
    pub home_family: u16,
    pub destination: TravelDestination,
    pub restored_home_ticks: u32,
    pub restored_home_world_ticks: u32,
    pub restored_home_clock: (i16, i16, i16),
    pub funds_at_arrival: i32,
    pub funds_at_return: i32,
    pub funds_delta: i32,
    pub magic_coins_at_arrival: i32,
    pub magic_coins_at_return: i32,
    pub magic_coins_delta: i32,
    /// Returning household members reconciled onto the home lot: `(guid, home_object_id)`.
    pub returning_travelers: Vec<(u32, u16)>,
    /// Committed neighborhood revision after the atomic home + neighborhood save.
    pub home_revision: u64,
}

/// Output of [`return_from_destination`].
pub struct ReturnResult {
    /// Resumed home [`Lot`] at its frozen departure tick with updated travelers,
    /// funds, relationships, outfits, MagiCoins, and inventory tokens.
    pub home_lot: Lot,
    /// Detailed reconciliation summary.
    pub report: ReturnReport,
}

impl fmt::Debug for ReturnResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReturnResult")
            .field("home_house", &self.home_lot.load.house)
            .field("report", &self.report)
            .finish()
    }
}

/// Validated party member metadata computed before any mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedMember {
    pub guid: u32,
    pub neighbor_id: i16,
    pub is_household_member: bool,
    pub home_object_id: Option<u16>,
    pub age: i16,
    pub kind: i16,
    pub fame: i16,
}

/// Validated departure plan produced by [`validate_travel_plan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedTravelPlan {
    pub destination: TravelDestination,
    pub transport: TravelTransport,
    pub accommodation: VacationAccommodation,
    pub fare: i32,
    pub leader_guid: u32,
    pub members: Vec<ValidatedMember>,
}

/// Whether a person record represents a human (`kind <= 1`; `2..=5` are pets).
#[must_use]
fn is_human(kind: i16) -> bool {
    (0..=1).contains(&kind)
}

/// Whether a human person is an adult (`age == 0 || age >= 18`, matching
/// `curve_table` / `0x459d23`).
#[must_use]
fn is_adult_human(age: i16, kind: i16) -> bool {
    is_human(kind) && (age == 0 || age >= 18)
}

/// Counts a household's total MagiCoins (`kind = 7`, `guid = 0x99e81bec`) across
/// all members' inventories (`cFamily::GetMagicCoins`, `0x410282`).
#[must_use]
pub fn family_magic_coins(world: &World, family_members: &[u32]) -> i32 {
    let mut total = 0i32;
    for &guid in family_members {
        if guid == 0 {
            continue;
        }
        let Some(record) = world.neighbors.first_of(guid) else {
            continue;
        };
        if let Some(inv) = world
            .neighbors
            .inventories
            .iter()
            .find(|i| i.neighbor == record.id)
        {
            for item in &inv.items {
                if item.kind == MAGICOIN_TOKEN_KIND && item.guid == MAGICOIN_TOKEN_GUID {
                    total = total.saturating_add(i32::from(item.count));
                }
            }
        }
    }
    total
}

/// Returns the current token count for `(neighbor_id, kind, guid)` in `world`.
#[must_use]
pub fn neighbor_token_count(world: &World, neighbor_id: i16, kind: i32, guid: u32) -> u16 {
    world
        .neighbors
        .inventories
        .iter()
        .find(|inv| inv.neighbor == neighbor_id)
        .and_then(|inv| {
            inv.items
                .iter()
                .find(|item| item.kind == kind && item.guid == guid)
                .map(|item| item.count)
        })
        .unwrap_or(0)
}

/// Sets the token count for `(neighbor_id, kind, guid)` in `world.neighbors.inventories`
/// (creating the inventory entry if needed; removing the token when `count == 0`).
pub fn set_neighbor_token_count(
    world: &mut World,
    neighbor_id: i16,
    kind: i32,
    guid: u32,
    count: u16,
) {
    let pos = world
        .neighbors
        .inventories
        .iter()
        .position(|inv| inv.neighbor == neighbor_id);
    let inv = match pos {
        Some(idx) => &mut world.neighbors.inventories[idx],
        None => {
            if count == 0 {
                return;
            }
            world.neighbors.inventories.push(Inventory {
                neighbor: neighbor_id,
                items: Vec::new(),
            });
            world.neighbors.inventories.last_mut().expect("just pushed")
        }
    };
    if let Some(idx) = inv
        .items
        .iter()
        .position(|item| item.kind == kind && item.guid == guid)
    {
        if count == 0 {
            inv.items.remove(idx);
        } else {
            inv.items[idx].count = count;
        }
    } else if count > 0 {
        inv.items.push(Item { kind, guid, count });
    }
}

/// Validates destination classification, expansion availability, transport,
/// accommodation, party eligibility, and fare against the home world and
/// stored neighborhood without mutating any state.
#[allow(clippy::too_many_arguments)]
pub fn validate_travel_plan(
    home_world: &World,
    home_house: u32,
    home_family_id: u16,
    installed_expansions: u16,
    stored: &StoredNeighborhood,
    request: &TravelRequest,
) -> Result<ValidatedTravelPlan, TravelError> {
    let dest_house = request.destination_house;
    if dest_house == home_house {
        return Err(TravelError::SameHouse { house: home_house });
    }
    let zoning = i16::try_from(dest_house)
        .ok()
        .and_then(|lot| stored.zoning.get(&lot).copied());
    let Some(destination) = TravelDestination::classify(dest_house, &stored.zoning) else {
        return Err(TravelError::NotTravelDestination {
            house: dest_house,
            zoning,
        });
    };
    let required_bit = destination.expansion_bit();
    if installed_expansions & required_bit == 0 {
        return Err(TravelError::ExpansionNotInstalled {
            destination,
            required_bit,
            installed: installed_expansions,
        });
    }
    if let Some((occupant, _)) = stored.family_in(dest_house) {
        return Err(TravelError::DestinationOccupied {
            house: dest_house,
            family: occupant,
        });
    }
    let transport = request
        .transport
        .unwrap_or_else(|| destination.default_transport());
    if transport.destination() != destination {
        return Err(TravelError::IncompatibleTransport {
            destination,
            transport,
        });
    }
    if destination != TravelDestination::VacationIsland
        && request.accommodation != VacationAccommodation::None
    {
        return Err(TravelError::IncompatibleAccommodation {
            destination,
            accommodation: request.accommodation,
        });
    }
    let fare = request
        .fare_override
        .unwrap_or_else(|| transport.default_fare());
    if fare < 0 {
        return Err(TravelError::InvalidFare(fare));
    }
    if home_family_id == 0 {
        return Err(TravelError::InvalidParty(
            "home lot has no active household (family 0)".into(),
        ));
    }
    let family =
        stored
            .families
            .get(&home_family_id)
            .ok_or_else(|| TravelError::FamilyNotFound {
                neighborhood: stored.name.clone(),
                family: home_family_id,
            })?;
    let home_house_i32 = i32::try_from(home_house).map_err(|e| TravelError::Lot(e.to_string()))?;
    if family.house != home_house_i32 {
        return Err(TravelError::SessionMismatch(format!(
            "family {home_family_id} lives on house {}, not home house {home_house}",
            family.house
        )));
    }
    if family.secondary_house() != 0 {
        return Err(TravelError::SessionMismatch(format!(
            "family {home_family_id} is already away at destination house {}",
            family.secondary_house()
        )));
    }
    if family.members.is_empty() {
        return Err(TravelError::InvalidParty(format!(
            "family {home_family_id} has no members"
        )));
    }

    // Departures require live budget mode (`globals[32] == 0`) so fare
    // deductions always execute against real household funds.
    let budget_mode = home_world
        .globals
        .get(GLOBAL_BUDGET_MODE)
        .copied()
        .unwrap_or(0);
    if budget_mode != 0 {
        return Err(TravelError::SessionMismatch(format!(
            "cannot depart while home lot {home_house} is in unlimited budget mode (globals[32]={budget_mode})"
        )));
    }
    if fare > 0 && home_world.budget.funds < fare {
        return Err(TravelError::InsufficientFunds {
            family: home_family_id,
            available: home_world.budget.funds,
            required: fare,
        });
    }

    let lookup_member = |guid: u32,
                         must_be_household: bool|
     -> Result<ValidatedMember, TravelError> {
        if stored.deleted_characters.contains(&guid) {
            return Err(TravelError::InvalidParty(format!(
                "Sim 0x{guid:08X} has been deleted from the neighborhood"
            )));
        }
        let is_member = family.members.contains(&guid);
        if must_be_household && !is_member {
            return Err(TravelError::InvalidParty(format!(
                "Sim 0x{guid:08X} is not a member of household {home_family_id}"
            )));
        }
        let home_obj = home_world.people.iter().copied().find(|&id| {
            home_world
                .objects
                .get(&id)
                .is_some_and(|o| o.guid == guid && o.person.is_some())
        });
        let record = home_world.neighbors.first_of(guid).ok_or_else(|| {
            TravelError::InvalidParty(format!(
                "Sim 0x{guid:08X} has no neighborhood record in NBRS"
            ))
        })?;
        let (age, kind, fame) = if let Some(obj_id) = home_obj
            && let Some(person) = home_world
                .objects
                .get(&obj_id)
                .and_then(|o| o.person.as_ref())
        {
            (
                person.data.get(PERSON_AGE).copied().unwrap_or(0),
                person.data.get(PERSON_KIND).copied().unwrap_or(0),
                person.data.get(PERSON_FAME_LEVEL).copied().unwrap_or(0),
            )
        } else {
            (
                record.data.get(PERSON_AGE).copied().unwrap_or(0),
                record.data.get(PERSON_KIND).copied().unwrap_or(0),
                record.data.get(PERSON_FAME_LEVEL).copied().unwrap_or(0),
            )
        };
        if must_be_household && home_obj.is_none() {
            return Err(TravelError::InvalidParty(format!(
                "household member 0x{guid:08X} is not currently present on home lot {home_house}"
            )));
        }
        Ok(ValidatedMember {
            guid,
            neighbor_id: record.id,
            is_household_member: is_member,
            home_object_id: home_obj,
            age,
            kind,
            fame,
        })
    };

    // Collect all living household members currently on the home lot.
    let home_members_present: Vec<u32> = family
        .members
        .iter()
        .copied()
        .filter(|&guid| {
            home_world.people.iter().any(|&id| {
                home_world
                    .objects
                    .get(&id)
                    .is_some_and(|o| o.guid == guid && o.person.is_some())
            })
        })
        .collect();

    let leader_guid = request.party.leader_guid();
    let leader = lookup_member(leader_guid, true)?;

    let mut members: Vec<ValidatedMember> = Vec::new();
    match (&destination, &request.party) {
        (TravelDestination::Downtown, TravelParty::Single { .. }) => {
            if !is_adult_human(leader.age, leader.kind) {
                return Err(TravelError::InvalidParty(
                    "Downtown solo trips require an adult human household member".into(),
                ));
            }
            members.push(leader);
        }
        (TravelDestination::Downtown, TravelParty::Date { companion_guid, .. }) => {
            if !is_adult_human(leader.age, leader.kind) {
                return Err(TravelError::InvalidParty(
                    "Downtown date leader must be an adult human household member".into(),
                ));
            }
            if *companion_guid == leader.guid {
                return Err(TravelError::InvalidParty(
                    "Downtown date companion must be distinct from the leader".into(),
                ));
            }
            let companion = lookup_member(*companion_guid, false)?;
            if !is_adult_human(companion.age, companion.kind) {
                return Err(TravelError::InvalidParty(
                    "Downtown date companion must be an adult human Sim".into(),
                ));
            }
            members.push(leader);
            members.push(companion);
        }
        (TravelDestination::Downtown, _) => {
            return Err(TravelError::InvalidParty(
                "Downtown accepts only TravelParty::Single or TravelParty::Date".into(),
            ));
        }

        (TravelDestination::VacationIsland, TravelParty::WholeFamily { .. }) => {
            if !is_adult_human(leader.age, leader.kind) {
                return Err(TravelError::InvalidParty(
                    "Vacation Island trips require an adult human leader".into(),
                ));
            }
            members.push(leader);
            for &guid in &home_members_present {
                if guid != leader_guid {
                    members.push(lookup_member(guid, true)?);
                }
            }
        }
        (TravelDestination::VacationIsland, _) => {
            return Err(TravelError::InvalidParty(
                "Vacation Island requires TravelParty::WholeFamily".into(),
            ));
        }

        (TravelDestination::OldTown, TravelParty::WholeFamily { .. }) => {
            if !is_human(leader.kind) {
                return Err(TravelError::InvalidParty(
                    "Old Town trips require a human household member as leader".into(),
                ));
            }
            members.push(leader);
            for &guid in &home_members_present {
                if guid != leader_guid {
                    members.push(lookup_member(guid, true)?);
                }
            }
        }
        (TravelDestination::OldTown, _) => {
            return Err(TravelError::InvalidParty(
                "Old Town community trips require TravelParty::WholeFamily".into(),
            ));
        }

        (TravelDestination::StudioTown, TravelParty::Single { .. }) => {
            if !is_adult_human(leader.age, leader.kind) {
                return Err(TravelError::InvalidParty(
                    "Studio Town trips require a single adult human household member".into(),
                ));
            }
            if transport == TravelTransport::StudioLimo && leader.fame < STUDIO_LIMO_MIN_FAME {
                return Err(TravelError::InvalidParty(format!(
                    "Studio Town limo requires fame >= {STUDIO_LIMO_MIN_FAME}, leader has {}",
                    leader.fame
                )));
            }
            members.push(leader);
        }
        (TravelDestination::StudioTown, _) => {
            return Err(TravelError::InvalidParty(
                "Studio Town accepts only TravelParty::Single (adult Sim)".into(),
            ));
        }

        (TravelDestination::MagicTown, TravelParty::Single { .. }) => {
            if !is_human(leader.kind) {
                return Err(TravelError::InvalidParty(
                    "Magic Town solo trips require a human Sim (adult or child)".into(),
                ));
            }
            members.push(leader);
        }
        (TravelDestination::MagicTown, TravelParty::WholeFamily { .. }) => {
            if !is_human(leader.kind) {
                return Err(TravelError::InvalidParty(
                    "Magic Town family trips require a human leader".into(),
                ));
            }
            members.push(leader);
            for &guid in &home_members_present {
                if guid != leader_guid {
                    members.push(lookup_member(guid, true)?);
                }
            }
        }
        (TravelDestination::MagicTown, TravelParty::HouseholdGroup { member_guids, .. }) => {
            if !is_human(leader.kind) {
                return Err(TravelError::InvalidParty(
                    "Magic Town group leader must be a human household member".into(),
                ));
            }
            // Allow `leader_guid` to appear at most once in `member_guids` and
            // reject any duplicate GUID (including repeated leader GUIDs).
            let mut seen_in_list = BTreeSet::new();
            for &guid in member_guids {
                if !seen_in_list.insert(guid) {
                    return Err(TravelError::InvalidParty(format!(
                        "duplicate member GUID 0x{guid:08X} in travel party"
                    )));
                }
            }
            members.push(leader);
            for &guid in member_guids {
                if guid == leader_guid {
                    continue;
                }
                members.push(lookup_member(guid, true)?);
            }
        }
        (TravelDestination::MagicTown, TravelParty::Date { .. }) => {
            return Err(TravelError::InvalidParty(
                "Magic Town does not accept TravelParty::Date; use Single, WholeFamily, or \
                 HouseholdGroup"
                    .into(),
            ));
        }
    }

    Ok(ValidatedTravelPlan {
        destination,
        transport,
        accommodation: request.accommodation,
        fare,
        leader_guid,
        members,
    })
}

/// Live per-traveler snapshot carried across the home ↔ destination boundary
/// (and persisted in the REDB `TRVL` envelope for crash/restart recovery).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LiveTravelerSnapshot {
    guid: u32,
    neighbor_id: i16,
    is_household_member: bool,
    home_object_id: Option<u16>,
    motives: [f32; 16],
    previous_motives: [f32; 16],
    person_data: Vec<i16>,
    accessories: Vec<Accessory>,
    outfit: Option<PersonOutfit>,
    made_outfits: Option<BTreeMap<usize, String>>,
    had_hotel_key_before_trip: bool,
}

/// Departure neighborhood snapshot used as the 3-way merge base when
/// `return_from_destination` reconciles destination changes against a
/// `StoredNeighborhood` that advanced due to unrelated concurrent writes.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DepartureBaseline {
    revision: u64,
    neighbors: Vec<Option<sims_formats::neighborhood::Neighbor>>,
    inventories: Vec<Option<sims_formats::neighborhood::Inventory>>,
    type_attributes: Option<sims_formats::tatt::Tatt>,
    made_characters: sims_sim::characters::Characters,
    home_family_members: Vec<u32>,
    home_family_budget: i32,
}

impl DepartureBaseline {
    fn from_stored(stored: &StoredNeighborhood, committed_revision: u64, home_family: u16) -> Self {
        let (members, budget) = stored
            .families
            .get(&home_family)
            .map(|f| (f.members.clone(), f.budget))
            .unwrap_or_default();
        Self {
            revision: committed_revision,
            neighbors: stored.neighbors.clone(),
            inventories: stored.inventories.clone(),
            type_attributes: stored.type_attributes.clone(),
            made_characters: stored.made_characters.clone(),
            home_family_members: members,
            home_family_budget: budget,
        }
    }
}

/// Active-trip envelope persisted atomically in
/// `NEIGHBORHOOD_LOTS[(neighborhood_id, home_house)]` while away.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedTravelRecord {
    trip_id: u128,
    is_date: bool,
    view_centre: Option<[i8; 2]>,
    session: TravelSession,
    baseline: DepartureBaseline,
    departure_snapshots: Vec<LiveTravelerSnapshot>,
    frozen_home_bytes: Vec<u8>,
}

fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn alloc_trip_id(
    store_id: u128,
    instance_id: u128,
    revision: u64,
    home_house: u32,
    dest_house: u32,
    home_family: u16,
) -> u128 {
    let seq = NEXT_TRIP_SEQ.fetch_add(1, Ordering::Relaxed);
    let hi = mix64(
        (store_id as u64)
            ^ ((instance_id >> 64) as u64)
            ^ revision.rotate_left(17)
            ^ ((u64::from(home_house) << 32) | u64::from(dest_house)),
    );
    let lo = mix64(
        ((store_id >> 64) as u64)
            ^ (instance_id as u64)
            ^ seq.rotate_left(29)
            ^ u64::from(home_family),
    );
    let id = (u128::from(hi) << 64) | u128::from(lo);
    if id == 0 { 1 } else { id }
}

fn trip_binding_tag(store_id: u128, trip_id: u128) -> [u8; 32] {
    let w0 = mix64((store_id as u64) ^ (trip_id as u64) ^ 0x5452_564c_0001);
    let w1 = mix64(((store_id >> 64) as u64) ^ ((trip_id >> 64) as u64) ^ 0x5452_564c_0002);
    let w2 = mix64(
        (store_id as u64).rotate_left(19) ^ (trip_id as u64).rotate_left(13) ^ 0x5452_564c_0003,
    );
    let w3 = mix64(
        ((store_id >> 64) as u64).rotate_left(7)
            ^ ((trip_id >> 64) as u64).rotate_left(23)
            ^ 0x5452_564c_0004,
    );
    let mut out = [0u8; 32];
    out[0..8].copy_from_slice(&w0.to_le_bytes());
    out[8..16].copy_from_slice(&w1.to_le_bytes());
    out[16..24].copy_from_slice(&w2.to_le_bytes());
    out[24..32].copy_from_slice(&w3.to_le_bytes());
    if out == [0u8; 32] {
        out[0] = 1;
    }
    out
}

fn is_travel_envelope(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && &bytes[0..4] == TRAVEL_ENVELOPE_MAGIC
}

fn encode_travel_envelope(record: &PersistedTravelRecord) -> Result<Vec<u8>, TravelError> {
    let payload = postcard::to_allocvec(record)
        .map_err(|e| TravelError::Lot(format!("encoding active travel envelope: {e}")))?;
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(TRAVEL_ENVELOPE_MAGIC);
    out.extend_from_slice(&TRAVEL_ENVELOPE_VERSION.to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

fn decode_travel_envelope(bytes: &[u8]) -> Result<PersistedTravelRecord, TravelError> {
    if !is_travel_envelope(bytes) {
        return Err(TravelError::SessionMismatch(
            "home lot does not hold an active away-from-home travel envelope".into(),
        ));
    }
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if version != TRAVEL_ENVELOPE_VERSION {
        return Err(TravelError::SessionMismatch(format!(
            "unsupported travel envelope version {version} (expected {TRAVEL_ENVELOPE_VERSION})"
        )));
    }
    let (record, rest) = postcard::take_from_bytes::<PersistedTravelRecord>(&bytes[8..])
        .map_err(|e| TravelError::SessionMismatch(format!("corrupt travel envelope: {e}")))?;
    if !rest.is_empty() {
        return Err(TravelError::SessionMismatch(format!(
            "corrupt travel envelope: {} trailing byte(s)",
            rest.len()
        )));
    }
    if record.trip_id == 0 || record.trip_id != record.session.trip_id {
        return Err(TravelError::SessionMismatch(
            "corrupt travel envelope: inconsistent trip_id".into(),
        ));
    }
    Ok(record)
}

/// Captures live traveler state from `world` (falling back to `NBRS` person data
/// for off-lot date companions).
fn capture_traveler_snapshot(
    world: &World,
    member: &ValidatedMember,
) -> Result<LiveTravelerSnapshot, TravelError> {
    let had_hotel_key = neighbor_token_count(
        world,
        member.neighbor_id,
        HOTEL_KEY_TOKEN_KIND,
        HOTEL_KEY_TOKEN_GUID,
    ) > 0;
    let made_outfits = world.made_outfits.get(&member.guid).cloned();
    if let Some(obj_id) = member.home_object_id
        && let Some(obj) = world.objects.get(&obj_id)
        && let Some(person) = obj.person.as_ref()
    {
        return Ok(LiveTravelerSnapshot {
            guid: member.guid,
            neighbor_id: member.neighbor_id,
            is_household_member: member.is_household_member,
            home_object_id: Some(obj_id),
            motives: person.motives,
            previous_motives: person.previous_motives,
            person_data: person.data.clone(),
            accessories: person.accessories.clone(),
            outfit: world.person_outfits.get(&obj_id).cloned(),
            made_outfits,
            had_hotel_key_before_trip: had_hotel_key,
        });
    }
    let record = world.neighbors.get(member.neighbor_id).ok_or_else(|| {
        TravelError::InvalidParty(format!(
            "missing neighbor record {} for Sim 0x{:08X}",
            member.neighbor_id, member.guid
        ))
    })?;
    Ok(LiveTravelerSnapshot {
        guid: member.guid,
        neighbor_id: member.neighbor_id,
        is_household_member: member.is_household_member,
        home_object_id: None,
        // Neutral full motives for an off-lot date companion joining from town.
        motives: [50.0; 16],
        previous_motives: [50.0; 16],
        person_data: record.data.clone(),
        accessories: Vec::new(),
        outfit: None,
        made_outfits,
        had_hotel_key_before_trip: had_hotel_key,
    })
}

/// Safely selects `sim` on `lot`, handling Superstar celebrity deselection
/// (`person.data[81] >= 10`, unported star-marker pool `0x446c15`) and active
/// Downtown date links (`person.data[77] != 0`, unported second indicator
/// `0x44664c`) while preserving exact fame and date words on every Sim.
pub fn select_sim(lot: &mut Lot, sim: u16) -> Result<Vec<crate::SoundEvent>, TravelError> {
    let mut saved: Vec<(u16, i16, i16)> = Vec::new();
    let prev = selected(&lot.world);
    for id in prev.into_iter().chain(std::iter::once(sim)) {
        if saved.iter().any(|&(s, _, _)| s == id) {
            continue;
        }
        if let Some(person) = lot
            .world
            .objects
            .get_mut(&id)
            .and_then(|o| o.person.as_mut())
        {
            let fame = person.data.get(PERSON_FAME_LEVEL).copied().unwrap_or(0);
            let date = person.data.get(PERSON_DATE).copied().unwrap_or(0);
            saved.push((id, fame, date));
            if let Some(f) = person.data.get_mut(PERSON_FAME_LEVEL) {
                *f = 0;
            }
            if let Some(d) = person.data.get_mut(PERSON_DATE) {
                *d = 0;
            }
        }
    }
    let result = lot.select(sim);
    for (id, fame, date) in saved {
        if let Some(person) = lot
            .world
            .objects
            .get_mut(&id)
            .and_then(|o| o.person.as_mut())
        {
            if let Some(f) = person.data.get_mut(PERSON_FAME_LEVEL) {
                *f = fame;
            }
            if let Some(d) = person.data.get_mut(PERSON_DATE) {
                *d = date;
            }
        }
    }
    result.map_err(|e| TravelError::Lot(format!("selecting Sim {sim}: {e}")))
}

/// Internal world-level equivalent of [`select_sim`] used during destination
/// arrival and home reconciliation.
fn select_sim_in_world(
    interpreter: &Interpreter<'_>,
    world: &mut World,
    sim: u16,
) -> Result<(), TravelError> {
    let mut saved: Vec<(u16, i16, i16)> = Vec::new();
    let prev = selected(world);
    for id in prev.into_iter().chain(std::iter::once(sim)) {
        if saved.iter().any(|&(s, _, _)| s == id) {
            continue;
        }
        if let Some(person) = world.objects.get_mut(&id).and_then(|o| o.person.as_mut()) {
            let fame = person.data.get(PERSON_FAME_LEVEL).copied().unwrap_or(0);
            let date = person.data.get(PERSON_DATE).copied().unwrap_or(0);
            saved.push((id, fame, date));
            if let Some(f) = person.data.get_mut(PERSON_FAME_LEVEL) {
                *f = 0;
            }
            if let Some(d) = person.data.get_mut(PERSON_DATE) {
                *d = 0;
            }
        }
    }
    let result = interpreter.select(world, Some(sim));
    for (id, fame, date) in saved {
        if let Some(person) = world.objects.get_mut(&id).and_then(|o| o.person.as_mut()) {
            if let Some(f) = person.data.get_mut(PERSON_FAME_LEVEL) {
                *f = fame;
            }
            if let Some(d) = person.data.get_mut(PERSON_DATE) {
                *d = date;
            }
        }
    }
    result.map_err(|e| TravelError::Lot(format!("selecting Sim {sim}: {e}")))
}

/// Atomically departs from `home_lot` to `request.destination_house`:
///
/// 1. Validates destination classification, expansion bit, occupancy,
///    transport, accommodation, party eligibility, and household funds.
/// 2. Stages the frozen home lot snapshot (deducting the fare, flushing every
///    live home person to `NBRS`, recomputing lot values, preserving the home
///    clock and non-traveling Sims/pets, and setting `family.secondary_house =
///    destination_house` per `0x425d95`).
/// 3. Boots the destination lot from REDB (`saved_lot` if previously visited/
///    edited, otherwise `base_house`), configures away-from-home live mode
///    (`globals[32] = 0`, `globals[9] = home_family`, `globals[10] =
///    destination_house`), injects the traveling party with their live motives,
///    skills, career/fame, relationships, outfits, funds, MagiCoins, and
///    inventory tokens, and selects the party leader.
/// 4. Commits the frozen home snapshot and updated `StoredNeighborhood` to REDB
///    only after destination boot succeeds (so any failure leaves REDB and
///    `home_lot` untouched).
pub fn depart_to_destination(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    home_lot: &mut Lot,
    request: &TravelRequest,
) -> Result<DepartureResult, TravelError> {
    if home_lot.load.finished.is_none() {
        return Err(TravelError::Lot("home lot load did not complete".into()));
    }
    if home_lot.halted {
        return Err(TravelError::Lot("home lot is halted".into()));
    }
    if home_lot.asking.is_some() || home_lot.asking_special_panel.is_some() || home_lot.answered {
        return Err(TravelError::Lot(
            "home lot is waiting on an unanswered modal dialog or special panel".into(),
        ));
    }
    let Some(origin) = home_lot.neighborhood_origin.clone() else {
        return Err(TravelError::Lot(
            "home lot was not opened from a NeighborhoodStore (or was replaced via restore)".into(),
        ));
    };
    if origin.store_id != store.store_id() || origin.neighborhood_id != neighborhood_id {
        return Err(TravelError::NeighborhoodMismatch {
            expected: format!("{}@store:{:032x}", origin.neighborhood_id, origin.store_id),
            actual: format!("{neighborhood_id}@store:{:032x}", store.store_id()),
        });
    }

    let dest_snapshot = store
        .load_lot_snapshot(neighborhood_id, request.destination_house)?
        .ok_or_else(|| TravelError::NeighborhoodNotFound(neighborhood_id.to_owned()))?;
    if dest_snapshot.neighborhood.instance_id != origin.instance_id {
        return Err(TravelError::NeighborhoodMismatch {
            expected: format!("{neighborhood_id}#{:032x}", origin.instance_id),
            actual: format!(
                "{neighborhood_id}#{:032x}",
                dest_snapshot.neighborhood.instance_id
            ),
        });
    }
    if dest_snapshot.neighborhood.revision != origin.revision {
        return Err(TravelError::StaleNeighborhood {
            neighborhood: neighborhood_id.to_owned(),
            expected_revision: origin.revision,
            current_revision: dest_snapshot.neighborhood.revision,
        });
    }
    if origin.revision == u64::MAX {
        return Err(TravelError::Lot(
            "neighborhood revision counter exhausted before departure".into(),
        ));
    }
    if dest_snapshot.saved_lot.is_none() && dest_snapshot.base_house.is_none() {
        return Err(TravelError::HouseNotFound {
            neighborhood: neighborhood_id.to_owned(),
            house: request.destination_house,
        });
    }

    let install = Install::detect(install_root).map_err(|e| TravelError::Lot(e.to_string()))?;
    let detected_expansions = u16::from(install.expansions.0);
    let effective_expansions = detected_expansions & home_lot.program.expansions;

    let home_house = home_lot.load.house;
    let home_family = home_lot.load.family;
    let plan = validate_travel_plan(
        &home_lot.world,
        home_house,
        home_family,
        effective_expansions,
        &dest_snapshot.neighborhood,
        request,
    )?;

    // Capture live traveler snapshots before mutating or freezing anything.
    let mut snapshots = Vec::with_capacity(plan.members.len());
    for member in &plan.members {
        snapshots.push(capture_traveler_snapshot(&home_lot.world, member)?);
    }

    let family_members = dest_snapshot.neighborhood.families[&home_family]
        .members
        .clone();
    let funds_before_fare = home_lot.world.budget.funds;
    let magic_coins_at_arrival = family_magic_coins(&home_lot.world, &family_members);
    let departure_ticks = home_lot.ticks;
    let departure_world_ticks = home_lot.world.ticks;
    let departure_clock = (
        home_lot
            .world
            .globals
            .get(GLOBAL_HOURS)
            .copied()
            .unwrap_or(0),
        home_lot
            .world
            .globals
            .get(GLOBAL_MINUTES)
            .copied()
            .unwrap_or(0),
        home_lot
            .world
            .globals
            .get(GLOBAL_SECONDS)
            .copied()
            .unwrap_or(0),
    );

    // Stage the frozen home lot snapshot (without mutating `home_lot` yet).
    let mut frozen_home = home_lot
        .save()
        .map_err(|e| TravelError::Lot(format!("snapshotting home lot {home_house}: {e}")))?;
    ensure_today_budget_slot(&mut frozen_home.world);

    // Deduct travel fare from the staged home lot snapshot.
    if plan.fare > 0 {
        let new_funds = frozen_home.world.budget.funds.wrapping_sub(plan.fare);
        frozen_home
            .world
            .set_funds(new_funds)
            .map_err(|e| TravelError::Lot(format!("deducting travel fare: {e}")))?;
    }
    let funds_at_arrival = frozen_home.world.budget.funds;
    let actual_fare = funds_before_fare.wrapping_sub(funds_at_arrival);
    if actual_fare > 0
        && let Some(Some(today)) = frozen_home.world.budget.days.first_mut()
    {
        today[5] = today[5].wrapping_add(actual_fare);
    }

    // Flush every live home person into `NBRS` on the staged frozen home copy.
    for person_id in frozen_home.world.people.clone() {
        if frozen_home
            .world
            .objects
            .get(&person_id)
            .is_some_and(|o| o.person.is_some())
        {
            people::save_to_record(&mut frozen_home.world, person_id)
                .map_err(|e| TravelError::Lot(format!("saving home person {person_id}: {e}")))?;
        }
    }

    // Recompute home lot values & write back neighborhood state onto a staged copy.
    let prices = LotPrices::load(&install, &home_lot.surfaces);
    let tuning = NeighborhoodTuning::from_program(&home_lot.program);
    let home_interp = Interpreter {
        view_rotation: home_lot.view_rotation,
        view_centre: home_lot.view_centre,
        ..Interpreter::new(&home_lot.program, &home_lot.catalog, Stream::Global)
    };
    let family_flags = update_lot_values(&mut frozen_home.world, &home_interp, &prices)?;
    let home_total_value = lot_total_value(
        frozen_home.world.budget.lot_value,
        frozen_home.world.budget.objects_value,
        frozen_home.world.budget.architecture_value,
        tuning.arch_value_multiplier,
    );

    let mut staged_stored = dest_snapshot.neighborhood.clone();
    staged_stored.write_back_world(
        &frozen_home.world,
        home_house,
        home_family,
        home_total_value,
        Some(family_flags),
        tuning.friendship_threshold,
    );
    // Mark `secondary_house = destination_house` (`0x425d95`, FAMI `+0x13c`)
    // while the household is away on the destination lot.
    if let Some(fam) = staged_stored.families.get_mut(&home_family) {
        fam.upgrade_to_v9();
        let dest_i32 = i32::try_from(request.destination_house)
            .map_err(|e| TravelError::Lot(e.to_string()))?;
        fam.set_secondary_house(dest_i32);
    }
    frozen_home.deletion_epoch = staged_stored.deletion_epoch;

    let trip_id = alloc_trip_id(
        store.store_id(),
        origin.instance_id,
        origin.revision,
        home_house,
        request.destination_house,
        home_family,
    );
    let is_date = matches!(request.party, TravelParty::Date { .. });

    // Boot destination lot in memory BEFORE committing REDB, so any destination
    // boot/injection failure rolls back cleanly with zero REDB/home mutations.
    let runtime = StoreRuntime::from_snapshot(
        install_root,
        staged_stored.clone(),
        &dest_snapshot.character_files,
    )?;
    let (mut dest_lot, traveler_records) = boot_destination_lot(
        runtime,
        store.store_id(),
        trip_id,
        neighborhood_id,
        request.destination_house,
        plan.transport,
        plan.accommodation,
        plan.leader_guid,
        is_date,
        request.view_centre,
        &request.answers,
        request.frames.clone(),
        home_family,
        funds_at_arrival,
        &snapshots,
        dest_snapshot.saved_lot.as_deref(),
        dest_snapshot.base_house,
    )?;

    let mut session = TravelSession {
        store_id: store.store_id(),
        instance_id: origin.instance_id,
        trip_id,
        neighborhood_id: neighborhood_id.to_owned(),
        home_house,
        home_family,
        destination_house: request.destination_house,
        destination: plan.destination,
        transport: plan.transport,
        accommodation: plan.accommodation,
        fare_paid: actual_fare,
        departure_ticks,
        departure_world_ticks,
        departure_clock,
        funds_before_fare,
        funds_at_arrival,
        magic_coins_at_arrival,
        leader_guid: plan.leader_guid,
        travelers: traveler_records,
    };

    // Destination boot succeeded: persist the authoritative trip envelope
    // (wrapping `frozen_home_bytes`, `session`, and `departure_snapshots`) +
    // updated neighborhood atomically in REDB with an in-transaction revision
    // check. Wrapping in `TRVL` simultaneously enables crash/halt recovery and
    // prevents ordinary `lifecycle::enter_lot` from opening or overwriting the
    // frozen home lot while the household is away.
    let frozen_home_bytes = frozen_home.encode();
    let baseline = DepartureBaseline::from_stored(&staged_stored, origin.revision + 1, home_family);
    let envelope_bytes = encode_travel_envelope(&PersistedTravelRecord {
        trip_id,
        is_date,
        view_centre: request.view_centre,
        session: session.clone(),
        baseline,
        departure_snapshots: snapshots,
        frozen_home_bytes,
    })?;
    let (committed_instance, committed_revision) = match store.save_lot_and_neighborhood_checked(
        neighborhood_id,
        home_house,
        Some((origin.instance_id, origin.revision)),
        &envelope_bytes,
        &staged_stored,
        &[],
    )? {
        Ok(version) => version,
        Err(CheckedCommitError::InstanceMismatch { expected, current }) => {
            return Err(TravelError::NeighborhoodMismatch {
                expected: format!("{neighborhood_id}#{expected:032x}"),
                actual: format!("{neighborhood_id}#{current:032x}"),
            });
        }
        Err(CheckedCommitError::RevisionMismatch { expected, current }) => {
            return Err(TravelError::StaleNeighborhood {
                neighborhood: neighborhood_id.to_owned(),
                expected_revision: expected,
                current_revision: current,
            });
        }
    };

    // Invalidate the frozen `home_lot` handle's origin so a caller cannot
    // overwrite REDB via `save_and_leave_lot` or depart again while away.
    // `dest_lot` already has `neighborhood_origin = None` (so ordinary
    // `save_and_leave_lot` can never overwrite the Build/Buy destination
    // snapshot) and `loaded = trip_binding_tag(store_id, trip_id)`.
    if actual_fare > 0 {
        let _ = home_lot.world.set_funds(funds_at_arrival);
    }
    home_lot.neighborhood_origin = None;
    session.instance_id = committed_instance;
    dest_lot.neighborhood_origin = None;
    dest_lot.loaded = trip_binding_tag(store.store_id(), trip_id);

    Ok(DepartureResult {
        destination_lot: dest_lot,
        session,
        home_revision: committed_revision,
    })
}

/// Inspects whether `home_house` in `neighborhood_id` currently has an active
/// away-from-home trip persisted in REDB (e.g. after a process restart or
/// destination fault).
///
/// Returns `Ok(None)` when `home_house` is at home normally, `Ok(Some(session))`
/// when a valid active trip envelope matches `StoredNeighborhood`, or
/// `Err(TravelError::SessionMismatch(_))` if the persisted envelope is corrupt
/// or inconsistent with `StoredNeighborhood`.
pub fn inspect_active_trip(
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    home_house: u32,
) -> Result<Option<TravelSession>, TravelError> {
    let Some(snapshot) = store.load_lot_snapshot(neighborhood_id, home_house)? else {
        return Err(TravelError::NeighborhoodNotFound(
            neighborhood_id.to_owned(),
        ));
    };
    let Some(bytes) = snapshot.saved_lot.as_deref() else {
        return Ok(None);
    };
    if !is_travel_envelope(bytes) {
        return Ok(None);
    }
    let persisted = decode_travel_envelope(bytes)?;
    verify_persisted_trip_against_snapshot(
        store.store_id(),
        neighborhood_id,
        home_house,
        &snapshot.neighborhood,
        &persisted,
        true,
    )?;
    Ok(Some(persisted.session))
}

/// Resumes a persisted active away-from-home trip for `home_house` (for example
/// after a crash/restart or a halted destination lot), rebooting the
/// destination lot cleanly from the persisted departure snapshots and rotating
/// `trip_id` atomically in REDB so any prior destination handle is invalidated.
pub fn resume_active_trip(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    home_house: u32,
    answers: &[u8],
    frames: Frames,
) -> Result<DepartureResult, TravelError> {
    let install = Install::detect(install_root).map_err(|e| TravelError::Lot(e.to_string()))?;
    let detected_expansions = u16::from(install.expansions.0);

    let home_snapshot = store
        .load_lot_snapshot(neighborhood_id, home_house)?
        .ok_or_else(|| TravelError::NeighborhoodNotFound(neighborhood_id.to_owned()))?;
    let Some(bytes) = home_snapshot.saved_lot.as_deref() else {
        return Err(TravelError::SessionMismatch(format!(
            "house {home_house} has no active away-from-home trip"
        )));
    };
    let mut persisted = decode_travel_envelope(bytes)?;
    verify_persisted_trip_against_snapshot(
        store.store_id(),
        neighborhood_id,
        home_house,
        &home_snapshot.neighborhood,
        &persisted,
        false,
    )?;

    let required_bit = persisted.session.destination.expansion_bit();
    if detected_expansions & required_bit == 0 {
        return Err(TravelError::ExpansionNotInstalled {
            destination: persisted.session.destination,
            required_bit,
            installed: detected_expansions,
        });
    }

    let dest_house = persisted.session.destination_house;
    let dest_snapshot = store
        .load_lot_snapshot(neighborhood_id, dest_house)?
        .ok_or_else(|| TravelError::NeighborhoodNotFound(neighborhood_id.to_owned()))?;
    if dest_snapshot.neighborhood.instance_id != home_snapshot.neighborhood.instance_id {
        return Err(TravelError::NeighborhoodMismatch {
            expected: format!(
                "{neighborhood_id}#{:032x}",
                home_snapshot.neighborhood.instance_id
            ),
            actual: format!(
                "{neighborhood_id}#{:032x}",
                dest_snapshot.neighborhood.instance_id
            ),
        });
    }
    if dest_snapshot.neighborhood.revision != home_snapshot.neighborhood.revision {
        return Err(TravelError::StaleNeighborhood {
            neighborhood: neighborhood_id.to_owned(),
            expected_revision: home_snapshot.neighborhood.revision,
            current_revision: dest_snapshot.neighborhood.revision,
        });
    }
    if home_snapshot.neighborhood.revision == u64::MAX {
        return Err(TravelError::Lot(
            "neighborhood revision counter exhausted before resume".into(),
        ));
    }
    if dest_snapshot.saved_lot.is_none() && dest_snapshot.base_house.is_none() {
        return Err(TravelError::HouseNotFound {
            neighborhood: neighborhood_id.to_owned(),
            house: dest_house,
        });
    }

    let new_trip_id = alloc_trip_id(
        store.store_id(),
        home_snapshot.neighborhood.instance_id,
        home_snapshot.neighborhood.revision,
        home_house,
        dest_house,
        persisted.session.home_family,
    );

    let runtime = StoreRuntime::from_snapshot(
        install_root,
        home_snapshot.neighborhood.clone(),
        &dest_snapshot.character_files,
    )?;
    let (mut dest_lot, traveler_records) = boot_destination_lot(
        runtime,
        store.store_id(),
        new_trip_id,
        neighborhood_id,
        dest_house,
        persisted.session.transport,
        persisted.session.accommodation,
        persisted.session.leader_guid,
        persisted.is_date,
        persisted.view_centre,
        answers,
        frames,
        persisted.session.home_family,
        persisted.session.funds_at_arrival,
        &persisted.departure_snapshots,
        dest_snapshot.saved_lot.as_deref(),
        dest_snapshot.base_house,
    )?;

    persisted.trip_id = new_trip_id;
    persisted.session.trip_id = new_trip_id;
    persisted.session.instance_id = home_snapshot.neighborhood.instance_id;
    persisted.session.travelers = traveler_records;
    persisted.baseline = DepartureBaseline::from_stored(
        &home_snapshot.neighborhood,
        home_snapshot.neighborhood.revision + 1,
        persisted.session.home_family,
    );
    let envelope_bytes = encode_travel_envelope(&persisted)?;

    let (committed_instance, committed_revision) = match store.save_lot_and_neighborhood_checked(
        neighborhood_id,
        home_house,
        Some((
            home_snapshot.neighborhood.instance_id,
            home_snapshot.neighborhood.revision,
        )),
        &envelope_bytes,
        &home_snapshot.neighborhood,
        &[],
    )? {
        Ok(version) => version,
        Err(CheckedCommitError::InstanceMismatch { expected, current }) => {
            return Err(TravelError::NeighborhoodMismatch {
                expected: format!("{neighborhood_id}#{expected:032x}"),
                actual: format!("{neighborhood_id}#{current:032x}"),
            });
        }
        Err(CheckedCommitError::RevisionMismatch { expected, current }) => {
            return Err(TravelError::StaleNeighborhood {
                neighborhood: neighborhood_id.to_owned(),
                expected_revision: expected,
                current_revision: current,
            });
        }
    };

    persisted.session.instance_id = committed_instance;
    dest_lot.neighborhood_origin = None;
    dest_lot.loaded = trip_binding_tag(store.store_id(), new_trip_id);

    Ok(DepartureResult {
        destination_lot: dest_lot,
        session: persisted.session,
        home_revision: committed_revision,
    })
}

/// Settles/cancels an active (or crashed/halted) away-from-home trip directly
/// from the persisted frozen home snapshot in REDB without needing an intact
/// in-memory destination [`Lot`].
///
/// Revokes any temporary check-in hotel keys, clears `secondary_house = 0`,
/// restores the frozen home lot at its exact departure tick and clock, unwraps
/// the `TRVL` envelope back to a normal saved lot, and commits atomically via
/// checked REDB transaction.
pub fn abort_active_trip(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    home_house: u32,
    answers: &[u8],
    frames: Frames,
) -> Result<ReturnResult, TravelError> {
    let home_snapshot = store
        .load_lot_snapshot(neighborhood_id, home_house)?
        .ok_or_else(|| TravelError::NeighborhoodNotFound(neighborhood_id.to_owned()))?;
    let Some(bytes) = home_snapshot.saved_lot.as_deref() else {
        return Err(TravelError::SessionMismatch(format!(
            "house {home_house} has no active away-from-home trip"
        )));
    };
    let persisted = decode_travel_envelope(bytes)?;
    verify_persisted_trip_against_snapshot(
        store.store_id(),
        neighborhood_id,
        home_house,
        &home_snapshot.neighborhood,
        &persisted,
        true,
    )?;

    let auth_session = &persisted.session;
    let expected_home_i32 =
        i32::try_from(auth_session.home_house).map_err(|e| TravelError::Lot(e.to_string()))?;

    let mut updated_stored = home_snapshot.neighborhood.clone();
    let prev_last_lot = updated_stored.data[2];
    updated_stored.data[2] = i16::try_from(auth_session.home_house).unwrap_or(prev_last_lot);
    if let Some(fam) = updated_stored.families.get_mut(&auth_session.home_family) {
        fam.upgrade_to_v9();
        fam.budget = auth_session.funds_at_arrival;
        fam.house = expected_home_i32;
        fam.set_secondary_house(0);
    }
    if matches!(
        auth_session.accommodation,
        VacationAccommodation::HotelCheckIn {
            grant_hotel_key: true
        }
    ) {
        for traveler in &auth_session.travelers {
            if !traveler.had_hotel_key_before_trip {
                clear_stored_inventory_token(
                    &mut updated_stored,
                    traveler.neighbor_id,
                    HOTEL_KEY_TOKEN_KIND,
                    HOTEL_KEY_TOKEN_GUID,
                );
            }
        }
    }

    let home_runtime = StoreRuntime::from_snapshot(
        install_root,
        updated_stored.clone(),
        &home_snapshot.character_files,
    )?;
    let (mut resumed_home, returning_travelers) = reconcile_frozen_home_lot(
        home_runtime,
        store.store_id(),
        auth_session,
        &persisted.frozen_home_bytes,
        auth_session.funds_at_arrival,
        &persisted.departure_snapshots,
        answers,
        frames,
    )?;

    let mut home_saved_after = resumed_home
        .save()
        .map_err(|e| TravelError::Lot(format!("encoding reconciled home lot: {e}")))?;
    home_saved_after.deletion_epoch = updated_stored.deletion_epoch;
    let home_lot_bytes = home_saved_after.encode();

    let (final_instance, final_revision) = match store.save_lot_and_neighborhood_checked(
        neighborhood_id,
        home_house,
        Some((
            home_snapshot.neighborhood.instance_id,
            home_snapshot.neighborhood.revision,
        )),
        &home_lot_bytes,
        &updated_stored,
        &[],
    )? {
        Ok(version) => version,
        Err(CheckedCommitError::InstanceMismatch { expected, current }) => {
            return Err(TravelError::NeighborhoodMismatch {
                expected: format!("{neighborhood_id}#{expected:032x}"),
                actual: format!("{neighborhood_id}#{current:032x}"),
            });
        }
        Err(CheckedCommitError::RevisionMismatch { expected, current }) => {
            return Err(TravelError::StaleNeighborhood {
                neighborhood: neighborhood_id.to_owned(),
                expected_revision: expected,
                current_revision: current,
            });
        }
    };

    resumed_home.neighborhood_origin = Some(NeighborhoodOrigin {
        store_id: store.store_id(),
        neighborhood_id: neighborhood_id.to_owned(),
        instance_id: final_instance,
        revision: final_revision,
    });

    let family_members = updated_stored
        .families
        .get(&auth_session.home_family)
        .map(|f| f.members.clone())
        .unwrap_or_default();
    let magic_coins_at_return = family_magic_coins(&resumed_home.world, &family_members);
    let restored_home_clock = (
        resumed_home
            .world
            .globals
            .get(GLOBAL_HOURS)
            .copied()
            .unwrap_or(0),
        resumed_home
            .world
            .globals
            .get(GLOBAL_MINUTES)
            .copied()
            .unwrap_or(0),
        resumed_home
            .world
            .globals
            .get(GLOBAL_SECONDS)
            .copied()
            .unwrap_or(0),
    );
    let report = ReturnReport {
        neighborhood: auth_session.neighborhood_id.clone(),
        home_house: auth_session.home_house,
        destination_house: auth_session.destination_house,
        home_family: auth_session.home_family,
        destination: auth_session.destination,
        restored_home_ticks: resumed_home.ticks,
        restored_home_world_ticks: resumed_home.world.ticks,
        restored_home_clock,
        funds_at_arrival: auth_session.funds_at_arrival,
        funds_at_return: auth_session.funds_at_arrival,
        funds_delta: 0,
        magic_coins_at_arrival: auth_session.magic_coins_at_arrival,
        magic_coins_at_return,
        magic_coins_delta: magic_coins_at_return.wrapping_sub(auth_session.magic_coins_at_arrival),
        returning_travelers,
        home_revision: final_revision,
    };

    Ok(ReturnResult {
        home_lot: resumed_home,
        report,
    })
}

fn verify_persisted_trip_against_snapshot(
    store_id: u128,
    neighborhood_id: &str,
    home_house: u32,
    stored: &StoredNeighborhood,
    persisted: &PersistedTravelRecord,
    allow_cleared_secondary: bool,
) -> Result<(), TravelError> {
    if persisted.session.store_id != store_id {
        return Err(TravelError::NeighborhoodMismatch {
            expected: format!(
                "{}@store:{:032x}",
                persisted.session.neighborhood_id, persisted.session.store_id
            ),
            actual: format!("{neighborhood_id}@store:{store_id:032x}"),
        });
    }
    if persisted.session.neighborhood_id != neighborhood_id {
        return Err(TravelError::SessionMismatch(format!(
            "persisted trip belongs to neighborhood {}, requested {neighborhood_id}",
            persisted.session.neighborhood_id
        )));
    }
    if persisted.session.home_house != home_house {
        return Err(TravelError::SessionMismatch(format!(
            "persisted trip belongs to home house {}, requested {home_house}",
            persisted.session.home_house
        )));
    }
    if stored.instance_id != persisted.session.instance_id {
        return Err(TravelError::NeighborhoodMismatch {
            expected: format!("{neighborhood_id}#{:032x}", persisted.session.instance_id),
            actual: format!("{neighborhood_id}#{:032x}", stored.instance_id),
        });
    }
    let stored_family = stored
        .families
        .get(&persisted.session.home_family)
        .ok_or_else(|| TravelError::FamilyNotFound {
            neighborhood: neighborhood_id.to_owned(),
            family: persisted.session.home_family,
        })?;
    let expected_home_i32 =
        i32::try_from(home_house).map_err(|e| TravelError::Lot(e.to_string()))?;
    let expected_dest_i32 = i32::try_from(persisted.session.destination_house)
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    let secondary_ok = stored_family.secondary_house() == expected_dest_i32
        || (allow_cleared_secondary && stored_family.secondary_house() == 0);
    if stored_family.house != expected_home_i32 || !secondary_ok {
        return Err(TravelError::SessionMismatch(format!(
            "family {} is at home={}, secondary_house={}, expected home={}, secondary_house={}",
            persisted.session.home_family,
            stored_family.house,
            stored_family.secondary_house(),
            expected_home_i32,
            expected_dest_i32,
        )));
    }
    Ok(())
}

fn clear_stored_inventory_token(
    stored: &mut StoredNeighborhood,
    neighbor_id: i16,
    kind: i32,
    guid: u32,
) {
    for inv in stored.inventories.iter_mut().flatten() {
        if inv.neighbor == neighbor_id {
            inv.items.retain(|it| !(it.kind == kind && it.guid == guid));
        }
    }
}

fn relationship_map(rels: &[sims_formats::neighborhood::Relationship]) -> BTreeMap<i32, Vec<i32>> {
    rels.iter().map(|r| (r.target, r.values.clone())).collect()
}

fn inventory_map(
    invs: &[Option<sims_formats::neighborhood::Inventory>],
) -> BTreeMap<i16, Vec<sims_formats::neighborhood::InventoryItem>> {
    let mut map = BTreeMap::new();
    for inv in invs.iter().flatten() {
        map.insert(inv.neighbor, inv.items.clone());
    }
    map
}

fn tatt_map(tatt: &Option<sims_formats::tatt::Tatt>) -> BTreeMap<u32, Vec<i16>> {
    let mut map = BTreeMap::new();
    if let Some(t) = tatt {
        for e in &t.entries {
            map.insert(e.guid, e.values.clone());
        }
    }
    map
}

fn count_stored_family_friends(stored: &StoredNeighborhood, family_id: u16, threshold: i32) -> i32 {
    let Some(family) = stored.families.get(&family_id) else {
        return 0;
    };
    let mutual = |a: &sims_formats::neighborhood::Neighbor,
                  b: &sims_formats::neighborhood::Neighbor| {
        let ab = a
            .relationships
            .iter()
            .find(|r| r.target == i32::from(b.id))
            .and_then(|r| r.values.first().copied())
            .unwrap_or(0);
        let ba = b
            .relationships
            .iter()
            .find(|r| r.target == i32::from(a.id))
            .and_then(|r| r.values.first().copied())
            .unwrap_or(0);
        ab >= threshold && ba >= threshold
    };
    let mut count = 0i32;
    for candidate in stored.neighbors.iter().flatten() {
        if family.members.contains(&candidate.guid)
            || candidate.person_data.get(61).copied().unwrap_or(0) == 0
            || candidate.person_data.get(PERSON_KIND).copied().unwrap_or(0) > 1
        {
            continue;
        }
        for &guid in &family.members {
            if guid == 0 {
                continue;
            }
            let Some(member) = stored.neighbors.iter().flatten().find(|n| n.guid == guid) else {
                continue;
            };
            if mutual(candidate, member) {
                count = count.wrapping_add(1);
                break;
            }
        }
    }
    count
}

/// Three-way reconciles destination-visit changes (`dest_world`) against the
/// departure `baseline` and the `latest` `StoredNeighborhood`:
///
/// - When `latest.revision == baseline.revision` (no concurrent writes while
///   away), applies the destination write-back directly.
/// - When `latest.revision != baseline.revision` (another writer committed
///   while away), applies ONLY fields that actually changed during the visit:
///   unchanged neighbors, inventories, relationships, and `TATT` entries keep
///   `latest`'s values untouched; disjoint updates merge cleanly; and any true
///   conflict (concurrent edits to the same Sim's `person_data`, same
///   relationship row, same inventory, same `TATT` entry, same `home_family`
///   roster/budget, or same newly allocated neighbor/character slot) returns
///   `Err(TravelError::StaleNeighborhood { .. })` instead of silently
///   clobbering either side.
fn merge_destination_into_latest_neighborhood(
    baseline: &DepartureBaseline,
    latest: &StoredNeighborhood,
    dest_world: &World,
    session: &TravelSession,
    funds_at_return: i32,
    friendship_threshold: i32,
) -> Result<StoredNeighborhood, TravelError> {
    let expected_home_i32 =
        i32::try_from(session.home_house).map_err(|e| TravelError::Lot(e.to_string()))?;

    // Build a clean baseline-seeded scratch copy before running
    // `write_back_world` so `dest_written` reflects PURELY `baseline +
    // destination_visit_deltas` with zero contamination from `latest`.
    let mut dest_written = latest.clone();
    dest_written.neighbors = baseline.neighbors.clone();
    dest_written.inventories = baseline.inventories.clone();
    dest_written.type_attributes = baseline.type_attributes.clone();
    dest_written.made_characters = baseline.made_characters.clone();
    dest_written.write_back_world(
        dest_world,
        session.destination_house,
        session.home_family,
        0,
        None,
        friendship_threshold,
    );

    let conflict = || TravelError::StaleNeighborhood {
        neighborhood: session.neighborhood_id.clone(),
        expected_revision: baseline.revision,
        current_revision: latest.revision,
    };

    let mut merged = latest.clone();
    let prev_last_lot = merged.data[2];
    merged.data[2] = i16::try_from(session.home_house).unwrap_or(prev_last_lot);

    if latest.revision == baseline.revision {
        merged.neighbors = dest_written.neighbors;
        merged.inventories = dest_written.inventories;
        merged.type_attributes = dest_written.type_attributes;
        merged.made_characters = dest_written.made_characters;
        if let Some(fam) = merged.families.get_mut(&session.home_family) {
            fam.upgrade_to_v9();
            fam.budget = funds_at_return;
            fam.house = expected_home_i32;
            fam.set_secondary_house(0);
            if let Some(dest_fam) = dest_written.families.get(&session.home_family) {
                fam.friends = dest_fam.friends;
                if let Ok(Some(magic)) = dest_fam.magic_history() {
                    let _ = fam.set_magic_history(magic);
                }
            }
        }
        return Ok(merged);
    }

    // Concurrent-write path (`latest.revision != baseline.revision`):
    // 1. Verify the away household itself was not concurrently altered.
    let Some(latest_fam) = latest.families.get(&session.home_family) else {
        return Err(TravelError::FamilyNotFound {
            neighborhood: session.neighborhood_id.clone(),
            family: session.home_family,
        });
    };
    if latest_fam.members != baseline.home_family_members
        || latest_fam.budget != baseline.home_family_budget
    {
        return Err(conflict());
    }

    // 2. Three-way merge `made_characters` (reject file/GUID collisions).
    for (&guid, ch) in &dest_written.made_characters.made {
        if baseline.made_characters.made.contains_key(&guid) {
            continue;
        }
        if merged.deleted_characters.contains(&guid)
            || merged.made_characters.made.contains_key(&guid)
            || merged
                .made_characters
                .made
                .values()
                .any(|other| other.file == ch.file)
        {
            return Err(conflict());
        }
        merged.made_characters.made.insert(guid, ch.clone());
    }
    if dest_written.made_characters.next_file > merged.made_characters.next_file {
        merged.made_characters.next_file = dest_written.made_characters.next_file;
    }

    // 3. Three-way merge `NBRS` slots.
    let max_slots = baseline
        .neighbors
        .len()
        .max(dest_written.neighbors.len())
        .max(merged.neighbors.len());
    for idx in 0..max_slots {
        let b_slot = baseline.neighbors.get(idx).and_then(|s| s.as_ref());
        let d_slot = dest_written.neighbors.get(idx).and_then(|s| s.as_ref());
        let l_slot = merged.neighbors.get(idx).and_then(|s| s.as_ref());

        // Unchanged during destination visit -> preserve `latest` verbatim.
        if d_slot == b_slot {
            continue;
        }

        match (b_slot, d_slot, l_slot) {
            (None, Some(d_nbr), None) => {
                if merged.deleted_characters.contains(&d_nbr.guid) {
                    return Err(conflict());
                }
                if merged.neighbors.len() <= idx {
                    merged.neighbors.resize(idx + 1, None);
                }
                merged.neighbors[idx] = Some(d_nbr.clone());
            }
            (None, Some(_), Some(_)) => {
                // Both sides allocated the same free neighbor slot concurrently.
                return Err(conflict());
            }
            (Some(b_nbr), Some(d_nbr), Some(l_nbr)) => {
                if d_nbr.guid != b_nbr.guid
                    || l_nbr.guid != b_nbr.guid
                    || l_nbr.id != b_nbr.id
                    || d_nbr.id != b_nbr.id
                {
                    return Err(conflict());
                }
                if l_nbr == b_nbr {
                    merged.neighbors[idx] = Some(d_nbr.clone());
                    continue;
                }
                // Both sides modified this neighbor: allow only disjoint
                // core/relationship updates, rejecting overlapping edits.
                let dest_core_changed = d_nbr.person_mode != b_nbr.person_mode
                    || d_nbr.person_data != b_nbr.person_data
                    || d_nbr.name != b_nbr.name;
                let latest_core_changed = l_nbr.person_mode != b_nbr.person_mode
                    || l_nbr.person_data != b_nbr.person_data
                    || l_nbr.name != b_nbr.name;
                if dest_core_changed && latest_core_changed {
                    return Err(conflict());
                }

                let b_rels = relationship_map(&b_nbr.relationships);
                let d_rels = relationship_map(&d_nbr.relationships);
                let l_rels = relationship_map(&l_nbr.relationships);
                let targets: BTreeSet<i32> = b_rels
                    .keys()
                    .chain(d_rels.keys())
                    .chain(l_rels.keys())
                    .copied()
                    .collect();
                let mut merged_rels = Vec::new();
                for target in targets {
                    let bv = b_rels.get(&target);
                    let dv = d_rels.get(&target);
                    let lv = l_rels.get(&target);
                    let d_changed = dv != bv;
                    let l_changed = lv != bv;
                    let chosen = match (d_changed, l_changed) {
                        (false, _) => lv,
                        (true, false) => dv,
                        (true, true) if dv == lv => dv,
                        (true, true) => return Err(conflict()),
                    };
                    if let Some(values) = chosen {
                        merged_rels.push(sims_formats::neighborhood::Relationship {
                            target,
                            values: values.clone(),
                        });
                    }
                }
                let mut combined = if dest_core_changed {
                    d_nbr.clone()
                } else {
                    l_nbr.clone()
                };
                combined.relationships = merged_rels;
                merged.neighbors[idx] = Some(combined);
            }
            (Some(_), Some(_), None) => {
                // Neighbor modified on destination was concurrently deleted in `latest`.
                return Err(conflict());
            }
            (Some(b_nbr), None, Some(l_nbr)) => {
                if l_nbr != b_nbr {
                    return Err(conflict());
                }
                merged.neighbors[idx] = None;
            }
            (Some(_), None, None) | (None, None, _) => {}
        }
    }

    // 4. Three-way merge `inventories` (`NGBH`).
    let base_invs = inventory_map(&baseline.inventories);
    let dest_invs = inventory_map(&dest_written.inventories);
    let latest_invs = inventory_map(&merged.inventories);
    let all_nids: BTreeSet<i16> = base_invs.keys().chain(dest_invs.keys()).copied().collect();
    for nid in all_nids {
        let bv = base_invs.get(&nid);
        let dv = dest_invs.get(&nid);
        let lv = latest_invs.get(&nid);
        if dv == bv {
            continue;
        }
        if lv != bv && lv != dv {
            return Err(conflict());
        }
        merged
            .inventories
            .retain(|entry| entry.as_ref().is_none_or(|inv| inv.neighbor != nid));
        if let Some(items) = dv {
            merged
                .inventories
                .push(Some(sims_formats::neighborhood::Inventory {
                    neighbor: nid,
                    items: items.clone(),
                }));
        }
    }

    // 5. Three-way merge `type_attributes` (`TATT`).
    let base_tatt = tatt_map(&baseline.type_attributes);
    let dest_tatt = tatt_map(&dest_written.type_attributes);
    let latest_tatt = tatt_map(&merged.type_attributes);
    let all_guids: BTreeSet<u32> = base_tatt.keys().chain(dest_tatt.keys()).copied().collect();
    for guid in all_guids {
        let bv = base_tatt.get(&guid);
        let dv = dest_tatt.get(&guid);
        let lv = latest_tatt.get(&guid);
        if dv == bv {
            continue;
        }
        if lv != bv && lv != dv {
            return Err(conflict());
        }
        let target_tatt = merged
            .type_attributes
            .get_or_insert_with(|| sims_formats::tatt::Tatt {
                version: dest_written
                    .type_attributes
                    .as_ref()
                    .map_or(0, |t| t.version),
                packed: dest_written
                    .type_attributes
                    .as_ref()
                    .is_none_or(|t| t.packed),
                entries: Vec::new(),
                count_end: 0,
                consumed: 0,
            });
        match dv {
            Some(values) => {
                if let Some(existing) = target_tatt.entries.iter_mut().find(|e| e.guid == guid) {
                    existing.values = values.clone();
                } else {
                    target_tatt.entries.push(sims_formats::tatt::Entry {
                        guid,
                        values: values.clone(),
                        count_end: 0,
                        end: 0,
                    });
                }
            }
            None => {
                target_tatt.entries.retain(|e| e.guid != guid);
            }
        }
    }

    // 6. Update ONLY `session.home_family`, preserving all other families and
    //    recomputing `friends` against the 3-way merged neighborhood.
    let merged_friends =
        count_stored_family_friends(&merged, session.home_family, friendship_threshold);
    if let Some(fam) = merged.families.get_mut(&session.home_family) {
        fam.upgrade_to_v9();
        fam.budget = funds_at_return;
        fam.house = expected_home_i32;
        fam.set_secondary_house(0);
        fam.friends = merged_friends;
        if let Some(dest_fam) = dest_written.families.get(&session.home_family)
            && let Ok(Some(magic)) = dest_fam.magic_history()
        {
            let _ = fam.set_magic_history(magic);
        }
    }

    Ok(merged)
}

/// Returns the traveling party from `destination_lot` back to the frozen home
/// lot recorded in `session`:
///
/// 1. Flushes every live Sim on `destination_lot` into `NBRS` and extracts
///    returning travelers' updated motives, skills, career/fame, outfits,
///    relationships (including newly met townies/vacationers/celebrities/magic
///    NPCs), household funds, MagiCoins, and inventory tokens.
/// 2. Revokes any temporary Vacation `Token - Hotel Key` issued at check-in
///    while preserving purchased souvenirs, gifts, outfits, magic ingredients,
///    and MagiCoins.
/// 3. Preserves any Build/Buy community-lot edits stored in REDB untouched
///    (matching `Sims.exe` 1.91, where live visits never overwrite the
///    destination lot).
/// 4. Restores and reconciles the frozen home lot at its exact departure tick,
///    placing returning household travelers cleanly at the home curb/portal
///    with updated state and zero duplicate or orphaned objects, and commits
///    the reconciled home lot and updated `StoredNeighborhood` in a single
///    atomic checked REDB transaction.
pub fn return_from_destination(
    install_root: &Path,
    store: &NeighborhoodStore,
    session: &TravelSession,
    destination_lot: &mut Lot,
    answers: &[u8],
    frames: Frames,
) -> Result<ReturnResult, TravelError> {
    if destination_lot.load.finished.is_none() {
        return Err(TravelError::Lot(
            "destination lot load did not complete".into(),
        ));
    }
    if destination_lot.halted {
        return Err(TravelError::Lot("destination lot is halted".into()));
    }
    if destination_lot.asking.is_some()
        || destination_lot.asking_special_panel.is_some()
        || destination_lot.answered
    {
        return Err(TravelError::Lot(
            "destination lot is waiting on an unanswered modal dialog or special panel".into(),
        ));
    }
    if session.store_id != store.store_id() {
        return Err(TravelError::NeighborhoodMismatch {
            expected: format!(
                "{}@store:{:032x}",
                session.neighborhood_id, session.store_id
            ),
            actual: format!(
                "{}@store:{:032x}",
                session.neighborhood_id,
                store.store_id()
            ),
        });
    }
    if session.trip_id == 0 {
        return Err(TravelError::SessionMismatch(
            "travel session is missing a valid trip_id".into(),
        ));
    }
    if destination_lot.load.house != session.destination_house {
        return Err(TravelError::SessionMismatch(format!(
            "destination lot is house {}, session expects house {}",
            destination_lot.load.house, session.destination_house
        )));
    }
    if destination_lot.load.family != session.home_family {
        return Err(TravelError::SessionMismatch(format!(
            "destination lot active family is {}, session expects family {}",
            destination_lot.load.family, session.home_family
        )));
    }
    if destination_lot.loaded == [0u8; 32] {
        return Err(TravelError::Lot(
            "destination lot was already returned or is not bound to an active trip".into(),
        ));
    }
    let expected_tag = trip_binding_tag(store.store_id(), session.trip_id);
    if destination_lot.loaded != expected_tag {
        return Err(TravelError::SessionMismatch(format!(
            "destination lot handle is bound to a different trip than session trip {:032x}",
            session.trip_id
        )));
    }

    let home_snapshot = store
        .load_lot_snapshot(&session.neighborhood_id, session.home_house)?
        .ok_or_else(|| TravelError::NeighborhoodNotFound(session.neighborhood_id.clone()))?;
    if home_snapshot.neighborhood.instance_id != session.instance_id {
        return Err(TravelError::NeighborhoodMismatch {
            expected: format!("{}#{:032x}", session.neighborhood_id, session.instance_id),
            actual: format!(
                "{}#{:032x}",
                session.neighborhood_id, home_snapshot.neighborhood.instance_id
            ),
        });
    }
    let Some(envelope_bytes) = home_snapshot.saved_lot.as_deref() else {
        return Err(TravelError::SessionMismatch(format!(
            "active trip envelope for home lot {} is missing from REDB",
            session.home_house
        )));
    };
    let persisted = decode_travel_envelope(envelope_bytes)?;
    verify_persisted_trip_against_snapshot(
        store.store_id(),
        &session.neighborhood_id,
        session.home_house,
        &home_snapshot.neighborhood,
        &persisted,
        false,
    )?;
    if persisted.trip_id != session.trip_id
        || persisted.session.home_family != session.home_family
        || persisted.session.destination_house != session.destination_house
        || persisted.session.destination != session.destination
    {
        return Err(TravelError::SessionMismatch(format!(
            "stale or mismatched travel session (trip {:032x} vs active trip {:032x})",
            session.trip_id, persisted.trip_id
        )));
    }

    // Use the REDB-persisted authoritative session & frozen home bytes so
    // caller-mutated or retained session fields cannot supply wrong rosters,
    // hotel-key rules, or home clocks.
    let auth_session = &persisted.session;
    let frozen_home_bytes = &persisted.frozen_home_bytes;

    // Clone the destination lot's saved snapshot so we can flush persons,
    // capture returning state, and strip visiting travelers without leaving
    // `destination_lot` half-mutated on error.
    let mut dest_saved = destination_lot
        .save()
        .map_err(|e| TravelError::Lot(format!("snapshotting destination lot: {e}")))?;

    // Resolve each traveler's authoritative live object ID FIRST (preferring
    // the recorded `destination_object_id`) so any duplicate instance sharing
    // the same GUID or neighbor ID cannot overwrite the authoritative traveler
    // when flushing live persons to `NBRS`.
    let mut preferred_traveler_ids: Vec<(usize, u16)> =
        Vec::with_capacity(auth_session.travelers.len());
    let mut preferred_id_set: BTreeSet<u16> = BTreeSet::new();
    let mut preferred_guids: BTreeSet<u32> = BTreeSet::new();
    let mut preferred_nids: BTreeSet<i16> = BTreeSet::new();
    for (idx, traveler) in auth_session.travelers.iter().enumerate() {
        let live_id = dest_saved
            .world
            .objects
            .get(&traveler.destination_object_id)
            .is_some_and(|o| o.guid == traveler.guid && o.person.is_some())
            .then_some(traveler.destination_object_id)
            .or_else(|| {
                dest_saved.world.people.iter().copied().find(|&id| {
                    dest_saved
                        .world
                        .objects
                        .get(&id)
                        .is_some_and(|o| o.guid == traveler.guid && o.person.is_some())
                })
            });
        if let Some(id) = live_id {
            preferred_traveler_ids.push((idx, id));
            preferred_id_set.insert(id);
            preferred_guids.insert(traveler.guid);
            preferred_nids.insert(traveler.neighbor_id);
        }
    }

    // 1. Flush non-duplicate live persons first, then authoritative travelers
    //    so preferred `destination_object_id`s always win in `NBRS`.
    for person_id in dest_saved.world.people.clone() {
        if preferred_id_set.contains(&person_id) {
            continue;
        }
        let Some(obj) = dest_saved.world.objects.get(&person_id) else {
            continue;
        };
        let Some(person) = obj.person.as_ref() else {
            continue;
        };
        let nid = person.data.get(PERSON_NEIGHBOR).copied().unwrap_or(0);
        if preferred_guids.contains(&obj.guid) || (nid > 0 && preferred_nids.contains(&nid)) {
            continue;
        }
        people::save_to_record(&mut dest_saved.world, person_id)
            .map_err(|e| TravelError::Lot(format!("saving destination person {person_id}: {e}")))?;
    }
    for &(_, person_id) in &preferred_traveler_ids {
        people::save_to_record(&mut dest_saved.world, person_id).map_err(|e| {
            TravelError::Lot(format!("saving destination traveler {person_id}: {e}"))
        })?;
    }

    // 2. Revoke temporary Vacation check-in hotel keys granted on arrival
    //    (preserving any pre-existing key and all souvenirs/gifts/MagiCoins).
    if matches!(
        auth_session.accommodation,
        VacationAccommodation::HotelCheckIn {
            grant_hotel_key: true
        }
    ) {
        for traveler in &auth_session.travelers {
            if !traveler.had_hotel_key_before_trip {
                set_neighbor_token_count(
                    &mut dest_saved.world,
                    traveler.neighbor_id,
                    HOTEL_KEY_TOKEN_KIND,
                    HOTEL_KEY_TOKEN_GUID,
                    0,
                );
            }
        }
    }

    // 3. Capture returning state for every traveler on the destination lot
    //    using the exact same resolved live IDs as step 1.
    let resolved_by_index: BTreeMap<usize, u16> = preferred_traveler_ids.into_iter().collect();
    let mut returning_states: Vec<LiveTravelerSnapshot> =
        Vec::with_capacity(auth_session.travelers.len());
    for (idx, traveler) in auth_session.travelers.iter().enumerate() {
        let live_id = resolved_by_index.get(&idx).copied();
        let made_outfits = dest_saved.world.made_outfits.get(&traveler.guid).cloned();
        if let Some(id) = live_id
            && let Some(obj) = dest_saved.world.objects.get(&id)
            && let Some(person) = obj.person.as_ref()
        {
            returning_states.push(LiveTravelerSnapshot {
                guid: traveler.guid,
                neighbor_id: traveler.neighbor_id,
                is_household_member: traveler.is_household_member,
                home_object_id: traveler.home_object_id,
                motives: person.motives,
                previous_motives: person.previous_motives,
                person_data: person.data.clone(),
                accessories: person.accessories.clone(),
                outfit: dest_saved.world.person_outfits.get(&id).cloned(),
                made_outfits,
                had_hotel_key_before_trip: traveler.had_hotel_key_before_trip,
            });
        } else if let Some(record) = dest_saved.world.neighbors.first_of(traveler.guid) {
            returning_states.push(LiveTravelerSnapshot {
                guid: traveler.guid,
                neighbor_id: record.id,
                is_household_member: traveler.is_household_member,
                home_object_id: traveler.home_object_id,
                motives: [50.0; 16],
                previous_motives: [50.0; 16],
                person_data: record.data.clone(),
                accessories: Vec::new(),
                outfit: None,
                made_outfits,
                had_hotel_key_before_trip: traveler.had_hotel_key_before_trip,
            });
        }
    }

    let funds_at_return = dest_saved.world.budget.funds;
    let family_members = home_snapshot
        .neighborhood
        .families
        .get(&auth_session.home_family)
        .map(|f| f.members.clone())
        .ok_or_else(|| TravelError::FamilyNotFound {
            neighborhood: auth_session.neighborhood_id.clone(),
            family: auth_session.home_family,
        })?;
    let magic_coins_at_return = family_magic_coins(&dest_saved.world, &family_members);

    // 4. Three-way reconcile destination neighborhood-wide updates against
    //    `persisted.baseline` onto `home_snapshot.neighborhood`, preserving
    //    unrelated edits made while away and rejecting true conflicts.
    let tuning = NeighborhoodTuning::from_program(&destination_lot.program);

    // Mark the returning household's friend count stale so `write_back_world`
    // always recomputes `FAMI.friends` from any new destination friendships.
    let home_family_i32 = i32::from(auth_session.home_family);
    for live_family in &mut dest_saved.world.families {
        if live_family.id == home_family_i32 {
            live_family.stale = true;
        }
    }

    let updated_stored = merge_destination_into_latest_neighborhood(
        &persisted.baseline,
        &home_snapshot.neighborhood,
        &dest_saved.world,
        auth_session,
        funds_at_return,
        tuning.friendship_threshold,
    )?;

    // 5. Reconcile & resume the frozen home lot in memory BEFORE committing to
    //    REDB, leaving any Build/Buy community-lot snapshot in REDB untouched
    //    (matching `Sims.exe` 1.91: live away-from-home visits never overwrite
    //    the destination lot with transient visit NPCs/mess).
    let home_runtime = StoreRuntime::from_snapshot(
        install_root,
        updated_stored.clone(),
        &home_snapshot.character_files,
    )?;
    let (mut resumed_home, returning_travelers) = reconcile_frozen_home_lot(
        home_runtime,
        store.store_id(),
        auth_session,
        frozen_home_bytes,
        funds_at_return,
        &returning_states,
        answers,
        frames,
    )?;

    // Snapshot reconciled home lot so REDB holds the post-return home state too.
    let mut home_saved_after = resumed_home
        .save()
        .map_err(|e| TravelError::Lot(format!("encoding reconciled home lot: {e}")))?;
    home_saved_after.deletion_epoch = updated_stored.deletion_epoch;
    let home_lot_bytes = home_saved_after.encode();

    // 6. Commit the reconciled home lot + updated neighborhood in a SINGLE
    //    atomic checked REDB transaction fenced on the latest home snapshot
    //    revision (so unrelated writes before return succeed, while any
    //    concurrent write racing with this return fails cleanly via CAS).
    let (final_instance, final_revision) = match store.save_lot_and_neighborhood_checked(
        &auth_session.neighborhood_id,
        auth_session.home_house,
        Some((
            home_snapshot.neighborhood.instance_id,
            home_snapshot.neighborhood.revision,
        )),
        &home_lot_bytes,
        &updated_stored,
        &[],
    )? {
        Ok(version) => version,
        Err(CheckedCommitError::InstanceMismatch { expected, current }) => {
            return Err(TravelError::NeighborhoodMismatch {
                expected: format!("{}#{expected:032x}", auth_session.neighborhood_id),
                actual: format!("{}#{current:032x}", auth_session.neighborhood_id),
            });
        }
        Err(CheckedCommitError::RevisionMismatch { expected, current }) => {
            return Err(TravelError::StaleNeighborhood {
                neighborhood: auth_session.neighborhood_id.clone(),
                expected_revision: expected,
                current_revision: current,
            });
        }
    };

    // Clear the destination lot's trip binding so it cannot be returned twice,
    // and bind `resumed_home` to the final committed revision.
    destination_lot.neighborhood_origin = None;
    destination_lot.loaded = [0u8; 32];
    resumed_home.neighborhood_origin = Some(NeighborhoodOrigin {
        store_id: store.store_id(),
        neighborhood_id: auth_session.neighborhood_id.clone(),
        instance_id: final_instance,
        revision: final_revision,
    });

    let restored_home_clock = (
        resumed_home
            .world
            .globals
            .get(GLOBAL_HOURS)
            .copied()
            .unwrap_or(0),
        resumed_home
            .world
            .globals
            .get(GLOBAL_MINUTES)
            .copied()
            .unwrap_or(0),
        resumed_home
            .world
            .globals
            .get(GLOBAL_SECONDS)
            .copied()
            .unwrap_or(0),
    );
    let report = ReturnReport {
        neighborhood: auth_session.neighborhood_id.clone(),
        home_house: auth_session.home_house,
        destination_house: auth_session.destination_house,
        home_family: auth_session.home_family,
        destination: auth_session.destination,
        restored_home_ticks: resumed_home.ticks,
        restored_home_world_ticks: resumed_home.world.ticks,
        restored_home_clock,
        funds_at_arrival: auth_session.funds_at_arrival,
        funds_at_return,
        funds_delta: funds_at_return.wrapping_sub(auth_session.funds_at_arrival),
        magic_coins_at_arrival: auth_session.magic_coins_at_arrival,
        magic_coins_at_return,
        magic_coins_delta: magic_coins_at_return.wrapping_sub(auth_session.magic_coins_at_arrival),
        returning_travelers,
        home_revision: final_revision,
    };

    Ok(ReturnResult {
        home_lot: resumed_home,
        report,
    })
}

/// Opens an unoccupied travel destination lot from REDB in Build/Buy edit mode
/// (`family = 0`, `globals[32] = 1` unlimited budget, paused, zero visiting
/// travelers). Edits made to the returned [`Lot`] (Buy/Build/lot info) can be
/// persisted via [`crate::lifecycle::save_and_leave_lot`] and are then loaded cleanly on
/// every subsequent [`depart_to_destination`] visit.
pub fn open_destination_for_edit(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    destination_house: u32,
    answers: &[u8],
    frames: Frames,
) -> Result<Lot, TravelError> {
    let dest_snapshot = store
        .load_lot_snapshot(neighborhood_id, destination_house)?
        .ok_or_else(|| TravelError::NeighborhoodNotFound(neighborhood_id.to_owned()))?;
    let zoning = i16::try_from(destination_house)
        .ok()
        .and_then(|lot| dest_snapshot.neighborhood.zoning.get(&lot).copied());
    let Some(destination) =
        TravelDestination::classify(destination_house, &dest_snapshot.neighborhood.zoning)
    else {
        return Err(TravelError::NotTravelDestination {
            house: destination_house,
            zoning,
        });
    };
    if let Some((occupant, _)) = dest_snapshot.neighborhood.family_in(destination_house) {
        return Err(TravelError::DestinationOccupied {
            house: destination_house,
            family: occupant,
        });
    }
    if dest_snapshot.saved_lot.is_none() && dest_snapshot.base_house.is_none() {
        return Err(TravelError::HouseNotFound {
            neighborhood: neighborhood_id.to_owned(),
            house: destination_house,
        });
    }
    let install = Install::detect(install_root).map_err(|e| TravelError::Lot(e.to_string()))?;
    let detected_expansions = u16::from(install.expansions.0);
    let required_bit = destination.expansion_bit();
    if detected_expansions & required_bit == 0 {
        return Err(TravelError::ExpansionNotInstalled {
            destination,
            required_bit,
            installed: detected_expansions,
        });
    }
    let runtime = StoreRuntime::from_snapshot(
        install_root,
        dest_snapshot.neighborhood,
        &dest_snapshot.character_files,
    )?;
    boot_destination_for_edit(
        runtime,
        store.store_id(),
        neighborhood_id,
        destination_house,
        answers,
        frames,
        dest_snapshot.saved_lot.as_deref(),
        dest_snapshot.base_house,
    )
}

/// Boots a destination lot in Build/Buy edit mode (`family = 0`, `globals[32] = 1`).
#[allow(clippy::too_many_arguments)]
fn boot_destination_for_edit(
    runtime: StoreRuntime,
    store_id: u128,
    neighborhood_id: &str,
    dest_house: u32,
    answers: &[u8],
    frames: Frames,
    saved_lot: Option<&[u8]>,
    base_house: Option<Vec<u8>>,
) -> Result<Lot, TravelError> {
    let StoreRuntime {
        install,
        index,
        catalog,
        program,
        surfaces,
        stored,
        ..
    } = runtime;
    let origin_instance_id = stored.instance_id;
    let origin_revision = stored.revision;
    let (
        mut world,
        mut load,
        view_rotation,
        floor_rotation,
        view_centre,
        start_ticks,
        saved_ticks,
        lot_name,
        lot_description,
    ) = load_destination_world(
        &install,
        &index,
        &catalog,
        &program,
        &surfaces,
        &stored,
        neighborhood_id,
        dest_house,
        0,
        0,
        None,
        &[],
        None,
        saved_lot,
        base_house,
    )?;
    if let Some(slot) = world.globals.get_mut(GLOBAL_BUDGET_MODE) {
        *slot = 1;
    }
    if let Some(slot) = world.globals.get_mut(GLOBAL_PAUSED) {
        *slot = 1;
    }
    if let Some(slot) = world.globals.get_mut(GLOBAL_MODE) {
        *slot = 1;
    }
    let interpreter = Interpreter {
        view_rotation,
        view_centre,
        ..Interpreter::new(&program, &catalog, Stream::Global)
    };
    let _ = interpreter
        .announce_house(&mut world)
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    let counted = interpreter
        .before_frame(&mut world)
        .map_err(|e| TravelError::Lot(e.to_string()))?
        .len();
    load.finished = Some(Finished {
        indicators_made: false,
        sim: None,
        counted,
    });
    let loaded = world.hash();
    let fast = crate::program::ProgramFast::build(&program, &catalog);
    Ok(Lot {
        program,
        fast,
        catalog: Arc::new(catalog),
        surfaces,
        world,
        tuning: None,
        player: Answers::choices(answers),
        frames,
        ticks: saved_ticks,
        start: start_ticks,
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
        floor_rotation,
        view_centre,
        replay_view: None,
        last_dialog_text: String::new(),
        character_names: BTreeMap::new(),
        default_character_name: crate::boot::read_default_character_name(&install),
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
        neighborhood_origin: Some(NeighborhoodOrigin {
            store_id,
            neighborhood_id: neighborhood_id.to_owned(),
            instance_id: origin_instance_id,
            revision: origin_revision,
        }),
        coop: None,
        move_objects: false,
    })
}

/// Shared destination world loader used by both [`boot_destination_lot`] (live
/// visit) and [`boot_destination_for_edit`] (Build/Buy edit mode).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn load_destination_world(
    install: &Install,
    index: &Index,
    catalog: &Catalog,
    program: &Program,
    surfaces: &Surfaces,
    stored: &StoredNeighborhood,
    neighborhood_id: &str,
    dest_house: u32,
    family_id: u16,
    arrival_funds: i32,
    family_magic: Option<[i16; 6]>,
    family_members: &[u32],
    requested_view_centre: Option<[i8; 2]>,
    saved_lot: Option<&[u8]>,
    base_house: Option<Vec<u8>>,
) -> Result<
    (
        World,
        LoadReport,
        u8,
        u8,
        Option<[i8; 2]>,
        u32,
        u32,
        String,
        String,
    ),
    TravelError,
> {
    if let Some(saved_bytes) = saved_lot {
        let saved = Saved::decode(saved_bytes).map_err(|e| {
            TravelError::Lot(format!(
                "decoding saved destination house {dest_house}: {e}"
            ))
        })?;
        if saved.house != dest_house {
            return Err(TravelError::Lot(format!(
                "saved destination lot is house {}, requested {dest_house}",
                saved.house
            )));
        }
        let mut world = saved.world;
        let content_nh = stored.to_content_neighborhood();
        let reg = neighborhood_registry(
            index,
            catalog,
            &stored.made_characters,
            &stored.deleted_characters,
        );
        world
            .read_neighbors(&content_nh, &reg)
            .map_err(|e| TravelError::Lot(e.to_string()))?;
        world.type_attributes = program
            .type_attributes(catalog)
            .map_err(|e| TravelError::Lot(e.to_string()))?;
        if let Some(saved_tatt) = &stored.type_attributes {
            world.read_type_attributes(saved_tatt);
        }
        world.characters = stored.made_characters.clone();

        let view_centre = requested_view_centre.or(saved.view_centre);
        let active = (!family_members.is_empty()).then_some(family_members);
        let interpreter = Interpreter {
            view_rotation: saved.view_rotation,
            view_centre,
            active_family: active,
            ..Interpreter::new(program, catalog, Stream::Global)
        };
        if let Some(slot) = world.globals.get_mut(GLOBAL_BUDGET_MODE) {
            *slot = 0;
        }
        world
            .select_family(
                i16::try_from(family_id).map_err(|e| TravelError::Lot(e.to_string()))?,
                i16::try_from(dest_house).map_err(|e| TravelError::Lot(e.to_string()))?,
                arrival_funds,
                family_magic,
            )
            .map_err(|e| TravelError::Lot(e.to_string()))?;
        ensure_today_budget_slot(&mut world);
        // Strip orphaned/deleted saved persons before re-attaching remaining
        // persons and indicators. A `Saved` destination lot already completed
        // controller reconciliation and `visit_objects` on its initial base
        // load (just like `lifecycle::boot_from_saved_lot`), so we preserve
        // its exact furnishings and controllers without re-running
        // `reconcile_controllers` or `Interpreter::clear`.
        let lot_epoch = saved.deletion_epoch;
        let orphaned: Vec<u16> = world
            .people
            .iter()
            .copied()
            .filter(|&pid| {
                world.objects.get(&pid).is_some_and(|o| {
                    o.person.is_some()
                        && (stored.deleted_characters.contains(&o.guid)
                            || world.neighbors.first_of(o.guid).is_none()
                            || stored
                                .deleted_at_epoch
                                .get(&o.guid)
                                .is_some_and(|&del| lot_epoch < del))
                })
            })
            .collect();
        let mut people_to_reset = BTreeSet::new();
        let mut objects_to_reset = BTreeSet::new();
        for pid in orphaned {
            let (p_reset, o_reset) =
                strip_person_for_travel_deferred(&interpreter, &mut world, pid)?;
            people_to_reset.extend(p_reset);
            objects_to_reset.extend(o_reset);
        }
        for pid in world.people.clone() {
            if world.objects.get(&pid).is_some_and(|o| o.person.is_some()) {
                interpreter
                    .attach_neighbor(&mut world, pid)
                    .map_err(|e| TravelError::Lot(format!("attaching person {pid}: {e}")))?;
            }
        }
        apply_post_purge_resets(&interpreter, &mut world, people_to_reset, objects_to_reset)?;
        let _ = interpreter
            .ensure_indicators(&mut world)
            .map_err(|e| TravelError::Lot(e.to_string()))?;
        let mut load = LoadReport {
            family: family_id,
            house: dest_house,
            ..LoadReport::default()
        };
        load.libraries = program.libraries.len();
        return Ok((
            world,
            load,
            saved.view_rotation,
            saved.floor_rotation,
            view_centre,
            saved.start,
            saved.ticks,
            saved.lot_name,
            saved.lot_description,
        ));
    }

    let bytes = base_house.ok_or_else(|| TravelError::HouseNotFound {
        neighborhood: neighborhood_id.to_owned(),
        house: dest_house,
    })?;
    let virtual_path = PathBuf::from(format!("{neighborhood_id}/Houses/House{dest_house:02}.iff"));
    let stripped_guids: BTreeSet<u32> = stored
        .deleted_characters
        .iter()
        .copied()
        .chain(stored.deleted_at_epoch.keys().copied())
        .collect();
    let mut file = House::parse_bytes_with_deleted(bytes, &virtual_path, catalog, &stripped_guids)
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    let view_rotation = opening_rotation(&file);
    let context = Context {
        expansions: program.expansions,
        ..Context::COMPLETE_COLLECTION
    };
    let mut world = World::empty();
    world.crt_rng = CrtRng::from_state(1);
    let mut load = LoadReport::default();

    let active = (!family_members.is_empty()).then_some(family_members);
    let interpreter = Interpreter {
        view_rotation,
        view_centre: requested_view_centre,
        active_family: active,
        ..Interpreter::new(program, catalog, Stream::Global)
    };
    world
        .skeleton_files
        .extend(crate::objects::read_before_the_first_load(&program.people));
    let content_nh = stored.to_content_neighborhood();
    let reg = neighborhood_registry(
        index,
        catalog,
        &stored.made_characters,
        &stored.deleted_characters,
    );
    world
        .read_neighbors(&content_nh, &reg)
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    world
        .drop_empty_families()
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    world.type_attributes = program
        .type_attributes(catalog)
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    if let Some(saved_tatt) = &stored.type_attributes {
        world.read_type_attributes(saved_tatt);
    }
    world.characters = stored.made_characters.clone();
    interpreter
        .create_indicators(&mut world)
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    world.read_simulator(&file.simulator);
    world.read_house_model(
        file.model
            .as_ref()
            .ok_or_else(|| TravelError::Lot("destination house has no HOUS 1 chunk".into()))?,
    );
    world
        .apply_context(context)
        .map_err(|e| TravelError::Lot(e.to_string()))?;

    // Step 5 (`0x415e20..0x415e67`, matching `boot.rs:952-989`): populate
    // bare-lot arrays before `reconcile_controllers` on shrunken lots
    // (needed by Vacation lots 40..=48 whose controllers run `Find Location For`
    // during init).
    let requested = match world.globals.get(GLOBAL_LOT_SIZE).copied().unwrap_or(0) {
        0 => FULL_SIZE,
        size => size,
    };
    if requested >= 4 && requested < world.size {
        let full = usize::from(FULL_SIZE as u16);
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
        world
            .rebuild_rooms()
            .map_err(|e| TravelError::Lot(e.to_string()))?;
        world.globals[GLOBAL_INDOOR_AREA] = 0;
        load.created = interpreter
            .reconcile_controllers(&mut world)
            .map_err(|e| TravelError::Lot(e.to_string()))?
            .created
            .len();
    } else {
        load.created = interpreter
            .resize(&mut world)
            .map_err(|e| TravelError::Lot(e.to_string()))?
            .created
            .len();
    }
    world
        .clear_family()
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    load.queued = world.removals.len();
    load.removed = interpreter
        .clear(&mut world)
        .map_err(|e| TravelError::Lot(e.to_string()))?
        .len();
    world.arrays.clear();

    let version = file
        .version()
        .ok_or_else(|| TravelError::Lot("destination house has no HOUS 1 version".into()))?;
    world
        .read_arrays(&file.arrays, version)
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    load.renumbered = world
        .renumber_surfaces(
            file.floor_map.as_deref(),
            file.wall_map.as_deref(),
            surfaces,
        )
        .map_err(|e| TravelError::Lot(e.to_string()))?;

    for entry in &file.types.entries {
        if catalog.object(entry.guid).is_none() && !stripped_guids.contains(&entry.guid) {
            return Err(TravelError::Lot(format!(
                "destination house {dest_house} type {} (0x{:08X}) has no catalog definition",
                entry.id, entry.guid
            )));
        }
    }
    load.types = file.types.entries.len();
    load.changed = interpreter
        .read_types(&mut world, &file.types)
        .map_err(|e| TravelError::Lot(e.to_string()))?
        .len();
    load.cleared_again = interpreter
        .clear(&mut world)
        .map_err(|e| TravelError::Lot(e.to_string()))?
        .len();
    let (stripped_people_to_reset, stripped_objects_to_reset) = strip_deleted_base_objects(
        &mut world,
        &mut file.objects,
        &stored.deleted_characters,
        &stored.deleted_at_epoch,
    );
    let people = People::load(
        install,
        index,
        catalog,
        file.objects.instances.values().map(|o| o.guid),
    )
    .map_err(|e| TravelError::Lot(e.to_string()))?;
    let read = interpreter
        .read_objects_with_frame_tree_bits(&mut world, &file.objects, &people, &BTreeSet::new())
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    load.read = read.objects.len();
    load.draws = read.draws;
    let reconciled = interpreter
        .reconcile_controllers(&mut world)
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    load.reconciled_created = reconciled.created.len();
    load.reconciled_removed = reconciled.removed.len();
    load.repaired = world
        .repair_grid()
        .map_err(|e| TravelError::Lot(e.to_string()))?
        .len();
    world.finish_object_read();
    load.queued_again = world.removals.len();
    world
        .rebuild_rooms()
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    let roomed = interpreter
        .assign_rooms(&mut world)
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    load.moved = roomed.moved.len();
    load.asked = roomed.asked.len();
    interpreter
        .update_rooms(&mut world, 0)
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    world.mark_all_rooms();
    interpreter
        .update_pending_rooms(&mut world)
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    load.dropped = world.discard_removals().len();

    if let Some(slot) = world.globals.get_mut(GLOBAL_BUDGET_MODE) {
        *slot = 0;
    }
    load.family = family_id;
    load.house = dest_house;
    world
        .select_family(
            i16::try_from(family_id).map_err(|e| TravelError::Lot(e.to_string()))?,
            i16::try_from(dest_house).map_err(|e| TravelError::Lot(e.to_string()))?,
            arrival_funds,
            family_magic,
        )
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    ensure_today_budget_slot(&mut world);

    load.to_visit = world.order.len();
    let visited = interpreter.visit_objects(&mut world, &people);
    load.visited = visited.objects.len();
    load.reset = visited.reset.len();
    load.stopped = visited.stopped.map(|fault| fault.to_string());
    if let Some(reason) = &load.stopped {
        return Err(TravelError::Lot(format!(
            "destination house {dest_house} load did not finish: {reason}"
        )));
    }
    for person_id in stripped_people_to_reset {
        if world
            .objects
            .get(&person_id)
            .is_some_and(|o| o.person.is_some())
        {
            interpreter
                .reset_person_after_purge(&mut world, person_id)
                .map_err(|e| {
                    TravelError::Lot(format!(
                        "resetting person {person_id} after stripped base character: {e}"
                    ))
                })?;
        }
    }
    for object_id in stripped_objects_to_reset {
        if world.objects.contains_key(&object_id) {
            interpreter
                .reset(&mut world, object_id, false)
                .map_err(|e| {
                    TravelError::Lot(format!(
                        "resetting object {object_id} after stripped base character: {e}"
                    ))
                })?;
        }
    }
    let _ = interpreter
        .ensure_indicators(&mut world)
        .map_err(|e| TravelError::Lot(e.to_string()))?;

    load.saved_objects = file.objects.instances.len();
    load.saved_people = file
        .objects
        .instances
        .values()
        .filter(|o| o.person.is_some())
        .count();
    load.saved_ticks = file.simulator.ticks;
    load.libraries = program.libraries.len();
    let start = world.ticks;
    let (lot_name, lot_description) =
        read_destination_lot_info(install, neighborhood_id, dest_house);
    Ok((
        world,
        load,
        view_rotation,
        view_rotation,
        requested_view_centre,
        start,
        0u32,
        lot_name,
        lot_description,
    ))
}

/// Boots a destination lot from REDB (`saved_lot` or `base_house`), initializes
/// destination SimAntics controllers & live spending mode (`globals[32] = 0`),
/// injects the traveling party with their live state, and selects the leader.
#[allow(clippy::too_many_arguments)]
fn boot_destination_lot(
    runtime: StoreRuntime,
    store_id: u128,
    trip_id: u128,
    neighborhood_id: &str,
    dest_house: u32,
    transport: TravelTransport,
    accommodation: VacationAccommodation,
    leader_guid: u32,
    is_date: bool,
    requested_view_centre: Option<[i8; 2]>,
    answers: &[u8],
    frames: Frames,
    home_family: u16,
    arrival_funds: i32,
    snapshots: &[LiveTravelerSnapshot],
    saved_lot: Option<&[u8]>,
    base_house: Option<Vec<u8>>,
) -> Result<(Lot, Vec<TravelerRecord>), TravelError> {
    let StoreRuntime {
        install,
        index,
        catalog,
        program,
        surfaces,
        stored,
        ..
    } = runtime;
    let family_magic = stored
        .families
        .get(&home_family)
        .map(|f| f.magic_history())
        .transpose()
        .map_err(|e| TravelError::Lot(e.to_string()))?
        .flatten();
    let family_members = stored
        .families
        .get(&home_family)
        .map(|f| f.members.clone())
        .unwrap_or_default();

    let (
        mut world,
        mut load,
        view_rotation,
        floor_rotation,
        effective_view_centre,
        start_ticks,
        saved_ticks,
        lot_name,
        lot_description,
    ) = load_destination_world(
        &install,
        &index,
        &catalog,
        &program,
        &surfaces,
        &stored,
        neighborhood_id,
        dest_house,
        home_family,
        arrival_funds,
        family_magic,
        &family_members,
        requested_view_centre,
        saved_lot,
        base_house,
    )?;

    // Unpause and configure live away-from-home play.
    if let Some(slot) = world.globals.get_mut(GLOBAL_BUDGET_MODE) {
        *slot = 0;
    }
    if let Some(slot) = world.globals.get_mut(GLOBAL_PAUSED) {
        *slot = 0;
    }
    if let Some(slot) = world.globals.get_mut(GLOBAL_MODE) {
        *slot = 0;
    }

    // Issue Vacation hotel keys on check-in before spawning/ticking so
    // category 4/7 hotel doors and beds unlock immediately (`0x458319`).
    if matches!(
        accommodation,
        VacationAccommodation::HotelCheckIn {
            grant_hotel_key: true
        }
    ) {
        for snap in snapshots {
            let current = neighbor_token_count(
                &world,
                snap.neighbor_id,
                HOTEL_KEY_TOKEN_KIND,
                HOTEL_KEY_TOKEN_GUID,
            );
            if current == 0 {
                set_neighbor_token_count(
                    &mut world,
                    snap.neighbor_id,
                    HOTEL_KEY_TOKEN_KIND,
                    HOTEL_KEY_TOKEN_GUID,
                    1,
                );
            }
        }
    }

    // Inject traveling party near the destination's arrival portal/curb.
    let (traveler_records, leader_id, counted) = {
        let interpreter = Interpreter {
            view_rotation,
            view_centre: effective_view_centre,
            active_family: Some(&family_members),
            ..Interpreter::new(&program, &catalog, Stream::Global)
        };
        let anchor = find_arrival_anchor(&world, Some(transport));
        let residential = !TravelDestination::is_community_mode(dest_house, &program.zoning);
        let mut records = Vec::with_capacity(snapshots.len());
        let mut leader_obj_id = None;
        let mut used_tiles = BTreeSet::new();

        for snap in snapshots {
            let mut matching: Vec<u16> = world
                .people
                .iter()
                .copied()
                .filter(|&id| {
                    world
                        .objects
                        .get(&id)
                        .is_some_and(|o| o.guid == snap.guid && o.person.is_some())
                })
                .collect();
            let (person_id, reused) = if let Some(primary) = matching.first().copied() {
                for extra in matching.drain(1..) {
                    strip_person_for_travel(&interpreter, &mut world, extra)?;
                }
                people::clear_person_uses_and_counts(&mut world, primary);
                if let Some(route) = world.routing.routes.get_mut(&primary) {
                    route.contexts.clear();
                }
                if let Some(person) = world
                    .objects
                    .get_mut(&primary)
                    .and_then(|o| o.person.as_mut())
                {
                    person.queue.clear();
                    person.active = crate::object_uses::no_action();
                    person.motive_deltas.clear();
                }
                let _ = crate::placement::unlink_off_lot(&mut world, primary);
                (primary, true)
            } else {
                let id = interpreter
                    .create_person(&mut world, snap.guid)
                    .map_err(|e| {
                        TravelError::Lot(format!(
                            "creating traveler 0x{:08X} on house {dest_house}: {e}",
                            snap.guid
                        ))
                    })?;
                (id, false)
            };
            // Place on a distinct walkable tile near the arrival portal BEFORE
            // applying fame >= 10 (so `placed_from_outside` 0x447a82 does not
            // trap on unported star-marker pool for non-selected celebrities).
            let saved_fame = snap
                .person_data
                .get(PERSON_FAME_LEVEL)
                .copied()
                .unwrap_or(0);
            if let Some(person) = world
                .objects
                .get_mut(&person_id)
                .and_then(|o| o.person.as_mut())
            {
                if let Some(f) = person.data.get_mut(PERSON_FAME_LEVEL) {
                    *f = 0;
                }
                if let Some(d) = person.data.get_mut(PERSON_DATE) {
                    *d = 0;
                }
            }
            let tile_pos = find_free_arrival_position(
                &interpreter,
                &mut world,
                person_id,
                anchor,
                &used_tiles,
            )?;
            used_tiles.insert([tile_pos[0].div_euclid(16), tile_pos[1].div_euclid(16)]);
            place_person_on_tile(&interpreter, &mut world, person_id, tile_pos)?;
            if reused {
                interpreter
                    .reset_person_after_purge(&mut world, person_id)
                    .map_err(|e| {
                        TravelError::Lot(format!(
                            "resetting reused traveler {person_id} on house {dest_house}: {e}"
                        ))
                    })?;
            }

            // Overlay live motives, skills, career/fame, accessories, outfits, and visitor word.
            apply_traveler_live_state(&mut world, person_id, snap, residential, saved_fame, None);
            if snap.guid == leader_guid {
                leader_obj_id = Some(person_id);
            }
            records.push(TravelerRecord {
                guid: snap.guid,
                neighbor_id: snap.neighbor_id,
                is_household_member: snap.is_household_member,
                home_object_id: snap.home_object_id,
                destination_object_id: person_id,
                had_hotel_key_before_trip: snap.had_hotel_key_before_trip,
            });
        }

        let _ = interpreter
            .announce_house(&mut world)
            .map_err(|e| TravelError::Lot(e.to_string()))?;
        let leader = leader_obj_id.ok_or_else(|| {
            TravelError::InvalidParty(format!(
                "leader 0x{:08X} was not injected onto destination house {dest_house}",
                leader_guid
            ))
        })?;
        select_sim_in_world(&interpreter, &mut world, leader)?;

        // Establish the mutual VM date link (`person.data[77] = partner_id`)
        // on Downtown Date arrivals after initial selection so date scripts see
        // each Sim's active date partner (`0x44664c`).
        if is_date && records.len() == 2 {
            let id_a = records[0].destination_object_id;
            let id_b = records[1].destination_object_id;
            if let Some(person_a) = world.objects.get_mut(&id_a).and_then(|o| o.person.as_mut())
                && let Some(slot) = person_a.data.get_mut(PERSON_DATE)
            {
                *slot = id_b as i16;
            }
            if let Some(person_b) = world.objects.get_mut(&id_b).and_then(|o| o.person.as_mut())
                && let Some(slot) = person_b.data.get_mut(PERSON_DATE)
            {
                *slot = id_a as i16;
            }
        }

        interpreter
            .go_live(&mut world)
            .map_err(|e| TravelError::Lot(e.to_string()))?;
        let counted = interpreter
            .before_frame(&mut world)
            .map_err(|e| TravelError::Lot(e.to_string()))?
            .len();
        (records, leader, counted)
    };

    load.finished = Some(Finished {
        indicators_made: false,
        sim: Some(leader_id),
        counted,
    });
    // Live destination handles carry `neighborhood_origin = None` so ordinary
    // `lifecycle::save_and_leave_lot` can never overwrite the Build/Buy
    // destination snapshot, and bind `loaded` to `(store_id, trip_id)`.
    let loaded = trip_binding_tag(store_id, trip_id);
    let fast = crate::program::ProgramFast::build(&program, &catalog);
    let lot = Lot {
        program,
        fast,
        catalog: Arc::new(catalog),
        surfaces,
        world,
        tuning: None,
        player: Answers::choices(answers),
        frames,
        ticks: saved_ticks,
        start: start_ticks,
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
        floor_rotation,
        view_centre: effective_view_centre,
        replay_view: None,
        last_dialog_text: String::new(),
        character_names: BTreeMap::new(),
        default_character_name: crate::boot::read_default_character_name(&install),
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
    };
    Ok((lot, traveler_records))
}

/// Restores the frozen home lot snapshot from REDB, reconciles neighborhood
/// records (`NBRS`, `FAMI`, `NGBH`, `TATT`, `made_characters`, inventories,
/// MagiCoins, funds), updates and repositions returning household travelers at
/// the home curb/portal with zero duplicate or orphaned objects, and preserves
/// the frozen home clock and non-traveling Sims/pets intact.
#[allow(clippy::too_many_arguments)]
fn reconcile_frozen_home_lot(
    runtime: StoreRuntime,
    store_id: u128,
    session: &TravelSession,
    frozen_home_bytes: &[u8],
    final_funds: i32,
    returning_states: &[LiveTravelerSnapshot],
    answers: &[u8],
    frames: Frames,
) -> Result<(Lot, Vec<(u32, u16)>), TravelError> {
    let saved = Saved::decode(frozen_home_bytes).map_err(|e| {
        TravelError::Lot(format!(
            "decoding frozen home lot {}: {e}",
            session.home_house
        ))
    })?;
    if saved.house != session.home_house {
        return Err(TravelError::SessionMismatch(format!(
            "frozen home lot is house {}, session expects {}",
            saved.house, session.home_house
        )));
    }
    let StoreRuntime {
        install,
        index,
        catalog,
        program,
        surfaces,
        stored,
        ..
    } = runtime;
    let origin_instance_id = stored.instance_id;
    let origin_revision = stored.revision;
    let family_members = stored
        .families
        .get(&session.home_family)
        .map(|f| f.members.clone())
        .unwrap_or_default();
    let family_magic = stored
        .families
        .get(&session.home_family)
        .map(|f| f.magic_history())
        .transpose()
        .map_err(|e| TravelError::Lot(e.to_string()))?
        .flatten();

    let mut world = saved.world;
    // Capture authoritative frozen clock and tick counters directly from the
    // decoded `Saved` home snapshot (never trusting caller-supplied session
    // fields).
    let frozen_ticks = saved.ticks;
    let frozen_world_ticks = world.ticks;
    let frozen_clock = (
        world.globals.get(GLOBAL_HOURS).copied().unwrap_or(0),
        world.globals.get(GLOBAL_MINUTES).copied().unwrap_or(0),
        world.globals.get(GLOBAL_SECONDS).copied().unwrap_or(0),
    );
    let content_nh = stored.to_content_neighborhood();
    let reg = neighborhood_registry(
        &index,
        &catalog,
        &stored.made_characters,
        &stored.deleted_characters,
    );
    world
        .read_neighbors(&content_nh, &reg)
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    world.type_attributes = program
        .type_attributes(&catalog)
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    if let Some(saved_tatt) = &stored.type_attributes {
        world.read_type_attributes(saved_tatt);
    }
    world.characters = stored.made_characters.clone();

    if let Some(slot) = world.globals.get_mut(GLOBAL_BUDGET_MODE) {
        *slot = 0;
    }
    world
        .select_family(
            i16::try_from(session.home_family).map_err(|e| TravelError::Lot(e.to_string()))?,
            i16::try_from(session.home_house).map_err(|e| TravelError::Lot(e.to_string()))?,
            final_funds,
            family_magic,
        )
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    ensure_today_budget_slot(&mut world);

    // Preserve exact frozen home clock (`globals[0/5/6]` and `world.ticks`).
    if let Some(h) = world.globals.get_mut(GLOBAL_HOURS) {
        *h = frozen_clock.0;
    }
    if let Some(m) = world.globals.get_mut(GLOBAL_MINUTES) {
        *m = frozen_clock.1;
    }
    if let Some(s) = world.globals.get_mut(GLOBAL_SECONDS) {
        *s = frozen_clock.2;
    }
    world.ticks = frozen_world_ticks;

    let (returning_travelers, selected_sim) = {
        let interpreter = Interpreter {
            view_rotation: saved.view_rotation,
            view_centre: saved.view_centre,
            active_family: Some(&family_members),
            ..Interpreter::new(&program, &catalog, Stream::Global)
        };
        let _ = interpreter
            .ensure_indicators(&mut world)
            .map_err(|e| TravelError::Lot(e.to_string()))?;

        // Purge any orphaned/deleted non-traveling visitors (e.g. a neighbor
        // whose home lot was bulldozed while this household was away) BEFORE
        // re-attaching surviving persons, matching `lifecycle::boot_from_saved_lot`.
        let returning_household_guids: BTreeSet<u32> = returning_states
            .iter()
            .filter(|s| s.is_household_member)
            .map(|s| s.guid)
            .collect();
        let lot_epoch = saved.deletion_epoch;
        let orphaned_home: Vec<u16> = world
            .people
            .iter()
            .copied()
            .filter(|&pid| {
                world.objects.get(&pid).is_some_and(|o| {
                    o.person.is_some()
                        && (stored.deleted_characters.contains(&o.guid)
                            || world.neighbors.first_of(o.guid).is_none()
                            || stored
                                .deleted_at_epoch
                                .get(&o.guid)
                                .is_some_and(|&del| lot_epoch < del))
                })
            })
            .collect();
        let mut home_people_to_reset = BTreeSet::new();
        let mut home_objects_to_reset = BTreeSet::new();
        for pid in orphaned_home {
            let (p_reset, o_reset) =
                strip_person_for_travel_deferred(&interpreter, &mut world, pid)?;
            home_people_to_reset.extend(p_reset);
            home_objects_to_reset.extend(o_reset);
        }

        // Re-attach all surviving non-traveling Sims on the home lot from latest NBRS.
        for pid in world.people.clone() {
            if let Some(obj) = world.objects.get(&pid)
                && obj.person.is_some()
                && !returning_household_guids.contains(&obj.guid)
            {
                interpreter
                    .attach_neighbor(&mut world, pid)
                    .map_err(|e| TravelError::Lot(format!("attaching home person {pid}: {e}")))?;
            }
        }
        apply_post_purge_resets(
            &interpreter,
            &mut world,
            home_people_to_reset,
            home_objects_to_reset,
        )?;

        // Deselect cleanly before repositioning returning travelers so we don't
        // move an active indicator prematurely or hit 0x446c15 on famous Sims.
        if let Some(sel) = selected(&world) {
            if let Some(obj) = world.objects.get_mut(&sel) {
                obj.selected = false;
            }
            let [first, second] = world.indicators;
            if first != 0 && world.objects.contains_key(&first) {
                let _ = crate::placement::unlink_off_lot(&mut world, first);
            }
            if second != 0 && world.objects.contains_key(&second) {
                let _ = crate::placement::unlink_off_lot(&mut world, second);
            }
            if let Some(slot) = world.globals.get_mut(GLOBAL_SELECTED) {
                *slot = 0;
            }
        }

        let anchor = find_arrival_anchor(&world, None);
        let mut used_tiles = BTreeSet::new();
        let mut reconciled_pairs = Vec::new();
        let mut leader_home_id = None;
        let home_i16 =
            i16::try_from(session.home_house).map_err(|e| TravelError::Lot(e.to_string()))?;

        for snap in returning_states.iter().filter(|s| s.is_household_member) {
            // Find all instances of this returning member on the home lot,
            // stripping any duplicate so zero duplicate/orphaned objects exist.
            let mut matching: Vec<u16> = world
                .people
                .iter()
                .copied()
                .filter(|&id| {
                    world
                        .objects
                        .get(&id)
                        .is_some_and(|o| o.guid == snap.guid && o.person.is_some())
                })
                .collect();
            let person_id = if let Some(primary) = matching.first().copied() {
                for extra in matching.drain(1..) {
                    strip_person_for_travel(&interpreter, &mut world, extra)?;
                }
                // Release any stale pre-departure object uses / routes / seat
                // cleanly before moving the returning Sim to the curb.
                people::clear_person_uses_and_counts(&mut world, primary);
                if let Some(route) = world.routing.routes.get_mut(&primary) {
                    route.contexts.clear();
                }
                if let Some(person) = world
                    .objects
                    .get_mut(&primary)
                    .and_then(|o| o.person.as_mut())
                {
                    person.queue.clear();
                    person.active = crate::object_uses::no_action();
                    person.motive_deltas.clear();
                }
                let _ = crate::placement::unlink_off_lot(&mut world, primary);
                primary
            } else {
                interpreter
                    .create_person(&mut world, snap.guid)
                    .map_err(|e| {
                        TravelError::Lot(format!(
                            "recreating returning traveler 0x{:08X} on home lot: {e}",
                            snap.guid
                        ))
                    })?
            };

            interpreter
                .attach_neighbor(&mut world, person_id)
                .map_err(|e| {
                    TravelError::Lot(format!(
                        "attaching returning traveler {person_id} on home lot: {e}"
                    ))
                })?;

            let saved_fame = snap
                .person_data
                .get(PERSON_FAME_LEVEL)
                .copied()
                .unwrap_or(0);
            if let Some(person) = world
                .objects
                .get_mut(&person_id)
                .and_then(|o| o.person.as_mut())
            {
                if let Some(f) = person.data.get_mut(PERSON_FAME_LEVEL) {
                    *f = 0;
                }
                if let Some(d) = person.data.get_mut(PERSON_DATE) {
                    *d = 0;
                }
            }

            let tile_pos = find_free_arrival_position(
                &interpreter,
                &mut world,
                person_id,
                anchor,
                &used_tiles,
            )?;
            used_tiles.insert([tile_pos[0].div_euclid(16), tile_pos[1].div_euclid(16)]);
            place_person_on_tile(&interpreter, &mut world, person_id, tile_pos)?;

            // Reset thread to Main cleanly so returning Sim resumes normal
            // home-lot autonomy/orders without stale pre-departure frames.
            interpreter
                .reset_person_after_purge(&mut world, person_id)
                .map_err(|e| {
                    TravelError::Lot(format!(
                        "resetting returning traveler {person_id} at home curb: {e}"
                    ))
                })?;

            // Re-apply returning live motives, skills, fame, outfits, and home flags.
            apply_traveler_live_state(
                &mut world,
                person_id,
                snap,
                true,
                saved_fame,
                Some(home_i16),
            );
            if snap.guid == session.leader_guid {
                leader_home_id = Some(person_id);
            }
            reconciled_pairs.push((snap.guid, person_id));
        }

        let target_sim = leader_home_id
            .or_else(|| reconciled_pairs.first().map(|&(_, id)| id))
            .or_else(|| world.people.first().copied());
        if let Some(sim) = target_sim {
            select_sim_in_world(&interpreter, &mut world, sim)?;
        }
        interpreter
            .go_live(&mut world)
            .map_err(|e| TravelError::Lot(e.to_string()))?;
        let _ = interpreter
            .before_frame(&mut world)
            .map_err(|e| TravelError::Lot(e.to_string()))?;
        (reconciled_pairs, target_sim)
    };

    let mut load = LoadReport {
        family: session.home_family,
        house: session.home_house,
        finished: Some(Finished {
            indicators_made: false,
            sim: selected_sim,
            counted: 0,
        }),
        ..LoadReport::default()
    };
    load.libraries = program.libraries.len();

    let player = if answers.is_empty() {
        saved.answers
    } else if saved.answered || saved.replay_view.is_some() {
        let mut p = saved.answers;
        p.extend_choices_after(answers);
        p
    } else {
        Answers::choices(answers)
    };

    let fast = crate::program::ProgramFast::build(&program, &catalog);
    let lot = Lot {
        program,
        fast,
        catalog: Arc::new(catalog),
        surfaces,
        world,
        tuning: None,
        player,
        frames,
        ticks: frozen_ticks,
        start: saved.start,
        load,
        halted: false,
        play: Play { fresh: saved.fresh },
        asks: false,
        asking: None,
        asking_special_panel: None,
        special_panels: saved.special_panels,
        special_panel_responses: saved.special_panel_responses,
        answered: saved.answered,
        view_rotation: saved.view_rotation,
        floor_rotation: saved.floor_rotation,
        view_centre: saved.view_centre,
        replay_view: saved.replay_view,
        last_dialog_text: saved.last_dialog_text,
        character_names: saved.character_names,
        default_character_name: crate::boot::read_default_character_name(&install),
        lot_name: saved.lot_name,
        lot_description: saved.lot_description,
        loaded: saved.loaded,
        buy_undo: Vec::new(),
        buy_redo: Vec::new(),
        buy_orphans: Vec::new(),
        active_buy_cursor: None,
        build_undo: Vec::new(),
        build_redo: Vec::new(),
        undo_order: Vec::new(),
        redo_order: Vec::new(),
        active_wall_cursor: None,
        neighborhood_origin: Some(NeighborhoodOrigin {
            store_id,
            neighborhood_id: session.neighborhood_id.clone(),
            instance_id: origin_instance_id,
            revision: origin_revision,
        }),
        coop: None,
        move_objects: false,
    };

    Ok((lot, returning_travelers))
}

/// Ensures `world.budget.days[0]` is `Some([0; 8])` so native `Alter Budget`
/// (`primitives/budget.rs::spend`) never fails on a lot whose base `SIMI`
/// stored an empty budget history.
fn ensure_today_budget_slot(world: &mut World) {
    if world.budget.days.is_empty() {
        world.budget.days = vec![Some([0; 8]); 6];
    } else if world.budget.days[0].is_none() {
        world.budget.days[0] = Some([0; 8]);
    }
}

/// Overlays a traveler's live motives, persistent & outfit person-data words,
/// outfits, and visitor/home flags onto `person_id`.
fn apply_traveler_live_state(
    world: &mut World,
    person_id: u16,
    snap: &LiveTravelerSnapshot,
    residential: bool,
    fame_level: i16,
    home_house_override: Option<i16>,
) {
    if let Some(obj) = world.objects.get_mut(&person_id)
        && let Some(person) = obj.person.as_mut()
    {
        person.motives = snap.motives;
        person.previous_motives = snap.previous_motives;
        // Copy persistent fields + outfit words (8, 21, 22, 23).
        for word in people::persistent_fields(10).into_iter().chain([
            PERSON_BODY_SUIT,
            PERSON_VARIANT,
            PERSON_HAND_LEFT,
            PERSON_HAND_RIGHT,
        ]) {
            if let Some(&val) = snap.person_data.get(word)
                && let Some(slot) = person.data.get_mut(word)
            {
                *slot = val;
            }
        }
        if let Some(slot) = person.data.get_mut(PERSON_NEIGHBOR) {
            *slot = snap.neighbor_id;
        }
        if let Some(slot) = person.data.get_mut(PERSON_FAME_LEVEL) {
            *slot = fame_level;
        }
        if let Some(slot) = person.data.get_mut(PERSON_DATE) {
            *slot = 0;
        }
        if let Some(home) = home_house_override
            && let Some(slot) = person.data.get_mut(PERSON_HOME)
        {
            *slot = home;
        }
        if let Some(slot) = person.data.get_mut(PERSON_VISITOR) {
            *slot = visitor_again(0, snap.is_household_member, residential);
        }
        person.accessories = snap.accessories.clone();
    }
    if let Some(outfit) = &snap.outfit {
        world.person_outfits.insert(person_id, outfit.clone());
    }
    if let Some(made_outfits) = &snap.made_outfits {
        world.made_outfits.insert(snap.guid, made_outfits.clone());
    }
}

/// Chooses the preferred curb/portal anchor tile for arriving Sims on `world`.
fn find_arrival_anchor(world: &World, transport: Option<TravelTransport>) -> [i32; 2] {
    let preferred_guids: &[u32] = match transport {
        Some(TravelTransport::MagicHole) => &[
            MAGIC_PORTAL_GUID,
            MAGIC_CAB_STOP_GUID,
            PED_PORTAL_GUID,
            CAR_PORTAL_GUID,
        ],
        Some(TravelTransport::MagicBlimp) => &[
            MAGIC_CAB_STOP_GUID,
            CAR_PORTAL_GUID,
            PED_PORTAL_GUID,
            MAGIC_PORTAL_GUID,
        ],
        _ => &[PED_PORTAL_GUID, CAR_PORTAL_GUID],
    };
    let size = i32::from(world.size);
    for &guid in preferred_guids {
        for &id in &world.order {
            if let Some(obj) = world.objects.get(&id)
                && obj.guid == guid
                && obj.position[0] >= 0
                && obj.position[1] >= 0
            {
                let tx = obj.position[0].div_euclid(16).clamp(1, (size - 2).max(1));
                let ty = obj.position[1].div_euclid(16).clamp(1, (size - 2).max(1));
                return [tx * 16 + 8, ty * 16 + 8];
            }
        }
    }
    let mid = (size / 2).clamp(2, (size - 2).max(2));
    [
        mid * 16 + 8,
        (size - 4).clamp(2, (size - 2).max(2)) * 16 + 8,
    ]
}

/// Finds a distinct walkable ground-level tile near `anchor` using the native
/// `0x4347f0` spiral search (`Interpreter::search`), falling back to a local
/// grid scan if every spiral tile around `anchor` is blocked.
fn find_free_arrival_position(
    interpreter: &Interpreter<'_>,
    world: &mut World,
    person_id: u16,
    anchor: [i32; 2],
    used_tiles: &BTreeSet<[i32; 2]>,
) -> Result<[i32; 2], TravelError> {
    let size = i32::from(world.size).max(4);
    let ax = anchor[0].div_euclid(16);
    let ay = anchor[1].div_euclid(16);
    let rules = Rules {
        same_room: false,
        empty: true,
        unmarked: false,
    };

    // Try expanding candidate starts around `anchor` so multiple party members
    // land on distinct adjacent curb/sidewalk tiles.
    let offsets: &[[i32; 2]] = &[
        [0, 0],
        [1, 0],
        [-1, 0],
        [0, -1],
        [0, 1],
        [2, 0],
        [-2, 0],
        [1, -1],
        [-1, -1],
        [1, 1],
        [-1, 1],
        [0, -2],
        [0, 2],
        [3, 0],
        [-3, 0],
        [0, -3],
    ];
    for &[dx, dy] in offsets {
        let sx = (ax + dx).clamp(1, size - 2);
        let sy = (ay + dy).clamp(1, size - 2);
        let start = [sx * 16 + 8, sy * 16 + 8];
        if let Some(found) = interpreter
            .search(world, person_id, start, 1, rules)
            .map_err(|e| TravelError::Lot(format!("searching arrival tile for {person_id}: {e}")))?
        {
            let tile = [found[0].div_euclid(16), found[1].div_euclid(16)];
            if !used_tiles.contains(&tile) {
                return Ok(found);
            }
        }
    }

    // Broad fallback: search from lot centre, still respecting `used_tiles`.
    let centre = [(size / 2) * 16 + 8, (size / 2) * 16 + 8];
    if let Some(found) = interpreter
        .search(world, person_id, centre, 1, rules)
        .map_err(|e| {
            TravelError::Lot(format!(
                "searching centre arrival tile for {person_id}: {e}"
            ))
        })?
    {
        let tile = [found[0].div_euclid(16), found[1].div_euclid(16)];
        if !used_tiles.contains(&tile) {
            return Ok(found);
        }
    }

    // Last-resort full placement check (`Interpreter::fits`: bounds, ground/
    // pool/water, slope, flags, walls, collisions, and footprint).
    for y in 2..(size - 2) {
        for x in 2..(size - 2) {
            if used_tiles.contains(&[x, y]) {
                continue;
            }
            let pos = [x * 16 + 8, y * 16 + 8];
            let tile_empty = crate::placement::head(world, [x as i8, y as i8, 1]).is_none();
            if tile_empty
                && interpreter.fits(world, person_id, pos, 1).map_err(|e| {
                    TravelError::Lot(format!(
                        "testing fallback arrival tile [{x}, {y}] for {person_id}: {e}"
                    ))
                })?
            {
                return Ok(pos);
            }
        }
    }

    Err(TravelError::Lot(format!(
        "no walkable arrival tile found on lot size {size} for person {person_id}"
    )))
}

/// Places `person_id` onto ground-level `pos`, updating grid, room, and footprint.
fn place_person_on_tile(
    interpreter: &Interpreter<'_>,
    world: &mut World,
    person_id: u16,
    pos: [i32; 2],
) -> Result<(), TravelError> {
    let place = Place {
        on_lot: true,
        position: pos,
        container: None,
        slot: -1,
        level: 1,
    };
    interpreter
        .put(world, person_id, place, true)
        .map_err(|e| TravelError::Lot(format!("placing person {person_id} at {pos:?}: {e}")))?;
    Ok(())
}

/// Removes any saved base-house object instances referencing deleted
/// characters (or people missing from `world.neighbors`) before
/// `read_objects_with_frame_tree_bits`, matching `lifecycle.rs`.
fn strip_deleted_base_objects(
    world: &mut World,
    objects: &mut sims_formats::saved_objects::Objects,
    deleted_characters: &BTreeSet<u32>,
    deleted_at_epoch: &BTreeMap<u32, u64>,
) -> (BTreeSet<u16>, BTreeSet<u16>) {
    let deleted_ids: Vec<u16> = objects
        .order
        .iter()
        .copied()
        .filter(|id| {
            objects.instances.get(id).is_some_and(|inst| {
                deleted_characters.contains(&inst.guid)
                    || deleted_at_epoch.get(&inst.guid).is_some_and(|&e| e > 0)
                    || (inst.person.is_some() && world.neighbors.first_of(inst.guid).is_none())
            })
        })
        .collect();
    let mut people_to_reset = BTreeSet::new();
    let mut objects_to_reset = BTreeSet::new();
    if deleted_ids.is_empty() {
        return (people_to_reset, objects_to_reset);
    }
    let full = FULL_SIZE as usize;
    for id in deleted_ids {
        let Some(removed) = objects.instances.remove(&id) else {
            continue;
        };
        objects.order.retain(|&other| other != id);
        objects.types_by_id.remove(&id);
        let id_i16 = id as i16;
        let deleted_type_id = removed.type_id as i16;

        if let Some(person) = &removed.person {
            for used in &person.object_uses {
                if let Ok(target_id) = u16::try_from(used.target)
                    && let Some(target_obj) = objects.instances.get_mut(&target_id)
                    && let Some(users) = target_obj.data.get_mut(62)
                    && *users > 0
                {
                    *users -= 1;
                }
            }
        }

        // Promote any surviving `slot 0` successor into `removed`'s tile grid
        // cell or predecessor slot.
        let next_id = removed
            .slots
            .first()
            .and_then(|s| u16::try_from(s.object).ok())
            .filter(|&n| n != 0 && objects.instances.contains_key(&n));
        let slot_word = removed.data.get(14).copied().unwrap_or(-1);
        let before_word = removed.data.get(26).copied().unwrap_or(0);
        if slot_word >= 0
            && let Ok(parent_id) = u16::try_from(before_word)
            && let Some(parent) = objects.instances.get_mut(&parent_id)
            && let Some(slot_entry) = parent.slots.get_mut(slot_word as usize)
            && slot_entry.object == id_i16
        {
            slot_entry.object = next_id.map_or(0, |n| n as i16);
        } else {
            let level = if removed.position[2] == 2 { 1 } else { 0 };
            let grid_id = crate::placement::LEVEL_GRIDS[level];
            let tx = removed.position[0].div_euclid(16);
            let ty = removed.position[1].div_euclid(16);
            if tx >= 0
                && ty >= 0
                && (tx as usize) < full
                && (ty as usize) < full
                && let Some(grid) = world.arrays.get_mut(&grid_id)
                && grid.stride == 2
            {
                let cell = (tx as usize * full + ty as usize) * 2;
                if let Some(bytes) = grid.data.get_mut(cell..cell + 2)
                    && bytes == id_i16.to_le_bytes()
                {
                    let replacement = next_id.unwrap_or(0u16);
                    bytes.copy_from_slice(&replacement.to_le_bytes());
                }
            }
        }
        if let Some(next) = next_id
            && let Some(next_obj) = objects.instances.get_mut(&next)
        {
            if let Some(s) = next_obj.data.get_mut(14) {
                *s = slot_word;
            }
            if let Some(b) = next_obj.data.get_mut(26) {
                *b = before_word;
            }
        }

        let target_i32 = i32::from(id_i16);
        for (&other_id, inst) in &mut objects.instances {
            for slot in &mut inst.slots {
                if slot.object == id_i16 {
                    slot.object = 0;
                }
            }
            inst.relationships.retain(|r| r.target != target_i32);
            let stack_hits = inst
                .stack
                .iter()
                .any(|f| f.stack_object == id_i16 || f.code_owner_type == deleted_type_id);
            if let Some(person) = inst.person.as_mut() {
                let uses_hit = person.object_uses.iter().any(|u| u.target == id_i16);
                let active_hit = person.active.target == id_i16
                    || person.active.icon == id_i16
                    || person.active.caller == id_i16;
                if uses_hit || active_hit || stack_hits {
                    people_to_reset.insert(other_id);
                    person.active = crate::object_uses::no_action();
                }
                person.object_uses.retain(|u| u.target != id_i16);
                person
                    .queue
                    .retain(|a| a.target != id_i16 && a.icon != id_i16 && a.caller != id_i16);
                if person.last.target == id_i16 || person.last.caller == id_i16 {
                    person.last = crate::object_uses::no_action();
                } else if person.last.icon == id_i16 {
                    person.last.icon = 0;
                }
            } else if stack_hits {
                objects_to_reset.insert(other_id);
            }
        }
        let id_bytes = id_i16.to_le_bytes();
        for grid_id in crate::placement::LEVEL_GRIDS {
            if let Some(grid) = world.arrays.get_mut(&grid_id)
                && grid.stride == 2
            {
                for cell in grid.data.as_chunks_mut::<2>().0 {
                    if *cell == id_bytes {
                        *cell = [0, 0];
                    }
                }
            }
        }
    }
    (people_to_reset, objects_to_reset)
}

/// Removes a person cleanly from `world` without leaving dangling slots, routes,
/// object uses, grid links, or live BHAV execution frames referencing the
/// removed object or its code owner, immediately resetting affected surviving
/// threads.
fn strip_person_for_travel(
    interpreter: &Interpreter<'_>,
    world: &mut World,
    id: u16,
) -> Result<(), TravelError> {
    let (people_to_reset, objects_to_reset) =
        strip_person_for_travel_deferred(interpreter, world, id)?;
    apply_post_purge_resets(
        interpreter,
        world,
        people_to_reset.into_iter().collect(),
        objects_to_reset.into_iter().collect(),
    )
}

/// Applies post-purge thread resets collected across one or more
/// [`strip_person_for_travel_deferred`] calls after all orphans have been
/// stripped and surviving persons re-attached.
fn apply_post_purge_resets(
    interpreter: &Interpreter<'_>,
    world: &mut World,
    people_to_reset: BTreeSet<u16>,
    objects_to_reset: BTreeSet<u16>,
) -> Result<(), TravelError> {
    for other in people_to_reset {
        if world
            .objects
            .get(&other)
            .is_some_and(|o| o.person.is_some())
        {
            interpreter
                .reset_person_after_purge(world, other)
                .map_err(|e| {
                    TravelError::Lot(format!("resetting person {other} after orphan purge: {e}"))
                })?;
        }
    }
    for other in objects_to_reset {
        if world.objects.contains_key(&other) {
            interpreter.reset(world, other, false).map_err(|e| {
                TravelError::Lot(format!("resetting object {other} after orphan purge: {e}"))
            })?;
        }
    }
    Ok(())
}

/// Removes a person cleanly from `world` and returns `(people_to_reset,
/// objects_to_reset)` so callers purging multiple orphans can finish all
/// removals and re-attach survivors before running post-purge resets
/// (matching `lifecycle.rs::purge_orphaned_person`).
fn strip_person_for_travel_deferred(
    interpreter: &Interpreter<'_>,
    world: &mut World,
    id: u16,
) -> Result<(Vec<u16>, Vec<u16>), TravelError> {
    let Some(deleted_guid) = world.objects.get(&id).map(|o| o.guid) else {
        return Ok((Vec::new(), Vec::new()));
    };
    people::clear_person_uses_and_counts(world, id);
    let _ = crate::placement::unlink_off_lot(world, id);
    if world.event_owner == id {
        world.event_owner = 0;
    }
    if selected(world) == Some(id) {
        if let Some(obj) = world.objects.get_mut(&id) {
            obj.selected = false;
        }
        if let Some(slot) = world.globals.get_mut(GLOBAL_SELECTED) {
            *slot = 0;
        }
    }
    let held_ids: Vec<u16> = world
        .objects
        .get(&id)
        .map(|o| {
            o.slots
                .iter()
                .skip(1)
                .filter_map(|s| u16::try_from(s.object).ok())
                .filter(|&held| held != 0)
                .collect()
        })
        .unwrap_or_default();
    for held in held_ids {
        if !world.objects.contains_key(&held) {
            continue;
        }
        if world.indicators.contains(&held) {
            if let Some(ind) = world.objects.get_mut(&held) {
                ind.position = [-16, -16, 1];
                if let Some(slot0) = ind.slots.first_mut() {
                    slot0.object = 0;
                }
            }
            continue;
        }
        let held_guid = world.objects[&held].guid;
        if world.types.values().any(|&g| g == held_guid)
            && crate::interpreter::object_definition(interpreter.catalog, world, held_guid)
                .is_some()
        {
            let _ = interpreter.remove_now(world, held);
        } else if let Some(obj) = world.objects.get_mut(&held)
            && let Some(slot0) = obj.slots.first_mut()
        {
            slot0.object = 0;
        }
    }
    let id_i16 = id as i16;
    let target = i32::from(id_i16);
    let mut people_to_reset = Vec::new();
    let mut objects_to_reset = Vec::new();
    let deleted_types: BTreeSet<i16> = world
        .types
        .iter()
        .filter_map(|(&ty, &guid)| {
            if guid == deleted_guid
                || crate::interpreter::object_definition(interpreter.catalog, world, guid).is_none()
            {
                i16::try_from(ty).ok()
            } else {
                None
            }
        })
        .collect();
    for (&other_id, obj) in &mut world.objects {
        if other_id == id {
            continue;
        }
        for slot in &mut obj.slots {
            if slot.object == id_i16 {
                slot.object = 0;
            }
        }
        obj.relationships.retain(|r| r.target != target);
        let stack_hits_deleted = obj.stack.iter().any(|frame| {
            frame.stack_object == id_i16 || deleted_types.contains(&frame.code_owner_type)
        });
        if let Some(person) = obj.person.as_mut() {
            let uses_deleted = person.object_uses.iter().any(|u| u.target == id_i16);
            let active_hits_deleted = person.active.target == id_i16
                || person.active.icon == id_i16
                || person.active.caller == id_i16;
            if uses_deleted || active_hits_deleted || stack_hits_deleted {
                people_to_reset.push(other_id);
                person.active = crate::object_uses::no_action();
            }
            person.object_uses.retain(|u| u.target != id_i16);
            person
                .queue
                .retain(|a| a.target != id_i16 && a.icon != id_i16 && a.caller != id_i16);
            if person.last.target == id_i16 || person.last.caller == id_i16 {
                person.last = crate::object_uses::no_action();
            } else if person.last.icon == id_i16 {
                person.last.icon = 0;
            }
        } else if stack_hits_deleted {
            objects_to_reset.push(other_id);
        }
    }
    world.objects.remove(&id);
    world.frame_storage.remove(&id);
    world.headlines.remove(&id);
    world.fires.retain(|&other| other != id);
    world.order.retain(|&other| other != id);
    world.people.retain(|&other| other != id);
    world.removals.retain(|&other| other != id);
    for instances in world.instances.values_mut() {
        instances.retain(|&other| other != id);
    }
    world.animators.remove(&id);
    world.base_layers.remove(&id);
    world.translated_bones.remove(&id);
    world.held_poses.remove(&id);
    world.stops_asked.remove(&id);
    world.person_ticks.remove(&id);
    world.person_outfits.remove(&id);
    world.routing.routes.remove(&id);
    for route in world.routing.routes.values_mut() {
        route.contexts.retain(|c| c.target != id);
        for c in &mut route.contexts {
            if c.objects[1] == id {
                c.byte_84 = true;
                c.objects[1] = 0;
                c.action = 0;
            }
        }
    }
    world.routing.slot_flags.retain(|&(obj, _), _| obj != id);
    world.routing.slots.retain(|&(obj, _), _| obj != id);
    world.routing.unknown_words.retain(|&(obj, _)| obj != id);
    let id_bytes = id_i16.to_le_bytes();
    for grid_id in crate::placement::LEVEL_GRIDS {
        if let Some(grid) = world.arrays.get_mut(&grid_id)
            && grid.stride == 2
        {
            for cell in grid.data.as_chunks_mut::<2>().0 {
                if *cell == id_bytes {
                    *cell = [0, 0];
                }
            }
        }
    }
    world.repair_grid().map_err(|e| {
        TravelError::Lot(format!("repairing grid after stripping person {id}: {e}"))
    })?;
    Ok((people_to_reset, objects_to_reset))
}

/// Reads `(lot_name, lot_description)` for `lot_id` from the install's
/// neighborhood `*Desc.iff` file (`STR# (2000 + lot_id)`).
fn read_destination_lot_info(
    install: &Install,
    neighborhood_id: &str,
    lot_id: u32,
) -> (String, String) {
    let desc_file = TravelDestination::desc_file(lot_id);
    let rel = format!("{neighborhood_id}/Houses/{desc_file}");
    let path = install
        .file(&rel)
        .or_else(|_| install.file(&format!("UserData/Houses/{desc_file}")));
    let Ok(path) = path else {
        return (String::new(), String::new());
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return (String::new(), String::new());
    };
    let Ok(iff) = Iff::parse(bytes) else {
        return (String::new(), String::new());
    };
    let Ok(table_id) = u16::try_from(2000u32.saturating_add(lot_id)) else {
        return (String::new(), String::new());
    };
    let Some(chunk) = iff.get(b"STR#", table_id) else {
        return (String::new(), String::new());
    };
    let Ok(table) = StringTable::parse(iff.data(chunk)) else {
        return (String::new(), String::new());
    };
    let name = table.get(0, US_ENGLISH).unwrap_or_default().to_owned();
    let desc = table.get(1, US_ENGLISH).unwrap_or_default().to_owned();
    (name, desc)
}

/// Shared install + neighborhood resources loaded for a REDB lot transition.
struct StoreRuntime {
    install: Install,
    index: Index,
    catalog: Catalog,
    program: Program,
    surfaces: Surfaces,
    stored: StoredNeighborhood,
}

impl StoreRuntime {
    fn from_snapshot(
        install_root: &Path,
        stored: StoredNeighborhood,
        character_files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Self, TravelError> {
        let install = Install::detect(install_root).map_err(|e| TravelError::Lot(e.to_string()))?;
        let mut index = Index::build(&install).map_err(|e| TravelError::Lot(e.to_string()))?;
        if !character_files.is_empty() {
            index
                .add_character_files(character_files)
                .map_err(|e| TravelError::Lot(e.to_string()))?;
        }
        let catalog = Catalog::build(&index).map_err(|e| TravelError::Lot(e.to_string()))?;
        let mut program = Program::link(&index, &catalog, catalog.objects.iter().map(|o| o.guid))
            .map_err(|e| TravelError::Lot(e.to_string()))?;
        program.zoning = stored.zoning.clone();
        program.expansions = u16::from(install.expansions.0);
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
        program.animations =
            Animations::load(&install, &index).map_err(|e| TravelError::Lot(e.to_string()))?;
        let person_types = catalog
            .objects
            .iter()
            .filter(|o| o.kind == 2)
            .map(|o| o.guid);
        program.people = People::load_each(&install, &index, &catalog, person_types)
            .map_err(|e| TravelError::Lot(e.to_string()))?
            .0;
        program.icon_frames =
            crate::program::icon_frames(&install).map_err(|e| TravelError::Lot(e.to_string()))?;
        program.wardrobe = Some(match sims_content::Assets::scan(&install.root) {
            Ok(assets) => sims_content::Wardrobe::load(&assets),
            Err(error) => sims_content::Wardrobe::unread(error),
        });
        program.ordered_wardrobe =
            Some(match sims_content::ImportWardrobe::load_lenient(&install) {
                Ok((ordered, errors)) if errors.is_empty() => Ok(ordered),
                Ok((_, errors)) => {
                    let (file, error) = errors.into_iter().next().unwrap();
                    Err(format!("{file} ({error})"))
                }
                Err(error) => Err(error.to_string()),
            });
        if !character_files.is_empty() {
            program.character_files =
                crate::program::character_files(&index, Path::new(Index::CHARACTERS));
        }
        program
            .characters
            .extend(stored.made_characters.made.keys().copied());
        program
            .characters
            .retain(|guid| !stored.deleted_characters.contains(guid));
        program
            .character_files
            .extend(stored.made_characters.made.values().map(|c| c.file));
        let surfaces = Surfaces::build(&install).map_err(|e| TravelError::Lot(e.to_string()))?;
        Ok(Self {
            install,
            index,
            catalog,
            program,
            surfaces,
            stored,
        })
    }
}

fn neighborhood_registry(
    index: &Index,
    catalog: &Catalog,
    made: &sims_sim::characters::Characters,
    deleted: &BTreeSet<u32>,
) -> Vec<sims_sim::neighbors::Registered> {
    let mut reg = registry(index, catalog);
    reg.retain(|entry| !deleted.contains(&entry.guid));
    let existing: BTreeSet<u32> = reg.iter().map(|entry| entry.guid).collect();
    for (&guid, character) in &made.made {
        if existing.contains(&guid) || deleted.contains(&guid) {
            continue;
        }
        let kind = catalog
            .object(character.template)
            .map_or(2, |object| object.kind);
        reg.push(sims_sim::neighbors::Registered {
            guid,
            kind,
            resolves: true,
        });
    }
    reg
}

/// Neighborhood tuning constants (`Global.far` `FCNS #4` & `FCNS #2`).
#[derive(Debug, Clone, Copy)]
struct NeighborhoodTuning {
    arch_value_multiplier: f32,
    friendship_threshold: i32,
}

impl Default for NeighborhoodTuning {
    fn default() -> Self {
        Self {
            arch_value_multiplier: DEFAULT_ARCH_VALUE_MULTIPLIER,
            friendship_threshold: DEFAULT_FRIENDSHIP_THRESHOLD,
        }
    }
}

impl NeighborhoodTuning {
    fn from_program(program: &Program) -> Self {
        let mut tuning = Self::default();
        if let Some(global) = program.libraries.get(&program.global) {
            if let Some(fcns) = global.tuning.get(&NEIGHBORHOOD_TUNING)
                && let Some(v) = fcns
                    .entries
                    .iter()
                    .find(|e| e.name.eq_ignore_ascii_case("architecture value multiplier"))
                    .map(|e| e.value)
            {
                tuning.arch_value_multiplier = v;
            }
            if let Some(fcns) = global.tuning.get(&AUTONOMY_TUNING)
                && let Some(v) = fcns
                    .entries
                    .iter()
                    .find(|e| e.name.eq_ignore_ascii_case("friendship threshold"))
                    .map(|e| e.value)
            {
                tuning.friendship_threshold = v as i32;
            }
        }
        tuning
    }
}

/// Architecture surface prices (`Build.iff` + pattern catalogs) needed by
/// `0x47b006` (`architecture_value`).
#[derive(Debug, Clone, Default)]
struct LotPrices {
    styles: BTreeMap<u16, i32>,
    pool: i32,
    floors: BTreeMap<u16, i32>,
    walls: BTreeMap<u16, i32>,
}

impl LotPrices {
    fn load(install: &Install, surfaces: &Surfaces) -> Self {
        let tables: BTreeMap<u16, StringTable> = install
            .file("GameData/Build.iff")
            .ok()
            .and_then(|path| Iff::parse(std::fs::read(path).ok()?).ok())
            .map(|iff| {
                [
                    STYLE_TABLE,
                    FLOOR_TABLE,
                    WALL_TABLE,
                    POOL_TABLE,
                    FLOOR_DEFAULT_TABLE,
                    WALL_DEFAULT_TABLE,
                ]
                .into_iter()
                .filter_map(|id| {
                    let chunk = iff.get(b"STR#", id)?;
                    Some((
                        id,
                        StringTable::parse_chunk(iff.data(chunk), iff.resource_flags(chunk))
                            .ok()?,
                    ))
                })
                .collect()
            })
            .unwrap_or_default();
        let price = |table: u16, entry: u16| -> Option<i32> {
            let at = usize::from(entry).checked_sub(1)? * 3;
            Some(atoi(&tables.get(&table)?.entry(at, US_ENGLISH)?.raw_text))
        };
        let mut styles: BTreeMap<u16, i32> = PRICED_STYLES
            .iter()
            .zip(1u16..)
            .map(|(&style, entry)| (style, price(STYLE_TABLE, entry).unwrap_or(0)))
            .collect();
        styles.insert(SECOND_STYLE, styles[&PRICED_STYLES[0]]);
        let pool = price(POOL_TABLE, 1).unwrap_or(0);
        let mut floors = BTreeMap::new();
        for key in 1u16..=29 {
            floors.insert(key, price(FLOOR_TABLE, key).unwrap_or(0));
        }
        let mut walls = BTreeMap::new();
        for key in (0u16..=30).chain(248..=255) {
            walls.insert(key, price(WALL_TABLE, key).unwrap_or(0));
        }
        let plug_ins = |catalog: &SurfaceCatalog, default: i32, out: &mut BTreeMap<u16, i32>| {
            for file in catalog.files.iter().skip(1) {
                for &key in file.keys.values() {
                    out.insert(key, file.price.unwrap_or(default));
                }
            }
        };
        plug_ins(
            &surfaces.floors,
            price(FLOOR_DEFAULT_TABLE, 1).unwrap_or(0),
            &mut floors,
        );
        plug_ins(
            &surfaces.walls,
            price(WALL_DEFAULT_TABLE, 1).unwrap_or(0),
            &mut walls,
        );
        Self {
            styles,
            pool,
            floors,
            walls,
        }
    }
}

fn atoi(bytes: &[u8]) -> i32 {
    let mut i = 0;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let negative = match bytes.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut value = 0i64;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        value = value
            .wrapping_mul(10)
            .wrapping_add(i64::from(bytes[i] - b'0'));
        i += 1;
    }
    if negative {
        value.wrapping_neg() as i32
    } else {
        value as i32
    }
}

fn architecture_value_and_built(world: &World, prices: &LotPrices) -> (i32, bool) {
    let full = FULL_SIZE as usize;
    let size = usize::try_from(world.size).unwrap_or(0).min(full);
    let array = |id: u16, stride: usize| {
        world
            .arrays
            .get(&id)
            .filter(|a| a.stride == stride && a.data.len() == full * full * stride)
            .map(|a| a.data.as_slice())
    };
    let mut walls_sum = 0i32;
    let mut rest = 0i32;
    let mut built = false;
    let floor_price = |key: u16| prices.floors.get(&key).copied().unwrap_or(0);
    for level in 0..2 {
        let (Some(flags), Some(floors), Some(walls)) = (
            array(TILE_FLAGS[level], 1),
            array(FLOORS[level], 2),
            array(WALLS[level], WALL_CELL),
        ) else {
            continue;
        };
        for b in 0..size {
            for a in 0..size {
                let cell = a * full + b;
                if flags[cell] & TILE_LOCKED != 0 {
                    continue;
                }
                let wall = ExpandedWall::get(walls, size, a, b);
                let segments = u32::from(wall.segments);
                if segments != 0 {
                    built = true;
                }
                let style = |index: usize| match index {
                    0..6 => wall.styles[index].unwrap_or(0),
                    _ => wall.patterns.get(index - 6).copied().unwrap_or(0),
                };
                let mut bit = segments & segments.wrapping_neg();
                while bit != 0 {
                    let index = bit.trailing_zeros() as usize;
                    let kind = style(index);
                    walls_sum =
                        walls_sum.wrapping_add(prices.styles.get(&kind).copied().unwrap_or(0));
                    if PATTERNED_WALL_STYLES.contains(&kind) {
                        let side = index + usize::from(bit == 0x10 || bit == 0x20);
                        let pattern = wall.patterns.get(side).copied().unwrap_or(0);
                        rest = rest.wrapping_add(prices.walls.get(&pattern).copied().unwrap_or(0));
                    }
                    loop {
                        bit <<= 1;
                        if segments & bit == bit || bit >= 0x20 {
                            break;
                        }
                    }
                    if bit >= 0x20 {
                        break;
                    }
                }
                let floor = u16::from_le_bytes([floors[cell * 2], floors[cell * 2 + 1]]);
                if floor != 0 {
                    built = true;
                }
                if floor == SPLIT_FLOOR {
                    if segments & 0x30 != 0 {
                        for half in [style(0), wall.patterns[0]] {
                            if half != 0 {
                                rest = rest.wrapping_add(floor_price(half) / 2);
                            }
                        }
                    }
                } else if floor != 0 {
                    rest = rest.wrapping_add(floor_price(floor));
                }
                if level == 0
                    && let Some(pool) = array(POOL_ARRAY, 1)
                {
                    if pool[cell] < POOL_TILE_LIMIT {
                        built = true;
                        rest = rest.wrapping_add(prices.pool);
                    } else if pool[cell] == POOL_TILE_LIMIT {
                        built = true;
                    }
                }
            }
        }
    }
    ((walls_sum / 2).wrapping_add(rest), built)
}

fn update_lot_values(
    world: &mut World,
    interpreter: &Interpreter<'_>,
    prices: &LotPrices,
) -> Result<(bool, bool), TravelError> {
    let mut objects_value = 0i32;
    let mut arch_objects_value = 0i32;
    let mut has_cat1 = false;
    let mut has_cat9 = false;
    let mut built_from_objects = false;

    for id in world.order.clone() {
        let Some(object) = world.objects.get(&id) else {
            continue;
        };
        let category = object.data.get(DATA_CATEGORY).copied().unwrap_or(0);
        let lead = object
            .multipart
            .as_ref()
            .is_none_or(|part| part.parent == 0 || part.parent as u16 == id);
        let Ok(words) = interpreter.words(world, id).copied() else {
            continue;
        };
        if category == 1 {
            if crate::placement::stands_on_lot(world, id).unwrap_or(false) {
                has_cat1 = true;
            }
        } else if category == 9 {
            has_cat9 = true;
        }
        if (words[word::GROUP] != 0 && !lead) || words[word::TYPE] == DEF_TYPE_SKIPPED {
            continue;
        }
        let on_lot = crate::placement::stands_on_lot(world, id).unwrap_or(false);
        if on_lot && DEF_BUILT.iter().any(|&w| words[w] != 0) {
            let [a, b, level] = World::tile(world.objects[&id].position);
            if crate::placement::tile_flags(world, a, b, i32::from(level)).unwrap_or(TILE_LOCKED)
                & TILE_LOCKED
                == 0
            {
                built_from_objects = true;
            }
        }
        let valued = interpreter.valued(world, id).unwrap_or(false);
        let val = i32::from(object_value(world, &words, id).unwrap_or(0));
        if on_lot && valued {
            objects_value = objects_value.wrapping_add(val);
        } else if let Some(def) = crate::interpreter::object_definition(
            interpreter.catalog,
            world,
            world.objects[&id].guid,
        ) && let Ok(Some(master)) = interpreter
            .program
            .master_definition(def.resource, def.chunk)
            && !matches!(master[DEF_BUILD_KIND], 0 | 4)
        {
            arch_objects_value = arch_objects_value.wrapping_add(val);
        }
    }

    let (arch_surfaces, built_from_arch) = architecture_value_and_built(world, prices);
    world.budget.objects_value = objects_value;
    world.budget.architecture_value = arch_surfaces.wrapping_add(arch_objects_value);
    world
        .update_statistics()
        .map_err(|e| TravelError::Lot(e.to_string()))?;
    if let Some(slot) = world.globals.get_mut(GLOBAL_BUILT) {
        *slot = i16::from(built_from_objects || built_from_arch);
    }
    Ok((has_cat1, has_cat9))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_formats::neighborhood::Family;
    use sims_formats::saved_objects::{Interaction, Person};
    use sims_sim::characters::Characters;
    use sims_sim::neighbors::{Neighbor, PERSON_WORDS};

    fn test_zoning() -> BTreeMap<i16, i32> {
        let mut z = BTreeMap::new();
        for lot in 1..=10 {
            z.insert(lot, 1);
        }
        for lot in 50..=79 {
            z.insert(
                lot,
                if matches!(lot, 58 | 61 | 70..=75) {
                    2
                } else {
                    1
                },
            );
        }
        for lot in 90..=92 {
            z.insert(lot, 1);
        }
        for lot in 93..=98 {
            z.insert(lot, 2);
        }
        z
    }

    fn make_person_obj(
        id: u16,
        guid: u32,
        neighbor: i16,
        age: i16,
        kind: i16,
        fame: i16,
    ) -> sims_sim::Object {
        let idle = Interaction {
            join: 0,
            uid: 0,
            caller: 0,
            target: 0,
            icon: 0,
            table_index: -1,
            args: [0; 4],
            priority: 0,
            action: 0,
            attenuation: 0.0,
            flags: 0,
        };
        let mut data = vec![0i16; PERSON_WORDS];
        data[PERSON_NEIGHBOR] = neighbor;
        data[PERSON_AGE] = age;
        data[PERSON_KIND] = kind;
        data[PERSON_FAME_LEVEL] = fame;
        sims_sim::Object {
            id,
            guid,
            footprint: [0; 4],
            position: [0; 3],
            unknown_short: 0,
            attributes: Vec::new(),
            temps: vec![0; 8],
            data: vec![0; 128],
            obsolete_word: 0,
            stack_capacity_bytes: 0,
            stack: Vec::new(),
            relationship_flag: 0,
            relationships: Vec::new(),
            slots: Vec::new(),
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
                previous_motives: [50.0; 16],
                motives: [50.0; 16],
                data,
                routing_frames: 0,
                active: idle.clone(),
                last: idle,
                queue: Vec::new(),
                object_uses: Vec::new(),
                motive_deltas: Vec::new(),
            }),
            registry_word: None,
            last_result: false,
            removal_skips_resets: false,
            selected: false,
        }
    }

    fn make_neighbor(id: i16, guid: u32, age: i16, kind: i16, fame: i16) -> Neighbor {
        let mut data = vec![0i16; PERSON_WORDS];
        data[PERSON_AGE] = age;
        data[PERSON_KIND] = kind;
        data[PERSON_FAME_LEVEL] = fame;
        Neighbor {
            id,
            guid,
            revision: 9,
            data,
            relationships: Vec::new(),
            friends: 0,
            stale: false,
        }
    }

    fn fixture_world_and_nh() -> (World, StoredNeighborhood) {
        let mut world = World::empty();
        world.globals = vec![0; 64];
        world.budget.funds = 2_000;
        // Adult 1 (0x1001), Child 2 (0x1002), Pet Dog 3 (0x1003), Townie Adult 4 (0x2001)
        world
            .objects
            .insert(1, make_person_obj(1, 0x1001, 1, 27, 0, 15));
        world
            .objects
            .insert(2, make_person_obj(2, 0x1002, 2, 9, 1, 0));
        world
            .objects
            .insert(3, make_person_obj(3, 0x1003, 3, 27, 2, 0));
        world.people = vec![1, 2, 3];
        world.neighbors.slots = vec![
            Some(make_neighbor(1, 0x1001, 27, 0, 15)),
            Some(make_neighbor(2, 0x1002, 9, 1, 0)),
            Some(make_neighbor(3, 0x1003, 27, 2, 0)),
            Some(make_neighbor(4, 0x2001, 27, 1, 0)),
        ];
        set_neighbor_token_count(&mut world, 1, MAGICOIN_TOKEN_KIND, MAGICOIN_TOKEN_GUID, 125);
        set_neighbor_token_count(&mut world, 2, MAGICOIN_TOKEN_KIND, MAGICOIN_TOKEN_GUID, 25);

        let mut families = BTreeMap::new();
        families.insert(
            5,
            Family {
                version: 9,
                house: 5,
                number: 5,
                budget: 2_000,
                architecture_value: 20_000,
                friends: 0,
                flags: 0,
                members: vec![0x1001, 0x1002, 0x1003],
                tail: vec![0; 16],
            },
        );
        let stored = StoredNeighborhood {
            name: "Test".into(),
            families,
            family_surnames: BTreeMap::new(),
            neighbors: Vec::new(),
            inventories: Vec::new(),
            data: [0; 16],
            options: None,
            type_attributes: None,
            zoning: test_zoning(),
            lot_rows: Vec::new(),
            made_characters: Characters::default(),
            deleted_characters: BTreeSet::new(),
            deletion_epoch: 0,
            deleted_at_epoch: BTreeMap::new(),
            instance_id: 1,
            revision: 0,
        };
        (world, stored)
    }

    #[test]
    fn destination_classification_covers_all_complete_collection_zones() {
        let z = test_zoning();
        assert_eq!(classify_destination(0, &z), None);
        assert_eq!(classify_destination(5, &z), None);
        assert_eq!(classify_destination(50, &z), None); // Old Town residential
        assert_eq!(
            classify_destination(21, &z),
            Some(TravelDestination::Downtown)
        );
        assert_eq!(
            classify_destination(30, &z),
            Some(TravelDestination::Downtown)
        );
        assert_eq!(
            classify_destination(40, &z),
            Some(TravelDestination::VacationIsland)
        );
        assert_eq!(
            classify_destination(48, &z),
            Some(TravelDestination::VacationIsland)
        );
        assert_eq!(
            classify_destination(58, &z),
            Some(TravelDestination::OldTown)
        );
        assert_eq!(
            classify_destination(70, &z),
            Some(TravelDestination::OldTown)
        );
        assert_eq!(
            classify_destination(81, &z),
            Some(TravelDestination::StudioTown)
        );
        assert_eq!(
            classify_destination(89, &z),
            Some(TravelDestination::StudioTown)
        );
        assert_eq!(
            classify_destination(90, &z),
            Some(TravelDestination::MagicTown)
        );
        assert_eq!(
            classify_destination(93, &z),
            Some(TravelDestination::MagicTown)
        );
        assert_eq!(
            classify_destination(98, &z),
            Some(TravelDestination::MagicTown)
        );
        assert!(TravelDestination::is_magic_residential(90, &z));
        assert!(!TravelDestination::is_magic_residential(93, &z));
        assert!(TravelDestination::is_community_mode(21, &z));
        assert!(TravelDestination::is_community_mode(40, &z));
        assert!(TravelDestination::is_community_mode(58, &z));
        assert!(TravelDestination::is_community_mode(81, &z));
        assert!(TravelDestination::is_community_mode(93, &z));
    }

    #[test]
    fn eligibility_rules_enforce_expansion_party_age_species_fame_and_funds() {
        let (mut world, stored) = fixture_world_and_nh();
        let all_ep = 0xffu16;

        // MagiCoins count sums across household members.
        assert_eq!(family_magic_coins(&world, &[0x1001, 0x1002, 0x1003]), 150);

        // Downtown: adult solo OK, adult + townie date OK, child solo rejected.
        let dt_solo = validate_travel_plan(
            &world,
            5,
            5,
            all_ep,
            &stored,
            &TravelRequest::downtown(21, 0x1001),
        )
        .unwrap();
        assert_eq!(dt_solo.fare, DOWNTOWN_FARE);
        assert_eq!(dt_solo.members.len(), 1);

        let dt_date = validate_travel_plan(
            &world,
            5,
            5,
            all_ep,
            &stored,
            &TravelRequest::downtown_date(21, 0x1001, 0x2001),
        )
        .unwrap();
        assert_eq!(dt_date.members.len(), 2);
        assert!(!dt_date.members[1].is_household_member);

        assert!(matches!(
            validate_travel_plan(
                &world,
                5,
                5,
                all_ep,
                &stored,
                &TravelRequest::downtown(21, 0x1002),
            ),
            Err(TravelError::InvalidParty(_))
        ));

        // Vacation Island: requires whole family, adult leader, $500 fare.
        let vac = validate_travel_plan(
            &world,
            5,
            5,
            all_ep,
            &stored,
            &TravelRequest::vacation(
                40,
                0x1001,
                VacationAccommodation::HotelCheckIn {
                    grant_hotel_key: true,
                },
            ),
        )
        .unwrap();
        assert_eq!(vac.fare, VACATION_FARE);
        assert_eq!(vac.members.len(), 3);

        // Studio Town: adult solo OK; limo OK for fame=15, rejected for fame < 10.
        let st_limo = TravelRequest {
            transport: Some(TravelTransport::StudioLimo),
            ..TravelRequest::studio_town(81, 0x1001)
        };
        assert_eq!(
            validate_travel_plan(&world, 5, 5, all_ep, &stored, &st_limo)
                .unwrap()
                .fare,
            0
        );
        world
            .objects
            .get_mut(&1)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[PERSON_FAME_LEVEL] = 5;
        assert!(matches!(
            validate_travel_plan(&world, 5, 5, all_ep, &stored, &st_limo),
            Err(TravelError::InvalidParty(_))
        ));

        // Magic Town: child solo via free Magic Hole ($0) allowed; blimp costs $50.
        let mt_kid = validate_travel_plan(
            &world,
            5,
            5,
            all_ep,
            &stored,
            &TravelRequest::magic_town(
                90,
                TravelParty::Single { sim_guid: 0x1002 },
                TravelTransport::MagicHole,
            ),
        )
        .unwrap();
        assert_eq!(mt_kid.fare, 0);
        assert_eq!(mt_kid.members.len(), 1);

        // Missing expansion bit rejected.
        assert!(matches!(
            validate_travel_plan(
                &world,
                5,
                5,
                EXPANSION_HOT_DATE,
                &stored,
                &TravelRequest::studio_town(81, 0x1001),
            ),
            Err(TravelError::ExpansionNotInstalled {
                destination: TravelDestination::StudioTown,
                ..
            })
        ));

        // Insufficient funds rejected cleanly.
        world.budget.funds = 40;
        assert!(matches!(
            validate_travel_plan(
                &world,
                5,
                5,
                all_ep,
                &stored,
                &TravelRequest::downtown(21, 0x1001),
            ),
            Err(TravelError::InsufficientFunds {
                available: 40,
                required: 50,
                ..
            })
        ));

        // Negative fare override rejected.
        world.budget.funds = 2_000;
        let bad_fare = TravelRequest {
            fare_override: Some(-10),
            ..TravelRequest::downtown(21, 0x1001)
        };
        assert!(matches!(
            validate_travel_plan(&world, 5, 5, all_ep, &stored, &bad_fare),
            Err(TravelError::InvalidFare(-10))
        ));

        // Incompatible transport & non-Vacation accommodation rejected.
        let bad_transport = TravelRequest {
            transport: Some(TravelTransport::MagicHole),
            ..TravelRequest::downtown(21, 0x1001)
        };
        assert!(matches!(
            validate_travel_plan(&world, 5, 5, all_ep, &stored, &bad_transport),
            Err(TravelError::IncompatibleTransport { .. })
        ));
        let bad_accom = TravelRequest {
            accommodation: VacationAccommodation::Campground,
            ..TravelRequest::downtown(21, 0x1001)
        };
        assert!(matches!(
            validate_travel_plan(&world, 5, 5, all_ep, &stored, &bad_accom),
            Err(TravelError::IncompatibleAccommodation { .. })
        ));

        // Departing from wrong home house or while already away (`secondary_house != 0`) rejected.
        assert!(matches!(
            validate_travel_plan(
                &world,
                6,
                5,
                all_ep,
                &stored,
                &TravelRequest::downtown(21, 0x1001),
            ),
            Err(TravelError::SessionMismatch(_))
        ));
        let mut away_stored = stored.clone();
        away_stored
            .families
            .get_mut(&5)
            .unwrap()
            .set_secondary_house(21);
        assert!(matches!(
            validate_travel_plan(
                &world,
                5,
                5,
                all_ep,
                &away_stored,
                &TravelRequest::studio_town(81, 0x1001),
            ),
            Err(TravelError::SessionMismatch(_))
        ));

        // Occupied Magic Town residential lot rejected.
        let mut occupied_stored = stored.clone();
        occupied_stored.families.insert(
            9,
            Family {
                version: 9,
                house: 90,
                number: 9,
                budget: 5_000,
                architecture_value: 10_000,
                friends: 0,
                flags: 0,
                members: vec![0x2001],
                tail: vec![0; 16],
            },
        );
        assert!(matches!(
            validate_travel_plan(
                &world,
                5,
                5,
                all_ep,
                &occupied_stored,
                &TravelRequest::magic_town(
                    90,
                    TravelParty::Single { sim_guid: 0x1001 },
                    TravelTransport::MagicHole,
                ),
            ),
            Err(TravelError::DestinationOccupied {
                house: 90,
                family: 9
            })
        ));

        // HouseholdGroup: leader listed once is accepted; duplicate leader or
        // duplicate member is rejected; pet leader on Old Town is rejected.
        let group_ok = validate_travel_plan(
            &world,
            5,
            5,
            all_ep,
            &stored,
            &TravelRequest::magic_town(
                93,
                TravelParty::HouseholdGroup {
                    leader_guid: 0x1001,
                    member_guids: vec![0x1001, 0x1002],
                },
                TravelTransport::MagicBlimp,
            ),
        )
        .unwrap();
        assert_eq!(group_ok.members.len(), 2);

        assert!(matches!(
            validate_travel_plan(
                &world,
                5,
                5,
                all_ep,
                &stored,
                &TravelRequest::magic_town(
                    93,
                    TravelParty::HouseholdGroup {
                        leader_guid: 0x1001,
                        member_guids: vec![0x1001, 0x1001],
                    },
                    TravelTransport::MagicBlimp,
                ),
            ),
            Err(TravelError::InvalidParty(_))
        ));
        assert!(matches!(
            validate_travel_plan(
                &world,
                5,
                5,
                all_ep,
                &stored,
                &TravelRequest::magic_town(
                    93,
                    TravelParty::HouseholdGroup {
                        leader_guid: 0x1001,
                        member_guids: vec![0x1002, 0x1002],
                    },
                    TravelTransport::MagicBlimp,
                ),
            ),
            Err(TravelError::InvalidParty(_))
        ));
        assert!(matches!(
            validate_travel_plan(
                &world,
                5,
                5,
                all_ep,
                &stored,
                &TravelRequest::old_town(58, 0x1003),
            ),
            Err(TravelError::InvalidParty(_))
        ));

        // Unlimited-budget edit mode (`globals[32] != 0`) cannot depart.
        world.globals[GLOBAL_BUDGET_MODE] = 1;
        assert!(matches!(
            validate_travel_plan(
                &world,
                5,
                5,
                all_ep,
                &stored,
                &TravelRequest::downtown(21, 0x1001),
            ),
            Err(TravelError::SessionMismatch(_))
        ));
        world.globals[GLOBAL_BUDGET_MODE] = 0;
    }

    #[test]
    fn live_install_mutations_reconcile_motives_skills_fame_outfits_magicoins_tokens_and_relations()
    {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let db_path = std::env::temp_dir().join(format!(
            "sims-vm-travel-mutations-{}.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&db_path);
        struct Guard(PathBuf);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _guard = Guard(db_path.clone());

        let store = NeighborhoodStore::open(&db_path).unwrap();
        store
            .import_directory("UserData", &root.join("UserData"), 0xff)
            .unwrap();
        let nh = store.load_neighborhood("UserData").unwrap().unwrap();
        let goth_members = nh.family_in(5).unwrap().1.members.clone();
        let mortimer_guid = goth_members[0];
        let bella_guid = goth_members[1];
        let cassandra_guid = goth_members[2];

        // Pick a non-household adult townie/neighbor belonging to another family
        // (`person_data[61] != 0`) for Downtown Date & family-friend tests.
        let non_goth_adult_guid = nh
            .neighbors
            .iter()
            .flatten()
            .find(|n| {
                !goth_members.contains(&n.guid)
                    && n.person_data.len() > PERSON_KIND
                    && n.person_data[61] != 0
                    && is_adult_human(n.person_data[PERSON_AGE], n.person_data[PERSON_KIND])
            })
            .expect("Complete Collection UserData has non-Goth adult family neighbors")
            .guid;

        let mut goth =
            crate::lifecycle::enter_lot(&root, &store, "UserData", 5, &[3], Frames::default())
                .unwrap();
        for _ in 0..5 {
            goth.step().unwrap();
        }
        let mortimer_nid = goth.world.neighbors.first_of(mortimer_guid).unwrap().id;
        let companion_nid = goth
            .world
            .neighbors
            .first_of(non_goth_adult_guid)
            .unwrap()
            .id;

        // Seed initial MagiCoins (100) on Mortimer before departure.
        set_neighbor_token_count(
            &mut goth.world,
            mortimer_nid,
            MAGICOIN_TOKEN_KIND,
            MAGICOIN_TOKEN_GUID,
            100,
        );

        // 1. Downtown Date (Mortimer + non-household adult companion) to House 21.
        let date_req = TravelRequest::downtown_date(21, mortimer_guid, non_goth_adult_guid);
        let dep = depart_to_destination(&root, &store, "UserData", &mut goth, &date_req).unwrap();
        let mut dt = dep.destination_lot;
        let session = dep.session;
        assert_eq!(session.travelers.len(), 2);
        assert!(session.travelers[0].is_household_member);
        assert!(!session.travelers[1].is_household_member);
        assert_eq!(session.magic_coins_at_arrival, 100);

        // Verify Downtown Date established mutual `person.data[PERSON_DATE]` link!
        let dt_mortimer_id = session.travelers[0].destination_object_id;
        let dt_companion_id = session.travelers[1].destination_object_id;
        assert_eq!(
            dt.world.objects[&dt_mortimer_id]
                .person
                .as_ref()
                .unwrap()
                .data[PERSON_DATE],
            dt_companion_id as i16
        );
        assert_eq!(
            dt.world.objects[&dt_companion_id]
                .person
                .as_ref()
                .unwrap()
                .data[PERSON_DATE],
            dt_mortimer_id as i16
        );

        // Mutate funds, motives, skills, fame, accessories, outfits, made_outfits,
        // MagiCoins, souvenir/gift token, and relationship with the date companion!
        let spent_funds = dt.world.budget.funds - 135;
        dt.world.set_funds(spent_funds).unwrap();
        {
            let person = dt
                .world
                .objects
                .get_mut(&dt_mortimer_id)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            person.motives[5] = 77.5; // Hunger
            person.motives[7] = 66.25; // Energy
            person.data[9] = 600; // Cooking skill
            person.data[10] = 500; // Charisma skill
            person.data[PERSON_FAME_LEVEL] = 25; // 2.5 celebrity stars
            person.accessories.push(Accessory {
                name: "xskin-sunglasses".into(),
                binding: "HEAD".into(),
            });
        }
        let mut custom_outfit = dt
            .world
            .person_outfits
            .get(&dt_mortimer_id)
            .cloned()
            .unwrap_or_default();
        custom_outfit.last_dressed = [2, 1, 0, 0];
        dt.world
            .person_outfits
            .insert(dt_mortimer_id, custom_outfit);
        dt.world
            .made_outfits
            .entry(mortimer_guid)
            .or_default()
            .insert(1, "b300mafitlgt_formal".into());
        set_neighbor_token_count(
            &mut dt.world,
            mortimer_nid,
            MAGICOIN_TOKEN_KIND,
            MAGICOIN_TOKEN_GUID,
            145,
        );
        // Gift/souvenir token (`kind = 0`).
        const GIFT_GUID: u32 = 0xfeed_c0de;
        set_neighbor_token_count(&mut dt.world, mortimer_nid, 0, GIFT_GUID, 2);
        // Raise bidirectional relationship with the Downtown date companion.
        for (from, to, score) in [
            (mortimer_nid, companion_nid, 82),
            (companion_nid, mortimer_nid, 76),
        ] {
            let rec = dt.world.neighbors.get_mut(from).unwrap();
            if let Some(row) = rec
                .relationships
                .iter_mut()
                .find(|r| r.target == i32::from(to))
            {
                if row.values.is_empty() {
                    row.values.push(score);
                } else {
                    row.values[0] = score;
                }
            } else {
                rec.relationships.push(sims_sim::neighbors::Row {
                    target: i32::from(to),
                    values: vec![score, score],
                });
            }
        }

        // Return home from Downtown and verify EVERY mutation reconciled!
        let ret =
            return_from_destination(&root, &store, &session, &mut dt, &[3], Frames::default())
                .unwrap();
        let mut home = ret.home_lot;
        assert_eq!(ret.report.funds_at_return, spent_funds);
        assert_eq!(ret.report.funds_delta, -135);
        assert_eq!(ret.report.magic_coins_at_arrival, 100);
        assert_eq!(ret.report.magic_coins_at_return, 145);
        assert_eq!(ret.report.magic_coins_delta, 45);
        assert_eq!(ret.report.returning_travelers.len(), 1);

        // Non-household companion was NOT spawned on Goth House 5.
        assert!(
            !home
                .world
                .people
                .iter()
                .any(|id| home.world.objects[id].guid == non_goth_adult_guid)
        );
        // Non-traveling Bella and Cassandra remain present.
        assert!(
            home.world
                .people
                .iter()
                .any(|id| home.world.objects[id].guid == bella_guid)
        );
        assert!(
            home.world
                .people
                .iter()
                .any(|id| home.world.objects[id].guid == cassandra_guid)
        );

        let home_mortimer_id = ret.report.returning_travelers[0].1;
        let mort = home.world.objects[&home_mortimer_id]
            .person
            .as_ref()
            .unwrap();
        assert_eq!(mort.motives[5], 77.5);
        assert_eq!(mort.motives[7], 66.25);
        assert_eq!(mort.data[9], 600);
        assert_eq!(mort.data[10], 500);
        assert_eq!(mort.data[PERSON_FAME_LEVEL], 25);
        assert_eq!(mort.data[PERSON_DATE], 0, "date link cleared on return");
        assert!(
            mort.accessories
                .iter()
                .any(|a| a.name == "xskin-sunglasses" && a.binding == "HEAD"),
            "traveler accessories reconciled on return"
        );
        assert_eq!(
            home.world
                .person_outfits
                .get(&home_mortimer_id)
                .unwrap()
                .last_dressed,
            [2, 1, 0, 0]
        );
        assert_eq!(
            home.world.made_outfits[&mortimer_guid][&1],
            "b300mafitlgt_formal"
        );
        assert_eq!(
            neighbor_token_count(
                &home.world,
                mortimer_nid,
                MAGICOIN_TOKEN_KIND,
                MAGICOIN_TOKEN_GUID
            ),
            145
        );
        assert_eq!(
            neighbor_token_count(&home.world, mortimer_nid, 0, GIFT_GUID),
            2
        );
        // Bidirectional relationship persisted to both live world and REDB!
        let live_rel = home
            .world
            .neighbors
            .get(mortimer_nid)
            .unwrap()
            .relationships
            .iter()
            .find(|r| r.target == i32::from(companion_nid))
            .unwrap();
        assert_eq!(live_rel.values[0], 82);
        let stored_after = store.load_neighborhood("UserData").unwrap().unwrap();
        assert_eq!(stored_after.families[&5].secondary_house(), 0);
        assert_eq!(stored_after.families[&5].budget, spent_funds);
        assert!(stored_after.families[&5].friends >= 1);

        // Switching away from famous Mortimer (`fame=25`) via `select_sim`
        // succeeds cleanly without tripping unported star-marker pool `0x446c15`
        // and preserves exact fame!
        let bella_id = home
            .world
            .people
            .iter()
            .copied()
            .find(|id| home.world.objects[id].guid == bella_guid)
            .unwrap();
        select_sim(&mut home, bella_id).unwrap();
        assert_eq!(selected(&home.world), Some(bella_id));
        assert_eq!(
            home.world.objects[&home_mortimer_id]
                .person
                .as_ref()
                .unwrap()
                .data[PERSON_FAME_LEVEL],
            25
        );
        select_sim(&mut home, home_mortimer_id).unwrap();
        assert_eq!(selected(&home.world), Some(home_mortimer_id));

        // Famous Mortimer (fame=25) ticks cleanly at home in strict mode,
        // and now qualifies for a complimentary `$0` Studio Town Limo!
        for _ in 0..20 {
            home.step().unwrap();
        }
        let limo_req = TravelRequest {
            transport: Some(TravelTransport::StudioLimo),
            ..TravelRequest::studio_town(81, mortimer_guid)
        };
        let limo_dep =
            depart_to_destination(&root, &store, "UserData", &mut home, &limo_req).unwrap();
        assert_eq!(limo_dep.session.fare_paid, STUDIO_LIMO_FARE);
        assert_eq!(limo_dep.destination_lot.world.budget.funds, spent_funds);
        let mut st = limo_dep.destination_lot;
        let ret_st = return_from_destination(
            &root,
            &store,
            &limo_dep.session,
            &mut st,
            &[3],
            Frames::default(),
        )
        .unwrap();
        let mut home_final = ret_st.home_lot;
        for _ in 0..20 {
            home_final.step().unwrap();
        }
    }

    #[test]
    fn three_way_merge_preserves_unrelated_writes_and_rejects_conflicts() {
        let (mut dest_world, mut base_stored) = fixture_world_and_nh();
        // Seed StoredNeighborhood with matching NBRS, inventories, and TATT.
        base_stored.neighbors = vec![
            Some(sims_formats::neighborhood::Neighbor {
                version: 10,
                version_extra: Some(1),
                name: "Sim1".into(),
                unknown_zero: 0,
                person_mode: 9,
                person_data: dest_world.neighbors.get(1).unwrap().data.clone(),
                id: 1,
                guid: 0x1001,
                unknown_negative: -1,
                relationships: Vec::new(),
            }),
            Some(sims_formats::neighborhood::Neighbor {
                version: 10,
                version_extra: Some(1),
                name: "Sim2".into(),
                unknown_zero: 0,
                person_mode: 9,
                person_data: dest_world.neighbors.get(2).unwrap().data.clone(),
                id: 2,
                guid: 0x1002,
                unknown_negative: -1,
                relationships: Vec::new(),
            }),
            Some(sims_formats::neighborhood::Neighbor {
                version: 10,
                version_extra: Some(1),
                name: "Pet3".into(),
                unknown_zero: 0,
                person_mode: 9,
                person_data: dest_world.neighbors.get(3).unwrap().data.clone(),
                id: 3,
                guid: 0x1003,
                unknown_negative: -1,
                relationships: Vec::new(),
            }),
            Some(sims_formats::neighborhood::Neighbor {
                version: 10,
                version_extra: Some(1),
                name: "Townie4".into(),
                unknown_zero: 0,
                person_mode: 9,
                person_data: dest_world.neighbors.get(4).unwrap().data.clone(),
                id: 4,
                guid: 0x2001,
                unknown_negative: -1,
                relationships: Vec::new(),
            }),
        ];
        base_stored.inventories = vec![
            Some(sims_formats::neighborhood::Inventory {
                neighbor: 1,
                items: vec![sims_formats::neighborhood::InventoryItem {
                    kind: MAGICOIN_TOKEN_KIND,
                    guid: MAGICOIN_TOKEN_GUID,
                    count: 125,
                }],
            }),
            Some(sims_formats::neighborhood::Inventory {
                neighbor: 2,
                items: vec![sims_formats::neighborhood::InventoryItem {
                    kind: MAGICOIN_TOKEN_KIND,
                    guid: MAGICOIN_TOKEN_GUID,
                    count: 25,
                }],
            }),
        ];
        base_stored.type_attributes = Some(sims_formats::tatt::Tatt {
            version: 0,
            packed: true,
            entries: vec![sims_formats::tatt::Entry {
                guid: 0xaaaa_0001,
                values: vec![1, 2],
                count_end: 0,
                end: 0,
            }],
            count_end: 0,
            consumed: 0,
        });
        base_stored
            .families
            .get_mut(&5)
            .unwrap()
            .set_secondary_house(21);
        base_stored.revision = 10;

        let baseline = DepartureBaseline::from_stored(&base_stored, 10, 5);
        let session = TravelSession {
            store_id: 1,
            instance_id: 1,
            trip_id: 42,
            neighborhood_id: "UserData".into(),
            home_house: 5,
            home_family: 5,
            destination_house: 21,
            destination: TravelDestination::Downtown,
            transport: TravelTransport::DowntownCab,
            accommodation: VacationAccommodation::None,
            fare_paid: 50,
            departure_ticks: 0,
            departure_world_ticks: 0,
            departure_clock: (8, 0, 0),
            funds_before_fare: 2_050,
            funds_at_arrival: 2_000,
            magic_coins_at_arrival: 150,
            leader_guid: 0x1001,
            travelers: vec![TravelerRecord {
                guid: 0x1001,
                neighbor_id: 1,
                is_household_member: true,
                home_object_id: Some(1),
                destination_object_id: 1,
                had_hotel_key_before_trip: false,
            }],
        };

        // Destination mutates Sim 1's cooking skill, relationship with 4, and inventory.
        dest_world.neighbors.get_mut(1).unwrap().data[9] = 700;
        dest_world
            .neighbors
            .get_mut(1)
            .unwrap()
            .relationships
            .push(sims_sim::neighbors::Row {
                target: 4,
                values: vec![80, 80],
            });
        set_neighbor_token_count(&mut dest_world, 1, 0, 0xfeed_0001, 1);

        // Concurrent writer advances revision to 11 and edits Townie 4's skill,
        // Sim 2's inventory, Sim 1's relationship with Sim 2 (disjoint target!),
        // and a TATT entry that destination did not change.
        let mut latest = base_stored.clone();
        latest.revision = 11;
        latest.neighbors[3].as_mut().unwrap().person_data[9] = 450;
        latest.neighbors[0].as_mut().unwrap().relationships.push(
            sims_formats::neighborhood::Relationship {
                target: 2,
                values: vec![95, 95],
            },
        );
        latest.inventories[1].as_mut().unwrap().items[0].count = 99;
        latest.type_attributes.as_mut().unwrap().entries[0].values = vec![9, 9];

        // 1. Disjoint concurrent edits merge cleanly with ZERO overwrites!
        let merged = merge_destination_into_latest_neighborhood(
            &baseline,
            &latest,
            &dest_world,
            &session,
            1_850,
            50,
        )
        .unwrap();
        assert_eq!(merged.neighbors[0].as_ref().unwrap().person_data[9], 700);
        assert_eq!(
            merged.neighbors[3].as_ref().unwrap().person_data[9],
            450,
            "concurrent Townie 4 edit preserved"
        );
        let sim1_rels = relationship_map(&merged.neighbors[0].as_ref().unwrap().relationships);
        assert_eq!(sim1_rels.get(&4), Some(&vec![80, 80]));
        assert_eq!(
            sim1_rels.get(&2),
            Some(&vec![95, 95]),
            "disjoint concurrent relationship row preserved"
        );
        let invs = inventory_map(&merged.inventories);
        assert_eq!(
            invs[&2][0].count, 99,
            "concurrent Sim 2 inventory preserved"
        );
        assert!(invs[&1].iter().any(|it| it.guid == 0xfeed_0001));
        assert_eq!(
            merged.type_attributes.as_ref().unwrap().entries[0].values,
            vec![9, 9],
            "concurrent TATT edit preserved"
        );
        assert_eq!(merged.families[&5].secondary_house(), 0);
        assert_eq!(merged.families[&5].budget, 1_850);

        // 2. Same-Sim core `person_data` conflict is rejected (never silently lost).
        let mut core_conflict = latest.clone();
        core_conflict.neighbors[0].as_mut().unwrap().person_data[9] = 333;
        assert!(matches!(
            merge_destination_into_latest_neighborhood(
                &baseline,
                &core_conflict,
                &dest_world,
                &session,
                1_850,
                50,
            ),
            Err(TravelError::StaleNeighborhood { .. })
        ));

        // 3. Same-inventory conflict is rejected.
        let mut inv_conflict = latest.clone();
        inv_conflict.inventories[0].as_mut().unwrap().items[0].count = 500;
        assert!(matches!(
            merge_destination_into_latest_neighborhood(
                &baseline,
                &inv_conflict,
                &dest_world,
                &session,
                1_850,
                50,
            ),
            Err(TravelError::StaleNeighborhood { .. })
        ));

        // 4. Newly created character neighbor-slot collision is rejected.
        let mut slot_dest_world = dest_world.clone();
        slot_dest_world
            .neighbors
            .slots
            .push(Some(make_neighbor(5, 0x9001, 27, 0, 0)));
        let mut slot_conflict = latest.clone();
        slot_conflict
            .neighbors
            .push(Some(sims_formats::neighborhood::Neighbor {
                version: 10,
                version_extra: Some(1),
                name: "ConcurrentNewSim".into(),
                unknown_zero: 0,
                person_mode: 9,
                person_data: vec![0; PERSON_WORDS],
                id: 5,
                guid: 0x9002,
                unknown_negative: -1,
                relationships: Vec::new(),
            }));
        assert!(matches!(
            merge_destination_into_latest_neighborhood(
                &baseline,
                &slot_conflict,
                &slot_dest_world,
                &session,
                1_850,
                50,
            ),
            Err(TravelError::StaleNeighborhood { .. })
        ));
    }

    #[test]
    fn deleted_frozen_home_visitor_is_purged_on_return_and_abort() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let db_path = std::env::temp_dir().join(format!(
            "sims-vm-travel-deleted-visitor-{}.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&db_path);
        struct Guard(PathBuf);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _guard = Guard(db_path.clone());

        let store = NeighborhoodStore::open(&db_path).unwrap();
        store
            .import_directory("UserData", &root.join("UserData"), 0xff)
            .unwrap();
        let nh = store.load_neighborhood("UserData").unwrap().unwrap();
        let goth_members = nh.family_in(5).unwrap().1.members.clone();
        let mortimer_guid = goth_members[0];
        let newbie_guid = nh.family_in(7).unwrap().1.members[0];

        // Enter Goth House 5 and inject a visiting Newbie Sim onto the home lot
        // before departure so the frozen home snapshot contains that visitor.
        let mut goth =
            crate::lifecycle::enter_lot(&root, &store, "UserData", 5, &[3], Frames::default())
                .unwrap();
        {
            let interp = Interpreter::new(&goth.program, &goth.catalog, Stream::Global);
            let visitor_id = interp.create_person(&mut goth.world, newbie_guid).unwrap();
            let anchor = find_arrival_anchor(&goth.world, None);
            let pos = find_free_arrival_position(
                &interp,
                &mut goth.world,
                visitor_id,
                anchor,
                &BTreeSet::new(),
            )
            .unwrap();
            place_person_on_tile(&interp, &mut goth.world, visitor_id, pos).unwrap();
        }
        assert!(
            goth.world
                .people
                .iter()
                .any(|id| goth.world.objects[id].guid == newbie_guid)
        );

        // Depart Mortimer to Downtown House 21 (freezing House 5 WITH the visitor).
        let dep = depart_to_destination(
            &root,
            &store,
            "UserData",
            &mut goth,
            &TravelRequest::downtown(21, mortimer_guid),
        )
        .unwrap();
        let mut dt = dep.destination_lot;

        // While Goth is away, delete the visitor's neighbor record in REDB
        // (simulating bulldozing a lot that owned a tombstone visitor).
        let mut nh_now = store.load_neighborhood("UserData").unwrap().unwrap();
        nh_now.deleted_characters.insert(newbie_guid);
        nh_now.deletion_epoch += 1;
        nh_now
            .deleted_at_epoch
            .insert(newbie_guid, nh_now.deletion_epoch);
        for slot in &mut nh_now.neighbors {
            if slot.as_ref().is_some_and(|n| n.guid == newbie_guid) {
                *slot = None;
            }
        }
        store.save_neighborhood("UserData", &nh_now).unwrap();

        // Return home MUST purge the deleted visitor cleanly instead of failing
        // in `attach_neighbor`!
        let ret = return_from_destination(
            &root,
            &store,
            &dep.session,
            &mut dt,
            &[3],
            Frames::default(),
        )
        .unwrap();
        let mut home_after = ret.home_lot;
        assert!(
            !home_after
                .world
                .people
                .iter()
                .any(|id| home_after.world.objects[id].guid == newbie_guid),
            "deleted visitor was purged during return reconciliation"
        );
        for _ in 0..20 {
            home_after.step().unwrap();
        }
    }
}
