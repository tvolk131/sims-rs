//! Ordinary 58dc72 prefix: native person clock, carry/base selection and rate.
//! Completion identifies the original movement or shared-tail continuation;
//! it never acknowledges the unimplemented movement controller on its behalf.
use super::{
    animator::{Settings, Slot},
    refresh::Character,
};
use serde::{Deserialize, Serialize};
use sims_formats::avatar::Skeleton;
use sims_vm::{Program, animation::CarrySelector};

/// 585483 with the person's 585502 virtual: 42 is multiplied by the original
/// f32 1.2 and truncated *before* multiplying by the signed lot tick. Nonzero
/// animator +a8 selects 42 instead. The return keeps the low signed 32 bits.
pub fn clock(tick: i32, mode: u8) -> i32 {
    tick.wrapping_mul(if mode == 0 { 50 } else { 42 })
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RateSettings {
    pub base: f32,
    pub positive: f32,
    pub negative: f32,
    pub change: f32,
}
impl RateSettings {
    pub(super) fn target(self, walk_style: i16, species: i16, mode: i32) -> Result<f64, String> {
        if [self.base, self.positive, self.negative, self.change]
            .iter()
            .any(|v| !v.is_finite())
        {
            return Err("invalid native movement-rate settings".into());
        }
        let multiplier = if walk_style == 1 && (mode == 0 || (4..=9).contains(&mode)) {
            if mode != 0 && species > 1 { 0. } else { 8. }
        } else {
            1.
        };
        let slope = if multiplier < 0. {
            self.negative
        } else {
            self.positive
        };
        Ok(f64::from(self.base) + f64::from(slope) * multiplier)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// Animator +a8, +198, +1a4, +1a8/+1ac, +190, +228 and +22c.
    pub clock_mode: u8,
    pub base_posture: i32,
    pub pickup: i32,
    pub carry: CarrySelector,
    pub mode: i32,
    pub rate: f32,
    pub target_rate: f32,
}
/// Values captured before carry/base services. Movement consumes these even if
/// those services change the live person fields later in the same update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub root_motion: u8,
    pub movement_input: i16,
    pub walk_style: i16,
}
/// Re-read for each service continuation; source flags are not cached by begin.
#[derive(Debug, Clone, Copy)]
pub struct Source {
    pub posture: i16,
    pub species: i16,
    pub definition_flags: u8,
    pub walk_style: i16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Continuation {
    Tail,
    Movement { now: i32, step: u32, entry: Entry },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    CarriedSlot,
    Animation { index: u32, slot: Slot },
    OutsideLot,
}
#[derive(Debug, Clone)]
pub enum Response {
    CarriedSlot(bool),
    /// None is an actual missing resource. Loading/decode errors must suspend
    /// or fail before responding, rather than being converted to None.
    Animation {
        name: Option<String>,
        ready: bool,
    },
    OutsideLot(bool),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Carry,
    CarryChoice,
    CarryClear,
    CarryLookup,
    CarryFinish,
    Base,
    BaseLookup,
    Rate,
    Bounds,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Pass {
    tick: i32,
    now: i32,
    entry: Entry,
    stage: Stage,
    carry_selection: i32,
    base_index: u32,
    pending: Option<Request>,
    continuation: Option<Continuation>,
}
/// Suspended prefix control flow without a second copy of the live controls.
/// Household owners retain State separately, so callbacks always see the same
/// clock/mode/carry words that the next continuation consumes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint(Pass);
impl Checkpoint {
    pub fn pending_request(&self) -> Option<&Request> {
        self.0.pending.as_ref()
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prefix {
    pub state: State,
    pass: Option<Pass>,
}
impl Prefix {
    pub fn resume(state: State, checkpoint: Checkpoint) -> Self {
        Self {
            state,
            pass: Some(checkpoint.0),
        }
    }
    pub fn suspend(self) -> Result<(State, Checkpoint), String> {
        Ok((
            self.state,
            Checkpoint(self.pass.ok_or("native ordinary prefix absent")?),
        ))
    }
    pub fn new(state: State) -> Self {
        Self { state, pass: None }
    }
    pub fn begin(
        &mut self,
        character: &mut Character,
        tick: i32,
        entry: Entry,
    ) -> Result<(), String> {
        if self.pass.is_some() || character.busy() {
            return Err("native ordinary update already pending".into());
        }
        let now = clock(tick, self.state.clock_mode);
        character.clock = now;
        character.skin.animator.discard_exhausted_blend()?;
        self.pass = Some(Pass {
            tick,
            now,
            entry,
            stage: Stage::Carry,
            carry_selection: -1,
            base_index: 0,
            pending: None,
            continuation: None,
        });
        Ok(())
    }
    pub fn next(
        &mut self,
        character: &mut Character,
        source: Source,
        settings: Settings,
        rate: RateSettings,
    ) -> Result<Option<Request>, String> {
        let pass = self.pass.as_mut().ok_or("native ordinary prefix absent")?;
        loop {
            if let Some(request) = &pass.pending {
                return Ok(Some(request.clone()));
            }
            let request = match pass.stage {
                Stage::Carry => {
                    if self.state.carry.mode == 6 {
                        pass.stage = Stage::Base;
                        continue;
                    }
                    if self.state.pickup != 0 {
                        pass.stage = Stage::CarryChoice;
                        continue;
                    }
                    Some(Request::CarriedSlot)
                }
                Stage::CarryChoice => {
                    pass.stage = if pass.carry_selection != self.state.carry.selection {
                        Stage::CarryClear
                    } else {
                        Stage::Base
                    };
                    continue;
                }
                Stage::CarryClear => {
                    character.skin.animator.clear(
                        Slot::Carry,
                        clock(pass.tick, self.state.clock_mode),
                        settings,
                    )?;
                    pass.stage = if pass.carry_selection == 0 {
                        Stage::CarryLookup
                    } else {
                        Stage::CarryFinish
                    };
                    continue;
                }
                Stage::CarryLookup => Some(Request::Animation {
                    index: 38,
                    slot: Slot::Carry,
                }),
                Stage::CarryFinish => {
                    self.state.carry.selection = pass.carry_selection;
                    pass.stage = Stage::Base;
                    continue;
                }
                Stage::Base => {
                    if let Some(index) = super::posture::prepare(
                        &mut character.skin.animator,
                        &mut self.state,
                        super::posture::Input {
                            posture: source.posture,
                            species: source.species,
                            definition_flags: source.definition_flags,
                            tick: pass.tick,
                        },
                        settings,
                    )? {
                        pass.base_index = index;
                        pass.stage = Stage::BaseLookup;
                    } else {
                        pass.stage = Stage::Rate;
                    }
                    continue;
                }
                Stage::BaseLookup => Some(Request::Animation {
                    index: pass.base_index,
                    slot: Slot::Base,
                }),
                Stage::Rate => {
                    let target = rate.target(source.walk_style, source.species, self.state.mode)?;
                    if !self.state.rate.is_finite() {
                        return Err("invalid native current movement rate".into());
                    }
                    self.state.target_rate = target as f32;
                    let current = f64::from(self.state.rate);
                    let change = f64::from(rate.change);
                    let next = if (current < target - change || target + change < current)
                        && current < target
                    {
                        current + change
                    } else {
                        target
                    };
                    self.state.rate = next as f32;
                    pass.stage = Stage::Bounds;
                    continue;
                }
                Stage::Bounds => Some(Request::OutsideLot),
                Stage::Complete => return Ok(None),
            };
            pass.pending = request.clone();
            return Ok(request);
        }
    }
    pub fn acknowledge(
        &mut self,
        character: &mut Character,
        program: &Program,
        skeleton: &Skeleton,
        settings: Settings,
        response: Response,
    ) -> Result<(), String> {
        let pass = self.pass.as_mut().ok_or("native ordinary prefix absent")?;
        match (
            pass.pending
                .as_ref()
                .ok_or("native ordinary prefix request absent")?,
            response,
        ) {
            (Request::CarriedSlot, Response::CarriedSlot(held)) => {
                self.state.carry.mode = i32::from(held);
                pass.carry_selection = if held { 0 } else { -1 };
                pass.stage = Stage::CarryChoice;
            }
            (Request::Animation { slot, .. }, Response::Animation { name, ready }) => {
                if let Some(name) = name {
                    let asset = program
                        .animations
                        .get(&name)
                        .ok_or_else(|| format!("native selected animation not loaded: {name}"))?;
                    if *slot == Slot::Carry {
                        let started = ready
                            && character.skin.animator.start(
                                *slot,
                                skeleton,
                                Some(&asset.clip),
                                false,
                                clock(pass.tick, self.state.clock_mode),
                                settings,
                            )?;
                        if started {
                            character
                                .skin
                                .animator
                                .set_priority(character.skin.animator.slot(*slot).unwrap(), 10)?;
                        }
                    } else {
                        super::posture::selected(
                            &mut character.skin.animator,
                            &self.state,
                            pass.tick,
                            skeleton,
                            Some(&asset.clip),
                            ready,
                            settings,
                        )?;
                    }
                }
                pass.stage = if *slot == Slot::Carry {
                    Stage::CarryFinish
                } else {
                    Stage::Rate
                };
            }
            (Request::OutsideLot, Response::OutsideLot(outside)) => {
                pass.continuation = Some(if outside {
                    Continuation::Tail
                } else {
                    Continuation::Movement {
                        now: pass.now,
                        step: 42,
                        entry: pass.entry,
                    }
                });
                pass.stage = Stage::Complete;
            }
            _ => return Err("native ordinary prefix response mismatch".into()),
        }
        pass.pending = None;
        Ok(())
    }
    pub fn finish(&mut self) -> Result<Continuation, String> {
        let pass = self.pass.as_ref().ok_or("native ordinary prefix absent")?;
        if pass.stage != Stage::Complete {
            return Err("native ordinary prefix is incomplete".into());
        }
        let result = pass
            .continuation
            .ok_or("native ordinary continuation absent")?;
        self.pass = None;
        Ok(result)
    }
}
