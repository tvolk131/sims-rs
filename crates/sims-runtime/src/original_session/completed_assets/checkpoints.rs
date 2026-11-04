//! Typed retained diagnostic owners. This is not a complete Session admission.
use super::*;
use sims_fixtures::{BlobRef, Store};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Retained {
    pub simulation: sims_sim::Snapshot,
    pub characters: BTreeMap<ObjectId, SavedAssets>,
    pub ordinary: graphic_loading::Saved,
    pub meshes: MeshPool,
    pub native_meshes: Vec<u32>,
    pub dependent: BTreeMap<ObjectId, sims_content::character_loading::live::dependent::Job>,
    pub presentation: crate::original_session::completed_presentation::Bindings,
    pub floors: Option<BTreeMap<u16, sims_content::character_loading::floors::Binding>>,
}
pub struct Owners<'a> {
    pub program: &'a Program,
    pub simulation: &'a sims_sim::Simulation,
    pub characters: &'a BTreeMap<ObjectId, SavedAssets>,
    pub ordinary: &'a graphic_loading::Registry,
    pub meshes: &'a MeshPool,
    pub native_meshes: &'a [u32],
    pub dependent: &'a BTreeMap<ObjectId, sims_content::character_loading::live::dependent::Job>,
    pub presentation: &'a crate::original_session::completed_presentation::Bindings,
    pub floors: &'a Option<BTreeMap<u16, sims_content::character_loading::floors::Binding>>,
}
impl Retained {
    pub fn put(store: &Store, index: &Index, input: Owners<'_>) -> Result<BlobRef> {
        let cut = Self {
            simulation: sims_sim::Snapshot::capture(input.program, input.simulation),
            characters: input.characters.clone(),
            ordinary: input.ordinary.snapshot(),
            meshes: input.meshes.clone(),
            native_meshes: input.native_meshes.into(),
            dependent: input.dependent.clone(),
            presentation: input.presentation.clone(),
            floors: input.floors.clone(),
        };
        // Verify the restored value after the exact structural roundtrip. A
        // second identical immutable-source restore before writing adds no
        // validation: a bad live cut remains bad after the equality check and
        // is rejected below. This path never advances the VM.
        let blob = store.put_value(&cut)?;
        let cold: Self = store.get_value(&blob)?;
        ensure!(cold == cut, "retained owner cut changed in durable storage");
        cold.validate(index, input.program)?;
        Ok(blob)
    }
    pub fn validate(&self, index: &Index, program: &Program) -> Result<()> {
        let simulation = self
            .simulation
            .clone()
            .restore(program)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            self.native_meshes.len() == self.meshes.meshes.len()
                && self.native_meshes.iter().all(|&p| p != 0)
                && self
                    .native_meshes
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == self.native_meshes.len(),
            "retained shared mesh identity differs"
        );
        ensure!(
            self.characters.keys().eq(simulation.characters.keys()),
            "retained Character owners differ"
        );
        for (&id, assets) in &self.characters {
            let avatar = BoundAvatar::restore(index, &assets.skeleton, id as u64, &assets.binding)?;
            ensure!(
                avatar.snapshot() == assets.binding,
                "retained immutable avatar changed"
            );
        }
        let mut ordinary = graphic_loading::Registry::restore(index, self.ordinary.clone())?;
        ensure!(
            ordinary.snapshot() == self.ordinary,
            "retained ordinary registry changed"
        );
        let mut meshes = self.meshes.clone();
        let mut dependent = self.dependent.clone();
        let owners = sims_content::character_loading::live::dependent::Owners {
            index,
            ordinary: &mut ordinary,
            mesh_pool: &mut meshes,
            jobs: &mut dependent,
        };
        owners.validate_dependent(&simulation)?;
        if let Some(floors) = &self.floors {
            sims_content::character_loading::floors::restore(index, floors)?;
        }
        Ok(())
    }
}
