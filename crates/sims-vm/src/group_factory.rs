//! Native 0x440b2d multipart branch, staged around original Init/Main updates.
//! Allocation/registration failures and membership mutation during callbacks
//! remain explicit boundaries. Group resource selection and Reset are shared.
use crate::{Definition, ObjectId, Program, Status, Vm, construction::ObjectDate};
use serde::{Deserialize, Serialize};
mod construction;
pub use construction::Construction;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Factory {
    pub source: String,
    pub catalog: u16,
    pub parts: Vec<ObjectId>,
    cursor: usize,
    begun: bool,
}
impl Factory {
    pub fn begin(
        vm: &mut Vm,
        p: &Program,
        source: &str,
        catalog: u16,
        now: ObjectDate,
        time: [i16; 2],
    ) -> Result<Self, String> {
        let module = p.modules.get(source).ok_or("group factory module absent")?;
        let plan = crate::multitile::construction_plan(module, catalog)?;
        let ids: Vec<_> = (1..=i16::MAX)
            .filter(|id| !vm.world.objects.contains_key(id))
            .take(plan.len())
            .collect();
        if ids.len() != plan.len() {
            return Err("group factory registry exhaustion cleanup is unsupported".into());
        }
        let before = vm.clone();
        let result = (|| {
            for (i, (&id, &number)) in ids.iter().zip(&plan).enumerate() {
                let w = &module.object_definition_words[&number];
                vm.register_object_factory(
                    p,
                    id,
                    Definition {
                        source: source.into(),
                        object: number,
                        guid: w[14] as u16 as u32 | ((w[15] as u16 as u32) << 16),
                        slots: w[20] as u16,
                    },
                    true,
                    (i != 0).then_some(ids[0]),
                    now,
                    time,
                )?;
            }
            Self::registered(vm, p, source, catalog, ids, now)
        })();
        if result.is_err() {
            *vm = before;
        }
        result
    }
    ///Shared postconstruction handoff. All parts have already returned from
    ///their graphic service and been dated before the catalog price and Reset.
    fn registered(
        vm: &mut Vm,
        p: &Program,
        source: &str,
        catalog: u16,
        parts: Vec<ObjectId>,
        now: ObjectDate,
    ) -> Result<Self, String> {
        let module = p.modules.get(source).ok_or("group factory module absent")?;
        let plan = crate::multitile::construction_plan(module, catalog)?;
        if parts.is_empty()
            || parts.len() != plan.len()
            || crate::multitile::chain(&vm.world, parts[0])? != parts
            || parts.iter().zip(plan).any(|(id, number)| {
                vm.object_threads.contains_key(id)
                    || vm.lifecycles.contains_key(id)
                    || vm
                        .world
                        .objects
                        .get(id)
                        .and_then(|o| o.definition.as_ref())
                        .is_none_or(|d| d.source != source || d.object != number)
            })
        {
            return Err("group Reset requires its fully registered resource chain".into());
        }
        //No later per-part price store overwrites the original Init's writes.
        vm.world
            .objects
            .get_mut(&parts[0])
            .unwrap()
            .data
            .insert(41, module.object_definition_words[&catalog][18]);
        vm.prepare_multitile_reset(p, parts[0], now)?;
        Ok(Self {
            source: source.into(),
            catalog,
            parts,
            cursor: 0,
            begun: false,
        })
    }
    pub fn head(&self) -> ObjectId {
        self.parts[0]
    }
    pub fn complete(&self) -> bool {
        self.cursor == self.parts.len()
    }
    pub fn current(&mut self, vm: &mut Vm, p: &Program) -> Result<Option<ObjectId>, String> {
        if self.complete() {
            return Ok(None);
        }
        let id = self.parts[self.cursor];
        if !self.begun {
            vm.begin_multitile_init(p, id)?;
            self.begun = true;
        }
        Ok(Some(id))
    }
    pub fn finish_current(&mut self, vm: &mut Vm, p: &Program) -> Result<(), String> {
        if !self.begun || self.complete() {
            return Err("group update was not started".into());
        }
        let id = self.parts[self.cursor];
        let t = &vm.threads[&vm.object_threads[&id]];
        if !matches!(t.status, Status::Sleeping { .. } | Status::Returned(_))
            || t.callback.is_some()
        {
            return Err("group Init/Main has not completed its update".into());
        }
        if crate::multitile::chain(&vm.world, self.head())? != self.parts {
            return Err("group factory membership mutation requires native cleanup".into());
        }
        vm.finish_object_update(id, true)?;
        vm.refresh_registry_flags(p, id)?;
        vm.finish_factory_object_visit(id)?;
        self.cursor += 1;
        self.begun = false;
        if self.complete() {
            for id in &self.parts {
                vm.world.objects.get_mut(id).unwrap().activate_native()?;
                vm.world
                    .objects
                    .get_mut(id)
                    .unwrap()
                    .idle
                    .as_mut()
                    .unwrap()
                    .set_flags(1, true);
            }
        }
        Ok(())
    }
    pub fn validate(&self, vm: &Vm, p: &Program) -> Result<(), String> {
        if self.parts.is_empty()
            || self.cursor > self.parts.len()
            || (self.complete() && self.begun)
        {
            return Err("invalid group factory cursor".into());
        }
        let module = p
            .modules
            .get(&self.source)
            .ok_or("group factory source absent")?;
        let plan = crate::multitile::construction_plan(module, self.catalog)?;
        if plan.len() != self.parts.len()
            || crate::multitile::chain(&vm.world, self.head())? != self.parts
        {
            return Err("group factory layout mismatch".into());
        }
        for (i, (&id, &number)) in self.parts.iter().zip(&plan).enumerate() {
            let o = &vm.world.objects[&id];
            let d = o
                .definition
                .as_ref()
                .ok_or("group part definition absent")?;
            let l = vm
                .lifecycles
                .get(&id)
                .ok_or("group part lifecycle absent")?;
            let t = vm
                .threads
                .get(&l.thread)
                .ok_or("group part thread absent")?;
            let started = i < self.cursor || i == self.cursor && self.begun;
            if d.source != self.source
                || d.object != number
                || l.run_init != started
                || !started
                    && (l.phase != crate::lifecycle::Phase::Main || t.status != Status::Ready)
                || o.idle
                    .as_ref()
                    .is_none_or(|idle| (idle.flags & 1 != 0) != self.complete())
            {
                return Err("invalid group factory part lifecycle".into());
            }
        }
        Ok(())
    }
}
