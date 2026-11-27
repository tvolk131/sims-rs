//! What a person wears: the appearances its saved record names, and from
//! them the meshes and skin textures it is drawn with.
//!
//! A person's record (`sims_formats::saved_objects::Person::appearance`)
//! names five appearances, each with a texture string: the body, an
//! unused pair, the left hand, the right hand and the head. Accessories
//! follow. For example Bella Goth's body is `b004fafit_01` with
//! `BODY=b004fafitmed_fatale01`.
//!
//! An appearance is a record of an appearance catalog (`<name>.cmx.bcf`,
//! or the text form `<name>.cmx`). It binds meshes to bones, such as
//! `xskin-b004fafit_01-PELVIS-BODY` to `PELVIS`. A mesh is a `.bmf` file,
//! or failing that the text form `.skn` (the original tries the binary
//! form first, 0x59e32a, as the old prototype found:
//! legacy/crates/sims-content/src/avatar.rs, `mesh_resource`).
//!
//! The texture string is `KEY=texture`. A mesh whose name contains `KEY`
//! (case matters) takes that texture: `BODY` above, `HAND` for the hands
//! (`xskin-hflo-L_HAND-HANDFL`), `HEAD-HEAD` for a head. Any other mesh
//! keeps the texture its own file names, such as the glasses of a head
//! (`xskin-c006fc_nerd-HEAD-NERDSPECS`, texture `nerdspecs`). This is the
//! old prototype's reading of the original's 0x599e87
//! (legacy/crates/sims-content/src/saved_avatar.rs, `texture_override`).

use crate::assets::Assets;
use crate::rig;
use crate::texture::{Texture, read_bmp};
use sims_formats::avatar::{
    Appearance, Catalog, Mesh, Skeleton, read_bcf, read_bmf, read_cmx, read_skn,
};
use sims_formats::saved_objects::Person;
use std::collections::BTreeMap;

/// The appearances a person wears, each with its texture string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outfit {
    pub parts: Vec<(String, String)>,
}

/// The name the original writes for an empty appearance slot (the string
/// at 0x6c0fd4): the pets' body strings name their heads so (it writes it
/// over rows 3 and 31 to 33 at 0x40a5d8), and a slot with no suit is
/// listed so (0x58fc68). No catalog holds an appearance of that name, so
/// the slot binds nothing.
pub const NO_APPEARANCE: &str = "NONE";

/// Whether an outfit names a part with `name`: not empty, nor
/// [`NO_APPEARANCE`].
pub fn names_a_part(name: &str) -> bool {
    !name.is_empty() && name != NO_APPEARANCE
}

impl Outfit {
    /// The appearances a saved person record names, in its order: body,
    /// left hand, right hand, head, then the accessories. Empty names and
    /// [`NO_APPEARANCE`] are skipped.
    pub fn from_person(person: &Person) -> Self {
        let a = &person.appearance;
        let mut parts: Vec<(String, String)> = [(0, 1), (4, 5), (6, 7), (8, 9)]
            .into_iter()
            .map(|(m, t)| (a[m].clone(), a[t].clone()))
            .collect();
        parts.extend(
            person
                .accessories
                .iter()
                .map(|x| (x.name.clone(), x.binding.clone())),
        );
        parts.retain(|(name, _)| names_a_part(name));
        Self { parts }
    }

    /// The default outfit a person type's body strings give, for a person
    /// whose record names none (a person a script made, before anything
    /// chose its clothes): the body (row 1), left hand (row 17), right
    /// hand (row 18) and head (row 2), counting from 0, each written
    /// `appearance,KEY=texture`. A row may hold a second descriptor after
    /// a `;`; the first is the everyday one. These are the rows the old prototype's
    /// port of the original's outfit choice (0x58a178) reads for a person
    /// in everyday clothes with open hands
    /// (legacy/crates/sims-sim/src/pose/appearance_selection.rs). Whether
    /// the original draws a newly made person in them is **unverified**;
    /// the simulation does not yet keep the outfit it chose. Empty names
    /// and [`NO_APPEARANCE`] are skipped.
    pub fn from_body_strings(strings: &[String]) -> Self {
        let mut parts: Vec<(String, String)> = [1, 17, 18, 2]
            .into_iter()
            .filter_map(|row| strings.get(row))
            .map(|s| body_string(s))
            .collect();
        parts.retain(|(name, _)| names_a_part(name));
        Self { parts }
    }
}

