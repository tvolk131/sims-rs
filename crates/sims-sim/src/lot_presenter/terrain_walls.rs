//! First word of467690's rotated wall view, as consumed by49d14b.
//! 4795c5/53b98f copy this word from the saved cell; neighboring styles and
//! wallpaper do not alter it. This does not construct the remaining40 bytes.
use sims_formats::lot::WallTile;

pub fn segments(
    size: usize,
    cells: &[WallTile],
    point: [i8; 3],
    rotation: u8,
) -> Result<u8, String> {
    if !(1..=64).contains(&size) || cells.len() != size * size {
        return Err("terrain wall grid differs".into());
    }
    //467690 always rotates within64, independently of the active lot extent.
    let [x, y, _] = super::terrain_shade::inverse(point, rotation)?;
    if x < 0 || y < 0 || x as usize >= size || y as usize >= size {
        return Err("terrain wall point outside retained grid".into());
    }
    let bits = cells[x as usize * size + y as usize].segments;
    if bits & !63 != 0 {
        return Err("terrain wall segment bits differ".into());
    }
    if rotation == 0 || bits == 0 {
        return Ok(bits);
    }
    let mut bit = 1u8 << bits.trailing_zeros();
    let mut result = 0;
    loop {
        if bits & bit != 0 {
            // Initialized6f4670 rotation table, single-bit entries only.
            result |= if bit < 16 {
                ((bit << rotation) | (bit >> (4 - rotation))) & 15
            } else if rotation & 1 == 0 {
                bit
            } else {
                bit ^ 48
            };
        }
        bit <<= 1;
        //53be48 exits before bit32 unless32 was the initial lowest bit.
        if bit >= 32 {
            break;
        }
    }
    Ok(result)
}
