//! Original 0x4d322c display cache controller. The host owns real font/surface
//! operations; this module preserves their order and the cache publication gate.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    /// Bounds of the allocated surface; a failed create can still own a surface.
    pub surface: Option<[i32; 4]>,
    pub rect: [i32; 4],
    pub clock: u32,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Measure { text: Vec<u8> },
    Allocate,
    Release,
    Create { size: [i32; 2] },
    Clock,
    Lock,
    Fill { color: u32, rect: [i32; 4] },
    Unlock,
    FontColor,
    SetFontColor { color: u32 },
    Draw { text: Vec<u8>, x: i32, y: i32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Bounds([i32; 4]),
    Created { bounds: [i32; 4], success: bool },
    Clock(u32),
    Color(u32),
    Bool(bool),
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Measure,
    Surface,
    Allocate,
    Release,
    Create,
    Clock,
    Gate,
    Lock,
    Fill,
    Unlock,
    FontColor,
    SetColor,
    Draw,
    RestoreColor,
    Publish,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    stage: Stage,
    pending: Option<Request>,
    text: Vec<u8>,
    background: u32,
    foreground: u32,
    rect: [i32; 4],
    size: [i32; 2],
    created: bool,
    filled: bool,
    now: u32,
    previous_color: u32,
}

impl Execution {
    pub fn input(&self) -> (&[u8], u32, u32) {
        (&self.text, self.background, self.foreground)
    }
    pub fn new(
        text: Vec<u8>,
        background: u32,
        foreground: u32,
        font_available: bool,
    ) -> Result<Self, String> {
        super::validate_text(&text)?;
        Ok(Self {
            stage: if font_available {
                Stage::Measure
            } else {
                Stage::Complete
            },
            pending: None,
            text,
            background,
            foreground,
            rect: [0; 4],
            size: [0; 2],
            created: false,
            filled: false,
            now: 0,
            previous_color: 0,
        })
    }

