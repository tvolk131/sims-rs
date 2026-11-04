//! 44acff UI effects. Calls are synchronous host boundaries, not tick yields.
pub mod widgets;
use crate::{Fault, Frame, Program, Status, Thread, ThreadId, Vm, World};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    Control,
    Portrait,
    Relationship,
}
impl Kind {
    pub fn first(mode: u8) -> Option<Self> {
        match mode {
            0 => Some(Self::Control),
            1 => Some(Self::Portrait),
            2 => Some(Self::Relationship),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub kind: Kind,
    pub value: i16,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Call {
    tick: u64,
    frame: Frame,
    operands: [u8; 8],
    request: Request,
}
impl Call {
    pub fn request(&self) -> Request {
        self.request
    }
    pub(crate) fn validate(&self, vm: &Vm, p: &Program, frame: &Frame) -> Result<(), String> {
        let (source, routine) = p.resolve(&self.frame.code_owner, self.frame.location.bhav)?;
        let ins = routine
            .body
            .instructions
            .get(self.frame.location.instruction)
            .ok_or("UI-effect instruction absent")?;
        if self.tick != vm.tick
            || self.request.id == 0
            || self.request.id >= vm.next_request
            || source != self.frame.location.source
            || ins.opcode != 0x22
            || ins.operands != self.operands
            || self.frame.location != frame.location
            || self.frame.code_owner != frame.code_owner
            || self.frame.caller != frame.caller
            || self.frame.callee != frame.callee
            || self.frame.storage_offset != frame.storage_offset
            || self.request.enabled != (self.operands[4] & 1 != 0)
            || !matches!(
                (self.operands[0], self.request.kind),
                (0, Kind::Control | Kind::Portrait) | (1, Kind::Portrait) | (2, Kind::Relationship)
            )
        {
            return Err("invalid saved UI-effect call".into());
        }
        Ok(())
    }
}

/// 4256f4/+4: signed, one-based neighborhood slots, not live object IDs.
pub fn neighbor_guid(world: &World, id: i16) -> Result<Option<u32>, String> {
    if id <= 0 {
        return Ok(None);
    }
    let state = world
        .person
        .as_ref()
        .ok_or("UI-effect neighborhood absent")?;
    let slots = state
        .neighbor_slots
        .as_ref()
        .ok_or("UI-effect neighbor vector absent")?;
    let Some(Some(key)) = slots.get(id as usize - 1) else {
        return Ok(None);
    };
    Ok(Some(
        state
            .neighbors
            .get(key)
            .ok_or("UI-effect neighbor record absent")?
            .guid,
    ))
}

impl Vm {
    pub(crate) fn ui_effect_primitive(
        &mut self,
        p: &Program,
        t: &mut Thread,
        ins: &sims_formats::behavior::Instruction,
    ) -> Result<String, String> {
        let Some(kind) = Kind::first(ins.operands[0]) else {
            t.last_result = true;
            t.stack
                .last_mut()
                .ok_or("UI-effect frame absent")?
                .continuation = 0;
            crate::branch(p, t, ins.on_true)?;
            return Ok("UI-effect unknown mode returns true without reading operands".into());
        };
        let value = self.read(p, t, ins.operands[1], crate::word(&ins.operands, 2))?;
        let id = self.next_request;
        self.next_request = id.checked_add(1).ok_or("UI-effect request ID overflow")?;
        t.status = Status::AwaitingUiEffect(Box::new(Call {
            tick: self.tick,
            frame: t.stack.last().ok_or("UI-effect frame absent")?.clone(),
            operands: ins.operands,
            request: Request {
                id,
                kind,
                value,
                enabled: ins.operands[4] & 1 != 0,
            },
        }));
        Ok("UI-effect host call pending".into())
    }

    /// The host has completed this exact native void call. A control call is
    /// followed by a fresh data read and a portrait call, even for mode zero.
    pub fn complete_ui_effect(
        &mut self,
        p: &Program,
        thread: ThreadId,
        request: Request,
    ) -> Result<(), String> {
        let t = self.threads.get(&thread).ok_or("UI-effect thread absent")?;
        let Status::AwaitingUiEffect(call) = &t.status else {
            return Err("UI-effect call absent".into());
        };
        if call.request != request {
            return Err("stale UI-effect response".into());
        }
        call.validate(self, p, t.stack.last().ok_or("UI-effect frame absent")?)?;
        if request.kind == Kind::Control {
            let mut call = call.clone();
            let mut t = t.clone();
            // Do not reuse the first value: a host callback can change the
            // addressed state. Keep mutations from the completed first call.
            match self.read(p, &mut t, call.operands[1], crate::word(&call.operands, 2)) {
                Ok(value) => {
                    call.request.kind = Kind::Portrait;
                    call.request.value = value;
                    t.status = Status::AwaitingUiEffect(call);
                    self.threads.insert(thread, t);
                }
                Err(message) => {
                    t.status = Status::Faulted(Fault {
                        message: message.clone(),
                        stack: t.stack.iter().map(|f| f.location.clone()).collect(),
                    });
                    self.threads.insert(thread, t);
                    return Err(message);
                }
            }
            Ok(())
        } else {
            let ins = crate::current(p, t)?.clone();
            self.complete_host_primitive(p, thread, true, ins.on_true, ins.on_false)
        }
    }
}
