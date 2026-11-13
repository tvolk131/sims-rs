use super::*;
use crate::Resource;
use sha2::Digest;

mod application;
mod world;

fn mounted(entries: &[(&str, &[u8])]) -> (Index, Directories) {
    let root = std::env::temp_dir().join("bootstrap-services-owned-test");
    let mut directories = Directories::default();
    let resources = entries
        .iter()
        .enumerate()
        .map(|(source, (name, bytes))| {
            let split = name.rfind('/').unwrap() + 1;
            directories
                .insert_first(name[..split].as_bytes(), name[split..].as_bytes(), source)
                .unwrap();
            Resource {
                locator: name.to_string(),
                name: name[split..].to_string(),
                source: root.join(name),
                offset: 0,
                size: bytes.len() as u64,
                unpacked_size: bytes.len() as u64,
                archived: false,
                owned: Some(bytes.to_vec().into()),
            }
        })
        .collect();
    (
        Index {
            root,
            resources,
            archives: 0,
            diagnostics: vec![],
        },
        directories,
    )
}

fn services<'a>(index: &'a Index, dirs: &'a Directories) -> BootstrapServices<'a> {
    BootstrapServices::new(
        Memory::new(8 * 1024 * 1024).unwrap(),
        index,
        dirs,
        LocalePolicy::C,
    )
}

fn request(filename: &[u8]) -> opening::OpenRequest {
    opening::OpenRequest {
        filename: filename.iter().map(|b| u16::from(*b)).collect(),
        access: 0x80000000,
        sharing: 1,
        security: 0,
        disposition: 3,
        flags: 0,
        template: 0,
    }
}

fn owner(value: Allocated) -> Owner {
    let Allocated::Bytes { token, data } = value else {
        panic!("unexpected null allocation")
    };
    Owner { token, data }
}

#[test]
fn mounted_winner_controls_bytes_without_basename_or_path_fallback() {
    let (index, _) = mounted(&[
        ("./GameData/Scenario.tds", b"loser"),
        ("./Other/Scenario.tds", b"winner"),
    ]);
    let mut dirs = Directories::default();
    assert!(
        dirs.insert_first(b"./GameData/", b"Scenario.tds", 1)
            .unwrap()
    );
    assert!(
        !dirs
            .insert_first(b"./GameData/", b"Scenario.tds", 0)
            .unwrap()
    );
    let mut host = services(&index, &dirs);
    let path::Opened::File(cap) =
        path::Host::open_scenario(&mut host, b"./GAMEDATA\\SCENARIO.TDS", b"rb").unwrap()
    else {
        panic!()
    };
    let view = host.file(cap as u32).unwrap();
    assert_eq!(view.bytes, b"winner");
    assert_eq!(view.identity.resource, 1);
    assert_eq!(
        view.identity.bytes_sha256,
        format!("{:x}", sha2::Sha256::digest(b"winner"))
    );
    assert_eq!(
        path::Host::open_scenario(&mut host, b"./GameData/absent.tds", b"rb").unwrap(),
        path::Opened::Missing
    );
    for filename in [
        b"GameData/Scenario.tds".as_slice(),
        b"Scenario.tds",
        b"./Other/Scenario.tds",
    ] {
        assert!(matches!(
            path::Host::open_scenario(&mut host, filename, b"rb"),
            Err(Stop::Unavailable(_))
        ));
    }
    assert_eq!(host.live_capabilities(), 1);
    path::Host::close_scenario(&mut host, cap).unwrap();
    assert!(host.file(cap as u32).is_err());
}

