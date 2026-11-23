//! Zero-dependency deterministic TOML subset parser and `object.toml` schema.
//!
//! Supports tables `[section]`, arrays of tables `[[array]]`, strings (basic &
//! literal), integers (decimal & `0x` hex with `_` separators), floats,
//! booleans, inline/multiline arrays, and inline tables `{ k = v }`.
//! Uses `BTreeMap` exclusively for deterministic iteration order.

use crate::art::FurnitureShape;
use crate::audio::SoundPreset;
use crate::error::{Error, Result, Span};
use crate::parser::motive_by_name;
use std::collections::BTreeMap;

/// A parsed TOML value.
#[derive(Debug, Clone, PartialEq)]
pub enum TomlValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Array(Vec<TomlValue>),
    Table(BTreeMap<String, TomlValue>),
}

/// Parses a deterministic subset of TOML into a root table.
pub fn parse_toml(source: &str) -> Result<BTreeMap<String, TomlValue>> {
    let mut root = BTreeMap::new();
    let mut active_section: SectionTarget = SectionTarget::Root;

    let lines: Vec<&str> = source.lines().collect();
    let mut line_idx = 0usize;
    while line_idx < lines.len() {
        let line_no = line_idx + 1;
        let raw = strip_comment(lines[line_idx]).trim();
        if raw.is_empty() {
            line_idx += 1;
            continue;
        }
        if raw.starts_with("[[") && raw.ends_with("]]") {
            let name = raw[2..raw.len() - 2].trim();
            validate_key(name, Span::new(line_no, 1))?;
            let entry = root
                .entry(name.to_owned())
                .or_insert_with(|| TomlValue::Array(Vec::new()));
            let TomlValue::Array(arr) = entry else {
                return Err(Error::at(
                    Span::new(line_no, 1),
                    format!("'[[{name}]]' conflicts with non-array table '{name}'"),
                ));
            };
            arr.push(TomlValue::Table(BTreeMap::new()));
            let idx = arr.len() - 1;
            active_section = SectionTarget::ArrayTable(name.to_owned(), idx);
            line_idx += 1;
            continue;
        }
        if raw.starts_with('[') && raw.ends_with(']') {
            let name = raw[1..raw.len() - 1].trim();
            validate_key(name, Span::new(line_no, 1))?;
            let entry = root
                .entry(name.to_owned())
                .or_insert_with(|| TomlValue::Table(BTreeMap::new()));
            if !matches!(entry, TomlValue::Table(_)) {
                return Err(Error::at(
                    Span::new(line_no, 1),
                    format!("'[{name}]' conflicts with existing non-table key"),
                ));
            }
            active_section = SectionTarget::Table(name.to_owned());
            line_idx += 1;
            continue;
        }

        // Key = value (collect continuation lines if an inline array/table spans multiple lines).
        let eq = raw
            .find('=')
            .ok_or_else(|| Error::at(Span::new(line_no, 1), "expected '=' in key-value pair"))?;
        let key = raw[..eq].trim();
        validate_key(key, Span::new(line_no, 1))?;
        let mut val_buf = raw[eq + 1..].trim().to_owned();
        while needs_continuation(&val_buf) {
            line_idx += 1;
            if line_idx >= lines.len() {
                return Err(Error::at(
                    Span::new(line_no, 1),
                    format!("unterminated multiline value for key '{key}'"),
                ));
            }
            let next = strip_comment(lines[line_idx]).trim();
            if !next.is_empty() {
                val_buf.push(' ');
                val_buf.push_str(next);
            }
        }
        let mut val_parser = ValueParser::new(&val_buf, line_no);
        let value = val_parser.parse_value()?;
        val_parser.skip_ws();
        if !val_parser.is_eof() {
            return Err(Error::at(
                Span::new(line_no, val_parser.pos + 1),
                format!("unexpected trailing characters after value for '{key}'"),
            ));
        }

        let target_table = match &active_section {
            SectionTarget::Root => &mut root,
            SectionTarget::Table(name) => match root.get_mut(name) {
                Some(TomlValue::Table(t)) => t,
                _ => unreachable!("table verified above"),
            },
            SectionTarget::ArrayTable(name, idx) => match root.get_mut(name) {
                Some(TomlValue::Array(arr)) => match arr.get_mut(*idx) {
                    Some(TomlValue::Table(t)) => t,
                    _ => unreachable!("array table entry verified above"),
                },
                _ => unreachable!("array table verified above"),
            },
        };
        if target_table.insert(key.to_owned(), value).is_some() {
            return Err(Error::at(
                Span::new(line_no, 1),
                format!("duplicate key '{key}'"),
            ));
        }
        line_idx += 1;
    }

    Ok(root)
}

enum SectionTarget {
    Root,
    Table(String),
    ArrayTable(String, usize),
}

fn validate_key(key: &str, span: Span) -> Result<()> {
    if key.is_empty()
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(Error::at(span, format!("invalid TOML key '{key}'")));
    }
    Ok(())
}

fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_double {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_double = false;
            }
        } else if in_single {
            if b == b'\'' {
                in_single = false;
            }
        } else {
            match b {
                b'"' => in_double = true,
                b'\'' => in_single = true,
                b'#' => return &line[..i],
                _ => {}
            }
        }
    }
    line
}

