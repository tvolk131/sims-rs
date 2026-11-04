//! Resolve saved surface identities to original SPR2/SPR# resources. This read-only
//! presentation binding preserves saved IDs; it does not run the native global
//! catalog allocator or its nearest-color missing-content fallback.
use crate::{Index, saves::House};
use anyhow::{Context, Result, ensure};
use sims_formats::{
    iff::Iff,
    sprite::{Frame, LegacySpriteSet, SpriteSet},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub struct FloorMaterial {
    pub saved_id: u16,
    pub source: String,
    pub local: u16,
    /// Small, medium, large; native catalog 0x4a0051/0x4a0744.
    pub frames: [Frame; 3],
}

#[derive(Debug)]
pub struct WallMaterial {
    pub saved_id: u16,
    pub source: String,
    pub local: u16,
    /// Small, medium, large; each has both orthogonal faces, broad diagonal,
    /// and narrow diagonal. Catalog 0x4c4d7e/0x4c552e.
    pub frames: [[Frame; 4]; 3],
}

pub fn load_walls(index: &Index, house: &House) -> Result<BTreeMap<u16, WallMaterial>> {
    let mut used = BTreeSet::new();
    for floor in &house.floors {
        for t in &floor.walls {
            if t.segments & 0x30 != 0 {
                used.extend([t.patterns[2], t.patterns[3]]);
            }
            for (bit, pattern) in [(1, 0), (2, 1), (4, 3), (8, 2)] {
                if t.segments & bit != 0 {
                    used.insert(t.patterns[pattern]);
                }
            }
        }
    }
    let mut bindings = BTreeMap::new();
    for file in &house.wall_map {
        for e in &file.entries {
            ensure!(
                bindings
                    .insert(e.id, (file.name.clone(), e.local))
                    .is_none(),
                "duplicate saved wall mapping {}",
                e.id
            );
        }
    }
    let mut files = BTreeMap::<usize, Iff>::new();
    let mut result = BTreeMap::new();
    for id in used {
        let (name, local, offsets) = if id <= 30 || (248..=255).contains(&id) {
            ("walls.iff".to_owned(), id, [1536, 1792, 2048])
        } else {
            let (name, local) = bindings
                .get(&id)
                .with_context(|| format!("saved wall {id} has no source mapping"))?
                .clone();
            (name, local, [0, 1792, 2048])
        };
        let source = resource(index, &name)?;
        if let std::collections::btree_map::Entry::Vacant(e) = files.entry(source) {
            e.insert(Iff::parse(index.resources[source].read()?)?);
        }
        let iff = &files[&source];
        let mut zooms = vec![];
        for offset in offsets {
            let chunk = local
                .checked_add(offset)
                .context("wall sprite ID overflow")?;
            let c = iff.get(b"SPR#", chunk).with_context(|| {
                format!(
                    "{}: wall SPR# {chunk} missing",
                    index.resources[source].locator
                )
            })?;
            let set = LegacySpriteSet::parse(iff.data(c))?;
            ensure!(set.frames.len() == 4, "unexpected wall sprite frame count");
            let frames = (0..4)
                .map(|f| set.decode(iff.data(c), f, iff))
                .collect::<sims_formats::Result<Vec<_>>>()?;
            zooms.push(frames.try_into().unwrap());
        }
        result.insert(
            id,
            WallMaterial {
                saved_id: id,
                source: index.resources[source].locator.clone(),
                local,
                frames: zooms.try_into().unwrap(),
            },
        );
    }
    Ok(result)
}

/// Original built-in wall silhouettes, independent of wallpaper color. A white
/// engine palette retains only the decoded alpha/shape, including apertures.
/// Dynamic styles without a built-in SPR# are absent, not fabricated masks.
pub fn load_wall_shapes(
    index: &Index,
    styles: impl IntoIterator<Item = u8>,
) -> Result<BTreeMap<(u8, bool), [[Frame; 4]; 3]>> {
    let source = resource(index, "walls.iff")?;
    let iff = Iff::parse(index.resources[source].read()?)?;
    let palette = sims_formats::sprite::Palette(vec![[255, 255, 255]; 256]);
    let mut result = BTreeMap::new();
    for style in styles.into_iter().collect::<BTreeSet<_>>() {
        for cut in [false, true] {
            if iff
                .get(b"SPR#", u16::from(style) * 2 + u16::from(cut))
                .is_none()
            {
                continue;
            }
            let mut zooms = vec![];
            for offset in [0, 512, 1024] {
                let id = offset + u16::from(style) * 2 + u16::from(cut);
                zooms.push(decode_wall_shape(&iff, id, &palette)?);
            }
            result.insert((style, cut), zooms.try_into().unwrap());
        }
    }
    Ok(result)
}

/// OBJD+a2 names six object-owned SPR# masks: full small/medium/large,
/// then cutaway small/medium/large. Their IDs are independent of the saved
/// dynamic wall style allocated to the object by the engine.
pub fn load_object_wall_shapes(
    index: &Index,
    source: &str,
    base: u16,
) -> Result<[[[Frame; 4]; 3]; 2]> {
    let id = index.find(source)?;
    let iff = Iff::parse(index.resources[id].read()?)?;
    let palette = sims_formats::sprite::Palette(vec![[255, 255, 255]; 256]);
    let mut modes = vec![];
    for cut in 0..2 {
        let mut zooms = vec![];
        for zoom in 0..3 {
            let id = base
                .checked_add(cut * 3 + zoom)
                .context("object wall mask ID overflow")?;
            zooms.push(decode_wall_shape(&iff, id, &palette)?);
        }
        modes.push(zooms.try_into().unwrap());
    }
    Ok(modes.try_into().unwrap())
}

fn decode_wall_shape(
    iff: &Iff,
    id: u16,
    palette: &sims_formats::sprite::Palette,
) -> Result<[Frame; 4]> {
    let chunk = iff
        .get(b"SPR#", id)
        .with_context(|| format!("wall mask SPR# {id} missing"))?;
    let set = LegacySpriteSet::parse(iff.data(chunk))?;
    ensure!(set.frames.len() == 4, "wall shape frame count differs");
    let frames: Vec<_> = (0..4)
        .map(|i| set.decode_with_palette(iff.data(chunk), i, palette))
        .collect::<sims_formats::Result<_>>()?;
    Ok(frames.try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires private installed wall SPR# resources"]
    fn installed_wall_silhouettes_retain_cutaway_and_opening_transparency() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/cabinet-extracted");
        let index = Index::scan(&root)?;
        let shapes = load_wall_shapes(&index, [1, 2, 3, 4, 15, 21, 22])?;
        let opaque = |f: &Frame| f.rgba.chunks_exact(4).filter(|p| p[3] != 0).count();
        for zoom in 0..3 {
            for direction in 0..4 {
                let wall = &shapes[&(1, false)][zoom][direction];
                let short = &shapes[&(1, true)][zoom][direction];
                ensure!(
                    opaque(short) > 0 && opaque(short) < opaque(wall),
                    "cutaway must retain only a base strip"
                );
                for style in [2, 3, 4, 15, 21] {
                    let f = &shapes[&(style, false)][zoom][direction];
                    ensure!(
                        (f.width, f.height) == (wall.width, wall.height),
                        "shape projection dimensions differ"
                    );
                    // Original low-zoom diagonal frames can be solid too;
                    // several openings retain a solid narrow diagonal.
                    // Do not invent an aperture there.
                    ensure!(
                        opaque(f) > 0 && opaque(f) <= opaque(wall),
                        "invalid silhouette {style}/{direction}"
                    );
                    if direction < 2 || zoom == 2 && direction == 2 {
                        ensure!(
                            opaque(f) < opaque(wall),
                            "opening/fence lost transparency {style}/{direction}"
                        );
                    }
                }
            }
        }
        ensure!(
            !shapes.contains_key(&(22, false)),
            "object-provided style must not be fabricated"
        );
        for base in [150, 550, 650] {
            let modes = load_object_wall_shapes(
                &index,
                "GameData_ALL_Recursive/Objects/Objects.far::Windows.iff",
                base,
            )?;
            for zoom in 0..3 {
                // Some diagonal cut variants are deliberately solid even
                // when their full-height variant has an aperture.
                for direction in 0..4 {
                    let solid = &shapes[&(1, false)][zoom][direction];
                    let full = &modes[0][zoom][direction];
                    let short = &modes[1][zoom][direction];
                    ensure!(
                        (full.width, full.height) == (solid.width, solid.height),
                        "object mask projection differs"
                    );
                    ensure!(
                        opaque(full) <= opaque(solid) && opaque(short) <= opaque(solid),
                        "object mask/cutaway transparency lost: base{base}/zoom{zoom}/direction{direction}: full={}, short={}, solid={}",
                        opaque(full),
                        opaque(short),
                        opaque(solid)
                    );
                    if direction < 2 {
                        ensure!(
                            opaque(full) < opaque(solid) && opaque(short) < opaque(full),
                            "window aperture/cutaway missing"
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn resource(index: &Index, name: &str) -> Result<usize> {
    let candidates: Vec<_> = index
        .resources
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.name
                .rsplit(['/', '\\'])
                .next()
                .is_some_and(|s| s.eq_ignore_ascii_case(name))
        })
        .map(|(id, _)| id)
        .collect();
    select_resource(index, name, &candidates)
}

/// Borrowed, per-operation filename index. Only candidate enumeration is
/// retained; every lookup still reads and compares all matching resources.
pub(crate) struct ResourceNames<'a> {
    index: &'a Index,
    names: BTreeMap<usize, Vec<(usize, &'a str)>>,
}
impl<'a> ResourceNames<'a> {
    pub(crate) fn new(index: &'a Index) -> Self {
        let mut names = BTreeMap::<usize, Vec<(usize, &str)>>::new();
        for (id, resource) in index.resources.iter().enumerate() {
            if let Some(name) = resource.name.rsplit(['/', '\\']).next() {
                names.entry(name.len()).or_default().push((id, name));
            }
        }
        Self { index, names }
    }
    pub(crate) fn resource(&self, name: &str) -> Result<usize> {
        let candidates: Vec<_> = self.names.get(&name.len()).into_iter().flatten()
            .filter(|(_, candidate)| candidate.eq_ignore_ascii_case(name))
            .map(|&(id, _)| id).collect();
        select_resource(self.index, name, &candidates)
    }
}

fn select_resource(index: &Index, name: &str, candidates: &[usize]) -> Result<usize> {
    let &first = candidates
        .first()
        .with_context(|| format!("surface file missing: {name}"))?;
    let res = &index.resources[first];
    let bytes = res.read()?;
    for &id in candidates.iter().skip(1) {
        let other = &index.resources[id];
        ensure!(
            other.read()? == bytes,
            "ambiguous surface file {name}: {} / {}",
            res.locator,
            other.locator
        );
    }
    Ok(first)
}

#[cfg(test)]
#[path = "surfaces/resource_names_tests.rs"]
mod resource_names_tests;

pub fn load_floors(index: &Index, house: &House) -> Result<BTreeMap<u16, FloorMaterial>> {
    let mut used = BTreeSet::new();
    for floor in &house.floors {
        for (&id, wall) in floor.coverings.iter().zip(&floor.walls) {
            if id == 255 {
                ensure!(
                    matches!(wall.segments & 0x30, 16 | 32),
                    "split floor lacks a single diagonal"
                );
                // 0x4a0e3b remaps these two overloaded wall-cell fields for
                // floor 255. They are not wall-covering IDs in a split cell.
                used.extend([wall.styles[0], wall.patterns[0]]);
            } else {
                used.insert(id);
            }
        }
    }
    used.remove(&0);
    let mut bindings = BTreeMap::<u16, (String, u16)>::new();
    for file in &house.floor_map {
        for e in &file.entries {
            ensure!(
                bindings
                    .insert(e.id, (file.name.clone(), e.local))
                    .is_none(),
                "duplicate saved floor mapping {}",
                e.id
            );
        }
    }
    let mut files = BTreeMap::<usize, Iff>::new();
    let mut result = BTreeMap::new();
    for id in used {
        let (name, local) = if (1..30).contains(&id) {
            ("floors.iff".to_owned(), id)
        } else {
            bindings
                .get(&id)
                .with_context(|| format!("saved floor {id} has no source mapping"))?
                .clone()
        };
        let source = resource(index, &name)?;
        if let std::collections::btree_map::Entry::Vacant(e) = files.entry(source) {
            e.insert(Iff::parse(index.resources[source].read()?)?);
        }
        let iff = &files[&source];
        let mut frames = vec![];
        for offset in [0, 256, 512] {
            let chunk = local
                .checked_add(offset)
                .context("floor sprite ID overflow")?;
            let c = iff.get(b"SPR2", chunk).with_context(|| {
                format!(
                    "{}: floor SPR2 {chunk} missing",
                    index.resources[source].locator
                )
            })?;
            let set = SpriteSet::parse(iff.data(c))?;
            ensure!(set.frames.len() == 1, "unexpected floor sprite frame count");
            frames.push(set.decode(iff.data(c), 0, iff)?);
        }
        result.insert(
            id,
            FloorMaterial {
                saved_id: id,
                source: index.resources[source].locator.clone(),
                local,
                frames: frames.try_into().unwrap(),
            },
        );
    }
    Ok(result)
}
