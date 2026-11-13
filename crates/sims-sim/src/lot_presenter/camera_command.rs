//! Original4ced2e/4ced60/4cee08 UI camera setters and4ca5af depth table.
use super::{motion::wide::Wide, open};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ui {
    /// Interface+0d, distinct from world/view draw-state flags.
    pub flags: u8,
    /// Interface+34. Written before the synchronous camera call.
    pub zoom: u32,
    #[serde(default, skip_serializing_if = "zero")]
    pub rotation: u32,
    #[serde(default, skip_serializing_if = "zero")]
    pub floor: i32,
}
fn zero<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Camera(open::Request),
    /// Original57a9d4 on the application's audio owner.
    RefreshAudio,
    /// CPState+4c's selected tool virtual+3c (4cee3d), after the setter.
    /// Historical serialized name; not a panel publication callback.
    RefreshControls,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Tail {
    #[default]
    Audio,
    Rotation,
    Controls,
    ///4cedc0/4cede1 wrap4ced2e and then call the selected tool.
    ZoomStep,
    Direct,
}
fn audio(tail: &Tail) -> bool {
    *tail == Tail::Audio
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Zoom {
    request: Option<Request>,
    // Omitted in historical zoom-only saves.
    #[serde(default, skip_serializing_if = "audio")]
    tail: Tail,
}
impl Zoom {
    pub fn validate(&self) -> Result<(), String> {
        let valid = matches!(
            (&self.request, self.tail),
            (None, _)
                | (
                    Some(Request::Camera(open::Request::Zoom { .. }) | Request::RefreshAudio),
                    Tail::Audio | Tail::ZoomStep
                )
                | (
                    Some(Request::Camera(open::Request::Rotate { .. })),
                    Tail::Rotation
                )
                | (
                    Some(Request::Camera(
                        open::Request::Scroll { .. }
                            | open::Request::CenterPerson { .. }
                            | open::Request::MotionFrame
                            | open::Request::Zoom { .. }
                            | open::Request::Rotate { .. }
                            | open::Request::Floor { .. }
                            | open::Request::WallsDown(_)
                            | open::Request::Cutaway(_)
                            | open::Request::Roof { .. }
                    )),
                    Tail::Direct
                )
                | (
                    Some(Request::Camera(open::Request::Floor { .. })),
                    Tail::Controls
                )
                | (
                    Some(Request::RefreshControls),
                    Tail::Rotation | Tail::Controls | Tail::ZoomStep
                )
        );
        if valid {
            Ok(())
        } else {
            Err("camera UI setter lost its completion order".into())
        }
    }
    pub fn begin(ui: &mut Ui, zoom: u32, deferred: u32) -> Self {
        ui.flags |= 0x10;
        ui.zoom = zoom;
        Self {
            tail: Tail::Audio,
            request: (deferred == 0).then_some(Request::Camera(open::Request::Zoom {
                zoom,
                force: 0,
                refresh_ui: 1,
            })),
        }
    }
    ///4cedc0/4cede1 and4ce9f8: clamped button presses return without even
    ///calling4ced2e. An accepted deferred step still calls the selected tool.
    pub fn step(ui: &mut Ui, inward: bool, house: i16, deferred: u32) -> Self {
        let allowed = if inward {
            (ui.zoom as i32) < 3
        } else {
            (ui.zoom as i32) > if house == 99 { 2 } else { 1 }
        };
        if !allowed {
            return Self {
                request: None,
                tail: Tail::ZoomStep,
            };
        }
        let zoom = if inward {
            ui.zoom.wrapping_add(1)
        } else {
            ui.zoom.wrapping_sub(1)
        };
        let mut call = Self::begin(ui, zoom, deferred);
        call.tail = Tail::ZoomStep;
        if deferred != 0 {
            call.request = Some(Request::RefreshControls);
        }
        call
    }
    /// 4ced60: rotation property first, camera call, then dirty bit and controls.
    pub fn rotation(ui: &mut Ui, rotation: u32, deferred: u32) -> Self {
        ui.rotation = rotation;
        let request = if deferred == 0 {
            Request::Camera(open::Request::Rotate {
                rotation,
                force: 0,
                refresh_ui: 1,
            })
        } else {
            ui.flags |= 8;
            Request::RefreshControls
        };
        Self {
            request: Some(request),
            tail: Tail::Rotation,
        }
    }
    /// 4cee08 normalizes only zero, then writes the flag/property before calling.
    pub fn floor(ui: &mut Ui, level: i32, deferred: u32) -> Self {
        ui.flags |= 1;
        ui.floor = if level == 0 { 1 } else { level };
        Self {
            request: Some(if deferred == 0 {
                Request::Camera(open::Request::Floor {
                    level: ui.floor,
                    force: 0,
                })
            } else {
                Request::RefreshControls
            }),
            tail: Tail::Controls,
        }
    }
    /// Direct native4a81cc; no UI property writes or setter completion tail.
    pub fn scroll(position: [i32; 2]) -> Self {
        Self {
            request: Some(Request::Camera(open::Request::Scroll {
                position,
                immediate: 1,
            })),
            tail: Tail::Direct,
        }
    }
    pub fn center_person(point: [i32; 2], level: i32) -> Self {
        Self {
            request: Some(Request::Camera(open::Request::CenterPerson {
                point,
                level,
            })),
            tail: Tail::Direct,
        }
    }
    pub fn motion_frame() -> Self {
        Self {
            request: Some(Request::Camera(open::Request::MotionFrame)),
            tail: Tail::Direct,
        }
    }
    ///4ce44f calls the camera directly, without UI setter writes or tails.
    pub fn publication(request: open::Request) -> Result<Self, String> {
        if !matches!(
            request,
            open::Request::Zoom {
                force: 0,
                refresh_ui: 1,
                ..
            } | open::Request::Rotate {
                force: 0,
                refresh_ui: 1,
                ..
            } | open::Request::Floor { force: 0, .. }
                | open::Request::WallsDown(_)
                | open::Request::Cutaway(_)
                | open::Request::Roof { force: 0, .. }
        ) {
            return Err("unrelated publication camera call".into());
        }
        Ok(Self {
            request: Some(Request::Camera(request)),
            tail: Tail::Direct,
        })
    }
    pub fn request(&self) -> Option<&Request> {
        self.request.as_ref()
    }
    pub fn complete(&self) -> bool {
        self.request.is_none()
    }
    pub fn direct(&self) -> bool {
        self.tail == Tail::Direct
    }
    pub fn acknowledge(&mut self, request: &Request) -> Result<(), String> {
        self.reply(request, None)
    }
    pub fn acknowledge_with_ui(&mut self, request: &Request, ui: &mut Ui) -> Result<(), String> {
        self.reply(request, Some(ui))
    }
    fn reply(&mut self, request: &Request, ui: Option<&mut Ui>) -> Result<(), String> {
        self.validate()?;
        if self.request.as_ref() != Some(request) {
            return Err("camera UI setter acknowledgement mismatch".into());
        }
        self.request = match request {
            Request::Camera(_) if self.tail == Tail::Direct => None,
            Request::Camera(_) => Some(match self.tail {
                Tail::Audio | Tail::ZoomStep => Request::RefreshAudio,
                Tail::Rotation => {
                    ui.ok_or("rotation setter needs its retained UI")?.flags |= 8;
                    Request::RefreshControls
                }
                Tail::Controls => Request::RefreshControls,
                Tail::Direct => unreachable!(),
            }),
            Request::RefreshAudio if self.tail == Tail::ZoomStep => Some(Request::RefreshControls),
            Request::RefreshAudio | Request::RefreshControls => None,
        };
        Ok(())
    }
}

/// Native255 words; index255 is transparent and is not written by4ca5af.
/// x87 intermediates retain64 significand bits and the two actual float spills.
/// Masked invalid __ftol conversions store INT64_MIN, with a zero low word.
pub fn depth_table(scale_bits: u32) -> [u16; 255] {
    fn f(n: f32) -> Wide {
        Wide::from_f32(n.to_bits()).unwrap()
    }
    let Some(reciprocal) =
        Wide::from_f32(scale_bits).and_then(|scale| Wide::integer(1).divided(scale))
    else {
        return [0; 255];
    };
    let Some(a) = Wide::from_f32(reciprocal.times_unsigned(12).f32_bits()) else {
        return [0; 255];
    };
    let d = f(3.)
        .times(f(1.4142135381698608))
        .times(reciprocal)
        .times(f(1.7320507764816284))
        .add(a)
        .times(f(16383.75));
    let Some(step) = Wide::from_f32(d.times(f(0.007874015718698502)).f32_bits()) else {
        return [0; 255];
    };
    let base = a.times(f(-16383.75)).add(d.negated());
    std::array::from_fn(|i| {
        step.times_unsigned(i as u32)
            .add(base)
            .add(Wide::half())
            .ftol() as u16
    })
}
