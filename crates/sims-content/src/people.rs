//! What the original needs, besides a house file and the neighborhood's
//! records, to construct the people a house's saved objects include: their
//! skeletons and the profiles their resets read.

use crate::import_wardrobe::{AvatarPaths, mount_avatar_paths, resources_with_dir};
use crate::{Catalog, Error, Index, Install, Resource, Result};
use sims_formats::avatar::Cmx;
use sims_formats::avatar::catalog::{Asset, Binary};
use sims_formats::avatar::index::{IndexReader, Kind};
use sims_formats::iff::Iff;
use sims_formats::strings::{StringTable, decode};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// The skeleton a type of person is built on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skeleton {
    /// The skeleton's name, from the type's body strings.
    pub name: String,
    /// The file that holds it, as a path relative to the install.
    pub file: String,
    /// Its place among the skeletons of that file.
    pub place: usize,
    /// For every skeleton of the file, in the file's order, the wiggle
    /// scale of each bone (the integer at bone offset 0x78).
    pub scales: Vec<Vec<i32>>,
    /// The skeleton's bones, in the file's order, as far as posing reads
    /// them: what the walk's strides are worked out from (0x58831d).
    pub bones: Vec<Bone>,
}

/// One bone of a skeleton, as posing reads it. Floats are kept as the
/// bits of the file's single-precision values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bone {
    /// The bone's name, which a clip's motion names it by.
    pub name: String,
    /// The bone it hangs from, by place; None for the root.
    pub parent: Option<usize>,
    /// Its rest position relative to its parent.
    pub translation: [u32; 3],
    /// Its rest rotation relative to its parent, a quaternion.
    pub rotation: [u32; 4],
    /// Whether a layer below weight 1 blends with what the bone holds
    /// (the third of the bone's flags, native bone +0x74, read by
    /// 0x597fc0); a bone that does not blend takes the layer's sample
    /// whatever its weight.
    pub blends: bool,
}

/// What a person's reset reads from the person type's files when it
/// reloads the person's appearance (0x5888d3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    /// Strings 1 and 2 of the type's body strings, counting from 0: the
    /// initial body and head outfit strings (`"suit,BODY=..."` /
    /// `"suit,HEAD-HEAD=..."`). None is a string the table does not have.
    pub body: Option<String>,
    pub head: Option<String>,
    /// Strings 12, 13 and 14 of the type's body strings, counting from 0:
    /// the gender, the age and the skin tone. None is a string the table
    /// does not have; an empty string is still a string.
    pub gender: Option<String>,
    pub age: Option<String>,
    pub skin: Option<String>,
    /// The skin tone names: string table 8192 of the type's semi-global
    /// file, in order. Empty when the file or the table is missing.
    pub tones: Vec<String>,
    /// Strings 3 to 6 of the type's body strings, counting from 0, which
    /// the animator binds its anchors to (0x5924a4): the bone that holds
    /// the head's point ("HEAD"), that point's name ("Top"), the bone that
    /// holds the hand's ("R_HAND") and that point's name ("Palm"). None
    /// is a string the table does not have.
    pub rig: [Option<String>; 4],
}

/// The skeletons and profiles of a set of person types.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct People {
    /// Each person type's skeleton, by the type's GUID.
    pub skeletons: BTreeMap<u32, Skeleton>,
    /// Each person type's profile, by the type's GUID.
    pub profiles: BTreeMap<u32, Profile>,
    /// The names of the suits (appearances) the avatar registry holds
    /// (eager `.cmx` and `.bcf` catalogs in the `0x5835ed` appearance
    /// paths, lowercased by `0x594d73`/`0x595140`, plus indexed `.ndx`
    /// appearance entries from `0x584562` → `0x5a0212` → `0x59fc48` whose
    /// first-registered `.bcf` catalog defines the appearance).
    pub suits: BTreeSet<String>,
    /// Each avatar catalog or index in the original's search paths whose
    /// skeletons or suits stopped decoding, by location, with the error:
    /// its suits after that point are missing from `suits`.
    pub suit_errors: BTreeMap<String, String>,
    /// Every suit name (`.cmx` / `.bcf` appearance) whose bindings include
    /// an exact byte-for-byte `"HEAD"` bone (`0x4e810d` → `0x4e81a4` →
    /// `0x431726`), as checked by the portrait renderer at `0x4e7824`.
    pub head_suits: BTreeSet<String>,
    /// Whether each loaded person type's initial body/head suits (body
    /// strings 1 and 2) bind `"HEAD"` byte-for-byte (`0x4e725f` → `0x4e7817`
    /// → `0x4e810d` → `0x4e81a4` → `0x431726` → `0x4e7824`). Absent when a
    /// suit catalog failed to decode before confirming a binding or in
    /// synthetic fixtures that do not populate it.
    pub portrait_head: BTreeMap<u32, bool>,
}

/// The body strings that name a person's initial body and head suits.
const BODY_SUIT: usize = 1;
const HEAD_SUIT: usize = 2;

/// The string table an object definition of type 2 names its body in: the
/// definition's word 19 (0x457f60). Its first string is the skeleton's
/// name.
const BODY_STRINGS: usize = 19 * 2;

/// The body strings a reset reads (0x5888d3), counting from 0.
const GENDER: usize = 12;
const AGE: usize = 13;
const SKIN: usize = 14;
/// The first of the four body strings the animator's anchors read
/// (0x5924a4), counting from 0.
const RIG: usize = 3;

/// The string table of a semi-global file that names the skin tones.
const SKIN_TONES: u16 = 8192;

/// One skeleton registration in [`AvatarRegistry`].
#[derive(Debug, Clone)]
enum SkeletonEntry {
    /// Registered eagerly by `0x59f844` → `0x59f087` → `0x59f132`.
    Loaded {
        file: String,
        place: usize,
        scales: Vec<Vec<i32>>,
        bones: Vec<Bone>,
    },
    /// Registered lazily by `0x584562` → `0x5a0212` → `0x59fc48` from an
    /// `.ndx` file, and loaded on demand at `0x5a0384` → `0x5a3810` →
    /// `0x59e60e`.
    Indexed {
        ndx_location: String,
        directory: String,
        catalog_bcf: Vec<u8>,
    },
    /// Attempted lazy lookup that did not populate the entry (`0x5a389b`):
    /// subsequent catalog loads discard matching records (`0x59f18c`).
    Failed(String),
}

