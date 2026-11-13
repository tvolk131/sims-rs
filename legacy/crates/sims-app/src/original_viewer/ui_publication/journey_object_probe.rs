use super::*;
use sims_vm::action::Action;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct JourneyObjectIdentity {
    pub lifetime: Option<u32>,
    pub person: bool,
    pub loaded_interaction_table_count: Option<usize>,
}

fn loaded_interaction_table_count(
    object: &sims_vm::Object,
    program: &sims_vm::Program,
) -> Option<usize> {
    let definition = object.definition.as_ref()?;
    let module = program.modules.get(&definition.source)?;
    module
        .object_interactions
        .get(&definition.object)
        .map(Vec::len)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct JourneyObjectInput {
    pub tick: u64,
    pub actor: (i16, u32),
    pub clicked: (i16, u32),
    pub resolved: (i16, u32),
    pub ready: bool,
    pub actions: Vec<Action>,
    pub action_targets: BTreeMap<i16, u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct JourneyObjectProgress {
    pub thread: Option<u64>,
    pub status: Option<&'static str>,
    pub routine: Option<u16>,
    pub pc: Option<usize>,
    pub depth: usize,
    pub continuation: Option<u32>,
    pub frame_caller: Option<(i16, Option<u32>)>,
    pub frame_target: Option<(i16, Option<u32>)>,
    pub wake_tick: Option<u64>,
    pub wait_request: Option<u64>,
    pub wait_target: Option<(i16, Option<u32>)>,
    pub route_request: Option<u64>,
    pub route_caller: Option<(i16, Option<u32>)>,
    pub route_target: Option<(i16, Option<u32>)>,
    pub route_next: Option<usize>,
    pub route_points: Option<usize>,
    pub route_started_tick: Option<u64>,
    pub route_last_tick: Option<u64>,
    pub service_pending: bool,
    pub callback_pending: bool,
    pub dispatch_action: Option<u32>,
    pub scheduled_visit: Option<(u64, Option<i16>)>,
}

impl JourneyObjectProgress {
    pub(crate) fn capture(
        vm: &sims_vm::Vm,
        actor: i16,
        route: Option<&sims_sim::Route>,
        scheduled_visit: Option<(u64, Option<i16>)>,
    ) -> Self {
        let handle = |object| {
            (object, vm.world.scheduler_storage.as_ref().and_then(|storage| storage.handle(object)))
        };
        let thread_id = vm.object_threads.get(&actor).copied();
        let thread = thread_id.and_then(|id| vm.threads.get(&id));
        let frame = thread.and_then(|thread| thread.stack.last());
        let mut progress = Self {
            thread: thread_id,
            routine: frame.map(|frame| frame.location.bhav),
            pc: frame.map(|frame| frame.location.instruction),
            depth: thread.map_or(0, |thread| thread.stack.len()),
            continuation: frame.map(|frame| frame.continuation),
            frame_caller: frame.map(|frame| handle(frame.caller)),
            frame_target: frame.and_then(|frame| frame.action_target).map(handle),
            service_pending: thread.is_some_and(|thread| thread.service.is_some()),
            callback_pending: thread.is_some_and(|thread| thread.callback.is_some()),
            dispatch_action: thread.and_then(|thread| thread.dispatch.as_ref()).map(|dispatch| dispatch.action.id),
            scheduled_visit,
            ..Self::default()
        };
        if let Some(thread) = thread {
            use sims_vm::Status;
            progress.status = Some(match &thread.status {
                Status::Ready => "ready",
                Status::Sleeping { wake_tick } => {
                    progress.wake_tick = Some(*wake_tick);
                    "sleeping"
                }
                Status::NativeSuspended(_) => "native_suspended",
                Status::AwaitingRoute(request) => {
                    progress.wait_request = Some(request.id);
                    progress.wait_target = Some(handle(request.target));
                    "awaiting_route"
                }
                Status::AwaitingSnap(request) => {
                    progress.wait_request = Some(request.route.id);
                    progress.wait_target = Some(handle(request.route.target));
                    "awaiting_snap"
                }
                Status::AwaitingAnimation(_) => "awaiting_animation",
                Status::AwaitingAccessory { .. } => "awaiting_accessory",
                Status::AwaitingTurn(_) => "awaiting_turn",
                Status::AwaitingReach(_) => "awaiting_reach",
                Status::AwaitingCreation(_) => "awaiting_creation",
                Status::AwaitingImmediate(_) => "awaiting_immediate",
                Status::AwaitingTransfer(_) => "awaiting_transfer",
                Status::AwaitingPlacement(_) => "awaiting_placement",
                Status::AwaitingRemoval(_) => "awaiting_removal",
                Status::AwaitingBurn(_) => "awaiting_burn",
                Status::AwaitingDialog(_) => "awaiting_dialog",
                Status::AwaitingUiEffect(_) => "awaiting_ui_effect",
                Status::Returned(_) => "returned",
                Status::Faulted(_) => "faulted",
                Status::RegistrySuspended(_) => "registry_suspended",
                Status::NativeRegistryReentry(_) => "native_registry_reentry",
            });
        }
        if let Some(route) = route {
            progress.route_request = Some(route.request);
            progress.route_caller = Some(handle(route.caller));
            progress.route_target = Some(handle(route.target));
            progress.route_next = Some(route.next);
            progress.route_points = Some(route.path.len());
            progress.route_started_tick = Some(route.started_tick);
            progress.route_last_tick = Some(route.last_tick);
        }
        progress
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct JourneyObject {
    #[serde(skip_serializing)]
    pub progress: Option<Box<JourneyObjectProgress>>,
    pub objects: BTreeMap<i16, JourneyObjectIdentity>,
    pub input: Option<JourneyObjectInput>,
    pub ground_input: bool,
    pub pointer_pending: bool,
}

impl JourneyObject {
    pub(crate) fn handle(&self, object: i16) -> Result<(i16, u32)> {
        Ok((
            object,
            self.objects
                .get(&object)
                .and_then(|identity| identity.lifetime)
                .context("object publication lifetime unavailable")?,
        ))
    }

    pub(super) fn capture(
        app: &Viewer,
        targets: &BTreeMap<i16, std::result::Result<JourneyTarget, String>>,
    ) -> Result<Self> {
        let session = app
            .session
            .as_ref()
            .context("object publication Session absent")?;
        let world = &session.simulation.vm.world;
        let storage = world
            .scheduler_storage
            .as_ref()
            .context("object publication scheduler storage absent")?;
        let actor = session
            .simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?;
        let mut ids = BTreeSet::new();
        if let Some(scene) = &app.scene {
            ids.extend(
                scene
                    .draws
                    .iter()
                    .filter_map(|draw| draw.object)
                    .filter(|object| *object > 0),
            );
        }
        ids.extend(
            targets
                .values()
                .filter_map(|target| match target.as_ref().ok()? {
                    JourneyTarget::Person(object)
                    | JourneyTarget::Object { target: object, .. } => Some(*object),
                }),
        );
        ids.extend(actor);
        ids.extend(world.ui_owner.owner);
        if let Some(selected) = actor.and_then(|actor| world.actions.as_ref()?.actors.get(&actor)) {
            ids.extend(
                selected
                    .active
                    .iter()
                    .chain(selected.queue.pending.iter())
                    .chain(selected.previous.iter())
                    .map(|action| action.target),
            );
        }
        #[derive(serde::Deserialize)]
        struct Metadata {
            tick: u64,
            clicked: i16,
        }
        let metadata = session
            .host
            .presentation
            .object_input
            .as_ref()
            .map(|input| serde_json::from_value::<Metadata>(serde_json::to_value(input)?))
            .transpose()?;
        if let (Some(input), Some(metadata)) = (&session.host.presentation.object_input, &metadata)
        {
            ids.extend([input.actor(), metadata.clicked, input.target()]);
            ids.extend(input.actions().iter().map(|action| action.target));
        }
        let objects = ids
            .into_iter()
            .filter_map(|object| {
                world.objects.get(&object).map(|state| {
                    (
                        object,
                        JourneyObjectIdentity {
                            lifetime: storage.handle(object),
                            person: state.person.is_some(),
                            loaded_interaction_table_count: loaded_interaction_table_count(
                                state,
                                &session.program,
                            ),
                        },
                    )
                })
            })
            .collect();
        let mut snapshot = Self {
            progress: None,
            objects,
            input: None,
            ground_input: session.host.presentation.ground_input.is_some(),
            pointer_pending: app.viewport_enter_requested
                || !app.move_inputs.is_empty()
                || app.pan != [0, 0],
        };
        if let (Some(input), Some(metadata)) = (&session.host.presentation.object_input, metadata) {
            snapshot.input = Some(JourneyObjectInput {
                tick: metadata.tick,
                actor: snapshot.handle(input.actor())?,
                clicked: snapshot.handle(metadata.clicked)?,
                resolved: snapshot.handle(input.target())?,
                ready: input.ready(),
                actions: input.actions().to_vec(),
                action_targets: input
                    .actions()
                    .iter()
                    .map(|action| {
                        snapshot
                            .handle(action.target)
                            .map(|(object, lifetime)| (object, lifetime))
                    })
                    .collect::<Result<_>>()?,
            });
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_vm::{Definition, Module, Object, Program, program::MenuEntry};

    #[test]
    fn tutorial_object_loaded_table_count_preserves_missing_and_empty() {
        let mut object = Object::default();
        let mut program = Program::default();
        assert_eq!(loaded_interaction_table_count(&object, &program), None);
        object.definition = Some(Definition {
            source: "synthetic-loaded-table".into(),
            object: 7,
            guid: 0,
            slots: 0,
        });
        assert_eq!(loaded_interaction_table_count(&object, &program), None);
        let mut module = Module::default();
        module
            .interaction_tables
            .insert(7, vec![MenuEntry::default()]);
        module
            .object_interactions
            .insert(8, vec![MenuEntry::default()]);
        program
            .modules
            .insert("synthetic-loaded-table".into(), module);
        assert_eq!(loaded_interaction_table_count(&object, &program), None);
        program
            .modules
            .get_mut("synthetic-loaded-table")
            .unwrap()
            .object_interactions
            .insert(7, Vec::new());
        assert_eq!(loaded_interaction_table_count(&object, &program), Some(0));
        program
            .modules
            .get_mut("synthetic-loaded-table")
            .unwrap()
            .object_interactions
            .get_mut(&7)
            .unwrap()
            .extend([MenuEntry::default(), MenuEntry::default()]);
        assert_eq!(loaded_interaction_table_count(&object, &program), Some(2));
        for count in [None, Some(0), Some(2)] {
            let identity = JourneyObjectIdentity {
                lifetime: None,
                person: false,
                loaded_interaction_table_count: count,
            };
            assert_eq!(
                serde_json::to_value(identity).unwrap()["loaded_interaction_table_count"],
                serde_json::to_value(count).unwrap()
            );
        }
    }
    #[test]
    fn tutorial_object_progress_snapshot_copies_vm_wait_frame_and_route_scalars() {
        let mut vm = sims_vm::Vm::new(sims_vm::World::default(), 1);
        let mut thread: sims_vm::Thread = serde_json::from_value(serde_json::json!({
            "service":null,"dispatch":null,"autonomy":null,"find_best":[],
            "completed_actions":[],"last_result":false,"callback":null,"callback_result":null,
            "stack":[{"continuation":3,"action_target":230,"action_icon":false,
                "code_owner":"synthetic","location":{"source":"synthetic","bhav":4100,"instruction":8},
                "caller":105,"callee":230,"stack_object":230,"args":[],"locals":[],
                "opaque_locals":[],"storage_offset":0}],
            "frame_memory":[],"check":false,"interrupted":false,"status":{"Sleeping":{"wake_tick":205}}
        })).unwrap();
        vm.object_threads.insert(105, 7);
        vm.threads.insert(7, thread.clone());
        let before = (vm.tick, vm.rng_state, vm.object_threads[&105], vm.threads.len(),
            vm.threads[&7].stack[0].location.instruction);
        let sleeping = JourneyObjectProgress::capture(&vm, 105, None, Some((7, Some(105))));
        assert_eq!(sleeping.thread, Some(7));
        assert_eq!(sleeping.status, Some("sleeping"));
        assert_eq!(sleeping.wake_tick, Some(205));
        assert_eq!((sleeping.routine, sleeping.pc, sleeping.continuation), (Some(4100), Some(8), Some(3)));
        assert_eq!(sleeping.frame_target, Some((230, None)));
        assert_eq!(before, (vm.tick, vm.rng_state, vm.object_threads[&105], vm.threads.len(),
            vm.threads[&7].stack[0].location.instruction));
        assert!(matches!(&vm.threads[&7].status, sims_vm::Status::Sleeping { wake_tick: 205 }));
        thread.status = sims_vm::Status::AwaitingRoute(sims_vm::RouteRequest {
            id: 9, origin: thread.stack[0].location.clone(), caller: 105, target: 230,
            destination: sims_vm::routing::Destination::Front, no_failure_trees: false,
        });
        vm.threads.insert(7, thread);
        let route = sims_sim::Route {
            request: 9, caller: 105, target: 230,
            target_body: sims_sim::Body { level: 1, position: sims_sim::Point::new(32, 32),
                facing: 1, radius: 1, elevation: 0 },
            goal: sims_sim::Destination { position: sims_sim::Point::new(16, 16), facing: 1, entry_flags: 0 },
            path: vec![sims_sim::Point::new(0, 0), sims_sim::Point::new(16, 16)],
            original_path: None, next: 1, started_tick: 200, last_tick: 201,
            locomotion: sims_sim::locomotion::Locomotion {
                initial_facing: 1, distances: vec![], phases: vec![], elapsed: 0,
            },
        };
        let moving = JourneyObjectProgress::capture(&vm, 105, Some(&route), None);
        assert_eq!(moving.status, Some("awaiting_route"));
        assert_eq!(moving.wait_request, Some(9));
        assert_eq!(moving.wait_target, Some((230, None)));
        assert_eq!(moving.route_request, Some(9));
        assert_eq!((moving.route_next, moving.route_points), (Some(1), Some(2)));
        assert_eq!((moving.route_started_tick, moving.route_last_tick), (Some(200), Some(201)));
        assert_eq!(moving.wake_tick, None);
        assert_eq!(JourneyObjectProgress::capture(&vm, 999, None, None).thread, None);
    }

}
