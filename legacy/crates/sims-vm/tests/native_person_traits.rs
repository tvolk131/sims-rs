use serde::Deserialize;
use sims_vm::{person_factory, rng};
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Fixture {
    streams: Vec<Stream>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Stream {
    seed: u32,
    original: Vec<[u32; 2]>,
}
#[derive(Deserialize)]
struct Case {
    seed: u32,
    before: Vec<i16>,
    original: Expected,
}
#[derive(Deserialize)]
struct Expected {
    person: Vec<i16>,
    state: u32,
    draws: usize,
}

#[cfg(feature = "private-fixtures")]
#[test]
fn crt_stream_and_default_traits_match_original_including_retries() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/person-traits.json")).unwrap();
    assert_eq!(fixture.streams.len(), 16);
    assert_eq!(fixture.cases.len(), 48);
    for stream in fixture.streams {
        let mut state = stream.seed;
        for expected in stream.original {
            let value = rng::next_crt(&mut state);
            assert_eq!([value, state], expected);
        }
    }
    for case in fixture.cases {
        let mut person: BTreeMap<_, _> = case
            .before
            .iter()
            .enumerate()
            .map(|(i, &v)| (i as i16, v))
            .collect();
        let mut state = case.seed;
        person_factory::default_traits(&mut person, &mut state).unwrap();
        assert_eq!(
            person.values().copied().collect::<Vec<_>>(),
            case.original.person,
            "seed {}",
            case.seed
        );
        assert_eq!(state, case.original.state);
        let mut expected_state = case.seed;
        for _ in 0..case.original.draws {
            rng::next_crt(&mut expected_state);
        }
        assert_eq!(state, expected_state);
        assert!(case.original.draws >= 30);
    }
}

#[test]
fn incomplete_storage_is_rejected_before_fields_or_random_state_change() {
    let mut person: BTreeMap<_, _> = (0..255).map(|i| (i, -7)).collect();
    let before = person.clone();
    let mut state = 0xabcdef01;
    assert!(person_factory::default_traits(&mut person, &mut state).is_err());
    assert_eq!(person, before);
    assert_eq!(state, 0xabcdef01);
}
