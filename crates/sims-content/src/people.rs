//! What the original needs, besides a house file and the neighborhood's
//! records, to construct the people a house's saved objects include: their
//! skeletons and the profiles their resets read.

use crate::file_table::SuitTable;
use crate::import_wardrobe::{AvatarPaths, mount_avatar_paths, resources_with_dir};
use crate::{Catalog, Error, Index, Install, Resource, Result};
use sims_formats::avatar::catalog::{Asset, Binary};
use sims_formats::avatar::index::{IndexReader, Kind};
use sims_formats::avatar::{Cmx, read_bmf, read_skn};
use sims_formats::far::encode_name;
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
    /// Non-empty body string rows beyond the default body/head rows (row index
    /// -> raw string), for outfit changes (`0x58a178`: naked row 15, swim
    /// rows 16/31, formal rows 27/30, pajamas row 32, winter row 33, high
    /// fashion row 34, and special NPC suit rows).
    pub suit_rows: BTreeMap<usize, String>,
}

/// Decoded contents of one `.ndx`-referenced `.bcf` skeleton catalog: its
/// skeletons in file order plus any decode error encountered partway through.
pub type IndexedCatalog = (Vec<Skeleton>, Option<String>);

/// Decoded appearances of one `.ndx`-referenced `.bcf` catalog (`0x5a38da` →
/// `0x59f35f`): each lowercased appearance name in file order mapped to whether
/// its first definition in that catalog has a valid `"HEAD"` mesh binding, plus
/// any decode error encountered partway through.
pub type IndexedSuitCatalog = (Vec<(String, bool)>, Option<String>);

/// The skeletons and profiles of a set of person types.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct People {
    /// Each person type's skeleton, by the type's GUID.
    pub skeletons: BTreeMap<u32, Skeleton>,
    /// Skeletons already loaded at startup (eager `.bcf` catalogs only —
    /// `.ndx` catalogs stay unloaded in [`People::indexed_skeletons`] until
    /// runtime lookup order resolves them), keyed byte-for-byte by skeleton
    /// name (`0x5a0384`, `0x5a280f`).
    pub skeletons_by_name: BTreeMap<String, Skeleton>,
    /// Still-unloaded `.ndx`-indexed skeletons (`0x584562` → `0x59fc48`),
    /// keyed byte-for-byte by skeleton name: `Ok(catalog_location)` pointing
    /// into [`People::indexed_catalogs`], or `Err(reason)` if blocked by an
    /// earlier decode error or missing catalog.
    pub indexed_skeletons: BTreeMap<String, std::result::Result<String, String>>,
    /// Decoded skeletons for each catalog referenced by
    /// [`People::indexed_skeletons`], keyed by catalog location, so runtime
    /// lookups (`0x5a0384` → `0x59f18c`–`0x59f19f`) can populate unloaded
    /// sibling entries in exact runtime call order.
    pub indexed_catalogs: BTreeMap<String, IndexedCatalog>,
    /// Each person type's body string 0 (skeleton name, `STR#` word 19 row 0),
    /// kept even when the `.bcf` skeleton itself is unavailable (for the early
    /// pet guard at `0x440d63`).
    pub skeleton_names: BTreeMap<u32, String>,
    /// Each person type's profile, by the type's GUID.
    pub profiles: BTreeMap<u32, Profile>,
    /// The names of the suits (appearances) the avatar registry holds
    /// (eager `.cmx` and `.bcf` catalogs in the `0x5835ed` appearance
    /// paths, lowercased by `0x594d73`/`0x595140`, plus indexed `.ndx`
    /// appearance entries from `0x584562` → `0x5a0212` → `0x59fc48` whose
    /// first-registered `.bcf` catalog defines the appearance).
    pub suits: BTreeSet<String>,
    /// Appearance names registered eagerly from `.cmx` and `.bcf` catalogs at
    /// startup (`0x59f844`, already loaded so later `.ndx` loads cannot
    /// override them at `0x59f34c`).
    pub eager_suits: BTreeSet<String>,
    /// Still-unloaded `.ndx`-indexed appearances (`0x584562` → `0x59fc48`),
    /// keyed by lowercased appearance name: `Ok(catalog_location)` pointing
    /// into [`People::indexed_suit_catalogs`], or `Err(reason)` if its catalog
    /// is missing or unreadable.
    pub indexed_suits: BTreeMap<String, std::result::Result<String, String>>,
    /// Decoded appearances for each `.bcf` catalog referenced by
    /// [`People::indexed_suits`], keyed by catalog location, so runtime
    /// lookups (`0x5a044d` → `0x5a38da` → `0x59f35f`) can populate unloaded
    /// sibling appearance entries in exact runtime call order.
    pub indexed_suit_catalogs: BTreeMap<String, IndexedSuitCatalog>,
    /// Per-catalog required `"HEAD"` mesh bone lists for each `.bcf` catalog
    /// in [`People::indexed_suit_catalogs`] (`catalog_location -> suit_name ->
    /// Vec<Vec<String>>`), so runtime loads (`0x59f34c..0x59f35f`) preserve the
    /// first-loaded catalog's mesh requirements (`0x599f8a..0x59a01c`).
    pub indexed_suit_head_bones: BTreeMap<String, BTreeMap<String, Vec<Vec<String>>>>,
    /// Each avatar catalog or index in the original's search paths whose
    /// skeletons or suits stopped decoding, by location, with the error:
    /// its suits after that point are missing from `suits`.
    pub suit_errors: BTreeMap<String, String>,
    /// Every winning suit name (`.cmx` / `.bcf` appearance, first registration
    /// wins per `0x59ea68`/`0x59f34c`) that has at least one exact byte-for-byte
    /// `"HEAD"` bone binding (`0x4e810d` → `0x4e81a4` → `0x431726`) whose
    /// `.bmf` or `.skn` mesh exists and decodes (`0x595629` → `0x59e1b6` →
    /// `0x59e32a`).
    pub head_suits: BTreeSet<String>,
    /// For each suit in [`People::head_suits`] (and lazy `.ndx` suits with
    /// decodable `"HEAD"` meshes), the list of required skeleton bone names for
    /// each decodable `"HEAD"` mesh (`0x599f8a..0x59a01c` rejects a mesh if any
    /// bone it names is missing from the Sim's skeleton).
    pub head_mesh_bones: BTreeMap<String, Vec<Vec<String>>>,
    /// Whether each loaded person type's initial body/head suits (body
    /// strings 1 and 2) bind `"HEAD"` byte-for-byte with a valid mesh
    /// (`0x4e725f` → `0x4e7817` → `0x4e810d` → `0x4e81a4` → `0x431726` →
    /// `0x4e7824`). Absent when a suit catalog failed to decode before
    /// confirming a binding or in synthetic fixtures that do not populate it.
    pub portrait_head: BTreeMap<u32, bool>,
    /// The startup avatar registry's appearance hash table (`[AvatarRegistry + 0x28]`,
    /// `0x59f844` + `0x59fc48`), in exact native bucket/chain layout for
    /// `0x409255` (`0x6e48a0`).
    pub suit_table: SuitTable,
    /// Texture stems (`BMP` then `TGA`, truncated to 63 bytes) from the
    /// startup texture search paths (`0x414633` → `0x40895a` → `0x6e48b0`),
    /// in exact native scan order.
    pub textures: Vec<String>,
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
    eager_suits: BTreeSet<String>,
    indexed_suits: BTreeMap<String, std::result::Result<String, String>>,
    indexed_catalogs: BTreeMap<String, IndexedCatalog>,
    indexed_suit_catalogs: BTreeMap<String, IndexedSuitCatalog>,
    indexed_suit_head_bones: BTreeMap<String, BTreeMap<String, Vec<Vec<String>>>>,
    head_suits: BTreeSet<String>,
    head_mesh_bones: BTreeMap<String, Vec<Vec<String>>>,
    suit_errors: BTreeMap<String, String>,
    suit_table: SuitTable,
    textures: Vec<String>,
}

