//! ViewControl53b4b1 timer callback. cTSWinProc event1A at5dcc48 forwards to
//! virtual118 and reports success after it returns; the event argument is not
//! forwarded. Every callback unregisters first and rearms last, even when no
//! widget changes. It never writes ViewControl's timer-registered flag.
use super::buttons::{Fields, Inputs, Request};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetState {
    pub selected: i32,
    pub auxiliary: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Toggle,
    Register,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Callback {
    view: u32,
    stage: Stage,
    pending: Option<Request>,
}
impl Callback {
    pub fn new(view: u32) -> Self {
        Self {
            view,
            stage: Stage::Entry,
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
        let valid = match (self.stage, self.pending) {
            (_, None) => true,
            (Stage::Toggle, Some(Request::Unregister { view, .. })) => view == self.view,
            (
                Stage::Register,
                Some(
                    Request::Selected { value: 0 | 1, .. }
                    | Request::Auxiliary { value: 0 | 1, .. },
                ),
            ) => true,
            (
                Stage::Complete,
                Some(Request::Register {
                    view,
                    interval: 500,
                    argument: 0,
                    ..
                }),
            ) => view == self.view,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("ViewControl timer callback lost its child".into())
        }
    }
    pub fn acknowledge(&mut self, r: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(r) {
            return Err("ViewControl timer callback reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(
        &mut self,
        f: &Fields,
        i: Inputs,
        widgets: &BTreeMap<u32, WidgetState>,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        let r = loop {
            match self.stage {
                Stage::Entry => {
                    self.stage = Stage::Toggle;
                    break Request::Unregister {
                        manager: i.timer_owner,
                        view: self.view,
                    };
                }
                Stage::Toggle => {
                    let r = match f.phase {
                        3 if i.speed != 994 => {
                            let w = widgets
                                .get(&i.ultra)
                                .filter(|_| i.ultra != 0)
                                .ok_or("ViewControl Ultra widget absent")?;
                            Some(Request::Selected {
                                widget: i.ultra,
                                value: i32::from(w.selected == 0),
                            })
                        }
                        2 if i.pause != 0 => {
                            let w = widgets
                                .get(&i.pause)
                                .ok_or("ViewControl pause widget absent")?;
                            Some(Request::Auxiliary {
                                widget: i.pause,
                                value: i32::from(w.auxiliary == 0),
                            })
                        }
                        _ => None,
                    };
                    self.stage = Stage::Register;
                    if let Some(r) = r {
                        break r;
                    }
                }
                Stage::Register => {
                    self.stage = Stage::Complete;
                    break Request::Register {
                        manager: i.timer_owner,
                        view: self.view,
                        interval: 500,
                        argument: 0,
                    };
                }
                Stage::Complete => return Ok(None),
            }
        };
        self.pending = Some(r);
        Ok(Some(r))
    }
}
