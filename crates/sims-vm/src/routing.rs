//! Native relative-position descriptor construction (Sims 1.91, 0x0045c634).
use serde::{Deserialize, Serialize};
use sims_formats::execution::Slot;

/// Object-owned native routing descriptor (15 words), separate from the source
/// IFF. GoHere/shoo objects replace their first descriptor after construction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Descriptor(pub [u32; 15]);
impl Descriptor {
    pub fn slot(&self) -> Slot {
        let w = self.0;
        Slot {
            kind: 3,
            offset: [
                f32::from_bits(w[0]),
                f32::from_bits(w[1]),
                f32::from_bits(w[2]),
            ],
            standing: w[4] as i32,
            sitting: w[5] as i32,
            ground: w[6] as i32,
            flags: w[7],
            snap_target: w[8] as i32,
            proximity: [w[9] as i32, w[10] as i32, w[11] as i32],
            gradient: f32::from_bits(w[12]),
            height: 0,
            facing: w[13] as i32,
            resolution: w[14] as i32,
            max_size: 0,
            unknown: 0,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let s = self.slot();
        if s.offset.iter().any(|v| !v.is_finite() || v.abs() > 512.)
            || !s.gradient.is_finite()
            || s.gradient.abs() > 10000.
            || !(1..=512).contains(&s.resolution)
            || !(-3..=7).contains(&s.facing)
        {
            return Err("invalid object-owned route descriptor".into());
        }
        Ok(())
    }
}

/// Original 0x4520e9, including the coincident-point result and strict 2:1 cutoffs.
pub fn direction(from: [i32; 2], to: [i32; 2]) -> i16 {
    let dx = to[0].wrapping_sub(from[0]);
    let dy = to[1].wrapping_sub(from[1]);
    let x = dx.wrapping_abs();
    let y = dy.wrapping_abs();
    let mut d = if y.wrapping_mul(2) < x {
        4
    } else {
        2 + i16::from(y <= x.wrapping_mul(2))
    };
    if dy < 0 {
        if d == 2 {
            d = 6;
        } else if d == 3 {
            d = 5;
        }
    }
    if dx < 0 {
        if d == 3 {
            d = 1;
        } else if d == 4 {
            d = 0;
        } else if d == 5 {
            d = 7;
        }
    }
    d
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Destination {
    Slot {
        source: String,
        chunk: u16,
        index: usize,
        version: u32,
        slot: Slot,
    },
    Relative(RelativePosition),
    /// Snap mode 1: use the target's first containment slot.
    FirstSlot,
    /// Snap mode 2: place one tile in front of the target, preserving facing.
    Front,
}
impl Destination {
    pub fn slot(&self) -> Result<(&Slot, u32), String> {
        match self {
            Self::Slot { slot, version, .. } => Ok((slot, *version)),
            _ => Err("inline destination has no SLOT resource".into()),
        }
    }
}

/// Native route descriptor fields used by this wrapper. Coordinates/proximities
/// are already in sixteenth-tile units, independent of serialized SLOT versions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelativePosition {
    pub flags: u32,
    pub proximity: [i32; 3],
    pub facing: i32,
}
impl RelativePosition {
    /// Exact initialized 60-byte native descriptor, for differential probes.
    pub fn words(&self) -> [u32; 15] {
        [
            0,
            0,
            0,
            u32::MAX,
            1,
            0,
            0,
            self.flags,
            u32::MAX,
            self.proximity[0] as u32,
            self.proximity[1] as u32,
            self.proximity[2] as u32,
            0.1875f32.to_bits(),
            self.facing as u32,
            16,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preparation {
    Continue,
    InvalidFacing,
    Start(RelativePosition),
}

pub fn relative_position(operands: &[u8; 8], continuation: u32) -> Preparation {
    if continuation != 0 {
        return Preparation::Continue;
    }
    let side = operands[2] as i8;
    let (directions, proximity) = match side {
        0 | 2 | 4 | 6 => (1 << side, [16; 3]),
        1 | 3 | 5 | 7 => ((1 << (side - 1)) | (1 << ((side + 1) & 7)), [16; 3]),
        -2 => (0, [16; 3]),
        _ => (0x55, [16, 32, 16]),
    };
    let facing = match operands[3] as i8 {
        -2 => -2,
        -1 => -3,
        n @ 0..=7 => i32::from(n),
        _ => return Preparation::InvalidFacing,
    };
    Preparation::Start(RelativePosition {
        flags: directions
            | if operands[6] & 2 == 0 { 0x4000 } else { 0 }
            | if operands[6] & 4 != 0 { 0x8000 } else { 0 },
        proximity,
        facing,
    })
}

/// Native containment slot load, 0x46def8. Category zero is inferred; fixed
/// surface categories override the file's Z, while explicit categories keep it.
pub fn containment_height(category: i32, z: f32) -> (i32, f32) {
    match category {
        0 => (if z == 0. { 1 } else { 5 }, z),
        1 => (1, 0.),
        2 => (2, 2.5),
        3 | 8 => (category, 4.),
        4 => (4, f32::from_bits(0x4081999a)),
        _ => (category, z),
    }
}
#[cfg(test)]
mod domestic_slot_tests {
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_height_conversion_vectors() {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/domestic-slot-height.json"))
                .unwrap();
        for c in doc["cases"].as_array().unwrap() {
            let (category, z) = super::containment_height(
                c["category"].as_i64().unwrap() as i32,
                c["z"].as_f64().unwrap() as f32,
            );
            assert_eq!(
                category,
                c["original_category"].as_i64().unwrap() as i32,
                "{c}"
            );
            assert_eq!(
                z.to_bits(),
                c["original_z_bits"].as_u64().unwrap() as u32,
                "{c}"
            );
        }
    }
}