/// Exported startup skeleton state from [`AvatarRegistry::export_skeletons`].
struct ExportedSkeletons {
    loaded: BTreeMap<String, Skeleton>,
    indexed: BTreeMap<String, std::result::Result<String, String>>,
    catalogs: BTreeMap<String, IndexedCatalog>,
}

impl AvatarRegistry {
    /// Builds the startup avatar registry for `install`.
    pub fn load(install: &Install) -> Result<Self> {
        let paths = mount_avatar_paths(install, false)?;
        let mut skeletons: BTreeMap<String, (usize, SkeletonEntry)> = BTreeMap::new();
        let mut first_skeleton_error: Option<(usize, String)> = None;
        let mut suits = BTreeSet::new();
        let mut suit_table = SuitTable::default();
        let mut head_bindings: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut suit_errors = BTreeMap::new();
        let mut step = 0usize;

        // 0x59f844: all eager `.cmx` catalogs first, then all eager `.bcf`.
        for (_, _, source) in resources_with_dir(&paths.files, &paths.appearance, b".cmx")? {
            let location = source.location(&install.root);
            let bytes = source.read()?;
            if let Err((blocks_skeletons, error)) =
                read_cmx_catalog(&bytes, &mut suits, &mut suit_table, &mut head_bindings)
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
            if let Err(error) = read_bcf_appearances(
                &mut binary,
                &mut suits,
                Some(&mut suit_table),
                None,
                &mut head_bindings,
            ) {
                suit_errors.insert(location, error.to_string());
            }
        }

        // Snapshot eager `.cmx`/`.bcf` appearances before scanning `.ndx`
        // files (`0x59f844`): already-loaded eager appearances cannot be
        // overwritten by later `.ndx` catalog loads (`0x59f34c`).
        let eager_suits = suits.clone();

        // 0x584562 → 0x5a0212 → 0x59fc48: `.ndx` catalogs in indexed paths.
        // Keep the first appearance registration (0x59fe53) and export each
        // referenced `.bcf` catalog so runtime lookups (`0x5a044d` → `0x5a38da`
        // → `0x59f35f`) can fill unloaded sibling entries in call order.
        let mut indexed_suit_entries: BTreeMap<String, (String, String, Vec<u8>)> = BTreeMap::new();
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
                            if !suit_table.contains(&entry.name) {
                                suit_table.insert_name(entry.name.clone());
                            }
                            let name = decode(&entry.name, 1);
                            if !eager_suits.contains(&name)
                                && !indexed_suit_entries.contains_key(&name)
                            {
                                let mut catalog_bcf = entry.catalog;
                                catalog_bcf.extend_from_slice(b".bcf");
                                indexed_suit_entries.insert(
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

        // Decode every `.bcf` catalog referenced by EITHER an `.ndx` appearance
        // OR an `.ndx` skeleton ONCE (keyed by resolved resource location) so
        // runtime lazy loads (`0x5a0384` / `0x5a044d` → `0x59e60e` → `0x59f087`)
        // populate BOTH skeletons and appearances from the same catalog.
        let mut mesh_cache: BTreeMap<String, Option<Vec<String>>> = BTreeMap::new();
        let mut head_mesh_bones: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
        let mut indexed_suits: BTreeMap<String, std::result::Result<String, String>> =
            BTreeMap::new();
        let mut indexed_catalogs: BTreeMap<String, IndexedCatalog> = BTreeMap::new();
        let mut indexed_suit_catalogs: BTreeMap<String, IndexedSuitCatalog> = BTreeMap::new();
        let mut indexed_suit_head_bones: BTreeMap<String, BTreeMap<String, Vec<Vec<String>>>> =
            BTreeMap::new();
        let mut by_catalog: BTreeMap<String, (Resource, Vec<String>, bool)> = BTreeMap::new();
        for (name, (ndx_location, directory, catalog_bcf)) in indexed_suit_entries {
            match paths.files.lookup(&directory, &catalog_bcf) {
                Ok(Some(resource)) => {
                    let location = resource.location(&install.root);
                    indexed_suits.insert(name.clone(), Ok(location.clone()));
                    let entry = by_catalog
                        .entry(location)
                        .or_insert_with(|| (resource, Vec::new(), true));
                    entry.1.push(name);
                    entry.2 = true;
                }
                Ok(None) => {
                    let catalog_name = decode(&catalog_bcf, 1);
                    indexed_suits.insert(
                        name.clone(),
                        Err(format!(
                            "0x5a38da: {ndx_location} points appearance {name:?} to \
                             {directory}/{catalog_name}, which is missing"
                        )),
                    );
                }
                Err(error) => {
                    let reason = error.to_string();
                    suit_errors.insert(ndx_location, reason.clone());
                    indexed_suits.insert(name, Err(reason));
                }
            }
        }
        for (_, entry) in skeletons.values() {
            if let SkeletonEntry::Indexed {
                directory,
                catalog_bcf,
                ..
            } = entry
                && let Ok(Some(resource)) = paths.files.lookup(directory, catalog_bcf)
            {
                let location = resource.location(&install.root);
                by_catalog
                    .entry(location)
                    .or_insert_with(|| (resource, Vec::new(), false));
            }
        }
        for (location, (resource, names, has_suit_refs)) in by_catalog {
            let bytes = match resource.read() {
                Ok(bytes) => bytes,
                Err(error) => {
                    let reason = error.to_string();
                    if has_suit_refs {
                        suit_errors.insert(location.clone(), reason.clone());
                    }
                    indexed_catalogs.insert(location.clone(), (Vec::new(), Some(reason.clone())));
                    indexed_suit_catalogs.insert(location, (Vec::new(), Some(reason)));
                    continue;
                }
            };
            let mut binary = Binary::new(&bytes);
            let (decoded_skels, skeleton_error) = read_bcf_skeletons(&mut binary);
            let all_scales: Vec<Vec<i32>> =
                decoded_skels.iter().map(|(_, s, _)| s.clone()).collect();
            let catalog_skels = decoded_skels
                .into_iter()
                .enumerate()
                .map(|(place, (skel_name, _, bones))| Skeleton {
                    name: skel_name,
                    file: location.clone(),
                    place,
                    scales: all_scales.clone(),
                    bones,
                })
                .collect();
            indexed_catalogs.insert(
                location.clone(),
                (
                    catalog_skels,
                    skeleton_error.as_ref().map(ToString::to_string),
                ),
            );
            if let Some(error) = skeleton_error {
                let reason = error.to_string();
                if has_suit_refs {
                    suit_errors.insert(location.clone(), reason.clone());
                }
                indexed_suit_catalogs.insert(location, (Vec::new(), Some(reason)));
                continue;
            }
            let mut catalog_suits = BTreeSet::new();
            let mut catalog_suit_order = Vec::new();
            let mut catalog_head_bindings: BTreeMap<String, Vec<String>> = BTreeMap::new();
            let app_error = match read_bcf_appearances(
                &mut binary,
                &mut catalog_suits,
                None,
                Some(&mut catalog_suit_order),
                &mut catalog_head_bindings,
            ) {
                Ok(()) => None,
                Err(error) => {
                    let reason = error.to_string();
                    if has_suit_refs {
                        suit_errors.insert(location.clone(), reason.clone());
                    }
                    Some(reason)
                }
            };
            let mut catalog_heads = Vec::with_capacity(catalog_suit_order.len());
            let mut catalog_head_bones: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
            for suit_name in &catalog_suit_order {
                let valid_meshes: Vec<Vec<String>> = catalog_head_bindings
                    .get(suit_name)
                    .map(|meshes| {
                        meshes
                            .iter()
                            .filter_map(|mesh| valid_mesh_bones(&paths, mesh, &mut mesh_cache))
                            .collect()
                    })
                    .unwrap_or_default();
                let has_head = !valid_meshes.is_empty();
                if has_head {
                    catalog_head_bones.insert(suit_name.clone(), valid_meshes);
                }
                catalog_heads.push((suit_name.clone(), has_head));
            }
            if !catalog_head_bones.is_empty() {
                indexed_suit_head_bones.insert(location.clone(), catalog_head_bones);
            }
            indexed_suit_catalogs.insert(location, (catalog_heads, app_error));
            for name in names {
                if catalog_suits.contains(&name)
                    && suits.insert(name.clone())
                    && let Some(meshes) = catalog_head_bindings.remove(&name)
                {
                    head_bindings.insert(name, meshes);
                }
            }
        }

        // 0x40895a: collect all BMP then TGA texture stems in startup texture
        // search paths (`paths.texture_paths`), truncated to 63 bytes.
        let mut textures = Vec::new();
        for extension in [b".bmp", b".tga"] {
            for (_, mut name, _) in
                resources_with_dir(&paths.files, &paths.texture_paths, extension)?
            {
                name.truncate(63);
                textures.push(decode(&name, 1));
            }
        }

        // 0x595629 -> 0x59e1b6 -> 0x59e32a: a "HEAD" binding attaches only
        // when its mesh exists and decodes as `<mesh>.bmf` (0x59a932), or
        // `<mesh>.skn` (0x59a629) when no `.bmf` exists, and every bone named
        // by the mesh exists in the target skeleton (`0x599f8a..0x59a01c`).
        let mut head_suits = BTreeSet::new();
        for (suit_name, meshes) in head_bindings {
            let valid_meshes: Vec<Vec<String>> = meshes
                .iter()
                .filter_map(|mesh| valid_mesh_bones(&paths, mesh, &mut mesh_cache))
                .collect();
            if !valid_meshes.is_empty() {
                head_suits.insert(suit_name.clone());
                head_mesh_bones.insert(suit_name, valid_meshes);
            }
        }

        Ok(Self {
            root: install.root.clone(),
            paths,
            skeletons,
            first_skeleton_error,
            suits,
            eager_suits,
            indexed_suits,
            indexed_catalogs,
            indexed_suit_catalogs,
            indexed_suit_head_bones,
            head_suits,
            head_mesh_bones,
            suit_errors,
            suit_table,
            textures,
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

    /// Exports the startup registry's skeleton state for runtime lookup
    /// (`0x5a0384`, `0x59f18c`–`0x59f19f`, `0x5a389b`):
    /// - `loaded`: every already-`Loaded` skeleton (eager `.bcf` catalogs plus
    ///   any `.ndx` catalogs and their siblings already loaded for the
    ///   requested person GUIDs);
    /// - `indexed`: each still-unloaded `.ndx` entry mapped to its catalog
    ///   location (`Ok(loc)`) or blocking error (`Err(reason)`);
    /// - `catalogs`: decoded skeletons (and optional error) for each catalog
    ///   referenced by `indexed`, so runtime lookups fill unloaded sibling
    ///   entries in exact runtime call order.
    fn export_skeletons(&self) -> ExportedSkeletons {
        let mut loaded = BTreeMap::new();
        let mut indexed = BTreeMap::new();
        let catalogs = self.indexed_catalogs.clone();
        for (name, (reg_step, entry)) in &self.skeletons {
            if let Some((err_step, reason)) = &self.first_skeleton_error
                && *reg_step >= *err_step
            {
                indexed.insert(
                    name.clone(),
                    Err(format!(
                        "0x5a0384: {reason}, so whether it overrides {name:?} is unknown"
                    )),
                );
                continue;
            }
            match entry {
                SkeletonEntry::Loaded {
                    file,
                    place,
                    scales,
                    bones,
                } => {
                    loaded.insert(
                        name.clone(),
                        Skeleton {
                            name: name.clone(),
                            file: file.clone(),
                            place: *place,
                            scales: scales.clone(),
                            bones: bones.clone(),
                        },
                    );
                }
                SkeletonEntry::Failed(reason) => {
                    indexed.insert(name.clone(), Err(reason.clone()));
                }
                SkeletonEntry::Indexed {
                    ndx_location,
                    directory,
                    catalog_bcf,
                } => match self.paths.files.lookup(directory, catalog_bcf) {
                    Ok(Some(resource)) => {
                        let location = resource.location(&self.root);
                        indexed.insert(name.clone(), Ok(location));
                    }
                    Ok(None) => {
                        let catalog_name = decode(catalog_bcf, 1);
                        indexed.insert(
                            name.clone(),
                            Err(format!(
                                "0x5a3810: {ndx_location} points skeleton {name:?} to \
                                 {directory}/{catalog_name}, which is missing"
                            )),
                        );
                    }
                    Err(error) => {
                        indexed.insert(name.clone(), Err(error.to_string()));
                    }
                },
            }
        }
        ExportedSkeletons {
            loaded,
            indexed,
            catalogs,
        }
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

    /// As [`People::load`], but a person type whose skeleton name is missing
    /// or whose skeleton cannot be resolved is left out of `skeletons` instead
    /// of failing the whole load, while its [`Profile`] and `portrait_head`
    /// metadata are retained so runtime-made clones with a valid skeleton
    /// override can still be constructed and rendered. Returns the types left
    /// out of `skeletons`, by GUID, with the reason.
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
            if object.kind != 2 || profiles.contains_key(&guid) {
                continue;
            }
            let resource = &index.resources[object.resource];
            let body = body_strings(index, resource, object.chunk)?;
            let row = |n: usize| body.get(n).cloned();
            let suit_name = |n: usize| {
                body.get(n)
                    .map(|raw| {
                        raw.split(';')
                            .next()
                            .unwrap_or(raw)
                            .split(',')
                            .next()
                            .unwrap_or(raw)
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
            let suit_rows: BTreeMap<usize, String> = body
                .iter()
                .enumerate()
                .skip(3)
                .filter(|(_, text)| !text.is_empty())
                .map(|(i, text)| (i, text.clone()))
                .collect();
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
                    suit_rows,
                },
            );
            let location = resource.location(&index.root);
            if let Some(name) = body.first().filter(|name| !name.is_empty()).cloned() {
                names.insert(guid, name);
            } else {
                unresolved.insert(
                    guid,
                    format!("{location}: the person's body strings name no skeleton"),
                );
            }
        }
        // Export startup skeletons BEFORE per-GUID resolution so `.ndx`
        // skeletons stay unloaded in `indexed_skeletons` / `indexed_catalogs`
        // rather than leaking into `skeletons_by_name` or overwriting sibling
        // `.ndx` entries in GUID order (`0x5a0384`, `0x59f18c`–`0x59f19f`).
        let registry = AvatarRegistry::load(install)?;
        let ExportedSkeletons {
            loaded: skeletons_by_name,
            indexed: indexed_skeletons,
            catalogs: indexed_catalogs,
        } = registry.export_skeletons();
        let mut skeletons = BTreeMap::new();
        for (&guid, name) in &names {
            if let Some(skeleton) = skeletons_by_name.get(name) {
                skeletons.insert(guid, skeleton.clone());
            } else if let Some(entry) = indexed_skeletons.get(name) {
                match entry {
                    Ok(loc) => {
                        let (items, decode_error) = &indexed_catalogs[loc];
                        if let Some(skeleton) = items.iter().find(|s| s.name == *name) {
                            skeletons.insert(guid, skeleton.clone());
                        } else {
                            let reason = if let Some(error) = decode_error {
                                format!("0x5a3810: the skeletons of {loc} do not decode ({error})")
                            } else {
                                format!("0x5a03c0: {loc} does not define skeleton {name:?}")
                            };
                            unresolved.insert(guid, reason);
                        }
                    }
                    Err(reason) => {
                        unresolved.insert(guid, reason.clone());
                    }
                }
            } else {
                let reason = match &registry.first_skeleton_error {
                    Some((_, reason)) => {
                        format!("0x5a0384: {reason}, so whether it defines {name:?} is unknown")
                    }
                    None => format!("0x5a03c0: no skeleton file defines the skeleton {name:?}"),
                };
                unresolved.insert(guid, reason);
            }
        }
        let mut portrait_head = BTreeMap::new();
        for (guid, suits) in default_suits {
            let skel = skeletons.get(&guid);
            if suits.iter().any(|s| {
                suit_head_matches_skeleton_parts(
                    &registry.head_mesh_bones,
                    s,
                    registry.head_suits.contains(s),
                    skel,
                )
            }) {
                portrait_head.insert(guid, true);
            } else if registry.suit_errors.is_empty() {
                portrait_head.insert(guid, false);
            }
        }
        Ok((
            Self {
                skeletons,
                skeletons_by_name,
                indexed_skeletons,
                indexed_catalogs,
                skeleton_names: names,
                profiles,
                suits: registry.suits,
                eager_suits: registry.eager_suits,
                indexed_suits: registry.indexed_suits,
                indexed_suit_catalogs: registry.indexed_suit_catalogs,
                indexed_suit_head_bones: registry.indexed_suit_head_bones,
                suit_errors: registry.suit_errors,
                head_suits: registry.head_suits,
                head_mesh_bones: registry.head_mesh_bones,
                portrait_head,
                suit_table: registry.suit_table,
                textures: registry.textures,
            },
            unresolved,
        ))
    }

    /// Whether `suit` has a `"HEAD"` mesh binding whose required bones all
    /// exist in `skeleton` (`0x599f8a..0x59a01c`). When `head_mesh_bones` has
    /// no entry for `suit` or `skeleton` has no decoded bone names (synthetic
    /// fixtures), falls back to `has_head`.
    pub fn suit_head_matches_skeleton(
        &self,
        suit: &str,
        has_head: bool,
        skeleton: Option<&Skeleton>,
    ) -> bool {
        self.suit_head_matches_skeleton_with_bones(
            suit,
            has_head,
            self.head_mesh_bones.get(suit).map(Vec::as_slice),
            skeleton,
        )
    }

    /// Like [`People::suit_head_matches_skeleton`], using an explicit
    /// catalog-specific `mesh_bones` override when available (for runtime-loaded
    /// `.ndx` catalogs where first registration wins at `0x59f34c..0x59f35f`).
    pub fn suit_head_matches_skeleton_with_bones(
        &self,
        _suit: &str,
        has_head: bool,
        mesh_bones: Option<&[Vec<String>]>,
        skeleton: Option<&Skeleton>,
    ) -> bool {
        suit_head_matches_skeleton_bones(mesh_bones, has_head, skeleton)
    }
}

fn suit_head_matches_skeleton_parts(
    head_mesh_bones: &BTreeMap<String, Vec<Vec<String>>>,
    suit: &str,
    has_head: bool,
    skeleton: Option<&Skeleton>,
) -> bool {
    suit_head_matches_skeleton_bones(
        head_mesh_bones.get(suit).map(Vec::as_slice),
        has_head,
        skeleton,
    )
}

fn suit_head_matches_skeleton_bones(
    mesh_bones: Option<&[Vec<String>]>,
    has_head: bool,
    skeleton: Option<&Skeleton>,
) -> bool {
    if !has_head {
        return false;
    }
    let Some(skel) = skeleton else {
        return true;
    };
    if skel.bones.is_empty() {
        return !skel.scales.get(skel.place).is_some_and(Vec::is_empty);
    }
    if !skel.bones.iter().any(|b| b.name == "HEAD") {
        return false;
    }
    let Some(meshes) = mesh_bones else {
        return true;
    };
    meshes.iter().any(|req_bones| {
        req_bones
            .iter()
            .all(|req| skel.bones.iter().any(|b| b.name == *req))
    })
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
    body.first()
        .filter(|name| !name.is_empty())
        .cloned()
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

/// US-English entries in the native loader's order (`0x470002`–`0x470049`):
/// when a later US-English run follows another language, it is inserted ahead
/// of the earlier run.
fn loaded_english_rows(entries: &[sims_formats::strings::TextEntry]) -> Vec<String> {
    let mut loaded: Vec<String> = Vec::new();
    let mut run = None;
    let mut place = 0;
    for entry in entries {
        if run != Some(entry.language) {
            run = Some(entry.language);
            place = 0;
        }
        if entry.language == 1 {
            loaded.insert(place.min(loaded.len()), entry.text.clone());
            place += 1;
        }
    }
    loaded
}

/// Decodes a body `STR#` chunk as the native body loader (`0x587b65` →
/// `0x46feac(0, 1)`) does:
/// - uses [`StringTable::parse_chunk`] with `flags` (honoring flag `0x10` for
///   little-endian length-prefixed tables at `0x4700d7`–`0x47014c`);
/// - for length-prefixed strings, truncates at the first embedded NUL
///   (`0x470404`–`0x470434`) and applies the `0xa1 -> 'o'` / `0xd5 -> '\''`
///   byte mapping (`0x4700f5`–`0x47010f`);
/// - orders version `-3` US-English entries with later runs inserted first
///   (`0x470002`–`0x470049`);
/// - when a version `-3` table has entries (`count > 0`) and none matched US
///   English (`0x46fae6 == 0`), runs the native byte-by-byte English rescan
///   (`0x47006b`–`0x4700d0`);
/// - pads short tables to 35 rows (`0x23`) with empty strings
///   (`0x587b7b`–`0x587ba5`).
fn decode_body_string_table(data: &[u8], flags: u16, label: &str) -> Result<Vec<String>> {
    const BODY_ROWS: usize = 35;
    let parsed = StringTable::parse_chunk(data, flags).map_err(|e| Error::format(label, e))?;
    let mut loaded = if parsed.format == sims_formats::strings::Format::MacResource {
        parsed
            .entries
            .iter()
            .map(|entry| {
                let before_nul = entry
                    .raw_text
                    .split(|&b| b == 0)
                    .next()
                    .unwrap_or(&entry.raw_text);
                let mapped: Vec<u8> = before_nul
                    .iter()
                    .map(|&b| match b {
                        0xa1 => b'o',
                        0xd5 => b'\'',
                        _ => b,
                    })
                    .collect();
                decode(&mapped, 1)
            })
            .collect()
    } else {
        let mut loaded = loaded_english_rows(&parsed.entries);
        if parsed.format == sims_formats::strings::Format::Localized
            && loaded.is_empty()
            && !parsed.entries.is_empty()
        {
            let mut pos = 4usize;
            for _ in 0..parsed.entries.len() {
                let Some(&b) = data.get(pos) else {
                    return Err(Error::Unsupported(format!(
                        "{label}: English rescan (0x47006b..0x4700d0) reads past chunk end"
                    )));
                };
                pos += 1;
                if b == 1 {
                    let Some(end1) = data[pos..].iter().position(|&c| c == 0) else {
                        return Err(Error::Unsupported(format!(
                            "{label}: unterminated string in English rescan (0x4700a3)"
                        )));
                    };
                    loaded.push(decode(&data[pos..pos + end1], 1));
                    pos += end1 + 1;
                    let Some(end2) = data[pos..].iter().position(|&c| c == 0) else {
                        return Err(Error::Unsupported(format!(
                            "{label}: unterminated comment in English rescan (0x4700bf)"
                        )));
                    };
                    pos += end2 + 1;
                }
            }
        }
        loaded
    };
    if loaded.len() < BODY_ROWS {
        loaded.resize(BODY_ROWS, String::new());
    }
    Ok(loaded)
}

/// The body strings of the person definition `chunk` of a character
/// file, decoded as `0x587b65` → `0x46feac(0, 1)`.
fn body_strings(index: &Index, resource: &Resource, chunk: u16) -> Result<Vec<String>> {
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
    decode_body_string_table(
        iff.data(strings),
        iff.resource_flags(strings),
        &format!("{location} STR# #{table}"),
    )
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

/// Decodes `<mesh>.bmf` (`0x59e36d` → `0x59a932`), or `<mesh>.skn`
/// (`0x59e403` → `0x59a629`) ONLY when no `<mesh>.bmf` is found across the
/// avatar search paths (`0x59e10b`–`0x59e134`), returning the mesh's required
/// bone names (`0x599f8a..0x59a01c`). Once a `.bmf` file opens, a decode
/// failure at `0x59e3be` returns `None` (`0x59e3e1`) without falling back to
/// `.skn`.
fn valid_mesh_bones(
    paths: &AvatarPaths,
    mesh: &str,
    cache: &mut BTreeMap<String, Option<Vec<String>>>,
) -> Option<Vec<String>> {
    if mesh.is_empty() {
        return None;
    }
    let key = mesh.to_ascii_lowercase();
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let base = encode_name(&key).unwrap_or_else(|| key.as_bytes().to_vec());
    let mut bmf_name = base.clone();
    bmf_name.extend_from_slice(b".bmf");
    let mut found_bmf = false;
    let mut bones = None;
    for dir in paths.appearance.iter().chain(&paths.indexed) {
        if let Ok(Some(resource)) = paths.files.lookup(dir, &bmf_name) {
            found_bmf = true;
            if let Ok(bytes) = resource.read()
                && let Ok(decoded) = read_bmf(&bytes)
            {
                bones = Some(decoded.bones);
            }
            break;
        }
    }
    if !found_bmf {
        let mut skn_name = base;
        skn_name.extend_from_slice(b".skn");
        for dir in paths.appearance.iter().chain(&paths.indexed) {
            if let Ok(Some(resource)) = paths.files.lookup(dir, &skn_name) {
                if let Ok(bytes) = resource.read()
                    && let Ok(decoded) = read_skn(&bytes)
                {
                    bones = Some(decoded.bones);
                }
                break;
            }
        }
    }
    cache.insert(key, bones.clone());
    bones
}

/// Reads a text `.cmx` catalog's skeleton count and appearances (`0x59e746`,
/// `0x594d73`). First registration wins for duplicate appearance names
/// (`0x59ea68`/`0x59eb11`). Returns `Err((true, e))` if the header, skeleton
/// count or a non-empty skeleton section fails (so skeleton lookup must fail
/// loudly), and `Err((false, e))` if an appearance record fails after 0
/// skeletons.
fn read_cmx_catalog(
    bytes: &[u8],
    suits: &mut BTreeSet<String>,
    suit_table: &mut SuitTable,
    head_bindings: &mut BTreeMap<String, Vec<String>>,
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
        if let Asset::Appearance(appearance) = cmx.read(Kind::Appearance).map_err(|e| (false, e))?
            && suits.insert(appearance.name.clone())
        {
            let key = encode_name(&appearance.name)
                .unwrap_or_else(|| appearance.name.as_bytes().to_vec());
            if !suit_table.contains(&key) {
                suit_table.insert_name(key);
            }
            let meshes: Vec<String> = appearance
                .bindings
                .into_iter()
                .filter(|b| b.bone == "HEAD")
                .map(|b| b.mesh)
                .collect();
            if !meshes.is_empty() {
                head_bindings.insert(appearance.name, meshes);
            }
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
/// record (`0x595140`, `0x5950f5` lowercasing each suit name). First
/// registration wins for duplicate appearance names (`0x59f2e6`/`0x59f34c`).
/// A short appearance count (`Ok(None)`) is treated as a read failure
/// (`0x59f1c1`).
fn read_bcf_appearances(
    binary: &mut Binary<'_>,
    suits: &mut BTreeSet<String>,
    mut suit_table: Option<&mut SuitTable>,
    mut order: Option<&mut Vec<String>>,
    head_bindings: &mut BTreeMap<String, Vec<String>>,
) -> sims_formats::Result<()> {
    let Some(count) = binary.count(Kind::Appearance)? else {
        return Err(sims_formats::Error::new(
            binary.position(),
            "missing BCF appearance count",
        ));
    };
    for _ in 0..count {
        if let Asset::Appearance(appearance) = binary.read(Kind::Appearance)?
            && suits.insert(appearance.name.clone())
        {
            if let Some(table) = suit_table.as_deref_mut() {
                let key = encode_name(&appearance.name)
                    .unwrap_or_else(|| appearance.name.as_bytes().to_vec());
                if !table.contains(&key) {
                    table.insert_name(key);
                }
            }
            if let Some(order) = order.as_deref_mut() {
                order.push(appearance.name.clone());
            }
            let meshes: Vec<String> = appearance
                .bindings
                .into_iter()
                .filter(|b| b.bone == "HEAD")
                .map(|b| b.mesh)
                .collect();
            if !meshes.is_empty() {
                head_bindings.insert(appearance.name, meshes);
            }
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
        let mut suit_table = SuitTable::default();
        let mut head_bindings = BTreeMap::new();
        read_cmx_catalog(cmx, &mut suits, &mut suit_table, &mut head_bindings).unwrap();
        assert_eq!(
            suits,
            ["b001_body", "c001_exact", "c002_lower"]
                .map(String::from)
                .into()
        );
        assert_eq!(
            head_bindings,
            BTreeMap::from([(
                "c001_exact".to_string(),
                vec!["xskin-c001-HEAD-HEAD".to_string()]
            )])
        );
    }

    #[test]
    fn duplicate_appearances_missing_meshes_empty_rig_and_indexed_catalogs() {
        let dir = ScratchDir::new("round14-people");
        let bcf_with_apps = |skels: &[(&str, i32)], apps: &[(&str, &str, &str)]| {
            let push_str = |out: &mut Vec<u8>, s: &str| {
                out.push(s.len() as u8);
                out.extend_from_slice(s.as_bytes());
            };
            let mut bytes = (skels.len() as u32).to_le_bytes().to_vec();
            for &(name, scale) in skels {
                push_str(&mut bytes, name);
                bytes.extend(1u32.to_le_bytes());
                push_str(&mut bytes, "HEAD");
                push_str(&mut bytes, "NULL");
                bytes.extend(0u32.to_le_bytes());
                for value in [0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0] {
                    bytes.extend(value.to_le_bytes());
                }
                bytes.extend([0u8; 12]);
                bytes.extend(scale.to_le_bytes());
                bytes.extend(0.0f32.to_le_bytes());
            }
            bytes.extend((apps.len() as u32).to_le_bytes());
            for &(name, bone, mesh) in apps {
                push_str(&mut bytes, name);
                bytes.extend([0u8; 8]);
                bytes.extend(1u32.to_le_bytes());
                push_str(&mut bytes, bone);
                push_str(&mut bytes, mesh);
                bytes.extend([0u8; 8]);
            }
            bytes
        };
        let valid_skn = b"xskin-valid\r\n\
            tex\r\n\
            1\r\n\
            HEAD\r\n\
            1\r\n\
            0 1 2\r\n\
            1\r\n\
            0 0 3 0 0\r\n\
            3\r\n\
            0 0\r\n\
            1 0\r\n\
            0 1\r\n\
            0\r\n\
            3\r\n\
            0 0 0 0 0 1\r\n\
            1 0 0 0 0 1\r\n\
            0 1 0 0 0 1\r\n";
        dir.write(
            "GameData/Skins/test.bcf",
            &bcf_with_apps(
                &[("adult", 1)],
                &[
                    ("dup_pelvis_first", "PELVIS", "xskin-valid"),
                    ("dup_pelvis_first", "HEAD", "xskin-valid"),
                    ("missing_mesh_head", "HEAD", "xskin-absent"),
                    ("corrupt_bmf_fallback", "HEAD", "xskin-fallback"),
                    ("good_head", "HEAD", "xskin-valid"),
                ],
            ),
        );
        dir.write("GameData/Skins/xskin-valid.skn", valid_skn);
        dir.write("GameData/Skins/xskin-fallback.bmf", b"not-a-valid-bmf");
        dir.write("GameData/Skins/xskin-fallback.skn", valid_skn);
        // Indexed alpha.bcf also defines sibling beta(scale 3) vs beta.bcf beta(scale 4).
        dir.write(
            "GameData/Animation/Animation.ndx",
            &ndx(&[("alpha", "alpha"), ("beta", "beta")], &[]),
        );
        dir.write(
            "GameData/Animation/alpha.bcf",
            &bcf_with_apps(&[("alpha", 2), ("beta", 3)], &[]),
        );
        dir.write(
            "GameData/Animation/beta.bcf",
            &bcf_with_apps(&[("beta", 4)], &[]),
        );

        let iff_person = |guid: u32, rows: &[&[u8]]| {
            let mut def = vec![0u8; 216];
            def[..4].copy_from_slice(&138u32.to_le_bytes());
            def[18..20].copy_from_slice(&2u16.to_le_bytes());
            def[28..32].copy_from_slice(&guid.to_le_bytes());
            def[38..40].copy_from_slice(&200u16.to_le_bytes());
            let mut data = (rows.len() as u16).to_le_bytes().to_vec();
            for row in rows {
                data.push(row.len() as u8);
                data.extend_from_slice(row);
            }
            sims_formats::iff::write(&[
                sims_formats::iff::ChunkData {
                    tag: *b"OBJD",
                    id: 128,
                    flags: 0,
                    label: String::new(),
                    data: def,
                },
                sims_formats::iff::ChunkData {
                    tag: *b"STR#",
                    id: 200,
                    flags: 0x10,
                    label: String::new(),
                    data,
                },
            ])
            .unwrap()
        };
        dir.write(
            "GameData/Objects/missing.iff",
            &iff_person(1, &[b"missing", b"", b"good_head,HEAD-HEAD=tex"]),
        );
        dir.write(
            "GameData/Objects/empty.iff",
            &iff_person(2, &[b"", b"", b"good_head,HEAD-HEAD=tex"]),
        );

        let install = dir.install();
        let resources = vec![
            Resource {
                mount: 0,
                path: dir.0.join("GameData/Objects/missing.iff"),
                member: None,
                inline: None,
            },
            Resource {
                mount: 0,
                path: dir.0.join("GameData/Objects/empty.iff"),
                member: None,
                inline: None,
            },
        ];
        let index = Index {
            root: dir.0.clone(),
            mounts: vec![],
            resources,
            diagnostics: vec![],
        };
        let catalog = Catalog {
            global: 0,
            objects: vec![
                crate::Object {
                    guid: 1,
                    resource: 0,
                    chunk: 128,
                    kind: 2,
                    master: 0,
                },
                crate::Object {
                    guid: 2,
                    resource: 1,
                    chunk: 128,
                    kind: 2,
                    master: 0,
                },
            ],
            by_guid: BTreeMap::from([(1, 0), (2, 1)]),
            semi_globals: BTreeMap::new(),
            diagnostics: vec![],
        };

        // People::load_each succeeds for both missing-rig and empty-row-0 types,
        // retaining Profile and portrait_head for both!
        let (people, unresolved) = People::load_each(&install, &index, &catalog, [1, 2]).unwrap();
        assert!(unresolved.contains_key(&1));
        assert!(unresolved.contains_key(&2));
        assert!(people.profiles.contains_key(&1));
        assert!(people.profiles.contains_key(&2));
        assert_eq!(people.portrait_head.get(&1), Some(&true));
        assert_eq!(people.portrait_head.get(&2), Some(&true));

        // Duplicate appearance (first PELVIS wins), missing mesh, and corrupt
        // BMF (0x59e3be -> 0x59e3e1 does NOT fall back to SKN) are excluded;
        // valid SKN with no BMF is included.
        assert!(!people.head_suits.contains("dup_pelvis_first"));
        assert!(!people.head_suits.contains("missing_mesh_head"));
        assert!(!people.head_suits.contains("corrupt_bmf_fallback"));
        assert!(people.head_suits.contains("good_head"));
        assert_eq!(
            people.head_mesh_bones.get("good_head"),
            Some(&vec![vec!["HEAD".to_owned()]])
        );

        // Indexed alpha and beta stay unloaded in indexed_skeletons / indexed_catalogs,
        // preserving alpha.bcf's sibling beta(scale 3) for runtime load order.
        assert!(!people.skeletons_by_name.contains_key("alpha"));
        assert!(!people.skeletons_by_name.contains_key("beta"));
        let alpha_loc = people.indexed_skeletons["alpha"].as_ref().unwrap();
        assert!(
            people.indexed_catalogs[alpha_loc]
                .0
                .iter()
                .any(|s| s.name == "beta" && s.scales[s.place] == vec![3])
        );
    }

    #[test]
    fn loaded_english_rows_inserts_later_us_english_run_first() {
        let entry = |language: u8, text: &str| sims_formats::strings::TextEntry {
            language,
            text: text.to_owned(),
            comment: String::new(),
            raw_text: text.as_bytes().to_vec(),
            raw_comment: Vec::new(),
        };
        let rows = loaded_english_rows(&[
            entry(1, "kat"),
            entry(1, "old_body"),
            entry(2, "de_filler"),
            entry(1, "human-custom"),
        ]);
        assert_eq!(rows, vec!["human-custom", "kat", "old_body"]);
    }

    #[test]
    fn decode_body_string_table_supports_le_length_prefixed_and_english_rescan() {
        // 35-row little-endian length-prefixed STR# with flag 0x10
        // (0x4700d7..0x47014c): header `23 00` is 35 rows, not 8960.
        let mut le_pascal = Vec::from(35u16.to_le_bytes());
        for i in 0..35usize {
            let s = if i == 0 {
                "adult".to_string()
            } else {
                format!("row{i}")
            };
            le_pascal.push(s.len() as u8);
            le_pascal.extend_from_slice(s.as_bytes());
        }
        let rows = decode_body_string_table(&le_pascal, 0x10, "test").unwrap();
        assert_eq!(rows.len(), 35);
        assert_eq!(rows[0], "adult");
        assert_eq!(rows[34], "row34");

        // Length-prefixed string with embedded NUL (0x470404..0x470434):
        // bytes after the first NUL are dropped, and short tables are padded
        // to 35 empty rows (0x587b7b..0x587ba5).
        let mut nul_pascal = Vec::from(1u16.to_le_bytes());
        nul_pascal.push(7);
        nul_pascal.extend_from_slice(b"adult\0x");
        let rows = decode_body_string_table(&nul_pascal, 0x10, "nul").unwrap();
        assert_eq!(rows.len(), 35);
        assert_eq!(rows[0], "adult");
        assert_eq!(rows[13], "");
        assert_eq!(rows[14], "");

        // Version -3 table with no US-English entries (language 3 only) whose
        // bytes include an embedded 0x01 byte: 0x587b65 -> 0x47006b..0x4700d0
        // rescans byte-by-byte and extracts the string following 0x01, then
        // pads to 35 rows.
        let mut rescan = Vec::new();
        rescan.extend_from_slice(&(-3i16).to_le_bytes());
        rescan.extend_from_slice(&2u16.to_le_bytes());
        rescan.push(3); // non-US-English language byte
        rescan.extend_from_slice(b"\x01adult\0\0");
        rescan.push(3);
        rescan.extend_from_slice(b"unused\0\0");
        let rows = decode_body_string_table(&rescan, 0x10, "rescan").unwrap();
        assert_eq!(rows.len(), 35);
        assert_eq!(rows[0], "adult");
        assert_eq!(rows[13], "");
        assert_eq!(rows[14], "");
    }
}
