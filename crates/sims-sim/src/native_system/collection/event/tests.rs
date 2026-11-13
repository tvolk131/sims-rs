use super::*;

fn cursor() -> super::super::Construction {
    super::super::tests::event_fixture()
}

#[derive(Default)]
struct Port {
    handle: u64,
    attempts: usize,
    effects: usize,
    returned: Option<Return>,
    lost: bool,
    hold: bool,
    fail_sync: bool,
    syncs: usize,
    admitted: usize,
}
impl Host for Port {
    fn create_event(&mut self, call: &Call) -> Result<Return, Stop> {
        self.attempts += 1;
        if self.hold {
            return Err(unavailable("authored pre-effect hold"));
        }
        if self.returned.is_none() {
            self.returned = Some(Return::from_host(call, self.handle)?);
            self.effects += 1;
        }
        let reply = self.returned.as_ref().unwrap().clone();
        if self.lost {
            self.lost = false;
            return Err(unavailable("authored lost actual return"));
        }
        Ok(reply)
    }
    fn synchronize_event(&mut self, cut: &Checkpoint) -> Result<(), Stop> {
        self.syncs += 1;
        cut.validate()?;
        if cut.source_ordinal() < self.admitted {
            return Err(unsupported("authored monotonic host"));
        }
        if self.fail_sync {
            self.fail_sync = false;
            return Err(unavailable("authored cut hold"));
        }
        self.admitted = cut.source_ordinal();
        Ok(())
    }
}
fn finish(cursor: &mut super::super::Construction, host: &mut impl Host) -> Checkpoint {
    for _ in 0..16 {
        if cursor.event_step(host).unwrap() == Some(Cut::BeforeCounter) {
            return cursor.event_checkpoint().unwrap().unwrap();
        }
    }
    panic!("event source did not reach counter boundary");
}

