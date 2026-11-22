//! REDB-backed persistent Household Photo Album / Storybook storage.
//!
//! Stores per-`(neighborhood_id, family_id)` photo album entries inside the
//! same REDB database file as [`crate::NeighborhoodStore`] (via
//! [`crate::NeighborhoodStore::albums`]), matching `sims-ui::album`'s view
//! model and commands (`add`, `update_text`, `move_entry`, `delete`, `list`,
//! plus metadata-only [`PhotoAlbumStore::list_summaries`] and paged
//! [`PhotoAlbumStore::list_page`]).
//!
//! ## Integrity, Ordering, Bounds & Lifetime Guarantees
//! - Every index row (`photo_album_index`) and photo row (`photo_album_photos`)
//!   is framed with a 4-byte magic (`"ALBI"` / `"ALBP"`), 4-byte little-endian
//!   format version (`ALBUM_FORMAT = 1`), 32-byte domain-separated SHA-256
//!   checksum, strict exact-length `postcard` body decoding, and key-binding
//!   verification (`neighborhood_id`, `family_id`, and `photo_id`).
//! - Encoded record-size ceilings are enforced **before** hashing or decoding,
//!   and string/vector lengths (including RGB565 pixel counts against declared
//!   `width × height`) are bounded **during** deserialization before allocation.
//! - Photo IDs are strictly monotonic (`>= 1`) per household lifetime and are
//!   never reused after deletion.
//! - Ordering is stored explicitly in the per-family index row (`order: Vec<u64>`),
//!   making [`PhotoAlbumStore::move_entry`] atomic and lightweight without
//!   rewriting photo bitmaps, while [`PhotoAlbumStore::list`],
//!   [`PhotoAlbumStore::list_summaries`], and [`PhotoAlbumStore::list_page`]
//!   execute in one consistent MVCC read transaction. Bulk pixel reads enforce
//!   [`MAX_BULK_ALBUM_PIXEL_BYTES`] so valid full albums cannot exhaust memory.
//! - Every mutating operation requires an [`AlbumFamilyToken`] obtained via
//!   [`PhotoAlbumStore::family_token`], verified inside the REDB write
//!   transaction against both `NEIGHBORHOODS[neighborhood_id].instance_id`,
//!   live membership in `StoredNeighborhood::families`, and the per-family
//!   monotonic `photo_album_family_epoch` counter (bumped atomically whenever
//!   a household is removed from `StoredNeighborhood::families` in any
//!   neighborhood save). Deleting or replacing a household immediately
//!   invalidates all stale tokens even if a replacement household later reuses
//!   the same `family_id` and photo ID `1`.
//! - [`crate::NeighborhoodStore::delete_neighborhood`] and
//!   [`crate::NeighborhoodStore::import_directory`] atomically purge all album
//!   indices, photos, and family-epoch rows for that neighborhood inside their
//!   write transaction.

use crate::Error;
use crate::migration::decode_postcard_exact;
use crate::neighborhood_store::{NEIGHBORHOODS as NEIGHBORHOODS_TABLE, StoredNeighborhood};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

/// 4-byte magic prefix for per-family photo album index rows.
const INDEX_MAGIC: [u8; 4] = *b"ALBI";

/// 4-byte magic prefix for individual photo album entry rows.
const PHOTO_MAGIC: [u8; 4] = *b"ALBP";

/// Domain separator for index row outer SHA-256 checksums.
const INDEX_HASH_DOMAIN: &[u8] = b"sims-sim:album-index:v1";

/// Domain separator for photo entry outer SHA-256 checksums.
const PHOTO_HASH_DOMAIN: &[u8] = b"sims-sim:album-photo:v1";

/// Length of the fixed envelope prefix: 4-byte magic + 4-byte version +
/// 32-byte SHA-256 checksum.
const HEADER_LEN: usize = 4 + 4 + 32;

/// Maximum UTF-8 byte length accepted for a neighborhood ID in album records.
const MAX_NEIGHBORHOOD_ID_BYTES: usize = 1024;

/// Earliest supported photo album record format version.
pub const ALBUM_MIN_FORMAT: u32 = 1;

/// Current photo album record format version.
pub const ALBUM_FORMAT: u32 = 1;

/// Maximum character length of a photo title (matching `sims-ui::album::MAX_TITLE_LEN`).
pub const MAX_ALBUM_TITLE_CHARS: usize = 80;

/// Maximum character length of a photo caption (matching `sims-ui::album::MAX_CAPTION_LEN`).
pub const MAX_ALBUM_CAPTION_CHARS: usize = 512;

/// Maximum width or height (in pixels) of a stored photo bitmap.
pub const MAX_ALBUM_PHOTO_DIM: u32 = 1024;

/// Maximum pixel count of a single stored photo bitmap (`1024 × 768`).
pub const MAX_ALBUM_PHOTO_PIXELS: usize = 1024 * 768;

/// Maximum number of entries retained in one `(neighborhood_id, family_id)` album.
pub const MAX_ALBUM_ENTRIES: usize = 256;

/// Aggregate decoded RGB565 pixel-buffer memory budget enforced by bulk album
/// reads (`list` / `list_page`, 32 MiB). Callers with larger albums should use
/// [`PhotoAlbumStore::list_summaries`] paired with [`PhotoAlbumStore::list_page`]
/// or [`PhotoAlbumStore::get`].
pub const MAX_BULK_ALBUM_PIXEL_BYTES: usize = 32 * 1024 * 1024;

/// Maximum encoded byte length accepted for an `ALBUM_INDEX` row before hashing
/// or deserializing (header + neighborhood_id + up to 256 varint u64 IDs + slack).
pub const MAX_ALBUM_INDEX_ENCODED_BYTES: usize = 16 * 1024;

/// Maximum encoded byte length accepted for an `ALBUM_PHOTOS` row before
/// hashing or deserializing (header + text/metadata + up to `MAX_ALBUM_PHOTO_PIXELS`
/// varint-encoded `u16`s at <= 3 bytes each).
pub const MAX_ALBUM_PHOTO_ENCODED_BYTES: usize = HEADER_LEN + 8 * 1024 + 3 * MAX_ALBUM_PHOTO_PIXELS;

/// Per-household album index (`next_photo_id` + ordered `photo_id` list) keyed
/// by `(neighborhood_id, family_id)`.
const ALBUM_INDEX: TableDefinition<(&str, u16), &[u8]> = TableDefinition::new("photo_album_index");

/// Individual photo album entries keyed by `(neighborhood_id, family_id, photo_id)`.
const ALBUM_PHOTOS: TableDefinition<(&str, u16, u64), &[u8]> =
    TableDefinition::new("photo_album_photos");

/// Monotonic per-`(neighborhood_id, family_id)` household lifetime epoch
/// (defaults to `1`, incremented atomically whenever `family_id` is removed
/// from `StoredNeighborhood::families`) so stale album mutations cannot target
/// a replacement household reusing the same `family_id`.
const ALBUM_FAMILY_EPOCH: TableDefinition<(&str, u16), u64> =
    TableDefinition::new("photo_album_family_epoch");

/// Initial household lifetime epoch assigned when no prior deletions have
/// occurred for `(neighborhood_id, family_id)`.
const INITIAL_FAMILY_EPOCH: u64 = 1;

/// Transactional lifetime token binding a household photo album mutation to
/// both the current neighborhood instance (`instance_id`) and the current
/// household lifetime (`family_id`, `family_epoch`).
///
/// Obtained via [`PhotoAlbumStore::family_token`]. Deleting or re-importing the
/// neighborhood, or deleting the family from `StoredNeighborhood::families`,
/// atomically invalidates every previously issued token for that household even
/// if a new household later reuses the same `family_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AlbumFamilyToken {
    pub instance_id: u128,
    pub family_id: u16,
    pub family_epoch: u64,
}

/// Bounded 16-bit RGB565 photo bitmap stored inside an [`AlbumPhoto`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlbumImage {
    pub width: u32,
    pub height: u32,
    /// Row-major top-down 16-bit RGB565 pixels (`len == width * height`).
    pub rgb565: Vec<u16>,
}