fn needs_continuation(s: &str) -> bool {
    let mut brackets = 0i32;
    let mut braces = 0i32;
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for b in s.bytes() {
        if in_double {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_double = false;
            }
        } else if in_single {
            if b == b'\'' {
                in_single = false;
            }
        } else {
            match b {
                b'"' => in_double = true,
                b'\'' => in_single = true,
                b'[' => brackets += 1,
                b']' => brackets -= 1,
                b'{' => braces += 1,
                b'}' => braces -= 1,
                _ => {}
            }
        }
    }
    brackets > 0 || braces > 0
}

/// Maximum nesting depth for inline arrays and inline tables in TOML values.
const MAX_TOML_DEPTH: usize = 64;

struct ValueParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    depth: usize,
}

impl<'a> ValueParser<'a> {
    fn new(src: &'a str, line: usize) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            line,
            depth: 0,
        }
    }

    fn span(&self) -> Span {
        Span::new(self.line, self.pos + 1)
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_value(&mut self) -> Result<TomlValue> {
        self.skip_ws();
        let span = self.span();
        let Some(b) = self.peek() else {
            return Err(Error::at(span, "expected TOML value"));
        };
        match b {
            b'"' => self.parse_basic_string().map(TomlValue::String),
            b'\'' => self.parse_literal_string().map(TomlValue::String),
            b'[' => self.parse_array().map(TomlValue::Array),
            b'{' => self.parse_inline_table().map(TomlValue::Table),
            b't' if self.src[self.pos..].starts_with("true") => {
                self.pos += 4;
                Ok(TomlValue::Bool(true))
            }
            b'f' if self.src[self.pos..].starts_with("false") => {
                self.pos += 5;
                Ok(TomlValue::Bool(false))
            }
            b'-' | b'+' | b'0'..=b'9' => self.parse_number(),
            other => Err(Error::at(
                span,
                format!("unexpected character {:?} in TOML value", char::from(other)),
            )),
        }
    }

    fn parse_basic_string(&mut self) -> Result<String> {
        let span = self.span();
        self.bump(); // "
        let mut out = String::new();
        while let Some(b) = self.bump() {
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self
                        .bump()
                        .ok_or_else(|| Error::at(span, "unterminated escape in TOML string"))?;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        other => {
                            return Err(Error::at(
                                span,
                                format!("unsupported escape '\\{}'", char::from(other)),
                            ));
                        }
                    }
                }
                other => {
                    // Handle multi-byte UTF-8 safely by slicing from `self.src`.
                    if other < 0x80 {
                        out.push(char::from(other));
                    } else {
                        let start = self.pos - 1;
                        let ch = self.src[start..]
                            .chars()
                            .next()
                            .ok_or_else(|| Error::at(span, "invalid UTF-8 in TOML string"))?;
                        out.push(ch);
                        self.pos = start + ch.len_utf8();
                    }
                }
            }
        }
        Err(Error::at(span, "unterminated TOML string"))
    }

    fn parse_literal_string(&mut self) -> Result<String> {
        let span = self.span();
        self.bump(); // '
        let start = self.pos;
        while let Some(b) = self.bump() {
            if b == b'\'' {
                return Ok(self.src[start..self.pos - 1].to_owned());
            }
        }
        Err(Error::at(span, "unterminated TOML literal string"))
    }

    fn parse_number(&mut self) -> Result<TomlValue> {
        let span = self.span();
        let start = self.pos;
        while let Some(b) = self.peek() {
            if matches!(b, b',' | b']' | b'}' | b' ' | b'\t' | b'\r' | b'\n') {
                break;
            }
            self.pos += 1;
        }
        let raw: String = self.src[start..self.pos]
            .chars()
            .filter(|&c| c != '_')
            .collect();
        if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
            let v = i64::from_str_radix(hex, 16)
                .map_err(|_| Error::at(span, format!("invalid hex integer '{raw}'")))?;
            return Ok(TomlValue::Int(v));
        }
        if raw.contains('.') || raw.contains('e') || raw.contains('E') {
            let v = raw
                .parse::<f64>()
                .map_err(|_| Error::at(span, format!("invalid float '{raw}'")))?;
            if !v.is_finite() {
                return Err(Error::at(span, "non-finite float in TOML"));
            }
            return Ok(TomlValue::Float(v));
        }
        let v = raw
            .parse::<i64>()
            .map_err(|_| Error::at(span, format!("invalid integer '{raw}'")))?;
        Ok(TomlValue::Int(v))
    }

    fn parse_array(&mut self) -> Result<Vec<TomlValue>> {
        let span = self.span();
        if self.depth >= MAX_TOML_DEPTH {
            return Err(Error::at(
                span,
                format!("TOML value nesting exceeds maximum depth of {MAX_TOML_DEPTH}"),
            ));
        }
        self.depth += 1;
        self.bump(); // [
        let mut items = Vec::new();
        let res = loop {
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.bump();
                break Ok(items);
            }
            match self.parse_value() {
                Ok(v) => items.push(v),
                Err(e) => break Err(e),
            }
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.bump();
                }
                Some(b']') => {
                    self.bump();
                    break Ok(items);
                }
                _ => {
                    break Err(Error::at(span, "expected ',' or ']' in TOML array"));
                }
            }
        };
        self.depth -= 1;
        res
    }

    fn parse_inline_table(&mut self) -> Result<BTreeMap<String, TomlValue>> {
        let span = self.span();
        if self.depth >= MAX_TOML_DEPTH {
            return Err(Error::at(
                span,
                format!("TOML value nesting exceeds maximum depth of {MAX_TOML_DEPTH}"),
            ));
        }
        self.depth += 1;
        self.bump(); // {
        let mut map = BTreeMap::new();
        let res = loop {
            self.skip_ws();
            if self.peek() == Some(b'}') {
                self.bump();
                break Ok(map);
            }
            let key_start = self.pos;
            while let Some(b) = self.peek() {
                if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            let key = &self.src[key_start..self.pos];
            if let Err(e) = validate_key(key, self.span()) {
                break Err(e);
            }
            self.skip_ws();
            if self.bump() != Some(b'=') {
                break Err(Error::at(
                    self.span(),
                    format!("expected '=' after inline table key '{key}'"),
                ));
            }
            let val = match self.parse_value() {
                Ok(v) => v,
                Err(e) => break Err(e),
            };
            if map.insert(key.to_owned(), val).is_some() {
                break Err(Error::at(
                    span,
                    format!("duplicate key '{key}' in inline table"),
                ));
            }
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.bump();
                }
                Some(b'}') => {
                    self.bump();
                    break Ok(map);
                }
                _ => {
                    break Err(Error::at(span, "expected ',' or '}' in inline table"));
                }
            }
        };
        self.depth -= 1;
        res
    }
}