/// A body string's everyday appearance and its texture string: the part
/// before any `;`, split at its first `,` (see
/// [`Outfit::from_body_strings`]).
pub fn body_string(s: &str) -> (String, String) {
    let first = s.split_once(';').map_or(s, |(a, _)| a);
    match first.split_once(',') {
        Some((name, textures)) => (name.trim().to_owned(), textures.trim().to_owned()),
        None => (first.trim().to_owned(), String::new()),
    }
}

/// One mesh to draw, bound to the skeleton.
#[derive(Debug, Clone)]
pub struct Part {
    /// The appearance the mesh belongs to.
    pub appearance: String,
    /// The bone the appearance binds the mesh to.
    pub bone: usize,
    /// For each bone the mesh names, the skeleton's bone.
    pub bones: Vec<usize>,
    pub mesh: Mesh,
    /// The texture's name, or None when the mesh has none.
    pub texture_name: Option<String>,
    pub texture: Texture,
    /// The appearance marks the binding as censored (a part shown only
    /// with the censor off).
    pub censor: u32,
}

/// A person ready to pose: its skeleton and the meshes it wears.
#[derive(Debug, Clone)]
pub struct Body {
    pub skeleton: Skeleton,
    pub parts: Vec<Part>,
}

impl Body {
    /// Loads the skeleton named `skeleton` ([`load_skeleton`], such as
    /// `adult`) and every part of `outfit`.
    pub fn load(assets: &Assets, skeleton: &str, outfit: &Outfit) -> Result<Self, String> {
        let skeleton = load_skeleton(assets, skeleton)?;
        let mut parts = Vec::new();
        for (name, textures) in &outfit.parts {
            let appearance = load_appearance(assets, name)?;
            for binding in &appearance.bindings {
                let bone = rig::bone(&skeleton, &binding.bone).ok_or_else(|| {
                    format!(
                        "the appearance {name:?} binds to the bone {:?}, which the skeleton \
                         {:?} lacks",
                        binding.bone, skeleton.name
                    )
                })?;
                let mesh = load_mesh(assets, &binding.mesh)?;
                let bones = mesh
                    .bones
                    .iter()
                    .map(|b| {
                        rig::bone(&skeleton, b).ok_or_else(|| {
                            format!("the mesh {:?} names the missing bone {b:?}", mesh.name)
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let texture_name = texture_for(&binding.mesh, textures)
                    .or_else(|| (!mesh.texture.is_empty()).then(|| mesh.texture.clone()));
                let texture = match &texture_name {
                    Some(t) => load_texture(assets, t)?,
                    None => Texture::solid([200, 200, 200, 255]),
                };
                parts.push(Part {
                    appearance: name.clone(),
                    bone,
                    bones,
                    mesh,
                    texture_name,
                    texture,
                    censor: binding.censor,
                });
            }
        }
        Ok(Self { skeleton, parts })
    }
}

/// The texture a texture string gives the mesh `mesh`: the part after the
/// first `=`, if the part before it is in the mesh's name (0x599e87).
/// None when the string gives the mesh nothing, and the mesh's own
/// texture is used.
pub fn texture_for(mesh: &str, textures: &str) -> Option<String> {
    if mesh.is_empty() {
        return None;
    }
    let (key, value) = textures.split_once('=')?;
    (mesh.contains(key) && !value.is_empty()).then(|| value.to_owned())
}

/// The skeleton named `name`, from `<name>-skeleton.cmx.bcf` or, when no
/// file has that name, `<name>_skeleton.cmx.bcf`.
///
/// The original keys skeletons by the name inside the file, not the
/// file's: the avatar registry's scan at startup (0x59f844, through the
/// reader 0x59f087) reads every skeleton file of its folders. People's
/// files are named with a hyphen; the pets' (in the shared expansion
/// archive's skeletons folder) with an underscore.
///
/// A file that cannot be read, or holds no skeleton of that name, does
/// not end the search: the next file is tried, as the registry finds the
/// name in whichever file holds it.
pub fn load_skeleton(assets: &Assets, name: &str) -> Result<Skeleton, String> {
    let files = skeleton_files(name);
    let mut tried: Vec<&String> = files.iter().filter(|f| assets.contains(f)).collect();
    if tried.is_empty() {
        // Neither is listed: the hyphened name is read as before, for
        // its error.
        tried.push(&files[0]);
    }
    named_skeleton(
        tried.into_iter().map(|file| {
            let read = assets
                .read(file)
                .and_then(|bytes| read_bcf(&bytes).map_err(|e| format!("{file}: {e}")))
                .map(|catalog| catalog.skeletons);
            (file.as_str(), read)
        }),
        name,
    )
}

/// The first skeleton named `name` (ignoring case) in `files`, each a
/// file name and its skeletons (or why they could not be read), in the
/// order they are tried. When none has it, every file's reason.
fn named_skeleton<'a>(
    files: impl IntoIterator<Item = (&'a str, Result<Vec<Skeleton>, String>)>,
    name: &str,
) -> Result<Skeleton, String> {
    let mut reasons = Vec::new();
    for (file, skeletons) in files {
        match skeletons {
            Ok(skeletons) => match skeletons
                .into_iter()
                .find(|s| s.name.eq_ignore_ascii_case(name))
            {
                Some(skeleton) => return Ok(skeleton),
                None => reasons.push(format!("{file} holds no skeleton named {name:?}")),
            },
            Err(e) => reasons.push(e),
        }
    }
    Err(reasons.join("; "))
}

/// The files that may hold the skeleton `name`, in the order they are
/// tried ([`load_skeleton`]).
fn skeleton_files(name: &str) -> [String; 2] {
    [
        format!("{name}-skeleton.cmx.bcf"),
        format!("{name}_skeleton.cmx.bcf"),
    ]
}

/// The folders the original loads appearance catalogs from, in its order
/// (0x5835ed, as the old prototype traced it:
/// legacy/crates/sims-sim/src/pose/clothing/catalog_paths.rs). The game's
/// own skins come first, then the shared expansion folders, then each
/// expansion's from the newest back, then the first expansion's.
const APPEARANCE_FOLDERS: &[&str] = &[
    "gamedata/skins",
    "gamedata/cmx",
    "gamedata/cmx2",
    "gamedata/cmx3",
    "gamedata/cmx4",
    "gamedata/cmx5",
    "gamedata/cmx6",
    "gamedata/cmx7",
    "gamedata/cmx8",
    "gamedata/cmx9",
    "gamedata/cmx10",
    "gamedata/cmxpreview",
    "gamedata/people1",
    "gamedata/people2",
    "gamedata/accessories1",
    "gamedata/accessories2",
    "expansionshared/skins",
    "expansionshared/skinsbuy",
    "expansionshared/animations",
    "expansionshared/skeletons",
    "templatenpcs",
    "expansionshared/accessories",
    "expansionpack7/skins",
    "expansionpack7/animations",
    "expansionpack7/accessories",
    "expansionpack6/skins",
    "expansionpack6/animations",
    "expansionpack6/accessories",
    "expansionpack5/skins",
    "expansionpack5/animations",
    "expansionpack5/accessories",
    "deluxe/skins",
    "deluxe/animations",
    "deluxe/accessories",
    "expansionpack4/skins",
    "expansionpack4/animations",
    "expansionpack4/accessories",
    "expansionpack3/skins",
    "expansionpack3/animations",
    "expansionpack3/accessories",
    "expansionpack2/skins",
    "expansionpack2/animations",
    "expansionpack2/accessories",
    "expansionpack",
    "expansionpack/expansionpackassets",
];

/// Where a folder comes in the original's appearance search: its place in
/// [`APPEARANCE_FOLDERS`], or after all of them for any other folder.
fn appearance_rank(folder: &str) -> usize {
    APPEARANCE_FOLDERS
        .iter()
        .position(|f| *f == folder)
        .unwrap_or(APPEARANCE_FOLDERS.len())
}

/// The appearance named `name`, from the catalog the original takes it
/// from.
///
/// The original loads every catalog in its search folders at startup and
/// keeps the first appearance of each name: a later record of a name
/// already loaded is released (0x59f087, the records' loop, and 0x5954a5).
/// Its file list (0x5c1a7d, as the old prototype ported it:
/// legacy/crates/sims-sim/src/pose/clothing/catalog_files.rs) takes every
/// text catalog (`.cmx`) in every folder before any binary one (`.bcf`),
/// the folders in `APPEARANCE_FOLDERS` order, and skips a file name
/// already listed from an earlier folder. `load_order` puts the
/// install's catalogs in that order and `index` keeps the first of each
/// name, so `B629MAFitlgt_labtech` is found in `b629ma_labtech.cmx` and
/// `b200mafit_ctb` in `GameData/Skins`, as in the original.
///
/// Two things are approximated. Files in one folder are taken in name
/// order; the original's order there is its file map's, which is not
/// ported, and no folder of the install holds two catalogs that give the
/// same appearance. And catalogs in folders the original does not search
/// (such as an expansion's object folder) are loaded after all the others,
/// where the original may not load them at all.
pub fn load_appearance(assets: &Assets, name: &str) -> Result<Appearance, String> {
    let (file, resource) = appearance_index(assets)
        .get(&name.to_ascii_lowercase())
        .ok_or_else(|| format!("no catalog holds the appearance {name:?}"))?;
    let bytes = resource.read().map_err(|e| format!("{file}: {e}"))?;
    parse_catalog(file, &bytes)?
        .appearances
        .into_iter()
        .find(|a| a.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("{file} no longer holds the appearance {name:?}"))
}

/// The name of every appearance the install's catalogs give, in
/// lowercase and in name order (see [`load_appearance`]). Reads every
/// catalog the first time.
pub fn appearance_names(assets: &Assets) -> impl Iterator<Item = &str> {
    appearance_index(assets).keys().map(String::as_str)
}

/// Where each appearance is loaded from, built on first use.
fn appearance_index(assets: &Assets) -> &BTreeMap<String, (String, sims_content::Resource)> {
    assets.appearances(|| {
        let mut files = Vec::new();
        for file in assets.names().filter(|n| is_catalog(n)) {
            for resource in assets.sources(file) {
                files.push((file.to_owned(), assets.folder(resource), resource.clone()));
            }
        }
        index(load_order(files).into_iter().map(|(file, resource)| {
            let catalog = resource
                .read()
                .map_err(|e| e.to_string())
                .and_then(|bytes| parse_catalog(&file, &bytes));
            ((file, resource), catalog)
        }))
    })
}

/// Whether the file `name` (lowercase) is an appearance catalog: a text or
/// binary catalog that is not a skeleton's.
fn is_catalog(name: &str) -> bool {
    (name.ends_with(".cmx") || name.ends_with(".cmx.bcf")) && !name.ends_with("-skeleton.cmx.bcf")
}

/// Catalog files, each with its lowercase file name and folder
/// ([`Assets::folder`]), in the order the original loads them: every text
/// catalog before any binary one, each kind by folder
/// ([`APPEARANCE_FOLDERS`], other folders last), and in one folder by the
/// order given. A file name already taken from an earlier folder is
/// dropped.
fn load_order<T>(mut files: Vec<(String, String, T)>) -> Vec<(String, T)> {
    files.sort_by_key(|(file, folder, _)| (file.ends_with(".bcf"), appearance_rank(folder)));
    let mut seen = std::collections::BTreeSet::new();
    files
        .into_iter()
        .filter(|(file, _, _)| seen.insert(file.clone()))
        .map(|(file, _, item)| (file, item))
        .collect()
}

/// For each appearance name (lowercase), the first catalog in `catalogs`
/// that holds it. A catalog that could not be read is skipped, as the
/// original goes on after a file it cannot read.
fn index<T>(catalogs: impl IntoIterator<Item = (T, Result<Catalog, String>)>) -> BTreeMap<String, T>
where
    T: Clone,
{
    let mut index = BTreeMap::new();
    for (location, catalog) in catalogs {
        let Ok(catalog) = catalog else { continue };
        for appearance in catalog.appearances {
            index
                .entry(appearance.name.to_ascii_lowercase())
                .or_insert_with(|| location.clone());
        }
    }
    index
}

/// The catalog `bytes` hold: binary for a `.bcf` file name, text
/// otherwise.
fn parse_catalog(file: &str, bytes: &[u8]) -> Result<Catalog, String> {
    if file.ends_with(".bcf") {
        read_bcf(bytes).map_err(|e| format!("{file}: {e}"))
    } else {
        read_cmx(bytes).map_err(|e| format!("{file}: {e}"))
    }
}

/// The mesh named `name`, from `<name>.bmf` or `<name>.skn`.
pub fn load_mesh(assets: &Assets, name: &str) -> Result<Mesh, String> {
    let binary = format!("{name}.bmf");
    let text = format!("{name}.skn");
    if assets.contains(&binary) {
        read_bmf(&assets.read(&binary)?).map_err(|e| format!("{binary}: {e}"))
    } else if assets.contains(&text) {
        read_skn(&assets.read(&text)?).map_err(|e| format!("{text}: {e}"))
    } else {
        Err(format!("no file holds the mesh {name:?}"))
    }
}

/// The texture named `name` (`<name>.bmp`).
pub fn load_texture(assets: &Assets, name: &str) -> Result<Texture, String> {
    let file = format!("{name}.bmp");
    read_bmp(&assets.read(&file)?).map_err(|e| format!("{file}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pet's head row names no appearance: its outfit is the body alone.
    #[test]
    fn an_empty_slot_names_no_part() {
        let mut strings = vec![String::new(); 35];
        strings[0] = "bones".into();
        strings[1] = "coat,BODY=coat".into();
        strings[2] = NO_APPEARANCE.into();
        let outfit = Outfit::from_body_strings(&strings);
        assert_eq!(outfit.parts, [("coat".to_owned(), "BODY=coat".to_owned())]);
    }

    /// A skeleton is looked for under the hyphened name first, then the
    /// underscored one the pets' files have.
    #[test]
    fn a_skeleton_file_is_hyphened_or_underscored() {
        assert_eq!(
            skeleton_files("bones"),
            ["bones-skeleton.cmx.bcf", "bones_skeleton.cmx.bcf"]
        );
    }

    /// A hyphened file holding another skeleton, or one that cannot be
    /// read, does not hide the underscored file that holds the name.
    #[test]
    fn the_skeleton_is_found_in_whichever_file_holds_its_name() {
        let named = |name: &str| Skeleton {
            name: name.to_owned(),
            bones: Vec::new(),
        };
        let found = named_skeleton(
            [
                ("a-skeleton", Ok(vec![named("other")])),
                ("a_skeleton", Ok(vec![named("x"), named("Bones")])),
            ],
            "bones",
        );
        assert_eq!(found.map(|s| s.name).as_deref(), Ok("Bones"));
        let found = named_skeleton(
            [
                ("a-skeleton", Err("unreadable".to_owned())),
                ("a_skeleton", Ok(vec![named("bones")])),
            ],
            "bones",
        );
        assert!(found.is_ok());
        let missing = named_skeleton(
            [
                ("a-skeleton", Ok(vec![named("other")])),
                ("a_skeleton", Err("unreadable".to_owned())),
            ],
            "bones",
        )
        .unwrap_err();
        assert!(
            missing.contains("a-skeleton") && missing.contains("unreadable"),
            "{missing}"
        );
    }

    #[test]
    fn a_type_without_clothes_chosen_wears_its_default_rows() {
        let mut strings = vec![String::new(); 35];
        strings[0] = "adult".into();
        strings[1] = "body_01,BODY=bodytex".into();
        strings[2] = "head_01,HEAD-HEAD=headtex".into();
        strings[17] = "HfLO,HAND=huaolgt".into();
        strings[18] = "HfRO;HfRG,HAND=other".into();
        let outfit = Outfit::from_body_strings(&strings);
        let pair = |a: &str, b: &str| (a.to_owned(), b.to_owned());
        assert_eq!(
            outfit.parts,
            [
                pair("body_01", "BODY=bodytex"),
                pair("HfLO", "HAND=huaolgt"),
                pair("HfRO", ""),
                pair("head_01", "HEAD-HEAD=headtex"),
            ]
        );
        // Short or empty tables give what they have, and never panic.
        assert!(Outfit::from_body_strings(&[]).parts.is_empty());
        assert_eq!(
            Outfit::from_body_strings(&strings[..2]).parts,
            [pair("body_01", "BODY=bodytex")]
        );
    }

    #[test]
    fn catalogs_load_text_first_then_by_folder() {
        let file = |name: &str, folder: &str, id: u32| (name.to_owned(), folder.to_owned(), id);
        let order = load_order(vec![
            file("a.cmx.bcf", "gamedata/skins", 1),
            file("a.cmx", "deluxe/skins", 2),
            file("b.cmx", "somewhere/else", 3),
            file("b.cmx", "expansionpack7/skins", 4),
            file("c.cmx", "gamedata/skins", 5),
            file("b.cmx", "gamedata/skins", 6),
            file("d.cmx.bcf", "expansionshared/skins", 7),
        ]);
        let ids: Vec<u32> = order.iter().map(|(_, id)| *id).collect();
        // Text before binary, however early the binary's folder; by folder
        // within each; a name already taken from an earlier folder is
        // dropped (3 and 4 lose to 6); one folder keeps the order given.
        assert_eq!(ids, [5, 6, 2, 1, 7]);
        assert_eq!(order[0].0, "c.cmx");
    }

    #[test]
    fn the_first_catalog_that_holds_a_name_keeps_it() {
        let catalog = |names: &[&str]| {
            let appearances = names
                .iter()
                .map(|name| Appearance {
                    name: (*name).into(),
                    kind: 0,
                    bindings: Vec::new(),
                })
                .collect();
            Ok(Catalog {
                skeletons: Vec::new(),
                appearances,
            })
        };
        let index = index([
            (1, catalog(&["Shirt"])),
            (2, Err("unreadable".to_owned())),
            (3, catalog(&["shirt", "Hat"])),
        ]);
        assert_eq!(index.get("shirt"), Some(&1));
        // A catalog that cannot be read does not stop the ones after it.
        assert_eq!(index.get("hat"), Some(&3));
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn a_texture_string_names_the_meshes_whose_names_hold_its_key() {
        let s = "HEAD-HEAD=c006fcmed_nerd";
        assert_eq!(
            texture_for("xskin-c006fc_nerd-HEAD-HEAD", s).as_deref(),
            Some("c006fcmed_nerd")
        );
        assert_eq!(texture_for("xskin-c006fc_nerd-HEAD-NERDSPECS", s), None);
        assert_eq!(
            texture_for("xskin-hflo-L_HAND-HANDFL", "HAND=huaomed").as_deref(),
            Some("huaomed")
        );
        // The key's case matters.
        assert_eq!(
            texture_for("xskin-hflo-L_HAND-HANDFL", "hand=huaomed"),
            None
        );
        assert_eq!(texture_for("x-BODY", "BODY="), None);
        assert_eq!(texture_for("x-BODY", ""), None);
        assert_eq!(texture_for("", "=a"), None);
    }
}
