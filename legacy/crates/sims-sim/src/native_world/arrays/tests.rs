use super::*;
use crate::native_world::{self, Request};
use crate::saved_matrix::{self, AllocationRequest, Lookup};

mod game;
mod resource_paths;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Event {
    Prefix(Request),
    Root(u8, u32),
    Name(u32),
    Backing(AllocationRequest),
    Vector(u32),
    Resource(u8, u32),
    Free(u64),
    ScenarioOpen(u64),
    ScenarioClose(u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Services {
    next: u64,
    live: BTreeMap<u64, u32>,
    events: Vec<Event>,
    deny_names: bool,
    root_reply: Option<Allocated>,
    vector_reply: Option<Allocated>,
    resource_reply: Option<Allocated>,
    deny_resources: bool,
    name_alias: Option<u64>,
    backing_alias: Option<u64>,
}

impl Services {
    fn new() -> Self {
        Self {
            next: 0x100000,
            live: BTreeMap::from([(0x1000, 324)]),
            events: Vec::new(),
            deny_names: false,
            root_reply: None,
            vector_reply: None,
            resource_reply: None,
            deny_resources: false,
            name_alias: None,
            backing_alias: None,
        }
    }

    fn allocation(&mut self, bytes: u32) -> Allocated {
        let token = self.next;
        self.next += u64::from((bytes + 15) & !15);
        assert!(self.live.insert(token, bytes).is_none());
        Allocated::Bytes {
            token,
            data: vec![0xa5; bytes as usize],
        }
    }
}

impl native_world::resources::Host for Services {
    fn allocate_resource_vector(&mut self, vector: u8, bytes: u32) -> Result<Allocated, Stop> {
        if self.deny_resources {
            return Err(Stop::Unavailable(
                "resource allocator temporarily unavailable".into(),
            ));
        }
        self.events.push(Event::Resource(vector, bytes));
        Ok(self
            .resource_reply
            .take()
            .unwrap_or_else(|| self.allocation(bytes)))
    }
}

impl native_world::Host for Services {
    fn allocate(&mut self, request: Request) -> Result<Allocated, Stop> {
        self.events.push(Event::Prefix(request));
        Ok(self.allocation(request.bytes()?))
    }
}

impl saved_matrix::Host for Services {
    fn lookup(&mut self, _: u32, _: u32, _: u32) -> Result<Lookup, Stop> {
        Err(Stop::Unsupported(
            "fresh arrays must not read saved sources".into(),
        ))
    }
    fn source(&mut self, _: u64) -> Result<Vec<u8>, Stop> {
        Err(Stop::Unsupported("fresh array source unavailable".into()))
    }
    fn release(&mut self, _: u64) -> Result<(), Stop> {
        Err(Stop::Unsupported("fresh arrays own no source lease".into()))
    }
    fn free(&mut self, token: u64) -> Result<(), Stop> {
        self.events.push(Event::Free(token));
        Err(Stop::Unsupported(
            "fresh array teardown must not invent backing frees".into(),
        ))
    }
    fn allocate(&mut self, request: AllocationRequest) -> Result<Allocated, Stop> {
        self.events.push(Event::Backing(request));
        if let Some(token) = self.backing_alias.take() {
            return Ok(Allocated::Bytes {
                token,
                data: vec![0xa5; request.data_bytes as usize],
            });
        }
        let Allocated::Bytes { token, mut data } = self.allocation(request.total_bytes) else {
            unreachable!()
        };
        data.truncate(request.data_bytes as usize);
        Ok(Allocated::Bytes { token, data })
    }
}

impl native_matrix::Host for Services {
    fn require_owner(&self, owner: u64, bytes: u32) -> Result<(), Stop> {
        if self.live.iter().any(|(start, extent)| {
            *start <= owner && owner + u64::from(bytes) <= start + u64::from(*extent)
        }) {
            Ok(())
        } else {
            Err(Stop::Unavailable(
                "matrix header lacks its real root allocation".into(),
            ))
        }
    }
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        if self.deny_names {
            return Err(Stop::Unavailable(
                "name allocator temporarily unavailable".into(),
            ));
        }
        self.events.push(Event::Name(bytes));
        if let Some(token) = self.name_alias.take() {
            return Ok(Allocated::Bytes {
                token,
                data: vec![0xa5; bytes as usize],
            });
        }
        Ok(self.allocation(bytes))
    }
    fn release_name(&mut self, token: u64) -> Result<(), Stop> {
        if self.live.remove(&token).is_none() {
            return Err(Stop::Unsupported(
                "name/temporary free lacks actual ownership".into(),
            ));
        }
        self.events.push(Event::Free(token));
        Ok(())
    }
}

impl vector::Host for Services {
    fn allocate_vector_elements(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        self.events.push(Event::Vector(bytes));
        Ok(self
            .vector_reply
            .take()
            .unwrap_or_else(|| self.allocation(bytes)))
    }
}

impl Host for Services {
    fn allocate_array_root(&mut self, slot: u8, bytes: u32) -> Result<Allocated, Stop> {
        self.events.push(Event::Root(slot, bytes));
        Ok(self
            .root_reply
            .take()
            .unwrap_or_else(|| self.allocation(bytes)))
    }
}

fn construction(context: Context) -> (Construction, Services) {
    let mut host = Services::new();
    let mut registry = sims_vm::commander::Registry::<u32>::default();
    registry.construct(0x1000).unwrap();
    registry.bind(0x1000, 0x667ec0).unwrap();
    let mut prefix = Prefix::new(registry).unwrap();
    for _ in 0..100 {
        if prefix.step(&mut host).unwrap().is_some() {
            break;
        }
    }
    (Construction::new(prefix, context).unwrap(), host)
}

fn finish(construction: &mut Construction, host: &mut Services, roundtrip: bool) -> usize {
    let mut transitions = 0;
    for _ in 0..5000 {
        let previous_events = host.events.len();
        let cut = construction.step(host).unwrap();
        if roundtrip && (previous_events != host.events.len() || cut.is_some()) {
            let encoded = serde_json::to_vec(&(&construction, &host)).unwrap();
            let restored: (Construction, Services) = serde_json::from_slice(&encoded).unwrap();
            assert_eq!((&restored.0, &restored.1), (&*construction, &*host));
            (*construction, *host) = restored;
            transitions += 1;
        }
        if cut.is_some() {
            return transitions;
        }
    }
    panic!("actual fresh world-array sequence did not reach its bounded cut");
}

#[test]
fn genuine_complete_array_ownership_names_payloads_and_service_roundtrips() {
    let context = Context {
        roof_owners: 0,
        random: 0x87654321,
    };
    let (mut execution, mut host) = construction(context);
    let suffix = execution.state.prefix.owner.data[0x70..].to_vec();
    assert!(finish(&mut execution, &mut host, true) > 100);
    assert_eq!(execution.cut(), Some(Cut::CoordinateTablesEntry));
    assert_eq!(execution.state.prefix.owner.data[0x70..], suffix);
    assert_eq!(
        execution.state.context,
        Context {
            roof_owners: 2,
            random: context.random
        }
    );
    let roots: Vec<_> = host
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Root(slot, bytes) => Some((*slot, *bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(roots, SPECS.map(|spec| (spec.slot, spec.extent() as u32)));
    assert_eq!(
        host.events
            .iter()
            .filter(|event| matches!(event, Event::Name(_)))
            .count(),
        61
    );
    assert_eq!(
        host.events
            .iter()
            .filter(|event| matches!(event, Event::Free(_)))
            .count(),
        41
    );
    assert_eq!(
        host.events
            .iter()
            .filter(|event| matches!(event, Event::Backing(_)))
            .count(),
        20
    );
    assert_eq!(
        host.events
            .iter()
            .filter(|event| matches!(event, Event::Vector(16384)))
            .count(),
        2
    );
    assert_eq!(host.live.len(), 72);
    let before = execution.clone();
    let calls = host.events.len();
    assert_eq!(
        execution.step(&mut host).unwrap(),
        Some(Cut::CoordinateTablesEntry)
    );
    assert_eq!(execution, before);
    assert_eq!(host.events.len(), calls);
    let state = execution.into_state().unwrap();
    let grass = &state.matrices.nodes[&state.identity(0x1c).unwrap()].matrix;
    let target = &state.matrices.nodes[&state.identity(0x20).unwrap()].matrix;
    let (Allocation::Known(grass), Allocation::Known(target)) =
        (&grass.allocation, &target.allocation)
    else {
        panic!("actual generated grass ownership absent")
    };
    assert_eq!(grass.data, target.data);
    assert!(grass.data.iter().any(|byte| *byte != 0));
}

#[test]
fn active_size_publication_precedes_real_empty_teardown_and_first_allocation() {
    assert_eq!(
        empty_teardown_slots().collect::<Vec<_>>(),
        [
            0x14, 0x18, 0x1c, 0x20, 0x24, 0x44, 0x3c, 0x2c, 0x34, 0x4c, 0x68, 0x58, 0x28, 0x48,
            0x40, 0x30, 0x38, 0x50, 0x6c, 0x5c,
        ]
    );
    let (mut execution, mut host) = construction(Context {
        roof_owners: 0,
        random: 0,
    });
    let before = host.events.len();
    assert_eq!(&execution.state.prefix.owner.data[0x10..0x14], &[0; 4]);
    execution.step(&mut host).unwrap();
    assert_eq!(
        &execution.state.prefix.owner.data[0x10..0x14],
        &64_u32.to_le_bytes()
    );
    assert_eq!(host.events.len(), before);
    execution.step(&mut host).unwrap();
    assert_eq!(host.events.len(), before);
    execution.step(&mut host).unwrap();
    assert_eq!(host.events.last(), Some(&Event::Root(0x14, 32)));
    assert!(execution.into_state().is_err());
}

#[test]
fn null_and_malformed_roots_remain_pending_without_positive_size_success() {
    for reply in [
        Allocated::Null,
        Allocated::Bytes {
            token: 0,
            data: vec![0; 32],
        },
        Allocated::Bytes {
            token: 0x100004,
            data: vec![0; 32],
        },
        Allocated::Bytes {
            token: 0xfffffff0,
            data: vec![0; 32],
        },
        Allocated::Bytes {
            token: 0x200000,
            data: vec![0; 31],
        },
    ] {
        let (mut execution, mut host) = construction(Context {
            roof_owners: 0,
            random: 0,
        });
        host.root_reply = Some(reply);
        for _ in 0..3 {
            execution.step(&mut host).unwrap();
        }
        let retained = execution.clone();
        let events = host.events.len();
        assert!(execution.step(&mut host).is_err());
        assert_eq!(execution, retained);
        assert!(execution.step(&mut host).is_err());
        assert_eq!(host.events.len(), events);
        assert_eq!(execution.cut(), None);
    }
}

#[test]
fn unavailable_temporary_allocator_retries_without_reallocating_the_root() {
    let (mut execution, mut host) = construction(Context {
        roof_owners: 0,
        random: 0,
    });
    for _ in 0..4 {
        execution.step(&mut host).unwrap();
    }
    host.deny_names = true;
    let retained = execution.clone();
    let events = host.events.len();
    assert!(execution.step(&mut host).is_err());
    assert_eq!(execution, retained);
    assert_eq!(host.events.len(), events);
    host.deny_names = false;
    finish(&mut execution, &mut host, false);
    assert_eq!(
        host.events
            .iter()
            .filter(|event| matches!(event, Event::Root(0x14, 32)))
            .count(),
        1
    );
}

#[test]
fn actual_roof_counter_wraps_and_native_grass_preserves_supplied_rng() {
    for initial in [u32::MAX - 1, u32::MAX] {
        let context = Context {
            roof_owners: initial,
            random: 0xffffffff,
        };
        let (mut execution, mut host) = construction(context);
        finish(&mut execution, &mut host, false);
        assert_eq!(
            execution.state.context,
            Context {
                roof_owners: initial.wrapping_add(2),
                random: context.random
            }
        );
    }
}

#[test]
fn malformed_vector_elements_remain_retained_without_repeated_growth_allocation() {
    let (mut execution, mut host) = construction(Context {
        roof_owners: 0,
        random: 0,
    });
    host.vector_reply = Some(Allocated::Bytes {
        token: 0x200000,
        data: vec![0; 16383],
    });
    let mut rejected = false;
    for _ in 0..3000 {
        if execution.step(&mut host).is_err() {
            rejected = true;
            break;
        }
    }
    assert!(rejected);
    let retained = execution.clone();
    let events = host.events.len();
    assert!(execution.step(&mut host).is_err());
    assert_eq!(execution, retained);
    assert_eq!(host.events.len(), events);
    assert_eq!(
        host.events
            .iter()
            .filter(|event| matches!(event, Event::Vector(_)))
            .count(),
        1
    );
    assert_eq!(execution.cut(), None);
}

#[test]
fn corrupted_restored_ownership_and_derived_fields_do_not_become_success() {
    let (mut execution, mut host) = construction(Context {
        roof_owners: 0,
        random: 0,
    });
    finish(&mut execution, &mut host, false);
    let mut variants = Vec::new();
    let mut changed = execution.clone();
    changed.state.roots.get_mut(&0x60).unwrap().data[0x24] = 0;
    variants.push(changed);
    let mut changed = execution.clone();
    changed.state.vectors[0]
        .as_mut()
        .unwrap()
        .backing
        .as_mut()
        .unwrap()
        .data[0] = 0;
    variants.push(changed);
    let mut changed = execution.clone();
    changed.state.prefix.owner.data[0x58] ^= 1;
    variants.push(changed);
    let mut changed = execution.clone();
    changed.state.context.roof_owners = 0;
    variants.push(changed);
    let mut changed = execution.clone();
    changed.pending = Some(Allocated::Null);
    variants.push(changed);
    for changed in variants {
        let mut restored: Construction =
            serde_json::from_slice(&serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(restored.step(&mut host).is_err());
        assert!(restored.into_state().is_err());
    }
}

fn fixed_construction(current_world: u32) -> (native_world::fixed::Construction, Services) {
    let (mut arrays, mut host) = construction(Context {
        roof_owners: 0,
        random: 0x12345678,
    });
    finish(&mut arrays, &mut host, false);
    (
        native_world::fixed::Construction::new(
            arrays,
            native_world::fixed::Context {
                requested: 64,
                current_world,
            },
        )
        .unwrap(),
        host,
    )
}

#[test]
fn fixed_tables_continue_actual_array_ownership_without_fabricated_services() {
    let (mut execution, host) = fixed_construction(0);
    let original = execution.arrays().clone();
    let events = host.events.clone();
    let expected_regions = [0, 1, 1, 1, 2, 4];
    assert!(execution.clone().into_state().is_err());
    for (index, count) in expected_regions.into_iter().enumerate() {
        if index != 0 {
            execution.step().unwrap();
            let restored =
                serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
            assert_eq!(execution, restored);
            execution = restored;
        }
        assert_eq!(execution.tables().regions().len(), count);
        assert_eq!(execution.arrays(), &original);
        assert_eq!(host.events, events);
        assert_eq!(execution.cut().is_some(), index == 5);
    }
    assert_eq!(execution.step().unwrap(), execution.cut());
    let state = execution.into_state().unwrap();
    assert_eq!(state.arrays, original);
    assert_eq!(
        state.tables.regions().values().map(Vec::len).sum::<usize>(),
        26960
    );
}

#[test]
fn fixed_coordinate_tables_cover_every_original_rotation_and_keep_height() {
    let (mut execution, _) = fixed_construction(0);
    assert!(execution.tables().rotate_point([0, 0, 0], 1).is_err());
    assert_eq!(
        execution.tables().rotate_point([-128, 127, -4], 0).unwrap(),
        [-128, 127, -4]
    );
    execution.step().unwrap();
    for column in 0..64 {
        for row in 0..64 {
            let point = [column, row, -7];
            for orientation in 1..=3 {
                let expected = match orientation {
                    1 => [63 - row, column, -7],
                    2 => [63 - column, 63 - row, -7],
                    _ => [row, 63 - column, -7],
                };
                assert_eq!(
                    execution.tables().rotate_point(point, orientation).unwrap(),
                    expected
                );
            }
        }
    }
    for (point, orientation) in [([-1, 0, 0], 1), ([0, 64, 0], 2), ([0, 0, 0], 4)] {
        assert!(execution.tables().rotate_point(point, orientation).is_err());
    }
}

#[test]
fn fixed_wall_masks_and_one_based_directions_follow_actual_tables() {
    let (mut execution, _) = fixed_construction(0);
    for _ in 0..5 {
        execution.step().unwrap();
    }
    let regions = execution.tables().regions();
    let masks = &regions[&0x6f4670];
    let directions = &regions[&0x6f4620];
    let corners = &regions[&0x6f4a70];
    for orientation in 0..4 {
        for mask in 0..64 {
            let mut expected = 0_u32;
            for bit in 0..6 {
                if mask & (1 << bit) != 0 {
                    let rotated = if bit < 4 {
                        (bit + orientation) % 4
                    } else {
                        4 + (bit - 4 + orientation) % 2
                    };
                    expected |= 1 << rotated;
                }
            }
            let offset = (orientation * 64 + mask) * 4;
            assert_eq!(
                u32::from_le_bytes(masks[offset..offset + 4].try_into().unwrap()),
                expected
            );
        }
        assert_eq!(&directions[orientation * 20..orientation * 20 + 4], &[0; 4]);
        for direction in 1..=4 {
            let offset = (orientation * 5 + direction) * 4;
            assert_eq!(
                u32::from_le_bytes(directions[offset..offset + 4].try_into().unwrap()),
                ((direction + orientation - 1) % 4 + 1) as u32
            );
        }
    }
    for mask in 0..=255_u8 {
        let (junction, endpoint) = crate::lot_presenter::walls::grid::classification(mask);
        assert_eq!(corners[usize::from(mask)], u8::from(endpoint));
        assert_eq!(corners[256 + usize::from(mask)], u8::from(junction));
    }
}

#[test]
fn fixed_non_null_current_world_is_not_unconditional_notification_success() {
    let (mut execution, _) = fixed_construction(0x1000);
    execution.step().unwrap();
    execution.step().unwrap();
    let before = execution.clone();
    assert!(matches!(execution.step(), Err(Stop::Unavailable(_))));
    assert_eq!(execution, before);
    assert_eq!(execution.tables().regions().len(), 1);
    assert_eq!(execution.cut(), None);
    assert!(execution.into_state().is_err());
}

#[test]
fn fixed_restored_tables_phase_and_requested_size_fail_closed() {
    let (mut execution, _) = fixed_construction(0);
    for _ in 0..5 {
        execution.step().unwrap();
    }
    let encoded = serde_json::to_value(execution).unwrap();
    let mut variants = Vec::new();
    for (table, index) in [
        ("coordinates", 0),
        ("wall_corners", 1),
        ("wall_masks", 1),
        ("wall_directions", 1),
    ] {
        let mut changed = encoded.clone();
        changed["tables"][table][index] = serde_json::json!(255);
        variants.push(changed);
    }
    for (field, value) in [("requested", 63), ("current_world", 0x1000)] {
        let mut changed = encoded.clone();
        changed["context"][field] = serde_json::json!(value);
        variants.push(changed);
    }
    let mut changed = encoded.clone();
    changed["phase"] = serde_json::json!("Coordinates");
    variants.push(changed);
    let mut changed = encoded;
    changed["arrays"]["prefix"]["owner"]["data"][0x10] = serde_json::json!(63);
    variants.push(changed);
    for changed in variants {
        let mut restored: native_world::fixed::Construction =
            serde_json::from_value(changed).unwrap();
        assert!(restored.step().is_err());
        assert!(restored.into_state().is_err());
    }
}

fn lighting_construction(
    application_mode: u32,
    pixel_shift: u8,
) -> (native_world::lighting::Construction, Services) {
    let (mut fixed, host) = fixed_construction(0);
    for _ in 0..5 {
        fixed.step().unwrap();
    }
    let initial = native_world::lighting::Tables::new(
        (0..4320).map(|index| (index * 37 + 165) as u8).collect(),
        pixel_shift,
    )
    .unwrap();
    (
        native_world::lighting::Construction::new(
            fixed,
            initial,
            native_world::lighting::Context { application_mode },
        )
        .unwrap(),
        host,
    )
}

fn finish_lighting(execution: &mut native_world::lighting::Construction) {
    for index in 0..4 {
        assert_eq!(execution.step().unwrap().is_some(), index == 3);
        let restored = serde_json::from_slice(&serde_json::to_vec(execution).unwrap()).unwrap();
        assert_eq!(*execution, restored);
        *execution = restored;
    }
}

#[test]
fn lighting_preserves_genuine_graph_and_all_service_ownership_through_roundtrips() {
    let (mut execution, host) = lighting_construction(0, 3);
    let fixed = execution.fixed().clone();
    let events = host.events.clone();
    assert!(execution.clone().into_state().is_err());
    finish_lighting(&mut execution);
    assert_eq!(execution.fixed(), &fixed);
    assert_eq!(host.events, events);
    assert_eq!(execution.step().unwrap(), execution.cut());
    let state = execution.into_state().unwrap();
    state.validate().unwrap();
    assert_eq!(state.fixed, fixed);
}

#[test]
fn lighting_preserves_every_mode_zero_and_mode_one_unwritten_input_byte() {
    for mode in [0, 1, 3] {
        let (mut execution, _) = lighting_construction(mode, 3);
        let initial = execution.tables().bytes().to_vec();
        finish_lighting(&mut execution);
        let bytes = execution.tables().bytes();
        for row in 0..32 {
            let offset = 1024 + row * 64 + 32;
            assert_eq!(&bytes[offset..offset + 32], &initial[offset..offset + 32]);
        }
        if mode == 1 {
            assert_eq!(&bytes[4192..4256], &initial[4192..4256]);
        }
        assert_eq!(execution.tables().pixel_shift(), 3);
    }
}

#[test]
fn lighting_mode_two_overwrites_expanded_rows_and_the_actual_pixel_shift() {
    let (mut execution, _) = lighting_construction(2, 35);
    finish_lighting(&mut execution);
    assert_eq!(execution.tables().pixel_shift(), 2);
    let bytes = execution.tables().bytes();
    assert_eq!(bytes[1024 + 31 * 64 + 63], 62);
    assert_eq!(bytes[3072 + 31 * 32 + 31], 31);
    assert_eq!(bytes[31], 1);
    assert_eq!(
        u16::from_le_bytes(bytes[4128 + 63 * 2..4128 + 64 * 2].try_into().unwrap()),
        0x7e0
    );
}

#[test]
fn lighting_channel_packing_and_native_shift_mask_are_preserved() {
    let (mut ordinary, _) = lighting_construction(0, 3);
    let (mut wrapped, _) = lighting_construction(0, 35);
    finish_lighting(&mut ordinary);
    finish_lighting(&mut wrapped);
    assert_eq!(ordinary.tables().bytes(), wrapped.tables().bytes());
    assert_eq!(wrapped.tables().pixel_shift(), 35);
    for mode in [0, 1, 2] {
        let (mut execution, _) = lighting_construction(mode, 3);
        finish_lighting(&mut execution);
        let bytes = execution.tables().bytes();
        assert_eq!(bytes[4096 + 31], 31);
        assert_eq!(
            u16::from_le_bytes(bytes[4256 + 31 * 2..4256 + 32 * 2].try_into().unwrap()),
            if mode == 1 { 0x7c00 } else { 0xf800 }
        );
    }
}

#[test]
fn lighting_rejects_incomplete_input_and_corrupt_restored_written_or_untouched_state() {
    assert!(native_world::lighting::Tables::new(vec![0; 4319], 3).is_err());
    let (mut execution, _) = lighting_construction(0, 3);
    finish_lighting(&mut execution);
    let encoded = serde_json::to_value(execution).unwrap();
    let mut variants = Vec::new();
    for offset in [0, 1056, 3072, 4096, 4128, 4256] {
        let mut changed = encoded.clone();
        let byte = changed["tables"]["bytes"][offset].as_u64().unwrap();
        changed["tables"]["bytes"][offset] = serde_json::json!(byte ^ 255);
        variants.push(changed);
    }
    let mut changed = encoded.clone();
    changed["initial"]["bytes"][1056] = serde_json::json!(0);
    variants.push(changed);
    let mut changed = encoded.clone();
    changed["tables"]["pixel_shift"] = serde_json::json!(2);
    variants.push(changed);
    let mut changed = encoded.clone();
    changed["context"]["application_mode"] = serde_json::json!(2);
    variants.push(changed);
    let mut changed = encoded;
    changed["phase"] = serde_json::json!("PixelMode");
    variants.push(changed);
    for changed in variants {
        let mut restored: native_world::lighting::Construction =
            serde_json::from_value(changed).unwrap();
        assert!(restored.step().is_err());
        assert!(restored.into_state().is_err());
    }
}

fn resource_construction() -> (native_world::resources::Construction, Services) {
    let (mut lighting, host) = lighting_construction(2, 3);
    finish_lighting(&mut lighting);
    (
        native_world::resources::Construction::new(
            lighting,
            Owner {
                token: 0x6f01a0,
                data: vec![0; 100],
            },
            native_world::resources::Context {
                level_bits: 12.0_f32.to_bits(),
            },
        )
        .unwrap(),
        host,
    )
}

#[test]
fn resources_preserve_real_graph_reset_reserve_order_and_every_metadata_cut() {
    use native_world::resources::Cut;
    let (mut execution, mut host) = resource_construction();
    let lighting = execution.lighting().clone();
    let old_events = host.events.clone();
    for index in 0..42 {
        assert!(execution.clone().into_state().is_err());
        let cut = execution.step(&mut host).unwrap();
        assert_eq!(
            cut,
            match index {
                6 => Some(Cut::ResetReturn),
                41 => Some(Cut::VectorsReserved),
                _ => None,
            }
        );
        if index < 7 {
            assert_eq!(host.events, old_events);
            assert_eq!(execution.owner().data, [0; 100]);
        }
        assert_eq!(execution.lighting(), &lighting);
        let restored: (native_world::resources::Construction, Services) = serde_json::from_slice(
            &serde_json::to_vec(&(execution.clone(), host.clone())).unwrap(),
        )
        .unwrap();
        assert_eq!(restored, (execution.clone(), host.clone()));
        (execution, host) = restored;
    }
    assert_eq!(
        &host.events[old_events.len()..],
        [0, 1, 4, 2, 5, 3, 6].map(|index| Event::Resource(index, 16))
    );
    let events = host.events.clone();
    assert_eq!(
        execution.step(&mut host).unwrap(),
        Some(Cut::VectorsReserved)
    );
    assert_eq!(host.events, events);
    let state = execution.into_state().unwrap();
    state.validate().unwrap();
    for (index, backing) in state.backings.iter().enumerate() {
        let backing = backing.as_ref().unwrap();
        assert_eq!(backing.data, [0xa5; 16]);
        let beginning = backing.token as u32;
        assert_eq!(
            state.owner.data[index * 12..index * 12 + 12],
            [beginning, beginning, beginning + 16]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(&state.owner.data[84..], &[0; 16]);
}

#[test]
fn resources_require_explicit_complete_empty_static_ownership_and_reached_lighting() {
    let (mut lighting, _) = lighting_construction(0, 3);
    let owner = Owner {
        token: 0x6f01a0,
        data: vec![0; 100],
    };
    let context = native_world::resources::Context {
        level_bits: 12.0_f32.to_bits(),
    };
    assert!(
        native_world::resources::Construction::new(lighting.clone(), owner.clone(), context)
            .is_err()
    );
    finish_lighting(&mut lighting);
    for invalid in [
        Owner {
            token: 0x6f01a0,
            data: vec![0; 84],
        },
        Owner {
            token: 0x6f01a4,
            data: vec![0; 100],
        },
        Owner {
            token: 0x6f01a0,
            data: vec![0xa5; 100],
        },
    ] {
        assert!(
            native_world::resources::Construction::new(lighting.clone(), invalid, context).is_err()
        );
    }
    let mut changed = owner.clone();
    changed.data[96] = 1;
    assert!(
        native_world::resources::Construction::new(lighting.clone(), changed, context).is_err()
    );
    assert!(native_world::resources::Construction::new(lighting, owner, context).is_ok());
}

#[test]
fn resources_retry_only_unanswered_allocation_and_retain_rejected_pending_responses() {
    let (mut execution, mut host) = resource_construction();
    for _ in 0..7 {
        execution.step(&mut host).unwrap();
    }
    host.deny_resources = true;
    let before = execution.clone();
    assert!(execution.step(&mut host).is_err());
    assert_eq!(execution, before);
    host.deny_resources = false;
    for response in [
        Allocated::Null,
        Allocated::Bytes {
            token: 0,
            data: vec![0; 16],
        },
        Allocated::Bytes {
            token: 0x900000,
            data: vec![0; 15],
        },
        Allocated::Bytes {
            token: 0xfffffff0,
            data: vec![0; 16],
        },
        Allocated::Bytes {
            token: 0x100000000,
            data: vec![0; 16],
        },
    ] {
        let mut branch = execution.clone();
        let mut services = host.clone();
        services.resource_reply = Some(response);
        branch.step(&mut services).unwrap();
        let pending = branch.clone();
        let events = services.events.clone();
        for _ in 0..2 {
            assert!(branch.step(&mut services).is_err());
            assert_eq!(branch, pending);
            assert_eq!(services.events, events);
        }
    }
    host.resource_reply = Some(Allocated::Bytes {
        token: 0x900000,
        data: (0..16).map(|index| index * 13).collect(),
    });
    while execution.cut() != Some(native_world::resources::Cut::VectorsReserved) {
        execution.step(&mut host).unwrap();
    }
    assert_eq!(
        execution.backings()[0].as_ref().unwrap().data,
        (0..16).map(|index| index * 13).collect::<Vec<_>>()
    );
}

#[test]
fn resources_reject_aliases_with_real_world_tables_owner_context_and_each_other() {
    let (mut execution, mut host) = resource_construction();
    for _ in 0..7 {
        execution.step(&mut host).unwrap();
    }
    for token in [
        execution.lighting().fixed.arrays.prefix.owner.token,
        0x6de86c,
        0x6edf48,
        0x6c66fc,
        0x6f016c,
        0x6f01a0,
    ] {
        let mut branch = execution.clone();
        let mut services = host.clone();
        services.resource_reply = Some(Allocated::Bytes {
            token,
            data: vec![0; 16],
        });
        branch.step(&mut services).unwrap();
        assert!(branch.step(&mut services).is_err());
        assert!(branch.backings().iter().all(Option::is_none));
    }
    for _ in 0..5 {
        execution.step(&mut host).unwrap();
    }
    let token = execution.backings()[0].as_ref().unwrap().token;
    host.resource_reply = Some(Allocated::Bytes {
        token,
        data: vec![0; 16],
    });
    execution.step(&mut host).unwrap();
    assert!(execution.step(&mut host).is_err());
    assert!(execution.backings()[1].is_none());
}

#[test]
fn resources_fail_closed_on_restored_phase_headers_backings_context_or_tail_corruption() {
    let (mut execution, mut host) = resource_construction();
    while execution.cut() != Some(native_world::resources::Cut::VectorsReserved) {
        execution.step(&mut host).unwrap();
    }
    let encoded = serde_json::to_value(execution).unwrap();
    let mut variants = Vec::new();
    for offset in [0, 4, 8, 84, 88, 92, 96] {
        let mut changed = encoded.clone();
        let byte = changed["state"]["owner"]["data"][offset].as_u64().unwrap();
        changed["state"]["owner"]["data"][offset] = serde_json::json!(byte ^ 255);
        variants.push(changed);
    }
    for phase in [
        serde_json::json!({"Reset": 0}),
        serde_json::json!({"Allocate": 7}),
        serde_json::json!({"Publish": {"ordinal": 0, "words": 3}}),
    ] {
        let mut changed = encoded.clone();
        changed["phase"] = phase;
        variants.push(changed);
    }
    let mut changed = encoded.clone();
    changed["state"]["backings"][0] = serde_json::Value::Null;
    variants.push(changed);
    let mut changed = encoded.clone();
    changed["state"]["context"]["level_bits"] = serde_json::json!(24.0_f32.to_bits());
    variants.push(changed);
    let mut changed = encoded;
    changed["pending"] = serde_json::json!("Null");
    variants.push(changed);
    for changed in variants {
        let mut restored: native_world::resources::Construction =
            serde_json::from_value(changed).unwrap();
        let events = host.events.clone();
        assert!(restored.step(&mut host).is_err());
        assert!(restored.into_state().is_err());
        assert_eq!(host.events, events);
    }
}
