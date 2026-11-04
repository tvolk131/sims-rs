//!467482's ordinary tile-light lookup and explicit diagonal continuation.
//! The result is the caller's f32 spill. Split-room selection has a separate
//! room/descriptor owner and is never replaced with a normal-map answer.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sample {
    Value(u32),
    Split { point: [i8; 3] },
}

/// Fields read by467642..467679 from a room already returned by4671f8.
/// A missing room pointer is represented separately, never by a default room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomLight {
    pub light: u32,
    pub pool: bool,
    pub occupied: bool,
    pub lights: i32,
    pub people: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitSelection {
    pub directions: [i32; 2],
    pub room: Option<usize>,
    pub light: u32,
}

///467574..46768d, after the actual wall and room-pair lookups return.
/// Fractions on the retained64-tile map are exact multiples of1/16 under
/// either supported x87 precision. Strict inequalities preserve diagonal ties.
/// This helper does not supply missing room-pair or expanded-wall owners.
pub fn split_sample(
    physical: [i32; 2],
    rotation: u8,
    flags: u32,
    directions: [i32; 2],
    rooms: [Option<RoomLight>; 2],
) -> Result<SplitSelection, String> {
    if rotation > 3 || physical.iter().any(|&p| !(0..1024).contains(&p)) {
        return Err("split light point outside retained map".into());
    }
    if rooms
        .iter()
        .flatten()
        .any(|r| !f32::from_bits(r.light).is_finite())
    {
        return Err("nonfinite split room light requires its x87 owner".into());
    }
    let directions = directions.map(|d| {
        if d == 0 {
            0
        } else {
            (d.wrapping_sub(i32::from(rotation)).wrapping_sub(1) & 3) + 1
        }
    });
    let fine = physical.map(|p| p - ((p >> 4) << 4));
    let wanted = if flags & 0x10 != 0 {
        if fine[0] + fine[1] > 16 {
            2
        } else {
            4
        }
    } else if fine[1] - fine[0] > 0 {
        3
    } else {
        1
    };
    let index = usize::from(directions[0] != 0 && directions[0] != wanted);
    let room = rooms[index];
    let light = match room {
        None => 1f32,
        Some(r) => {
            let light = f32::from_bits(r.light);
            if r.pool || (r.occupied && r.lights == 0 && r.people > 0 && light < 0.8) {
                0.8
            } else {
                light
            }
        }
    };
    Ok(SplitSelection {
        directions,
        room: room.map(|_| index),
        light: light.to_bits(),
    })
}
pub fn map_sample(
    physical: [i32; 2],
    level: i32,
    rotation: u8,
    size: usize,
    primary: &[i16],
    light: &[u8],
) -> Result<Sample, String> {
    if !(1..=64).contains(&size)
        || rotation > 3
        || primary.len() != size * size
        || light.len() != size * size
    {
        return Err("point light map owner/extent differs".into());
    }
    let p = [
        (physical[1] >> 4) as i8,
        (physical[0] >> 4) as i8,
        level as i8,
    ];
    if p[..2].iter().any(|&v| !(0..64).contains(&v)) {
        return Err("point light rotation lookup outside retained table".into());
    }
    let r = crate::lot_presenter::room_masks::rotate(p, 4u8.wrapping_sub(rotation) & 3);
    if r[..2].iter().any(|&v| v as usize >= size) {
        return Err("point light lookup outside retained map".into());
    }
    let index = r[0] as usize * size + r[1] as usize;
    if primary[index] == -5 {
        return Ok(Sample::Split { point: p });
    }
    Ok(Sample::Value(
        (f32::from(light[index]) * f32::from_bits(0x3b808081)).to_bits(),
    ))
}
