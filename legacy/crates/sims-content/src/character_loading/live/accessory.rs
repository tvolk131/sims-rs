//! Shared scripted and animation-event accessory lifecycle, with one saved
//! boundary per record operation and transactional geometry/resource services.
use super::*;
use sims_sim::pose::{accessory, appearance};
use sims_vm::{ThreadId, person::AccessoryCommand};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    Script {
        thread: ThreadId,
        command: AccessoryCommand,
    },
    Property(properties::Request),
}
impl Source {
    fn strings(&self) -> Result<(&str, Option<&str>)> {
        Ok(match self {
            Self::Script { command, .. } => (&command.name, (!command.remove).then_some("")),
            Self::Property(properties::Request::Dress { name, binding }) => (name, Some(binding)),
            Self::Property(properties::Request::Undress { name }) => (name, None),
            _ => anyhow::bail!("non-accessory property in accessory job"),
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub source: Source,
    pub execution: accessory::Execution,
}
impl Job {
    pub fn validate(&self, id: ObjectId) -> Result<()> {
        let (name, texture) = self.source.strings()?;
        ensure!(
            self.execution.matches(name, texture),
            "accessory job input changed"
        );
        if let Source::Script { command, .. } = &self.source {
            ensure!(id == command.caller, "accessory job caller changed");
        }
        Ok(())
    }
}
impl<P: Presentation> Host<P> {
    pub(super) fn validate_accessory_bindings(&self, simulation: &Simulation) -> Result<()> {
        ensure!(
            simulation.vm.native_accessory_host,
            "live accessory host disabled"
        );
        for (&id, job) in &self.accessories {
            job.validate(id)?;
            ensure!(
                self.characters.contains_key(&id) && simulation.characters.contains_key(&id),
                "accessory character absent"
            );
            job.execution
                .validate(&self.characters[&id].avatar.records)
                .map_err(anyhow::Error::msg)?;
            ensure!(
                !self.reloads.contains_key(&id),
                "accessory overlaps appearance reload"
            );
            match &job.source {
                Source::Script { thread, command } => {
                    ensure!(
                        matches!(simulation.vm.threads.get(thread).map(|t| &t.status),
                        Some(sims_vm::Status::AwaitingAccessory { request, origin, caller, name })
                        if *request == command.request && *origin == command.origin && *caller == id && *name == command.name),
                        "accessory VM continuation changed"
                    );
                }
                Source::Property(property) => {
                    ensure!(
                        simulation
                            .pending_character_services()
                            .contains(&(id, Request::Property(property.clone()))),
                        "accessory animation continuation changed"
                    );
                }
            }
        }
        Ok(())
    }
    pub(super) fn service_accessory(
        &mut self,
        simulation: &mut Simulation,
        id: ObjectId,
        source: Source,
    ) -> Result<()> {
        let mut job = if let Some(job) = self.accessories.get(&id) {
            ensure!(
                job.source == source,
                "accessory request changed during service"
            );
            job.clone()
        } else {
            let (name, texture) = source.strings()?;
            let execution = accessory::Execution::new(name.into(), texture.map(str::to_owned));
            Job { source, execution }
        };
        // Decode and bind against copies. A failed asset read must retain the
        // exact pending request and original geometry for a retry or snapshot.
        let mut assets = self
            .characters
            .get(&id)
            .context("accessory assets absent")?
            .clone();
        let mut resident = simulation
            .characters
            .get(&id)
            .context("accessory resident absent")?
            .clone();
        let mut pool = self.mesh_pool.clone();
        if let Some(request) = job.execution.pending_request().cloned() {
            let (record, execution) = match request {
                accessory::Request::Clear(id) | accessory::Request::Destroy(id) => {
                    (id, appearance::Execution::clear())
                }
                accessory::Request::Bind {
                    record,
                    name,
                    texture,
                } => (
                    record,
                    appearance::Execution::bind(appearance::Input::Replace { name, texture }),
                ),
            };
            assets.model.with_bound(&mut resident, |bound| {
                assets.geometry.apply_record(
                    &self.index,
                    &mut assets.avatar,
                    record,
                    execution,
                    bound,
                    &mut pool,
                )
            })?;
            job.execution
                .acknowledge(&mut assets.avatar.records)
                .map_err(anyhow::Error::msg)?;
        } else {
            // Publish a pending operation before invoking its asset services.
            // In particular an appended empty record can be saved before bind.
            job.execution
                .next(&mut assets.avatar.records)
                .map_err(anyhow::Error::msg)?;
        }
        self.characters.insert(id, assets);
        simulation.characters.insert(id, resident);
        self.mesh_pool = pool;
        self.accessories.insert(id, job);
        Ok(())
    }
}
