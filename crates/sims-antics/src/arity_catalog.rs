//! Callee-arity resolution for lifting and equivalence checks.
//!
//! An inline subroutine call (opcode `>= 256` with operands other than `0xFF * 8`) reads
//! as many argument words as its callee declares; a callee with more than 4 arguments
//! reads them from the following instruction records. Structured lifting and
//! `check_bhav_equivalent_with_arities` therefore need every callee's arity. Which file
//! holds the callee depends on its ID and on the *code owner* the VM runs the caller
//! under (`sims_vm::interpreter::behavior_from_owner`):
//!
//! - `256..=4095` (global): `Global.iff`, for every owner.
//! - `4096..=8191` (private): the owner object's own IFF.
//! - `8192..` (semi-global): the IFF the owner's `GLOB` chunk links to.
//!
//! The same table, applied to the *calling* tree's own ID, says which file a running
//! tree came from and therefore which owner it runs under ([`CallerScope`]):
//!
//! - A `256..=4095` tree runs only from `Global.iff`, under the calling object: its
//!   private and semi-global callees are owner-dependent.
//! - A `4096..=8191` tree runs only as its own file's private tree, so that file is the
//!   code owner: private callees are its own, semi-global callees come from *its* `GLOB`
//!   link (none: the VM has no semi-global library and traps).
//! - An `8192+` tree runs only as the semi-global library of an object linking to its
//!   file: semi-global callees are that file's own, private callees are the caller's.
//!
//! A tree outside those cases never runs, and its owner-dependent ranges stay unknown.
//! Unknown arities make the lifter keep the labeled-CFG layout, which is always sound.
//!
//! The rule: **a count is known only when the VM, booted the same way, would dispatch
//! this exact call to this exact tree.** `InstallArityCatalog` (feature
//! `install-catalog`, on by default) asks the VM's own code rather than restating it:
//!
//! - **Boot path.** It builds the `sims_content` index and catalog as the VM boots. A
//!   `GLOB` target in a mount that a neighborhood's `Characters` directory precedes
//!   (`Index::characters_mount`, e.g. `Downloads/` and the expansion mounts) could be
//!   shadowed by a `Characters` file of the same name; `lift` does not know the
//!   neighborhood, so that range is unknown.
//! - **Registered owners.** A file's private trees run only if it holds the registered
//!   (GUID-winning) `OBJD` of some object, and a file is a semi-global library only if
//!   such an owner's `GLOB` names it. A `GLOB` in a file with no registered `OBJD`
//!   counts for nothing.
//! - **VM loader.** Every library that supplies counts (owner, semi-global, global) must
//!   be a regular file the VM's library loader (`sims_vm::load_library`) accepts. If
//!   `Global.iff` is rejected the VM does not link at all, so every range is unknown.
//! - **Runnable callee.** The VM's own checks, shared in `sims_formats::behavior`
//!   (`TreeScope`, `instruction_refused`, `Behavior::frame_dimensions`), drop a callee
//!   the VM would not dispatch: an empty tree, an unsupported format, or a flagged
//!   call opcode (`0x8000` set).
//!
//! The `structured_equiv` corpus test and `sims-antics lift` both use it, so the CLI and
//! the corpus see the same arities.
//!
//! Roles are inferred only for a positively identified file: one under the install root
//! that matches exactly one index entry by file identity ([`ResourceMatch::Unique`]).
//! Anything else gets [`FileArities::no_role`], with every range unknown.

#[cfg(feature = "install-catalog")]
use sims_content::{ArchivePool, Catalog, Index, Install, Resource};
use sims_formats::behavior::{Behavior, TreeScope, instruction_refused};
use sims_formats::iff::Iff;
use std::collections::BTreeMap;
#[cfg(feature = "install-catalog")]
use std::collections::BTreeSet;
use std::ops::RangeInclusive;
use std::path::Path;
#[cfg(any(not(unix), feature = "install-catalog"))]
use std::path::PathBuf;

/// Global tree IDs, resolved from `Global.iff`.
pub const GLOBAL_TREES: RangeInclusive<u16> = 256..=4095;
/// Private tree IDs, resolved from the owner's own IFF.
pub const PRIVATE_TREES: RangeInclusive<u16> = 4096..=8191;
/// Semi-global tree IDs, resolved from the owner's `GLOB`-linked IFF.
pub const SEMI_GLOBAL_TREES: RangeInclusive<u16> = 8192..=u16::MAX;

