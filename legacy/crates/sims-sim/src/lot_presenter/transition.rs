//! Original4a7b92 zoom and4a7dfe rotation ownership. Callees that own graphics,
//! scrolling, world rotation and UI remain explicit acknowledged services.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub origin: [i32; 2],
    pub pending: [i32; 2],
    pub zoom: u8,
    /// View+58,+5c,+60,+64.
    pub widths: [i32; 4],
    pub level: i32,
    pub world_rotation: u32,
    pub display_rotation: u32,
    pub world_size: i32,
    pub world_bounds: [i32; 4],
    pub camera_mode: u8,
    pub moving: u8,
    pub skip_edges: u8,
    pub cutaway: u8,
    pub include_visible: u8,
    pub graphics_enabled: bool,
    pub ui_present: bool,
    /// Raw finite float bits indexed by zoom. The zero slot is unused here.
    pub depth_scales: [u32; 4],
}
impl State {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=3).contains(&self.zoom) || !(0..=64).contains(&self.world_size) {
            return Err("native camera transition fields unavailable".into());
        }
        Ok(())
    }
    fn take_pending(&mut self) -> [i32; 2] {
        let p = std::array::from_fn(|n| self.origin[n].wrapping_add(self.pending[n]));
        self.pending = [0, 0];
        p
    }
    fn checked_center(&self, point: [i32; 2]) -> [i32; 2] {
        //433067 physical XY -> tile column,row;434cb7 signed interior check.
        let [x, y] = [point[1] >> 4, point[0] >> 4].map(|v| i32::from(v as i8));
        let b = self.world_bounds;
        if x >= b[0].wrapping_add(1)
            && x <= b[1].wrapping_sub(1)
            && y >= b[2].wrapping_add(1)
            && y <= b[3].wrapping_sub(1)
        {
            point
        } else {
            let q = (self.world_size / 4) as i8;
            let x = q.wrapping_add(b[0] as i8);
            let y = q.wrapping_add(b[2] as i8);
            //434c98 returns the center of the fallback tile, axes reversed.
            [(i32::from(y) << 4) | 8, (i32::from(x) << 4) | 8]
        }
    }
}
/// 47bc1f called with pointers in physical Y,X order and a1024-point extent.
fn rotate(point: [i32; 2], rotation: u32) -> [i32; 2] {
    let [x, y] = point;
    match rotation {
        1 => [y, 1023i32.wrapping_sub(x)],
        2 => [1023i32.wrapping_sub(x), 1023i32.wrapping_sub(y)],
        3 => [1023i32.wrapping_sub(y), x],
        _ => point,
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    MotionCoordinate {
        axis: u8,
    },
    CancelMotion,
    Scroll {
        position: [i32; 2],
        immediate: u8,
    },
    ResolveCenter,
    SynchronizeRenderer,
    InitializeLookup {
        zoom: u8,
    },
    Center {
        point: [i32; 2],
        level: i32,
        style: u8,
        immediate: u8,
    },
    RotateWorld {
        rotation: u32,
        force: u8,
    },
    TerrainExtrema,
    RefreshDrawables,
    EdgeSurfaces,
    ResolveUi,
    ResetUi {
        token: u32,
    },
    InvalidateView,
    Compose {
        include_visible: u8,
    },
    DepthTable {
        scale_bits: u32,
    },
    Broadcast {
        event: u16,
        value: u32,
    },
    RefreshRooms {
        level: i32,
    },
    /// 43dfc4 releases definition graphics caches. Historical serialized name;
    /// this does not call CPState's4ce41b property/panel publication.
    RefreshUi,
    ResolveSurface,
    FlushSurface {
        token: u32,
    },
    FinishRotation,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    Coordinate(i32),
    Point([i32; 2]),
    Token(Option<u32>),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Operation {
    Zoom(u8),
    Rotation(u32),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    MotionX,
    MotionY(i32),
    Cancel([i32; 2]),
    PreScroll([i32; 2]),
    CenterPoint,
    ZoomSync,
    ZoomLookup(u8),
    WorldRotation,
    Terrain,
    Center(i32),
    PostScroll([i32; 2]),
    ZoomEdges,
    Drawables,
    Ui,
    UiReset(u32),
    Invalidate,
    ZoomCompose(u8),
    Depth(u32),
    ZoomBroadcast(u32),
    RotationBroadcast,
    RotationSync,
    RotationEdges,
    RotationLookup(u8),
    Rooms(i32),
    RotationCompose(u8),
    UiRefresh,
    Surface,
    Flush(u32),
    Finish,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    operation: Operation,
    force: u8,
    refresh_ui: u8,
    point: [i32; 2],
    old_display_rotation: u32,
    stage: Stage,
}
impl Cursor {
    fn begin(s: &State, operation: Operation, force: u8, refresh_ui: u8) -> Result<Self, String> {
        s.validate()?;
        let equal = match operation {
            Operation::Zoom(z) => s.zoom == z,
            Operation::Rotation(r) => s.world_rotation == r,
        };
        let stage = if equal && force == 0 {
            Stage::Done
        } else if s.camera_mode == 0 {
            Stage::CenterPoint
        } else if s.moving != 0 {
            Stage::MotionX
        } else {
            Stage::Cancel(std::array::from_fn(|n| {
                s.origin[n].wrapping_add(s.pending[n])
            }))
        };
        Ok(Self {
            operation,
            force,
            refresh_ui,
            point: [0, 0],
            old_display_rotation: 0,
            stage,
        })
    }
    pub fn zoom(s: &State, zoom: u8, force: u8, refresh_ui: u8) -> Result<Self, String> {
        if !(1..=3).contains(&zoom) {
            return Err("native camera transition zoom unavailable".into());
        }
        Self::begin(s, Operation::Zoom(zoom), force, refresh_ui)
    }
    pub fn rotation(s: &State, rotation: u32, force: u8, refresh_ui: u8) -> Result<Self, String> {
        Self::begin(s, Operation::Rotation(rotation), force, refresh_ui)
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn request(&self) -> Option<Request> {
        Some(match self.stage {
            Stage::MotionX => Request::MotionCoordinate { axis: 0 },
            Stage::MotionY(_) => Request::MotionCoordinate { axis: 1 },
            Stage::Cancel(_) => Request::CancelMotion,
            Stage::PreScroll(position) | Stage::PostScroll(position) => Request::Scroll {
                position,
                immediate: 1,
            },
            Stage::CenterPoint => Request::ResolveCenter,
            Stage::ZoomSync | Stage::RotationSync => Request::SynchronizeRenderer,
            Stage::ZoomLookup(zoom) | Stage::RotationLookup(zoom) => {
                Request::InitializeLookup { zoom }
            }
            Stage::WorldRotation => {
                let Operation::Rotation(rotation) = self.operation else {
                    return None;
                };
                Request::RotateWorld { rotation, force: 0 }
            }
            Stage::Terrain => Request::TerrainExtrema,
            Stage::Center(level) => Request::Center {
                point: self.point,
                level,
                style: 0,
                immediate: 1,
            },
            Stage::ZoomEdges | Stage::RotationEdges => Request::EdgeSurfaces,
            Stage::Drawables => Request::RefreshDrawables,
            Stage::Ui => Request::ResolveUi,
            Stage::UiReset(token) => Request::ResetUi { token },
            Stage::Invalidate => Request::InvalidateView,
            Stage::ZoomCompose(include_visible) | Stage::RotationCompose(include_visible) => {
                Request::Compose { include_visible }
            }
            Stage::Depth(scale_bits) => Request::DepthTable { scale_bits },
            Stage::ZoomBroadcast(value) => Request::Broadcast { event: 249, value },
            Stage::RotationBroadcast => Request::Broadcast {
                event: 132,
                value: 0,
            },
            Stage::Rooms(level) => Request::RefreshRooms { level },
            Stage::UiRefresh => Request::RefreshUi,
            Stage::Surface => Request::ResolveSurface,
            Stage::Flush(token) => Request::FlushSurface { token },
            Stage::Finish => Request::FinishRotation,
            Stage::Done => return None,
        })
    }
    pub fn validate(&self, s: &State) -> Result<(), String> {
        s.validate()?;
        let zoom_stage = matches!(
            self.stage,
            Stage::ZoomSync
                | Stage::ZoomLookup(_)
                | Stage::ZoomEdges
                | Stage::ZoomCompose(_)
                | Stage::Depth(_)
                | Stage::ZoomBroadcast(_)
        );
        let rotation_stage = matches!(
            self.stage,
            Stage::WorldRotation
                | Stage::Terrain
                | Stage::Drawables
                | Stage::RotationBroadcast
                | Stage::RotationSync
                | Stage::RotationEdges
                | Stage::RotationLookup(_)
                | Stage::Rooms(_)
                | Stage::RotationCompose(_)
                | Stage::Finish
        );
        if zoom_stage && !matches!(self.operation, Operation::Zoom(_))
            || rotation_stage && !matches!(self.operation, Operation::Rotation(_))
        {
            return Err("invalid native camera transition continuation".into());
        }
        Ok(())
    }
    fn ui(&mut self, s: &State) {
        self.stage = if s.ui_present {
            Stage::Ui
        } else {
            Stage::Invalidate
        };
    }
    fn tail(&mut self) {
        self.stage = if self.refresh_ui != 0 {
            Stage::UiRefresh
        } else {
            Stage::Surface
        };
    }
    fn depth(&mut self, s: &State) {
        self.stage = Stage::Depth(s.depth_scales[s.zoom as usize]);
    }
    pub fn acknowledge(
        &mut self,
        s: &mut State,
        tiles: &mut [u16],
        request: &Request,
        reply: Reply,
    ) -> Result<(), String> {
        self.validate(s)?;
        if self.request().as_ref() != Some(request) {
            return Err("native camera transition acknowledgement mismatch".into());
        }
        let valid = match (&self.stage, &reply) {
            (Stage::MotionX | Stage::MotionY(_), Reply::Coordinate(_))
            | (Stage::CenterPoint, Reply::Point(_))
            | (Stage::Ui, Reply::Token(_))
            | (Stage::Surface, Reply::Token(Some(_))) => true,
            (
                Stage::MotionX
                | Stage::MotionY(_)
                | Stage::CenterPoint
                | Stage::Ui
                | Stage::Surface,
                _,
            ) => false,
            (_, Reply::Done) => true,
            _ => false,
        };
        if !valid {
            return Err("native camera transition reply type mismatch".into());
        }
        // Validate the borrowed tile plane before the inline native tail writes.
        let swap = matches!(self.stage, Stage::Flush(_))
            && matches!(self.operation, Operation::Rotation(_))
            && self.force == 0
            && self.old_display_rotation.wrapping_sub(s.display_rotation) & 1 != 0;
        let cells = (s.world_size as usize).pow(2);
        if swap && tiles.len() < cells {
            return Err("native rotation tile plane unavailable".into());
        }
        match self.stage {
            Stage::MotionX => {
                let Reply::Coordinate(x) = reply else {
                    unreachable!()
                };
                self.stage = Stage::MotionY(x);
            }
            Stage::MotionY(x) => {
                let Reply::Coordinate(y) = reply else {
                    unreachable!()
                };
                self.stage = Stage::Cancel([x, y]);
            }
            Stage::Cancel(p) => {
                s.pending = [0, 0];
                self.stage = Stage::PreScroll(p);
            }
            Stage::PreScroll(_) => self.stage = Stage::CenterPoint,
            Stage::CenterPoint => {
                let Reply::Point(p) = reply else {
                    unreachable!()
                };
                self.point = s.checked_center(p);
                match self.operation {
                    Operation::Zoom(z) => {
                        s.zoom = z;
                        s.widths = [16, 8, 8, 4].map(|v| v << z);
                        self.stage = Stage::ZoomSync;
                    }
                    Operation::Rotation(r) => {
                        self.point = rotate(self.point, s.world_rotation.wrapping_neg() & 3);
                        self.old_display_rotation = s.display_rotation;
                        s.display_rotation = r;
                        self.stage = Stage::WorldRotation;
                    }
                }
            }
            Stage::ZoomSync => self.stage = Stage::ZoomLookup(s.zoom),
            Stage::ZoomLookup(_) => self.stage = Stage::Center(s.level),
            Stage::WorldRotation => self.stage = Stage::Terrain,
            Stage::Terrain => {
                self.point = rotate(self.point, s.world_rotation);
                self.stage = Stage::Center(s.level);
            }
            Stage::Center(_) => self.stage = Stage::PostScroll(s.take_pending()),
            Stage::PostScroll(_) => match self.operation {
                Operation::Zoom(_) if s.skip_edges == 0 => self.stage = Stage::ZoomEdges,
                Operation::Zoom(_) => self.ui(s),
                Operation::Rotation(_) => self.stage = Stage::Drawables,
            },
            Stage::ZoomEdges | Stage::Drawables => self.ui(s),
            Stage::Ui => {
                let Reply::Token(token) = reply else {
                    unreachable!()
                };
                self.stage = token.map_or(Stage::Invalidate, Stage::UiReset);
            }
            Stage::UiReset(_) => self.stage = Stage::Invalidate,
            Stage::Invalidate => match self.operation {
                Operation::Zoom(_) if s.cutaway != 0 => {
                    self.stage = Stage::ZoomCompose(s.include_visible)
                }
                Operation::Zoom(_) => self.depth(s),
                Operation::Rotation(_) => self.stage = Stage::RotationBroadcast,
            },
            Stage::ZoomCompose(_) => self.depth(s),
            Stage::Depth(_) => self.stage = Stage::ZoomBroadcast(u32::from(s.zoom)),
            Stage::ZoomBroadcast(_) => self.tail(),
            Stage::RotationBroadcast => self.stage = Stage::RotationSync,
            Stage::RotationSync => self.stage = Stage::RotationEdges,
            Stage::RotationEdges => {
                self.stage = if s.graphics_enabled {
                    Stage::RotationLookup(s.zoom)
                } else {
                    Stage::Rooms(s.level)
                }
            }
            Stage::RotationLookup(_) => self.stage = Stage::Rooms(s.level),
            Stage::Rooms(_) => {
                if s.cutaway != 0 {
                    self.stage = Stage::RotationCompose(s.include_visible);
                } else {
                    self.tail();
                }
            }
            Stage::RotationCompose(_) => self.tail(),
            Stage::UiRefresh => self.stage = Stage::Surface,
            Stage::Surface => {
                let Reply::Token(Some(token)) = reply else {
                    unreachable!()
                };
                self.stage = Stage::Flush(token);
            }
            Stage::Flush(_) => {
                if swap {
                    for value in &mut tiles[..cells] {
                        if *value == 10 || *value == 11 {
                            *value ^= 1;
                        }
                    }
                }
                self.stage = if matches!(self.operation, Operation::Rotation(_)) {
                    Stage::Finish
                } else {
                    Stage::Done
                };
            }
            Stage::Finish => self.stage = Stage::Done,
            Stage::Done => unreachable!(),
        }
        Ok(())
    }
}
