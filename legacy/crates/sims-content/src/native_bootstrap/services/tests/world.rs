//! A complete public source-component journey. Every application/static/context
//! input here is explicitly synthetic test data, never a production supplier.
use super::*;
use crate::{house_startup::saved_arrays, object_resources::MountedStream};
use sims_sim::{
    native_world::{arrays, fixed, lighting, resources, returned, startup},
    object_catalog::iff::Iff,
    saved_matrix::{Allocation, Scratch},
};
use sims_vm::commander::Registry;

macro_rules! reach {
    ($step:expr, $target:expr) => {{
        let mut reached = false;
        for _ in 0..20_000 {
            if $step.unwrap() == Some($target) {
                reached = true;
                break;
            }
        }
        assert!(reached, "constructor did not reach {}", stringify!($target));
    }};
}

fn view(frame: &Owner, start: usize, bytes: usize) -> Owner {
    Owner {
        token: frame.token + start as u64,
        data: frame.data[start..start + bytes].to_vec(),
    }
}

fn iff_bytes() -> Vec<u8> {
    let mut bytes = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1".to_vec();
    bytes.resize(64, 0);
    // Real IFF absence returns -98. Supply every requested resource instead of
    // manufacturing a zero-status missing reply. These are explicit synthetic
    // parsed-array payloads, including the supported two-byte head format.
    for (id, stride) in [
        (0_u16, 4_i16),
        (4, 1),
        (11, 2),
        (12, 14),
        (3, 2),
        (8, 1),
        (111, 2),
        (112, 14),
        (103, 2),
        (108, 1),
        (6, 1),
        (7, 1),
        (9, 1),
        (10, 1),
    ] {
        let mut payload: Vec<_> = [stride, 64, 64, 0]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect();
        payload.resize(8 + 64 * 4, 0xcc);
        payload.resize(
            payload.len() + 64 * 64 * stride as usize,
            if id == 0 { 0x52 } else { 0x31 },
        );
        let mut chunk = b"Arry".to_vec();
        chunk.extend(((76 + payload.len()) as u32).to_be_bytes());
        chunk.extend(id.to_be_bytes());
        chunk.extend(0_u16.to_be_bytes());
        chunk.resize(76, 0);
        chunk.extend(payload);
        bytes.extend(chunk);
    }
    bytes
}

