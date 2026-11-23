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
//! An object IFF's trees run with that object as owner, so its private and semi-global
//! ranges are known. `Global.iff` and semi-global IFFs run under whichever object called
//! them, so their owner-dependent ranges stay unknown (a semi-global IFF does know its own
//! semi-global range: every owner that reaches it links to it). Unknown arities make the
//! lifter keep the labeled-CFG layout, which is always sound.
//!
//! [`InstallArityCatalog`] resolves those files through `sims_content`'s [`Catalog`] (the
//! same index, catalog, and `GLOB` links the VM boots from). The `structured_equiv` corpus
//! test and `sims-antics lift` both use it, so the CLI and the corpus see the same arities.

use crate::verify::bhav_param_limit;
use sims_content::{ArchivePool, Catalog, Index, Install};
use sims_formats::behavior::Behavior;
use sims_formats::iff::Iff;
use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

/// Global tree IDs, resolved from `Global.iff`.
pub const GLOBAL_TREES: RangeInclusive<u16> = 256..=4095;
/// Private tree IDs, resolved from the owner's own IFF.
pub const PRIVATE_TREES: RangeInclusive<u16> = 4096..=8191;
/// Semi-global tree IDs, resolved from the owner's `GLOB`-linked IFF.
pub const SEMI_GLOBAL_TREES: RangeInclusive<u16> = 8192..=u16::MAX;

/// `tree ID -> argument count` for the BHAVs in `iff` whose ID is in `range`.
///
/// Every parseable BHAV chunk counts, deleted or not, in any order: the VM's library
/// loader keeps the last physical chunk of an ID (deleted ones included) while the
/// original's loader skips deleted chunks. An ID whose chunks disagree on arity is left
/// out (unknown), so the result is sound under either selection rule.
#[must_use]
pub fn bhav_arities_in_range(iff: &Iff, range: RangeInclusive<u16>) -> BTreeMap<u16, usize> {
    let mut seen: BTreeMap<u16, Option<usize>> = BTreeMap::new();
    for chunk in &iff.chunks {
        if &chunk.tag != b"BHAV" || !range.contains(&chunk.id) {
            continue;
        }
        if let Ok(bhav) = Behavior::parse(iff.data(chunk)) {
            let arity = bhav_param_limit(&bhav);
            seen.entry(chunk.id)
                .and_modify(|known| {
                    if *known != Some(arity) {
                        *known = None;
                    }
                })
                .or_insert(Some(arity));
        }
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
}

impl ResolvedArities {
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
        format!(
            "{}; {}; {}",
            self.global.describe("global 256..=4095:"),
            self.private.describe("private 4096..=8191:"),
            self.semi_global.describe("semi-global 8192+:"),
        )
    }
}

/// How a lifted file matched the install index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceMatch {
    /// Exactly one index resource.
    Unique(usize),
    /// Several index resources (e.g. one file reached through symlinks in two mounts),
    /// which may give it different code-owner roles.
    Ambiguous(usize),
    /// Not in the index.
    Unindexed,
    /// The path as written names an index entry, but resolves to a different file (a
    /// symlinked directory before `..`), and the file actually opened is not indexed.
    PathMismatch,
}

/// Lexically normalizes an absolute path: drops `.` and folds `..` into its parent.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// An install's index and catalog, with `Global.iff` arities loaded and semi-global
/// arities cached per resource.
pub struct InstallArityCatalog {
    pub index: Index,
    pub pool: ArchivePool,
    pub catalog: Catalog,
    /// `Global.iff` arities, or why they could not be read.
    global: Result<BTreeMap<u16, usize>, String>,
    global_location: String,
    semi_cache: BTreeMap<usize, Result<BTreeMap<u16, usize>, String>>,
}

