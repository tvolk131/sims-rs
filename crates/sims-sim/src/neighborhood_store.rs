//! Multi-neighborhood and multi-lot REDB persistence store.
//!
//! Imports shipped neighborhood directories (`UserData`, `UserData2`..`UserData8`)
//! into a single REDB database without modifying any install files, and persists
//! neighborhood-wide records (`NGBH`, `FAMI`, `NBRS`, `TATT`, `LotZoning`,
//! `Characters/*.iff`, runtime-made characters) together with per-lot base
//! `HouseXX.iff` blobs and saved lot snapshots atomically.
//!
//! All valuation, net-worth, and writeback formulas match `Sims.exe` 1.91:
//! - Lot total value (`0x4253ff`, `0x4272c5`, `0x427f78`, `0x4282bc`):
//!   `lot_value + objects_value + trunc(architecture_value as f32 * mult)`,
//!   where `mult` is `Global.far` `FCNS #4` `"architecture value multiplier"`
//!   (`0.7f`, `0x3f333333`).
//! - Family net worth (`0x4272c5` / `0x410271`):
//!   `family.budget + family.architecture_value` (when not currently inside
//!   that family's lot).
//! - Neighborhood save writeback (`0x428687` → `0x4252cc`):
//!   flushes `FAMI` (`budget`, `architecture_value`, `house`, `friends`,
//!   `flags`, `members`, `magic_history`), `NBRS` (slots, person data,
//!   relationships), `NGBH` (16 data words, inventories, options), and `TATT`.

use crate::album_store::PhotoAlbumStore;
use crate::autosave::{AutosavePolicy, AutosaveStore};
use crate::characters::Characters;
use crate::migration::{
    MigratedRecord, MigrationError, MigrationReport, NEIGHBORHOOD_MIGRATIONS,
    NeighborhoodSchemaStatus, StoreMigrationError, StoreMigrationReport,
    decode_neighborhood_strict, decode_neighborhood_with_report,
};
use crate::{Error, Result, World};
use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_content::Neighborhood;
use sims_core::{Ext, F32};
use sims_formats::iff::Iff;
use sims_formats::lot::Simulator;
use sims_formats::neighborhood::{
    Family, Inventory, InventoryItem, Neighbor, Relationship as SavedRelationship,
};
use sims_formats::tatt::{Entry as TattEntry, Tatt};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Key inside [`NEIGHBORHOOD_META`] holding this REDB store's persistent
/// 128-bit identity (little-endian bytes).
const STORE_ID_KEY: &str = "store_id";

static NEXT_UNIQUE_ID: AtomicU64 = AtomicU64::new(1);

/// Generates a fresh 128-bit identity unique across re-imports and separate
/// store files without touching the wall clock.
fn fresh_unique_id(tag: &str, previous: u128, seed: &[u8]) -> u128 {
    let seq = NEXT_UNIQUE_ID.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(tag.as_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(previous.to_le_bytes());
    hasher.update(seq.to_le_bytes());
    hasher.update(seed);
    let digest = hasher.finalize();
    let bytes: [u8; 16] = digest[..16].try_into().expect("sha256 is 32 bytes");
    let mut id = u128::from_le_bytes(bytes);
    if id == 0 || id == previous {
        id = previous.wrapping_add(1);
        if id == 0 {
            id = 1;
        }
    }
    id
}

/// Reason a checked neighborhood write was rejected inside its REDB write
/// transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckedCommitError {
    /// The neighborhood ID was re-imported or replaced by a different instance.
    InstanceMismatch { expected: u128, current: u128 },
    /// Another operation committed a newer revision to the same instance.
    RevisionMismatch { expected: u64, current: u64 },
}

/// Versioned `StoredNeighborhood` metadata keyed by neighborhood ID.
pub(crate) const NEIGHBORHOODS: TableDefinition<&str, &[u8]> =
    TableDefinition::new("neighborhoods");

/// Raw immutable `Characters/*.iff` bytes keyed by `(neighborhood_id, filename)`.
const NEIGHBORHOOD_CHARACTERS: TableDefinition<(&str, &str), &[u8]> =
    TableDefinition::new("neighborhood_characters");

/// Raw base `Houses/HouseXX.iff` bytes keyed by `(neighborhood_id, house_number)`.
const NEIGHBORHOOD_HOUSES: TableDefinition<(&str, u32), &[u8]> =
    TableDefinition::new("neighborhood_houses");

/// Latest saved lot bytes (`sims_vm::boot::save::Saved::encode`) keyed by
/// `(neighborhood_id, house_number)`.
const NEIGHBORHOOD_LOTS: TableDefinition<(&str, u32), &[u8]> =
    TableDefinition::new("neighborhood_lots");

/// Persisted custom neighborhood map lot thumbnails keyed by
/// `(neighborhood_id, lot_id)`.
const NEIGHBORHOOD_LOT_THUMBNAILS: TableDefinition<(&str, u32), &[u8]> =
    TableDefinition::new("neighborhood_lot_thumbnails");

/// 4-byte magic prefix for framed [`StoredLotThumbnail`] rows.
const LOT_THUMBNAIL_MAGIC: [u8; 4] = *b"THMB";

/// Domain separator for [`StoredLotThumbnail`] outer SHA-256 checksums.
const LOT_THUMBNAIL_HASH_DOMAIN: &[u8] = b"sims-sim:lot-thumbnail:v1";

/// Length of the fixed lot-thumbnail envelope header (magic + version + SHA-256).
const LOT_THUMBNAIL_HEADER_LEN: usize = 4 + 4 + 32;

/// Earliest supported [`StoredLotThumbnail`] wire format version.
pub const LOT_THUMBNAIL_MIN_FORMAT: u32 = 1;

/// Current [`StoredLotThumbnail`] wire format version (V2: deflate-compressed
/// raw little-endian RGB565 pixels without postcard varint bloat).
pub const LOT_THUMBNAIL_FORMAT: u32 = 2;

/// Maximum width or height accepted for a persisted lot map thumbnail surface
/// (matching `sims-render`'s `MAX_LOT_CELL_DIM`).
pub const MAX_LOT_THUMBNAIL_DIM: u32 = 1024;

/// Maximum pixel count accepted for a single persisted lot map thumbnail surface.
pub const MAX_LOT_THUMBNAIL_PIXELS: usize = 1024 * 1024;

/// Base maximum encoded byte length (excluding `neighborhood_id.len()`) accepted
/// for a single lot thumbnail row before hashing or deserializing (header +
/// metadata + up to 2 surfaces × `1024 × 1024` varint-encoded `u16`s at <= 3
/// bytes each).
pub const MAX_LOT_THUMBNAIL_ENCODED_BYTES: usize =
    LOT_THUMBNAIL_HEADER_LEN + 8 * 1024 + 2 * 3 * MAX_LOT_THUMBNAIL_PIXELS;

/// Default aggregate decoded RGB565 pixel-buffer memory budget enforced by
/// [`NeighborhoodStore::load_lot_thumbnails`] (32 MiB). Callers needing to
/// inspect subsets of large neighborhoods should use
/// [`NeighborhoodStore::load_lot_thumbnails_for`] or
/// [`NeighborhoodStore::load_lot_thumbnail`].
pub const DEFAULT_LOT_THUMBNAILS_MAX_BYTES: usize = 32 * 1024 * 1024;

fn checked_thumbnail_pixel_count(width: u32, height: u32) -> Result<usize> {
    if width == 0 || height == 0 {
        return Err(Error::Store(
            "lot thumbnail surface dimensions must be non-zero".into(),
        ));
    }
    if width > MAX_LOT_THUMBNAIL_DIM || height > MAX_LOT_THUMBNAIL_DIM {
        return Err(Error::Store(format!(
            "lot thumbnail dimensions {width}x{height} exceed maximum {MAX_LOT_THUMBNAIL_DIM}x{MAX_LOT_THUMBNAIL_DIM}"
        )));
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| Error::Store("lot thumbnail pixel count overflows".into()))?;
    if expected > MAX_LOT_THUMBNAIL_PIXELS {
        return Err(Error::Store(format!(
            "lot thumbnail pixel count {expected} exceeds maximum {MAX_LOT_THUMBNAIL_PIXELS}"
        )));
    }
    Ok(expected)
}

/// A bounded 16-bit RGB565 surface for a persisted neighborhood map lot
/// thumbnail (`0xf81f` is the native magenta color key).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThumbnailSurface565 {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u16>,
}

struct ExactThumbnailPixelsSeed {
    expected: usize,
}

impl<'de> serde::de::DeserializeSeed<'de> for ExactThumbnailPixelsSeed {
    type Value = Vec<u16>;

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Self::Value, D::Error> {
        struct ExactThumbnailPixelsVisitor {
            expected: usize,
        }

        impl<'de> serde::de::Visitor<'de> for ExactThumbnailPixelsVisitor {
            type Value = Vec<u16>;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
                        "lot thumbnail pixel count {hint} does not match expected {}",
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
                        "lot thumbnail pixel count exceeds expected {}",
                        self.expected
                    )));
                }
                Ok(out)
            }
        }

        deserializer.deserialize_seq(ExactThumbnailPixelsVisitor {
            expected: self.expected,
        })
    }
}

impl<'de> Deserialize<'de> for ThumbnailSurface565 {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct SurfaceVisitor;

        impl<'de> serde::de::Visitor<'de> for SurfaceVisitor {
            type Value = ThumbnailSurface565;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("ThumbnailSurface565 { width, height, pixels }")
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
                let expected = checked_thumbnail_pixel_count(width, height)
                    .map_err(serde::de::Error::custom)?;
                let pixels = seq
                    .next_element_seed(ExactThumbnailPixelsSeed { expected })?
                    .ok_or_else(|| serde::de::Error::invalid_length(2, &self))?;
                Ok(ThumbnailSurface565 {
                    width,
                    height,
                    pixels,
                })
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                struct BoundedPixelsSeed;
                impl<'de> serde::de::DeserializeSeed<'de> for BoundedPixelsSeed {
                    type Value = Vec<u16>;
                    fn deserialize<D: serde::Deserializer<'de>>(
                        self,
                        deserializer: D,
                    ) -> std::result::Result<Self::Value, D::Error> {
                        struct V;
                        impl<'de> serde::de::Visitor<'de> for V {
                            type Value = Vec<u16>;
                            fn expecting(
                                &self,
                                f: &mut std::fmt::Formatter<'_>,
                            ) -> std::fmt::Result {
                                write!(f, "at most {MAX_LOT_THUMBNAIL_PIXELS} RGB565 u16 pixels")
                            }
                            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                                self,
                                mut seq: A,
                            ) -> std::result::Result<Self::Value, A::Error>
                            {
                                if let Some(hint) = seq.size_hint()
                                    && hint > MAX_LOT_THUMBNAIL_PIXELS
                                {
                                    return Err(serde::de::Error::custom(format!(
                                        "lot thumbnail pixel count {hint} exceeds maximum {MAX_LOT_THUMBNAIL_PIXELS}"
                                    )));
                                }
                                let cap = seq
                                    .size_hint()
                                    .map_or(0, |h| h.min(MAX_LOT_THUMBNAIL_PIXELS));
                                let mut out = Vec::with_capacity(cap);
                                while let Some(px) = seq.next_element::<u16>()? {
                                    if out.len() >= MAX_LOT_THUMBNAIL_PIXELS {
                                        return Err(serde::de::Error::custom(format!(
                                            "lot thumbnail pixel count exceeds maximum {MAX_LOT_THUMBNAIL_PIXELS}"
                                        )));
                                    }
                                    out.push(px);
                                }
                                Ok(out)
                            }
                        }
                        deserializer.deserialize_seq(V)
                    }
                }

                let mut width: Option<u32> = None;
                let mut height: Option<u32> = None;
                let mut pixels: Option<Vec<u16>> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "width" => {
                            if width.is_some() {
                                return Err(serde::de::Error::duplicate_field("width"));
                            }
                            width = Some(map.next_value()?);
                        }
                        "height" => {
                            if height.is_some() {
                                return Err(serde::de::Error::duplicate_field("height"));
                            }
                            height = Some(map.next_value()?);
                        }
                        "pixels" => {
                            if pixels.is_some() {
                                return Err(serde::de::Error::duplicate_field("pixels"));
                            }
                            if let (Some(w), Some(h)) = (width, height) {
                                let expected = checked_thumbnail_pixel_count(w, h)
                                    .map_err(serde::de::Error::custom)?;
                                pixels = Some(
                                    map.next_value_seed(ExactThumbnailPixelsSeed { expected })?,
                                );
                            } else {
                                pixels = Some(map.next_value_seed(BoundedPixelsSeed)?);
                            }
                        }
                        _ => {
                            let _: serde::de::IgnoredAny = map.next_value()?;
                        }
                    }
                }
                let width = width.ok_or_else(|| serde::de::Error::missing_field("width"))?;
                let height = height.ok_or_else(|| serde::de::Error::missing_field("height"))?;
                let pixels = pixels.ok_or_else(|| serde::de::Error::missing_field("pixels"))?;
                ThumbnailSurface565::new(width, height, pixels).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_struct(
            "ThumbnailSurface565",
            &["width", "height", "pixels"],
            SurfaceVisitor,
        )
    }
}

