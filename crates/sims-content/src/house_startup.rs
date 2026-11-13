//! Explicit retained owner at415e20 entry, after SIMI and before saved ObjM.
//! The supported resize profile is a shrinking lot with off-lot objects.
//! Ordinary3D owners require the retained asset host; pending resize/placement
//! services must be implemented before that particular hot world can proceed.
use crate::{save_storage::Execution, saves::House};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::{
    Simulation,
    architecture::Architecture,
    lot_environment::{
        Tuning,
        manager::{Manager, Settings},
    },
    lot_geometry::Lot,
    lot_links::Links,
    lot_placement, lot_runtime,
};
use sims_vm::{Program, Vm};
use std::collections::BTreeSet;

pub mod saved_arrays;

/// One retained asset owner spans resize/clear and the saved-object reader.
/// Preparation validates existing storage; it must not initialize replacements.
pub trait AssetHost {
    fn prepare_incoming(
        &mut self,
        vm: &Vm,
        program: &Program,
    ) -> Result<BTreeSet<sims_vm::ObjectId>>;
    fn service_teardown(&mut self, simulation: &mut Simulation, program: &Program) -> Result<()>;
    fn finish_teardown(&mut self, vm: &Vm) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incoming {
    pub vm: Vm,
    pub lot: Lot,
    pub indicators: sims_vm::load_indicators::Indicators,
    pub pools: sims_vm::indicator_pools::Pools,
    pub room_settings: Settings,
}

impl Incoming {
    /// Consume, never reconstruct, the incoming VM. The boundary callback is a
    /// read-only observer for headless differential capture.
    pub fn prepare_read(
        self,
        house: &House,
        program: &Program,
        catalog: &[crate::catalog::Controller],
        boundary: impl FnMut(&str, &Vm, &[sims_vm::removal::Pending]) -> Result<()>,
        host: &mut impl FnMut(&mut Simulation, &Program) -> Result<(), String>,
    ) -> Result<Execution> {
        self.validate_incoming()?;
        require_no_unowned_graphics(&self.vm, program)?;
        self.prepare_read_inner(house, program, catalog, boundary, BTreeSet::new(), host)
    }

    /// Use the same real asset host before and after registry destruction. The
    /// returned identity set records actual positive Animator ownership; it is
    /// independent of person settings or a definition's current graphics bit.
    pub fn prepare_read_with_assets(
        self,
        house: &House,
        program: &Program,
        catalog: &[crate::catalog::Controller],
        boundary: impl FnMut(&str, &Vm, &[sims_vm::removal::Pending]) -> Result<()>,
        host: &mut dyn AssetHost,
    ) -> Result<Execution> {
        self.validate_incoming()?;
        let owners = host.prepare_incoming(&self.vm, program)?;
        for &id in &owners {
            ensure!(
                self.vm
                    .world
                    .objects
                    .get(&id)
                    .is_some_and(|o| o.person.is_none()),
                "incoming graphic has no ordinary object: {id}"
            );
        }
        let execution =
            self.prepare_read_inner(house, program, catalog, boundary, owners, &mut |s, p| {
                host.service_teardown(s, p).map_err(|e| format!("{e:#}"))
            })?;
        host.finish_teardown(&execution.vm)?;
        Ok(execution)
    }

    /// Retain the complete simulation after the original first clear. This
    /// consumes no saved House file, replays no TATT and does not discard its
    /// architecture, links, indicator pools or teardown ownership into a new VM.
    pub fn prepare_cleared_with_assets(
        self,
        program: &Program,
        catalog: &[crate::catalog::Controller],
        boundary: impl FnMut(&str, &Vm, &[sims_vm::removal::Pending]) -> Result<()>,
        host: &mut dyn AssetHost,
    ) -> Result<Simulation> {
        self.validate_incoming()?;
        let owners = host.prepare_incoming(&self.vm, program)?;
        for &id in &owners {
            ensure!(
                self.vm
                    .world
                    .objects
                    .get(&id)
                    .is_some_and(|o| o.person.is_none()),
                "incoming graphic has no ordinary object: {id}"
            );
        }
        let simulation =
            self.prepare_cleared_inner(program, catalog, boundary, owners, &mut |s, p| {
                host.service_teardown(s, p).map_err(|e| format!("{e:#}"))
            })?;
        host.finish_teardown(&simulation.vm)?;
        Ok(simulation)
    }