/// `tree ID -> argument count` for the BHAVs in `iff` whose ID is in `range` and that
/// a call could reach.
///
/// The count is what the VM's call reads ([`Behavior::frame_dimensions`]); an ID the VM
/// refuses as an instruction opcode ([`instruction_refused`]) is never called. Every
/// BHAV chunk of an ID counts, deleted or not, in any order: the VM's library loader
/// keeps the last physical chunk (deleted ones included) while the original's loader
/// skips deleted chunks. An ID with a chunk that does not parse or cannot be called, or
/// whose chunks disagree, is left out (unknown), so the result is sound under either
/// selection rule.
#[must_use]
pub fn bhav_arities_in_range(iff: &Iff, range: RangeInclusive<u16>) -> BTreeMap<u16, usize> {
    let mut seen: BTreeMap<u16, Option<usize>> = BTreeMap::new();
    for chunk in &iff.chunks {
        if &chunk.tag != b"BHAV" || !range.contains(&chunk.id) {
            continue;
        }
        let arity = Behavior::parse(iff.data(chunk))
            .ok()
            .and_then(|bhav| bhav.frame_dimensions().ok())
            .map(|(args, _)| args)
            .filter(|_| !instruction_refused(chunk.id));
        seen.entry(chunk.id)
            .and_modify(|known| {
                if *known != arity {
                    *known = None;
                }
            })
            .or_insert(arity);
    }
    seen.into_iter()
        .filter_map(|(id, arity)| arity.map(|a| (id, a)))
        .collect()
}

/// Where one ID range's arities came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AritySource {
    /// The lifted file itself.
    ThisFile,
    /// The lifted file itself, for the reason given.
    ThisFileBecause(String),
    /// A file found through the install catalog (path relative to the install root).
    Catalog(String),
    /// A file named on the command line.
    Explicit(String),
    /// Not loaded; the string says why. Inline calls into this range have unknown arity.
    Missing(String),
}

/// One ID range's arities and their source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArityPart {
    pub source: AritySource,
    pub count: usize,
}

impl ArityPart {
    fn missing(why: &str) -> Self {
        Self {
            source: AritySource::Missing(why.to_owned()),
            count: 0,
        }
    }

    fn describe(&self, range: &str) -> String {
        match &self.source {
            AritySource::ThisFile => format!("{range} {} from this file", self.count),
            AritySource::ThisFileBecause(why) => {
                format!("{range} {} from this file ({why})", self.count)
            }
            AritySource::Catalog(location) => {
                format!("{range} {} from {location} (install catalog)", self.count)
            }
            AritySource::Explicit(location) => {
                format!("{range} {} from {location} (explicit)", self.count)
            }
            AritySource::Missing(why) => format!("{range} none ({why})"),
        }
    }
}

/// Merged callee arities for one IFF and where each range came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedArities {
    pub arities: BTreeMap<u16, usize>,
    pub private: ArityPart,
    pub global: ArityPart,
    pub semi_global: ArityPart,
    /// A suggestion appended to [`Self::describe`], e.g. the flags that would supply
    /// the unknown ranges.
    pub hint: Option<String>,
}

/// What the header suggests when no code-owner role could be inferred.
pub const NO_ROLE_HINT: &str = "pass --no-catalog to use this file's own private trees, \
     --globals <Global.iff> or --semiglobal <file.iff> for the others";

impl ResolvedArities {
    #[cfg(feature = "install-catalog")]
    fn unset() -> Self {
        Self {
            arities: BTreeMap::new(),
            private: ArityPart::missing("unset"),
            global: ArityPart::missing("unset"),
            semi_global: ArityPart::missing("unset"),
            hint: None,
        }
    }

    /// No code-owner role could be inferred for the file (`why`): every range is unknown,
    /// because even its private calls may belong to a calling object (a semi-global's
    /// `4096+` calls do).
    #[must_use]
    pub fn no_role(why: &str) -> Self {
        Self::unknown(&format!("no inferred role: {why}"))
    }

    /// Every range unknown for the same reason, with the flag hint.
    #[must_use]
    pub fn unknown(why: &str) -> Self {
        Self {
            arities: BTreeMap::new(),
            private: ArityPart::missing(why),
            global: ArityPart::missing(why),
            semi_global: ArityPart::missing(why),
            hint: Some(NO_ROLE_HINT.to_owned()),
        }
    }