impl ThumbnailSurface565 {
    /// Constructs a validated RGB565 thumbnail surface.
    pub fn new(width: u32, height: u32, pixels: Vec<u16>) -> Result<Self> {
        let surface = Self {
            width,
            height,
            pixels,
        };
        surface.validate()?;
        Ok(surface)
    }

    /// Size of the decoded RGB565 pixel buffer in bytes (`pixels.len() * 2`).
    #[must_use]
    pub fn pixel_bytes(&self) -> usize {
        self.pixels.len().saturating_mul(2)
    }

    /// Validates non-zero bounded dimensions and exact pixel count.
    pub fn validate(&self) -> Result<()> {
        let expected = checked_thumbnail_pixel_count(self.width, self.height)?;
        if self.pixels.len() != expected {
            return Err(Error::Store(format!(
                "lot thumbnail pixel count {} does not match {}x{} ({expected})",
                self.pixels.len(),
                self.width,
                self.height
            )));
        }
        Ok(())
    }
}

/// Persisted custom neighborhood map thumbnail for one lot (`normal` cell and
/// optional `roofs_off` cell), matching `cWinLot`'s two 5-6-5 thumbnail planes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredLotThumbnail {
    pub normal: ThumbnailSurface565,
    pub roofs_off: Option<ThumbnailSurface565>,
}

impl StoredLotThumbnail {
    /// Validates both the required `normal` surface and optional `roofs_off`
    /// surface.
    pub fn validate(&self) -> Result<()> {
        self.normal.validate()?;
        if let Some(roofs_off) = &self.roofs_off {
            roofs_off.validate()?;
        }
        Ok(())
    }

    /// Combined decoded RGB565 pixel-buffer size in bytes across `normal` and
    /// optional `roofs_off`.
    #[must_use]
    pub fn pixel_bytes(&self) -> usize {
        self.normal.pixel_bytes().saturating_add(
            self.roofs_off
                .as_ref()
                .map_or(0, ThumbnailSurface565::pixel_bytes),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct LotThumbnailRecordV1 {
    neighborhood_id: String,
    lot_id: u32,
    normal: ThumbnailSurface565,
    roofs_off: Option<ThumbnailSurface565>,
}

/// V2 compact thumbnail surface: raw little-endian RGB565 pixel bytes
/// compressed with `flate2::Compression::fast()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ThumbnailSurface565V2 {
    width: u32,
    height: u32,
    deflated_rgb565_le: Vec<u8>,
}

impl ThumbnailSurface565V2 {
    fn from_surface(surface: &ThumbnailSurface565) -> Result<Self> {
        let mut raw = Vec::with_capacity(surface.pixels.len().saturating_mul(2));
        for &px in &surface.pixels {
            raw.extend_from_slice(&px.to_le_bytes());
        }
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::fast());
        encoder
            .write_all(&raw)
            .map_err(|e| Error::Snapshot(format!("compress V2 lot thumbnail: {e}")))?;
        let deflated_rgb565_le = encoder
            .finish()
            .map_err(|e| Error::Snapshot(format!("finish V2 lot thumbnail compression: {e}")))?;
        Ok(Self {
            width: surface.width,
            height: surface.height,
            deflated_rgb565_le,
        })
    }

    fn into_surface(self) -> Result<ThumbnailSurface565> {
        let expected_pixels = checked_thumbnail_pixel_count(self.width, self.height)?;
        let expected_bytes = expected_pixels
            .checked_mul(2)
            .ok_or_else(|| Error::Snapshot("lot thumbnail RGB565 byte count overflows".into()))?;
        let mut decoder = DeflateDecoder::new(self.deflated_rgb565_le.as_slice())
            .take((expected_bytes as u64).saturating_add(1));
        let mut raw = Vec::with_capacity(expected_bytes);
        decoder
            .read_to_end(&mut raw)
            .map_err(|e| Error::Snapshot(format!("decompress V2 lot thumbnail: {e}")))?;
        if raw.len() != expected_bytes {
            return Err(Error::Snapshot(format!(
                "decompressed V2 lot thumbnail byte length {} does not match expected {expected_bytes}",
                raw.len()
            )));
        }
        let consumed = decoder.into_inner().total_in();
        if consumed != self.deflated_rgb565_le.len() as u64 {
            return Err(Error::Snapshot(format!(
                "V2 lot thumbnail compressed stream has trailing bytes (consumed {consumed} of {} bytes)",
                self.deflated_rgb565_le.len()
            )));
        }
        let mut pixels = Vec::with_capacity(expected_pixels);
        for chunk in raw.as_chunks::<2>().0 {
            pixels.push(u16::from_le_bytes(*chunk));
        }
        Ok(ThumbnailSurface565 {
            width: self.width,
            height: self.height,
            pixels,
        })
    }
}

struct BoundedThumbnailBytesSeed {
    max_len: usize,
}

impl<'de> serde::de::DeserializeSeed<'de> for BoundedThumbnailBytesSeed {
    type Value = Vec<u8>;

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Self::Value, D::Error> {
        struct Visitor {
            max_len: usize,
        }

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Vec<u8>;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "at most {} bytes", self.max_len)
            }

            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> std::result::Result<Vec<u8>, E> {
                if v.len() > self.max_len {
                    return Err(E::custom(format!(
                        "compressed thumbnail buffer len {} exceeds maximum {}",
                        v.len(),
                        self.max_len
                    )));
                }
                Ok(v.to_vec())
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                if let Some(hint) = seq.size_hint()
                    && hint > self.max_len
                {
                    return Err(serde::de::Error::custom(format!(
                        "compressed thumbnail buffer len {hint} exceeds maximum {}",
                        self.max_len
                    )));
                }
                let cap = seq.size_hint().map_or(0, |h| h.min(self.max_len));
                let mut out = Vec::with_capacity(cap);
                while let Some(b) = seq.next_element::<u8>()? {
                    if out.len() >= self.max_len {
                        return Err(serde::de::Error::custom(format!(
                            "compressed thumbnail buffer exceeds maximum {}",
                            self.max_len
                        )));
                    }
                    out.push(b);
                }
                Ok(out)
            }
        }

        deserializer.deserialize_seq(Visitor {
            max_len: self.max_len,
        })
    }
}

impl<'de> Deserialize<'de> for ThumbnailSurface565V2 {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct SurfaceV2Visitor;

        impl<'de> serde::de::Visitor<'de> for SurfaceV2Visitor {
            type Value = ThumbnailSurface565V2;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("ThumbnailSurface565V2 { width, height, deflated_rgb565_le }")
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
                let expected_pixels = checked_thumbnail_pixel_count(width, height)
                    .map_err(serde::de::Error::custom)?;
                let max_len = expected_pixels
                    .saturating_mul(2)
                    .saturating_add(4096)
                    .min(MAX_LOT_THUMBNAIL_ENCODED_BYTES);
                let deflated_rgb565_le = seq
                    .next_element_seed(BoundedThumbnailBytesSeed { max_len })?
                    .ok_or_else(|| serde::de::Error::invalid_length(2, &self))?;
                Ok(ThumbnailSurface565V2 {
                    width,
                    height,
                    deflated_rgb565_le,
                })
            }
        }

        deserializer.deserialize_struct(
            "ThumbnailSurface565V2",
            &["width", "height", "deflated_rgb565_le"],
            SurfaceV2Visitor,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct LotThumbnailRecordV2 {
    neighborhood_id: String,
    lot_id: u32,
    normal: ThumbnailSurface565V2,
    roofs_off: Option<ThumbnailSurface565V2>,
}

struct LotThumbnailRecordV2Seed<'a> {
    expected_neighborhood_id: &'a str,
}

impl<'de> serde::de::DeserializeSeed<'de> for LotThumbnailRecordV2Seed<'_> {
    type Value = LotThumbnailRecordV2;

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Self::Value, D::Error> {
        struct RecordVisitor<'a> {
            expected_neighborhood_id: &'a str,
        }

        impl<'de> serde::de::Visitor<'de> for RecordVisitor<'_> {
            type Value = LotThumbnailRecordV2;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("LotThumbnailRecordV2")
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                seq.next_element_seed(ExpectedThumbnailIdSeed {
                    expected: self.expected_neighborhood_id,
                })?
                .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
                let lot_id: u32 = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
                let normal: ThumbnailSurface565V2 = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(2, &self))?;
                let roofs_off: Option<ThumbnailSurface565V2> = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(3, &self))?;
                Ok(LotThumbnailRecordV2 {
                    neighborhood_id: self.expected_neighborhood_id.to_owned(),
                    lot_id,
                    normal,
                    roofs_off,
                })
            }
        }

        deserializer.deserialize_struct(
            "LotThumbnailRecordV2",
            &["neighborhood_id", "lot_id", "normal", "roofs_off"],
            RecordVisitor {
                expected_neighborhood_id: self.expected_neighborhood_id,
            },
        )
    }
}

struct ExpectedThumbnailIdSeed<'a> {
    expected: &'a str,
}

impl<'de> serde::de::DeserializeSeed<'de> for ExpectedThumbnailIdSeed<'_> {
    type Value = ();

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<(), D::Error> {
        struct IdVisitor<'a> {
            expected: &'a str,
        }
        impl<'de> serde::de::Visitor<'de> for IdVisitor<'_> {
            type Value = ();
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("expected thumbnail neighborhood_id key string")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<(), E> {
                if v != self.expected {
                    return Err(E::custom(format!(
                        "key binding mismatch: neighborhood_id len {} != expected len {}",
                        v.len(),
                        self.expected.len()
                    )));
                }
                Ok(())
            }
        }
        deserializer.deserialize_str(IdVisitor {
            expected: self.expected,
        })
    }
}

struct LotThumbnailRecordSeed<'a> {
    expected_neighborhood_id: &'a str,
}

impl<'de> serde::de::DeserializeSeed<'de> for LotThumbnailRecordSeed<'_> {
    type Value = LotThumbnailRecordV1;

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Self::Value, D::Error> {
        struct RecordVisitor<'a> {
            expected_neighborhood_id: &'a str,
        }

        impl<'de> serde::de::Visitor<'de> for RecordVisitor<'_> {
            type Value = LotThumbnailRecordV1;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("LotThumbnailRecordV1")
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                seq.next_element_seed(ExpectedThumbnailIdSeed {
                    expected: self.expected_neighborhood_id,
                })?
                .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
                let lot_id: u32 = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
                let normal: ThumbnailSurface565 = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(2, &self))?;
                let roofs_off: Option<ThumbnailSurface565> = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(3, &self))?;
                Ok(LotThumbnailRecordV1 {
                    neighborhood_id: self.expected_neighborhood_id.to_owned(),
                    lot_id,
                    normal,
                    roofs_off,
                })
            }
        }

        deserializer.deserialize_struct(
            "LotThumbnailRecordV1",
            &["neighborhood_id", "lot_id", "normal", "roofs_off"],
            RecordVisitor {
                expected_neighborhood_id: self.expected_neighborhood_id,
            },
        )
    }
}

