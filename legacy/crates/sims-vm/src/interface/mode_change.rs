//! Original4cea5d mode transition. Direct writes and synchronous services keep
//! their original order. This is the transition, not4ce390's later dirty-frame
//! publication or a substitute for any service's implementation.
use serde::{Deserialize, Serialize};

/// The owner's borrowed projection of the fields this routine writes. The
/// camera byte and notification byte belong to the view/process respectively;
/// a host must publish them to those owners, not create duplicate state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    /// Interface+8,+c,+68,+9c,+a0,+a4.
    pub mode: i32,
    pub dirty: u32,
    pub build_tool: i32,
    pub buy_tool: i32,
    pub catalog: i32,
    pub subcatalog: i32,
    /// Root+4 -> View+4c0; process6e57d0.
    pub camera_enabled: u8,
    pub notify_pending: u8,
}

/// Read afresh at every resume, after completed callbacks. No wall-clock read
/// occurs here; the real frontend retains its application-clock input.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Inputs {
    pub application_clock: u32,
    /// Root+18; native explicitly skips both null and address-8.
    pub object_tool_address: u32,
    pub build_view_present: bool,
    /// Global data+50 and signed lot number+24, plus42b9e3's zoning result.
    pub lot_mode: i16,
    pub lot_number: i16,
    pub zoning: i32,
    /// Interface+40, App+90/+f5.
    pub paused: u8,
    pub menu_present: bool,
    pub menu_active: u8,
}

