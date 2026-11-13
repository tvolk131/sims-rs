//! Original temporary-hide decision shared by presentation and native replay.
///4aebc1 restores the saved truth value, which need not equal the old raw word.
pub fn restored(prior:i16,active:bool,current:i16)->i16{if active{i16::from(prior!=0)}else{current}}
fn contains(mask: &super::walls::cutaway::Mask, [x, y, _]: [i8; 3]) -> Result<bool, String> {
    let i = i32::from(y)
        .checked_sub(mask.first_row)
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| mask.rows.get(i))
        .ok_or("object cutaway mask row unavailable")?;
    Ok((0..64).contains(&x) && i & (1u64 << x as u32) != 0)
}
/// Tile has already been rotated into the view's64-cell coordinate system.
/// wall_tool is4cea33's specific wall-editing exemption, not Buy/Build mode.
pub fn hidden(
    rotation: u8,
    level: i32,
    all_walls: bool,
    cutaway: bool,
    mask: &super::walls::cutaway::Mask,
    tile: [i8; 3],
    flags: u16,
    placement: u16,
    facing: i16,
    wall_tool: bool,
) -> Result<bool, String> {
    if !(all_walls || cutaway && contains(mask, tile)?) || wall_tool || flags & 0x400 == 0 {
        return Ok(false);
    }
    let mut neighbor = tile;
    if !all_walls {
        let direction = normal(placement, facing, rotation);
        let [dx, dy] = match direction {
            1 => [-1, 0],
            2 => [0, -1],
            4 => [1, 0],
            8 => [0, 1],
            _ => [0, 0],
        };
        neighbor[0] = neighbor[0].wrapping_add(dx);
        neighbor[1] = neighbor[1].wrapping_add(dy);
    }
    Ok(i32::from(tile[2]) == level && (all_walls || contains(mask, neighbor)?))
}
pub fn normal(placement: u16, facing: i16, rotation: u8) -> u8 {
    let bit = match placement & 5 {
        1 => 2u8,
        4 => 8,
        _ => 0,
    };
    let turn = (i32::from(facing) / 2 + i32::from(rotation)).rem_euclid(4) as u32;
    ((u16::from(bit) << turn | u16::from(bit) >> (4 - turn)) & 15) as u8
}
