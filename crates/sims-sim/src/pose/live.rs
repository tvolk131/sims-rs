//! One actor's resumable native skin pass and its real property callbacks.
//! The household host retains person storage and completes requested services.
use super::{
    animator::Animator,
    properties::{Context, Handler, Request},
    sampling::BoneInput,
};
use serde::{Deserialize, Serialize};
use sims_vm::Program;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Live {
    pub animator: Animator,
    pub bones: Vec<BoneInput>,
    handler: Option<Handler>,
}
impl Live {
    pub fn new(animator: Animator, bones: Vec<BoneInput>) -> Self {
        Self {
            animator,
            bones,
            handler: None,
        }
    }
    pub fn begin(
        &mut self,
        now: i32,
        sample_pose: bool,
        callback_registered: bool,
        force: bool,
    ) -> Result<bool, String> {
        if self.handler.is_some() {
            return Err("native property callback is pending".into());
        }
        self.animator.begin(
            now,
            sample_pose,
            callback_registered,
            force,
            &mut self.bones,
        )
    }
    pub fn next(
        &mut self,
        program: &Program,
        person: &mut BTreeMap<i16, i16>,
        context: Context,
    ) -> Result<Option<Request>, String> {
        loop {
            if let Some(handler) = &mut self.handler {
                if let Some(request) = handler.next(&mut self.animator, person, context)? {
                    return Ok(Some(request));
                }
                self.animator.acknowledge()?;
                self.handler = None;
            }
            let Some(event) = self.animator.next(program, &mut self.bones)? else {
                return Ok(None);
            };
            self.handler = Some(Handler::new(event));
        }
    }
    pub fn acknowledge(&mut self, surface: Option<i32>) -> Result<(), String> {
        self.handler
            .as_mut()
            .ok_or("no pending native property callback")?
            .acknowledge(surface)
    }
    pub fn busy(&self) -> bool {
        self.handler.is_some() || self.animator.busy()
    }
}
