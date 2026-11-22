//! Intermediate representations shared across the `.antics` compiler, binary
//! chunk encoders, procedural asset generators, and in-memory chunk lifter.

use sims_formats::behavior::{Instruction, Interaction};
use sims_formats::execution::{ObjectFunction, Slot};
use sims_formats::sprite::DrawView;

/// Standard IFF chunk flags for little-endian ("converted") resources.
pub const IFF_CONVERTED_FLAG: u16 = 0x0010;

/// A `BHAV` behavior tree chunk.
#[derive(Debug, Clone)]
pub struct BhavChunk {
    pub id: u16,
    pub label: String,
    pub format: u16,
    pub kind: u8,
    pub args: u8,
    pub locals: u16,
    pub revision: u32,
    pub instructions: Vec<Instruction>,
    /// Optional trailing bytes preserved only during verbatim in-memory lifting.
    pub trailing: Vec<u8>,
}

impl BhavChunk {
    #[must_use]
    pub fn new(id: u16, label: impl Into<String>, instructions: Vec<Instruction>) -> Self {
        Self {
            id,
            label: label.into(),
            format: 0x8002,
            kind: 0,
            args: 0,
            locals: 0,
            revision: 0,
            instructions,
            trailing: Vec::new(),
        }
    }
}

/// High-level typed specification for building a clean 216-byte `OBJD` chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjdSpec {
    pub id: u16,
    pub label: String,
    pub version: u32,
    pub initial_stack_size: u16,
    pub base_graphic_id: u16,
    pub num_graphics: u16,
    pub main_id: u16,
    pub gardening_id: u16,
    pub tree_table_id: u16,
    pub interaction_group: i16,
    pub object_type: u16,
    pub master_id: u16,
    pub sub_index: i16,
    pub wash_hands_id: u16,
    pub animation_table: u16,
    pub guid: u32,
    pub disabled: u16,
    pub portal_id: u16,
    pub price: u16,
    pub body_strings_id: u16,
    pub slots_id: u16,
    pub stand_function: u16,
    pub catalog_strings_id: u16,
    pub init_id: u16,
    pub catalog_sort_flags: u16,
    pub room_flags: u16,
    pub function_flags: u16,
}

impl ObjdSpec {
    #[must_use]
    pub fn new(id: u16, label: impl Into<String>, guid: u32) -> Self {
        Self {
            id,
            label: label.into(),
            version: 138,
            initial_stack_size: 0,
            base_graphic_id: 100,
            num_graphics: 1,
            main_id: 0,
            gardening_id: 0,
            tree_table_id: 128,
            interaction_group: 0,
            object_type: 4,
            master_id: 0,
            sub_index: 0,
            wash_hands_id: 0,
            animation_table: 0,
            guid,
            disabled: 0,
            portal_id: 0,
            price: 100,
            body_strings_id: 0,
            slots_id: 128,
            stand_function: 0,
            catalog_strings_id: 2000,
            init_id: 0,
            catalog_sort_flags: 0,
            room_flags: 0,
            function_flags: 0,
        }
    }

    /// Converts this specification into a 216-byte `ObjdChunk`.
    #[must_use]
    pub fn to_chunk(&self) -> ObjdChunk {
        let mut words = vec![0u16; 108];
        words[0] = (self.version & 0xFFFF) as u16;
        words[1] = (self.version >> 16) as u16;
        words[2] = self.initial_stack_size;
        words[3] = self.base_graphic_id;
        words[4] = self.num_graphics;
        words[5] = self.main_id;
        words[6] = self.gardening_id;
        words[7] = self.tree_table_id;
        words[8] = self.interaction_group as u16;
        words[9] = self.object_type;
        words[10] = self.master_id;
        words[11] = self.sub_index as u16;
        words[12] = self.wash_hands_id;
        words[13] = self.animation_table;
        words[14] = (self.guid & 0xFFFF) as u16;
        words[15] = (self.guid >> 16) as u16;
        words[16] = self.disabled;
        words[17] = self.portal_id;
        words[18] = self.price;
        words[19] = self.body_strings_id;
        words[20] = self.slots_id;
        words[33] = self.stand_function;
        words[40] = self.catalog_sort_flags;
        words[41] = self.catalog_strings_id;
        words[43] = self.init_id;
        words[44] = self.room_flags;
        words[45] = self.function_flags;
        ObjdChunk {
            id: self.id,
            label: self.label.clone(),
            version: self.version,
            words,
            trailing_byte: None,
        }
    }
}

