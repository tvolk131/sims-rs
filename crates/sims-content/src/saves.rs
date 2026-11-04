//! Read-only original house/household import, before runtime activation.
//! Resolving a character is neighborhood-scoped; a user00001 in another
//! neighborhood must never replace this household's resident.
use crate::Index;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub use sims_formats::lot::WallTile;
use sims_formats::{iff::Iff, lot, neighborhood, saved_objects, strings::StringTable};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefinitionSource {
    pub source: String,
    pub sha256: String,
    pub chunk: u16,
    pub kind: u16,
    pub multipart: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resident {
    pub guid: u32,
    pub neighbor: i16,
    pub object: u16,
    pub name: String,
    pub character_source: String,
    /// Original STR# body data, retained even when a saved appearance exists.
    pub body_strings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Household {
    pub id: u16,
    pub name: String,
    pub family: neighborhood::Family,
    pub residents: Vec<Resident>,
}
pub use sims_sim::lot_geometry::{
    Floor as FloorGeometry, FloorSource as Floor, Lot as LotGeometry,
};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingTileHead {
    pub floor: i16,
    pub x: usize,
    pub y: usize,
    pub object: u16,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct House {
    pub source: String,
    pub number: i16,
    pub neighborhood_source: String,
    pub sources: BTreeMap<String, String>,
    pub width: usize,
    pub height: usize,
    pub floors: Vec<Floor>,
    pub floor_map: Vec<lot::SurfaceMapping>,
    pub wall_map: Vec<lot::SurfaceMapping>,
    pub arrays: BTreeMap<u16, lot::Array>,
    pub house: lot::House,
    pub view: lot::HouseView,
    pub simulator: lot::Simulator,
    pub types: lot::ObjectTypes,
    pub objects: saved_objects::Objects,
    pub definitions: BTreeMap<u32, DefinitionSource>,
    pub neighbors: neighborhood::Neighbors,
    pub neighborhood: neighborhood::Neighborhood,
    pub resource_storage: Option<sims_formats::tatt::Tatt>,
    pub families: BTreeMap<u16, neighborhood::Family>,
    pub household: Option<Household>,
    /// Original grids can contain stale references. Retain them during parsing;
    /// native load repair (0x00443cb3) belongs to the activation phase.
    pub missing_tile_heads: Vec<MissingTileHead>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Compatibility {
    pub type_id: u16,
    pub guid: u32,
    pub source: String,
    pub init: u16,
    pub main: u16,
    pub saved: [u32; 2],
    pub current: [u32; 2],
    pub changed: bool,
}

fn required<'a>(iff: &'a Iff, tag: &[u8; 4], id: u16) -> Result<&'a [u8]> {
    Ok(iff.data(
        iff.get(tag, id)
            .with_context(|| format!("missing {} {id}", String::from_utf8_lossy(tag)))?,
    ))
}
fn text(iff: &Iff, tag: &[u8; 4], id: u16) -> Result<Vec<String>> {
    let Some(c) = iff.get(tag, id) else {
        return Ok(Vec::new());
    };
    Ok(StringTable::parse(iff.data(c))?
        .entries
        .into_iter()
        .filter(|s| s.language == 1)
        .map(|s| s.text)
        .collect())
}
fn read(index: &Index, source: &str, sources: &mut BTreeMap<String, String>) -> Result<Iff> {
    let id = index.find(source)?;
    let data = index.resources[id].read()?;
    sources.insert(
        index.resources[id].locator.clone(),
        format!("{:x}", Sha256::digest(&data)),
    );
    Ok(Iff::parse(data)?)
}

/// Resolve each GUID to its original resource. Duplicate providers are rejected,
/// except byte-identical resources, and neighborhood character files take
/// precedence over unrelated global content with the same GUID.
pub fn definitions(
    index: &Index,
    neighborhood: &str,
    guids: &BTreeSet<u32>,
) -> Result<BTreeMap<u32, DefinitionSource>> {
    let character_prefix = format!("{neighborhood}/characters/").to_ascii_lowercase();
    let mut candidates = BTreeMap::<u32, Vec<(DefinitionSource, bool)>>::new();
    for resource in &index.resources {
        if !resource.is_iff() {
            continue;
        }
        let location = resource.locator.to_ascii_lowercase();
        let is_local = location.starts_with(&character_prefix);
        if location.contains("/characters/") && !is_local {
            continue;
        }
        let iff = Iff::parse(resource.read()?)?;
        let mut sha256 = None;
        for c in iff.all(b"OBJD") {
            let b = iff.data(c);
            if b.len() < 32 {
                continue;
            }
            let guid = u32::from_le_bytes(b[28..32].try_into().unwrap());
            if !guids.contains(&guid) {
                continue;
            }
            let def = DefinitionSource {
                source: resource.locator.clone(),
                sha256: sha256
                    .get_or_insert_with(|| format!("{:x}", Sha256::digest(&iff.bytes)))
                    .clone(),
                chunk: c.id,
                kind: u16::from_le_bytes(b[18..20].try_into().unwrap()),
                multipart: u16::from_le_bytes(b[20..22].try_into().unwrap()) != 0,
            };
            candidates.entry(guid).or_default().push((def, is_local));
        }
    }
    let mut result = BTreeMap::new();
    for guid in guids {
        let mut choices = candidates
            .remove(guid)
            .with_context(|| format!("missing original object GUID {guid:08x}"))?;
        if choices.iter().any(|c| c.1) {
            choices.retain(|c| c.1);
        }
        ensure!(
            choices
                .iter()
                .all(|c| c.0.sha256 == choices[0].0.sha256 && c.0.chunk == choices[0].0.chunk),
            "ambiguous original object {guid:08x}: {:?}",
            choices.iter().map(|c| &c.0.source).collect::<Vec<_>>()
        );
        result.insert(*guid, choices.remove(0).0);
    }
    Ok(result)
}

impl House {
    /// Native wall queries over an original floor, retaining column-major cells.
    pub fn wall_grid(&self, level: i32) -> Result<sims_sim::lot_walls::Grid<'_>> {
        ensure!(
            self.width == self.height,
            "original wall grid must be square"
        );
        let floor = level
            .checked_sub(1)
            .and_then(|n| usize::try_from(n).ok())
            .and_then(|n| self.floors.get(n))
            .context("original floor not present")?;
        sims_sim::lot_walls::Grid::new(self.width, &floor.walls).map_err(anyhow::Error::msg)
    }
    pub fn pool_cells(&self) -> Result<&[u8]> {
        let pool = self
            .arrays
            .get(&9)
            .context("original lot has no pool map")?;
        ensure!(pool.stride == 1, "unsupported original pool-map stride");
        Ok(&pool.data)
    }
    pub fn corner_heights(&self) -> Result<Vec<[u8; 4]>> {
        let altitude = self
            .arrays
            .get(&0)
            .context("original lot has no altitude map")?;
        ensure!(altitude.stride == 4, "unsupported original altitude stride");
        Ok(altitude
            .data
            .chunks_exact(4)
            .map(|p| p.try_into().unwrap())
            .collect())
    }
    pub fn water_cells(&self) -> Result<&[u8]> {
        let water = self
            .arrays
            .get(&10)
            .context("original lot has no water map")?;
        ensure!(water.stride == 1, "unsupported original water-map stride");
        Ok(&water.data)
    }
    /// First native room pass, including the saved pool map at world+0x60.
    pub fn room_partition(&self, level: i32) -> Result<sims_sim::lot_rooms::Partition> {
        self.wall_grid(level)?;
        let floor = &self.floors[(level - 1) as usize];
        sims_sim::lot_rooms::partition(
            self.width,
            level,
            &floor.walls,
            &floor.coverings,
            Some(self.pool_cells()?),
        )
        .map_err(anyhow::Error::msg)
    }
    /// Native partition/exterior/build-support passes in floor order. Supply
    /// object heads after global reconciliation and native ground-grid repair.
    /// Observer notifications and the subsequent roof/render rebuild are not
    /// replayed here; this returns geometry for the spatial activation host.
    pub fn lot_geometry(&self, heads: &[Vec<u16>]) -> Result<LotGeometry> {
        self.lot_geometry_inner(heads, None)
    }
    ///The retained world+10 owner controls the scan; saved arrays keep their
    ///allocated stride. Do not reconstruct an absent owner from array width.
    pub fn lot_geometry_in_active(&self, heads: &[Vec<u16>], active: usize) -> Result<LotGeometry> {
        self.lot_geometry_inner(heads, Some(active))
    }
    fn lot_geometry_inner(&self, heads: &[Vec<u16>], active: Option<usize>) -> Result<LotGeometry> {
        ensure!(
            heads.len() == self.floors.len(),
            "incomplete repaired object grids"
        );
        let heights = self.corner_heights()?;
        let pool = self.pool_cells()?.to_vec();
        let water = self.water_cells()?.to_vec();
        let mut sources = self.floors.clone();
        for (floor, heads) in sources.iter_mut().zip(heads) {
            floor.object_heads.clone_from(heads);
        }
        match active {
            Some(active) => {
                LotGeometry::rebuild_in_active(self.width, active, heights, pool, water, sources)
            }
            None => LotGeometry::rebuild(self.width, heights, pool, water, sources),
        }
        .map_err(anyhow::Error::msg)
    }
    /// Link the exact saved GUID providers, including each character's private
    /// scripts. No fixture person or furniture module supplies their behavior.
    pub fn link_program(&self, index: &Index) -> Result<sims_vm::Program> {
        let mut program = sims_vm::Program::default();
        // Match the explicit English resource-selection policy of load_program.
        program.text_language = Some(1);
        program.careers = Some(crate::careers::load(index)?);
        let sources: BTreeSet<_> = self.definitions.values().map(|d| &d.source).collect();
        for source in sources {
            let loaded = crate::vm::load_program(index, source, "Global.iff")?;
            if program.globals.is_empty() {
                program.globals = loaded.globals;
            }
            for (name, module) in loaded.modules {
                if let Some(old) = program.modules.get(&name) {
                    ensure!(
                        old.content_hash == module.content_hash,
                        "inconsistent module {name}"
                    );
                } else {
                    program.modules.insert(name, module);
                }
            }
        }
        Ok(program)
    }
    /// Load dynamic object/visitor resources in this house's neighborhood scope.
    /// Reuses the import resolver, including its conflicting-provider checks.
    pub fn link_creation_resources(
        &self,
        index: &Index,
        program: &mut sims_vm::Program,
        guids: &BTreeSet<u32>,
    ) -> Result<()> {
        link_neighborhood_creation_resources(index, program, &self.neighborhood_source, guids)
    }
    pub fn compatibility(&self, program: &sims_vm::Program) -> Result<Vec<Compatibility>> {
        let mut report = Vec::new();
        for typ in &self.types.entries {
            let def = &self.definitions[&typ.guid];
            let callbacks = program
                .modules
                .get(&def.source)
                .and_then(|m| m.object_functions.get(&def.chunk))
                .with_context(|| format!("missing callback table {:08x}", typ.guid))?;
            let init = callbacks.first().map_or(0, |f| f.action);
            let main = callbacks.get(1).map_or(0, |f| f.action);
            let current = [
                program
                    .behavior_signature(&def.source, init)
                    .map_err(anyhow::Error::msg)?,
                program
                    .behavior_signature(&def.source, main)
                    .map_err(anyhow::Error::msg)?,
            ];
            let saved = [typ.init_signature, typ.main_signature];
            report.push(Compatibility {
                type_id: typ.id,
                guid: typ.guid,
                source: def.source.clone(),
                init,
                main,
                saved,
                current,
                changed: saved != current,
            });
        }
        Ok(report)
    }
    pub fn load(index: &Index, source: &str) -> Result<Self> {
        let selected = &index.resources[index.find(source)?].locator;
        let (neighborhood_dir, filename) = selected
            .rsplit_once("/Houses/")
            .context("choose an original neighborhood/Houses/HouseNN.iff")?;
        let number: i16 = filename
            .strip_prefix("House")
            .and_then(|s| s.strip_suffix(".iff"))
            .context("invalid original house filename")?
            .parse()?;
        let neighborhood_source = format!("{neighborhood_dir}/Neighborhood.iff");
        let mut sources = BTreeMap::new();
        let iff = read(index, selected, &mut sources)?;
        let nh = read(index, &neighborhood_source, &mut sources)?;
        let simulator = lot::Simulator::parse(required(&iff, b"SIMI", 1)?)?;
        let house = lot::House::parse(required(&iff, b"HOUS", 1)?)?;
        let view = lot::HouseView::parse(required(&iff, b"HOUS", 0)?)?;
        let types = lot::ObjectTypes::parse(required(&iff, b"objt", 0)?)?;
        let neighbors = neighborhood::Neighbors::parse(required(&nh, b"NBRS", 1)?)?;
        let neighborhood = neighborhood::Neighborhood::parse(required(&nh, b"NGBH", 1)?)?;
        let resource_storage = nh
            .get(b"TATT", 1)
            .map(|c| sims_formats::tatt::Tatt::parse(nh.data(c)))
            .transpose()?;
        let guids = types.entries.iter().map(|t| t.guid).collect();
        let definitions = definitions(index, neighborhood_dir, &guids)?;
        for def in definitions.values() {
            sources.insert(def.source.clone(), def.sha256.clone());
        }
        let objects = saved_objects::Objects::parse(required(&iff, b"ObjM", 1)?, &types, |guid| {
            definitions.get(&guid).map(|d| d.multipart)
        })?;
        let mut arrays = BTreeMap::new();
        for c in iff.all(b"Arry") {
            ensure!(
                !arrays.contains_key(&c.id),
                "duplicate original array {}",
                c.id
            );
            arrays.insert(c.id, lot::Array::parse(iff.data(c))?);
        }
        let grid = arrays.get(&0).context("missing altitude array")?;
        let (width, height) = (grid.width, grid.height);
        ensure!(
            arrays
                .values()
                .all(|a| a.width == width && a.height == height),
            "inconsistent original grid dimensions"
        );
        let mut floors = Vec::new();
        let modern = simulator.version >= 0x3f;
        let get = |id, stride| -> Result<&lot::Array> {
            let a = arrays
                .get(&id)
                .with_context(|| format!("missing array {id}"))?;
            ensure!(
                a.stride == stride,
                "array {id} has stride {}, expected {stride}",
                a.stride
            );
            Ok(a)
        };
        for floor in 0..2 {
            let base = floor * 100;
            let ground = get(
                base + if modern { 11 } else { 1 },
                if modern { 2 } else { 1 },
            )?;
            let wall = get(
                base + if modern { 12 } else { 2 },
                if modern { 14 } else { 8 },
            )?;
            let coverings = if modern {
                ground
                    .data
                    .chunks_exact(2)
                    .map(|s| u16::from_le_bytes(s.try_into().unwrap()))
                    .collect()
            } else {
                ground.data.iter().map(|&b| u16::from(b)).collect()
            };
            let walls = wall
                .data
                .chunks_exact(wall.stride)
                .map(|b| {
                    let mut values = [0; 6];
                    for (i, v) in values.iter_mut().enumerate() {
                        *v = if modern {
                            u16::from_le_bytes([b[2 + i * 2], b[3 + i * 2]])
                        } else {
                            u16::from(b[2 + i])
                        };
                    }
                    WallTile {
                        segments: b[0],
                        flags: b[1],
                        styles: [values[0], values[1]],
                        patterns: values[2..].try_into().unwrap(),
                    }
                })
                .collect();
            let object_heads = get(base + 3, 2)?
                .data
                .chunks_exact(2)
                .map(|s| u16::from_le_bytes(s.try_into().unwrap()))
                .collect();
            let flags = get(base + 8, 1)?.data.clone();
            floors.push(Floor {
                coverings,
                walls,
                object_heads,
                flags,
            });
        }
        let map = |tag: &[u8; 4], floor| -> Result<_> {
            if let Some(c) = iff.get(tag, u16::from(modern)) {
                Ok(lot::surface_map(iff.data(c), c.id, floor)?)
            } else {
                bail!("missing surface map {}", String::from_utf8_lossy(tag))
            }
        };
        let floor_map = map(b"FLRm", true)?;
        let wall_map = map(b"WALm", false)?;
        let mut missing_tile_heads = Vec::new();
        for (level, floor) in floors.iter().enumerate() {
            for (cell, &object) in floor.object_heads.iter().enumerate() {
                if object != 0 && !objects.instances.contains_key(&object) {
                    missing_tile_heads.push(MissingTileHead {
                        floor: level as i16 + 1,
                        x: cell / height,
                        y: cell % height,
                        object,
                    });
                }
            }
        }
        let mut household = None;
        let mut families = BTreeMap::new();
        for chunk in nh.all(b"FAMI") {
            let mut family = neighborhood::Family::parse(nh.data(chunk))?;
            families.insert(chunk.id, family.clone());
            // Keep raw FAMI in `families`, but select the household after the
            // original reload's missing-neighbor cleanup (0x424c10..0x424c56).
            family
                .members
                .retain(|guid| neighbors.entries.iter().flatten().any(|n| n.guid == *guid));
            if family.house != i32::from(number) || family.members.is_empty() {
                continue;
            }
            ensure!(
                household.is_none(),
                "multiple households assigned to house {number}"
            );
            let name = text(&nh, b"FAMs", chunk.id)?
                .first()
                .cloned()
                .unwrap_or_default();
            let mut residents = Vec::new();
            for &guid in &family.members {
                let saved = objects
                    .instances
                    .values()
                    .find(|o| o.guid == guid && o.person.is_some())
                    .with_context(|| {
                        format!("family member {guid:08x} has no saved person instance")
                    })?;
                let neighbor = neighbors
                    .entries
                    .iter()
                    .flatten()
                    .find(|n| n.guid == guid)
                    .with_context(|| format!("no neighbor for family member {guid:08x}"))?;
                let def = &definitions[&guid];
                let character = read(index, &def.source, &mut sources)?;
                let name = text(&character, b"CTSS", 2000)?
                    .first()
                    .cloned()
                    .unwrap_or_else(|| neighbor.name.clone());
                let body_strings = text(&character, b"STR#", 200)?;
                residents.push(Resident {
                    guid,
                    neighbor: neighbor.id,
                    object: saved.id,
                    name,
                    character_source: def.source.clone(),
                    body_strings,
                });
            }
            household = Some(Household {
                id: chunk.id,
                name,
                family,
                residents,
            });
        }
        Ok(Self {
            source: selected.clone(),
            number,
            neighborhood_source,
            sources,
            width,
            height,
            floors,
            floor_map,
            wall_map,
            arrays,
            house,
            view,
            simulator,
            types,
            objects,
            definitions,
            neighbors,
            neighborhood,
            resource_storage,
            families,
            household,
            missing_tile_heads,
        })
    }
}

/// Dynamic resource loading retains the imported neighborhood's character
/// precedence without requiring an entire parsed house in the live asset host.
pub fn link_neighborhood_creation_resources(
    index: &Index,
    program: &mut sims_vm::Program,
    neighborhood_source: &str,
    guids: &BTreeSet<u32>,
) -> Result<()> {
    let (neighborhood, _) = neighborhood_source
        .rsplit_once('/')
        .context("neighborhood source has no directory")?;
    for definition in definitions(index, neighborhood, guids)?.values() {
        for (name, module) in
            crate::vm::load_program(index, &definition.source, &program.globals)?.modules
        {
            if let Some(old) = program.modules.get(&name) {
                ensure!(
                    old.content_hash == module.content_hash,
                    "inconsistent creation module {name}"
                );
            } else {
                program.modules.insert(name, module);
            }
        }
    }
    Ok(())
}
