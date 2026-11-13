use super::*;
use crate::native_world::resources::population;

mod returned;

struct PopulationHost {
    next: u64,
    calls: Vec<Option<u32>>,
    reply: Option<Allocated>,
    deny_allocate: bool,
    deny_free: bool,
}

impl population::Host for PopulationHost {
    fn allocate_population(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        if self.deny_allocate {
            return Err(Stop::Unavailable("population allocator pending".into()));
        }
        self.calls.push(Some(bytes));
        let token = self.next;
        self.next += 32;
        Ok(self.reply.take().unwrap_or_else(|| Allocated::Bytes {
            token,
            data: (0..bytes).map(|index| (index + 53) as u8).collect(),
        }))
    }

    fn free_null(&mut self) -> Result<(), Stop> {
        if self.deny_free {
            return Err(Stop::Unavailable("population free pending".into()));
        }
        self.calls.push(None);
        Ok(())
    }
}

fn construction() -> (population::Construction, PopulationHost) {
    let (mut execution, mut host) = handle_construction();
    reach_handle(&mut execution, &mut host, handle::Cut::Published);
    let handle = execution.into_state().unwrap();
    let execution = population::Construction::new(handle).unwrap();
    let host = PopulationHost {
        next: 0x76000000,
        calls: vec![],
        reply: None,
        deny_allocate: false,
        deny_free: false,
    };
    (execution, host)
}

fn reach(execution: &mut population::Construction, host: &mut PopulationHost, ordinal: u8) {
    for _ in 0..160 {
        if execution.cut().is_some_and(|cut| cut.ordinal == ordinal) {
            return;
        }
        execution.step(host).unwrap();
    }
    panic!("population cut was not reached");
}

fn word(owner: &Owner, offset: usize) -> u32 {
    u32::from_le_bytes(owner.data[offset..offset + 4].try_into().unwrap())
}

#[test]
fn complete_population_keeps_all_owners_and_matches_native_layout_size() {
    let (mut execution, mut host) = construction();
    let before = execution.handle().clone();
    let mut cuts = vec![];
    let mut roundtrips = 0;
    for _ in 0..160 {
        if let Some(cut) = execution.step(&mut host).unwrap() {
            cuts.push(cut);
        }
        let restored: population::Construction =
            serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
        assert_eq!(execution, restored);
        restored.validate().unwrap();
        execution = restored;
        roundtrips += 1;
        if execution
            .cut()
            .is_some_and(|cut| cut.kind == population::Kind::LoadReturned)
        {
            break;
        }
    }
    assert_eq!(cuts.len(), 100);
    assert_eq!(roundtrips, 131);
    assert!(
        cuts.iter()
            .enumerate()
            .all(|(index, cut)| usize::from(cut.ordinal) == index)
    );
    assert_eq!(host.calls.iter().filter(|call| call.is_some()).count(), 31);
    assert_eq!(host.calls.iter().filter(|call| call.is_none()).count(), 3);
    assert_eq!(
        execution
            .owners()
            .iter()
            .map(|owner| owner.data.len())
            .sum::<usize>(),
        912
    );
    assert_eq!(execution.handle(), &before);
    let state = execution.into_state().unwrap();
    state.validate().unwrap();
    for (level, dimension) in [5, 5, 4, 3].into_iter().enumerate() {
        let parent = &state.owners()[level * 7];
        assert_eq!(word(parent, 0), 0);
        assert_eq!(word(parent, 4), level as u32);
        assert_eq!(word(parent, 8), dimension);
        assert_eq!(word(parent, 12), dimension * 64);
        assert_eq!(word(parent, 16), 232);
        assert_eq!(word(&state.backings()[0], level * 4), parent.token as u32);
        for channel in 0..3 {
            for second in 0..2 {
                let child = &state.owners()[level * 7 + 1 + channel * 2 + second];
                assert_eq!(word(child, 4), dimension - 1);
                assert_eq!(word(child, 8), level as u32);
                assert_eq!(word(child, 12), channel as u32 + 1);
                assert_eq!(word(child, 16), 1 << (channel + 1));
                assert_eq!(word(child, 20), second as u32);
                assert_eq!(word(child, 28), parent.token as u32);
                assert_eq!(
                    word(&state.backings()[1 + channel + second * 3], level * 4),
                    child.token as u32
                );
            }
        }
    }
    for (index, width) in [224, 448, 896].into_iter().enumerate() {
        let tail = &state.owners()[28 + index];
        assert_eq!(word(tail, 4), width);
        assert_eq!(word(tail, 8), 32 << index);
        assert_eq!(word(state.resources(), 84 + index * 4), tail.token as u32);
    }
    let last = state.owners().last().unwrap();
    assert_eq!(word(last, 12) + word(last, 4).pow(2) / 2, 2271104);
}