fn to_handle(host: &mut BootstrapServices<'_>) -> (handle::State, Vec<Owner>, u64) {
    let mut app = host.memory.allocate_owner(324).unwrap();
    let mut path_string = strings::Construction::new(view(&app, 180, 12), None).unwrap();
    for _ in 0..32 {
        if path_string
            .step(&mut host.scoped_strings(path::Scope::Path))
            .unwrap()
        {
            break;
        }
    }
    let path_string = path_string.into_state().unwrap();
    app.data[180..192].copy_from_slice(&path_string.owner.data);
    let mut registry = Registry::<u32>::default();
    let commander = (app.token + 0x68) as u32;
    registry.construct(commander).unwrap();
    registry.bind(commander, 0x667ec0).unwrap();
    for (i, word) in [0x667ec0_u32, 0, 0, 0].into_iter().enumerate() {
        app.data[104 + i * 4..108 + i * 4].copy_from_slice(&word.to_le_bytes());
    }
    host.memory.synchronize_owner(&app).unwrap();
    let application = path::Application::new(app, path_string.backing, vec![]).unwrap();
    let mut prefix = native_world::Prefix::new(registry).unwrap();
    reach!(prefix.step(host), native_world::Cut::ArrayConstructionEntry);
    let mut arrays = arrays::Construction::new(
        prefix,
        arrays::Context {
            roof_owners: 0,
            random: 0x12345678,
        },
    )
    .unwrap();
    reach!(arrays.step(host), arrays::Cut::CoordinateTablesEntry);
    let mut fixed = fixed::Construction::new(
        arrays,
        fixed::Context {
            requested: 64,
            current_world: 0,
        },
    )
    .unwrap();
    reach!(fixed.step(), fixed::Cut::LightingEntry);
    let mut light = lighting::Construction::new(
        fixed,
        lighting::Tables::new(vec![0x53; 4320], 3).unwrap(),
        lighting::Context {
            application_mode: 2,
        },
    )
    .unwrap();
    reach!(light.step(), lighting::Cut::ResourceEntry);
    // Source-required fixed logical static regions. They are complete owned test
    // inputs, never retokenized ordinary heap allocations or captured statics.
    let resource_static = Owner {
        token: 0x6f01a0,
        data: vec![0; 100],
    };
    let manager_static = Owner {
        token: 0x6fe548,
        data: vec![0x61; 72],
    };
    let mut resources = resources::Construction::new(
        light,
        resource_static.clone(),
        resources::Context {
            level_bits: 12_f32.to_bits(),
        },
    )
    .unwrap();
    reach!(resources.step(host), resources::Cut::VectorsReserved);
    let path_buffer = host.memory.allocate_owner(272).unwrap();
    let output = host.memory.allocate_owner(12).unwrap();
    let scenario = host.memory.allocate_owner(12).unwrap();
    let mut path = path::Construction::new(
        resources,
        path::Incoming {
            application,
            buffer: path_buffer,
            output_descriptor: output,
            scenario_descriptor: scenario,
            retained: vec![],
        },
        path::Inputs::lightmap(),
    )
    .unwrap();
    reach!(path.step(host), path::Cut::ComposedPath);
    let mut path = path.into_state().unwrap();
    assert_eq!(
        host.live_capabilities(),
        0,
        "actual Scenario probe must have closed"
    );
    let mut manager = manager::Construction::new(manager_static.clone(), [0, 0], vec![]).unwrap();
    let mut done = false;
    for _ in 0..64 {
        if manager.step(host).unwrap() {
            done = true;
            break;
        }
    }
    assert!(done);
    let manager = manager.into_state().unwrap();
    let synchronization = manager.synchronization;
    host.section(synchronization, 0x6fe578, 24).unwrap();
    assert!(manager.retained.is_empty());
    path.retained
        .extend(std::iter::once(manager.owner).chain(manager.buckets));
    let name_frame = host.memory.allocate_owner(24).unwrap();
    let mut names = names::Construction::new(
        path,
        names::Incoming {
            basename_descriptor: view(&name_frame, 0, 12),
            directory_descriptor: view(&name_frame, 12, 12),
            synchronization,
            manager_retained: vec![],
        },
    )
    .unwrap();
    reach!(names.step(host), names::Cut::FileAllocationEntry);
    let mut file = file::Construction::new(names.into_state().unwrap()).unwrap();
    reach!(file.step(host), file::Cut::Returned);
    let file = file.into_state().unwrap();
    assert_eq!(file.context.retired_prefix.token, name_frame.token);
    let opening_frame = host.memory.allocate_owner(0x56c).unwrap();
    let encoding = Owner {
        token: 0x6d28a8,
        data: 0xfffffffd_u32.to_le_bytes().to_vec(),
    };
    let dispatch = Owner {
        token: 0x6fe5d8,
        data: 0x7b60ae9c_u32.to_le_bytes().to_vec(),
    };
    let incoming = opening::Incoming {
        output: view(&opening_frame, 0x560, 12),
        local: view(&opening_frame, 0x410, 12),
        replacement: view(&name_frame, 0, 12),
        buffer: view(&opening_frame, 0, 1024),
        buffer_units: 512,
        encoding: encoding.clone(),
        wide_dispatch: dispatch.clone(),
        arguments: [1, 2, 1],
    };
    let mut opening = opening::Execution::new(file, incoming).unwrap();
    reach!(opening.step(host), opening::Cut::Returned);
    let opening = opening.into_state().unwrap();
    assert_eq!(
        host.file(opening.capability()).unwrap().role,
        FileRole::ResourceReadonly
    );
    let mut handle = handle::Construction::new(opening).unwrap();
    reach!(handle.step(host), handle::Cut::Published);
    (
        handle.into_state().unwrap(),
        vec![
            resource_static,
            manager_static,
            name_frame,
            opening_frame,
            encoding,
            dispatch,
        ],
        synchronization,
    )
}