    pub fn is_complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }

    pub fn validate(&self, cache: &Option<Cache>) -> Result<(), String> {
        super::validate_text(&self.text)?;
        if !matches!(
            self.stage,
            Stage::Measure | Stage::Surface | Stage::Complete
        ) {
            let entry = cache
                .as_ref()
                .ok_or("notification cache continuation absent")?;
            if self.stage != Stage::Allocate && entry.surface.is_none() {
                return Err("notification surface continuation absent".into());
            }
        }
        let expected = match self.stage {
            Stage::Measure => Some(Request::Measure {
                text: self.text.clone(),
            }),
            Stage::Allocate => Some(Request::Allocate),
            Stage::Release => Some(Request::Release),
            Stage::Create => Some(Request::Create { size: self.size }),
            Stage::Clock => Some(Request::Clock),
            Stage::Lock => Some(Request::Lock),
            Stage::Fill => Some(Request::Fill {
                color: self.background,
                rect: self.rect,
            }),
            Stage::Unlock => Some(Request::Unlock),
            Stage::FontColor => Some(Request::FontColor),
            Stage::SetColor => Some(Request::SetFontColor {
                color: self.foreground,
            }),
            Stage::Draw => Some(Request::Draw {
                text: self.text.clone(),
                x: 2,
                y: 2,
            }),
            Stage::RestoreColor => Some(Request::SetFontColor {
                color: self.previous_color,
            }),
            _ => None,
        };
        if self
            .pending
            .as_ref()
            .is_some_and(|p| expected.as_ref() != Some(p))
        {
            return Err("notification display request differs from continuation".into());
        }
        Ok(())
    }

    pub fn next(&mut self, cache: &mut Option<Cache>) -> Result<Option<Request>, String> {
        self.validate(cache)?;
        if let Some(request) = &self.pending {
            return Ok(Some(request.clone()));
        }
        loop {
            let request = match self.stage {
                Stage::Measure => Request::Measure {
                    text: self.text.clone(),
                },
                Stage::Surface => {
                    let entry = cache.get_or_insert_with(Cache::default);
                    self.size = dimensions(self.rect);
                    if let Some(bounds) = entry.surface {
                        let previous = dimensions(bounds);
                        let mut grown = bounds;
                        if previous[0] < self.size[0] {
                            grown[2] = bounds[0].wrapping_add(self.size[0]);
                        }
                        if previous[1] < self.size[1] {
                            grown[3] = bounds[1].wrapping_add(self.size[1]);
                        }
                        self.size = dimensions(grown);
                        if grown == bounds {
                            self.created = true;
                            self.stage = Stage::Clock;
                        } else {
                            self.stage = Stage::Release;
                        }
                    } else {
                        self.stage = Stage::Allocate;
                    }
                    continue;
                }
                Stage::Allocate => Request::Allocate,
                Stage::Release => Request::Release,
                Stage::Create => Request::Create { size: self.size },
                Stage::Clock => Request::Clock,
                Stage::Gate => {
                    let entry = cache.as_mut().expect("notification cache established");
                    if self.created && (entry.clock.wrapping_add(100) < self.now || !entry.active) {
                        // Native publishes the rectangle before attempting the
                        // lock; failures leave this change observable.
                        entry.rect = self.rect;
                        self.stage = Stage::Lock;
                    } else {
                        self.stage = Stage::Complete;
                    }
                    continue;
                }
                Stage::Lock => Request::Lock,
                Stage::Fill => Request::Fill {
                    color: self.background,
                    rect: self.rect,
                },
                Stage::Unlock => Request::Unlock,
                Stage::FontColor => Request::FontColor,
                Stage::SetColor => Request::SetFontColor {
                    color: self.foreground,
                },
                Stage::Draw => Request::Draw {
                    text: self.text.clone(),
                    x: 2,
                    y: 2,
                },
                Stage::RestoreColor => Request::SetFontColor {
                    color: self.previous_color,
                },
                Stage::Publish => {
                    let entry = cache.as_mut().expect("notification cache established");
                    entry.clock = self.now;
                    entry.active = true;
                    self.stage = Stage::Complete;
                    continue;
                }
                Stage::Complete => return Ok(None),
            };
            self.pending = Some(request.clone());
            return Ok(Some(request));
        }
    }

    /// A response commits only the completed service. Invalid replies leave the
    /// continuation and cache untouched. Clock is sampled after surface work.
    pub fn acknowledge(
        &mut self,
        cache: &mut Option<Cache>,
        response: Response,
    ) -> Result<(), String> {
        self.validate(cache)?;
        if self.pending.is_none() {
            return Err("no pending notification display service".into());
        }
        match (self.stage, response) {
            (Stage::Measure, Response::Bounds(mut bounds)) => {
                bounds[2] = bounds[2].wrapping_add(4);
                bounds[3] = bounds[3].wrapping_add(4);
                self.rect = bounds;
                self.stage = Stage::Surface;
            }
            (Stage::Allocate, Response::Bounds(bounds)) => {
                cache.as_mut().ok_or("notification cache absent")?.surface = Some(bounds);
                self.stage = Stage::Create;
            }
            (Stage::Release, Response::Done) => self.stage = Stage::Create,
            (Stage::Create, Response::Created { bounds, success }) => {
                cache.as_mut().ok_or("notification cache absent")?.surface = Some(bounds);
                self.created = success;
                self.stage = Stage::Clock;
            }
            (Stage::Clock, Response::Clock(now)) => {
                self.now = now;
                self.stage = Stage::Gate;
            }
            (Stage::Lock, Response::Bool(locked)) => {
                self.stage = if locked { Stage::Fill } else { Stage::Complete };
            }
            (Stage::Fill, Response::Bool(filled)) => {
                self.filled = filled;
                self.stage = Stage::Unlock;
            }
            (Stage::Unlock, Response::Done) => {
                self.stage = if self.filled {
                    Stage::FontColor
                } else {
                    Stage::Complete
                };
            }
            (Stage::FontColor, Response::Color(color)) => {
                self.previous_color = color;
                self.stage = Stage::SetColor;
            }
            (Stage::SetColor, Response::Done) => self.stage = Stage::Draw,
            (Stage::Draw, Response::Done) => self.stage = Stage::RestoreColor,
            (Stage::RestoreColor, Response::Done) => self.stage = Stage::Publish,
            _ => return Err("incorrect notification display response".into()),
        }
        self.pending = None;
        Ok(())
    }
}

fn dimensions(rect: [i32; 4]) -> [i32; 2] {
    [rect[2].wrapping_sub(rect[0]), rect[3].wrapping_sub(rect[1])]
}

/// Original render branch at 0x437e9c, after selecting a person and cache entry.
pub fn within_display_lifetime(cached: u32, now: u32) -> bool {
    now.wrapping_sub(cached) < 100
}
