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

pub mod ast;
pub mod decompile;
pub mod fmt;
pub mod lexer;
pub mod loader;
pub mod lower;
pub mod parser;

pub use decompile::{
    DecompileV2Options, DecompileV2Report, DecompiledTree, TreeCheck, TreeStyle,
    decompile_iff_to_v2, decompile_iff_to_v2_report,
};
pub use fmt::{format_v2_source, is_v2_formatted};
pub use loader::{PackResolverConfig, ResolvedModule, resolve_v2_file, resolve_v2_source};
pub use lower::{ExpandedV2Object, lower_resolved_module};
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
    compile_object_from_manifest(expanded.manifest, &[(&air_label, &expanded.air_source)])
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

        for guid in std::iter::once(compiled.guid).chain(compiled.tile_guids.iter().copied()) {
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
