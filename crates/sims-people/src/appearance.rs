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

/// The appearances a person wears, each with its texture string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outfit {
    pub parts: Vec<(String, String)>,
}

impl Outfit {
    /// The appearances a saved person record names, in its order: body,
    /// left hand, right hand, head, then the accessories. Empty names are
    /// skipped.
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
        parts.retain(|(name, _)| !name.is_empty());
        Self { parts }
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
    /// Loads the skeleton named `skeleton` (`<name>-skeleton.cmx.bcf`,
    /// such as `adult`) and every part of `outfit`.
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

/// The skeleton `<name>-skeleton.cmx.bcf` holds under the name `name`.
pub fn load_skeleton(assets: &Assets, name: &str) -> Result<Skeleton, String> {
    let file = format!("{name}-skeleton.cmx.bcf");
    let catalog = read_bcf(&assets.read(&file)?).map_err(|e| format!("{file}: {e}"))?;
    catalog
        .skeletons
        .into_iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("{file} holds no skeleton named {name:?}"))
}

/// The appearance named `name`: from `<name>.cmx.bcf` or `<name>.cmx`
/// when that file holds it, otherwise from the first catalog, in name
/// order, that holds an appearance of that name. Career uniforms need the
/// second way: `B629MAFitlgt_labtech` is in `b629ma_labtech.cmx`. The old
/// prototype looked appearances up by their own names the same way
/// (legacy/crates/sims-content/src/avatar.rs); which catalog the
/// original takes when two hold the same name is **unverified**.
pub fn load_appearance(assets: &Assets, name: &str) -> Result<Appearance, String> {
    let find = |catalog: Catalog| {
        catalog
            .appearances
            .into_iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
    };
    for file in [format!("{name}.cmx.bcf"), format!("{name}.cmx")] {
        if assets.contains(&file)
            && let Some(appearance) = find(read_catalog(assets, &file)?)
        {
            return Ok(appearance);
        }
    }
    for file in assets
        .names()
        .filter(|n| n.ends_with(".cmx") || n.ends_with(".cmx.bcf"))
    {
        // Skeleton and animation files are not appearance catalogs.
        if file.ends_with("-skeleton.cmx.bcf") {
            continue;
        }
        let Ok(catalog) = read_catalog(assets, file) else {
            continue;
        };
        if let Some(appearance) = find(catalog) {
            return Ok(appearance);
        }
    }
    Err(format!("no catalog holds the appearance {name:?}"))
}

/// A catalog file, binary (`.cmx.bcf`) or text (`.cmx`).
fn read_catalog(assets: &Assets, file: &str) -> Result<Catalog, String> {
    let bytes = assets.read(file)?;
    if file.ends_with(".bcf") {
        read_bcf(&bytes).map_err(|e| format!("{file}: {e}"))
    } else {
        read_cmx(&bytes).map_err(|e| format!("{file}: {e}"))
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
