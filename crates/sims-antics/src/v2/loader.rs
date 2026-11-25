//! Deterministic module loader and `use` import resolver for `.antics` v2.
//!
//! Enforces symlink-free containment within pack roots, `pub fn`/`pub template`
//! visibility across module boundaries, and cycle detection.

use crate::error::{Error, Result, Span};
use crate::manifest::{TomlValue, parse_toml};
use crate::v2::ast::{
    AbiParamSlot, BitflagsDecl, ConstDecl, EnumDecl, ExternBconDecl, ExternFnDecl, ExternModDecl,
    FnDecl, ObjectDecl, ReturnAbi, TemplateDecl, V2File, V2Item, V2Type, object_type_ident,
};
use crate::v2::parser::{parse_v2_file, parse_v2_std_file};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// A fully parsed and import-resolved module graph rooted at one `.antics` file.
#[derive(Debug, Clone)]
pub struct ResolvedModule {
    /// Canonical module identifier (e.g. `"open_pack::objects::open_cot"` or `"<root>"`).
    pub module_id: String,
    /// Functions visible by local name in this module (local + imported), paired with their owning module id.
    pub functions: BTreeMap<String, (String, FnDecl)>,
    /// Templates visible by local name in this module (local + imported), paired with their owning module id.
    pub templates: BTreeMap<String, (String, TemplateDecl)>,
    /// Enums visible by local name in this module (local + imported), paired with their owning module id.
    pub enums: BTreeMap<String, (String, EnumDecl)>,
    /// Bitflags visible by local name in this module (local + imported), paired with their owning module id.
    pub bitflags: BTreeMap<String, (String, BitflagsDecl)>,
    /// Object declarations visible by type name in this module (local + imported), paired with their owning module id.
    pub imported_objects: BTreeMap<String, (String, ObjectDecl)>,
    /// Concrete object declarations defined in this module.
    pub objects: Vec<ObjectDecl>,
    /// Every transitively loaded module's local scope (keyed by module id) so
    /// functions/templates/objects imported from module M resolve their own references inside M's scope.
    pub module_scopes: BTreeMap<String, ModuleScope>,
}

/// Exported and visible symbols inside a single loaded `.antics` v2 module.
#[derive(Debug, Clone, Default)]
pub struct ModuleScope {
    pub functions: BTreeMap<String, (String, FnDecl)>,
    pub templates: BTreeMap<String, (String, TemplateDecl)>,
    pub enums: BTreeMap<String, (String, EnumDecl)>,
    pub bitflags: BTreeMap<String, (String, BitflagsDecl)>,
    pub objects: BTreeMap<String, (String, ObjectDecl)>,
    /// `extern mod` BHAV bindings visible by local name (local + imported), with their owning module id.
    pub extern_mods: BTreeMap<String, (String, ExternModDecl)>,
    /// `extern bcon` constant bindings visible by local name (local + imported), with their owning module id.
    pub extern_bcons: BTreeMap<String, (String, ExternBconDecl)>,
    /// `const` items visible by local name (local + imported), with their owning module id.
    pub consts: BTreeMap<String, (String, ConstDecl)>,
}

/// Optional pack root and external dependency roots for `use` resolution.
#[derive(Debug, Clone, Default)]
pub struct PackResolverConfig {
    /// Logical pack identifier aliases that map to `pack_root` (e.g. `"open_pack"`).
    pub pack_aliases: BTreeSet<String>,
    /// Root directory of the active pack on disk.
    pub pack_root: Option<PathBuf>,
    /// Declared external dependency pack roots (`dep_pack_name -> dir`).
    pub dependencies: BTreeMap<String, PathBuf>,
    /// Optional in-memory virtual files keyed by `"pack_name::mod::path"` (used in tests/in-memory compilation).
    pub virtual_modules: BTreeMap<String, String>,
}

impl PackResolverConfig {
    /// Builds a resolver config from a pack directory on disk, reading `[pack]` and
    /// optional `[dependencies]` from `pack.toml` if present.
    pub fn from_pack_dir(pack_dir: &Path) -> Result<Self> {
        let meta = std::fs::symlink_metadata(pack_dir).map_err(|e| {
            Error::new(format!(
                "failed to access pack directory {}: {e}",
                pack_dir.display()
            ))
        })?;
        if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
            return Err(Error::new(format!(
                "pack directory {} must be a real directory, not a symlink",
                pack_dir.display()
            )));
        }
        let canon = std::fs::canonicalize(pack_dir).map_err(|e| {
            Error::new(format!(
                "failed to resolve pack directory {}: {e}",
                pack_dir.display()
            ))
        })?;

        let mut aliases = BTreeSet::new();
        aliases.insert("crate".to_owned());
        aliases.insert("self".to_owned());
        if let Some(base) = pack_dir.file_name().and_then(|s| s.to_str()) {
            let norm = normalize_pack_ident(base);
            if !norm.is_empty() {
                aliases.insert(norm);
            }
        }