struct ExactU16VecSeed {
    expected: usize,
}

impl<'de> serde::de::DeserializeSeed<'de> for ExactU16VecSeed {
    type Value = Vec<u16>;

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Self::Value, D::Error> {
        struct ExactU16VecVisitor {
            expected: usize,
        }

        impl<'de> serde::de::Visitor<'de> for ExactU16VecVisitor {
            type Value = Vec<u16>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "a sequence of exactly {} RGB565 u16 pixels",
                    self.expected
                )
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                if let Some(hint) = seq.size_hint()
                    && hint != self.expected
                {
                    return Err(serde::de::Error::custom(format!(
                        "photo rgb565 pixel count {hint} does not match expected {}",
                        self.expected
                    )));
                }
                let mut out = Vec::with_capacity(self.expected);
                for i in 0..self.expected {
                    let Some(px) = seq.next_element::<u16>()? else {
                        return Err(serde::de::Error::invalid_length(i, &self));
                    };
                    out.push(px);
                }
                if seq.next_element::<u16>()?.is_some() {
                    return Err(serde::de::Error::custom(format!(
                        "photo rgb565 pixel count exceeds expected {}",
                        self.expected
                    )));
                }
                Ok(out)
            }
        }

        deserializer.deserialize_seq(ExactU16VecVisitor {
            expected: self.expected,
        })
    }
}

impl<'de> Deserialize<'de> for AlbumImage {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct AlbumImageVisitor;

        impl<'de> serde::de::Visitor<'de> for AlbumImageVisitor {
            type Value = AlbumImage;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("AlbumImage { width, height, rgb565 }")
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let width: u32 = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
                let height: u32 = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
                let expected =
                    checked_photo_pixel_count(width, height).map_err(serde::de::Error::custom)?;
                let rgb565 = seq
                    .next_element_seed(ExactU16VecSeed { expected })?
                    .ok_or_else(|| serde::de::Error::invalid_length(2, &self))?;
                Ok(AlbumImage {
                    width,
                    height,
                    rgb565,
                })
            }
        }

        deserializer.deserialize_struct(
            "AlbumImage",
            &["width", "height", "rgb565"],
            AlbumImageVisitor,
        )
    }
}

impl AlbumImage {
    /// Constructs a validated RGB565 photo bitmap.
    pub fn new(width: u32, height: u32, rgb565: Vec<u16>) -> std::result::Result<Self, AlbumError> {
        let image = Self {
            width,
            height,
            rgb565,
        };
        image.validate()?;
        Ok(image)
    }

    /// Converts a top-down RGBA8 buffer (`width * height * 4` bytes) into a
    /// validated RGB565 [`AlbumImage`].
    pub fn from_rgba(
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> std::result::Result<Self, AlbumError> {
        let expected_pixels = checked_photo_pixel_count(width, height)?;
        let expected_bytes = expected_pixels
            .checked_mul(4)
            .ok_or_else(|| AlbumError::InvalidInput("photo RGBA byte length overflows".into()))?;
        if rgba.len() != expected_bytes {
            return Err(AlbumError::InvalidInput(format!(
                "photo RGBA buffer len {} does not match {width}x{height}x4 ({expected_bytes})",
                rgba.len()
            )));
        }
        let mut rgb565 = Vec::with_capacity(expected_pixels);
        for px in rgba.as_chunks::<4>().0 {
            rgb565.push(pack_rgb565(px[0], px[1], px[2]));
        }
        Ok(Self {
            width,
            height,
            rgb565,
        })
    }

    /// Expands this RGB565 photo bitmap into an opaque top-down RGBA8 buffer
    /// (`width * height * 4` bytes).
    #[must_use]
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.rgb565.len().saturating_mul(4));
        for &pixel in &self.rgb565 {
            let [r, g, b] = unpack_rgb565(pixel);
            out.extend_from_slice(&[r, g, b, 255]);
        }
        out
    }

    /// Size of the decoded RGB565 pixel buffer in bytes (`rgb565.len() * 2`).
    #[must_use]
    pub fn pixel_bytes(&self) -> usize {
        self.rgb565.len().saturating_mul(2)
    }

    /// Validates non-zero bounded dimensions and exact `rgb565` pixel count.
    pub fn validate(&self) -> std::result::Result<(), AlbumError> {
        let expected = checked_photo_pixel_count(self.width, self.height)?;
        if self.rgb565.len() != expected {
            return Err(AlbumError::InvalidInput(format!(
                "photo rgb565 pixel count {} does not match {}x{} ({expected})",
                self.rgb565.len(),
                self.width,
                self.height
            )));
        }
        Ok(())
    }
}

/// Packs 8-bit `(r, g, b)` into 16-bit RGB565 (`R5 G6 B5`).
#[inline]
const fn pack_rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0xf8) << 8) | ((g as u16 & 0xfc) << 3) | ((b as u16) >> 3)
}

/// Expands 16-bit RGB565 into 8-bit `[r, g, b]` via high-bit replication.
#[inline]
const fn unpack_rgb565(pixel: u16) -> [u8; 3] {
    let r5 = ((pixel >> 11) & 0x1f) as u8;
    let g6 = ((pixel >> 5) & 0x3f) as u8;
    let b5 = (pixel & 0x1f) as u8;
    [
        (r5 << 3) | (r5 >> 2),
        (g6 << 2) | (g6 >> 4),
        (b5 << 3) | (b5 >> 2),
    ]
}

fn checked_photo_pixel_count(width: u32, height: u32) -> std::result::Result<usize, AlbumError> {
    if width == 0 || height == 0 {
        return Err(AlbumError::InvalidInput(
            "photo dimensions must be non-zero".into(),
        ));
    }
    if width > MAX_ALBUM_PHOTO_DIM || height > MAX_ALBUM_PHOTO_DIM {
        return Err(AlbumError::InvalidInput(format!(
            "photo dimensions {width}x{height} exceed maximum {MAX_ALBUM_PHOTO_DIM}x{MAX_ALBUM_PHOTO_DIM}"
        )));
    }
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| AlbumError::InvalidInput("photo pixel count overflows".into()))?;
    if pixels > MAX_ALBUM_PHOTO_PIXELS {
        return Err(AlbumError::InvalidInput(format!(
            "photo pixel count {pixels} exceeds maximum {MAX_ALBUM_PHOTO_PIXELS}"
        )));
    }
    Ok(pixels)
}

/// Input payload for appending a new page/photo to a household's album via
/// [`PhotoAlbumStore::add`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewAlbumPhoto {
    pub title: String,
    pub caption: String,
    /// Optional in-game day number when the photo was captured.
    pub sim_day: Option<u32>,
    /// Optional photo bitmap (`None` for a blank storybook page).
    pub image: Option<AlbumImage>,
}

impl NewAlbumPhoto {
    /// Creates a blank storybook page with no image bitmap.
    #[must_use]
    pub fn blank(title: impl Into<String>, caption: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            caption: caption.into(),
            sim_day: None,
            image: None,
        }
    }

    /// Creates a photo album entry carrying a snapshot bitmap.
    #[must_use]
    pub fn with_image(
        title: impl Into<String>,
        caption: impl Into<String>,
        image: AlbumImage,
    ) -> Self {
        Self {
            title: title.into(),
            caption: caption.into(),
            sim_day: None,
            image: Some(image),
        }
    }

    /// Attaches an in-game day number to this entry.
    #[must_use]
    pub const fn with_sim_day(mut self, sim_day: u32) -> Self {
        self.sim_day = Some(sim_day);
        self
    }
}

/// Persisted photo / storybook page returned by [`PhotoAlbumStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumPhoto {
    pub id: u64,
    pub title: String,
    pub caption: String,
    pub sim_day: Option<u32>,
    pub image: Option<AlbumImage>,
}

/// Lightweight metadata-only view of one photo album entry returned by
/// [`PhotoAlbumStore::list_summaries`] without retaining pixel buffers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumPhotoSummary {
    pub id: u64,
    pub title: String,
    pub caption: String,
    pub sim_day: Option<u32>,
    pub image_dimensions: Option<(u32, u32)>,
}