/// A raw/versioned `OBJD` chunk supporting exact round-tripping across all
/// Sims 1 versions (`136 | 138 | 139 | 140 | 141 | 142`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjdChunk {
    pub id: u16,
    pub label: String,
    pub version: u32,
    /// Full word array (including the 2 version words at indices 0..2).
    pub words: Vec<u16>,
    /// Optional odd trailing byte if an original OBJD had odd byte length.
    pub trailing_byte: Option<u8>,
}

/// An `OBJf` object function callback table chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjfChunk {
    pub id: u16,
    pub label: String,
    pub reserved: u32,
    pub version: u32,
    pub entries: Vec<ObjectFunction>,
    pub trailing: Vec<u8>,
}

impl ObjfChunk {
    #[must_use]
    pub fn new(id: u16, label: impl Into<String>, entries: Vec<ObjectFunction>) -> Self {
        Self {
            id,
            label: label.into(),
            reserved: 0,
            version: 0,
            entries,
            trailing: Vec::new(),
        }
    }
}

/// A `BCON` integer tuning constant table chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BconChunk {
    pub id: u16,
    pub label: String,
    pub flags: u8,
    pub values: Vec<i16>,
    pub trailing: Vec<u8>,
}

impl BconChunk {
    #[must_use]
    pub fn new(id: u16, label: impl Into<String>, values: Vec<i16>) -> Self {
        Self {
            id,
            label: label.into(),
            flags: 0,
            values,
            trailing: Vec::new(),
        }
    }
}

/// On-disk binary format of a `STR#`, `CTSS`, or `TTAs` chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringFormat {
    /// `0x0000`: u16 count + 1-byte length-prefixed strings.
    Pascal,
    /// `0xFFFF` (-1): u16 count + NUL-terminated strings.
    Terminated,
    /// `0xFFFE` (-2): u16 count + NUL-terminated (string, comment) pairs.
    Commented,
    /// `0xFFFD` (-3): u16 count + (language u8, NUL-terminated string, NUL-terminated comment).
    Localized,
    /// Classic Mac OS `STR#` resource: big-endian u16 count + length-prefixed strings.
    MacResource,
}

/// One entry in a `STR#`, `CTSS`, or `TTAs` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringEntry {
    /// Raw language byte stored in `Localized` (`0xFFFD`) tables (default `1` = US English).
    pub language: u8,
    pub text: String,
    pub comment: String,
    /// Optional raw bytes preserved verbatim during lifting to keep non-ASCII
    /// or non-canonical encodings byte-exact.
    pub raw_text: Option<Vec<u8>>,
    pub raw_comment: Option<Vec<u8>>,
}

impl StringEntry {
    #[must_use]
    pub fn english(text: impl Into<String>) -> Self {
        Self {
            language: 1,
            text: text.into(),
            comment: String::new(),
            raw_text: None,
            raw_comment: None,
        }
    }

    #[must_use]
    pub fn with_comment(text: impl Into<String>, comment: impl Into<String>) -> Self {
        Self {
            language: 1,
            text: text.into(),
            comment: comment.into(),
            raw_text: None,
            raw_comment: None,
        }
    }
}

/// A `STR#`, `CTSS`, or `TTAs` string table chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringTableChunk {
    pub tag: [u8; 4],
    pub id: u16,
    pub label: String,
    pub format: StringFormat,
    /// If true and `entries` is empty, emit only the 2-byte version word.
    pub version_only_empty: bool,
    pub entries: Vec<StringEntry>,
    pub trailing: Vec<u8>,
}

