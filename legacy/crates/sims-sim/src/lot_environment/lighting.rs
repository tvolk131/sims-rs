//! Original room light bytes and the two-source local light map.
use crate::lot_rooms::Partition;
use serde::{Deserialize, Serialize};
pub mod point;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Map {
    /// Native fixed-stride row-major 64x64 map, unlike the lot's column arrays.
    pub cells: Vec<[i8; 4]>,
}
impl Default for Map {
    fn default() -> Self {
        Self {
            cells: vec![[127; 4]; 64 * 64],
        }
    }
}
fn count(cell: [i8; 4]) -> usize {
    usize::from(cell[0] != 127) + usize::from(cell[2] != 127)
}
fn distance(x: i32, y: i32) -> i32 {
    x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y))
}
/// 4b5037. Slots retain insertion order; replacement does not sort them.
pub fn insert(cell: &mut [i8; 4], x: i32, y: i32) {
    match count(*cell) {
        0 => {
            cell[0] = x as i8;
            cell[1] = y as i8;
        }
        1 => {
            cell[2] = x as i8;
            cell[3] = y as i8;
        }
        _ => {
            let d = distance(x, y);
            if d < distance(i32::from(cell[0]), i32::from(cell[1])) {
                cell[0] = x as i8;
                cell[1] = y as i8;
            } else if d < distance(i32::from(cell[2]), i32::from(cell[3])) {
                cell[2] = x as i8;
                cell[3] = y as i8;
            }
        }
    }
}
/// 4b51ae. Removal promotes the second slot only when the first matched.
pub fn remove(cell: &mut [i8; 4], x: i32, y: i32) {
    let first = i32::from(cell[0]) == x && i32::from(cell[1]) == y;
    match count(*cell) {
        1 if first => *cell = [127; 4],
        2 => {
            if first {
                cell[0] = cell[2];
                cell[1] = cell[3];
            } else if i32::from(cell[2]) != x || i32::from(cell[3]) != y {
                return;
            }
            cell[2] = 127;
            cell[3] = 127;
        }
        _ => {}
    }
}
/// Original __ftol integer result is indefinite on overflow/NaN. Callers retain
/// its low byte/word. Widened arithmetic preserves the native float input store.
fn ftol(value: f64) -> i64 {
    if !value.is_finite()
        || value < -9_223_372_036_854_775_808.0
        || value >= 9_223_372_036_854_775_808.0
    {
        i64::MIN
    } else {
        value.trunc() as i64
    }
}
pub fn radius(range: f32) -> i32 {
    ftol(f64::from(range) / 3.0 + 0.5) as i32
}
/// 46772d. The original stores the low byte, not a saturating color conversion.
pub fn light_byte(light: f32) -> u8 {
    ftol(f64::from(light) * 255.0 + 0.5) as u8
}

/// 454f5c / 454f29 followed by the light's +5 height adjustment in 467e05.
/// Terrain height and the reciprocal height scale each spill to f32 first.
pub fn position(point: [i32; 2], height: f32) -> [f32; 3] {
    let scale = (1.0 / (2.0 * 6.0f64.sqrt())) as f32;
    let y = (f64::from(scale) * f64::from(height) * 3.0) as f32;
    [
        (f64::from(point[0]) * 0.1875) as f32,
        y + 5.0,
        (f64::from(point[1]) * 0.1875) as f32,
    ]
}

impl Map {
    /// 4b50b5 / 4b4e8b. Draw respects the source room and native diagonal-room
    /// directory. Clear is spatial only. `rooms_enabled` is native 6e5428.
    pub fn update(
        &mut self,
        size: usize,
        rooms: &Partition,
        origin: [i8; 3],
        range: f32,
        draw: bool,
        rooms_enabled: bool,
    ) -> Result<(), String> {
        if !(2..=64).contains(&size)
            || self.cells.len() != 4096
            || rooms.primary.len() != size * size
            || !range.is_finite()
        {
            return Err("invalid native light-map inputs".into());
        }
        let inside = |p: i8| p > 0 && i32::from(p) < size as i32 - 1;
        if !inside(origin[0]) || !inside(origin[1]) {
            return Ok(());
        }
        let source = rooms.primary[origin[0] as usize * size + origin[1] as usize];
        let radius = radius(range);
        let Some(start) = radius.checked_neg() else {
            return Err("native light radius overflow".into());
        };
        for dy in start..=radius {
            for dx in start..=radius {
                let x = origin[0].wrapping_add(dx as i8);
                let y = origin[1].wrapping_add(dy as i8);
                if !inside(x) || !inside(y) {
                    continue;
                }
                if draw {
                    let target = rooms.primary[x as usize * size + y as usize];
                    if target != source
                        && !(target == -5
                            && rooms_enabled
                            && rooms.halves.iter().any(|h| {
                                h.tile == [x as usize, y as usize]
                                    && h.rooms.values().any(|&r| r == source)
                            }))
                    {
                        continue;
                    }
                }
                let cell = &mut self.cells[y as usize * 64 + x as usize];
                if draw {
                    insert(cell, dx, dy);
                } else {
                    remove(cell, dx, dy);
                }
            }
        }
        Ok(())
    }
}

/// Original column-major light-byte publication, preserving repeated tiles.
pub fn publish(
    bytes: &mut [Vec<u8>],
    size: usize,
    tiles: &[[usize; 3]],
    light: f32,
) -> Result<(), String> {
    let first = tiles
        .first()
        .ok_or("native light publication requires room tiles")?;
    let floor = first[2]
        .checked_sub(1)
        .and_then(|i| bytes.get_mut(i))
        .ok_or("native light floor absent")?;
    if floor.len() != size * size {
        return Err("invalid native room light grid".into());
    }
    let value = light_byte(light);
    for &[x, y, _] in tiles {
        if x >= size || y >= size {
            return Err("native light tile outside lot".into());
        }
        floor[x * size + y] = value;
    }
    Ok(())
}
