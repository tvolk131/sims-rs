//! Original resource services for fresh 588dcb/58831d character initialization.
//! The caller supplies pre-deserialization person storage and physical/view
//! services. This never initializes from an already-running saved Main frame.
use crate::Index;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::pose::{
    animator::Settings,
    initialize::{Initialization, Request},
    properties,
};
use sims_vm::{ObjectId, Program, Vm};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prepared {
    pub object: ObjectId,
    pub execution: Initialization,
    pub appearance_owner: sims_sim::pose::appearance_reload::Owner,
    source: String,
}
impl Prepared {
    /// The native post-constructor binds its profile before rebuilding the
    /// model and before reading saved motives/person words. Register the fresh
    /// object first; saved script frames must not have been installed yet.
    pub fn begin(
        index: &Index,
        vm: &mut Vm,
        program: &Program,
        object: ObjectId,
        settings: Settings,
        crt: &mut u32,
    ) -> Result<Self> {
        Self::begin_inner(index, vm, program, object, settings, crt, None, None)
    }
    /// Live construction uses the process resource owner. `begin` above is the
    /// isolated, already-loaded-prototype diagnostic used by native rig probes.
    pub fn begin_with_resources(
        index: &Index,
        vm: &mut Vm,
        program: &Program,
        object: ObjectId,
        settings: Settings,
        crt: &mut u32,
        resources: &mut crate::skeleton_resources::Cache,
    ) -> Result<Self> {
        Self::begin_with_startup(index, vm, program, object, settings, crt, resources, None)
    }

    pub(crate) fn begin_with_startup(
        index: &Index,
        vm: &mut Vm,
        program: &Program,
        object: ObjectId,
        settings: Settings,
        crt: &mut u32,
        resources: &mut crate::skeleton_resources::Cache,
        startup: Option<&mut crate::startup_memo::Memo>,
    ) -> Result<Self> {
        Self::begin_inner(
            index,
            vm,
            program,
            object,
            settings,
            crt,
            Some(resources),
            startup,
        )
    }
    fn begin_inner(
        index: &Index,
        vm: &mut Vm,
        program: &Program,
        object: ObjectId,
        settings: Settings,
        crt: &mut u32,
        resources: Option<&mut crate::skeleton_resources::Cache>,
        startup: Option<&mut crate::startup_memo::Memo>,
    ) -> Result<Self> {
        ensure!(
            !vm.object_threads.contains_key(&object),
            "fresh animator initialization must precede saved script installation"
        );
        let definition = vm
            .world
            .objects
            .get(&object)
            .and_then(|o| o.definition.as_ref())
            .context("initializer definition absent")?
            .clone();
        let module = program
            .modules
            .get(&definition.source)
            .context("initializer module absent")?;
        let words = module
            .object_definition_words
            .get(&definition.object)
            .context("initializer OBJD absent")?;
        let profile = vm
            .world
            .body_profiles
            .load(program, &definition)
            .map_err(anyhow::Error::msg)?
            .context("initializer requires a person definition")?
            .to_vec();
        let name = profile
            .first()
            .context("initializer skeleton name absent")?;
        let flags = *words
            .get(0xcc / 2)
            .context("initializer person flags absent")? as u8;
        let skeleton = match resources {
            Some(resources) => resources.load_with_startup(index, name, crt, startup)?,
            None => crate::avatar::load_skeleton_with_startup(index, name, startup)?.0,
        };
        ensure!(
            skeleton.name.eq_ignore_ascii_case(name),
            "initializer skeleton name mismatch"
        );
        // 588dcb calls 5888d3 before constructing the model. Execute its
        // no-model lifecycle; clearing fresh empty records has no host effects.
        let mut appearance = crate::character_appearance::Service::begin(
            object,
            sims_sim::pose::appearance_reload::Owner::constructed(false),
        );
        let mut records = sims_sim::pose::appearance::Records::default();
        while let Some(request) = appearance.advance(vm, program, &mut records)? {
            let crate::character_appearance::Request::Record(
                sims_sim::pose::appearance_reload::Request::ClearRecord(record),
            ) = request
            else {
                anyhow::bail!("fresh no-model appearance reload requested {request:?}");
            };
            ensure!(
                records.entries[record].binding.is_none(),
                "fresh appearance unexpectedly bound"
            );
            appearance.acknowledge(&mut records)?;
        }
        ensure!(appearance.is_complete());
        let mut appearance_owner = appearance.owner;
        appearance_owner.model_present = true;
        let execution =
            Initialization::new(skeleton, &profile, vm.tick as i32, flags, settings, crt)
                .map_err(anyhow::Error::msg)?;
        Ok(Self {
            object,
            execution,
            appearance_owner,
            source: definition.source,
        })
    }

