use sims_vm::interface::{
    buy::tool::{Reply, Request as R, Selection},
    state::State,
    tool,
};

#[test]
fn buy_tool_selectors_publish_before_callbacks_and_unknown_values_only_store() {
    for selector in [-7, 0, 1, 2, 3, 4, i32::MAX] {
        let mut s = State::constructed(760);
        let mut c = Selection::new(selector);
        let original = s.clone();
        let r = c.advance(&mut s).unwrap();
        let mut expected = original;
        expected.buy_tool = selector;
        assert_eq!(s, expected);
        assert_eq!(
            r,
            match selector {
                0 | 3 => Some(R::Select {
                    index: 7,
                    argument: u32::MAX
                }),
                1 | 2 => Some(R::Clear),
                _ => None,
            }
        );
        if let Some(r) = r {
            let saved = serde_json::to_vec(&c).unwrap();
            c = serde_json::from_slice(&saved).unwrap();
            s.buy_tool = 99;
            let before = s.clone();
            assert_eq!(c.advance(&mut s).unwrap(), Some(r));
            assert_eq!(s, before);
            c.acknowledge(r, Reply::Done).unwrap();
        }
        assert_eq!(c.advance(&mut s).unwrap(), None);
        assert!(c.complete());
    }
}

#[test]
fn selected_definition_guid_is_captured_across_save_and_reentrant_changes() {
    let mut s = State::constructed(760);
    s.buy.selected_object = 51;
    let mut c = Selection::new(3);
    assert_eq!(c.advance(&mut s).unwrap(), Some(R::Guid(51)));
    let before = c.clone();
    assert!(c.acknowledge(R::Guid(51), Reply::Done).is_err());
    assert_eq!(c, before);
    c.acknowledge(R::Guid(51), Reply::Guid(0)).unwrap();
    c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
    s.buy.selected_object = 123;
    s.buy_tool = 1;
    assert_eq!(c.advance(&mut s).unwrap(), Some(R::SelectDefinition(0)));
    assert_eq!(s.buy_tool, 1);
    assert_eq!(s.buy.selected_object, 123);
    c.acknowledge(R::SelectDefinition(0), Reply::Done).unwrap();
    assert!(c.complete());
}

#[test]
fn registry_lookup_uses_signed_low_word_and_keeps_null_slots() {
    let mut slots = std::array::from_fn(|n| Some(100 + n as u32));
    slots[7] = None;
    slots[8] = Some(0);
    for (index, expected) in [
        (0, Some(100)),
        (11, Some(111)),
        (12, None),
        (-1, None),
        (65536, Some(100)),
        (65547, Some(111)),
        (65535, None),
        (0x8000, None),
        (7, None),
        (8, None),
    ] {
        assert_eq!(tool::lookup(&slots, index), expected);
    }
}
