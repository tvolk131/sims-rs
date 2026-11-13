use super::*;
use crate::native_world::startup;
use crate::saved_matrix::{AllocationRequest, Lookup, Scratch};
use crate::saved_world;
use serde::{Deserialize, Serialize};

mod geometry;
mod naming_lifecycle;
mod resize;

fn raw_world() -> startup::World {
    let mut execution = returned_construction();
    reach_return(&mut execution, returned::Cut::Returned);
    startup::World::from_returned(execution.into_state().unwrap()).unwrap()
}

fn matrix(world: &startup::World, slot: u8) -> &crate::saved_matrix::Matrix {
    let root = &world.arrays().roots[&slot];
    let interior = u64::from(root.data.len() == 40) * 4;
    &world.arrays().matrices.nodes[&(root.token + interior)].matrix
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceHost {
    side: i16,
    allocation: u64,
    lookups: Vec<u32>,
    sources: usize,
    releases: usize,
    frees: Vec<u64>,
    allocations: usize,
    names: usize,
    lease: bool,
    block_source: bool,
    block_release: bool,
    block_allocate: bool,
    missing: bool,
    name_next: u64,
    name_allocations: usize,
    name_releases: usize,
    live_names: BTreeMap<u64, Vec<u8>>,
    block_name_release: bool,
    pending_name_release: Option<u64>,
    name_alias: Option<u64>,
}

impl SourceHost {
    fn new(side: i16) -> Self {
        Self {
            side,
            allocation: 0x79000000,
            lookups: vec![],
            sources: 0,
            releases: 0,
            frees: vec![],
            allocations: 0,
            names: 0,
            lease: false,
            block_source: false,
            block_release: false,
            block_allocate: false,
            missing: false,
            name_next: 0x7b000000,
            name_allocations: 0,
            name_releases: 0,
            live_names: BTreeMap::new(),
            block_name_release: false,
            pending_name_release: None,
            name_alias: None,
        }
    }
}

impl crate::saved_matrix::Host for SourceHost {
    fn lookup(&mut self, tag: u32, id: u32, callback: u32) -> Result<Lookup, Stop> {
        assert_eq!(tag, u32::from_be_bytes(*b"Arry"));
        assert_eq!(callback, crate::saved_matrix::NOOP_RESOURCE_CALLBACK);
        self.lookups.push(id);
        if self.missing || id != 0 {
            return Ok(Lookup::Missing { native_status: 0 });
        }
        assert!(!self.lease);
        self.lease = true;
        Ok(Lookup::Found { lease: 1 })
    }

    fn source(&mut self, lease: u64) -> Result<Vec<u8>, Stop> {
        assert_eq!(lease, 1);
        assert!(self.lease);
        if self.block_source {
            return Err(Stop::Unavailable("raw source pending".into()));
        }
        self.sources += 1;
        let mut bytes: Vec<_> = [4, self.side, self.side, 0]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect();
        bytes.resize(8 + self.side as usize * 4, 0xcc);
        bytes.resize(
            bytes.len() + self.side as usize * self.side as usize * 4,
            0x52,
        );
        Ok(bytes)
    }

    fn release(&mut self, lease: u64) -> Result<(), Stop> {
        assert_eq!(lease, 1);
        assert!(self.lease);
        if self.block_release {
            return Err(Stop::Unavailable("raw release pending".into()));
        }
        self.lease = false;
        self.releases += 1;
        Ok(())
    }

    fn free(&mut self, token: u64) -> Result<(), Stop> {
        self.frees.push(token);
        Ok(())
    }

    fn allocate(&mut self, request: AllocationRequest) -> Result<Allocated, Stop> {
        if self.block_allocate {
            return Err(Stop::Unavailable("raw allocation pending".into()));
        }
        self.allocations += 1;
        Ok(Allocated::Bytes {
            token: self.allocation,
            data: vec![0xa5; request.data_bytes as usize],
        })
    }
}

impl saved_world::Host for SourceHost {
    fn construct_temporary(
        &mut self,
        _: saved_world::Legacy,
    ) -> Result<saved_world::Temporary, Stop> {
        panic!("version63 never constructs legacy arrays")
    }

    fn destroy_temporary(&mut self, _: &saved_world::Temporary) -> Result<(), Stop> {
        panic!("version63 never destroys legacy arrays")
    }

    fn name_arrays(&mut self) -> Result<(), Stop> {
        self.names += 1;
        Err(Stop::Unsupported(
            "raw owning reader must not use a generic names callback".into(),
        ))
    }
}

impl native_matrix::Host for SourceHost {
    fn require_owner(&self, _: u64, _: u32) -> Result<(), Stop> {
        Err(Stop::Unsupported(
            "naming must not construct matrix replacements".into(),
        ))
    }
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        let token = self.name_alias.take().unwrap_or(self.name_next);
        self.name_next += u64::from((bytes + 15) & !15);
        self.name_allocations += 1;
        let data = vec![0; bytes as usize];
        self.live_names.insert(token, data.clone());
        Ok(Allocated::Bytes { token, data })
    }
    fn release_name(&mut self, token: u64) -> Result<(), Stop> {
        if self.pending_name_release == Some(token) {
            self.pending_name_release = None;
            return Ok(());
        }
        self.live_names
            .remove(&token)
            .ok_or_else(|| Stop::Unavailable("test name allocation is absent".into()))?;
        self.name_releases += 1;
        if self.block_name_release {
            self.block_name_release = false;
            self.pending_name_release = Some(token);
            return Err(Stop::Unavailable(
                "name free completed with acknowledgment pending".into(),
            ));
        }
        Ok(())
    }
}

