use super::*;
use serde_json::json;
use sims_sim::lot_selection::{Notification, NotificationOwner, NotificationViewRefresh};
use sims_vm::commander::{Registry, house};
use std::collections::BTreeMap;

fn callback(event: u32, payload: u32, budget: usize) -> (notifications::State, dispatch::Service) {
    let mut registry = Registry::default();
    registry.construct(8).unwrap();
    registry.bind(8, Receiver::Interface).unwrap();
    let mut dispatcher = dispatch::Dispatch::begin(
        registry,
        house::Fields {
            viewer_interface: true,
            include_visible: 0,
            renderer_dirty: 0,
            ui: house::Interface {
                mask_owner_present: false,
                panel_active: false,
                shared_present: false,
                gated: 0,
                creation_gated: 0,
                dirty: 0,
            },
        },
        event,
        payload,
    )
    .unwrap();
    for _ in 0..32 {
        match dispatcher
            .advance(&mut BTreeMap::new(), true, budget, |_, _, _, _| {})
            .unwrap()
        {
            dispatch::Progress::Pending => (),
            dispatch::Progress::Service(caller) if caller.request == Listener::UiRefresh => {
                let state = serde_json::from_value(json!({
                    "dispatch": dispatcher,
                    "viewer": null,
                    "pending": notifications::Service::Listener(caller.clone()),
                    "selection": Notification {
                        owner: NotificationOwner::Selection { target: Some(105) },
                        payload: payload as i32,
                    },
                }))
                .unwrap();
                return (state, caller);
            }
            dispatch::Progress::Service(caller) => dispatcher.acknowledge(&caller).unwrap(),
            other => panic!("notification returned before Interface UiRefresh: {other:?}"),
        }
    }
    panic!("notification did not reach Interface UiRefresh")
}

fn token() -> NotificationViewRefresh {
    serde_json::from_value(json!({
        "notification": Notification {
            owner: NotificationOwner::Selection { target: Some(105) },
            payload: 105,
        },
        "tick": 193002,
        "caller_serial": 2,
    }))
    .unwrap()
}

#[test]
fn selection_notification_property_caller_slices_and_retains_dispatch_and_token() {
    let mut results = vec![];
    for budget in [1, 1000] {
        let (notifications, caller) = callback(sims_vm::selection::SIGNAL, 105, budget);
        let execution = Execution {
            tick: 193002,
            caller,
            native: native::Call::new(10001, true).unwrap(),
            pause: None,
            people: None,
            panels: false,
            funds_started: false,
            selection_view_refresh: Some(token()),
        };
        let bytes = serde_json::to_vec(&(notifications.clone(), execution.clone())).unwrap();
        let (restored, retained): (notifications::State, Execution) =
            serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored, notifications);
        assert_eq!(retained, execution);
        validate_selection_view_refresh_caller(
            &restored,
            &retained.caller,
            retained.selection_view_refresh.unwrap().notification(),
        )
        .unwrap();
        results.push((restored, retained));
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn selection_notification_property_caller_rejects_unrelated_and_mismatched_dispatch() {
    let (notifications, caller) = callback(sims_vm::selection::SIGNAL, 105, 1);
    for mismatch in ["recipient", "handler", "request", "event", "value"] {
        let mut wrong = caller.clone();
        match mismatch {
            "recipient" => wrong.invocation.recipient = 7,
            "handler" => wrong.invocation.handler = Receiver::Renderer,
            "request" => wrong.request = Listener::PanelRefresh,
            "event" => wrong.invocation.event = 0xda,
            "value" => wrong.invocation.value = 106,
            _ => unreachable!(),
        }
        assert!(
            validate_selection_view_refresh_caller(&notifications, &wrong, token().notification())
                .is_err()
        );
    }
    for (event, payload) in [(0xda, 105), (sims_vm::selection::SIGNAL, 106)] {
        let (unrelated, other) = callback(event, payload, 1);
        assert!(
            validate_selection_view_refresh_caller(&unrelated, &other, token().notification())
                .is_err()
        );
    }
    let mut value = serde_json::to_value(&notifications).unwrap();
    value["pending"] = serde_json::Value::Null;
    let absent: notifications::State = serde_json::from_value(value).unwrap();
    assert!(
        validate_selection_view_refresh_caller(&absent, &caller, token().notification()).is_err()
    );
    let mut stale = notifications;
    stale.dispatch.acknowledge(&caller).unwrap();
    assert!(
        validate_selection_view_refresh_caller(&stale, &caller, token().notification()).is_err()
    );
}

#[test]
fn selection_notification_property_caller_preserves_tokenless_deletion_without_refresh_authority() {
    let (notifications, caller) = callback(sims_vm::selection::SIGNAL, u32::MAX, 1);
    let notification = Notification {
        owner: NotificationOwner::Deletion {
            object: 105,
            depth: 1,
        },
        payload: -1,
    };
    let mut value = serde_json::to_value(&notifications).unwrap();
    value["selection"] = serde_json::to_value(notification).unwrap();
    let notifications: notifications::State = serde_json::from_value(value).unwrap();
    validate_tokenless_selection_notification(Some((notification, true)), &notifications, &caller)
        .unwrap();
    let execution = Execution {
        tick: 193002,
        caller: caller.clone(),
        native: native::Call::new(10001, true).unwrap(),
        pause: None,
        people: None,
        panels: false,
        funds_started: false,
        selection_view_refresh: None,
    };
    let bytes = serde_json::to_vec(&(notifications, execution)).unwrap();
    let (restored, retained): (notifications::State, Execution) =
        serde_json::from_slice(&bytes).unwrap();
    assert!(retained.selection_view_refresh.is_none());
    validate_tokenless_selection_notification(
        Some((notification, true)),
        &restored,
        &retained.caller,
    )
    .unwrap();
    for pending in [
        None,
        Some((notification, false)),
        Some((token().notification(), true)),
    ] {
        assert!(validate_tokenless_selection_notification(pending, &restored, &caller).is_err());
    }
    let mut stale = restored;
    stale.dispatch.acknowledge(&caller).unwrap();
    assert!(
        validate_tokenless_selection_notification(Some((notification, true)), &stale, &caller)
            .is_err()
    );
}
