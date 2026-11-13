//! Ordered4ce44f world publication for every dirty-mask branch. This is a
//! caller kernel: camera, audio, tool and lot children require their actual
//! owners. Inputs are fresh projections of those owners, never retained copies.
use super::state::State;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    pub menu_present: bool,
    pub menu_active: u8,
    pub closing: u8,
    pub include_visible: u8,
    /// CPState+34/+38/+3c/+48, distinct from already-published view fields.
    pub zoom: u32,
    pub rotation: u32,
    pub floor: i32,
    pub walls: i32,
    /// CPState+4c/+50/+60 and process6e54fc.
    pub tool: Option<u32>,
    pub tool_argument: u32,
    pub tool_gate: u32,
    pub current_tool: Option<u32>,
    /// CPState+1ec.
    #[serde(rename = "sound_object")]
    pub selected_person_index: u32,
    pub lot_speed: i16,
    /// Lot+50, signed lot number+24, and42b9e3 zoning result.
    pub lot_kind: i16,
    pub lot_number: i16,
    pub zoning: i32,
}
impl Inputs {
    fn menu_blocks(self) -> bool {
        self.menu_present && self.menu_active == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Music {
    Live,
    Buy,
    MagicBuy,
    Build,
    MagicBuild,
    Vacation,
    Unleashed,
    Superstar,
    MagicTown,
    Downtown,
    Camera,
    Options,
}
impl Music {
    pub fn name(self) -> &'static str {
        match self {
            Self::Live => "bkground_live",
            Self::Buy => "bkground_buy1",
            Self::MagicBuy => "music_magictown_buy",
            Self::Build => "bkground_build",
            Self::MagicBuild => "music_magictown_build",
            Self::Vacation => "station_vacation",
            Self::Unleashed => "station_unleashed",
            Self::Superstar => "station_superstar",
            Self::MagicTown => "music_magictown",
            Self::Downtown => "station_dtbuy",
            Self::Camera => "bkground_camera",
            Self::Options => "bkground_option",
        }
    }
}
/// Pure predicates4074c8/4ce7e5/4074b1/428825/4ce805 in original order.
pub fn music(mode: i32, lot_kind: i16, lot: i16, zoning: i32) -> Option<Music> {
    Some(match mode {
        2 => Music::Live,
        1 if lot_kind == 1 => {
            if (40..=49).contains(&lot) {
                Music::Vacation
            } else if zoning == 2 && !(93..=98).contains(&lot) {
                Music::Unleashed
            } else if (81..=89).contains(&lot) {
                Music::Superstar
            } else if (93..=98).contains(&lot) {
                Music::MagicTown
            } else {
                Music::Downtown
            }
        }
        1 => {
            if (90..=92).contains(&lot) {
                Music::MagicBuy
            } else {
                Music::Buy
            }
        }
        0 if lot_kind == 0 => {
            if (90..=92).contains(&lot) {
                Music::MagicBuild
            } else {
                Music::Build
            }
        }
        3 => Music::Options,
        4 => Music::Camera,
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    RoomMask(u8),     //4ab992
    DrawingMode(u32), //4c4222
    PauseAudio,
    ResumeAudio,
    RefreshAudio,
    Zoom(u32),     //4a7b92(value,0,1)
    Rotation(u32), //4a7dfe(value,0,1)
    Floor(i32),    //4a80c7(value,0)
    SetSpeed(i32), //46d697; includes its synchronous C2
    Pause,
    Resume,                  //46d813/46d83a
    WallsDown(u32),          //4a814d
    Cutaway(u32),            //4a817e
    Roof(u32),               //4b1459(value,0)
    SelectTool(Option<u32>), //476a35, including null
    /// Virtual+30; caller defines only DX. Undetermined ABI high bits are not
    /// invented here. The tool backend must bind the actual word argument.
    ActivateTool {
        tool: Option<u32>,
        argument_low: u16,
    },
    ///4e989b resolves a portrait-cache index before445449 selection.
    ///Keep the old wire tag; this was previously mislabeled as audio.
    #[serde(rename = "SoundObject")]
    SelectPersonIndex(u32), //4e989b
    StopMusic,        //57a6fb(-1)
    PlayMusic(Music), //57a54c(name,-1)
    WindowReset,      //4d2406(0), after Camera/Options music
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Drawing,
    ModeAudio,
    Zoom,
    ZoomAudio,
    Rotation,
    Floor,
    LotMode,
    Transport,
    PauseAudio,
    ResumeAudio,
    Walls,
    Cutaway(u32, u32),
    Roof(u32),
    Tool,
    ActivateTool,
    Sound,
    MusicGate,
    Music,
    Window,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    mask: u32,
    stage: Stage,
    pending: Option<Request>,
}
impl Update {
    pub fn mask(&self) -> u32 {
        self.mask
    }
    pub fn new(mask: u32) -> Self {
        Self {
            mask,
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
        use Request as R;
        use Stage as S;
        let valid = match (self.stage, self.pending) {
            (_, None) => true,
            (S::Drawing, Some(R::RoomMask(_)))
            | (S::ModeAudio, Some(R::DrawingMode(0 | 1)))
            | (S::Zoom, Some(R::PauseAudio | R::ResumeAudio))
            | (S::Music, Some(R::StopMusic)) => self.mask & 2 != 0,
            (S::ZoomAudio, Some(R::Zoom(_))) | (S::Rotation, Some(R::RefreshAudio)) => {
                self.mask & 0x1000 != 0
            }
            (S::Floor, Some(R::Rotation(_))) => self.mask & 0x800 != 0,
            (S::LotMode, Some(R::Floor(_))) => self.mask & 0x100 != 0,
            (S::Transport, Some(R::SetSpeed(_))) => self.mask & 0x40 != 0,
            (S::PauseAudio, Some(R::Pause))
            | (S::ResumeAudio, Some(R::Resume))
            | (S::Walls, Some(R::PauseAudio | R::ResumeAudio)) => self.mask & 0x800000 != 0,
            (S::Cutaway(b, c), Some(R::WallsDown(a))) => {
                self.mask & 0x80 != 0
                    && matches!((a, b, c), (0, 0, 0) | (1, 0, 0) | (0, 1, 0) | (0, 0, 1))
            }
            (S::Roof(c), Some(R::Cutaway(b))) => {
                self.mask & 0x80 != 0 && matches!((b, c), (0, 0) | (1, 0) | (0, 1))
            }
            (S::Tool, Some(R::Roof(0 | 1))) => self.mask & 0x80 != 0,
            (S::ActivateTool, Some(R::SelectTool(Some(t)))) => self.mask & 0x630 != 0 && t != 0,
            (S::Sound, Some(R::SelectTool(None) | R::ActivateTool { .. })) => {
                self.mask & 0x630 != 0
            }
            (S::MusicGate, Some(R::SelectPersonIndex(_))) => self.mask & 8 != 0,
            (S::Window, Some(R::PlayMusic(Music::Camera | Music::Options))) => self.mask & 2 != 0,
            (S::Complete, Some(R::PlayMusic(m))) => {
                self.mask & 2 != 0 && !matches!(m, Music::Camera | Music::Options)
            }
            (S::Complete, Some(R::WindowReset)) => self.mask & 2 != 0,
            _ => false,
        };
        let required = match self.stage {
            S::Drawing | S::ModeAudio | S::Music | S::Window => 2,
            S::ZoomAudio => 0x1000,
            S::PauseAudio | S::ResumeAudio => 0x800000,
            S::Cutaway(..) | S::Roof(_) => 0x80,
            S::ActivateTool => 0x630,
            _ => 0,
        };
        if valid && (required == 0 || self.mask & required != 0) {
            Ok(())
        } else {
            Err("world publication lost its child".into())
        }
    }
    /// Restricted entry used by the existing transport host. Other projections
    /// are deliberately unobserved: reject their mask bits before advancing.
    /// This does not construct missing camera/tool state in that host.
    pub fn advance_transport(
        &mut self,
        state: &State,
        lot_mode: &mut i16,
        input: super::transport::Inputs,
    ) -> Result<Option<Request>, String> {
        if self.mask & !super::transport::DIRTY != 0 {
            return Err("transport host cannot service unrelated publication bits".into());
        }
        self.advance(
            state,
            lot_mode,
            Inputs {
                menu_present: input.menu_present,
                menu_active: input.menu_active,
                closing: input.closing,
                lot_speed: input.lot_speed,
                include_visible: 0,
                zoom: 0,
                rotation: 0,
                floor: 0,
                walls: 0,
                tool: None,
                tool_argument: 0,
                tool_gate: 0,
                current_tool: None,
                selected_person_index: 0,
                lot_kind: 0,
                lot_number: 0,
                zoning: 0,
            },
        )
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("world publication reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(
        &mut self,
        state: &State,
        lot_mode: &mut i16,
        input: Inputs,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        use Request as R;
        use Stage as S;
        let r = loop {
            match self.stage {
                S::Entry => {
                    if input.menu_blocks() || input.closing != 0 {
                        self.stage = S::Complete;
                        continue;
                    }
                    self.stage = S::Zoom;
                    if self.mask & 2 != 0 {
                        self.stage = S::Drawing;
                        break R::RoomMask(input.include_visible);
                    }
                }
                S::Drawing => {
                    self.stage = S::ModeAudio;
                    if state.mode != 3 {
                        break R::DrawingMode(u32::from(!matches!(state.mode, 2 | 4)));
                    }
                }
                S::ModeAudio => {
                    self.stage = S::Zoom;
                    if state.mode != 2 {
                        break R::PauseAudio;
                    }
                    if state.paused == 0 {
                        break R::ResumeAudio;
                    }
                }
                S::Zoom => {
                    self.stage = S::Rotation;
                    if self.mask & 0x1000 != 0 {
                        self.stage = S::ZoomAudio;
                        break R::Zoom(input.zoom);
                    }
                }
                S::ZoomAudio => {
                    self.stage = S::Rotation;
                    break R::RefreshAudio;
                }
                S::Rotation => {
                    self.stage = S::Floor;
                    if self.mask & 0x800 != 0 {
                        break R::Rotation(input.rotation);
                    }
                }
                S::Floor => {
                    self.stage = S::LotMode;
                    if self.mask & 0x100 != 0 {
                        break R::Floor(input.floor);
                    }
                }
                S::LotMode => {
                    self.stage = S::Transport;
                    if self.mask & 0x42 != 0 {
                        match state.mode {
                            0 => *lot_mode = 1,
                            1 => *lot_mode = 2,
                            2 => *lot_mode = 0,
                            3 => *lot_mode = 3,
                            4 => *lot_mode = 4,
                            _ => (),
                        }
                    }
                    if self.mask & 0x40 != 0 && state.speed != i32::from(input.lot_speed) {
                        break R::SetSpeed(state.speed);
                    }
                }
                S::Transport => {
                    self.stage = S::Walls;
                    if self.mask & 0x800000 != 0 && state.mode == 2 {
                        if state.paused == 0 {
                            self.stage = S::ResumeAudio;
                            break R::Resume;
                        }
                        self.stage = S::PauseAudio;
                        break R::Pause;
                    }
                }
                S::PauseAudio => {
                    self.stage = S::Walls;
                    break R::PauseAudio;
                }
                S::ResumeAudio => {
                    self.stage = S::Walls;
                    break R::ResumeAudio;
                }
                S::Walls => {
                    self.stage = S::Tool;
                    if self.mask & 0x80 != 0 {
                        let (a, b, c) = match input.walls {
                            1 => (0, 0, 0),
                            2 => (1, 0, 0),
                            4 => (0, 1, 0),
                            _ => (0, 0, 1),
                        };
                        self.stage = S::Cutaway(b, c);
                        break R::WallsDown(a);
                    }
                }
                S::Cutaway(b, c) => {
                    self.stage = S::Roof(c);
                    break R::Cutaway(b);
                }
                S::Roof(c) => {
                    self.stage = S::Tool;
                    break R::Roof(c);
                }
                S::Tool => {
                    self.stage = S::Sound;
                    if self.mask & 0x630 != 0 && input.tool_gate == 0 {
                        if input.tool == Some(0) || input.current_tool == Some(0) {
                            self.stage = S::Tool;
                            return Err("null tool must use None".into());
                        }
                        if input.tool.is_none() {
                            break R::SelectTool(None);
                        }
                        self.stage = S::ActivateTool;
                        if input.tool != input.current_tool {
                            break R::SelectTool(input.tool);
                        }
                    }
                }
                S::ActivateTool => {
                    self.stage = S::Sound;
                    break R::ActivateTool {
                        tool: input.tool,
                        argument_low: input.tool_argument as u16,
                    };
                }
                S::Sound => {
                    self.stage = S::MusicGate;
                    if self.mask & 8 != 0 {
                        break R::SelectPersonIndex(input.selected_person_index);
                    }
                }
                S::MusicGate => {
                    self.stage = S::Complete;
                    if self.mask & 2 != 0 && !input.menu_blocks() {
                        self.stage = S::Music;
                        break R::StopMusic;
                    }
                }
                S::Music => {
                    self.stage = S::Complete;
                    if let Some(track) =
                        music(state.mode, input.lot_kind, input.lot_number, input.zoning)
                    {
                        if matches!(track, Music::Camera | Music::Options) {
                            self.stage = S::Window;
                        }
                        break R::PlayMusic(track);
                    }
                }
                S::Window => {
                    self.stage = S::Complete;
                    break R::WindowReset;
                }
                S::Complete => return Ok(None),
            }
        };
        self.pending = Some(r);
        Ok(self.pending)
    }
}
