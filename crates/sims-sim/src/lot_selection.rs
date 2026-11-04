//! Native registry selection with owned physical callbacks. Asset services stay
//! pending until the live host completes them; the household scheduler is frozen.
use crate::{
    Simulation, Step,
    lot_links::{Descriptor, Links},
    lot_placement::{self, VirtualOperation},
};
use serde::{Deserialize, Serialize};
use sims_vm::{
    ObjectId, Program, Vm, World, indicator_pools as pool, selection,
    selection_indicators as indicator,
};
use std::collections::BTreeSet;
mod initialize;

struct View<'a> {
    world: &'a mut World,
    program: &'a Program,
    links: &'a Links,
    indicators: &'a sims_vm::load_indicators::Indicators,
    graphics: &'a BTreeSet<ObjectId>,
    enabled: bool,
    pools: Option<&'a mut pool::Pools>,
}
impl View<'_> {
    fn object(&self, id: ObjectId) -> Result<&sims_vm::Object, String> {
        self.world
            .objects
            .get(&id)
            .ok_or_else(|| "selection object absent".into())
    }
    fn native_flags(&self, id: ObjectId) -> Result<u32, String> {
        self.object(id)?
            .native_flags
            .ok_or_else(|| "selection native flags absent".into())
    }
    fn head_slot(&self, id: ObjectId) -> Result<Option<indicator::Slot>, String> {
        let object = self.object(id)?;
        if object.person.is_none() {
            return Err("indicator attachment requires native person slots".into());
        }
        Ok(object.slots.get(1).map(|occupant| indicator::Slot {
            flags: sims_vm::person::BUILTIN_SLOT_FLAGS[1],
            occupant: occupant.unwrap_or(0),
        }))
    }
}
impl selection::View for View<'_> {
    fn person_order(&self) -> &[ObjectId] {
        &self.world.person.as_ref().unwrap().person_order
    }
    fn flags(&self, object: ObjectId) -> Result<u32, String> {
        self.native_flags(object)
    }
    fn script_id(&self, object: ObjectId) -> Result<i16, String> {
        self.object(object)?
            .data
            .get(&11)
            .copied()
            .ok_or_else(|| "selection script ID absent".into())
    }
}
impl selection::Flags for View<'_> {
    fn scope(&self, object: ObjectId) -> Result<selection::FlagScope, String> {
        let value = self.object(object)?;
        let definition = value
            .definition
            .as_ref()
            .ok_or("flag scope definition absent")?;
        if sims_vm::construction::definition_words(self.program, definition)?[10] == 0 {
            return Ok(selection::FlagScope::Single);
        }
        let part = value
            .multitile
            .as_ref()
            .ok_or("multipart flag scope lacks native links")?;
        Ok(
            if sims_vm::multitile::dynamic(self.world, self.program, object)? {
                selection::FlagScope::Single
            } else {
                selection::FlagScope::Group(part.head)
            },
        )
    }
    fn flags(&self, object: ObjectId) -> Result<u32, String> {
        self.native_flags(object)
    }
    fn write_flags(&mut self, object: ObjectId, value: u32) -> Result<(), String> {
        self.native_flags(object)?;
        self.world.objects.get_mut(&object).unwrap().native_flags = Some(value);
        Ok(())
    }
    fn next(&self, object: ObjectId) -> Result<Option<ObjectId>, String> {
        Ok(self
            .object(object)?
            .multitile
            .as_ref()
            .ok_or("multipart flag member lacks native links")?
            .next)
    }
}
impl indicator::View for View<'_> {
    fn enabled(&self) -> bool {
        self.enabled && !self.world.registry_ui_disabled
    }
    fn primary(&self) -> Option<ObjectId> {
        self.indicators.primary
    }
    fn secondary(&self) -> Option<ObjectId> {
        self.indicators.secondary
    }
    fn selected(&self) -> Result<Option<ObjectId>, String> {
        selection::selected(self)
    }
    fn flags(&self, object: ObjectId) -> Result<u32, String> {
        self.native_flags(object)
    }
    fn is_person(&self, object: ObjectId) -> Result<bool, String> {
        Ok(self.object(object)?.person.is_some())
    }
    fn person_data(&self, person: ObjectId, index: i16) -> Result<i16, String> {
        self.object(person)?
            .person
            .as_ref()
            .and_then(|p| p.get(&index))
            .copied()
            .ok_or_else(|| "indicator person field absent".into())
    }
    fn expansions(&self) -> u16 {
        self.world.globals[&20] as u16
    }
    fn lookup(&self, id: i16) -> Option<ObjectId> {
        (id > 0 && self.world.objects.contains_key(&id)).then_some(id)
    }
    fn guid(&self, object: ObjectId) -> Result<u32, String> {
        Ok(self
            .object(object)?
            .definition
            .as_ref()
            .ok_or("indicator definition absent")?
            .guid)
    }
    fn outside(&self, object: ObjectId) -> Result<bool, String> {
        let p = self
            .world
            .positions
            .get(&object)
            .ok_or("indicator position absent")?;
        Ok(!self.links.on_lot([p.x, p.y]))
    }
    fn slot(&self, object: ObjectId) -> Result<Option<indicator::Slot>, String> {
        self.head_slot(object)
    }
    fn position(&self, object: ObjectId) -> Result<[i32; 3], String> {
        let p = self
            .world
            .positions
            .get(&object)
            .ok_or("indicator position absent")?;
        Ok([p.x, p.y, p.level])
    }
    fn graphic(&self, object: ObjectId) -> Result<indicator::Graphic, String> {
        if !self.graphics.contains(&object) {
            return Err(format!("selection graphic owner absent: {object}"));
        }
        Ok(indicator::Graphic(object as u64))
    }
    fn spare(&self) -> Option<Option<ObjectId>> {
        self.pools.as_ref().and_then(|p| p.spare.last().copied())
    }
    fn pop_spare(&mut self) -> Result<(), String> {
        self.pools
            .as_mut()
            .ok_or("selection pools absent")?
            .spare
            .pop()
            .ok_or("selection spare vector empty")?;
        Ok(())
    }
}
impl pool::View for View<'_> {
    fn enabled(&self) -> bool {
        self.enabled && !self.world.registry_ui_disabled
    }
    fn expansions(&self) -> u16 {
        indicator::View::expansions(self)
    }
    fn guid(&self, object: ObjectId) -> Result<u32, String> {
        indicator::View::guid(self, object)
    }
    fn kind(&self, object: ObjectId) -> Result<i16, String> {
        let d = self
            .object(object)?
            .definition
            .as_ref()
            .ok_or("indicator definition absent")?;
        Ok(sims_vm::construction::definition_words(self.program, d)?[9])
    }
    fn slot(&self, object: ObjectId) -> Result<Option<indicator::Slot>, String> {
        self.head_slot(object)
    }
    fn lookup(&self, id: i16) -> Option<ObjectId> {
        indicator::View::lookup(self, id)
    }
    fn hide(&mut self, object: ObjectId) -> Result<(), String> {
        let value = self
            .world
            .objects
            .get_mut(&object)
            .and_then(|o| o.data.get_mut(&8))
            .ok_or("indicator hidden flag absent")?;
        *value |= 0x4000;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Graphic {
        object: ObjectId,
        call: indicator::GraphicCall,
        placement: bool,
    },
    /// Resolve the star resource once, before any of the refill factories.
    Refill,
    FactoryGraphic(ObjectId),
    /// Captured signed payload from445449. The original broadcast must return
    /// before selection or its enclosing deletion may continue.
    Notify(i32),
    IndicatorResource(usize),
    CreationNotification(ObjectId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationOwner {
    Selection { target: Option<ObjectId> },
    Deletion { object: ObjectId, depth: usize },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub owner: NotificationOwner,
    pub payload: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationViewRefresh {
    notification: Notification,
    tick: u64,
    caller_serial: u64,
}
impl NotificationViewRefresh {
    pub fn notification(self) -> Notification {
        self.notification
    }
    pub fn caller_serial(self) -> u64 {
        self.caller_serial
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    ///Saved4e989b child: enclosing publication serial and captured cache index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    publication: Option<(u64, u32)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    factory_root: Option<sims_vm::Definition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    direct_root: Option<(Option<ObjectId>, indicator::Entry)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    initialization: Option<initialize::State>,
    enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    release_root: Option<ObjectId>,
    /// Independent 4464db caller; retains its current child through all physical
    /// and asset services. It never runs Selection's flags or E4 epilogue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh: Option<sims_vm::indicator_refresh::Drain>,
    selection: selection::Selection,
    flags: Option<selection::FlagUpdate>,
    indicators: Option<indicator::Update>,
    release: Option<pool::Release>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refill: Option<pool::Refill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    star_resource: Option<sims_vm::Definition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    factory: Option<crate::lot_creation::OffLot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group_factory: Option<crate::lot_creation::group::Group>,
    placement: Option<lot_placement::live::Execution>,
    external: Option<Request>,
    started: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notification_view_refresh: Option<NotificationViewRefresh>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) notification_view_person: Option<ObjectId>,
}
impl Execution {
    pub fn target(&self) -> Option<ObjectId> {
        self.selection.target()
    }
    pub fn notification_view_person(&self) -> Option<ObjectId> {
        self.notification_view_person
    }
    pub fn publication_owner(&self) -> Option<(u64, u32)> {
        self.publication
    }
    pub fn publication_target(&self) -> Option<Option<ObjectId>> {
        self.publication.map(|_| self.selection.target())
    }
    pub(crate) fn clear() -> Self {
        Self {
            publication: None,
            factory_root: None,
            direct_root: None,
            initialization: None,
            enabled: true,
            release_root: None,
            refresh: None,
            selection: selection::Selection::new(None),
            flags: None,
            indicators: None,
            release: None,
            refill: None,
            star_resource: None,
            factory: None,
            group_factory: None,
            placement: None,
            external: None,
            started: false,
            notification_view_refresh: None,
            notification_view_person: None,
        }
    }
    pub(crate) fn release(object: ObjectId) -> Self {
        Self {
            release_root: Some(object),
            release: Some(pool::Release::new(Some(object))),
            ..Self::clear()
        }
    }
    pub(crate) fn is_release(&self, object: ObjectId) -> bool {
        self.release_root == Some(object)
    }
    pub(crate) fn is_clear(&self) -> bool {
        self.factory_root.is_none()
            && self.direct_root.is_none()
            && self.initialization.is_none()
            && self.release_root.is_none()
            && self.refresh.is_none()
            && self.selection.target().is_none()
    }
    pub fn is_refresh(&self) -> bool {
        self.refresh.is_some()
    }
    pub fn is_initialization(&self) -> bool {
        self.initialization.is_some()
    }
    pub fn direct_indicator(&self) -> Option<(Option<ObjectId>, indicator::Entry)> {
        self.direct_root
    }
    pub fn direct_factory(&self) -> Option<&sims_vm::Definition> {
        self.factory_root.as_ref()
    }
    fn owns_creation_notifications(&self) -> bool {
        self.initialization.is_some() || self.direct_root.is_some() || self.factory_root.is_some()
    }
    pub(crate) fn constructing_graphic(&self) -> Option<ObjectId> {
        self.factory
            .as_ref()
            .and_then(crate::lot_creation::OffLot::pending_graphic)
            .or_else(|| {
                self.group_factory
                    .as_ref()
                    .and_then(crate::lot_creation::group::Group::pending_graphic)
            })
    }
    pub(crate) fn constructing_objects(&self) -> Vec<ObjectId> {
        match &self.group_factory {
            Some(g) => g.constructing().to_vec(),
            None => self.constructing_graphic().into_iter().collect(),
        }
    }
    pub(crate) fn group_construction(&self) -> Option<&sims_vm::group_factory::Construction> {
        self.group_factory
            .as_ref()
            .and_then(crate::lot_creation::group::Group::construction)
    }
    fn factory_created(&self) -> Option<ObjectId> {
        self.factory
            .as_ref()
            .and_then(crate::lot_creation::OffLot::pending_created)
            .or_else(|| {
                self.group_factory
                    .as_ref()
                    .and_then(crate::lot_creation::group::Group::pending_created)
            })
    }
    pub fn owns_callback(
        &self,
        vm: &Vm,
        p: &Program,
        thread: sims_vm::ThreadId,
        depth: usize,
    ) -> bool {
        self.placement
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }
    pub fn validate(&self, vm: &Vm, p: &Program) -> Result<(), String> {
        if vm.world.person.is_none() || !vm.world.globals.contains_key(&20) {
            return Err("saved selection context absent".into());
        }
        self.selection.validate()?;
        if self.publication.is_some_and(|p| p.0 == 0)
            || self.publication.is_some()
                && (self.factory_root.is_some()
                    || self.direct_root.is_some()
                    || self.initialization.is_some()
                    || self.release_root.is_some()
                    || self.refresh.is_some())
        {
            return Err("selection publication has a different root caller".into());
        }
        if let Some(d) = &self.factory_root {
            sims_vm::construction::definition_words(p, d)?;
            if self.direct_root.is_some()
                || self.initialization.is_some()
                || self.release_root.is_some()
                || self.refresh.is_some()
                || self.flags.is_some()
                || self.indicators.is_some()
                || self.release.is_some()
                || self.refill.is_some()
                || self.star_resource.is_some()
                || self.placement.is_some()
                || self.selection != selection::Selection::new(None)
                || self
                    .factory
                    .as_ref()
                    .and_then(crate::lot_creation::OffLot::object)
                    .is_some_and(|id| {
                        vm.world
                            .objects
                            .get(&id)
                            .is_none_or(|o| o.definition.as_ref() != Some(d))
                    })
            {
                return Err("direct factory lost its load caller".into());
            }
        }
        if let Some(root) = self.direct_root {
            if self.initialization.is_some()
                || self.release_root.is_some()
                || self.refresh.is_some()
                || self.flags.is_some()
                || self.selection != selection::Selection::new(None)
                || self.indicators.as_ref().map(|i| (i.object(), i.entry())) != Some(root)
            {
                return Err("direct indicator call lost its post-load caller".into());
            }
        }
        if let Some(init) = &self.initialization {
            init.validate(self, vm, p)?;
        }
        if let Some(drain) = &self.refresh {
            drain.validate()?;
            if self.release_root.is_some()
                || self.flags.is_some()
                || self.selection != selection::Selection::new(None)
            {
                return Err("indicator drain has an unrelated selection owner".into());
            }
            let expected = drain.pending().map(|r| match r {
                sims_vm::indicator_refresh::Request::Date(id) => (id, indicator::Entry::Date),
                sims_vm::indicator_refresh::Request::Star(id) => (id, indicator::Entry::Star),
            });
            if expected != self.indicators.as_ref().map(|i| (i.object(), i.entry())) {
                return Err("indicator drain lost its retained callback".into());
            }
        } else if self.direct_root.is_none()
            && self
                .indicators
                .as_ref()
                .is_some_and(|i| i.entry() != indicator::Entry::Selection)
        {
            return Err("indicator notification callback lost its drain owner".into());
        }
        if self.release_root.is_some_and(|id| id <= 0)
            || self.release_root.is_some()
                && self
                    .release
                    .as_ref()
                    .is_some_and(|r| r.object() != self.release_root)
            || self.release_root.is_some()
                && (self.selection != selection::Selection::new(None)
                    || self.flags.is_some()
                    || self.indicators.is_some()
                    || self.refill.is_some()
                    || self.factory.is_some()
                    || self.star_resource.is_some())
        {
            return Err("invalid independent indicator release".into());
        }
        if let Some(v) = &self.flags {
            v.validate()?;
        }
        if let Some(v) = &self.indicators {
            v.validate()?;
        }
        if let Some(v) = &self.release {
            v.validate()?;
        }
        if let Some(v) = &self.refill {
            v.validate()?;
            if self
                .indicators
                .as_ref()
                .and_then(indicator::Update::pending)
                != Some(indicator::Request::Refill)
                || self.release.is_some()
                || self.placement.is_some()
                || v.resource() != self.star_resource.as_ref().map(|_| pool::Resource(1))
            {
                return Err("star refill lost its indicator caller".into());
            }
        }
        if let Some(definition) = &self.star_resource {
            let w = sims_vm::construction::definition_words(p, definition)?;
            if self.refill.is_none()
                || definition.guid != indicator::STAR_GUID
                || (u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16)) != definition.guid
                || w[20] as u16 != definition.slots
            {
                return Err("star refill resource binding differs".into());
            }
        }
        if let Some(v) = &self.factory {
            v.validate_direct(vm, p)?;
            if self.initialization.is_none()
                && self.factory_root.is_none()
                && (self.star_resource.is_none()
                    || self.refill.as_ref().and_then(pool::Refill::pending)
                        != Some(pool::Request::CreateStar(pool::Resource(1)))
                    || v.object()
                        .is_some_and(|id| vm.world.objects[&id].definition != self.star_resource))
            {
                return Err("star factory lost its retained resource caller".into());
            }
        }
        if let Some(g) = &self.group_factory {
            g.validate(vm, p)?;
            if self.factory_root.as_ref() != Some(g.definition()) || self.factory.is_some() {
                return Err("direct group factory lost its catalog caller".into());
            }
        }
        if let Some(v) = &self.placement {
            v.validate(vm, p, v.operation.binding().0)?;
        }
        if self.started && self.external.is_none() {
            return Err("selection asset service lost its owner".into());
        }
        if let Some(token) = self.notification_view_refresh {
            if !self.started
                || token.tick != vm.tick
                || self.external != Some(Request::Notify(token.notification.payload))
                || match token.notification.owner {
                    NotificationOwner::Selection { target } => target != self.selection.target(),
                    NotificationOwner::Deletion { object, depth } => object <= 0 || depth == 0,
                }
            {
                return Err("selection view refresh lost its notification token".into());
            }
        }
        if self.notification_view_person.is_some()
            && self.notification_view_refresh.is_none_or(|token| token.caller_serial == 0)
        {
            return Err("selection view person lost its notification token".into());
        }
        if let Some(Request::Notify(payload)) = self.external {
            if self.selection.pending() != Some(selection::Request::Notify(payload))
                || self.refresh.is_some()
                || self.release_root.is_some()
                || self.flags.is_some()
                || self.indicators.is_some()
                || self.placement.is_some()
                || self.factory.is_some()
            {
                return Err("selection notification lost its synchronous caller".into());
            }
        }
        if let Some(Request::Graphic {
            object,
            call,
            placement: true,
        }) = self.external
        {
            if call != indicator::GraphicCall::Refresh
                || self.placement.as_ref().and_then(|e| e.pending.as_ref())
                    != Some(&lot_placement::Request::RefreshAnimator(object))
            {
                return Err("selection placement graphic binding differs".into());
            }
        }
        if let Some(Request::FactoryGraphic(id)) = self.external {
            if self.constructing_graphic() != Some(id) {
                return Err("selection factory graphic binding differs".into());
            }
        }
        if let Some(Request::CreationNotification(id)) = self.external {
            if !self.owns_creation_notifications() || self.factory_created() != Some(id) {
                return Err("creation notification lost its original factory".into());
            }
        }
        if self.external == Some(Request::Refill)
            && (self.started
                || self
                    .indicators
                    .as_ref()
                    .and_then(indicator::Update::pending)
                    != Some(indicator::Request::Refill)
                || self
                    .refill
                    .as_ref()
                    .is_some_and(|r| r.pending() != Some(pool::Request::LookupStar)))
        {
            return Err("selection star lookup binding differs".into());
        }
        if let Some(Request::Graphic {
            object,
            call,
            placement: false,
        }) = self.external
        {
            if self
                .indicators
                .as_ref()
                .and_then(indicator::Update::pending)
                != Some(indicator::Request::Graphic {
                    graphic: indicator::Graphic(object as u64),
                    call,
                })
            {
                return Err("selection graphic lost its native caller".into());
            }
        }
        Ok(())
    }
    pub(crate) fn advance(
        &mut self,
        sim: &mut Simulation,
        p: &Program,
        graphics: &BTreeSet<ObjectId>,
        ordinary: &BTreeSet<ObjectId>,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        loop {
            sim.publish_native_selection()?;
            // Older snapshots retained the unstarted refill call as the
            // external boundary. Enter its newly supported controller once.
            if self.external == Some(Request::Refill) && self.refill.is_none() && !self.started {
                self.external = None;
                self.refill = Some(pool::Refill::default());
            }
            if self.external.is_some() || *remaining == 0 {
                return Ok(false);
            }
            if let Some(group) = &mut self.group_factory {
                match group.advance(sim, p, remaining, step)? {
                    Some(crate::lot_creation::Request::Graphic(id)) => {
                        self.external = Some(Request::FactoryGraphic(id))
                    }
                    Some(crate::lot_creation::Request::Created(id)) => {
                        self.external = Some(Request::CreationNotification(id))
                    }
                    Some(crate::lot_creation::Request::Budget) => (),
                    Some(other) => {
                        return Err(format!(
                            "direct group factory awaits original service: {other:?}"
                        ));
                    }
                    None => return Ok(true),
                }
                return Ok(false);
            }
            if let Some(factory) = &mut self.factory {
                // The child may use the whole instruction budget. Never run a
                // second child or the parent with that same budget afterward.
                let budget = std::mem::take(remaining);
                match factory.advance(&mut sim.vm, p, budget)? {
                    Some(crate::lot_creation::Request::Graphic(id)) => {
                        self.external = Some(Request::FactoryGraphic(id));
                    }
                    Some(crate::lot_creation::Request::Created(id)) => {
                        if self.initialization.is_some()
                            || self.direct_root.is_some()
                            || self.factory_root.is_some()
                        {
                            self.external = Some(Request::CreationNotification(id));
                        } else {
                            sim.native_lot.as_mut().unwrap().observers.push(
                                lot_placement::Request::Signal {
                                    event: 0xe2,
                                    value: id,
                                },
                            );
                        }
                    }
                    Some(crate::lot_creation::Request::Budget) => (),
                    Some(request) => {
                        return Err(format!("star factory awaits original service: {request:?}"));
                    }
                    None => {
                        if self.factory_root.is_some() {
                            // Retain440b2d's return until the enclosing caller
                            // consumes it, including the null-ID exhaustion case.
                            return Ok(true);
                        }
                        if let Some(init) = &mut self.initialization {
                            init.finish_factory(sim, factory)?;
                            self.factory = None;
                            return Ok(false);
                        }
                        let id = factory.object().unwrap_or(0);
                        self.refill
                            .as_mut()
                            .ok_or("star factory caller absent")?
                            .acknowledge(
                                pool::Request::CreateStar(pool::Resource(1)),
                                pool::Reply::Created(id),
                            )?;
                        self.factory = None;
                    }
                }
                return Ok(false);
            }
            if let Some(definition) = &self.factory_root {
                let g = &sim.vm.world.globals;
                let read = |i| g.get(&i).copied().ok_or("factory calendar absent");
                let now = sims_vm::construction::ObjectDate {
                    year: read(8)?,
                    month: read(7)?,
                    day: read(1)?,
                };
                let time = [read(0)?, read(5)?];
                if sims_vm::construction::definition_words(p, definition)?[10] != 0 {
                    self.group_factory = Some(crate::lot_creation::group::Group::new(
                        p,
                        definition.clone(),
                        now,
                        time,
                    )?);
                    *remaining -= 1;
                    continue;
                }
                let native = sim.native_lot.as_mut().ok_or("factory native lot absent")?;
                let factory = crate::lot_creation::OffLot::begin_definition_with_graphics(
                    &mut sim.vm,
                    p,
                    &mut native.placement.links,
                    definition.clone(),
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
                *remaining -= 1;
                continue;
            }
            if self.initialization.is_some() {
                if self.advance_initialization(sim, p)? {
                    return Ok(true);
                }
                *remaining -= 1;
                continue;
            }
            if let Some(place) = &mut self.placement {
                if !sim.drive_live_placement_with_graphics(p, place, remaining, step, ordinary)? {
                    if let Some(lot_placement::Request::RefreshAnimator(id)) = place.pending {
                        if ordinary.contains(&id) {
                            self.external = Some(Request::Graphic {
                                object: id,
                                call: indicator::GraphicCall::Refresh,
                                placement: true,
                            });
                        }
                    }
                    return Ok(false);
                }
                self.placement = None;
                // The physical call belongs either to Release or directly to
                // the indicator controller. Their requests remain pending.
                if let Some(release) = &mut self.release {
                    let native = sim.native_lot.as_ref().unwrap();
                    let view = View {
                        world: &mut sim.vm.world,
                        program: p,
                        links: &native.placement.links,
                        indicators: &native.indicators,
                        graphics,
                        enabled: self.enabled,
                        pools: None,
                    };
                    let request = release
                        .advance(&mut sim.native_indicator_pools, &view)?
                        .ok_or("selection release lost detach")?;
                    release.acknowledge(request)?;
                } else {
                    let native = sim.native_lot.as_ref().unwrap();
                    let mut view = View {
                        world: &mut sim.vm.world,
                        program: p,
                        links: &native.placement.links,
                        indicators: &native.indicators,
                        graphics,
                        enabled: self.enabled,
                        pools: Some(&mut sim.native_indicator_pools),
                    };
                    let indicators = self
                        .indicators
                        .as_mut()
                        .ok_or("placement indicator owner absent")?;
                    let request = indicators
                        .advance(&mut view)?
                        .ok_or("selection attachment request absent")?;
                    indicators.acknowledge(request, indicator::Reply::Done)?;
                }
                continue;
            }
            let native = sim
                .native_lot
                .as_ref()
                .ok_or("selection native lot absent")?;
            let mut view = View {
                world: &mut sim.vm.world,
                program: p,
                links: &native.placement.links,
                indicators: &native.indicators,
                graphics,
                enabled: self.enabled,
                pools: None,
            };
            if let Some(refill) = &mut self.refill {
                match refill.advance(&mut sim.native_indicator_pools, &mut view)? {
                    Some(pool::Request::LookupStar) => self.external = Some(Request::Refill),
                    Some(pool::Request::CreateStar(pool::Resource(1))) => {
                        let definition = self
                            .star_resource
                            .clone()
                            .ok_or("star resource owner absent")?;
                        let g = &sim.vm.world.globals;
                        let read = |i| g.get(&i).copied().ok_or("star factory calendar absent");
                        let now = sims_vm::construction::ObjectDate {
                            year: read(8)?,
                            month: read(7)?,
                            day: read(1)?,
                        };
                        let time = [read(0)?, read(5)?];
                        let native = sim.native_lot.as_mut().unwrap();
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
                    Some(other) => return Err(format!("unexpected star refill service {other:?}")),
                    None => {
                        if !refill.complete() {
                            return Err("star refill stopped before completion".into());
                        }
                        self.indicators
                            .as_mut()
                            .ok_or("star indicator caller absent")?
                            .acknowledge(indicator::Request::Refill, indicator::Reply::Done)?;
                        self.refill = None;
                        self.star_resource = None;
                    }
                }
                *remaining -= 1;
                continue;
            }
            if let Some(release) = &mut self.release {
                if let Some(pool::Request::Detach(id)) =
                    release.advance(&mut sim.native_indicator_pools, &view)?
                {
                    self.placement = Some(lot_placement::live::Execution::new_virtual(
                        VirtualOperation::remove(view.world, view.links, id, false)?,
                    ));
                    continue;
                }
                if !release.complete() {
                    return Err("unsupported selection pool release service".into());
                }
                self.release = None;
                if self.release_root.is_some() {
                    return Ok(true);
                }
                view.pools = Some(&mut sim.native_indicator_pools);
                let indicators = self
                    .indicators
                    .as_mut()
                    .ok_or("release indicator owner absent")?;
                let request = indicators
                    .advance(&mut view)?
                    .ok_or("selection release request absent")?;
                indicators.acknowledge(request, indicator::Reply::Done)?;
                continue;
            }
            if let Some(indicators) = &mut self.indicators {
                view.pools = Some(&mut sim.native_indicator_pools);
                if let Some(request) = indicators.advance(&mut view)? {
                    match request {
                        indicator::Request::Parent(id) => {
                            let parent = view.links.capture(view.world, id)?.container;
                            indicators.acknowledge(request, indicator::Reply::Parent(parent))?;
                        }
                        indicator::Request::Detach(id) => {
                            self.placement = Some(lot_placement::live::Execution::new_virtual(
                                VirtualOperation::remove(view.world, view.links, id, true)?,
                            ))
                        }
                        indicator::Request::Attach {
                            indicator,
                            person,
                            position: [x, y, level],
                        } => {
                            self.placement = Some(lot_placement::live::Execution::new_virtual(
                                VirtualOperation::set(
                                    view.world,
                                    view.links,
                                    indicator,
                                    Descriptor {
                                        on_lot: view.links.on_lot([x, y]),
                                        position: [x, y],
                                        container: Some(person),
                                        slot: 1,
                                        level,
                                    },
                                    true,
                                )?,
                            ));
                        }
                        indicator::Request::Release(id) => {
                            self.release = Some(pool::Release::new(Some(id)))
                        }
                        indicator::Request::RecordStar(id) => {
                            view.pools.as_mut().unwrap().active.push(Some(id));
                            indicators.acknowledge(request, indicator::Reply::Done)?;
                        }
                        indicator::Request::Graphic { graphic, call } => {
                            let object = i16::try_from(graphic.0)
                                .map_err(|_| "selection graphic handle invalid")?;
                            self.external = Some(Request::Graphic {
                                object,
                                call,
                                placement: false,
                            });
                        }
                        indicator::Request::Refill => self.refill = Some(pool::Refill::default()),
                    }
                    *remaining -= 1;
                    continue;
                }
                self.indicators = None;
                if self.direct_root.is_some() {
                    return Ok(true);
                }
                if let Some(drain) = &mut self.refresh {
                    drain.acknowledge(drain.pending().ok_or("indicator drain callback absent")?)?;
                    continue;
                }
                let request = self
                    .selection
                    .advance(&view)?
                    .ok_or("selection indicators owner absent")?;
                self.selection.acknowledge(request)?;
                continue;
            }
            if let Some(flags) = &mut self.flags {
                if let Some(id) = flags.advance(&mut view)? {
                    sim.vm.refresh_object(id, id, 0, 0)?;
                    flags.acknowledge(id)?;
                    *remaining -= 1;
                    continue;
                }
                self.flags = None;
                let request = self
                    .selection
                    .advance(&view)?
                    .ok_or("selection flags owner absent")?;
                self.selection.acknowledge(request)?;
                continue;
            }
            if self.release_root.is_some() {
                return Ok(true);
            }
            if let Some(drain) = &mut self.refresh {
                let Some(request) = drain.advance(&mut view.world.indicator_queues)? else {
                    return Ok(true);
                };
                let (person, entry) = match request {
                    sims_vm::indicator_refresh::Request::Date(id) => (id, indicator::Entry::Date),
                    sims_vm::indicator_refresh::Request::Star(id) => (id, indicator::Entry::Star),
                };
                self.indicators = Some(indicator::Update::new(person, entry));
                *remaining -= 1;
                continue;
            }
            let Some(request) = self.selection.advance(&view)? else {
                return Ok(true);
            };
            match request {
                selection::Request::Flags { object, low } => {
                    self.flags = Some(selection::FlagUpdate::new(object, low))
                }
                selection::Request::Invalidate(id) => {
                    sim.vm.refresh_object(id, id, 0, 0)?;
                    self.selection.acknowledge(request)?;
                }
                selection::Request::Indicators(id) => {
                    self.indicators = Some(indicator::Update::new(
                        Some(id),
                        indicator::Entry::Selection,
                    ))
                }
                selection::Request::Notify(payload) => {
                    if sim.native_selection_broadcasts {
                        self.external = Some(Request::Notify(payload));
                    } else {
                        // No application registry is installed in this explicit
                        // headless profile; its only listener is Globals.
                        sim.vm.world.notify_person_selection(payload)?;
                        self.selection.acknowledge(request)?;
                    }
                }
            }
            *remaining -= 1;
        }
    }
}
impl Simulation {
    /// Shared4320da setter with434101 invalidation. Multipart links are read
    /// live between members; dynamic groups update only the selected part.
    pub fn update_native_low_flags(
        &mut self,
        p: &Program,
        object: ObjectId,
        requested: u32,
    ) -> Result<(), String> {
        let mut call = selection::FlagUpdate::new(object, requested);
        let graphics = BTreeSet::new();
        loop {
            let native = self
                .native_lot
                .as_ref()
                .ok_or("flag setter native lot absent")?;
            let mut view = View {
                world: &mut self.vm.world,
                program: p,
                links: &native.placement.links,
                indicators: &native.indicators,
                graphics: &graphics,
                enabled: true,
                pools: None,
            };
            let Some(id) = call.advance(&mut view)? else {
                return Ok(());
            };
            self.vm.refresh_object(id, id, 0, 0)?;
            call.acknowledge(id)?;
        }
    }
    pub(crate) fn validate_selection_notification_mode(&self) -> Result<(), String> {
        if !self.native_selection_broadcasts
            && self
                .native_selection
                .iter()
                .chain(
                    self.deletion
                        .iter()
                        .filter_map(|d| d.entry.as_ref()?.selection.as_ref()),
                )
                .any(|e| matches!(e.external, Some(Request::Notify(_))))
        {
            return Err("selection notification has no installed dispatcher".into());
        }
        Ok(())
    }
    fn publish_native_selection(&mut self) -> Result<(), String> {
        let selected = self.native_selected_person()?;
        self.native_lot
            .as_mut()
            .ok_or("selection native lot absent")?
            .selected_person = selected;
        if let Some(a) = self
            .vm
            .world
            .person
            .as_mut()
            .and_then(|p| p.autonomy.as_mut())
        {
            a.selected_person = selected;
        }
        Ok(())
    }
    pub fn native_selected_person(&self) -> Result<Option<ObjectId>, String> {
        for &id in &self
            .vm
            .world
            .person
            .as_ref()
            .ok_or("selection person registry absent")?
            .person_order
        {
            let flags = self
                .vm
                .world
                .objects
                .get(&id)
                .and_then(|o| o.native_flags)
                .ok_or("selection native flags absent")?;
            if flags & 2 != 0 {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }
    pub fn begin_native_selection(
        &mut self,
        target: Option<ObjectId>,
        enabled: bool,
    ) -> Result<(), String> {
        if (self.ground_placement.is_some()
            || self.object_pickup.is_some()
            || self.object_placement.is_some()
            || self.cursor_work_pending())
            || self.native_selection.is_some()
            || self.vm.player_menu.is_some()
            || !self.deletion.is_empty()
            || self.synchronous_removal.is_some()
            || self.portrait_person.is_some()
            || self.vm.threads.values().any(|t| t.callback.is_some())
            || self.characters.values().any(|r| {
                r.refresh_tick.is_some() || r.ordinary_update.is_some() || r.character.busy()
            })
        {
            return Err("selection overlaps a synchronous native service".into());
        }
        if self.native_lot.is_none()
            || self.vm.world.person.is_none()
            || !self.vm.world.globals.contains_key(&20)
        {
            return Err("selection native context absent".into());
        }
        if target.is_some_and(|id| {
            self.vm
                .world
                .objects
                .get(&id)
                .is_none_or(|o| o.person.is_none())
        }) {
            return Err("selection target is not a registered person".into());
        }
        self.native_selection = Some(Execution {
            publication: None,
            factory_root: None,
            direct_root: None,
            initialization: None,
            enabled,
            release_root: None,
            refresh: None,
            selection: selection::Selection::new(target),
            flags: None,
            indicators: None,
            release: None,
            refill: None,
            star_resource: None,
            factory: None,
            group_factory: None,
            placement: None,
            external: None,
            started: false,
            notification_view_refresh: None,
            notification_view_person: None,
        });
        Ok(())
    }
    ///The actual445449 child owned by a saved4e989b publication caller.
    pub fn begin_publication_selection(
        &mut self,
        target: Option<ObjectId>,
        owner: (u64, u32),
    ) -> Result<(), String> {
        if owner.0 == 0 {
            return Err("selection publication identity absent".into());
        }
        self.begin_native_selection(target, true)?;
        self.native_selection.as_mut().unwrap().publication = Some(owner);
        Ok(())
    }
    /// The 4464db portion of the native presenter dispatch. The caller owns
    /// when the event occurs; this is never implicitly called by tick/slice.
    pub fn begin_native_indicator_refresh(&mut self, enabled: bool) -> Result<(), String> {
        self.begin_native_selection(None, enabled)?;
        self.native_selection.as_mut().unwrap().refresh = Some(Default::default());
        Ok(())
    }
    pub fn begin_native_indicator_initialization(&mut self) -> Result<(), String> {
        self.begin_native_selection(None, true)?;
        self.native_selection.as_mut().unwrap().initialization = Some(Default::default());
        Ok(())
    }
    /// Direct440b2d with saved graphic, Reset/Init and E2 boundaries.
    pub fn begin_native_object_factory(
        &mut self,
        p: &Program,
        definition: sims_vm::Definition,
    ) -> Result<(), String> {
        sims_vm::construction::definition_words(p, &definition)?;
        self.begin_native_selection(None, true)?;
        self.native_selection.as_mut().unwrap().factory_root = Some(definition);
        Ok(())
    }
    /// Direct44664c/446ee9 entry used by43fc94, without445449 selection or E4.
    pub fn begin_native_indicator_update(
        &mut self,
        object: Option<ObjectId>,
        entry: indicator::Entry,
        enabled: bool,
    ) -> Result<(), String> {
        let update = indicator::Update::new(object, entry);
        update.validate()?;
        if object.is_some_and(|id| !self.vm.world.objects.contains_key(&id)) {
            return Err("direct indicator object absent".into());
        }
        self.begin_native_selection(None, enabled)?;
        let e = self.native_selection.as_mut().unwrap();
        e.direct_root = Some((object, entry));
        e.indicators = Some(update);
        Ok(())
    }
    pub fn advance_native_selection(
        &mut self,
        p: &Program,
        ordinary: &BTreeSet<ObjectId>,
        budget: usize,
    ) -> Result<Step, String> {
        self.advance_native_selection_result(p, ordinary, budget)
            .map(|(step, _)| step)
    }
    /// Direct440b2d's result belongs to its caller, not a scan of newly added
    /// world IDs. Outer None is pending; Some(None) is native factory failure.
    pub fn advance_native_object_factory(
        &mut self,
        p: &Program,
        ordinary: &BTreeSet<ObjectId>,
        budget: usize,
    ) -> Result<(Step, Option<Option<ObjectId>>), String> {
        if self
            .native_selection
            .as_ref()
            .and_then(Execution::direct_factory)
            .is_none()
        {
            return Err("direct native factory absent".into());
        }
        self.advance_native_selection_result(p, ordinary, budget)
    }
    fn advance_native_selection_result(
        &mut self,
        p: &Program,
        ordinary: &BTreeSet<ObjectId>,
        budget: usize,
    ) -> Result<(Step, Option<Option<ObjectId>>), String> {
        let mut execution = self
            .native_selection
            .take()
            .ok_or("native selection absent")?;
        let graphics = ordinary
            .iter()
            .chain(self.characters.keys())
            .copied()
            .collect();
        let mut step = Step {
            stop: crate::StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        let mut remaining = budget;
        let result = execution.advance(self, p, &graphics, ordinary, &mut remaining, &mut step);
        self.native_selection = Some(execution);
        let mut created = None;
        if result? {
            let e = self.native_selection.as_ref().unwrap();
            if e.factory_root.is_some() {
                created = Some(if let Some(g) = &e.group_factory {
                    g.object()
                } else {
                    e.factory
                        .as_ref()
                        .ok_or("completed factory return absent")?
                        .object()
                });
            }
            self.publish_native_selection()?;
            self.native_selection = None;
        }
        Ok((step, created))
    }
    pub fn pending_native_selection(&self) -> Option<(Request, bool)> {
        self.selection_service()
            .and_then(|e| e.external.map(|r| (r, e.started)))
    }
    pub fn selection_work_pending(&self) -> bool {
        self.selection_service().is_some()
    }
    pub fn pending_selection_notification(&self) -> Option<(Notification, bool)> {
        let execution = self.selection_service()?;
        let Request::Notify(payload) = execution.external? else {
            return None;
        };
        let owner = if let Some(e) = &self.native_selection {
            NotificationOwner::Selection {
                target: e.selection.target(),
            }
        } else {
            NotificationOwner::Deletion {
                object: self.deletion.last()?.target,
                depth: self.deletion.len(),
            }
        };
        Some((Notification { owner, payload }, execution.started))
    }
    pub fn start_selection_notification(&mut self, call: Notification) -> Result<(), String> {
        if !self.native_selection_broadcasts
            || self.pending_selection_notification() != Some((call, false))
            || self.selection_service().is_some_and(|execution| {
                execution.notification_view_refresh.is_some()
                    || execution.notification_view_person.is_some()
            })
        {
            return Err("stale selection notification start".into());
        }
        let tick = self.vm.tick;
        let execution = service_mut(&mut self.native_selection, &mut self.deletion).unwrap();
        execution.started = true;
        execution.notification_view_refresh = Some(NotificationViewRefresh {
            notification: call,
            tick,
            caller_serial: 0,
        });
        Ok(())
    }
    pub fn selection_notification_view_refresh(
        &mut self,
        call: Notification,
        caller_serial: u64,
    ) -> Result<NotificationViewRefresh, String> {
        let token = self
            .native_selection
            .as_ref()
            .and_then(|execution| execution.notification_view_refresh)
            .ok_or("selection view refresh notification token absent")?;
        if token.notification != call
            || token.caller_serial != 0
            || caller_serial == 0
            || self.native_selection.as_ref().unwrap().notification_view_person.is_some()
        {
            return Err("selection view refresh notification token differs".into());
        }
        self.validate_selection_notification_view_refresh_parent(token)?;
        let token = NotificationViewRefresh { caller_serial, ..token };
        self.native_selection.as_mut().unwrap().notification_view_refresh = Some(token);
        Ok(token)
    }
    pub fn validate_selection_notification_view_refresh(
        &self,
        token: NotificationViewRefresh,
    ) -> Result<(), String> {
        if token.caller_serial == 0 {
            return Err("selection view refresh notification has no caller lifetime".into());
        }
        self.validate_selection_notification_view_refresh_parent(token)
    }
    fn validate_selection_notification_view_refresh_parent(
        &self,
        token: NotificationViewRefresh,
    ) -> Result<(), String> {
        let execution = self
            .native_selection
            .as_ref()
            .ok_or("selection view refresh has no standalone selection parent")?;
        if !matches!(token.notification.owner, NotificationOwner::Selection { .. })
            || !self.deletion.is_empty()
            || !self.native_selection_broadcasts
            || token.tick != self.vm.tick
            || execution.notification_view_refresh != Some(token)
            || self.pending_selection_notification() != Some((token.notification, true))
            || execution.selection.pending()
                != Some(selection::Request::Notify(token.notification.payload))
        {
            return Err("selection view refresh notification parent differs".into());
        }
        execution.selection.validate()?;
        if let NotificationOwner::Selection {
            target: Some(target),
        } = token.notification.owner
        {
            if self
                .vm
                .world
                .objects
                .get(&target)
                .is_none_or(|object| object.person.is_none())
            {
                return Err("selection view refresh parent person absent".into());
            }
        }
        if execution.flags.is_some()
            || execution.indicators.is_some()
            || execution.placement.is_some()
            || execution.refresh.is_some()
            || execution.factory.is_some()
            || execution.factory_root.is_some()
            || execution.direct_root.is_some()
            || execution.initialization.is_some()
            || execution.release_root.is_some()
            || execution.release.is_some()
            || execution.refill.is_some()
            || execution.star_resource.is_some()
            || execution.group_factory.is_some()
            || self.ground_placement.is_some()
            || self.object_pickup.is_some()
            || self.object_placement.is_some()
            || self.cursor_work_pending()
            || self.synchronous_removal.is_some()
            || self.portrait_person.is_some()
            || !self.native_creations.is_empty()
            || self.person_factories.values().any(|factory| !factory.finalized)
            || self.native_lot.as_ref().is_none_or(|lot| {
                lot.route.is_some()
                    || lot.shoo.is_some()
                    || lot.movement.is_some()
                    || lot.completion.is_some()
                    || lot.snap.is_some()
                    || lot.find_location.is_some()
                    || lot.removal.is_some()
            })
            || self.vm.player_menu.is_some()
            || self.vm.threads.values().any(|thread| thread.callback.is_some())
            || self.characters.iter().any(|(&object, resident)| {
                resident.ordinary_update.is_some()
                    || resident.person_update.is_some()
                    || resident.skill_profile.is_some()
                    || resident.refresh_tick.is_some_and(|tick| {
                        tick != token.tick || execution.notification_view_person != Some(object)
                    })
                    || resident.refresh_tick.is_none() && resident.character.busy()
            })
            || execution
                .notification_view_person
                .is_some_and(|object| !self.characters.contains_key(&object))
        {
            return Err("selection view refresh overlaps unrelated native work".into());
        }
        Ok(())
    }
    pub fn complete_selection_notification(&mut self, call: Notification) -> Result<(), String> {
        if !self.native_selection_broadcasts
            || self.pending_selection_notification() != Some((call, true))
        {
            return Err("stale selection notification return".into());
        }
        let e = service_mut(&mut self.native_selection, &mut self.deletion).unwrap();
        if e.notification_view_person.is_some() {
            return Err("selection notification still owns a view person refresh".into());
        }
        e.selection
            .acknowledge(selection::Request::Notify(call.payload))?;
        e.external = None;
        e.started = false;
        e.notification_view_refresh = None;
        Ok(())
    }
    pub(crate) fn selection_service(&self) -> Option<&Execution> {
        self.native_selection
            .as_ref()
            .or_else(|| self.deletion.last()?.entry.as_ref()?.selection.as_ref())
    }
    pub fn start_selection_graphic(&mut self, request: Request) -> Result<(), String> {
        let e = service_mut(&mut self.native_selection, &mut self.deletion)
            .ok_or("native selection absent")?;
        if e.external != Some(request)
            || e.started
            || matches!(
                request,
                Request::Refill
                    | Request::Notify(_)
                    | Request::IndicatorResource(_)
                    | Request::CreationNotification(_)
            )
        {
            return Err("stale selection graphic start".into());
        }
        e.started = true;
        Ok(())
    }
    pub fn acknowledge_native_selection(&mut self, request: Request) -> Result<(), String> {
        let e = service_mut(&mut self.native_selection, &mut self.deletion)
            .ok_or("native selection absent")?;
        if e.external != Some(request) || !e.started {
            return Err("stale selection graphic completion".into());
        }
        match request {
            Request::Graphic {
                object,
                placement: true,
                ..
            } => {
                let p = e.placement.as_mut().ok_or("selection placement absent")?;
                if p.pending != Some(lot_placement::Request::RefreshAnimator(object)) {
                    return Err("selection placement refresh differs".into());
                }
                p.pending = None;
            }
            Request::Graphic {
                object,
                call,
                placement: false,
            } => e
                .indicators
                .as_mut()
                .ok_or("selection indicator owner absent")?
                .acknowledge(
                    indicator::Request::Graphic {
                        graphic: indicator::Graphic(object as u64),
                        call,
                    },
                    indicator::Reply::Done,
                )?,
            Request::Refill => {
                return Err("selection star refill requires its factory owner".into());
            }
            Request::FactoryGraphic(_) => {
                return Err("selection factory graphic requires lifecycle handoff".into());
            }
            Request::Notify(_) => {
                return Err("selection notification requires its broadcast owner".into());
            }
            Request::IndicatorResource(_) | Request::CreationNotification(_) => {
                return Err("indicator initialization requires its owning caller".into());
            }
        }
        e.external = None;
        e.started = false;
        Ok(())
    }
    pub fn complete_selection_resource(
        &mut self,
        p: &Program,
        definition: Option<sims_vm::Definition>,
    ) -> Result<(), String> {
        let e = service_mut(&mut self.native_selection, &mut self.deletion)
            .ok_or("native selection absent")?;
        if e.external != Some(Request::Refill) || e.started {
            return Err("stale star resource reply".into());
        }
        if let Some(definition) = &definition {
            let w = sims_vm::construction::definition_words(p, definition)?;
            if definition.guid != indicator::STAR_GUID
                || (u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16)) != definition.guid
                || w[20] as u16 != definition.slots
            {
                return Err("star resource reply has a different identity".into());
            }
        }
        e.refill
            .as_mut()
            .ok_or("star lookup continuation absent")?
            .acknowledge(
                pool::Request::LookupStar,
                pool::Reply::Resource(definition.as_ref().map(|_| pool::Resource(1))),
            )?;
        e.star_resource = definition;
        e.external = None;
        Ok(())
    }
    pub fn complete_selection_factory_graphic(
        &mut self,
        p: &Program,
        id: ObjectId,
    ) -> Result<(), String> {
        let e = service_mut(&mut self.native_selection, &mut self.deletion)
            .ok_or("native selection absent")?;
        if e.external != Some(Request::FactoryGraphic(id)) || !e.started {
            return Err("stale star graphic reply".into());
        }
        if let Some(g) = &mut e.group_factory {
            g.complete_graphic(&mut self.vm, p, id)?;
        } else {
            e.factory
                .as_mut()
                .ok_or("star factory absent")?
                .complete_graphic(&mut self.vm, p, id)?;
        }
        e.external = None;
        e.started = false;
        Ok(())
    }
    pub fn refresh_selection_person(
        &mut self,
        p: &Program,
        id: ObjectId,
        budget: usize,
    ) -> Result<bool, String> {
        if self.pending_native_selection()
            != Some((
                Request::Graphic {
                    object: id,
                    call: indicator::GraphicCall::Refresh,
                    placement: false,
                },
                true,
            ))
        {
            return Err("selection person refresh has no owning request".into());
        }
        let mut remaining = budget;
        self.drive_character_refresh(p, id, &mut remaining, false)
    }
}

fn service_mut<'a>(
    standalone: &'a mut Option<Execution>,
    deletion: &'a mut [crate::removal::Deletion],
) -> Option<&'a mut Execution> {
    standalone
        .as_mut()
        .or_else(|| deletion.last_mut()?.entry.as_mut()?.selection.as_mut())
}

#[cfg(test)]
#[path = "lot_selection/notification_view_refresh_tests.rs"]
mod notification_view_refresh_tests;