        let mut dependencies = BTreeMap::new();
        let pack_toml_path = pack_dir.join("pack.toml");
        if let Ok(pt_meta) = std::fs::symlink_metadata(&pack_toml_path)
            && !pt_meta.file_type().is_symlink()
            && pt_meta.file_type().is_file()
        {
            let text = read_regular_file_checked(&pack_toml_path, &canon)?;
            let table = parse_toml(&text).map_err(|e| e.with_context(pack_toml_path.display()))?;
            if let Some(TomlValue::Table(pack_tbl)) = table.get("pack") {
                if let Some(TomlValue::String(id)) = pack_tbl.get("id") {
                    let norm = normalize_pack_ident(id);
                    if !norm.is_empty() {
                        aliases.insert(norm);
                    }
                }
                if let Some(TomlValue::String(name)) = pack_tbl.get("name") {
                    let norm = normalize_pack_ident(name);
                    if !norm.is_empty() {
                        aliases.insert(norm);
                    }
                }
            }
            if let Some(TomlValue::Table(deps_tbl)) = table.get("dependencies") {
                for (dep_name, dep_val) in deps_tbl {
                    validate_ident_segment(dep_name, Span::default())?;
                    let TomlValue::String(rel) = dep_val else {
                        return Err(Error::new(format!(
                            "[dependencies].{dep_name} in {} must be a relative directory string",
                            pack_toml_path.display()
                        )));
                    };
                    let dep_canon = resolve_relative_dir_no_symlinks(&canon, rel, dep_name)?;
                    dependencies.insert(normalize_pack_ident(dep_name), dep_canon);
                }
            }
        }

        Ok(Self {
            pack_aliases: aliases,
            pack_root: Some(canon),
            dependencies,
            virtual_modules: BTreeMap::new(),
        })
    }

    /// Infers a pack root by walking up from `file_path` looking for `pack.toml`
    /// without following symlinks; falls back to the file's parent directory.
    pub fn infer_for_file(file_path: &Path) -> Result<Self> {
        let start_dir = match file_path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        let mut cur = start_dir;
        for _ in 0..8 {
            let candidate = cur.join("pack.toml");
            if let Ok(meta) = std::fs::symlink_metadata(&candidate)
                && !meta.file_type().is_symlink()
                && meta.file_type().is_file()
            {
                return Self::from_pack_dir(cur);
            }
            let Some(parent) = cur.parent() else {
                break;
            };
            if parent.as_os_str().is_empty() {
                break;
            }
            cur = parent;
        }
        Self::from_pack_dir(start_dir)
    }
}

/// Resolves a relative dependency directory from `base_canon`, rejecting absolute paths
/// and any symlink component along the traversal.
fn resolve_relative_dir_no_symlinks(
    base_canon: &Path,
    rel: &str,
    dep_name: &str,
) -> Result<PathBuf> {
    use std::path::Component;

    if rel.is_empty() {
        return Err(Error::new(format!(
            "[dependencies].{dep_name} path cannot be empty"
        )));
    }
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(Error::new(format!(
            "[dependencies].{dep_name} must be a relative path, got '{}'",
            rel
        )));
    }
    let mut cur = base_canon.to_path_buf();
    for comp in rel_path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !cur.pop() {
                    return Err(Error::new(format!(
                        "[dependencies].{dep_name} path '{rel}' escapes filesystem root"
                    )));
                }
            }
            Component::Normal(seg) => {
                cur.push(seg);
                let meta = std::fs::symlink_metadata(&cur).map_err(|e| {
                    Error::new(format!(
                        "failed to access dependency pack '{dep_name}' at {}: {e}",
                        cur.display()
                    ))
                })?;
                if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
                    return Err(Error::new(format!(
                        "dependency pack '{dep_name}' path component {} must be a real directory, not a symlink",
                        cur.display()
                    )));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(Error::new(format!(
                    "[dependencies].{dep_name} must be a relative path, got '{rel}'"
                )));
            }
        }
    }
    let final_meta = std::fs::symlink_metadata(&cur).map_err(|e| {
        Error::new(format!(
            "failed to access dependency pack '{dep_name}' at {}: {e}",
            cur.display()
        ))
    })?;
    if final_meta.file_type().is_symlink() || !final_meta.file_type().is_dir() {
        return Err(Error::new(format!(
            "dependency pack '{dep_name}' at {} must be a real directory, not a symlink",
            cur.display()
        )));
    }
    std::fs::canonicalize(&cur).map_err(|e| {
        Error::new(format!(
            "failed to resolve dependency pack '{dep_name}': {e}"
        ))
    })
}