    /// Private arities from `iff` only; global and semi-global ranges stay unknown so
    /// stray chunks in an object IFF cannot mask the real external arities.
    #[must_use]
    pub fn private_only(iff: &Iff, why_missing: &str) -> Self {
        let arities = bhav_arities_in_range(iff, PRIVATE_TREES);
        Self {
            private: ArityPart {
                source: AritySource::ThisFile,
                count: arities.len(),
            },
            global: ArityPart::missing(why_missing),
            semi_global: ArityPart::missing(why_missing),
            arities,
            hint: None,
        }
    }

    /// Replaces the global range with `arities` (filtered to [`GLOBAL_TREES`]).
    pub fn set_global(&mut self, arities: &BTreeMap<u16, usize>, source: AritySource) {
        self.global = self.replace_range(GLOBAL_TREES, arities, source);
    }

    /// Replaces the private range with `arities` (filtered to [`PRIVATE_TREES`]).
    pub fn set_private(&mut self, arities: &BTreeMap<u16, usize>, source: AritySource) {
        self.private = self.replace_range(PRIVATE_TREES, arities, source);
    }

    /// Replaces the semi-global range with `arities` (filtered to [`SEMI_GLOBAL_TREES`]).
    pub fn set_semi_global(&mut self, arities: &BTreeMap<u16, usize>, source: AritySource) {
        self.semi_global = self.replace_range(SEMI_GLOBAL_TREES, arities, source);
    }

    fn replace_range(
        &mut self,
        range: RangeInclusive<u16>,
        arities: &BTreeMap<u16, usize>,
        source: AritySource,
    ) -> ArityPart {
        self.arities.retain(|id, _| !range.contains(id));
        let mut count = 0;
        for (&id, &arity) in arities.range(range) {
            self.arities.insert(id, arity);
            count += 1;
        }
        ArityPart { source, count }
    }

    /// The part covering callee `id`, or `None` for primitives.
    #[must_use]
    pub fn part_for(&self, id: u16) -> Option<&ArityPart> {
        if GLOBAL_TREES.contains(&id) {
            Some(&self.global)
        } else if PRIVATE_TREES.contains(&id) {
            Some(&self.private)
        } else if SEMI_GLOBAL_TREES.contains(&id) {
            Some(&self.semi_global)
        } else {
            None
        }
    }

    /// One line naming every range's source, for an output header.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut out = format!(
            "{}; {}; {}",
            self.global.describe("global 256..=4095:"),
            self.private.describe("private 4096..=8191:"),
            self.semi_global.describe("semi-global 8192+:"),
        );
        if let Some(hint) = &self.hint {
            out.push_str("; ");
            out.push_str(hint);
        }
        out
    }
}

/// Which dispatch a calling tree runs under, from its own ID: the VM finds a running
/// tree with the same owner table it uses for callees (`behavior_from_owner`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CallerScope {
    /// `256..=4095`: a `Global.iff` tree, run under the calling object.
    Global,
    /// `4096..=8191`: the owner object's own tree.
    Private,
    /// `8192+`: a tree of the owner's semi-global library.
    SemiGlobal,
}

impl CallerScope {
    pub const ALL: [Self; 3] = [Self::Global, Self::Private, Self::SemiGlobal];

    /// The scope a tree with ID `tree` runs in.
    #[must_use]
    pub fn of(tree: u16) -> Self {
        match TreeScope::of(tree) {
            Some(TreeScope::Private) => Self::Private,
            Some(TreeScope::SemiGlobal) => Self::SemiGlobal,
            Some(TreeScope::Global) | None => Self::Global,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Global => "trees 256..=4095",
            Self::Private => "trees 4096..=8191",
            Self::SemiGlobal => "trees 8192+",
        }
    }
}

/// Callee arities for one file's trees, per [`CallerScope`] of the calling tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileArities {
    global: ResolvedArities,
    private: ResolvedArities,
    semi_global: ResolvedArities,
}

impl FileArities {
    /// The same arities for every calling tree.
    #[must_use]
    pub fn uniform(arities: ResolvedArities) -> Self {
        Self {
            global: arities.clone(),
            private: arities.clone(),
            semi_global: arities,
        }
    }

