//! Original 0x44f2bf function search. The lot supplies native registry order and
//! positions. Nonempty eligibility trees and indoor/outdoor maps remain explicit
//! boundaries; empty and unconditional function tables are evaluated natively.
use crate::{ObjectId, Program, Thread, Vm};

pub fn selector(mode: i16) -> Option<(Option<i16>, Option<usize>)> {
    Some(match mode {
        0 => (Some(31), Some(18)),
        1 => (Some(32), Some(19)),
        2 => (Some(33), Some(20)),
        3 => (Some(36), Some(21)),
        6 => (Some(37), Some(24)),
        7 => (Some(38), Some(25)),
        8 => (None, Some(26)),
        10 => (Some(61), Some(14)),
        11 => (Some(39), Some(28)),
        12 => (Some(64), Some(16)),
        13 => (Some(65), Some(17)),
        14 => (Some(15), Some(29)),
        15 => (None, None),
        _ => return None,
    })
}
/// A suspended native stack-local search, retained only across diagnostic slices.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Query {
    pub depth: usize,
    pub caller: ObjectId,
    pub reference: ObjectId,
    pub mode: i16,
    pub flags: u8,
    pub room: Option<i16>,
    pub order: Vec<ObjectId>,
    pub cursor: usize,
    pub pending: Option<ObjectId>,
    pub best: Option<ObjectId>,
    pub best_score: f32,
}
impl Query {
    pub(crate) fn validate(&self, vm: &Vm, p: &Program, t: &Thread) -> Result<(), String> {
        let q = self;
        let parent = q
            .depth
            .checked_sub(1)
            .and_then(|n| t.stack.get(n))
            .ok_or("Find Best parent missing")?;
        let (_, routine) = p.resolve(&parent.code_owner, parent.location.bhav)?;
        let ins = routine
            .body
            .instructions
            .get(parent.location.instruction)
            .ok_or("Find Best parent instruction missing")?;
        let (_, function) = selector(q.mode).ok_or("Find Best invalid saved selector")?;
        let target = q.pending.ok_or("Find Best pending candidate missing")?;
        if ins.opcode != 14
            || i16::from_le_bytes([ins.operands[0], ins.operands[1]]) != q.mode
            || ins.operands[6] != q.flags
            || parent.caller != q.caller
            || q.cursor == 0
            || q.cursor > q.order.len()
            || q.order[q.cursor - 1] != target
            || !q.best_score.is_finite()
            || q.best_score < 0.
            || q.best.is_some_and(|id| !q.order[..q.cursor].contains(&id))
            || !vm.world.objects.contains_key(&q.reference)
            || vm
                .world
                .person
                .as_ref()
                .is_none_or(|w| w.object_order != q.order)
        {
            return Err("invalid Find Best continuation".into());
        }
        let d = vm
            .world
            .objects
            .get(&target)
            .and_then(|o| o.definition.as_ref())
            .ok_or("Find Best candidate resource missing")?;
        let entry = function
            .and_then(|i| {
                p.modules
                    .get(&d.source)?
                    .object_functions
                    .get(&d.object)?
                    .get(i)
            })
            .ok_or("Find Best function missing")?;
        if entry.condition == 0 {
            return Err("Find Best continuation lacks condition".into());
        }
        if t.callback
            .iter()
            .flat_map(|b| b.chain())
            .any(|b| b.base_depth == q.depth)
        {
            let child = &t.stack[q.depth];
            if child.code_owner != d.source
                || child.location.bhav != entry.condition
                || child.caller != q.caller
                || child.callee != target
            {
                return Err("Find Best callback binding mismatch".into());
            }
        } else if t.stack.len() != q.depth || t.callback_result.is_none() {
            return Err("Find Best callback result missing".into());
        }
        Ok(())
    }
}
impl Vm {
    pub(crate) fn find_best_function(
        &mut self,
        p: &Program,
        t: &mut Thread,
        ins: &sims_formats::behavior::Instruction,
    ) -> Result<String, String> {
        let b = &ins.operands;
        let mode = i16::from_le_bytes([b[0], b[1]]);
        let Some((score_field, function)) = selector(mode) else {
            self.branch(p, t, ins.on_false)?;
            return Ok(format!(
                "Find Best unsupported native selector {mode} → false"
            ));
        };
        let caller = t.stack.last().unwrap().caller;
        if mode == 8
            && self.world.objects[&caller]
                .person
                .as_ref()
                .is_none_or(|person| person.get(&65).is_none_or(|v| *v > 1))
        {
            self.branch(p, t, ins.on_false)?;
            return Ok("Find Best excludes non-human caller".into());
        }
        let mut q = if t.find_best.last().is_some_and(|q| q.depth == t.stack.len()) {
            t.find_best.pop().unwrap()
        } else {
            let flags = b[6];
            let reference = if flags & 8 != 0 {
                let id = self.read(p, t, b[4], i16::from_le_bytes([b[2], b[3]]))?;
                if !self.world.objects.contains_key(&id) {
                    return Err("Find Best missing reference object (native error 0x17)".into());
                }
                id
            } else {
                caller
            };
            let room = if flags & 4 != 0 {
                Some(
                    *self.world.objects[&reference]
                        .data
                        .get(&29)
                        .ok_or("Find Best reference room unavailable")?,
                )
            } else {
                None
            };
            if flags & 3 != 0 {
                return Err("Find Best indoor/outdoor room classification unavailable".into());
            }
            let order = self
                .world
                .person
                .as_ref()
                .ok_or("Find Best native object registry unavailable")?
                .object_order
                .clone();
            Query {
                depth: t.stack.len(),
                caller,
                reference,
                mode,
                flags,
                room,
                order,
                cursor: 0,
                pending: None,
                best: None,
                best_score: 0.,
            }
        };
        if let Some(candidate) = q.pending.take() {
            let accepted = t
                .callback_result
                .take()
                .ok_or("Find Best callback did not return")?;
            if accepted && self.world.objects[&caller].data[&66] != 0 {
                self.rank_function(p, &mut q, candidate)?;
            }
        }
        while let Some(&candidate) = q.order.get(q.cursor) {
            q.cursor += 1;
            let object = self
                .world
                .objects
                .get(&candidate)
                .ok_or("Find Best candidate disappeared during check")?;
            let data = |field| {
                object
                    .data
                    .get(&field)
                    .copied()
                    .ok_or_else(|| format!("Find Best candidate field {field} unavailable"))
            };
            if data(25)? >= 1
                || data(8)? & 512 != 0
                || q.room
                    .is_some_and(|room| object.data.get(&29) != Some(&room))
            {
                continue;
            }
            let (score, binding) = if let Some(function) = function {
                let def = object
                    .definition
                    .as_ref()
                    .ok_or("Find Best candidate definition unavailable")?;
                let entry = p
                    .modules
                    .get(&def.source)
                    .and_then(|m| m.object_functions.get(&def.object))
                    .and_then(|f| f.get(function));
                let Some(entry) = entry.filter(|e| e.action != 0) else {
                    continue;
                };
                (
                    score_field.map_or(Ok(50), data)?,
                    (entry.condition != 0).then(|| (def.source.clone(), entry.condition)),
                )
            } else {
                if data(59)? != 4 {
                    continue;
                }
                (50, None)
            };
            *self
                .world
                .objects
                .get_mut(&caller)
                .unwrap()
                .data
                .get_mut(&66)
                .ok_or("Find Best score storage unavailable")? = score;
            if score_field.is_some() && score == 0 {
                continue;
            }
            if let Some((owner, bhav)) = binding {
                self.push_callback_start(
                    p,
                    t,
                    crate::Start {
                        owner: &owner,
                        bhav,
                        caller,
                        callee: candidate,
                        stack_object: candidate,
                        args: self.world.objects[&caller].temps.to_vec(),
                        check: true,
                    },
                )?;
                q.pending = Some(candidate);
                t.find_best.push(q);
                return Ok(format!(
                    "Find Best entered original eligibility callback on #{candidate}"
                ));
            }
            self.rank_function(p, &mut q, candidate)?;
        }
        let success = q.best.is_some();
        if let Some(id) = q.best {
            t.stack.last_mut().unwrap().stack_object = id;
        }
        t.last_result = success;
        self.branch(p, t, if success { ins.on_true } else { ins.on_false })?;
        Ok(format!(
            "Find Best function selector {mode}: {:?}, score {}",
            q.best, q.best_score
        ))
    }
    fn rank_function(&self, p: &Program, q: &mut Query, candidate: ObjectId) -> Result<(), String> {
        let score = self.world.objects[&q.caller].data[&66];
        let def = self.world.objects[&candidate]
            .definition
            .as_ref()
            .ok_or("Find Best distance resource unavailable")?;
        let kind = crate::construction::definition_words(p, def)?[9];
        let positions = &self
            .world
            .person
            .as_ref()
            .unwrap()
            .autonomy
            .as_ref()
            .ok_or("Find Best positions unavailable")?
            .placements;
        let mut distance = if kind == 7 {
            0.
        } else {
            let a = positions
                .get(&q.reference)
                .ok_or("Find Best reference position missing")?;
            let b = positions
                .get(&candidate)
                .ok_or("Find Best candidate position missing")?;
            let dx = a.x.wrapping_sub(b.x);
            let dy = a.y.wrapping_sub(b.y);
            let dz = (i32::from(b.level) - i32::from(a.level)).wrapping_mul(20);
            (f64::from(dz) * f64::from(dz)
                + f64::from(dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)))
                    * f64::from(0.00390625f32))
            .sqrt()
        };
        if q.mode == 11 && distance > 1. {
            distance *= 0.5;
        }
        //44f5e2 reads the initialized process coefficient for each eligible
        //candidate; it is not an immutable code literal or a fresh FCNS lookup.
        let attenuation = crate::autonomy::process_float(
            &self.world,
            crate::autonomy_constants::Field::FunctionalDistance,
        )?;
        let value = (f64::from(score) - f64::from(attenuation) * distance).max(1.);
        if f64::from(q.best_score) < value {
            q.best_score = value as f32;
            q.best = Some(candidate);
        }
        Ok(())
    }
}
