//! Source-bound 585beb/5a0a8a visibility. No cached boolean substitutes for the
//! live placement, model bounds, scale, camera or viewport calculation.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sims_sim::{
    character::Resident,
    pose::{
        initialize::Initialization,
        model::{Bounds, Matrix},
        model_visibility,
        transform::Placement,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// 415711 -> viewer vtable+10 -> viewer+54 device -> device+ec matrix.
    pub camera: Matrix,
    /// Actual 6d0458 process byte, independent of render selection/visibility.
    pub enabled: bool,
}
impl Source {
    ///590b8b uses the person's actual model pointer instead of585beb's
    /// process-wide6d0458 byte. Placement early returns and projection agree.
    pub fn visible_person(
        self,
        resident: &Resident,
        model: Option<&sims_content::character_model::State>,
        viewport: [i32; 4],
    ) -> Result<bool> {
        if let Some(model) = model {
            Self {
                enabled: true,
                ..self
            }
            .visible(resident, model, viewport)
        } else {
            // The dirty-placement return precedes the native null-model test.
            Ok(resident.character.placement.dirty)
        }
    }
    pub fn visible(
        self,
        resident: &Resident,
        model: &sims_content::character_model::State,
        viewport: [i32; 4],
    ) -> Result<bool> {
        let character = &resident.character;
        self.visible_at(
            &character.placement,
            model.local_bounds,
            character.model.scale,
            Matrix {
                values: character.matrix,
                kind: 0,
            },
            viewport,
        )
    }

    pub fn visible_initialized(
        self,
        initialized: &Initialization,
        viewport: [i32; 4],
    ) -> Result<bool> {
        self.visible_at(
            &initialized.scalars.placement,
            initialized.bound.local_bounds,
            initialized.bound.model.scale,
            Matrix {
                values: initialized.bound.matrix.values,
                kind: 0,
            },
            viewport,
        )
    }

    fn visible_at(
        self,
        position: &Placement,
        bounds: Bounds,
        scale: Matrix,
        matrix: Matrix,
        viewport: [i32; 4],
    ) -> Result<bool> {
        if position.dirty {
            return Ok(true);
        }
        if !(f64::from(position.base[0]) + f64::from(position.offset[0]) >= 0.)
            || !(f64::from(position.base[1]) + f64::from(position.offset[1]) >= 0.)
            || !self.enabled
        {
            return Ok(false);
        }
        let result = model_visibility::project(bounds, scale, matrix, self.camera, viewport)
            .map_err(anyhow::Error::msg)?;
        Ok(result.visible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initialized_visibility_retains_native_early_return_order() -> Result<()> {
        let mut position = Placement {
            base: [-999.; 3],
            offset: [0.; 3],
            previous: [0.; 3],
            dirty: true,
        };
        let source = Source {
            camera: Matrix::IDENTITY,
            enabled: false,
        };
        assert!(source.visible_at(
            &position,
            Bounds::EMPTY,
            Matrix::IDENTITY,
            Matrix::IDENTITY,
            [0; 4]
        )?);
        position.dirty = false;
        assert!(!source.visible_at(
            &position,
            Bounds::EMPTY,
            Matrix::IDENTITY,
            Matrix::IDENTITY,
            [0; 4]
        )?);
        let source = Source {
            enabled: true,
            ..source
        };
        assert!(!source.visible_at(
            &position,
            Bounds::EMPTY,
            Matrix::IDENTITY,
            Matrix::IDENTITY,
            [0; 4]
        )?);
        position.base = [-1., 0., 0.];
        position.offset = [2., 0., 0.];
        let bounds = Bounds {
            minimum: [0.; 3],
            maximum: [1.; 3],
        };
        let expected = model_visibility::project(
            bounds,
            Matrix::IDENTITY,
            Matrix::IDENTITY,
            source.camera,
            [0, 0, 800, 600],
        )
        .map_err(anyhow::Error::msg)?
        .visible;
        assert_eq!(
            source.visible_at(
                &position,
                bounds,
                Matrix::IDENTITY,
                Matrix::IDENTITY,
                [0, 0, 800, 600]
            )?,
            expected
        );
        Ok(())
    }
}
