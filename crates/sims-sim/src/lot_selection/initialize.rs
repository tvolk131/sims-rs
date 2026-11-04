//!445e4a owns the actual440b2d factories, including their graphic/E2 boundaries.
use super::*;
use sims_vm::load_indicators::{GUIDS, Initialization, Reply, Request as Call};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct State {
    call: Initialization,
    resources: [Option<sims_vm::Definition>; 2],
}
impl State {
    pub(super) fn validate(&self, e: &Execution, vm: &Vm, p: &Program) -> Result<(), String> {
        self.call.validate()?;
        if e.factory_root.is_some()
            || e.direct_root.is_some()
            || e.release_root.is_some()
            || e.refresh.is_some()
            || e.flags.is_some()
            || e.indicators.is_some()
            || e.release.is_some()
            || e.refill.is_some()
            || e.star_resource.is_some()
            || e.placement.is_some()
            || e.selection != selection::Selection::new(None)
        {
            return Err("indicator initializer has an unrelated continuation".into());
        }
        for (i, d) in self.resources.iter().enumerate() {
            if (self.call.available()[i] == Some(true)) != d.is_some() {
                return Err("indicator initializer lookup binding differs".into());
            }
            if let Some(d) = d {
                validate_resource(p, i, d)?;
            }
        }
        if let Some(f) = &e.factory {
            let Some(Call::Create(i)) = self.call.pending() else {
                return Err("indicator factory caller absent".into());
            };
            if self.resources[i].is_none()
                || f.object().is_some_and(|id| {
                    vm.world
                        .objects
                        .get(&id)
                        .is_none_or(|o| o.definition != self.resources[i])
                })
            {
                return Err("indicator factory retained resource differs".into());
            }
        }
        match e.external {
            None => (),
            Some(Request::IndicatorResource(i))
                if self.call.pending() == Some(Call::Lookup(i)) && !e.started =>
            {
                ()
            }
            Some(Request::FactoryGraphic(id))
                if e.factory
                    .as_ref()
                    .and_then(crate::lot_creation::OffLot::pending_graphic)
                    == Some(id) =>
            {
                ()
            }
            Some(Request::CreationNotification(id))
                if e.factory
                    .as_ref()
                    .and_then(crate::lot_creation::OffLot::pending_created)
                    == Some(id) =>
            {
                ()
            }
            _ => return Err("indicator initializer external service differs".into()),
        }
        Ok(())
    }
    pub(super) fn finish_factory(
        &mut self,
        sim: &mut Simulation,
        f: &crate::lot_creation::OffLot,
    ) -> Result<(), String> {
        let request = self
            .call
            .pending()
            .ok_or("indicator factory caller absent")?;
        let id = f
            .object()
            .filter(|id| sim.vm.world.objects.contains_key(id));
        self.call.acknowledge(
            &mut sim.native_lot.as_mut().unwrap().indicators,
            request,
            Reply::Created(id),
        )
    }
}
fn validate_resource(p: &Program, i: usize, d: &sims_vm::Definition) -> Result<(), String> {
    let w = sims_vm::construction::definition_words(p, d)?;
    if GUIDS.get(i) != Some(&d.guid)
        || w[20] as u16 != d.slots
        || (u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16)) != d.guid
    {
        return Err("indicator lookup resource differs".into());
    }
    Ok(())
}
impl Execution {
    pub(super) fn advance_initialization(
        &mut self,
        sim: &mut Simulation,
        p: &Program,
    ) -> Result<bool, String> {
        let init = self.initialization.as_mut().unwrap();
        let native = sim
            .native_lot
            .as_mut()
            .ok_or("indicator initialization lot absent")?;
        match init.call.advance(&native.indicators)? {
            None => return Ok(true),
            Some(Call::Lookup(i)) => self.external = Some(Request::IndicatorResource(i)),
            Some(Call::Create(i)) => {
                let definition = init.resources[i]
                    .clone()
                    .ok_or("indicator resource absent")?;
                let g = &sim.vm.world.globals;
                let read = |i| {
                    g.get(&i)
                        .copied()
                        .ok_or("indicator factory calendar absent")
                };
                let now = sims_vm::construction::ObjectDate {
                    year: read(8)?,
                    month: read(7)?,
                    day: read(1)?,
                };
                let time = [read(0)?, read(5)?];
                let factory = crate::lot_creation::OffLot::begin_definition_with_graphics(
                    &mut sim.vm,
                    p,
                    &mut native.placement.links,
                    definition,
                    now,
                    time,
                )?;
                if let Some(id) = factory.object() {
                    native
                        .placement
                        .postconstruct_bounds(&sim.vm.world, p, id)?;
                    sim.unplaced.insert(id);
                }
                self.factory = Some(factory);
            }
            Some(request @ Call::Hide(id)) => {
                *sim.vm
                    .world
                    .objects
                    .get_mut(&id)
                    .and_then(|o| o.data.get_mut(&8))
                    .ok_or("indicator hidden field absent")? |= 0x4000;
                init.call
                    .acknowledge(&mut native.indicators, request, Reply::Done)?;
            }
        }
        Ok(false)
    }
}
impl Simulation {
    pub fn complete_indicator_resource(
        &mut self,
        p: &Program,
        slot: usize,
        definition: Option<sims_vm::Definition>,
    ) -> Result<(), String> {
        let e = self
            .native_selection
            .as_mut()
            .ok_or("indicator initialization absent")?;
        if e.external != Some(Request::IndicatorResource(slot)) || e.started {
            return Err("stale indicator resource reply".into());
        }
        if let Some(d) = &definition {
            validate_resource(p, slot, d)?;
        }
        let init = e
            .initialization
            .as_mut()
            .ok_or("indicator initialization absent")?;
        init.call.acknowledge(
            &mut self.native_lot.as_mut().unwrap().indicators,
            Call::Lookup(slot),
            Reply::Available(definition.is_some()),
        )?;
        init.resources[slot] = definition;
        e.external = None;
        Ok(())
    }
    pub fn start_indicator_created(&mut self, id: ObjectId) -> Result<(), String> {
        let e = self
            .native_selection
            .as_mut()
            .ok_or("indicator initialization absent")?;
        if !e.owns_creation_notifications()
            || e.external != Some(Request::CreationNotification(id))
            || e.started
        {
            return Err("stale indicator creation notification start".into());
        }
        e.started = true;
        Ok(())
    }
    pub fn complete_indicator_created(&mut self, id: ObjectId) -> Result<(), String> {
        let e = self
            .native_selection
            .as_mut()
            .ok_or("indicator initialization absent")?;
        if !e.owns_creation_notifications()
            || e.external != Some(Request::CreationNotification(id))
            || !e.started
        {
            return Err("stale indicator creation notification return".into());
        }
        e.external = None;
        e.started = false;
        Ok(())
    }
}