/// The original's startup avatar registry (`0x5835ed`, `0x59f844`,
/// `0x584562` → `0x5a0212` → `0x59fc48`), for skeleton (`0x5a0384`) and
/// suit (`0x5a044d`) lookup.
///
/// - **Search paths and order (`0x5835ed`).** Replays the startup mounts
///   (`0x43ae81`) and splits avatar paths into eager `appearance` folders
///   (`GameData/Skins` first, `ExpansionShared/Skeletons`, expansions in
///   reverse, loose download providers) and `indexed` folders
///   (`GameData/Animation` and download providers with `<stem>.ndx`).
///   Files outside these paths (for example `GameData/Objects`) are not
///   scanned.
/// - **Eager pass (`0x59f844`).** Reads all `.cmx` catalogs before `.bcf`
///   catalogs in `appearance` order (`0x5c1a7d`). Skeleton names keep
///   their exact byte case (`0x59f163`) and the first registration wins
///   (`0x59f132`, `0x59f18c`). Text `.cmx` skeleton sections are not
///   implemented and record a blocking skeleton error so a replacement
///   `.cmx` rig cannot be silently missed.
/// - **Indexed pass (`0x584562` → `0x5a0212` → `0x59fc48`).** Reads `.ndx`
///   streams in `indexed` order. A skeleton already registered by the
///   eager pass or an earlier `.ndx` is skipped (`0x59fd07`). Otherwise
///   `".bcf"` is appended to its catalog name (`0x59fd18`) and resolved
///   against the `.ndx` file's mounted directory on demand (`0x5a3810` →
///   `0x59e60e`).
/// - **Lookup (`0x5a0384`, `0x5a280f`).** Compares skeleton names byte for
///   byte. Fails loudly if an earlier catalog or index in registration
///   order stopped decoding its skeletons before the name was registered
///   (since it could define the name), on a miss (`0x5a03c0`), or when an
///   indexed catalog is missing or undecodable (`0x5a3810`).
pub struct AvatarRegistry {
    root: PathBuf,
    paths: AvatarPaths,
    skeletons: BTreeMap<String, (usize, SkeletonEntry)>,
    first_skeleton_error: Option<(usize, String)>,
    suits: BTreeSet<String>,
    head_suits: BTreeSet<String>,
    suit_errors: BTreeMap<String, String>,
}

impl AvatarRegistry {
    /// Builds the startup avatar registry for `install`.
    pub fn load(install: &Install) -> Result<Self> {
        let paths = mount_avatar_paths(install, false)?;
        let mut skeletons: BTreeMap<String, (usize, SkeletonEntry)> = BTreeMap::new();
        let mut first_skeleton_error: Option<(usize, String)> = None;
        let mut suits = BTreeSet::new();
        let mut head_suits = BTreeSet::new();
        let mut suit_errors = BTreeMap::new();
        let mut step = 0usize;

        // 0x59f844: all eager `.cmx` catalogs first, then all eager `.bcf`.
        for (_, _, source) in resources_with_dir(&paths.files, &paths.appearance, b".cmx")? {
            let location = source.location(&install.root);
            let bytes = source.read()?;
            if let Err((blocks_skeletons, error)) =
                read_cmx_catalog(&bytes, &mut suits, &mut head_suits)
            {
                suit_errors.insert(location.clone(), error.to_string());
                if blocks_skeletons && first_skeleton_error.is_none() {
                    step += 1;
                    first_skeleton_error = Some((
                        step,
                        format!("the skeletons of {location} do not decode ({error})"),
                    ));
                }
            }
        }

        for (_, _, source) in resources_with_dir(&paths.files, &paths.appearance, b".bcf")? {
            let location = source.location(&install.root);
            let bytes = source.read()?;
            let mut binary = Binary::new(&bytes);
            let (decoded, skeleton_error) = read_bcf_skeletons(&mut binary);
            let all_scales: Vec<Vec<i32>> = decoded
                .iter()
                .map(|(_, scales, _)| scales.clone())
                .collect();
            for (place, (name, _, bones)) in decoded.into_iter().enumerate() {
                step += 1;
                skeletons.entry(name).or_insert_with(|| {
                    (
                        step,
                        SkeletonEntry::Loaded {
                            file: location.clone(),
                            place,
                            scales: all_scales.clone(),
                            bones,
                        },
                    )
                });
            }
            if let Some(error) = skeleton_error {
                suit_errors.insert(location.clone(), error.to_string());
                if first_skeleton_error.is_none() {
                    step += 1;
                    first_skeleton_error = Some((
                        step,
                        format!("the skeletons of {location} do not decode ({error})"),
                    ));
                }
                continue;
            }
            if let Err(error) = read_bcf_appearances(&mut binary, &mut suits, &mut head_suits) {
                suit_errors.insert(location, error.to_string());
            }
        }

        // 0x584562 → 0x5a0212 → 0x59fc48: `.ndx` catalogs in indexed paths.
        // Keep the first appearance registration (0x59fe53) and verify it
        // against its referenced `.bcf` catalog below (0x5a044d → 0x5a38da).
        let mut indexed_suits: BTreeMap<String, (String, String, Vec<u8>)> = BTreeMap::new();
        for (directory, _, source) in resources_with_dir(&paths.files, &paths.indexed, b".ndx")? {
            let location = source.location(&install.root);
            let bytes = source.read()?;
            let mut reader = IndexReader::new(&bytes);
            while let Some(item) = reader.next() {
                match item {
                    Ok(entry) => match entry.kind {
                        Kind::Skeleton => {
                            let name = decode(&entry.name, 1);
                            let mut catalog_bcf = entry.catalog;
                            catalog_bcf.extend_from_slice(b".bcf");
                            step += 1;
                            skeletons.entry(name).or_insert_with(|| {
                                (
                                    step,
                                    SkeletonEntry::Indexed {
                                        ndx_location: location.clone(),
                                        directory: directory.clone(),
                                        catalog_bcf,
                                    },
                                )
                            });
                        }
                        Kind::Appearance => {
                            let name = decode(&entry.name, 1);
                            if !suits.contains(&name) && !indexed_suits.contains_key(&name) {
                                let mut catalog_bcf = entry.catalog;
                                catalog_bcf.extend_from_slice(b".bcf");
                                indexed_suits.insert(
                                    name,
                                    (location.clone(), directory.clone(), catalog_bcf),
                                );
                            }
                        }
                        Kind::Animation => {}
                    },
                    Err(error) => match reader.failed_kind() {
                        Some(Kind::Skeleton) => {
                            suit_errors.insert(location.clone(), error.to_string());
                            if first_skeleton_error.is_none() {
                                step += 1;
                                first_skeleton_error = Some((
                                    step,
                                    format!("the index {location} does not decode ({error})"),
                                ));
                            }
                        }
                        Some(Kind::Appearance) => {
                            suit_errors.insert(location.clone(), error.to_string());
                        }
                        Some(Kind::Animation) | None => {}
                    },
                }
            }
        }

        // Decode each referenced `.bcf` catalog once (keyed by resolved
        // resource location so case/slash aliases share one decode) to confirm
        // the indexed appearance names it actually defines (0x5a044d,
        // 0x5950f5, 0x5a27f2). Unreadable suit-only catalogs record into
        // `suit_errors` instead of aborting skeleton-only callers.
        let mut by_catalog: BTreeMap<String, (Resource, Vec<String>)> = BTreeMap::new();
        for (name, (ndx_location, directory, catalog_bcf)) in indexed_suits {
            match paths.files.lookup(&directory, &catalog_bcf) {
                Ok(Some(resource)) => {
                    let location = resource.location(&install.root);
                    by_catalog
                        .entry(location)
                        .or_insert_with(|| (resource, Vec::new()))
                        .1
                        .push(name);
                }
                Ok(None) => {}
                Err(error) => {
                    suit_errors.insert(ndx_location, error.to_string());
                }
            }
        }
        for (location, (resource, names)) in by_catalog {
            let bytes = match resource.read() {
                Ok(bytes) => bytes,
                Err(error) => {
                    suit_errors.insert(location, error.to_string());
                    continue;
                }
            };
            let mut binary = Binary::new(&bytes);
            let (_, skeleton_error) = read_bcf_skeletons(&mut binary);
            if let Some(error) = skeleton_error {
                suit_errors.insert(location, error.to_string());
                continue;
            }
            let mut catalog_suits = BTreeSet::new();
            let mut catalog_head_suits = BTreeSet::new();
            if let Err(error) =
                read_bcf_appearances(&mut binary, &mut catalog_suits, &mut catalog_head_suits)
            {
                suit_errors.insert(location, error.to_string());
            }
            for name in names {
                if catalog_suits.contains(&name) {
                    if catalog_head_suits.contains(&name) {
                        head_suits.insert(name.clone());
                    }
                    suits.insert(name);
                }
            }
        }

        Ok(Self {
            root: install.root.clone(),
            paths,
            skeletons,
            first_skeleton_error,
            suits,
            head_suits,
            suit_errors,
        })
    }