    /// See [`ResolvedArities::no_role`].
    #[must_use]
    pub fn no_role(why: &str) -> Self {
        Self::uniform(ResolvedArities::no_role(why))
    }

    /// See [`ResolvedArities::unknown`].
    #[must_use]
    pub fn unknown(why: &str) -> Self {
        Self::uniform(ResolvedArities::unknown(why))
    }

    /// See [`ResolvedArities::private_only`].
    #[must_use]
    pub fn private_only(iff: &Iff, why_missing: &str) -> Self {
        Self::uniform(ResolvedArities::private_only(iff, why_missing))
    }

    /// The arities for calling trees in `scope`.
    #[must_use]
    pub fn for_scope(&self, scope: CallerScope) -> &ResolvedArities {
        match scope {
            CallerScope::Global => &self.global,
            CallerScope::Private => &self.private,
            CallerScope::SemiGlobal => &self.semi_global,
        }
    }

    /// The arities for the calling tree `tree`.
    #[must_use]
    pub fn for_tree(&self, tree: u16) -> &ResolvedArities {
        self.for_scope(CallerScope::of(tree))
    }

    fn each_mut(&mut self) -> [&mut ResolvedArities; 3] {
        [&mut self.global, &mut self.private, &mut self.semi_global]
    }

    /// Replaces the global range for every calling tree.
    pub fn set_global(&mut self, arities: &BTreeMap<u16, usize>, source: &AritySource) {
        for scope in self.each_mut() {
            scope.set_global(arities, source.clone());
        }
    }

    /// Replaces the semi-global range for every calling tree.
    pub fn set_semi_global(&mut self, arities: &BTreeMap<u16, usize>, source: &AritySource) {
        for scope in self.each_mut() {
            scope.set_semi_global(arities, source.clone());
        }
    }

    /// One line naming every range's source for the calling trees `trees` (all scopes
    /// when empty). When those trees' scopes resolve differently, each scope is shown
    /// as `for trees …: …`, separated by ` | `.
    #[must_use]
    pub fn describe_for(&self, trees: &[u16]) -> String {
        let mut scopes: Vec<CallerScope> = trees.iter().map(|&t| CallerScope::of(t)).collect();
        scopes.sort_unstable();
        scopes.dedup();
        if scopes.is_empty() {
            scopes = CallerScope::ALL.to_vec();
        }
        let first = self.for_scope(scopes[0]);
        if scopes.iter().all(|&s| self.for_scope(s) == first) {
            return first.describe();
        }
        scopes
            .iter()
            .map(|&s| format!("for {}: {}", s.label(), self.for_scope(s).describe()))
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

/// How an opened file matched the install index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceMatch {
    /// Exactly one index resource: the file's code-owner role is known.
    Unique(usize),
    /// The file resolves outside the install root (canonical paths).
    OutsideInstall,
    /// No index resource is this file.
    Unindexed,
    /// Several index resources are this file (hard links, or symlinks into two mounts).
    Ambiguous(Vec<usize>),
    /// Index paths of the same kind (loose file, or archive with a member of the same
    /// name) could not be `stat`ed, so this file might be one of them; each with its
    /// error.
    MayAlias(Vec<String>),
}

/// The identity of a file: `(device, inode)` on Unix. For an opened file it comes from
/// the open handle (`fstat`), so a file replaced at the same path after it was read (an
/// atomic save) is a different file, and so is a path that reaches another file through
/// symlinks and `..`. Elsewhere it is the canonical path, which cannot see a replacement
/// or an NTFS hard link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIdentity(#[cfg(unix)] (u64, u64), #[cfg(not(unix))] PathBuf);

impl FileIdentity {
    /// The identity of `file`, opened from `path`.
    pub fn of_open(file: &std::fs::File, path: &Path) -> std::io::Result<Self> {
        Self::from_metadata(&file.metadata()?, path)
    }

    /// The identity of the file `path` names now (following symlinks).
    pub fn of_path(path: &Path) -> std::io::Result<Self> {
        Self::from_metadata(&std::fs::metadata(path)?, path)
    }

    /// The identity behind `meta`, which was read for `path`.
    pub fn from_metadata(meta: &std::fs::Metadata, path: &Path) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let _ = path;
            Ok(Self((meta.dev(), meta.ino())))
        }
        #[cfg(not(unix))]
        {
            let _ = meta;
            Ok(Self(std::fs::canonicalize(path)?))
        }
    }
}