/// Pure40749a/4074c8/4ce7e5/4074b1/428825/4ce805 selection chain.
/// The final90..92 query is executed natively but both outcomes select1.
pub fn catalog(lot_mode: i16, lot: i16, zoning: i32) -> i32 {
    if lot_mode != 1 {
        return 1;
    }
    if (21..=30).contains(&lot) {
        2
    } else if (40..=49).contains(&lot) {
        3
    } else if zoning == 2 && !(93..=98).contains(&lot) {
        4
    } else if (81..=89).contains(&lot) {
        5
    } else if (93..=98).contains(&lot) {
        6
    } else {
        1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Notice {
    Build,
    Buy,
    Live,
    Camera,
    Options,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ///416534 on Root+0.
    RefreshHouse,
    ///43dfc4 on Root+28.
    RefreshObjects,
    ///41570c ->5a0585(application clock), after both refreshes.
    ///Its5a05a0/5a05f3 children perform timed resource-list updates, not RNG seeding.
    TimedResourceUpdates(u32),
    ///40726a's comeandsee controller reconciliation.
    ReconcileLotControllers,
    ///4cefce(null,0), including the old tool's virtual+60 callback.
    ClearTool,
    ///4933dc on Root+18+8. Address is an opaque retained host identity.
    CloseObjectTool(u32),
    ///4b0bd1 on Interface+1f0.
    ClearBuildCache,
    ///4cf0c9(current Interface+68).
    SelectBuildTool(i32),
    ///4cf3da, after catalog and subcatalog publication.
    ResetBuySelection,
    ///4cf360(current Interface+9c).
    SelectBuyTool(i32),
    ///4cef9b(6,0), including both original activation calls.
    SelectLiveTool,
    ///4cfc75(0,0).
    ExpandPanel,
    ///4d31b7; this is the window's pause presentation, not VM clock gating.
    WindowPause { paused: u8, show_overlay: u8 },
    ///4df821 with the original requested-mode message.
    Notice(Notice),
    ///40760d(0x91,0), before clearing process6e57d0.
    Notify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    LiveGate,
    RefreshObjects,
    TimedResources,
    Controllers,
    ToolGate,
    ObjectToolGate,
    BuildCacheGate,
    Publish,
    BuildPause,
    BuyTool,
    BuyPause,
    LivePause,
    LiveNotice,
    CameraPanel,
    CameraPause,
    OptionsPause,
    Notice,
    Notify,
    ClearNotify,
    Dirty,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    target: i32,
    force: u8,
    stage: Stage,
    pending: Option<Request>,
}
impl Change {
    pub fn target(&self) -> i32 {
        self.target
    }
    pub fn new(target: i32, force: u8) -> Self {
        Self {
            target,
            force,
            stage: Stage::Entry,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    fn notice(&self) -> Notice {
        match self.target {
            0 => Notice::Build,
            1 => Notice::Buy,
            2 => Notice::Live,
            4 => Notice::Camera,
            _ => Notice::Options,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        use Request as R;
        use Stage as S;
        let valid = match (&self.pending, self.stage) {
            (None, _) => true,
            (Some(R::RefreshHouse), S::RefreshObjects)
            | (Some(R::RefreshObjects), S::TimedResources)
            | (Some(R::TimedResourceUpdates(_)), S::Controllers)
            | (Some(R::ReconcileLotControllers), S::ToolGate) => self.target == 2,
            (Some(R::ClearTool), S::ObjectToolGate) => matches!(self.target, 0 | 1),
            (Some(R::ClearTool), S::CameraPanel) => self.target == 4,
            (Some(R::ClearTool), S::OptionsPause) => !matches!(self.target, 0..=2 | 4),
            (Some(R::CloseObjectTool(address)), S::BuildCacheGate) => {
                *address != 0 && *address != 8
            }
            (Some(R::ClearBuildCache), S::Publish) => true,
            (Some(R::SelectBuildTool(_)), S::BuildPause) => self.target == 0,
            (Some(R::ResetBuySelection), S::BuyTool) | (Some(R::SelectBuyTool(_)), S::BuyPause) => {
                self.target == 1
            }
            (Some(R::SelectLiveTool), S::LivePause)
            | (
                Some(R::WindowPause {
                    show_overlay: 0, ..
                }),
                S::LiveNotice,
            ) => self.target == 2,
            (Some(R::ExpandPanel), S::CameraPause) => self.target == 4,
            (
                Some(R::WindowPause {
                    paused: 1,
                    show_overlay,
                }),
                S::Notice,
            ) => self.target != 2 && *show_overlay == u8::from(matches!(self.target, 0 | 1 | 4)),
            (Some(R::Notice(n)), S::Notify) => *n == self.notice(),
            (Some(R::Notify), S::ClearNotify) => true,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("mode transition lost its ordered service".into())
        }
    }
    pub fn acknowledge(&mut self, expected: &Request) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(expected) {
            return Err("mode transition reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(
        &mut self,
        fields: &mut Fields,
        input: Inputs,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        use Request as R;
        use Stage as S;
        let request = loop {
            match self.stage {
                S::Entry => {
                    if fields.mode == self.target && self.force == 0 {
                        self.stage = S::Complete;
                        continue;
                    }
                    if fields.mode == 4 {
                        fields.camera_enabled = 0;
                    }
                    self.stage = S::LiveGate;
                }
                S::LiveGate => {
                    self.stage = S::ToolGate;
                    if fields.mode != 2 && self.target == 2 {
                        self.stage = S::RefreshObjects;
                        break R::RefreshHouse;
                    }
                }
                S::RefreshObjects => {
                    self.stage = S::TimedResources;
                    break R::RefreshObjects;
                }
                S::TimedResources => {
                    self.stage = S::Controllers;
                    break R::TimedResourceUpdates(input.application_clock);
                }
                S::Controllers => {
                    self.stage = S::ToolGate;
                    break R::ReconcileLotControllers;
                }
                S::ToolGate => {
                    self.stage = S::ObjectToolGate;
                    if matches!(self.target, 0 | 1) {
                        break R::ClearTool;
                    }
                }
                S::ObjectToolGate => {
                    self.stage = S::BuildCacheGate;
                    if fields.mode != 2
                        && input.object_tool_address != 0
                        && input.object_tool_address.wrapping_add(8) != 0
                    {
                        break R::CloseObjectTool(input.object_tool_address.wrapping_add(8));
                    }
                }
                S::BuildCacheGate => {
                    self.stage = S::Publish;
                    if (fields.mode == 0 || self.target == 0) && input.build_view_present {
                        break R::ClearBuildCache;
                    }
                }
                S::Publish => {
                    fields.mode = self.target;
                    match self.target {
                        0 => {
                            self.stage = S::BuildPause;
                            break R::SelectBuildTool(fields.build_tool);
                        }
                        1 => {
                            fields.catalog =
                                catalog(input.lot_mode, input.lot_number, input.zoning);
                            fields.subcatalog = 0;
                            self.stage = S::BuyTool;
                            break R::ResetBuySelection;
                        }
                        2 => {
                            self.stage = S::LivePause;
                            break R::SelectLiveTool;
                        }
                        4 => {
                            self.stage = S::CameraPanel;
                            break R::ClearTool;
                        }
                        _ => {
                            self.stage = S::OptionsPause;
                            break R::ClearTool;
                        }
                    }
                }
                S::BuyTool => {
                    self.stage = S::BuyPause;
                    break R::SelectBuyTool(fields.buy_tool);
                }
                S::BuildPause | S::BuyPause | S::CameraPause => {
                    self.stage = S::Notice;
                    break R::WindowPause {
                        paused: 1,
                        show_overlay: 1,
                    };
                }
                S::LivePause => {
                    self.stage = S::LiveNotice;
                    break R::WindowPause {
                        paused: input.paused,
                        show_overlay: 0,
                    };
                }
                S::LiveNotice => {
                    self.stage = if input.menu_present && input.menu_active == 0 {
                        S::Notify
                    } else {
                        S::Notice
                    };
                }
                S::CameraPanel => {
                    self.stage = S::CameraPause;
                    break R::ExpandPanel;
                }
                S::OptionsPause => {
                    self.stage = S::Notice;
                    break R::WindowPause {
                        paused: 1,
                        show_overlay: 0,
                    };
                }
                S::Notice => {
                    self.stage = S::Notify;
                    break R::Notice(self.notice());
                }
                S::Notify => {
                    self.stage = S::Dirty;
                    if fields.notify_pending != 0 {
                        self.stage = S::ClearNotify;
                        break R::Notify;
                    }
                }
                S::ClearNotify => {
                    fields.notify_pending = 0;
                    self.stage = S::Dirty;
                }
                S::Dirty => {
                    fields.dirty |= 0xfff7ffbf;
                    self.stage = S::Complete;
                }
                S::Complete => return Ok(None),
            }
        };
        self.pending = Some(request.clone());
        Ok(Some(request))
    }
}