/// Normalizes a pack name/slug into an ASCII identifier (`"open-pack"` -> `"open_pack"`).
#[must_use]
pub fn normalize_pack_ident(raw: &str) -> String {
    let mut out = String::new();
    let mut prev_underscore = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore && !out.is_empty() {
            out.push('_');
            prev_underscore = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

/// Loads and resolves one root `.antics` v2 source string and all transitive `use` imports.
pub fn resolve_v2_source(
    root_name: &str,
    source: &str,
    config: &PackResolverConfig,
) -> Result<ResolvedModule> {
    let file = parse_v2_file(source).map_err(|e| e.with_context(root_name))?;
    let mut loader = ModuleLoader {
        config,
        loaded: BTreeMap::new(),
        loading_stack: Vec::new(),
    };
    loader.resolve_parsed_module(root_name.to_owned(), &file, true)
}

/// Maximum transitive module import nesting depth.
const MAX_IMPORT_DEPTH: usize = 32;

/// Reads one `.antics` or `.air` file from disk after verifying symlink-free components
/// and (when `pack_dir` is `Some`) containment within the pack root.
pub fn read_v2_checked_file(
    file_path: &Path,
    pack_dir: Option<&Path>,
) -> Result<(String, PackResolverConfig)> {
    let config = match pack_dir {
        Some(dir) => PackResolverConfig::from_pack_dir(dir)?,
        None => PackResolverConfig::infer_for_file(file_path)?,
    };
    let abs_file = if file_path.is_absolute() {
        file_path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| Error::new(format!("failed to determine current directory: {e}")))?
            .join(file_path)
    };
    let meta = std::fs::symlink_metadata(file_path)
        .map_err(|e| Error::new(format!("failed to access {}: {e}", file_path.display())))?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(Error::new(format!(
            "expected regular file at {} (refusing symlinks and directories)",
            file_path.display()
        )));
    }
    let canon_file = std::fs::canonicalize(file_path)
        .map_err(|e| Error::new(format!("failed to resolve {}: {e}", file_path.display())))?;
    if let Some(root_canon) = &config.pack_root {
        if !canon_file.starts_with(root_canon) && pack_dir.is_some() {
            return Err(Error::new(format!(
                "source file {} is outside pack root {}",
                canon_file.display(),
                root_canon.display()
            )));
        }
        let base_to_check = match pack_dir {
            Some(dir) if dir.is_absolute() => dir.to_path_buf(),
            Some(dir) => std::env::current_dir()
                .map_err(|e| Error::new(format!("failed to determine current directory: {e}")))?
                .join(dir),
            None => root_canon.clone(),
        };
        if let Ok(rel) = abs_file.strip_prefix(&base_to_check) {
            verify_no_symlink_components(&base_to_check, rel)?;
        } else if let Ok(rel) = abs_file.strip_prefix(root_canon) {
            verify_no_symlink_components(root_canon, rel)?;
        } else if canon_file.starts_with(root_canon) {
            // Alternate lexical path (e.g. `/tmp/alias/sub/demo.antics` with `--pack /tmp/pack`):
            // walk the original path BEFORE canonicalization, rejecting any symlink component
            // along the path up to the common ancestor or canonical pack root.
            verify_no_symlink_ancestors_to_root(&abs_file, &base_to_check, root_canon)?;
        }
        // Also verify symlink-free components along the canonical path relative to root_canon.
        if let Ok(rel) = canon_file.strip_prefix(root_canon) {
            verify_no_symlink_components(root_canon, rel)?;
        }
    }
    let root_for_read = config
        .pack_root
        .as_deref()
        .filter(|r| canon_file.starts_with(r))
        .unwrap_or_else(|| canon_file.parent().unwrap_or(Path::new("/")));
    let source = read_regular_file_checked(&canon_file, root_for_read)?;
    Ok((source, config))
}

/// Loads and resolves one `.antics` v2 file from disk (symlink-free regular file check,
/// rejecting symlinked ancestor components inside or aliasing the pack root).
pub fn resolve_v2_file(file_path: &Path, pack_dir: Option<&Path>) -> Result<ResolvedModule> {
    let (source, config) = read_v2_checked_file(file_path, pack_dir)?;
    let label = file_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("<input.antics>");
    resolve_v2_source(label, &source, &config)
}

fn is_macos_system_symlink(path: &Path) -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    if path.components().count() != 2 {
        return false;
    }
    std::fs::canonicalize(path)
        .map(|canon| canon.starts_with("/private/"))
        .unwrap_or(false)
}

/// Walks an uncanonicalized source path to catch symlinked ancestor components even when
/// `abs_file` does not lexically start with `base_to_check` or `root_canon` (e.g.
/// `/tmp/alias/sub/demo.antics` where `/tmp/alias -> /tmp/pack`).
fn verify_no_symlink_ancestors_to_root(
    abs_file: &Path,
    base_to_check: &Path,
    root_canon: &Path,
) -> Result<()> {
    // 1. Check from the longest common lexical prefix with `base_to_check` down to `abs_file`
    //    (allows macOS's top-level `/var` / `/tmp` -> `/private/*` system symlinks while
    //    still catching any real symlink component like `/tmp/alias/...`).
    let mut common = PathBuf::new();
    for (a, b) in abs_file.components().zip(base_to_check.components()) {
        if a == b {
            common.push(a.as_os_str());
        } else {
            break;
        }
    }
    if let Ok(rel) = abs_file.strip_prefix(&common) {
        verify_no_symlink_components(&common, rel)?;
    }
    // 2. Also walk parent links upward while canonical path remains within `root_canon`
    //    (checking `symlink_metadata` at each step BEFORE testing canonical equality, so a
    //    symlink pointing directly to `root_canon` itself is rejected).
    let mut cur = abs_file;
    while std::fs::canonicalize(cur)
        .ok()
        .is_some_and(|c| c.starts_with(root_canon))
    {
        let meta = std::fs::symlink_metadata(cur)
            .map_err(|e| Error::new(format!("failed to inspect {}: {e}", cur.display())))?;
        if meta.file_type().is_symlink() && !is_macos_system_symlink(cur) {
            return Err(Error::new(format!(
                "refusing to follow symlink path component: {}",
                cur.display()
            )));
        }
        if std::fs::canonicalize(cur).ok().as_deref() == Some(root_canon) {
            break;
        }
        let Some(parent) = cur.parent() else {
            break;
        };
        if parent == cur {
            break;
        }
        cur = parent;
    }
    Ok(())
}

fn verify_no_symlink_components(base: &Path, rel: &Path) -> Result<()> {
    use std::path::Component;
    let mut cur = base.to_path_buf();
    for comp in rel.components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(seg) => {
                cur.push(seg);
                let meta = std::fs::symlink_metadata(&cur)
                    .map_err(|e| Error::new(format!("failed to inspect {}: {e}", cur.display())))?;
                if meta.file_type().is_symlink() && !is_macos_system_symlink(&cur) {
                    return Err(Error::new(format!(
                        "refusing to follow symlink path component: {}",
                        cur.display()
                    )));
                }
            }
            Component::ParentDir => {
                return Err(Error::new(format!(
                    "refusing '..' traversal component inside {}",
                    rel.display()
                )));
            }
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    Ok(())
}

