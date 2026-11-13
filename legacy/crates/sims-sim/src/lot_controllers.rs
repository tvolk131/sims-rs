//!40726a's comeandsee reconciliation. Factory and SetObject retain their real
//!continuations; the traversal rereads the live registry after every callback.
use crate::{Simulation, Step, StepStop, lot_links::Descriptor, lot_placement};
use serde::{Deserialize, Serialize};
use sims_vm::{Definition, Object, ObjectId, Program, ThreadId, Vm};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Scan,
    Factory,
    Place,
    Placement,
    Next,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    tick: u64,
    definition: Option<Definition>,
    current: Option<(ObjectId, u32)>,
    helper: Option<(ObjectId, u32)>,
    phase: Phase,
    placement: Option<lot_placement::live::Execution>,
}

///473574 uses a 1900 epoch and thirty days for every month. Invalid year/month
///returns zero; the day is deliberately not range checked by the native helper.
pub fn day_number(year: i32, month: i32, day: i32) -> i32 {
    if year < 1900 || !(1..=12).contains(&month) {
        return 0;
    }
    year.wrapping_sub(1900)
        .wrapping_mul(360)
        .wrapping_add(month.wrapping_sub(1).wrapping_mul(30))
        .wrapping_add(day.wrapping_sub(1))
}

///43520d. Clock/object words are signed shorts, promoted before the arithmetic.
pub fn elapsed_minutes(clock: &crate::clock::LotClock, object: &Object) -> Result<i32, String> {
    let word = |n| {
        object
            .data
            .get(&n)
            .copied()
            .map(i32::from)
            .ok_or("controller object date absent")
    };
    let mut minutes = i32::from(clock.minute).wrapping_sub(word(21)?);
    let mut hours = i32::from(clock.hour).wrapping_sub(word(24)?);
    let mut days = day_number(clock.year.into(), clock.month.into(), clock.day.into())
        .wrapping_sub(day_number(word(45)?, word(46)?, word(47)?));
    if minutes < 0 {
        minutes = minutes.wrapping_add(60);
        hours = hours.wrapping_sub(1);
    }
    if hours < 0 {
        hours = hours.wrapping_add(24);
        days = days.wrapping_sub(1);
    }
    Ok(days
        .wrapping_mul(24)
        .wrapping_add(hours)
        .wrapping_mul(60)
        .wrapping_add(minutes))
}

pub fn excluded_lot(lot: i16, zoning: i32) -> bool {
    (21..31).contains(&lot) || (81..90).contains(&lot) || (40..50).contains(&lot) || zoning == 2
}