/// Tree reference in `object.toml`: either a numeric BHAV ID or a tree label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRef {
    Id(u16),
    Name(String),
}

/// Motive advertisement entry in `[[interaction]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MotiveAdSpec {
    pub motive_index: usize,
    pub minimum: i16,
    pub delta: i16,
    pub personality: u16,
}

/// Pie-menu interaction specification in `object.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct InteractionSpec {
    pub name: String,
    pub action: TreeRef,
    pub test: Option<TreeRef>,
    pub flags: u32,
    pub flags2: u32,
    pub autonomy: u32,
    pub attenuation_code: u32,
    pub attenuation: f32,
    pub joining_index: i32,
    pub motives: Vec<MotiveAdSpec>,
}

/// Routing or container slot specification in `object.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotSpec {
    pub kind: u16,
    pub offset: [f32; 3],
    pub standing: i32,
    pub sitting: i32,
    pub ground: i32,
    pub flags: u32,
    pub snap_target: i32,
    pub proximity: [i32; 3],
    pub max_size: i32,
    pub gradient: f32,
    pub height: i32,
    pub facing: i32,
    pub resolution: i32,
}

/// Procedural sprite specification in `[sprite]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpriteSpec {
    pub shape: FurnitureShape,
    pub primary_rgb: [u8; 3],
    pub accent_rgb: [u8; 3],
    pub trim_rgb: [u8; 3],
}

impl Default for SpriteSpec {
    fn default() -> Self {
        Self {
            shape: FurnitureShape::Box,
            primary_rgb: [152, 112, 76],
            accent_rgb: [204, 172, 132],
            trim_rgb: [84, 58, 38],
        }
    }
}

/// Synthesized sound entry in `[[sound]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundSpec {
    pub id: u16,
    pub name: String,
    pub preset: SoundPreset,
}

/// Tuning constant table entry in `[[constant]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstantSpec {
    pub id: u16,
    pub label: String,
    pub flags: u8,
    pub values: Vec<i16>,
}

/// Explicit `OBJf` object callback entry in `[[callback]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackSpec {
    pub index: usize,
    pub action: Option<TreeRef>,
    pub condition: Option<TreeRef>,
}

/// Multi-tile sub-object definition in `[[tile]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileSpec {
    pub guid: u32,
    pub name: Option<String>,
    pub objd_id: Option<u16>,
    /// Encoded multi-tile offset `(dx << 8) | (dy & 0xff)`.
    pub sub_index: i16,
    pub level: i16,
    pub lead: bool,
    pub interaction_group: Option<i16>,
    pub dgrp_id: Option<u16>,
    pub shape: Option<FurnitureShape>,
    pub slot_id: Option<u16>,
    pub ttab_id: Option<u16>,
    pub init: Option<TreeRef>,
    pub main: Option<TreeRef>,
    pub attributes: Option<u16>,
}

/// Explicit `STR#` string table in `[[string_table]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringTableSpec {
    pub id: u16,
    pub label: String,
    pub entries: Vec<String>,
}

/// Typed `pack.toml` manifest describing an on-disk clean-room asset pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackManifest {
    pub name: String,
    pub version: String,
    pub author: String,
    pub license: String,
    pub archive: String,
    pub objects: Vec<String>,
}

