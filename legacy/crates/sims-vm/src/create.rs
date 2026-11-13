//! Create Object Instance is synchronous in the original engine. The host may
//! suspend its factory stages for diagnostic budgets, but must not advance time.
use crate::{Location, ObjectId, Program, Status, ThreadId, Vm, current};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub origin: Location,
    pub caller: ObjectId,
    pub target: ObjectId,
    pub operands: [u8; 8],
    /// Allocated, registered, still off-lot while its Init/Main run.
    pub created: Option<ObjectId>,
    pub placement: Option<Placement>,
}
impl Vm {
    /// Original 44e0c8 selects the destination only after resolving the resource.
    /// Mode 7 always reads argument zero; mode 8 has a signed byte local index.
    /// Capture this ID and its position before entering the object factory:
    /// callbacks may change the frame or remove the selected object.
    pub fn creation_placement_target(
        &self,
        thread: ThreadId,
        request: &Request,
    ) -> Result<ObjectId, String> {
        let mode = request.operands[4];
        if matches!(mode, 0 | 2 | 5 | 9) {
            return Ok(request.caller);
        }
        if !matches!(mode, 7 | 8) {
            return Ok(request.target);
        }
        let frame = self
            .threads
            .get(&thread)
            .and_then(|t| t.stack.last())
            .ok_or("creation frame absent")?;
        if mode == 7 {
            return frame
                .args
                .first()
                .copied()
                .ok_or_else(|| "creation needs parameter 0 (native error 0x08)".into());
        }
        let index = request.operands[6] as i8;
        if index < 0 || index as usize >= frame.locals.len() {
            return Err("creation local invalid (native error 0x08)".into());
        }
        if frame.opaque_locals.contains(&(index as usize)) {
            return Err("creation reads opaque local".into());
        }
        Ok(frame.locals[index as usize])
    }

