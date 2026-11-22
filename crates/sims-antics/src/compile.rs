//! End-to-end compilation pipeline turning `.antics` + `object.toml` + `pack.toml`
//! into complete IFF 2.5 objects, synthesized WAVs, and `FAR!byAZ` archives.

use crate::art::generate_furniture_sprites;
use crate::audio::synthesize_preset;
use crate::encode::{
    encode_bcon, encode_bhav, encode_dgrp, encode_far, encode_fwav, encode_iff_with_rsmp,
    encode_objd, encode_objf, encode_palt, encode_slot, encode_spr2, encode_string_table,
    encode_ttab,
};
use crate::error::{Error, Result};
use crate::ir::{
    BconChunk, FarEntry, FwavChunk, IFF_CONVERTED_FLAG, ObjdSpec, ObjfChunk, SlotChunk,
    StringEntry, StringFormat, StringTableChunk, TtabChunk,
};
use crate::lower::{LoweredModule, lower_module};
use crate::manifest::{ObjectManifest, PackManifest, SpriteSpec, TreeRef};
use crate::parser::parse_module;
use sims_formats::behavior::{Interaction, MotiveAdvertisement};
use sims_formats::execution::{ObjectFunction, Slot};
use sims_formats::iff::ChunkData;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Compiled standalone `.antics` module.
pub type CompiledModule = LoweredModule;

/// A fully compiled clean-room Sims 1 object (`.iff` bytes + synthesized WAVs).
#[derive(Debug, Clone)]
pub struct CompiledObject {
    pub guid: u32,
    /// Additional tile GUIDs emitted for multi-tile objects (empty for single-tile objects).
    pub tile_guids: Vec<u32>,
    pub name: String,
    pub iff_bytes: Vec<u8>,
    /// Synthesized RIFF/WAVE sound effects keyed by FWAV sound event name.
    pub sounds_wav: BTreeMap<String, Vec<u8>>,
    pub behaviors: BTreeMap<u16, crate::ir::BhavChunk>,
}

/// One compiled object entry inside a [`CompiledPack`].
#[derive(Debug, Clone)]
pub struct CompiledPackObject {
    pub slug: String,
    pub iff_name: String,
    pub object: CompiledObject,
}

/// A fully compiled clean-room asset pack (`open-pack.far` + individual IFFs + WAV bank).
#[derive(Debug, Clone)]
pub struct CompiledPack {
    pub name: String,
    pub version: String,
    pub archive_name: String,
    pub objects: Vec<CompiledPackObject>,
    pub far_bytes: Vec<u8>,
    /// Deterministic synthesized WAV sound bank keyed by FWAV sound event name.
    pub sounds_wav: BTreeMap<String, Vec<u8>>,
}