impl PackManifest {
    /// Parses a `pack.toml` document, strictly validating section names and object slugs.
    pub fn parse(source: &str) -> Result<Self> {
        let root = parse_toml(source)?;
        for (key, value) in &root {
            match (key.as_str(), value) {
                ("pack" | "dependencies", TomlValue::Table(_)) => {}
                ("pack" | "dependencies", _) => {
                    return Err(Error::new(format!(
                        "section '[{key}]' must be a table, not an array of tables"
                    )));
                }
                (other, _) => {
                    return Err(Error::new(format!(
                        "unknown top-level key or section '{other}' in pack.toml"
                    )));
                }
            }
        }
        let pack = get_table(&root, "pack")
            .ok_or_else(|| Error::new("pack.toml requires a [pack] section"))?;
        let name = req_str(pack, "name")?;
        let version = opt_str(pack, "version").unwrap_or_else(|| "0.1.0".to_owned());
        let author = opt_str(pack, "author").unwrap_or_default();
        let license = opt_str(pack, "license").unwrap_or_else(|| "MIT".to_owned());
        let archive = opt_str(pack, "archive").unwrap_or_else(|| "open-pack.far".to_owned());
        validate_relative_filename(&archive, "[pack].archive")?;
        let objects = opt_str_array(pack, "objects")?;
        if objects.is_empty() {
            return Err(Error::new("[pack].objects must list at least one object"));
        }
        let mut seen = std::collections::BTreeSet::new();
        for slug in &objects {
            validate_slug(slug)?;
            if !seen.insert(slug.clone()) {
                return Err(Error::new(format!(
                    "duplicate object slug '{slug}' in pack.toml"
                )));
            }
        }
        Ok(Self {
            name,
            version,
            author,
            license,
            archive,
            objects,
        })
    }
}

fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty()
        || !slug
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(Error::new(format!(
            "invalid object slug '{slug}' (allowed: ASCII letters, digits, '-', '_')"
        )));
    }
    Ok(())
}

fn validate_relative_filename(name: &str, field: &str) -> Result<()> {
    let valid_chars = !name.is_empty()
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    if !valid_chars {
        return Err(Error::new(format!(
            "invalid filename '{name}' in {field} (allowed: plain relative ASCII filename using letters, digits, '.', '-', '_')"
        )));
    }
    let mut comps = std::path::Path::new(name).components();
    match (comps.next(), comps.next()) {
        (Some(std::path::Component::Normal(_)), None) => Ok(()),
        _ => Err(Error::new(format!(
            "invalid filename '{name}' in {field} (must be a single normal path component)"
        ))),
    }
}

fn parse_furniture_shape(name: &str, field: &str) -> Result<FurnitureShape> {
    match name {
        "bench" => Ok(FurnitureShape::Bench),
        "cot" | "bed" => Ok(FurnitureShape::Cot),
        "basin" => Ok(FurnitureShape::Basin),
        "toilet" => Ok(FurnitureShape::Toilet),
        "sofa" | "couch" => Ok(FurnitureShape::Sofa),
        "television" | "tv" => Ok(FurnitureShape::Television),
        "bookshelf" | "bookcase" => Ok(FurnitureShape::Bookshelf),
        "table" => Ok(FurnitureShape::Table),
        "chair" => Ok(FurnitureShape::Chair),
        "floor_lamp" | "lamp" => Ok(FurnitureShape::FloorLamp),
        "trash_can" | "trash" => Ok(FurnitureShape::TrashCan),
        "front_door" | "door" => Ok(FurnitureShape::FrontDoor),
        "mailbox" => Ok(FurnitureShape::Mailbox),
        "double_bed" => Ok(FurnitureShape::DoubleBed),
        "double_bed_left" => Ok(FurnitureShape::DoubleBedLeft),
        "double_bed_right" => Ok(FurnitureShape::DoubleBedRight),
        "fridge" => Ok(FurnitureShape::Fridge),
        "counter" => Ok(FurnitureShape::Counter),
        "stove" => Ok(FurnitureShape::Stove),
        "meal_plate" | "plate" => Ok(FurnitureShape::MealPlate),
        "wall_phone" | "phone" => Ok(FurnitureShape::WallPhone),
        "shower" => Ok(FurnitureShape::Shower),
        "box" => Ok(FurnitureShape::Box),
        other => Err(Error::new(format!("unknown {field} '{other}'"))),
    }
}

/// Typed `object.toml` manifest bundling metadata, scripts, interactions, slots,
/// constants, callbacks, tiles, string tables, procedural sprites, and synthesized audio.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectManifest {
    pub name: String,
    pub description: String,
    pub guid: u32,
    pub price: u16,
    pub object_type: u16,
    pub catalog_sort_flags: u16,
    pub objd_id: u16,
    pub ttab_id: u16,
    pub slot_id: u16,
    pub ctss_id: u16,
    pub dgrp_id: u16,
    pub spr2_id: u16,
    pub palt_id: u16,
    pub master_id: Option<u16>,
    pub attributes: u16,
    pub interaction_group: i16,
    pub data_58: i16,
    pub init: Option<TreeRef>,
    pub main: Option<TreeRef>,
    pub scripts: Vec<String>,
    pub sprite: SpriteSpec,
    /// Optional additional graphic states emitted at `dgrp_id + 1..` (`OBJD.num_graphics`).
    pub extra_graphics: Vec<SpriteSpec>,
    pub interactions: Vec<InteractionSpec>,
    pub slots: Vec<SlotSpec>,
    pub sounds: Vec<SoundSpec>,
    pub constants: Vec<ConstantSpec>,
    pub callbacks: Vec<CallbackSpec>,
    pub tiles: Vec<TileSpec>,
    pub string_tables: Vec<StringTableSpec>,
}