#[test]
fn readonly_size_cursor_and_roles_come_from_the_retained_open() {
    let (index, dirs) = mounted(&[
        ("./GameData/Scenario.tds", b"probe"),
        ("./GameData/lightmap.dat", b"123456789"),
    ]);
    let mut host = services(&index, &dirs);
    let path::Opened::File(probe) =
        path::Host::open_scenario(&mut host, b"./GameData/Scenario.tds", b"rb").unwrap()
    else {
        panic!()
    };
    let opening::Opened::File(cap) =
        opening::Host::open_readonly(&mut host, &request(b"./GameData/lightmap.dat")).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        handle::Host::file_size(&mut host, cap).unwrap(),
        handle::FileSize::Bytes(9)
    );
    assert!(handle::Host::file_size(&mut host, probe as u32).is_err());
    assert!(path::Host::close_scenario(&mut host, u64::from(cap)).is_err());
    let mut out = [0; 5];
    assert_eq!(host.read(cap, &mut out).unwrap(), 5);
    assert_eq!(&out, b"12345");
    assert_eq!(
        opening::Host::seek_current(&mut host, cap).unwrap(),
        opening::Seeked::Position(5)
    );
    assert_eq!(host.read(cap, &mut out).unwrap(), 4);
    assert_eq!(&out[..4], b"6789");
    assert_eq!(host.read(cap, &mut out).unwrap(), 0);
    assert_eq!(host.file(cap).unwrap().position, 9);
    assert!(host.read(probe as u32, &mut out).is_err());
    path::Host::close_scenario(&mut host, probe).unwrap();
    assert!(path::Host::close_scenario(&mut host, probe).is_err());
    let path::Opened::File(next) =
        path::Host::open_scenario(&mut host, b"./GameData/Scenario.tds", b"rb").unwrap()
    else {
        panic!()
    };
    assert!(next > probe && next != u64::from(cap));
    assert_eq!(host.live_capabilities(), 2);
}

#[test]
fn unsupported_open_parameters_stop_before_publication() {
    let (index, dirs) = mounted(&[("./GameData/lightmap.dat", b"bytes")]);
    let mut host = services(&index, &dirs);
    let good = request(b"./GameData/lightmap.dat");
    for field in 0..10 {
        let mut bad = good.clone();
        match field {
            0 => bad.access ^= 1,
            1 => bad.sharing = 3,
            2 => bad.security = 1,
            3 => bad.disposition = 4,
            4 => bad.flags = 1,
            5 => bad.template = 1,
            6 => bad.filename.push(0),
            7 => bad.filename[0] = 128,
            8 => bad.filename.clear(),
            _ => bad.filename = vec![b'a' as u16; 257],
        }
        assert!(opening::Host::open_readonly(&mut host, &bad).is_err());
        assert_eq!(
            (
                host.live_capabilities(),
                host.memory.live_allocations(),
                host.next
            ),
            (0, 0, CAP_START)
        );
    }
    assert!(path::Host::open_scenario(&mut host, b"./GameData/lightmap.dat", b"r").is_err());
    assert_eq!(
        opening::Host::open_readonly(&mut host, &request(b"./GameData/absent.dat")).unwrap(),
        opening::Opened::Failed
    );
    assert_eq!(host.next, CAP_START);
}

#[test]
fn scoped_releases_require_original_role_base_and_complete_extent() {
    let (index, dirs) = mounted(&[]);
    let mut host = services(&index, &dirs);
    let old = owner(
        path::Host::allocate_string(&mut host, path::Scope::Path, strings::Request::Construct(8))
            .unwrap(),
    );
    let grown = owner(
        path::Host::allocate_string(&mut host, path::Scope::Path, strings::Request::Grow(24))
            .unwrap(),
    );
    assert!(names::Host::free_string(&mut host, &old).is_err());
    for bad in [
        Owner {
            token: old.token + 1,
            data: old.data.clone(),
        },
        Owner {
            token: old.token,
            data: vec![0; 7],
        },
    ] {
        assert!(path::Host::free_string(&mut host, path::Scope::Path, &bad).is_err());
    }
    let unscoped = owner(
        strings::Host::allocate_string(&mut host.memory, strings::Request::Construct(8)).unwrap(),
    );
    assert!(path::Host::free_string(&mut host, path::Scope::Path, &unscoped).is_err());
    assert_eq!(host.memory.live_bytes(), 40);
    let mut changed = old.clone();
    changed.data.fill(0x72); // A model's modified bytes are not stale ownership.
    path::Host::free_string(&mut host, path::Scope::Path, &changed).unwrap();
    assert!(path::Host::free_string(&mut host, path::Scope::Path, &changed).is_err());
    assert_eq!(host.memory.allocation_bytes(grown.token).unwrap().len(), 24);
    strings::Host::free_string(&mut host.scoped_strings(path::Scope::Path), grown.token).unwrap();
    assert_eq!(host.memory.live_bytes(), 8);
}