    /// The second reload occurs after the model/rig exists. The caller must
    /// complete its binding, texture and material services before installing it.
    pub fn appearance_reload(&self) -> Result<crate::character_appearance::Service> {
        ensure!(
            self.execution.pending_request() == Some(&Request::ReloadProfile),
            "initializer is not awaiting profile reload"
        );
        Ok(crate::character_appearance::Service::begin(
            self.object,
            self.appearance_owner.clone(),
        ))
    }
    pub fn appearance_reloaded(
        &mut self,
        service: crate::character_appearance::Service,
    ) -> Result<()> {
        ensure!(
            service.object == self.object && service.is_complete() && service.owner.model_present,
            "appearance reload belongs to another initializer or is incomplete"
        );
        ensure!(
            self.execution.pending_request() == Some(&Request::ReloadProfile),
            "initializer is not awaiting profile reload"
        );
        self.appearance_owner = service.owner;
        self.execution
            .profile_reloaded()
            .map_err(anyhow::Error::msg)
    }

    /// Complete the pending native geometry service using this model's actual
    /// appearance bindings. The shared mesh pool is part of the host snapshot.
    pub fn refresh_geometry(
        &mut self,
        geometry: &mut crate::character_geometry::CharacterGeometry,
        pool: &mut crate::character_geometry::MeshPool,
    ) -> Result<()> {
        let Some(Request::Geometry { force }) = self.execution.pending_request() else {
            anyhow::bail!("initializer geometry is not pending");
        };
        let force = *force;
        geometry
            .state
            .refresh(&mut self.execution.bound, &mut pool.meshes, force)
            .map_err(anyhow::Error::msg)?;
        self.execution.geometry_done().map_err(anyhow::Error::msg)
    }

    /// Drive resource and direct field services until actual world/presentation
    /// work is pending. The returned request must be completed before retrying.
    pub fn advance(
        &mut self,
        index: &Index,
        vm: &mut Vm,
        program: &mut Program,
        context: properties::Context,
    ) -> Result<Option<Request>> {
        self.advance_with_startup(index, vm, program, context, None)
    }

