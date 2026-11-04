//!4c9e38 common ground depth. Lookup spans are borrowed and bounded.
use super::room_masks::terrain_corners;
pub struct Inputs<'a> {
    pub size: usize,
    pub rotation: u8,
    pub heights: &'a [[u8; 4]],
    pub zoom: u8,
    pub row_start: i32,
    pub rows: &'a [i32],
    pub curve_start: i32,
    pub curve: &'a [i16],
}
impl Inputs<'_> {
    pub fn sample(&self, point: [i8; 3], clamp: u8) -> Result<i32, String> {
        if !(1..=3).contains(&self.zoom) {
            return Err("tile depth projection zoom absent".into());
        }
        //4c9e38 forces the projected level to one, then cancels origin Y.
        let y = (i32::from(point[0]) + i32::from(point[1]))
            .wrapping_mul(4i32.wrapping_shl(u32::from(self.zoom)));
        let row = y
            .checked_sub(self.row_start)
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| self.rows.get(i))
            .copied()
            .ok_or("tile depth row outside retained span")?;
        let h = i32::from(terrain_corners(self.size, self.heights, point, self.rotation)?[1]);
        let curve = h
            .checked_sub(self.curve_start)
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| self.curve.get(i))
            .copied()
            .ok_or("tile depth height outside retained curve")?;
        let value = row.wrapping_add(i32::from(curve));
        Ok(if clamp == 1 {
            value.clamp(0, 65535)
        } else {
            value
        })
    }
}
