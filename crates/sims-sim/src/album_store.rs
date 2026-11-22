//! REDB-backed persistent Household Photo Album / Storybook storage.
//!
//! Stores per-`(neighborhood_id, family_id)` photo album entries inside the
//! same REDB database file as [`crate::NeighborhoodStore`] (via
//! [`crate::NeighborhoodStore::albums`]), matching `sims-ui::album`'s view
//! model and commands (`add`, `update_text`, `move_entry`, `delete`, `list`).
//!
//! ## Integrity, Ordering & Lifetime Guarantees
//! - Every index row (`photo_album_index`) and photo row (`photo_album_photos`)
//!   is framed with a 4-byte magic (`"ALBI"` / `"ALBP"`), 4-byte little-endian
//!   format version (`ALBUM_FORMAT = 1`), 32-byte domain-separated SHA-256
//!   checksum, strict exact-length `postcard` body decoding, and key-binding
//!   verification (`neighborhood_id`, `family_id`, and `photo_id`).
//! - Photo IDs are strictly monotonic (`>= 1`) per `(neighborhood_id, family_id)`
//!   and are never reused after deletion.
//! - Ordering is stored explicitly in the per-family index row (`order: Vec<u64>`),
//!   making [`PhotoAlbumStore::move_entry`] atomic and lightweight without
//!   rewriting photo bitmaps, while [`PhotoAlbumStore::list`] reads the index
//!   and every referenced photo in one consistent MVCC read transaction.
//! - All text lengths, image dimensions, pixel counts, and per-household entry
//!   counts are strictly bounded on both write and read.
//! - Every mutating operation enforces `expected_instance_id` neighborhood
//!   lifetime fencing inside its REDB write transaction against
//!   `NEIGHBORHOODS[neighborhood_id]`, aborting cleanly with
//!   [`AlbumError::StaleNeighborhoodInstance`] after a neighborhood delete or
//!   re-import.
//! - [`crate::NeighborhoodStore::delete_neighborhood`] and
//!   [`crate::NeighborhoodStore::import_directory`] atomically purge all album
//!   rows for that neighborhood inside their write transaction.

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

/// Per-household album index (`next_photo_id` + ordered `photo_id` list) keyed
/// by `(neighborhood_id, family_id)`.
const ALBUM_INDEX: TableDefinition<(&str, u16), &[u8]> = TableDefinition::new("photo_album_index");

/// Individual photo album entries keyed by `(neighborhood_id, family_id, photo_id)`.
const ALBUM_PHOTOS: TableDefinition<(&str, u16, u64), &[u8]> =
    TableDefinition::new("photo_album_photos");

