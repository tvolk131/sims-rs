//! Ordinary Animator584898/584978/584bee, with synchronous service ownership.
//! The initial model gate is captured once; later pointers/vector entries are
//! read live. Destroy services include the matching allocator release before
//! acknowledgment clears the owner's pointer. No person-only cleanup runs.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub model: Option<u64>,
    /// Foreground, outgoing blend, base (no person carry slot).
    pub layers: [Option<u64>; 3],
    pub auxiliary: Vec<Option<u64>>,
    pub generic: Vec<Option<u64>>,
    pub focus: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Vector {
    Auxiliary,
    Generic,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Storage {
    Name,
    Auxiliary,
    Generic,
    Layers,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Reset,
    ClearAppearances,
    DetachLayer(u64),
    DestroyModel(u64),
    ClearFocus,
    DestroyRecord {
        vector: Vector,
        index: usize,
        record: u64,
    },
    Storage(Storage),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Reset,
    Clear,
    Layer(usize),
    Model,
    Focus,
    Record(Vector, usize),
    Storage(usize),
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    stage: Stage,
    pending: Option<Request>,
}
impl Default for Execution {
    fn default() -> Self {
        Self {
            stage: Stage::Start,
            pending: None,
        }
    }
}
impl Execution {
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done && self.pending.is_none()
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn validate(&self, state: &State) -> Result<(), String> {
        let vector = |kind| match kind {
            Vector::Auxiliary => &state.auxiliary,
            Vector::Generic => &state.generic,
        };
        if state.model == Some(0)
            || state.layers.contains(&Some(0))
            || matches!(self.stage, Stage::Layer(i) if i > 3)
            || matches!(self.stage, Stage::Record(k, i) if i > vector(k).len())
            || matches!(self.stage, Stage::Storage(i) if i >= 4)
            || self.pending.is_some_and(|r| match (self.stage, r) {
                (Stage::Reset, Request::Reset)
                | (Stage::Clear, Request::ClearAppearances)
                | (Stage::Focus, Request::ClearFocus) => false,
                (Stage::Layer(i), Request::DetachLayer(id)) => i >= 3 || id == 0,
                (Stage::Model, Request::DestroyModel(id)) => id == 0,
                (
                    Stage::Record(k, i),
                    Request::DestroyRecord {
                        vector: v, index, ..
                    },
                ) => k != v || i != index || i >= vector(k).len(),
                (Stage::Storage(i), Request::Storage(s)) => Self::storage(i) != Some(s),
                _ => true,
            })
        {
            return Err("invalid ordinary Animator destruction continuation".into());
        }
        Ok(())
    }
    fn storage(i: usize) -> Option<Storage> {
        [
            Storage::Name,
            Storage::Auxiliary,
            Storage::Generic,
            Storage::Layers,
        ]
        .get(i)
        .copied()
    }
    pub fn advance(&mut self, s: &mut State) -> Result<Option<Request>, String> {
        self.validate(s)?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            let request = match self.stage {
                Stage::Start => {
                    self.stage = if s.model.is_some() {
                        Stage::Reset
                    } else {
                        Stage::Focus
                    };
                    continue;
                }
                Stage::Reset => Request::Reset,
                Stage::Clear => Request::ClearAppearances,
                Stage::Layer(i) if i < 3 => {
                    if let Some(id) = s.layers[i] {
                        Request::DetachLayer(id)
                    } else {
                        self.stage = Stage::Layer(i + 1);
                        continue;
                    }
                }
                Stage::Layer(_) => {
                    self.stage = Stage::Model;
                    continue;
                }
                Stage::Model => {
                    if let Some(id) = s.model {
                        Request::DestroyModel(id)
                    } else {
                        self.stage = Stage::Focus;
                        continue;
                    }
                }
                Stage::Focus if s.focus => Request::ClearFocus,
                Stage::Focus => {
                    self.stage = Stage::Record(Vector::Auxiliary, 0);
                    continue;
                }
                Stage::Record(kind, i) => {
                    let v = match kind {
                        Vector::Auxiliary => &mut s.auxiliary,
                        Vector::Generic => &mut s.generic,
                    };
                    if i == v.len() {
                        self.stage = if kind == Vector::Auxiliary {
                            // 584bee erases its auxiliary vector after visiting
                            // every slot. Generic pointers are zeroed in place.
                            v.clear();
                            Stage::Record(Vector::Generic, 0)
                        } else {
                            Stage::Storage(0)
                        };
                        continue;
                    }
                    if let Some(record) = v[i] {
                        Request::DestroyRecord {
                            vector: kind,
                            index: i,
                            record,
                        }
                    } else {
                        self.stage = Stage::Record(kind, i + 1);
                        continue;
                    }
                }
                Stage::Storage(i) => {
                    Request::Storage(Self::storage(i).ok_or("destructor storage cursor")?)
                }
                Stage::Done => return Ok(None),
            };
            self.pending = Some(request);
            return Ok(self.pending);
        }
    }
    pub fn acknowledge(&mut self, s: &mut State, request: Request) -> Result<(), String> {
        self.validate(s)?;
        if self.pending != Some(request) {
            return Err("stale ordinary Animator destructor acknowledgment".into());
        }
        self.stage = match self.stage {
            Stage::Reset => Stage::Clear,
            Stage::Clear => Stage::Layer(0),
            Stage::Layer(i) => {
                s.layers[i] = None;
                Stage::Layer(i + 1)
            }
            Stage::Model => {
                s.model = None;
                Stage::Focus
            }
            Stage::Focus => {
                s.focus = false;
                Stage::Record(Vector::Auxiliary, 0)
            }
            Stage::Record(k, i) => {
                (match k {
                    Vector::Auxiliary => &mut s.auxiliary,
                    Vector::Generic => &mut s.generic,
                })[i] = None;
                Stage::Record(k, i + 1)
            }
            Stage::Storage(i) => {
                if i == 3 {
                    Stage::Done
                } else {
                    Stage::Storage(i + 1)
                }
            }
            _ => return Err("ordinary Animator destructor has no service".into()),
        };
        self.pending = None;
        Ok(())
    }
}
