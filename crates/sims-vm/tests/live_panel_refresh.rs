//! Source-derived51ce79 tests. No original executable is run by this suite.
use serde::{Deserialize, Serialize};
use sims_vm::interface::live_panel::{
    Neighbor,
    refresh::{Inputs, Refresh, Reply, Request, mood_state},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Host {
    dirty: u32,
    selected: i32,
    mode: i32,
    resident: i32,
    offer: bool,
    offer_visible: u8,
    offer_panel: bool,
    human: bool,
    portraits: Vec<bool>,
    motive_elapsed: u8,
    relation_elapsed: u8,
    requests: Vec<Request>,
}
impl Default for Host {
    fn default() -> Self {
        Self {
            dirty: 0,
            selected: 27,
            mode: 1,
            resident: 0,
            offer: true,
            offer_visible: 0,
            offer_panel: true,
            human: true,
            portraits: vec![true, false, true],
            motive_elapsed: 0,
            relation_elapsed: 0,
            requests: vec![],
        }
    }
}
impl Host {
    fn input(&self) -> Inputs<'_> {
        Inputs {
            dirty: self.dirty,
            mode: self.mode,
            resident_slot: self.resident,
            offer_panel_present: self.offer_panel,
            portrait_slots: &self.portraits,
        }
    }
    fn reply(&mut self, r: &Request) -> Reply {
        self.requests.push(r.clone());
        match r {
            Request::Selected => Reply::Selected(self.selected),
            Request::Switch(mode) => {
                self.mode = *mode;
                Reply::Done
            }
            Request::JobOffer(_) => Reply::Bool(self.offer),
            Request::OfferVisible => Reply::Byte(self.offer_visible),
            Request::Elapsed(0x200) => {
                let again = self.motive_elapsed > 0;
                self.motive_elapsed = self.motive_elapsed.saturating_sub(1);
                Reply::Bool(again)
            }
            Request::Elapsed(0x210) => {
                let again = self.relation_elapsed > 0;
                self.relation_elapsed = self.relation_elapsed.saturating_sub(1);
                Reply::Bool(again)
            }
            Request::Elapsed(_) => panic!("unknown original refresh timer"),
            Request::LookupNeighbor(_) => Reply::Neighbor(Some(Neighbor {
                career: 0,
                kind: if self.human { 1 } else { 2 },
            })),
            Request::FamilyMember(slot) => Reply::Selected(27 + *slot as i32),
            Request::Motives(n) => Reply::Mood(Some(((*n - 27) as f32 * 30.).to_bits())),
            _ => Reply::Done,
        }
    }
}
fn roundtrip<T: Serialize + for<'a> Deserialize<'a>>(value: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap()
}
fn run(mut host: Host, restore: bool, mut callback: impl FnMut(&Request, &mut Host)) -> Host {
    let mut call = Refresh::default();
    for _ in 0..1000 {
        if restore {
            (call, host) = roundtrip(&(call, host));
        }
        let Some(request) = call.advance(host.input()).unwrap() else {
            assert!(call.complete());
            return host;
        };
        if restore {
            (call, host) = roundtrip(&(call, host));
        }
        let before = serde_json::to_vec(&call).unwrap();
        // Both stale acknowledgements and the wrong reply kind are atomic.
        assert!(
            call.acknowledge(
                &Request::PortraitMood {
                    slot: u32::MAX,
                    state: -9
                },
                Reply::Done
            )
            .is_err()
        );
        assert!(
            call.acknowledge(
                &request,
                match request {
                    Request::Selected | Request::FamilyMember(_) => Reply::Bool(false),
                    _ => Reply::Selected(-99),
                }
            )
            .is_err()
        );
        assert_eq!(serde_json::to_vec(&call).unwrap(), before);
        assert_eq!(call.advance(host.input()).unwrap(), Some(request.clone()));
        assert_eq!(serde_json::to_vec(&call).unwrap(), before);
        let reply = host.reply(&request);
        callback(&request, &mut host);
        call.acknowledge(&request, reply).unwrap();
    }
    panic!("unbounded live refresh");
}
fn count(host: &Host, predicate: impl Fn(&Request) -> bool) -> usize {
    host.requests.iter().filter(|r| predicate(r)).count()
}