#[cfg(feature = "install-catalog")]
/// An install's index and catalog, built exactly as the VM boots them, with callee
/// arities taken from libraries the VM's own loader accepts, and a side table of file
/// identities used only to find which entry a lifted file is.
pub struct InstallArityCatalog {
    pub index: Index,
    pub pool: ArchivePool,
    pub catalog: Catalog,
    /// The install root, canonicalized.
    root_canonical: Option<PathBuf>,
    /// `Global.iff` arities, or why the VM could not link with it.
    global: Result<BTreeMap<u16, usize>, String>,
    global_location: String,
    /// Callable arities of each library loaded so far, or why the VM rejects it.
    libraries: BTreeMap<usize, Result<BTreeMap<u16, usize>, String>>,
    /// Files holding the registered (GUID-winning) definition of some object: the only
    /// files whose private trees run, with that file as code owner.
    owners: BTreeSet<usize>,
    /// The mount a neighborhood's `Characters` directory would take: a resource in a
    /// mount at or above it can be shadowed by name by a `Characters` file.
    characters_mount: usize,
    /// Each index resource's file identity when its path names a regular file.
    identities: Vec<Option<FileIdentity>>,
    /// Index resources whose path could not be `stat`ed for a reason other than
    /// `NotFound`, each with its error.
    unidentified: Vec<(usize, String)>,
}

#[cfg(feature = "install-catalog")]
impl InstallArityCatalog {
    /// Indexes the install at `root`, builds its catalog from the whole index (as the VM
    /// does), and loads `Global.iff` arities. Errors name paths relative to the install
    /// (`<install>/...`).
    pub fn load(root: &Path) -> Result<Self, String> {
        let root_text = root.to_string_lossy().into_owned();
        let relative = |e: String| e.replace(&root_text, "<install>");
        let install = Install::detect(root).map_err(|e| relative(e.to_string()))?;
        let index = Index::build(&install).map_err(|e| relative(e.to_string()))?;
        let pool = ArchivePool::new();
        let catalog =
            Catalog::build_with_pool(&index, &pool).map_err(|e| relative(e.to_string()))?;

        // One `stat` per path. A path that is gone (`NotFound`, e.g. a broken symlink)
        // or not a regular file cannot be the regular file being lifted; any other
        // failure (e.g. no search permission) leaves a path that might be.
        let mut stat_of: BTreeMap<PathBuf, Result<Option<FileIdentity>, String>> = BTreeMap::new();
        let mut identities = Vec::with_capacity(index.resources.len());
        let mut unidentified = Vec::new();
        for (number, r) in index.resources.iter().enumerate() {
            let stat =
                stat_of
                    .entry(r.path.clone())
                    .or_insert_with(|| match std::fs::metadata(&r.path) {
                        Ok(meta) if meta.is_file() => FileIdentity::from_metadata(&meta, &r.path)
                            .map(Some)
                            .map_err(|e| e.to_string()),
                        Ok(_) => Ok(None),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                        Err(e) => Err(e.to_string()),
                    });
            match stat {
                Ok(identity) => identities.push(identity.clone()),
                Err(e) => {
                    identities.push(None);
                    unidentified.push((number, e.clone()));
                }
            }
        }

        let global_location = index.resources[catalog.global].location(&index.root);
        let global = library_arities(&index, &pool, catalog.global, true);
        let owners = catalog
            .by_guid
            .values()
            .map(|&n| catalog.objects[n].resource)
            .collect();
        let characters_mount = index.characters_mount();
        let mut libraries = BTreeMap::new();
        libraries.insert(catalog.global, global.clone());
        Ok(Self {
            root_canonical: std::fs::canonicalize(root).ok(),
            index,
            pool,
            catalog,
            global,
            global_location,
            libraries,
            owners,
            characters_mount,
            identities,
            unidentified,
        })
    }

    /// The nearest install root above `path`. Ancestors of the path as given are tried
    /// first, then ancestors of its canonical form.
    #[must_use]
    pub fn find_install_root(path: &Path) -> Option<PathBuf> {
        let given = std::path::absolute(path).ok();
        let canonical = std::fs::canonicalize(path).ok();
        [given, canonical].into_iter().flatten().find_map(|start| {
            start
                .ancestors()
                .skip(1)
                .find(|dir| Install::detect(dir).is_ok())
                .map(Path::to_path_buf)
        })
    }

