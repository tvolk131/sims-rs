//! Original application Draw body4d1b37. Each callback and clock remains an
//! explicit saved child; this cursor does not stand in for a world renderer.
pub mod framework;
pub mod regions;
pub mod context;
pub mod targets;
pub mod background;
pub mod ground;
use super::{
    fps::History,
    profiler::{Buffer, Record},
};
use serde::{Deserialize, Serialize};
pub mod view;
pub mod actions;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    /// Process-global6f0f48, shared by nested Draw calls.
    pub busy: u8,
    pub view: Option<u32>,   // application+174, reread for the end callback
    pub unload: Option<u32>, // application+11ac, read after ApplicationView
    pub framework: u32,      // *6fe944
    pub world: u32,          // (*6e48bc)+4
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Timestamp { name: u32 },
    FpsClock,
    ViewBegin { owner: u32 },
    ApplicationView,
    Unload { owner: u32 },
    Framework { owner: u32 },
    World { owner: u32, argument: u32 },
    ViewEnd { owner: u32 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Timestamp(u64),
    Clock(u32),
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Entry,
    BeginMarker,
    Fps,
    ViewBegin,
    ApplicationView,
    Unload,
    Framework,
    World,
    EndMarker,
    ViewEnd,
    Clear,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    phase: Phase,
    pending: Option<Request>,
}
impl Frame {
    pub fn at_entry() -> Self {
        Self {
            phase: Phase::Entry,
            pending: None,
        }
    }
    pub fn is_at_entry(&self) -> bool {
        self.phase == Phase::Entry
    }
    pub fn past_framework(&self)->bool {matches!(self.phase,Phase::World|Phase::EndMarker|Phase::ViewEnd|Phase::Clear|Phase::Complete)}
    pub fn past_world(&self)->bool {matches!(self.phase,Phase::EndMarker|Phase::ViewEnd|Phase::Clear|Phase::Complete)}
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    /// Caller position, including a skipped optional view. Source-bound users
    /// must separately validate that their view actually exists at this call.
    pub fn past_view_begin(&self) -> bool {
        matches!(
            self.phase,
            Phase::ApplicationView
                | Phase::Unload
                | Phase::Framework
                | Phase::World
                | Phase::EndMarker
                | Phase::ViewEnd
                | Phase::Clear
                | Phase::Complete
        )
    }
    pub fn at_application_view(&self) -> bool { self.phase == Phase::ApplicationView }
    pub fn past_application_view(&self) -> bool {
        matches!(self.phase, Phase::Unload | Phase::Framework | Phase::World | Phase::EndMarker | Phase::ViewEnd | Phase::Clear | Phase::Complete)
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Some(request) = self.pending {
            let valid = matches!(
                (self.phase, request),
                (Phase::BeginMarker, Request::Timestamp { name: 0x6c7c7c })
                    | (Phase::Fps, Request::FpsClock)
                    | (Phase::ViewBegin, Request::ViewBegin { owner: 1.. })
                    | (Phase::ApplicationView, Request::ApplicationView)
                    | (Phase::Unload, Request::Unload { owner: 1.. })
                    | (Phase::Framework, Request::Framework { owner: 1.. })
                    | (
                        Phase::World,
                        Request::World {
                            owner: 1..,
                            argument: 0
                        }
                    )
                    | (Phase::EndMarker, Request::Timestamp { name: 0x6c7c64 })
                    | (Phase::ViewEnd, Request::ViewEnd { owner: 1.. })
            );
            if !valid {
                return Err("application Draw saved child differs from cursor".into());
            }
        }
        Ok(())
    }
    pub fn advance(&mut self, fields: &mut Fields) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        for _ in 0..4 {
            let request = match self.phase {
                Phase::Entry => {
                    if fields.busy != 0 {
                        self.phase = Phase::Complete;
                        return Ok(None);
                    }
                    fields.busy = 1;
                    self.phase = Phase::BeginMarker;
                    continue;
                }
                Phase::BeginMarker => Request::Timestamp { name: 0x6c7c7c },
                Phase::Fps => Request::FpsClock,
                Phase::ViewBegin => match fields.view {
                    Some(owner) if owner != 0 => Request::ViewBegin { owner },
                    None => {
                        self.phase = Phase::ApplicationView;
                        continue;
                    }
                    _ => return Err("application Draw has null view identity".into()),
                },
                Phase::ApplicationView => Request::ApplicationView,
                Phase::Unload => match fields.unload {
                    Some(owner) if owner != 0 => Request::Unload { owner },
                    None => {
                        self.phase = Phase::Framework;
                        continue;
                    }
                    _ => return Err("application Draw has null unload identity".into()),
                },
                Phase::Framework => {
                    if fields.framework == 0 {
                        return Err("application Draw framework absent".into());
                    }
                    Request::Framework {
                        owner: fields.framework,
                    }
                }
                Phase::World => {
                    if fields.world == 0 {
                        return Err("application Draw world absent".into());
                    }
                    Request::World {
                        owner: fields.world,
                        argument: 0,
                    }
                }
                Phase::EndMarker => Request::Timestamp { name: 0x6c7c64 },
                Phase::ViewEnd => match fields.view {
                    Some(owner) if owner != 0 => Request::ViewEnd { owner },
                    None => {
                        self.phase = Phase::Clear;
                        continue;
                    }
                    _ => return Err("application Draw has null end view identity".into()),
                },
                Phase::Clear => {
                    fields.busy = 0;
                    self.phase = Phase::Complete;
                    return Ok(None);
                }
                Phase::Complete => return Ok(None),
            };
            self.pending = Some(request);
            return Ok(self.pending);
        }
        Err("application Draw exceeded cursor work bound".into())
    }
    pub fn acknowledge(
        &mut self,
        request: Request,
        reply: Reply,
        history: &mut History,
        profiler: &mut Buffer,
    ) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("application Draw child does not own reply".into());
        }
        match (request, reply) {
            (Request::Timestamp { name }, Reply::Timestamp(timestamp)) => {
                profiler.append(Record {
                    timestamp,
                    name,
                    argument: 0,
                })?
            }
            (Request::FpsClock, Reply::Clock(now)) => history.sample(now)?,
            (
                Request::ViewBegin { .. }
                | Request::ApplicationView
                | Request::Unload { .. }
                | Request::Framework { .. }
                | Request::World { .. }
                | Request::ViewEnd { .. },
                Reply::Done,
            ) => (),
            _ => return Err("application Draw reply kind differs".into()),
        }
        self.phase = match self.phase {
            Phase::BeginMarker => Phase::Fps,
            Phase::Fps => Phase::ViewBegin,
            Phase::ViewBegin => Phase::ApplicationView,
            Phase::ApplicationView => Phase::Unload,
            Phase::Unload => Phase::Framework,
            Phase::Framework => Phase::World,
            Phase::World => Phase::EndMarker,
            Phase::EndMarker => Phase::ViewEnd,
            Phase::ViewEnd => Phase::Clear,
            _ => unreachable!("validated pending child has a call phase"),
        };
        self.pending = None;
        Ok(())
    }
}
