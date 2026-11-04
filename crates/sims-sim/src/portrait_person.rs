//! Live 4e9652 owner. Construction, one-pass Find Location, person +88 movement
//! and failed-search destruction remain synchronous with the calling lot tick.
//! Resource readiness and the actual camera projection are explicit host inputs.
use crate::lot_removal::immediate::Removal as Cleanup;
use crate::{
    Simulation, Step, lot_acceptance, lot_placement,
    placement::{SearchTile, SearchView, Spiral, search_tile_allowed},
};
use serde::{Deserialize, Serialize};
use sims_vm::{
    Definition, ObjectId, Program, ThreadId, Vm,
    image_cache::temporary::{Request, Response, Search as Descriptor, Temporary},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Resource { guid: u32 },
    CameraOrigin,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Service(Service),
    Yielded,
    Complete(Option<ObjectId>),
}

fn validate_definition(p: &Program, d: &Definition) -> Result<(), String> {
    let words = p
        .modules
        .get(&d.source)
        .and_then(|m| m.object_definition_words.get(&d.object))
        .filter(|w| w.len() > 65)
        .ok_or("portrait person definition not loaded")?;
    if words[9] != 2
        || words[10] != 0
        || words[65] < 0
        || words[20] as u16 != d.slots
        || (u32::from(words[14] as u16) | (u32::from(words[15] as u16) << 16)) != d.guid
    {
        return Err("portrait resource is not a single-part person definition".into());
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Search {
    view: SearchView,
    projected: [i32; 2],
    size: i32,
    center: [i32; 2],
    room: i16,
    cursor: Spiral,
    candidate: Option<[i32; 2]>,
    acceptance: Option<lot_acceptance::live::Execution>,
    found: Option<bool>,
}
impl Search {
    fn new(s: &Simulation, view: SearchView, projected: [i32; 2]) -> Result<Self, String> {
        let lot = s
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .ok_or("portrait search architecture absent")?;
        let size = lot.size as i32;
        let center = view.origin(size, projected);
        let cursor = Spiral::new(size, center)?;
        let [column, row] = cursor.center_tile();
        let room = lot
            .floor(1)
            .ok_or("portrait search ground floor absent")?
            .rooms
            .primary[column as usize * lot.size + row as usize];
        Ok(Self {
            view,
            projected,
            size,
            center,
            room,
            cursor,
            candidate: None,
            acceptance: None,
            found: None,
        })
    }
    fn validate(&self, s: &Simulation, p: &Program, id: ObjectId) -> Result<(), String> {
        if s.architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .is_none_or(|a| a.size as i32 != self.size)
            || self.center != self.view.origin(self.size, self.projected)
            || self.cursor.direction() != -1
            || self
                .candidate
                .is_some_and(|point| self.cursor.last_point() != Some(point))
            || self.acceptance.is_some() && self.candidate.is_none()
            || self.found == Some(true)
                && self
                    .acceptance
                    .as_ref()
                    .and_then(|a| a.outcome())
                    .is_none_or(|o| !o.accepted)
            || self.found == Some(false) && (self.candidate.is_some() || self.acceptance.is_some())
        {
            return Err("invalid temporary portrait search".into());
        }
        self.cursor.validate(self.size, self.center)?;
        if self.found == Some(false) && self.cursor.clone().next().is_some() {
            return Err("portrait search stopped before exhaustion".into());
        }
        if let Some(a) = &self.acceptance {
            a.validate(&s.vm, p)?;
            if a.binding() != (id, self.candidate.unwrap(), 1) {
                return Err("portrait acceptance binding mismatch".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Execution {
    tick: u64,
    temporary: Temporary,
    resource: Option<(u32, Definition)>,
    created: Option<ObjectId>,
    search: Option<Search>,
    movement: Option<lot_placement::live::Execution>,
    cleanup: Option<Cleanup>,
}
impl Execution {
    pub fn index(&self) -> u32 {
        self.temporary.index()
    }
    pub fn returned(&self) -> Option<Option<ObjectId>> {
        self.temporary
            .returned()
            .map(|id| id.map(|id| id as ObjectId))
    }
    pub fn created(&self) -> Option<ObjectId> {
        self.created
    }
    pub fn request(&self) -> Result<Option<Request>, String> {
        self.temporary.request()
    }
    pub(crate) fn owns_deletion(&self, id: ObjectId) -> bool {
        self.cleanup.as_ref().is_some_and(|c| c.owns_deletion(id))
    }
    pub(crate) fn owns_callback(
        &self,
        vm: &Vm,
        p: &Program,
        thread: ThreadId,
        depth: usize,
    ) -> bool {
        self.search
            .as_ref()
            .and_then(|s| s.acceptance.as_ref())
            .is_some_and(|a| a.owns_callback(vm, p, thread, depth))
            || self
                .movement
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self
                .cleanup
                .as_ref()
                .is_some_and(|c| c.owns_callback(vm, p, thread, depth))
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        self.temporary.validate(&s.vm.world.image_cache)?;
        let request = self.temporary.request()?;
        if self.tick != s.vm.tick
            || s.native_lot.is_none()
            || s.household.is_none()
            || self.created.is_some_and(|id| id <= 0)
            || self.temporary.created() != 0
                && self.created.map(|id| id as u32) != Some(self.temporary.created())
            || self.created.is_some() && self.resource.is_none()
            || self.search.is_some() && self.created.is_none()
            || self.movement.is_some() && self.search.as_ref().is_none_or(|s| s.found != Some(true))
            || self.cleanup.is_some()
                && (self.movement.is_some()
                    || self.search.as_ref().is_none_or(|s| s.found != Some(false)))
            || request.is_none() && self.returned().is_none()
            || self
                .returned()
                .flatten()
                .is_some_and(|id| self.created != Some(id) || self.movement.is_none())
            || self.created.is_some() && self.returned() == Some(None) && self.cleanup.is_none()
            || matches!(request, Some(Request::Create { .. })) && self.resource.is_none()
        {
            return Err("invalid live temporary portrait owner".into());
        }
        if let Some((handle, definition)) = &self.resource {
            if *handle == 0 || definition.guid != self.temporary.guid() {
                return Err("invalid portrait resource identity".into());
            }
            validate_definition(p, definition)?;
            if matches!(request, Some(Request::Resource { .. }))
                || matches!(request, Some(Request::Create { resource }) if resource != *handle)
            {
                return Err("portrait resource continuation mismatch".into());
            }
        }
        if let Some(id) = self.created {
            let destroying = self.cleanup.as_ref().is_some_and(|c| c.deleting);
            if !destroying {
                let f = s
                    .person_factories
                    .get(&id)
                    .ok_or("portrait factory absent")?;
                if f.parent.is_some()
                    || s.vm
                        .world
                        .objects
                        .get(&id)
                        .and_then(|o| o.definition.as_ref())
                        != self.resource.as_ref().map(|r| &r.1)
                    || !matches!(request, Some(Request::Create { .. })) && !f.finalized
                {
                    return Err("portrait factory binding mismatch".into());
                }
            }
            match request {
                Some(Request::Person { id: selected }) if selected != id => {
                    return Err("portrait lookup identity mismatch".into());
                }
                Some(Request::Find { person, .. } | Request::Move { person, .. })
                    if person != id as u32 =>
                {
                    return Err("portrait person identity mismatch".into());
                }
                Some(Request::Remove { created }) if created != id as u32 => {
                    return Err("portrait cleanup identity mismatch".into());
                }
                _ => {}
            }
            if let Some(search) = &self.search {
                search.validate(s, p, id)?;
            }
            if let Some(movement) = &self.movement {
                movement.validate(&s.vm, p, id)?;
                if movement.operation.binding()
                    != crate::lot_route::movement::commit_operation(
                        id,
                        self.search.as_ref().unwrap().candidate.unwrap(),
                        1,
                    )
                    .binding()
                    || self.returned().is_some() && !movement.complete
                {
                    return Err("portrait move binding mismatch".into());
                }
            }
            if let Some(c) = &self.cleanup {
                if c.target != id {
                    return Err("portrait cleanup binding mismatch".into());
                }
                c.validate(s, p, self.returned().is_some())?;
            }
        }
        Ok(())
    }
}

impl Simulation {
    pub fn begin_temporary_portrait_person(&mut self, index: u32) -> Result<(), String> {
        if self.portrait_person.is_some()
            || self.cursor_work_pending()
            || self.synchronous_removal.is_some()
            || self.ground_placement.is_some()
            || self.object_placement.is_some()
            || self.vm.player_menu.is_some()
            || !self.deletion.is_empty()
            || self.person_factories.values().any(|s| !s.finalized)
            || self.native_lot.is_none()
            || self.household.is_none()
        {
            return Err("temporary portrait requires an available native household owner".into());
        }
        let mut temporary = Temporary::new(index);
        temporary.advance(&self.vm.world.image_cache)?;
        self.portrait_person = Some(Execution {
            tick: self.vm.tick,
            temporary,
            resource: None,
            created: None,
            search: None,
            movement: None,
            cleanup: None,
        });
        Ok(())
    }
    /// Supply the native manager's resource lookup after its readiness/name
    /// guards, with the exact definition loaded into Program. None means a
    /// missing resource; an unresolved host service is an error, not None.
    pub fn supply_portrait_person_resource(
        &mut self,
        p: &Program,
        resource: Option<(u32, Definition)>,
    ) -> Result<(), String> {
        let e = self
            .portrait_person
            .as_ref()
            .ok_or("temporary portrait owner absent")?;
        let Some(Request::Resource { guid }) = e.temporary.request()? else {
            return Err("portrait is not awaiting a resource".into());
        };
        if let Some((handle, definition)) = &resource {
            validate_definition(p, definition)?;
            if *handle == 0 || definition.guid != guid {
                return Err("portrait resource is not the requested person definition".into());
            }
        }
        let e = self.portrait_person.as_mut().unwrap();
        e.temporary.respond(
            &self.vm.world.image_cache,
            Response::Resource(resource.as_ref().map(|r| r.0)),
        )?;
        e.resource = resource;
        Ok(())
    }
    /// The camera is sampled at Find Location, after construction. It must
    /// project SearchView::midpoint with mode one; retain the supplied result
    /// across budget splits rather than resampling a moving UI camera.
    pub fn supply_portrait_person_camera(
        &mut self,
        view: SearchView,
        projected: [i32; 2],
    ) -> Result<(), String> {
        let e = self
            .portrait_person
            .as_ref()
            .ok_or("temporary portrait owner absent")?;
        if e.search.is_some() || !matches!(e.temporary.request()?, Some(Request::Find { .. })) {
            return Err("portrait is not awaiting a camera origin".into());
        }
        let search = Search::new(self, view, projected)?;
        self.portrait_person.as_mut().unwrap().search = Some(search);
        Ok(())
    }
    pub fn take_temporary_portrait_person(&mut self) -> Result<Option<ObjectId>, String> {
        let result = self
            .portrait_person
            .as_ref()
            .and_then(|e| e.returned())
            .ok_or("temporary portrait is incomplete")?;
        self.portrait_person = None;
        Ok(result)
    }
    pub fn drive_temporary_portrait_person(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Progress, String> {
        self.portrait_person
            .as_ref()
            .ok_or("temporary portrait owner absent")?
            .validate(self, p)?;
        let mut e = self.portrait_person.take().unwrap();
        let result = self.drive_temporary_portrait_inner(p, &mut e, remaining, step);
        self.portrait_person = Some(e);
        self.sync_spatial_world();
        self.validate_native_lot()?;
        result
    }
    fn drive_temporary_portrait_inner(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Progress, String> {
        loop {
            if let Some(result) = e.returned() {
                return Ok(Progress::Complete(result));
            }
            if *remaining == 0 {
                return Ok(Progress::Yielded);
            }
            let request = e
                .temporary
                .advance(&self.vm.world.image_cache)?
                .ok_or("incomplete portrait without service")?;
            let response = match request {
                Request::Resource { guid } => {
                    return Ok(Progress::Service(Service::Resource { guid }));
                }
                Request::Create { resource } => {
                    let (handle, definition) =
                        e.resource.as_ref().ok_or("portrait definition absent")?;
                    if resource != *handle {
                        return Err("portrait resource changed".into());
                    }
                    if e.created.is_none() {
                        e.created = self.begin_direct_person_factory(p, definition.clone())?;
                    }
                    if let Some(id) = e.created {
                        if !self.drive_direct_person_factory(p, id, remaining, step)? {
                            return Ok(Progress::Yielded);
                        }
                    }
                    Response::Created(e.created.map_or(0, |id| id as u32))
                }
                Request::Person { id } => Response::Person(
                    self.vm
                        .world
                        .objects
                        .get(&id)
                        .filter(|o| o.person.is_some())
                        .map(|_| id as u32),
                ),
                Request::Find {
                    person,
                    descriptor,
                    output: _,
                } => {
                    if descriptor != Descriptor::default() {
                        return Err("unsupported portrait search descriptor".into());
                    }
                    let Some(search) = &mut e.search else {
                        return Ok(Progress::Service(Service::CameraOrigin));
                    };
                    if !self.drive_portrait_search(
                        p,
                        person as ObjectId,
                        search,
                        remaining,
                        step,
                    )? {
                        return Ok(Progress::Yielded);
                    }
                    Response::Found {
                        found: search.found.unwrap(),
                        position: search.candidate.unwrap_or([0; 2]),
                        anchored_level: None,
                    }
                }
                Request::Move {
                    person,
                    position,
                    level,
                    flags,
                } => {
                    if flags != [0; 2] {
                        return Err("unsupported portrait movement flags".into());
                    }
                    let id = person as ObjectId;
                    if e.movement.is_none() {
                        e.movement = Some(lot_placement::live::Execution::new(
                            crate::lot_route::movement::commit_operation(id, position, level),
                        ));
                    }
                    if !self.drive_live_placement(
                        p,
                        e.movement.as_mut().unwrap(),
                        remaining,
                        step,
                    )? {
                        return Ok(Progress::Yielded);
                    }
                    // +88 is base SetObject(notify=1), then 45e43d, with no
                    // second acceptance or the script wrapper's +74 refresh.
                    self.update_original_person_room(id)?;
                    Response::Done
                }
                Request::Remove { created } => {
                    let id =
                        ObjectId::try_from(created).map_err(|_| "invalid portrait removal ID")?;
                    if e.cleanup.is_none() {
                        e.cleanup = Some(Cleanup::new(self, p, id)?);
                    }
                    let c = e.cleanup.as_mut().unwrap();
                    if !self.drive_immediate_removal(p, c, remaining, step)? {
                        return Ok(Progress::Yielded);
                    }
                    Response::Done
                }
            };
            e.temporary.respond(&self.vm.world.image_cache, response)?;
        }
    }
    fn drive_portrait_search(
        &mut self,
        p: &Program,
        id: ObjectId,
        e: &mut Search,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        loop {
            if e.found.is_some() {
                return Ok(true);
            }
            if e.candidate.is_none() {
                let lot = self
                    .architecture
                    .as_ref()
                    .unwrap()
                    .original
                    .as_ref()
                    .unwrap();
                let floor = lot.floor(1).unwrap();
                let n = self.native_lot.as_ref().unwrap();
                for point in e.cursor.by_ref() {
                    let cell = (point[1] >> 4) as usize * lot.size + (point[0] >> 4) as usize;
                    let tile = SearchTile {
                        segments: floor.source.walls[cell].segments,
                        head: n.placement.links.heads[0][cell],
                        room: floor.rooms.primary[cell],
                        pool: lot.pool[cell],
                        ground_flags: n.placement.flags[0][cell],
                    };
                    if search_tile_allowed(tile, 1, 1, Some(e.room), false) {
                        e.candidate = Some(point);
                        break;
                    }
                }
                if e.candidate.is_none() {
                    e.found = Some(false);
                    return Ok(true);
                }
            }
            if e.acceptance.is_none() {
                e.acceptance = Some(lot_acceptance::live::Execution::new(
                    self,
                    id,
                    e.candidate.unwrap(),
                    1,
                )?);
            }
            if !self.drive_native_acceptance(p, e.acceptance.as_mut().unwrap(), remaining, step)? {
                return Ok(false);
            }
            if e.acceptance.as_ref().unwrap().outcome().unwrap().accepted {
                e.found = Some(true);
                return Ok(true);
            }
            e.acceptance = None;
            e.candidate = None;
        }
    }
}