struct LoadedModuleData {
    /// Only items declared `pub` in this module (plus object declarations exported by type name).
    pub_functions: BTreeMap<String, FnDecl>,
    pub_templates: BTreeMap<String, TemplateDecl>,
    pub_enums: BTreeMap<String, EnumDecl>,
    pub_bitflags: BTreeMap<String, BitflagsDecl>,
    pub_objects: BTreeMap<String, ObjectDecl>,
    pub_extern_mods: BTreeMap<String, ExternModDecl>,
    pub_extern_bcons: BTreeMap<String, ExternBconDecl>,
    pub_consts: BTreeMap<String, ConstDecl>,
    /// All items declared locally (for private-visibility error diagnostics).
    all_local_names: BTreeSet<String>,
    /// Complete scope visible inside this module.
    scope: ModuleScope,
}

impl LoadedModuleData {
    /// Every name a `use module::*;` imports, sorted.
    fn pub_names(&self) -> BTreeSet<String> {
        self.pub_functions
            .keys()
            .chain(self.pub_templates.keys())
            .chain(self.pub_enums.keys())
            .chain(self.pub_bitflags.keys())
            .chain(self.pub_objects.keys())
            .chain(self.pub_extern_mods.keys())
            .chain(self.pub_extern_bcons.keys())
            .chain(self.pub_consts.keys())
            .cloned()
            .collect()
    }
}

/// `#[id(N)] fn name(p: T, ..) -> R`, for merge diagnostics.
fn extern_fn_text(f: &ExternFnDecl) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| match p.slot {
            AbiParamSlot::Positional => format!("{}: {}", p.name, p.ty.display_name()),
            AbiParamSlot::Temp(n) => format!("#[temp({n})] {}: {}", p.name, p.ty.display_name()),
        })
        .collect();
    let ret = match &f.ret {
        ReturnAbi::Void => String::new(),
        ReturnAbi::Bool => " -> bool".to_owned(),
        ReturnAbi::Result => " -> Result".to_owned(),
        ReturnAbi::Value { ty, temp } => format!(" -> {} (temp {temp})", ty.display_name()),
        ReturnAbi::ResultValue { ty, temp } => {
            format!(" -> Result<{}> (temp {temp})", ty.display_name())
        }
    };
    format!("#[id({})] fn {}({}){ret}", f.id, f.name, params.join(", "))
}

/// Whether two extern fns call their tree the same way: parameter types and slots, and the
/// return ABI (parameter names may differ).
fn same_extern_abi(a: &ExternFnDecl, b: &ExternFnDecl) -> bool {
    a.ret == b.ret
        && a.params.len() == b.params.len()
        && a.params
            .iter()
            .zip(&b.params)
            .all(|(x, y)| x.ty == y.ty && x.slot == y.slot)
}

/// Whether `ty` names a type that resolves in the declaring scope: an enum, bitflags or object of
/// a module, or `Self` (built-in types such as `Height` are the same everywhere).
fn names_a_module_type(ty: &V2Type) -> bool {
    match ty {
        V2Type::Named(name) => name == "Self" || !crate::v2::ast::is_reserved_type_name(name),
        V2Type::MutRef(inner) => names_a_module_type(inner),
        _ => false,
    }
}

/// Whether `f` names a type of its module (an enum, an object), which resolves in the scope
/// that owns its declaration.
pub(crate) fn names_module_types(f: &ExternFnDecl) -> bool {
    f.params.iter().any(|p| names_a_module_type(&p.ty))
        || matches!(&f.ret, ReturnAbi::Value { ty, .. } | ReturnAbi::ResultValue { ty, .. } if names_a_module_type(ty))
}

/// Merges two `extern mod` blocks bound to the same name, e.g. a local `extern mod globals { .. }`
/// (owned by `a_owner`) and `use std::globals::*;` (owned by `b_owner`), into one block and the
/// module that owns it.
///
/// The merged block's types resolve in its owner, so only one side may name module types: that
/// side owns the merge (otherwise the first does), and the merge is refused when both sides of
/// two modules do. A fn both declare must be the same declaration (id, parameters, return), and
/// each fn must call its tree the way every fn of the other side for that tree does. The result
/// does not depend on which side comes first. Errors point at `at`.
pub(crate) fn merge_extern_mods(
    a_owner: &str,
    a: &ExternModDecl,
    b_owner: &str,
    b: &ExternModDecl,
    at: Span,
) -> Result<(String, ExternModDecl)> {
    let typed_a = a.fns.iter().find(|f| names_module_types(f));
    let typed_b = b.fns.iter().find(|f| names_module_types(f));
    let ((base_owner, base), (extra_owner, extra)) = match (typed_a, typed_b) {
        (Some(fa), Some(fb)) if a_owner != b_owner => {
            return Err(Error::at(
                at,
                format!(
                    "extern mod '{}' cannot merge {a_owner}'s and {b_owner}'s declarations: both name types of their own module (`{}` and `{}`)",
                    a.name,
                    extern_fn_text(fa),
                    extern_fn_text(fb)
                ),
            ));
        }
        (None, Some(_)) => ((b_owner, b), (a_owner, a)),
        _ => ((a_owner, a), (b_owner, b)),
    };
    let mut merged = base.clone();
    for f in &extra.fns {
        // Every fn of the other side for the same tree, an exact duplicate's aliases too.
        if let Some(b) = base
            .fns
            .iter()
            .find(|b| b.id == f.id && !same_extern_abi(b, f))
        {
            return Err(Error::at(
                at,
                format!(
                    "extern mod '{}' calls tree {} two ways: `{}` in {base_owner} and `{}` in {extra_owner}",
                    base.name,
                    f.id,
                    extern_fn_text(b),
                    extern_fn_text(f)
                ),
            ));
        }
        if let Some(b) = base.fns.iter().find(|b| b.name == f.name) {
            let same = b.id == f.id
                && same_extern_abi(b, f)
                && b.params
                    .iter()
                    .zip(&f.params)
                    .all(|(x, y)| x.name == y.name);
            if same {
                continue;
            }
            return Err(Error::at(
                at,
                format!(
                    "extern mod '{}' declares fn '{}' twice, differently: `{}` in {base_owner} and `{}` in {extra_owner}",
                    base.name,
                    f.name,
                    extern_fn_text(b),
                    extern_fn_text(f)
                ),
            ));
        }
        merged.fns.push(f.clone());
    }
    Ok((base_owner.to_owned(), merged))
}