    /// Resolves the skeleton named `name` byte-for-byte (`0x5a0384`,
    /// `0x5a280f`), loading an `.ndx`-indexed `.bcf` catalog on demand
    /// (`0x5a3810` → `0x59e60e` → `0x59f087`). Every skeleton decoded from
    /// that catalog before any error is immediately registered (filling
    /// existing unloaded entries via `0x5a2207` and preserving already-loaded
    /// or previously attempted entries via `0x59f18c`). An unresolved lazy
    /// lookup is marked attempted (`0x5a389b`) so later catalog loads cannot
    /// overwrite it.
    pub fn skeleton(&mut self, name: &str) -> Result<Skeleton> {
        let Some((reg_step, entry)) = self.skeletons.get(name) else {
            return Err(Error::Unsupported(match &self.first_skeleton_error {
                Some((_, reason)) => {
                    format!("0x5a0384: {reason}, so whether it defines {name:?} is unknown")
                }
                None => format!("0x5a03c0: no skeleton file defines the skeleton {name:?}"),
            }));
        };
        if let Some((err_step, reason)) = &self.first_skeleton_error
            && *reg_step >= *err_step
        {
            return Err(Error::Unsupported(format!(
                "0x5a0384: {reason}, so whether it defines {name:?} is unknown"
            )));
        }
        let alpha_step = *reg_step;
        let (ndx_location, directory, catalog_bcf) = match entry {
            SkeletonEntry::Loaded {
                file,
                place,
                scales,
                bones,
            } => {
                return Ok(Skeleton {
                    name: name.to_owned(),
                    file: file.clone(),
                    place: *place,
                    scales: scales.clone(),
                    bones: bones.clone(),
                });
            }
            SkeletonEntry::Failed(reason) => return Err(Error::Unsupported(reason.clone())),
            SkeletonEntry::Indexed {
                ndx_location,
                directory,
                catalog_bcf,
            } => (ndx_location.clone(), directory.clone(), catalog_bcf.clone()),
        };

        let Some(resource) = self.paths.files.lookup(&directory, &catalog_bcf)? else {
            let catalog = decode(&catalog_bcf, 1);
            let reason = format!(
                "0x5a3810: {ndx_location} points skeleton {name:?} to {catalog:?}, \
                 which is missing in {directory}"
            );
            if let Some((_, entry)) = self.skeletons.get_mut(name) {
                *entry = SkeletonEntry::Failed(reason.clone());
            }
            return Err(Error::Unsupported(reason));
        };
        let location = resource.location(&self.root);
        let bytes = resource.read()?;
        let mut binary = Binary::new(&bytes);
        let (decoded, error) = read_bcf_skeletons(&mut binary);
        let all_scales: Vec<Vec<i32>> = decoded
            .iter()
            .map(|(_, scales, _)| scales.clone())
            .collect();
        for (place, (skel_name, _, bones)) in decoded.into_iter().enumerate() {
            let loaded = SkeletonEntry::Loaded {
                file: location.clone(),
                place,
                scales: all_scales.clone(),
                bones,
            };
            match self.skeletons.get_mut(&skel_name) {
                Some((entry_step, entry @ SkeletonEntry::Indexed { .. })) => {
                    *entry_step = (*entry_step).min(alpha_step);
                    *entry = loaded;
                }
                Some((_, SkeletonEntry::Loaded { .. } | SkeletonEntry::Failed(_))) => {}
                None => {
                    self.skeletons.insert(skel_name, (alpha_step, loaded));
                }
            }
        }
        if let Some((
            _,
            SkeletonEntry::Loaded {
                file,
                place,
                scales,
                bones,
            },
        )) = self.skeletons.get(name)
        {
            return Ok(Skeleton {
                name: name.to_owned(),
                file: file.clone(),
                place: *place,
                scales: scales.clone(),
                bones: bones.clone(),
            });
        }
        let reason = if let Some(error) = error {
            format!("0x5a3810: the skeletons of {location} do not decode ({error})")
        } else {
            format!(
                "0x5a03c0: {ndx_location} points skeleton {name:?} to {location}, \
                 which does not define it"
            )
        };
        if let Some((_, entry)) = self.skeletons.get_mut(name) {
            *entry = SkeletonEntry::Failed(reason.clone());
        }
        Err(Error::Unsupported(reason))
    }

    /// The wiggle bone scales of the skeleton named `name` (`0x5a0384`,
    /// `0x598da3`).
    pub fn skeleton_scales(&mut self, name: &str) -> Result<Vec<i32>> {
        let skeleton = self.skeleton(name)?;
        Ok(skeleton.scales[skeleton.place].clone())
    }
}