fn lot_thumbnail_checksum(version: u32, body: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(LOT_THUMBNAIL_HASH_DOMAIN);
    hasher.update(version.to_le_bytes());
    hasher.update(body);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn encode_lot_thumbnail(
    neighborhood_id: &str,
    lot_id: u32,
    thumbnail: &StoredLotThumbnail,
) -> Result<Vec<u8>> {
    thumbnail.validate()?;
    let record = LotThumbnailRecordV2 {
        neighborhood_id: neighborhood_id.to_owned(),
        lot_id,
        normal: ThumbnailSurface565V2::from_surface(&thumbnail.normal)?,
        roofs_off: match &thumbnail.roofs_off {
            Some(surface) => Some(ThumbnailSurface565V2::from_surface(surface)?),
            None => None,
        },
    };
    let body = postcard::to_allocvec(&record).map_err(|e| {
        Error::Snapshot(format!(
            "encode lot thumbnail ({neighborhood_id}, {lot_id}): {e}"
        ))
    })?;
    let total_len = LOT_THUMBNAIL_HEADER_LEN.saturating_add(body.len());
    let max_bytes = MAX_LOT_THUMBNAIL_ENCODED_BYTES.saturating_add(neighborhood_id.len());
    if total_len > max_bytes {
        return Err(Error::Store(format!(
            "encoded lot thumbnail size {total_len} exceeds maximum {max_bytes}"
        )));
    }
    let checksum = lot_thumbnail_checksum(LOT_THUMBNAIL_FORMAT, &body);
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(&LOT_THUMBNAIL_MAGIC);
    out.extend_from_slice(&LOT_THUMBNAIL_FORMAT.to_le_bytes());
    out.extend_from_slice(&checksum);
    out.extend_from_slice(&body);
    Ok(out)
}

fn decode_lot_thumbnail(
    neighborhood_id: &str,
    lot_id: u32,
    bytes: &[u8],
) -> Result<StoredLotThumbnail> {
    let record_name = format!("{neighborhood_id}:lot{lot_id}:thumbnail");
    if bytes.len() < 8 {
        return Err(Error::Snapshot(format!(
            "{record_name}: shorter than 8-byte header ({} bytes)",
            bytes.len()
        )));
    }
    if bytes[0..4] != LOT_THUMBNAIL_MAGIC {
        return Err(Error::Snapshot(format!(
            "{record_name}: invalid lot thumbnail magic"
        )));
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes"));
    if !(LOT_THUMBNAIL_MIN_FORMAT..=LOT_THUMBNAIL_FORMAT).contains(&version) {
        return Err(Error::Snapshot(format!(
            "{record_name}: unsupported lot thumbnail format {version} (supported {LOT_THUMBNAIL_MIN_FORMAT}..={LOT_THUMBNAIL_FORMAT})"
        )));
    }
    let max_bytes = MAX_LOT_THUMBNAIL_ENCODED_BYTES.saturating_add(neighborhood_id.len());
    if bytes.len() > max_bytes {
        return Err(Error::Snapshot(format!(
            "{record_name}: encoded lot thumbnail size {} exceeds maximum {max_bytes}",
            bytes.len()
        )));
    }
    if bytes.len() < LOT_THUMBNAIL_HEADER_LEN {
        return Err(Error::Snapshot(format!(
            "{record_name}: shorter than {LOT_THUMBNAIL_HEADER_LEN}-byte header ({} bytes)",
            bytes.len()
        )));
    }
    let expected: [u8; 32] = bytes[8..40].try_into().expect("32 bytes");
    let body = &bytes[LOT_THUMBNAIL_HEADER_LEN..];
    let actual = lot_thumbnail_checksum(version, body);
    if actual != expected {
        return Err(Error::Snapshot(format!(
            "{record_name}: corrupt lot thumbnail checksum"
        )));
    }
    let mut de = postcard::Deserializer::from_bytes(body);
    let (stored_neighborhood, stored_lot, normal, roofs_off) = match version {
        1 => {
            let decoded = serde::de::DeserializeSeed::deserialize(
                LotThumbnailRecordSeed {
                    expected_neighborhood_id: neighborhood_id,
                },
                &mut de,
            )
            .map_err(|e| Error::Snapshot(format!("{record_name}: {e}")))?;
            (
                decoded.neighborhood_id,
                decoded.lot_id,
                decoded.normal,
                decoded.roofs_off,
            )
        }
        2 => {
            let decoded = serde::de::DeserializeSeed::deserialize(
                LotThumbnailRecordV2Seed {
                    expected_neighborhood_id: neighborhood_id,
                },
                &mut de,
            )
            .map_err(|e| Error::Snapshot(format!("{record_name}: {e}")))?;
            let normal = decoded.normal.into_surface()?;
            let roofs_off = match decoded.roofs_off {
                Some(surface) => Some(surface.into_surface()?),
                None => None,
            };
            (decoded.neighborhood_id, decoded.lot_id, normal, roofs_off)
        }
        _ => {
            return Err(Error::Snapshot(format!(
                "{record_name}: unsupported lot thumbnail format {version} (supported {LOT_THUMBNAIL_MIN_FORMAT}..={LOT_THUMBNAIL_FORMAT})"
            )));
        }
    };
    let trailing = de
        .finalize()
        .map_err(|e| Error::Snapshot(format!("{record_name}: {e}")))?;
    if !trailing.is_empty() {
        return Err(Error::Snapshot(format!(
            "{record_name}: {} unexpected trailing byte(s)",
            trailing.len()
        )));
    }
    if stored_neighborhood != neighborhood_id || stored_lot != lot_id {
        return Err(Error::Snapshot(format!(
            "{record_name}: key binding mismatch: stored ({stored_neighborhood:?}, lot {stored_lot}), expected ({neighborhood_id:?}, lot {lot_id})"
        )));
    }
    Ok(StoredLotThumbnail { normal, roofs_off })
}

fn purge_lot_thumbnails_in_txn(txn: &redb::WriteTransaction, id: &str) -> Result<()> {
    let mut table = txn.open_table(NEIGHBORHOOD_LOT_THUMBNAILS).map_err(store)?;
    let mut lots = Vec::new();
    for item in table.range((id, 0u32)..=(id, u32::MAX)).map_err(store)? {
        let (k, _) = item.map_err(store)?;
        lots.push(k.value().1);
    }
    for lot_id in lots {
        table.remove((id, lot_id)).map_err(store)?;
    }
    Ok(())
}

/// Atomically purges photo albums and advances `photo_album_family_epoch` for
/// every non-zero family removed between `previous` and `next`, AND purges any
/// preexisting orphan album rows for newly allocated non-zero families in
/// `next` (e.g. from older databases before transactional family-delete album
/// cleanup), inside the same REDB write transaction as the neighborhood commit.
fn purge_removed_family_albums_in_txn(
    txn: &redb::WriteTransaction,
    id: &str,
    previous: &StoredNeighborhood,
    next: &StoredNeighborhood,
) -> Result<()> {
    for &family_id in previous.families.keys() {
        if family_id != 0 && !next.families.contains_key(&family_id) {
            crate::album_store::purge_family_in_txn(txn, id, family_id)?;
        }
    }
    for &family_id in next.families.keys() {
        if family_id != 0 && !previous.families.contains_key(&family_id) {
            crate::album_store::purge_orphan_family_in_txn(txn, id, family_id)?;
        }
    }
    Ok(())
}

/// Store-level metadata (e.g. active neighborhood ID).
const NEIGHBORHOOD_META: TableDefinition<&str, &[u8]> = TableDefinition::new("neighborhood_meta");

/// Key inside [`NEIGHBORHOOD_META`] holding the active neighborhood ID as UTF-8.
const ACTIVE_KEY: &str = "active";

/// Key inside [`NEIGHBORHOOD_META`] holding a monotonic 64-bit counter used so
/// every created, recreated, re-imported, or rolled-over neighborhood instance
/// inside this REDB file receives a strictly unique 128-bit `instance_id`.
const INSTANCE_SEQ_KEY: &str = "instance_seq";

/// Key inside [`NEIGHBORHOOD_META`] recording the last atomically migrated
/// neighborhood schema version (4-byte little-endian `u32`).
const SCHEMA_VERSION_KEY: &str = "schema_version";

/// Current [`StoredNeighborhood`] schema version. Older versions
/// (`NEIGHBORHOOD_MIN_FORMAT..NEIGHBORHOOD_FORMAT`) are upgraded deterministically
/// via [`crate::migration::NEIGHBORHOOD_MIGRATIONS`] (format 2 added
/// `instance_id` and `deleted_characters`; format 3 added `deletion_epoch` and
/// `deleted_at_epoch`).
pub const NEIGHBORHOOD_FORMAT: u32 = 3;

/// Default `"architecture value multiplier"` from `Global.far` `FCNS #4`
/// (`0x6e4a88`, `0x3f333333`).
pub const DEFAULT_ARCH_VALUE_MULTIPLIER: f32 = 0.7_f32;

/// Default `"new family start hour"` from `Global.far` `FCNS #4` (`0x6e4a84`).
pub const DEFAULT_NEW_FAMILY_START_HOUR: i16 = 7;

/// Default `"fill value for layout history"` from `Global.far` `FCNS #4`
/// (`0x6e4a78`, used by `0x4165f3` on move-in / evict / bulldoze).
pub const DEFAULT_LAYOUT_HISTORY_FILL: i32 = 200;

/// Family record flag bit 0 (`0x1`): the lot has an object of category 1
/// (`0x4160e5`).
pub const FAMILY_FLAG_HAS_CATEGORY_1: u32 = 1 << 0;

/// Family record flag bit 1 (`0x2`): the lot has an object of category 9
/// (`0x4160f9`).
pub const FAMILY_FLAG_HAS_CATEGORY_9: u32 = 1 << 1;

/// Family record flag bit 2 (`0x4`): newly moved-in household; first entry
/// resets the clock, budget history, usage history, unpauses, and broadcasts
/// `"reset lot"` (`0x427373`, `0x428487..0x4284f9`).
pub const FAMILY_FLAG_NEW_MOVE_IN: u32 = 1 << 2;

/// Family record flag bit 3 (`0x8`): cleared on save, move-in, and eviction
/// (`0x427370`, `0x42743e`, `0x428762`).
pub const FAMILY_FLAG_BIT_3: u32 = 1 << 3;

/// Computes a lot's total value as `Sims.exe` 1.91 computes it (`0x4253ff`,
/// `0x4272c5`, `0x427f78`, `0x4282bc`):
/// `fild architecture_value; fmul dword ptr [0x6e4a88]; call __ftol`.
///
/// Uses exact 64-bit-significand x87 extended multiplication (`31 + 24 = 55 <=
/// 64` bits, zero intermediate rounding) followed by a single round-to-nearest-
/// even to 24-bit `f32` and truncation toward zero.
#[must_use]
pub fn lot_total_value(
    lot_value: i32,
    objects_value: i32,
    architecture_value: i32,
    arch_multiplier: f32,
) -> i32 {
    let scaled_arch = (Ext::from_i32(architecture_value)
        * F32::from_bits(arch_multiplier.to_bits()).ext())
    .single()
    .to_i32_truncate();
    lot_value
        .wrapping_add(objects_value)
        .wrapping_add(scaled_arch)
}

/// Computes a family's net worth outside of live lot play (`0x4272c5` /
/// `0x410271`): `family.budget + family.architecture_value`.
#[must_use]
pub fn family_net_worth(family: &Family) -> i32 {
    family.budget.wrapping_add(family.architecture_value)
}

/// Consistent point-in-time snapshot of a neighborhood and one house read in a
/// single REDB read transaction, preventing lot entry from mixing two
/// committed revisions.
#[derive(Debug, Clone)]
pub struct LotSnapshot {
    pub neighborhood: StoredNeighborhood,
    pub character_files: BTreeMap<String, Vec<u8>>,
    pub saved_lot: Option<Vec<u8>>,
    pub base_house: Option<Vec<u8>>,
}

/// Summary returned by [`NeighborhoodStore::import_directory`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSummary {
    pub id: String,
    pub families: usize,
    pub neighbors: usize,
    pub characters: usize,
    pub houses: usize,
}

/// Neighborhood-wide persistent state stored in REDB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredNeighborhood {
    pub name: String,
    /// Families by FAMI chunk ID.
    pub families: BTreeMap<u16, Family>,
    /// Neighbor records in native slot order (including empty `None` slots).
    pub neighbors: Vec<Option<Neighbor>>,
    /// Neighbor inventories from `NGBH`.
    pub inventories: Vec<Option<Inventory>>,
    /// Neighborhood's sixteen `NGBH` data words.
    pub data: [i16; 16],
    /// Neighborhood's eleven `NGBH` option booleans.
    pub options: Option<[bool; 11]>,
    /// Saved type attributes (`TATT #1`), if any.
    pub type_attributes: Option<Tatt>,
    /// Lot zoning by house number: `1` residential, `2` community.
    pub zoning: BTreeMap<i16, i32>,
    /// Lot numbers in `LotZoning.iff` row order.
    pub lot_rows: Vec<i32>,
    /// Runtime-created characters (`Make New Character`, primitive 19) persisted
    /// across lot switches.
    pub made_characters: Characters,
    /// Character GUIDs permanently deleted via Bulldoze (`0x426fcd`) so even
    /// characters residing inside read-only `.far` archives or multi-OBJD files
    /// are never re-registered into `NBRS` on subsequent lot loads.
    #[serde(default)]
    pub deleted_characters: BTreeSet<u32>,
    /// Monotonic counter incremented whenever Bulldoze deletes tombstones,
    /// paired with `deleted_at_epoch` and each saved lot's `deletion_epoch` so
    /// reusing a deleted character GUID later never revives that GUID's stale
    /// instance on an older saved lot or imported base house.
    #[serde(default)]
    pub deletion_epoch: u64,
    /// Latest `deletion_epoch` at which each character GUID was deleted via
    /// Bulldoze (retained even if a replacement character later reuses the same
    /// GUID).
    #[serde(default)]
    pub deleted_at_epoch: BTreeMap<u32, u64>,
    /// Unique 128-bit identity assigned on each import/creation so re-importing
    /// a different neighborhood under the same ID invalidates previously opened
    /// lots even when both are at revision 0.
    #[serde(default)]
    pub instance_id: u128,
    /// Monotonic revision bumped on each save writeback.
    pub revision: u64,
}

