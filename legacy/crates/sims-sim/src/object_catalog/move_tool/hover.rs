//! 486d95 hover transfer. It clears the old low flags before publishing+4c,
//! then looks up the requested short ID afresh and sets its low flags.
use serde::{Deserialize, Serialize};
pub trait View {
    fn lookup(&self, id: i16) -> Result<Option<u32>, String>;
    fn flags(&self, object: u32) -> Result<u32, String>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub object: u32,
    pub low: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Publish,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transfer {
    target: i16,
    flag: u8,
    stage: Stage,
    pending: Option<Request>,
}
impl Transfer {
    pub(super) fn clearing(&self) -> bool {
        self.target == 0 && self.flag == 0
    }
    pub fn new(target: u32, flag: u32) -> Self {
        Self {
            target: target as i16,
            flag: flag as u8,
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
        if self.pending.is_some_and(|r| {
            r.object == 0
                || match self.stage {
                    Stage::Entry => true,
                    Stage::Publish => r.low & !0xe != 0,
                    Stage::Complete => r.low & !0x1f != 0 || r.low & 1 == 0,
                }
        }) {
            Err("move hover lost its low-flag child".into())
        } else {
            Ok(())
        }
    }
    pub fn acknowledge(&mut self, r: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(r) {
            return Err("move hover reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, hover: &mut i16, v: &impl View) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            match self.stage {
                Stage::Entry => {
                    if *hover == self.target {
                        self.stage = Stage::Complete;
                        return Ok(None);
                    }
                    if let Some(object) = v.lookup(*hover)? {
                        self.pending = Some(Request {
                            object,
                            low: v.flags(object)? & 0xe,
                        });
                        self.stage = Stage::Publish;
                        return Ok(self.pending);
                    }
                    self.stage = Stage::Publish;
                }
                Stage::Publish => {
                    *hover = self.target;
                    if let Some(object) = v.lookup(self.target)? {
                        self.pending = Some(Request {
                            object,
                            low: (v.flags(object)? & 0x1e) | if self.flag != 0 { 0x11 } else { 1 },
                        });
                        self.stage = Stage::Complete;
                        return Ok(self.pending);
                    }
                    self.stage = Stage::Complete;
                }
                Stage::Complete => return Ok(None),
            }
        }
    }
}

struct Live<'a>(&'a sims_vm::World);
impl View for Live<'_> {
    fn lookup(&self, id: i16) -> Result<Option<u32>, String> {
        //40743b's bounded script-ID registry lookup. VM object IDs are script
        //IDs in this adapter; retained pointer identities stay inside the caller.
        Ok((id > 0 && self.0.objects.contains_key(&id)).then_some(id as u32))
    }
    fn flags(&self, id: u32) -> Result<u32, String> {
        let id =
            i16::try_from(id).map_err(|_| "move hover object identity outside live registry")?;
        self.0
            .objects
            .get(&id)
            .and_then(|o| o.native_flags)
            .ok_or("move hover native flags absent".into())
    }
}
impl crate::Simulation {
    /// Concrete486d95 ->4320da ->434101 on the current lot. No ticks, script
    ///frames or clock credit are processed. All multipart invalidation remains
    ///in the existing native low-flag service, rather than direct flag edits.
    pub fn advance_move_tool_hover(
        &mut self,
        p: &sims_vm::Program,
        hover: &mut i16,
        call: &mut Transfer,
    ) -> Result<(), String> {
        while let Some(r) = call.advance(hover, &Live(&self.vm.world))? {
            let id =
                i16::try_from(r.object).map_err(|_| "move hover identity outside live registry")?;
            self.update_native_low_flags(p, id, r.low)?;
            call.acknowledge(r)?;
        }
        Ok(())
    }
}
