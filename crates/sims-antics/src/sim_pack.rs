//! Complete self-contained standalone `open-pack/` compiler combining:
//! - all clean-room household objects (`open-pack.far` + per-object `.iff` + synthesized WAV bank),
//! - procedural `.flr` floor and `.wll` wall IFF files from `surfaces.toml`,
//! - starter `House01.iff` + `Neighborhood.iff` with a 2-Sim clean-room household
//!   from `starter-house.toml`.
//!
//! Ready for dual-mode standalone `sims-game` boot with zero commercial assets.

use crate::compile::{CompiledPack, compile_pack_dir};
use crate::error::{Error, Result};
use crate::house::{
    CompiledSurfaces, PlacedObjectInfo, StarterHouseManifest, StarterPlacementSpec, StarterSimSpec,
    SurfacesManifest, build_starter_house_iff, build_starter_neighborhood_iff, compile_surfaces,
};
use crate::lift::lift_objd;
use sims_formats::iff::Iff;
use std::collections::BTreeMap;
use std::path::Path;

/// A complete self-contained standalone open-pack world bundle compiled 100%
/// from plain-text `open-pack/` sources.
#[derive(Debug, Clone)]
pub struct StandalonePack {
    pub pack: CompiledPack,
    pub floors: BTreeMap<String, Vec<u8>>,
    pub walls: BTreeMap<String, Vec<u8>>,
    pub house_iff_name: String,
    pub house_iff: Vec<u8>,
    pub neighborhood_iff: Vec<u8>,
    pub house_manifest: StarterHouseManifest,
    pub starter_sims: Vec<StarterSimSpec>,
    pub placements: Vec<StarterPlacementSpec>,
}

fn read_pack_text_file(root: &Path, root_canon: &Path, filename: &str) -> Result<String> {
    use std::io::Read;

    let path = root.join(filename);
    let meta = std::fs::symlink_metadata(&path)
        .map_err(|e| Error::new(format!("failed to access {}: {e}", path.display())))?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(Error::new(format!(
            "expected regular non-symlink file at {}",
            path.display()
        )));
    }
    let canon = std::fs::canonicalize(&path)
        .map_err(|e| Error::new(format!("failed to resolve {}: {e}", path.display())))?;
    if !canon.starts_with(root_canon) {
        return Err(Error::new(format!(
            "file {} escapes pack root {}",
            canon.display(),
            root_canon.display()
        )));
    }
    let mut file = std::fs::File::open(&canon)
        .map_err(|e| Error::new(format!("failed to open {}: {e}", canon.display())))?;
    let open_meta = file
        .metadata()
        .map_err(|e| Error::new(format!("failed to stat {}: {e}", canon.display())))?;
    if !open_meta.file_type().is_file() {
        return Err(Error::new(format!(
            "expected regular file at {}",
            canon.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if open_meta.dev() != meta.dev() || open_meta.ino() != meta.ino() {
            return Err(Error::new(format!(
                "path {} changed between validation and open",
                canon.display()
            )));
        }
    }
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| Error::new(format!("failed to read {}: {e}", canon.display())))?;
    Ok(text)
}

/// Compiles a complete standalone `open-pack/` directory (`pack.toml`, `objects/*`,
/// `surfaces.toml`, `starter-house.toml`) into an in-memory [`StandalonePack`]
/// ready to write to disk or boot directly in `sims-game`.
pub fn compile_standalone_pack_dir(dir: &Path) -> Result<StandalonePack> {
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

    let pack = compile_pack_dir(dir)?;

    let surfaces_toml = read_pack_text_file(dir, &root_canon, "surfaces.toml")?;
    let surfaces_manifest =
        SurfacesManifest::parse(&surfaces_toml).map_err(|e| e.with_context("surfaces.toml"))?;
    let compiled_surfaces: CompiledSurfaces = compile_surfaces(&surfaces_manifest)?;

    let house_toml = read_pack_text_file(dir, &root_canon, "starter-house.toml")?;
    let house_manifest = StarterHouseManifest::parse(&house_toml)
        .map_err(|e| e.with_context("starter-house.toml"))?;

    // Extract object placement metadata directly from the compiled IFFs.
    let mut catalog = BTreeMap::new();
    for entry in &pack.objects {
        let iff = Iff::parse(entry.object.iff_bytes.clone())?;
        let mut price = 100u16;
        let mut attributes = 0usize;
        let mut placed_guid = entry.object.guid;
        let is_multipart = !entry.object.tile_guids.is_empty();
        // For multi-tile objects, place the lead tile GUID (`sub_index == 0` / `lead_tile != 0`).
        if is_multipart {
            for chunk in iff.all(b"OBJD") {
                if let Ok(objd) = lift_objd(chunk.id, &chunk.label, iff.data(chunk)) {
                    let w = |i: usize| objd.words.get(i).copied().unwrap_or(0);
                    let sub_index = w(11) as i16;
                    let guid = u32::from(w(14)) | (u32::from(w(15)) << 16);
                    let obj_price = w(18);
                    let num_attributes = w(58);
                    let lead_tile = w(63);
                    if sub_index == -1 {
                        price = obj_price;
                    } else if lead_tile != 0 {
                        placed_guid = guid;
                        attributes = usize::from(num_attributes);
                    }
                }
            }
        } else if let Some(chunk) = iff.all(b"OBJD").next()
            && let Ok(objd) = lift_objd(chunk.id, &chunk.label, iff.data(chunk))
        {
            let w = |i: usize| objd.words.get(i).copied().unwrap_or(0);
            price = w(18);
            attributes = usize::from(w(58));
        }
        catalog.insert(
            entry.slug.clone(),
            PlacedObjectInfo {
                slug: entry.slug.clone(),
                name: entry.object.name.clone(),
                guid: placed_guid,
                price,
                attributes,
                is_multipart,
            },
        );
    }

    let house_iff = build_starter_house_iff(&house_manifest, &compiled_surfaces, &catalog)?;
    let neighborhood_iff = build_starter_neighborhood_iff(&house_manifest)?;
    let house_iff_name = format!("House{:02}.iff", house_manifest.lot_number);
    let starter_sims = house_manifest.sims.clone();
    let placements = house_manifest.placements.clone();

    Ok(StandalonePack {
        pack,
        floors: compiled_surfaces.floors,
        walls: compiled_surfaces.walls,
        house_iff_name,
        house_iff,
        neighborhood_iff,
        house_manifest,
        starter_sims,
        placements,
    })
}
