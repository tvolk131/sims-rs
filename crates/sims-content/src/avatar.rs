//! One explicit base-game adult appearance, with source provenance retained.
use crate::Index;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use sims_formats::avatar::{self, Mesh, Skeleton};
use std::{collections::BTreeMap, io::Cursor};

#[derive(Debug, Clone)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Debug, Clone)]
pub struct Part {
    /// Exact BCF appearance binding name; distinct from the mesh's bone table.
    pub attachment: String,
    pub mesh: Mesh,
    pub texture: Texture,
    pub builtin_material: Option<crate::avatar_material::Builtin>,
    /// Kind-1 appearance meshes delimit the censor pass, not ordinary clothing.
    pub censor: bool,
}
#[derive(Debug, Clone)]
pub struct Avatar {
    pub skeleton: Skeleton,
    pub parts: Vec<Part>,
    pub sources: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Accessory {
    pub parts: Vec<Part>,
    pub sources: BTreeMap<String, String>,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct AccessoryReport {
    pub catalogs: usize,
    pub loaded: Vec<String>,
    pub catalog_errors: Vec<String>,
    /// STR#304 includes built-in outfit labels as well as appearance names.
    pub unresolved_labels: Vec<String>,
}
/// Loaded appearance layers referenced by linked STR#304 resources. Descriptive
/// built-in outfit labels need not name a BCF appearance. Any actual unresolved
/// accessory request remains a VM error at the original instruction.
pub fn load_linked_accessories(
    index: &Index,
    program: &sims_vm::Program,
) -> Result<BTreeMap<String, Accessory>> {
    Ok(load_linked_accessories_report(index, program)?.0)
}
pub fn load_linked_accessories_report(
    index: &Index,
    program: &sims_vm::Program,
) -> Result<(BTreeMap<String, Accessory>, AccessoryReport)> {
    let wanted: std::collections::BTreeSet<_> = program
        .modules
        .values()
        .filter_map(|m| m.strings.get(&304))
        .flatten()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    load_named_accessories(index, &wanted)
}

/// Bind exact runtime accessory requests after validating their original
/// catalog, mesh and texture dependencies. No unrelated table entry is loaded.
pub fn bind_named_accessories(
    index: &Index,
    program: &mut sims_vm::Program,
    names: &std::collections::BTreeSet<String>,
) -> Result<AccessoryReport> {
    let wanted = names
        .iter()
        .filter(|name| !program.accessories.contains_key(*name))
        .cloned()
        .collect();
    let (accessories, report) = load_named_accessories(index, &wanted)?;
    program
        .accessories
        .extend(accessories.into_iter().map(|(name, a)| (name, a.sources)));
    Ok(report)
}

pub fn load_accessories(index: &Index) -> Result<BTreeMap<String, Accessory>> {
    Ok(load_named_accessories(
        index,
        &["suit-fork".into(), "suit-butterknife".into()].into(),
    )?
    .0)
}

/// Resolve the skeleton's registered name through the original startup/NDX
/// catalogs. The name is not a filename: pets use dog_skeleton/kat_skeleton.
/// Native provider order and lazy record replacement select the actual asset.
pub fn load_skeleton(index: &Index, name: &str) -> Result<(Skeleton, usize)> {
    load_skeleton_with_startup(index, name, None)
}

pub(crate) fn load_skeleton_with_startup(
    index: &Index,
    name: &str,
    startup: Option<&mut crate::startup_memo::Memo>,
) -> Result<(Skeleton, usize)> {
    use sims_formats::avatar::{catalog::Asset, index::Kind};
    let (mut installed, directories) = match startup {
        Some(startup) => startup.installed_catalog(index)?,
        None => crate::clothing_catalog::installed_catalog(index)?,
    };
    let record = installed
        .catalog
        .resolve(index, &directories, Kind::Skeleton, name.as_bytes())?
        .with_context(|| format!("skeleton {name} absent from installed avatar registry"))?;
    let Asset::Skeleton(skeleton) = &record.value else {
        bail!("skeleton registry returned another asset kind");
    };
    Ok((skeleton.clone(), record.source))
}

/// Resolve an embedded name in an appearance-only BCF/CMX catalog, without assuming
/// its filename. Mixed skeleton/animation catalogs remain outside this loader.
/// Byte-identical providers are equivalent; conflicting providers need an
/// explicit content-order policy instead of an arbitrary archive sort order.
pub fn find_appearance_catalog(index: &Index, name: &str) -> Result<usize> {
    appearance_catalog(index, name)?
        .with_context(|| format!("appearance {name} absent from catalogs"))
}

/// None is only absence from this descriptor scan. Callers needing a native
/// missing-resource reply must separately check the installed startup registry.
pub(crate) fn appearance_catalog(index: &Index, name: &str) -> Result<Option<usize>> {
    let mut scan = crate::resource_scan::Scan::default();
    appearance_catalog_read(index, name, |r| scan.read(r))
}
pub(crate) fn appearance_catalog_read(
    index: &Index,
    name: &str,
    mut read: impl FnMut(&crate::Resource) -> Result<Vec<u8>>,
) -> Result<Option<usize>> {
    let mut found: Option<(usize, Vec<u8>)> = None;
    for (id, resource) in index.resources.iter().enumerate() {
        if !is_appearance_catalog(resource) {
            continue;
        }
        let bytes = read(resource)?;
        if !is_text_catalog(&resource.name)
            && (bytes.get(..4) != Some(&[0; 4]) || bytes.get(4..8) == Some(&[0; 4]))
        {
            continue;
        }
        let catalog = read_catalog(&resource.name, &bytes)
            .with_context(|| format!("read appearance catalog {}", resource.locator))?;
        if !catalog
            .appearances
            .iter()
            .any(|a| a.name.eq_ignore_ascii_case(name))
        {
            continue;
        }
        if let Some((previous, previous_bytes)) = &found {
            if previous_bytes != &bytes {
                bail!(
                    "conflicting appearance {name}: {} and {}",
                    index.resources[*previous].locator,
                    resource.locator
                );
            }
        } else {
            found = Some((id, bytes));
        }
    }
    Ok(found.map(|(id, _)| id))
}

fn is_text_catalog(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".cmx")
}
fn is_appearance_catalog(resource: &crate::Resource) -> bool {
    // The extracted cabinet also contains the separate Sims Creator tool's
    // private reference skins. They are not installed in the game's skin paths
    // (Creator_Files/Readme/readme.txt). This is source membership, not priority
    // between two game providers; those conflicts still fail explicitly.
    if resource
        .locator
        .to_ascii_lowercase()
        .starts_with("creator_files/")
    {
        return false;
    }
    is_text_catalog(&resource.name) || resource.name.to_ascii_lowercase().ends_with(".cmx.bcf")
}
pub(crate) fn read_catalog(name: &str, bytes: &[u8]) -> sims_formats::Result<avatar::Catalog> {
    if is_text_catalog(name) {
        avatar::read_cmx(bytes)
    } else {
        avatar::read_bcf(bytes)
    }
}

/// The supported binary-enabled native profile checks BMF first (59e32a), then
/// SKN when no binary resource exists. Ambiguous or invalid BMFs remain errors.
pub(crate) fn mesh_resource(index: &Index, catalog: usize, name: &str) -> Result<usize> {
    let binary = format!("{name}.bmf");
    let filename = if index
        .resources
        .iter()
        .any(|r| r.name.eq_ignore_ascii_case(&binary))
    {
        binary
    } else {
        format!("{name}.skn")
    };
    crate::assets::sibling(index, catalog, &filename)
}
pub(crate) fn read_mesh(name: &str, bytes: &[u8]) -> sims_formats::Result<Mesh> {
    if name.to_ascii_lowercase().ends_with(".skn") {
        avatar::read_skn(bytes)
    } else {
        avatar::read_bmf(bytes)
    }
}

pub fn load_named_accessories(
    index: &Index,
    wanted: &std::collections::BTreeSet<String>,
) -> Result<(BTreeMap<String, Accessory>, AccessoryReport)> {
    let mut result = BTreeMap::new();
    let mut report = AccessoryReport::default();
    if wanted.is_empty() {
        return Ok((result, report));
    }
    for (id, resource) in index.resources.iter().enumerate() {
        if !is_appearance_catalog(resource) {
            continue;
        }
        let bytes = resource.read()?;
        // Appearance-only catalogs. Other avatar kinds remain the avatar loader's responsibility.
        if !is_text_catalog(&resource.name)
            && (bytes.get(..4) != Some(&[0; 4]) || bytes.get(4..8) == Some(&[0; 4]))
        {
            continue;
        }
        let catalog = match read_catalog(&resource.name, &bytes) {
            Ok(c) => c,
            Err(e) => {
                report
                    .catalog_errors
                    .push(format!("{}: {e}", resource.locator));
                continue;
            }
        };
        report.catalogs += 1;
        for appearance in catalog
            .appearances
            .iter()
            .filter(|a| wanted.contains(&a.name.to_ascii_lowercase()))
        {
            if appearance.kind != 0 {
                bail!("unsupported accessory appearance kind: {}", appearance.name);
            }
            let mut sources = BTreeMap::from([(
                resource.locator.clone(),
                format!("{:x}", Sha256::digest(&bytes)),
            )]);
            let mut parts = vec![];
            for binding in &appearance.bindings {
                if binding.censor != 0 {
                    bail!(
                        "censored accessory binding unsupported: {}",
                        appearance.name
                    );
                }
                let mesh_id = mesh_resource(index, id, &binding.mesh)?;
                let mesh_res = &index.resources[mesh_id];
                let data = mesh_res.read()?;
                sources.insert(
                    mesh_res.locator.clone(),
                    format!("{:x}", Sha256::digest(&data)),
                );
                let mesh = read_mesh(&mesh_res.name, &data)?;
                let texture_name = format!("{}.bmp", mesh.texture);
                let texture_id = crate::assets::sibling(index, mesh_id, &texture_name)?;
                let texture_res = &index.resources[texture_id];
                let data = texture_res.read()?;
                sources.insert(
                    texture_res.locator.clone(),
                    format!("{:x}", Sha256::digest(&data)),
                );
                let mut reader =
                    image::ImageReader::with_format(Cursor::new(data), image::ImageFormat::Bmp);
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(2048);
                limits.max_image_height = Some(2048);
                limits.max_alloc = Some(32 * 1024 * 1024);
                reader.limits(limits);
                let image = reader.decode()?.to_rgba8();
                parts.push(Part {
                    attachment: binding.bone.clone(),
                    builtin_material: None,
                    censor: false,
                    mesh,
                    texture: Texture {
                        width: image.width(),
                        height: image.height(),
                        rgba: image.into_raw(),
                    },
                });
            }
            let name = appearance.name.to_ascii_lowercase();
            if result
                .insert(name.clone(), Accessory { parts, sources })
                .is_some()
            {
                bail!("conflicting accessory providers: {name}");
            }
        }
    }
    report.loaded = result.keys().cloned().collect();
    report.unresolved_labels = wanted
        .iter()
        .filter(|n| !result.contains_key(*n))
        .cloned()
        .collect();
    Ok((result, report))
}

pub fn load_adult(index: &Index) -> Result<Avatar> {
    load_adult_outfit(index, false)
}
/// Explicit male fit/light template profile. The engine's arbitrary imported
/// person skin-selection rules remain separate from the native outfit fields.
pub fn load_adult_outfit(index: &Index, naked: bool) -> Result<Avatar> {
    load_adult_profile(index, naked, false)
}
pub fn load_adult_profile(index: &Index, naked: bool, casual: bool) -> Result<Avatar> {
    load_profile(index, naked, casual, false)
}
pub fn load_adult_sleepwear(index: &Index) -> Result<Avatar> {
    load_profile(index, false, false, true)
}
fn load_profile(index: &Index, naked: bool, casual: bool, sleepwear: bool) -> Result<Avatar> {
    let mut sources = BTreeMap::new();
    let mut read = |group: &str, name: &str| -> Result<Vec<u8>> {
        let locator = format!("GameData_ALL_Recursive/{group}/{group}.far::{name}");
        let resource = &index.resources[index.find(&locator)?];
        let bytes = resource.read()?;
        sources.insert(locator, format!("{:x}", Sha256::digest(&bytes)));
        Ok(bytes)
    };
    let mut catalog = avatar::read_bcf(&read("Animation", "adult-skeleton.cmx.bcf")?)?;
    if catalog.skeletons.len() != 1 {
        bail!("adult profile needs exactly one skeleton");
    }
    let skeleton = catalog.skeletons.remove(0);
    let mut parts = Vec::new();
    for (appearance, texture) in [
        if sleepwear {
            ("pajamamfit_01", "PajamaMlgt_01.bmp")
        } else if naked {
            ("nmfit_01", "NMFitlgt_01.bmp")
        } else {
            (
                "b002mafit_01",
                if casual {
                    "B002MAFitlgt_casual.bmp"
                } else {
                    "B002MAFitlgt_Ross.bmp"
                },
            )
        },
        ("c001ma_ross", "C001MAlgt_Ross.bmp"),
        ("hmlo", "HUAOlgt.bmp"),
        ("hmro", "HUAOlgt.bmp"),
    ] {
        let catalog = avatar::read_bcf(&read("Animation", &format!("{appearance}.cmx.bcf"))?)?;
        let app = catalog
            .appearances
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(appearance))
            .context("appearance name not in catalog")?;
        for binding in &app.bindings {
            let mesh = avatar::read_bmf(&read("Animation", &format!("{}.bmf", binding.mesh))?)?;
            for name in mesh.bones.iter().chain(std::iter::once(&binding.bone)) {
                if !skeleton
                    .bones
                    .iter()
                    .any(|b| b.name.eq_ignore_ascii_case(name))
                {
                    bail!("mesh references absent skeleton bone {name}");
                }
            }
            let bytes = read("Textures", texture)?;
            let mut reader =
                image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Bmp);
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(2048);
            limits.max_image_height = Some(2048);
            limits.max_alloc = Some(32 * 1024 * 1024);
            reader.limits(limits);
            let image = reader.decode()?.to_rgba8();
            parts.push(Part {
                attachment: binding.bone.clone(),
                builtin_material: None,
                censor: false,
                mesh,
                texture: Texture {
                    width: image.width(),
                    height: image.height(),
                    rgba: image.into_raw(),
                },
            });
        }
    }
    Ok(Avatar {
        skeleton,
        parts,
        sources,
    })
}
