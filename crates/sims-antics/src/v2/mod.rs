//! High-level `.antics` v2 authoring language frontend and `.antics -> .air -> .iff`
//! lowering compiler.
//!
//! Architectural split:
//! - **`.air` (Antics IR):** Explicit low-level textual IR (`bcon`, `tree`, `interaction`,
//!   `my.attr[...]`, `stack.attr[...]`, `op 0xNNNN`) handled by `lexer`/`parser`/`ast`/`lower`/
//!   `lift`/`structure_lift`/`verify`.
//! - **`.antics` v2 (this module):** High-level Rust-flavored object authoring language with
//!   `use` imports, `pub fn` helpers, `pub template`, `object "Name" [from Template]`, flat
//!   `param`/`state`/`sound`/`slot` declarations, `self`/`sim` context safety, `with lock(...)`,
//!   `defer { ... }`, `repeat N cancel_on_abort { ... }`, and short-circuit boolean expressions,
//!   lowering cleanly to `.air` + IFF metadata.
//!
//! Object `for` loops accept an `else { ... }` block on exhaustion, including when
//! `self.parts()` wraps to its starting part. A `break` in the loop bypasses the block.
//! The block can fall through and cannot access the iteration binding or use `break` or
//! `continue` to target an enclosing loop. Range loops do not accept `else`.
//!
//! `set_to_next(part, dest: stack_obj)` steps the current stack object in place. The caller
//! initializes the cursor when needed. `set_to_next(category, dest: stack_obj)` uses
//! `param[0]` as the category, and `set_to_next(on_tile, dest: stack_obj)` uses mode 8
//! to step through the current object's tile. An explicit `dest:` preserves the destination
//! flag in the operand bytes. `create_object(GUID, dest: stack_obj)` leaves the primitive's
//! result in the cursor. In Sim trees that use these operations, `self` still names the entry
//! object and temporary cursor moves preserve the current value.

pub mod ast;
mod attrs;
pub mod decompile;
pub mod fmt;
pub mod lexer;
pub mod lint;
pub mod loader;
pub mod lower;
mod mir;
mod opt;
pub mod parser;
mod raw_facts;
mod specialize;

pub use decompile::{
    DecompileV2Options, DecompileV2Report, DecompiledTree, TreeCheck, TreeStyle,
    decompile_iff_to_v2, decompile_iff_to_v2_report,
};
pub use fmt::{format_v2_source, is_v2_formatted};
pub use lint::{
    LINT_RULES, LintDiagnostic, LintInput, LintRule, Severity, diagnostics_to_json, lint_v2_paths,
    lint_v2_source, lint_v2_sources, pack_dir_of, render_diagnostic,
};
pub use loader::{PackResolverConfig, ResolvedModule, resolve_v2_file, resolve_v2_source};
pub use lower::{ExpandedV2Object, VariantSpec, lower_resolved_module};
pub use parser::parse_v2_file;

use crate::compile::{
    CompiledObject, CompiledPack, CompiledPackObject, compile_object_from_manifest, package_far,
};
use crate::error::{Error, Result};
use crate::manifest::PackManifest;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Returns `true` if `source` begins with a high-level `.antics` v2 top-level keyword
/// (`use`, `pub`, `fn`, `template`, `object`, `enum`, `bitflags`, `extern`) rather than `.air` IR (`bcon`, `str`, `tree`, `interaction`).
///
/// Scans only leading whitespace/comments and the first identifier so v2 files containing
/// later lexer errors are still routed to the v2 compiler for accurate diagnostics.
#[must_use]
pub fn is_v2_source(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut pos = 0usize;
    while pos < bytes.len() {
        while pos < bytes.len() && matches!(bytes[pos], b' ' | b'\t' | b'\r' | b'\n') {
            pos += 1;
        }
        if pos + 1 < bytes.len() && bytes[pos] == b'/' && bytes[pos + 1] == b'/' {
            pos += 2;
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        if pos + 1 < bytes.len() && bytes[pos] == b'/' && bytes[pos + 1] == b'*' {
            pos += 2;
            let mut depth = 1usize;
            while pos + 1 < bytes.len() && depth > 0 {
                if bytes[pos] == b'/' && bytes[pos + 1] == b'*' {
                    depth += 1;
                    pos += 2;
                } else if bytes[pos] == b'*' && bytes[pos + 1] == b'/' {
                    depth -= 1;
                    pos += 2;
                } else {
                    pos += 1;
                }
            }
            continue;
        }
        break;
    }
    let start = pos;
    while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
        pos += 1;
    }
    matches!(
        &source[start..pos],
        "use" | "pub" | "fn" | "template" | "object" | "enum" | "bitflags" | "extern"
    )
}

