use super::*;

fn bytes(allocated: Allocated) -> (u64, Vec<u8>) {
    match allocated {
        Allocated::Bytes { token, data } => (token, data),
        Allocated::Null => panic!("the concrete Memory hook never manufactures NULL"),
    }
}

fn state(memory: &Memory) -> (u64, usize, usize, Vec<(u64, Kind, Vec<u8>)>) {
    (
        memory.next,
        memory.issued,
        memory.live_bytes,
        memory.regions.iter()
            .map(|(address, region)| (*address, region.kind, region.bytes.clone()))
            .collect(),
    )
}

// Deterministic private reservation fault, not a claim that the host actually
// ran out of memory. No allocation pressure, native pointer or public injector.
fn reserve_failure() -> std::collections::TryReserveError {
    Vec::<u8>::new().try_reserve_exact(usize::MAX).unwrap_err()
}

#[test]
fn program_global_real_bytes_are_zero_aligned_and_distinct_from_world_owners() {
    let mut memory = Memory::new(64).unwrap();
    let world = memory.allocate_owner(9).unwrap();
    let (global, mut image) = bytes(memory.allocate_program_global(17).unwrap());
    assert_ne!(global, world.token);
    assert_eq!(global % 16, 0);
    assert_ne!(global, 0);
    assert!(u32::try_from(global).is_ok());
    assert_eq!(image, [0; 17]);
    assert_eq!(memory.allocation_bytes(global).unwrap(), [0; 17]);
    assert_eq!(memory.allocation_kind(global).unwrap(), Kind::ProgramGlobal);
    image[0] = 0xff;
    assert_eq!(memory.allocation_bytes(global).unwrap(), [0; 17]);

    let before = state(&memory);
    assert!(memory.release_owner(global).is_err());
    assert!(memory.release(world.token, Kind::ProgramGlobal).is_err());
    assert!(memory.release(global + 1, Kind::ProgramGlobal).is_err());
    assert_eq!(state(&memory), before);
    memory.release(global, Kind::ProgramGlobal).unwrap();
    assert!(memory.allocation_bytes(global).is_err());
    assert!(memory.release(global, Kind::ProgramGlobal).is_err());
    assert_eq!(memory.allocation_bytes(world.token).unwrap(), world.data);
    let (later, _) = bytes(memory.allocate_program_global(17).unwrap());
    assert!(later > global);
    assert_eq!((memory.live_bytes(), memory.live_allocations()), (26, 2));
}

#[test]
fn program_global_profile_refusals_never_become_allocator_null() {
    let mut memory = Memory::new(16).unwrap();
    let mut world = memory.allocate_owner(8).unwrap();
    world.data[3] = 0xa5;
    memory.synchronize_owner(&world).unwrap();
    let identity = memory.identity();
    let before = state(&memory);

    for amount in [0, MAX_ALLOCATION_BYTES as u32 + 1, 9] {
        let mut reservations = 0;
        let result = memory.allocate_program_global_using(amount, |_, _| {
            reservations += 1;
            Err(reserve_failure())
        });
        match result {
            Err(ProgramAllocationError::Refused(Stop::Unsupported(_))) if amount != 9 => {}
            Err(ProgramAllocationError::Refused(Stop::Unavailable(_))) if amount == 9 => {}
            other => panic!("profile refusal was misclassified: {other:?}"),
        }
        assert_eq!(reservations, 0);
        assert_eq!(state(&memory), before);
        assert!(memory.has_identity(&identity));
    }
    assert!(matches!(memory.allocate_owner(9), Err(Stop::Unavailable(_))));
    assert_eq!(state(&memory), before);
}

#[test]
fn program_global_spent_issuance_budget_is_a_refusal_even_after_every_free() {
    let mut memory = Memory::new(1).unwrap();
    for _ in 0..MAX_REQUESTS {
        let (global, _) = bytes(memory.allocate_program_global(1).unwrap());
        memory.release(global, Kind::ProgramGlobal).unwrap();
    }
    assert_eq!((memory.live_bytes(), memory.live_allocations()), (0, 0));
    let before = state(&memory);
    assert!(matches!(
        memory.allocate_program_global(1),
        Err(ProgramAllocationError::Refused(Stop::Unavailable(_)))
    ));
    assert_eq!(state(&memory), before);
}

fn check_reservation_fault(fail_at: usize) {
    let mut memory = Memory::new(64).unwrap();
    let mut world = memory.allocate_owner(8).unwrap();
    world.data[4] = 0xc3;
    memory.synchronize_owner(&world).unwrap();
    let before = state(&memory);
    let identity = memory.identity();
    let expected_address = (memory.next + 15) & !15;
    let mut reservations = 0;
    assert_eq!(
        memory.allocate_program_global_using(16, |buffer, count| {
            reservations += 1;
            if reservations == fail_at {
                Err(reserve_failure())
            } else {
                buffer.try_reserve_exact(count)
            }
        }),
        Err(ProgramAllocationError::OutOfMemory)
    );
    assert_eq!(reservations, fail_at);
    assert_eq!(state(&memory), before);
    assert!(memory.has_identity(&identity));
    let (global, data) = bytes(memory.allocate_program_global(16).unwrap());
    assert_eq!(global, expected_address); // failure spent no address or issuance
    assert_eq!(data, [0; 16]);
    assert_eq!(memory.allocation_bytes(world.token).unwrap(), world.data);
}

#[test]
fn program_global_backing_reservation_failure_publishes_nothing_and_retries() {
    check_reservation_fault(1);
}

#[test]
fn program_global_return_image_reservation_failure_releases_unpublished_backing() {
    check_reservation_fault(2);
}

