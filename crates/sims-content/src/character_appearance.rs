//! Original profile resource services for 5888d3. Records/geometry and texture
//! notifications remain ordered caller services; no request is acknowledged
//! merely because its asset name was resolved.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::pose::{
    appearance::Records,
    appearance_reload::{Owner, Reload, Request as ReloadRequest},
    appearance_selection::{Request as SelectionRequest, Selection},
};
use sims_vm::{Definition, ObjectId, Program, Vm};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Record(ReloadRequest),
    Selection(SelectionRequest),
}

/// Complete the shared native image-cache portion of an appearance notification.
/// Pixel allocation and the final image destructor belong to the presenter.
pub(crate) fn notify_images(vm: &mut Vm, id: ObjectId, request: &Request) -> Result<()> {
    if let Request::Selection(SelectionRequest::TextureNotification { first }) = request {
        let guid = vm
            .world
            .objects
            .get(&id)
            .and_then(|o| o.definition.as_ref())
            .context("appearance notification resource absent")?
            .guid;
        vm.world
            .image_cache
            .notify(guid, *first)
            .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    pub object: ObjectId,
    pub owner: Owner,
    reload: Reload,
    selection: Option<Selection>,
    profile: Option<Definition>,
    pending: Option<Request>,
    bound_placement_started: bool,
    #[serde(default)]
    clothing_used: bool,
}
impl Service {
    pub fn begin(object: ObjectId, mut owner: Owner) -> Self {
        let reload = Reload::begin(&mut owner);
        Self {
            object,
            owner,
            reload,
            selection: None,
            profile: None,
            pending: None,
            bound_placement_started: false,
            clothing_used: false,
        }
    }
    pub fn is_complete(&self) -> bool {
        self.reload.is_complete() && self.pending.is_none()
    }
    pub fn clothing_used(&self) -> bool {
        self.clothing_used
    }
    pub(crate) fn validate_profile_cache(
        &self,
        profiles: &sims_vm::body_profiles::State,
    ) -> Result<()> {
        if let Some(definition) = &self.profile {
            profiles.get(definition).map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    /// Drive a model-present reload with actual record assets and geometry.
    /// Presentation notifications, tint and auxiliary-layer detachment remain
    /// explicit services; decoded names alone never acknowledge mesh changes.
    pub fn advance_bound(
        &mut self,
        index: &crate::Index,
        vm: &mut Vm,
        program: &Program,
        avatar: &mut crate::saved_avatar::BoundAvatar,
        geometry: &mut crate::character_geometry::CharacterGeometry,
        bound: &mut sims_sim::pose::model::Bound,
        placement: &mut sims_sim::pose::transform::Placement,
        pool: &mut crate::character_geometry::MeshPool,
    ) -> Result<Option<Request>> {
        self.advance_bound_with_clothing(
            index,
            vm,
            program,
            avatar,
            geometry,
            bound,
            placement,
            pool,
            &mut crate::clothing_catalog::Unavailable,
        )
    }
    pub fn advance_bound_with_clothing(
        &mut self,
        index: &crate::Index,
        vm: &mut Vm,
        program: &Program,
        avatar: &mut crate::saved_avatar::BoundAvatar,
        geometry: &mut crate::character_geometry::CharacterGeometry,
        bound: &mut sims_sim::pose::model::Bound,
        placement: &mut sims_sim::pose::transform::Placement,
        pool: &mut crate::character_geometry::MeshPool,
        clothing: &mut impl crate::clothing_catalog::Provider,
    ) -> Result<Option<Request>> {
        use sims_sim::pose::appearance::{Execution, Input};
        ensure!(
            self.owner.model_present,
            "bound appearance reload has no model"
        );
        // Reload::begin already wrote +d9. Publish it to the placement view
        // before record callbacks, then keep the shared word synchronized.
        if self.bound_placement_started {
            self.owner.selection.dirty = placement.dirty;
        } else {
            placement.dirty = self.owner.selection.dirty;
            self.bound_placement_started = true;
        }
        loop {
            let request = self.advance_with_clothing(vm, program, &mut avatar.records, clothing)?;
            placement.dirty = self.owner.selection.dirty;
            let Some(request) = request else {
                return Ok(None);
            };
            let (record, execution) = match request {
                Request::Record(ReloadRequest::Detach(id)) => {
                    geometry.detach_auxiliary(index, avatar, id, bound, pool)?;
                    self.acknowledge(&mut avatar.records)?;
                    continue;
                }
                Request::Record(
                    ReloadRequest::ClearRecord(record) | ReloadRequest::DestroyAccessory(record),
                ) => (record, Execution::clear()),
                Request::Selection(SelectionRequest::Bind { record, descriptor }) => (
                    record,
                    Execution::bind(Input::Replace {
                        name: descriptor.name,
                        texture: descriptor.texture,
                    }),
                ),
                other => return Ok(Some(other)),
            };
            geometry.apply_record(index, avatar, record, execution, bound, pool)?;
            self.acknowledge(&mut avatar.records)?;
        }
    }
    pub fn advance(
        &mut self,
        vm: &mut Vm,
        program: &Program,
        records: &mut Records,
    ) -> Result<Option<Request>> {
        self.advance_with_clothing(
            vm,
            program,
            records,
            &mut crate::clothing_catalog::Unavailable,
        )
    }
    pub fn advance_with_clothing(
        &mut self,
        vm: &mut Vm,
        program: &Program,
        records: &mut Records,
        clothing: &mut impl crate::clothing_catalog::Provider,
    ) -> Result<Option<Request>> {
        if let Some(request) = &self.pending {
            return Ok(Some(request.clone()));
        }
        loop {
            let object = vm
                .world
                .objects
                .get_mut(&self.object)
                .context("appearance object absent")?;
            let person = object.person.as_mut().context("appearance person absent")?;
            let Some(request) = self
                .reload
                .next(&mut self.owner, records, person)
                .map_err(anyhow::Error::msg)?
            else {
                return Ok(None);
            };
            let output = match request {
                ReloadRequest::BindProfile => {
                    let definition = object
                        .definition
                        .as_ref()
                        .context("appearance definition absent")?;
                    vm.world
                        .body_profiles
                        .load(program, definition)
                        .map_err(anyhow::Error::msg)?
                        .context("appearance requires a person definition")?;
                    profile(program, &vm.world.body_profiles, definition)?;
                    self.profile = Some(definition.clone());
                    self.reload
                        .acknowledge(&mut self.owner, records)
                        .map_err(anyhow::Error::msg)?;
                    continue;
                }
                ReloadRequest::SelectAppearance => {
                    let (_, _, flags) = profile(
                        program,
                        &vm.world.body_profiles,
                        self.profile
                            .as_ref()
                            .context("appearance profile not bound")?,
                    )?;
                    if self.selection.is_none() {
                        let selected = crate::clothing_profile::select_appearance_with_career(
                            program,
                            &mut vm.world.body_profiles,
                            self.profile.as_ref().unwrap(),
                            &mut self.owner.selection,
                            person,
                            flags,
                            false,
                            &mut object.career_appearance,
                            clothing,
                        )?;
                        self.clothing_used |= selected.clothing_used;
                        self.selection = Some(selected);
                    }
                    if let Some(request) = self
                        .selection
                        .as_mut()
                        .unwrap()
                        .next(&self.owner.selection, records)
                    {
                        Request::Selection(request)
                    } else {
                        self.selection = None;
                        self.reload
                            .acknowledge(&mut self.owner, records)
                            .map_err(anyhow::Error::msg)?;
                        continue;
                    }
                }
                ReloadRequest::IdentityFields => {
                    let (strings, tones, _) = profile(
                        program,
                        &vm.world.body_profiles,
                        self.profile
                            .as_ref()
                            .context("appearance profile not bound")?,
                    )?;
                    sims_vm::person_reset::reload_profile_identity_fields(
                        person,
                        &strings.iter().map(|s| Some(s.as_str())).collect::<Vec<_>>(),
                        &tones.iter().map(String::as_str).collect::<Vec<_>>(),
                    )
                    .map_err(anyhow::Error::msg)?;
                    self.reload
                        .acknowledge(&mut self.owner, records)
                        .map_err(anyhow::Error::msg)?;
                    continue;
                }
                other => Request::Record(other),
            };
            self.pending = Some(output.clone());
            return Ok(Some(output));
        }
    }
    /// Call only after completing the returned asset/geometry/presentation work.
    pub fn acknowledge(&mut self, records: &mut Records) -> Result<()> {
        match self
            .pending
            .as_ref()
            .context("appearance service has no pending request")?
        {
            Request::Record(_) => self
                .reload
                .acknowledge(&mut self.owner, records)
                .map_err(anyhow::Error::msg)?,
            Request::Selection(_) => self
                .selection
                .as_mut()
                .context("appearance selection absent")?
                .acknowledge()
                .map_err(anyhow::Error::msg)?,
        }
        self.pending = None;
        Ok(())
    }
}
fn profile<'a>(
    program: &'a Program,
    profiles: &'a sims_vm::body_profiles::State,
    d: &Definition,
) -> Result<(&'a [String], &'a [String], u16)> {
    let module = program
        .modules
        .get(&d.source)
        .context("appearance profile module absent")?;
    let words = module
        .object_definition_words
        .get(&d.object)
        .context("appearance OBJD absent")?;
    ensure!(words.len() > 0xcc / 2, "appearance OBJD flags absent");
    let profile = profiles.get(d).map_err(anyhow::Error::msg)?;
    let tones = module
        .semiglobal
        .as_ref()
        .and_then(|s| program.modules.get(s))
        .and_then(|m| m.strings.get(&8192))
        .context("appearance skin-tone strings absent")?;
    Ok((profile, tones, words[0xcc / 2] as u16))
}