///4074df tests relationships in their retained insertion order, including
///missing objects and nonmatching definitions before the first positive row.
pub fn existing(
    vm: &Vm,
    owner: ObjectId,
    definition: &Definition,
) -> Result<Option<ObjectId>, String> {
    let object = vm
        .world
        .objects
        .get(&owner)
        .ok_or("controller owner absent")?;
    for &id in object.relationship_keys()?.iter() {
        if vm
            .world
            .objects
            .get(&id)
            .and_then(|o| o.definition.as_ref())
            .is_some_and(|d| d.source == definition.source && d.object == definition.object)
            && object.relationships[&id]
                .first()
                .is_some_and(|value| *value > 0)
        {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

///4071a3 averages all multipart members, using signed wrapping sums and
///division toward zero. A selected secondary still starts at its group head.
pub fn center(vm: &Vm, owner: ObjectId) -> Result<[i32; 2], String> {
    let ids = if vm
        .world
        .objects
        .get(&owner)
        .ok_or("controller owner absent")?
        .multitile
        .is_some()
    {
        sims_vm::multitile::chain(&vm.world, owner)?
    } else {
        vec![owner]
    };
    let mut sum = [0i32; 2];
    for id in &ids {
        let p = vm
            .world
            .positions
            .get(id)
            .ok_or("controller group position absent")?;
        sum[0] = sum[0].wrapping_add(p.x);
        sum[1] = sum[1].wrapping_add(p.y);
    }
    let count = i32::try_from(ids.len()).map_err(|_| "controller group count overflow")?;
    if count == 0 {
        return Err("controller group has no members".into());
    }
    Ok(sum.map(|v| v / count))
}

impl Execution {
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub fn factory_pending(&self) -> bool {
        self.phase == Phase::Factory
    }
    pub fn pending_graphic(&self) -> Option<ObjectId> {
        match self.placement.as_ref()?.pending {
            Some(lot_placement::Request::RefreshAnimator(id)) => Some(id),
            _ => None,
        }
    }
    pub(crate) fn owns_callback(
        &self,
        vm: &Vm,
        p: &Program,
        thread: ThreadId,
        depth: usize,
    ) -> bool {
        self.placement
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        if self.tick != s.vm.tick || s.native_lot.is_none() {
            return Err("controller reconciliation crossed its lot boundary".into());
        }
        if let Some(definition) = &self.definition {
            sims_vm::construction::definition_words(p, definition)?;
        } else if self.phase != Phase::Complete {
            return Err("controller reconciliation lost its resource".into());
        }
        if !matches!(self.phase, Phase::Scan | Phase::Complete) && self.current.is_none() {
            return Err("controller reconciliation lost its current object".into());
        }
        if matches!(self.phase, Phase::Place | Phase::Placement) && self.helper.is_none() {
            return Err("controller reconciliation lost its helper".into());
        }
        for (id, handle) in self.current.iter().chain(self.helper.iter()) {
            if s.vm
                .world
                .scheduler_storage
                .as_ref()
                .and_then(|r| r.handle(*id))
                != Some(*handle)
            {
                return Err("controller reconciliation object lifetime changed".into());
            }
        }
        if let Some((id, _)) = self.helper {
            let actual = s.vm.world.objects[&id].definition.as_ref();
            if actual
                .zip(self.definition.as_ref())
                .is_none_or(|(a, b)| a.source != b.source || a.object != b.object)
            {
                return Err("controller helper resource changed".into());
            }
        }
        if self.phase == Phase::Factory {
            if s.native_selection.as_ref().and_then(|e| e.direct_factory())
                != self.definition.as_ref()
            {
                return Err("controller factory lost its owner".into());
            }
        } else if s.native_selection.is_some() {
            return Err("controller overlaps another factory".into());
        }
        if let Some(e) = &self.placement {
            if self.phase != Phase::Placement {
                return Err("orphan controller placement".into());
            }
            e.validate(
                &s.vm,
                p,
                self.helper.ok_or("controller placement helper absent")?.0,
            )?;
        } else if self.phase == Phase::Placement {
            return Err("controller placement absent".into());
        }
        Ok(())
    }
}

impl Simulation {
    fn controller_lifetime(&self, id: ObjectId) -> Result<(ObjectId, u32), String> {
        Ok((
            id,
            self.vm
                .world
                .scheduler_storage
                .as_ref()
                .and_then(|r| r.handle(id))
                .ok_or("controller registry lifetime absent")?,
        ))
    }
    pub fn begin_controller_reconciliation(
        &mut self,
        p: &Program,
        definition: Option<Definition>,
    ) -> Result<(), String> {
        if self.controller_reconciliation.is_some()
            || self.native_selection.is_some()
            || self.ground_placement.is_some()
            || self.synchronous_removal.is_some()
            || self.object_pickup.is_some()
            || self.object_placement.is_some()
            || self.cursor_work_pending()
            || self.vm.player_menu.is_some()
            || !self.deletion.is_empty()
            || self.portrait_person.is_some()
            || self.vm.threads.values().any(|t| t.callback.is_some())
            || self.characters.values().any(|r| {
                r.refresh_tick.is_some() || r.ordinary_update.is_some() || r.character.busy()
            })
        {
            return Err("controller reconciliation overlaps synchronous work".into());
        }
        if let Some(d) = &definition {
            sims_vm::construction::definition_words(p, d)?;
        }
        let people = self
            .vm
            .world
            .person
            .as_ref()
            .ok_or("controller registry absent")?;
        let lot = *self
            .vm
            .world
            .globals
            .get(&10)
            .ok_or("controller lot number absent")?;
        let zoning = if lot == 0 {
            1
        } else {
            people.lot_zoning.get(&lot).copied().unwrap_or(0)
        };
        let current = people
            .object_order
            .first()
            .copied()
            .map(|id| self.controller_lifetime(id))
            .transpose()?;
        self.controller_reconciliation = Some(Execution {
            tick: self.vm.tick,
            definition: definition.clone(),
            current,
            helper: None,
            phase: if definition.is_none() || excluded_lot(lot, zoning) {
                Phase::Complete
            } else {
                Phase::Scan
            },
            placement: None,
        });
        Ok(())
    }
    pub fn advance_controller_reconciliation(
        &mut self,
        p: &Program,
        ordinary: &BTreeSet<ObjectId>,
        budget: usize,
    ) -> Result<(Step, bool), String> {
        if budget == 0 {
            return Err("controller reconciliation needs a work budget".into());
        }
        let e = self
            .controller_reconciliation
            .as_ref()
            .ok_or("controller reconciliation absent")?;
        e.validate(self, p)?;
        let mut e = self.controller_reconciliation.take().unwrap();
        let mut step = Step {
            stop: StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        let result = self.drive_controllers(p, ordinary, budget, &mut e, &mut step);
        self.controller_reconciliation = Some(e);
        result?;
        let e = self.controller_reconciliation.as_ref().unwrap();
        e.validate(self, p)?;
        Ok((step, e.complete()))
    }
    fn drive_controllers(
        &mut self,
        p: &Program,
        ordinary: &BTreeSet<ObjectId>,
        mut budget: usize,
        e: &mut Execution,
        step: &mut Step,
    ) -> Result<(), String> {
        while budget > 0 && !e.complete() {
            if !matches!(e.phase, Phase::Factory | Phase::Placement) {
                budget -= 1;
            }
            match e.phase {
                Phase::Scan => {
                    let Some((id, _)) = e.current else {
                        e.phase = Phase::Complete;
                        continue;
                    };
                    let o = self
                        .vm
                        .world
                        .objects
                        .get(&id)
                        .ok_or("controller candidate absent")?;
                    let d = o
                        .definition
                        .as_ref()
                        .ok_or("controller candidate definition absent")?;
                    if sims_vm::construction::definition_words(p, d)?[9] != 4 {
                        e.phase = Phase::Next;
                        continue;
                    }
                    let on_lot = self
                        .native_lot
                        .as_ref()
                        .unwrap()
                        .placement
                        .links
                        .capture(&mut self.vm.world, id)?
                        .on_lot;
                    let o = &self.vm.world.objects[&id];
                    if !on_lot
                        || *o.data.get(&34).ok_or("controller disable word absent")? != 0
                        || *o.data.get(&8).ok_or("controller flags absent")? as u16 & 0x4000 != 0
                        || o.multitile
                            .as_ref()
                            .is_some_and(|part| part.head.is_some_and(|head| head != id))
                    {
                        e.phase = Phase::Next;
                        continue;
                    }
                    e.helper = existing(&self.vm, id, e.definition.as_ref().unwrap())?
                        .map(|id| self.controller_lifetime(id))
                        .transpose()?;
                    if e.helper.is_some() {
                        e.phase = Phase::Place;
                        continue;
                    }
                    let clock = &self
                        .household
                        .as_ref()
                        .ok_or("controller clock absent")?
                        .clock;
                    let lot = self.vm.world.globals[&10];
                    let residential = sims_vm::zoning::is_residential(
                        &self.vm.world.person.as_ref().unwrap().lot_zoning,
                        lot,
                    );
                    if elapsed_minutes(clock, o)? == 0 && residential {
                        self.begin_native_object_factory(
                            p,
                            e.definition.as_ref().unwrap().clone(),
                        )?;
                        e.phase = Phase::Factory;
                    } else {
                        e.phase = Phase::Next;
                    }
                }
                Phase::Factory => {
                    let (child, result) =
                        self.advance_native_object_factory(p, ordinary, budget)?;
                    step.trace.extend(child.trace);
                    step.events.extend(child.events);
                    if let Some(result) = result {
                        e.helper = result.map(|id| self.controller_lifetime(id)).transpose()?;
                        if let Some((helper, _)) = e.helper {
                            self.vm
                                .world
                                .objects
                                .get_mut(&e.current.unwrap().0)
                                .ok_or("controller owner removed by factory")?
                                .insert_relationship(helper, vec![1]);
                            e.phase = Phase::Place;
                        } else {
                            e.phase = Phase::Next;
                        }
                    }
                    return Ok(());
                }
                Phase::Place => {
                    let owner = e.current.unwrap().0;
                    let helper = e.helper.unwrap().0;
                    let position = center(&self.vm, owner)?;
                    let level = self
                        .vm
                        .world
                        .positions
                        .get(&owner)
                        .ok_or("controller owner position absent")?
                        .level;
                    let on_lot = self
                        .native_lot
                        .as_ref()
                        .unwrap()
                        .placement
                        .links
                        .capture(&mut self.vm.world, helper)?
                        .on_lot;
                    let current = self
                        .vm
                        .world
                        .positions
                        .get(&helper)
                        .ok_or("controller helper position absent")?;
                    if !on_lot || position != [current.x, current.y] || level != current.level {
                        let op = lot_placement::VirtualOperation::set(
                            &self.vm.world,
                            &self.native_lot.as_ref().unwrap().placement.links,
                            helper,
                            Descriptor {
                                on_lot: true,
                                position,
                                level,
                                container: None,
                                slot: 0,
                            },
                            true,
                        )?;
                        e.placement = Some(lot_placement::live::Execution::new_virtual(op));
                        e.phase = Phase::Placement;
                    } else {
                        e.phase = Phase::Next;
                    }
                }
                Phase::Placement => {
                    if self.drive_live_placement_with_graphics(
                        p,
                        e.placement.as_mut().unwrap(),
                        &mut budget,
                        step,
                        ordinary,
                    )? {
                        e.placement = None;
                        e.phase = Phase::Next;
                    } else {
                        return Ok(());
                    }
                }
                Phase::Next => {
                    let order = &self
                        .vm
                        .world
                        .person
                        .as_ref()
                        .ok_or("controller registry absent")?
                        .object_order;
                    let index = order
                        .iter()
                        .position(|&id| Some(id) == e.current.map(|v| v.0))
                        .ok_or("controller owner unregistered during callback")?;
                    e.current = order
                        .get(index + 1)
                        .copied()
                        .map(|id| self.controller_lifetime(id))
                        .transpose()?;
                    e.helper = None;
                    e.phase = Phase::Scan;
                }
                Phase::Complete => (),
            }
        }
        Ok(())
    }
    pub fn finish_controller_reconciliation(&mut self, p: &Program) -> Result<(), String> {
        let e = self
            .controller_reconciliation
            .as_ref()
            .ok_or("controller reconciliation absent")?;
        e.validate(self, p)?;
        if !e.complete() {
            return Err("controller reconciliation has not returned".into());
        }
        self.controller_reconciliation = None;
        Ok(())
    }
}