#[test]
fn constructors_preserve_unwritten_allocator_bytes_until_offset_assignment() {
    let (mut execution, mut host) = construction();
    reach(&mut execution, &mut host, 1);
    assert_eq!(&execution.owners()[0].data[20..24], &[73, 74, 75, 76]);
    reach(&mut execution, &mut host, 5);
    assert_eq!(&execution.owners()[1].data[24..28], &[77, 78, 79, 80]);
    reach(&mut execution, &mut host, 93);
    assert_eq!(&execution.owners()[28].data[12..16], &[65, 66, 67, 68]);
}

#[test]
fn unavailable_allocation_keeps_the_actual_reached_graph() {
    let (mut execution, mut host) = construction();
    let before = execution.clone();
    host.deny_allocate = true;
    assert!(matches!(
        execution.step(&mut host),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!(execution, before);
    assert!(host.calls.is_empty());
    host.deny_allocate = false;
    reach(&mut execution, &mut host, 0);
    assert_eq!(host.calls, [Some(24)]);
}

#[test]
fn null_allocation_is_latched_without_reissuing_the_host_call() {
    let (mut execution, mut host) = construction();
    host.reply = Some(Allocated::Null);
    assert_eq!(execution.step(&mut host).unwrap(), None);
    for _ in 0..3 {
        assert!(matches!(
            execution.step(&mut host),
            Err(Stop::Unavailable(_))
        ));
    }
    execution.validate().unwrap();
    assert_eq!(host.calls, [Some(24)]);
    assert!(execution.owners().is_empty());
}

#[test]
fn malformed_or_aliasing_allocation_stays_pending_without_reallocation() {
    for (token, bytes) in [(0x76000000, 23), (0x6f01a0, 24), (0x75000000, 24), (0, 24)] {
        let (mut execution, mut host) = construction();
        host.reply = Some(Allocated::Bytes {
            token,
            data: vec![0x47; bytes],
        });
        assert_eq!(execution.step(&mut host).unwrap(), None);
        assert!(execution.step(&mut host).is_err());
        assert!(execution.step(&mut host).is_err());
        assert_eq!(host.calls, [Some(24)]);
        assert!(execution.owners().is_empty());
    }
}

#[test]
fn actual_reservation_backing_cannot_be_reused_as_a_population_owner() {
    let (mut execution, mut host) = construction();
    host.reply = Some(Allocated::Bytes {
        token: execution.backings()[0].token,
        data: vec![0x47; 24],
    });
    execution.step(&mut host).unwrap();
    assert!(execution.step(&mut host).is_err());
    assert_eq!(host.calls, [Some(24)]);
}

#[test]
fn tail_free_unavailability_retains_early_publication_and_retries_only_free() {
    let (mut execution, mut host) = construction();
    reach(&mut execution, &mut host, 92);
    host.deny_free = true;
    assert!(execution.step(&mut host).is_err());
    let retained = execution.clone();
    retained.validate().unwrap();
    assert_eq!(
        word(execution.resources(), 84),
        execution.owners()[28].token as u32
    );
    assert_eq!(&execution.owners()[28].data[..12], &[0; 12]);
    assert!(execution.step(&mut host).is_err());
    assert_eq!(execution, retained);
    host.deny_free = false;
    assert_eq!(
        execution.step(&mut host).unwrap().unwrap().kind,
        population::Kind::TailConstructed
    );
    assert_eq!(host.calls.iter().filter(|call| call.is_none()).count(), 1);
    assert_eq!(host.calls.iter().filter(|call| call.is_some()).count(), 29);
}

#[test]
fn restored_population_rejects_changed_owners_capacities_and_resource_publication() {
    let (mut execution, mut host) = construction();
    reach(&mut execution, &mut host, 21);
    let value = serde_json::to_value(&execution).unwrap();
    for (field, index, offset) in [
        ("owners", 1, 24),
        ("backings", 0, 12),
        ("initial_owners", 0, 20),
    ] {
        let mut broken = value.clone();
        broken[field][index]["data"][offset] = 1.into();
        let broken: population::Construction = serde_json::from_value(broken).unwrap();
        assert!(broken.validate().is_err());
    }
    let mut broken = value.clone();
    broken["resources"]["data"][84] = 1.into();
    let broken: population::Construction = serde_json::from_value(broken).unwrap();
    assert!(broken.validate().is_err());
    let mut broken = value;
    broken["cursor"] = 255.into();
    let broken: population::Construction = serde_json::from_value(broken).unwrap();
    assert!(broken.validate().is_err());
}

#[test]
fn incomplete_population_cannot_be_reported_as_returned() {
    let (execution, _) = construction();
    assert!(execution.into_state().is_err());
}

#[test]
fn unqualified_level_profile_stops_instead_of_guessing_dimensions() {
    let (execution, _) = construction();
    let mut value = serde_json::to_value(execution).unwrap();
    value["handle"]["execution"]["opened"]["execution"]["closed"]["context"]["resources"]["resources"]
        ["context"]["level_bits"] = serde_json::json!(13_f32.to_bits());
    let broken: population::Construction = serde_json::from_value(value).unwrap();
    assert!(broken.validate().is_err());
}
