use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Event {
    Allocate(u8, u32),
    Initialize(u64, u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Services {
    events: Vec<Event>,
    reply: Option<Allocated>,
    deny_allocation: bool,
    deny_lock: bool,
    lock: u64,
}

impl Default for Services {
    fn default() -> Self {
        Self {
            events: vec![],
            reply: None,
            deny_allocation: false,
            deny_lock: false,
            lock: 1,
        }
    }
}

impl Host for Services {
    fn allocate_buckets(&mut self, map: u8, bytes: u32) -> Result<Allocated, Stop> {
        if self.deny_allocation {
            return Err(Stop::Unavailable("allocator pending".into()));
        }
        self.events.push(Event::Allocate(map, bytes));
        Ok(self.reply.take().unwrap_or_else(|| Allocated::Bytes {
            token: 0x30000000 + u64::from(map) * 0x400,
            data: vec![0xa5; bytes as usize],
        }))
    }

    fn initialize_section(&mut self, address: u64, bytes: u32) -> Result<u64, Stop> {
        if self.deny_lock {
            return Err(Stop::Unavailable("platform pending".into()));
        }
        self.events.push(Event::Initialize(address, bytes));
        Ok(self.lock)
    }
}

fn incoming() -> Owner {
    Owner {
        token: MANAGER,
        data: vec![0xa5; 72],
    }
}

fn constructed() -> State {
    let mut construction = Construction::new(incoming(), [0, 0], vec![]).unwrap();
    let mut host = Services::default();
    while !construction.step(&mut host).unwrap() {}
    construction.into_state().unwrap()
}

#[test]
fn complete_manager_constructor_preserves_supplied_padding_and_opaque_lock_bytes() {
    let mut construction = Construction::new(incoming(), [0x7f, 0xd2], vec![]).unwrap();
    let mut host = Services::default();
    let mut steps = 0;
    loop {
        let complete = construction.step(&mut host).unwrap();
        let encoded = serde_json::to_vec(&(construction.clone(), host.clone())).unwrap();
        let restored: (Construction, Services) = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored, (construction.clone(), host.clone()));
        (construction, host) = restored;
        steps += 1;
        if complete {
            break;
        }
    }
    assert_eq!(steps, 13);
    assert_eq!(
        host.events,
        vec![
            Event::Allocate(0, 772),
            Event::Allocate(1, 772),
            Event::Initialize(MANAGER + 48, 24)
        ]
    );
    let state = construction.into_state().unwrap();
    assert_eq!(&state.owner.data[..3], &[0x7f; 3]);
    assert_eq!(&state.owner.data[20..23], &[0xd2; 3]);
    assert_eq!(state.owner.data[3], 0xa5);
    assert_eq!(state.owner.data[23], 0xa5);
    assert_eq!(&state.owner.data[40..44], &[0xa5; 4]);
    assert_eq!(state.owner.data[44], 0);
    assert_eq!(&state.owner.data[45..], &[0xa5; 27]);
    assert!(
        state
            .buckets
            .iter()
            .all(|backing| backing.data == vec![0; 772])
    );
    assert_eq!(
        state.lookup(b".\\gamedata\\", b"lightmap.dat").unwrap(),
        Lookup::Missing
    );
}

#[test]
fn manager_retries_unavailable_services_without_repeating_successful_calls() {
    let mut construction = Construction::new(incoming(), [0, 0], vec![]).unwrap();
    let mut host = Services {
        deny_allocation: true,
        ..Services::default()
    };
    construction.step(&mut host).unwrap();
    let before = construction.clone();
    assert!(matches!(
        construction.step(&mut host),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!(construction, before);
    assert!(host.events.is_empty());
    host.deny_allocation = false;
    host.deny_lock = true;
    while construction.phase != Phase::Synchronize {
        construction.step(&mut host).unwrap();
    }
    let before = construction.clone();
    assert!(matches!(
        construction.step(&mut host),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!(construction, before);
    host.deny_lock = false;
    while !construction.step(&mut host).unwrap() {}
    assert_eq!(host.events.len(), 3);
}

#[test]
fn manager_rejected_allocator_replies_stay_pending_without_reallocation() {
    for reply in [
        Allocated::Null,
        Allocated::Bytes {
            token: 0,
            data: vec![0; 772],
        },
        Allocated::Bytes {
            token: MANAGER,
            data: vec![0; 772],
        },
        Allocated::Bytes {
            token: 0x4000,
            data: vec![0; 771],
        },
        Allocated::Bytes {
            token: u64::from(u32::MAX) - 771,
            data: vec![0; 772],
        },
    ] {
        let mut construction = Construction::new(incoming(), [0, 0], vec![]).unwrap();
        let mut host = Services {
            reply: Some(reply),
            ..Services::default()
        };
        construction.step(&mut host).unwrap();
        construction.step(&mut host).unwrap();
        let before = construction.clone();
        assert!(construction.step(&mut host).is_err());
        assert!(construction.step(&mut host).is_err());
        assert_eq!(construction, before);
        assert_eq!(host.events.len(), 1);
    }
}

#[test]
fn manager_null_lock_remains_pending_and_never_reinitializes() {
    let mut construction = Construction::new(incoming(), [0, 0], vec![]).unwrap();
    let mut host = Services {
        lock: 0,
        ..Services::default()
    };
    while construction.phase != Phase::AcceptSynchronization {
        construction.step(&mut host).unwrap();
    }
    assert!(construction.step(&mut host).is_err());
    assert!(construction.step(&mut host).is_err());
    assert_eq!(host.events.len(), 3);
    assert!(construction.into_state().is_err());
}

#[test]
fn manager_constructor_rejects_corrupt_restored_stores_and_allocations() {
    let mut construction = Construction::new(incoming(), [0, 0], vec![]).unwrap();
    assert!(construction.clone().into_state().is_err());
    let mut host = Services::default();
    while construction.phase != Phase::Clear(0) {
        construction.step(&mut host).unwrap();
    }
    for offset in [0, 3, 4, 8, 12, 16, 48] {
        let mut corrupt = construction.clone();
        corrupt.owner.data[offset] ^= 1;
        assert!(corrupt.validate().is_err());
    }
    let mut corrupt = construction.clone();
    corrupt.backings[0].as_mut().unwrap().data[0] ^= 1;
    assert!(corrupt.validate().is_err());
    let mut corrupt = construction;
    corrupt.phase = Phase::Initialize(2);
    assert!(corrupt.validate().is_err());
    assert!(Construction::new(incoming(), [0, 0], vec![incoming()]).is_err());
}

#[test]
fn manager_uses_the_native_lower_bound_bucket_table_and_signed_cstring_hash() {
    assert_eq!(bucket_count(0), 53);
    assert_eq!(bucket_count(97), 97);
    assert_eq!(bucket_count(100), 193);
    assert_eq!(bucket_count(193), 193);
    assert_eq!(bucket_count(194), 389);
    assert_eq!(bucket_count(u32::MAX), 4294967291);
    for (input, expected) in [
        (b"".as_slice(), 0),
        (b".\\gamedata\\", 2204932287),
        (b"lightmap.dat", 2235268868),
        (b"abc", 3977453403),
        (b"\x80", 3273477671),
        (b"\xff", 2925896852),
        (b"a\0ignored", 3392050242),
    ] {
        assert_eq!(hash(input), expected);
    }
}

#[test]
fn manager_split_matches_ascii_slash_last_separator_and_cstring_bounds() {
    assert_eq!(
        split(b"./GameData/lightmap.dat").unwrap(),
        (b".\\gamedata\\".to_vec(), b"lightmap.dat".to_vec())
    );
    assert_eq!(
        split(b"LightMap.DAT\0ignored").unwrap(),
        (vec![], b"lightmap.dat".to_vec())
    );
    assert_eq!(split(b"A\\B/").unwrap(), (b"a\\b\\".to_vec(), vec![]));
    assert_eq!(split(&[0xc0, b'A']).unwrap(), (vec![], vec![0xc0, b'a']));
    assert!(split(&vec![b'A'; 259]).is_ok());
    assert!(split(&vec![b'A'; 260]).is_err());
    assert_eq!(hash(b"abc\0ignored"), hash(b"abc"));
    assert_ne!(hash(&[0xff]), hash(&[0x7f]));
    assert_eq!(hash(b""), 0);
}

fn populated() -> State {
    let mut state = constructed();
    let directory = b".\\gamedata\\";
    let bucket = (hash(directory) % BUCKETS as u32) as usize;
    write_word(&mut state.buckets[0], bucket * 4, 0x40000000);
    write_word(&mut state.owner, 16, 1);
    let mut node = Owner {
        token: 0x40000000,
        data: vec![0; 36],
    };
    for (offset, value) in [
        (4, 0x40000100),
        (8, 0x4000010b),
        (12, 0x4000010c),
        (20, 0x40000200),
        (24, 0x40000204),
        (28, 0x40000204),
        (32, 2),
    ] {
        write_word(&mut node, offset, value);
    }
    let mut directory_backing = directory.to_vec();
    directory_backing.push(0);
    state.retained.push(node);
    state.retained.push(Owner {
        token: 0x40000100,
        data: directory_backing,
    });
    state.retained.push(Owner {
        token: 0x40000200,
        data: 0x40000300_u32.to_le_bytes().to_vec(),
    });
    let mut first = Owner {
        token: 0x40000300,
        data: vec![0; 20],
    };
    for (offset, value) in [
        (0, 0x40000400),
        (4, 0x40000500),
        (8, 0x40000505),
        (12, 0x40000506),
    ] {
        write_word(&mut first, offset, value);
    }
    let mut second = Owner {
        token: 0x40000400,
        data: vec![0; 20],
    };
    for (offset, value) in [(4, 0x40000600), (8, 0x4000060c), (12, 0x4000060d)] {
        write_word(&mut second, offset, value);
    }
    state.retained.extend([
        first,
        second,
        Owner {
            token: 0x40000500,
            data: b"other\0".to_vec(),
        },
        Owner {
            token: 0x40000600,
            data: b"lightmap.dat\0".to_vec(),
        },
    ]);
    state
}

#[test]
fn manager_lookup_traverses_directory_and_basename_chains_without_mutation() {
    let state = populated();
    let before = state.clone();
    assert_eq!(
        state.lookup(b".\\gamedata\\", b"lightmap.dat").unwrap(),
        Lookup::Resource(0x40000410)
    );
    assert_eq!(
        state.lookup(b".\\gamedata\\", b"other").unwrap(),
        Lookup::Resource(0x40000310)
    );
    assert_eq!(
        state.lookup(b".\\gamedata\\", b"missing").unwrap(),
        Lookup::Missing
    );
    assert_eq!(
        state.lookup(b"missing", b"lightmap.dat").unwrap(),
        Lookup::Missing
    );
    assert_eq!(state, before);
}

#[test]
fn manager_lookup_rejects_unowned_chains_cycles_and_invalid_string_views() {
    let state = populated();
    for (address, offset, value) in [
        (0x40000300, 0, 0x40000300),
        (0x40000300, 0, 0x50000000),
        (0x40000000, 24, 0x40000200),
        (0x40000400, 8, 0x40000610),
    ] {
        let mut corrupt = state.clone();
        let owner = corrupt
            .retained
            .iter_mut()
            .find(|owner| owner.token == address)
            .unwrap();
        write_word(owner, offset, value);
        assert!(corrupt.lookup(b".\\gamedata\\", b"missing").is_err());
    }
    let mut corrupt = state;
    corrupt
        .retained
        .iter_mut()
        .find(|owner| owner.token == 0x40000600)
        .unwrap()
        .data[12] = 1;
    assert!(corrupt.lookup(b".\\gamedata\\", b"lightmap.dat").is_err());
}

#[test]
fn manager_restored_owner_alias_and_bucket_descriptor_corruption_fail_closed() {
    let state = constructed();
    for offset in [4, 8, 12, 24, 28, 32] {
        let mut corrupt = state.clone();
        corrupt.owner.data[offset] ^= 1;
        assert!(corrupt.validate().is_err());
    }
    let mut corrupt = state.clone();
    corrupt.retained.push(corrupt.buckets[0].clone());
    assert!(corrupt.validate().is_err());
    let mut corrupt = state;
    corrupt.synchronization = 0;
    assert!(corrupt.validate().is_err());
}
