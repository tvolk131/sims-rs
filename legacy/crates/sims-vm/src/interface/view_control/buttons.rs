//! 53b536 dispatch, 53b6bc transition and 53b757 cleanup for ViewControl's
//! pause/speed indication. Cleanup runs even for an equal-state transition.
//! Timer and widget requests must be completed by their actual owners.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub phase: i32,           // ViewControl+1e4
    pub timer_registered: u8, // +1e0
    pub alternate: u8,        // +1ec
}
impl Fields {
    /// The three explicit stores537fed/537ff3/537ff9 only. This does not
    /// construct the parent ViewControl, its widgets or other derived fields.
    pub fn constructed() -> Self {
        Self {
            phase: 0,
            timer_registered: 0,
            alternate: 0,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    /// Fresh fields of the currently registered CPState+d4. The getters
    ///416920/4939c0/4b4513 are direct reads of+8/+40/+44 without callbacks.
    pub mode: i32,
    pub paused: u8,
    pub speed: i32,
    /// App+84 window byte119; only the original mode-change gates read it.
    pub window_blocked: u8,
    /// The current shared timer manager6fe944, not a private local timer.
    pub timer_owner: u32,
    /// Fresh ViewControl+118/+124 widget identities.
    pub pause: u32,
    pub ultra: u32,
}

/// Pure53b536 decision before its only effectful child53b6bc. None means no
/// transition; Some(current_phase) still requires complete cleanup and entry.
pub fn decision(fields: Fields, i: Inputs, reason: i32) -> Option<i32> {
    let active = i.mode == 2 && i.window_blocked == 0;
    let running = || {
        if fields.alternate != 0 && i.speed != 994 {
            3
        } else {
            0
        }
    };
    match (fields.phase, reason) {
        (0 | 1 | 3, 1) => (i.paused != 0).then_some(2),
        (0, 2) => (fields.alternate != 0).then_some(3),
        (0 | 3, 3) => (!active).then_some(1),
        (1, 3) if active => Some(if i.paused != 0 { 2 } else { running() }),
        (2, 0) => (i.mode == 2).then_some(0),
        (2, 1) => Some(if i.paused != 0 {
            2
        } else if i.mode != 2 {
            1
        } else {
            running()
        }),
        (3, 0) => (fields.alternate == 0).then_some(0),
        (3, 2) => Some(0),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Register {
        manager: u32,
        view: u32,
        interval: u32,
        argument: u32,
    }, //5daf06
    Unregister {
        manager: u32,
        view: u32,
    }, //5daf6c
    Auxiliary {
        widget: u32,
        value: i32,
    }, //5e443a, widget+e4
    Selected {
        widget: u32,
        value: i32,
    }, //5e480a, widget+e0
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Dispatch(i32),
    Transition(i32),
    Cleanup(i32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Cleanup(i32),
    Unregistered(i32),
    CleanupWidgets(i32),
    CleanupSelected,
    CleanupAuxiliary,
    Write,
    Register(i32),
    Registered(i32),
    EnterSelected,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    kind: Kind,
    view: u32,
    target: Option<i32>,
    stage: Stage,
    pending: Option<Request>,
}
impl Call {
    pub fn matches(&self, view: u32, kind: Kind) -> bool {
        self.view == view && self.kind == kind
    }
    pub fn new(view: u32, kind: Kind) -> Self {
        Self {
            kind,
            view,
            target: None,
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
        use Stage as S;
        let valid = match self.kind {
            Kind::Cleanup(_) => self.target.is_none(),
            Kind::Transition(n) => {
                self.target.is_none() && self.stage == S::Entry || self.target == Some(n)
            }
            Kind::Dispatch(_) => {
                self.stage == S::Entry || self.stage == S::Complete || self.target.is_some()
            }
        } && match self.stage {
            S::Register(n) | S::Registered(n) => matches!(n, 2 | 3) && self.target == Some(n),
            S::EnterSelected => self.target == Some(1),
            S::Unregistered(n) => matches!(n, 2 | 3),
            _ => true,
        };
        let pending = match (self.stage, self.pending) {
            (_, None) => true,
            (S::Unregistered(2 | 3), Some(Request::Unregister { view, .. })) => view == self.view,
            (
                S::Registered(2 | 3),
                Some(Request::Register {
                    view,
                    interval: 500,
                    argument: 0,
                    ..
                }),
            ) => view == self.view,
            (S::CleanupSelected, Some(Request::Auxiliary { value: 0, .. })) => true,
            (S::CleanupAuxiliary, Some(Request::Selected { value: 0, .. })) => true,
            (
                S::Write,
                Some(Request::Auxiliary { value: 0, .. } | Request::Selected { value: 0, .. }),
            ) => true,
            (S::EnterSelected, Some(Request::Auxiliary { value: 1, .. })) => true,
            (S::Complete, Some(Request::Selected { value: 1, .. })) => {
                matches!(self.target, Some(1 | 2))
            }
            _ => false,
        };
        if valid && pending {
            Ok(())
        } else {
            Err("ViewControl button caller lost its continuation".into())
        }
    }
    pub fn acknowledge(&mut self, r: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(r) {
            return Err("ViewControl button reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, f: &mut Fields, i: Inputs) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        use Stage as S;
        let request = loop {
            match self.stage {
                S::Entry => {
                    let old = match self.kind {
                        Kind::Dispatch(reason) => {
                            self.target = decision(*f, i, reason);
                            if self.target.is_none() {
                                self.stage = S::Complete;
                                return Ok(None);
                            }
                            f.phase
                        }
                        Kind::Transition(value) => {
                            self.target = Some(value);
                            f.phase
                        }
                        Kind::Cleanup(value) => value,
                    };
                    self.stage = S::Cleanup(old);
                }
                S::Cleanup(old) => {
                    self.stage = S::CleanupWidgets(old);
                    if matches!(old, 2 | 3) && f.timer_registered != 0 {
                        self.stage = S::Unregistered(old);
                        break Request::Unregister {
                            manager: i.timer_owner,
                            view: self.view,
                        };
                    }
                }
                S::Unregistered(old) => {
                    f.timer_registered = 0;
                    self.stage = S::CleanupWidgets(old);
                }
                S::CleanupWidgets(old) => {
                    self.stage = S::Write;
                    match old {
                        1 => {
                            self.stage = S::CleanupSelected;
                            break Request::Auxiliary {
                                widget: i.pause,
                                value: 0,
                            };
                        }
                        2 => {
                            self.stage = S::CleanupAuxiliary;
                            break Request::Selected {
                                widget: i.pause,
                                value: 0,
                            };
                        }
                        3 if i.speed != 994 => {
                            break Request::Selected {
                                widget: i.ultra,
                                value: 0,
                            };
                        }
                        _ => (),
                    }
                }
                S::CleanupSelected => {
                    self.stage = S::Write;
                    break Request::Selected {
                        widget: i.pause,
                        value: 0,
                    };
                }
                S::CleanupAuxiliary => {
                    self.stage = S::Write;
                    break Request::Auxiliary {
                        widget: i.pause,
                        value: 0,
                    };
                }
                S::Write => {
                    self.stage = S::Complete;
                    if let Some(value) = self.target {
                        f.phase = value;
                        match value {
                            1 => {
                                self.stage = S::EnterSelected;
                                break Request::Auxiliary {
                                    widget: i.pause,
                                    value: 1,
                                };
                            }
                            2 | 3 => self.stage = S::Register(value),
                            _ => (),
                        }
                    }
                }
                S::Register(value) => {
                    if f.timer_registered == 0 {
                        self.stage = S::Registered(value);
                        break Request::Register {
                            manager: i.timer_owner,
                            view: self.view,
                            interval: 500,
                            argument: 0,
                        };
                    }
                    self.stage = S::Complete;
                    if value == 2 {
                        break Request::Selected {
                            widget: i.pause,
                            value: 1,
                        };
                    }
                }
                S::Registered(value) => {
                    f.timer_registered = 1;
                    self.stage = S::Complete;
                    if value == 2 {
                        break Request::Selected {
                            widget: i.pause,
                            value: 1,
                        };
                    }
                }
                S::EnterSelected => {
                    self.stage = S::Complete;
                    break Request::Selected {
                        widget: i.pause,
                        value: 1,
                    };
                }
                S::Complete => return Ok(None),
            }
        };
        self.pending = Some(request);
        Ok(Some(request))
    }
}
