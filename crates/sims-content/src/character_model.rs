//! Ownership handoff from the completed native initializer to a live resident.
//! The resident owns animation, hierarchy, placement and the skin generation.
//! This host state retains the remaining model caches and calibration words.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::{
    character::{Controls, Resident},
    pose::{
        initialize::Initialization,
        model::{Bound, Bounds, Matrix},
        properties,
        refresh::{Censor, Character},
        skeleton::{Geometry, Instance, WiggleState},
    },
};

pub(crate) fn validate_published_transform(resident: &Resident) -> Result<()> {
    ensure!(
        resident.character.matrix == resident.character.model.bounds_transform.values,
        "resident model transform is not published"
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub dirty: bool,
    pub evaluated_generation: u32,
    pub bounds_generation: u32,
    pub bone_generations: Vec<u32>,
    pub wiggle: Vec<WiggleState>,
    pub flags: Vec<[u32; 3]>,
    pub named_bone_flags: Vec<bool>,
    pub local_bounds: Bounds,
    pub render_bounds: Bounds,
    pub bone_bounds: Vec<Bounds>,
    pub bone_bounds_generations: Vec<u32>,
    pub previous_mode: i32,
    pub phase_window: [f32; 2],
    pub stop_distance: [f32; 2],
    pub clip_distances: [f32; 6],
    pub needs_appearance: String,
}

impl State {
    /// Transfer the completed fresh model, optionally after the enclosing reader
    /// has installed saved head/offset words and descriptors. Fresh unwritten
    /// head coordinates remain absent. No pose is rebuilt or layer discarded.
    pub(crate) fn handoff(
        initialized: Initialization,
        censor: Censor,
        evaluated_tick: u64,
        animation_events: i32,
        engaged: i32,
        context: properties::Context,
    ) -> Result<(Resident, Self)> {
        ensure!(
            initialized.is_complete() && initialized.initialized,
            "resident handoff requires completed native initialization"
        );
        ensure!(
            !initialized.bound.skeleton.skin.busy() && !initialized.movement.busy(),
            "resident handoff during an initializer callback"
        );
        ensure!(
            initialized.bound.matrix == initialized.bound.model.bounds_transform,
            "initialized model transform is not published"
        );
        let head = initialized.scalars.head().map_err(anyhow::Error::msg)?;
        let visible = initialized
            .visible
            .ok_or_else(|| anyhow::anyhow!("initialized visibility absent"))?;
        let needs_appearance = initialized
            .needs_appearance
            .ok_or_else(|| anyhow::anyhow!("initialized needs appearance absent"))?;
        let bound = initialized.bound;
        let generation = bound.skeleton.geometry.generation;
        let state = Self {
            dirty: bound.skeleton.geometry.dirty,
            evaluated_generation: bound.skeleton.geometry.evaluated_generation,
            bounds_generation: bound.skeleton.geometry.bounds_generation,
            bone_generations: bound.skeleton.geometry.bone_generations,
            wiggle: bound.skeleton.wiggle,
            flags: bound.skeleton.flags,
            named_bone_flags: bound.skeleton.named_bone_flags,
            local_bounds: bound.local_bounds,
            render_bounds: bound.render_bounds,
            bone_bounds: bound.bone_bounds,
            bone_bounds_generations: bound.bone_bounds_generations,
            previous_mode: initialized.scalars.previous_mode,
            phase_window: initialized.scalars.phase_window,
            stop_distance: initialized.scalars.stop_distance,
            clip_distances: initialized.scalars.clip_distances,
            needs_appearance,
        };
        let mut character = Character::new(
            bound.skeleton.skin,
            bound.skeleton.hierarchy,
            head,
            initialized.rig,
            bound.matrix.values,
            initialized.scalars.placement,
            bound.model,
            censor,
            initialized.scalars.clock,
        )
        .map_err(anyhow::Error::msg)?;
        character.visible = visible;
        let mut movement = initialized.movement;
        ensure!(
            movement.path.is_none(),
            "initializer unexpectedly owns a route"
        );
        movement.path = Some(sims_sim::pose::movement::path::Path::initialized(
            sims_sim::pose::movement::path::Settings::COMPLETE_COLLECTION,
            state.phase_window,
            state.stop_distance,
            state.clip_distances,
        ));
        let mut resident = Resident::new(
            character,
            evaluated_tick,
            Controls {
                ordinary: initialized.ordinary,
                movement,
                settings: initialized.settings,
                movement_anchors: initialized.anchors,
                geometry_generation: generation,
                animation_events,
                engaged,
            },
            context,
        );
        resident.source = Some(initialized.source);
        Ok((resident, state))
    }

    /// Assemble a synchronous geometry-service input from its current owners.
    /// In particular reset may change Resident::geometry_generation; it is not
    /// duplicated in this serialized cache. The matrix tag comes from the last
    /// model publication, including while the next model update is suspended.
    fn bound(&self, resident: &Resident) -> Result<Bound> {
        validate_published_transform(resident)?;
        let character = &resident.character;
        Ok(Bound {
            skeleton: Instance {
                skin: character.skin.clone(),
                hierarchy: character.hierarchy.clone(),
                geometry: Geometry {
                    dirty: self.dirty,
                    generation: resident.controls.geometry_generation,
                    evaluated_generation: self.evaluated_generation,
                    bounds_generation: self.bounds_generation,
                    bone_generations: self.bone_generations.clone(),
                },
                wiggle: self.wiggle.clone(),
                flags: self.flags.clone(),
                named_bone_flags: self.named_bone_flags.clone(),
            },
            model: character.model.clone(),
            matrix: Matrix {
                values: character.matrix,
                kind: character.model.bounds_transform.kind,
            },
            local_bounds: self.local_bounds,
            render_bounds: self.render_bounds,
            bone_bounds: self.bone_bounds.clone(),
            bone_bounds_generations: self.bone_bounds_generations.clone(),
        })
    }

    /// Temporary transaction, not a second independently saved animator. Host
    /// geometry services are synchronous and cannot advance a character's
    /// pending continuation. Callers commit any external mesh/asset changes
    /// only after this operation succeeds.
    pub fn with_bound<T>(
        &mut self,
        resident: &mut Resident,
        operation: impl FnOnce(&mut Bound) -> Result<T>,
    ) -> Result<T> {
        let mut bound = self.bound(resident)?;
        // 59824f/5864a4 advance both native bone+130 and+150. Hierarchy
        // consumes the former; these source deltas belong to this host's
        // vertex cache. Apply to the temporary transaction so errors retain
        // the original stamps and pending delivery unchanged.
        for (bone, delta) in bound.skeleton.skin.animator.take_vertex_generation_deltas() {
            let stamp = bound
                .skeleton
                .geometry
                .bone_generations
                .get_mut(bone)
                .ok_or_else(|| anyhow::anyhow!("sampled vertex-cache bone absent"))?;
            *stamp = stamp.wrapping_add(delta);
        }
        let result = operation(&mut bound)?;
        ensure!(
            bound.matrix == bound.model.bounds_transform,
            "geometry service changed published transform inconsistently"
        );
        let geometry = bound.skeleton.geometry;
        self.dirty = geometry.dirty;
        self.evaluated_generation = geometry.evaluated_generation;
        self.bounds_generation = geometry.bounds_generation;
        self.bone_generations = geometry.bone_generations;
        self.wiggle = bound.skeleton.wiggle;
        self.flags = bound.skeleton.flags;
        self.named_bone_flags = bound.skeleton.named_bone_flags;
        self.local_bounds = bound.local_bounds;
        self.render_bounds = bound.render_bounds;
        self.bone_bounds = bound.bone_bounds;
        self.bone_bounds_generations = bound.bone_bounds_generations;
        resident.controls.geometry_generation = geometry.generation;
        resident.character.skin = bound.skeleton.skin;
        resident.character.hierarchy = bound.skeleton.hierarchy;
        resident.character.model = bound.model;
        resident.character.matrix = bound.matrix.values;
        Ok(result)
    }
}