/// Typed errors returned by [`PhotoAlbumStore`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlbumError {
    /// Caller supplied invalid input (empty neighborhood ID, zero IDs, oversized
    /// text, invalid bitmap dimensions/length).
    InvalidInput(String),
    /// Household album has reached [`MAX_ALBUM_ENTRIES`].
    AlbumFull {
        neighborhood_id: String,
        family_id: u16,
        max_entries: usize,
    },
    /// Requested photo ID does not exist in `(neighborhood_id, family_id)`.
    PhotoNotFound {
        neighborhood_id: String,
        family_id: u16,
        photo_id: u64,
    },
    /// Requested reorder destination index is out of bounds (`index >= len`).
    InvalidIndex { index: usize, len: usize },
    /// Neighborhood was deleted or replaced by a different instance before the
    /// write transaction committed.
    StaleNeighborhoodInstance {
        neighborhood_id: String,
        expected_instance_id: u128,
        current_instance_id: Option<u128>,
    },
    /// Household was deleted or replaced by a new household reusing `family_id`
    /// before the write transaction committed.
    StaleFamilyToken {
        neighborhood_id: String,
        family_id: u16,
        expected_epoch: u64,
        current_epoch: Option<u64>,
    },
    /// A bulk pixel read exceeded [`MAX_BULK_ALBUM_PIXEL_BYTES`]; callers should
    /// use [`PhotoAlbumStore::list_summaries`] + [`PhotoAlbumStore::list_page`]
    /// or [`PhotoAlbumStore::get`].
    BudgetExceeded {
        record: String,
        limit_bytes: usize,
        required_bytes: usize,
    },
    /// Outer SHA-256 verification failed on an album index or photo record.
    CorruptChecksum {
        record: String,
        expected: [u8; 32],
        actual: [u8; 32],
    },
    /// Record was written by an unsupported or newer format version.
    UnsupportedVersion {
        record: String,
        found: u32,
        min_supported: u32,
        max_supported: u32,
    },
    /// Record framing, key binding, postcard body, or structural invariants failed.
    DecodeFailed { record: String, detail: String },
    /// Underlying REDB storage or transaction error.
    Store(String),
}

impl fmt::Display for AlbumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(msg) => write!(f, "invalid photo album input: {msg}"),
            Self::AlbumFull {
                neighborhood_id,
                family_id,
                max_entries,
            } => write!(
                f,
                "photo album for neighborhood {neighborhood_id:?} family {family_id} is full \
                 (maximum {max_entries} entries)"
            ),
            Self::PhotoNotFound {
                neighborhood_id,
                family_id,
                photo_id,
            } => write!(
                f,
                "photo {photo_id} not found in neighborhood {neighborhood_id:?} family {family_id}"
            ),
            Self::InvalidIndex { index, len } => {
                write!(
                    f,
                    "album move index {index} out of bounds for {len} entries"
                )
            }
            Self::StaleNeighborhoodInstance {
                neighborhood_id,
                expected_instance_id,
                current_instance_id: Some(current),
            } => write!(
                f,
                "stale neighborhood instance for {neighborhood_id:?}: expected \
                 {expected_instance_id:#034x}, current {current:#034x}"
            ),
            Self::StaleNeighborhoodInstance {
                neighborhood_id,
                expected_instance_id,
                current_instance_id: None,
            } => write!(
                f,
                "stale neighborhood instance for {neighborhood_id:?}: expected \
                 {expected_instance_id:#034x}, neighborhood no longer exists"
            ),
            Self::StaleFamilyToken {
                neighborhood_id,
                family_id,
                expected_epoch,
                current_epoch: Some(current),
            } => write!(
                f,
                "stale household token for neighborhood {neighborhood_id:?} family {family_id}: \
                 expected epoch {expected_epoch}, current epoch {current}"
            ),
            Self::StaleFamilyToken {
                neighborhood_id,
                family_id,
                expected_epoch,
                current_epoch: None,
            } => write!(
                f,
                "stale household token for neighborhood {neighborhood_id:?} family {family_id}: \
                 expected epoch {expected_epoch}, household no longer exists"
            ),
            Self::BudgetExceeded {
                record,
                limit_bytes,
                required_bytes,
            } => write!(
                f,
                "{record}: bulk album read requires at least {required_bytes} pixel bytes, \
                 exceeding limit {limit_bytes}; use list_summaries + list_page/get"
            ),
            Self::CorruptChecksum { record, .. } => {
                write!(f, "{record}: corrupt photo album checksum")
            }
            Self::UnsupportedVersion {
                record,
                found,
                min_supported,
                max_supported,
            } => write!(
                f,
                "{record}: unsupported photo album format {found} \
                 (supported {min_supported}..={max_supported})"
            ),
            Self::DecodeFailed { record, detail } => {
                write!(f, "{record}: failed to decode photo album record: {detail}")
            }
            Self::Store(msg) => write!(f, "photo album store error: {msg}"),
        }
    }
}

impl std::error::Error for AlbumError {}

impl From<AlbumError> for Error {
    fn from(error: AlbumError) -> Self {
        match error {
            AlbumError::Store(msg) | AlbumError::InvalidInput(msg) => Error::Store(msg),
            other => Error::Snapshot(other.to_string()),
        }
    }
}

fn deserialize_bounded_string<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
    field: &'static str,
    max_bytes: usize,
    max_chars: Option<usize>,
) -> std::result::Result<String, D::Error> {
    struct BoundedStringVisitor {
        field: &'static str,
        max_bytes: usize,
        max_chars: Option<usize>,
    }

    impl<'de> serde::de::Visitor<'de> for BoundedStringVisitor {
        type Value = String;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "a bounded UTF-8 string for {}", self.field)
        }

        fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<String, E> {
            if v.len() > self.max_bytes {
                return Err(E::custom(format!(
                    "{} byte length {} exceeds maximum {}",
                    self.field,
                    v.len(),
                    self.max_bytes
                )));
            }
            if let Some(limit) = self.max_chars {
                let chars = v.chars().count();
                if chars > limit {
                    return Err(E::custom(format!(
                        "{} character length {chars} exceeds maximum {limit}",
                        self.field
                    )));
                }
            }
            Ok(v.to_owned())
        }
    }

    deserializer.deserialize_str(BoundedStringVisitor {
        field,
        max_bytes,
        max_chars,
    })
}

fn deserialize_neighborhood_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<String, D::Error> {
    deserialize_bounded_string(
        deserializer,
        "neighborhood_id",
        MAX_NEIGHBORHOOD_ID_BYTES,
        None,
    )
}

fn deserialize_title<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<String, D::Error> {
    deserialize_bounded_string(
        deserializer,
        "title",
        MAX_ALBUM_TITLE_CHARS * 4,
        Some(MAX_ALBUM_TITLE_CHARS),
    )
}

fn deserialize_caption<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<String, D::Error> {
    deserialize_bounded_string(
        deserializer,
        "caption",
        MAX_ALBUM_CAPTION_CHARS * 4,
        Some(MAX_ALBUM_CAPTION_CHARS),
    )
}