impl ObjectManifest {
    /// Parses an `object.toml` document, strictly validating section names and types.
    pub fn parse(source: &str) -> Result<Self> {
        let root = parse_toml(source)?;
        for (key, value) in &root {
            match (key.as_str(), value) {
                ("object" | "sprite", TomlValue::Table(_)) => {}
                (
                    "interaction" | "slot" | "sound" | "constant" | "callback" | "tile"
                    | "string_table",
                    TomlValue::Array(_),
                ) => {}
                ("object" | "sprite", _) => {
                    return Err(Error::new(format!(
                        "section '[{key}]' must be a table, not an array of tables"
                    )));
                }
                (
                    "interaction" | "slot" | "sound" | "constant" | "callback" | "tile"
                    | "string_table",
                    _,
                ) => {
                    return Err(Error::new(format!(
                        "section '{key}' must be declared as an array of tables '[[{key}]]'"
                    )));
                }
                (other, _) => {
                    return Err(Error::new(format!(
                        "unknown top-level key or section '{other}' in object.toml"
                    )));
                }
            }
        }

        let obj = get_table(&root, "object")
            .ok_or_else(|| Error::new("object.toml requires an [object] section"))?;

        let name = req_str(obj, "name")?;
        let description = opt_str(obj, "description").unwrap_or_default();
        let guid = req_u32(obj, "guid")?;
        let price = opt_u16(obj, "price")?.unwrap_or(100);
        let object_type = opt_u16(obj, "type")?.unwrap_or(4);
        let objd_id = opt_u16(obj, "objd_id")?.unwrap_or(128);
        let ttab_id = opt_u16(obj, "ttab_id")?.unwrap_or(128);
        let slot_id = opt_u16(obj, "slot_id")?.unwrap_or(128);
        let ctss_id = opt_u16(obj, "ctss_id")?.unwrap_or(2000);
        let dgrp_id = opt_u16(obj, "dgrp_id")?.unwrap_or(100);
        let spr2_id = opt_u16(obj, "spr2_id")?.unwrap_or(100);
        let palt_id = opt_u16(obj, "palt_id")?.unwrap_or(100);
        let master_id = opt_u16(obj, "master_id")?;
        let attributes = opt_u16(obj, "attributes")?.unwrap_or(0);
        let interaction_group = opt_i16(obj, "interaction_group")?.unwrap_or(0);
        let data_58 = opt_i16(obj, "data_58")?.unwrap_or(0);
        let init = opt_tree_ref(obj, "init")?;
        let main = opt_tree_ref(obj, "main")?;
        let scripts = opt_str_array(obj, "scripts")?;
        for script in &scripts {
            validate_relative_filename(script, "[object].scripts")?;
        }

        let sprite = match get_table(&root, "sprite") {
            Some(t) => SpriteSpec {
                shape: match opt_str(t, "shape").as_deref() {
                    Some(name) => parse_furniture_shape(name, "[sprite].shape")?,
                    None => FurnitureShape::Box,
                },
                primary_rgb: opt_rgb(t, "primary_rgb")?.unwrap_or([152, 112, 76]),
                accent_rgb: opt_rgb(t, "accent_rgb")?.unwrap_or([204, 172, 132]),
                trim_rgb: opt_rgb(t, "trim_rgb")?.unwrap_or([84, 58, 38]),
            },
            None => SpriteSpec::default(),
        };

        let mut interactions = Vec::new();
        if let Some(TomlValue::Array(arr)) = root.get("interaction") {
            for item in arr {
                let TomlValue::Table(t) = item else {
                    return Err(Error::new("[[interaction]] entries must be tables"));
                };
                let name = req_str(t, "name")?;
                let action = opt_tree_ref(t, "action")?
                    .ok_or_else(|| Error::new("[[interaction]] requires 'action'"))?;
                let test_ref = opt_tree_ref(t, "test")?;
                let guard_ref = opt_tree_ref(t, "guard")?;
                if test_ref.is_some() && guard_ref.is_some() {
                    return Err(Error::new(format!(
                        "[[interaction]] '{name}' cannot specify both 'test' and 'guard'"
                    )));
                }
                let test = test_ref.or(guard_ref);
                let flags = opt_u32(t, "flags")?.unwrap_or(0);
                let flags2 = opt_u32(t, "flags2")?.unwrap_or(0);
                let autonomy = opt_u32(t, "autonomy")?.unwrap_or(50);
                let attenuation_code = opt_u32(t, "attenuation_code")?.unwrap_or(0);
                let attenuation = opt_f32(t, "attenuation")?.unwrap_or(0.02);
                let joining_index = opt_i32(t, "joining_index")?.unwrap_or(-1);
                let mut motives = Vec::new();
                if let Some(m_field) = t.get("motives") {
                    let TomlValue::Array(m_arr) = m_field else {
                        return Err(Error::new("[[interaction]].motives must be an array"));
                    };
                    for m_val in m_arr {
                        let TomlValue::Table(mt) = m_val else {
                            return Err(Error::new("interaction motive entries must be tables"));
                        };
                        let motive_index = parse_motive_index(mt)?;
                        let minimum = opt_i16(mt, "min")?.unwrap_or(-100);
                        let delta = opt_i16(mt, "delta")?.unwrap_or(0);
                        let personality = opt_u16(mt, "personality")?.unwrap_or(0);
                        motives.push(MotiveAdSpec {
                            motive_index,
                            minimum,
                            delta,
                            personality,
                        });
                    }
                }
                interactions.push(InteractionSpec {
                    name,
                    action,
                    test,
                    flags,
                    flags2,
                    autonomy,
                    attenuation_code,
                    attenuation,
                    joining_index,
                    motives,
                });
            }
        }

        let mut slots = Vec::new();
        if let Some(TomlValue::Array(arr)) = root.get("slot") {
            for item in arr {
                let TomlValue::Table(t) = item else {
                    return Err(Error::new("[[slot]] entries must be tables"));
                };
                slots.push(SlotSpec {
                    kind: opt_u16(t, "kind")?.unwrap_or(3),
                    offset: opt_f32_3(t, "offset")?.unwrap_or([0.0, -0.75, 0.0]),
                    standing: opt_i32(t, "standing")?.unwrap_or(1),
                    sitting: opt_i32(t, "sitting")?.unwrap_or(0),
                    ground: opt_i32(t, "ground")?.unwrap_or(1),
                    flags: opt_u32(t, "flags")?.unwrap_or(1),
                    snap_target: opt_i32(t, "snap_target")?.unwrap_or(-1),
                    proximity: opt_i32_3(t, "proximity")?.unwrap_or([0, 0, 0]),
                    max_size: opt_i32(t, "max_size")?.unwrap_or(100),
                    gradient: opt_f32(t, "gradient")?.unwrap_or(0.0),
                    height: opt_i32(t, "height")?.unwrap_or(5),
                    facing: opt_i32(t, "facing")?.unwrap_or(0),
                    resolution: opt_i32(t, "resolution")?.unwrap_or(16),
                });
            }
        }

        let mut sounds = Vec::new();
        if let Some(TomlValue::Array(arr)) = root.get("sound") {
            for item in arr {
                let TomlValue::Table(t) = item else {
                    return Err(Error::new("[[sound]] entries must be tables"));
                };
                let id = req_u16(t, "id")?;
                let name = req_str(t, "name")?;
                validate_relative_filename(&name, "[[sound]].name")?;
                let preset = match opt_str(t, "preset").as_deref() {
                    Some("ui_click") | None => SoundPreset::UiClick,
                    Some("chime") => SoundPreset::Chime,
                    Some("wood_creak") => SoundPreset::WoodCreak,
                    Some("water_flush") => SoundPreset::WaterFlush,
                    Some("soft_thud") => SoundPreset::SoftThud,
                    Some("phone_ring") => SoundPreset::PhoneRing,
                    Some("phone_dial") => SoundPreset::PhoneDial,
                    Some("sizzle") => SoundPreset::Sizzle,
                    Some("shower_spray") => SoundPreset::ShowerSpray,
                    Some("wrench_clank") => SoundPreset::WrenchClank,
                    Some(other) => {
                        return Err(Error::new(format!("unknown [[sound]].preset '{other}'")));
                    }
                };
                sounds.push(SoundSpec { id, name, preset });
            }
        }

        let mut constants = Vec::new();
        if let Some(TomlValue::Array(arr)) = root.get("constant") {
            for item in arr {
                let TomlValue::Table(t) = item else {
                    return Err(Error::new("[[constant]] entries must be tables"));
                };
                let id = req_u16(t, "id")?;
                let label = opt_str(t, "label")
                    .or_else(|| opt_str(t, "name"))
                    .unwrap_or_else(|| format!("BCON_{id}"));
                let flags = opt_u8(t, "flags")?.unwrap_or(0);
                let values = req_i16_array(t, "values")?;
                if values.len() > 255 {
                    return Err(Error::new("[[constant]].values cannot exceed 255 entries"));
                }
                constants.push(ConstantSpec {
                    id,
                    label,
                    flags,
                    values,
                });
            }
        }

        let mut callbacks = Vec::new();
        let mut seen_callback_indices = std::collections::BTreeSet::new();
        if let Some(TomlValue::Array(arr)) = root.get("callback") {
            for item in arr {
                let TomlValue::Table(t) = item else {
                    return Err(Error::new("[[callback]] entries must be tables"));
                };
                let index = usize::from(req_u16(t, "index")?);
                if index > 64 {
                    return Err(Error::new(format!(
                        "[[callback]].index {index} exceeds reasonable OBJf table bound (64)"
                    )));
                }
                if index <= 1 {
                    return Err(Error::new(format!(
                        "[[callback]].index {index} is reserved for Init (0) / Main (1); set 'init' or 'main' on [object] or [[tile]] instead"
                    )));
                }
                if !seen_callback_indices.insert(index) {
                    return Err(Error::new(format!("duplicate [[callback]].index {index}")));
                }
                let action = opt_tree_ref(t, "action")?;
                let condition = opt_tree_ref(t, "condition")?;
                callbacks.push(CallbackSpec {
                    index,
                    action,
                    condition,
                });
            }
        }

        let mut tiles = Vec::new();
        if let Some(TomlValue::Array(arr)) = root.get("tile") {
            for item in arr {
                let TomlValue::Table(t) = item else {
                    return Err(Error::new("[[tile]] entries must be tables"));
                };
                let tile_guid = req_u32(t, "guid")?;
                let tile_name = opt_str(t, "name");
                let tile_objd_id = opt_u16(t, "objd_id")?;
                let sub_index = parse_sub_index(t)?;
                let level = opt_i16(t, "level")?.unwrap_or(0);
                let lead = opt_bool(t, "lead")?.unwrap_or(false);
                let tile_interaction_group = opt_i16(t, "interaction_group")?;
                let tile_dgrp_id = opt_u16(t, "dgrp_id")?;
                let tile_shape = match opt_str(t, "shape").as_deref() {
                    Some(name) => Some(parse_furniture_shape(name, "[[tile]].shape")?),
                    None => None,
                };
                let tile_slot_id = opt_u16(t, "slot_id")?;
                let tile_ttab_id = opt_u16(t, "ttab_id")?;
                let tile_init = opt_tree_ref(t, "init")?;
                let tile_main = opt_tree_ref(t, "main")?;
                let tile_attributes = opt_u16(t, "attributes")?;
                tiles.push(TileSpec {
                    guid: tile_guid,
                    name: tile_name,
                    objd_id: tile_objd_id,
                    sub_index,
                    level,
                    lead,
                    interaction_group: tile_interaction_group,
                    dgrp_id: tile_dgrp_id,
                    shape: tile_shape,
                    slot_id: tile_slot_id,
                    ttab_id: tile_ttab_id,
                    init: tile_init,
                    main: tile_main,
                    attributes: tile_attributes,
                });
            }
        }

        let mut string_tables = Vec::new();
        if let Some(TomlValue::Array(arr)) = root.get("string_table") {
            for item in arr {
                let TomlValue::Table(t) = item else {
                    return Err(Error::new("[[string_table]] entries must be tables"));
                };
                let id = req_u16(t, "id")?;
                let label = opt_str(t, "label")
                    .or_else(|| opt_str(t, "name"))
                    .unwrap_or_else(|| format!("STR_{id}"));
                let entries = opt_str_array(t, "entries")?;
                string_tables.push(StringTableSpec { id, label, entries });
            }
        }

        Ok(Self {
            name,
            description,
            guid,
            price,
            object_type,
            catalog_sort_flags: 0,
            objd_id,
            ttab_id,
            slot_id,
            ctss_id,
            dgrp_id,
            spr2_id,
            palt_id,
            master_id,
            attributes,
            interaction_group,
            data_58,
            init,
            main,
            scripts,
            sprite,
            extra_graphics: Vec::new(),
            interactions,
            slots,
            sounds,
            constants,
            callbacks,
            tiles,
            string_tables,
        })
    }
}

