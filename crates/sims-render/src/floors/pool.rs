//! Read-only pool artwork selection from Arry9. Arry10 is separate pond water.
//! 4ad8ac selects one of16 original pool diamonds, floor12 for rim16, and
//! 461e39's corner caps. These SPR2 resources have one frame; no clock advances.
use crate::{Camera, Texture};
use anyhow::{Context, Result, ensure};
use sims_content::{Index, saves::House};
use sims_formats::{
    iff::Iff,
    sprite::{Frame, SpriteSet},
};

#[derive(Debug)]
pub(crate) struct Materials {
    surfaces: [[usize; 17]; 3],
    corners: [[usize; 4]; 3],
}

impl Materials {
    pub(super) fn surface(&self, kind: u8, camera: &Camera) -> Option<usize> {
        (kind <= 16).then(|| {
            let shape = if kind == 16 {
                16
            } else {
                rotate(kind, camera.rotation) as usize
            };
            self.surfaces[(camera.sprite_zoom().0 - 1) as usize][shape]
        })
    }
    pub(super) fn corner(&self, corner: usize, camera: &Camera) -> usize {
        self.corners[(camera.sprite_zoom().0 - 1) as usize][corner]
    }
}

///4614d6 preserves the high nibble and rotates low four bits through53d2fc's
///mapping: bit1→8,2→1,4→2,8→4 for one quarter turn.
pub(super) fn rotate(value: u8, rotation: u8) -> u8 {
    let mut low = value & 15;
    for _ in 0..rotation {
        low = (low >> 1) | ((low & 1) << 3);
    }
    (value & 240) | low
}

///461e39 after the caller has supplied canonical array coordinates. Rules are
///the original immutable40-byte table at6e52a0, not a guessed neighbor mask.
pub(super) fn corner_mask(size: usize, cells: &[u8], x: i32, z: i32, rotation: u8) -> Result<u8> {
    ensure!(
        size <= 64 && cells.len() == size * size && rotation < 4,
        "invalid pool selection inputs"
    );
    let cell = |x: i32, z: i32| {
        (x >= 0 && z >= 0 && x < size as i32 && z < size as i32)
            .then(|| cells[x as usize * size + z as usize])
    };
    let Some(center) = cell(x, z) else {
        return Ok(0);
    };
    let rules: [[i32; 10]; 4] = [
        [3, -1, 0, 1, 8, 0, -1, 2, 4, 1],
        [6, -1, 0, 4, 8, 0, 1, 2, 1, 2],
        [9, 0, -1, 8, 4, 1, 0, 1, 2, 8],
        [12, 0, 1, 8, 1, 1, 0, 4, 2, 4],
    ];
    let mut mask = 0;
    for [
        required,
        ax,
        az,
        absent_a,
        present_a,
        bx,
        bz,
        absent_b,
        present_b,
        result,
    ] in rules
    {
        if center & required as u8 != required as u8 {
            continue;
        }
        if let (Some(a), Some(b)) = (cell(x + ax, z + az), cell(x + bx, z + bz)) {
            if a & absent_a as u8 == 0
                && a & present_a as u8 != 0
                && b & absent_b as u8 == 0
                && b & present_b as u8 != 0
            {
                mask |= result as u8;
            }
        }
    }
    Ok(rotate(mask, rotation))
}

/// Place sparse original corner sprites on the full diamond canvas using their
/// authored SPR2 offsets. Padding here must remain transparent: edge extension
/// would turn a small corner cap into a strip across the whole pool.
fn corner_canvas(frame: Frame, zoom: u32) -> Result<Texture> {
    let width = (16u32 << zoom) - 1;
    let height = 8u32 << zoom;
    ensure!(
        frame.x >= 0
            && frame.y >= 0
            && frame.x as u32 + frame.width <= width
            && frame.y as u32 + frame.height <= height,
        "pool corner exceeds authored tile canvas"
    );
    let mut texture = Texture {
        width,
        height,
        rgba: vec![0; (width * height * 4) as usize].into(),
        depth: vec![255; (width * height) as usize].into(),
    };
    let rgba = std::sync::Arc::make_mut(&mut texture.rgba);
    let depth = std::sync::Arc::make_mut(&mut texture.depth);
    for y in 0..frame.height as usize {
        let source = y * frame.width as usize;
        let dest = (y + frame.y as usize) * width as usize + frame.x as usize;
        rgba[dest * 4..(dest + frame.width as usize) * 4]
            .copy_from_slice(&frame.rgba[source * 4..(source + frame.width as usize) * 4]);
        depth[dest..dest + frame.width as usize]
            .copy_from_slice(&frame.depth[source..source + frame.width as usize]);
    }
    Ok(texture)
}