/// Parses and lowers one `.antics` source string into validated `BHAV`/`BCON`/`STR#` chunks.
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
    let mut combined_items = Vec::new();
    for &(filename, src) in antics_sources {
        let parsed = parse_module(src).map_err(|e| e.with_context(filename))?;
        combined_items.extend(parsed.items);
    }
    let lowered = lower_module(&crate::ast::Module {
        items: combined_items,
    })
    .map_err(|e| {
        if antics_sources.len() == 1 {
            e.with_context(antics_sources[0].0)
        } else {
            e.with_context(&manifest.name)
        }
    })?;
    let behaviors = lowered.behaviors;
    let tree_ids_by_name = lowered.tree_ids_by_name;
    let mut constants = lowered.constants;
    let mut strings = lowered.strings;

    // Private object IFFs are only addressed via private BCON range 4096..=4159 by sims-vm
    // (`Program::constant` maps 256..=319 to Global.iff, 8192..=8255 to semi-globals, and
    // cannot address any chunk ID outside 4096..=4159 in a private object IFF).
    for &bcon_id in constants.keys() {
        if !(4096..=4159).contains(&bcon_id) {
            return Err(Error::new(format!(
                "object '{}' declares unreachable private BCON id {bcon_id} (private object \
                 BCON tables must use IDs in 4096..=4159)",
                manifest.name
            )));
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

    // Helper to build an OBJf entry table for a given OBJD chunk ID.
    let build_objf_entries = |entry_init: u16, entry_main: u16| -> Result<Vec<ObjectFunction>> {
        let min_callbacks = manifest
            .callbacks
            .iter()
            .map(|cb| cb.index + 1)
            .max()
            .unwrap_or(0)
            .max(2);
        let mut entries = vec![
            ObjectFunction {
                condition: 0,
                action: 0,
            };
            min_callbacks
        ];
        entries[0].action = entry_init;
        entries[1].action = entry_main;
        for cb in &manifest.callbacks {
            if let Some(action) = &cb.action {
                entries[cb.index].action = resolve_tree(action)?;
            }
            if let Some(condition) = &cb.condition {
                entries[cb.index].condition = resolve_tree(condition)?;
            }
        }
        Ok(entries)
    };

    // Track per-tile sprite sets so each multi-tile sub-object can emit its own distinct
    // PALT/SPR2/DGRP while validating every tile resource reference against emitted chunks.
    let mut extra_tile_sprites: Vec<(u16, String, SpriteSpec)> = Vec::new();
    let mut emitted_dgrp_ids = BTreeSet::from([manifest.dgrp_id]);
    let mut emitted_spr2_ids = BTreeSet::from([manifest.spr2_id]);
    let mut emitted_palt_ids = BTreeSet::from([manifest.palt_id]);

    // 1. OBJD + OBJf (single-tile or multi-tile master + tile definitions)
    let mut tile_guids = Vec::with_capacity(manifest.tiles.len());
    if manifest.tiles.is_empty() {
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
        objd_spec.master_id = manifest.master_id.unwrap_or(0);
        objd_spec.num_attributes = manifest.attributes;
        objd_spec.interaction_group = manifest.interaction_group;
        objd_spec.data_58 = manifest.data_58;
        chunks.push(ChunkData {
            tag: *b"OBJD",
            id: manifest.objd_id,
            flags: IFF_CONVERTED_FLAG,
            label: manifest.name.clone(),
            data: encode_objd(&objd_spec.to_chunk())?,
        });

        let objf = ObjfChunk::new(
            manifest.objd_id,
            format!("{} Functions", manifest.name),
            build_objf_entries(init_id, main_id)?,
        );
        chunks.push(ChunkData {
            tag: *b"OBJf",
            id: objf.id,
            flags: IFF_CONVERTED_FLAG,
            label: objf.label.clone(),
            data: encode_objf(&objf)?,
        });
    } else {
        let group_id = manifest.master_id.unwrap_or(1);
        if group_id == 0 {
            return Err(Error::new(format!(
                "multi-tile object '{}' must have a non-zero master_id",
                manifest.name
            )));
        }
        let explicit_leads = manifest.tiles.iter().filter(|t| t.lead).count();
        if explicit_leads > 1 {
            return Err(Error::new(format!(
                "multi-tile object '{}' declares {explicit_leads} lead tiles (at most 1 allowed)",
                manifest.name
            )));
        }
        let has_origin_lead = manifest
            .tiles
            .iter()
            .any(|t| t.lead || (t.sub_index == 0 && t.level == 0));
        if !has_origin_lead {
            return Err(Error::new(format!(
                "multi-tile object '{}' must mark one tile with lead = true or place a tile at [0, 0] level 0",
                manifest.name
            )));
        }

        let mut seen_guids = BTreeSet::from([manifest.guid]);
        let mut seen_objd_ids = BTreeSet::from([manifest.objd_id]);
        let mut seen_tile_coords = BTreeSet::new();

        // Catalog master OBJD (sub_index = -1, never instantiated directly; factory expands tiles).
        let mut master_spec = ObjdSpec::new(manifest.objd_id, &manifest.name, manifest.guid);
        master_spec.price = manifest.price;
        master_spec.object_type = manifest.object_type;
        master_spec.base_graphic_id = manifest.dgrp_id;
        master_spec.num_graphics = 1;
        master_spec.tree_table_id = manifest.ttab_id;
        master_spec.slots_id = manifest.slot_id;
        master_spec.catalog_strings_id = manifest.ctss_id;
        master_spec.init_id = init_id;
        master_spec.main_id = main_id;
        master_spec.master_id = group_id;
        master_spec.sub_index = -1;
        master_spec.lead_tile = 0;
        master_spec.num_attributes = manifest.attributes;
        master_spec.interaction_group = manifest.interaction_group;
        master_spec.data_58 = manifest.data_58;
        chunks.push(ChunkData {
            tag: *b"OBJD",
            id: master_spec.id,
            flags: IFF_CONVERTED_FLAG,
            label: master_spec.label.clone(),
            data: encode_objd(&master_spec.to_chunk())?,
        });
        let master_objf = ObjfChunk::new(
            master_spec.id,
            format!("{} Functions", manifest.name),
            build_objf_entries(init_id, main_id)?,
        );
        chunks.push(ChunkData {
            tag: *b"OBJf",
            id: master_objf.id,
            flags: IFF_CONVERTED_FLAG,
            label: master_objf.label.clone(),
            data: encode_objf(&master_objf)?,
        });

        // Pass 1: Register all per-tile sprite shapes first so shared tile DGRP references
        // are independent of [[tile]] declaration order.
        let mut resolved_tile_dgrps: Vec<Option<u16>> = vec![None; manifest.tiles.len()];
        for (idx, tile) in manifest.tiles.iter().enumerate() {
            let tile_label = tile
                .name
                .clone()
                .unwrap_or_else(|| format!("{} Tile {}", manifest.name, idx));
            match (tile.dgrp_id, tile.shape) {
                (Some(dgrp_id), Some(shape)) => {
                    if dgrp_id == 0 {
                        return Err(Error::new(format!(
                            "tile '{tile_label}' cannot combine dgrp_id = 0 with a sprite shape"
                        )));
                    }
                    if !emitted_dgrp_ids.insert(dgrp_id)
                        || !emitted_spr2_ids.insert(dgrp_id)
                        || !emitted_palt_ids.insert(dgrp_id)
                    {
                        return Err(Error::new(format!(
                            "tile '{tile_label}' dgrp_id {dgrp_id} collides with an existing PALT/SPR2/DGRP chunk id"
                        )));
                    }
                    extra_tile_sprites.push((
                        dgrp_id,
                        tile_label,
                        SpriteSpec {
                            shape,
                            ..manifest.sprite
                        },
                    ));
                    resolved_tile_dgrps[idx] = Some(dgrp_id);
                }
                (None, Some(shape)) => {
                    let dgrp_id = manifest
                        .dgrp_id
                        .checked_add(1 + idx as u16)
                        .ok_or_else(|| Error::new("multi-tile dgrp_id overflow"))?;
                    if !emitted_dgrp_ids.insert(dgrp_id)
                        || !emitted_spr2_ids.insert(dgrp_id)
                        || !emitted_palt_ids.insert(dgrp_id)
                    {
                        return Err(Error::new(format!(
                            "auto-assigned tile dgrp_id {dgrp_id} for '{tile_label}' collides with an existing sprite chunk id"
                        )));
                    }
                    extra_tile_sprites.push((
                        dgrp_id,
                        tile_label,
                        SpriteSpec {
                            shape,
                            ..manifest.sprite
                        },
                    ));
                    resolved_tile_dgrps[idx] = Some(dgrp_id);
                }
                (Some(_), None) | (None, None) => {}
            }
        }

        // Pass 2: Validate every tile's DGRP/TTAB/SLOT references against all emitted chunks
        // and emit per-tile OBJD + OBJf chunks.
        for (idx, tile) in manifest.tiles.iter().enumerate() {
            if !seen_guids.insert(tile.guid) {
                return Err(Error::new(format!(
                    "duplicate GUID {:#010x} across master/tiles in '{}'",
                    tile.guid, manifest.name
                )));
            }
            let tile_objd_id = match tile.objd_id {
                Some(id) => id,
                None => manifest
                    .objd_id
                    .checked_add(1 + idx as u16)
                    .ok_or_else(|| Error::new("multi-tile objd_id overflow"))?,
            };
            if !seen_objd_ids.insert(tile_objd_id) {
                return Err(Error::new(format!(
                    "duplicate OBJD id {tile_objd_id} across master/tiles in '{}'",
                    manifest.name
                )));
            }
            if !seen_tile_coords.insert((tile.sub_index, tile.level)) {
                return Err(Error::new(format!(
                    "duplicate tile position (sub_index {}, level {}) in '{}'",
                    tile.sub_index, tile.level, manifest.name
                )));
            }
            let tile_label = tile
                .name
                .clone()
                .unwrap_or_else(|| format!("{} Tile {}", manifest.name, idx));

            let tile_dgrp_id = if let Some(dgrp_id) = resolved_tile_dgrps[idx] {
                dgrp_id
            } else if let Some(dgrp_id) = tile.dgrp_id {
                if dgrp_id != 0 && !emitted_dgrp_ids.contains(&dgrp_id) {
                    return Err(Error::new(format!(
                        "tile '{tile_label}' references unemitted dgrp_id {dgrp_id} (set [[tile]].shape to emit a distinct tile sprite, or use 0 / {})",
                        manifest.dgrp_id
                    )));
                }
                dgrp_id
            } else {
                manifest.dgrp_id
            };

            let tile_ttab_id = match tile.ttab_id {
                None => manifest.ttab_id,
                Some(id) if id == 0 || id == 0xFFFF || id == manifest.ttab_id => id,
                Some(id) => {
                    return Err(Error::new(format!(
                        "tile '{tile_label}' references unemitted ttab_id {id} (valid: 0, 0xFFFF, or {})",
                        manifest.ttab_id
                    )));
                }
            };

            let tile_slot_id = match tile.slot_id {
                None => manifest.slot_id,
                Some(id) if id == 0 || id == manifest.slot_id => id,
                Some(id) => {
                    return Err(Error::new(format!(
                        "tile '{tile_label}' references unemitted slot_id {id} (valid: 0 or {})",
                        manifest.slot_id
                    )));
                }
            };

            let tile_init = match &tile.init {
                Some(r) => resolve_tree(r)?,
                None => init_id,
            };
            let tile_main = match &tile.main {
                Some(r) => resolve_tree(r)?,
                None => main_id,
            };
            let is_lead =
                tile.lead || (explicit_leads == 0 && tile.sub_index == 0 && tile.level == 0);
            let mut tile_spec = ObjdSpec::new(tile_objd_id, &tile_label, tile.guid);
            tile_spec.price = 0;
            tile_spec.object_type = manifest.object_type;
            tile_spec.base_graphic_id = tile_dgrp_id;
            tile_spec.num_graphics = u16::from(tile_dgrp_id != 0);
            tile_spec.tree_table_id = tile_ttab_id;
            tile_spec.slots_id = tile_slot_id;
            tile_spec.catalog_strings_id = manifest.ctss_id;
            tile_spec.init_id = tile_init;
            tile_spec.main_id = tile_main;
            tile_spec.master_id = group_id;
            tile_spec.sub_index = tile.sub_index;
            tile_spec.level = tile.level;
            tile_spec.lead_tile = u16::from(is_lead);
            tile_spec.interaction_group =
                tile.interaction_group
                    .unwrap_or(if is_lead { -1 } else { 1 });
            tile_spec.num_attributes = tile.attributes.unwrap_or(manifest.attributes);
            tile_spec.data_58 = manifest.data_58;

            chunks.push(ChunkData {
                tag: *b"OBJD",
                id: tile_spec.id,
                flags: IFF_CONVERTED_FLAG,
                label: tile_spec.label.clone(),
                data: encode_objd(&tile_spec.to_chunk())?,
            });
            let tile_objf = ObjfChunk::new(
                tile_spec.id,
                format!("{tile_label} Functions"),
                build_objf_entries(tile_init, tile_main)?,
            );
            chunks.push(ChunkData {
                tag: *b"OBJf",
                id: tile_objf.id,
                flags: IFF_CONVERTED_FLAG,
                label: tile_objf.label.clone(),
                data: encode_objf(&tile_objf)?,
            });
            tile_guids.push(tile.guid);
        }
    }

    // Merge manifest [[constant]] tables into BCON map (rejecting duplicates and unreachable IDs).
    for c in &manifest.constants {
        if !(4096..=4159).contains(&c.id) {
            return Err(Error::new(format!(
                "[[constant]] id {} in '{}' is outside private BCON range 4096..=4159",
                c.id, manifest.name
            )));
        }
        let chunk = BconChunk {
            id: c.id,
            label: c.label.clone(),
            flags: c.flags,
            values: c.values.clone(),
            trailing: Vec::new(),
        };
        if constants.insert(chunk.id, chunk).is_some() {
            return Err(Error::new(format!(
                "duplicate BCON id {} between scripts and [[constant]] in '{}'",
                c.id, manifest.name
            )));
        }
    }

    // Merge manifest [[string_table]] chunks into STR# map (rejecting duplicates).
    for st in &manifest.string_tables {
        let entries = st.entries.iter().map(StringEntry::english).collect();
        let chunk =
            StringTableChunk::new(*b"STR#", st.id, &st.label, StringFormat::Localized, entries);
        if strings.insert(chunk.id, chunk).is_some() {
            return Err(Error::new(format!(
                "duplicate STR# id {} between scripts and [[string_table]] in '{}'",
                st.id, manifest.name
            )));
        }
    }

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

    // 5b. STR# chunks
    for str_chunk in strings.values() {
        chunks.push(ChunkData {
            tag: *b"STR#",
            id: str_chunk.id,
            flags: IFF_CONVERTED_FLAG,
            label: str_chunk.label.clone(),
            data: encode_string_table(str_chunk)?,
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

    // 8. Procedural isometric sprites: PALT + SPR2 + DGRP (base + any per-tile overrides)
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
    for (tile_sprite_id, tile_label, tile_sprite_spec) in &extra_tile_sprites {
        let tile_sprites = generate_furniture_sprites(
            tile_sprite_spec,
            *tile_sprite_id,
            *tile_sprite_id,
            *tile_sprite_id,
            tile_label,
        );
        chunks.push(ChunkData {
            tag: *b"PALT",
            id: tile_sprites.palt.id,
            flags: IFF_CONVERTED_FLAG,
            label: tile_sprites.palt.label.clone(),
            data: encode_palt(&tile_sprites.palt)?,
        });
        chunks.push(ChunkData {
            tag: *b"SPR2",
            id: tile_sprites.spr2.id,
            flags: IFF_CONVERTED_FLAG,
            label: tile_sprites.spr2.label.clone(),
            data: encode_spr2(&tile_sprites.spr2)?,
        });
        chunks.push(ChunkData {
            tag: *b"DGRP",
            id: tile_sprites.dgrp.id,
            flags: IFF_CONVERTED_FLAG,
            label: tile_sprites.dgrp.label.clone(),
            data: encode_dgrp(&tile_sprites.dgrp)?,
        });
    }

    // 9. FWAV + synthesized WAV audio assets (rejecting duplicate IDs or names)
    let mut sounds_wav = BTreeMap::new();
    let mut seen_sound_ids = BTreeSet::new();
    let mut seen_sound_lower = BTreeSet::new();
    for sound in &manifest.sounds {
        if !seen_sound_ids.insert(sound.id) {
            return Err(Error::new(format!(
                "duplicate [[sound]] id {} in '{}'",
                sound.id, manifest.name
            )));
        }
        if !seen_sound_lower.insert(sound.name.to_ascii_lowercase()) {
            return Err(Error::new(format!(
                "duplicate/case-colliding [[sound]] name '{}' in '{}'",
                sound.name, manifest.name
            )));
        }
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
        tile_guids,
        name: manifest.name,
        iff_bytes,
        sounds_wav,
        behaviors,
    })
}

/// Verifies that `child` inside `root` has no symlink along the relative path and
/// canonicalizes strictly within `root_canon`.
fn ensure_contained_path(
    root: &Path,
    root_canon: &Path,
    rel_components: &[&str],
) -> Result<PathBuf> {
    let mut current = root.to_path_buf();
    for &comp in rel_components {
        current.push(comp);
        let meta = std::fs::symlink_metadata(&current)
            .map_err(|e| Error::new(format!("failed to access {}: {e}", current.display())))?;
        if meta.file_type().is_symlink() {
            return Err(Error::new(format!(
                "refusing to follow symlink inside pack root: {}",
                current.display()
            )));
        }
    }
    let canon = std::fs::canonicalize(&current)
        .map_err(|e| Error::new(format!("failed to resolve {}: {e}", current.display())))?;
    if !canon.starts_with(root_canon) {
        return Err(Error::new(format!(
            "path {} escapes root {}",
            current.display(),
            root_canon.display()
        )));
    }
    Ok(canon)
}

fn read_contained_file(root: &Path, root_canon: &Path, rel_components: &[&str]) -> Result<String> {
    use std::io::Read;

    let canon = ensure_contained_path(root, root_canon, rel_components)?;
    let pre_meta = std::fs::symlink_metadata(&canon)
        .map_err(|e| Error::new(format!("failed to inspect {}: {e}", canon.display())))?;
    if pre_meta.file_type().is_symlink() || !pre_meta.file_type().is_file() {
        return Err(Error::new(format!(
            "expected regular file at {}",
            canon.display()
        )));
    }
    let mut file = std::fs::File::open(&canon)
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

/// Compiles an `object.toml` and its referenced `.antics` scripts from a directory on disk,
/// enforcing symlink-free containment within `dir`.
pub fn compile_object_dir(dir: &Path) -> Result<CompiledObject> {
    let root_meta = std::fs::symlink_metadata(dir)
        .map_err(|e| Error::new(format!("failed to access {}: {e}", dir.display())))?;
    if root_meta.file_type().is_symlink() || !root_meta.file_type().is_dir() {
        return Err(Error::new(format!(
            "object directory {} must be a real directory, not a symlink",
            dir.display()
        )));
    }
    let root_canon = std::fs::canonicalize(dir)
        .map_err(|e| Error::new(format!("failed to resolve {}: {e}", dir.display())))?;
    let manifest_toml = read_contained_file(dir, &root_canon, &["object.toml"])?;
    let parsed = ObjectManifest::parse(&manifest_toml)
        .map_err(|e| e.with_context(dir.join("object.toml").display()))?;
    let mut owned_scripts = Vec::with_capacity(parsed.scripts.len());
    for script in &parsed.scripts {
        let text = read_contained_file(dir, &root_canon, &[script.as_str()])?;
        owned_scripts.push((script.clone(), text));
    }
    let refs: Vec<(&str, &str)> = owned_scripts
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    compile_object_from_sources(&manifest_toml, &refs)
}

/// Backwards-compatible alias for [`compile_object_dir`].
pub fn compile_object_from_dir(dir: &Path) -> Result<CompiledObject> {
    compile_object_dir(dir)
}

/// Compiles an entire clean-room pack directory containing `pack.toml` and
/// `objects/<slug>/{object.toml, *.antics}` into a packaged `FAR!byAZ` archive
/// and deterministic synthesized WAV sound bank.
pub fn compile_pack_dir(dir: &Path) -> Result<CompiledPack> {
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
    let pack_toml = read_contained_file(dir, &root_canon, &["pack.toml"])?;
    let manifest = PackManifest::parse(&pack_toml)
        .map_err(|e| e.with_context(dir.join("pack.toml").display()))?;

    let mut objects = Vec::with_capacity(manifest.objects.len());
    let mut seen_guids = BTreeMap::new();
    let mut seen_iff_lower = BTreeSet::new();
    let mut sounds_wav: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut seen_sound_lower: BTreeMap<String, String> = BTreeMap::new();

    for slug in &manifest.objects {
        let iff_name = format!("{}.iff", slug.replace('-', "_"));
        if !seen_iff_lower.insert(iff_name.to_ascii_lowercase()) {
            return Err(Error::new(format!(
                "object slug '{slug}' produces colliding IFF filename '{iff_name}'"
            )));
        }
        let obj_dir = ensure_contained_path(dir, &root_canon, &["objects", slug.as_str()])?;
        let compiled =
            compile_object_dir(&obj_dir).map_err(|e| e.with_context(format!("object '{slug}'")))?;
        for guid in std::iter::once(compiled.guid).chain(compiled.tile_guids.iter().copied()) {
            if let Some(prev_slug) = seen_guids.insert(guid, slug.clone()) {
                return Err(Error::new(format!(
                    "duplicate GUID {guid:#010x} between objects '{prev_slug}' and '{slug}'"
                )));
            }
        }
        for (sound_name, wav_bytes) in &compiled.sounds_wav {
            let lower = sound_name.to_ascii_lowercase();
            if let Some(prev_name) = seen_sound_lower.get(&lower)
                && prev_name != sound_name
            {
                return Err(Error::new(format!(
                    "case-colliding sound names '{prev_name}' and '{sound_name}' in '{slug}'"
                )));
            }
            seen_sound_lower.insert(lower, sound_name.clone());
            if let Some(existing) = sounds_wav.get(sound_name) {
                if existing != wav_bytes {
                    return Err(Error::new(format!(
                        "conflicting synthesized audio for sound name '{sound_name}' in '{slug}'"
                    )));
                }
            } else {
                sounds_wav.insert(sound_name.clone(), wav_bytes.clone());
            }
        }
        objects.push(CompiledPackObject {
            slug: slug.clone(),
            iff_name,
            object: compiled,
        });
    }

    let far_members: Vec<(&str, &[u8])> = objects
        .iter()
        .map(|o| (o.iff_name.as_str(), o.object.iff_bytes.as_slice()))
        .collect();
    let far_bytes = package_far(&far_members)?;

    Ok(CompiledPack {
        name: manifest.name,
        version: manifest.version,
        archive_name: manifest.archive,
        objects,
        far_bytes,
        sounds_wav,
    })
}

/// Packages multiple compiled `.iff` (or arbitrary) members into a `FAR!byAZ` archive.
pub fn package_far(entries: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    let far_entries: Vec<FarEntry<'_>> = entries
        .iter()
        .map(|&(name, data)| FarEntry { name, data })
        .collect();
    encode_far(&far_entries)
}
