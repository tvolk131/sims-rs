use super::super::super::super::super::super::ui_publication::{
    JourneyObject, JourneyObjectInput, JourneyObjectProgress, JourneyTarget,
};
use super::*;
use std::collections::BTreeSet;

const ENTRY_BODY: &str = "2b02447b1c43b5b9406855b68ea932030e22c61d46f20ba3d1764151c189c39e";
const BOUNDARY: &str = "object stopped at first new instruction/modal without responding";
const WALL_SECONDS: f64 = 10.0;
const TICK_LIMIT: u64 = 256;

fn object(publication: &Published) -> Result<&JourneyObject> {
    publication
        .journey_object
        .as_ref()
        .context("object snapshot absent")?
        .as_ref()
        .map_err(|error| anyhow::anyhow!("object snapshot: {error}"))
}

fn entry_pins(prior: &Record, record: &Record, digest: &str) -> Result<()> {
    ensure!(
        record.id > prior.id
            && record.caller == prior.caller
            && record.caller > 0
            && record.source == SOURCE
            && record.origin.source == SOURCE
            && record.origin.bhav == 4125
            && record.origin.instruction == 7
            && record.operands == [0, 2, 17, 0, 0, 4, 0, 135]
            && record.status == 1
            && !record.modal()
            && record.stack_object == BOB_NEIGHBOR
            && record.title == "Tutorial"
            && digest == ENTRY_BODY
            && record.bitmap_icon == Some(5002)
            && record.icon_object.is_none()
            && record.job_icon.is_none()
            && record.global_bitmap_icon.is_none()
            && record.portrait.is_none()
            && record.presentation
                == Some(sims_vm::dialog::presentation::Presentation {
                    anchored: true,
                    hide_title: true,
                    large: false,
                })
            && record.buttons.len() == 1
            && record.buttons[0].label == "OK"
            && record.buttons[0].result == 2,
        "unsupported object entry: exact live replacement-Host instruction pins unavailable"
    );
    Ok(())
}

fn persistent_native(initial: &walking::Native, current: &walking::Native) -> Result<()> {
    ensure!(
        initial.actor == BOB_OBJECT
            && current.actor == initial.actor
            && initial.actor_lifetime.is_some()
            && current.actor_lifetime == initial.actor_lifetime
            && initial.owner.is_some()
            && current.owner == initial.owner
            && initial.owner_lifetime.is_some()
            && current.owner_lifetime == initial.owner_lifetime
            && current.owner_source.as_deref() == Some(SOURCE)
            && current.position.is_some(),
        "object persistent actor/caller/source/lifetime changed"
    );
    Ok(())
}

fn credit_after_owners<Output>(
    initial: &walking::Native,
    current: &walking::Native,
    credit: impl FnOnce() -> Result<Output>,
) -> Result<Output> {
    persistent_native(initial, current)?;
    credit()
}

fn notification_unchanged(initial: &walking::Native, current: &walking::Native) -> Result<()> {
    persistent_native(initial, current)?;
    ensure!(
        current.tick == initial.tick
            && current.position == initial.position
            && current.serial == initial.serial
            && current.active == initial.active
            && current.pending == initial.pending
            && current.previous == initial.previous,
        "object handoff settle changed tick, position, or prior actions"
    );
    Ok(())
}

fn drained(streams: usize, host_idle: bool) -> Result<bool> {
    if streams != 0 {
        return Ok(false);
    }
    ensure!(
        host_idle,
        "object zero-stream busy Host unsupported; no synthetic wake"
    );
    Ok(true)
}

fn input_gate(streams: usize, idle: bool, children: bool) -> Result<()> {
    ensure!(
        streams == 0 && idle && !children,
        "object input requires zero registered streams, idle Host, and no children"
    );
    Ok(())
}

fn settle_pointer_with<Context>(
    context: &mut Context,
    mut pending: impl FnMut(&mut Context) -> Result<bool>,
    mut service: impl FnMut(&mut Context) -> Result<()>,
) -> Result<()> {
    while pending(context)? {
        service(context)?;
    }
    Ok(())
}

fn redraw_notifications(messages: &[Message], held_popup: bool) -> Result<()> {
    ensure!(
        messages.is_empty()
            || (held_popup
                && messages.iter().all(|message| matches!(
                    message,
                    Message::ViewportEnter | Message::ViewportLeave
                ))),
        "object redraw outside held Object popup phase or emitted non-notification message"
    );
    Ok(())
}

fn redraw_binding(expected: Cut, current: Cut, source: &Value, published: &Value) -> Result<()> {
    expected.require(current)?;
    ensure!(
        source == published,
        "object redraw publication binding changed"
    );
    Ok(())
}

fn bound_redraw_observation<Context>(
    context: &mut Context,
    retain: impl FnOnce(&mut Context),
    validate: impl FnOnce(&mut Context) -> Result<()>,
    observe: impl FnOnce(&mut Context) -> Result<()>,
) -> Result<()> {
    retain(context);
    validate(context)?;
    observe(context)
}

fn menu_identity(menu: Option<&Menu>) -> Value {
    menu.map_or(Value::Null, |menu| {
        let choices =
            |choices: &[super::super::super::super::super::super::ui_publication::Choice]| {
                choices.iter().map(|choice| json!({
                "label":choice.label,
                "intent":choice.intent.as_ref().map(|intent| format!("{:?}", intent.message()))
            })).collect::<Vec<_>>()
            };
        json!({"title":menu.title,"diagnostic_title":menu.diagnostic_title,
            "preview":menu.preview,"ready":menu.ready,"error":menu.error,
            "popup_visible":menu.popup_visible,"popup":choices(&menu.popup),
            "diagnostic":choices(&menu.diagnostic),
            "cancel":menu.cancel.as_ref().map(|intent| format!("{:?}", intent.message()))})
    })
}

fn redraw_identity(publication: &Published) -> Result<Value> {
    Ok(
        json!({"native":walking::native(publication)?,"object":object(publication)?,
        "owner":publication.native_popup,"menu":menu_identity(publication.menu.as_ref()),
        "dialog":publication.dialog,"modal":publication.modal,
        "targets":publication.journey_targets,"hud_height":publication.hud_height,
        "playing":publication.playing,"people_visible":publication.people_visible,
        "resident":publication.resident,"residents":publication.residents.iter()
            .map(|resident| json!({"object":resident.object,"neighbor":resident.neighbor,
                "name":resident.name,"enabled":resident.enabled,"highlighted":resident.highlighted}))
            .collect::<Vec<_>>()}),
    )
}

fn diagnostic_press_proof(proof: &Proof) -> Result<()> {
    ensure!(
        proof.viewport_left_press_sent
            && !proof.viewport_left_release_sent
            && !proof.offer_observed
            && !proof.command_sent
            && !proof.enqueue_accepted
            && !proof.active_seen
            && !proof.native_completion_observed
            && !proof.hover_qualified,
        "object diagnostic popup press cannot receive release/offer/command/enqueue/completion credit"
    );
    Ok(())
}

fn popup_press_identity(
    initial_cut: Cut,
    current_cut: Cut,
    initial: &walking::Native,
    current: &walking::Native,
    snapshot: &JourneyObject,
    owner: &sims_runtime::original_session::live_menu::State,
    menu: &Menu,
    pick: &Pick,
) -> Result<()> {
    initial_cut.require(Cut {
        popup_epoch: initial_cut.popup_epoch,
        ..current_cut
    })?;
    ensure!(
        initial_cut.popup_epoch.checked_add(1) == Some(current_cut.popup_epoch),
        "object popup press is not the first native owner epoch; no menu shopping"
    );
    notification_unchanged(initial, current)?;
    let input = snapshot
        .input
        .as_ref()
        .context("object popup press input absent")?;
    ensure!(
        current.input_pending
            && !current.children_pending
            && !snapshot.pointer_pending
            && !snapshot.ground_input
            && input.tick == current.tick,
        "object popup press pending/input/tick state changed"
    );
    let metadata: PopupOwner = serde_json::from_value(serde_json::to_value(owner)?)?;
    visible_actions(menu, input, &metadata, pick)?;
    for handle in [input.actor, input.clicked, input.resolved] {
        ensure!(
            snapshot.handle(handle.0)? == handle,
            "object popup press actor/clicked/resolved lifetime changed"
        );
    }
    for action in &input.actions {
        let identity = snapshot
            .objects
            .get(&action.target)
            .context("object popup press action-target absent")?;
        ensure!(
            !identity.person
                && input.action_targets.get(&action.target).copied()
                    == Some(snapshot.handle(action.target)?.1),
            "object popup press action-target person/lifetime changed"
        );
    }
    Ok(())
}

