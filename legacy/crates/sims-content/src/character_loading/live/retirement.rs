//! Actual record/geometry services for the native animator destructor. The
//! registry has already unlinked the person; its retained owner cannot run.
use super::*;
use sims_sim::pose::{animator::LayerId, appearance, appearance_reload, destruction};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub execution: destruction::Execution,
    pub state: destruction::State,
    clear: Option<appearance_reload::Reload>,
    profile_released: bool,
}
impl Job {
    fn begin(
        assets: &Assets,
        retired: &sims_sim::removal::character::Retired,
        refs: RetirementReferences,
    ) -> Self {
        let records = &assets.avatar.records;
        let rig = &retired.resident.character.rig;
        Self {
            execution: Default::default(),
            state: destruction::State {
                model: Some(u64::from(retired.resident.context.actor as u16)),
                profile: (!assets.model.needs_appearance.is_empty())
                    .then_some(u64::from(retired.resident.context.actor as u16)),
                layers: retired
                    .resident
                    .character
                    .skin
                    .animator
                    .destruction_layers()
                    .map(|id| id.map(|id| id.0)),
                anchors: [rig.forward, rig.head, rig.hand, rig.pelvis]
                    .map(|a| a.map(|a| a.bone as u64)),
                head: Some(records.head as u64),
                accessories: records
                    .accessories
                    .iter()
                    .map(|&id| Some(id as u64))
                    .collect(),
                hands: records.hands.map(|id| Some(id as u64)).to_vec(),
                bodies: records.body.map(|id| Some(id as u64)).to_vec(),
                generic: vec![Some(records.generic as u64)],
                selected: refs.selected,
                focus: refs.focus,
                base_focus: refs.base_focus,
            },
            clear: None,
            profile_released: false,
        }
    }

    pub fn validate(&self, records: &appearance::Records, owner: &Owner) -> Result<()> {
        self.execution
            .validate(&self.state)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            owner.model_present == self.state.model.is_some(),
            "retired model owner changed"
        );
        ensure!(
            self.clear.is_none()
                || self.execution.pending_request()
                    == Some(&destruction::Request::ClearAppearances),
            "retired clear lost its destructor request"
        );
        let check = |id: Option<u64>, expected: usize| -> Result<()> {
            if let Some(id) = id {
                ensure!(id == expected as u64, "retired record identity changed");
            } else {
                ensure!(
                    records.entries.get(expected) == Some(&appearance::Record::default()),
                    "destroyed record still owns data"
                );
            }
            Ok(())
        };
        check(self.state.head, records.head)?;
        ensure!(
            self.state.hands.len() == 2
                && self.state.bodies.len() == 2
                && self.state.generic.len() == 1,
            "retired record vector shape changed"
        );
        for (i, id) in records.hands.iter().enumerate() {
            check(self.state.hands[i], *id)?;
        }
        for (i, id) in records.body.iter().enumerate() {
            check(self.state.bodies[i], *id)?;
        }
        check(self.state.generic[0], records.generic)?;
        ensure!(
            self.state.accessories.len() == records.accessories.len(),
            "retired accessory vector changed"
        );
        for (i, id) in records.accessories.iter().enumerate() {
            check(self.state.accessories[i], *id)?;
        }
        Ok(())
    }
}

impl<P: Presentation> Host<P> {
    pub(super) fn validate_retirement_bindings(&self, simulation: &Simulation) -> Result<()> {
        for (&id, retired) in &simulation.retired_characters {
            let assets = self
                .characters
                .get(&id)
                .context("retired character assets absent")?;
            ensure!(
                !simulation.characters.contains_key(&id)
                    && !simulation.vm.world.objects.contains_key(&id),
                "retired character still registered"
            );
            ensure!(
                retired.asset_service_started == self.retirements.contains_key(&id),
                "retired character lost its asset continuation"
            );
            ensure!(
                !self.reloads.contains_key(&id)
                    && !self.accessories.contains_key(&id)
                    && !self.fresh.contains_key(&id),
                "retired character has a live asset service"
            );
            ensure!(
                retired.resident.source.as_ref() == Some(&assets.avatar.avatar.skeleton),
                "retired rig changed"
            );
            if let Some(job) = self.retirements.get(&id) {
                job.validate(&assets.avatar.records, &assets.appearance_owner)?;
                ensure!(
                    job.state.layers
                        == retired
                            .resident
                            .character
                            .skin
                            .animator
                            .destruction_layers()
                            .map(|id| id.map(|id| id.0)),
                    "retired animation pointers changed"
                );
                let rig = &retired.resident.character.rig;
                ensure!(
                    job.state.anchors
                        == [rig.forward, rig.head, rig.hand, rig.pelvis]
                            .map(|a| a.map(|a| a.bone as u64)),
                    "retired anchor pointers changed"
                );
                ensure!(
                    !job.profile_released || assets.model.needs_appearance.is_empty(),
                    "retired profile still retained"
                );
            }
        }
        ensure!(
            self.retirements
                .keys()
                .all(|id| simulation.retired_characters.contains_key(id)),
            "asset destructor lost its retired character"
        );
        Ok(())
    }