#[test]
fn dirty_paths_restore_at_every_service_and_preserve_refresh_order() {
    for dirty in [
        0,
        1,
        8,
        9,
        0x10000,
        0x8000,
        0x2000000,
        0x4000000,
        0x20080000,
        u32::MAX,
    ] {
        for mode in 0..=8 {
            for resident in [-1, 0] {
                let start = Host {
                    dirty,
                    mode,
                    resident,
                    motive_elapsed: 1,
                    relation_elapsed: 2,
                    ..Default::default()
                };
                let a = run(start.clone(), false, |_, _| {});
                assert_eq!(a, run(start, true, |_, _| {}));
                assert_eq!(a.dirty, dirty); //51ce79 does not clear the CPState mask.
                assert_eq!(a.requests[0], Request::Selected);
                assert_eq!(
                    count(&a, |r| *r == Request::RebuildPortraits),
                    usize::from(dirty & 0x20080000 != 0)
                );
                assert_eq!(
                    count(&a, |r| *r == Request::Invalidate),
                    usize::from(dirty & 9 != 0)
                );
                assert_eq!(
                    count(&a, |r| matches!(r, Request::Motive { .. })),
                    if dirty & 9 != 0 { 32 } else { 0 }
                );
                let motives: Vec<_> = a
                    .requests
                    .iter()
                    .filter_map(|r| match r {
                        Request::Motive { list, index } => Some((*list, *index)),
                        _ => None,
                    })
                    .collect();
                for (i, &(list, index)) in motives.iter().enumerate() {
                    assert_eq!(
                        (list, index),
                        (if i % 2 == 0 { 0x128 } else { 0x134 }, (i % 16 / 2) as u8)
                    );
                }
                assert_eq!(
                    count(&a, |r| matches!(r, Request::PublishSelection(27))),
                    usize::from(resident == -1 || dirty & 8 != 0)
                );
                assert_eq!(
                    count(&a, |r| matches!(r, Request::Personality { .. })),
                    if resident != -1 && mode == 2 && dirty & 0x10008 != 0 {
                        5
                    } else {
                        0
                    }
                );
                assert_eq!(
                    count(&a, |r| *r == Request::Relationships),
                    if resident != -1 && mode == 5 && dirty & 0x6008008 != 0 {
                        3
                    } else {
                        0
                    }
                );
                assert_eq!(
                    count(&a, |r| *r == Request::Inventory),
                    usize::from(resident != -1 && mode == 6 && dirty & 0x6008008 != 0)
                );
                assert_eq!(
                    count(&a, |r| *r == Request::Interests),
                    usize::from(resident != -1 && mode == 7 && dirty & 0x6008008 != 0)
                );
                assert_eq!(
                    &a.requests[a.requests.len() - 8..],
                    &[
                        Request::FamilyMember(0),
                        Request::Motives(27),
                        Request::Motives(27),
                        Request::PortraitMood { slot: 0, state: 0 },
                        Request::FamilyMember(2),
                        Request::Motives(29),
                        Request::Motives(29),
                        Request::PortraitMood { slot: 2, state: 1 },
                    ]
                );
            }
        }
    }
}

#[test]
fn callbacks_change_live_fields_but_not_captured_flags_or_selected_index() {
    let a = run(
        Host {
            dirty: 9,
            ..Default::default()
        },
        true,
        |r, h| match r {
            Request::Selected => {
                h.dirty = 0;
                h.selected = 99;
            }
            Request::Invalidate => {
                h.mode = 3;
            }
            Request::Visible { member: 0x124, .. } => {
                h.mode = 2;
            }
            Request::Personality { index: 0, .. } => {
                h.mode = 7;
                h.resident = -1;
            }
            _ => (),
        },
    );
    assert!(a.requests.contains(&Request::JobOffer(27)));
    assert_eq!(count(&a, |r| matches!(r, Request::Motive { .. })), 16);
    assert_eq!(
        count(&a, |r| matches!(
            r,
            Request::Personality { selected: 27, .. }
        )),
        5
    );
    assert!(a.requests.contains(&Request::Interests));
    assert!(a.requests.contains(&Request::PublishSelection(27)));
    assert!(!a.requests.contains(&Request::PublishSelection(99)));

    // The relationship loop does not rebranch after473311's callback.
    let b = run(
        Host {
            dirty: 0x8000,
            mode: 5,
            relation_elapsed: 2,
            ..Default::default()
        },
        true,
        |r, h| {
            if *r == Request::Relationships {
                h.mode = 7;
            }
        },
    );
    assert_eq!(count(&b, |r| *r == Request::Relationships), 3);
    assert!(!b.requests.contains(&Request::Interests));

    // Invalid cached resident skips the personality/extras block even if the
    // selection publication callback repairs that field before returning.
    let c = run(
        Host {
            dirty: u32::MAX,
            mode: 2,
            resident: -1,
            ..Default::default()
        },
        true,
        |r, h| {
            if matches!(r, Request::PublishSelection(_)) {
                h.resident = 0;
            }
        },
    );
    assert!(
        !c.requests
            .iter()
            .any(|r| matches!(r, Request::Personality { .. }))
    );
}

