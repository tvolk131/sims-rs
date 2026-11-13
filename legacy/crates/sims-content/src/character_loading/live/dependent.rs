//! Source58f8ff..58fa82: physical slot2 is logical slot1 (record0 is sentinel).
//! The ordinary child owns its nested frame; the person cannot overtake it.
use super::*;
use crate::graphic_update::{Command, Step};
use sims_sim::pose::{graphic_frame, model::Matrix};
use sims_vm::slot_geometry::Anchor;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    parent: ObjectId,
    child: ObjectId,
    tick: u64,
    matrix: Matrix,
    stage: u8,
    started: bool,
}
/// Borrowed, retained dependent owners. Replay can advance these without
/// inventing unrelated neighborhood, clothing, or frontend host state.
pub struct Owners<'a> {
    pub index: &'a Index,
    pub ordinary: &'a mut crate::graphic_loading::Registry,
    pub mesh_pool: &'a mut MeshPool,
    pub jobs: &'a mut BTreeMap<ObjectId, Job>,
}
impl<P: Presentation> Host<P> {
    pub fn dependent_pending(&self) -> bool {
        !self.dependent_jobs.is_empty()
    }
    pub fn validate_dependent(&self, simulation: &Simulation) -> Result<()> {
        validate(&self.dependent_jobs, &self.ordinary, simulation)
    }
    pub fn service_dependent(
        &mut self,
        simulation: &mut Simulation,
        program: &Program,
        parent: ObjectId,
        view: graphic::View,
    ) -> Result<()> {
        Owners {
            index: &self.index,
            ordinary: &mut self.ordinary,
            mesh_pool: &mut self.mesh_pool,
            jobs: &mut self.dependent_jobs,
        }
        .service_dependent(simulation, program, parent, view)
    }
}
fn validate(
    jobs: &BTreeMap<ObjectId, Job>,
    ordinary: &crate::graphic_loading::Registry,
    simulation: &Simulation,
) -> Result<()> {
    for (&id, j) in jobs {
        ensure!(
            id == j.parent && j.tick == simulation.vm.tick && j.stage <= 4,
            "dependent graphic lost its parent/tick/stage"
        );
        ensure!(
            simulation
                .pending_character_services()
                .iter()
                .any(|(p, r)| *p == id && *r == Request::OrdinaryCarriedObject),
            "dependent graphic lost its person continuation"
        );
        ensure!(
            simulation.vm.world.objects[&id].slots.get(1) == Some(&Some(j.child)),
            "dependent graphic containment changed during callback"
        );
        let child = ordinary
            .graphics
            .get(&j.child)
            .context("dependent child model absent")?;
        ensure!(
            child.update_pending() == j.started,
            "dependent child lost its nested frame"
        );
    }
    Ok(())
}

impl Owners<'_> {
    pub fn validate_dependent(&self, simulation: &Simulation) -> Result<()> {
        validate(self.jobs, self.ordinary, simulation)
    }
    pub fn service_dependent(
        &mut self,
        simulation: &mut Simulation,
        program: &Program,
        parent: ObjectId,
        view: graphic::View,
    ) -> Result<()> {
        let request = Request::OrdinaryCarriedObject;
        ensure!(
            simulation
                .pending_character_services()
                .contains(&(parent, request.clone())),
            "dependent person tail is not pending"
        );
        if !self.jobs.contains_key(&parent) {
            let o = &simulation.vm.world.objects[&parent];
            let child = o
                .slots
                .get(1)
                .copied()
                .flatten()
                .context("dependent slot is empty")?;
            self.ordinary
                .graphics
                .get(&child)
                .context("dependent requires its actual ordinary model")?;
            let anchor = o
                .native_slot_geometry
                .as_ref()
                .and_then(|v| v.get(1))
                .and_then(|s| s.anchor.as_ref())
                .context("dependent slot anchor owner is unobserved")?;
            if *anchor == Anchor::Absent {
                simulation
                    .complete_character_service(parent, &request, Response::Done)
                    .map_err(anyhow::Error::msg)?;
                return Ok(());
            }
            let Anchor::Named(name) = anchor else {
                unreachable!()
            };
            let resident = &simulation.characters[&parent];
            let skeleton = resident
                .source
                .as_ref()
                .context("dependent parent skeleton name lookup absent")?;
            let bone = skeleton
                .bones
                .iter()
                .position(|b| b.name.as_bytes() == name);
            // A source-owned skeleton name proves this already-interned name.
            // Unknown names also consult/mutate the native global name catalog;
            // do not manufacture the missing-catalog result as identity.
            let bone =
                bone.context("dependent name needs its retained native name-catalog owner")?;
            let point = resident
                .character
                .hierarchy
                .absolute
                .get(bone)
                .context("dependent bone hierarchy absent")?
                .translation;
            let mut matrix = Matrix::IDENTITY;
            matrix.values[12..15].copy_from_slice(&point);
            matrix.kind = 0;
            matrix = matrix
                .multiply_with_control(
                    resident.character.model.placement,
                    resident.character.skin.animator.fp_control(),
                )
                .map_err(anyhow::Error::msg)?;
            self.jobs.insert(
                parent,
                Job {
                    parent,
                    child,
                    tick: simulation.vm.tick,
                    matrix,
                    stage: 0,
                    started: false,
                },
            );
        }
        self.validate_dependent(simulation)?;
        let mut job = self.jobs.remove(&parent).unwrap();
        let result = (|| -> Result<bool> {
            if job.stage == 4 {
                simulation
                    .complete_character_service(parent, &request, Response::Done)
                    .map_err(anyhow::Error::msg)?;
                return Ok(true);
            }
            if job.stage == 2 {
                let bits = simulation.vm.world.objects[&parent]
                    .motives
                    .as_ref()
                    .context("dependent parent motives absent")?[3];
                let value = f32::from_bits(bits);
                ensure!(
                    value.is_finite()
                        && f64::from(value) >= f64::from(i32::MIN)
                        && f64::from(value) < 2147483648.,
                    "dependent marker motive conversion unsupported"
                );
                self.ordinary
                    .graphics
                    .get_mut(&job.child)
                    .unwrap()
                    .marker_value = Some(value.trunc() as i32);
                job.stage = 3;
                return Ok(false);
            }
            if !job.started {
                let command = match job.stage {
                    0 => Command::TransformControl(true),
                    1 => Command::BindTransform(job.matrix),
                    3 => Command::Frame {
                        entry: graphic_frame::Entry::Ordinary,
                        only_visible: false,
                    },
                    _ => anyhow::bail!("invalid dependent service stage"),
                };
                self.ordinary
                    .graphics
                    .get_mut(&job.child)
                    .unwrap()
                    .begin_update(command)?;
                job.started = true;
            }
            match graphic::advance_retained(
                self.index,
                self.ordinary,
                self.mesh_pool,
                simulation,
                program,
                job.child,
                view,
            )? {
                Step::Complete => {
                    job.started = false;
                    job.stage += 1;
                }
                Step::Progress => (),
                Step::Pending(p) => {
                    anyhow::bail!("dependent graphic callback requires its original owner: {p:?}")
                }
            }
            Ok(false)
        })();
        if !matches!(result, Ok(true)) {
            self.jobs.insert(parent, job);
        }
        result.map(|_| ())
    }
}