/// Expands one `.antics` v2 source file on disk into its lowered `.air` (Antics IR) text.
///
/// If the file contains multiple `object` declarations, their lowered `.air` outputs
/// are separated cleanly by blank lines.
pub fn expand_v2_to_air(file_path: &Path, pack_dir: Option<&Path>) -> Result<String> {
    let resolved = resolve_v2_file(file_path, pack_dir)?;
    let expanded = lower_resolved_module(&resolved)?;
    Ok(expanded
        .into_iter()
        .map(|o| o.air_source)
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Expands an in-memory `.antics` v2 source string (with optional virtual/on-disk pack resolver)
/// into lowered [`ExpandedV2Object`]s containing both `.air` text and `ObjectManifest`.
pub fn expand_v2_source_to_objects(
    source_name: &str,
    source: &str,
    config: &PackResolverConfig,
) -> Result<Vec<ExpandedV2Object>> {
    let resolved = resolve_v2_source(source_name, source, config)?;
    lower_resolved_module(&resolved)
}

/// Compiles one in-memory `.antics` v2 source string (containing a single `object`)
/// through `.antics -> .air -> .iff` into a [`CompiledObject`].
pub fn compile_v2_source(
    source_name: &str,
    source: &str,
    config: &PackResolverConfig,
) -> Result<CompiledObject> {
    let mut expanded = expand_v2_source_to_objects(source_name, source, config)?;
    if expanded.len() != 1 {
        return Err(Error::new(format!(
            "'{source_name}' declares {} objects; compile_v2_source expects exactly 1",
            expanded.len()
        )));
    }
    let obj = expanded.remove(0);
    compile_expanded_v2_object(obj)
}

/// Compiles one `.antics` v2 object file on disk through `.antics -> .air -> .iff`
/// into a [`CompiledObject`].
pub fn compile_v2_object_file(file_path: &Path, pack_dir: Option<&Path>) -> Result<CompiledObject> {
    let resolved = resolve_v2_file(file_path, pack_dir)?;
    let mut expanded = lower_resolved_module(&resolved)?;
    if expanded.len() != 1 {
        return Err(Error::new(format!(
            "{} declares {} objects; compile_v2_object_file expects exactly 1",
            file_path.display(),
            expanded.len()
        )));
    }
    compile_expanded_v2_object(expanded.remove(0))
}

/// Compiles an already-expanded [`ExpandedV2Object`] through the `.air` compiler and IFF encoder.
pub fn compile_expanded_v2_object(expanded: ExpandedV2Object) -> Result<CompiledObject> {
    let air_label = format!("{}.air", expanded.slug);
    let objd_id = expanded.manifest.objd_id;
    let ctss_id = expanded.manifest.ctss_id;
    let mut compiled =
        compile_object_from_manifest(expanded.manifest, &[(&air_label, &expanded.air_source)])?;
    if !expanded.variants.is_empty() {
        compiled.iff_bytes =
            append_variants(&compiled.iff_bytes, objd_id, ctss_id, &expanded.variants)?;
    }
    if !expanded.shared_sound_names.is_empty() {
        compiled.iff_bytes =
            rename_shared_sounds(&compiled.iff_bytes, &expanded.shared_sound_names)?;
        for (_, placeholder, _) in &expanded.shared_sound_names {
            compiled.sounds_wav.remove(placeholder);
        }
    }
    Ok(compiled)
}

/// Gives each listed FWAV chunk (`(id, placeholder, name)`) its shared sound name and label
/// back. The compiler rejects repeated sound names, so the manifest carried placeholders.
fn rename_shared_sounds(iff_bytes: &[u8], renames: &[(u16, String, String)]) -> Result<Vec<u8>> {
    use crate::encode::{encode_fwav, encode_iff_with_rsmp};
    use crate::ir::FwavChunk;
    use sims_formats::iff::{ChunkData, Iff};

    let iff = Iff::parse(iff_bytes.to_vec())
        .map_err(|e| Error::new(format!("re-reading compiled IFF for sound names: {e}")))?;
    let mut chunks: Vec<ChunkData> = iff
        .chunks
        .iter()
        .filter(|c| &c.tag != b"rsmp")
        .map(|c| ChunkData {
            tag: c.tag,
            id: c.id,
            flags: c.flags,
            label: c.label.clone(),
            data: iff.data(c).to_vec(),
        })
        .collect();
    for (id, placeholder, name) in renames {
        let chunk = chunks
            .iter_mut()
            .find(|c| &c.tag == b"FWAV" && c.id == *id && c.label == *placeholder)
            .ok_or_else(|| Error::new(format!("compiled IFF has no FWAV #{id} '{placeholder}'")))?;
        chunk.label = name.clone();
        chunk.data = encode_fwav(&FwavChunk {
            id: *id,
            label: name.clone(),
            sound_name: name.clone(),
        })?;
    }
    encode_iff_with_rsmp(&chunks)
}

/// OBJD words a catalog variant replaces (word index into the chunk, version words included).
const OBJD_MAIN_ID: usize = 5;
const OBJD_GUID_LO: usize = 14;
const OBJD_GUID_HI: usize = 15;
const OBJD_PRICE: usize = 18;
const OBJD_CATALOG_STRINGS_ID: usize = 41;
const OBJD_INIT_ID: usize = 43;

/// Appends one OBJD + OBJf + CTSS per catalog variant to a compiled single-tile IFF. Each is
/// the primary's chunk with the variant's words replaced, so every other definition word (and
/// the whole BHAV/BCON/STR#/TTAB/SLOT/graphics pool) is shared. Ids follow the primary's.
fn append_variants(
    iff_bytes: &[u8],
    objd_id: u16,
    ctss_id: u16,
    variants: &[VariantSpec],
) -> Result<Vec<u8>> {
    use crate::encode::{encode_iff_with_rsmp, encode_string_table};
    use crate::ir::{StringEntry, StringFormat, StringTableChunk};
    use sims_formats::iff::{ChunkData, Iff};

    let iff = Iff::parse(iff_bytes.to_vec())
        .map_err(|e| Error::new(format!("re-reading compiled IFF for variants: {e}")))?;
    let mut chunks: Vec<ChunkData> = iff
        .chunks
        .iter()
        .filter(|c| &c.tag != b"rsmp")
        .map(|c| ChunkData {
            tag: c.tag,
            id: c.id,
            flags: c.flags,
            label: c.label.clone(),
            data: iff.data(c).to_vec(),
        })
        .collect();
    let find = |chunks: &[ChunkData], tag: &[u8; 4], id: u16| {
        chunks.iter().position(|c| &c.tag == tag && c.id == id)
    };
    let objd_pos = find(&chunks, b"OBJD", objd_id)
        .ok_or_else(|| Error::new("compiled IFF has no primary OBJD"))?;
    let objf_pos = find(&chunks, b"OBJf", objd_id)
        .ok_or_else(|| Error::new("compiled IFF has no primary OBJf"))?;
    let ctss_pos = find(&chunks, b"CTSS", ctss_id)
        .ok_or_else(|| Error::new("compiled IFF has no primary CTSS"))?;
    let objd = chunks[objd_pos].clone();
    let objf = chunks[objf_pos].clone();
    let ctss_flags = chunks[ctss_pos].flags;
    if objd.data.len() < 2 * (OBJD_INIT_ID + 1) {
        return Err(Error::new("primary OBJD is too short for catalog variants"));
    }
    if objf.data.len() < 16 + 2 * 4 {
        return Err(Error::new("primary OBJf has fewer than 2 entries"));
    }

    let mut defs = Vec::new();
    let mut strings = Vec::new();
    for (k, v) in variants.iter().enumerate() {
        let offset = u16::try_from(k + 1).map_err(|_| Error::new("too many variants"))?;
        let (Some(vid), Some(vctss)) = (objd_id.checked_add(offset), ctss_id.checked_add(offset))
        else {
            return Err(Error::new(format!(
                "variant '{}' runs past the last OBJD/CTSS id",
                v.name
            )));
        };
        for (tag, id) in [(b"OBJD", vid), (b"OBJf", vid), (b"CTSS", vctss)] {
            if find(&chunks, tag, id).is_some() {
                return Err(Error::new(format!(
                    "variant '{}' needs {} id {id}, which the object already uses",
                    v.name,
                    String::from_utf8_lossy(tag)
                )));
            }
        }
        let mut def = objd.data.clone();
        let mut put = |word: usize, value: u16| {
            def[2 * word..2 * word + 2].copy_from_slice(&value.to_le_bytes());
        };
        put(OBJD_GUID_LO, (v.guid & 0xFFFF) as u16);
        put(OBJD_GUID_HI, (v.guid >> 16) as u16);
        put(OBJD_PRICE, v.price);
        put(OBJD_CATALOG_STRINGS_ID, vctss);
        put(OBJD_INIT_ID, v.init_id);
        put(OBJD_MAIN_ID, v.main_id);
        defs.push(ChunkData {
            tag: *b"OBJD",
            id: vid,
            flags: objd.flags,
            label: v.name.clone(),
            data: def,
        });
        let mut funcs = objf.data.clone();
        funcs[18..20].copy_from_slice(&v.init_id.to_le_bytes());
        funcs[22..24].copy_from_slice(&v.main_id.to_le_bytes());
        defs.push(ChunkData {
            tag: *b"OBJf",
            id: vid,
            flags: objf.flags,
            label: format!("{} Functions", v.name),
            data: funcs,
        });
        let table = StringTableChunk::new(
            *b"CTSS",
            vctss,
            format!("{} Catalog", v.name),
            StringFormat::Localized,
            vec![
                StringEntry::english(&v.name),
                StringEntry::english(&v.description),
            ],
        );
        strings.push(ChunkData {
            tag: *b"CTSS",
            id: vctss,
            flags: ctss_flags,
            label: table.label.clone(),
            data: encode_string_table(&table)?,
        });
    }
    // Definitions go right after the primary's OBJf, catalog strings after its CTSS.
    let (first, second) = if objf_pos < ctss_pos {
        ((objf_pos, defs), (ctss_pos, strings))
    } else {
        ((ctss_pos, strings), (objf_pos, defs))
    };
    chunks.splice(second.0 + 1..second.0 + 1, second.1);
    chunks.splice(first.0 + 1..first.0 + 1, first.1);
    encode_iff_with_rsmp(&chunks)
}

/// Every GUID an IFF's OBJD chunks define (primary, tiles, and catalog variants).
fn iff_objd_guids(iff_bytes: &[u8]) -> Result<Vec<u32>> {
    use sims_formats::iff::Iff;
    let iff = Iff::parse(iff_bytes.to_vec())
        .map_err(|e| Error::new(format!("re-reading compiled IFF: {e}")))?;
    Ok(iff
        .chunks
        .iter()
        .filter(|c| &c.tag == b"OBJD")
        .filter_map(|c| {
            let d = iff.data(c);
            let word = |w: usize| {
                d.get(2 * w..2 * w + 2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
            };
            Some(u32::from(word(OBJD_GUID_LO)?) | (u32::from(word(OBJD_GUID_HI)?) << 16))
        })
        .collect())
}

/// Compiles a `.antics` v2 pack directory containing `pack.toml` and v2 object `.antics` files
/// (`objects/<slug>.antics` or `objects/<slug>/object.antics`) into a [`CompiledPack`].
pub fn compile_v2_pack_dir(dir: &Path) -> Result<CompiledPack> {
    let root_meta = std::fs::symlink_metadata(dir)
        .map_err(|e| Error::new(format!("failed to access {}: {e}", dir.display())))?;
    if root_meta.file_type().is_symlink() || !root_meta.file_type().is_dir() {
        return Err(Error::new(format!(
            "pack directory {} must be a real directory, not a symlink",
            dir.display()
        )));
    }
    let root_canon = std::fs::canonicalize(dir)
        .map_err(|e| Error::new(format!("failed to resolve {}: {e}", dir.display())))?;
    let pack_toml_path = root_canon.join("pack.toml");
    let pack_toml = loader::read_regular_file_checked(&pack_toml_path, &root_canon)?;
    let manifest =
        PackManifest::parse(&pack_toml).map_err(|e| e.with_context(pack_toml_path.display()))?;

    let mut objects = Vec::with_capacity(manifest.objects.len());
    let mut seen_guids = BTreeMap::new();
    let mut seen_iff_lower = BTreeSet::new();
    let mut sounds_wav: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut seen_sound_lower: BTreeMap<String, String> = BTreeMap::new();

    for slug in &manifest.objects {
        let iff_name = format!("{}.iff", slug.replace('-', "_"));
        if !seen_iff_lower.insert(iff_name.to_ascii_lowercase()) {
            return Err(Error::new(format!(
                "object slug '{slug}' produces colliding IFF filename '{iff_name}'"
            )));
        }
        let v2_path = resolve_v2_object_path_in_pack(&root_canon, slug)?.ok_or_else(|| {
            Error::new(format!(
                "v2 object '{slug}' not found (expected objects/{slug}.antics or objects/{slug}/object.antics)"
            ))
        })?;
        let compiled = compile_v2_object_file(&v2_path, Some(&root_canon))
            .map_err(|e| e.with_context(format!("object '{slug}'")))?;

        let mut guids: BTreeSet<u32> = iff_objd_guids(&compiled.iff_bytes)?.into_iter().collect();
        guids.insert(compiled.guid);
        guids.extend(compiled.tile_guids.iter().copied());
        for guid in guids {
            if let Some(prev_slug) = seen_guids.insert(guid, slug.clone()) {
                return Err(Error::new(format!(
                    "duplicate GUID {guid:#010x} between objects '{prev_slug}' and '{slug}'"
                )));
            }
        }
        for (sound_name, wav_bytes) in &compiled.sounds_wav {
            let lower = sound_name.to_ascii_lowercase();
            if let Some(prev_name) = seen_sound_lower.get(&lower)
                && prev_name != sound_name
            {
                return Err(Error::new(format!(
                    "case-colliding sound names '{prev_name}' and '{sound_name}' in '{slug}'"
                )));
            }
            seen_sound_lower.insert(lower, sound_name.clone());
            if let Some(existing) = sounds_wav.get(sound_name) {
                if existing != wav_bytes {
                    return Err(Error::new(format!(
                        "conflicting synthesized audio for sound name '{sound_name}' in '{slug}'"
                    )));
                }
            } else {
                sounds_wav.insert(sound_name.clone(), wav_bytes.clone());
            }
        }
        objects.push(CompiledPackObject {
            slug: slug.clone(),
            iff_name,
            object: compiled,
        });
    }

    let far_members: Vec<(&str, &[u8])> = objects
        .iter()
        .map(|o| (o.iff_name.as_str(), o.object.iff_bytes.as_slice()))
        .collect();
    let far_bytes = package_far(&far_members)?;

    Ok(CompiledPack {
        name: manifest.name,
        version: manifest.version,
        archive_name: manifest.archive,
        objects,
        far_bytes,
        sounds_wav,
    })
}

/// Resolves a v2 object `.antics` source file for `slug` inside `root_canon` without following symlinks.
/// Checks `objects/<slug>.antics` and `objects/<slug>/object.antics`.
pub(crate) fn resolve_v2_object_path_in_pack(
    root_canon: &Path,
    slug: &str,
) -> Result<Option<PathBuf>> {
    let objects_dir = root_canon.join("objects");
    if let Ok(ometa) = std::fs::symlink_metadata(&objects_dir) {
        if ometa.file_type().is_symlink() || !ometa.file_type().is_dir() {
            return Err(Error::new(format!(
                "objects directory {} must be a real directory",
                objects_dir.display()
            )));
        }
    } else {
        return Ok(None);
    }

    let flat = objects_dir.join(format!("{slug}.antics"));
    if let Ok(meta) = std::fs::symlink_metadata(&flat) {
        if meta.file_type().is_symlink() {
            return Err(Error::new(format!(
                "refusing to follow symlink {}",
                flat.display()
            )));
        }
        if meta.file_type().is_file() {
            let canon = std::fs::canonicalize(&flat)
                .map_err(|e| Error::new(format!("failed to resolve {}: {e}", flat.display())))?;
            if !canon.starts_with(root_canon) {
                return Err(Error::new(format!(
                    "object file {} escapes pack root",
                    canon.display()
                )));
            }
            return Ok(Some(canon));
        }
    }

    let sub_dir = objects_dir.join(slug);
    if let Ok(dmeta) = std::fs::symlink_metadata(&sub_dir) {
        if dmeta.file_type().is_symlink() {
            return Err(Error::new(format!(
                "refusing to follow symlink {}",
                sub_dir.display()
            )));
        }
        if dmeta.file_type().is_dir() {
            let nested = sub_dir.join("object.antics");
            if let Ok(fmeta) = std::fs::symlink_metadata(&nested) {
                if fmeta.file_type().is_symlink() {
                    return Err(Error::new(format!(
                        "refusing to follow symlink {}",
                        nested.display()
                    )));
                }
                if fmeta.file_type().is_file() {
                    let canon = std::fs::canonicalize(&nested).map_err(|e| {
                        Error::new(format!("failed to resolve {}: {e}", nested.display()))
                    })?;
                    if !canon.starts_with(root_canon) {
                        return Err(Error::new(format!(
                            "object file {} escapes pack root",
                            canon.display()
                        )));
                    }
                    return Ok(Some(canon));
                }
            }
        }
    }
    Ok(None)
}