impl People {
    /// Finds the skeleton and profile of each person type in `guids`. A
    /// GUID that is not a person type (OBJD type 2) is skipped.
    ///
    /// Skeletons and suits are resolved through [`AvatarRegistry`], matching
    /// the original's startup avatar search paths (`0x5835ed`): eager
    /// `.cmx` and `.bcf` catalogs (`GameData/Skins` first,
    /// `ExpansionShared/Skeletons`, expansions in reverse, loose downloads),
    /// then `.ndx` index files (`GameData/Animation/Animation.ndx` and
    /// indexed downloads), with byte-exact skeleton lookup (`0x5a0384`,
    /// `0x5a280f`) where the first registration wins (`0x59f132`).
    pub fn load(
        install: &Install,
        index: &Index,
        catalog: &Catalog,
        guids: impl IntoIterator<Item = u32>,
    ) -> Result<Self> {
        let (people, mut unresolved) = Self::load_each(install, index, catalog, guids)?;
        match unresolved.pop_first() {
            Some((_, reason)) => Err(Error::Unsupported(reason)),
            None => Ok(people),
        }
    }

    /// As [`People::load`], but a person type whose skeleton cannot be
    /// resolved is left out of both maps instead of failing the whole load.
    /// Returns the types left out, by GUID, with the reason. Making a person
    /// of a type left out fails, as it would with no entry at all.
    pub fn load_each(
        install: &Install,
        index: &Index,
        catalog: &Catalog,
        guids: impl IntoIterator<Item = u32>,
    ) -> Result<(Self, BTreeMap<u32, String>)> {
        let mut unresolved = BTreeMap::new();
        let mut names = BTreeMap::new();
        let mut profiles = BTreeMap::new();
        let mut default_suits: BTreeMap<u32, Vec<String>> = BTreeMap::new();
        for guid in guids {
            let Some(object) = catalog.object(guid) else {
                continue;
            };
            if object.kind != 2 || names.contains_key(&guid) {
                continue;
            }
            let resource = &index.resources[object.resource];
            let body = body_strings(index, resource, object.chunk)?;
            let row = |n: usize| body.get(n, 1).map(str::to_owned);
            let suit_name = |n: usize| {
                body.get(n, 1)
                    .map(|raw| {
                        raw.split(',')
                            .next()
                            .unwrap_or(raw)
                            .trim()
                            .to_ascii_lowercase()
                    })
                    .filter(|s| !s.is_empty() && s != "none")
            };
            let initial_suits: Vec<String> = [BODY_SUIT, HEAD_SUIT]
                .into_iter()
                .filter_map(suit_name)
                .collect();
            default_suits.insert(guid, initial_suits);
            let tones = match catalog.semi_globals.get(&object.resource) {
                Some(&Some(semi_global)) => skin_tones(index, &index.resources[semi_global])?,
                _ => Vec::new(),
            };
            profiles.insert(
                guid,
                Profile {
                    body: row(BODY_SUIT),
                    head: row(HEAD_SUIT),
                    gender: row(GENDER),
                    age: row(AGE),
                    skin: row(SKIN),
                    tones,
                    rig: std::array::from_fn(|i| row(RIG + i)),
                },
            );
            let location = resource.location(&index.root);
            let name = body
                .get(0, 1)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| {
                    Error::Unsupported(format!(
                        "{location}: the person's body strings name no skeleton"
                    ))
                })?;
            names.insert(guid, name);
        }
        let mut registry = AvatarRegistry::load(install)?;
        let mut skeletons = BTreeMap::new();
        for (guid, name) in names {
            match registry.skeleton(&name) {
                Ok(skeleton) => {
                    skeletons.insert(guid, skeleton);
                }
                Err(Error::Unsupported(reason)) => {
                    unresolved.insert(guid, reason);
                }
                Err(error) => return Err(error),
            }
        }
        profiles.retain(|guid, _| !unresolved.contains_key(guid));
        let mut portrait_head = BTreeMap::new();
        for (guid, suits) in default_suits {
            if unresolved.contains_key(&guid) {
                continue;
            }
            if suits.iter().any(|s| registry.head_suits.contains(s)) {
                portrait_head.insert(guid, true);
            } else if registry.suit_errors.is_empty() {
                portrait_head.insert(guid, false);
            }
        }
        Ok((
            Self {
                skeletons,
                profiles,
                suits: registry.suits,
                suit_errors: registry.suit_errors,
                head_suits: registry.head_suits,
                portrait_head,
            },
            unresolved,
        ))
    }
}

/// The skeleton the definition `guid` names: the first of its body
/// strings (the string table its word 19 names, 0x457f60). Unlike
/// [`People::load`], this reads any definition, such as the pools'
/// person templates, whose OBJD types are not 2.
pub fn skeleton_name(index: &Index, catalog: &Catalog, guid: u32) -> Result<String> {
    let object = catalog
        .object(guid)
        .ok_or_else(|| Error::Unsupported(format!("no definition has the GUID {guid:#010x}")))?;
    let resource = &index.resources[object.resource];
    let body = body_strings(index, resource, object.chunk)?;
    body.get(0, 1)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            Error::Unsupported(format!(
                "{}: the definition's body strings name no skeleton",
                resource.location(&index.root)
            ))
        })
}

/// The bone scales of the skeleton named `name`, looked up through
/// [`AvatarRegistry`] (`0x5a0384`, `0x5a280f`). For multiple lookups on one
/// install, reuse [`AvatarRegistry::load`].
pub fn skeleton_scales(install: &Install, name: &str) -> Result<Vec<i32>> {
    AvatarRegistry::load(install)?.skeleton_scales(name)
}

/// The body strings of the person definition `chunk` of a character
/// file.
fn body_strings(index: &Index, resource: &Resource, chunk: u16) -> Result<StringTable> {
    let location = resource.location(&index.root);
    let iff = Iff::parse(resource.read()?).map_err(|e| Error::format(&location, e))?;
    let missing = |what: &str| Error::Unsupported(format!("{location}: {what}"));
    let definition = iff
        .get(b"OBJD", chunk)
        .ok_or_else(|| missing("the person's definition is gone"))?;
    let table = iff
        .data(definition)
        .get(BODY_STRINGS..BODY_STRINGS + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| missing("the person's definition names no body strings"))?;
    let strings = iff
        .get(b"STR#", table)
        .ok_or_else(|| missing("the person's body strings are missing"))?;
    StringTable::parse(iff.data(strings))
        .map_err(|e| Error::format(&format!("{location} STR# #{table}"), e))
}

/// The skin tone names of a semi-global file: its string table 8192.
fn skin_tones(index: &Index, resource: &Resource) -> Result<Vec<String>> {
    let location = resource.location(&index.root);
    let iff = Iff::parse(resource.read()?).map_err(|e| Error::format(&location, e))?;
    let Some(chunk) = iff.get(b"STR#", SKIN_TONES) else {
        return Ok(Vec::new());
    };
    let table = StringTable::parse(iff.data(chunk))
        .map_err(|e| Error::format(&format!("{location} STR# #{SKIN_TONES}"), e))?;
    Ok(table
        .entries
        .iter()
        .filter(|entry| entry.language == 1)
        .map(|entry| entry.text.clone())
        .collect())
}

