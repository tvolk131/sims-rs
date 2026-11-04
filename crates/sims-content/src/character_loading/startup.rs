//! Asset ownership before the old registry is cleared. The Reader retains the
//! same caches/resources through teardown and the subsequent saved person read.
use super::*;
use sims_sim::Simulation;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedGraphics {
    pub ordinary: crate::graphic_loading::Saved,
    pub mesh_pool: MeshPool,
}

impl<P: Presentation> Reader<'_, P> {
    /// Attach complete retained storage. The Reader's explicit initializer/view
    /// settings remain caller-owned inputs. Restoring assets performs no fresh
    /// graphic initialization, frame pass, clock query or CRT draw.
    pub fn install_retained_graphics(&mut self, input: RetainedGraphics) -> Result<()> {
        ensure!(
            self.ordinary.snapshot() == crate::graphic_loading::Saved::default()
                && self.mesh_pool == MeshPool::default()
                && self.characters.is_empty()
                && self.fresh.is_empty()
                && self.constants.is_empty(),
            "incoming graphics overlap existing Reader ownership"
        );
        let ordinary = crate::graphic_loading::Registry::restore(self.index, input.ordinary)?;
        self.ordinary = ordinary;
        self.mesh_pool = input.mesh_pool;
        Ok(())
    }
}

impl<P: Presentation> crate::house_startup::AssetHost for Reader<'_, P> {
    fn prepare_incoming(&mut self, vm: &Vm, program: &Program) -> Result<BTreeSet<ObjectId>> {
        ensure!(
            self.characters.is_empty()
                && self.fresh.is_empty()
                && self.constants.is_empty()
                && self.initial_process.is_none(),
            "incoming graphics require one retained nonperson Reader"
        );
        self.ordinary.validate(vm)?;
        ensure!(
            self.ordinary.pending_objects().next().is_none()
                && !self.ordinary.updates_pending()
                && self.ordinary.retirements.is_empty()
                && self
                    .ordinary
                    .retained
                    .objects()
                    .all(|id| !self.ordinary.retained.started(id)),
            "incoming ordinary graphic service is already suspended"
        );
        // This requires every definition's positive graphics bit to have an
        // actual retained asset owner. A clear bit alone is not native pointer
        // absence proof; a strict native mapper supplies every observed owner.
        self.ordinary.presenter_readiness(vm, program)?;
        Ok(self.ordinary.owned_objects().collect())
    }

    fn service_teardown(&mut self, simulation: &mut Simulation, program: &Program) -> Result<()> {
        self.ordinary
            .validate_with_retired(&simulation.vm, &simulation.retired_graphics)?;
        if let Some(id) = simulation
            .deletion
            .last()
            .map(|d| d.target)
            .filter(|id| simulation.retired_graphics.contains_key(id))
        {
            return crate::graphic_retirement::service(
                self.index,
                &mut self.ordinary,
                &mut self.mesh_pool,
                self.settings,
                &mut self.presentation,
                simulation,
                id,
            );
        }
        if let Some((id, _)) = simulation.pending_removal_graphic() {
            return crate::graphic_loading::retained::refresh::service(
                &mut self.ordinary,
                simulation,
                program,
                id,
            );
        }
        anyhow::bail!("retained startup requires an owning pending-asset service")
    }

    fn finish_teardown(&mut self, vm: &Vm) -> Result<()> {
        self.ordinary.validate(vm)?;
        ensure!(
            self.ordinary.graphics.is_empty()
                && self.ordinary.retained.objects().next().is_none()
                && self.ordinary.pending_objects().next().is_none()
                && self.ordinary.retirements.is_empty(),
            "old ordinary graphics survived the frozen registry clear"
        );
        Ok(())
    }
}