    pub(crate) fn advance_with_startup(
        &mut self,
        index: &Index,
        vm: &mut Vm,
        program: &mut Program,
        context: properties::Context,
        mut startup: Option<&mut crate::startup_memo::Memo>,
    ) -> Result<Option<Request>> {
        loop {
            let person = vm
                .world
                .objects
                .get_mut(&self.object)
                .and_then(|o| o.person.as_mut())
                .context("initializer person absent")?;
            let Some(request) = self
                .execution
                .next(program, person, context)
                .map_err(anyhow::Error::msg)?
            else {
                return Ok(None);
            };
            match &request {
                Request::NeedsAppearance { name } => {
                    if let Some(startup) = startup.as_deref_mut() {
                        startup.find_appearance_catalog(index, name)?;
                    } else {
                        crate::avatar::find_appearance_catalog(index, name)?;
                    }
                    self.execution
                        .needs_appearance(name)
                        .map_err(anyhow::Error::msg)?;
                }
                Request::Animation { index: selection } => {
                    let person = vm.world.objects[&self.object].person.as_ref().unwrap();
                    let field = |n| {
                        person
                            .get(&n)
                            .copied()
                            .with_context(|| format!("initializer person field {n} absent"))
                    };
                    let name = sims_vm::animation::posture_name(
                        field(58)?,
                        field(64)?,
                        field(65)?,
                        *selection,
                        |table, i| {
                            let (source, table) = match table {
                                sims_vm::animation::NameTable::Private(t) => {
                                    (self.source.as_str(), t)
                                }
                                sims_vm::animation::NameTable::Global(t) => {
                                    (program.globals.as_str(), t)
                                }
                            };
                            program
                                .modules
                                .get(source)?
                                .strings
                                .get(&table)?
                                .get(i)
                                .map(String::as_str)
                        },
                    )
                    .map(str::to_ascii_lowercase);
                    if let Some(name) = &name
                        && !program.animations.contains_key(name)
                    {
                        crate::assets::load_named_animations(
                            index,
                            program,
                            &[name.clone()].into(),
                        )?;
                    }
                    if let Err(error) = self.execution.animation(name.as_deref(), program) {
                        // Re-read only immutable lookup metadata on failure.
                        // Missing captured STR# input must remain distinguishable
                        // from a named animation that could not be loaded.
                        let mut lookups = Vec::new();
                        sims_vm::animation::posture_name(
                            field(58)?,
                            field(64)?,
                            field(65)?,
                            *selection,
                            |table, i| {
                                let (source, table) = match table {
                                    sims_vm::animation::NameTable::Private(t) =>
                                        (self.source.as_str(), t),
                                    sims_vm::animation::NameTable::Global(t) =>
                                        (program.globals.as_str(), t),
                                };
                                let strings = program.modules.get(source)
                                    .and_then(|module| module.strings.get(&table));
                                lookups.push((source, table, i, strings.map(Vec::len)));
                                strings.and_then(|strings| strings.get(i)).map(String::as_str)
                            },
                        );
                        anyhow::bail!(
                            "{error}; actor {} source {} selection {} age {} special {} species {}; STR# lookups (source, table, index, resident length): {lookups:?}",
                            self.object, self.source, selection,
                            field(58)?, field(64)?, field(65)?,
                        );
                    }
                }
                Request::ZeroAltitude => {
                    // 59209f(0), original object +76 / object data 22.
                    vm.world
                        .objects
                        .get_mut(&self.object)
                        .unwrap()
                        .data
                        .insert(22, 0);
                    self.execution
                        .altitude_written()
                        .map_err(anyhow::Error::msg)?;
                }
                _ => return Ok(Some(request)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::startup_memo::{Memo, tests::Fixture};
    use sims_formats::avatar::{Bone, Skeleton};
    use std::{fs, sync::Arc};

    #[test]
    fn prepared_appearance_discovery_first_advance_uses_loading_metadata_without_rescanning() {
        let mut fixture = Fixture::new();
        fixture.write(
            "GameData_ALL_Recursive/Animation/adult.cmx",
            b"// synthetic\nversion 300\n0\n1\nadult-needs\n7\n0\n0\n0\n",
        );
        fixture.index = Arc::new(Index::scan(&fixture.root).unwrap());
        let mut startup = Memo::for_session(fixture.index.clone());
        startup.prepare_session().unwrap();
        let skeleton = Skeleton {
            name: "adult".into(),
            bones: vec![Bone {
                name: "root".into(),
                parent: None,
                translation: [0.; 3],
                rotation: [0., 0., 0., 1.],
                flags: [0; 3],
                properties: vec![],
                wiggle: Default::default(),
            }],
        };
        let mut crt = 1;
        let mut prepared = Prepared {
            object: 1,
            execution: Initialization::new(
                skeleton,
                &["adult", "", "", "", "", "", ""].map(str::to_owned),
                0,
                0,
                Settings {
                    fade_enabled: false,
                    fade_duration: 200,
                    sampling_config: 1,
                },
                &mut crt,
            )
            .unwrap(),
            appearance_owner: sims_sim::pose::appearance_reload::Owner::constructed(false),
            source: "synthetic".into(),
        };
        let mut vm = Vm::new(sims_vm::World::default(), 1);
        vm.world.objects.insert(
            1,
            sims_vm::Object {
                person: Some(Default::default()),
                ..Default::default()
            },
        );
        let mut program = Program::default();
        let context = properties::Context {
            actor: 1,
            sound_visible: false,
            selected: false,
            debug_sound: false,
            position_x: -999.,
            appearance_mode: 0,
            snow: false,
            lot_kind: 1,
        };
        let mut expected = prepared.clone();
        let mut ordinary = prepared.clone();
        assert_eq!(
            expected
                .advance(&fixture.index, &mut vm, &mut program, context)
                .unwrap(),
            Some(Request::ReloadProfile)
        );
        fs::remove_file(
            fixture
                .root
                .join("GameData_ALL_Recursive/Animation/adult.cmx"),
        )
        .unwrap();
        assert_eq!(
            prepared
                .advance_with_startup(
                    &fixture.index,
                    &mut vm,
                    &mut program,
                    context,
                    Some(&mut startup)
                )
                .unwrap(),
            Some(Request::ReloadProfile)
        );
        assert_eq!(prepared, expected);
        assert!(
            ordinary
                .advance(&fixture.index, &mut vm, &mut program, context)
                .is_err()
        );
        assert!(ordinary.execution.needs_appearance.is_none());
        assert_eq!(
            ordinary.execution.pending_request(),
            Some(&Request::NeedsAppearance {
                name: "adult-needs".into()
            })
        );
    }
}
