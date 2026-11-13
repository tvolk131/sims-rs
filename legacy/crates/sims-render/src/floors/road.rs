//! 4a7dfe changes built-in road10/11 on odd view turns. Saved floor words
//! already contain the saved HOUS0 camera phase; the renderer keeps that phase
//! alongside its textures instead of changing simulation arrays on camera input.
use crate::Texture;
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_formats::{iff::Iff, sprite::SpriteSet};
use std::collections::BTreeMap;

pub(super) fn material(id: u16, saved_rotation: u8, rotation: u8) -> u16 {
    if matches!(id, 10 | 11) && ((saved_rotation ^ rotation) & 1) != 0 {
        id ^ 1
    } else {
        id
    }
}

/// A house using only one authored road direction still needs the opposite
/// resource after a camera turn. This augments renderer resources only.
pub(super) fn bind_views(
    index: &Index,
    materials: &mut BTreeMap<u16, [usize; 3]>,
    textures: &mut Vec<Texture>,
) -> Result<()> {
    if !materials.contains_key(&10) && !materials.contains_key(&11) {
        return Ok(());
    }
    let iff = Iff::parse(index.resources[index.find("floors.iff")?].read()?)?;
    for id in [10, 11] {
        if materials.contains_key(&id) {
            continue;
        }
        let mut bound = [0; 3];
        for zoom in 0..3usize {
            let number = id + zoom as u16 * 256;
            let chunk = iff
                .get(b"SPR2", number)
                .with_context(|| format!("road SPR2{number} absent"))?;
            let raw = iff.data(chunk);
            let set = SpriteSet::parse(raw)?;
            ensure!(
                set.frames.len() == 1,
                "road SPR2{number} frame count differs"
            );
            let mut frame = set.decode(raw, 0, &iff)?;
            super::extend_floor_padding(frame.width, &mut frame.rgba);
            bound[zoom] = textures.len();
            textures.push(Texture {
                width: frame.width,
                height: frame.height,
                rgba: frame.rgba.into(),
                depth: frame.depth.into(),
            });
        }
        materials.insert(id, bound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_view_phase_follows_original_odd_turn_swap_only() {
        for saved in 0..4 {
            for rotation in 0..4 {
                for id in [0, 9, 10, 11, 12, 255, 261] {
                    let actual = material(id, saved, rotation);
                    let expected = if matches!(id, 10 | 11) && ((rotation + 4 - saved) % 4) % 2 == 1
                    {
                        id ^ 1
                    } else {
                        id
                    };
                    assert_eq!(actual, expected, "id{id} saved{saved} view{rotation}");
                    assert_eq!(material(actual, rotation, saved), id);
                }
            }
        }
    }
}
