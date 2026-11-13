use super::*;
use crate::saved_matrix::{Allocation, AllocationRequest, Lookup};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Services {
    owners: BTreeSet<Id>,
    next: u64,
    live: BTreeMap<u64, u32>,
    events: Vec<(String, u64, u32)>,
    bad_name_reply: bool,
    null_matrix_reply: bool,
    block_release: bool,
}

impl Services {
    fn new() -> Self {
        Self {
            next: 0x100000,
            owners: (1..=100).collect(),
            ..Self::default()
        }
    }

    fn allocate_bytes(&mut self, kind: &str, size: u32, data: u32) -> Allocated {
        let token = self.next;
        self.next += u64::from(size) + 16;
        self.live.insert(token, size);
        self.events.push((kind.into(), token, size));
        Allocated::Bytes {
            token,
            data: vec![0xa5; data as usize],
        }
    }
}

impl saved_matrix::Host for Services {
    fn lookup(&mut self, _: u32, _: u32, _: u32) -> Result<Lookup, Stop> {
        Err(Stop::Unsupported(
            "unexpected saved lookup during construction".into(),
        ))
    }
    fn source(&mut self, _: u64) -> Result<Vec<u8>, Stop> {
        Err(Stop::Unsupported(
            "unexpected saved source during construction".into(),
        ))
    }
    fn release(&mut self, _: u64) -> Result<(), Stop> {
        Err(Stop::Unsupported(
            "unexpected saved lease during construction".into(),
        ))
    }
    fn free(&mut self, _: u64) -> Result<(), Stop> {
        Err(Stop::Unsupported(
            "fresh matrix unexpectedly freed an old backing".into(),
        ))
    }
    fn allocate(&mut self, request: AllocationRequest) -> Result<Allocated, Stop> {
        if self.null_matrix_reply {
            self.events
                .push(("null-matrix".into(), 0, request.total_bytes));
            return Ok(Allocated::Null);
        }
        Ok(self.allocate_bytes("matrix", request.total_bytes, request.data_bytes))
    }
}

impl Host for Services {
    fn require_owner(&self, owner: Id, bytes: u32) -> Result<(), Stop> {
        if self.owners.contains(&owner) && bytes == 32 {
            Ok(())
        } else {
            Err(Stop::Unavailable("actual constructor owner absent".into()))
        }
    }
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        let data = if self.bad_name_reply {
            bytes - 1
        } else {
            bytes
        };
        Ok(self.allocate_bytes("name", bytes, data))
    }
    fn release_name(&mut self, token: u64) -> Result<(), Stop> {
        if self.block_release {
            return Err(Stop::Unavailable("name release has not returned".into()));
        }
        let bytes = self
            .live
            .remove(&token)
            .ok_or_else(|| Stop::Unsupported("released matrix name is not live".into()))?;
        self.events.push(("release-name".into(), token, bytes));
        Ok(())
    }
}

fn construct(registry: &mut Registry, host: &mut Services, owner: Id, stride: i32, size: i32) {
    let mut execution = Construction::new(owner, stride, size, size, Vec::new()).unwrap();
    for _ in 0..1000 {
        let returned = execution.step(registry, host).unwrap();
        let encoded =
            serde_json::to_vec(&(registry.clone(), execution.clone(), host.clone())).unwrap();
        let restored: (Registry, Construction, Services) =
            serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            restored,
            (registry.clone(), execution.clone(), host.clone())
        );
        (*registry, execution, *host) = restored;
        if returned == Some(owner) {
            return;
        }
    }
    panic!("matrix constructor exceeded bounded test transitions")
}

