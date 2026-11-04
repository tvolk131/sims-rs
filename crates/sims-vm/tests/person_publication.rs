use native::{Decision, View};
use sims_vm::{
    ObjectId, image_cache,
    interface::{person_selection as native, state::State},
};
use std::cell::RefCell;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Input {
    index: u32,
    registry: bool,
    instance: bool,
    zoning: i32,
    lot: i16,
    override_word: u32,
    visitor: i16,
    family: i16,
    controlled: i32,
}
struct Fixture {
    input: Input,
    reads: RefCell<Vec<String>>,
}
impl Fixture {
    fn read(&self, name: impl Into<String>) {
        self.reads.borrow_mut().push(name.into());
    }
}
impl View for Fixture {
    fn cache_guid(&self, n: u32) -> Result<u32, String> {
        self.read(format!("cache:{n}"));
        Ok(if n == 0 { 0x1234 } else { 0 })
    }
    fn registry_present(&self) -> bool {
        self.read("registry");
        self.input.registry
    }
    fn first_instance(&mut self, g: u32) -> Result<Option<ObjectId>, String> {
        assert_eq!(g, 0x1234);
        self.read("instance");
        Ok(self.input.instance.then_some(21))
    }
    fn zoning(&self) -> Result<i32, String> {
        self.read("zoning");
        Ok(self.input.zoning)
    }
    fn lot_number(&self) -> Result<i16, String> {
        self.read("lot");
        Ok(self.input.lot)
    }
    fn selection_override(&self) -> Result<u32, String> {
        self.read("override");
        Ok(self.input.override_word)
    }
    fn person_data(&self, _: ObjectId, index: i16) -> Result<i16, String> {
        self.read(format!("person:{index}"));
        match index {
            32 => Ok(self.input.visitor),
            61 => Ok(self.input.family),
            _ => unreachable!(),
        }
    }
    fn controlled_family(&self) -> Result<i32, String> {
        self.read("family");
        Ok(self.input.controlled)
    }
}
fn input() -> Input {
    Input {
        index: 0,
        registry: true,
        instance: true,
        zoning: 1,
        lot: 5,
        override_word: 0,
        visitor: 0,
        family: 5,
        controlled: 5,
    }
}
fn evaluate(i: Input) -> (Decision, Vec<String>) {
    let index = i.index;
    let mut f = Fixture {
        input: i,
        reads: RefCell::default(),
    };
    (native::select(&mut f, index).unwrap(), f.reads.into_inner())
}

#[test]
fn absent_registry_invalid_index_and_missing_instance_have_distinct_returns() {
    let mut i = input();
    i.registry = false;
    assert_eq!(
        evaluate(i),
        (Decision::Return, vec!["cache:0".into(), "registry".into()])
    );
    for index in [1, u32::MAX, 0x8000_0000] {
        let mut i = input();
        i.index = index;
        assert_eq!(
            evaluate(i),
            (
                Decision::Select(None),
                vec![format!("cache:{index}"), "registry".into()]
            )
        );
    }
    let mut i = input();
    i.instance = false;
    assert_eq!(evaluate(i).0, Decision::Select(None));
}

#[test]
fn lot_policy_preserves_precedence_signed_fields_and_early_returns() {
    for zoning in [-1, 0, 1, 2, 3] {
        for lot in [i16::MIN, 0, 20, 21, 30, 31, 39, 40, 49, 50, i16::MAX] {
            for visitor in [-1, 0, 1, 2, 3] {
                for override_word in [0, 1, 256, u32::MAX] {
                    for family in [-1, 5, 6] {
                        let mut i = input();
                        i.zoning = zoning;
                        i.lot = lot;
                        i.visitor = visitor;
                        i.override_word = override_word;
                        i.family = family;
                        let (result, reads) = evaluate(i);
                        let expected = match (zoning, lot, override_word, visitor, family) {
                            (1, _, 0, 0, _) | (1, _, 1..=u32::MAX, _, _) => {
                                Decision::Select(Some(21))
                            }
                            (1, _, 0, _, _) => Decision::Select(None),
                            (_, 21..=30, 0, 2, _) => Decision::Select(None),
                            (_, 21..=30, _, _, _) => Decision::Select(Some(21)),
                            (2, _, 0, 2, f) | (_, 40..=49, 0, 2, f) if f != 5 => {
                                Decision::Select(None)
                            }
                            (2, _, _, _, _) | (_, 40..=49, _, _, _) => Decision::Select(Some(21)),
                            _ => Decision::Return,
                        };
                        assert_eq!(result, expected);
                        if result == Decision::Return {
                            assert!(!reads.contains(&"override".into()));
                        }
                        if override_word != 0 {
                            assert!(!reads.iter().any(|s| s.starts_with("person:")));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn cache_lookup_uses_first_guid_unsigned_index_and_legacy_cp_absence() {
    let c = image_cache::State {
        entries: vec![
            image_cache::Entry::new(7),
            image_cache::Entry::new(9),
            image_cache::Entry::new(7),
        ],
        ..Default::default()
    };
    assert_eq!(native::cache_index(&c, 7), 0);
    assert_eq!(native::cache_index(&c, 8), -1);
    assert_eq!(native::cache_guid(&c, 2), 7);
    assert_eq!(native::cache_guid(&c, u32::MAX), 0);
    let mut state = serde_json::to_value(State::constructed(760)).unwrap();
    assert_eq!(state["selected_person_index"], -1);
    state
        .as_object_mut()
        .unwrap()
        .remove("selected_person_index");
    let legacy: State = serde_json::from_value(state.clone()).unwrap();
    assert_eq!(legacy.selected_person_index, None);
    assert_eq!(serde_json::to_value(legacy).unwrap(), state);
}

#[test]
#[ignore = "requires guarded original4e989b execution"]
fn original_person_publication_policy_matches() {
    let p = std::env::var("SIMS_PERSON_PUBLICATION_ORACLE").unwrap();
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
    assert_eq!(data["native_execution"], true);
    assert_eq!(
        data["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        data["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let rows = data["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 3303);
    for row in rows {
        let i: Input = serde_json::from_value(row["input"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(evaluate(i).0).unwrap(),
            row["original"]
        );
    }
}