impl StoredNeighborhood {
    /// Builds a `sims_content::Neighborhood` view of this stored neighborhood
    /// so existing loaders (`World::read_neighbors`) can read it directly.
    ///
    /// Omits any non-zero empty family (`members.is_empty()`), matching
    /// `Sims.exe` `0x425efb` (house load deletes empty households) so an empty
    /// household can never block loading another lot.
    #[must_use]
    pub fn to_content_neighborhood(&self) -> Neighborhood {
        let families = self
            .families
            .iter()
            .filter(|&(&id, family)| id == 0 || !family.members.is_empty())
            .map(|(&id, family)| (id, family.clone()))
            .collect();
        Neighborhood {
            directory: PathBuf::new(),
            families,
            type_attributes: self.type_attributes.clone(),
            neighbors: self.neighbors.clone(),
            inventories: self.inventories.clone(),
            data: self.data,
            options: self.options,
        }
    }

    /// The family currently occupying `house`, matching `Sims.exe` `0x425d95`:
    /// checks both the primary `house` field (`+0x110`) and the v8+ secondary
    /// house field (`+0x13c`, `tail[0..4]`). `house == 0` returns `None`.
    #[must_use]
    pub fn family_in(&self, house: u32) -> Option<(u16, &Family)> {
        if house == 0 {
            return None;
        }
        self.families
            .iter()
            .find(|(_, family)| {
                u32::try_from(family.house).ok() == Some(house)
                    || u32::try_from(family.secondary_house()).ok() == Some(house)
            })
            .map(|(&id, family)| (id, family))
    }

    /// Mutable access to the family currently occupying `house` (checking both
    /// primary and secondary house fields per `0x425d95`), if any.
    pub fn family_in_mut(&mut self, house: u32) -> Option<(u16, &mut Family)> {
        if house == 0 {
            return None;
        }
        self.families
            .iter_mut()
            .find(|(_, family)| {
                u32::try_from(family.house).ok() == Some(house)
                    || u32::try_from(family.secondary_house()).ok() == Some(house)
            })
            .map(|(&id, family)| (id, family))
    }

    /// Playable Family Bin households (`id != 0`, `house == 0`,
    /// `secondary_house() == 0`, non-empty member list), in FAMI chunk ID order.
    #[must_use]
    pub fn family_bin(&self) -> Vec<(u16, &Family)> {
        self.families
            .iter()
            .filter(|&(&id, family)| {
                id != 0
                    && family.house == 0
                    && family.secondary_house() == 0
                    && !family.members.is_empty()
            })
            .map(|(&id, family)| (id, family))
            .collect()
    }

    /// Whether `house` is zoned residential (`1`, matching `0x427327..0x42734a`).
    #[must_use]
    pub fn is_residential(&self, house: u32) -> bool {
        i16::try_from(house)
            .ok()
            .and_then(|lot| self.zoning.get(&lot).copied())
            == Some(1)
    }

    /// Encodes this neighborhood state with the [`NEIGHBORHOOD_FORMAT`] header.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = NEIGHBORHOOD_FORMAT.to_le_bytes().to_vec();
        out.extend(postcard::to_allocvec(self).expect("encoding to memory cannot fail"));
        out
    }

    /// Decodes bytes written by [`StoredNeighborhood::encode`] (or an older
    /// supported schema version `1..=NEIGHBORHOOD_FORMAT`), applying registered
    /// forward migrations deterministically in memory.
    pub fn decode(name: &str, bytes: &[u8]) -> Result<Self> {
        Self::decode_with_report(name, bytes)
            .map(|(neighborhood, _)| neighborhood)
            .map_err(|error| Error::Snapshot(error.to_string()))
    }

    /// Decodes bytes from any supported schema version (`1..=NEIGHBORHOOD_FORMAT`)
    /// and returns both the migrated [`StoredNeighborhood`] and its
    /// [`MigrationReport`], or a structured [`MigrationError`].
    pub fn decode_with_report(
        name: &str,
        bytes: &[u8],
    ) -> std::result::Result<(Self, MigrationReport), MigrationError> {
        decode_neighborhood_with_report(name, bytes)
    }

    /// Decodes bytes strictly at [`NEIGHBORHOOD_FORMAT`] without running
    /// forward migrations for older schema versions.
    pub fn decode_strict(name: &str, bytes: &[u8]) -> std::result::Result<Self, MigrationError> {
        decode_neighborhood_strict(name, bytes)
    }

    /// Writes back live simulation neighborhood state from `world` into this
    /// persistent neighborhood record, following `Sims.exe` 1.91
    /// `SaveCurrentHouse` (`0x428687`) → `cTSNeighborhood::Save` (`0x4252cc`).
    ///
    /// Callers must have already flushed live persons via
    /// `sims_vm::people::save_to_record` (`0x425a69`) and refreshed lot values
    /// (`0x416085`) before calling this method.
    pub fn write_back_world(
        &mut self,
        world: &World,
        current_house: u32,
        family_id: u16,
        lot_value_total: i32,
        family_flags_update: Option<(bool, bool)>,
        friend_threshold: i32,
    ) {
        // 1. NGBH data words & inventories (0x42535d, 0x425371).
        self.data = world.neighborhood_data;
        if world.globals.get(26).copied().unwrap_or(0) != 0
            && current_house != 0
            && let Ok(lot_word) = i16::try_from(current_house)
        {
            self.data[2] = lot_word;
        }
        self.inventories = world
            .neighbors
            .inventories
            .iter()
            .map(|inv| {
                Some(Inventory {
                    neighbor: inv.neighbor,
                    items: inv
                        .items
                        .iter()
                        .map(|item| InventoryItem {
                            kind: item.kind,
                            guid: item.guid,
                            count: item.count,
                        })
                        .collect(),
                })
            })
            .collect();

        // 2. NBRS slots (0x4253b0): preserve stored header metadata where slot
        // identity matches, updating revision, person_data, and relationships.
        let mut updated_neighbors = Vec::with_capacity(world.neighbors.slots.len());
        for (index, slot) in world.neighbors.slots.iter().enumerate() {
            let Some(live) = slot else {
                updated_neighbors.push(None);
                continue;
            };
            let existing = self
                .neighbors
                .get(index)
                .and_then(|entry| entry.as_ref())
                .filter(|prev| prev.id == live.id && prev.guid == live.guid);
            let person_data = if live.revision > 0 {
                live.data.clone()
            } else {
                Vec::new()
            };
            let relationships = live
                .relationships
                .iter()
                .map(|row| SavedRelationship {
                    target: row.target,
                    values: row.values.clone(),
                })
                .collect();
            let saved = match existing {
                Some(prev) => Neighbor {
                    version: if person_data.len() > 80 {
                        10
                    } else {
                        prev.version
                    },
                    version_extra: if person_data.len() > 80 {
                        Some(prev.version_extra.unwrap_or(1))
                    } else {
                        prev.version_extra
                    },
                    name: prev.name.clone(),
                    unknown_zero: prev.unknown_zero,
                    person_mode: live.revision,
                    person_data,
                    id: live.id,
                    guid: live.guid,
                    unknown_negative: prev.unknown_negative,
                    relationships,
                },
                None => Neighbor {
                    version: 10,
                    version_extra: Some(1),
                    name: String::new(),
                    unknown_zero: 0,
                    person_mode: live.revision,
                    person_data,
                    id: live.id,
                    guid: live.guid,
                    unknown_negative: -1,
                    relationships,
                },
            };
            updated_neighbors.push(Some(saved));
        }
        self.neighbors = updated_neighbors;

        // 3. FAMI records (0x4286a0..0x428771, 0x4253dd..0x425453, 0x410696).
        // Native `0x410696` always serializes FAMI version 9.
        for stored in self.families.values_mut() {
            stored.upgrade_to_v9();
        }
        for live_family in &world.families {
            let Ok(id) = u16::try_from(live_family.id) else {
                continue;
            };
            let Some(stored) = self.families.get_mut(&id) else {
                continue;
            };
            stored.house = live_family.house;
            stored.members = live_family.members.clone();
            stored.friends = if live_family.stale {
                world.count_family_friends(live_family, friend_threshold)
            } else {
                live_family.friends
            };
        }
        let residential = current_house != 0 && self.is_residential(current_house);
        if family_id != 0
            && let Some(active) = self.families.get_mut(&family_id)
        {
            active.budget = world.budget.funds;
            active.flags &= !(FAMILY_FLAG_BIT_3 | FAMILY_FLAG_NEW_MOVE_IN);
            if let Some((has_cat1, has_cat9)) = family_flags_update {
                if has_cat1 {
                    active.flags |= FAMILY_FLAG_HAS_CATEGORY_1;
                } else {
                    active.flags &= !FAMILY_FLAG_HAS_CATEGORY_1;
                }
                if has_cat9 {
                    active.flags |= FAMILY_FLAG_HAS_CATEGORY_9;
                } else {
                    active.flags &= !FAMILY_FLAG_HAS_CATEGORY_9;
                }
            }
            if current_house != 0 && u32::try_from(active.house).ok() == Some(current_house) {
                active.architecture_value = lot_value_total;
            }
            // 0x410a87: merge the signed maximum of each retained magic-history
            // word and the live Magic Controller (GUID 0xb6c90029) attribute
            // on a residential lot before saving FAMI v9.
            let mut history = match &world.magic_history {
                Some(magic) if magic.family == i32::from(family_id) => magic.values,
                _ => active.magic_history().ok().flatten().unwrap_or([0; 6]),
            };
            if residential
                && world.globals.get(9).copied().map(i32::from) == Some(i32::from(family_id))
                && let Some(controller) = world
                    .order
                    .iter()
                    .filter_map(|id| world.objects.get(id))
                    .find(|obj| obj.guid == 0xb6c9_0029)
            {
                for (slot, word) in history.iter_mut().enumerate() {
                    if let Some(&attr) = controller.attributes.get(slot)
                        && attr > *word
                    {
                        *word = attr;
                    }
                }
            }
            let _ = active.set_magic_history(history);
        }
        // Drop any non-zero family that now has zero members (matching
        // `0x425efb` when the last member dies or moves out via Generic Sims
        // Call 6) so empty households never persist or block other lot loads.
        self.families
            .retain(|&id, family| id == 0 || !family.members.is_empty());

        // 4. TATT (0x425455 → 0x43e0be): merge updated world type attributes.
        if !world.type_attributes.is_empty() {
            let (version, packed, mut entries) = match self.type_attributes.take() {
                Some(prev) => (prev.version, prev.packed, prev.entries),
                None => (0, true, Vec::new()),
            };
            for record in &world.type_attributes {
                if let Some(existing) = entries.iter_mut().find(|e| e.guid == record.guid) {
                    existing.values = record.values.clone();
                } else if record.values.iter().any(|&v| v != 0) {
                    entries.push(TattEntry {
                        guid: record.guid,
                        values: record.values.clone(),
                        count_end: 0,
                        end: 0,
                    });
                }
            }
            self.type_attributes = Some(Tatt {
                version,
                packed,
                entries,
                count_end: 0,
                consumed: 0,
            });
        }

        // 5. Runtime-made characters: replace from the authoritative live world
        // so bulldozed deletions are never resurrected by older lot saves, and
        // clear any deletion tombstone superseded by a newly re-allocated GUID.
        self.made_characters = world.characters.clone();
        for guid in self.made_characters.made.keys() {
            self.deleted_characters.remove(guid);
        }

        self.revision = self.revision.saturating_add(1);
    }
}

