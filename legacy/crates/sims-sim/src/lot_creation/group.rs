//!Direct440b2d multipart owner. The resource catalog identity differs from
//!the first constructed part; no synthetic SimAntics parent is introduced.
use crate::{Simulation, Step, lot_creation::Request};
use serde::{Deserialize, Serialize};
use sims_vm::{
    Definition, ObjectId, Program, Vm,
    construction::ObjectDate,
    group_factory::{Construction, Factory},
};
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Work {
    Construct(Construction),
    Initialize(Factory),
    Created(Factory),
    Complete(Factory),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    definition: Definition,
    work: Work,
}
impl Group {
    pub fn new(
        p: &Program,
        definition: Definition,
        now: ObjectDate,
        time: [i16; 2],
    ) -> Result<Self, String> {
        let w = sims_vm::construction::definition_words(p, &definition)?;
        if w[10] == 0
            || definition.guid != (w[14] as u16 as u32 | ((w[15] as u16 as u32) << 16))
            || definition.slots != w[20] as u16
        {
            return Err("direct group factory requires its catalog definition".into());
        }
        let construction = Construction::new(p, &definition.source, definition.object, now, time)?;
        Ok(Self {
            definition,
            work: Work::Construct(construction),
        })
    }
    pub fn definition(&self) -> &Definition {
        &self.definition
    }
    pub(crate) fn construction(&self) -> Option<&Construction> {
        match &self.work {
            Work::Construct(c) => Some(c),
            _ => None,
        }
    }
    pub fn parts(&self) -> &[ObjectId] {
        match &self.work {
            Work::Construct(c) => c.parts(),
            Work::Initialize(f) | Work::Created(f) | Work::Complete(f) => &f.parts,
        }
    }
    pub fn constructing(&self) -> &[ObjectId] {
        match &self.work {
            Work::Construct(c) => c.parts(),
            _ => &[],
        }
    }
    pub fn object(&self) -> Option<ObjectId> {
        self.parts().first().copied()
    }
    pub fn pending_graphic(&self) -> Option<ObjectId> {
        match &self.work {
            Work::Construct(c) => c.pending(),
            _ => None,
        }
    }
    pub fn pending_created(&self) -> Option<ObjectId> {
        match &self.work {
            Work::Created(f) => Some(f.head()),
            _ => None,
        }
    }
    pub fn complete_graphic(
        &mut self,
        vm: &mut Vm,
        p: &Program,
        id: ObjectId,
    ) -> Result<(), String> {
        let Work::Construct(c) = &mut self.work else {
            return Err("group graphic lost construction owner".into());
        };
        c.finish_part(vm, p, id)
    }
    pub fn validate(&self, vm: &Vm, p: &Program) -> Result<(), String> {
        let w = sims_vm::construction::definition_words(p, &self.definition)?;
        if w[10] == 0
            || self.definition.guid != (w[14] as u16 as u32 | ((w[15] as u16 as u32) << 16))
            || self.definition.slots != w[20] as u16
        {
            return Err("group factory catalog identity differs".into());
        }
        match &self.work {
            Work::Construct(c) => {
                c.validate(vm, p)?;
                if c.resource() != (self.definition.source.as_str(), self.definition.object)
                    || c.pending().is_some_and(|id| {
                        vm.world.objects[&id]
                            .definition
                            .as_ref()
                            .and_then(|d| sims_vm::construction::definition_words(p, d).ok())
                            .is_none_or(|w| w.get(102).is_none_or(|w| w & 1 == 0))
                    })
                {
                    return Err("group construction lost catalog or graphic boundary".into());
                }
            }
            Work::Initialize(f) | Work::Created(f) | Work::Complete(f) => {
                f.validate(vm, p)?;
                if f.source != self.definition.source
                    || f.catalog != self.definition.object
                    || !matches!(self.work, Work::Initialize(_)) && !f.complete()
                {
                    return Err("group factory lost its initialization caller".into());
                }
            }
        }
        Ok(())
    }
    pub(crate) fn advance(
        &mut self,
        s: &mut Simulation,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Option<Request>, String> {
        loop {
            if *remaining == 0 {
                return Ok(Some(Request::Budget));
            }
            match &mut self.work {
                Work::Construct(c) => {
                    if let Some(id) = c.pending() {
                        return Ok(Some(Request::Graphic(id)));
                    }
                    if let Some(id) = c.register_next(&mut s.vm, p)? {
                        let native = s
                            .native_lot
                            .as_mut()
                            .ok_or("group factory native lot absent")?;
                        native.placement.links.next.insert(id, 0);
                        native.placement.postconstruct_bounds(&s.vm.world, p, id)?;
                        s.unplaced.insert(id);
                        *remaining -= 1;
                        let w = sims_vm::construction::definition_words(
                            p,
                            s.vm.world.objects[&id].definition.as_ref().unwrap(),
                        )?;
                        if w.get(102).is_some_and(|w| w & 1 != 0) {
                            return Ok(Some(Request::Graphic(id)));
                        }
                        c.finish_part(&mut s.vm, p, id)?;
                    } else {
                        let f = c.finish(&mut s.vm, p)?;
                        self.work = Work::Initialize(f);
                        *remaining -= 1;
                    }
                }
                Work::Initialize(f) => {
                    if let Some(id) = f.current(&mut s.vm, p)? {
                        if !s.drive_factory_update(p, id, remaining, step)? {
                            return Ok(Some(Request::Budget));
                        }
                        f.finish_current(&mut s.vm, p)?;
                    } else {
                        let id = f.head();
                        self.work = Work::Created(f.clone());
                        return Ok(Some(Request::Created(id)));
                    }
                }
                Work::Created(f) => {
                    self.work = Work::Complete(f.clone());
                    return Ok(None);
                }
                Work::Complete(_) => return Ok(None),
            }
        }
    }
}
