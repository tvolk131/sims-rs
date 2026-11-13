//! Defined scalar state after 58464f/587d2c, before model/rig initialization.
//! Construction is not activation: layer/skin generations, appearance binding,
//! reset callbacks and movement measurements belong to subsequent services.
use super::{movement, ordinary, transform::Placement};
use serde::{Deserialize, Serialize};

/// Some native words are deliberately left untouched by the constructors.
/// Keep them absent until a subsequent service writes them; allocation bytes
/// must never become a deterministic-looking Rust default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub clock: i32,
    pub clock_mode: u8,
    pub selection: i32,
    pub placement: Placement,
    pub movement: movement::State,
    pub mode: i32,
    pub previous_mode: i32,
    pub base_posture: i32,
    pub pickup: i32,
    pub carry_mode: i32,
    pub carry_selection: Option<i32>,
    pub rate: f32,
    pub target_rate: f32,
    pub head_clock: u32,
    pub head_angles: [f32; 3],
    pub head_target_point: Option<[f32; 3]>,
    pub phase_window: [f32; 2],
    pub stop_distance: [f32; 2],
    /// Native +1f8..+20c; actual values are measured when rig resources bind.
    pub clip_distances: [f32; 6],
}
impl State {
    pub fn new(lot_tick: i32) -> Self {
        Self {
            clock: ordinary::clock(lot_tick, 0),
            clock_mode: 0,
            selection: -1,
            placement: Placement {
                base: [-999.; 3],
                offset: [0.; 3],
                previous: [0.; 3],
                dirty: true,
            },
            movement: movement::State {
                status: 0,
                flags: [0; 2],
                start: [0.; 3],
                end: [0.; 3],
                turn_delta: 0.,
                nominal_turn: 0.,
                reason: 0,
                apply_rate: 0,
            },
            mode: 3,
            previous_mode: 3,
            base_posture: -1,
            pickup: 0,
            carry_mode: 0,
            carry_selection: None,
            rate: 1.,
            target_rate: 1.,
            head_clock: 0,
            head_angles: [0.; 3],
            head_target_point: None,
            phase_window: [0.; 2],
            stop_distance: [0.; 2],
            clip_distances: [0.; 6],
        }
    }
    /// Refuse consumption until the reset/selection service initializes +1ac.
    pub fn prefix(&self) -> Result<ordinary::State, String> {
        Ok(ordinary::State {
            clock_mode: self.clock_mode,
            base_posture: self.base_posture,
            pickup: self.pickup,
            carry: sims_vm::animation::CarrySelector {
                mode: self.carry_mode,
                selection: self
                    .carry_selection
                    .ok_or("native carry selection has not been initialized")?,
            },
            mode: self.mode,
            rate: self.rate,
            target_rate: self.target_rate,
        })
    }
    pub fn head(&self) -> Result<crate::head_look::State, String> {
        let state = crate::head_look::State {
            last_clock: self.head_clock,
            basis_heading: self.head_angles[0],
            target_heading: self.head_angles[1],
            turn: self.head_angles[2],
            target_point: self.head_target_point,
        };
        state.validate()?;
        Ok(state)
    }
}