/// Bounded 16-bit RGB565 photo bitmap stored inside an [`AlbumPhoto`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumImage {
    pub width: u32,
    pub height: u32,
    /// Row-major top-down 16-bit RGB565 pixels (`len == width * height`).
    pub rgb565: Vec<u16>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AlbumIndexV1 {
    neighborhood_id: String,
    family_id: u16,
    next_photo_id: u64,
    order: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AlbumPhotoV1 {
    neighborhood_id: String,
    family_id: u16,
    id: u64,
    title: String,
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
    value: &T,
) -> std::result::Result<Vec<u8>, AlbumError> {
    let body = postcard::to_allocvec(value).map_err(|error| AlbumError::DecodeFailed {
        record: record.to_owned(),
        detail: format!("failed to encode record: {error}"),
    })?;
    let checksum = record_checksum(domain, ALBUM_FORMAT, &body);
    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
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
    let index: AlbumIndexV1 = decode_framed(&record, INDEX_MAGIC, INDEX_HASH_DOMAIN, bytes)?;
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
    let photo: AlbumPhotoV1 = decode_framed(&record, PHOTO_MAGIC, PHOTO_HASH_DOMAIN, bytes)?;
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
    validate_text(&photo.title, &photo.caption).map_err(|error| AlbumError::DecodeFailed {
        record: record.clone(),
        detail: error.to_string(),
    })?;
    if let Some(image) = &photo.image {
        image.validate().map_err(|error| AlbumError::DecodeFailed {
            record,
            detail: error.to_string(),
        })?;
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

fn validate_neighborhood_id(neighborhood_id: &str) -> std::result::Result<(), AlbumError> {
    if neighborhood_id.is_empty() {
        return Err(AlbumError::InvalidInput(
            "neighborhood_id must not be empty".into(),
        ));
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

fn validate_instance_id(expected_instance_id: u128) -> std::result::Result<(), AlbumError> {
    if expected_instance_id == 0 {
        return Err(AlbumError::InvalidInput(
            "expected_instance_id must be non-zero".into(),
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

fn verify_neighborhood_instance_in_txn(
    txn: &redb::WriteTransaction,
    neighborhood_id: &str,
    expected_instance_id: u128,
) -> std::result::Result<(), AlbumError> {
    let nh_table = txn.open_table(NEIGHBORHOODS_TABLE).map_err(store_err)?;
    let Some(raw) = nh_table.get(neighborhood_id).map_err(store_err)? else {
        return Err(AlbumError::StaleNeighborhoodInstance {
            neighborhood_id: neighborhood_id.to_owned(),
            expected_instance_id,
            current_instance_id: None,
        });
    };
    let nh = StoredNeighborhood::decode(neighborhood_id, raw.value()).map_err(|error| {
        AlbumError::Store(format!(
            "cannot verify neighborhood {neighborhood_id:?} instance: {error}"
        ))
    })?;
    if nh.instance_id != expected_instance_id {
        return Err(AlbumError::StaleNeighborhoodInstance {
            neighborhood_id: neighborhood_id.to_owned(),
            expected_instance_id,
            current_instance_id: Some(nh.instance_id),
        });
    }
    Ok(())
}

/// REDB-backed household photo album store sharing a [`crate::NeighborhoodStore`]'s
/// `Arc<Database>` handle.
///
/// `PhotoAlbumStore` is `Clone + Send + Sync` and enforces `expected_instance_id`
/// lifetime fencing inside every mutating write transaction.
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

    /// Appends a new photo or blank storybook page to `(neighborhood_id, family_id)`,
    /// assigning a strictly monotonic `photo_id >= 1`, enforcing [`MAX_ALBUM_ENTRIES`],
    /// and fencing the write transaction against `expected_instance_id`.
    pub fn add(
        &self,
        neighborhood_id: &str,
        expected_instance_id: u128,
        family_id: u16,
        entry: NewAlbumPhoto,
    ) -> std::result::Result<AlbumPhoto, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        validate_instance_id(expected_instance_id)?;
        validate_family_id(family_id)?;
        validate_text(&entry.title, &entry.caption)?;
        if let Some(image) = &entry.image {
            image.validate()?;
        }

        let txn = self.db.begin_write().map_err(store_err)?;
        let assigned_id;
        {
            verify_neighborhood_instance_in_txn(&txn, neighborhood_id, expected_instance_id)?;
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
            let encoded_photo =
                encode_framed(&photo_name, PHOTO_MAGIC, PHOTO_HASH_DOMAIN, &photo_record)?;

            index.next_photo_id = next_id;
            index.order.push(next_id);
            let index_name = format!("{neighborhood_id}:family{family_id}:album-index");
            let encoded_index = encode_framed(&index_name, INDEX_MAGIC, INDEX_HASH_DOMAIN, &index)?;

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

    /// Updates the title and caption of an existing photo in
    /// `(neighborhood_id, family_id)` atomically, fenced by `expected_instance_id`.
    pub fn update_text(
        &self,
        neighborhood_id: &str,
        expected_instance_id: u128,
        family_id: u16,
        photo_id: u64,
        title: &str,
        caption: &str,
    ) -> std::result::Result<AlbumPhoto, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        validate_instance_id(expected_instance_id)?;
        validate_family_id(family_id)?;
        if photo_id == 0 {
            return Err(AlbumError::InvalidInput("photo_id must be non-zero".into()));
        }
        validate_text(title, caption)?;

        let txn = self.db.begin_write().map_err(store_err)?;
        let updated: AlbumPhotoV1;
        {
            verify_neighborhood_instance_in_txn(&txn, neighborhood_id, expected_instance_id)?;
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
            let mut photo = match photo_table
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
            };
            photo.title = title.to_owned();
            photo.caption = caption.to_owned();
            let photo_name = format!("{neighborhood_id}:family{family_id}:photo#{photo_id}");
            let encoded = encode_framed(&photo_name, PHOTO_MAGIC, PHOTO_HASH_DOMAIN, &photo)?;
            photo_table
                .insert((neighborhood_id, family_id, photo_id), encoded.as_slice())
                .map_err(store_err)?;
            updated = photo;
        }
        txn.commit().map_err(store_err)?;
        Ok(updated.into())
    }

    /// Reorders `photo_id` to zero-based `new_index` (`0..entries.len()`)
    /// inside `(neighborhood_id, family_id)` atomically, fenced by
    /// `expected_instance_id`. Returns the new deterministic photo ID order.
    pub fn move_entry(
        &self,
        neighborhood_id: &str,
        expected_instance_id: u128,
        family_id: u16,
        photo_id: u64,
        new_index: usize,
    ) -> std::result::Result<Vec<u64>, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        validate_instance_id(expected_instance_id)?;
        validate_family_id(family_id)?;
        if photo_id == 0 {
            return Err(AlbumError::InvalidInput("photo_id must be non-zero".into()));
        }

        let txn = self.db.begin_write().map_err(store_err)?;
        let new_order;
        {
            verify_neighborhood_instance_in_txn(&txn, neighborhood_id, expected_instance_id)?;
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
                let encoded = encode_framed(&index_name, INDEX_MAGIC, INDEX_HASH_DOMAIN, &index)?;
                index_table
                    .insert((neighborhood_id, family_id), encoded.as_slice())
                    .map_err(store_err)?;
            }
            new_order = index.order;
        }
        txn.commit().map_err(store_err)?;
        Ok(new_order)
    }

    /// Deletes a single photo entry from `(neighborhood_id, family_id)`
    /// atomically, fenced by `expected_instance_id`, while preserving
    /// `next_photo_id` so deleted IDs are never reused. Returns `true` if the
    /// photo existed.
    pub fn delete(
        &self,
        neighborhood_id: &str,
        expected_instance_id: u128,
        family_id: u16,
        photo_id: u64,
    ) -> std::result::Result<bool, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        validate_instance_id(expected_instance_id)?;
        validate_family_id(family_id)?;
        if photo_id == 0 {
            return Err(AlbumError::InvalidInput("photo_id must be non-zero".into()));
        }

        let txn = self.db.begin_write().map_err(store_err)?;
        let existed;
        {
            verify_neighborhood_instance_in_txn(&txn, neighborhood_id, expected_instance_id)?;
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
            let encoded = encode_framed(&index_name, INDEX_MAGIC, INDEX_HASH_DOMAIN, &index)?;
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

    /// Removes the entire photo album (index and all photo rows) for
    /// `(neighborhood_id, family_id)` inside a single write transaction fenced
    /// by `expected_instance_id`. Returns `true` if any album data existed.
    pub fn delete_family_album(
        &self,
        neighborhood_id: &str,
        expected_instance_id: u128,
        family_id: u16,
    ) -> std::result::Result<bool, AlbumError> {
        validate_neighborhood_id(neighborhood_id)?;
        validate_instance_id(expected_instance_id)?;
        validate_family_id(family_id)?;

        let txn = self.db.begin_write().map_err(store_err)?;
        let mut existed = false;
        {
            verify_neighborhood_instance_in_txn(&txn, neighborhood_id, expected_instance_id)?;
            let mut index_table = txn.open_table(ALBUM_INDEX).map_err(store_err)?;
            if index_table
                .remove((neighborhood_id, family_id))
                .map_err(store_err)?
                .is_some()
            {
                existed = true;
            }
            let mut photo_table = txn.open_table(ALBUM_PHOTOS).map_err(store_err)?;
            let mut photo_ids = Vec::new();
            for item in photo_table
                .range((neighborhood_id, family_id, 0u64)..=(neighborhood_id, family_id, u64::MAX))
                .map_err(store_err)?
            {
                let (k, _) = item.map_err(store_err)?;
                photo_ids.push(k.value().2);
            }
            if !photo_ids.is_empty() {
                existed = true;
            }
            for photo_id in photo_ids {
                photo_table
                    .remove((neighborhood_id, family_id, photo_id))
                    .map_err(store_err)?;
            }
        }
        txn.commit().map_err(store_err)?;
        Ok(existed)
    }

    /// Lists all photo album entries for `(neighborhood_id, family_id)` in
    /// deterministic page order inside a single MVCC read transaction.
    pub fn list(
        &self,
        neighborhood_id: &str,
        family_id: u16,
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

        let mut entries = Vec::with_capacity(index.order.len());
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

/// Removes all photo album indices and photo rows for `neighborhood_id` inside
/// an open REDB write transaction.
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
    Ok(())
}
