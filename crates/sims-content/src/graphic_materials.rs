//! Retained CPU material backend for ordinary graphic effect publication.
//! Native device allocation/destruction is outside this adapter. The caller
//! supplies backend mode, flags, ownership and ramp inputs explicitly; source
//! colors come from the decoded built-in material, never from its preview BMP.
use crate::{character_geometry::CharacterGeometry, saved_avatar::BoundAvatar};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::pose::model_material::{self as native, Drawable, Material, Reply, Request, Vector};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy)]
pub struct Backend {
    pub mode: u32,
    pub flags: u32,
    pub references: u32,
    pub ramp_size: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphicMaterials {
    pub state: native::State,
    /// CPU equivalents of the material payloads sent to the device. Separate
    /// from mutable source state, which callbacks may change after publication.
    pub published: BTreeMap<u64, [u32; 20]>,
    pub bindings: BTreeMap<usize, u64>,
}
impl GraphicMaterials {
    /// Prepare a backend for already-bound, built-in-material geometry. Keep
    /// this owner between effect updates. This does not claim to reconstruct
    /// native device construction, cache sharing or detached mesh destruction.
    /// Texture resources require their real material/texture handles and are
    /// deliberately rejected here rather than given invented white materials.
    pub fn from_builtin_geometry(
        geometry: &CharacterGeometry,
        avatar: &BoundAvatar,
        controls: [u8; 3],
        backend: Backend,
    ) -> Result<Self> {
        ensure!(backend.references > 0, "retained material owner required");
        let mut owner = Self {
            state: native::State {
                controls,
                group: Some(1),
                groups: BTreeMap::new(),
                drawables: BTreeMap::new(),
                materials: BTreeMap::new(),
            },
            published: BTreeMap::new(),
            bindings: BTreeMap::new(),
        };
        let mut children = vec![];
        for (ordinal, indices) in geometry.state.appearances.iter().enumerate() {
            let (&identity, _) = geometry
                .appearances
                .iter()
                .find(|(_, n)| **n == ordinal)
                .context("material appearance identity absent")?;
            let range = avatar
                .part_ranges
                .get(&identity)
                .context("material appearance parts absent")?;
            let parts = avatar
                .avatar
                .parts
                .get(range.clone())
                .context("material part range invalid")?;
            ensure!(
                parts.len() == indices.len(),
                "material binding count differs from assets"
            );
            for (&binding, part) in indices.iter().zip(parts) {
                ensure!(
                    geometry.state.bindings.get(binding).is_some(),
                    "material drawable absent"
                );
                let builtin = part
                    .builtin_material
                    .as_ref()
                    .context("texture material backend not yet supplied")?;
                let id = u64::try_from(binding)?
                    .checked_add(1)
                    .context("material identity overflow")?;
                ensure!(
                    owner.bindings.insert(binding, id).is_none(),
                    "duplicate material binding"
                );
                let words = builtin.words(backend.ramp_size);
                owner.state.materials.insert(
                    id,
                    Material {
                        words,
                        references: backend.references,
                        texture: None,
                    },
                );
                owner.state.drawables.insert(
                    id,
                    Drawable {
                        mode: backend.mode,
                        flags: backend.flags,
                        materials: Vector {
                            storage: vec![Some(id)],
                            len: 1,
                        },
                    },
                );
                owner.published.insert(id, words);
                children.push(Some(id));
            }
        }
        owner.state.groups.insert(
            1,
            Vector {
                len: children.len(),
                storage: children,
            },
        );
        Ok(owner)
    }
    /// Complete at most one material service, permitting a save between each
    /// native callback. Publication retains all 20 words, not only albedo.
    pub fn advance(&mut self, update: &mut native::Update, normal_mode: u32) -> Result<bool> {
        let Some(request) = update
            .advance(&mut self.state, normal_mode)
            .map_err(anyhow::Error::msg)?
        else {
            return Ok(true);
        };
        let reply = match &request {
            Request::Cast { object } => {
                Reply::Cast(object.filter(|id| self.state.drawables.contains_key(id)))
            }
            Request::Acquire { material } => {
                self.state.acquire(*material).map_err(anyhow::Error::msg)?;
                Reply::Done
            }
            Request::Publish { material, words } => {
                ensure!(
                    self.state.materials.contains_key(material),
                    "published material absent"
                );
                self.published.insert(*material, *words);
                Reply::Done
            }
            Request::Release { material } => {
                self.state
                    .release_retained(*material)
                    .map_err(anyhow::Error::msg)?;
                Reply::Done
            }
        };
        update
            .acknowledge(&request, reply)
            .map_err(anyhow::Error::msg)?;
        Ok(update.complete())
    }
    pub fn apply(&mut self, controls: [u8; 3], normal_mode: u32) -> Result<()> {
        let mut update = native::Update::new(controls);
        while !self.advance(&mut update, normal_mode)? {}
        Ok(())
    }
    pub fn published_for_binding(&self, binding: usize) -> Result<&[u32; 20]> {
        let identity = self
            .bindings
            .get(&binding)
            .context("material binding absent")?;
        self.published
            .get(identity)
            .context("binding material not published")
    }
}