pub(super) fn bind(
    index: &Index,
    house: &House,
    textures: &mut Vec<Texture>,
) -> Result<Option<Materials>> {
    if !house.pool_cells()?.iter().any(|v| *v <= 16) {
        return Ok(None);
    }
    let id = index.find("floors.iff")?;
    let iff = Iff::parse(index.resources[id].read()?)?;
    let mut materials = Materials {
        surfaces: [[0; 17]; 3],
        corners: [[0; 4]; 3],
    };
    for zoom in 1..=3u32 {
        for shape in 0..21u16 {
            let sprite = match shape {
                0..=15 => 1024 + (zoom as u16 - 1) * 16 + shape,
                16 => 12 + (zoom as u16 - 1) * 256,
                _ => 1072 + (zoom as u16 - 1) * 4 + shape - 17,
            };
            let chunk = iff
                .get(b"SPR2", sprite)
                .with_context(|| format!("pool SPR2{sprite} absent"))?;
            let raw = iff.data(chunk);
            let set = SpriteSet::parse(raw)?;
            ensure!(
                set.frames.len() == 1,
                "pool SPR2{sprite} frame count differs"
            );
            let mut frame = set.decode(raw, 0, &iff)?;
            let texture = if shape < 17 {
                ensure!(
                    frame.x == 0 && frame.y == 0,
                    "pool surface SPR2 offset differs"
                );
                super::extend_floor_padding(frame.width, &mut frame.rgba);
                Texture {
                    width: frame.width,
                    height: frame.height,
                    rgba: frame.rgba.into(),
                    depth: frame.depth.into(),
                }
            } else {
                corner_canvas(frame, zoom)?
            };
            let target = if shape < 17 {
                &mut materials.surfaces[(zoom - 1) as usize][shape as usize]
            } else {
                &mut materials.corners[(zoom - 1) as usize][(shape - 17) as usize]
            };
            *target = textures.len();
            textures.push(texture);
        }
    }
    Ok(Some(materials))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    #[derive(Deserialize)]
    struct Mask {
        value: u8,
        rotation: u8,
        original: u8,
    }
    #[derive(Deserialize)]
    struct Corners {
        size: usize,
        cells: Vec<u8>,
        position: [i32; 2],
        original: [u8; 4],
    }
    #[derive(Deserialize)]
    struct Fixture {
        masks: Vec<Mask>,
        corners: Vec<Corners>,
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn native_pool_rotation_and_corner_predicates() -> Result<()> {
        let fixture: Fixture = serde_json::from_str(include_str!(
            "../../tests/fixtures/native-pool-selectors.json"
        ))?;
        assert_eq!(fixture.masks.len(), 1024);
        for row in fixture.masks {
            assert_eq!(rotate(row.value, row.rotation), row.original);
        }
        assert_eq!(fixture.corners.len(), 160);
        for row in fixture.corners {
            for rotation in 0..4 {
                assert_eq!(
                    corner_mask(
                        row.size,
                        &row.cells,
                        row.position[0],
                        row.position[1],
                        rotation
                    )?,
                    row.original[rotation as usize]
                );
            }
        }
        Ok(())
    }
    #[test]
    fn sparse_corner_canvas_retains_offsets_pixels_and_holes() -> Result<()> {
        let authored = vec![
            22, 33, 44, 255, 0, 0, 0, 0, 55, 66, 77, 128, 88, 99, 11, 255,
        ];
        let texture = corner_canvas(
            Frame {
                width: 2,
                height: 2,
                x: 9,
                y: 1,
                flags: 1,
                rgba: authored.clone(),
                depth: vec![255; 4],
            },
            1,
        )?;
        for y in 0..2usize {
            assert_eq!(
                &texture.rgba[((y + 1) * 31 + 9) * 4..((y + 1) * 31 + 11) * 4],
                &authored[y * 8..y * 8 + 8]
            );
        }
        assert_eq!(texture.rgba.chunks_exact(4).filter(|p| p[3] > 0).count(), 3);
        Ok(())
    }
}
