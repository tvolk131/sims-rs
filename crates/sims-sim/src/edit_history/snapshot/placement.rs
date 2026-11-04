//!48bfab: restore one saved placement. Original virtual services remain
//!synchronous; captured object/container and mutable local XY survive replies.
use super::{References, Registry, Snapshot};
use serde::{Deserialize, Serialize};

pub trait View: Registry {
    fn facing(&self, object: u32) -> Result<i16, String>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub object: u32,
    pub position: [i32; 2],
    pub level: i32,
    pub container: Option<u32>,
    pub slot: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ///Virtual+6c(0), not unconditional object deletion.
    RemoveFromLot(u32),
    ///Virtual+4c(1), at most eight invocations.
    Rotate(u32),
    ///Virtual+60, with mutable local position storage.
    Check(Place),
    ///Virtual+64, rereading snapshot level/slot after Check.
    Place(Place),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Check { value: u32, position: [i32; 2] },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Orientation,
    Rotated,
    FinalOrientation,
    Check,
    Place,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    stage: Stage,
    object: Option<u32>,
    container: Option<u32>,
    rotations: u8,
    position: [i32; 2],
    check: u8,
    result: bool,
    pending: Option<Request>,
}
impl Default for Call {
    fn default() -> Self {
        Self::new()
    }
}
impl Call {
    pub fn new() -> Self {
        Self {
            stage: Stage::Entry,
            object: None,
            container: None,
            rotations: 0,
            position: [0; 2],
            check: 0,
            result: false,
            pending: None,
        }
    }
    pub fn result(&self) -> Option<bool> {
        (self.stage == Stage::Complete && self.pending.is_none()).then_some(self.result)
    }
    pub fn validate(&self) -> Result<(), String> {
        let pending = self.pending.is_none_or(|r| match (self.stage, r) {
            (Stage::Orientation, Request::RemoveFromLot(o))
            | (Stage::Rotated, Request::Rotate(o)) => Some(o) == self.object,
            (Stage::Check, Request::Check(p)) | (Stage::Place, Request::Place(p)) => {
                Some(p.object) == self.object
                    && p.container == self.container
                    && p.position == self.position
            }
            _ => false,
        });
        if !pending
            || self.object == Some(0)
            || self.container == Some(0)
            || self.rotations > 8
            || (!matches!(self.stage, Stage::Entry | Stage::Complete) && self.object.is_none())
        {
            Err("snapshot placement lost its captured service".into())
        } else {
            Ok(())
        }
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("snapshot placement reply differs".into());
        }
        match (request, reply) {
            (Request::Check(_), Reply::Check { value, position }) => {
                self.check = value as u8;
                self.position = position;
            }
            (Request::RemoveFromLot(_) | Request::Rotate(_) | Request::Place(_), Reply::Done) => (),
            _ => return Err("snapshot placement reply kind differs".into()),
        }
        self.pending = None;
        Ok(())
    }
    fn args(&self, snapshot: &Snapshot) -> Result<Place, String> {
        Ok(Place {
            object: self.object.ok_or("snapshot placement object absent")?,
            position: self.position,
            level: snapshot.level.ok_or("uncaptured snapshot level")?,
            container: self.container,
            slot: snapshot.slot.ok_or("uncaptured snapshot slot")?,
        })
    }
    pub fn advance(
        &mut self,
        snapshot: &Snapshot,
        refs: &References,
        view: &impl View,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        let request = loop {
            match self.stage {
                Stage::Entry => {
                    self.object = refs.object(snapshot)?;
                    if let Some(object) = self.object {
                        self.stage = Stage::Orientation;
                        break Request::RemoveFromLot(object);
                    }
                    self.stage = Stage::Complete;
                }
                Stage::Orientation => {
                    let object = self.object.unwrap();
                    if i32::from(view.facing(object)?)
                        == snapshot.facing.ok_or("uncaptured snapshot facing")?
                    {
                        self.stage = Stage::FinalOrientation;
                    } else {
                        self.stage = Stage::Rotated;
                        break Request::Rotate(object);
                    }
                }
                Stage::Rotated => {
                    self.rotations += 1;
                    self.stage = if self.rotations < 8 {
                        Stage::Orientation
                    } else {
                        Stage::FinalOrientation
                    };
                }
                Stage::FinalOrientation => {
                    if i32::from(view.facing(self.object.unwrap())?)
                        != snapshot.facing.ok_or("uncaptured snapshot facing")?
                    {
                        self.stage = Stage::Complete;
                        continue;
                    }
                    let container = snapshot.container.ok_or("uncaptured snapshot container")?;
                    self.container = if container == 0 {
                        None
                    } else {
                        view.lookup(container)?
                    };
                    if container != 0 && self.container.is_none() {
                        self.stage = Stage::Complete;
                        continue;
                    }
                    self.position = snapshot.position;
                    let args = self.args(snapshot)?;
                    self.stage = Stage::Check;
                    break Request::Check(args);
                }
                Stage::Check => {
                    if self.check == 0 {
                        self.stage = Stage::Complete;
                        continue;
                    }
                    let args = self.args(snapshot)?;
                    self.stage = Stage::Place;
                    break Request::Place(args);
                }
                Stage::Place => {
                    self.result = true;
                    self.stage = Stage::Complete;
                }
                Stage::Complete => return Ok(None),
            }
        };
        self.pending = Some(request);
        Ok(self.pending)
    }
}