    fn validate_incoming(&self) -> Result<()> {
        self.lot.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.vm.world.positions.values().all(|p| p.x < 0 && p.y < 0),
            "retained startup currently requires off-lot incoming objects"
        );
        ensure!(
            self.vm.world.objects.values().all(|o| o.person.is_none()),
            "retained startup requires an incoming character host"
        );
        Ok(())
    }

    fn prepare_read_inner(
        self,
        house: &House,
        program: &Program,
        catalog: &[crate::catalog::Controller],
        mut boundary: impl FnMut(&str, &Vm, &[sims_vm::removal::Pending]) -> Result<()>,
        owners: BTreeSet<sims_vm::ObjectId>,
        host: &mut impl FnMut(&mut Simulation, &Program) -> Result<(), String>,
    ) -> Result<Execution> {
        let mut simulation =
            self.prepare_cleared_inner(program, catalog, &mut boundary, owners, host)?;
        //Historical unqualified suffix: the native next stage reads saved-world
        //arrays and Gtab/objt, not Neighborhood TATT. Verified callers use the
        //retained cleared-Simulation seam until that source pipeline is bound.
        if let Some(saved) = &house.resource_storage {
            simulation.vm.world.resource_storage.load(saved);
        }
        boundary("tatt_return", &simulation.vm, &simulation.removals)?;
        //43fc02 clears again before449c5e. Do not replace this with new World.
        simulation
            .clear_for_house_read(program, host)
            .map_err(anyhow::Error::msg)?;
        boundary("objm_read_entry", &simulation.vm, &simulation.removals)?;
        let mut execution = Execution::from_retained(simulation.vm);
        execution.removals = simulation.removals;
        Ok(execution)
    }