#[test]
fn wide_byte_extents_and_cross_role_frees_are_checked() {
    let (index, dirs) = mounted(&[]);
    let mut host = services(&index, &dirs);
    for bytes in [0, 1, 3, u32::MAX, u32::MAX - 1] {
        assert!(opening::Host::allocate_wide(&mut host, opening::Request::Grow(bytes)).is_err());
    }
    assert_eq!(host.memory.live_allocations(), 0);
    let wide =
        owner(opening::Host::allocate_wide(&mut host, opening::Request::Construct(16)).unwrap());
    let ordinary = host.memory.allocate_owner(16).unwrap();
    assert!(opening::Host::free_wide(&mut host, &ordinary).is_err());
    assert!(names::Host::free_string(&mut host, &wide).is_err());
    assert!(
        opening::Host::free_wide(
            &mut host,
            &Owner {
                token: wide.token,
                data: vec![0; 8]
            }
        )
        .is_err()
    );
    opening::Host::free_wide(&mut host, &wide).unwrap();
    assert!(opening::Host::free_wide(&mut host, &wide).is_err());
    assert_eq!(host.memory.live_bytes(), 16);
}

#[test]
fn manager_capability_resolves_one_real_lock_with_exact_identity() {
    let (index, dirs) = mounted(&[("./GameData/Scenario.tds", b"probe")]);
    let mut host = services(&index, &dirs);
    assert!(manager::Host::initialize_section(&mut host, 0x6fe578, 23).is_err());
    assert_eq!(host.live_capabilities(), 0);
    let cap = manager::Host::initialize_section(&mut host, 0x6fe578, 24).unwrap();
    let lock = Arc::clone(host.section(cap, 0x6fe578, 24).unwrap());
    let held = lock.lock().unwrap();
    assert!(host.section(cap, 0x6fe578, 24).unwrap().try_lock().is_err());
    drop(held);
    assert!(host.section(cap, 0x6fe578, 24).unwrap().try_lock().is_ok());
    assert!(host.section(cap, 0x6fe579, 24).is_err());
    assert!(host.section(cap + (1_u64 << 32), 0x6fe578, 24).is_err());
    assert!(manager::Host::initialize_section(&mut host, 0x6fe578, 24).is_err());
    assert!(path::Host::close_scenario(&mut host, cap).is_err());
    assert!(handle::Host::file_size(&mut host, cap as u32).is_err());
    assert_eq!(host.live_capabilities(), 1);
}

#[test]
fn capability_exhaustion_is_terminal_without_reuse_or_partial_publication() {
    let (index, dirs) = mounted(&[("./GameData/Scenario.tds", b"probe")]);
    let mut host = services(&index, &dirs);
    host.next = CAP_START + MAX_CAPABILITIES - 1;
    let path::Opened::File(last) =
        path::Host::open_scenario(&mut host, b"./GameData/Scenario.tds", b"rb").unwrap()
    else {
        panic!()
    };
    path::Host::close_scenario(&mut host, last).unwrap();
    assert!(path::Host::open_scenario(&mut host, b"./GameData/Scenario.tds", b"rb").is_err());
    assert!(manager::Host::initialize_section(&mut host, 0x6fe578, 24).is_err());
    assert_eq!(host.live_capabilities(), 0);
    host.next = u32::MAX;
    assert!(path::Host::open_scenario(&mut host, b"./GameData/Scenario.tds", b"rb").is_err());
    assert_eq!(host.next, u32::MAX);
}

#[test]
fn present_unbound_mount_nodes_stop_instead_of_fabricating_missing_files() {
    use sims_sim::pose::clothing::catalog_mount::{Manager, Metadata};
    let (index, _) = mounted(&[]);
    let mut manager = Manager::default();
    for name in [b"Scenario.tds".as_slice(), b"lightmap.dat"] {
        manager
            .directory_mut(b"./GameData/")
            .unwrap()
            .insert(name, Metadata::default())
            .unwrap();
    }
    let dirs = Directories::from_mounted(&manager, |_, _, _| Ok(None)).unwrap();
    let mut host = services(&index, &dirs);
    assert!(matches!(
        path::Host::open_scenario(&mut host, b"./GameData/Scenario.tds", b"rb"),
        Err(Stop::Unavailable(_))
    ));
    assert!(matches!(
        opening::Host::open_readonly(&mut host, &request(b"./GameData/lightmap.dat")),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!(
        path::Host::open_scenario(&mut host, b"./GameData/absent.tds", b"rb").unwrap(),
        path::Opened::Missing
    );
    assert_eq!(host.live_capabilities(), 0);
    assert_eq!(host.next, CAP_START);
}