#[test]
fn public_world_and_real_iff_reader_retain_one_service_domain() {
    let iff = iff_bytes();
    let lightmap = vec![0x6b; 2_271_104];
    let (index, dirs) = mounted(&[
        ("./GameData/Scenario.tds", b"actual probe bytes"),
        ("./GameData/lightmap.dat", &lightmap),
        ("./Fixture/House.iff", &iff),
    ]);
    let mut host = services(&index, &dirs);
    let (handle, mut frames, synchronization) = to_handle(&mut host);
    let cap = handle.opened().capability();
    assert_eq!(host.file(cap).unwrap().bytes, lightmap);
    let mut population = population::Construction::new(handle).unwrap();
    reach!(
        population
            .step(&mut host)
            .map(|cut| cut.map(|cut| cut.kind)),
        population::Kind::LoadReturned
    );
    let corners = host.memory.allocate_owner(24).unwrap();
    let mut returned = returned::Construction::new(
        population.into_state().unwrap(),
        returned::Incoming {
            corners: [
                view(&corners, 4, 4),
                view(&corners, 8, 4),
                view(&corners, 0, 4),
                view(&corners, 20, 4),
            ],
        },
    )
    .unwrap();
    reach!(returned.step(), returned::Cut::Returned);
    frames.push(corners);
    let world = startup::World::from_returned(returned.into_state().unwrap()).unwrap();
    world.validate().unwrap();
    let identity = world.owner().token;
    assert_eq!(world.active_size(), 64);
    let mut stream = MountedStream::new(&index, &dirs);
    let mut iff = Iff::default();
    assert_eq!(iff.open(b"./Fixture/House.iff", 0, &mut stream).unwrap(), 0);
    let file = saved_arrays::File::from_returned(iff, stream);
    // Unsupported profile returns the complete inputs, preserving all actual
    // file/heap/lock storage. Retrying uses that same returned owner set.
    let inputs = saved_arrays::Inputs {
        world,
        file,
        scratch: Scratch {
            requested_converter: 0,
        },
        services: host,
        version: 52,
    };
    let mut inputs = match saved_arrays::ReadOwner::begin(inputs) {
        Err(error) => {
            assert!(matches!(error.stop, Stop::Unsupported(_)));
            error.inputs
        }
        Ok(_) => panic!("unsupported profile accepted"),
    };
    assert_eq!(inputs.world.owner().token, identity);
    assert_eq!(inputs.services.file(cap).unwrap().bytes.len(), 2_271_104);
    inputs.version = 63;
    let mut read = match saved_arrays::ReadOwner::begin(inputs) {
        Ok(read) => read,
        Err(error) => panic!("{:?}", error.stop),
    };
    read = match read.into_returned() {
        Err(read) => read,
        Ok(_) => panic!("incomplete read extracted"),
    };
    let mut completed = false;
    for _ in 0..4096 {
        if let Some(status) = read.step().unwrap() {
            assert_eq!(status, 0, "the first terminal status must be successful");
            completed = true;
            break;
        }
    }
    assert!(completed, "saved-array reader did not return");
    assert_eq!(read.step().unwrap(), Some(0));
    assert_eq!(read.services().live_capabilities(), 2);
    let returned = match read.into_returned() {
        Ok(returned) => returned,
        Err(_) => panic!("complete read retained"),
    };
    let (world, execution, scratch, mut file, mut host) = returned.into_parts();
    world.validate().unwrap();
    assert_eq!(world.owner().token, identity);
    assert_eq!(world.namings().len(), 1);
    assert_eq!(execution.status(), Some(0));
    assert!(execution.abandoned_reads.is_empty());
    assert_eq!(scratch.requested_converter, 0);
    let root = &world.arrays().roots[&0x14];
    let Allocation::Known(backing) = &world.arrays().matrices.nodes[&root.token].matrix.allocation
    else {
        panic!()
    };
    assert_eq!(backing.data, vec![0x52; 64 * 64 * 4]);
    host.memory.synchronize_matrix(backing).unwrap();
    assert_eq!(host.file(cap).unwrap().bytes, lightmap);
    assert!(
        host.section(synchronization, 0x6fe578, 24)
            .unwrap()
            .try_lock()
            .is_ok()
    );
    assert_eq!(host.live_capabilities(), 2);
    for frame in &frames {
        if frame.token >= 0x1000_0000 {
            host.memory
                .require_owner_view(frame.token, frame.data.len() as u32)
                .unwrap();
        }
    }
    // This is still the actual retained IFF and its ordinary lease/release path.
    let Lookup::Found { lease } = crate::native_bootstrap::ResourceSource::lookup(
        &mut file,
        u32::from_be_bytes(*b"Arry"),
        0,
        saved_matrix::NOOP_RESOURCE_CALLBACK,
    )
    .unwrap() else {
        panic!()
    };
    crate::native_bootstrap::ResourceSource::release(&mut file, lease).unwrap();
}

#[test]
fn actual_wrong_lightmap_lengths_stop_population_without_losing_open_capabilities() {
    for length in [2_271_103, 2_271_105] {
        let lightmap = vec![0x6b; length];
        let (index, dirs) = mounted(&[
            ("./GameData/Scenario.tds", b"probe"),
            ("./GameData/lightmap.dat", &lightmap),
        ]);
        let mut host = services(&index, &dirs);
        let (handle, _frames, synchronization) = to_handle(&mut host);
        let cap = handle.opened().capability();
        let mut population = population::Construction::new(handle).unwrap();
        let mut stopped = false;
        for _ in 0..512 {
            match population.step(&mut host) {
                Err(Stop::Unsupported(_)) | Err(Stop::Unavailable(_)) => {
                    stopped = true;
                    break;
                }
                Ok(_) => (),
            }
        }
        assert!(
            stopped,
            "actual wrong byte length must not qualify the resource profile"
        );
        assert_eq!(host.file(cap).unwrap().bytes.len(), length);
        host.section(synchronization, 0x6fe578, 24).unwrap();
        assert_eq!(host.live_capabilities(), 2);
        assert!(population.validate().is_err());
        let before = (host.memory.live_bytes(), host.memory.live_allocations());
        assert!(population.step(&mut host).is_err());
        assert_eq!(
            (host.memory.live_bytes(), host.memory.live_allocations()),
            before
        );
    }
}