#[test]
fn offer_branch_preserves_byte_tests_and_selected_minus_one_skips_job_query() {
    for offer in [false, true] {
        for visible in [0, 1, 2, 255] {
            let a = run(
                Host {
                    mode: 3,
                    offer,
                    offer_visible: visible,
                    ..Default::default()
                },
                true,
                |_, _| {},
            );
            let events: Vec<_> = a
                .requests
                .iter()
                .filter_map(|r| match r {
                    Request::Visible { member, shown } => Some((*member, *shown)),
                    _ => None,
                })
                .collect();
            if (offer && visible == 0) || (!offer && visible == 1) {
                assert_eq!(
                    events,
                    [
                        (0x11c, offer),
                        (0x110, !offer),
                        (0x114, false),
                        (0x118, false),
                        (0x124, false)
                    ]
                );
            } else {
                assert!(events.is_empty());
            }
        }
    }
    let a = run(
        Host {
            mode: 3,
            selected: -1,
            ..Default::default()
        },
        true,
        |r, h| {
            if *r == Request::Switch(0) {
                h.mode = 3;
            }
        },
    );
    assert!(a.requests.contains(&Request::Switch(0)));
    assert!(!a.requests.iter().any(|r| matches!(r, Request::JobOffer(_))));
}

#[test]
fn portrait_loop_rereads_slots_and_does_not_limit_itself_to_eight() {
    let a = run(
        Host {
            portraits: vec![true; 10],
            ..Default::default()
        },
        true,
        |r, h| {
            if matches!(r, Request::PortraitMood { slot: 0, .. }) {
                h.portraits[1] = false;
            }
            if matches!(r, Request::PortraitMood { slot: 9, .. }) {
                h.portraits.push(true);
            }
        },
    );
    assert!(!a.requests.contains(&Request::FamilyMember(1)));
    assert!(a.requests.contains(&Request::FamilyMember(10)));
    assert_eq!(count(&a, |r| matches!(r, Request::FamilyMember(_))), 10);
}

#[test]
fn portrait_mood_uses_second_getter_and_retains_native_nan_boundary_rules() {
    for (v, expected) in [
        (f32::NEG_INFINITY, 2),
        (-25.000002, 2),
        (-25., 0),
        (-0., 0),
        (25., 0),
        (25.000002, 1),
        (f32::INFINITY, 1),
        (f32::NAN, 2),
    ] {
        assert_eq!(mood_state(v.to_bits()), expected);
    }
    let mut host = Host {
        portraits: vec![true],
        ..Default::default()
    };
    let mut call = Refresh::default();
    let mut getters = 0;
    loop {
        let Some(r) = call.advance(host.input()).unwrap() else {
            break;
        };
        let reply = if matches!(r, Request::Motives(_)) {
            getters += 1;
            Reply::Mood(Some(if getters == 1 { 100f32 } else { -100f32 }.to_bits()))
        } else {
            if matches!(r, Request::PortraitMood { .. }) {
                assert_eq!(r, Request::PortraitMood { slot: 0, state: 2 });
            }
            host.reply(&r)
        };
        call.acknowledge(&r, reply).unwrap();
        call = roundtrip(&call);
    }
    assert_eq!(getters, 2);
    // Missing second data is a preserved boundary, never synthesized zero.
    let mut call = Refresh::default();
    getters = 0;
    loop {
        let r = call.advance(host.input()).unwrap().unwrap();
        let reply = if matches!(r, Request::Motives(_)) {
            getters += 1;
            Reply::Mood((getters == 1).then_some(0))
        } else {
            host.reply(&r)
        };
        call.acknowledge(&r, reply).unwrap();
        if getters == 2 {
            break;
        }
    }
    let before = serde_json::to_vec(&call).unwrap();
    for _ in 0..2 {
        call = roundtrip(&call);
        assert!(
            call.advance(host.input())
                .unwrap_err()
                .contains("missing second motives")
        );
        assert_eq!(serde_json::to_vec(&call).unwrap(), before);
    }
}