impl InstallArityCatalog {
    /// Indexes the install at `root`, builds its catalog, and loads `Global.iff` arities.
    /// Errors name paths relative to the install (`<install>/...`).
    pub fn load(root: &Path) -> Result<Self, String> {
        let root_text = root.to_string_lossy().into_owned();
        let relative = |e: String| e.replace(&root_text, "<install>");
        let install = Install::detect(root).map_err(|e| relative(e.to_string()))?;
        let mut index = Index::build(&install).map_err(|e| relative(e.to_string()))?;
        // The catalog scan opens every IFF; drop anything that is not a regular file
        // (a FIFO would block the open). Symlinks to regular files stay.
        let mut regular: BTreeMap<PathBuf, bool> = BTreeMap::new();
        index.resources.retain(|r| {
            *regular
                .entry(r.path.clone())
                .or_insert_with(|| std::fs::metadata(&r.path).is_ok_and(|m| m.is_file()))
        });
        let pool = ArchivePool::new();
        let catalog =
            Catalog::build_with_pool(&index, &pool).map_err(|e| relative(e.to_string()))?;
        let global_resource = &index.resources[catalog.global];
        let global_location = global_resource.location(&index.root);
        let global = global_resource
            .read_with(&pool)
            .map_err(|e| {
                format!(
                    "failed to read {global_location}: {}",
                    relative(e.to_string())
                )
            })
            .and_then(|bytes| {
                Iff::parse(bytes).map_err(|e| format!("failed to parse {global_location}: {e}"))
            })
            .map(|iff| bhav_arities_in_range(&iff, GLOBAL_TREES));
        Ok(Self {
            index,
            pool,
            catalog,
            global,
            global_location,
            semi_cache: BTreeMap::new(),
        })
    }

    /// The nearest install root above `path`. Ancestors of the path as given are tried
    /// first (so an install reached through a symlink is the one whose catalog applies),
    /// then ancestors of its canonical form.
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