impl StringTableChunk {
    #[must_use]
    pub fn new(
        tag: [u8; 4],
        id: u16,
        label: impl Into<String>,
        format: StringFormat,
        entries: Vec<StringEntry>,
    ) -> Self {
        Self {
            tag,
            id,
            label: label.into(),
            format,
            version_only_empty: false,
            entries,
            trailing: Vec::new(),
        }
    }
}

/// A `TTAB` pie-menu interaction table chunk.
#[derive(Debug, Clone)]
pub struct TtabChunk {
    pub id: u16,
    pub label: String,
    pub version: u16,
    /// Only used when `version >= 9` and `!interactions.is_empty()`.
    pub compressed: bool,
    /// Raw compression flag byte for `version >= 9` (`0` or `1`).
    pub compression_byte: u8,
    pub interactions: Vec<Interaction>,
    /// Optional explicit field bit-width overrides (`0..=3` or `255` for zero-bit)
    /// captured when lifting compressed chunks that used non-minimal widths.
    pub compressed_width_hints: Vec<u8>,
    /// Final trailing bits in the last byte of a compressed stream, if non-zero.
    pub trailing_bits: u8,
    pub trailing: Vec<u8>,
}

impl TtabChunk {
    #[must_use]
    pub fn new(id: u16, label: impl Into<String>, interactions: Vec<Interaction>) -> Self {
        Self {
            id,
            label: label.into(),
            version: 10,
            compressed: false,
            compression_byte: 0,
            interactions,
            compressed_width_hints: Vec::new(),
            trailing_bits: 0,
            trailing: Vec::new(),
        }
    }
}

/// A `SLOT` routing/container slot table chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotChunk {
    pub id: u16,
    pub label: String,
    pub reserved: u32,
    pub version: u32,
    pub entries: Vec<Slot>,
    pub trailing: Vec<u8>,
}

impl SlotChunk {
    #[must_use]
    pub fn new(id: u16, label: impl Into<String>, entries: Vec<Slot>) -> Self {
        Self {
            id,
            label: label.into(),
            reserved: 0,
            version: 10,
            entries,
            trailing: Vec::new(),
        }
    }
}

/// A `PALT` 256-color RGB palette chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaltChunk {
    pub id: u16,
    pub label: String,
    pub colors: Vec<[u8; 3]>,
}

/// One pixel in an uncompressed `SPR2` frame prior to RLE encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Spr2Pixel {
    /// Palette index (`0..=255`).
    pub color: u8,
    /// Z-buffer depth (`0..=255`; smaller is closer to camera).
    pub depth: u8,
    /// 5-bit alpha (`0` = transparent, `31` = fully opaque).
    pub alpha: u8,
}

/// One frame in an `SPR2` sprite set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spr2Frame {
    pub width: u16,
    pub height: u16,
    /// Bit 0 = color, bit 1 = z-buffer, bit 2 = alpha.
    pub flags: u32,
    pub palette_id: u16,
    pub transparent_index: u16,
    pub x: i16,
    pub y: i16,
    /// Row-major `width * height` pixels.
    pub pixels: Vec<Spr2Pixel>,
}

/// An `SPR2` sprite set chunk (version `1000` or `1001`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spr2Chunk {
    pub id: u16,
    pub label: String,
    pub version: u32,
    pub default_palette: u32,
    pub frames: Vec<Spr2Frame>,
}

/// A `DGRP` isometric draw group chunk (versions `20000..=20004`).
#[derive(Debug, Clone)]
pub struct DgrpChunk {
    pub id: u16,
    pub label: String,
    pub version: u16,
    pub views: Vec<DrawView>,
}

/// An `FWAV` sound event name chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FwavChunk {
    pub id: u16,
    pub label: String,
    pub sound_name: String,
}

/// One member to package into a `FAR` archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FarEntry<'a> {
    pub name: &'a str,
    pub data: &'a [u8],
}
