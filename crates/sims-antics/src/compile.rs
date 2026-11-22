//! End-to-end compilation pipeline turning `.antics` + `object.toml` into
//! complete IFF 2.5 objects, synthesized WAVs, and `FAR!byAZ` archives.

use crate::art::generate_furniture_sprites;
use crate::audio::synthesize_preset;
use crate::encode::{
    encode_bcon, encode_bhav, encode_dgrp, encode_far, encode_fwav, encode_iff_with_rsmp,
    encode_objd, encode_objf, encode_palt, encode_slot, encode_spr2, encode_string_table,
    encode_ttab,
};
use crate::error::{Error, Result};
use crate::ir::{
    FarEntry, FwavChunk, IFF_CONVERTED_FLAG, ObjdSpec, ObjfChunk, SlotChunk, StringEntry,
    StringFormat, StringTableChunk, TtabChunk,
};
use crate::lower::{LoweredModule, lower_module};
use crate::manifest::{ObjectManifest, TreeRef};
use crate::parser::parse_module;
use sims_formats::behavior::{Interaction, MotiveAdvertisement};
use sims_formats::execution::{ObjectFunction, Slot};
use sims_formats::iff::ChunkData;
use std::collections::BTreeMap;
use std::path::Path;

/// Compiled standalone `.antics` module.
pub type CompiledModule = LoweredModule;

/// A fully compiled clean-room Sims 1 object (`.iff` bytes + synthesized WAVs).
#[derive(Debug, Clone)]
pub struct CompiledObject {
    pub guid: u32,
    pub name: String,
    pub iff_bytes: Vec<u8>,
    /// Synthesized RIFF/WAVE sound effects keyed by FWAV sound event name.
    pub sounds_wav: BTreeMap<String, Vec<u8>>,
    pub behaviors: BTreeMap<u16, crate::ir::BhavChunk>,
}

/// Parses and lowers one `.antics` source string into validated `BHAV`/`BCON` chunks.
pub fn compile_antics(source: &str) -> Result<CompiledModule> {
    let ast = parse_module(source)?;
    lower_module(&ast)
}