/// The built-in module that declares the prelude types (`Height`, `Placement`).
pub(crate) const STD_PRELUDE_MODULE: &str = "std::flags";

/// The source of a built-in `std::<module>` (`use std::globals::*;`), if there is one.
pub(crate) fn std_module_source(segs: &[String]) -> Option<&'static str> {
    match segs {
        [m] if m == "globals" => Some(include_str!("std/globals.antics")),
        [m] if m == "flags" => Some(include_str!("std/flags.antics")),
        [m] if m == "engine" => Some(include_str!("std/engine.antics")),
        _ => None,
    }
}

struct ModuleLoader<'a> {
    config: &'a PackResolverConfig,
    loaded: BTreeMap<String, LoadedModuleData>,
    loading_stack: Vec<String>,
}

impl<'a> ModuleLoader<'a> {
    fn resolve_parsed_module(
        &mut self,
        module_id: String,
        file: &V2File,
        is_root: bool,
    ) -> Result<ResolvedModule> {
        self.loading_stack.push(module_id.clone());

        let mut functions: BTreeMap<String, (String, FnDecl)> = BTreeMap::new();
        let mut templates: BTreeMap<String, (String, TemplateDecl)> = BTreeMap::new();
        let mut enums: BTreeMap<String, (String, EnumDecl)> = BTreeMap::new();
        let mut bitflags: BTreeMap<String, (String, BitflagsDecl)> = BTreeMap::new();
        let mut imported_objects: BTreeMap<String, (String, ObjectDecl)> = BTreeMap::new();
        let mut objects = Vec::new();
        let mut pub_functions = BTreeMap::new();
        let mut pub_templates = BTreeMap::new();
        let mut pub_enums = BTreeMap::new();
        let mut pub_bitflags = BTreeMap::new();
        let mut pub_objects = BTreeMap::new();
        let mut extern_mods: BTreeMap<String, (String, ExternModDecl)> = BTreeMap::new();
        let mut extern_bcons: BTreeMap<String, (String, ExternBconDecl)> = BTreeMap::new();
        let mut pub_extern_mods = BTreeMap::new();
        let mut pub_extern_bcons = BTreeMap::new();
        let mut consts: BTreeMap<String, (String, ConstDecl)> = BTreeMap::new();
        let mut pub_consts = BTreeMap::new();
        let mut all_local_names = BTreeSet::new();

        // 1. Register local items first so duplicate local names are caught clearly.
        for item in &file.items {
            match item {
                V2Item::Const(c) => {
                    if !all_local_names.insert(c.name.clone()) {
                        return Err(Error::at(
                            c.span,
                            format!("duplicate top-level item '{}' in {module_id}", c.name),
                        ));
                    }
                    if c.is_pub {
                        pub_consts.insert(c.name.clone(), c.clone());
                    }
                    consts.insert(c.name.clone(), (module_id.clone(), c.clone()));
                }
                V2Item::Fn(f) => {
                    if !all_local_names.insert(f.name.clone()) {
                        return Err(Error::at(
                            f.span,
                            format!("duplicate top-level item '{}' in {module_id}", f.name),
                        ));
                    }
                    if f.is_pub {
                        pub_functions.insert(f.name.clone(), f.clone());
                    }
                    functions.insert(f.name.clone(), (module_id.clone(), f.clone()));
                }
                V2Item::Template(t) => {
                    if !all_local_names.insert(t.name.clone()) {
                        return Err(Error::at(
                            t.span,
                            format!("duplicate top-level item '{}' in {module_id}", t.name),
                        ));
                    }
                    if t.is_pub {
                        pub_templates.insert(t.name.clone(), t.clone());
                    }
                    templates.insert(t.name.clone(), (module_id.clone(), t.clone()));
                }
                V2Item::Enum(e) => {
                    if !all_local_names.insert(e.name.clone()) {
                        return Err(Error::at(
                            e.span,
                            format!("duplicate top-level item '{}' in {module_id}", e.name),
                        ));
                    }
                    if e.is_pub {
                        pub_enums.insert(e.name.clone(), e.clone());
                    }
                    enums.insert(e.name.clone(), (module_id.clone(), e.clone()));
                }
                V2Item::Bitflags(b) => {
                    if !all_local_names.insert(b.name.clone()) {
                        return Err(Error::at(
                            b.span,
                            format!("duplicate top-level item '{}' in {module_id}", b.name),
                        ));
                    }
                    if b.is_pub {
                        pub_bitflags.insert(b.name.clone(), b.clone());
                    }
                    bitflags.insert(b.name.clone(), (module_id.clone(), b.clone()));
                }
                V2Item::ExternMod(m) => {
                    if !all_local_names.insert(m.name.clone()) {
                        return Err(Error::at(
                            m.span,
                            format!("duplicate top-level item '{}' in {module_id}", m.name),
                        ));
                    }
                    if m.is_pub {
                        pub_extern_mods.insert(m.name.clone(), m.clone());
                    }
                    extern_mods.insert(m.name.clone(), (module_id.clone(), m.clone()));
                }
                V2Item::ExternBcon(b) => {
                    if !all_local_names.insert(b.name.clone()) {
                        return Err(Error::at(
                            b.span,
                            format!("duplicate top-level item '{}' in {module_id}", b.name),
                        ));
                    }
                    if b.is_pub {
                        pub_extern_bcons.insert(b.name.clone(), b.clone());
                    }
                    extern_bcons.insert(b.name.clone(), (module_id.clone(), b.clone()));
                }
                V2Item::Object(obj) => {
                    let type_name = object_type_ident(&obj.name);
                    if crate::v2::ast::is_reserved_type_name(&type_name) {
                        return Err(Error::at(
                            obj.span,
                            format!(
                                "object '{}' derives reserved built-in type name '{type_name}'",
                                obj.name
                            ),
                        ));
                    }
                    if !all_local_names.insert(type_name.clone()) {
                        return Err(Error::at(
                            obj.span,
                            format!(
                                "duplicate top-level item/object type '{type_name}' in {module_id}"
                            ),
                        ));
                    }
                    pub_objects.insert(type_name.clone(), obj.clone());
                    imported_objects.insert(type_name, (module_id.clone(), obj.clone()));
                    objects.push(obj.clone());
                }
            }
        }

        // 2. Resolve each `use` declaration.
        for use_decl in &file.uses {
            let target_key = self.load_module_path(&use_decl.module_path, use_decl.span)?;
            let target = self
                .loaded
                .get(&target_key)
                .expect("load_module_path inserts into self.loaded");
            let glob_items: Vec<crate::v2::ast::UseItem>;
            let use_items = match use_decl.items.as_slice() {
                [only] if only.name == "*" => {
                    glob_items = target
                        .pub_names()
                        .into_iter()
                        // `std::flags` declares the prelude types (`Height`, `Placement`),
                        // which every module already sees under their built-in names.
                        .filter(|n| !crate::v2::ast::is_reserved_type_name(n))
                        // A name already bound here (a local item or an earlier import)
                        // shadows the glob's.
                        .filter(|n| {
                            // Two `extern mod` blocks of one name merge instead.
                            (extern_mods.contains_key(n) && target.pub_extern_mods.contains_key(n))
                                || !(functions.contains_key(n)
                                    || templates.contains_key(n)
                                    || enums.contains_key(n)
                                    || bitflags.contains_key(n)
                                    || imported_objects.contains_key(n)
                                    || extern_mods.contains_key(n)
                                    || extern_bcons.contains_key(n)
                                    || consts.contains_key(n))
                        })
                        .map(|name| crate::v2::ast::UseItem {
                            span: only.span,
                            name,
                            alias: None,
                        })
                        .collect();
                    glob_items.as_slice()
                }
                items => items,
            };
            for item in use_items {
                let local_name = item.alias.as_ref().unwrap_or(&item.name);
                // `use std::flags::Height;` names the prelude type itself: nothing to bind.
                if target_key == STD_PRELUDE_MODULE
                    && item.alias.is_none()
                    && crate::v2::ast::is_std_prelude_type_name(local_name)
                {
                    continue;
                }
                if crate::v2::ast::is_reserved_type_name(local_name) {
                    return Err(Error::at(
                        item.span,
                        format!("cannot bind import to reserved built-in type name '{local_name}'"),
                    ));
                }
                if local_name == "world" {
                    return Err(Error::at(
                        item.span,
                        "'world' names the engine globals ('world.hour') and cannot name an import",
                    ));
                }
                let name_taken = functions.contains_key(local_name)
                    || templates.contains_key(local_name)
                    || enums.contains_key(local_name)
                    || bitflags.contains_key(local_name)
                    || imported_objects.contains_key(local_name)
                    || extern_mods.contains_key(local_name)
                    || extern_bcons.contains_key(local_name)
                    || consts.contains_key(local_name);
                if let Some(fn_decl) = target.pub_functions.get(&item.name) {
                    if name_taken {
                        return Err(Error::at(
                            item.span,
                            format!(
                                "imported name '{local_name}' collides with an existing binding"
                            ),
                        ));
                    }
                    functions.insert(local_name.clone(), (target_key.clone(), fn_decl.clone()));
                } else if let Some(tpl_decl) = target.pub_templates.get(&item.name) {
                    if name_taken {
                        return Err(Error::at(
                            item.span,
                            format!(
                                "imported name '{local_name}' collides with an existing binding"
                            ),
                        ));
                    }
                    templates.insert(local_name.clone(), (target_key.clone(), tpl_decl.clone()));
                } else if let Some(enum_decl) = target.pub_enums.get(&item.name) {
                    if name_taken {
                        return Err(Error::at(
                            item.span,
                            format!(
                                "imported name '{local_name}' collides with an existing binding"
                            ),
                        ));
                    }
                    enums.insert(local_name.clone(), (target_key.clone(), enum_decl.clone()));
                } else if let Some(bf_decl) = target.pub_bitflags.get(&item.name) {
                    if name_taken {
                        return Err(Error::at(
                            item.span,
                            format!(
                                "imported name '{local_name}' collides with an existing binding"
                            ),
                        ));
                    }
                    bitflags.insert(local_name.clone(), (target_key.clone(), bf_decl.clone()));
                } else if let Some(obj_decl) = target.pub_objects.get(&item.name) {
                    if name_taken {
                        return Err(Error::at(
                            item.span,
                            format!(
                                "imported name '{local_name}' collides with an existing binding"
                            ),
                        ));
                    }
                    imported_objects
                        .insert(local_name.clone(), (target_key.clone(), obj_decl.clone()));
                } else if let Some(m_decl) = target.pub_extern_mods.get(&item.name) {
                    if let Some((base_owner, base)) = extern_mods.get(local_name) {
                        let merged =
                            merge_extern_mods(base_owner, base, &target_key, m_decl, item.span)?;
                        extern_mods.insert(local_name.clone(), merged);
                        continue;
                    }
                    if name_taken {
                        return Err(Error::at(
                            item.span,
                            format!(
                                "imported name '{local_name}' collides with an existing binding"
                            ),
                        ));
                    }
                    extern_mods.insert(local_name.clone(), (target_key.clone(), m_decl.clone()));
                } else if let Some(b_decl) = target.pub_extern_bcons.get(&item.name) {
                    if name_taken {
                        return Err(Error::at(
                            item.span,
                            format!(
                                "imported name '{local_name}' collides with an existing binding"
                            ),
                        ));
                    }
                    extern_bcons.insert(local_name.clone(), (target_key.clone(), b_decl.clone()));
                } else if let Some(c_decl) = target.pub_consts.get(&item.name) {
                    if name_taken {
                        return Err(Error::at(
                            item.span,
                            format!(
                                "imported name '{local_name}' collides with an existing binding"
                            ),
                        ));
                    }
                    consts.insert(local_name.clone(), (target_key.clone(), c_decl.clone()));
                } else if target.all_local_names.contains(&item.name) {
                    return Err(Error::at(
                        item.span,
                        format!(
                            "item '{}' in module '{}' is private (declare it 'pub' to import)",
                            item.name, target_key
                        ),
                    ));
                } else {
                    return Err(Error::at(
                        item.span,
                        format!("module '{}' has no item named '{}'", target_key, item.name),
                    ));
                }
            }
        }

        self.loading_stack.pop();

        let scope = ModuleScope {
            functions: functions.clone(),
            templates: templates.clone(),
            enums: enums.clone(),
            bitflags: bitflags.clone(),
            objects: imported_objects.clone(),
            extern_mods,
            extern_bcons,
            consts,
        };

        if !is_root {
            self.loaded.insert(
                module_id.clone(),
                LoadedModuleData {
                    pub_functions,
                    pub_templates,
                    pub_enums,
                    pub_bitflags,
                    pub_objects,
                    pub_extern_mods,
                    pub_extern_bcons,
                    pub_consts,
                    all_local_names,
                    scope: scope.clone(),
                },
            );
        }

        let mut module_scopes: BTreeMap<String, ModuleScope> = self
            .loaded
            .iter()
            .map(|(k, v)| (k.clone(), v.scope.clone()))
            .collect();
        module_scopes.insert(module_id.clone(), scope);

        Ok(ResolvedModule {
            module_id,
            functions,
            templates,
            enums,
            bitflags,
            imported_objects,
            objects,
            module_scopes,
        })
    }

