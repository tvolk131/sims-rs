//! Source-derived integration checks for the recovered UI calls. These are
//! deliberately not labeled fresh original-code differential execution.
use serde::{Deserialize, Serialize};
use sims_vm::interface::live_panel::{
    Neighbor, PANELS, Reply, Request, Switch, button_member, button_resource,
};
use sims_vm::ui_effect::widgets::button;
use sims_vm::ui_owner::wait;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Host {
    mode: i32,
    buttons: BTreeMap<u16, i32>,
    visible: BTreeMap<u16, bool>,
    flags: u32,
    neighbor: i32,
    data: Option<Neighbor>,
    bindings: Vec<(u16, u8, i32)>,
    events: Vec<(i32, Request)>,
}
impl Host {
    fn new(mode: i32, data: Option<Neighbor>) -> Self {
        Self {
            mode,
            buttons: (1..=7)
                .map(|m| (button_member(m).unwrap(), i32::from(m == mode)))
                .collect(),
            visible: PANELS.into_iter().map(|m| (m, true)).collect(),
            flags: 0x80000000,
            neighbor: 27,
            data,
            bindings: vec![],
            events: vec![],
        }
    }
}
fn restore<T: Serialize + for<'a> Deserialize<'a>>(value: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap()
}
fn run(
    requested: i32,
    mut host: Host,
    restored: bool,
    mutate: bool,
    house: bool,
    offer: bool,
    offer_panel: bool,
) -> Host {
    let mut call = Switch::new(requested);
    let mut count = 0;
    loop {
        if restored {
            (call, host) = restore(&(call, host));
        }
        let Some(request) = call.advance(&mut host.mode, true).unwrap() else {
            break;
        };
        let saved = serde_json::to_vec(&(call.clone(), host.clone())).unwrap();
        assert!(
            call.acknowledge(&Request::Dirty(0xdeadbeef), Reply::Done)
                .is_err()
        );
        assert_eq!(
            saved,
            serde_json::to_vec(&(call.clone(), host.clone())).unwrap()
        );
        assert_eq!(
            call.advance(&mut host.mode, true).unwrap(),
            Some(request.clone())
        );
        host.events.push((host.mode, request.clone()));
        let reply = match request.clone() {
            Request::ClosePopup => {
                if mutate {
                    host.mode = 5;
                }
                Reply::Done
            }
            Request::Button { member, state } => {
                // Execute the shared setter rather than making the category
                // switch silently bypass its callback sequence.
                let mut setter = button::Setter::new(state);
                while let Some(r) = setter
                    .advance(host.buttons.get_mut(&member).unwrap())
                    .unwrap()
                {
                    if restored {
                        setter = restore(&setter);
                    }
                    let reply = if r == button::Request::Parent {
                        button::Reply::Parent(None)
                    } else {
                        button::Reply::Done
                    };
                    setter.acknowledge(&r, reply).unwrap();
                }
                if mutate && state == 1 {
                    host.mode = 0;
                }
                Reply::Done
            }
            Request::HouseAllowed => Reply::Bool(house),
            Request::Visible { member, shown } => {
                host.visible.insert(member, shown);
                if mutate && member == 0x410 && !shown {
                    host.mode = 2;
                }
                Reply::Done
            }
            Request::SelectedNeighbor => Reply::Selected(host.neighbor),
            Request::LookupNeighbor(_) => Reply::Neighbor(host.data),
            Request::JobOffer(_) => Reply::Bool(offer),
            Request::OfferPanelPresent => Reply::Bool(offer_panel),
            Request::BindMotive {
                list,
                index,
                neighbor,
            } => {
                host.bindings.push((list, index, neighbor));
                // A binding callback changes the selected resident. The next
                // original getter must observe it, even within the same pair.
                host.neighbor += 1;
                Reply::Done
            }
            Request::Dirty(flags) => {
                host.flags |= flags;
                Reply::Done
            }
            Request::Refresh | Request::Invalidate => Reply::Done,
        };
        call.acknowledge(&request, reply).unwrap();
        count += 1;
        assert!(count < 200);
    }
    assert!(call.complete());
    host
}