/// Compiles an `object.toml` manifest together with named `.antics` sources
/// entirely in memory into a complete `.iff` (with `rsmp`) and synthesized WAVs.
pub fn compile_object_from_sources(
    manifest_toml: &str,
    antics_sources: &[(&str, &str)],
) -> Result<CompiledObject> {
    let manifest = ObjectManifest::parse(manifest_toml)?;
    let mut behaviors = BTreeMap::new();
    let mut tree_ids_by_name = BTreeMap::new();
    let mut constants = BTreeMap::new();

    for &(filename, src) in antics_sources {
        let compiled = compile_antics(src).map_err(|e| e.with_context(filename))?;
        for (name, id) in compiled.tree_ids_by_name {
            if let Some(prev) = tree_ids_by_name.insert(name.clone(), id)
                && prev != id
            {
                return Err(Error::new(format!(
                    "duplicate tree name '{name}' across scripts ({prev} vs {id})"
                )));
            }
        }
        for (id, bhav) in compiled.behaviors {
            if behaviors.insert(id, bhav).is_some() {
                return Err(Error::new(format!(
                    "duplicate BHAV id {id} across scripts in '{}'",
                    manifest.name
                )));
            }
        }
        for (id, bcon) in compiled.constants {
            if constants.insert(id, bcon).is_some() {
                return Err(Error::new(format!(
                    "duplicate BCON id {id} across scripts in '{}'",
                    manifest.name
                )));
            }
        }
    }

    let resolve_tree = |r: &TreeRef| -> Result<u16> {
        match r {
            TreeRef::Id(id) => Ok(*id),
            TreeRef::Name(name) => tree_ids_by_name.get(name).copied().ok_or_else(|| {
                Error::new(format!(
                    "object '{}' references unknown tree '{name}'",
                    manifest.name
                ))
            }),
        }
    };

    let init_id = match &manifest.init {
        Some(r) => resolve_tree(r)?,
        None => 0,
    };
    let main_id = match &manifest.main {
        Some(r) => resolve_tree(r)?,
        None => 0,
    };

    let mut chunks = Vec::new();

    // 1. OBJD
    let mut objd_spec = ObjdSpec::new(manifest.objd_id, &manifest.name, manifest.guid);
    objd_spec.price = manifest.price;
    objd_spec.object_type = manifest.object_type;
    objd_spec.base_graphic_id = manifest.dgrp_id;
    objd_spec.num_graphics = 1;
    objd_spec.tree_table_id = manifest.ttab_id;
    objd_spec.slots_id = manifest.slot_id;
    objd_spec.catalog_strings_id = manifest.ctss_id;
    objd_spec.init_id = init_id;
    objd_spec.main_id = main_id;
    chunks.push(ChunkData {
        tag: *b"OBJD",
        id: manifest.objd_id,
        flags: IFF_CONVERTED_FLAG,
        label: manifest.name.clone(),
        data: encode_objd(&objd_spec.to_chunk())?,
    });

    // 2. OBJf (Init = index 0, Main = index 1)
    let objf = ObjfChunk::new(
        manifest.objd_id,
        format!("{} Functions", manifest.name),
        vec![
            ObjectFunction {
                condition: 0,
                action: init_id,
            },
            ObjectFunction {
                condition: 0,
                action: main_id,
            },
        ],
    );
    chunks.push(ChunkData {
        tag: *b"OBJf",
        id: objf.id,
        flags: IFF_CONVERTED_FLAG,
        label: objf.label.clone(),
        data: encode_objf(&objf)?,
    });

    // 3. CTSS (catalog name + description)
    let ctss = StringTableChunk::new(
        *b"CTSS",
        manifest.ctss_id,
        format!("{} Catalog", manifest.name),
        StringFormat::Localized,
        vec![
            StringEntry::english(&manifest.name),
            StringEntry::english(&manifest.description),
        ],
    );
    chunks.push(ChunkData {
        tag: *b"CTSS",
        id: ctss.id,
        flags: IFF_CONVERTED_FLAG,
        label: ctss.label.clone(),
        data: encode_string_table(&ctss)?,
    });

    // 4. BHAV chunks
    for bhav in behaviors.values() {
        chunks.push(ChunkData {
            tag: *b"BHAV",
            id: bhav.id,
            flags: IFF_CONVERTED_FLAG,
            label: bhav.label.clone(),
            data: encode_bhav(bhav)?,
        });
    }

    // 5. BCON chunks
    for bcon in constants.values() {
        chunks.push(ChunkData {
            tag: *b"BCON",
            id: bcon.id,
            flags: IFF_CONVERTED_FLAG,
            label: bcon.label.clone(),
            data: encode_bcon(bcon)?,
        });
    }

    // 6. TTAB + TTAs
    let mut ttab_entries = Vec::with_capacity(manifest.interactions.len());
    let mut ttas_entries = Vec::with_capacity(manifest.interactions.len());
    for (idx, inter) in manifest.interactions.iter().enumerate() {
        let action = resolve_tree(&inter.action)?;
        let test = match &inter.test {
            Some(t) => resolve_tree(t)?,
            None => 0,
        };
        // Motive table is indexed by motive ID (0..=max_motive_index).
        let max_motive = inter
            .motives
            .iter()
            .map(|m| m.motive_index + 1)
            .max()
            .unwrap_or(0);
        let mut motives = vec![
            MotiveAdvertisement {
                minimum: 0,
                delta: 0,
                personality: 0,
            };
            max_motive
        ];
        for ad in &inter.motives {
            motives[ad.motive_index] = MotiveAdvertisement {
                minimum: ad.minimum,
                delta: ad.delta,
                personality: ad.personality,
            };
        }
        ttab_entries.push(Interaction {
            action,
            test,
            string_index: idx as u32,
            flags: inter.flags,
            autonomy: inter.autonomy,
            attenuation_code: inter.attenuation_code,
            attenuation: inter.attenuation,
            joining_index: inter.joining_index,
            motives,
            flags2: inter.flags2,
        });
        ttas_entries.push(StringEntry::english(&inter.name));
    }
    let ttab = TtabChunk::new(
        manifest.ttab_id,
        format!("{} Interactions", manifest.name),
        ttab_entries,
    );
    chunks.push(ChunkData {
        tag: *b"TTAB",
        id: ttab.id,
        flags: IFF_CONVERTED_FLAG,
        label: ttab.label.clone(),
        data: encode_ttab(&ttab)?,
    });
    let ttas = StringTableChunk::new(
        *b"TTAs",
        manifest.ttab_id,
        format!("{} Pie Strings", manifest.name),
        StringFormat::Localized,
        ttas_entries,
    );
    chunks.push(ChunkData {
        tag: *b"TTAs",
        id: ttas.id,
        flags: IFF_CONVERTED_FLAG,
        label: ttas.label.clone(),
        data: encode_string_table(&ttas)?,
    });

    // 7. SLOT
    let slot_entries: Vec<Slot> = manifest
        .slots
        .iter()
        .map(|s| Slot {
            kind: s.kind,
            offset: s.offset,
            standing: s.standing,
            sitting: s.sitting,
            ground: s.ground,
            flags: s.flags,
            snap_target: s.snap_target,
            proximity: s.proximity,
            max_size: s.max_size,
            unknown: 0,
            gradient: s.gradient,
            height: s.height,
            facing: s.facing,
            resolution: s.resolution,
        })
        .collect();
    let slot_chunk = SlotChunk::new(
        manifest.slot_id,
        format!("{} Slots", manifest.name),
        slot_entries,
    );
    chunks.push(ChunkData {
        tag: *b"SLOT",
        id: slot_chunk.id,
        flags: IFF_CONVERTED_FLAG,
        label: slot_chunk.label.clone(),
        data: encode_slot(&slot_chunk)?,
    });

    // 8. Procedural isometric sprites: PALT + SPR2 + DGRP
    let sprites = generate_furniture_sprites(
        &manifest.sprite,
        manifest.palt_id,
        manifest.spr2_id,
        manifest.dgrp_id,
        &manifest.name,
    );
    chunks.push(ChunkData {
        tag: *b"PALT",
        id: sprites.palt.id,
        flags: IFF_CONVERTED_FLAG,
        label: sprites.palt.label.clone(),
        data: encode_palt(&sprites.palt)?,
    });
    chunks.push(ChunkData {
        tag: *b"SPR2",
        id: sprites.spr2.id,
        flags: IFF_CONVERTED_FLAG,
        label: sprites.spr2.label.clone(),
        data: encode_spr2(&sprites.spr2)?,
    });
    chunks.push(ChunkData {
        tag: *b"DGRP",
        id: sprites.dgrp.id,
        flags: IFF_CONVERTED_FLAG,
        label: sprites.dgrp.label.clone(),
        data: encode_dgrp(&sprites.dgrp)?,
    });

    // 9. FWAV + synthesized WAV audio assets
    let mut sounds_wav = BTreeMap::new();
    for sound in &manifest.sounds {
        let fwav = FwavChunk {
            id: sound.id,
            label: sound.name.clone(),
            sound_name: sound.name.clone(),
        };
        chunks.push(ChunkData {
            tag: *b"FWAV",
            id: fwav.id,
            flags: IFF_CONVERTED_FLAG,
            label: fwav.label.clone(),
            data: encode_fwav(&fwav)?,
        });
        sounds_wav.insert(sound.name.clone(), synthesize_preset(sound.preset));
    }

    let iff_bytes = encode_iff_with_rsmp(&chunks)?;
    Ok(CompiledObject {
        guid: manifest.guid,
        name: manifest.name,
        iff_bytes,
        sounds_wav,
        behaviors,
    })
}

/// Compiles an `object.toml` and its referenced `.antics` scripts from a directory on disk.
pub fn compile_object_from_dir(dir: &Path) -> Result<CompiledObject> {
    let manifest_path = dir.join("object.toml");
    let manifest_toml = std::fs::read_to_string(&manifest_path)
        .map_err(|e| Error::new(format!("failed to read {}: {e}", manifest_path.display())))?;
    let parsed = ObjectManifest::parse(&manifest_toml)?;
    let mut owned_scripts = Vec::with_capacity(parsed.scripts.len());
    for script in &parsed.scripts {
        let path = dir.join(script);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| Error::new(format!("failed to read {}: {e}", path.display())))?;
        owned_scripts.push((script.clone(), text));
    }
    let refs: Vec<(&str, &str)> = owned_scripts
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    compile_object_from_sources(&manifest_toml, &refs)
}

/// Packages multiple compiled `.iff` (or arbitrary) members into a `FAR!byAZ` archive.
pub fn package_far(entries: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    let far_entries: Vec<FarEntry<'_>> = entries
        .iter()
        .map(|&(name, data)| FarEntry { name, data })
        .collect();
    encode_far(&far_entries)
}