fn finish(
    world: &mut startup::World,
    execution: &mut saved_world::Execution,
    scratch: &mut Scratch,
    host: &mut SourceHost,
) {
    for node in world.arrays().matrices.nodes.values() {
        host.live_names
            .entry(node.name.token)
            .or_insert_with(|| node.name.data.clone());
    }
    let mut owner =
        startup::reader::Owner::resume_parts(world.clone(), execution.clone(), *scratch).unwrap();
    for _ in 0..512 {
        if owner.step(host).unwrap() == Some(0) {
            let (returned_world, returned_execution, returned_scratch) =
                owner.into_returned().unwrap().into_parts();
            *world = returned_world;
            *execution = returned_execution;
            *scratch = returned_scratch;
            return;
        }
    }
    panic!("raw saved-array sequence did not return");
}

#[test]
fn raw_world_transfers_complete_graph_without_derived_room_rebuild() {
    let world = raw_world();
    assert_eq!(world.arrays(), world.construction().arrays());
    assert_eq!(world.owner(), world.construction().owner());
    assert_eq!(world.active_size(), 64);
    assert_eq!(world.arrays().matrices.nodes.len(), 20);
    assert_eq!(
        world.retained_spans().unwrap(),
        world.construction().population().retained_spans().unwrap()
    );
    for slot in [0x2c, 0x30] {
        let Allocation::Known(backing) = &matrix(&world, slot).allocation else {
            panic!("actual raw primary backing absent");
        };
        assert!(backing.data.iter().all(|byte| *byte == 0));
    }
    let restored: startup::World =
        serde_json::from_slice(&serde_json::to_vec(&world).unwrap()).unwrap();
    restored.validate().unwrap();
    assert_eq!(world, restored);
}

#[test]
fn saved_reader_mutates_actual_registry_matrix_not_constructor_provenance() {
    let mut world = raw_world();
    let original = world.construction().clone();
    let spans = world.retained_spans().unwrap();
    let mut execution = saved_world::Execution::new(63).unwrap();
    let mut host = SourceHost::new(64);
    finish(
        &mut world,
        &mut execution,
        &mut Scratch {
            requested_converter: 0,
        },
        &mut host,
    );
    let Allocation::Known(backing) = &matrix(&world, 0x14).allocation else {
        panic!("live saved backing absent");
    };
    assert_eq!(backing.data, vec![0x52; 64 * 64 * 4]);
    assert_eq!(world.construction(), &original);
    assert_ne!(world.arrays(), original.arrays());
    assert_eq!(world.retained_spans().unwrap(), spans);
    assert_eq!((host.sources, host.releases, host.names), (1, 1, 0));
    assert_eq!(world.namings().len(), 1);
    assert!(host.frees.is_empty());
    assert_eq!(host.allocations, 0);
    for slot in [0x2c, 0x30, 0x4c, 0x50, 0x58, 0x5c] {
        let root = &original.arrays().roots[&slot];
        let interior = u64::from(root.data.len() == 40) * 4;
        assert_eq!(
            matrix(&world, slot),
            &original.arrays().matrices.nodes[&(root.token + interior)].matrix
        );
    }
}

