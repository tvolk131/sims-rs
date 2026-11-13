use serde_json::{Value, json};
use sims_vm::{ObjectId, Vm, action::Actor, dialog::Record};

pub(super) fn enable(simulation: &mut sims_sim::Simulation) {
    if super::super::support::window_session::enabled() {
        simulation.enable_terminal_observations(4096);
    }
}

pub(super) fn terminals(simulation: &sims_sim::Simulation, resident: ObjectId) -> Value {
    use sims_vm::observations::ActionTerminalReason;
    let actions = simulation.vm.terminal_observations();
    let routes = simulation.route_terminal_observations();
    let action_events = actions
        .events()
        .iter()
        .filter(|event| event.actor == resident)
        .map(|event| {
            json!({
                "actor":event.actor,"serial":event.serial,"target":event.target,
                "thread":event.thread,"invocation":event.invocation,"tick":event.tick,
                "reason":match event.reason {
                    ActionTerminalReason::NormalReturn => "normal_return",
                    ActionTerminalReason::Reset => "reset",
                    ActionTerminalReason::Replacement => "replacement",
                    ActionTerminalReason::Unqualified => "unqualified",
                },
                "cancelled":event.cancelled,"result":event.result
            })
        })
        .collect::<Vec<_>>();
    let route_events = routes
        .events()
        .iter()
        .filter(|event| event.actor == resident)
        .map(|event| {
            json!({
                "actor":event.actor,"target":event.target,"request":event.request,
                "action_serial":event.action_serial,"action_invocation":event.action_invocation,
                "thread":event.thread,"tick":event.tick,"success":event.success,
                "native_result":event.native_result,"endpoint":event.endpoint
            })
        })
        .collect::<Vec<_>>();
    json!({"enabled":actions.enabled() && routes.enabled(),
        "dropped":actions.dropped().saturating_add(routes.dropped()),
        "action":action_events.last(),"route":route_events.last(),
        "actions":action_events,"routes":route_events})
}

pub(super) fn dialog(record: &Record) -> Value {
    json!({
        "id":record.id,"caller":record.caller,"source":record.source,
        "origin":record.origin,"status":record.status,
        "offered_results":record.buttons.iter().map(|button| button.result).collect::<Vec<_>>(),
        "enter_result":record.key_result(13),
        "escape_result":record.key_result(27),
        "space_result":record.key_result(32)
    })
}

pub(super) fn answer_dialog(
    vm: &mut Vm,
    id: u64,
    result: u32,
    recording: bool,
) -> Result<Option<Value>, String> {
    vm.answer_dialog(id, result)?;
    Ok(recording.then(|| {
        let record = vm
            .world
            .dialogs
            .records
            .iter()
            .find(|record| record.id == id)
            .unwrap();
        json!({"id":id,"result":result,"status":record.status,"tick":vm.tick,
            "caller":record.caller,"source":record.source,"origin":record.origin})
    }))
}

pub(super) fn actions(actor: Option<&Actor>) -> Value {
    json!({
        "active_id":actor.and_then(|actor| actor.active.as_ref()).map(|action| action.id),
        "pending_ids":actor.map(|actor| actor.queue.pending.iter().map(|action| action.id).collect::<Vec<_>>()).unwrap_or_default(),
        "active_cancelled":actor.and_then(|actor| actor.active.as_ref()).is_some_and(|action| action.flags & sims_vm::action::CANCELLED != 0),
        "previous_id":actor.and_then(|actor| actor.previous.as_ref()).map(|action| action.id),
        "previous_cancelled":actor.and_then(|actor| actor.previous.as_ref()).is_some_and(|action| action.flags & sims_vm::action::CANCELLED != 0)
    })
}