    /// Which index resource the file read from `path` is: `identity` is that file's
    /// identity (from the handle it was read through), and `member` its FAR member
    /// `name` at byte `offset`. Only a file inside the install root (canonical paths)
    /// identified as exactly one entry, with no unidentifiable entry of the same kind
    /// that could also be it, is [`ResourceMatch::Unique`].
    #[must_use]
    pub fn resource_for(
        &self,
        path: &Path,
        identity: &FileIdentity,
        member: Option<(&str, u64)>,
    ) -> ResourceMatch {
        let inside = match (&self.root_canonical, std::fs::canonicalize(path)) {
            (Some(root), Ok(file)) => file.starts_with(root),
            _ => false,
        };
        if !inside {
            return ResourceMatch::OutsideInstall;
        }
        let same_kind = |resource: &sims_content::Resource| match (&resource.member, member) {
            (None, None) => true,
            (Some(have), Some((name, _))) => have.name.eq_ignore_ascii_case(name),
            _ => false,
        };
        let may_alias: Vec<String> = self
            .unidentified
            .iter()
            .filter(|(number, _)| same_kind(&self.index.resources[*number]))
            .map(|(number, error)| {
                format!(
                    "{} ({error})",
                    self.index.resources[*number].location(&self.index.root)
                )
            })
            .collect();
        if !may_alias.is_empty() {
            return ResourceMatch::MayAlias(may_alias);
        }
        let member_matches = |resource: &sims_content::Resource| match (&resource.member, member) {
            (None, None) => true,
            (Some(have), Some((name, offset))) => have.name == name && have.offset == offset,
            _ => false,
        };
        let matches: Vec<usize> = self
            .index
            .resources
            .iter()
            .zip(&self.identities)
            .enumerate()
            .filter(|(_, (r, id))| member_matches(r) && id.as_ref() == Some(identity))
            .map(|(n, _)| n)
            .collect();
        match matches.as_slice() {
            [] => ResourceMatch::Unindexed,
            [one] => ResourceMatch::Unique(*one),
            _ => ResourceMatch::Ambiguous(matches),
        }
    }

    /// Callable arities of the library at `resource`, as the VM would load it.
    fn library(&mut self, resource: usize) -> Result<BTreeMap<u16, usize>, String> {
        let is_global = resource == self.catalog.global;
        let (index, pool) = (&self.index, &self.pool);
        self.libraries
            .entry(resource)
            .or_insert_with(|| library_arities(index, pool, resource, is_global))
            .clone()
    }

    fn location(&self, resource: usize) -> String {
        self.index.resources[resource].location(&self.index.root)
    }

    /// Merged arities for the file matched to the index as `matched`, per calling tree,
    /// following the code-owner rules in the module docs. Catalog-derived arities (and
    /// the claim that private calls are local) apply only to a [`ResourceMatch::Unique`]
    /// file; anything else has no inferred role, and every range is unknown.
    pub fn resolve(&mut self, matched: ResourceMatch) -> FileArities {
        match matched {
            ResourceMatch::Unique(resource) => self.resolve_entry(resource),
            ResourceMatch::OutsideInstall => {
                FileArities::no_role("file resolves outside the install root")
            }
            ResourceMatch::Unindexed => FileArities::no_role("file not in install index"),
            ResourceMatch::Ambiguous(entries) => {
                let mut locations: Vec<String> = entries
                    .iter()
                    .map(|&r| self.index.resources[r].location(&self.index.root))
                    .collect();
                if locations.len() > 4 {
                    let more = locations.len() - 3;
                    locations.truncate(3);
                    locations.push(format!("{more} more"));
                }
                FileArities::no_role(&format!(
                    "the same file is indexed as {}",
                    locations.join(" and ")
                ))
            }
            ResourceMatch::MayAlias(paths) => {
                let mut shown = paths.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
                if paths.len() > 3 {
                    shown.push_str(&format!(" and {} more", paths.len() - 3));
                }
                FileArities::no_role(&format!(
                    "{} install index path(s) that could be this file could not be identified: {shown}",
                    paths.len()
                ))
            }
        }
    }