    fn prepare_cleared_inner(
        self,
        program: &Program,
        catalog: &[crate::catalog::Controller],
        mut boundary: impl FnMut(&str, &Vm, &[sims_vm::removal::Pending]) -> Result<()>,
        owners: BTreeSet<sims_vm::ObjectId>,
        host: &mut impl FnMut(&mut Simulation, &Program) -> Result<(), String>,
    ) -> Result<Simulation> {
        let mut execution = Execution::from_retained(self.vm);
        //Retain the actual owner through ObjM read. The backing allocation and
        //global23 cannot reconstruct a missing pre-resize world+10 field.
        let map = execution
            .vm
            .world
            .map_size
            .context("retained startup active world size is absent")?;
        map.checked_size(self.lot.size)
            .map_err(anyhow::Error::msg)?;
        boundary("resize_entry", &execution.vm, &execution.removals)?;
        let requested = execution
            .vm
            .world
            .globals
            .get(&23)
            .copied()
            .context("resize global23 absent")?;
        let requested = if requested == 0 {
            64
        } else {
            i32::from(requested)
        };
        let resized = execution
            .vm
            .world
            .map_size
            .as_mut()
            .unwrap()
            .begin_resize(requested)
            .is_some();
        let lot = if resized {
            shrink(self.lot, requested.try_into()?)?
        } else {
            self.lot
        };
        let architecture = Architecture::from_original(lot, program).map_err(anyhow::Error::msg)?;
        let mut links = Links {
            size: architecture.original.as_ref().unwrap().size,
            heads: architecture
                .original
                .as_ref()
                .unwrap()
                .floors
                .iter()
                .map(|f| f.source.object_heads.iter().map(|&id| id as i16).collect())
                .collect(),
            next: execution
                .vm
                .world
                .objects
                .keys()
                .map(|&id| (id, 0))
                .collect(),
        };
        links
            .publish_tile_chains(&mut execution.vm.world)
            .map_err(anyhow::Error::msg)?;
        if resized {
            //442866 cannot delete a negative coarse X/Y: its conjunction
            //requires both coordinates >= size-1. No incoming thread runs.
            architecture.publish_room_context(&mut execution.vm.world);
            let refresh = execution
                .vm
                .world
                .refresh
                .as_mut()
                .context("resize refresh owner absent")?;
            refresh
                .rooms
                .retain(|id, _| architecture.outdoors.contains_key(&(*id as i16)));
            for room in refresh.rooms.values_mut() {
                room.dirty = true;
            }
            refresh.dirty_categories = 3;
            let lot = architecture.original.as_ref().unwrap();
            let mut manager = Manager::new(
                lot,
                Tuning::from_program(program).map_err(anyhow::Error::msg)?,
                self.room_settings,
            )
            .map_err(anyhow::Error::msg)?;
            let period = execution.vm.world.globals[&4];
            manager
                .update(&mut execution.vm, program, lot, &links, period)
                .map_err(anyhow::Error::msg)?;
            //105's object-registry listener invokes442b9f synchronously.
            execution.reconcile_global_controllers(program, catalog)?;
        }
        boundary("resize_return", &execution.vm, &execution.removals)?;
        execution.vm.world.globals.insert(9, 0);
        for (&id, position) in &execution.vm.world.positions {
            links.next.entry(id).or_insert(0);
            execution
                .vm
                .world
                .refresh
                .as_mut()
                .context("clear refresh absent")?
                .draw
                .entry(id)
                .or_insert_with(|| sims_vm::refresh::DrawState::constructed(position.level));
        }
        let placement =
            lot_placement::State::new_for_world(links, &architecture, &execution.vm.world)
                .map_err(anyhow::Error::msg)?;
        let membership = sims_vm::person_loading::RoomMembership::constructed(&execution.vm.world);
        let mut simulation = Simulation::from_loaded_lot(
            execution.vm,
            architecture,
            lot_runtime::State {
                placement,
                membership,
                indicators: self.indicators,
                observers: vec![],
                diagnostics: sims_sim::lot_acceptance::person::Diagnostics {
                    error: 0,
                    blocker: None,
                },
                route: None,
                shoo: None,
                movement: None,
                completion: None,
                route_paths: Default::default(),
                snap: None,
                find_location: None,
                removal: None,
                snap_offsets: Default::default(),
                selected_person: None,
                room_effects: vec![],
                routing_policy: Default::default(),
            },
        )
        .map_err(anyhow::Error::msg)?;
        simulation.retained_ordinary_graphics = owners;
        simulation.native_indicator_pools = self.pools;
        simulation.removals = execution.removals;
        boundary("clear_entry", &simulation.vm, &simulation.removals)?;
        simulation
            .clear_for_house_read(program, host)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            simulation.retained_ordinary_graphics.is_empty(),
            "house clear retained an ordinary Animator owner"
        );
        boundary("clear_return", &simulation.vm, &simulation.removals)?;
        Ok(simulation)
    }
}

/// The diagnostic path has no asset host. A positive definition bit must not be
/// ignored because Simulation has no person settings. Reject before resize/RNG
/// work. A clear bit alone is not proof of a null native object+120 pointer;
/// actual hot imports must bind all observed owners through the asset host.
fn require_no_unowned_graphics(vm: &Vm, program: &Program) -> Result<()> {
    for (&id, object) in &vm.world.objects {
        let definition = object
            .definition
            .as_ref()
            .with_context(|| format!("retained startup definition absent: {id}"))?;
        let words = sims_vm::construction::definition_words(program, definition)
            .map_err(anyhow::Error::msg)?;
        let flags = words
            .get(102)
            .with_context(|| format!("retained startup graphic flags absent: {id}"))?;
        ensure!(
            flags & 1 == 0,
            "retained startup ordinary graphic owner is absent: {id} ({:08x})",
            definition.guid
        );
    }
    Ok(())
}

