//!440b2d registration/postconstruction prefix, before the first group Reset.
use super::{Definition, Factory, ObjectDate, ObjectId, Program, Vm};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    source: String,
    catalog: u16,
    now: ObjectDate,
    time: [i16; 2],
    parts: Vec<ObjectId>,
    pending: Option<ObjectId>,
}
impl Construction {
    pub fn new(
        p: &Program,
        source: &str,
        catalog: u16,
        now: ObjectDate,
        time: [i16; 2],
    ) -> Result<Self, String> {
        let module = p
            .modules
            .get(source)
            .ok_or("group construction module absent")?;
        crate::multitile::construction_plan(module, catalog)?;
        Ok(Self {
            source: source.into(),
            catalog,
            now,
            time,
            parts: vec![],
            pending: None,
        })
    }
    pub fn resource(&self) -> (&str, u16) {
        (&self.source, self.catalog)
    }
    pub fn parts(&self) -> &[ObjectId] {
        &self.parts
    }
    pub fn pending(&self) -> Option<ObjectId> {
        self.pending
    }
    pub fn register_next(&mut self, vm: &mut Vm, p: &Program) -> Result<Option<ObjectId>, String> {
        self.validate(vm, p)?;
        if self.pending.is_some() {
            return Err("group graphic has not returned".into());
        }
        let module = &p.modules[&self.source];
        let plan = crate::multitile::construction_plan(module, self.catalog)?;
        let Some(&number) = plan.get(self.parts.len()) else {
            return Ok(None);
        };
        //Each registration chooses the current free ID. Do not reserve IDs
        //across an externally visible postconstruction service.
        let id = (1..=i16::MAX)
            .find(|id| !vm.world.objects.contains_key(id))
            .ok_or("group factory registry exhaustion cleanup is unsupported")?;
        let w = &module.object_definition_words[&number];
        vm.begin_group_factory_part(
            p,
            id,
            Definition {
                source: self.source.clone(),
                object: number,
                guid: w[14] as u16 as u32 | ((w[15] as u16 as u32) << 16),
                slots: w[20] as u16,
            },
            self.parts.first().copied(),
        )?;
        self.parts.push(id);
        self.pending = Some(id);
        Ok(Some(id))
    }
    pub fn finish_part(&mut self, vm: &mut Vm, p: &Program, id: ObjectId) -> Result<(), String> {
        self.validate(vm, p)?;
        if self.pending != Some(id) {
            return Err("stale group postconstruction reply".into());
        }
        vm.finish_group_factory_part(p, id, self.now, self.time)?;
        self.pending = None;
        Ok(())
    }
    pub fn finish(&self, vm: &mut Vm, p: &Program) -> Result<Factory, String> {
        self.validate(vm, p)?;
        if self.pending.is_some() {
            return Err("group Reset precedes graphic return".into());
        }
        Factory::registered(
            vm,
            p,
            &self.source,
            self.catalog,
            self.parts.clone(),
            self.now,
        )
    }
    pub fn validate(&self, vm: &Vm, p: &Program) -> Result<(), String> {
        let module = p
            .modules
            .get(&self.source)
            .ok_or("group construction source absent")?;
        let plan = crate::multitile::construction_plan(module, self.catalog)?;
        if self.parts.len() > plan.len()
            || self.pending.is_some() && self.pending != self.parts.last().copied()
            || !self.parts.is_empty()
                && crate::multitile::chain(&vm.world, self.parts[0])? != self.parts
        {
            return Err("group construction prefix differs".into());
        }
        for (&id, &number) in self.parts.iter().zip(&plan) {
            let o = vm
                .world
                .objects
                .get(&id)
                .ok_or("group construction part absent")?;
            let w = &module.object_definition_words[&number];
            let pending = self.pending == Some(id);
            let date = if pending {
                [0; 5]
            } else {
                [
                    self.now.year,
                    self.now.month,
                    self.now.day,
                    self.time[0],
                    self.time[1],
                ]
            };
            let origin = if pending {
                [0; 3]
            } else {
                crate::multitile::origin(w)?
            };
            if id <= 0
                || o.definition.as_ref().is_none_or(|d| {
                    d.source != self.source
                        || d.object != number
                        || d.guid != (w[14] as u16 as u32 | ((w[15] as u16 as u32) << 16))
                        || d.slots != w[20] as u16
                })
                || vm.object_threads.contains_key(&id)
                || vm.lifecycles.contains_key(&id)
                || o.person.is_some()
                || o.cursor.is_some()
                || o.container.is_some()
                || o.data.get(&11) != Some(&id)
                || o.data.get(&29) != Some(&-5)
                || vm
                    .world
                    .positions
                    .get(&id)
                    .is_none_or(|q| (q.x, q.y, q.level) != (-16, -16, 1))
                || [45, 46, 47, 24, 21]
                    .into_iter()
                    .zip(date)
                    .any(|(i, v)| o.data.get(&i) != Some(&v))
                || o.multitile
                    .as_ref()
                    .is_none_or(|m| m.origin != origin || m.offset != [0; 3])
            {
                return Err("group construction ran ahead of its postconstructor".into());
            }
        }
        Ok(())
    }
}
