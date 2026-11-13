use serde::Deserialize;
use sims_sim::object_catalog::move_tool::{feedback::Call, sell::Feedback};

#[derive(Deserialize)]
struct Case {
    input: Input,
    original: Expected,
}
#[derive(Deserialize)]
struct Input {
    kind: Feedback,
    last: u32,
    now: u32,
    after: u32,
    mutate_last: bool,
}
#[derive(Deserialize, Debug, PartialEq, Eq)]
struct Expected {
    sounds: Vec<Feedback>,
    last: u32,
    clock: u32,
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_feedback_gate_and_post_sound_clock_store_survive_restore() {
    #[derive(Deserialize)]
    struct Corpus {
        cases: Vec<Case>,
        constructor: Constructor,
    }
    #[derive(Deserialize)]
    struct Constructor {
        timestamps: Vec<u32>,
        pending: i32,
        category: i32,
        special: u8,
    }
    let corpus: Corpus =
        serde_json::from_str(include_str!("fixtures/furniture-feedback.json")).unwrap();
    assert_eq!(corpus.cases.len(), 1458);
    assert_eq!(corpus.constructor.timestamps, vec![0; 7]);
    assert_eq!(corpus.constructor.pending, 0);
    assert_eq!(
        corpus.constructor.category,
        sims_sim::object_catalog::move_tool::Fields::constructor_fields().cursor
    );
    assert_eq!(corpus.constructor.special, 0);
    for c in corpus.cases {
        for cold in [false, true] {
            let mut call = Call::new(c.input.kind);
            let (mut last, mut clock) = (c.input.last, c.input.now);
            let mut sounds = vec![];
            for _ in 0..4 {
                if cold {
                    call = serde_json::from_str(&serde_json::to_string(&call).unwrap()).unwrap();
                }
                if let Some(kind) = call.advance(&mut last, clock).unwrap() {
                    let before = call.clone();
                    assert_eq!(call.advance(&mut last, clock).unwrap(), Some(kind));
                    assert_eq!(before, call);
                    sounds.push(kind);
                    clock = c.input.after;
                    if c.input.mutate_last {
                        last = 123456;
                    }
                    call.acknowledge(kind).unwrap();
                    assert!(call.acknowledge(kind).is_err());
                }
                if call.complete() {
                    break;
                }
            }
            assert!(call.complete());
            assert_eq!(
                Expected {
                    sounds,
                    last,
                    clock
                },
                c.original
            );
        }
    }
}
#[test]
fn sound_reply_must_own_the_pending_feedback() {
    let mut c = Call::new(Feedback::MoneyBack);
    let mut last = 0;
    assert!(c.acknowledge(Feedback::MoneyBack).is_err());
    assert_eq!(
        c.advance(&mut last, 401).unwrap(),
        Some(Feedback::MoneyBack)
    );
    let saved = c.clone();
    assert!(c.acknowledge(Feedback::Blocked).is_err());
    assert_eq!(saved, c);
    c.acknowledge(Feedback::MoneyBack).unwrap();
    assert_eq!(last, 0);
    assert_eq!(c.advance(&mut last, 900).unwrap(), None);
    assert_eq!(last, 900);
    assert!(c.complete());
}