#[test]
fn category_switch_restores_callbacks_and_uses_live_publication_fields() {
    for mode in [0, 1, 2, 3, 4, 5, 6, 7, 99] {
        for kind in [-1, 0, 1, 2, 4, 6] {
            for career in [-3, 0, 1, 17, 18] {
                let start = Host::new(3, Some(Neighbor { career, kind }));
                let a = run(mode, start.clone(), false, false, true, true, true);
                let b = run(mode, start, true, false, true, true, true);
                assert_eq!(a, b, "mode{mode}, career{career}, kind{kind}");
                assert_eq!(a.flags & 0x80000000, 0x80000000);
                let expected = match mode {
                    0 | 99 => vec![],
                    1 => vec![if kind <= 1 { 0x100 } else { 0x104 }],
                    2 => {
                        if kind <= 1 {
                            vec![0x108, 0x1c4]
                        } else {
                            vec![0x108]
                        }
                    }
                    3 => vec![if career != 0 && career < 18 {
                        0x124
                    } else if kind & 2 != 0 {
                        0x114
                    } else if kind & 4 != 0 {
                        0x118
                    } else {
                        0x11c
                    }],
                    4 => vec![0x120],
                    5 => vec![0x10c],
                    6 => vec![0x338],
                    7 => vec![0x410],
                    _ => unreachable!(),
                };
                let shown: Vec<_> = a
                    .visible
                    .iter()
                    .filter_map(|(&k, &v)| v.then_some(k))
                    .collect();
                assert_eq!(shown, expected);
                if mode == 1 {
                    assert_eq!(a.bindings.len(), 16);
                    for (i, &(list, index, n)) in a.bindings.iter().enumerate() {
                        assert_eq!(
                            (list, index, n),
                            (
                                if i % 2 == 0 { 0x128 } else { 0x134 },
                                i as u8 / 2,
                                27 + i as i32
                            )
                        );
                    }
                }
                if mode == 0 {
                    assert!(
                        !a.events
                            .iter()
                            .any(|(_, e)| matches!(e, Request::Refresh | Request::Invalidate))
                    );
                } else {
                    assert!(matches!(
                        &a.events[a.events.len() - 2..],
                        [(_, Request::Refresh), (_, Request::Invalidate)]
                    ));
                }
            }
        }
    }
    let a = run(1, Host::new(3, None), true, true, true, false, false);
    assert_eq!(a.mode, 0); // mutation after the new button callback persists
    assert_eq!(
        a.events[1].1,
        Request::Button {
            member: 0x160,
            state: 0
        }
    ); // old mode reread after close
    assert_eq!(a.buttons[&0x16c], 1); // branch reread after hiding children
    assert!(a.bindings.is_empty());
    assert_eq!(a.visible[&0x108], true);
    assert_eq!(a.visible[&0x1c4], false);
}

#[test]
fn switch_preserves_zero_return_gates_and_absent_optional_offer() {
    let blocked = run(4, Host::new(1, None), true, false, false, false, false);
    assert_eq!(blocked.mode, 0);
    assert!(blocked.visible.values().all(|v| !*v));
    assert_eq!(blocked.flags, 0x80000000);
    assert!(
        !blocked
            .events
            .iter()
            .any(|(_, r)| matches!(r, Request::Refresh | Request::Invalidate))
    );
    let missing = run(
        3,
        Host::new(1, Some(Neighbor { career: 0, kind: 1 })),
        true,
        false,
        true,
        true,
        false,
    );
    assert!(missing.visible.values().all(|v| !*v));
    assert_eq!(missing.buttons[&0x164], 1);
    assert_eq!(missing.flags, 0x80020000);
    let no_offer = run(
        3,
        Host::new(1, Some(Neighbor { career: 0, kind: 1 })),
        true,
        false,
        true,
        false,
        false,
    );
    assert!(no_offer.visible[&0x110]);
    assert!(
        !no_offer
            .events
            .iter()
            .any(|(_, r)| *r == Request::OfferPanelPresent)
    );
    let missing_person = run(1, Host::new(0, None), true, false, true, false, false);
    assert!(missing_person.visible[&0x104]);
}

