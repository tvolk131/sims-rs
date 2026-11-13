//!48d346's signed map and49cd43's four-cell quantization.
pub struct Map<'a> {
    pub width: usize,
    pub height: usize,
    pub rotation: u8,
    pub cells: &'a [i8],
}
pub fn inverse(point: [i8; 3], rotation: u8) -> Result<[i8; 3], String> {
    if rotation > 3 {
        return Err("terrain tile rotation differs".into());
    }
    Ok(super::room_masks::rotate(
        point,
        4u8.wrapping_sub(rotation) & 3,
    ))
}
impl Map<'_> {
    pub fn sample(&self, point: [i8; 3]) -> Result<i8, String> {
        if self.rotation > 3
            || self.width == 0
            || self.height == 0
            || self.width > 64
            || self.height > 64
            || self.cells.len() != self.width * self.height
        {
            return Err("terrain signed-map owner differs".into());
        }
        let [x, y, _] = inverse(point, self.rotation)?;
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return Err("terrain signed-map point outside retained grid".into());
        }
        Ok(self.cells[x as usize * self.height + y as usize])
    }
    pub fn corner(&self, point: [i8; 3], count: i32) -> Result<u8, String> {
        let [x, y, z] = point;
        let points = [
            point,
            [x.wrapping_sub(1), y, z],
            [x.wrapping_sub(1), y.wrapping_sub(1), z],
            [x, y.wrapping_sub(1), z],
        ];
        let mut sum = 0i32;
        for p in points {
            sum += i32::from(self.sample(p)?);
        }
        let mut value = (sum / 4).wrapping_add(128).wrapping_mul(count) / 256;
        if value < 0 {
            value = 0;
        }
        if value >= count {
            value = count.wrapping_sub(1);
        }
        Ok(value as u8)
    }
}