fn viewport_button_messages(
    messages: &[Message],
    statuses: &[iced::event::Status],
    down: bool,
    pick: &Pick,
) -> Result<()> {
    ensure!(
        statuses == [iced::event::Status::Captured]
            && messages.len() == 1
            && matches!(&messages[0], Message::ViewportButton { down: actual, pixel, picked: Some(object) }
            if *actual == down && *pixel == pick.pixel && *object == pick.clicked.0),
        "object actual viewport did not capture the current matching button event"
    );
    Ok(())
}

fn popup_press_path<Context>(
    enabled: bool,
    context: &mut Context,
    release: impl FnOnce(&mut Context) -> Result<()>,
    diagnose: impl FnOnce(&mut Context) -> Result<()>,
) -> Result<bool> {
    if enabled {
        release(context)?;
        Ok(true)
    } else {
        diagnose(context)?;
        Ok(false)
    }
}

fn release_attempt(proof: &Proof, attempted: &mut bool) -> Result<()> {
    diagnostic_press_proof(proof)?;
    ensure!(!*attempted, "object duplicate popup release attempt");
    *attempted = true;
    Ok(())
}

fn captured_release_with<Context>(
    context: &mut Context,
    messages: Vec<Message>,
    statuses: Vec<iced::event::Status>,
    pick: &Pick,
    log: impl FnOnce(&mut Context, &[Message], &[iced::event::Status]),
    recheck: impl FnOnce(&mut Context) -> Result<()>,
    forward: impl FnOnce(&mut Context, Message) -> Result<()>,
) -> Result<()> {
    log(context, &messages, &statuses);
    viewport_button_messages(&messages, &statuses, false, pick)?;
    recheck(context)?;
    forward(
        context,
        messages
            .into_iter()
            .next()
            .context("object release message absent")?,
    )
}

fn original_release_point(
    input: &UiInput,
    pick: &Pick,
    position: Point,
    scene: &sims_render::Scene,
    display_size: [u32; 2],
    viewport: Rectangle,
    exclusions: &[Rectangle],
    layouts: &[Rectangle],
) -> Result<()> {
    let display = Rectangle::new(
        Point::ORIGIN,
        iced::Size::new(display_size[0] as f32, display_size[1] as f32),
    );
    ensure!(
        !display_size.contains(&0)
            && layouts.contains(&viewport)
            && layouts
                .iter()
                .all(|bounds| rect(*bounds).iter().all(|axis| axis.is_finite())
                    && bounds.width >= 0.
                    && bounds.height >= 0.
                    && (!bounds.contains(position) || *bounds == viewport || *bounds == display)),
        "object original popup point overlaps a non-backdrop layout or unsupported geometry"
    );
    ensure!(
        matches!(input.cursor_event(), iced::Event::Mouse(mouse::Event::CursorMoved {
            position: current
        }) if current == position),
        "object popup release requires the original available cursor; no relocation"
    );
    ensure!(
        viewport_point(
            pick.pixel,
            [scene.width, scene.height],
            viewport,
            exclusions
        )? == position
            && scene_cursor(scene, viewport, mouse::Cursor::Available(position))
                == Some(pick.pixel)
            && scene.pick(pick.pixel) == Some(pick.clicked.0),
        "object original popup point is not the same unobscured scene pick"
    );
    Ok(())
}

fn require_same_scene(before: &sims_render::Scene, after: &sims_render::Scene) -> Result<()> {
    ensure!(
        before.width == after.width && before.height == after.height,
        "object republished scene dimensions changed"
    );
    ensure!(
        before.vertices.len() == after.vertices.len()
            && before
                .vertices
                .iter()
                .zip(&after.vertices)
                .all(|(before, after)| {
                    before.position.map(f32::to_bits) == after.position.map(f32::to_bits)
                        && before.uv.map(f32::to_bits) == after.uv.map(f32::to_bits)
                        && before.tint.map(f32::to_bits) == after.tint.map(f32::to_bits)
                        && before.depth_scale.to_bits() == after.depth_scale.to_bits()
                }),
        "object republished scene vertex bits changed"
    );
    ensure!(
        before.draws.len() == after.draws.len()
            && before
                .draws
                .iter()
                .zip(&after.draws)
                .all(|(before, after)| {
                    before.vertices == after.vertices
                        && before.texture == after.texture
                        && before.object == after.object
                }),
        "object republished scene draw/target identity changed"
    );
    ensure!(
        before.censors.len() == after.censors.len()
            && before
                .censors
                .iter()
                .zip(&after.censors)
                .all(|(before, after)| {
                    before.rect == after.rect
                        && before.depth.to_bits() == after.depth.to_bits()
                        && before.seed == after.seed
                }),
        "object republished scene censor bits changed"
    );
    ensure!(
        before.textures == after.textures,
        "object republished scene texture dimensions or complete payload changed"
    );
    Ok(())
}

struct Entry {
    instruction: Record,
    cut: Cut,
    initial: walking::Native,
    base: JourneyGround,
    hud_height: u32,
    retained_streams: usize,
    retained_tasks: usize,
}

impl Entry {
    fn capture(journey: &Journey, prior: &Record) -> Result<Self> {
        let publication = journey.host.publication_for_test();
        let instruction = publication
            .dialog
            .clone()
            .context("object live entry instruction absent")?;
        entry_pins(
            prior,
            &instruction,
            &engine::digest(instruction.body.as_bytes()),
        )?;
        ensure!(
            !publication.modal,
            "object entry is modal; no response permitted"
        );
        let entry = Self {
            instruction,
            cut: Cut::current(journey),
            initial: walking::native(publication)?.clone(),
            base: ground(publication)?.clone(),
            hud_height: publication.hud_height,
            retained_streams: journey.streams,
            retained_tasks: journey.tasks,
        };
        entry.persistent(publication, entry.cut)?;
        ensure!(
            entry.initial.owner == Some(entry.instruction.caller)
                && entry.base.actor_lifetime
                    == entry
                        .initial
                        .actor_lifetime
                        .context("object actor lifetime absent")?
                && !entry.initial.input_pending
                && object(publication)?.input.is_none()
                && !object(publication)?.ground_input
                && publication.menu.is_none()
                && publication.native_popup.is_none(),
            "object entry has competing input/menu owner"
        );
        Ok(entry)
    }