///4778fc changes world+10 without shrinking its allocated arrays. It clears
///only the new far row and column within the requested active square.
fn shrink(mut lot: Lot, size: usize) -> Result<Lot> {
    ensure!(
        size >= 4 && size < lot.active_extent(),
        "retained resize growth requires full backing arrays"
    );
    for coordinate in 0..size {
        for cell in [
            (size - 1) * lot.size + coordinate,
            coordinate * lot.size + size - 1,
        ] {
            lot.pool[cell] = 255;
            lot.water[cell] = 255;
            for floor in &mut lot.floors {
                floor.source.coverings[cell] = 0;
                floor.source.walls[cell] = Default::default();
                floor.source.object_heads[cell] = 0;
                floor.source.flags[cell] = 0;
            }
        }
    }
    Lot::rebuild_in_active(
        lot.size,
        size,
        lot.heights,
        lot.pool,
        lot.water,
        lot.floors.into_iter().map(|floor| floor.source).collect(),
    )
    .map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrink_retains_allocated_cells_and_uses_active_stride_for_the_far_border() {
        let source = sims_sim::lot_geometry::FloorSource {
            coverings: vec![45; 64],
            walls: vec![Default::default(); 64],
            object_heads: vec![19; 64],
            flags: vec![9; 64],
        };
        let original = Lot::rebuild(
            8,
            vec![[1, 2, 3, 4]; 64],
            vec![7; 64],
            vec![9; 64],
            vec![source.clone(), source],
        )
        .unwrap();
        let shrunk = shrink(original.clone(), 6).unwrap();
        assert_eq!(shrunk.size, 8);
        assert_eq!(shrunk.active_extent(), 6);
        assert_eq!(shrunk.heights, original.heights);
        for x in 0..8 {
            for y in 0..8 {
                let cell = x * 8 + y;
                let border = (x == 5 && y < 6) || (y == 5 && x < 6);
                assert_eq!(shrunk.pool[cell], if border { 255 } else { 7 });
                assert_eq!(shrunk.water[cell], if border { 255 } else { 9 });
                for floor in &shrunk.floors {
                    assert_eq!(floor.source.coverings[cell], if border { 0 } else { 45 });
                    assert_eq!(floor.source.object_heads[cell], if border { 0 } else { 19 });
                    assert_eq!(floor.source.flags[cell], if border { 0 } else { 9 });
                    if x >= 6 || y >= 6 {
                        assert_eq!(floor.rooms.primary[cell], -1);
                    }
                }
            }
        }
        shrunk.validate().unwrap();
        let smaller = shrink(shrunk, 4).unwrap();
        assert_eq!(smaller.size, 8);
        assert_eq!(smaller.active_extent(), 4);
        assert_eq!(smaller.floors[0].source.coverings[7 * 8 + 7], 45);
        assert!(shrink(smaller, 5).is_err());
    }

    fn fixture(flags: Option<i16>) -> (Program, Vm) {
        let mut words = vec![0; flags.map_or(59, |_| 103)];
        if let Some(flags) = flags {
            words[102] = flags;
        }
        let program = Program {
            modules: [(
                "fixture".into(),
                sims_vm::Module {
                    object_definition_words: [(128, words)].into(),
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        };
        let mut vm = Vm::new(sims_vm::World::default(), 0x12345678);
        vm.tick = 88444;
        vm.lot_rng_state = Some(0x33445566);
        vm.crt_rng_state = Some(0x778899aa);
        vm.world.objects.insert(
            1,
            sims_vm::Object {
                definition: Some(sims_vm::Definition {
                    source: "fixture".into(),
                    object: 128,
                    guid: 0x2f726fff,
                    slots: 0,
                }),
                ..Default::default()
            },
        );
        (program, vm)
    }

    #[test]
    fn unsupported_incoming_animator_stops_without_vm_or_rng_mutation() {
        for flags in [1, 3, -1] {
            let (program, vm) = fixture(Some(flags));
            let before = vm.clone();
            let error = require_no_unowned_graphics(&vm, &program).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("ordinary graphic owner is absent: 1 (2f726fff)")
            );
            assert_eq!(vm, before);
        }
    }

    #[test]
    fn absent_flags_are_unknown_and_non_graphic_fixture_is_unchanged() {
        let (program, vm) = fixture(None);
        let before = vm.clone();
        assert!(
            require_no_unowned_graphics(&vm, &program)
                .unwrap_err()
                .to_string()
                .contains("graphic flags absent")
        );
        assert_eq!(vm, before);
        let (program, vm) = fixture(Some(2));
        let before = vm.clone();
        require_no_unowned_graphics(&vm, &program).unwrap();
        assert_eq!(vm, before);
    }
}