/// A skeleton file's skeletons: each one's name, bone scales and bones.
type DecodedSkeletons = Vec<(String, Vec<i32>, Vec<Bone>)>;

/// Reads a text `.cmx` catalog's skeleton count and appearances (`0x59e746`,
/// `0x594d73`). Returns `Err((true, e))` if the header, skeleton count or a
/// non-empty skeleton section fails (so skeleton lookup must fail loudly),
/// and `Err((false, e))` if an appearance record fails after 0 skeletons.
fn read_cmx_catalog(
    bytes: &[u8],
    suits: &mut BTreeSet<String>,
    head_suits: &mut BTreeSet<String>,
) -> std::result::Result<(), (bool, sims_formats::Error)> {
    let mut cmx = Cmx::new(bytes).map_err(|e| (true, e))?;
    let Some(skel_count) = cmx.count(Kind::Skeleton).map_err(|e| (true, e))? else {
        return Err((
            true,
            sims_formats::Error::new(bytes.len(), "missing CMX skeleton count"),
        ));
    };
    if skel_count > 0 {
        cmx.read(Kind::Skeleton).map_err(|e| (true, e))?;
    }
    let Some(app_count) = cmx.count(Kind::Appearance).map_err(|e| (false, e))? else {
        return Ok(());
    };
    for _ in 0..app_count {
        if let Asset::Appearance(appearance) = cmx.read(Kind::Appearance).map_err(|e| (false, e))? {
            if appearance.bindings.iter().any(|b| b.bone == "HEAD") {
                head_suits.insert(appearance.name.clone());
            }
            suits.insert(appearance.name);
        }
    }
    Ok(())
}

/// Reads the `Kind::Skeleton` group of a binary `.bcf` catalog record by
/// record (`0x59f087`), keeping any skeletons decoded before an error.
fn read_bcf_skeletons(binary: &mut Binary<'_>) -> (DecodedSkeletons, Option<sims_formats::Error>) {
    let mut out = Vec::new();
    let count = match binary.count(Kind::Skeleton) {
        Ok(Some(count)) => count,
        Ok(None) => {
            return (
                out,
                Some(sims_formats::Error::new(
                    binary.position(),
                    "missing BCF skeleton count",
                )),
            );
        }
        Err(error) => return (out, Some(error)),
    };
    for _ in 0..count {
        match binary.read(Kind::Skeleton) {
            Ok(Asset::Skeleton(skeleton)) => {
                let scales = skeleton
                    .bones
                    .iter()
                    .map(|b| b.wiggle.gradient_scale)
                    .collect();
                let bones = skeleton
                    .bones
                    .into_iter()
                    .map(|b| Bone {
                        name: b.name,
                        parent: b.parent,
                        translation: b.translation.map(f32::to_bits),
                        rotation: b.rotation.map(f32::to_bits),
                        blends: b.flags[2] != 0,
                    })
                    .collect();
                out.push((skeleton.name, scales, bones));
            }
            Ok(_) => {}
            Err(error) => return (out, Some(error)),
        }
    }
    (out, None)
}

/// Reads the `Kind::Appearance` group of a binary `.bcf` catalog record by
/// record (`0x595140`, `0x5950f5` lowercasing each suit name). A short
/// appearance count (`Ok(None)`) is treated as a read failure (`0x59f1c1`).
fn read_bcf_appearances(
    binary: &mut Binary<'_>,
    suits: &mut BTreeSet<String>,
    head_suits: &mut BTreeSet<String>,
) -> sims_formats::Result<()> {
    let Some(count) = binary.count(Kind::Appearance)? else {
        return Err(sims_formats::Error::new(
            binary.position(),
            "missing BCF appearance count",
        ));
    };
    for _ in 0..count {
        if let Asset::Appearance(appearance) = binary.read(Kind::Appearance)? {
            if appearance.bindings.iter().any(|b| b.bone == "HEAD") {
                head_suits.insert(appearance.name.clone());
            }
            suits.insert(appearance.name);
        }
    }
    Ok(())
}