fn rename(registry: &mut Registry, host: &mut Services, owner: Id, name: &[u8]) {
    let mut execution = Naming::external(owner, name.to_vec()).unwrap();
    for _ in 0..1000 {
        let returned = execution.step(registry, host).unwrap();
        let encoded =
            serde_json::to_vec(&(registry.clone(), execution.clone(), host.clone())).unwrap();
        let restored: (Registry, Naming, Services) = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            restored,
            (registry.clone(), execution.clone(), host.clone())
        );
        (*registry, execution, *host) = restored;
        if returned {
            return;
        }
    }
    panic!("matrix naming exceeded bounded test transitions")
}

#[test]
fn native_constructor_profiles_allocate_then_clear_all_strides_and_roundtrip_every_step() {
    let mut registry = Registry::default();
    let mut host = Services::new();
    for (index, (stride, size)) in [(1, 64), (2, 64), (4, 64), (14, 64), (2, 128)]
        .into_iter()
        .enumerate()
    {
        let owner = index as u64 + 1;
        construct(&mut registry, &mut host, owner, stride, size);
        let node = &registry.nodes[&owner];
        assert_eq!(node.name.bytes().unwrap(), index.to_string().as_bytes());
        assert_eq!(node.name.data.len(), 8);
        let Allocation::Known(backing) = &node.matrix.allocation else {
            panic!("returned backing absent")
        };
        assert_eq!(backing.data, vec![0; (size * size * stride) as usize]);
        assert_eq!(
            backing.column_offsets,
            (0..size as u32)
                .map(|column| column * size as u32 * stride as u32)
                .collect::<Vec<_>>()
        );
        assert_eq!(registry.head, Some(owner));
    }
    assert_eq!(host.events.len(), 10);
}

#[test]
fn external_naming_unlinks_before_collision_search_and_republishes_original_order() {
    let mut registry = Registry::default();
    let mut host = Services::new();
    for owner in 1..=3 {
        construct(&mut registry, &mut host, owner, 1, 64);
    }
    rename(&mut registry, &mut host, 2, b"ground");
    assert_eq!(registry.head, Some(2));
    assert_eq!(registry.nodes[&2].next, Some(3));
    rename(&mut registry, &mut host, 1, b"ground");
    assert_eq!(registry.nodes[&1].name.bytes().unwrap(), b"ground0");
    rename(&mut registry, &mut host, 2, b"ground");
    assert_eq!(registry.nodes[&2].name.bytes().unwrap(), b"ground");
    assert_eq!(registry.nodes[&2].next, Some(1));
    assert_eq!(registry.nodes[&1].next, Some(3));
    assert_eq!(host.events.len(), 6);
}

#[test]
fn growth_uses_native_capacity_formula_and_releases_only_the_previous_name_buffer() {
    let mut registry = Registry::default();
    let mut host = Services::new();
    construct(&mut registry, &mut host, 1, 1, 64);
    let old = registry.nodes[&1].name.token;
    rename(&mut registry, &mut host, 1, b"altitude");
    assert_eq!(registry.nodes[&1].name.data, b"altitude\0");
    assert!(!host.live.contains_key(&old));
    assert_eq!(host.events[2].0, "name");
    assert_eq!(host.events[2].2, 9);
    assert_eq!(host.events[3], ("release-name".into(), old, 8));
}

#[test]
fn collision_growth_uses_previous_length_not_previous_capacity() {
    let mut registry = Registry::default();
    let mut host = Services::new();
    for owner in 1..=2 {
        construct(&mut registry, &mut host, owner, 1, 64);
    }
    rename(&mut registry, &mut host, 1, b"crowded");
    rename(&mut registry, &mut host, 2, b"crowded");
    assert_eq!(registry.nodes[&2].name.bytes().unwrap(), b"crowded0");
    assert_eq!(registry.nodes[&2].name.data.len(), 15);
}