    fn persistent(&self, publication: &Published, cut: Cut) -> Result<()> {
        self.cut.require(Cut {
            popup_epoch: self.cut.popup_epoch,
            ..cut
        })?;
        require_native_resident(publication, BOB_OBJECT)?;
        bob(&publication.residents)?;
        let native = walking::native(publication)?;
        persistent_native(&self.initial, native)?;
        ensure!(
            native.tick == tick(publication)?
                && publication.native_clock
                && publication.native_transport
                && authoritative_mode(publication) == Some(2)
                && object(publication)?.handle(BOB_OBJECT)?
                    == (BOB_OBJECT, self.base.actor_lifetime)
                && object(publication)?.handle(self.instruction.caller)?.1
                    == self
                        .initial
                        .owner_lifetime
                        .context("object caller lifetime absent")?,
            "object persistent native presentation changed"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
struct Pick {
    pixel: [f32; 2],
    clicked: (i16, u32),
    resolved: (i16, u32),
}

fn prefer_pick(
    selected: &mut Option<(Pick, Point)>,
    candidate: (Pick, Point),
    hint: Option<&JourneyTarget>,
) -> bool {
    let preferred = matches!(
        hint,
        Some(JourneyTarget::Object { target, table_sit: true })
            if *target == candidate.0.resolved.0
    );
    if selected.is_none() || preferred {
        *selected = Some(candidate);
    }
    preferred
}

fn offer_ready(input: Option<&JourneyObjectInput>) -> Result<bool> {
    Ok(input
        .context("object native input ended without offered actions; no retry or menu shopping")?
        .ready)
}

struct SceneCandidate {
    clicked: i16,
    table_sit: bool,
    pixels: Vec<[f32; 2]>,
}

fn scene_candidates(
    scene: &sims_render::Scene,
    snapshot: &JourneyObject,
    targets: &BTreeMap<i16, std::result::Result<JourneyTarget, String>>,
) -> Result<Vec<SceneCandidate>> {
    let mut indices = BTreeMap::new();
    let mut candidates: Vec<SceneCandidate> = Vec::new();
    for draw in &scene.draws {
        let Some(clicked) = draw.object.filter(|object| *object > 0) else {
            continue;
        };
        let Some(Ok(hint)) = targets.get(&clicked) else {
            continue;
        };
        let JourneyTarget::Object { table_sit, .. } = hint else {
            continue;
        };
        if classified_target(snapshot, Some(clicked), Some(hint)).is_err() {
            continue;
        }
        let vertices = scene
            .vertices
            .get(draw.vertices.start as usize..draw.vertices.end as usize)
            .context("object candidate draw vertex range invalid")?;
        for triangle in vertices.chunks_exact(3) {
            let pixel = [
                triangle
                    .iter()
                    .map(|vertex| (vertex.position[0] + 1.) * scene.width as f32 / 6.)
                    .sum::<f32>(),
                triangle
                    .iter()
                    .map(|vertex| (1. - vertex.position[1]) * scene.height as f32 / 6.)
                    .sum::<f32>(),
            ];
            if pixel.iter().all(|axis| axis.is_finite())
                && pixel[0] >= 0.
                && pixel[1] >= 0.
                && pixel[0] < scene.width as f32
                && pixel[1] < scene.height as f32
            {
                let index = *indices.entry(clicked).or_insert_with(|| {
                    let index = candidates.len();
                    candidates.push(SceneCandidate {
                        clicked,
                        table_sit: *table_sit,
                        pixels: Vec::new(),
                    });
                    index
                });
                candidates[index].pixels.push(pixel);
            }
        }
    }
    Ok(candidates)
}

fn classified_target(
    snapshot: &JourneyObject,
    picked: Option<i16>,
    hint: Option<&JourneyTarget>,
) -> Result<((i16, u32), (i16, u32))> {
    let clicked = picked.context("object pixel picked ground")?;
    let identity = snapshot
        .objects
        .get(&clicked)
        .context("clicked identity absent")?;
    ensure!(!identity.person, "clicked person excluded");
    let resolved = match hint {
        Some(JourneyTarget::Object { target, .. }) => *target,
        _ => anyhow::bail!("resolved person/unknown interaction target excluded"),
    };
    let identity = snapshot
        .objects
        .get(&resolved)
        .context("resolved identity absent")?;
    ensure!(!identity.person, "multitile-resolved person excluded");
    ensure!(
        identity
            .loaded_interaction_table_count
            .is_some_and(|count| count > 0),
        "resolved loaded interaction table absent/empty; count is only a search hint"
    );
    Ok((snapshot.handle(clicked)?, snapshot.handle(resolved)?))
}

fn target_pair(publication: &Published, picked: Option<i16>) -> Result<((i16, u32), (i16, u32))> {
    let hint = picked
        .and_then(|clicked| publication.journey_targets.get(&clicked))
        .and_then(|target| target.as_ref().ok());
    classified_target(object(publication)?, picked, hint)
}

fn viewport_point(
    pixel: [f32; 2],
    size: [u32; 2],
    viewport: Rectangle,
    exclusions: &[Rectangle],
) -> Result<Point> {
    ensure!(
        !size.contains(&0)
            && pixel
                .iter()
                .zip(size)
                .all(|(&axis, size)| axis.is_finite() && axis >= 20. && axis < size as f32 - 20.)
            && rect(viewport).iter().all(|axis| axis.is_finite())
            && viewport.width > 0.
            && viewport.height > 0.,
        "object pixel/edge-pan geometry invalid"
    );
    let scale = (viewport.width / size[0] as f32).min(viewport.height / size[1] as f32);
    ensure!(
        scale.is_finite() && scale > 0.,
        "object letterbox scale invalid"
    );
    let position = Point::new(
        viewport.x + (viewport.width - size[0] as f32 * scale) / 2. + pixel[0] * scale,
        viewport.y + (viewport.height - size[1] as f32 * scale) / 2. + pixel[1] * scale,
    );
    ensure!(
        viewport.contains(position)
            && exclusions
                .iter()
                .all(|bounds| rect(*bounds).iter().all(|axis| axis.is_finite())
                    && !bounds.contains(position)),
        "object pixel covered by HUD/instruction/control or letterboxing"
    );
    Ok(position)
}

#[derive(Clone)]
struct Offer {
    cut: Cut,
    owner: sims_runtime::original_session::live_menu::State,
    input: JourneyObjectInput,
    index: usize,
    label: String,
    action: Action,
    serial: u32,
    prior_ids: BTreeSet<u32>,
}

fn visible_actions(
    menu: &Menu,
    input: &JourneyObjectInput,
    owner: &PopupOwner,
    pick: &Pick,
) -> Result<()> {
    ensure!(!menu.preview && menu.ready && menu.popup_visible && menu.error.is_none()
        && input.ready && !input.actions.is_empty()
        && owner.kind == sims_runtime::original_session::live_menu::Kind::Object
        && owner.command.is_none() && owner.tick == input.tick
        && owner.actor == input.actor && input.actor.0 == BOB_OBJECT
        && owner.clicked == Some(input.clicked) && input.clicked == pick.clicked
        && owner.target == input.resolved && input.resolved == pick.resolved
        && menu.popup.len() == input.actions.len()
        && menu.popup.iter().enumerate().all(|(index, choice)|
            matches!(choice.intent.as_ref(), Some(Intent::ObjectChoice(actual)) if *actual == index)),
        "object ready visible native/input/leaf correspondence unavailable");
    Ok(())
}

fn current_payload(offer: &Offer, current: &JourneyObjectInput, cut: Cut) -> Result<()> {
    offer.cut.require(cut)?;
    ensure!(
        *current == offer.input
            && current.actions.get(offer.index) == Some(&offer.action)
            && current.action_targets.get(&offer.action.target)
                == offer.input.action_targets.get(&offer.action.target),
        "object offered payload/lifetime changed"
    );
    Ok(())
}

fn leaf(observation: &ElementObservation, label: &str) -> Result<(Rectangle, Point)> {
    let texts: Vec<_> = observation
        .texts
        .iter()
        .filter(|text| text.text == label)
        .collect();
    ensure!(
        !label.trim().is_empty() && texts.len() == 1,
        "object laid-out leaf absent/ambiguous"
    );
    let bounds = texts[0].container.context("object leaf container absent")?;
    let center = texts[0].bounds.center();
    ensure!(
        rect(bounds).iter().all(|axis| axis.is_finite())
            && bounds.width > 0.
            && bounds.height > 0.
            && bounds.contains(center),
        "object leaf text/container geometry invalid"
    );
    Ok((bounds, center))
}

fn choice_messages(
    messages: &[Message],
    statuses: &[iced::event::Status],
    index: usize,
) -> Result<()> {
    ensure!(
        statuses == [iced::event::Status::Captured; 2]
            && messages.len() == 1
            && matches!(messages[0], Message::ObjectChoice(actual) if actual == index),
        "object real captured leaf gesture did not emit exactly its offered ObjectChoice"
    );
    Ok(())
}

fn fresh_enqueue(
    offer: &Offer,
    current: &walking::Native,
    closed: bool,
    target_lifetime: Option<u32>,
) -> Result<Option<Action>> {
    if !closed {
        return Ok(None);
    }
    ensure!(
        target_lifetime
            == offer
                .input
                .action_targets
                .get(&offer.action.target)
                .copied(),
        "object enqueue target lifetime changed"
    );
    let fresh: Vec<_> = current
        .active
        .iter()
        .chain(current.pending.iter())
        .filter(|action| action.id > offer.serial && !offer.prior_ids.contains(&action.id))
        .collect();
    if fresh.is_empty() {
        return Ok(None);
    }
    ensure!(fresh.len() == 1, "object new Bob enqueue ambiguous");
    let action = fresh[0];
    let mut expected = offer.action.clone();
    expected.id = action.id;
    ensure!(
        *action == expected && action.id <= current.serial,
        "object new Bob action does not match the offered payload"
    );
    Ok(Some(action.clone()))
}

fn completed(accepted: &Action, current: &walking::Native) -> bool {
    if current
        .active
        .iter()
        .chain(current.pending.iter())
        .any(|action| action.id == accepted.id)
    {
        return false;
    }
    current.previous.as_ref().is_some_and(|previous| {
        let mut expected = accepted.clone();
        expected.flags = (expected.flags & !32) | 32;
        previous == &expected && previous.flags & 32 != 0
    })
}

#[derive(Default, serde::Serialize)]
struct Proof {
    offer_observed: bool,
    command_sent: bool,
    enqueue_accepted: bool,
    active_seen: bool,
    native_completion_observed: bool,
    viewport_left_press_sent: bool,
    viewport_left_release_sent: bool,
    hover_qualified: bool,
}

fn phase_capacity(
    usage: Usage,
    delta: Usage,
    initial: u64,
    current: u64,
    ticks: u64,
    elapsed: f64,
) -> Result<()> {
    usage.reserve(delta)?;
    ensure!(
        current
            .checked_sub(initial)
            .and_then(|delta| delta.checked_add(ticks))
            .is_some_and(|delta| delta <= TICK_LIMIT)
            && elapsed >= 0.
            && elapsed < WALL_SECONDS,
        "object tick/wall ceiling exhausted or reset"
    );
    Ok(())
}

const PROGRESS_CAPACITY: usize = 32;

fn prospective_usage(usage: Usage, delta: Usage) -> Usage {
    Usage {
        frames: usage.frames.saturating_add(delta.frames),
        service_polls: usage.service_polls.saturating_add(delta.service_polls),
        effect_polls: usage.effect_polls.saturating_add(delta.effect_polls),
        tasks: usage.tasks.saturating_add(delta.tasks),
        commands: usage.commands.saturating_add(delta.commands),
        pixel_scans: usage.pixel_scans.saturating_add(delta.pixel_scans),
    }
}

#[derive(Default, serde::Serialize)]
struct EffectCharges {
    dispatch: usize,
    redraw: usize,
    receive: usize,
    completion: usize,
    finished: usize,
    timeout: usize,
    other_output: usize,
    other_action: usize,
    disconnected: usize,
    receive_without_outcome: usize,
}

impl EffectCharges {
    fn received(&mut self, delivery: &std::result::Result<Delivery, mpsc::RecvTimeoutError>) {
        self.receive_without_outcome -= 1;
        match delivery {
            Ok(Delivery::Finished) => self.finished += 1,
            Ok(Delivery::Action(iced_runtime::Action::Output(Event::Completion))) => self.completion += 1,
            Ok(Delivery::Action(iced_runtime::Action::Output(_))) => self.other_output += 1,
            Ok(_) => self.other_action += 1,
            Err(mpsc::RecvTimeoutError::Timeout) => self.timeout += 1,
            Err(mpsc::RecvTimeoutError::Disconnected) => self.disconnected += 1,
        }
    }

    fn charge_total(&self) -> usize {
        self.dispatch + self.redraw + self.receive
    }

    fn outcome_total(&self) -> usize {
        self.completion + self.finished + self.timeout + self.other_output
            + self.other_action + self.disconnected
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
struct ProgressEntry {
    driver_ms: u64,
    native_tick: u64,
    actor: (i16, Option<u32>),
    position: Option<[i32; 3]>,
    owner: Option<(i16, Option<u32>)>,
    serial: u32,
    accepted: Option<u32>,
    active: Option<(u32, i16, u16, u32)>,
    active_target_lifetime: Option<u32>,
    previous: Option<u32>,
    pending_count: usize,
    instruction: Option<u64>,
    children_pending: bool,
    clock_pending: bool,
    input_pending: bool,
    pointer_pending: bool,
    ground_pending: bool,
    publication_pending: bool,
    streams: usize,
    registered_tasks: usize,
    vm: Option<JourneyObjectProgress>,
}

#[derive(Default, serde::Serialize)]
struct ProgressLedger {
    entries: [Option<ProgressEntry>; PROGRESS_CAPACITY],
    observations: usize,
    transitions: usize,
    omitted_transitions: usize,
}

impl ProgressLedger {
    fn record(&mut self, entry: ProgressEntry) {
        self.observations += 1;
        let last = self.transitions.saturating_sub(1).min(PROGRESS_CAPACITY - 1);
        if self.entries[last] == Some(entry) {
            return;
        }
        let slot = self.transitions.min(PROGRESS_CAPACITY - 1);
        if self.transitions >= PROGRESS_CAPACITY {
            self.omitted_transitions += 1;
        }
        self.entries[slot] = Some(entry);
        self.transitions += 1;
    }
}

#[derive(serde::Serialize)]
struct RejectedReservation {
    site: &'static str,
    consumed: Usage,
    requested_delta: Usage,
    prospective: Usage,
    consumed_global: Usage,
    prospective_global: Usage,
    tick_reservation: u64,
    native_tick: u64,
    operation_executed: bool,
}

#[derive(Default)]
struct Diagnostic {
    effects: EffectCharges,
    ledger: ProgressLedger,
    rejection: Option<RejectedReservation>,
    rejected_reservations: usize,
}

impl Diagnostic {
    fn receipt(&self, usage: Option<Usage>, initial_streams: usize, streams: usize) -> Value {
        json!({"schema":"newbie-object-progress-v1", "effects":self.effects,
            "actual_consumed_usage":usage,
            "effect_charge_total":self.effects.charge_total(),
            "effect_charges_reconcile":usage.map(|usage| usage.effect_polls == self.effects.charge_total()),
            "receive_outcomes_total":self.effects.outcome_total(),
            "receive_outcomes_reconcile":self.effects.receive
                == self.effects.outcome_total() + self.effects.receive_without_outcome,
            "stream_balance_reconciles":usage.and_then(|usage| initial_streams.checked_add(usage.tasks))
                .and_then(|registered| registered.checked_sub(self.effects.finished)) == Some(streams),
            "initial_streams":initial_streams,"remaining_streams":streams,
            "delivery_stream_identity":"unavailable: shared Delivery is untagged",
            "rejected_reservation":self.rejection,"rejected_reservations_uncharged":self.rejected_reservations,
            "ledger":self.ledger,
            "retention":"first 31 distinct adjacent transitions plus latest; duplicates counted as observations; omitted_transitions counts overwritten transitions",
            "native_scope":"actor object-thread top frame/status and matching route only; no nested resume traversal or native duration guarantee"})
    }
}

struct Driver<'a> {
    journey: &'a mut Journey,
    entry: Entry,
    start: Usage,
    wall: Instant,
    probes: usize,
    opened: bool,
    pick: Option<Pick>,
    offer: Option<Offer>,
    accepted: Option<Action>,
    proof: Proof,
    next: Option<Value>,
    boundary_valid: bool,
    popup_press_diagnostic: Option<Value>,
    redraw_source: Option<(Arc<Published>, Cut)>,
    popup_release_enabled: bool,
    popup_release_attempted: bool,
    diagnostic: Option<Box<Diagnostic>>,
}

impl Driver<'_> {
    fn usage(&self) -> Result<Usage> {
        Ok(Usage {
            pixel_scans: self.probes,
            ..Usage::current(self.journey).since(self.start)?
        })
    }

    fn reserve(
        &mut self,
        site: &'static str,
        delta: Usage,
        tick_reservation: u64,
    ) -> Result<()> {
        let native = walking::native(self.journey.host.publication_for_test())?;
        let native_tick = native.tick;
        let usage = self.usage()?;
        let result = (|| {
            phase_capacity(
                usage,
                delta,
                self.entry.initial.tick,
                native_tick,
                tick_reservation,
                self.wall.elapsed().as_secs_f64(),
            )?;
            walking::preflight(self.journey, delta)
        })();
        if result.is_err() && let Some(diagnostic) = self.diagnostic.as_mut() {
            diagnostic.rejected_reservations += 1;
            if diagnostic.rejection.is_none() {
                let global = Usage::current(self.journey);
                diagnostic.rejection = Some(RejectedReservation {
                    site, consumed: usage, requested_delta: delta,
                    prospective: prospective_usage(usage, delta),
                    consumed_global: global, prospective_global: prospective_usage(global, delta),
                    tick_reservation, native_tick, operation_executed: false,
                });
            }
        }
        result
    }

    fn record_progress(&mut self, publication: &Published, native: &walking::Native) {
        let Some(diagnostic) = self.diagnostic.as_mut() else {
            return;
        };
        let Some(Ok(snapshot)) = publication.journey_object.as_ref() else {
            return;
        };
        diagnostic.ledger.record(ProgressEntry {
            driver_ms: self.journey.millis,
            native_tick: native.tick,
            actor: (native.actor, native.actor_lifetime),
            position: native.position.map(|position| [position.x, position.y, position.level]),
            owner: native.owner.map(|owner| (owner, native.owner_lifetime)),
            serial: native.serial,
            accepted: self.accepted.as_ref().map(|action| action.id),
            active: native.active.as_ref().map(|action| (action.id, action.target, action.routine, action.flags)),
            active_target_lifetime: native.active.as_ref()
                .and_then(|action| native.targets.get(&action.target))
                .and_then(|target| target.lifetime),
            previous: native.previous.as_ref().map(|action| action.id),
            pending_count: native.pending.len(),
            instruction: publication.dialog.as_ref().map(|record| record.id),
            children_pending: native.children_pending,
            clock_pending: native.clock_pending,
            input_pending: native.input_pending,
            pointer_pending: snapshot.pointer_pending,
            ground_pending: snapshot.ground_input,
            publication_pending: publication.pending,
            streams: self.journey.streams,
            registered_tasks: self.journey.tasks,
            vm: snapshot.progress.as_deref().copied(),
        });
    }

    fn observe(&mut self) -> Result<()> {
        let publication = self.journey.host.publication_for_test().clone();
        bound_redraw_observation(
            self,
            |driver| {
                let changed = publication.modal
                    || publication
                        .dialog
                        .as_ref()
                        .is_some_and(|record| record != &driver.entry.instruction);
                if changed && driver.next.is_none() {
                    driver.next = Some(
                        json!({"record":publication.dialog,"modal":publication.modal,
                "cut":Cut::current(driver.journey),"response_sent":false}),
                    );
                }
            },
            |driver| {
                if let Some((source, cut)) = &driver.redraw_source {
                    driver.require_redraw_source(source, *cut)?;
                }
                Ok(())
            },
            |driver| driver.observe_publication(&publication),
        )
    }

    fn observe_publication(&mut self, publication: &Published) -> Result<()> {
        let changed = publication.modal
            || publication
                .dialog
                .as_ref()
                .is_some_and(|record| record != &self.entry.instruction);
        self.reserve("observe_publication", Usage::default(), 0)?;
        self.journey.healthy()?;
        self.journey.host.frontend_for_test()?;
        self.entry
            .persistent(publication, Cut::current(self.journey))?;
        let native = walking::native(publication)?;
        if !self.opened {
            notification_unchanged(&self.entry.initial, native)?;
            ensure!(
                publication.dialog.as_ref() == Some(&self.entry.instruction)
                    && !publication.modal
                    && publication.native_popup.is_none()
                    && publication.menu.is_none(),
                "object unchanged live handoff instruction/menu ownership lost"
            );
        }
        credit_after_owners(&self.entry.initial, native, || -> Result<()> {
            if self.proof.command_sent {
                let offer = self
                    .offer
                    .as_ref()
                    .context("object sent command has no retained offer")?;
                let snapshot = object(publication)?;
                let target_lifetime = snapshot
                    .objects
                    .get(&offer.action.target)
                    .and_then(|identity| identity.lifetime);
                ensure!(
                    target_lifetime
                        == offer
                            .input
                            .action_targets
                            .get(&offer.action.target)
                            .copied(),
                    "object accepted/offered target lifetime changed"
                );
                if self.accepted.is_none() {
                    let closed = snapshot.input.is_none()
                        && !snapshot.ground_input
                        && publication.native_popup.is_none()
                        && publication.menu.is_none();
                    self.accepted = fresh_enqueue(offer, native, closed, target_lifetime)?;
                    self.proof.enqueue_accepted = self.accepted.is_some();
                }
                if let Some(accepted) = &self.accepted {
                    self.proof.active_seen |= native.active.as_ref() == Some(accepted);
                    self.proof.native_completion_observed |= completed(accepted, native);
                }
            }
            Ok(())
        })?;
        if changed {
            let record = publication
                .dialog
                .as_ref()
                .context("object new modal has no record")?;
            ensure!(
                record.id > self.entry.instruction.id
                    && record.caller == self.entry.instruction.caller
                    && record.source == SOURCE
                    && record.origin.source == SOURCE
                    && record.status == 1,
                "object new instruction/modal has stale or changed provenance"
            );
            self.boundary_valid = true;
            anyhow::bail!(BOUNDARY);
        }
        ensure!(
            publication.dialog.as_ref() == Some(&self.entry.instruction),
            "object entry instruction silently disappeared"
        );
        self.record_progress(publication, native);
        Ok(())
    }

    fn dispatch(&mut self, event: Event) -> Result<()> {
        self.observe()?;
        self.reserve(
            "dispatch",
            Usage {
                commands: 1,
                effect_polls: 1,
                tasks: 1,
                ..Default::default()
            },
            0,
        )?;
        if let Event::Ui { message, .. } = &event {
            match message {
                Message::ViewportButton { down: true, .. } => {
                    self.proof.viewport_left_press_sent = true;
                    self.opened = true;
                }
                Message::ViewportButton { down: false, .. } => {
                    self.proof.viewport_left_release_sent = true
                }
                Message::ObjectChoice(_) => self.proof.command_sent = true,
                _ => {}
            }
        }
        self.journey.idle = false;
        self.journey.commands += 1;
        self.journey.effect_polls += 1;
        if let Some(diagnostic) = self.diagnostic.as_mut() {
            diagnostic.effects.dispatch += 1;
        }
        let task = self.journey.host.update(event);
        walking::keep_then_observe(
            true,
            self,
            task,
            |driver, task| walking::register_returned(driver.journey, task),
            Self::observe,
        )
    }

    fn drain(&mut self) -> Result<()> {
        loop {
            self.observe()?;
            if drained(
                self.journey.streams,
                self.journey.host.frontend_idle_for_test(),
            )? {
                self.journey.idle = true;
                return Ok(());
            }
            self.reserve(
                "receive",
                Usage {
                    effect_polls: 1,
                    ..Default::default()
                },
                0,
            )?;
            self.journey.effect_polls += 1;
            if let Some(diagnostic) = self.diagnostic.as_mut() {
                diagnostic.effects.receive += 1;
                diagnostic.effects.receive_without_outcome += 1;
            }
            let remaining = (WALL_SECONDS - self.wall.elapsed().as_secs_f64())
                .min(self.journey.wall_seconds - self.journey.wall.elapsed().as_secs_f64());
            ensure!(remaining > 0., "object receive wall limit");
            let delivery = self.journey.receiver
                .recv_timeout(Duration::from_secs_f64(remaining.min(0.1)));
            self.received(delivery)?;
        }
    }

    fn received(
        &mut self,
        delivery: std::result::Result<Delivery, mpsc::RecvTimeoutError>,
    ) -> Result<()> {
        if let Some(diagnostic) = self.diagnostic.as_mut() {
            diagnostic.effects.received(&delivery);
        }
        match delivery {
            Ok(Delivery::Finished) => {
                self.journey.streams = self
                    .journey
                    .streams
                    .checked_sub(1)
                    .context("object Finished without registered stream")?;
                self.observe()?;
            }
            Ok(Delivery::Action(iced_runtime::Action::Output(event))) => {
                ensure!(
                    self.journey.streams != 0,
                    "object Completion without registered stream"
                );
                self.dispatch(walking::notification_event(event)?)?;
            }
            Ok(_) => {
                anyhow::bail!("object unsupported delivery; no gameplay effect dispatched")
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn frame(&mut self, advance: bool) -> Result<()> {
        self.observe()?;
        let delta = if advance {
            Usage {
                frames: 1,
                ..Default::default()
            }
        } else {
            Usage {
                service_polls: 1,
                ..Default::default()
            }
        };
        self.reserve(
            "frame",
            Usage {
                commands: 1,
                effect_polls: 1,
                tasks: 1,
                ..delta
            },
            16,
        )?;
        if advance {
            self.journey.frames += 1;
            self.journey.millis += 40;
        } else {
            self.journey.service_polls += 1;
        }
        self.dispatch(Event::Frame(
            self.journey.epoch + Duration::from_millis(self.journey.millis),
        ))?;
        self.drain()
    }

    fn require_redraw_source(&self, source: &Published, cut: Cut) -> Result<()> {
        let current = self.journey.host.publication_for_test();
        redraw_binding(
            cut,
            Cut::current(self.journey),
            &redraw_identity(source)?,
            &redraw_identity(current)?,
        )?;
        let before = ground(source)?;
        let after = ground(current)?;
        ensure!(
            before.camera == after.camera
                && before.floor == after.floor
                && before.tick == after.tick
                && before.display_size == after.display_size,
            "object redraw camera/floor/tick/display changed"
        );
        require_same_scene(&before.scene, &after.scene)
    }

    fn observation(&mut self, input: &mut UiInput) -> Result<ElementObservation> {
        loop {
            self.observe()?;
            self.reserve(
                "redraw",
                Usage {
                    effect_polls: 1,
                    ..Default::default()
                },
                0,
            )?;
            self.journey.effect_polls += 1;
            if let Some(diagnostic) = self.diagnostic.as_mut() {
                diagnostic.effects.redraw += 1;
            }
            let source = self.journey.host.publication_for_test().clone();
            let cut = Cut::current(self.journey);
            let mut observation = input.element_observation(
                self.journey.host.frontend_for_test()?.view(),
                ground(&source)?.display_size,
            )?;
            let held_popup = self.proof.viewport_left_press_sent
                && !self.proof.viewport_left_release_sent
                && !self.proof.command_sent
                && source.native_popup.is_some();
            self.journey.events.push(json!({"kind":"tutorial_object_redraw",
                "cut":cut,"phase":if held_popup {"held_object_popup"} else {"old_empty_only"},
                "messages":observation.messages.iter().map(|message| format!("{message:?}"))
                    .collect::<Vec<_>>(),"owner":source.native_popup,
                "interaction":format!("{:?}",observation.interaction),
                "layouts":observation.layouts.iter().copied().map(rect).collect::<Vec<_>>(),
                "texts":observation.texts.iter().map(|text| json!({"text":text.text,
                    "bounds":rect(text.bounds),"container":text.container.map(rect)}))
                    .collect::<Vec<_>>(),"viewport_release_already_sent":self.proof.viewport_left_release_sent,
                "leaf_command_already_sent":self.proof.command_sent}));
            self.observe()?;
            self.require_redraw_source(&source, cut)?;
            redraw_notifications(&observation.messages, held_popup)?;
            if observation.messages.is_empty() {
                return Ok(observation);
            }
            diagnostic_press_proof(&self.proof)?;
            self.require_popup_press(&source)?;
            for message in std::mem::take(&mut observation.messages) {
                self.require_redraw_source(&source, cut)?;
                ensure!(
                    self.redraw_source.is_none(),
                    "object nested bound redraw dispatch"
                );
                self.redraw_source = Some((source.clone(), cut));
                let result = self.send(&source, message);
                self.redraw_source = None;
                result?;
                self.require_redraw_source(&source, cut)?;
            }
        }
    }

    fn require_popup_press(&mut self, publication: &Published) -> Result<()> {
        input_gate(
            self.journey.streams,
            self.journey.host.frontend_idle_for_test(),
            walking::native(publication)?.children_pending,
        )?;
        self.entry
            .persistent(publication, Cut::current(self.journey))?;
        let snapshot = object(publication)?;
        let pick = self
            .pick
            .clone()
            .context("object popup press pick absent")?;
        popup_press_identity(
            self.entry.cut,
            Cut::current(self.journey),
            &self.entry.initial,
            walking::native(publication)?,
            snapshot,
            publication
                .native_popup
                .as_ref()
                .context("object popup press owner absent")?,
            publication
                .menu
                .as_ref()
                .context("object popup press menu absent")?,
            &pick,
        )?;
        let current = ground(publication)?;
        ensure!(
            current.camera == self.entry.base.camera
                && current.floor == self.entry.base.floor
                && current.tick == self.entry.base.tick
                && current.display_size == self.entry.base.display_size
                && publication.hud_height == self.entry.hud_height
                && Arc::ptr_eq(
                    &current.scene,
                    publication.scene.as_ref().context("object scene absent")?
                )
                && publication
                    .journey_camera
                    .as_ref()
                    .and_then(|camera| camera.as_ref().ok())
                    == Some(&current.camera),
            "object popup press scene/camera/geometry changed"
        );
        require_same_scene(&self.entry.base.scene, &current.scene)?;
        ensure!(
            self.pick(publication, pick.pixel)? == pick,
            "object popup press exact pick/lifetimes changed"
        );
        Ok(())
    }

    fn release_popup_press(&mut self, input: &mut UiInput, position: Point) -> Result<()> {
        diagnostic_press_proof(&self.proof)?;
        self.observe()?;
        let admission = self.journey.host.publication_for_test().clone();
        self.require_popup_press(&admission)?;
        let observation = self.observation(input)?;
        self.observe()?;
        let source = self.journey.host.publication_for_test().clone();
        let cut = Cut::current(self.journey);
        self.require_popup_press(&source)?;
        let pick = self
            .pick
            .clone()
            .context("object popup release pick absent")?;
        let current = ground(&source)?;
        let (viewport, exclusions) = layout(&observation, &source, current)?;
        original_release_point(
            input,
            &pick,
            position,
            &current.scene,
            current.display_size,
            viewport,
            &exclusions,
            &observation.layouts,
        )?;
        release_attempt(&self.proof, &mut self.popup_release_attempted)?;
        let (publication, messages, statuses) = self.batch(
            input,
            &[iced::Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left,
            ))],
        )?;
        captured_release_with(
            self,
            messages,
            statuses,
            &pick,
            |driver, messages, statuses| {
                driver.journey.events.push(json!({"kind":"tutorial_object_popup_release_batch",
                    "source_cut":cut,"current_cut":Cut::current(driver.journey),
                    "owner":publication.native_popup,"pick":pick,
                    "position":[position.x,position.y],"viewport":rect(viewport),
                    "exclusions":exclusions.iter().copied().map(rect).collect::<Vec<_>>(),
                    "statuses":statuses.iter().map(|status| format!("{status:?}")).collect::<Vec<_>>(),
                    "messages":messages.iter().map(|message| format!("{message:?}")).collect::<Vec<_>>(),
                    "release_attempted":true,"forwarded":false,"offer_observed":false,
                    "command_sent":false,"enqueue_accepted":false}));
            },
            |driver| {
                ensure!(
                    driver.redraw_source.is_none(),
                    "object nested release source check"
                );
                driver.redraw_source = Some((source.clone(), cut));
                let checked = driver.observe();
                driver.redraw_source = None;
                checked?;
                driver.require_redraw_source(&publication, cut)?;
                driver.require_popup_press(&publication)
            },
            |driver, message| driver.send(&publication, message),
        )?;
        self.settle_gesture_pointer()
    }

    fn diagnose_popup_press(&mut self, input: &mut UiInput, position: Point) -> Result<()> {
        let publication = self.journey.host.publication_for_test();
        let snapshot = object(publication)?;
        if publication.native_popup.is_none()
            && publication.menu.is_none()
            && snapshot.input.is_none()
            && !snapshot.ground_input
        {
            return Ok(());
        }
        let observation = self.observation(input)?;
        self.observe()?;
        let publication = self.journey.host.publication_for_test().clone();
        diagnostic_press_proof(&self.proof)?;
        self.require_popup_press(&publication)?;
        let current = ground(&publication)?;
        let (viewport, exclusions) = layout(&observation, &publication, current)?;
        let pick = self
            .pick
            .as_ref()
            .context("object diagnostic pick absent")?;
        let viewport_check = viewport_point(
            pick.pixel,
            [current.scene.width, current.scene.height],
            viewport,
            &exclusions,
        );
        let diagnostic = json!({"kind":"tutorial_object_popup_press_diagnostic",
            "cut":Cut::current(self.journey),"owner":publication.native_popup,
            "input":object(&publication)?.input,"menu":menu_identity(publication.menu.as_ref()),
            "pick":pick,"position":[position.x,position.y],"viewport":rect(viewport),
            "exclusions":exclusions.into_iter().map(rect).collect::<Vec<_>>(),
            "old_viewport_point":viewport_check.as_ref().ok().map(|point| [point.x,point.y]),
            "old_viewport_check_error":viewport_check.err().map(|error| format!("{error:#}")),
            "interaction":format!("{:?}",observation.interaction),
            "redraw_stabilized":true,"release_attempted":false,"leaf_activation_attempted":false,
            "gesture_completed":false,"offer_observed":false,"enqueue_accepted":false,
            "requires_independent_release_adapter_review":true});
        self.journey.events.push(diagnostic.clone());
        self.popup_press_diagnostic = Some(diagnostic);
        anyhow::bail!(
            "object diagnostic popup-on-press boundary; no release, leaf, or acceptance credit"
        )
    }

    fn batch(
        &mut self,
        input: &mut UiInput,
        events: &[iced::Event],
    ) -> Result<(Arc<Published>, Vec<Message>, Vec<iced::event::Status>)> {
        self.observe()?;
        self.reserve(
            "input_batch",
            Usage {
                pixel_scans: events.len(),
                ..Default::default()
            },
            0,
        )?;
        self.probes += events.len();
        let publication = self.journey.host.publication_for_test().clone();
        let batch = input.element_batch(
            self.journey.host.frontend_for_test()?.view(),
            ground(&publication)?.display_size,
            events,
        );
        Ok((publication, batch.messages, batch.statuses))
    }

    fn send(&mut self, source: &Published, message: Message) -> Result<()> {
        self.observe()?;
        self.reserve(
            "send",
            Usage {
                commands: 1,
                effect_polls: 1,
                tasks: 1,
                ..Default::default()
            },
            0,
        )?;
        match &message {
            Message::ViewportButton { down: true, .. } => {
                ensure!(
                    !self.proof.viewport_left_press_sent,
                    "object duplicate viewport press"
                );
            }
            Message::ViewportButton { down: false, .. } => {
                ensure!(
                    self.proof.viewport_left_press_sent && !self.proof.viewport_left_release_sent,
                    "object duplicate/unpaired viewport release"
                );
            }
            Message::ObjectChoice(index) => {
                self.current_offer()?;
                ensure!(
                    !self.proof.command_sent
                        && self
                            .offer
                            .as_ref()
                            .is_some_and(|offer| offer.index == *index),
                    "object duplicate/stale leaf activation"
                );
            }
            Message::ViewportEnter
            | Message::ViewportLeave
            | Message::ViewportMotion { pan: [0, 0], .. } => {}
            _ => anyhow::bail!("object disallowed frontend message"),
        }
        let event = self.journey.host.ui_event_for_test(source, message);
        self.dispatch(event)?;
        self.drain()
    }

    fn motion(
        &mut self,
        input: &mut UiInput,
        position: Point,
        pixel: Option<[f32; 2]>,
    ) -> Result<()> {
        let (source, messages, _) = self.batch(
            input,
            &[iced::Event::Mouse(mouse::Event::CursorMoved { position })],
        )?;
        if let Some(pixel) = pixel {
            ensure!(
                messages.iter().any(|message| matches!(message,
                Message::ViewportMotion { pixel: Some(actual), pan: [0, 0] } if *actual == pixel)),
                "object actual viewport zero-pan motion missing"
            );
        }
        for message in messages {
            ensure!(
                matches!(
                    message,
                    Message::ViewportEnter
                        | Message::ViewportLeave
                        | Message::ViewportMotion { pan: [0, 0], .. }
                ),
                "object hover emitted non-motion command"
            );
            self.send(&source, message)?;
        }
        Ok(())
    }

    fn settle_gesture_pointer(&mut self) -> Result<()> {
        settle_pointer_with(
            self,
            |driver| {
                driver.drain()?;
                let publication = driver.journey.host.publication_for_test();
                Ok(object(publication)?.pointer_pending
                    || walking::native(publication)?.children_pending)
            },
            |driver| driver.frame(false),
        )
    }

    fn pick(&mut self, publication: &Published, pixel: [f32; 2]) -> Result<Pick> {
        self.reserve(
            "pick",
            Usage {
                pixel_scans: 1,
                ..Default::default()
            },
            0,
        )?;
        self.probes += 1;
        let (clicked, resolved) = target_pair(publication, ground(publication)?.scene.pick(pixel))?;
        Ok(Pick {
            pixel,
            clicked,
            resolved,
        })
    }

    fn revalidate_pick(
        &mut self,
        input: &mut UiInput,
        expected: &Pick,
        position: Point,
    ) -> Result<()> {
        self.observe()?;
        let publication = self.journey.host.publication_for_test().clone();
        let current = ground(&publication)?;
        input_gate(
            self.journey.streams,
            self.journey.host.frontend_idle_for_test(),
            walking::native(&publication)?.children_pending,
        )?;
        notification_unchanged(&self.entry.initial, walking::native(&publication)?)?;
        ensure!(
            current.camera == self.entry.base.camera
                && current.floor == self.entry.base.floor
                && current.tick == self.entry.base.tick
                && current.display_size == self.entry.base.display_size
                && publication.hud_height == self.entry.hud_height
                && Arc::ptr_eq(
                    &current.scene,
                    publication.scene.as_ref().context("object scene absent")?
                )
                && publication
                    .journey_camera
                    .as_ref()
                    .and_then(|camera| camera.as_ref().ok())
                    == Some(&current.camera)
                && !object(&publication)?.pointer_pending,
            "object scene/camera/geometry or pending motion changed"
        );
        require_same_scene(&self.entry.base.scene, &current.scene)?;
        let observation = self.observation(input)?;
        let (viewport, exclusions) = layout(&observation, &publication, current)?;
        ensure!(
            viewport_point(
                expected.pixel,
                [current.scene.width, current.scene.height],
                viewport,
                &exclusions
            )? == position,
            "object actual laid-out geometry changed"
        );
        ensure!(
            self.pick(&publication, expected.pixel)? == *expected,
            "object scene pick/lifetimes changed"
        );
        Ok(())
    }

    fn current_offer(&self) -> Result<()> {
        let offer = self.offer.as_ref().context("object offer absent")?;
        let publication = self.journey.host.publication_for_test();
        let snapshot = object(publication)?;
        let input = snapshot
            .input
            .as_ref()
            .context("object input owner ended")?;
        current_payload(offer, input, Cut::current(self.journey))?;
        ensure!(
            publication.native_popup.as_ref() == Some(&offer.owner)
                && !snapshot.ground_input
                && walking::native(publication)?.serial == offer.serial,
            "object retained popup/serial changed before command"
        );
        let owner: PopupOwner = serde_json::from_value(serde_json::to_value(&offer.owner)?)?;
        visible_actions(
            publication
                .menu
                .as_ref()
                .context("object visible menu ended")?,
            input,
            &owner,
            self.pick.as_ref().context("object clicked pick absent")?,
        )?;
        ensure!(
            snapshot.handle(input.actor.0)? == input.actor
                && snapshot.handle(input.clicked.0)? == input.clicked
                && snapshot.handle(input.resolved.0)? == input.resolved
                && snapshot.handle(offer.action.target)?.1
                    == input.action_targets[&offer.action.target]
                && publication.menu.as_ref().unwrap().popup[offer.index].label == offer.label,
            "object bound actor/clicked/resolved/action-target lifetime or label changed"
        );
        ensure!(
            target_pair(publication, Some(input.clicked.0))? == (input.clicked, input.resolved)
                && !snapshot.objects[&offer.action.target].person,
            "object clicked/resolved/action target became a person or redirect changed"
        );
        Ok(())
    }

    fn execute(&mut self) -> Result<()> {
        ensure!(
            self.journey.streams == self.entry.retained_streams
                && self.journey.tasks == self.entry.retained_tasks,
            "object same-Journey registered-task handoff changed"
        );
        self.drain()?;
        let publication = self.journey.host.publication_for_test().clone();
        input_gate(
            self.journey.streams,
            self.journey.host.frontend_idle_for_test(),
            walking::native(&publication)?.children_pending,
        )?;
        ensure!(
            publication.playing && !object(&publication)?.pointer_pending,
            "object paused or pending viewport input; no automatic playback recovery"
        );
        self.reserve("execute_entry", Usage::default(), 0)?;
        let mut input = UiInput::new()?;
        let geometry = self.observation(&mut input)?;
        let (viewport, exclusions) = layout(&geometry, &publication, &self.entry.base)?;
        let candidates = scene_candidates(
            &self.entry.base.scene,
            object(&publication)?,
            &publication.journey_targets,
        )?;
        let priorities: Vec<_> = candidates
            .iter()
            .map(|candidate| candidate.table_sit)
            .collect();
        let indices = prioritized_indices(&priorities, 8);
        let allowance = 48 / indices.len().max(1);
        let mut selected = None;
        'discovery: for index in indices {
            let candidate = &candidates[index];
            for centroid in spread_indices(candidate.pixels.len(), allowance) {
                self.reserve("candidate", Usage::default(), 0)?;
                let pixel = candidate.pixels[centroid];
                if let Ok(position) = viewport_point(
                    pixel,
                    [self.entry.base.scene.width, self.entry.base.scene.height],
                    viewport,
                    &exclusions,
                ) {
                    self.reserve(
                        "candidate_pick",
                        Usage {
                            pixel_scans: 1,
                            ..Default::default()
                        },
                        0,
                    )?;
                    if let Ok(pick) = self.pick(&publication, pixel)
                        && pick.clicked.0 == candidate.clicked
                    {
                        let hint = publication
                            .journey_targets
                            .get(&pick.clicked.0)
                            .and_then(|target| target.as_ref().ok());
                        prefer_pick(&mut selected, (pick, position), hint);
                        break 'discovery;
                    }
                }
            }
        }
        let (pick, position) =
            selected.context("object bounded scene search found no visible nonperson target")?;
        self.pick = Some(pick.clone());
        self.journey
            .events
            .push(json!({"kind":"tutorial_object_pick_hint","pick":pick,
            "metadata":publication.journey_targets.get(&pick.clicked.0),
            "resolved_loaded_interaction_table_count":object(&publication)?.objects.get(&pick.resolved.0)
                .and_then(|identity| identity.loaded_interaction_table_count),
            "search_only_not_availability":true,"scene_pick_probes":self.probes,
            "candidate_objects":candidates.len(),"candidate_centroids":candidates.iter().map(|candidate| candidate.pixels.len()).sum::<usize>(),
            "preferred_candidate_objects":priorities.iter().filter(|&&priority| priority).count(),
            "sampled_candidate_limit":8,"discovery_probe_limit":48}));
        self.motion(&mut input, position, Some(pick.pixel))?;
        self.settle_gesture_pointer()?;
        self.revalidate_pick(&mut input, &pick, position)?;
        for down in [true, false] {
            self.revalidate_pick(&mut input, &pick, position)?;
            let button = if down {
                mouse::Event::ButtonPressed(mouse::Button::Left)
            } else {
                mouse::Event::ButtonReleased(mouse::Button::Left)
            };
            let (source, messages, statuses) =
                self.batch(&mut input, &[iced::Event::Mouse(button)])?;
            viewport_button_messages(&messages, &statuses, down, &pick)?;
            for message in messages {
                self.send(&source, message)?;
            }
            if down {
                self.settle_gesture_pointer()?;
                if popup_press_path(
                    self.popup_release_enabled,
                    &mut (&mut *self, &mut input),
                    |(driver, input)| driver.release_popup_press(input, position),
                    |(driver, input)| driver.diagnose_popup_press(input, position),
                )? {
                    break;
                }
            }
        }
        self.journey.events.push(json!({"kind":"tutorial_object_viewport_click","pick":pick,
            "position":[position.x,position.y],"captured_press_release":true,"scene_pick_probes":self.probes}));
        loop {
            self.observe()?;
            let publication = self.journey.host.publication_for_test().clone();
            if offer_ready(object(&publication)?.input.as_ref())? {
                let snapshot = object(&publication)?;
                ensure!(
                    !snapshot.ground_input,
                    "object ready input overlaps ground input"
                );
                let native_input = snapshot.input.as_ref().unwrap();
                let owner = publication
                    .native_popup
                    .as_ref()
                    .context("object ready popup owner absent")?;
                let metadata: PopupOwner = serde_json::from_value(serde_json::to_value(owner)?)?;
                let menu = publication
                    .menu
                    .as_ref()
                    .context("object ready visible menu absent")?;
                visible_actions(menu, native_input, &metadata, &pick)?;
                let observation = self.observation(&mut input)?;
                let chosen = menu
                    .popup
                    .iter()
                    .enumerate()
                    .find_map(|(index, choice)| {
                        if choice.label == menu.title
                            || menu
                                .popup
                                .iter()
                                .filter(|other| other.label == choice.label)
                                .count()
                                != 1
                        {
                            return None;
                        }
                        leaf(&observation, &choice.label)
                            .ok()
                            .map(|geometry| (index, choice.label.clone(), geometry))
                    })
                    .context("object no uniquely laid-out currently offered native leaf")?;
                let (index, label, (bounds, center)) = chosen;
                let action = native_input.actions[index].clone();
                ensure!(
                    !snapshot
                        .objects
                        .get(&action.target)
                        .context("object action-target identity absent")?
                        .person,
                    "object offered action targets person; no menu shopping"
                );
                let native = walking::native(&publication)?;
                let prior_ids = self
                    .entry
                    .initial
                    .active
                    .iter()
                    .chain(self.entry.initial.pending.iter())
                    .chain(self.entry.initial.previous.iter())
                    .chain(native.active.iter())
                    .chain(native.pending.iter())
                    .chain(native.previous.iter())
                    .map(|action| action.id)
                    .collect();
                self.offer = Some(Offer {
                    cut: Cut::current(self.journey),
                    owner: owner.clone(),
                    input: native_input.clone(),
                    index,
                    label: label.clone(),
                    action: action.clone(),
                    serial: native.serial,
                    prior_ids,
                });
                self.current_offer()?;
                ensure!(
                    native_input.actor == (BOB_OBJECT, self.entry.base.actor_lifetime),
                    "object offer actor lifetime changed"
                );
                self.proof.offer_observed = true;
                self.journey.events.push(
                    json!({"kind":"tutorial_object_offer","cut":Cut::current(self.journey),
                    "owner":owner,"input":native_input,"index":index,"label":label,"action":action,
                    "serial_before_command":native.serial,"heading_is_not_leaf":true}),
                );
                let play_height = self.entry.base.display_size[1]
                    .checked_sub(self.entry.hud_height)
                    .context("object HUD geometry invalid")?
                    as f32;
                ensure!(
                    center.x >= 0.
                        && center.x < self.entry.base.display_size[0] as f32
                        && center.y >= 0.
                        && center.y < play_height
                        && !geometry.layouts.iter().any(|card| card.x > 0.
                            && card.y == 0.
                            && (card.x + card.width - self.entry.base.display_size[0] as f32)
                                .abs()
                                < 0.01
                            && card.height < play_height
                            && card.contains(center)),
                    "object leaf covered by instruction/HUD"
                );
                self.motion(&mut input, center, None)?;
                self.current_offer()?;
                let hover = self.observation(&mut input)?;
                ensure!(
                    hover.interaction == mouse::Interaction::Pointer
                        && leaf(&hover, &label)? == (bounds, center),
                    "object actual leaf hover interaction/draw evidence missing"
                );
                self.proof.hover_qualified = true;
                self.journey.events.push(
                    json!({"kind":"tutorial_object_hover","label":label,"index":index,
                    "bounds":rect(bounds),"position":[center.x,center.y],"interaction":"Pointer",
                    "draw_commands_observed":true,"gpu_raster_or_native_window_proved":false}),
                );
                self.current_offer()?;
                input_gate(
                    self.journey.streams,
                    self.journey.host.frontend_idle_for_test(),
                    walking::native(self.journey.host.publication_for_test())?.children_pending,
                )?;
                let (source, messages, statuses) = self.batch(
                    &mut input,
                    &[
                        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                    ],
                )?;
                choice_messages(&messages, &statuses, index)?;
                self.current_offer()?;
                for message in messages {
                    self.send(&source, message)?;
                }
                break;
            }
            self.frame(false)?;
        }
        loop {
            self.observe()?;
            ensure!(
                self.journey.host.publication_for_test().playing,
                "object playback stopped before next instruction; no automatic resume"
            );
            self.frame(true)?;
        }
    }
}

pub(super) fn run(journey: &mut Journey, prior: &Record) -> Result<Value> {
    let popup_release_enabled = object_popup_release_enabled_with(|name| std::env::var(name))?;
    let entry = match Entry::capture(journey, prior) {
        Ok(entry) => entry,
        Err(error) => {
            journey
                .events
                .push(json!({"kind":"tutorial_object_stop","status":"unsupported",
                "reason":format!("{error:#}"),"offer_observed":false,"command_sent":false,
                "enqueue_accepted":false,"active_seen":false,"native_completion_observed":false,
                "registered_streams":journey.streams,"total_registered_tasks":journey.tasks,
                "host_idle":journey.host.frontend_idle_for_test(),"response_sent":false}));
            return Err(error);
        }
    };
    let start = Usage::current(journey);
    journey.events.push(json!({"kind":"tutorial_object_start","instruction":entry.instruction,
        "cut":entry.cut,"initial":entry.initial,"registered_streams":entry.retained_streams,
        "total_registered_tasks":entry.retained_tasks,
        "limits":{"combined_frames_service":128,"effect_polls":512,"nonempty_task_streams":128,
            "commands":256,"cumulative_scene_pick_probes":64,"native_ticks":TICK_LIMIT,
            "wall_seconds":WALL_SECONDS,"within_remaining_global_time":true,"object_clicks":1,"leaf_activations":1}}));
    let mut driver = Driver {
        journey,
        entry,
        start,
        wall: Instant::now(),
        probes: 0,
        opened: false,
        pick: None,
        offer: None,
        accepted: None,
        proof: Proof::default(),
        next: None,
        boundary_valid: false,
        popup_press_diagnostic: None,
        redraw_source: None,
        popup_release_enabled,
        popup_release_attempted: false,
        diagnostic: popup_release_enabled.then(|| Box::new(Diagnostic::default())),
    };
    let result = driver.execute();
    if driver.journey.streams != 0 || !driver.journey.host.frontend_idle_for_test() {
        driver.journey.idle = false;
    }
    let boundary = driver.boundary_valid
        && result
            .as_ref()
            .err()
            .is_some_and(|error| error.to_string() == BOUNDARY);
    let publication = driver.journey.host.publication_for_test();
    let mut receipt = json!({"kind":"tutorial_object_stop",
        "status":if driver.popup_press_diagnostic.is_some() {"diagnostic_popup_on_press_no_release"}
            else if boundary && driver.proof.enqueue_accepted {"accepted_enqueue_first_new_instruction_boundary"}
            else if driver.proof.command_sent || driver.next.is_some() {"partial"} else {"unsupported"},
        "reason":result.as_ref().err().map(|error| format!("{error:#}")),"proof":driver.proof,
        "popup_press_diagnostic":driver.popup_press_diagnostic,
        "usage":driver.usage().ok(),"pick":driver.pick,"accepted_action":driver.accepted,
        "next_instruction_or_modal":driver.next,"persistent_owners_valid_at_boundary":driver.boundary_valid,
        "native_success_qualified":driver.proof.enqueue_accepted && driver.proof.native_completion_observed,
        "terminal_native":walking::native(publication).ok(),
        "terminal_object":object(publication).ok(),
        "retained_task_ownership":{"registered_streams":driver.journey.streams,"total_registered_tasks":driver.journey.tasks,
            "journey_idle":driver.journey.idle,"host_idle":driver.journey.host.frontend_idle_for_test(),
            "children_pending":walking::native(publication).ok().map(|native| native.children_pending),
            "cleanup":"same Journey retains executor/channel/Host/tasks; no cancellation, forced drain, synthetic wake or recovery"},
        "entry_response_sent":false,"next_response_sent":false,"arrival_qualified":false,"betty_qualified":false,
        "household_switching_qualified":false,"tutorial_completion_qualified":false,
        "generic_journey_continuation_permitted":false,"strict_original_parity_ticks":0});
    if let Some(diagnostic) = driver.diagnostic.as_ref() {
        receipt["progress_diagnostic"] = diagnostic.receipt(
            driver.usage().ok(), driver.entry.retained_streams, driver.journey.streams,
        );
    }
    driver.journey.events.push(receipt.clone());
    if boundary {
        Ok(receipt)
    } else {
        result?;
        anyhow::bail!("object stopped without next instruction")
    }
}

#[cfg(test)]
#[path = "object_interaction/tests.rs"]
mod tests;