fn opt_bool(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<bool>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(Error::new(format!("key '{key}' must be a boolean"))),
    }
}

fn parse_sub_index(t: &BTreeMap<String, TomlValue>) -> Result<i16> {
    match t.get("sub_index") {
        None => Ok(0),
        Some(TomlValue::Int(v)) => {
            let val = i16::try_from(*v)
                .map_err(|_| Error::new(format!("[[tile]].sub_index {v} out of i16 range")))?;
            if val == -1 {
                return Err(Error::new(
                    "[[tile]].sub_index cannot be -1 (reserved for the catalog master OBJD)",
                ));
            }
            Ok(val)
        }
        Some(TomlValue::Array(items)) if items.len() == 2 => {
            let (TomlValue::Int(dx), TomlValue::Int(dy)) = (&items[0], &items[1]) else {
                return Err(Error::new(
                    "[[tile]].sub_index [dx, dy] entries must be integers",
                ));
            };
            let dx = i8::try_from(*dx)
                .map_err(|_| Error::new(format!("[[tile]].sub_index dx {dx} out of i8 range")))?;
            let dy = u8::try_from(*dy)
                .map_err(|_| Error::new(format!("[[tile]].sub_index dy {dy} out of u8 range")))?;
            let encoded = (i16::from(dx) << 8) | i16::from(dy);
            if encoded == -1 {
                return Err(Error::new(
                    "[[tile]].sub_index [-1, 255] collides with master sentinel -1",
                ));
            }
            Ok(encoded)
        }
        Some(_) => Err(Error::new(
            "[[tile]].sub_index must be an i16 or a 2-element [dx, dy] integer array",
        )),
    }
}