    /// Merged arities for the index resource `resource`, per calling tree: a callee's
    /// count is known only when the VM, booted the same way, would dispatch that call
    /// to that tree.
    fn resolve_entry(&mut self, resource: usize) -> FileArities {
        // Without Global.iff the VM does not link at all (`Program::link_with_pool`).
        let global = match &self.global {
            Ok(global) => global.clone(),
            Err(why) => {
                return FileArities::unknown(&format!("the VM cannot link this install: {why}"));
            }
        };
        let is_global = resource == self.catalog.global;
        let global_location = self.global_location.clone();
        let with_global = || {
            let mut scope = ResolvedArities::unset();
            scope.set_global(&global, AritySource::Catalog(global_location.clone()));
            scope
        };

        // 256..=4095 trees run only from Global.iff, under the calling object.
        let mut global_scope = with_global();
        let why = if is_global {
            "owner-dependent: Global.iff runs under the calling object"
        } else {
            "never runs: the VM runs trees 256..=4095 only from Global.iff"
        };
        global_scope.private = ArityPart::missing(why);
        global_scope.semi_global = ArityPart::missing(why);

        // 4096..=8191 trees run only with this file as code owner, which takes a
        // registered definition here; its own GLOB link is the semi-global library.
        let mut private = with_global();
        if self.owners.contains(&resource) {
            match self.library(resource) {
                Ok(own) => private.set_private(&own, AritySource::ThisFile),
                Err(why) => private.private = ArityPart::missing(&why),
            }
            private.semi_global = match self.catalog.semi_globals.get(&resource).copied() {
                Some(Some(target)) => self.semi_global_part(target, &mut private),
                Some(None) => ArityPart::missing("GLOB link empty"),
                None => ArityPart::missing("no resolved GLOB link"),
            };
        } else {
            let why = "never runs: no registered object definition (OBJD) in this file";
            private.private = ArityPart::missing(why);
            private.semi_global = ArityPart::missing(why);
        }

        // 8192+ trees run only as the semi-global library of a registered object that
        // links here, under that object: semi-global callees are this file's, private
        // ones the caller's.
        let mut semi = with_global();
        let linked = self
            .owners
            .iter()
            .any(|owner| self.catalog.semi_globals.get(owner).copied().flatten() == Some(resource));
        if linked {
            semi.private = ArityPart::missing(
                "owner-dependent: semi-global file runs under the calling object",
            );
            match self.library(resource) {
                Ok(own) => semi.set_semi_global(
                    &own,
                    AritySource::ThisFileBecause("linked as a semi-global".to_owned()),
                ),
                Err(why) => semi.semi_global = ArityPart::missing(&why),
            }
        } else {
            let why = "never runs: no registered object links to this file as its semi-global";
            semi.private = ArityPart::missing(why);
            semi.semi_global = ArityPart::missing(why);
        }

        FileArities {
            global: global_scope,
            private,
            semi_global: semi,
        }
    }

    /// The semi-global part an owner gets from its `GLOB` target `target`, also
    /// merging its arities into `scope`. Unknown when a neighborhood's `Characters` file
    /// of the same name could take precedence: `lift` does not know which neighborhood
    /// the VM boots with, and the VM inserts that mount before `target`'s.
    fn semi_global_part(&mut self, target: usize, scope: &mut ResolvedArities) -> ArityPart {
        let location = self.location(target);
        if self.index.resources[target].mount >= self.characters_mount {
            return ArityPart::missing(&format!(
                "a neighborhood's Characters file named {} would take precedence over {location}, \
                 and the neighborhood is not known",
                self.index.resources[target].file_name()
            ));
        }
        match self.library(target) {
            Ok(map) => {
                scope.set_semi_global(&map, AritySource::Catalog(location));
                scope.semi_global.clone()
            }
            Err(why) => ArityPart::missing(&why),
        }
    }
}