#[test]
fn event_success_replays_exact_return_flag_and_list_vptr_without_counter_effect() {
    let mut c = cursor();
    let before = c.checkpoint().unwrap();
    let mut h = Port {
        handle: 0x0800_0123,
        ..Default::default()
    };
    let cut = finish(&mut c, &mut h);
    assert_eq!((h.attempts, h.effects, h.syncs), (1, 1, 1));
    assert_eq!(cut.call().arguments(), [0; 4]);
    assert_eq!(
        (cut.call().instruction(), cut.call().import_slot()),
        (0x5d9339, 0x666238)
    );
    assert!(cut.before_counter());
    assert_eq!(
        cut.writes()
            .iter()
            .map(|w| (w.offset, w.instruction))
            .collect::<Vec<_>>(),
        vec![(260, 0x5d9341), (268, 0x5d9346), (168, 0x6017f6)]
    );
    let mut expected = before.parent().clone();
    for (at, value) in [(260, 0x0800_0123_u32), (268, 1), (168, 0x676990)] {
        expected.data[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    assert_eq!(cut.parent(), &expected);
    assert_eq!(cut.caller_frame(), before.caller_frame());
    assert_eq!(&cut.parent().data[272..], &before.parent().data[272..]);
    assert!(
        cut.writes()
            .iter()
            .all(|w| w.after.len() == 4 && !w.child_return && w.owner == expected.token)
    );
    let calls = (h.attempts, h.effects, h.syncs);
    for _ in 0..3 {
        assert!(matches!(c.event_step(&mut h), Err(Stop::Unavailable(_))));
    }
    assert_eq!((h.attempts, h.effects, h.syncs), calls);
}

#[test]
fn event_null_source_return_does_not_manufacture_a_zero_flag_store() {
    let mut c = cursor();
    let mut h = Port::default();
    let cut = finish(&mut c, &mut h);
    assert_eq!(cut.returned().unwrap().handle(), 0);
    assert_eq!(
        cut.writes()
            .iter()
            .map(|w| (w.offset, w.instruction))
            .collect::<Vec<_>>(),
        vec![(260, 0x5d9341), (168, 0x6017f6)]
    );
    assert_eq!(&cut.parent().data[268..272], &[0; 4]);
    cut.validate().unwrap();
}

#[test]
fn event_requires_the_completed_preceding_synchronization_cut() {
    let ready = cursor();
    let mut early = super::super::Construction::new(super::super::Incoming {
        factory: ready.request().factory().clone(),
        frames: ready.request().frames(),
    })
    .unwrap();
    let mut h = Port::default();
    assert!(early.event_step(&mut h).is_err());
    assert!(early.event_checkpoint().unwrap().is_none());
    assert_eq!(h.attempts, 0);
}

#[test]
fn event_pre_effect_error_retains_the_sealed_call_and_empty_receipt() {
    let mut c = cursor();
    let mut h = Port {
        hold: true,
        handle: 41,
        ..Default::default()
    };
    assert!(c.event_step(&mut h).is_err());
    let call = c.event_call().unwrap().clone();
    let cut = c.event_checkpoint().unwrap().unwrap();
    assert!(cut.returned().is_none());
    assert_eq!(cut.source_ordinal(), 0);
    assert!(c.event_step(&mut h).is_err());
    assert!(call.same_call(c.event_call().unwrap()));
    assert_eq!(h.effects, 0);
    h.hold = false;
    finish(&mut c, &mut h);
    assert_eq!(h.effects, 1);
}

#[test]
fn event_lost_return_retries_one_owned_effect_and_exact_receipt() {
    let mut c = cursor();
    let mut h = Port {
        handle: 77,
        lost: true,
        ..Default::default()
    };
    assert!(c.event_step(&mut h).is_err());
    let actual = h.returned.as_ref().unwrap().clone();
    assert!(c.event_checkpoint().unwrap().unwrap().returned().is_none());
    c.event_step(&mut h).unwrap();
    assert!(actual.same_receipt(c.event_checkpoint().unwrap().unwrap().returned().unwrap()));
    finish(&mut c, &mut h);
    assert_eq!((h.attempts, h.effects), (2, 1));
}

#[test]
fn event_invalid_actual_reply_stays_staged_without_a_replacement_call() {
    let mut other = cursor();
    let mut hold = Port {
        hold: true,
        ..Default::default()
    };
    assert!(other.event_step(&mut hold).is_err());
    let wrong = Return::from_host(other.event_call().unwrap(), 99).unwrap();
    let mut c = cursor();
    let mut h = Port {
        returned: Some(wrong),
        ..Default::default()
    };
    assert!(c.event_step(&mut h).is_err());
    assert!(c.event_checkpoint().is_err());
    for _ in 0..3 {
        assert!(c.event_step(&mut h).is_err());
    }
    assert_eq!(h.attempts, 1);
    assert!(c.event.as_ref().unwrap().state.returned.is_some());
}

#[test]
fn event_cut_failure_retries_without_reissuing_or_reapplying_stores() {
    let mut c = cursor();
    let mut h = Port {
        handle: 73,
        fail_sync: true,
        ..Default::default()
    };
    let mut stopped = false;
    for _ in 0..8 {
        if c.event_step(&mut h).is_err() {
            stopped = true;
            break;
        }
    }
    assert!(stopped);
    let cut = c.event_checkpoint().unwrap().unwrap();
    assert!(cut.before_counter());
    let parent = cut.parent().clone();
    let writes = cut.writes().to_vec();
    assert_eq!(c.event_step(&mut h).unwrap(), Some(Cut::BeforeCounter));
    let final_cut = c.event_checkpoint().unwrap().unwrap();
    assert_eq!(final_cut.parent(), &parent);
    assert_eq!(final_cut.writes(), writes);
    assert_eq!((h.attempts, h.effects, h.syncs), (1, 1, 2));
}

#[test]
fn event_return_native_width_and_receipt_identity_are_not_interchangeable() {
    let mut c = cursor();
    let mut h = Port {
        hold: true,
        ..Default::default()
    };
    c.event_step(&mut h).unwrap_err();
    let call = c.event_call().unwrap();
    assert!(Return::from_host(call, 1_u64 << 32).is_err());
    let a = Return::from_host(call, 5).unwrap();
    let b = Return::from_host(call, 5).unwrap();
    assert!(a.same_receipt(&a.clone()));
    assert!(!a.same_receipt(&b));
    a.validate(call).unwrap();
    b.validate(call).unwrap();
}

#[test]
fn event_checkpoint_rejects_parent_history_width_owner_and_future_tamper() {
    let mut c = cursor();
    let mut h = Port {
        handle: 19,
        ..Default::default()
    };
    let cut = finish(&mut c, &mut h);
    let mut changed = cut.clone();
    changed.parent.data[500] ^= 1;
    assert!(changed.validate().is_err());
    let mut changed = cut.clone();
    changed.history[0].owner += 1;
    assert!(changed.validate().is_err());
    let mut changed = cut.clone();
    changed.history[1].instruction = 0x6017fc;
    assert!(changed.validate().is_err());
    let mut changed = cut.clone();
    changed.history[0].after.push(0);
    assert!(changed.validate().is_err());
    let mut changed = cut.clone();
    changed.history[1].child_return = true;
    assert!(changed.validate().is_err());
    let mut c = cursor();
    let mut hold = Port {
        hold: true,
        ..Default::default()
    };
    c.event_step(&mut hold).unwrap_err();
    let mut future = c.event_checkpoint().unwrap().unwrap();
    future.history[2].after[0] = 1;
    assert!(future.validate().is_err());
}

#[test]
fn event_history_reversal_recovers_the_complete_prefix_preimage() {
    for handle in [0, 0x0800_0001] {
        let mut c = cursor();
        let prefix = c.checkpoint().unwrap();
        let mut h = Port {
            handle,
            ..Default::default()
        };
        let cut = finish(&mut c, &mut h);
        let mut reversed = cut.parent().clone();
        for write in cut.writes().iter().rev() {
            let span = &mut reversed.data[write.offset..write.offset + 4];
            assert_eq!(&*span, write.after.as_slice());
            span.copy_from_slice(&write.before);
        }
        assert_eq!(&reversed, prefix.parent());
    }
}
