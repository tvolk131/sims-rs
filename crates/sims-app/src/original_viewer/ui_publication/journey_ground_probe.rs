use super::*;
use sims_vm::action::Action;

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct JourneyGroundPixel {
    pub pixel: [f32; 2],
    pub picked: Option<sims_vm::ObjectId>,
    pub point: Option<[i32; 2]>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct JourneyGroundInput {
    pub tick: u64,
    pub actor: sims_vm::ObjectId,
    pub point: [i32; 2],
    pub floor: i32,
    pub target: Option<sims_vm::ObjectId>,
    pub target_lifetime: Option<u32>,
    pub ready: bool,
    pub complete: bool,
    pub actions: Vec<Action>,
}

#[derive(Clone)]
pub(crate) struct JourneyGround {
    pub scene: Arc<Scene>,
    pub display_size: [u32; 2],
    pub widget_epoch: u64,
    pub tick: u64,
    pub actor: sims_vm::ObjectId,
    pub actor_lifetime: u32,
    pub floor: i32,
    pub camera: sims_sim::pose::portrait::view::Input,
    pub lot: Arc<sims_sim::lot_geometry::Lot>,
    pub lot_size: usize,
    pub input: Option<JourneyGroundInput>,
    pub object_input: bool,
    pub serial: u32,
    pub queue: Vec<Action>,
    pub queue_targets: BTreeMap<sims_vm::ObjectId, Option<u32>>,
}

impl JourneyGround {
    pub(crate) fn sample(&self, pixel: [f32; 2]) -> Result<JourneyGroundPixel> {
        let camera = sims_render::Camera::from_native(self.camera, self.floor)?;
        let result = camera.pick_original_ground_in(&self.lot, self.lot_size, pixel);
        Ok(JourneyGroundPixel {
            pixel,
            picked: self.scene.pick(pixel),
            point: result.as_ref().ok().copied().flatten(),
            error: result.err().map(|error| format!("{error:#}")),
        })
    }

    pub(super) fn capture(app: &Viewer) -> Result<Self> {
        let session = app
            .session
            .as_ref()
            .context("ground probe session absent")?;
        let scene = app.scene.as_ref().context("ground probe scene absent")?;
        let camera = OriginalHouse::native_camera(session)?;
        let retained = session.retained_camera_input()?;
        let retained_camera = sims_render::Camera::from_native(retained, camera.level)?;
        ensure!(
            [scene.width, scene.height] == [camera.width, camera.height]
                && retained.viewport == [0, 0, camera.width, camera.height]
                && retained_camera.center == camera.center
                && retained_camera.zoom == camera.zoom
                && retained_camera.rotation == camera.rotation,
            "ground probe scene/native/retained viewport mismatch"
        );
        let actor = session
            .simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?
            .context("ground probe native selected actor absent")?;
        let storage = session
            .simulation
            .vm
            .world
            .scheduler_storage
            .as_ref()
            .context("ground probe lifetime storage absent")?;
        let actor_lifetime = storage
            .handle(actor)
            .context("ground probe actor lifetime absent")?;
        let lot = session
            .simulation
            .architecture
            .as_ref()
            .and_then(|architecture| architecture.original.as_ref())
            .context("ground probe original architecture absent")?;
        let lot_size = sims_render::original_lot_size(&session.simulation)?;
        let input = session
            .host
            .presentation
            .ground_input
            .as_ref()
            .map(|input| -> Result<_> {
                #[derive(serde::Deserialize)]
                struct Coordinates {
                    tick: u64,
                    point: [i32; 2],
                    floor: i32,
                }
                let coordinates: Coordinates =
                    serde_json::from_value(serde_json::to_value(input)?)?;
                let target_lifetime = input
                    .target()
                    .map(|target| {
                        storage
                            .handle(target)
                            .context("ground target lifetime absent")
                    })
                    .transpose()?;
                Ok(JourneyGroundInput {
                    tick: coordinates.tick,
                    actor: input.actor(),
                    point: coordinates.point,
                    floor: coordinates.floor,
                    target: input.target(),
                    target_lifetime,
                    ready: input.ready(),
                    complete: input.complete(),
                    actions: input.actions().to_vec(),
                })
            })
            .transpose()?;
        let actions = session
            .simulation
            .vm
            .world
            .actions
            .as_ref()
            .context("ground probe native actions absent")?;
        let selected = actions
            .actors
            .get(&actor)
            .context("ground probe actor actions absent")?;
        Ok(Self {
            scene: scene.clone(),
            display_size: app.display_size,
            widget_epoch: app.widget_epoch,
            tick: session.simulation.vm.tick,
            actor,
            actor_lifetime,
            floor: camera.level,
            camera: retained,
            lot: Arc::new(lot.clone()),
            lot_size,
            input,
            object_input: session.host.presentation.object_input.is_some(),
            serial: actions.serial,
            queue: selected
                .active
                .iter()
                .chain(selected.queue.pending.iter())
                .cloned()
                .collect(),
            queue_targets: selected
                .active
                .iter()
                .chain(selected.queue.pending.iter())
                .map(|action| (action.target, storage.handle(action.target)))
                .collect(),
        })
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct JourneyWalkingTarget {
    pub lifetime: Option<u32>,
    pub position: Option<sims_vm::autonomy::Placement>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct JourneyWalking {
    pub tick: u64,
    pub actor: sims_vm::ObjectId,
    pub actor_lifetime: Option<u32>,
    pub position: Option<sims_vm::autonomy::Placement>,
    pub owner: Option<sims_vm::ObjectId>,
    pub owner_lifetime: Option<u32>,
    pub owner_source: Option<String>,
    pub children_pending: bool,
    pub clock_pending: bool,
    pub input_pending: bool,
    pub serial: u32,
    pub active: Option<Action>,
    pub pending: Vec<Action>,
    pub previous: Option<Action>,
    pub targets: BTreeMap<sims_vm::ObjectId, JourneyWalkingTarget>,
    pub dialog_statuses: BTreeMap<u64, u32>,
}

impl JourneyWalking {
    pub(super) fn capture(app: &Viewer) -> Result<Self> {
        let session = app.session.as_ref().context("walking Session absent")?;
        let world = &session.simulation.vm.world;
        let actor = session
            .simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?
            .context("walking native actor absent")?;
        let storage = world
            .scheduler_storage
            .as_ref()
            .context("walking scheduler lifetime storage absent")?;
        let actions = world.actions.as_ref().context("walking actions absent")?;
        let selected = actions
            .actors
            .get(&actor)
            .context("walking actor actions absent")?;
        let owner = world.ui_owner.owner;
        Ok(Self {
            tick: session.simulation.vm.tick,
            actor,
            actor_lifetime: storage.handle(actor),
            position: world.positions.get(&actor).copied(),
            owner,
            owner_lifetime: owner.and_then(|object| storage.handle(object)),
            owner_source: owner
                .and_then(|object| world.objects.get(&object))
                .and_then(|object| object.definition.as_ref())
                .map(|definition| definition.source.clone()),
            children_pending: app.presentation_children_pending(),
            clock_pending: session.clock_pending(),
            input_pending: app.viewport_enter_requested
                || !app.move_inputs.is_empty()
                || app.pan != [0, 0]
                || session.menu_command_pending()
                || session.host.presentation.ground_input.is_some()
                || session.host.presentation.object_input.is_some(),
            serial: actions.serial,
            active: selected.active.clone(),
            pending: selected.queue.pending.iter().cloned().collect(),
            previous: selected.previous.clone(),
            targets: selected
                .active
                .iter()
                .chain(selected.queue.pending.iter())
                .chain(selected.previous.iter())
                .map(|action| {
                    (
                        action.target,
                        JourneyWalkingTarget {
                            lifetime: storage.handle(action.target),
                            position: world.positions.get(&action.target).copied(),
                        },
                    )
                })
                .collect(),
            dialog_statuses: world
                .dialogs
                .records
                .iter()
                .map(|record| (record.id, record.status))
                .collect(),
        })
    }
}
