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
    let mut fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/person-traits.json")).unwrap();
    assert_eq!(fixture.streams.len(), 16);
    assert_eq!(fixture.cases.len(), 48);
    for (seed, iteration, state, draws, values) in [
        (
            3600189262,
            0usize,
            4281969421,
            37,
            [
                200, 900, 700, 400, 500, 200, 700, 0, 700, 600, 200, 800, 0, 400, 500,
            ],
        ),
        (
            4281969421,
            1,
            3939946756,
            41,
            [
                0, 700, 400, 600, 900, 1000, 1000, 0, 200, 1000, 400, 600, 400, 300, 300,
            ],
        ),
        (
            317955982,
            0,
            3110177490,
            36,
            [
                400, 900, 600, 700, 100, 800, 700, 700, 200, 100, 200, 200, 500, 600, 400,
            ],
        ),
        (
            3110177490,
            1,
            1750507675,
            31,
            [
                800, 500, 200, 1000, 500, 200, 600, 0, 300, 900, 1000, 400, 900, 400, 0,
            ],
        ),
    ] {
        let before: Vec<_> = (0..256)
            .map(|field| (((field * 257 + iteration * 31 + 12345) % 65536) as i32 - 32768) as i16)
            .collect();
        let mut person = before.clone();
        for (field, value) in [13, 14, 16, 20, 26, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55]
            .into_iter()
            .zip(values)
        {
            person[field] = value;
        }
        fixture.cases.push(Case {
            seed,
            before,
            original: Expected {
                person,
                state,
                draws,
            },
        });
    }
    assert_eq!(fixture.cases.len(), 52);
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
        let expected_state = (0..case.original.draws).fold(case.seed, |state, _| {
            state.wrapping_mul(214_013).wrapping_add(2_531_011)
        });
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