#[test]
fn button_state_callbacks_reread_parent_and_do_not_rewrite_callback_changes() {
    for restored in [false, true] {
        let mut setter = button::Setter::new(-7);
        let mut state = 0;
        let mut events = vec![];
        let mut parents = 0;
        while let Some(r) = setter.advance(&mut state).unwrap() {
            if restored {
                setter = restore(&setter);
            }
            events.push((state, r.clone()));
            let reply = match r {
                button::Request::Changed => {
                    state = 5;
                    button::Reply::Done
                }
                button::Request::Parent => {
                    parents += 1;
                    button::Reply::Parent(Some(if parents == 1 { 17 } else { 23 }))
                }
                _ => button::Reply::Done,
            };
            setter.acknowledge(&r, reply).unwrap();
        }
        assert_eq!(state, 5);
        assert_eq!(
            events,
            vec![
                (-7, button::Request::Changed),
                (5, button::Request::Invalidate),
                (5, button::Request::Parent),
                (5, button::Request::Parent),
                (5, button::Request::InvalidateParent(23))
            ]
        );
        assert!(setter.complete());
        let mut same = button::Setter::new(state);
        assert_eq!(same.advance(&mut state).unwrap(), None);
        assert!(same.complete());
    }
}

#[test]
fn tutorial_control_watch_observes_selected_button_not_enabled_widget() {
    assert_eq!(
        (1..=7).filter_map(button_resource).collect::<Vec<_>>(),
        vec![4500, 4501, 4503, 4502, 4504, 4505, 4507]
    );
    for button_state in [0, 1, 7, u32::MAX] {
        let mut poll = wait::Poll::begin(Some(1), None).unwrap();
        let mut state = wait::State::default();
        poll.acknowledge(
            &mut state,
            &wait::Request::PublishControl(0),
            wait::Reply::Done,
        )
        .unwrap();
        poll.acknowledge(
            &mut state,
            &wait::Request::Query(1),
            wait::Reply::Query(Some([8, 4500])),
        )
        .unwrap();
        poll.acknowledge(
            &mut state,
            &wait::Request::Observe(wait::Condition::Control(4500)),
            wait::Reply::Observed(wait::Observation::Control {
                image: Some(4500),
                state: button_state,
            }),
        )
        .unwrap();
        assert_eq!(
            poll.request(),
            if button_state == 0 {
                None
            } else {
                Some(&wait::Request::Got(1))
            }
        );
    }
}

#[test]
fn missing_required_live_pointers_stop_at_the_actual_dereference() {
    let mut setter = button::Setter::new(1);
    let mut state = 0;
    for (request, reply) in [
        (button::Request::Changed, button::Reply::Done),
        (button::Request::Invalidate, button::Reply::Done),
        (button::Request::Parent, button::Reply::Parent(Some(17))),
        (button::Request::Parent, button::Reply::Parent(None)),
    ] {
        assert_eq!(setter.advance(&mut state).unwrap(), Some(request.clone()));
        setter.acknowledge(&request, reply).unwrap();
        setter = restore(&setter);
    }
    let frozen = setter.clone();
    for _ in 0..2 {
        assert!(
            setter
                .advance(&mut state)
                .unwrap_err()
                .contains("second parent")
        );
        assert_eq!(setter, frozen);
    }
    let mut call = Switch::new(3);
    let mut mode = 0;
    let mut queries = 0;
    loop {
        let request = match call.advance(&mut mode, false) {
            Ok(Some(r)) => r,
            Err(error) => {
                assert!(error.contains("missing neighbor"));
                break;
            }
            Ok(None) => panic!("missing person silently completed the job panel"),
        };
        let reply = match request {
            Request::SelectedNeighbor => {
                queries += 1;
                Reply::Selected(-1)
            }
            Request::LookupNeighbor(-1) => Reply::Neighbor(None),
            Request::Visible { shown: false, .. } => Reply::Done,
            other => panic!("unexpected operation before missing-person dereference: {other:?}"),
        };
        call.acknowledge(&request, reply).unwrap();
        call = restore(&call);
    }
    assert_eq!(queries, 2);
    let frozen = call.clone();
    assert!(call.advance(&mut mode, false).is_err());
    assert_eq!(call, frozen);
}