/// Adds the suits one avatar file defines to `suits`. A file too short
/// to hold a group's count ends there, as in the original.
pub(crate) fn read_suits(bytes: &[u8], suits: &mut BTreeSet<String>) -> sims_formats::Result<()> {
    let mut catalog = Binary::new(bytes);
    for kind in [Kind::Skeleton, Kind::Appearance] {
        let Some(count) = catalog.count(kind)? else {
            return Ok(());
        };
        for _ in 0..count {
            if let Asset::Appearance(appearance) = catalog.read(kind)? {
                suits.insert(appearance.name);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Expansions, Layout};
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "sims-content-people-{name}-{}-{n}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, rel: &str, bytes: &[u8]) {
            let path = self.0.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, bytes).unwrap();
        }

        fn install(&self) -> Install {
            Install {
                root: self.0.clone(),
                layout: Layout::Cd,
                expansions: Expansions(0),
            }
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// An avatar file with no skeletons, the suits `names` (no bindings),
    /// and then `tail`.
    fn catalog(names: &[&str], tail: &[u8]) -> Vec<u8> {
        let mut bytes = 0u32.to_le_bytes().to_vec();
        bytes.extend((names.len() as u32).to_le_bytes());
        for name in names {
            bytes.push(name.len() as u8);
            bytes.extend(name.as_bytes());
            bytes.extend([0u8; 12]); // kind, reserved, no bindings
        }
        bytes.extend(tail);
        bytes
    }

    /// An avatar file holding the one-bone skeletons `skeletons`, each a
    /// name and its bone's scale, and no suits.
    fn skeletons(skeletons: &[(&str, i32)]) -> Vec<u8> {
        let string = |bytes: &mut Vec<u8>, text: &str| {
            bytes.push(text.len() as u8);
            bytes.extend(text.as_bytes());
        };
        let mut bytes = (skeletons.len() as u32).to_le_bytes().to_vec();
        for &(name, scale) in skeletons {
            string(&mut bytes, name);
            bytes.extend(1u32.to_le_bytes());
            string(&mut bytes, "ROOT");
            string(&mut bytes, "NULL");
            bytes.extend(0u32.to_le_bytes()); // no properties
            for value in [0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0] {
                bytes.extend(value.to_le_bytes()); // translation, rotation
            }
            bytes.extend([0u8; 12]); // flags
            bytes.extend(scale.to_le_bytes());
            bytes.extend(0.0f32.to_le_bytes()); // amplitude
        }
        bytes.extend(0u32.to_le_bytes()); // no suits
        bytes
    }

    /// Builds an `.ndx` file with the given skeleton and appearance entries
    /// (each `(name, catalog_without_bcf)`).
    fn ndx(skels: &[(&str, &str)], apps: &[(&str, &str)]) -> Vec<u8> {
        let push_str = |out: &mut Vec<u8>, s: &str| {
            out.push(s.len() as u8);
            out.extend_from_slice(s.as_bytes());
        };
        let mut out = Vec::new();
        for group in [skels, apps, &[]] {
            out.extend((group.len() as i32).to_le_bytes());
            for &(name, cat) in group {
                push_str(&mut out, name);
                push_str(&mut out, cat);
            }
        }
        out
    }

    #[test]
    fn skins_overrides_animation_ndx_and_objects_outside_search_are_ignored() {
        let dir = ScratchDir::new("skins-override");
        // Animation.ndx defines adult (scale 10) and child (scale 20).
        dir.write(
            "GameData/Animation/Animation.ndx",
            &ndx(
                &[
                    ("adult", "adult-skeleton.cmx"),
                    ("child", "child-skeleton.cmx"),
                ],
                &[("suit-from-ndx", "suits.cmx")],
            ),
        );
        dir.write(
            "GameData/Animation/adult-skeleton.cmx.bcf",
            &skeletons(&[("adult", 10)]),
        );
        dir.write(
            "GameData/Animation/child-skeleton.cmx.bcf",
            &skeletons(&[("child", 20)]),
        );
        dir.write(
            "GameData/Animation/suits.cmx.bcf",
            &catalog(&["suit-from-ndx"], &[]),
        );
        // Eager replacement rig in GameData/Skins overrides adult (scale 99)
        // and defines case-distinct Adult (scale 77). A second adult in the
        // same file (scale 55) loses to the first registration (0x59f132).
        dir.write(
            "GameData/Skins/override.cmx.bcf",
            &skeletons(&[("Adult", 77), ("adult", 99), ("adult", 55)]),
        );
        // A later eager folder (GameData/cmx, after GameData/Skins in 0x5835ed)
        // also loses to GameData/Skins's first registration.
        dir.write("GameData/cmx/later.cmx.bcf", &skeletons(&[("adult", 44)]));
        // An undecodable BCF in GameData/Objects (outside avatar search paths)
        // must not block skeleton or suit lookups.
        dir.write("GameData/Objects/broken.bcf", &[1, 0, 0, 0, 5, b'x']);

        let install = dir.install();
        let mut registry = AvatarRegistry::load(&install).unwrap();
        assert_eq!(registry.skeleton_scales("adult").unwrap(), [99]);
        assert_eq!(registry.skeleton_scales("Adult").unwrap(), [77]);
        assert_eq!(registry.skeleton_scales("child").unwrap(), [20]);
        assert!(registry.suits.contains("suit-from-ndx"));
        assert!(registry.suit_errors.is_empty());

        let miss = registry.skeleton_scales("ADULT").unwrap_err().to_string();
        assert!(miss.contains("0x5a03c0"), "{miss}");
    }

    #[test]
    fn undecodable_skin_blocks_later_registrations_but_not_earlier_ones() {
        let dir = ScratchDir::new("broken-skin");
        // GameData/Skins precedes GameData/cmx, which precedes GameData/Animation
        // in 0x5835ed's search order.
        dir.write("GameData/Skins/first.bcf", &skeletons(&[("adult", 11)]));
        dir.write("GameData/cmx/broken.bcf", &[1, 0, 0, 0, 9, b'x']);
        dir.write(
            "GameData/Animation/Animation.ndx",
            &ndx(&[("child", "child-skeleton.cmx")], &[]),
        );
        dir.write(
            "GameData/Animation/child-skeleton.cmx.bcf",
            &skeletons(&[("child", 22)]),
        );

        let install = dir.install();
        let mut registry = AvatarRegistry::load(&install).unwrap();
        // Registered in GameData/Skins before GameData/cmx/broken.bcf failed.
        assert_eq!(registry.skeleton_scales("adult").unwrap(), [11]);
        // Registered in Animation.ndx after broken.bcf failed: fails loudly.
        let blocked = registry.skeleton_scales("child").unwrap_err().to_string();
        assert!(blocked.contains("broken.bcf"), "{blocked}");
        // Unregistered name also reports broken.bcf rather than 0x5a03c0.
        let unknown = registry.skeleton_scales("other").unwrap_err().to_string();
        assert!(unknown.contains("broken.bcf"), "{unknown}");
    }

    #[test]
    fn cmx_with_skeleton_section_blocks_later_skeleton_lookups() {
        let dir = ScratchDir::new("cmx-skel");
        dir.write(
            "GameData/Skins/custom.cmx",
            b"// test\r\nversion 300\r\n1\r\nadult\r\n",
        );
        dir.write(
            "GameData/Animation/Animation.ndx",
            &ndx(&[("adult", "adult-skeleton.cmx")], &[]),
        );
        dir.write(
            "GameData/Animation/adult-skeleton.cmx.bcf",
            &skeletons(&[("adult", 10)]),
        );

        let install = dir.install();
        let mut registry = AvatarRegistry::load(&install).unwrap();
        let err = registry.skeleton_scales("adult").unwrap_err().to_string();
        assert!(err.contains("custom.cmx"), "{err}");
    }

    #[test]
    fn missing_ndx_catalog_fails_at_0x5a3810() {
        let dir = ScratchDir::new("missing-ndx-cat");
        dir.write(
            "GameData/Animation/Animation.ndx",
            &ndx(&[("adult", "missing.cmx")], &[]),
        );
        let install = dir.install();
        let mut registry = AvatarRegistry::load(&install).unwrap();
        let err = registry.skeleton_scales("adult").unwrap_err().to_string();
        assert!(err.contains("0x5a3810"), "{err}");
    }

    #[test]
    fn unrelated_malformed_far_does_not_abort_avatar_loading() {
        let dir = ScratchDir::new("unrelated-bad-far");
        dir.write("GameData/Objects/unrelated.far", b"not a valid FAR");
        dir.write(
            "GameData/Animation/Animation.ndx",
            &ndx(&[("adult", "adult-skeleton.cmx")], &[]),
        );
        dir.write(
            "GameData/Animation/adult-skeleton.cmx.bcf",
            &skeletons(&[("adult", 15)]),
        );
        let install = dir.install();
        let mut registry = AvatarRegistry::load(&install).unwrap();
        assert_eq!(registry.skeleton_scales("adult").unwrap(), [15]);
    }

    #[test]
    fn lazy_skeleton_load_registers_siblings_and_keeps_prefix_before_truncation() {
        let dir = ScratchDir::new("lazy-siblings");
        // alpha's catalog defines both alpha and beta; beta's own indexed
        // catalog is missing. Loading alpha populates beta (0x5a2207).
        dir.write(
            "GameData/Animation/Animation.ndx",
            &ndx(
                &[
                    ("alpha", "shared.cmx"),
                    ("beta", "missing.cmx"),
                    ("gamma", "truncated.cmx"),
                    ("delta", "truncated.cmx"),
                ],
                &[],
            ),
        );
        dir.write(
            "GameData/Animation/shared.cmx.bcf",
            &skeletons(&[("alpha", 11), ("beta", 22)]),
        );
        // truncated.cmx.bcf claims 2 skeletons: gamma succeeds, then record 2
        // is truncated. Native 0x59f087 registers gamma before failing.
        let mut truncated = skeletons(&[("gamma", 33)]);
        truncated[0..4].copy_from_slice(&2u32.to_le_bytes());
        truncated.truncate(truncated.len() - 4); // strip 0 suits count
        truncated.extend([9, b'x']);
        dir.write("GameData/Animation/truncated.cmx.bcf", &truncated);

        let install = dir.install();
        let mut registry = AvatarRegistry::load(&install).unwrap();
        assert_eq!(registry.skeleton_scales("alpha").unwrap(), [11]);
        assert_eq!(registry.skeleton_scales("beta").unwrap(), [22]);
        assert_eq!(registry.skeleton_scales("gamma").unwrap(), [33]);
        let delta_err = registry.skeleton_scales("delta").unwrap_err().to_string();
        assert!(delta_err.contains("0x5a3810"), "{delta_err}");
    }

    #[test]
    fn failed_lazy_lookup_stays_failed_and_unseen_sibling_survives_later_ndx_error() {
        // Case 1 (0x5a389b + 0x59f18c): looking up beta before alpha marks
        // beta as attempted, so loading alpha's shared catalog cannot later
        // overwrite beta.
        let dir = ScratchDir::new("lazy-attempted");
        dir.write(
            "GameData/Animation/Animation.ndx",
            &ndx(&[("alpha", "shared.cmx"), ("beta", "missing.cmx")], &[]),
        );
        dir.write(
            "GameData/Animation/shared.cmx.bcf",
            &skeletons(&[("alpha", 11), ("beta", 22)]),
        );
        let install = dir.install();
        let mut registry = AvatarRegistry::load(&install).unwrap();
        let beta_err = registry.skeleton_scales("beta").unwrap_err().to_string();
        assert!(beta_err.contains("0x5a3810"), "{beta_err}");
        assert_eq!(registry.skeleton_scales("alpha").unwrap(), [11]);
        let beta_still_err = registry.skeleton_scales("beta").unwrap_err().to_string();
        assert!(beta_still_err.contains("0x5a3810"), "{beta_still_err}");

        // Case 2: a truncated NDX registers alpha before failing. Loading
        // alpha decodes and registers unseen beta, which must be available
        // despite the later NDX error.
        let dir2 = ScratchDir::new("lazy-unseen-after-ndx-err");
        let mut bad_ndx = Vec::new();
        bad_ndx.extend(2i32.to_le_bytes()); // 2 skeletons, 2nd truncated
        bad_ndx.push(5);
        bad_ndx.extend_from_slice(b"alpha");
        bad_ndx.push(10);
        bad_ndx.extend_from_slice(b"shared.cmx");
        bad_ndx.extend([9, b'x']);
        dir2.write("GameData/Animation/Animation.ndx", &bad_ndx);
        dir2.write(
            "GameData/Animation/shared.cmx.bcf",
            &skeletons(&[("alpha", 11), ("beta", 22)]),
        );
        let install2 = dir2.install();
        let mut registry2 = AvatarRegistry::load(&install2).unwrap();
        assert_eq!(registry2.skeleton_scales("alpha").unwrap(), [11]);
        assert_eq!(registry2.skeleton_scales("beta").unwrap(), [22]);
    }

    #[test]
    fn unreadable_suit_catalog_and_short_appearance_count_record_suit_errors_only() {
        let dir = ScratchDir::new("unreadable-suit-cat");
        dir.write(
            "GameData/Animation/Animation.ndx",
            &ndx(
                &[("adult", "adult-skeleton.cmx")],
                &[
                    ("giant-suit", "oversized.cmx"),
                    ("short-suit", "short-app.cmx"),
                    ("alias-suit", "SHORT-APP.CMX"),
                ],
            ),
        );
        dir.write(
            "GameData/Animation/adult-skeleton.cmx.bcf",
            &skeletons(&[("adult", 19)]),
        );
        // Sparse file above the 128 MiB resource limit: unreadable for suits,
        // but must not abort AvatarRegistry::load or skeleton lookup.
        let oversized = dir.0.join("GameData/Animation/oversized.cmx.bcf");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(sims_formats::far::MAX_MEMBER + 1)
            .unwrap();
        // 0 skeletons + 2-byte truncated appearance count (0x59f1c1 failure).
        dir.write("GameData/Animation/short-app.cmx.bcf", &[0, 0, 0, 0, 1, 0]);

        let install = dir.install();
        let mut registry = AvatarRegistry::load(&install).unwrap();
        assert_eq!(registry.skeleton_scales("adult").unwrap(), [19]);
        assert!(!registry.suits.contains("giant-suit"));
        assert!(!registry.suits.contains("short-suit"));
        assert!(
            registry
                .suit_errors
                .keys()
                .any(|k| k.ends_with("oversized.cmx.bcf")),
            "{:?}",
            registry.suit_errors
        );
        assert!(
            registry
                .suit_errors
                .keys()
                .any(|k| k.ends_with("short-app.cmx.bcf")),
            "{:?}",
            registry.suit_errors
        );
    }

    #[test]
    fn indexed_suits_keep_first_reference_and_require_catalog_definition() {
        let dir = ScratchDir::new("indexed-suits");
        // hat's first reference is missing; its later duplicate points to an
        // existing catalog with hat (must not override first reference).
        // ghost points to an existing empty catalog (must not register).
        // coat points to an existing catalog defining coat (must register).
        dir.write(
            "GameData/Animation/Animation.ndx",
            &ndx(
                &[],
                &[
                    ("hat", "missing.cmx"),
                    ("hat", "real.cmx"),
                    ("ghost", "empty.cmx"),
                    ("coat", "real.cmx"),
                ],
            ),
        );
        dir.write(
            "GameData/Animation/real.cmx.bcf",
            &catalog(&["hat", "coat"], &[]),
        );
        dir.write("GameData/Animation/empty.cmx.bcf", &catalog(&[], &[]));

        let install = dir.install();
        let registry = AvatarRegistry::load(&install).unwrap();
        assert!(registry.suits.contains("coat"));
        assert!(!registry.suits.contains("hat"));
        assert!(!registry.suits.contains("ghost"));
        assert!(registry.suit_errors.is_empty());
    }

    #[test]
    fn ndx_errors_are_attributed_by_failing_group() {
        // Case 1: complete skeleton group (0 skeletons) + truncated first
        // appearance must NOT block skeletons from a later index, only record
        // suit_errors.
        let dir = ScratchDir::new("ndx-app-err");
        let mut bad_app = Vec::new();
        bad_app.extend(0i32.to_le_bytes()); // 0 skeletons
        bad_app.extend(1i32.to_le_bytes()); // 1 appearance, truncated
        bad_app.extend([9, b'x']);
        dir.write("GameData/Animation/Animation.ndx", &bad_app);
        dir.write(
            "GameData/Animation/other.ndx",
            &ndx(&[("adult", "adult-skeleton.cmx")], &[]),
        );
        dir.write(
            "GameData/Animation/adult-skeleton.cmx.bcf",
            &skeletons(&[("adult", 42)]),
        );
        // Ensure both .ndx files are read; Animation.ndx fails in appearance
        // group, so adult from other.ndx must still resolve.
        let install = dir.install();
        let mut registry = AvatarRegistry::load(&install).unwrap();
        assert_eq!(registry.skeleton_scales("adult").unwrap(), [42]);
        let miss = registry
            .skeleton_scales("nonexistent")
            .unwrap_err()
            .to_string();
        assert!(miss.contains("0x5a03c0"), "{miss}");
        assert!(
            registry
                .suit_errors
                .keys()
                .any(|k| k.ends_with("Animation.ndx")),
            "{:?}",
            registry.suit_errors
        );

        // Case 2: complete skeleton and appearance groups + truncated
        // animation group must add neither a skeleton error nor suit_errors.
        let dir2 = ScratchDir::new("ndx-anim-err");
        let mut bad_anim = Vec::new();
        bad_anim.extend(1i32.to_le_bytes()); // 1 skeleton
        bad_anim.push(5);
        bad_anim.extend_from_slice(b"adult");
        bad_anim.push(14);
        bad_anim.extend_from_slice(b"adult-skeleton");
        bad_anim.extend(0i32.to_le_bytes()); // 0 appearances
        bad_anim.extend(1i32.to_le_bytes()); // 1 animation, truncated
        bad_anim.extend([9, b'x']);
        dir2.write("GameData/Animation/Animation.ndx", &bad_anim);
        dir2.write(
            "GameData/Animation/adult-skeleton.bcf",
            &skeletons(&[("adult", 50)]),
        );
        let install2 = dir2.install();
        let mut registry2 = AvatarRegistry::load(&install2).unwrap();
        assert_eq!(registry2.skeleton_scales("adult").unwrap(), [50]);
        assert!(
            registry2.suit_errors.is_empty(),
            "{:?}",
            registry2.suit_errors
        );
    }

    #[test]
    fn missing_skeleton_count_in_eager_bcf_or_cmx_blocks_later_skeletons() {
        // 3-byte BCF in GameData/Skins must block later indexed skeletons and
        // must not report a misleading "catalog group out of sequence" error.
        let dir = ScratchDir::new("short-bcf");
        dir.write("GameData/Skins/short.bcf", &[1, 2, 3]);
        dir.write(
            "GameData/Animation/Animation.ndx",
            &ndx(&[("adult", "adult-skeleton.cmx")], &[]),
        );
        dir.write(
            "GameData/Animation/adult-skeleton.cmx.bcf",
            &skeletons(&[("adult", 10)]),
        );
        let install = dir.install();
        let mut registry = AvatarRegistry::load(&install).unwrap();
        let err = registry.skeleton_scales("adult").unwrap_err().to_string();
        assert!(err.contains("short.bcf"), "{err}");
        let suit_err = registry
            .suit_errors
            .values()
            .next()
            .expect("suit_errors should record short.bcf");
        assert!(!suit_err.contains("out of sequence"), "{suit_err}");

        // Header-only CMX in GameData/Skins must also block later skeletons.
        let dir2 = ScratchDir::new("header-only-cmx");
        dir2.write(
            "GameData/Skins/empty.cmx",
            b"// header only\r\nversion 300\r\n",
        );
        dir2.write(
            "GameData/Animation/Animation.ndx",
            &ndx(&[("adult", "adult-skeleton.cmx")], &[]),
        );
        dir2.write(
            "GameData/Animation/adult-skeleton.cmx.bcf",
            &skeletons(&[("adult", 10)]),
        );
        let install2 = dir2.install();
        let mut registry2 = AvatarRegistry::load(&install2).unwrap();
        let err2 = registry2.skeleton_scales("adult").unwrap_err().to_string();
        assert!(err2.contains("empty.cmx"), "{err2}");
    }

    #[test]
    fn a_file_with_animations_still_gives_its_suits() {
        // One animation follows, which neither this reader nor
        // `read_bcf` decodes.
        let bytes = catalog(&["Suit-Book101", "hat"], &1u32.to_le_bytes());
        let mut suits = BTreeSet::new();
        read_suits(&bytes, &mut suits).unwrap();
        assert_eq!(suits, ["hat", "suit-book101"].map(String::from).into());
    }

    #[test]
    fn a_file_that_fails_keeps_the_suits_before_the_failure() {
        let mut bytes = catalog(&["cane"], &[]);
        // Claims a second suit, whose name runs past the end.
        bytes[4] = 2;
        bytes.extend([9, b'x']);
        let mut suits = BTreeSet::new();
        assert!(read_suits(&bytes, &mut suits).is_err());
        assert_eq!(suits, ["cane".to_string()].into());
        // Too short for the first count: nothing, and no error.
        read_suits(&[0, 0], &mut suits).unwrap();
    }

    #[test]
    fn head_suits_match_bone_name_case_sensitively() {
        // 0x4e810d -> 0x4e81a4 -> 0x431726 compares the binding bone name
        // byte-for-byte against "HEAD".
        let cmx = b"// test\r\n\
version 300\r\n\
0\r\n\
3\r\n\
c001_exact\r\n\
0\r\n\
0\r\n\
1\r\n\
HEAD\r\n\
xskin-c001-HEAD-HEAD\r\n\
0\r\n\
0\r\n\
c002_lower\r\n\
0\r\n\
0\r\n\
1\r\n\
head\r\n\
xskin-c002-head\r\n\
0\r\n\
0\r\n\
b001_body\r\n\
0\r\n\
0\r\n\
1\r\n\
PELVIS\r\n\
xskin-b001-PELVIS-BODY\r\n\
0\r\n\
0\r\n";
        let mut suits = BTreeSet::new();
        let mut head_suits = BTreeSet::new();
        read_cmx_catalog(cmx, &mut suits, &mut head_suits).unwrap();
        assert_eq!(
            suits,
            ["b001_body", "c001_exact", "c002_lower"]
                .map(String::from)
                .into()
        );
        assert_eq!(head_suits, ["c001_exact".to_string()].into());
    }
}