fn get_table<'a>(
    root: &'a BTreeMap<String, TomlValue>,
    key: &str,
) -> Option<&'a BTreeMap<String, TomlValue>> {
    match root.get(key) {
        Some(TomlValue::Table(t)) => Some(t),
        _ => None,
    }
}

fn req_str(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<String> {
    opt_str(t, key).ok_or_else(|| Error::new(format!("missing required string key '{key}'")))
}

fn opt_str(t: &BTreeMap<String, TomlValue>, key: &str) -> Option<String> {
    match t.get(key) {
        Some(TomlValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn opt_u8(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<u8>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Int(v)) => u8::try_from(*v)
            .map(Some)
            .map_err(|_| Error::new(format!("key '{key}' value {v} out of u8 range"))),
        Some(_) => Err(Error::new(format!("key '{key}' must be an integer"))),
    }
}

fn req_i16_array(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Vec<i16>> {
    match t.get(key) {
        Some(TomlValue::Array(items)) => items
            .iter()
            .map(|v| match v {
                TomlValue::Int(n) => i16::try_from(*n)
                    .map_err(|_| Error::new(format!("'{key}' entry {n} out of i16 range"))),
                _ => Err(Error::new(format!("'{key}' array must contain integers"))),
            })
            .collect(),
        Some(_) => Err(Error::new(format!("'{key}' must be an array of integers"))),
        None => Err(Error::new(format!("missing required array key '{key}'"))),
    }
}

fn req_u16(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<u16> {
    opt_u16(t, key)?.ok_or_else(|| Error::new(format!("missing required u16 key '{key}'")))
}

fn opt_u16(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<u16>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Int(v)) => u16::try_from(*v)
            .map(Some)
            .map_err(|_| Error::new(format!("key '{key}' value {v} out of u16 range"))),
        Some(_) => Err(Error::new(format!("key '{key}' must be an integer"))),
    }
}

fn opt_i16(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<i16>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Int(v)) => i16::try_from(*v)
            .map(Some)
            .map_err(|_| Error::new(format!("key '{key}' value {v} out of i16 range"))),
        Some(_) => Err(Error::new(format!("key '{key}' must be an integer"))),
    }
}