#[test]
fn unreturned_release_retains_the_pending_allocation_without_repeating_its_request() {
    let mut registry = Registry::default();
    let mut host = Services::new();
    construct(&mut registry, &mut host, 1, 1, 64);
    let mut execution = Naming::external(1, b"altitude".to_vec()).unwrap();
    host.block_release = true;
    for _ in 0..100 {
        match execution.step(&mut registry, &mut host) {
            Ok(false) => {}
            Err(Stop::Unavailable(_)) => break,
            other => panic!("unexpected pending name release: {other:?}"),
        }
    }
    assert_eq!(host.events.len(), 3);
    let before = (registry.clone(), execution.clone(), host.clone());
    assert!(execution.step(&mut registry, &mut host).is_err());
    assert_eq!((registry.clone(), execution.clone(), host.clone()), before);
    let encoded = serde_json::to_vec(&before).unwrap();
    (registry, execution, host) = serde_json::from_slice(&encoded).unwrap();
    host.block_release = false;
    for _ in 0..100 {
        if execution.step(&mut registry, &mut host).unwrap() {
            break;
        }
    }
    assert_eq!(host.events.len(), 4);
    assert_eq!(registry.nodes[&1].name.bytes().unwrap(), b"altitude");
}

#[test]
fn invalid_allocator_reply_is_retained_without_reallocation_or_header_publication() {
    let mut registry = Registry::default();
    let mut host = Services::new();
    host.bad_name_reply = true;
    let mut execution = Construction::new(1, 1, 64, 64, Vec::new()).unwrap();
    execution.step(&mut registry, &mut host).unwrap();
    execution.step(&mut registry, &mut host).unwrap();
    for _ in 0..3 {
        assert!(execution.step(&mut registry, &mut host).is_err());
    }
    assert_eq!(host.events.len(), 1);
    assert_eq!(registry, Registry::default());
}

#[test]
fn absent_or_duplicate_owner_stops_before_any_new_allocation() {
    let mut registry = Registry::default();
    let mut host = Services::new();
    let mut absent = Construction::new(101, 1, 64, 64, Vec::new()).unwrap();
    assert!(absent.step(&mut registry, &mut host).is_err());
    assert!(host.events.is_empty());
    construct(&mut registry, &mut host, 1, 1, 64);
    let before = (registry.clone(), host.clone());
    let mut duplicate = Construction::new(1, 1, 64, 64, Vec::new()).unwrap();
    assert!(duplicate.step(&mut registry, &mut host).is_err());
    assert_eq!((registry, host), before);
}

#[test]
fn null_matrix_allocation_does_not_become_a_successful_native_constructor_return() {
    let mut registry = Registry::default();
    let mut host = Services::new();
    host.null_matrix_reply = true;
    let mut execution = Construction::new(1, 1, 64, 64, Vec::new()).unwrap();
    let mut stopped = false;
    for _ in 0..100 {
        match execution.step(&mut registry, &mut host) {
            Ok(None) => {}
            Err(Stop::Unavailable(_)) => {
                stopped = true;
                break;
            }
            other => panic!("unexpected null matrix allocation: {other:?}"),
        }
    }
    assert!(stopped);
    assert!(execution.step(&mut registry, &mut host).is_err());
    assert_eq!(host.events.len(), 2);
    assert_eq!(registry.nodes[&1].matrix.allocation, Allocation::Null);
}

#[test]
fn invalid_names_profiles_registry_cycles_and_allocation_aliases_fail_closed() {
    assert!(Naming::external(1, vec![b'a'; 512]).is_err());
    assert!(Naming::external(1, b"name\0tail".to_vec()).is_err());
    assert!(Construction::new(1, 1, 128, 128, Vec::new()).is_err());
    let mut registry = Registry::default();
    let mut host = Services::new();
    for owner in 1..=2 {
        construct(&mut registry, &mut host, owner, 1, 64);
    }
    let mut cycle = registry.clone();
    cycle.nodes.get_mut(&1).unwrap().next = Some(2);
    assert!(cycle.validate().is_err());
    let duplicated = registry.nodes[&1].name.token;
    registry.nodes.get_mut(&2).unwrap().name.token = duplicated;
    assert!(registry.validate().is_err());
}