/// A REDB-backed multi-neighborhood and multi-lot persistence store.
pub struct NeighborhoodStore {
    db: Arc<Database>,
    store_id: u128,
}

fn store(error: impl std::fmt::Display) -> Error {
    Error::Store(error.to_string())
}

/// Mixes a store's persisted database ID with its OS file identity (device +
/// inode on Unix, falling back to canonical path when unavailable) so copying a
/// `.redb` file on disk yields a distinct store identity while reopening or
/// renaming the same file on the same volume preserves it.
fn bind_store_file_identity(persisted_id: u128, canonical_path: &Path) -> u128 {
    let mut hasher = Sha256::new();
    hasher.update(b"store-file-identity");
    hasher.update(persisted_id.to_le_bytes());
    #[cfg(unix)]
    if let Ok(meta) = std::fs::metadata(canonical_path) {
        use std::os::unix::fs::MetadataExt;
        hasher.update(b"unix-dev-ino");
        hasher.update(meta.dev().to_le_bytes());
        hasher.update(meta.ino().to_le_bytes());
    } else {
        hasher.update(b"path");
        hasher.update(canonical_path.to_string_lossy().as_bytes());
    }
    #[cfg(not(unix))]
    {
        hasher.update(b"path");
        hasher.update(canonical_path.to_string_lossy().as_bytes());
    }
    let digest = hasher.finalize();
    let bytes: [u8; 16] = digest[..16].try_into().expect("sha256 is 32 bytes");
    let id = u128::from_le_bytes(bytes);
    if id == 0 { 1 } else { id }
}

/// Mints a strictly fresh non-zero 128-bit neighborhood `instance_id` using a
/// persisted monotonic counter inside [`NEIGHBORHOOD_META`], guaranteeing that
/// deleting and recreating a neighborhood (even with a retained snapshot across
/// process restarts) or rolling over `u64::MAX` never reuses an old `instance_id`.
fn mint_instance_id(
    txn: &redb::WriteTransaction,
    store_id: u128,
    prev: u128,
    id: &str,
) -> Result<u128> {
    let mut meta = txn.open_table(NEIGHBORHOOD_META).map_err(store)?;
    let seq = meta
        .get(INSTANCE_SEQ_KEY)
        .map_err(store)?
        .and_then(|v| v.value().try_into().ok())
        .map_or(0u64, u64::from_le_bytes)
        .wrapping_add(1);
    meta.insert(INSTANCE_SEQ_KEY, seq.to_le_bytes().as_slice())
        .map_err(store)?;
    let mut hasher = Sha256::new();
    hasher.update(b"neighborhood-instance");
    hasher.update(store_id.to_le_bytes());
    hasher.update(seq.to_le_bytes());
    hasher.update(prev.to_le_bytes());
    hasher.update(id.as_bytes());
    let digest = hasher.finalize();
    let bytes: [u8; 16] = digest[..16].try_into().expect("sha256 is 32 bytes");
    let candidate = u128::from_le_bytes(bytes);
    let candidate = if candidate == 0 { 1 } else { candidate };
    if candidate == prev {
        Ok(candidate.wrapping_add(1).max(1))
    } else {
        Ok(candidate)
    }
}

/// Strictly advances `(instance_id, revision)` past `(prev_instance_id, prev_revision)`:
/// - Normally keeps `prev_instance_id` and increments `revision` by 1.
/// - If `prev_revision == u64::MAX`, rotates to a brand-new `instance_id` and
///   resets `revision = 0` so stale-write protection can never saturate.
fn advance_version(
    txn: &redb::WriteTransaction,
    store_id: u128,
    id: &str,
    prev_instance_id: u128,
    prev_revision: u64,
) -> Result<(u128, u64)> {
    match prev_revision.checked_add(1) {
        Some(next_rev) => Ok((prev_instance_id, next_rev)),
        None => Ok((mint_instance_id(txn, store_id, prev_instance_id, id)?, 0)),
    }
}

/// Strictly advances `NEIGHBORHOODS[id]`'s version inside an open write
/// transaction whenever neighborhood `id` already exists, invalidating any
/// already-open lot session against stale overwrites.
fn bump_neighborhood_revision(
    txn: &redb::WriteTransaction,
    store_id: u128,
    id: &str,
) -> Result<()> {
    let mut table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
    let existing = table
        .get(id)
        .map_err(store)?
        .map(|v| StoredNeighborhood::decode(id, v.value()))
        .transpose()?;
    if let Some(mut nh) = existing {
        let (next_instance, next_rev) =
            advance_version(txn, store_id, id, nh.instance_id, nh.revision)?;
        nh.instance_id = next_instance;
        nh.revision = next_rev;
        let encoded = nh.encode();
        table.insert(id, encoded.as_slice()).map_err(store)?;
    }
    Ok(())
}

impl NeighborhoodStore {
    /// Opens an existing REDB neighborhood store or creates a new one,
    /// ensuring a persistent 128-bit store identity bound to both the database
    /// metadata and the underlying file identity (canonical path and Unix
    /// device/inode) so copied database files never share a store identity.
    pub fn open(path: &Path) -> Result<Self> {
        let db = Database::create(path).map_err(store)?;
        let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let existing = {
            let txn = db.begin_read().map_err(store)?;
            match txn.open_table(NEIGHBORHOOD_META) {
                Ok(table) => table
                    .get(STORE_ID_KEY)
                    .map_err(store)?
                    .and_then(|v| v.value().try_into().ok())
                    .map(u128::from_le_bytes),
                Err(redb::TableError::TableDoesNotExist(_)) => None,
                Err(error) => return Err(store(error)),
            }
        };
        let persisted_id = match existing {
            Some(id) => id,
            None => {
                let candidate = fresh_unique_id("store", 0, canon.to_string_lossy().as_bytes());
                let txn = db.begin_write().map_err(store)?;
                let committed_id = {
                    let mut table = txn.open_table(NEIGHBORHOOD_META).map_err(store)?;
                    if let Some(found) = table
                        .get(STORE_ID_KEY)
                        .map_err(store)?
                        .and_then(|v| v.value().try_into().ok())
                        .map(u128::from_le_bytes)
                    {
                        found
                    } else {
                        table
                            .insert(STORE_ID_KEY, candidate.to_le_bytes().as_slice())
                            .map_err(store)?;
                        candidate
                    }
                };
                txn.commit().map_err(store)?;
                committed_id
            }
        };
        let store_id = bind_store_file_identity(persisted_id, &canon);
        Ok(Self {
            db: Arc::new(db),
            store_id,
        })
    }

    /// Persistent 128-bit identity of this REDB store file.
    #[must_use]
    pub fn store_id(&self) -> u128 {
        self.store_id
    }

    /// Returns a crash-safe [`AutosaveStore`] handle backed by tables in this
    /// same REDB database file with [`AutosavePolicy::default`] (5 rotating
    /// autosaves per lot).
    ///
    /// The returned handle is `Clone + Send + Sync` and can be moved onto a
    /// background save worker thread without REDB file-lock conflicts.
    #[must_use]
    pub fn autosaves(&self) -> AutosaveStore {
        AutosaveStore::from_shared_db(Arc::clone(&self.db), AutosavePolicy::default())
    }

    /// Returns a crash-safe [`AutosaveStore`] handle sharing this REDB
    /// database file with a custom [`AutosavePolicy`].
    pub fn autosaves_with_policy(
        &self,
        policy: AutosavePolicy,
    ) -> std::result::Result<AutosaveStore, crate::autosave::SaveError> {
        let validated = AutosavePolicy::new(policy.max_autosaves)?;
        Ok(AutosaveStore::from_shared_db(
            Arc::clone(&self.db),
            validated,
        ))
    }

    /// Returns a [`PhotoAlbumStore`] handle backed by tables in this same REDB
    /// database file.
    ///
    /// The returned handle is `Clone + Send + Sync` and enforces
    /// `expected_instance_id` lifetime fencing inside every mutating write
    /// transaction.
    #[must_use]
    pub fn albums(&self) -> PhotoAlbumStore {
        PhotoAlbumStore::from_shared_db(Arc::clone(&self.db))
    }

