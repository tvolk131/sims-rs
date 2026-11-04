//! Character appearance attachment, replacement and removal. Mesh prototypes retain their
//! own caches; model bindings retain bone mappings and drawable bounds. The
//! headless drawable constructor supplies bounds/flags, not Windows GPU state.
use crate::{
    Index,
    saved_avatar::{BoundAvatar, GeometryEvent},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::pose::{
    appearance::Execution,
    geometry::{Binding, Drawable, State},
    mesh::{LocalBounds, MeshBounds, Pose},
    model::{Bound, Bounds},
    vertices::{Buffer, Vertex},
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MeshPool {
    pub meshes: Vec<MeshBounds>,
    prototypes: BTreeMap<String, usize>,
}
pub(crate) fn prototype_key(
    locator: &str,
    source_hash: &str,
    appearance: &str,
    ordinal: usize,
) -> Result<String> {
    Ok(serde_json::to_string(&(
        locator,
        source_hash,
        appearance,
        ordinal,
    ))?)
}
impl MeshPool {
    fn resolve(&mut self, key: String, local: &LocalBounds) -> Result<usize> {
        if let Some(&index) = self.prototypes.get(&key) {
            ensure!(
                self.meshes
                    .get(index)
                    .is_some_and(|m| m.local == local.local),
                "mesh prototype changed during reload"
            );
            return Ok(index);
        }
        let index = self.meshes.len();
        self.meshes.push(MeshBounds {
            local: local.local.clone(),
            stamps: vec![0; local.local.len()],
            bounds: Bounds::EMPTY,
        });
        self.prototypes.insert(key, index);
        Ok(index)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublishedVertices {
    pub vertices: Vec<Vertex>,
    pub buffer_flags: u32,
    pub range: [i32; 2],
}
impl PublishedVertices {
    /// Complete5d383a's null-bounds notification when native byte+c0 is zero.
    /// Publication requests this renderer service; accepting it clears only
    /// the pending buffer bit. Bounds-conversion handling is a separate path.
    pub fn acknowledge_null_bounds_upload(&mut self, conversion_pending: bool) -> Result<()> {
        ensure!(!conversion_pending, "native drawable bounds conversion is not implemented");
        self.buffer_flags &= !1;
        Ok(())
    }
    fn new(mesh: &sims_formats::avatar::Mesh) -> Self {
        // 599955 copies BMF samples, preserving primary UVs and zeroing the
        // auxiliary UVs. Renderer-private buffer flags start in our headless
        // backend's state; Windows device allocation is not reproduced.
        Self {
            vertices: mesh
                .vertices
                .iter()
                .map(|v| Vertex {
                    position: v.position,
                    normal: v.normal,
                    uv: v.uv,
                })
                .chain(mesh.blends.iter().map(|v| Vertex {
                    position: v.position,
                    normal: v.normal,
                    uv: [0.; 2],
                }))
                .collect(),
            buffer_flags: 0,
            range: [0, mesh.vertices.len() as i32 - 1],
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterGeometry {
    pub state: State,
    pub appearances: BTreeMap<u64, usize>,
    /// Per-instance CPU vertices. Bounds/flags belong to state.bindings and
    /// are combined with these fields only during a synchronous vertex pass.
    #[serde(default)]
    pub vertices: BTreeMap<usize, PublishedVertices>,
}
impl CharacterGeometry {
    /// 4e810d's selected (geometry binding, avatar part) pairs. The caller owns
    /// the preceding forced appearance selection; this only reads its result.
    pub fn portrait_bindings(&self, avatar: &BoundAvatar) -> Result<Vec<(usize, usize)>> {
        let mut appearances = vec![];
        for (ordinal, bindings) in self.state.appearances.iter().enumerate() {
            let (&identity, _) = self
                .appearances
                .iter()
                .find(|(_, n)| **n == ordinal)
                .context("portrait appearance identity absent")?;
            let range = avatar
                .part_ranges
                .get(&identity)
                .context("portrait appearance assets absent")?;
            let parts = avatar
                .avatar
                .parts
                .get(range.clone())
                .context("portrait part range invalid")?;
            ensure!(
                parts.len() == bindings.len(),
                "portrait binding count differs from assets"
            );
            let mut rows = vec![];
            for (offset, (&binding, part)) in bindings.iter().zip(parts).enumerate() {
                ensure!(
                    self.state.bindings.get(binding).is_some()
                        && self.vertices.contains_key(&binding),
                    "portrait drawable binding absent"
                );
                rows.push((part.attachment.as_bytes(), (binding, range.start + offset)));
            }
            appearances.push(rows);
        }
        let mut selected = vec![];
        sims_sim::pose::portrait::collect_models(appearances, &mut selected);
        Ok(selected)
    }
    pub fn new(bones: usize) -> Self {
        Self {
            state: State::empty(bones),
            appearances: BTreeMap::new(),
            vertices: BTreeMap::new(),
        }
    }
    /// Complete a pending live-character geometry service using this model's
    /// actual bindings. None leaves non-geometry services pending. The returned
    /// binding IDs are renderer upload notifications from the loaded-tail pass.
    /// All owners commit together; an error leaves the continuation untouched.
    pub fn service_resident(
        &mut self,
        model: &mut crate::character_model::State,
        resident: &mut sims_sim::character::Resident,
        avatar: &BoundAvatar,
        pool: &mut MeshPool,
    ) -> Result<Option<Vec<usize>>> {
        use sims_sim::pose::refresh::{Request, Response};
        let Some(request) = resident.character.pending_request().cloned() else {
            return Ok(None);
        };
        if !matches!(
            request,
            Request::Geometry { .. }
                | Request::ModelGeometry
                | Request::ModelBounds
                | Request::LoadedAppearance
        ) {
            return Ok(None);
        }
        if request == Request::ModelBounds {
            crate::character_model::validate_published_transform(resident)?;
            // The old Bound transaction only copied these bounds and cleared
            // bit 0. Done already clears that bit and advances the same pending
            // continuation, with no fallible work after its first write. Neither
            // operation changes the transform, so with_bound's publication
            // postcondition follows from the check above. Commit after all
            // fallible checks without copying geometry, meshes or the resident.
            resident
                .character
                .acknowledge(Response::Done)
                .map_err(anyhow::Error::msg)?;
            model.render_bounds = model.local_bounds;
            return Ok(Some(vec![]));
        }
        if let Request::Geometry {
            branch: Some(bone), ..
        } = request
        {
            ensure!(
                Some(bone) == resident.character.rig.turn_bone,
                "unsupported named-bone geometry selector"
            );
        }
        let mut next = self.clone();
        let mut next_pool = pool.clone();
        let mut next_model = model.clone();
        let mut next_resident = resident.clone();
        let notifications = next_model.with_bound(&mut next_resident, |bound| {
            match request {
                Request::Geometry {
                    branch: Some(_),
                    force,
                } => next
                    .state
                    .refresh_head(bound, &mut next_pool.meshes, force)
                    .map_err(anyhow::Error::msg)?,
                Request::Geometry {
                    branch: None,
                    force,
                } => next
                    .state
                    .refresh(bound, &mut next_pool.meshes, force)
                    .map_err(anyhow::Error::msg)?,
                Request::ModelGeometry => next
                    .state
                    .refresh(bound, &mut next_pool.meshes, false)
                    .map_err(anyhow::Error::msg)?,
                Request::LoadedAppearance => {
                    return next.update_loaded_vertices(avatar, bound, false);
                }
                _ => unreachable!(),
            }
            Ok(vec![])
        })?;
        next_resident
            .character
            .acknowledge(Response::Done)
            .map_err(anyhow::Error::msg)?;
        *self = next;
        *pool = next_pool;
        *model = next_model;
        *resident = next_resident;
        Ok(Some(notifications))
    }
    /// 5a1092's appearance/binding traversal after the saved head observation.
    /// Return renderer notifications in native order after actually executing
    /// each CPU update. The headless backend consumes those notifications
    /// without a Windows GPU; a graphical backend may upload these buffers.
    pub fn update_loaded_vertices(
        &mut self,
        avatar: &BoundAvatar,
        bound: &Bound,
        cached_drawable: bool,
    ) -> Result<Vec<usize>> {
        self.update_loaded_vertices_with_control(avatar, bound, cached_drawable, 0x037f)
    }

    pub fn update_loaded_vertices_with_control(
        &mut self,
        avatar: &BoundAvatar,
        bound: &Bound,
        cached_drawable: bool,
        control: u16,
    ) -> Result<Vec<usize>> {
        ensure!(matches!(control, 0x003f | 0x037f), "native vertex floating-point control unsupported");
        let mut notifications = vec![];
        for (appearance, indices) in self.state.appearances.iter().enumerate() {
            let (&identity, _) = self
                .appearances
                .iter()
                .find(|(_, index)| **index == appearance)
                .context("loaded appearance identity absent")?;
            let range = avatar
                .part_ranges
                .get(&identity)
                .context("loaded appearance assets absent")?;
            let parts = avatar
                .avatar
                .parts
                .get(range.clone())
                .context("loaded appearance range invalid")?;
            ensure!(
                parts.len() == indices.len(),
                "loaded mesh binding count differs from assets"
            );
            for (&index, part) in indices.iter().zip(parts) {
                let binding = self
                    .state
                    .bindings
                    .get_mut(index)
                    .context("loaded mesh binding absent")?;
                let published = self
                    .vertices
                    .get_mut(&index)
                    .context("loaded mesh vertex storage absent")?;
                if update_binding_vertices_with_control(
                    binding,
                    published,
                    part,
                    &bound.skeleton.hierarchy.absolute,
                    &bound.skeleton.hierarchy.updates,
                    false,
                    cached_drawable,
                    control,
                )? {
                    notifications.push(index);
                }
            }
        }
        Ok(notifications)
    }
    /// Execute saved-reader record services against the initialized model.
    /// Record and auxiliary destruction complete here. The caller handles the
    /// returned head notification; descriptors and refresh follow separately.
    pub fn advance_saved_appearance(
        &mut self,
        index: &Index,
        avatar: &mut BoundAvatar,
        read: &mut sims_sim::pose::appearance_read::Read,
        owner: &mut sims_sim::pose::appearance_reload::Owner,
        bound: &mut Bound,
        placement: &mut sims_sim::pose::transform::Placement,
        pool: &mut MeshPool,
    ) -> Result<Option<sims_sim::pose::appearance_read::Request>> {
        use sims_sim::pose::{
            appearance::Input, appearance_read::Request, appearance_reload::Request as Lifecycle,
        };
        ensure!(
            owner.model_present,
            "saved appearance reader has no initialized model"
        );
        // Both views represent animator +d9. Placement may have cleared it
        // since profile reload; publish the reader's writes back each step.
        owner.selection.dirty = placement.dirty;
        loop {
            let request = read
                .next(owner, &mut avatar.records)
                .map_err(anyhow::Error::msg)?;
            placement.dirty = owner.selection.dirty;
            let Some(request) = request else {
                return Ok(None);
            };
            let (record, execution) = match request {
                Request::Lifecycle(Lifecycle::Detach(id)) => {
                    self.detach_auxiliary(index, avatar, id, bound, pool)?;
                    read.acknowledge(owner, &mut avatar.records)
                        .map_err(anyhow::Error::msg)?;
                    continue;
                }
                Request::Lifecycle(
                    Lifecycle::ClearRecord(id) | Lifecycle::DestroyAccessory(id),
                ) => (id, Execution::clear()),
                Request::BindRecord(id) => (id, Execution::bind(Input::Current)),
                other => return Ok(Some(other)),
            };
            self.apply_record(index, avatar, record, execution, bound, pool)?;
            read.acknowledge(owner, &mut avatar.records)
                .map_err(anyhow::Error::msg)?;
        }
    }
    /// Initial profile selection into an unbound record.
    pub fn bind_fresh_record(
        &mut self,
        index: &Index,
        avatar: &mut BoundAvatar,
        record: usize,
        execution: Execution,
        bound: &mut Bound,
        pool: &mut MeshPool,
    ) -> Result<()> {
        ensure!(
            avatar
                .records
                .entries
                .get(record)
                .context("geometry record absent")?
                .binding
                .is_none(),
            "fresh geometry record already bound"
        );
        self.apply_record(index, avatar, record, execution, bound, pool)?;
        ensure!(
            avatar.records.entries[record].binding.is_some(),
            "fresh geometry binding absent"
        );
        Ok(())
    }

    /// Apply geometry services at the record controller's actual boundaries.
    /// Replacement attaches first, destroys the previous instance second, and
    /// finally performs the record-clear dirty write. Rehydration uses
    /// BoundAvatar::restore separately; it must not repeat these mutations.
    pub fn apply_record(
        &mut self,
        index: &Index,
        avatar: &mut BoundAvatar,
        record: usize,
        execution: Execution,
        bound: &mut Bound,
        pool: &mut MeshPool,
    ) -> Result<()> {
        avatar.apply_record_with_geometry(index, record, execution, |event| {
            self.apply_event(index, bound, pool, event)
        })
    }

    pub fn bind_auxiliary(
        &mut self,
        index: &Index,
        avatar: &mut BoundAvatar,
        name: &str,
        mask: u32,
        bound: &mut Bound,
        pool: &mut MeshPool,
    ) -> Result<u64> {
        avatar.bind_auxiliary_with_geometry(index, name, mask, |event| {
            self.apply_event(index, bound, pool, event)
        })
    }

    pub fn detach_auxiliary(
        &mut self,
        index: &Index,
        avatar: &mut BoundAvatar,
        id: u64,
        bound: &mut Bound,
        pool: &mut MeshPool,
    ) -> Result<()> {
        avatar
            .detach_auxiliary_with_geometry(id, |event| self.apply_event(index, bound, pool, event))
    }

    fn apply_event(
        &mut self,
        index: &Index,
        bound: &mut Bound,
        pool: &mut MeshPool,
        event: GeometryEvent<'_>,
    ) -> Result<()> {
        match event {
            GeometryEvent::Attach {
                binding,
                catalog_id,
                appearance,
                source_hash,
                skeleton,
                parts,
                ordinals,
            } => {
                ensure!(
                    !self.appearances.contains_key(&binding.id),
                    "geometry binding already installed"
                );
                let resource = &index.resources[catalog_id];
                let mut bindings = vec![];
                for (&ordinal, part) in ordinals.iter().zip(parts) {
                    let local = LocalBounds::from_mesh(&part.mesh).map_err(anyhow::Error::msg)?;
                    // Native loaded meshes belong to BCF binding prototypes.
                    // Use the catalog's canonical name, not lookup casing.
                    let key =
                        prototype_key(&resource.locator, source_hash, &appearance.name, ordinal)?;
                    let mesh = pool.resolve(key, &local)?;
                    ensure!(
                        part.mesh.bindings.len() == part.mesh.bones.len(),
                        "native mesh binding/name table shape unsupported"
                    );
                    let bones = part
                        .mesh
                        .bones
                        .iter()
                        .map(|name| {
                            skeleton
                                .bones
                                .iter()
                                .position(|b| &b.name == name)
                                .with_context(|| format!("native mesh bone {name} absent"))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    bindings.push(Binding {
                        mesh,
                        bones,
                        bounds: Bounds::EMPTY,
                        drawable: Drawable {
                            bounds: local.bounds,
                            flags: 6,
                        },
                    });
                }
                let appearance = self
                    .state
                    .attach(bound, bindings, &pool.meshes)
                    .map_err(anyhow::Error::msg)?;
                for (&index, part) in self.state.appearances[appearance].iter().zip(parts) {
                    self.vertices
                        .insert(index, PublishedVertices::new(&part.mesh));
                }
                self.appearances.insert(binding.id, appearance);
            }
            GeometryEvent::Detach(binding) => {
                let &appearance = self
                    .appearances
                    .get(&binding.id)
                    .context("removed appearance geometry absent")?;
                let removed = self.state.appearances[appearance].clone();
                self.state
                    .detach(bound, appearance)
                    .map_err(anyhow::Error::msg)?;
                for index in removed.into_iter().rev() {
                    self.vertices.remove(&index);
                }
                self.appearances.remove(&binding.id);
                for index in self.appearances.values_mut() {
                    if *index > appearance {
                        *index -= 1;
                    }
                }
            }
            GeometryEvent::Dirty(_) => bound.skeleton.geometry.dirty = true,
        }
        Ok(())
    }
}

/// Shared 599bf2 adapter. Rest mode ignores bone pointers, as 59a020 does;
/// ordinary publication uses each resolved live bone pose and update stamp.
pub(crate) fn update_binding_vertices(
    binding: &mut Binding,
    published: &mut PublishedVertices,
    part: &crate::avatar::Part,
    absolute: &[sims_sim::head_look::rig::AbsoluteBone],
    updates: &[u32],
    rest: bool,
    cached_drawable: bool,
) -> Result<bool> {
    update_binding_vertices_with_control(
        binding, published, part, absolute, updates, rest, cached_drawable, 0x037f,
    )
}

fn update_binding_vertices_with_control(
    binding: &mut Binding,
    published: &mut PublishedVertices,
    part: &crate::avatar::Part,
    absolute: &[sims_sim::head_look::rig::AbsoluteBone],
    updates: &[u32],
    rest: bool,
    cached_drawable: bool,
    control: u16,
) -> Result<bool> {
    let poses = if rest {
        vec![None; part.mesh.bindings.len()]
    } else {
        part.mesh
            .bindings
            .iter()
            .map(|row| {
                let bone = *binding
                    .bones
                    .get(row.bone)
                    .context("loaded mesh bone map absent")?;
                let pose = *absolute.get(bone).context("loaded mesh bone pose absent")?;
                let stamp = *updates.get(bone).context("loaded mesh bone stamp absent")?;
                Ok(Some(
                    Pose::from_absolute_with_control(pose, stamp, control).map_err(anyhow::Error::msg)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?
    };
    let mut buffer = Buffer {
        vertices: published.vertices.clone(),
        buffer_flags: published.buffer_flags,
        range: published.range,
        flags: binding.drawable.flags,
        bounds: binding.drawable.bounds,
    };
    let notify = buffer
        .update_with_control(&part.mesh, &poses, rest, cached_drawable, binding.bounds, control)
        .map_err(anyhow::Error::msg)?;
    published.vertices = buffer.vertices;
    published.buffer_flags = buffer.buffer_flags;
    published.range = buffer.range;
    binding.drawable.flags = buffer.flags;
    binding.drawable.bounds = buffer.bounds;
    Ok(notify)
}
