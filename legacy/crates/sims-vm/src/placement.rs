//! Synchronous Find Location request; the spatial host owns its world queries.
use crate::{Location, ObjectId, Program, Status, Thread, ThreadId, Vm, current};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub origin: Location,
    pub caller: ObjectId,
    pub target: ObjectId,
    pub reference: ObjectId,
    pub operands: [u8; 8],
}
impl Vm {
    pub(crate) fn describe_find_location(
        &self,
        t: &Thread,
        b: &[u8; 8],
        id: u64,
    ) -> Result<Request, String> {
        let f = t.stack.last().ok_or("missing Find Location frame")?;
        if !self.world.objects.contains_key(&f.stack_object) {
            return Err("Find Location missing target (native error 0x15)".into());
        }
        if ![0, 1, 3, 4, 5].contains(&b[0]) || b[2] & !7 != 0 || (b[0] == 0 && b[2] & 1 == 0) {
            return Err(
                "Find Location requires a supported search/detach mode and local-reference profile"
                    .into(),
            );
        }
        let reference = if b[2] & 1 != 0 {
            let index = usize::from(b[1]);
            let reference = *f
                .locals
                .get(index)
                .ok_or("Find Location local out of bounds (native error 0x33)")?;
            if f.opaque_locals.contains(&index) {
                return Err("Find Location reads opaque native local".into());
            }
            if !self.world.objects.contains_key(&reference) {
                return Err("Find Location missing reference (native error 0x17)".into());
            }
            reference
        } else {
            0
        };
        if matches!(b[0], 3 | 4) && reference == 0 {
            return Err(
                "directional Find Location requires a reference (native error 0x3d)".into(),
            );
        }
        Ok(Request {
            id,
            origin: f.location.clone(),
            caller: f.caller,
            target: f.stack_object,
            reference,
            operands: *b,
        })
    }
    pub fn finish_find_location(
        &mut self,
        p: &Program,
        thread: ThreadId,
        request: u64,
        success: bool,
    ) -> Result<(), String> {
        let t = self
            .threads
            .get(&thread)
            .ok_or("missing placement thread")?;
        if !matches!(&t.status,Status::AwaitingPlacement(r) if r.id==request) {
            return Err("placement request mismatch".into());
        }
        let ins = current(p, t)?.clone();
        self.complete_host_primitive(p, thread, success, ins.on_true, ins.on_false)
    }
}
