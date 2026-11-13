//! Base Animator frame control, 58598c (refresh) and 585a46 (ordinary update).
//! Services retain native model/skin identities across callbacks. A response
//! acknowledges completed work; it does not substitute for that work.
use super::{graphic_appearance::Model, graphic_transform::Placement, model::Matrix, ordinary};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Blend {
    pub layer: u64,
    pub weight: f32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub clock_mode: u8,
    pub now: i32,
    /// Preserve the byte: refresh passes it through to 5992b3 when requested.
    pub visible: u8,
    pub blend: Option<Blend>,
    pub placement: Placement,
    pub matrix: Matrix,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Entry {
    Refresh,
    Ordinary,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    ReleaseBlend {
        model: u64,
        layer: u64,
    },
    OutsideLot,
    Placement,
    Viewport,
    Visibility {
        viewport: [i32; 4],
    },
    /// 5a0ef5 (visible) or 5a0ffd (hidden). The visible path samples the skin,
    /// updates hierarchy/geometry/bounds and publishes the captured matrix.
    Model {
        model: u64,
        matrix: Matrix,
        now: i32,
        visible: bool,
    },
    Invalidate,
    /// 5992b3, force = 0. Callback dispatch belongs to the captured skin.
    Skin {
        skin: u64,
        now: i32,
        sample: u8,
    },
    /// 5994c3, force = 0, on the SAME skin captured before placement.
    Hierarchy {
        skin: u64,
    },
    /// 5a15bc, force = 0, using the model getter again after placement.
    Geometry {
        model: u64,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    Outside(bool),
    Viewport([i32; 4]),
    Visible(bool),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Blend,
    Outside,
    InitialPlacement,
    Viewport,
    Visibility,
    Model,
    Invalidate,
    Skin,
    Placement,
    Hierarchy,
    Geometry,
    FinalVisibility,
    Done,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Update {
    entry: Entry,
    only_visible: bool,
    stage: Stage,
    now: i32,
    sample: u8,
    visible: bool,
    viewport: Option<[i32; 4]>,
    skin: Option<u64>,
    pending: Option<Request>,
}
impl Update {
    pub fn new(entry: Entry, only_visible: bool) -> Self {
        Self {
            entry,
            only_visible,
            stage: Stage::Start,
            now: 0,
            sample: 1,
            visible: true,
            viewport: None,
            skin: None,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn validate(&self) -> Result<(), String> {
        let invalid = self
            .pending
            .as_ref()
            .is_some_and(|request| match (self.stage, request) {
                (Stage::Blend, Request::ReleaseBlend { model, layer }) => {
                    *model == 0 || *layer == 0
                }
                (Stage::Outside, Request::OutsideLot)
                | (Stage::InitialPlacement | Stage::Placement, Request::Placement)
                | (Stage::Viewport, Request::Viewport)
                | (Stage::Invalidate, Request::Invalidate) => false,
                (Stage::Visibility | Stage::FinalVisibility, Request::Visibility { viewport }) => {
                    Some(*viewport) != self.viewport
                }
                (
                    Stage::Model,
                    Request::Model {
                        model,
                        matrix,
                        now,
                        visible,
                    },
                ) => {
                    *model == 0
                        || matrix.values.iter().any(|v| !v.is_finite())
                        || matrix.kind != 0
                        || *now != self.now
                        || *visible != self.visible
                }
                (Stage::Skin, Request::Skin { skin, .. })
                | (Stage::Hierarchy, Request::Hierarchy { skin }) => {
                    *skin == 0 || Some(*skin) != self.skin
                }
                (Stage::Geometry, Request::Geometry { model }) => *model == 0,
                _ => true,
            });
        if invalid
            || self.skin == Some(0)
            || matches!(
                self.stage,
                Stage::Placement | Stage::Hierarchy | Stage::Geometry | Stage::FinalVisibility
            ) && self.skin.is_none()
            || matches!(
                self.stage,
                Stage::Visibility | Stage::Model | Stage::FinalVisibility
            ) && self.viewport.is_none()
        {
            return Err("invalid base Animator frame continuation".into());
        }
        Ok(())
    }
    /// Tick and model are live getters. Pending services never reread them.
    pub fn advance(
        &mut self,
        state: &mut State,
        tick: i32,
        model: Option<Model>,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        let current = || -> Result<Model, String> {
            model
                .filter(|m| m.model != 0 && m.skin != 0)
                .ok_or_else(|| "base Animator frame requires a live model and skin".into())
        };
        loop {
            let request = match self.stage {
                Stage::Start => {
                    if model.is_none()
                        || self.entry == Entry::Refresh && self.only_visible && state.visible == 0
                    {
                        self.stage = Stage::Done;
                        continue;
                    }
                    current()?;
                    if self.entry == Entry::Refresh && self.only_visible {
                        self.sample = state.visible;
                    }
                    // Base +38 is 585502 (constant 42); the ordinary extra
                    // call and +58 (46fb64) have no state effects.
                    self.now = ordinary::clock(tick, state.clock_mode);
                    state.now = self.now;
                    self.stage = Stage::Blend;
                    continue;
                }
                Stage::Blend => {
                    if let Some(blend) = state.blend {
                        if blend.layer == 0 || !blend.weight.is_finite() {
                            return Err("invalid base Animator blend".into());
                        }
                        if blend.weight.abs() <= 0.000001 {
                            Request::ReleaseBlend {
                                model: current()?.model,
                                layer: blend.layer,
                            }
                        } else {
                            self.after_blend();
                            continue;
                        }
                    } else {
                        self.after_blend();
                        continue;
                    }
                }
                Stage::Outside => Request::OutsideLot,
                Stage::InitialPlacement => {
                    if state.placement.base.contains(&-999.) {
                        Request::Placement
                    } else {
                        self.stage = Stage::Viewport;
                        continue;
                    }
                }
                Stage::Viewport => Request::Viewport,
                Stage::Visibility | Stage::FinalVisibility => Request::Visibility {
                    viewport: self.viewport.unwrap(),
                },
                Stage::Model => {
                    let model = current()?.model;
                    state.matrix.kind = 0;
                    Request::Model {
                        model,
                        matrix: state.matrix,
                        now: self.now,
                        visible: self.visible,
                    }
                }
                Stage::Invalidate => Request::Invalidate,
                Stage::Skin => {
                    if self.entry == Entry::Ordinary {
                        self.sample = u8::from(state.visible != 0 || !self.only_visible);
                    }
                    let skin = current()?.skin;
                    self.skin = Some(skin);
                    Request::Skin {
                        skin,
                        now: state.now,
                        sample: self.sample,
                    }
                }
                Stage::Placement => Request::Placement,
                Stage::Hierarchy => Request::Hierarchy {
                    skin: self.skin.unwrap(),
                },
                Stage::Geometry => Request::Geometry {
                    model: current()?.model,
                },
                Stage::Done => return Ok(None),
            };
            self.pending = Some(request);
            self.validate()?;
            return Ok(self.pending.clone());
        }
    }
    fn after_blend(&mut self) {
        self.stage = if self.entry == Entry::Ordinary {
            Stage::Outside
        } else {
            Stage::Invalidate
        };
    }
    pub fn acknowledge(
        &mut self,
        state: &mut State,
        request: &Request,
        reply: Reply,
    ) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(request) {
            return Err("stale base Animator frame acknowledgment".into());
        }
        match (self.stage, request, reply) {
            (Stage::Blend, Request::ReleaseBlend { .. }, Reply::Done) => {
                state.blend = None;
                self.after_blend();
            }
            (Stage::Outside, Request::OutsideLot, Reply::Outside(outside)) => {
                self.stage = if outside {
                    Stage::Done
                } else {
                    Stage::InitialPlacement
                }
            }
            (Stage::InitialPlacement, Request::Placement, Reply::Done) => {
                self.stage = Stage::Viewport
            }
            (Stage::Viewport, Request::Viewport, Reply::Viewport(viewport)) => {
                self.viewport = Some(viewport);
                self.stage = if self.only_visible {
                    Stage::Visibility
                } else {
                    Stage::Model
                };
            }
            (Stage::Visibility, Request::Visibility { .. }, Reply::Visible(visible)) => {
                self.visible = visible;
                self.stage = Stage::Model;
            }
            (Stage::Model, Request::Model { .. }, Reply::Done) => {
                state.visible = u8::from(self.visible);
                self.stage = Stage::Invalidate;
            }
            (Stage::Invalidate, Request::Invalidate, Reply::Done) => self.stage = Stage::Skin,
            (Stage::Skin, Request::Skin { .. }, Reply::Done) => self.stage = Stage::Placement,
            (Stage::Placement, Request::Placement, Reply::Done) => self.stage = Stage::Hierarchy,
            (Stage::Hierarchy, Request::Hierarchy { .. }, Reply::Done) => {
                self.stage = Stage::Geometry
            }
            (Stage::Geometry, Request::Geometry { .. }, Reply::Done) => {
                self.stage = if self.entry == Entry::Ordinary {
                    Stage::FinalVisibility
                } else {
                    Stage::Done
                }
            }
            (Stage::FinalVisibility, Request::Visibility { .. }, Reply::Visible(_)) => {
                self.stage = Stage::Done
            }
            _ => return Err("mismatched base Animator frame reply".into()),
        }
        self.pending = None;
        Ok(())
    }
}