    fn load_module_path(&mut self, path: &[String], span: Span) -> Result<String> {
        for seg in path {
            validate_ident_segment(seg, span)?;
        }
        let pack_seg = &path[0];
        let mod_segs = &path[1..];
        let canonical_key = path.join("::");

        if self.loading_stack.len() >= MAX_IMPORT_DEPTH {
            return Err(Error::at(
                span,
                format!("module import depth exceeds maximum of {MAX_IMPORT_DEPTH}"),
            ));
        }

        if let Some(pos) = self.loading_stack.iter().position(|m| m == &canonical_key) {
            let mut cycle: Vec<&str> = self.loading_stack[pos..]
                .iter()
                .map(String::as_str)
                .collect();
            cycle.push(&canonical_key);
            return Err(Error::at(
                span,
                format!("cyclic module import detected: {}", cycle.join(" -> ")),
            ));
        }

        if self.loaded.contains_key(&canonical_key) {
            return Ok(canonical_key);
        }

        // 1. Check virtual in-memory modules first (for unit tests and virtual packs).
        if let Some(virtual_src) = self.config.virtual_modules.get(&canonical_key) {
            let parsed = parse_v2_file(virtual_src).map_err(|e| e.with_context(&canonical_key))?;
            self.resolve_parsed_module(canonical_key.clone(), &parsed, false)?;
            return Ok(canonical_key);
        }

        // 2. The built-in `std` modules.
        if pack_seg == "std" {
            let Some(src) = std_module_source(mod_segs) else {
                return Err(Error::at(
                    span,
                    format!(
                        "unknown built-in module '{canonical_key}' (expected 'std::globals', 'std::flags' or 'std::engine')"
                    ),
                ));
            };
            let parsed = parse_v2_std_file(src).map_err(|e| e.with_context(&canonical_key))?;
            self.resolve_parsed_module(canonical_key.clone(), &parsed, false)?;
            return Ok(canonical_key);
        }

        // 3. Resolve pack root on disk.
        let root_canon = if self.config.pack_aliases.contains(pack_seg) {
            self.config.pack_root.as_ref().ok_or_else(|| {
                Error::at(
                    span,
                    format!(
                        "cannot resolve import '{canonical_key}' without a pack root directory"
                    ),
                )
            })?
        } else if let Some(dep_root) = self.config.dependencies.get(pack_seg) {
            dep_root
        } else {
            return Err(Error::at(
                span,
                format!(
                    "unknown pack '{pack_seg}' in 'use {canonical_key}' (not the active pack or a declared dependency)"
                ),
            ));
        };

        let file_path = resolve_module_file_in_root(root_canon, mod_segs, span)?;
        let src = read_regular_file_checked(&file_path, root_canon)?;
        let parsed = parse_v2_file(&src).map_err(|e| e.with_context(file_path.display()))?;
        self.resolve_parsed_module(canonical_key.clone(), &parsed, false)?;
        Ok(canonical_key)
    }
}

