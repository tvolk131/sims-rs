//!4aed23/4aedc2 descriptor query and4b4499 brightness getter.
//! Descriptor bytes are signed pairs, with127 marking an absent pair.
pub fn rotate(mut v: [u8; 4], rotation: u8) -> Result<[u8; 4], String> {
    if rotation > 3 {
        return Err("floor descriptor rotation differs".into());
    }
    let count = u8::from(v[0] != 127) + u8::from(v[2] != 127);
    if rotation == 0 || count == 0 {
        return Ok(v);
    }
    let turn = |[x, y]: [u8; 2]| match rotation {
        1 => [y.wrapping_neg(), x],
        2 => [x.wrapping_neg(), y.wrapping_neg()],
        _ => [y, x.wrapping_neg()],
    };
    let first = turn([v[0], v[1]]);
    v[..2].copy_from_slice(&first);
    if count > 1 {
        let second = turn([v[2], v[3]]);
        v[2..].copy_from_slice(&second);
    }
    Ok(v)
}
pub fn descriptor(cells: &[u8], point: [i8; 3], rotation: u8) -> Result<[u8; 4], String> {
    if cells.len() != 64 * 64 * 4 || point[..2].iter().any(|v| !(0..64).contains(v)) {
        return Err("floor descriptor grid differs".into());
    }
    let [x, y, _] = super::terrain_shade::inverse(point, rotation)?;
    let i = (y as usize * 64 + x as usize) * 4;
    rotate(cells[i..i + 4].try_into().unwrap(), rotation)
}
pub fn brightness(context: &[u32; 29]) -> u32 {
    if context[0x34 / 4] & 255 != 0 {
        1f32.to_bits()
    } else {
        context[0x48 / 4]
    }
}

///4aec97 counts populated pairs, including the native noncanonical layouts.
pub fn count(descriptor: [u8; 4]) -> usize {
    usize::from(descriptor[0] != 127) + usize::from(descriptor[2] != 127)
}
///4aec34 writes only the returned number of points. Its subtraction wraps
/// as a signed byte; an absent output is not replaced with a zero point.
pub fn neighbors(descriptor: [u8; 4], point: [i8; 3]) -> Vec<[i8; 3]> {
    (0..count(descriptor))
        .map(|n| {
            [
                point[0].wrapping_sub(descriptor[n * 2] as i8),
                point[1].wrapping_sub(descriptor[n * 2 + 1] as i8),
                point[2],
            ]
        })
        .collect()
}
///4aeca9 compacts the second pair only for index zero; other indexes remove
/// the second pair. A single-pair input always clears its first pair.
pub fn remove(mut descriptor: [u8; 4], index: i32) -> [u8; 4] {
    match count(descriptor) {
        1 => descriptor[..2].fill(127),
        2 => {
            if index == 0 {
                descriptor[0] = descriptor[2];
                descriptor[1] = descriptor[3];
            }
            descriptor[2..].fill(127);
        }
        _ => (),
    }
    descriptor
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DiagonalCoverings {
    pub orientation: u8,
    pub first: u16,
    pub second: u16,
}
///4af4e7..4af529/4afb25. Expanded style0/pattern0 encode the two
/// floor coverings on a diagonal tile. Both zero inherit the ordinary cover.
pub fn diagonal_coverings(
    flags: u8,
    mut style: u16,
    mut pattern: u16,
    covering: u16,
) -> Option<DiagonalCoverings> {
    let orientation = if flags & 16 != 0 {
        16
    } else if flags & 32 != 0 {
        32
    } else {
        return None;
    };
    if style == 0 && pattern == 0 {
        style = covering;
        pattern = covering;
    }
    if style == 0 && pattern == 0 {
        return None;
    }
    let (first, second) = if orientation == 16 {
        (pattern, style)
    } else {
        (style, pattern)
    };
    Some(DiagonalCoverings {
        orientation,
        first,
        second,
    })
}

/// The four native diagonal half-clip windows compare signed 16-bit words.
/// Descending halves additionally consume the retained view height word.
pub fn diagonal_clip(
    orientation: u8,
    half: usize,
    mut clip: [i16; 4],
    screen: [i32; 2],
    corner: u8,
    zoom: u32,
    height: Option<i16>,
) -> Result<[i16; 4], String> {
    if half > 1 || !(1..=3).contains(&zoom) {
        return Err("diagonal clip half/zoom differs".into());
    }
    match orientation {
        16 => {
            let height = height.ok_or("descending clip view height is unobserved")?;
            let cut = screen[1].wrapping_add(
                i32::from(height).wrapping_sub(i32::from(corner).wrapping_shl(zoom + 1)),
            ) as i16;
            if half == 0 {
                clip[0] = clip[0].max(cut);
            } else {
                clip[2] = clip[2].min(cut);
            }
        }
        32 => {
            let cut = screen[0] as i16;
            if half == 0 {
                clip[3] = clip[3].min(cut);
            } else {
                clip[1] = clip[1].max(cut);
            }
        }
        _ => return Err("diagonal clip orientation differs".into()),
    }
    Ok(clip)
}

///4c6383 after the native ordered map resolves its resident entry. Missing
/// entries leave the caller's output words untouched; None carries that result.
pub fn lookup(
    entries: &std::collections::BTreeMap<u32, ([u32; 3], u32)>,
    key: u32,
    zoom: u32,
) -> Result<Option<(u32, u32)>, String> {
    if !(1..=3).contains(&zoom) {
        return Err("floor resident zoom differs".into());
    }
    Ok(entries
        .get(&key)
        .map(|(images, palette)| (images[zoom as usize - 1], *palette)))
}