#[cfg(feature = "install-catalog")]
/// Callable arities (all IDs) of the library at index resource `resource`, or why the
/// VM would not load it. It must be a regular file (checked before any open, so a FIFO
/// cannot block) that the VM's own library loader accepts; a count is kept only where
/// the tree the VM keeps for an ID agrees with every chunk of that ID
/// ([`bhav_arities_in_range`]). Errors name paths relative to the install.
fn library_arities(
    index: &Index,
    pool: &ArchivePool,
    resource: usize,
    is_global: bool,
) -> Result<BTreeMap<u16, usize>, String> {
    let file: &Resource = &index.resources[resource];
    let location = file.location(&index.root);
    let relative = |e: String| e.replace(&*index.root.to_string_lossy(), "<install>");
    if !file.is_inline() {
        match std::fs::metadata(&file.path) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => return Err(format!("failed to read {location}: not a regular file")),
            Err(e) => return Err(format!("failed to read {location}: {e}")),
        }
    }
    let library = sims_vm::load_library(index, resource, is_global, pool)
        .map_err(|e| relative(format!("the VM would not load {location}: {e}")))?;
    let bytes = file
        .read_with(pool)
        .map_err(|e| relative(format!("failed to read {location}: {e}")))?;
    let iff = Iff::parse(bytes).map_err(|e| format!("failed to parse {location}: {e}"))?;
    Ok(bhav_arities_in_range(&iff, 0..=u16::MAX)
        .into_iter()
        .filter(|(id, arity)| {
            library
                .behaviors
                .get(id)
                .and_then(|bhav| bhav.frame_dimensions().ok())
                .map(|(args, _)| args)
                == Some(*arity)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::encode_bhav;
    use crate::ir::BhavChunk;
    use sims_formats::behavior::Instruction;
    use sims_formats::iff::{ChunkData, write as write_iff};

    fn bhav(id: u16, args: u8) -> ChunkData {
        let chunk = BhavChunk {
            id,
            label: format!("T{id}"),
            format: 0x8002,
            kind: 0,
            args,
            locals: 0,
            revision: 0,
            instructions: vec![Instruction {
                opcode: 2,
                on_true: 0xFE,
                on_false: 0xFE,
                operands: [0, 0, 0, 0, 0, 5, 0, 7],
            }],
            trailing: Vec::new(),
        };
        ChunkData {
            tag: *b"BHAV",
            id,
            flags: 0,
            label: chunk.label.clone(),
            data: encode_bhav(&chunk).expect("encode_bhav"),
        }
    }

    #[test]
    fn private_only_keeps_private_range_and_overrides_replace_only_their_range() {
        let iff = Iff::parse(
            write_iff(&[bhav(300, 0), bhav(4096, 2), bhav(8192, 3)]).expect("write_iff"),
        )
        .expect("parse");
        let mut resolved = ResolvedArities::private_only(&iff, "test");
        // Stray global/semi-global chunks in the object IFF are ignored.
        assert_eq!(resolved.arities, BTreeMap::from([(4096, 2)]));
        assert!(matches!(resolved.global.source, AritySource::Missing(_)));

        let globals = BTreeMap::from([(300, 6), (4096, 9), (9000, 9)]);
        resolved.set_global(&globals, AritySource::Explicit("g.iff".into()));
        assert_eq!(resolved.arities, BTreeMap::from([(300, 6), (4096, 2)]));
        assert_eq!(resolved.global.count, 1);

        let semi = BTreeMap::from([(8192, 1), (300, 0)]);
        resolved.set_semi_global(&semi, AritySource::Explicit("s.iff".into()));
        assert_eq!(
            resolved.arities,
            BTreeMap::from([(300, 6), (4096, 2), (8192, 1)])
        );
        assert_eq!(resolved.part_for(8192), Some(&resolved.semi_global));
        assert_eq!(resolved.part_for(12), None);
        let line = resolved.describe();
        assert!(
            line.contains("global 256..=4095: 1 from g.iff (explicit)"),
            "{line}"
        );
        assert!(
            line.contains("semi-global 8192+: 1 from s.iff (explicit)"),
            "{line}"
        );
    }

    #[test]
    fn conflicting_duplicate_ids_are_unknown_even_when_one_is_deleted() {
        let mut deleted_seven = bhav(300, 7);
        deleted_seven.flags = sims_formats::iff::DELETED;
        let iff = Iff::parse(
            write_iff(&[bhav(300, 4), deleted_seven, bhav(301, 2), bhav(301, 2)])
                .expect("write_iff"),
        )
        .expect("parse");
        // The VM keeps the deleted 7-arg #300; the original skips it. Neither is trusted.
        assert_eq!(
            bhav_arities_in_range(&iff, GLOBAL_TREES),
            BTreeMap::from([(301, 2)])
        );
    }
}
