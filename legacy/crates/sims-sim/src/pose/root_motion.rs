//! Ordinary 58f6eb..58f86a anchor correction. The caller supplies live native
//! gate inputs after the first model/skin pass, including the captured entry byte.
use super::{
    animator::{Animator, Slot},
    model::Matrix,
    transform::Placement,
};
use crate::head_look::rig::AbsoluteBone;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// The ordinary entry's saved 6f6d70 byte, possibly changed by movement.
    pub enabled: u8,
    /// Animator +188 movement status, tested for exact equality with one.
    pub movement_status: i32,
    /// Whether global 6f6bc4 equals this animator's person pointer.
    pub excluded_person: bool,
    /// Signed word returned by 46d78b on global 6e48bc's +1c object.
    pub world_word_30: i16,
    /// Live animator +190 after the first model/skin pass.
    pub movement_mode: i32,
}

impl Animator {
    /// Return true only when the caller must execute another native placement
    /// publication. The reference anchor persists; accepted corrections do not
    /// recapture it. No physical route or gameplay clock advances here.
    pub fn correct_root(
        &mut self,
        absolute: &[AbsoluteBone],
        placement: &mut Placement,
        matrix: Matrix,
        source: Source,
    ) -> Result<bool, String> {
        if self.busy() {
            return Err("native root correction during a pending skin pass".into());
        }
        if source.enabled != 1
            || self.slot(Slot::Foreground).is_none()
            || source.movement_status != 1
            || source.excluded_person
            || source.world_word_30 > 0x2f8
            || !matches!(source.movement_mode, 6 | 8..=10)
        {
            return Ok(false);
        }
        let Some(bone) = self.anchor_bone else {
            self.anchor_valid = false;
            return Ok(false);
        };
        let bone = absolute.get(bone).ok_or("native root anchor bone absent")?;
        // 595f32/5c39a0 transform the bone origin, then retain only its Z.
        // The actor's +38 matrix is unscaled; its cached kind is zeroed by the
        // first model pass. Do not substitute the scaled skeleton matrix.
        let bone_matrix = bone.matrix()?;
        let point = matrix.point([0., 0., bone_matrix[14]])?;
        if !self.anchor_valid {
            self.anchor_point = point;
            self.anchor_valid = true;
            return Ok(false);
        }
        let delta: [f32; 3] = std::array::from_fn(|i| self.anchor_point[i] - point[i]);
        let scale = 16. / 3.;
        let dx = f64::from(delta[0]) * scale;
        let dy = (f64::from(delta[2]) * scale) as f32;
        // The first component remains extended through the magnitude test and
        // final addition; the Z component spills before both operations.
        let square = dx * dx + f64::from(dy) * f64::from(dy);
        if square <= 10000. && square > f64::from(0.01_f32) {
            let result = [
                (f64::from(placement.base[0]) + f64::from(dy)) as f32,
                (f64::from(placement.base[1]) + dx) as f32,
            ];
            if result.iter().any(|v| !v.is_finite()) {
                return Err("native root correction overflow".into());
            }
            placement.base[..2].copy_from_slice(&result);
            return Ok(true);
        }
        Ok(false)
    }
}
