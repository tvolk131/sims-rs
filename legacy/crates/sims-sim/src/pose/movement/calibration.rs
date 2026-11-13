//! Movement measurements computed by 58831d after resources/rigs are bound.
//! This is the measurement arithmetic, not the complete appearance constructor.
use crate::pose::events;
use serde::{Deserialize, Serialize};
use sims_formats::animation::Animation;

/// 588df8 scans motions forward, each motion's native event list backward,
/// and properties forward. The first matching motion wins over later timestamps
/// in another motion. Comparison is case sensitive on already-decoded properties.
pub fn property(clip: &Animation, key: &str, value: &str) -> Result<Option<(usize, i32)>, String> {
    for (motion, data) in clip.motions.iter().enumerate() {
        for event in events::stream(data)?.into_iter().rev() {
            if event.properties.iter().any(|(k, v)| k == key && v == value) {
                return Ok(Some((motion, event.time)));
            }
        }
    }
    Ok(None)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Distances {
    pub start_distance: [f32; 2],
    pub lookahead: [f32; 2],
    pub special_lookahead: f32,
    pub short_lookahead: f32,
}
impl Distances {
    /// Distances from original selection indices 19,21,20,22,23, in lookup order.
    /// 588ed7 supplies zero when the requested resource is absent.
    pub fn new(clips: [f32; 5]) -> Result<Self, String> {
        if clips.iter().any(|v| !v.is_finite()) {
            return Err("invalid native movement clip distance".into());
        }
        let scale = f64::from((16. / 3.) as f32);
        let start_distance = [
            (f64::from(clips[0]) * scale) as f32,
            (f64::from(clips[3]) * scale) as f32,
        ];
        let lookahead = [
            (f64::from(clips[2]) * 0.5 * scale) as f32,
            (f64::from(clips[1]) * 0.5 * scale) as f32,
        ];
        Ok(Self {
            start_distance,
            lookahead,
            special_lookahead: (f64::from(clips[4]) * 0.5 * scale) as f32,
            short_lookahead: lookahead[0] * 0.125,
        })
    }
}

pub fn phases(times: [i32; 2], duration: f32) -> Result<[f32; 2], String> {
    if !duration.is_finite() || duration <= 0. {
        return Err("native movement calibration requires nonzero clip duration".into());
    }
    // Native order is anchor=1 then anchor=2; the path stores lower then upper.
    Ok([
        (f64::from(times[1]) / f64::from(duration)) as f32,
        (f64::from(times[0]) / f64::from(duration)) as f32,
    ])
}
/// Absolute bone-origin Z at lower then upper phase; each row is anchor 1, 2.
/// The native first product spills while the second remains extended.
pub fn stop_distances(z: [[f32; 2]; 2]) -> Result<[f32; 2], String> {
    if z.iter().flatten().any(|v| !v.is_finite()) {
        return Err("invalid native movement anchor point".into());
    }
    let scale = f64::from((16. / 3.) as f32);
    let first = |i: usize| f64::from((f64::from(z[i][0]) * scale) as f32);
    Ok([
        (first(1) - f64::from(z[1][1]) * scale) as f32,
        (f64::from(z[0][1]) * scale - first(0)) as f32,
    ])
}
