//! Handoff from the original load pass to the common scheduler. Native physical
//! links remain authoritative; body positions are a presentation/routing cache.
//! Services still lacking native placement adapters stop before mutating them.
use crate::{Body, Point, Room, Simulation, architecture::Architecture, lot_placement};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Vm, person_loading::RoomMembership};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct State {
    pub placement: lot_placement::State,
    pub membership: RoomMembership,
    pub indicators: sims_vm::load_indicators::Indicators,
    /// Pending presentation and lighting work, in native service order.
    pub observers: Vec<lot_placement::Request>,
    /// Fresh-process placement globals; unlike motives these are not SIMI data.
    pub diagnostics: crate::lot_acceptance::person::Diagnostics,
    pub route: Option<crate::lot_route::live::Execution>,
    pub shoo: Option<crate::lot_route::shoo::Execution>,
    pub movement: Option<crate::lot_route::movement::Execution>,
    pub completion: Option<crate::lot_route::completion::Execution>,
    /// Native person path survives nested route-context returns. Native X/Y order.
    pub route_paths: std::collections::BTreeMap<ObjectId, Vec<[i32; 2]>>,
    pub snap: Option<crate::lot_snap::Execution>,
    pub find_location: Option<crate::lot_find_location::Execution>,
    pub removal: Option<crate::lot_removal::Execution>,
    /// Current native animator compensation mirror, as f32 bits. Retained
    /// imports read the animator; this does not assert historical Snap work.
    /// Floor Snap with operand bit0 clear resets compensation to zero.
    pub snap_offsets: std::collections::BTreeMap<ObjectId, [u32; 3]>,
    /// Post-load selection is not restored yet. None is an explicit headless selection.
    pub selected_person: Option<ObjectId>,
    pub room_effects: Vec<sims_vm::person_loading::RoomEffect>,
    pub routing_policy: crate::lot_route::path::Policy,
}

impl Simulation {
    /// Install a loaded lot without running scripts, advancing time, rebuilding
    /// physical links, or replacing negative/off-lot positions. This handoff is
    /// also useful for prefix diagnostics; it does not certify full activation.
    pub fn from_loaded_lot(
        vm: Vm,
        architecture: Architecture,
        native: State,
    ) -> Result<Self, String> {
        let mut bodies = std::collections::BTreeMap::new();
        let mut unplaced = std::collections::BTreeSet::new();
        for (&id, object) in &vm.world.objects {
            let p = vm
                .world
                .positions
                .get(&id)
                .ok_or("loaded position absent")?;
            if p.x < 0 || p.y < 0 {
                unplaced.insert(id);
                continue;
            }
            let direction = *object.data.get(&1).ok_or("loaded direction absent")?;
            bodies.insert(
                id,
                Body {
                    position: Point::new(p.y, p.x),
                    level: p.level,
                    facing: 1 << (direction as u16 & 7),
                    radius: 0,
                    elevation: 0,
                },
            );
        }
        let mut s = Self::from_placements(
            vm,
            Room {
                width: architecture.width,
                height: architecture.height,
                obstacles: vec![],
            },
            bodies,
            unplaced,
            Some(architecture),
        )?;
        s.native_lot = Some(native);
        s.validate_native_lot()?;
        Ok(s)
    }