    /// Returns the last neighborhood schema version explicitly recorded in
    /// `NEIGHBORHOOD_META` by [`Self::migrate_all`], if any.
    pub fn recorded_schema_version(&self) -> Result<Option<u32>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOOD_META) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store(error)),
        };
        let Some(value) = table.get(SCHEMA_VERSION_KEY).map_err(store)? else {
            return Ok(None);
        };
        let bytes: [u8; 4] = value
            .value()
            .try_into()
            .map_err(|_| Error::Snapshot("recorded schema_version is not 4 bytes".into()))?;
        Ok(Some(u32::from_le_bytes(bytes)))
    }

    /// Inspects every stored neighborhood record in a single read-only
    /// transaction without modifying the database, reporting which records are
    /// current, which require forward migration, and which are corrupt or
    /// written by an unsupported version.
    pub fn inspect_schema(&self) -> Result<Vec<NeighborhoodSchemaStatus>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOODS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(error) => return Err(store(error)),
        };
        let mut statuses = Vec::new();
        for entry in table.iter().map_err(store)? {
            let (key, value) = entry.map_err(store)?;
            let id = key.value().to_owned();
            match NEIGHBORHOOD_MIGRATIONS.migrate_bytes(&id, value.value()) {
                Ok(migrated) if migrated.steps_applied == 0 => {
                    statuses.push(NeighborhoodSchemaStatus::Current {
                        id,
                        version: migrated.final_version,
                    });
                }
                Ok(migrated) => {
                    statuses.push(NeighborhoodSchemaStatus::NeedsMigration {
                        id,
                        from_version: migrated.original_version,
                        target_version: migrated.final_version,
                    });
                }
                Err(error) => {
                    statuses.push(NeighborhoodSchemaStatus::Invalid { id, error });
                }
            }
        }
        Ok(statuses)
    }

    /// Atomically migrates a single neighborhood row `id` to
    /// [`NEIGHBORHOOD_FORMAT`] inside one REDB write transaction.
    ///
    /// - Returns `Ok(None)` if `id` does not exist.
    /// - Returns `Ok(Some(record))` with `steps_applied == 0` if already
    ///   current (still validating integrity without rewriting).
    /// - If the record is corrupt, truncated, or written by an unsupported
    ///   version (`0` or `> NEIGHBORHOOD_FORMAT`), aborts the transaction and
    ///   returns `Err(StoreMigrationError::Migration(_))` without modifying
    ///   the database.
    pub fn migrate_neighborhood(
        &self,
        id: &str,
    ) -> std::result::Result<Option<MigratedRecord>, StoreMigrationError> {
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StoreMigrationError::Store(e.to_string()))?;
        let outcome = {
            let mut table = txn
                .open_table(NEIGHBORHOODS)
                .map_err(|e| StoreMigrationError::Store(e.to_string()))?;
            let raw = table
                .get(id)
                .map_err(|e| StoreMigrationError::Store(e.to_string()))?
                .map(|v| v.value().to_vec());
            let Some(raw) = raw else {
                return Ok(None);
            };
            let migrated = NEIGHBORHOOD_MIGRATIONS.migrate_bytes(id, &raw)?;
            if migrated.steps_applied > 0 {
                table
                    .insert(id, migrated.encoded.as_slice())
                    .map_err(|e| StoreMigrationError::Store(e.to_string()))?;
            }
            MigratedRecord {
                id: id.to_owned(),
                from_version: migrated.original_version,
                to_version: migrated.final_version,
                steps_applied: migrated.steps_applied,
            }
        };
        if outcome.steps_applied > 0 {
            txn.commit()
                .map_err(|e| StoreMigrationError::Store(e.to_string()))?;
        }
        Ok(Some(outcome))
    }

    /// Atomically validates and migrates **every** neighborhood record in this
    /// store to [`NEIGHBORHOOD_FORMAT`] inside a single REDB write transaction.
    ///
    /// Every row is validated and migrated in memory *before* any write is
    /// applied or committed. If any record is corrupt, truncated, or uses an
    /// unsupported/future version, the transaction is aborted and **zero**
    /// records are modified on disk.
    pub fn migrate_all(&self) -> std::result::Result<StoreMigrationReport, StoreMigrationError> {
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StoreMigrationError::Store(e.to_string()))?;
        let report;
        {
            let mut nh_table = txn
                .open_table(NEIGHBORHOODS)
                .map_err(|e| StoreMigrationError::Store(e.to_string()))?;

            // Phase 1: read all raw rows so no mutation happens until every
            // record passes validation and migration cleanly.
            let mut raw_rows: Vec<(String, Vec<u8>)> = Vec::new();
            for entry in nh_table
                .iter()
                .map_err(|e| StoreMigrationError::Store(e.to_string()))?
            {
                let (key, value) = entry.map_err(|e| StoreMigrationError::Store(e.to_string()))?;
                raw_rows.push((key.value().to_owned(), value.value().to_vec()));
            }

            // Phase 2: validate & migrate every row in memory. Any error
            // returns immediately, dropping `txn` without committing.
            let mut upgrades: Vec<(String, Vec<u8>, MigratedRecord)> = Vec::new();
            let mut already_current = 0usize;
            for (id, raw) in &raw_rows {
                let migrated = NEIGHBORHOOD_MIGRATIONS.migrate_bytes(id, raw)?;
                if migrated.steps_applied == 0 {
                    already_current += 1;
                } else {
                    upgrades.push((
                        id.clone(),
                        migrated.encoded,
                        MigratedRecord {
                            id: id.clone(),
                            from_version: migrated.original_version,
                            to_version: migrated.final_version,
                            steps_applied: migrated.steps_applied,
                        },
                    ));
                }
            }

            // Phase 3: write all upgraded rows and record schema version
            // atomically in this single write transaction.
            let mut migrated_records = Vec::with_capacity(upgrades.len());
            for (id, encoded, record) in upgrades {
                nh_table
                    .insert(id.as_str(), encoded.as_slice())
                    .map_err(|e| StoreMigrationError::Store(e.to_string()))?;
                migrated_records.push(record);
            }
            let mut meta_table = txn
                .open_table(NEIGHBORHOOD_META)
                .map_err(|e| StoreMigrationError::Store(e.to_string()))?;
            meta_table
                .insert(
                    SCHEMA_VERSION_KEY,
                    NEIGHBORHOOD_FORMAT.to_le_bytes().as_slice(),
                )
                .map_err(|e| StoreMigrationError::Store(e.to_string()))?;

            report = StoreMigrationReport {
                target_version: NEIGHBORHOOD_FORMAT,
                inspected: raw_rows.len(),
                already_current,
                migrated: migrated_records,
            };
        }
        txn.commit()
            .map_err(|e| StoreMigrationError::Store(e.to_string()))?;
        Ok(report)
    }

    /// Imports a neighborhood directory (`UserData`, `UserData2`..`UserData8`)
    /// into the store under `id` without modifying any files in `directory`.
    ///
    /// Re-importing an existing `id` atomically replaces its neighborhood
    /// metadata, character files, base house files, and any previously saved
    /// lots.
    pub fn import_directory(
        &self,
        id: &str,
        directory: &Path,
        expansions: u16,
    ) -> Result<ImportSummary> {
        if id.is_empty() {
            return Err(Error::Store("neighborhood id must not be empty".into()));
        }
        let neighborhood = Neighborhood::open(directory).map_err(store)?;
        let zoning = neighborhood.zoning(expansions).map_err(store)?;
        let lot_rows = neighborhood.lot_rows().map_err(store)?;

        let mut characters: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut character_file_count = 0usize;
        for (rel_path, path) in neighborhood.character_tree().map_err(store)? {
            if rel_path.ends_with('/') {
                characters.insert(rel_path, Vec::new());
            } else {
                let bytes = std::fs::read(&path).map_err(store)?;
                characters.insert(rel_path, bytes);
                character_file_count += 1;
            }
        }

        let mut houses: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
        for (number, path) in neighborhood.houses().map_err(store)? {
            let bytes = std::fs::read(&path).map_err(store)?;
            houses.insert(number, bytes);
        }

        let mut stored = StoredNeighborhood {
            name: id.to_owned(),
            families: neighborhood.families,
            neighbors: neighborhood.neighbors,
            inventories: neighborhood.inventories,
            data: neighborhood.data,
            options: neighborhood.options,
            type_attributes: neighborhood.type_attributes,
            zoning,
            lot_rows,
            made_characters: Characters::default(),
            deleted_characters: BTreeSet::new(),
            deletion_epoch: 0,
            deleted_at_epoch: BTreeMap::new(),
            instance_id: 0,
            revision: 0,
        };
        let summary = ImportSummary {
            id: id.to_owned(),
            families: stored.families.len(),
            neighbors: stored.neighbors.iter().flatten().count(),
            characters: character_file_count,
            houses: houses.len(),
        };

        let txn = self.db.begin_write().map_err(store)?;
        {
            let mut nh_table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            let prev_instance = nh_table
                .get(id)
                .map_err(store)?
                .and_then(|v| StoredNeighborhood::decode(id, v.value()).ok())
                .map_or(0, |prev| prev.instance_id);
            stored.instance_id = mint_instance_id(&txn, self.store_id, prev_instance, id)?;
            stored.revision = 0;
            let encoded = stored.encode();
            let mut char_table = txn.open_table(NEIGHBORHOOD_CHARACTERS).map_err(store)?;
            let mut house_table = txn.open_table(NEIGHBORHOOD_HOUSES).map_err(store)?;
            let mut lot_table = txn.open_table(NEIGHBORHOOD_LOTS).map_err(store)?;
            let mut meta_table = txn.open_table(NEIGHBORHOOD_META).map_err(store)?;

            // Remove any prior records for this neighborhood ID cleanly.
            let mut old_chars = Vec::new();
            for item in char_table.range((id, "")..).map_err(store)? {
                let (k, _) = item.map_err(store)?;
                let (nh_key, name) = k.value();
                if nh_key != id {
                    break;
                }
                old_chars.push(name.to_owned());
            }
            for name in old_chars {
                char_table.remove((id, name.as_str())).map_err(store)?;
            }
            let old_houses: Vec<u32> = house_table
                .range((id, 0u32)..=(id, u32::MAX))
                .map_err(store)?
                .filter_map(|item| item.ok().map(|(k, _)| k.value().1))
                .collect();
            for number in old_houses {
                house_table.remove((id, number)).map_err(store)?;
            }
            let old_lots: Vec<u32> = lot_table
                .range((id, 0u32)..=(id, u32::MAX))
                .map_err(store)?
                .filter_map(|item| item.ok().map(|(k, _)| k.value().1))
                .collect();
            for number in old_lots {
                lot_table.remove((id, number)).map_err(store)?;
            }
            crate::autosave::purge_neighborhood_in_txn(&txn, id)?;
            crate::album_store::purge_neighborhood_in_txn(&txn, id)?;
            purge_lot_thumbnails_in_txn(&txn, id)?;

            nh_table.insert(id, encoded.as_slice()).map_err(store)?;
            for (name, bytes) in &characters {
                char_table
                    .insert((id, name.as_str()), bytes.as_slice())
                    .map_err(store)?;
            }
            for (&number, bytes) in &houses {
                house_table
                    .insert((id, number), bytes.as_slice())
                    .map_err(store)?;
            }
            if meta_table.get(ACTIVE_KEY).map_err(store)?.is_none() {
                meta_table
                    .insert(ACTIVE_KEY, id.as_bytes())
                    .map_err(store)?;
            }
        }
        txn.commit().map_err(store)?;
        Ok(summary)
    }

    /// Lists all imported neighborhood IDs in sorted order.
    pub fn list_neighborhoods(&self) -> Result<Vec<String>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOODS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(error) => return Err(store(error)),
        };
        let mut names = Vec::new();
        for entry in table.iter().map_err(store)? {
            let (key, _) = entry.map_err(store)?;
            names.push(key.value().to_owned());
        }
        Ok(names)
    }

    /// Returns the currently selected active neighborhood ID, if any.
    pub fn active_neighborhood(&self) -> Result<Option<String>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOOD_META) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store(error)),
        };
        let Some(value) = table.get(ACTIVE_KEY).map_err(store)? else {
            return Ok(None);
        };
        let text = std::str::from_utf8(value.value())
            .map_err(|error| Error::Snapshot(format!("active neighborhood key: {error}")))?;
        Ok(Some(text.to_owned()))
    }

    /// Switches the active neighborhood to `id`, failing if `id` does not exist.
    pub fn set_active_neighborhood(&self, id: &str) -> Result<()> {
        let txn = self.db.begin_write().map_err(store)?;
        {
            let nh_table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            if nh_table.get(id).map_err(store)?.is_none() {
                return Err(Error::Store(format!("neighborhood {id:?} does not exist")));
            }
            let mut meta_table = txn.open_table(NEIGHBORHOOD_META).map_err(store)?;
            meta_table
                .insert(ACTIVE_KEY, id.as_bytes())
                .map_err(store)?;
        }
        txn.commit().map_err(store)
    }

    /// Deletes a neighborhood and all of its characters, base houses, and saved
    /// lots atomically. Returns `true` if the neighborhood existed.
    pub fn delete_neighborhood(&self, id: &str) -> Result<bool> {
        let txn = self.db.begin_write().map_err(store)?;
        let existed;
        {
            let mut nh_table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            existed = nh_table.remove(id).map_err(store)?.is_some();
            if existed {
                let mut char_table = txn.open_table(NEIGHBORHOOD_CHARACTERS).map_err(store)?;
                let mut chars = Vec::new();
                for item in char_table.range((id, "")..).map_err(store)? {
                    let (k, _) = item.map_err(store)?;
                    let (nh_key, name) = k.value();
                    if nh_key != id {
                        break;
                    }
                    chars.push(name.to_owned());
                }
                for name in chars {
                    char_table.remove((id, name.as_str())).map_err(store)?;
                }
                let mut house_table = txn.open_table(NEIGHBORHOOD_HOUSES).map_err(store)?;
                let houses: Vec<u32> = house_table
                    .range((id, 0u32)..=(id, u32::MAX))
                    .map_err(store)?
                    .filter_map(|item| item.ok().map(|(k, _)| k.value().1))
                    .collect();
                for number in houses {
                    house_table.remove((id, number)).map_err(store)?;
                }
                let mut lot_table = txn.open_table(NEIGHBORHOOD_LOTS).map_err(store)?;
                let lots: Vec<u32> = lot_table
                    .range((id, 0u32)..=(id, u32::MAX))
                    .map_err(store)?
                    .filter_map(|item| item.ok().map(|(k, _)| k.value().1))
                    .collect();
                for number in lots {
                    lot_table.remove((id, number)).map_err(store)?;
                }
                crate::autosave::purge_neighborhood_in_txn(&txn, id)?;
                crate::album_store::purge_neighborhood_in_txn(&txn, id)?;
                purge_lot_thumbnails_in_txn(&txn, id)?;
                let mut meta_table = txn.open_table(NEIGHBORHOOD_META).map_err(store)?;
                let is_active = meta_table
                    .get(ACTIVE_KEY)
                    .map_err(store)?
                    .is_some_and(|v| v.value() == id.as_bytes());
                if is_active {
                    meta_table.remove(ACTIVE_KEY).map_err(store)?;
                }
            }
        }
        txn.commit().map_err(store)?;
        Ok(existed)
    }

    /// Loads the stored neighborhood metadata for `id`, if present.
    pub fn load_neighborhood(&self, id: &str) -> Result<Option<StoredNeighborhood>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOODS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store(error)),
        };
        let Some(value) = table.get(id).map_err(store)? else {
            return Ok(None);
        };
        StoredNeighborhood::decode(id, value.value()).map(Some)
    }

    /// Loads a consistent point-in-time snapshot of neighborhood `id`, its
    /// character files, and `(saved_lot, base_house)` for `house` inside a
    /// single REDB read transaction.
    pub fn load_lot_snapshot(&self, id: &str, house: u32) -> Result<Option<LotSnapshot>> {
        let txn = self.db.begin_read().map_err(store)?;
        let nh_table = match txn.open_table(NEIGHBORHOODS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store(error)),
        };
        let Some(nh_value) = nh_table.get(id).map_err(store)? else {
            return Ok(None);
        };
        let neighborhood = StoredNeighborhood::decode(id, nh_value.value())?;

        let mut character_files = BTreeMap::new();
        match txn.open_table(NEIGHBORHOOD_CHARACTERS) {
            Ok(table) => {
                for entry in table.range((id, "")..).map_err(store)? {
                    let (key, value) = entry.map_err(store)?;
                    let (nh_key, name) = key.value();
                    if nh_key != id {
                        break;
                    }
                    character_files.insert(name.to_owned(), value.value().to_vec());
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(error) => return Err(store(error)),
        }

        let saved_lot = match txn.open_table(NEIGHBORHOOD_LOTS) {
            Ok(table) => table
                .get((id, house))
                .map_err(store)?
                .map(|value| value.value().to_vec()),
            Err(redb::TableError::TableDoesNotExist(_)) => None,
            Err(error) => return Err(store(error)),
        };

        let base_house = match txn.open_table(NEIGHBORHOOD_HOUSES) {
            Ok(table) => table
                .get((id, house))
                .map_err(store)?
                .map(|value| value.value().to_vec()),
            Err(redb::TableError::TableDoesNotExist(_)) => None,
            Err(error) => return Err(store(error)),
        };

        Ok(Some(LotSnapshot {
            neighborhood,
            character_files,
            saved_lot,
            base_house,
        }))
    }

    /// Saves `state` for neighborhood `id`, replacing any previous state.
    ///
    /// When overwriting an existing neighborhood record, strictly advances its
    /// committed `(instance_id, revision)` past the previously stored version
    /// (rotating `instance_id` on `u64::MAX` rollover). When creating or
    /// recreating a neighborhood that does not currently exist, always mints a
    /// fresh `instance_id` and starts at `revision = 0` so no session opened
    /// before deletion can overwrite the recreated neighborhood.
    pub fn save_neighborhood(&self, id: &str, state: &StoredNeighborhood) -> Result<()> {
        let txn = self.db.begin_write().map_err(store)?;
        {
            let mut table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            let previous = table
                .get(id)
                .map_err(store)?
                .map(|v| StoredNeighborhood::decode(id, v.value()))
                .transpose()?;
            let mut committed = state.clone();
            if let Some(prev) = &previous {
                let (next_instance, next_rev) =
                    advance_version(&txn, self.store_id, id, prev.instance_id, prev.revision)?;
                committed.instance_id = next_instance;
                committed.revision = next_rev;
                purge_removed_family_albums_in_txn(&txn, id, prev, &committed)?;
            } else {
                committed.instance_id =
                    mint_instance_id(&txn, self.store_id, state.instance_id, id)?;
                committed.revision = 0;
            }
            let encoded = committed.encode();
            table.insert(id, encoded.as_slice()).map_err(store)?;
        }
        txn.commit().map_err(store)
    }

    /// Atomically saves `state` for neighborhood `id` only if the committed
    /// neighborhood's `(instance_id, revision)` still matches
    /// `(expected_instance_id, expected_revision)`, always advancing
    /// `(instance_id, revision)` strictly past the committed version (rotating
    /// `instance_id` on `u64::MAX` rollover) so no caller can saturate revision
    /// or let a concurrent/open lot session silently overwrite the commit.
    ///
    /// On success, returns the exact `(instance_id, revision)` committed by
    /// this transaction so callers never need to reread the row afterward.
    pub fn save_neighborhood_checked(
        &self,
        id: &str,
        expected_instance_id: u128,
        expected_revision: u64,
        state: &StoredNeighborhood,
    ) -> Result<std::result::Result<(u128, u64), CheckedCommitError>> {
        let txn = self.db.begin_write().map_err(store)?;
        let committed_version;
        {
            let mut table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            let current = {
                let Some(existing) = table.get(id).map_err(store)? else {
                    return Err(Error::Store(format!("neighborhood {id:?} does not exist")));
                };
                StoredNeighborhood::decode(id, existing.value())?
            };
            if current.instance_id != expected_instance_id {
                return Ok(Err(CheckedCommitError::InstanceMismatch {
                    expected: expected_instance_id,
                    current: current.instance_id,
                }));
            }
            if current.revision != expected_revision {
                return Ok(Err(CheckedCommitError::RevisionMismatch {
                    expected: expected_revision,
                    current: current.revision,
                }));
            }
            let mut committed = state.clone();
            let (next_instance, next_rev) = advance_version(
                &txn,
                self.store_id,
                id,
                current.instance_id,
                current.revision,
            )?;
            committed.instance_id = next_instance;
            committed.revision = next_rev;
            committed_version = (next_instance, next_rev);
            purge_removed_family_albums_in_txn(&txn, id, &current, &committed)?;
            let encoded = committed.encode();
            table.insert(id, encoded.as_slice()).map_err(store)?;
        }
        txn.commit().map_err(store)?;
        Ok(Ok(committed_version))
    }

    /// Loads all character `.iff` files stored for neighborhood `id`.
    pub fn load_character_files(&self, id: &str) -> Result<BTreeMap<String, Vec<u8>>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOOD_CHARACTERS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(BTreeMap::new()),
            Err(error) => return Err(store(error)),
        };
        let mut files = BTreeMap::new();
        for entry in table.range((id, "")..).map_err(store)? {
            let (key, value) = entry.map_err(store)?;
            let (nh_key, name) = key.value();
            if nh_key != id {
                break;
            }
            files.insert(name.to_owned(), value.value().to_vec());
        }
        Ok(files)
    }

    /// Stores a character file under neighborhood `id`, bumping `id`'s
    /// neighborhood revision if it already exists so open lot sessions cannot
    /// overwrite the updated neighborhood state.
    pub fn save_character_file(&self, id: &str, filename: &str, bytes: &[u8]) -> Result<()> {
        let txn = self.db.begin_write().map_err(store)?;
        {
            let mut table = txn.open_table(NEIGHBORHOOD_CHARACTERS).map_err(store)?;
            table.insert((id, filename), bytes).map_err(store)?;
        }
        bump_neighborhood_revision(&txn, self.store_id, id)?;
        txn.commit().map_err(store)
    }

    /// Atomically verifies `(expected_instance_id, expected_revision)` against
    /// `NEIGHBORHOODS[id]` inside a single REDB write transaction, inserts
    /// every `(filename, bytes)` pair into `NEIGHBORHOOD_CHARACTERS`, advances
    /// `(instance_id, revision)` strictly past the committed version (rotating
    /// `instance_id` on `u64::MAX` rollover), and writes `state` into
    /// `NEIGHBORHOODS`.
    ///
    /// On an instance or revision mismatch (for example a concurrent
    /// [`Self::import_directory`] or lot save), aborts the transaction cleanly
    /// with **zero** partial writes and returns `Ok(Err(CheckedCommitError))`.
    pub fn save_characters_and_neighborhood_checked<'a, I>(
        &self,
        id: &str,
        expected_instance_id: u128,
        expected_revision: u64,
        character_files: I,
        state: &StoredNeighborhood,
    ) -> Result<std::result::Result<(u128, u64), CheckedCommitError>>
    where
        I: IntoIterator<Item = (&'a str, &'a [u8])>,
    {
        let mut files: Vec<(&str, &[u8])> = Vec::new();
        let mut seen_names: BTreeSet<&str> = BTreeSet::new();
        for (filename, bytes) in character_files {
            if filename.is_empty() {
                return Err(Error::Store("character filename must not be empty".into()));
            }
            if !seen_names.insert(filename) {
                return Err(Error::Store(format!(
                    "duplicate character filename {filename:?} in atomic batch"
                )));
            }
            files.push((filename, bytes));
        }

        let txn = self.db.begin_write().map_err(store)?;
        let committed_version;
        {
            let mut nh_table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            let current = {
                let Some(existing) = nh_table.get(id).map_err(store)? else {
                    return Err(Error::Store(format!("neighborhood {id:?} does not exist")));
                };
                StoredNeighborhood::decode(id, existing.value())?
            };
            if current.instance_id != expected_instance_id {
                return Ok(Err(CheckedCommitError::InstanceMismatch {
                    expected: expected_instance_id,
                    current: current.instance_id,
                }));
            }
            if current.revision != expected_revision {
                return Ok(Err(CheckedCommitError::RevisionMismatch {
                    expected: expected_revision,
                    current: current.revision,
                }));
            }
            let mut committed = state.clone();
            let (next_instance, next_rev) = advance_version(
                &txn,
                self.store_id,
                id,
                current.instance_id,
                current.revision,
            )?;
            committed.instance_id = next_instance;
            committed.revision = next_rev;
            committed_version = (next_instance, next_rev);
            purge_removed_family_albums_in_txn(&txn, id, &current, &committed)?;
            let encoded = committed.encode();
            if !files.is_empty() {
                let mut char_table = txn.open_table(NEIGHBORHOOD_CHARACTERS).map_err(store)?;
                for (filename, bytes) in files {
                    char_table.insert((id, filename), bytes).map_err(store)?;
                }
            }
            nh_table.insert(id, encoded.as_slice()).map_err(store)?;
        }
        txn.commit().map_err(store)?;
        Ok(Ok(committed_version))
    }

    /// Persists a custom neighborhood map thumbnail for `(id, lot_id)` inside a
    /// single REDB write transaction fenced by `expected_instance_id`.
    ///
    /// Validates any existing thumbnail record for `(id, lot_id)` before
    /// replacing it so corrupt or future-version rows are never silently
    /// overwritten (callers can explicitly remove an unreadable row via
    /// [`Self::delete_lot_thumbnail_checked`] for recovery). Does not bump
    /// [`StoredNeighborhood::revision`] so cosmetic map thumbnail refreshes
    /// never invalidate open lot sessions. Returns
    /// `Ok(Err(CheckedCommitError::InstanceMismatch))` with zero writes if the
    /// neighborhood was re-imported concurrently.
    pub fn save_lot_thumbnail_checked(
        &self,
        id: &str,
        expected_instance_id: u128,
        lot_id: u32,
        thumbnail: &StoredLotThumbnail,
    ) -> Result<std::result::Result<(), CheckedCommitError>> {
        if id.is_empty() {
            return Err(Error::Store("neighborhood id must not be empty".into()));
        }
        if expected_instance_id == 0 {
            return Err(Error::Store("expected_instance_id must be non-zero".into()));
        }
        let encoded = encode_lot_thumbnail(id, lot_id, thumbnail)?;
        let txn = self.db.begin_write().map_err(store)?;
        {
            let nh_table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            let Some(existing) = nh_table.get(id).map_err(store)? else {
                return Err(Error::Store(format!("neighborhood {id:?} does not exist")));
            };
            let current = StoredNeighborhood::decode(id, existing.value())?;
            if current.instance_id != expected_instance_id {
                return Ok(Err(CheckedCommitError::InstanceMismatch {
                    expected: expected_instance_id,
                    current: current.instance_id,
                }));
            }
            let mut thumb_table = txn.open_table(NEIGHBORHOOD_LOT_THUMBNAILS).map_err(store)?;
            {
                if let Some(prev) = thumb_table.get((id, lot_id)).map_err(store)? {
                    let _ = decode_lot_thumbnail(id, lot_id, prev.value())?;
                }
            }
            thumb_table
                .insert((id, lot_id), encoded.as_slice())
                .map_err(store)?;
        }
        txn.commit().map_err(store)?;
        Ok(Ok(()))
    }

    /// Loads the persisted custom neighborhood map thumbnail for `(id, lot_id)`,
    /// if any, verifying its magic, format version, SHA-256 checksum, key
    /// binding, and surface dimensions.
    pub fn load_lot_thumbnail(&self, id: &str, lot_id: u32) -> Result<Option<StoredLotThumbnail>> {
        if id.is_empty() {
            return Err(Error::Store("neighborhood id must not be empty".into()));
        }
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOOD_LOT_THUMBNAILS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store(error)),
        };
        let Some(value) = table.get((id, lot_id)).map_err(store)? else {
            return Ok(None);
        };
        decode_lot_thumbnail(id, lot_id, value.value()).map(Some)
    }

    /// Loads all persisted custom neighborhood map thumbnails for neighborhood
    /// `id` in a single MVCC read transaction, keyed by `lot_id`, enforcing
    /// [`DEFAULT_LOT_THUMBNAILS_MAX_BYTES`] across cumulative decoded pixel
    /// buffers.
    pub fn load_lot_thumbnails(&self, id: &str) -> Result<BTreeMap<u32, StoredLotThumbnail>> {
        self.load_lot_thumbnails_bounded(id, DEFAULT_LOT_THUMBNAILS_MAX_BYTES)
    }

    /// Like [`Self::load_lot_thumbnails`], enforcing an explicit cumulative
    /// decoded RGB565 pixel-buffer ceiling (`max_pixel_bytes`).
    pub fn load_lot_thumbnails_bounded(
        &self,
        id: &str,
        max_pixel_bytes: usize,
    ) -> Result<BTreeMap<u32, StoredLotThumbnail>> {
        if id.is_empty() {
            return Err(Error::Store("neighborhood id must not be empty".into()));
        }
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOOD_LOT_THUMBNAILS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(BTreeMap::new()),
            Err(error) => return Err(store(error)),
        };
        let mut thumbs = BTreeMap::new();
        let mut total_pixel_bytes = 0usize;
        for entry in table.range((id, 0u32)..=(id, u32::MAX)).map_err(store)? {
            let (key, value) = entry.map_err(store)?;
            let lot_id = key.value().1;
            let thumb = decode_lot_thumbnail(id, lot_id, value.value())?;
            let next_total = total_pixel_bytes.saturating_add(thumb.pixel_bytes());
            if next_total > max_pixel_bytes {
                return Err(Error::Store(format!(
                    "lot thumbnails bulk read for {id:?} requires at least {next_total} pixel bytes, \
                     exceeding budget {max_pixel_bytes}; load subsets via load_lot_thumbnails_for"
                )));
            }
            total_pixel_bytes = next_total;
            thumbs.insert(lot_id, thumb);
        }
        Ok(thumbs)
    }

    /// Loads persisted custom neighborhood map thumbnails only for the
    /// requested `lot_ids` in a single MVCC read transaction, enforcing a
    /// cumulative decoded RGB565 pixel-buffer ceiling (`max_pixel_bytes`).
    pub fn load_lot_thumbnails_for(
        &self,
        id: &str,
        lot_ids: &[u32],
        max_pixel_bytes: usize,
    ) -> Result<BTreeMap<u32, StoredLotThumbnail>> {
        if id.is_empty() {
            return Err(Error::Store("neighborhood id must not be empty".into()));
        }
        if lot_ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOOD_LOT_THUMBNAILS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(BTreeMap::new()),
            Err(error) => return Err(store(error)),
        };
        let mut thumbs = BTreeMap::new();
        let mut total_pixel_bytes = 0usize;
        for &lot_id in lot_ids {
            if thumbs.contains_key(&lot_id) {
                continue;
            }
            let Some(value) = table.get((id, lot_id)).map_err(store)? else {
                continue;
            };
            let thumb = decode_lot_thumbnail(id, lot_id, value.value())?;
            let next_total = total_pixel_bytes.saturating_add(thumb.pixel_bytes());
            if next_total > max_pixel_bytes {
                return Err(Error::Store(format!(
                    "lot thumbnails read for {id:?} requires at least {next_total} pixel bytes, \
                     exceeding budget {max_pixel_bytes}"
                )));
            }
            total_pixel_bytes = next_total;
            thumbs.insert(lot_id, thumb);
        }
        Ok(thumbs)
    }

    /// Deletes the persisted custom neighborhood map thumbnail for `(id, lot_id)`
    /// inside a single REDB write transaction fenced by `expected_instance_id`.
    /// Returns `Ok(Ok(true))` if a thumbnail existed and was removed.
    pub fn delete_lot_thumbnail_checked(
        &self,
        id: &str,
        expected_instance_id: u128,
        lot_id: u32,
    ) -> Result<std::result::Result<bool, CheckedCommitError>> {
        if id.is_empty() {
            return Err(Error::Store("neighborhood id must not be empty".into()));
        }
        if expected_instance_id == 0 {
            return Err(Error::Store("expected_instance_id must be non-zero".into()));
        }
        let txn = self.db.begin_write().map_err(store)?;
        let existed;
        {
            let nh_table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            let Some(existing) = nh_table.get(id).map_err(store)? else {
                return Err(Error::Store(format!("neighborhood {id:?} does not exist")));
            };
            let current = StoredNeighborhood::decode(id, existing.value())?;
            if current.instance_id != expected_instance_id {
                return Ok(Err(CheckedCommitError::InstanceMismatch {
                    expected: expected_instance_id,
                    current: current.instance_id,
                }));
            }
            let mut thumb_table = txn.open_table(NEIGHBORHOOD_LOT_THUMBNAILS).map_err(store)?;
            existed = thumb_table.remove((id, lot_id)).map_err(store)?.is_some();
        }
        txn.commit().map_err(store)?;
        Ok(Ok(existed))
    }

    /// Lists all house numbers available in neighborhood `id` (from base house
    /// files and/or saved lots), in ascending order.
    pub fn list_houses(&self, id: &str) -> Result<Vec<u32>> {
        let txn = self.db.begin_read().map_err(store)?;
        let mut houses = std::collections::BTreeSet::new();
        match txn.open_table(NEIGHBORHOOD_HOUSES) {
            Ok(table) => {
                for entry in table.range((id, 0u32)..=(id, u32::MAX)).map_err(store)? {
                    let (key, _) = entry.map_err(store)?;
                    houses.insert(key.value().1);
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(error) => return Err(store(error)),
        }
        match txn.open_table(NEIGHBORHOOD_LOTS) {
            Ok(table) => {
                for entry in table.range((id, 0u32)..=(id, u32::MAX)).map_err(store)? {
                    let (key, _) = entry.map_err(store)?;
                    houses.insert(key.value().1);
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(error) => return Err(store(error)),
        }
        Ok(houses.into_iter().collect())
    }

    /// Loads the imported base `HouseXX.iff` bytes for `(id, house)`, if any.
    pub fn load_base_house(&self, id: &str, house: u32) -> Result<Option<Vec<u8>>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOOD_HOUSES) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store(error)),
        };
        Ok(table
            .get((id, house))
            .map_err(store)?
            .map(|value| value.value().to_vec()))
    }

    /// Stores base `HouseXX.iff` bytes for `(id, house)`, bumping `id`'s
    /// neighborhood revision if it already exists so open lot sessions cannot
    /// overwrite the replaced base house on Save & Leave.
    pub fn save_base_house(&self, id: &str, house: u32, bytes: &[u8]) -> Result<()> {
        let txn = self.db.begin_write().map_err(store)?;
        {
            let mut table = txn.open_table(NEIGHBORHOOD_HOUSES).map_err(store)?;
            table.insert((id, house), bytes).map_err(store)?;
        }
        bump_neighborhood_revision(&txn, self.store_id, id)?;
        txn.commit().map_err(store)
    }

    /// Loads the latest saved lot bytes for `(id, house)`, if any.
    pub fn load_saved_lot(&self, id: &str, house: u32) -> Result<Option<Vec<u8>>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOOD_LOTS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store(error)),
        };
        Ok(table
            .get((id, house))
            .map_err(store)?
            .map(|value| value.value().to_vec()))
    }

    /// Atomically persists both a saved lot blob and the updated neighborhood
    /// state in a single REDB write transaction.
    pub fn save_lot_and_neighborhood(
        &self,
        id: &str,
        house: u32,
        lot_bytes: &[u8],
        state: &StoredNeighborhood,
    ) -> Result<()> {
        let _ = self.save_lot_and_neighborhood_checked(id, house, None, lot_bytes, state, &[])?;
        Ok(())
    }

    /// Atomically persists a saved lot blob, the updated neighborhood state,
    /// and any character-file deletions (e.g. bulldozed tombstones per
    /// `0x426fcd`) in a single REDB write transaction.
    ///
    /// When `expected` is `Some((expected_instance_id, expected_revision))`,
    /// verifies inside the write transaction that both the committed
    /// neighborhood's `instance_id` and `revision` still match; if the
    /// neighborhood was re-imported or another operation committed a newer
    /// revision meanwhile, aborts the transaction cleanly and returns
    /// `Ok(Err(CheckedCommitError))`. Always advances `(instance_id, revision)`
    /// strictly past the previously committed version and returns the exact
    /// committed `(instance_id, revision)` from the transaction.
    pub fn save_lot_and_neighborhood_checked(
        &self,
        id: &str,
        house: u32,
        expected: Option<(u128, u64)>,
        lot_bytes: &[u8],
        state: &StoredNeighborhood,
        remove_character_files: &[String],
    ) -> Result<std::result::Result<(u128, u64), CheckedCommitError>> {
        let txn = self.db.begin_write().map_err(store)?;
        let committed_version;
        {
            let mut nh_table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            let (committed, previous_for_purge) =
                if let Some((expected_instance_id, expected_revision)) = expected {
                    let current = {
                        let Some(existing) = nh_table.get(id).map_err(store)? else {
                            return Err(Error::Store(format!(
                                "neighborhood {id:?} does not exist"
                            )));
                        };
                        StoredNeighborhood::decode(id, existing.value())?
                    };
                    if current.instance_id != expected_instance_id {
                        return Ok(Err(CheckedCommitError::InstanceMismatch {
                            expected: expected_instance_id,
                            current: current.instance_id,
                        }));
                    }
                    if current.revision != expected_revision {
                        return Ok(Err(CheckedCommitError::RevisionMismatch {
                            expected: expected_revision,
                            current: current.revision,
                        }));
                    }
                    let mut next = state.clone();
                    let (next_instance, next_rev) = advance_version(
                        &txn,
                        self.store_id,
                        id,
                        current.instance_id,
                        current.revision,
                    )?;
                    next.instance_id = next_instance;
                    next.revision = next_rev;
                    (next, Some(current))
                } else {
                    let previous = nh_table
                        .get(id)
                        .map_err(store)?
                        .map(|v| StoredNeighborhood::decode(id, v.value()))
                        .transpose()?;
                    let mut next = state.clone();
                    if let Some(prev) = &previous {
                        let (next_instance, next_rev) = advance_version(
                            &txn,
                            self.store_id,
                            id,
                            prev.instance_id,
                            prev.revision,
                        )?;
                        next.instance_id = next_instance;
                        next.revision = next_rev;
                    } else {
                        next.instance_id =
                            mint_instance_id(&txn, self.store_id, state.instance_id, id)?;
                        next.revision = 0;
                    }
                    (next, previous)
                };
            committed_version = (committed.instance_id, committed.revision);
            if let Some(prev) = &previous_for_purge {
                purge_removed_family_albums_in_txn(&txn, id, prev, &committed)?;
            }
            let encoded = committed.encode();
            let mut lot_table = txn.open_table(NEIGHBORHOOD_LOTS).map_err(store)?;
            lot_table.insert((id, house), lot_bytes).map_err(store)?;
            nh_table.insert(id, encoded.as_slice()).map_err(store)?;
            if !remove_character_files.is_empty() {
                let mut char_table = txn.open_table(NEIGHBORHOOD_CHARACTERS).map_err(store)?;
                for rel_path in remove_character_files {
                    char_table.remove((id, rel_path.as_str())).map_err(store)?;
                }
            }
        }
        txn.commit().map_err(store)?;
        Ok(Ok(committed_version))
    }

    /// Computes a base house's total lot price directly from its `SIMI` chunk
    /// (`0x427f00` → `0x427f78`), without requiring a full catalog link.
    pub fn base_house_lot_price(
        &self,
        id: &str,
        house: u32,
        arch_multiplier: f32,
    ) -> Result<Option<i32>> {
        let Some(bytes) = self.load_base_house(id, house)? else {
            return Ok(None);
        };
        let iff =
            Iff::parse(bytes).map_err(|e| Error::Snapshot(format!("{id} House{house:02}: {e}")))?;
        let chunk = iff
            .chunks
            .iter()
            .find(|c| &c.tag == b"SIMI")
            .ok_or_else(|| Error::Snapshot(format!("{id} House{house:02}: missing SIMI chunk")))?;
        let sim = Simulator::parse(iff.data(chunk))
            .map_err(|e| Error::Snapshot(format!("{id} House{house:02} SIMI: {e}")))?;
        Ok(Some(lot_total_value(
            sim.lot_value,
            sim.objects_value,
            sim.architecture_value,
            arch_multiplier,
        )))
    }
}