pub(super) fn work(simulation: &sims_sim::Simulation, resident: ObjectId) -> Value {
    let thread_id = simulation.vm.object_threads.get(&resident).copied();
    let thread = thread_id.and_then(|id| simulation.vm.threads.get(&id));
    let native = simulation.native_lot.as_ref();
    let owns = |id| thread_id == Some(id);
    let native_route = native
        .and_then(|native| native.route.as_ref())
        .filter(|route| route.caller == resident)
        .map(|route| {
            json!({"request_id":route.request,"thread":route.thread,
            "caller":route.caller,"target":route.target,"failed":route.failed})
        });
    json!({
        "thread_id":thread_id,
        "thread_status":thread.map(|thread| &thread.status),
        "action_use_targets":thread.map(|thread| thread.stack.iter().filter_map(|frame| frame.action_target).collect::<Vec<_>>()).unwrap_or_default(),
        "completed_action_targets":thread.map(|thread| thread.completed_actions.iter().map(|record| record.1).collect::<Vec<_>>()).unwrap_or_default(),
        "route":thread_id.is_some_and(|id| simulation.routes.contains_key(&id)),
        "route_control_count":simulation.route_control.values().filter(|control| control.caller == resident).count(),
        "native_route":native_route,
        "native_movement":native.and_then(|native| native.movement.as_ref()).is_some_and(|movement| owns(movement.thread)),
        "native_completion":native.and_then(|native| native.completion.as_ref()).is_some_and(|completion| owns(completion.thread)),
        "native_shoo":native.and_then(|native| native.shoo.as_ref()).is_some_and(|shoo| owns(shoo.thread)),
        "cancellation_callback":simulation.cancellation.is_some(),
        "pending_cancellation_callbacks":simulation.vm.world.actions.as_ref().map_or(0, |actions| actions.callbacks.len())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Record {
        serde_json::from_value(json!({
            "id":7,"caller":12,"source":"synthetic","origin":{"source":"synthetic","bhav":4096,"instruction":0},
            "operands":[0,0,0,0,0,0,0,0],"stack_object":13,
            "title":"authored title","body":"authored body",
            "buttons":[{"label":"authored choice","result":2}],
            "icon_object":null,"job_icon":null,"bitmap_icon":null,"status":1
        })).unwrap()
    }

    #[test]
    fn dialog_observations_publish_identity_and_offered_keys_without_prose() {
        let observed = dialog(&record());
        assert_eq!(observed["id"], 7);
        assert_eq!(observed["offered_results"], json!([2]));
        assert_eq!(observed["enter_result"], 2);
        assert!(observed.get("title").is_none());
        assert!(observed.get("body").is_none());
        assert!(!observed.to_string().contains("authored"));
    }

    #[test]
    fn absent_actions_are_nullable_without_manufacturing_a_terminal_result() {
        let observed = actions(None);
        assert!(observed["active_id"].is_null());
        assert!(observed["previous_id"].is_null());
        assert_eq!(observed["pending_ids"], json!([]));
        assert_eq!(observed["active_cancelled"], false);
        assert!(observed.get("terminal").is_none());
    }

    #[test]
    fn dialog_answer_is_observed_only_after_an_offered_current_response_succeeds() {
        let mut vm = Vm::new(Default::default(), 1);
        vm.tick = 99;
        vm.world.dialogs.records.push(record());
        vm.world.dialogs.modal = Some(7);
        let before = vm.world.clone();
        assert!(answer_dialog(&mut vm, 7, 3, true).is_err());
        assert!(answer_dialog(&mut vm, 8, 2, true).is_err());
        assert_eq!(vm.world, before);
        vm.world.dialogs.modal = Some(8);
        assert!(answer_dialog(&mut vm, 7, 2, true).is_err());
        vm.world.dialogs.modal = Some(7);
        let mut unobserved = vm.clone();
        let answer = answer_dialog(&mut vm, 7, 2, true).unwrap().unwrap();
        assert!(
            answer_dialog(&mut unobserved, 7, 2, false)
                .unwrap()
                .is_none()
        );
        assert_eq!(vm.world, unobserved.world);
        assert_eq!(answer["id"], 7);
        assert_eq!(answer["result"], 2);
        assert_eq!(answer["tick"], 99);
        assert_eq!(answer["status"], vm.world.dialogs.records[0].status);
        assert!(vm.world.dialogs.modal.is_none());
        assert!(answer_dialog(&mut vm, 7, 2, true).is_err());
    }
}
