//! 44d96f is synchronous. Host services may suspend for a diagnostic budget,
//! but Burn itself never advances lot time or writes the Stack Object ID.
use crate::{Location, ObjectId, Program, Status, ThreadId, Vm, current};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub origin: Location,
    pub caller: ObjectId,
    pub target: ObjectId,
    pub operands: [u8; 8],
    pub started: bool,
}
impl Vm {
    pub fn finish_burn(
        &mut self,
        program: &Program,
        thread: ThreadId,
        request: &Request,
        success: bool,
    ) -> Result<(), String> {
        let t = self
            .threads
            .get(&thread)
            .ok_or("Burn caller thread absent")?;
        if !matches!(&t.status, Status::AwaitingBurn(r) if r == request) {
            return Err("stale Burn service response".into());
        }
        let frame = t.stack.last().ok_or("Burn caller frame absent")?;
        let ins = current(program, t)?.clone();
        if frame.location != request.origin
            || frame.caller != request.caller
            || frame.stack_object != request.target
            || ins.opcode != 9
            || ins.operands != request.operands
        {
            return Err("Burn response no longer belongs to the current instruction".into());
        }
        self.complete_host_primitive(program, thread, success, ins.on_true, ins.on_false)
    }
}