    /// Connect residential autonomy to the loaded native registry and positions.
    /// Selection and suppression are live UI inputs; free will stays imported.
    /// Exact original in-flight animator/route resumption is a separate service.
    pub fn enable_native_residential_autonomy(&mut self, suppressed: bool) -> Result<(), String> {
        let native = self.native_lot.as_ref().ok_or("native lot absent")?;
        if self.household.is_none() {
            return Err("loaded household clock absent".into());
        }
        let lot = *self
            .vm
            .world
            .globals
            .get(&10)
            .ok_or("loaded house number absent")?;
        let people = self
            .vm
            .world
            .person
            .as_mut()
            .ok_or("loaded person registry absent")?;
        if people.autonomy.is_some() {
            return Err("autonomy already enabled".into());
        }
        if people.lot_zoning.get(&lot) != Some(&1) {
            return Err("native residential autonomy needs loaded residential zoning".into());
        }
        if self.vm.world.objects.values().any(|o| o.idle.is_none()) {
            return Err("loaded object registry flags absent".into());
        }
        if !matches!(self.vm.world.tree_advertisements.len(), 0 | 16) {
            return Err("invalid shared tree advertisement storage".into());
        }
        // Shared scratch triplets are replaced from the TTAB before each check;
        // they are runtime storage, not saved motives or a demo tuning profile.
        self.vm.world.tree_advertisements.resize(16, [0; 3]);
        people.autonomy = Some(sims_vm::autonomy::Context {
            residential: true,
            selected_person: native.selected_person,
            suppressed,
            full_scan: None,
            advertised: None,
            placements: self.vm.world.positions.clone(),
        });
        Ok(())
    }

    pub(crate) fn validate_native_lot(&self) -> Result<(), String> {
        let Some(native) = &self.native_lot else {
            return Ok(());
        };
        let lot = self
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .ok_or("native runtime requires original architecture")?;
        let links = &native.placement.links;
        if self.vm.world.registry_order.is_some() == native.placement.spatial_dirty.is_some() {
            return Err("native lot must have exactly one spatial-dirty owner".into());
        }
        links.validate_arrays()?;
        if let Some(map) = self.vm.world.map_size {
            map.checked_size(lot.size)?;
        }
        if links.size != lot.size
            || links.heads.len() != lot.floors.len()
            || native.placement.flags.len() != links.heads.len()
            || native
                .placement
                .flags
                .iter()
                .any(|f| f.len() != lot.size * lot.size)
            || links.next.keys().ne(self.vm.world.objects.keys())
            || native
                .placement
                .bounds
                .keys()
                .ne(self.vm.world.objects.keys())
            || self
                .vm
                .world
                .positions
                .keys()
                .ne(self.vm.world.objects.keys())
        {
            return Err("native runtime spatial registry mismatch".into());
        }
        let world = &self.vm.world;
        links.validate_tile_chains(world)?;
        let people = world
            .objects
            .iter()
            .filter_map(|(&id, o)| o.person.as_ref().map(|_| id));
        if native.membership.0.keys().copied().ne(people) {
            return Err("native runtime room membership registry mismatch".into());
        }
        // Both registries use signed object-ID order. Walk body entries once,
        // preserving the lookup path's position order and ignoring extra bodies.
        let mut bodies = self.bodies.iter().peekable();
        for (&id, p) in &world.positions {
            while bodies.peek().is_some_and(|(key, _)| **key < id) {
                bodies.next();
            }
            let body = bodies.next_if(|(key, _)| **key == id).map(|(_, b)| b);
            if let Some(b) = body {
                if b.position != Point::new(p.y, p.x) || b.level != p.level {
                    return Err(format!(
                        "native runtime body #{id} bypassed physical placement"
                    ));
                }
            } else if self.cursor_constructor_pending(id) {
                self.validate_cursor_constructor(id)?;
            } else if !self.unplaced.contains(&id) || (p.x >= 0 && p.y >= 0) {
                return Err(format!(
                    "native runtime object #{id} has no placement cache"
                ));
            }
        }
        if native.selected_person.is_some_and(|id| {
            self.vm
                .world
                .objects
                .get(&id)
                .is_none_or(|o| o.person.is_none())
        }) {
            return Err("native selected person is absent".into());
        }
        for id in [native.indicators.primary, native.indicators.secondary]
            .into_iter()
            .flatten()
        {
            if !world.objects.contains_key(&id) {
                return Err("native runtime indicator is not registered".into());
            }
        }
        world.validate_indicator_queues()?;
        Ok(())
    }
}

#[cfg(test)]
mod validation_tests;

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod ordered_validation_tests;