    pub(super) fn service_retirement(
        &mut self,
        simulation: &mut Simulation,
        program: &Program,
        id: ObjectId,
    ) -> Result<()> {
        if !self.retirements.contains_key(&id) {
            let refs = self.presentation.retirement_references(id)?;
            let assets = self
                .characters
                .get(&id)
                .context("retirement assets absent")?;
            let retired = &simulation.retired_characters[&id];
            ensure!(
                assets.appearance_owner.model_present,
                "retirement requires initialized model"
            );
            let job = Job::begin(assets, retired, refs);
            simulation
                .begin_character_retirement(id)
                .map_err(anyhow::Error::msg)?;
            self.retirements.insert(id, job);
            return Ok(());
        }
        self.validate_retirement_bindings(simulation)?;
        let mut job = self.retirements[&id].clone();
        let mut assets = self.characters[&id].clone();
        let mut retired = simulation.retired_characters[&id].clone();
        let mut pool = self.mesh_pool.clone();
        let mut diagnostic = None;
        if job.execution.pending_request().is_none() {
            let refs = self.presentation.retirement_references(id)?;
            job.state.selected = refs.selected;
            job.state.focus = refs.focus;
            job.state.base_focus = refs.base_focus;
        }
        let request = job
            .execution
            .next(&mut job.state)
            .map_err(anyhow::Error::msg)?;
        if job.state.accessories.is_empty() {
            assets.avatar.records.accessories.clear();
        }
        let Some(request) = request else {
            ensure!(
                job.execution.is_complete()
                    && !assets.appearance_owner.model_present
                    && assets.geometry.state.appearances.is_empty()
                    && assets.avatar.avatar.parts.is_empty()
                    && retired.resident.character.skin.animator.layers().is_empty()
                    && assets
                        .avatar
                        .records
                        .entries
                        .iter()
                        .all(|r| r == &appearance::Record::default()),
                "native character destruction retained live storage"
            );
            simulation
                .complete_character_retirement(id)
                .map_err(anyhow::Error::msg)?;
            self.retirements.remove(&id);
            self.characters.remove(&id);
            // Renderer ownership ends with the character. Do not let a later
            // reused script ID inherit pixels from a destroyed presentation.
            if let Some(presenter) = &mut self.notifications {
                presenter.saved.entries.remove(&id);
            }
            return Ok(());
        };
        let mut finished = true;
        match request {
            destruction::Request::Reset => {
                let now = sims_sim::pose::ordinary::clock(
                    retired.tick as i32,
                    retired.resident.controls.ordinary.clock_mode,
                );
                diagnostic = retired.resident.reset(now).map_err(anyhow::Error::msg)?;
                job.state.layers = retired
                    .resident
                    .character
                    .skin
                    .animator
                    .destruction_layers()
                    .map(|id| id.map(|id| id.0));
            }
            destruction::Request::ClearAppearances => {
                let clear = job
                    .clear
                    .get_or_insert_with(appearance_reload::Reload::clear);
                if let Some(next) = clear
                    .next(
                        &mut assets.appearance_owner,
                        &assets.avatar.records,
                        &mut retired.person,
                    )
                    .map_err(anyhow::Error::msg)?
                {
                    match &next {
                        appearance_reload::Request::ClearRecord(record) => {
                            assets.model.with_bound(&mut retired.resident, |bound| {
                                assets.geometry.apply_record(
                                    &self.index,
                                    &mut assets.avatar,
                                    *record,
                                    appearance::Execution::clear(),
                                    bound,
                                    &mut pool,
                                )
                            })?;
                        }
                        appearance_reload::Request::Detach(binding) => {
                            assets.model.with_bound(&mut retired.resident, |bound| {
                                assets.geometry.detach_auxiliary(
                                    &self.index,
                                    &mut assets.avatar,
                                    *binding,
                                    bound,
                                    &mut pool,
                                )
                            })?;
                        }
                        _ => anyhow::bail!("unexpected destructor appearance request {next:?}"),
                    }
                    clear
                        .acknowledge(&mut assets.appearance_owner, &mut assets.avatar.records)
                        .map_err(anyhow::Error::msg)?;
                    finished = false;
                } else {
                    job.clear = None;
                    retired.resident.character.censor = Censor {
                        binding: assets.appearance_owner.attachment,
                        mask: assets.appearance_owner.attachment_metadata as i32,
                    };
                }
            }
            destruction::Request::ReleaseProfile(_) => {
                // The host retains the resolved needs-prototype name; immutable
                // catalog data and shared mesh caches have independent owners.
                assets.model.needs_appearance.clear();
                job.profile_released = true;
            }
            destruction::Request::DetachLayer(layer) => retired
                .resident
                .character
                .skin
                .animator
                .detach_for_destruction(LayerId(layer))
                .map_err(anyhow::Error::msg)?,
            destruction::Request::DestroyAnchor(bone) => {
                // Read the first remaining wrapper in native destruction order;
                // several wrappers can refer to the same underlying bone.
                let rig = &mut retired.resident.character.rig;
                let anchor = [
                    &mut rig.forward,
                    &mut rig.head,
                    &mut rig.hand,
                    &mut rig.pelvis,
                ]
                .into_iter()
                .find(|a| a.is_some())
                .context("retired anchor absent")?;
                ensure!(
                    anchor.unwrap().bone as u64 == bone,
                    "retired anchor identity changed"
                );
                *anchor = None;
            }
            destruction::Request::DestroyModel(_) => {
                ensure!(
                    assets.geometry.state.appearances.is_empty()
                        && assets.geometry.appearances.is_empty()
                        && assets.avatar.avatar.parts.is_empty(),
                    "model destruction requires cleared appearance instances"
                );
                retired
                    .resident
                    .character
                    .skin
                    .animator
                    .release_model_layers()
                    .map_err(anyhow::Error::msg)?;
                self.presentation
                    .retire(id, RetirementEvent::ModelRemoved)?;
                assets.appearance_owner.model_present = false;
            }
            destruction::Request::DestroyRecord(record) => {
                assets.model.with_bound(&mut retired.resident, |bound| {
                    assets.geometry.apply_record(
                        &self.index,
                        &mut assets.avatar,
                        record as usize,
                        appearance::Execution::clear(),
                        bound,
                        &mut pool,
                    )
                })?;
                assets.avatar.records.entries[record as usize] = Default::default();
            }
            destruction::Request::ClearSelected => self
                .presentation
                .retire(id, RetirementEvent::ClearSelected)?,
            destruction::Request::ClearFocus => {
                self.presentation.retire(id, RetirementEvent::ClearFocus)?
            }
            destruction::Request::ClearBaseFocus => self
                .presentation
                .retire(id, RetirementEvent::ClearBaseFocus)?,
            destruction::Request::BaseReset => ensure!(
                job.state.model.is_none(),
                "base destructor still owns a model"
            ),
            destruction::Request::BaseClear => {
                // The represented SAnimator has one base generic record and no
                // base auxiliary records; derived accessories were destroyed above.
                ensure!(
                    assets.avatar.records.accessories.is_empty(),
                    "base destructor retained derived accessories"
                );
            }
            destruction::Request::Storage(_) => (), // Rust owns these vector/string allocations.
        }
        if finished {
            job.execution
                .acknowledge(&mut job.state, &request)
                .map_err(anyhow::Error::msg)?;
        }
        retired
            .resident
            .character
            .validate(program)
            .map_err(anyhow::Error::msg)?;
        job.validate(&assets.avatar.records, &assets.appearance_owner)?;
        if let Some(value) = diagnostic {
            simulation
                .native_lot
                .as_mut()
                .context("retired character lot absent")?
                .diagnostics
                .error = value;
        }
        self.retirements.insert(id, job);
        self.characters.insert(id, assets);
        simulation.retired_characters.insert(id, retired);
        self.mesh_pool = pool;
        Ok(())
    }
}
