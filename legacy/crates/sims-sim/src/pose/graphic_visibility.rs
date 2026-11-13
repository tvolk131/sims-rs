//! Ordinary Animator visibility 585beb. Camera acquisition is a service;
//! projection uses the live model and matrix after that service returns.
use super::{graphic_frame::State, model::Matrix};
use serde::{Deserialize, Serialize};

/// Synchronous owner with a retained model. Camera acquisition is lazy, just
/// like 415711. Hosts whose camera service can replace the model should drive
/// `Update` directly with live identities instead.
pub fn evaluate(
    state: &mut State,
    dirty: u8,
    enabled: bool,
    model: Option<&super::model::Bound>,
    viewport: [i32; 4],
    camera: impl FnOnce() -> Result<Matrix, String>,
) -> Result<u8, String> {
    let mut pass = Update::new(viewport);
    let mut camera = Some(camera);
    while let Some(request) = pass.advance(state, dirty, enabled, model.map(|_| 1))? {
        let reply = match &request {
            Request::Camera => {
                let acquire = camera.take().ok_or("repeated visibility camera request")?;
                Reply::Camera(acquire()?)
            }
            Request::Project {
                matrix,
                camera,
                viewport,
                ..
            } => {
                let bound = model.ok_or("ordinary visibility model absent")?;
                let result = super::model_visibility::project(
                    bound.local_bounds,
                    bound.model.scale,
                    *matrix,
                    *camera,
                    *viewport,
                )?;
                Reply::Visible(u8::from(result.visible))
            }
        };
        pass.acknowledge(state, &request, reply)?;
    }
    Ok(state.visible)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    Camera,
    Project {
        model: u64,
        matrix: Matrix,
        camera: Matrix,
        viewport: [i32; 4],
    },
}
pub enum Reply {
    Camera(Matrix),
    Visible(u8),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Camera,
    Project,
    Done,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Update {
    stage: Stage,
    viewport: [i32; 4],
    camera: Option<Matrix>,
    pending: Option<Request>,
}
impl Update {
    pub fn new(viewport: [i32; 4]) -> Self {
        Self {
            stage: Stage::Start,
            viewport,
            camera: None,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.stage == Stage::Project && self.camera.is_none()
            || self
                .pending
                .as_ref()
                .is_some_and(|r| match (self.stage, r) {
                    (Stage::Camera, Request::Camera) => false,
                    (
                        Stage::Project,
                        Request::Project {
                            model,
                            camera,
                            viewport,
                            ..
                        },
                    ) => *model == 0 || Some(*camera) != self.camera || *viewport != self.viewport,
                    _ => true,
                })
        {
            return Err("invalid ordinary visibility continuation".into());
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        state: &mut State,
        dirty: u8,
        enabled: bool,
        model: Option<u64>,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        if self.stage == Stage::Start {
            state.visible = dirty;
            if dirty != 0 {
                self.stage = Stage::Done;
            } else {
                // Both additions remain in x87 until the comparison.
                let p = &state.placement;
                if !(f64::from(p.base[0]) + f64::from(p.offset[0]) >= 0.)
                    || !(f64::from(p.base[1]) + f64::from(p.offset[1]) >= 0.)
                    || !enabled
                    || model.is_none()
                {
                    self.stage = Stage::Done;
                } else {
                    if model == Some(0) {
                        return Err("invalid ordinary graphic model identity".into());
                    }
                    state.matrix.kind = 0;
                    self.stage = Stage::Camera;
                }
            }
        }
        let request = match self.stage {
            Stage::Camera => Request::Camera,
            Stage::Project => Request::Project {
                model: model
                    .filter(|&id| id != 0)
                    .ok_or("ordinary graphic model disappeared during camera service")?,
                matrix: state.matrix,
                camera: self.camera.unwrap(),
                viewport: self.viewport,
            },
            Stage::Done => return Ok(None),
            Stage::Start => unreachable!(),
        };
        self.pending = Some(request);
        Ok(self.pending.clone())
    }
    pub fn acknowledge(
        &mut self,
        state: &mut State,
        request: &Request,
        reply: Reply,
    ) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(request) {
            return Err("stale ordinary visibility acknowledgment".into());
        }
        match (request, reply) {
            (Request::Camera, Reply::Camera(camera)) => {
                self.camera = Some(camera);
                self.stage = Stage::Project;
            }
            (Request::Project { .. }, Reply::Visible(value)) => {
                state.visible = value;
                self.stage = Stage::Done;
            }
            _ => return Err("mismatched ordinary visibility reply".into()),
        }
        self.pending = None;
        Ok(())
    }
}