    /// 0x425afe selects a neighbor's linked resource before any factory work.
    /// This host represents that resource identity by its bound GUID.
    pub fn creation_guid(&self, request: &Request) -> Result<Option<u32>, String> {
        if request.operands[5] & 4 == 0 {
            return Ok(Some(u32::from_le_bytes(
                request.operands[..4].try_into().unwrap(),
            )));
        }
        let people = self
            .world
            .person
            .as_ref()
            .ok_or("creation neighborhood absent")?;
        let neighbor = if let Some(slots) = &people.neighbor_slots {
            usize::try_from(request.target)
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| slots.get(i))
                .and_then(|id| *id)
                .and_then(|id| people.neighbors.get(&id))
        } else {
            people.neighbors.get(&request.target)
        };
        Ok(neighbor.map(|n| n.guid))
    }

    /// Native wrapper tail, AFTER placement and the optional init-visitor call.
    /// The root parameter pointer is raw: with no arguments it aliases local 0
    /// or the reusable arena immediately after the root frame's header.
    pub fn finish_creation_with_parameters(
        &mut self,
        program: &Program,
        thread: ThreadId,
        request: u64,
        object: ObjectId,
    ) -> Result<(), String> {
        let parent = self.threads.get(&thread).ok_or("creation parent absent")?;
        let Status::AwaitingCreation(r) = &parent.status else {
            return Err("creation parameter parent is not waiting".into());
        };
        if r.id != request || r.created != Some(object) {
            return Err("creation parameter binding mismatch".into());
        }
        let flags = r.operands[5];
        if flags & 0x12 == 0x12 {
            return Err("conflicting creation parameter flags (native error 0x36)".into());
        }
        let stack_object = parent
            .stack
            .last()
            .ok_or("creator frame absent")?
            .stack_object;
        let value = if flags & 2 != 0 {
            r.caller
        } else {
            self.world.objects[&r.caller].temps[0]
        };
        if flags & 0x12 != 0 {
            let child = *self
                .object_threads
                .get(&object)
                .ok_or("created VM absent")?;
            let t = self
                .threads
                .get_mut(&child)
                .ok_or("created thread absent")?;
            if let Some(root) = t.stack.first() {
                let address = root.storage_offset + 8;
                // A zero-sized root followed immediately by another live frame
                // aliases its code word. Keep that unsupported layout explicit.
                if t.stack.iter().skip(1).any(|f| f.storage_offset == address) {
                    return Err("creation root parameter aliases another live frame header".into());
                }
                crate::retained_frames::ensure_capacity(t, address + 1)?;
                let root = &mut t.stack[0];
                if let Some(arg) = root.args.first_mut() {
                    *arg = value;
                } else if let Some(local) = root.locals.first_mut() {
                    *local = value;
                    root.opaque_locals.remove(&0);
                }
                if flags & 2 != 0 {
                    root.stack_object = stack_object;
                }
                crate::save_frame_storage(t)?;
                t.frame_memory
                    .resize(t.frame_memory.len().max(address + 1), Some(0));
                t.frame_memory[address] = Some(value);
            }
        }
        self.finish_creation(program, thread, request, Some(object))
    }

    pub fn finish_creation(
        &mut self,
        program: &Program,
        thread: ThreadId,
        request: u64,
        result: Option<ObjectId>,
    ) -> Result<(), String> {
        let t = self
            .threads
            .get_mut(&thread)
            .ok_or("missing creation thread")?;
        let Status::AwaitingCreation(r) = &t.status else {
            return Err("not awaiting creation".into());
        };
        if r.id != request || result.is_some_and(|id| r.created != Some(id)) {
            return Err("creation result does not match request".into());
        }
        let ins = current(program, t)?.clone();
        if let Some(id) = result {
            t.stack.last_mut().unwrap().stack_object = id;
        }
        self.complete_host_primitive(program, thread, result.is_some(), ins.on_true, ins.on_false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Placement {
    pub target: ObjectId,
    pub position: [i32; 2],
    pub level: i32,
    pub room: i16,
    pub wall_direction: Option<u8>,
}
/// Mode 9 reads a signed local rotation and tests the opposite wall before allocation.
pub fn relative_floor(position: [i32; 2], facing: i16, rotation: i16) -> ([i32; 2], u8) {
    let direction = (i32::from(facing) + i32::from(rotation)) % 8;
    let [x, y] = match direction & 7 {
        0 => [-16, 0],
        2 => [0, 16],
        4 => [16, 0],
        6 => [0, -16],
        _ => [0, 0],
    };
    (
        [position[0].wrapping_add(x), position[1].wrapping_add(y)],
        ((i32::from(facing) + 4 + i32::from(rotation)) % 8) as u8,
    )
}
#[cfg(test)]
mod tests {
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_relative_floor_coordinates_and_wall_direction() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/home-create.json")).unwrap();
        for row in v["cases"].as_array().unwrap() {
            let c = &row["input"];
            let (xy, d) = super::relative_floor(
                [32, 48],
                c["facing"].as_i64().unwrap() as i16,
                c["local"].as_i64().unwrap() as i16,
            );
            let calls = row["original"]["calls"].as_array().unwrap();
            let check = calls.iter().find(|c| c[0] == "check").unwrap();
            let wall = calls.iter().find(|c| c[0] == "wall").unwrap();
            assert_eq!(serde_json::json!(xy), check[1], "{c}");
            assert_eq!(serde_json::json!(d), wall[1], "{c}");
        }
    }
}

/// Original Create Object modes 0/3 use the eight-direction tile offset table.
pub fn in_front(position: [i32; 2], facing: u8) -> [i32; 2] {
    let (dx, dy) = [
        (-16, 0),
        (-16, 16),
        (0, 16),
        (16, 16),
        (16, 0),
        (16, -16),
        (0, -16),
        (-16, -16),
    ][usize::from(facing & 7)];
    [position[0].wrapping_add(dx), position[1].wrapping_add(dy)]
}
#[cfg(test)]
mod domestic_tests {
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_front_creation_coordinates() {
        let report: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/domestic-create-front.json"))
                .unwrap();
        for row in report["cases"].as_array().unwrap() {
            let c = &row["input"];
            let (xy, d) = if c["mode"] == 0 {
                ([32, 48], c["facing"].as_u64().unwrap_or(0) as u8)
            } else {
                ([80, 96], 2)
            };
            let Some(expected) = row["original"]["calls"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e[0] == "check")
            else {
                continue;
            };
            assert_eq!(
                serde_json::json!(super::in_front(xy, d)),
                expected[1],
                "{c}"
            );
        }
    }
}
