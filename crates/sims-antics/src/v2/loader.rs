//! Deterministic module loader and `use` import resolver for `.antics` v2.
//!
//! Enforces symlink-free containment within pack roots, `pub fn`/`pub template`
//! visibility across module boundaries, and cycle detection.

use crate::error::{Error, Result, Span};
use crate::manifest::{TomlValue, parse_toml};
use crate::v2::ast::{FnDecl, ObjectDecl, TemplateDecl, V2File, V2Item};
use crate::v2::parser::parse_v2_file;
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
    /// Concrete object declarations defined in this module.
    pub objects: Vec<ObjectDecl>,
    /// Every transitively loaded module's local function scope (keyed by module id) so
    /// functions/templates imported from module M resolve their own helper calls inside M's scope.
    pub module_scopes: BTreeMap<String, ModuleScope>,
}

/// Exported and visible symbols inside a single loaded `.antics` v2 module.
#[derive(Debug, Clone, Default)]
pub struct ModuleScope {
    pub functions: BTreeMap<String, (String, FnDecl)>,
    pub templates: BTreeMap<String, (String, TemplateDecl)>,
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
    /// Only items declared `pub` in this module.
    pub_functions: BTreeMap<String, FnDecl>,
    pub_templates: BTreeMap<String, TemplateDecl>,
    /// All items declared locally (for private-visibility error diagnostics).
    all_local_names: BTreeSet<String>,
    /// Complete scope visible inside this module.
    scope: ModuleScope,
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
        let mut objects = Vec::new();
        let mut pub_functions = BTreeMap::new();
        let mut pub_templates = BTreeMap::new();
        let mut all_local_names = BTreeSet::new();

        // 1. Register local items first so duplicate local names are caught clearly.
        for item in &file.items {
            match item {
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
                V2Item::Object(obj) => {
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
            for item in &use_decl.items {
                let local_name = item.alias.as_ref().unwrap_or(&item.name);
                if let Some(fn_decl) = target.pub_functions.get(&item.name) {
                    if functions.contains_key(local_name) || templates.contains_key(local_name) {
                        return Err(Error::at(
                            item.span,
                            format!(
                                "imported name '{local_name}' collides with an existing binding"
                            ),
                        ));
                    }
                    functions.insert(local_name.clone(), (target_key.clone(), fn_decl.clone()));
                } else if let Some(tpl_decl) = target.pub_templates.get(&item.name) {
                    if functions.contains_key(local_name) || templates.contains_key(local_name) {
                        return Err(Error::at(
                            item.span,
                            format!(
                                "imported name '{local_name}' collides with an existing binding"
                            ),
                        ));
                    }
                    templates.insert(local_name.clone(), (target_key.clone(), tpl_decl.clone()));
                } else if target.all_local_names.contains(&item.name) {
                    return Err(Error::at(
                        item.span,
                        format!(
                            "item '{}' in module '{}' is private (declare it 'pub fn' or 'pub template' to import)",
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
        };

        if !is_root {
            self.loaded.insert(
                module_id.clone(),
                LoadedModuleData {
                    pub_functions,
                    pub_templates,
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

        // 2. Resolve pack root on disk.
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
    let last = mod_segs.last().expect("non-empty");
    let prefix = &mod_segs[..mod_segs.len() - 1];

    // Candidate 1: `<root>/<prefix...>/<last>.antics`
    // Candidate 2: `<root>/<prefix...>/<last>/mod.antics`
    // Candidate 3: `<root>/src/<prefix...>/<last>.antics`
    // Candidate 4: `<root>/src/<prefix...>/<last>/mod.antics`
    let mut direct_file: Vec<String> = prefix.to_vec();
    direct_file.push(format!("{last}.antics"));

    let mut direct_mod: Vec<String> = mod_segs.to_vec();
    direct_mod.push("mod.antics".to_owned());

    let mut src_file: Vec<String> = vec!["src".to_owned()];
    src_file.extend(prefix.iter().cloned());
    src_file.push(format!("{last}.antics"));

    let mut src_mod: Vec<String> = vec!["src".to_owned()];
    src_mod.extend(mod_segs.iter().cloned());
    src_mod.push("mod.antics".to_owned());

    let mut found = Vec::new();
    for candidate in [direct_file, direct_mod, src_file, src_mod] {
        if let Some(path) = try_contained_regular_file(root_canon, &candidate, span)? {
            found.push(path);
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
        1 => Ok(found.remove(0)),
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
