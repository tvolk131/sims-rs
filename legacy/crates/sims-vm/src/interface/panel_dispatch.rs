//! Original50a8ec child dispatch. The top ViewControl update always precedes
//! the Arch/Catalog/People/Options/Camera children. Each child requires its real
//! backend; this does not acknowledge their work or reconstruct missing widgets.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    /// Mode and dirty fields from the panel's current+fc CPState owner. It can
    /// differ from the retained argument passed to child updates.
    pub mode: i32,
    pub dirty: u32,
    /// Child identities+dc/+e8/+e0/+e4/+ec/+f4.
    pub controls: Option<u32>,
    pub build: Option<u32>,
    pub buy: Option<u32>,
    pub people: Option<u32>,
    pub options: Option<u32>,
    pub camera: Option<u32>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    Controls,
    Build,
    HideBuild,
    Buy,
    HideBuy,
    People,
    InvalidatePeople,
    Options,
    Camera,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub operation: Operation,
    /// Null is preserved for mandatory callbacks, never silently skipped.
    pub child: Option<u32>,
    /// The original retained CPState argument; absent on no-argument children.
    pub state: Option<u32>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Controls,
    Build,
    Buy,
    People,
    InvalidatePeople,
    Options,
    Camera,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    argument: u32,
    stage: Stage,
    pending: Option<Request>,
}
impl Update {
    pub fn matches(&self, argument: u32) -> bool {
        self.argument == argument
    }
    pub fn new(argument: u32) -> Self {
        Self {
            argument,
            stage: Stage::Controls,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        use Operation as O;
        use Stage as S;
        let valid = if let Some(r) = self.pending {
            let stage = match r.operation {
                O::Controls => S::Build,
                O::Build | O::HideBuild => S::Buy,
                O::Buy | O::HideBuy => S::People,
                O::People => S::InvalidatePeople,
                O::InvalidatePeople => S::Options,
                O::Options => S::Camera,
                O::Camera => S::Complete,
            };
            let state = if matches!(r.operation, O::HideBuild | O::HideBuy | O::InvalidatePeople) {
                None
            } else {
                Some(self.argument)
            };
            self.stage == stage && r.state == state
        } else {
            true
        };
        if valid {
            Ok(())
        } else {
            Err("panel dispatcher lost its child".into())
        }
    }
    pub fn acknowledge(&mut self, r: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(r) {
            return Err("panel dispatcher reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, input: Inputs) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        use Operation as O;
        use Stage as S;
        let (operation, child) = loop {
            match self.stage {
                S::Controls => {
                    self.stage = S::Build;
                    break (O::Controls, input.controls);
                }
                S::Build => {
                    self.stage = S::Buy;
                    if input.build.is_some() {
                        break (
                            if input.mode == 0 {
                                O::Build
                            } else {
                                O::HideBuild
                            },
                            input.build,
                        );
                    }
                }
                S::Buy => {
                    self.stage = S::People;
                    if input.buy.is_some() {
                        break (if input.mode == 1 { O::Buy } else { O::HideBuy }, input.buy);
                    }
                }
                S::People => {
                    self.stage = S::Options;
                    if input.people.is_some() && input.mode == 2 {
                        self.stage = S::InvalidatePeople;
                        break (O::People, input.people);
                    }
                }
                S::InvalidatePeople => {
                    self.stage = S::Options;
                    if input.dirty & 2 != 0 {
                        break (O::InvalidatePeople, input.people);
                    }
                }
                S::Options => {
                    self.stage = S::Camera;
                    if input.options.is_some() && input.mode == 3 {
                        break (O::Options, input.options);
                    }
                }
                S::Camera => {
                    self.stage = S::Complete;
                    if input.camera.is_some() && input.mode == 4 {
                        break (O::Camera, input.camera);
                    }
                }
                S::Complete => return Ok(None),
            }
        };
        let state = if matches!(operation, O::HideBuild | O::HideBuy | O::InvalidatePeople) {
            None
        } else {
            Some(self.argument)
        };
        self.pending = Some(Request {
            operation,
            child,
            state,
        });
        Ok(self.pending)
    }
}