    /// The index resource for the file at `path`, or for its FAR member `name` at byte
    /// `offset` when `member` is given. The path as given (then lexically normalized)
    /// is tried first, but only accepted when that entry is the file actually opened:
    /// `link/../X.iff` opens the `X.iff` beside the link's *target*, not the one the
    /// lexical form names. Otherwise canonical paths are compared, and more than one
    /// canonical match is [`ResourceMatch::Ambiguous`].
    #[must_use]
    pub fn resource_for(&self, path: &Path, member: Option<(&str, u64)>) -> ResourceMatch {
        let member_matches = |resource: &sims_content::Resource| match (&resource.member, member) {
            (None, None) => true,
            (Some(have), Some((name, offset))) => have.name == name && have.offset == offset,
            _ => false,
        };
        let (Ok(given), Ok(canonical)) = (std::path::absolute(path), std::fs::canonicalize(path))
        else {
            return ResourceMatch::Unindexed;
        };
        let mut lexical_mismatch = false;
        for logical in [given.clone(), normalize_lexically(&given)] {
            if let Some(found) = self
                .index
                .resources
                .iter()
                .position(|r| member_matches(r) && r.path == logical)
            {
                if std::fs::canonicalize(&logical).is_ok_and(|c| c == canonical) {
                    return ResourceMatch::Unique(found);
                }
                lexical_mismatch = true;
            }
        }
        let mut canonical_of: BTreeMap<&Path, Option<PathBuf>> = BTreeMap::new();
        let matches: Vec<usize> = self
            .index
            .resources
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                member_matches(r)
                    && canonical_of
                        .entry(r.path.as_path())
                        .or_insert_with(|| std::fs::canonicalize(&r.path).ok())
                        .as_ref()
                        == Some(&canonical)
            })
            .map(|(n, _)| n)
            .collect();
        match matches.as_slice() {
            [] if lexical_mismatch => ResourceMatch::PathMismatch,
            [] => ResourceMatch::Unindexed,
            [one] => ResourceMatch::Unique(*one),
            many => ResourceMatch::Ambiguous(many.len()),
        }
    }

    fn semi_arities(&mut self, semi_res: usize) -> Result<BTreeMap<u16, usize>, String> {
        let index = &self.index;
        let pool = &self.pool;
        self.semi_cache
            .entry(semi_res)
            .or_insert_with(|| {
                let resource = &index.resources[semi_res];
                let location = resource.location(&index.root);
                resource
                    .read_with(pool)
                    .map_err(|e| {
                        format!("failed to read {location}: {e}")
                            .replace(&*index.root.to_string_lossy(), "<install>")
                    })
                    .and_then(|b| {
                        Iff::parse(b).map_err(|e| format!("failed to parse {location}: {e}"))
                    })
                    .map(|semi_iff| bhav_arities_in_range(&semi_iff, SEMI_GLOBAL_TREES))
            })
            .clone()
    }

    /// Merged arities for `iff`, matched to the index as `matched`, following the
    /// code-owner rules in the module docs. A file outside the index gets only its
    /// private range (it may come from another install); an ambiguous match gets only the
    /// owner-independent global range.
    pub fn resolve(&mut self, matched: ResourceMatch, iff: &Iff) -> ResolvedArities {
        let resource = match matched {
            ResourceMatch::Unique(resource) => Some(resource),
            ResourceMatch::Unindexed => {
                return ResolvedArities::private_only(iff, "file not in install index");
            }
            ResourceMatch::PathMismatch => {
                return ResolvedArities::private_only(
                    iff,
                    "path names an install index entry but opens a different, unindexed \
                     file (symlink before `..`)",
                );
            }
            ResourceMatch::Ambiguous(count) => {
                let why = format!("owner-dependent: file matches {count} install index entries");
                let mut resolved = ResolvedArities {
                    arities: BTreeMap::new(),
                    private: ArityPart::missing(&why),
                    global: ArityPart::missing("unset"),
                    semi_global: ArityPart::missing(&why),
                };
                self.apply_global(&mut resolved);
                return resolved;
            }
        };
        let is_global = resource == Some(self.catalog.global);
        let linked_by_others = resource.is_some_and(|r| {
            self.catalog
                .semi_globals
                .iter()
                .any(|(&owner, &target)| owner != r && target == Some(r))
        });
        let own_link = resource.and_then(|r| self.catalog.semi_globals.get(&r).copied());

        let mut resolved = ResolvedArities {
            arities: BTreeMap::new(),
            private: ArityPart::missing("unset"),
            global: ArityPart::missing("unset"),
            semi_global: ArityPart::missing("unset"),
        };
        self.apply_global(&mut resolved);

        if is_global {
            resolved.private =
                ArityPart::missing("owner-dependent: Global.iff runs under the calling object");
        } else if linked_by_others {
            resolved.private = ArityPart::missing(
                "owner-dependent: semi-global file runs under the calling object",
            );
        } else {
            resolved.set_private(
                &bhav_arities_in_range(iff, PRIVATE_TREES),
                AritySource::ThisFile,
            );
        }

        let this = resource;
        match (is_global, own_link) {
            (true, _) => {
                resolved.semi_global =
                    ArityPart::missing("owner-dependent: Global.iff runs under the calling object");
            }
            (false, Some(Some(target))) if !linked_by_others || Some(target) == this => {
                match self.semi_arities(target) {
                    Ok(map) => {
                        let location = self.index.resources[target].location(&self.index.root);
                        resolved.set_semi_global(&map, AritySource::Catalog(location));
                    }
                    Err(why) => resolved.semi_global = ArityPart::missing(&why),
                }
            }
            (false, Some(Some(_))) => {
                resolved.semi_global = ArityPart::missing(
                    "ambiguous: linked as a semi-global by other objects and has its own GLOB link",
                );
            }
            (false, _) if linked_by_others => {
                resolved.set_semi_global(
                    &bhav_arities_in_range(iff, SEMI_GLOBAL_TREES),
                    AritySource::ThisFileBecause("linked as a semi-global".to_owned()),
                );
            }
            (false, Some(None)) => resolved.semi_global = ArityPart::missing("GLOB link empty"),
            (false, None) => {
                resolved.semi_global = ArityPart::missing("no resolved GLOB link");
            }
        }
        resolved
    }

    fn apply_global(&self, resolved: &mut ResolvedArities) {
        match &self.global {
            Ok(global) => {
                resolved.set_global(global, AritySource::Catalog(self.global_location.clone()))
            }
            Err(why) => resolved.global = ArityPart::missing(why),
        }
    }
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