fn validate_ident_segment(seg: &str, span: Span) -> Result<()> {
    if seg.is_empty() || !seg.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(Error::at(
            span,
            format!("invalid module path segment '{seg}'"),
        ));
    }
    Ok(())
}

fn resolve_module_file_in_root(
    root_canon: &Path,
    mod_segs: &[String],
    span: Span,
) -> Result<PathBuf> {
    if mod_segs.is_empty() {
        return Err(Error::at(span, "module path must specify a module name"));
    }
    let mut candidates = Vec::new();
    let mut push_candidates = |segs: &[String]| {
        let last = segs.last().expect("non-empty");
        let prefix = &segs[..segs.len() - 1];

        let mut direct_file: Vec<String> = prefix.to_vec();
        direct_file.push(format!("{last}.antics"));
        candidates.push(direct_file);

        let mut direct_mod: Vec<String> = segs.to_vec();
        direct_mod.push("mod.antics".to_owned());
        candidates.push(direct_mod);

        let mut direct_obj: Vec<String> = segs.to_vec();
        direct_obj.push("object.antics".to_owned());
        candidates.push(direct_obj);

        let mut src_file: Vec<String> = vec!["src".to_owned()];
        src_file.extend(prefix.iter().cloned());
        src_file.push(format!("{last}.antics"));
        candidates.push(src_file);

        let mut src_mod: Vec<String> = vec!["src".to_owned()];
        src_mod.extend(segs.iter().cloned());
        src_mod.push("mod.antics".to_owned());
        candidates.push(src_mod);
    };

    push_candidates(mod_segs);
    let kebab_segs: Vec<String> = mod_segs.iter().map(|s| s.replace('_', "-")).collect();
    if kebab_segs != mod_segs {
        push_candidates(&kebab_segs);
    }

    let mut found = BTreeSet::new();
    for candidate in &candidates {
        if let Some(path) = try_contained_regular_file(root_canon, candidate, span)? {
            found.insert(path);
        }
    }
    match found.len() {
        0 => Err(Error::at(
            span,
            format!(
                "could not find module '{}' under pack root {}",
                mod_segs.join("::"),
                root_canon.display()
            ),
        )),
        1 => Ok(found.into_iter().next().expect("len == 1")),
        _ => Err(Error::at(
            span,
            format!(
                "ambiguous module '{}' matched multiple files under {}",
                mod_segs.join("::"),
                root_canon.display()
            ),
        )),
    }
}