fn deserialize_bounded_order<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<u64>, D::Error> {
    struct OrderVisitor;

    impl<'de> serde::de::Visitor<'de> for OrderVisitor {
        type Value = Vec<u64>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "at most {MAX_ALBUM_ENTRIES} photo IDs")
        }

        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            if let Some(hint) = seq.size_hint()
                && hint > MAX_ALBUM_ENTRIES
            {
                return Err(serde::de::Error::custom(format!(
                    "index entry count {hint} exceeds maximum {MAX_ALBUM_ENTRIES}"
                )));
            }
            let cap = seq.size_hint().map_or(0, |h| h.min(MAX_ALBUM_ENTRIES));
            let mut out = Vec::with_capacity(cap);
            while let Some(id) = seq.next_element::<u64>()? {
                if out.len() >= MAX_ALBUM_ENTRIES {
                    return Err(serde::de::Error::custom(format!(
                        "index entry count exceeds maximum {MAX_ALBUM_ENTRIES}"
                    )));
                }
                out.push(id);
            }
            Ok(out)
        }
    }

    deserializer.deserialize_seq(OrderVisitor)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AlbumIndexV1 {
    #[serde(deserialize_with = "deserialize_neighborhood_id")]
    neighborhood_id: String,
    family_id: u16,
    next_photo_id: u64,
    #[serde(deserialize_with = "deserialize_bounded_order")]
    order: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AlbumPhotoV1 {
    #[serde(deserialize_with = "deserialize_neighborhood_id")]
    neighborhood_id: String,
    family_id: u16,
    id: u64,
    #[serde(deserialize_with = "deserialize_title")]
    title: String,
    #[serde(deserialize_with = "deserialize_caption")]
    caption: String,
    sim_day: Option<u32>,
    image: Option<AlbumImage>,
}

fn record_checksum(domain: &[u8], version: u32, body: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(version.to_le_bytes());
    hasher.update(body);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn encode_framed<T: Serialize>(
    record: &str,
    magic: [u8; 4],
    domain: &[u8],
    max_encoded_bytes: usize,
    value: &T,
) -> std::result::Result<Vec<u8>, AlbumError> {
    let body = postcard::to_allocvec(value).map_err(|error| AlbumError::DecodeFailed {
        record: record.to_owned(),
        detail: format!("failed to encode record: {error}"),
    })?;
    let total_len = HEADER_LEN.saturating_add(body.len());
    if total_len > max_encoded_bytes {
        return Err(AlbumError::InvalidInput(format!(
            "{record}: encoded record size {total_len} exceeds maximum {max_encoded_bytes}"
        )));
    }
    let checksum = record_checksum(domain, ALBUM_FORMAT, &body);
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(&magic);
    out.extend_from_slice(&ALBUM_FORMAT.to_le_bytes());
    out.extend_from_slice(&checksum);
    out.extend_from_slice(&body);
    Ok(out)
}

fn decode_framed<'a, T: Deserialize<'a>>(
    record: &str,
    magic: [u8; 4],
    domain: &[u8],
    max_encoded_bytes: usize,
    bytes: &'a [u8],
) -> std::result::Result<T, AlbumError> {
    if bytes.len() < 8 {
        return Err(AlbumError::DecodeFailed {
            record: record.to_owned(),
            detail: format!("record shorter than 8-byte header ({} bytes)", bytes.len()),
        });
    }
    if bytes[0..4] != magic {
        return Err(AlbumError::DecodeFailed {
            record: record.to_owned(),
            detail: "invalid photo album record magic".into(),
        });
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes"));
    if !(ALBUM_MIN_FORMAT..=ALBUM_FORMAT).contains(&version) {
        return Err(AlbumError::UnsupportedVersion {
            record: record.to_owned(),
            found: version,
            min_supported: ALBUM_MIN_FORMAT,
            max_supported: ALBUM_FORMAT,
        });
    }
    if bytes.len() > max_encoded_bytes {
        return Err(AlbumError::DecodeFailed {
            record: record.to_owned(),
            detail: format!(
                "encoded record size {} exceeds maximum {max_encoded_bytes}",
                bytes.len()
            ),
        });
    }
    if bytes.len() < HEADER_LEN {
        return Err(AlbumError::DecodeFailed {
            record: record.to_owned(),
            detail: format!(
                "record shorter than {HEADER_LEN}-byte header ({} bytes)",
                bytes.len()
            ),
        });
    }
    let expected: [u8; 32] = bytes[8..40].try_into().expect("32 bytes");
    let body = &bytes[HEADER_LEN..];
    let actual = record_checksum(domain, version, body);
    if actual != expected {
        return Err(AlbumError::CorruptChecksum {
            record: record.to_owned(),
            expected,
            actual,
        });
    }
    decode_postcard_exact(record, version, body).map_err(|error| AlbumError::DecodeFailed {
        record: record.to_owned(),
        detail: error.to_string(),
    })
}

fn decode_index(
    neighborhood_id: &str,
    family_id: u16,
    bytes: &[u8],
) -> std::result::Result<AlbumIndexV1, AlbumError> {
    let record = format!("{neighborhood_id}:family{family_id}:album-index");
    let index: AlbumIndexV1 = decode_framed(
        &record,
        INDEX_MAGIC,
        INDEX_HASH_DOMAIN,
        MAX_ALBUM_INDEX_ENCODED_BYTES,
        bytes,
    )?;
    if index.neighborhood_id != neighborhood_id || index.family_id != family_id {
        return Err(AlbumError::DecodeFailed {
            record,
            detail: format!(
                "index key binding mismatch: stored ({:?}, family {}), expected ({:?}, family {})",
                index.neighborhood_id, index.family_id, neighborhood_id, family_id
            ),
        });
    }
    if index.order.len() > MAX_ALBUM_ENTRIES {
        return Err(AlbumError::DecodeFailed {
            record,
            detail: format!(
                "index entry count {} exceeds maximum {MAX_ALBUM_ENTRIES}",
                index.order.len()
            ),
        });
    }
    let mut seen = BTreeSet::new();
    for &id in &index.order {
        if id == 0 {
            return Err(AlbumError::DecodeFailed {
                record,
                detail: "index contains zero photo_id".into(),
            });
        }
        if id > index.next_photo_id {
            return Err(AlbumError::DecodeFailed {
                record,
                detail: format!(
                    "index photo_id {id} exceeds next_photo_id {}",
                    index.next_photo_id
                ),
            });
        }
        if !seen.insert(id) {
            return Err(AlbumError::DecodeFailed {
                record,
                detail: format!("index contains duplicate photo_id {id}"),
            });
        }
    }
    Ok(index)
}

fn decode_photo(
    neighborhood_id: &str,
    family_id: u16,
    photo_id: u64,
    bytes: &[u8],
) -> std::result::Result<AlbumPhotoV1, AlbumError> {
    let record = format!("{neighborhood_id}:family{family_id}:photo#{photo_id}");
    let photo: AlbumPhotoV1 = decode_framed(
        &record,
        PHOTO_MAGIC,
        PHOTO_HASH_DOMAIN,
        MAX_ALBUM_PHOTO_ENCODED_BYTES,
        bytes,
    )?;
    if photo.neighborhood_id != neighborhood_id
        || photo.family_id != family_id
        || photo.id != photo_id
    {
        return Err(AlbumError::DecodeFailed {
            record,
            detail: format!(
                "photo key binding mismatch: stored ({:?}, family {}, photo {}), \
                 expected ({:?}, family {}, photo {})",
                photo.neighborhood_id,
                photo.family_id,
                photo.id,
                neighborhood_id,
                family_id,
                photo_id
            ),
        });
    }
    Ok(photo)
}

impl From<AlbumPhotoV1> for AlbumPhoto {
    fn from(value: AlbumPhotoV1) -> Self {
        Self {
            id: value.id,
            title: value.title,
            caption: value.caption,
            sim_day: value.sim_day,
            image: value.image,
        }
    }
}

impl From<AlbumPhotoV1> for AlbumPhotoSummary {
    fn from(value: AlbumPhotoV1) -> Self {
        Self {
            id: value.id,
            title: value.title,
            caption: value.caption,
            sim_day: value.sim_day,
            image_dimensions: value.image.map(|img| (img.width, img.height)),
        }
    }
}

fn validate_neighborhood_id(neighborhood_id: &str) -> std::result::Result<(), AlbumError> {
    if neighborhood_id.is_empty() {
        return Err(AlbumError::InvalidInput(
            "neighborhood_id must not be empty".into(),
        ));
    }
    if neighborhood_id.len() > MAX_NEIGHBORHOOD_ID_BYTES {
        return Err(AlbumError::InvalidInput(format!(
            "neighborhood_id byte length {} exceeds maximum {MAX_NEIGHBORHOOD_ID_BYTES}",
            neighborhood_id.len()
        )));
    }
    Ok(())
}

fn validate_family_id(family_id: u16) -> std::result::Result<(), AlbumError> {
    if family_id == 0 {
        return Err(AlbumError::InvalidInput(
            "family_id must be non-zero".into(),
        ));
    }
    Ok(())
}

fn validate_family_token(token: AlbumFamilyToken) -> std::result::Result<(), AlbumError> {
    if token.instance_id == 0 {
        return Err(AlbumError::InvalidInput(
            "token.instance_id must be non-zero".into(),
        ));
    }
    validate_family_id(token.family_id)?;
    if token.family_epoch == 0 {
        return Err(AlbumError::InvalidInput(
            "token.family_epoch must be non-zero".into(),
        ));
    }
    Ok(())
}

fn validate_text(title: &str, caption: &str) -> std::result::Result<(), AlbumError> {
    let title_chars = title.chars().count();
    if title_chars > MAX_ALBUM_TITLE_CHARS {
        return Err(AlbumError::InvalidInput(format!(
            "photo title length {title_chars} exceeds maximum {MAX_ALBUM_TITLE_CHARS} characters"
        )));
    }
    let caption_chars = caption.chars().count();
    if caption_chars > MAX_ALBUM_CAPTION_CHARS {
        return Err(AlbumError::InvalidInput(format!(
            "photo caption length {caption_chars} exceeds maximum {MAX_ALBUM_CAPTION_CHARS} characters"
        )));
    }
    Ok(())
}

fn store_err(error: impl fmt::Display) -> AlbumError {
    AlbumError::Store(error.to_string())
}

fn verify_family_token_in_txn(
    txn: &redb::WriteTransaction,
    neighborhood_id: &str,
    token: AlbumFamilyToken,
) -> std::result::Result<(), AlbumError> {
    let nh_table = txn.open_table(NEIGHBORHOODS_TABLE).map_err(store_err)?;
    let Some(raw) = nh_table.get(neighborhood_id).map_err(store_err)? else {
        return Err(AlbumError::StaleNeighborhoodInstance {
            neighborhood_id: neighborhood_id.to_owned(),
            expected_instance_id: token.instance_id,
            current_instance_id: None,
        });
    };
    let nh = StoredNeighborhood::decode(neighborhood_id, raw.value()).map_err(|error| {
        AlbumError::Store(format!(
            "cannot verify neighborhood {neighborhood_id:?} instance: {error}"
        ))
    })?;
    if nh.instance_id != token.instance_id {
        return Err(AlbumError::StaleNeighborhoodInstance {
            neighborhood_id: neighborhood_id.to_owned(),
            expected_instance_id: token.instance_id,
            current_instance_id: Some(nh.instance_id),
        });
    }
    if !nh.families.contains_key(&token.family_id) {
        return Err(AlbumError::StaleFamilyToken {
            neighborhood_id: neighborhood_id.to_owned(),
            family_id: token.family_id,
            expected_epoch: token.family_epoch,
            current_epoch: None,
        });
    }
    let epoch_table = txn.open_table(ALBUM_FAMILY_EPOCH).map_err(store_err)?;
    let current_epoch = epoch_table
        .get((neighborhood_id, token.family_id))
        .map_err(store_err)?
        .map_or(INITIAL_FAMILY_EPOCH, |v| v.value());
    if current_epoch != token.family_epoch {
        return Err(AlbumError::StaleFamilyToken {
            neighborhood_id: neighborhood_id.to_owned(),
            family_id: token.family_id,
            expected_epoch: token.family_epoch,
            current_epoch: Some(current_epoch),
        });
    }
    Ok(())
}

/// REDB-backed household photo album store sharing a [`crate::NeighborhoodStore`]'s
/// `Arc<Database>` handle.
///
/// `PhotoAlbumStore` is `Clone + Send + Sync` and enforces [`AlbumFamilyToken`]
/// neighborhood + household lifetime fencing inside every mutating write
/// transaction.
#[derive(Clone)]
pub struct PhotoAlbumStore {
    db: Arc<Database>,
}

impl fmt::Debug for PhotoAlbumStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PhotoAlbumStore").finish_non_exhaustive()
    }
}

impl PhotoAlbumStore {
    pub(crate) fn from_shared_db(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Reads the authoritative [`AlbumFamilyToken`] for `(neighborhood_id, family_id)`
    /// inside a single MVCC read transaction.
    ///
    /// Returns `Ok(None)` if `neighborhood_id` does not exist or `family_id` is
    /// not currently a member of `StoredNeighborhood::families`.
    pub fn family_token(
        &self,
        neighborhood_id: &str,
        family_id: u16,
    ) -> std::result::Result<Option<AlbumFamilyToken>, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        validate_family_id(family_id)?;

        let txn = self.db.begin_read().map_err(store_err)?;
        let nh_table = match txn.open_table(NEIGHBORHOODS_TABLE) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store_err(error)),
        };
        let Some(raw) = nh_table.get(neighborhood_id).map_err(store_err)? else {
            return Ok(None);
        };
        let nh = StoredNeighborhood::decode(neighborhood_id, raw.value()).map_err(|error| {
            AlbumError::Store(format!(
                "cannot decode neighborhood {neighborhood_id:?}: {error}"
            ))
        })?;
        if !nh.families.contains_key(&family_id) {
            return Ok(None);
        }
        let family_epoch = match txn.open_table(ALBUM_FAMILY_EPOCH) {
            Ok(table) => table
                .get((neighborhood_id, family_id))
                .map_err(store_err)?
                .map_or(INITIAL_FAMILY_EPOCH, |v| v.value()),
            Err(redb::TableError::TableDoesNotExist(_)) => INITIAL_FAMILY_EPOCH,
            Err(error) => return Err(store_err(error)),
        };
        Ok(Some(AlbumFamilyToken {
            instance_id: nh.instance_id,
            family_id,
            family_epoch,
        }))
    }

    /// Appends a new photo or blank storybook page to `token.family_id` in
    /// `neighborhood_id`, assigning a strictly monotonic `photo_id >= 1`,
    /// enforcing [`MAX_ALBUM_ENTRIES`], and fencing the write transaction
    /// against `token`.
    pub fn add(
        &self,
        neighborhood_id: &str,
        token: AlbumFamilyToken,
        entry: NewAlbumPhoto,
    ) -> std::result::Result<AlbumPhoto, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        validate_family_token(token)?;
        validate_text(&entry.title, &entry.caption)?;
        if let Some(image) = &entry.image {
            image.validate()?;
        }
        let family_id = token.family_id;

        let txn = self.db.begin_write().map_err(store_err)?;
        let assigned_id;
        {
            verify_family_token_in_txn(&txn, neighborhood_id, token)?;
            let mut index_table = txn.open_table(ALBUM_INDEX).map_err(store_err)?;
            let mut photo_table = txn.open_table(ALBUM_PHOTOS).map_err(store_err)?;

            let mut index = match index_table
                .get((neighborhood_id, family_id))
                .map_err(store_err)?
            {
                Some(raw) => decode_index(neighborhood_id, family_id, raw.value())?,
                None => AlbumIndexV1 {
                    neighborhood_id: neighborhood_id.to_owned(),
                    family_id,
                    next_photo_id: 0,
                    order: Vec::new(),
                },
            };

            if index.order.len() >= MAX_ALBUM_ENTRIES {
                return Err(AlbumError::AlbumFull {
                    neighborhood_id: neighborhood_id.to_owned(),
                    family_id,
                    max_entries: MAX_ALBUM_ENTRIES,
                });
            }

            // Guarantee strictly monotonic ID allocation even if stray rows
            // exist past `next_photo_id`.
            let max_existing_id = photo_table
                .range((neighborhood_id, family_id, 0u64)..=(neighborhood_id, family_id, u64::MAX))
                .map_err(store_err)?
                .next_back()
                .transpose()
                .map_err(store_err)?
                .map_or(0u64, |(k, _)| k.value().2);
            let next_id = index
                .next_photo_id
                .max(max_existing_id)
                .checked_add(1)
                .ok_or_else(|| {
                    AlbumError::InvalidInput(format!(
                        "photo album ID sequence exhausted for ({neighborhood_id:?}, family {family_id})"
                    ))
                })?;
            assigned_id = next_id;

            let photo_record = AlbumPhotoV1 {
                neighborhood_id: neighborhood_id.to_owned(),
                family_id,
                id: next_id,
                title: entry.title.clone(),
                caption: entry.caption.clone(),
                sim_day: entry.sim_day,
                image: entry.image.clone(),
            };
            let photo_name = format!("{neighborhood_id}:family{family_id}:photo#{next_id}");
            let encoded_photo = encode_framed(
                &photo_name,
                PHOTO_MAGIC,
                PHOTO_HASH_DOMAIN,
                MAX_ALBUM_PHOTO_ENCODED_BYTES,
                &photo_record,
            )?;

            index.next_photo_id = next_id;
            index.order.push(next_id);
            let index_name = format!("{neighborhood_id}:family{family_id}:album-index");
            let encoded_index = encode_framed(
                &index_name,
                INDEX_MAGIC,
                INDEX_HASH_DOMAIN,
                MAX_ALBUM_INDEX_ENCODED_BYTES,
                &index,
            )?;

            photo_table
                .insert(
                    (neighborhood_id, family_id, next_id),
                    encoded_photo.as_slice(),
                )
                .map_err(store_err)?;
            index_table
                .insert((neighborhood_id, family_id), encoded_index.as_slice())
                .map_err(store_err)?;
        }
        txn.commit().map_err(store_err)?;

        Ok(AlbumPhoto {
            id: assigned_id,
            title: entry.title,
            caption: entry.caption,
            sim_day: entry.sim_day,
            image: entry.image,
        })
    }

    /// Updates the title and caption of an existing photo in `token.family_id`
    /// atomically, fenced by `token`.
    pub fn update_text(
        &self,
        neighborhood_id: &str,
        token: AlbumFamilyToken,
        photo_id: u64,
        title: &str,
        caption: &str,
    ) -> std::result::Result<AlbumPhoto, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        validate_family_token(token)?;
        if photo_id == 0 {
            return Err(AlbumError::InvalidInput("photo_id must be non-zero".into()));
        }
        validate_text(title, caption)?;
        let family_id = token.family_id;

        let txn = self.db.begin_write().map_err(store_err)?;
        let updated: AlbumPhotoV1;
        {
            verify_family_token_in_txn(&txn, neighborhood_id, token)?;
            let index_table = txn.open_table(ALBUM_INDEX).map_err(store_err)?;
            let Some(raw_index) = index_table
                .get((neighborhood_id, family_id))
                .map_err(store_err)?
            else {
                return Err(AlbumError::PhotoNotFound {
                    neighborhood_id: neighborhood_id.to_owned(),
                    family_id,
                    photo_id,
                });
            };
            let index = decode_index(neighborhood_id, family_id, raw_index.value())?;
            if !index.order.contains(&photo_id) {
                return Err(AlbumError::PhotoNotFound {
                    neighborhood_id: neighborhood_id.to_owned(),
                    family_id,
                    photo_id,
                });
            }

            let mut photo_table = txn.open_table(ALBUM_PHOTOS).map_err(store_err)?;
            let mut photo = {
                match photo_table
                    .get((neighborhood_id, family_id, photo_id))
                    .map_err(store_err)?
                {
                    Some(raw) => decode_photo(neighborhood_id, family_id, photo_id, raw.value())?,
                    None => {
                        return Err(AlbumError::DecodeFailed {
                            record: format!("{neighborhood_id}:family{family_id}:photo#{photo_id}"),
                            detail: "photo referenced by album index is missing".into(),
                        });
                    }
                }
            };
            photo.title = title.to_owned();
            photo.caption = caption.to_owned();
            let photo_name = format!("{neighborhood_id}:family{family_id}:photo#{photo_id}");
            let encoded = encode_framed(
                &photo_name,
                PHOTO_MAGIC,
                PHOTO_HASH_DOMAIN,
                MAX_ALBUM_PHOTO_ENCODED_BYTES,
                &photo,
            )?;
            photo_table
                .insert((neighborhood_id, family_id, photo_id), encoded.as_slice())
                .map_err(store_err)?;
            updated = photo;
        }
        txn.commit().map_err(store_err)?;
        Ok(updated.into())
    }

    /// Reorders `photo_id` to zero-based `new_index` (`0..entries.len()`)
    /// inside `token.family_id` atomically, fenced by `token`. Returns the new
    /// deterministic photo ID order.
    pub fn move_entry(
        &self,
        neighborhood_id: &str,
        token: AlbumFamilyToken,
        photo_id: u64,
        new_index: usize,
    ) -> std::result::Result<Vec<u64>, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        validate_family_token(token)?;
        if photo_id == 0 {
            return Err(AlbumError::InvalidInput("photo_id must be non-zero".into()));
        }
        let family_id = token.family_id;

        let txn = self.db.begin_write().map_err(store_err)?;
        let new_order;
        {
            verify_family_token_in_txn(&txn, neighborhood_id, token)?;
            let mut index_table = txn.open_table(ALBUM_INDEX).map_err(store_err)?;
            let mut index = {
                let Some(raw_index) = index_table
                    .get((neighborhood_id, family_id))
                    .map_err(store_err)?
                else {
                    return Err(AlbumError::PhotoNotFound {
                        neighborhood_id: neighborhood_id.to_owned(),
                        family_id,
                        photo_id,
                    });
                };
                decode_index(neighborhood_id, family_id, raw_index.value())?
            };
            let Some(old_pos) = index.order.iter().position(|&id| id == photo_id) else {
                return Err(AlbumError::PhotoNotFound {
                    neighborhood_id: neighborhood_id.to_owned(),
                    family_id,
                    photo_id,
                });
            };
            if new_index >= index.order.len() {
                return Err(AlbumError::InvalidIndex {
                    index: new_index,
                    len: index.order.len(),
                });
            }
            if old_pos != new_index {
                let moved = index.order.remove(old_pos);
                index.order.insert(new_index, moved);
                let index_name = format!("{neighborhood_id}:family{family_id}:album-index");
                let encoded = encode_framed(
                    &index_name,
                    INDEX_MAGIC,
                    INDEX_HASH_DOMAIN,
                    MAX_ALBUM_INDEX_ENCODED_BYTES,
                    &index,
                )?;
                index_table
                    .insert((neighborhood_id, family_id), encoded.as_slice())
                    .map_err(store_err)?;
            }
            new_order = index.order;
        }
        txn.commit().map_err(store_err)?;
        Ok(new_order)
    }

    /// Deletes a single photo entry from `token.family_id` atomically, fenced
    /// by `token`, while preserving `next_photo_id` so deleted IDs are never
    /// reused. Returns `true` if the photo existed.
    pub fn delete(
        &self,
        neighborhood_id: &str,
        token: AlbumFamilyToken,
        photo_id: u64,
    ) -> std::result::Result<bool, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        validate_family_token(token)?;
        if photo_id == 0 {
            return Err(AlbumError::InvalidInput("photo_id must be non-zero".into()));
        }
        let family_id = token.family_id;

        let txn = self.db.begin_write().map_err(store_err)?;
        let existed;
        {
            verify_family_token_in_txn(&txn, neighborhood_id, token)?;
            let mut index_table = txn.open_table(ALBUM_INDEX).map_err(store_err)?;
            let mut index = {
                let Some(raw_index) = index_table
                    .get((neighborhood_id, family_id))
                    .map_err(store_err)?
                else {
                    return Ok(false);
                };
                decode_index(neighborhood_id, family_id, raw_index.value())?
            };
            let Some(pos) = index.order.iter().position(|&id| id == photo_id) else {
                return Ok(false);
            };
            index.order.remove(pos);
            let index_name = format!("{neighborhood_id}:family{family_id}:album-index");
            let encoded = encode_framed(
                &index_name,
                INDEX_MAGIC,
                INDEX_HASH_DOMAIN,
                MAX_ALBUM_INDEX_ENCODED_BYTES,
                &index,
            )?;
            index_table
                .insert((neighborhood_id, family_id), encoded.as_slice())
                .map_err(store_err)?;
            let mut photo_table = txn.open_table(ALBUM_PHOTOS).map_err(store_err)?;
            photo_table
                .remove((neighborhood_id, family_id, photo_id))
                .map_err(store_err)?;
            existed = true;
        }
        txn.commit().map_err(store_err)?;
        Ok(existed)
    }

    /// Removes all photo entries for a living household `token.family_id`
    /// inside a single write transaction fenced by `token`, preserving
    /// `next_photo_id` (when the existing index is readable) so cleared photo
    /// IDs are never reused within the same household lifetime. Returns `true`
    /// if any album data existed.
    pub fn delete_family_album(
        &self,
        neighborhood_id: &str,
        token: AlbumFamilyToken,
    ) -> std::result::Result<bool, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        validate_family_token(token)?;
        let family_id = token.family_id;

        let txn = self.db.begin_write().map_err(store_err)?;
        let mut existed = false;
        {
            verify_family_token_in_txn(&txn, neighborhood_id, token)?;
            let mut index_table = txn.open_table(ALBUM_INDEX).map_err(store_err)?;
            let preserved_next_id = {
                match index_table
                    .get((neighborhood_id, family_id))
                    .map_err(store_err)?
                {
                    Some(raw) => {
                        existed = true;
                        decode_index(neighborhood_id, family_id, raw.value())
                            .ok()
                            .map(|idx| idx.next_photo_id)
                    }
                    None => None,
                }
            };
            let mut photo_table = txn.open_table(ALBUM_PHOTOS).map_err(store_err)?;
            let mut photo_ids = Vec::new();
            let mut max_photo_id = 0u64;
            for item in photo_table
                .range((neighborhood_id, family_id, 0u64)..=(neighborhood_id, family_id, u64::MAX))
                .map_err(store_err)?
            {
                let (k, _) = item.map_err(store_err)?;
                let pid = k.value().2;
                if pid > max_photo_id {
                    max_photo_id = pid;
                }
                photo_ids.push(pid);
            }
            if !photo_ids.is_empty() {
                existed = true;
            }
            for photo_id in photo_ids {
                photo_table
                    .remove((neighborhood_id, family_id, photo_id))
                    .map_err(store_err)?;
            }
            if existed {
                let keep_next = preserved_next_id.unwrap_or(0).max(max_photo_id);
                if keep_next > 0 {
                    let cleared = AlbumIndexV1 {
                        neighborhood_id: neighborhood_id.to_owned(),
                        family_id,
                        next_photo_id: keep_next,
                        order: Vec::new(),
                    };
                    let index_name = format!("{neighborhood_id}:family{family_id}:album-index");
                    let encoded = encode_framed(
                        &index_name,
                        INDEX_MAGIC,
                        INDEX_HASH_DOMAIN,
                        MAX_ALBUM_INDEX_ENCODED_BYTES,
                        &cleared,
                    )?;
                    index_table
                        .insert((neighborhood_id, family_id), encoded.as_slice())
                        .map_err(store_err)?;
                } else {
                    index_table
                        .remove((neighborhood_id, family_id))
                        .map_err(store_err)?;
                }
            }
        }
        txn.commit().map_err(store_err)?;
        Ok(existed)
    }

    /// Lists all photo album entries for `(neighborhood_id, family_id)` in
    /// deterministic page order inside a single MVCC read transaction,
    /// enforcing [`MAX_BULK_ALBUM_PIXEL_BYTES`].
    ///
    /// For very large albums whose combined RGB565 bitmaps exceed
    /// [`MAX_BULK_ALBUM_PIXEL_BYTES`], use [`Self::list_summaries`] paired with
    /// [`Self::list_page`] or [`Self::get`].
    pub fn list(
        &self,
        neighborhood_id: &str,
        family_id: u16,
    ) -> std::result::Result<Vec<AlbumPhoto>, AlbumError> {
        self.list_page_with_budget(
            neighborhood_id,
            family_id,
            0,
            MAX_ALBUM_ENTRIES,
            MAX_BULK_ALBUM_PIXEL_BYTES,
        )
    }

    /// Lists lightweight metadata summaries (dropping pixel buffers immediately
    /// after validating each entry) for all entries of `(neighborhood_id, family_id)`
    /// in deterministic page order inside a single MVCC read transaction.
    pub fn list_summaries(
        &self,
        neighborhood_id: &str,
        family_id: u16,
    ) -> std::result::Result<Vec<AlbumPhotoSummary>, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        validate_family_id(family_id)?;

        let txn = self.db.begin_read().map_err(store_err)?;
        let index_table = match txn.open_table(ALBUM_INDEX) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(error) => return Err(store_err(error)),
        };
        let Some(raw_index) = index_table
            .get((neighborhood_id, family_id))
            .map_err(store_err)?
        else {
            return Ok(Vec::new());
        };
        let index = decode_index(neighborhood_id, family_id, raw_index.value())?;
        if index.order.is_empty() {
            return Ok(Vec::new());
        }
        let photo_table = match txn.open_table(ALBUM_PHOTOS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => {
                return Err(AlbumError::DecodeFailed {
                    record: format!("{neighborhood_id}:family{family_id}:album-index"),
                    detail: "album photo table missing for non-empty index".into(),
                });
            }
            Err(error) => return Err(store_err(error)),
        };

        let mut summaries = Vec::with_capacity(index.order.len());
        for photo_id in index.order {
            let Some(raw_photo) = photo_table
                .get((neighborhood_id, family_id, photo_id))
                .map_err(store_err)?
            else {
                return Err(AlbumError::DecodeFailed {
                    record: format!("{neighborhood_id}:family{family_id}:photo#{photo_id}"),
                    detail: "photo referenced by album index is missing".into(),
                });
            };
            let photo = decode_photo(neighborhood_id, family_id, photo_id, raw_photo.value())?;
            summaries.push(photo.into());
        }
        Ok(summaries)
    }

    /// Loads one contiguous slice `offset..offset+limit` of photo album entries
    /// in deterministic page order inside a single MVCC read transaction,
    /// enforcing [`MAX_BULK_ALBUM_PIXEL_BYTES`].
    pub fn list_page(
        &self,
        neighborhood_id: &str,
        family_id: u16,
        offset: usize,
        limit: usize,
    ) -> std::result::Result<Vec<AlbumPhoto>, AlbumError> {
        self.list_page_with_budget(
            neighborhood_id,
            family_id,
            offset,
            limit,
            MAX_BULK_ALBUM_PIXEL_BYTES,
        )
    }

    /// Like [`Self::list_page`], with a caller-specified aggregate pixel-buffer
    /// byte ceiling (`max_pixel_bytes`).
    pub fn list_page_with_budget(
        &self,
        neighborhood_id: &str,
        family_id: u16,
        offset: usize,
        limit: usize,
        max_pixel_bytes: usize,
    ) -> std::result::Result<Vec<AlbumPhoto>, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        validate_family_id(family_id)?;

        let txn = self.db.begin_read().map_err(store_err)?;
        let index_table = match txn.open_table(ALBUM_INDEX) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(error) => return Err(store_err(error)),
        };
        let Some(raw_index) = index_table
            .get((neighborhood_id, family_id))
            .map_err(store_err)?
        else {
            return Ok(Vec::new());
        };
        let index = decode_index(neighborhood_id, family_id, raw_index.value())?;
        if index.order.is_empty() || limit == 0 || offset >= index.order.len() {
            return Ok(Vec::new());
        }
        let photo_table = match txn.open_table(ALBUM_PHOTOS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => {
                return Err(AlbumError::DecodeFailed {
                    record: format!("{neighborhood_id}:family{family_id}:album-index"),
                    detail: "album photo table missing for non-empty index".into(),
                });
            }
            Err(error) => return Err(store_err(error)),
        };

        let slice_end = offset.saturating_add(limit).min(index.order.len());
        let page_ids = &index.order[offset..slice_end];
        let mut entries = Vec::with_capacity(page_ids.len());
        let mut total_pixel_bytes = 0usize;
        for &photo_id in page_ids {
            let Some(raw_photo) = photo_table
                .get((neighborhood_id, family_id, photo_id))
                .map_err(store_err)?
            else {
                return Err(AlbumError::DecodeFailed {
                    record: format!("{neighborhood_id}:family{family_id}:photo#{photo_id}"),
                    detail: "photo referenced by album index is missing".into(),
                });
            };
            let photo = decode_photo(neighborhood_id, family_id, photo_id, raw_photo.value())?;
            let entry_bytes = photo.image.as_ref().map_or(0, AlbumImage::pixel_bytes);
            let next_total = total_pixel_bytes.saturating_add(entry_bytes);
            if next_total > max_pixel_bytes {
                return Err(AlbumError::BudgetExceeded {
                    record: format!("{neighborhood_id}:family{family_id}"),
                    limit_bytes: max_pixel_bytes,
                    required_bytes: next_total,
                });
            }
            total_pixel_bytes = next_total;
            entries.push(photo.into());
        }
        Ok(entries)
    }

    /// Loads one indexed photo entry by `(neighborhood_id, family_id, photo_id)`
    /// inside a single MVCC read transaction.
    pub fn get(
        &self,
        neighborhood_id: &str,
        family_id: u16,
        photo_id: u64,
    ) -> std::result::Result<Option<AlbumPhoto>, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        validate_family_id(family_id)?;
        if photo_id == 0 {
            return Err(AlbumError::InvalidInput("photo_id must be non-zero".into()));
        }

        let txn = self.db.begin_read().map_err(store_err)?;
        let index_table = match txn.open_table(ALBUM_INDEX) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store_err(error)),
        };
        let Some(raw_index) = index_table
            .get((neighborhood_id, family_id))
            .map_err(store_err)?
        else {
            return Ok(None);
        };
        let index = decode_index(neighborhood_id, family_id, raw_index.value())?;
        if !index.order.contains(&photo_id) {
            return Ok(None);
        }
        let photo_table = match txn.open_table(ALBUM_PHOTOS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => {
                return Err(AlbumError::DecodeFailed {
                    record: format!("{neighborhood_id}:family{family_id}:photo#{photo_id}"),
                    detail: "album photo table missing for indexed photo".into(),
                });
            }
            Err(error) => return Err(store_err(error)),
        };
        let Some(raw_photo) = photo_table
            .get((neighborhood_id, family_id, photo_id))
            .map_err(store_err)?
        else {
            return Err(AlbumError::DecodeFailed {
                record: format!("{neighborhood_id}:family{family_id}:photo#{photo_id}"),
                detail: "photo referenced by album index is missing".into(),
            });
        };
        let photo = decode_photo(neighborhood_id, family_id, photo_id, raw_photo.value())?;
        Ok(Some(photo.into()))
    }

    /// Lists all `family_id`s in `neighborhood_id` that currently have at least
    /// one photo album entry, in ascending order.
    pub fn list_families(
        &self,
        neighborhood_id: &str,
    ) -> std::result::Result<Vec<u16>, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        let txn = self.db.begin_read().map_err(store_err)?;
        let index_table = match txn.open_table(ALBUM_INDEX) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(error) => return Err(store_err(error)),
        };
        let mut families = Vec::new();
        for item in index_table
            .range((neighborhood_id, 0u16)..=(neighborhood_id, u16::MAX))
            .map_err(store_err)?
        {
            let (k, v) = item.map_err(store_err)?;
            let family_id = k.value().1;
            let index = decode_index(neighborhood_id, family_id, v.value())?;
            if !index.order.is_empty() {
                families.push(family_id);
            }
        }
        Ok(families)
    }
}