fn req_u32(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<u32> {
    opt_u32(t, key)?.ok_or_else(|| Error::new(format!("missing required u32 key '{key}'")))
}

fn opt_u32(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<u32>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Int(v)) => u32::try_from(*v)
            .map(Some)
            .map_err(|_| Error::new(format!("key '{key}' value {v} out of u32 range"))),
        Some(_) => Err(Error::new(format!("key '{key}' must be an integer"))),
    }
}

fn opt_i32(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<i32>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Int(v)) => i32::try_from(*v)
            .map(Some)
            .map_err(|_| Error::new(format!("key '{key}' value {v} out of i32 range"))),
        Some(_) => Err(Error::new(format!("key '{key}' must be an integer"))),
    }
}

fn opt_f32(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<f32>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Float(v)) => Ok(Some(*v as f32)),
        Some(TomlValue::Int(v)) => Ok(Some(*v as f32)),
        Some(_) => Err(Error::new(format!("key '{key}' must be a number"))),
    }
}

fn opt_tree_ref(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<TreeRef>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Int(v)) => u16::try_from(*v)
            .map(|id| Some(TreeRef::Id(id)))
            .map_err(|_| Error::new(format!("tree id '{key}' out of u16 range"))),
        Some(TomlValue::String(s)) => Ok(Some(TreeRef::Name(s.clone()))),
        Some(_) => Err(Error::new(format!(
            "tree reference '{key}' must be a string name or integer id"
        ))),
    }
}

fn opt_str_array(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Vec<String>> {
    match t.get(key) {
        None => Ok(Vec::new()),
        Some(TomlValue::Array(items)) => items
            .iter()
            .map(|v| match v {
                TomlValue::String(s) => Ok(s.clone()),
                _ => Err(Error::new(format!("'{key}' array must contain strings"))),
            })
            .collect(),
        Some(_) => Err(Error::new(format!("'{key}' must be an array of strings"))),
    }
}

fn opt_rgb(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<[u8; 3]>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Array(items)) if items.len() == 3 => {
            let mut rgb = [0u8; 3];
            for (i, item) in items.iter().enumerate() {
                let TomlValue::Int(v) = item else {
                    return Err(Error::new(format!("'{key}' elements must be integers")));
                };
                rgb[i] = u8::try_from(*v)
                    .map_err(|_| Error::new(format!("'{key}' channel {v} out of 0..=255")))?;
            }
            Ok(Some(rgb))
        }
        Some(_) => Err(Error::new(format!(
            "'{key}' must be a 3-element [r, g, b] array"
        ))),
    }
}

fn opt_f32_3(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<[f32; 3]>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Array(items)) if items.len() == 3 => {
            let mut out = [0.0f32; 3];
            for (i, item) in items.iter().enumerate() {
                out[i] = match item {
                    TomlValue::Float(v) => *v as f32,
                    TomlValue::Int(v) => *v as f32,
                    _ => return Err(Error::new(format!("'{key}' elements must be numbers"))),
                };
            }
            Ok(Some(out))
        }
        Some(_) => Err(Error::new(format!("'{key}' must be a 3-element array"))),
    }
}

fn opt_i32_3(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<[i32; 3]>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Array(items)) if items.len() == 3 => {
            let mut out = [0i32; 3];
            for (i, item) in items.iter().enumerate() {
                let TomlValue::Int(v) = item else {
                    return Err(Error::new(format!("'{key}' elements must be integers")));
                };
                out[i] = i32::try_from(*v)
                    .map_err(|_| Error::new(format!("'{key}' element {v} out of i32 range")))?;
            }
            Ok(Some(out))
        }
        Some(_) => Err(Error::new(format!("'{key}' must be a 3-element array"))),
    }
}

fn parse_motive_index(t: &BTreeMap<String, TomlValue>) -> Result<usize> {
    match t.get("motive") {
        Some(TomlValue::Int(v)) => usize::try_from(*v)
            .ok()
            .filter(|&i| i < 16)
            .ok_or_else(|| Error::new(format!("motive index {v} out of 0..16 range"))),
        Some(TomlValue::String(name)) => motive_by_name(name)
            .map(usize::from)
            .ok_or_else(|| Error::new(format!("unknown motive name '{name}'"))),
        _ => Err(Error::new("motive entry requires a 'motive' name or index")),
    }
}