fn try_contained_regular_file(
    root_canon: &Path,
    rel_parts: &[String],
    span: Span,
) -> Result<Option<PathBuf>> {
    let mut cur = root_canon.to_path_buf();
    for (idx, part) in rel_parts.iter().enumerate() {
        cur.push(part);
        match std::fs::symlink_metadata(&cur) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(Error::at(
                        span,
                        format!(
                            "refusing to follow symlink while resolving module: {}",
                            cur.display()
                        ),
                    ));
                }
                let is_last = idx + 1 == rel_parts.len();
                if is_last && !meta.file_type().is_file() {
                    return Ok(None);
                }
                if !is_last && !meta.file_type().is_dir() {
                    return Ok(None);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(Error::at(
                    span,
                    format!("failed to inspect {}: {e}", cur.display()),
                ));
            }
        }
    }
    let canon = std::fs::canonicalize(&cur).map_err(|e| {
        Error::at(
            span,
            format!("failed to canonicalize {}: {e}", cur.display()),
        )
    })?;
    if !canon.starts_with(root_canon) {
        return Err(Error::at(
            span,
            format!(
                "resolved module path {} escapes pack root {}",
                canon.display(),
                root_canon.display()
            ),
        ));
    }
    Ok(Some(canon))
}

pub(crate) fn read_regular_file_checked(path: &Path, root_canon: &Path) -> Result<String> {
    use std::io::Read;

    let pre_meta = std::fs::symlink_metadata(path)
        .map_err(|e| Error::new(format!("failed to stat {}: {e}", path.display())))?;
    if pre_meta.file_type().is_symlink() || !pre_meta.file_type().is_file() {
        return Err(Error::new(format!(
            "expected regular file at {}",
            path.display()
        )));
    }
    let canon = std::fs::canonicalize(path)
        .map_err(|e| Error::new(format!("failed to resolve {}: {e}", path.display())))?;
    if !canon.starts_with(root_canon) {
        return Err(Error::new(format!(
            "file {} escapes root {}",
            canon.display(),
            root_canon.display()
        )));
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        const O_NONBLOCK: i32 = 0x0004;
        opts.custom_flags(O_NONBLOCK);
    }
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        any(
            target_arch = "x86",
            target_arch = "x86_64",
            target_arch = "arm",
            target_arch = "aarch64",
            target_arch = "riscv64"
        )
    ))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        const O_NONBLOCK: i32 = 0x0800;
        opts.custom_flags(O_NONBLOCK);
    }
    let mut file = opts
        .open(&canon)
        .map_err(|e| Error::new(format!("failed to open {}: {e}", canon.display())))?;
    let open_meta = file
        .metadata()
        .map_err(|e| Error::new(format!("failed to stat open {}: {e}", canon.display())))?;
    if !open_meta.file_type().is_file() {
        return Err(Error::new(format!(
            "expected regular file at {}",
            canon.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if open_meta.dev() != pre_meta.dev() || open_meta.ino() != pre_meta.ino() {
            return Err(Error::new(format!(
                "file {} changed between validation and open",
                canon.display()
            )));
        }
    }
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| Error::new(format!("failed to read {}: {e}", canon.display())))?;
    Ok(text)
}
