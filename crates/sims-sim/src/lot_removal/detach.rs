//!Actual virtual+50 for deletion callers, including cursor421963.
use crate::{Simulation, Step, lot_cursor::rotation, lot_placement};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, ThreadId, Vm, World};

//Physical retains the exact representation of pre-cursor saved callers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Detach {
    Physical(lot_placement::live::Execution),
    Cursor {
        cursor_object: ObjectId,
        cursor_removal: Box<rotation::Execution>,
    },
}
impl Detach {
    pub fn new(
        world: &World,
        links: &crate::lot_links::Links,
        object: ObjectId,
    ) -> Result<Self, String> {
        let receiver = world
            .objects
            .get(&object)
            .ok_or("virtual removal receiver absent")?;
        if receiver.cursor.is_some() {
            Ok(Self::Cursor {
                cursor_object: object,
                cursor_removal: Box::new(rotation::Execution::remove_object(world, object)?),
            })
        } else {
            Ok(Self::Physical(lot_placement::live::Execution::new_virtual(
                if receiver.multitile.is_some() {
                    lot_placement::VirtualOperation::remove(world, links, object, true)?
                } else {
                    //People share base433125 at virtual+50. Preserve the
                    //existing person-removal adapter and its separate Reset.
                    lot_placement::VirtualOperation::Base(lot_placement::Operation::remove(
                        object, true,
                    ))
                },
            )))
        }
    }
    pub fn target(&self) -> ObjectId {
        match self {
            Self::Physical(e) => e.operation.binding().0,
            Self::Cursor { cursor_object, .. } => *cursor_object,
        }
    }
    pub fn complete(&self) -> bool {
        match self {
            Self::Physical(e) => e.complete,
            Self::Cursor { cursor_removal, .. } => cursor_removal.complete(),
        }
    }
    pub fn owns_callback(&self, v: &Vm, p: &Program, t: ThreadId, d: usize) -> bool {
        match self {
            Self::Physical(e) => e.owns_callback(v, p, t, d),
            Self::Cursor { cursor_removal, .. } => cursor_removal.owns_callback(v, p, t, d),
        }
    }
    pub fn validate(&self, s: &Simulation, p: &Program, object: ObjectId) -> Result<(), String> {
        let o =
            s.vm.world
                .objects
                .get(&object)
                .ok_or("virtual removal receiver absent")?;
        if self.target() != object {
            return Err("virtual removal receiver differs".into());
        }
        match self {
            Self::Physical(e) => {
                if o.cursor.is_some()
                    || e.operation.binding() != (object, None)
                    || o.multitile.is_some()
                        != matches!(e.operation, lot_placement::VirtualOperation::Group(_))
                {
                    return Err("physical removal virtual dispatch differs".into());
                }
                e.validate(&s.vm, p, object)
            }
            Self::Cursor { cursor_removal, .. } => {
                let h =
                    s.vm.world
                        .scheduler_storage
                        .as_ref()
                        .and_then(|r| r.handle(object))
                        .ok_or("cursor removal lifetime absent")?;
                if o.cursor.is_none() || !cursor_removal.is_removal(h) {
                    return Err("cursor removal virtual dispatch differs".into());
                }
                cursor_removal.validate(s, p)
            }
        }
    }
    pub fn graphic(&self) -> Option<&lot_placement::live::Execution> {
        match self {
            Self::Physical(e) => Some(e)
                .filter(|e| matches!(e.pending, Some(lot_placement::Request::RefreshAnimator(_)))),
            Self::Cursor { cursor_removal, .. } => cursor_removal.graphic(),
        }
    }
    pub fn graphic_mut(&mut self) -> Option<&mut lot_placement::live::Execution> {
        match self {
            Self::Physical(e) => Some(e)
                .filter(|e| matches!(e.pending, Some(lot_placement::Request::RefreshAnimator(_)))),
            Self::Cursor { cursor_removal, .. } => cursor_removal.graphic_mut(),
        }
    }
    pub fn advance(
        &mut self,
        s: &mut Simulation,
        p: &Program,
        left: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        match self {
            Self::Physical(e) => s.drive_removal_placement(p, e, left, step),
            Self::Cursor { cursor_removal, .. } => cursor_removal.advance(s, p, left, step),
        }
    }
}