#[test]
fn pending_source_roundtrip_retains_real_lease_without_repeating_lookup() {
    let mut world = raw_world();
    let mut execution = saved_world::Execution::new(63).unwrap();
    let mut scratch = Scratch {
        requested_converter: 0,
    };
    let mut host = SourceHost::new(64);
    host.block_source = true;
    for _ in 0..2 {
        world
            .step_saved_arrays(&mut execution, &mut scratch, &mut host)
            .unwrap();
    }
    for _ in 0..2 {
        assert!(matches!(
            world.step_saved_arrays(&mut execution, &mut scratch, &mut host),
            Err(Stop::Unavailable(_))
        ));
    }
    let (mut world, mut execution, mut scratch, mut host): (
        startup::World,
        saved_world::Execution,
        Scratch,
        SourceHost,
    ) = serde_json::from_slice(&serde_json::to_vec(&(world, execution, scratch, host)).unwrap())
        .unwrap();
    host.block_source = false;
    finish(&mut world, &mut execution, &mut scratch, &mut host);
    assert_eq!(host.lookups.iter().filter(|&&id| id == 0).count(), 1);
    assert_eq!((host.sources, host.releases), (1, 1));
    assert!(!host.lease);
}

#[test]
fn resize_preserves_released_pointer_and_pending_allocator_across_roundtrip() {
    let mut world = raw_world();
    let retained_count = world.retained_spans().unwrap().len();
    let Allocation::Known(original) = &matrix(&world, 0x14).allocation else {
        panic!("raw matrix absent");
    };
    let released = original.token;
    let mut execution = saved_world::Execution::new(63).unwrap();
    let mut scratch = Scratch {
        requested_converter: 0,
    };
    let mut host = SourceHost::new(32);
    host.block_allocate = true;
    let mut stopped = false;
    for _ in 0..30 {
        if matches!(
            world.step_saved_arrays(&mut execution, &mut scratch, &mut host),
            Err(Stop::Unavailable(_))
        ) {
            stopped = true;
            break;
        }
    }
    assert!(stopped);
    world.validate().unwrap();
    assert!(world.arrays().validate().is_err());
    assert_eq!(
        matrix(&world, 0x14).allocation,
        Allocation::Released { token: released }
    );
    let root = &world.arrays().roots[&0x14];
    assert_eq!(&root.data[12..16], &(released as u32).to_le_bytes());
    assert_eq!(matrix(&world, 0x14).width, 32);
    assert_eq!(world.active_size(), 64);
    assert!(
        !world
            .retained_spans()
            .unwrap()
            .iter()
            .any(|span| span.0 == released)
    );
    let (mut world, mut execution, mut scratch, mut host): (
        startup::World,
        saved_world::Execution,
        Scratch,
        SourceHost,
    ) = serde_json::from_slice(&serde_json::to_vec(&(world, execution, scratch, host)).unwrap())
        .unwrap();
    assert!(
        world
            .step_saved_arrays(&mut execution, &mut scratch, &mut host)
            .is_err()
    );
    host.block_allocate = false;
    finish(&mut world, &mut execution, &mut scratch, &mut host);
    assert_eq!(host.frees, [released]);
    assert_eq!(host.allocations, 1);
    assert_eq!(matrix(&world, 0x14).width, 32);
    assert_eq!(world.active_size(), 64);
    assert_eq!(world.retained_spans().unwrap().len(), retained_count);
}

#[test]
fn release_stop_preserves_copied_payload_without_repeating_source() {
    let mut world = raw_world();
    let mut execution = saved_world::Execution::new(63).unwrap();
    let mut scratch = Scratch {
        requested_converter: 0,
    };
    let mut host = SourceHost::new(64);
    host.block_release = true;
    let mut stopped = false;
    for _ in 0..30 {
        if world
            .step_saved_arrays(&mut execution, &mut scratch, &mut host)
            .is_err()
        {
            stopped = true;
            break;
        }
    }
    assert!(stopped && host.lease);
    let copied = matrix(&world, 0x14).clone();
    assert!(
        world
            .step_saved_arrays(&mut execution, &mut scratch, &mut host)
            .is_err()
    );
    assert_eq!(matrix(&world, 0x14), &copied);
    host.block_release = false;
    finish(&mut world, &mut execution, &mut scratch, &mut host);
    assert_eq!((host.sources, host.releases), (1, 1));
}

