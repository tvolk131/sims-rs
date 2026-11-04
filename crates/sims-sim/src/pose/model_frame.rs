//! Shared 5a0ef5/5a0ffd frame services for a retained model/skin instance.
//! Property callbacks are exposed verbatim: an ordinary object must not borrow
//! the person's property handler merely because both use the same skeleton.
use super::{
    animator::Event,
    model::{Bound, Matrix},
};
use serde::{Deserialize, Serialize};
use sims_vm::Program;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skin {
    now: i32,
    sample: bool,
    callbacks: bool,
    started: bool,
    complete: bool,
    pending: Option<Event>,
}
impl Skin {
    pub fn new(now: i32, sample: bool, callbacks: bool) -> Self {
        Self {
            now,
            sample,
            callbacks,
            started: false,
            complete: false,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.complete
    }
    pub fn next(&mut self, bound: &mut Bound, program: &Program) -> Result<Option<Event>, String> {
        if self.complete {
            return Ok(None);
        }
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        let skin = &mut bound.skeleton.skin;
        if !self.started {
            if skin.busy() {
                return Err("model skin service already active".into());
            }
            skin.animator.begin(
                self.now,
                self.sample,
                self.callbacks,
                false,
                &mut skin.bones,
            )?;
            self.started = true;
        }
        let event = skin.animator.next(program, &mut skin.bones)?;
        for (bone, delta) in skin.animator.take_bone_generation_deltas() {
            let stamp = bound
                .skeleton
                .hierarchy
                .updates
                .get_mut(bone)
                .ok_or("model skin bone stamp absent")?;
            *stamp = stamp.wrapping_add(delta);
        }
        for (bone, delta) in skin.animator.take_vertex_generation_deltas() {
            let stamp = bound
                .skeleton
                .geometry
                .bone_generations
                .get_mut(bone)
                .ok_or("model skin vertex stamp absent")?;
            *stamp = stamp.wrapping_add(delta);
        }
        self.complete = event.is_none();
        self.pending = event;
        Ok(self.pending.clone())
    }
    pub fn acknowledge(&mut self, bound: &mut Bound, event: &Event) -> Result<(), String> {
        if self.pending.as_ref() != Some(event) {
            return Err("stale model skin callback acknowledgment".into());
        }
        bound.skeleton.skin.animator.acknowledge()?;
        self.pending = None;
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    Property(Event),
    /// 59934b / 5a1103, force = 0, using the actual appearance/mesh pool.
    Geometry,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Skin,
    Hierarchy,
    Geometry,
    Bounds,
    Publish,
    Done,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Update {
    placement: Matrix,
    visible: bool,
    stage: Stage,
    skin: Skin,
    output: Option<Matrix>,
    pending: Option<Request>,
}
impl Update {
    pub fn new(mut placement: Matrix, now: i32, visible: bool, callbacks: bool) -> Self {
        // Both model entries construct a general matrix with 5c37a0, even if
        // the source carries an identity/translation tag.
        placement.kind = 0;
        Self {
            placement,
            visible,
            stage: Stage::Start,
            skin: Skin::new(now, true, callbacks),
            output: None,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn next(
        &mut self,
        bound: &mut Bound,
        program: &Program,
    ) -> Result<Option<Request>, String> {
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        loop {
            let request = match self.stage {
                Stage::Start => {
                    if bound.skeleton.skin.busy() {
                        return Err("model frame during skin callback".into());
                    }
                    self.output = Some(bound.model.prepare_with_control(
                        self.placement,
                        bound.skeleton.skin.animator.fp_control(),
                    )?);
                    self.stage = if self.visible {
                        Stage::Skin
                    } else {
                        Stage::Publish
                    };
                    continue;
                }
                Stage::Skin => {
                    if let Some(event) = self.skin.next(bound, program)? {
                        Request::Property(event)
                    } else {
                        self.stage = Stage::Hierarchy;
                        continue;
                    }
                }
                Stage::Hierarchy => {
                    bound
                        .skeleton
                        .hierarchy
                        .update(&mut bound.skeleton.skin, false)?;
                    self.stage = Stage::Geometry;
                    continue;
                }
                Stage::Geometry => Request::Geometry,
                Stage::Bounds => {
                    bound.render_bounds = bound.local_bounds;
                    bound.model.bounds_flags &= !1;
                    self.stage = Stage::Publish;
                    continue;
                }
                Stage::Publish => {
                    let transform = self.output.ok_or("model frame output absent")?;
                    bound.skeleton.skin.animator.placement_updated()?;
                    bound.matrix = transform;
                    bound.model.finish(transform);
                    self.stage = Stage::Done;
                    continue;
                }
                Stage::Done => return Ok(None),
            };
            self.pending = Some(request);
            return Ok(self.pending.clone());
        }
    }
    pub fn acknowledge(&mut self, bound: &mut Bound, request: &Request) -> Result<(), String> {
        if self.pending.as_ref() != Some(request) {
            return Err("stale model frame acknowledgment".into());
        }
        match (self.stage, request) {
            (Stage::Skin, Request::Property(event)) => self.skin.acknowledge(bound, event)?,
            (Stage::Geometry, Request::Geometry) => self.stage = Stage::Bounds,
            _ => return Err("invalid model frame continuation".into()),
        }
        self.pending = None;
        Ok(())
    }
}
