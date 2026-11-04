//! Deferred host boundary for the original non-yielding TTAB action path.
use crate::{Location, ObjectId, Program, Start, Status, ThreadId, Vm, action::Action};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub origin: Location,
    pub actor: ObjectId,
    pub action: Action,
}
impl Vm {
    pub fn finish_immediate(
        &mut self,
        p: &Program,
        id: ThreadId,
        request: u64,
        result: bool,
    ) -> Result<(), String> {
        let t = self.threads.get(&id).ok_or("missing immediate requester")?;
        if !matches!(&t.status,Status::AwaitingImmediate(r) if r.id==request) {
            return Err("immediate request mismatch".into());
        }
        let ins = crate::current(p, t)?.clone();
        self.complete_host_primitive(p, id, result, ins.on_true, ins.on_false)
    }
    pub fn begin_immediate_callback(
        &mut self,
        p: &Program,
        id: ThreadId,
        start: Start<'_>,
    ) -> Result<(), String> {
        let mut t = self
            .threads
            .get(&id)
            .ok_or("missing immediate actor thread")?
            .clone();
        crate::native_suspension::validate(p, &t)?;
        self.push_callback_start(p, &mut t, start)?;
        t.callback.as_mut().unwrap().external = true;
        self.threads.insert(id, t);
        Ok(())
    }
    pub fn immediate_eligibility(
        &mut self,
        actor: ObjectId,
        target: ObjectId,
        flags: u32,
    ) -> Result<bool, String> {
        self.action_eligibility_gate(actor, target, flags)
    }
}