#[test]
fn resource_alias_from_allocator_is_sticky_and_never_replays_effects() {
    let mut world = raw_world();
    let mut execution = saved_world::Execution::new(63).unwrap();
    let mut scratch = Scratch {
        requested_converter: 0,
    };
    let mut host = SourceHost::new(32);
    host.allocation = world.construction().population().resources().token;
    let mut fault = None;
    for _ in 0..30 {
        if let Err(error) = world.step_saved_arrays(&mut execution, &mut scratch, &mut host) {
            fault = Some(error);
            break;
        }
    }
    assert!(matches!(fault, Some(Stop::Unsupported(_))));
    assert_eq!(host.frees.len(), 1);
    assert_eq!(host.allocations, 1);
    let stopped = serde_json::to_vec(&execution).unwrap();
    host.allocation = 0x79000000;
    assert_eq!(
        world
            .step_saved_arrays(&mut execution, &mut scratch, &mut host)
            .unwrap_err(),
        fault.unwrap()
    );
    assert_eq!(serde_json::to_vec(&execution).unwrap(), stopped);
    assert_eq!(host.allocations, 1);
    let restored: startup::World =
        serde_json::from_slice(&serde_json::to_vec(&world).unwrap()).unwrap();
    assert!(restored.validate().is_err());
}

#[test]
fn raw_owner_rejects_missing_derived_root_and_unbound_name_mutation() {
    let world = raw_world();
    let mut encoded = serde_json::to_value(&world).unwrap();
    encoded["arrays"]["roots"]
        .as_object_mut()
        .unwrap()
        .remove("88");
    let invalid: startup::World = serde_json::from_value(encoded).unwrap();
    assert!(invalid.validate().is_err());
    let mut encoded = serde_json::to_value(&world).unwrap();
    let identity = world.arrays().roots[&0x14].token.to_string();
    encoded["arrays"]["matrices"]["nodes"][&identity]["name"]["data"][0] = 65.into();
    let invalid: startup::World = serde_json::from_value(encoded).unwrap();
    assert!(invalid.validate().is_err());
}

#[test]
fn raw_owner_rejects_derived_primary_replacement_and_unknown_fields() {
    let world = raw_world();
    let mut encoded = serde_json::to_value(&world).unwrap();
    let identity = world.arrays().roots[&0x2c].token.to_string();
    encoded["arrays"]["matrices"]["nodes"][&identity]["matrix"]["allocation"]["Known"]["data"][0] =
        1.into();
    let invalid: startup::World = serde_json::from_value(encoded).unwrap();
    assert!(invalid.validate().is_err());
    let mut encoded = serde_json::to_value(&world).unwrap();
    encoded["canonical_startup_complete"] = true.into();
    assert!(serde_json::from_value::<startup::World>(encoded).is_err());
}

#[test]
fn light_reset_mutates_actual_capacity_without_replacing_vector_owners() {
    let world = raw_world();
    let original = world.construction().clone();
    let mut encoded = serde_json::to_value(&world).unwrap();
    for level in 0..2 {
        encoded["arrays"]["vectors"][level]["backing"]["data"][17] = 12.into();
    }
    let mut world: startup::World = serde_json::from_value(encoded).unwrap();
    world.validate().unwrap();
    let mut execution = saved_world::Execution::new(63).unwrap();
    let mut host = SourceHost::new(64);
    host.missing = true;
    finish(
        &mut world,
        &mut execution,
        &mut Scratch {
            requested_converter: 0,
        },
        &mut host,
    );
    assert_eq!(world.arrays(), original.arrays());
    assert_eq!(world.construction(), &original);
    assert_eq!(host.names, 0);
    assert_eq!(world.namings().len(), 1);
    assert_eq!((host.sources, host.releases, host.allocations), (0, 0, 0));
}
