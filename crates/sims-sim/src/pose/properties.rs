//! Native 5912fd property callback, with explicit appearance/sound/world-query
//! boundaries. Properties are already in the native bag's insertion order.
use super::{
    animator::{Animator, Event},
    layer::atoi,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod surface;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// A named additional appearance binding, not the person's outfit number.
    Dress {
        name: String,
        binding: String,
    },
    Undress {
        name: String,
    },
    TraceSound {
        text: String,
    },
    Sound {
        name: String,
        object: i16,
    },
    /// Original 58900f/5890a8 surface query, with wrapping abs(property value).
    FootstepSurface {
        index: i32,
    },
}

/// Values supplied by the actual animator/world host, re-read for each request.
/// `sound_visible` is 432595's result; this does not replace that native query.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Context {
    pub actor: i16,
    pub sound_visible: bool,
    pub selected: bool,
    pub debug_sound: bool,
    pub position_x: f32,
    /// Native animator +0x1b0, not assumed equal to person outfit field 8.
    pub appearance_mode: i32,
    pub snow: bool,
    pub lot_kind: i16,
}

/// 58900f's position gate and Point conversion. Negative visual coordinates
/// return surface -1 without consulting any tile; otherwise 5890a8 receives
/// rounded coordinates shifted and narrowed to signed bytes, plus floor byte.
/// The material/object query itself remains an explicit host service.
pub fn footstep_tile(position: [f32; 2], level: i32) -> Result<Option<[i8; 3]>, String> {
    if position.iter().any(|v| !v.is_finite()) {
        return Err("nonfinite native footstep position".into());
    }
    if position.iter().any(|&v| v < 0.) {
        return Ok(None);
    }
    let mut tile = [0, 0, level as i8];
    for (i, value) in position.into_iter().enumerate() {
        let rounded = (f64::from(value) + 0.5).trunc();
        if rounded > i32::MAX as f64 {
            return Err("native footstep position exceeds integer conversion".into());
        }
        tile[i] = ((rounded as i32) >> 4) as i8;
    }
    Ok(Some(tile))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum After {
    Advance,
    Sound,
    Surface,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Handler {
    event: Event,
    index: usize,
    pending: Option<Request>,
    after: After,
    sound_after_trace: bool,
    surface: Option<i32>,
}
impl Handler {
    pub fn new(event: Event) -> Self {
        Self {
            event,
            index: 0,
            pending: None,
            after: After::Advance,
            sound_after_trace: false,
            surface: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.index == self.event.dispatch.entry.properties.len() && self.pending.is_none()
    }
    fn request(&mut self, request: Request, after: After) -> Option<Request> {
        self.pending = Some(request.clone());
        self.after = after;
        Some(request)
    }
    /// Native direct writes target the person's actual 256-word storage. Host
    /// services remain pending until acknowledged; retry cannot skip an effect.
    pub fn next(
        &mut self,
        animator: &mut Animator,
        person: &mut BTreeMap<i16, i16>,
        context: Context,
    ) -> Result<Option<Request>, String> {
        if let Some(pending) = &self.pending {
            return Ok(Some(pending.clone()));
        }
        while let Some((key, value)) = self
            .event
            .dispatch
            .entry
            .properties
            .get(self.index)
            .cloned()
        {
            let number = atoi(&value);
            match key.as_str() {
                "xevt" => animator.enqueue_numbered_event(number)?,
                "interruptible" | "interruptable" => {
                    animator.set_interruptible(self.event.layer, number)?
                }
                "anchor" => {
                    animator.set_anchor(self.event.layer, self.event.dispatch.bone, number != 0)?
                }
                "lefthand" => {
                    person.insert(22, number as i16);
                }
                "righthand" => {
                    person.insert(23, number as i16);
                }
                "censor" => {
                    person.insert(30, number as i16);
                }
                "discontinuity" => animator.discontinuity()?,
                "dress" => {
                    return Ok(self.request(
                        Request::Dress {
                            name: value,
                            binding: String::new(),
                        },
                        After::Advance,
                    ));
                }
                "undress" => {
                    return Ok(self.request(Request::Undress { name: value }, After::Advance));
                }
                "sound" => {
                    if context.debug_sound && !self.sound_after_trace {
                        return Ok(self.request(
                            Request::TraceSound {
                                text: format!("A: {value}"),
                            },
                            After::Sound,
                        ));
                    }
                    let target = if context.sound_visible {
                        Some(context.actor)
                    } else if *person
                        .get(&65)
                        .ok_or("sound requires native person field 65")?
                        > 1
                    {
                        Some(-4)
                    } else {
                        None
                    };
                    if let Some(object) = target {
                        return Ok(self.request(
                            Request::Sound {
                                name: value,
                                object,
                            },
                            After::Advance,
                        ));
                    }
                }
                "selectedsound" | "deselectedsound" => {
                    if context.selected == (key == "selectedsound") {
                        return Ok(self.request(
                            Request::Sound {
                                name: value,
                                object: context.actor,
                            },
                            After::Advance,
                        ));
                    }
                }
                "footstep" => {
                    if !context.position_x.is_finite() {
                        return Err("invalid native footstep position".into());
                    }
                    if (f64::from(context.position_x) + 999.).abs() > f64::from(0.000001_f32) {
                        let Some(surface) = self.surface else {
                            return Ok(self.request(
                                Request::FootstepSurface {
                                    index: number.wrapping_abs(),
                                },
                                After::Surface,
                            ));
                        };
                        if surface != -1 {
                            let flag = |field, mask| -> Result<bool, String> {
                                Ok(person.get(&field).ok_or_else(|| {
                                    format!("footstep requires native person field {field}")
                                })? & mask
                                    != 0)
                            };
                            let speed = animator
                                .layer(self.event.layer)
                                .ok_or("native footstep layer absent")?
                                .clock
                                .speed;
                            let direction = if speed < 0. {
                                matches!(number, -1 | -2)
                            } else {
                                matches!(number, 1 | 2)
                            };
                            if !flag(84, 1)? && !flag(68, 1)? && !flag(74, 8)? && direction {
                                let name = footstep_name(surface, context);
                                return Ok(self.request(
                                    Request::Sound {
                                        name,
                                        object: context.actor,
                                    },
                                    After::Advance,
                                ));
                            }
                        }
                    }
                }
                _ => (), // Original strcmp dispatch ignores unknown names.
            }
            self.advance();
        }
        Ok(None)
    }
    fn advance(&mut self) {
        self.index += 1;
        self.sound_after_trace = false;
        self.surface = None;
    }
    /// Only a surface request consumes a numeric result. Other requests must
    /// already have completed their host effect before acknowledgment.
    pub fn acknowledge(&mut self, surface: Option<i32>) -> Result<(), String> {
        if self.pending.is_none() {
            return Err("no pending native property service".into());
        }
        match self.after {
            After::Surface => {
                self.surface = Some(surface.ok_or("native footstep surface result absent")?)
            }
            After::Advance | After::Sound if surface.is_some() => {
                return Err("unexpected native property service result".into());
            }
            After::Advance => self.advance(),
            After::Sound => self.sound_after_trace = true,
        }
        self.pending = None;
        Ok(())
    }
}
fn footstep_name(surface: i32, context: Context) -> String {
    let no_shoe = matches!(context.appearance_mode, 1 | 2 | 5 | 14);
    let base = match surface {
        1 | 2 => "hard",
        3 | 5 => "medium",
        4 => "soft",
        6 => return "footstep_plant".into(),
        7 => return "footstep_trash".into(),
        8 => return "footstep_ash".into(),
        9 => return "footstep_roach".into(),
        10 => return "footstep_puddle".into(),
        11 => return "footstep_pool_swim_stroke".into(),
        _ if context.snow => return "footstep_snow".into(),
        _ if context.lot_kind == 99 => "soft",
        _ => "terrain",
    };
    format!("footstep_{base}{}", if no_shoe { "_noshoe" } else { "" })
}