/// Atomically purges one deleted family's photo album (`ALBUM_INDEX` and
/// `ALBUM_PHOTOS`) and advances its `ALBUM_FAMILY_EPOCH` counter inside an open
/// REDB write transaction so stale [`AlbumFamilyToken`]s cannot mutate a
/// replacement household reusing `family_id`.
///
/// Called by [`crate::NeighborhoodStore`] inside every neighborhood write
/// transaction whenever a non-zero family is removed from
/// `StoredNeighborhood::families`.
pub(crate) fn purge_family_in_txn(
    txn: &redb::WriteTransaction,
    neighborhood_id: &str,
    family_id: u16,
) -> std::result::Result<(), Error> {
    if family_id == 0 {
        return Ok(());
    }
    let mut index_table = txn
        .open_table(ALBUM_INDEX)
        .map_err(|e| Error::Store(e.to_string()))?;
    index_table
        .remove((neighborhood_id, family_id))
        .map_err(|e| Error::Store(e.to_string()))?;

    let mut photo_table = txn
        .open_table(ALBUM_PHOTOS)
        .map_err(|e| Error::Store(e.to_string()))?;
    let mut photo_ids = Vec::new();
    for item in photo_table
        .range((neighborhood_id, family_id, 0u64)..=(neighborhood_id, family_id, u64::MAX))
        .map_err(|e| Error::Store(e.to_string()))?
    {
        let (k, _) = item.map_err(|e| Error::Store(e.to_string()))?;
        photo_ids.push(k.value().2);
    }
    for photo_id in photo_ids {
        photo_table
            .remove((neighborhood_id, family_id, photo_id))
            .map_err(|e| Error::Store(e.to_string()))?;
    }

    let mut epoch_table = txn
        .open_table(ALBUM_FAMILY_EPOCH)
        .map_err(|e| Error::Store(e.to_string()))?;
    let next_epoch = {
        let current = epoch_table
            .get((neighborhood_id, family_id))
            .map_err(|e| Error::Store(e.to_string()))?
            .map_or(INITIAL_FAMILY_EPOCH, |v| v.value());
        current.checked_add(1).ok_or_else(|| {
            Error::Store(format!(
                "photo album family epoch exhausted for ({neighborhood_id:?}, family {family_id})"
            ))
        })?
    };
    epoch_table
        .insert((neighborhood_id, family_id), next_epoch)
        .map_err(|e| Error::Store(e.to_string()))?;
    Ok(())
}

