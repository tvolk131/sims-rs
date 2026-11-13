//! Native Run Functional Tree (0x44f681), sharing the original object-target
//! frame push with Run Tree By Name. Conditions are not checked by this primitive.
use crate::{Program, Thread, Vm};
use sims_formats::behavior::Instruction;
pub fn function(mode: u16, person_species: Option<i16>) -> Option<usize> {
    Some(match mode {
        0..=7 => mode as usize + 18,
        8 if person_species.is_some_and(|s| s < 2) => 26,
        9 => 27,
        10 => 14,
        11 => 28,
        12 => 16,
        13 => 17,
        14 => 29,
        _ => return None,
    })
}
impl Vm {
    pub(crate) fn run_functional_tree(
        &mut self,
        p: &Program,
        t: &mut Thread,
        ins: &Instruction,
    ) -> Result<String, String> {
        let f = t.stack.last().ok_or("missing functional tree frame")?;
        let target = f.stack_object;
        let obj = self
            .world
            .objects
            .get(&target)
            .ok_or("functional tree target missing (native error 0x15)")?;
        let def = obj
            .definition
            .as_ref()
            .ok_or("functional tree definition missing")?;
        let species = self.world.objects[&f.caller]
            .person
            .as_ref()
            .and_then(|v| v.get(&65))
            .copied();
        let binding = function(
            u16::from_le_bytes([ins.operands[0], ins.operands[1]]),
            species,
        )
        .and_then(|i| {
            p.modules
                .get(&def.source)?
                .object_functions
                .get(&def.object)?
                .get(i)
        })
        .filter(|e| e.action != 0)
        .map(|e| (def.source.clone(), e.action));
        if let Some((owner, bhav)) = binding {
            let actor = f.caller;
            self.push_target_tree(p, t, owner, bhav, target, 0)?;
            if ins.operands[2] & 1 != 0 && self.world.objects[&actor].person.is_some() {
                t.stack.last_mut().unwrap().action_icon = true;
                if let Some(a) = self
                    .world
                    .actions
                    .as_mut()
                    .and_then(|s| s.actors.get_mut(&actor))
                    .and_then(|s| s.active.as_mut())
                {
                    a.icon_owner = target;
                } else {
                    return Err(
                        "functional icon selection without a live action record unsupported".into(),
                    );
                }
            }
            Ok(format!("Run Functional Tree entered {bhav} on #{target}"))
        } else {
            t.last_result = false;
            self.branch(p, t, ins.on_false)?;
            Ok("Run Functional Tree unavailable".into())
        }
    }
}
