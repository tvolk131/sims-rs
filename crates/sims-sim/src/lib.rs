//! The simulation state of one lot.
//!
//! The original keeps one lot for as long as it runs and loads each house
//! into it, and [`World`] is that lot. [`World::empty`] is the lot before
//! any load, and [`World::read_simulator`] and [`World::read_arrays`] read
//! parts of a house file into it. The saved objects are read by `sims-vm`,
//! which holds the object definitions the original constructs them from.
//! [`World::from_house`] reads a whole saved house at once: every
//! object with its attributes, data and script stack, the scheduler cursor,
//! both random streams, the budget and the lot arrays. [`World::hash`]
//! fingerprints the state, and [`Snapshots`] stores it in a REDB file.
//! [`load`] holds the load steps that change state without running scripts,
//! and [`rooms`] works out the lot's rooms from its walls and counts
//! them: their size, what stands in them, their light and their score.
//! [`route`] holds the people's routes. [`motives`] decays a person's motives and works out the mood.
//! [`grass`] grows the lawn. [`neighbors`] holds the neighborhood's people, and [`animator`] what
//! each person's animator keeps.
//!
//! The state holds every field the readers decode. Bytes the readers keep
//! but do not understand (format tails, raw records) are left out: they are
//! not state anything here can act on.

pub mod album_store;
pub mod animator;
pub mod autosave;
pub mod characters;
mod cow;
mod error;
pub mod grass;
pub mod hash_flavor;
pub mod head;
pub mod load;
pub mod migration;
pub mod motives;
pub mod neighborhood_save;
pub mod neighborhood_store;
pub mod neighbors;
pub mod patch_import;
pub mod pose;
pub mod rooms;
pub mod route;
mod snapshot;
pub mod speed;
mod world;

pub use album_store::{
    ALBUM_FORMAT, ALBUM_MIN_FORMAT, AlbumError, AlbumFamilyToken, AlbumImage, AlbumPhoto,
    AlbumPhotoSummary, MAX_ALBUM_CAPTION_CHARS, MAX_ALBUM_ENTRIES, MAX_ALBUM_INDEX_ENCODED_BYTES,
    MAX_ALBUM_PHOTO_DIM, MAX_ALBUM_PHOTO_ENCODED_BYTES, MAX_ALBUM_PHOTO_PIXELS,
    MAX_ALBUM_TITLE_CHARS, MAX_BULK_ALBUM_PIXEL_BYTES, NewAlbumPhoto, PhotoAlbumStore,
};
pub use autosave::{
    AUTOSAVE_FORMAT, AUTOSAVE_MADE_ORDER_FORMAT, AUTOSAVE_MIN_FORMAT, AutosavePolicy,
    AutosaveStore, DEFAULT_MAX_AUTOSAVES, LoadedSave, RecoveryLoad, SaveError, SaveJob,
    SaveMetadata, SavePayload, SavePayloadKind, SaveSlot, SaveSlotEntry, SaveTarget,
};
pub use cow::{CowBox, CowKey, CowMap};
pub use error::Error;
pub use hash_flavor::{Sha256Flavor, hash_postcard};
pub use migration::{
    MigratedBytes, MigratedRecord, MigrationError, MigrationRegistry, MigrationReport,
    MigrationStep, NEIGHBORHOOD_MIGRATIONS, NEIGHBORHOOD_MIN_FORMAT, NeighborhoodSchemaStatus,
    StoreMigrationError, StoreMigrationReport, StoredNeighborhoodV1, StoredNeighborhoodV2,
    StoredNeighborhoodV3, decode_postcard_exact,
};
pub use neighborhood_store::{
    CheckedCommitError, DEFAULT_ARCH_VALUE_MULTIPLIER, DEFAULT_LAYOUT_HISTORY_FILL,
    DEFAULT_LOT_THUMBNAILS_MAX_BYTES, DEFAULT_NEW_FAMILY_START_HOUR, ExportSnapshot,
    FAMILY_FLAG_BIT_3, FAMILY_FLAG_HAS_CATEGORY_1, FAMILY_FLAG_HAS_CATEGORY_9,
    FAMILY_FLAG_NEW_MOVE_IN, ImportSummary, LOT_THUMBNAIL_FORMAT, LOT_THUMBNAIL_MIN_FORMAT,
    LotSnapshot, MAX_LOT_THUMBNAIL_DIM, MAX_LOT_THUMBNAIL_ENCODED_BYTES, MAX_LOT_THUMBNAIL_PIXELS,
    NEIGHBORHOOD_FORMAT, NeighborhoodStore, StoredLotThumbnail, StoredNeighborhood,
    ThumbnailSurface565, family_net_worth, lot_total_value,
};
pub use snapshot::{FORMAT as SNAPSHOT_FORMAT, ReadOnlySnapshots, Snapshots};
pub use world::{
    ADVERTISEMENTS, Advertisement, BoundSlot, Budget, Dialog, Dialogs, FULL_SIZE, FrameStorage,
    Headline, HouseUsage, IndicatorQueues, MagicHistory, Object, PersonOutfit, PersonTick,
    PortraitFile, SCRATCH_PTR_ARG_SLOT, SCRATCH_PTR_EXPRESSION, SCRATCH_PTR_SET,
    SCRATCH_PTR_SET_TO_NEXT, ScratchPtrBitSerialize, ScratchPtrs, TripRequest, TypeAttributes,
    Usage, Whereabouts, WorkwearCache, World,
};

pub type Result<T> = std::result::Result<T, Error>;