/// Removes all photo album indices, photo rows, and family-epoch rows for
/// `neighborhood_id` inside an open REDB write transaction.
///
/// Called by [`crate::NeighborhoodStore::delete_neighborhood`] and
/// [`crate::NeighborhoodStore::import_directory`] so deleted or re-imported
/// neighborhoods never leave orphaned photo albums behind.
pub(crate) fn purge_neighborhood_in_txn(
    txn: &redb::WriteTransaction,
    neighborhood_id: &str,
) -> std::result::Result<(), Error> {
    let mut index_table = txn
        .open_table(ALBUM_INDEX)
        .map_err(|e| Error::Store(e.to_string()))?;
    let mut families = Vec::new();
    for item in index_table
        .range((neighborhood_id, 0u16)..=(neighborhood_id, u16::MAX))
        .map_err(|e| Error::Store(e.to_string()))?
    {
        let (k, _) = item.map_err(|e| Error::Store(e.to_string()))?;
        families.push(k.value().1);
    }
    for family_id in families {
        index_table
            .remove((neighborhood_id, family_id))
            .map_err(|e| Error::Store(e.to_string()))?;
    }

    let mut photo_table = txn
        .open_table(ALBUM_PHOTOS)
        .map_err(|e| Error::Store(e.to_string()))?;
    let mut photo_keys = Vec::new();
    for item in photo_table
        .range((neighborhood_id, 0u16, 0u64)..=(neighborhood_id, u16::MAX, u64::MAX))
        .map_err(|e| Error::Store(e.to_string()))?
    {
        let (k, _) = item.map_err(|e| Error::Store(e.to_string()))?;
        let (_, family_id, photo_id) = k.value();
        photo_keys.push((family_id, photo_id));
    }
    for (family_id, photo_id) in photo_keys {
        photo_table
            .remove((neighborhood_id, family_id, photo_id))
            .map_err(|e| Error::Store(e.to_string()))?;
    }

    let mut epoch_table = txn
        .open_table(ALBUM_FAMILY_EPOCH)
        .map_err(|e| Error::Store(e.to_string()))?;
    let mut epoch_families = Vec::new();
    for item in epoch_table
        .range((neighborhood_id, 0u16)..=(neighborhood_id, u16::MAX))
        .map_err(|e| Error::Store(e.to_string()))?
    {
        let (k, _) = item.map_err(|e| Error::Store(e.to_string()))?;
        epoch_families.push(k.value().1);
    }
    for family_id in epoch_families {
        epoch_table
            .remove((neighborhood_id, family_id))
            .map_err(|e| Error::Store(e.to_string()))?;
    }
    Ok(())
}
