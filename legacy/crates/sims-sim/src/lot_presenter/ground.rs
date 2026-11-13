//!4af029..4af194: ground pass selection before placement and rasterization.
//! Neighbor clamps include the native active-size boundary, not size-1.
pub fn selected(
    enabled: u8,
    point: [i8; 3],
    current: u16,
    size: usize,
    active: i32,
    covering: &[u16],
    water: &[u8],
) -> Result<bool, String> {
    if enabled == 0 {
        return Ok(false);
    }
    if current == 0 {
        return Ok(true);
    }
    if covering.len() != size * size || water.len() != size * size {
        return Err("ground grids absent".into());
    }
    let [x, y] = [i32::from(point[0]), i32::from(point[1])];
    let neighbors = [
        [(x - 1).max(0), y],
        [(x + 1).min(active), y],
        [x, (y - 1).max(0)],
        [x, (y + 1).min(active)],
    ];
    let index = |[x, y]: [i32; 2]| -> Result<usize, String> {
        if x < 0 || y < 0 || x as usize >= size || y as usize >= size {
            return Err("ground lookup outside retained grid".into());
        }
        Ok(x as usize * size + y as usize)
    };
    for p in neighbors {
        if covering[index(p)?] == 0 {
            return Ok(true);
        }
    }
    //478f4a tests World+64/Arry10 against255 on these level-one points.
    for p in neighbors {
        if water[index(p)?] == 255 {
            return Ok(true);
        }
    }
    Ok(false)
}
