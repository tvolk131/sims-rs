//! Base Animator placement, 5867d3. The ordinary calculator shares terrain and
//! matrix math with 592132 but has no person previous-position/pool work.
//! Explicit transforms instead call the owner's +78 before updating cached XY.
use super::{model::Matrix, transform};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub base: [f32; 3],
    pub offset: [f32; 3],
    pub dirty: bool,
    /// Animator +108. Selection attachment sets this after binding a matrix.
    pub explicit: bool,
}
impl Placement {
    pub fn constructed() -> Self {
        Self {
            base: [-999.; 3],
            offset: [0.; 3],
            dirty: true,
            explicit: false,
        }
    }
    /// 584e03, invoked by the base Reset: only sentinel caches/dirty change.
    pub fn invalidate(&mut self) {
        self.base = [-999.; 3];
        self.dirty = true;
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Source {
    pub position: [i32; 2],
    pub facing: i16,
    pub level: i32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    /// Actual 585515 model/skin matrix publication must finish before the ack.
    Publish(Matrix),
    /// Actual owner virtual +78, in native X/Y order. The cXObject entry is
    /// 51d372 (a verified no-op); an override may perform observable work.
    Position([i32; 2]),
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Update {
    pending: Option<Request>,
    captured: Option<[f32; 2]>,
    complete: bool,
}
impl Update {
    pub fn complete(&self) -> bool {
        self.complete
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.complete && (self.pending.is_some() || self.captured.is_some())
            || matches!(self.pending, Some(Request::Position(_))) != self.captured.is_some()
            || self
                .captured
                .is_some_and(|v| v.iter().any(|n| !n.is_finite()))
            || matches!(&self.pending,Some(Request::Publish(m)) if m.values.iter().any(|n|!n.is_finite()))
        {
            return Err("invalid base Animator placement continuation".into());
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        placement: &mut Placement,
        matrix: Matrix,
        source: Source,
        view: transform::View,
        terrain: transform::Terrain<'_>,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.complete || self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        let request = if placement.explicit {
            let x = (f64::from(matrix.values[12]) * 0.1875) as f32;
            let z = f64::from(matrix.values[14]) * 0.1875;
            let integer = |v: f64| -> Result<i32, String> {
                let v = v.trunc();
                if !v.is_finite() || v < f64::from(i32::MIN) || v > f64::from(i32::MAX) {
                    return Err("base Animator position conversion overflow".into());
                }
                Ok(v as i32)
            };
            // X spills to f32 before __ftol; Z stays in x87 until its integer
            // conversion. Their asymmetry is observable at large coordinates.
            let position = [integer(f64::from(x))?, integer(z)?];
            self.captured = Some([z as f32, x]);
            Request::Position(position)
        } else {
            let mut temporary = transform::Placement {
                base: placement.base,
                offset: placement.offset,
                previous: [0.; 3],
                dirty: placement.dirty,
            };
            let result = temporary.transform(
                transform::Source {
                    position: source.position,
                    facing: source.facing,
                    level: source.level,
                    flags: [1; 2],
                },
                view,
                terrain,
            )?;
            placement.base = temporary.base;
            Request::Publish(Matrix {
                values: result.matrix,
                kind: 0,
            })
        };
        self.pending = Some(request);
        self.validate()?;
        Ok(self.pending.clone())
    }
    pub fn acknowledge(
        &mut self,
        placement: &mut Placement,
        expected: &Request,
    ) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(expected) {
            return Err("stale base Animator placement acknowledgment".into());
        }
        if let Some([y, x]) = self.captured.take() {
            // Preserve all callback writes except these two original stores.
            placement.base[1] = x;
            placement.base[0] = y;
        }
        self.pending = None;
        self.complete = true;
        Ok(())
    }
}
