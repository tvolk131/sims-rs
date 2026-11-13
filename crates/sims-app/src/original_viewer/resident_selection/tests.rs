//! Authored source-only regression tests. ALL UNRUN under the worker contract.
//! Detached publication facts and actual Iced widgets; no private game assets,
//! fake simulation, tutorial acknowledgements, loader tasks or engine execution.
use super::*;
use super::super::{headless_input::Input, neighborhood_usability};
use ui_publication::{Published, Resident, QueueAction, QueueEntry};
use ui_publication::view::Frontend;
use std::sync::Arc;

fn resident(object: i16, name: &str, enabled: bool) -> Resident {
    Resident { object, neighbor: object + 40, name: name.into(), enabled, highlighted: false }
}
fn identity(object: i16) -> Identity {
    Identity { object, neighbor: object + 40, guid: object as u32 + 1000,
        source: format!("authored/resident-{object}.iff"), lifetime: object as u32 + 100 }
}
fn snapshot(state: Selection) -> Snapshot {
    Snapshot { state, blocked: false, identities: [(4, identity(4)), (5, identity(5))].into() }
}
fn blank_needs() -> sims_render::dialog::client::live_panel::Panel {
    serde_json::from_value(serde_json::json!({
        "tick": 12, "person": null, "state": 1, "shown": true,
        "popup": null, "invalidations": 0
    })).unwrap()
}

fn frontend() -> Frontend {
    // A complete detached publication, with no Viewer construction or loader
    // task. Keep engine/asset owners out of the widget fixtures too.
    Frontend {
        data: Arc::new(Published {
            player_shortcuts_blocked: false,
            camera_shortcuts_blocked: true,
            load_picker_blocked: true,
            load_seed: std::path::PathBuf::new(),
            application: neighborhood_usability::Snapshot {
                provenance: "synthetic".into(),
                neighborhood: None,
                lot: None,
                count: None,
                manifest_sha256: None,
                routing: None,
                buy_frontend: None,
                requested_policy: None,
                requested_buy_preview: false,
                requested_inferred_routing: false,
                notice: String::new(),
            },
            scene: None,
            art: None,
            cluster: Some(Arc::new(hud::cluster::Published {
                art: None,
                values: hud::cluster::Values {
                    pending: false, ready: true, mode_raw: Some(2),
                    panel_state_raw: Some(0), can_show_live: true, can_show_buy: false,
                    floor: 1, native_clock: false, pause_raw: None, people_visible: true,
                    clock: String::new(), funds: String::new(), buttons: Default::default(),
                },
            })),
            widget_epoch: 7,
            title: String::new(),
            session_present: true,
            cursor_idle: false,
            native_clock: false,
            native_transport: false,
            native_speed: None,
            playing: false,
            speed: 1.,
            zoom: 1.,
            device_zoom: None,
            pending: false,
            modal: false,
            pause: None,
            paused_indicator: false,
            people_visible: true,
            panel_state: None,
            active_panel: None,
            root_visible: true,
            controls: vec![],
            pause_button: None,
            can_buy: false,
            hud_height: 0,
            tutorial_highlight: None,
            status: String::new(),
            save_load_feedback: None,
            error: None,
            policy: Default::default(),
            resident: 4,
            resident_selection: snapshot(Selection::Unselected),
            residents: vec![resident(4, "Bob", true), resident(5, "Betty", true)],
            money: None,
            clock_fallback: None,
            funds: String::new(),
            needs: Some(blank_needs()),
            legacy_motives: None,
            mood: None,
            menu: None,
            queue: vec![QueueEntry {
                action: QueueAction { resident: 4, resident_lifetime: Some(104),
                    serial: 71, target: 17, active: true, cancel_requested: false },
                label: "Read".into(), diagnostic_label: "Read".into(),
                target: "Bookcase".into(), image: None,
            }],
            catalog: None,
            furniture: None,
            tick_label: Some("Tick 12".into()),
            dialog: None,
            native_popup: None,
            native_popup_epoch: 0,
            window_observation: None,
            audit: None,
            native_selection: None,
            journey_targets: BTreeMap::new(),
            journey_camera: None,
            journey_tutorial_buttons: None,
            journey_camera_watch: None,
            journey_ground: None,
            journey_walking: None,
            journey_object: None,
        }),
        diagnostics: false,
        options_open: false,
        widget_epoch: 7,
    }
}

fn data(frontend: &mut Frontend) -> &mut Published {
    Arc::make_mut(&mut frontend.data)
}
fn cluster(frontend: &mut Frontend) -> &mut hud::cluster::Values {
    &mut Arc::make_mut(data(frontend).cluster.as_mut().unwrap()).values
}
fn ids(frontend: &Frontend) -> Vec<i16> {
    actions(frontend, true).iter().map(|action| action.identity.object).collect()
}

#[test]
fn resident_selection_projection_distinguishes_absence_error_stale_and_noncanonical() {
    assert_eq!(Selection::project(true, Ok(None), |_| panic!("no selected ID")), Selection::Unselected);
    for query in [Err("native flags absent".into()), Ok(Some(0)), Ok(Some(-1)), Ok(Some(99))] {
        assert_eq!(Selection::project(true, query, |id| id == 5), Selection::Unavailable);
    }
    assert_eq!(Selection::project(true, Ok(Some(5)), |id| id == 5), Selection::Selected(5));
    for query in [Ok(None), Ok(Some(5)), Err("missing registry".into())] {
        assert_eq!(Selection::project(false, query, |_| panic!("noncanonical owner")), Selection::Unavailable);
    }
    assert_eq!(Snapshot::default().state, Selection::Unavailable);
}

#[test]
fn resident_selection_unselected_ignores_fallback_and_offers_only_live_enabled_names() {
    let mut frontend = frontend();
    let published = data(&mut frontend);
    published.residents.extend([
        resident(6, "Disabled", false), resident(7, "Historical", true),
        resident(8, "   ", true), resident(9, "Replaced neighbor", true),
    ]);
    for id in [6, 8, 9] { published.resident_selection.identities.insert(id, identity(id)); }
    published.residents.last_mut().unwrap().neighbor += 1;
    let before = published.resident;
    assert_eq!(ids(&frontend), [4, 5]);
    assert_eq!(actions(&frontend, true).iter().map(|a| a.label.as_str()).collect::<Vec<_>>(),
        ["Select Bob", "Select Betty"]);
    for resident in &frontend.residents {
        assert!(!frontend.resident_selection.state.is_selected(resident.object));
        assert_ne!(portrait_style(frontend.resident_selection.state, resident.object, button::Status::Active)
            .border.color, Color::from_rgb8(0, 235, 235));
    }
    assert_eq!(frontend.resident, before);
}

#[test]
fn resident_selection_selected_border_follows_native_id_and_unavailable_makes_no_claim() {
    let mut frontend = frontend();
    data(&mut frontend).resident_selection.state = Selection::Selected(5);
    assert_eq!(frontend.resident, 4);
    assert!(!frontend.resident_selection.state.is_selected(4));
    assert!(frontend.resident_selection.state.is_selected(5));
    assert_eq!(portrait_style(frontend.resident_selection.state, 5, button::Status::Active)
        .border.color, Color::from_rgb8(0, 235, 235));
    assert!(overlay(&frontend, true).is_none());
    let published = data(&mut frontend);
    published.resident_selection.state = Selection::Unavailable;
    published.residents[0].highlighted = true;
    assert!(!frontend.resident_selection.state.is_selected(4));
    assert!(!frontend.resident_selection.state.is_selected(5));
    assert!(overlay(&frontend, true).is_none());
}

#[test]
fn resident_selection_highlighted_enabled_selected_portrait_still_offers_activation() {
    let mut frontend = frontend();
    let published = data(&mut frontend);
    published.resident_selection.state = Selection::Selected(4);
    published.residents[0].highlighted = true;
    assert_eq!(ids(&frontend), [4]);
    assert_eq!(actions(&frontend, true)[0].label, "Select Bob");
    data(&mut frontend).residents[0].enabled = false;
    assert!(overlay(&frontend, true).is_none());
}

fn menu() -> ui_publication::Menu {
    ui_publication::Menu {
        title: "Move here".into(), diagnostic_title: "Move here".into(),
        preview: false, ready: true, diagnostic: vec![], popup: vec![],
        cancel: None, error: None, popup_visible: true,
    }
}
fn dialog() -> sims_vm::dialog::Record {
    serde_json::from_value(serde_json::json!({
        "id": 1, "caller": 4, "origin": {"source": "authored", "bhav": 1, "instruction": 0},
        "source": "authored", "operands": [0,0,0,0,0,0,0,0], "stack_object": 4,
        "title": "Question", "body": "Answer", "buttons": [], "icon_object": null,
        "job_icon": null, "bitmap_icon": null, "status": 1
    })).unwrap()
}
fn gates() -> Vec<(&'static str, fn(&mut Frontend))> {
    vec![
        ("diagnostics", |f| f.diagnostics = true),
        ("Options", |f| f.options_open = true),
        ("no session", |f| data(f).session_present = false),
        ("owner input/picker/registry block", |f| data(f).resident_selection.blocked = true),
        ("pending presentation", |f| data(f).pending = true),
        ("modal", |f| data(f).modal = true),
        ("error", |f| data(f).error = Some("failed".into())),
        ("hidden root", |f| data(f).root_visible = false),
        ("hidden people", |f| data(f).people_visible = false),
        ("collapsed panel", |f| data(f).panel_state = Some(1)),
        ("Buy panel", |f| data(f).active_panel = Some(Panel::Buy)),
        ("Options panel", |f| data(f).active_panel = Some(Panel::Options)),
        ("disabled Live control", |f| data(f).controls.push((Widget::Live,
            sims_render::camera_device::panels::ButtonView { enabled: false, selected: true, auxiliary: 0 }))),
        ("missing cluster", |f| data(f).cluster = None),
        ("cluster not ready", |f| cluster(f).ready = false),
        ("cluster pending", |f| cluster(f).pending = true),
        ("cluster collapsed", |f| cluster(f).panel_state_raw = Some(1)),
        ("cluster hidden people", |f| cluster(f).people_visible = false),
        ("Buy mode", |f| cluster(f).mode_raw = Some(1)),
        ("Build mode", |f| cluster(f).mode_raw = Some(0)),
        ("unknown mode", |f| cluster(f).mode_raw = None),
        ("native menu", |f| data(f).menu = Some(menu())),
        ("native popup owner", |f| data(f).native_popup = Some(serde_json::from_value(serde_json::json!({
            "kind": "Ground", "tick": 12, "actor": [4,104], "target": [17,117],
            "clicked": null, "saved_poll_messages": 1, "saved_stopping": 0, "command": null
        })).unwrap())),
        ("dialog overlay", |f| data(f).dialog = Some(dialog())),
        ("Needs detail popup", |f| data(f).needs = Some(serde_json::from_value(serde_json::json!({
            "tick": 12, "person": null, "state": 1, "shown": true,
            "popup": 0, "invalidations": 0
        })).unwrap())),
        ("feedback overlay", |f| data(f).save_load_feedback = Some("Saved".into())),
    ]
}

#[test]
fn resident_selection_all_overlay_and_owner_gates_refuse_guidance() {
    assert!(overlay(&frontend(), true).is_some());
    assert!(overlay(&frontend(), false).is_none(), "Host picker/replacement/closing gate");
    for (name, block) in gates() {
        let mut frontend = frontend();
        block(&mut frontend);
        assert!(actions(&frontend, true).is_empty(), "{name}");
        assert!(overlay(&frontend, true).is_none(), "{name}");
    }
}

#[test]
fn resident_selection_publication_rendering_preserves_fallback_needs_and_queue_facts() {
    let frontend = frontend();
    let before = serde_json::to_value(frontend.needs.as_ref().unwrap()).unwrap();
    let queue = frontend.queue[0].action;
    let identities = frontend.resident_selection.identities.clone();
    for _ in 0..4 {
        // Construct both real and diagnostic trees without an engine/loader.
        drop(frontend.view());
        drop(frontend.selection_widgets_for_test(0));
        drop(frontend.selection_widgets_for_test(1));
        drop(frontend.selection_widgets_for_test(2));
        assert_eq!(frontend.resident, 4);
        assert_eq!(frontend.queue[0].action, queue);
        assert_eq!(frontend.queue[0].action.resident, frontend.resident);
        assert!(frontend.needs.as_ref().unwrap().person().is_none());
        assert_eq!(serde_json::to_value(frontend.needs.as_ref().unwrap()).unwrap(), before);
        assert_eq!(frontend.resident_selection.identities, identities);
        assert_eq!(frontend.resident_selection.state, Selection::Unselected);
        assert!(frontend.native_selection.is_none(), "separate diagnostic field is preserved");
    }
}

#[test]
fn resident_selection_needs_person_remains_its_own_authority() {
    let mut frontend = frontend();
    let published = data(&mut frontend);
    published.resident_selection.state = Selection::Selected(5);
    published.needs = Some(serde_json::from_value(serde_json::json!({
        "tick": 12, "person": { "object": 4, "guid": 1004, "cache_index": 0,
            "neighbor": 44, "name": "Needs owner", "values": [0,0,0,0,0,0,0,0] },
        "state": 1, "shown": true, "popup": null, "invalidations": 0
    })).unwrap());
    let before = serde_json::to_value(frontend.needs.as_ref().unwrap()).unwrap();
    drop(frontend.selection_widgets_for_test(1));
    drop(frontend.view());
    assert_eq!(frontend.needs.as_ref().unwrap().person().unwrap().object, 4);
    assert_eq!(frontend.resident_selection.state, Selection::Selected(5));
    assert_eq!(frontend.queue[0].action.resident, 4);
    assert_eq!(serde_json::to_value(frontend.needs.as_ref().unwrap()).unwrap(), before);
}

/// Parent-only acceptance hook for an existing real, already-loaded fixture.
/// UNRUN here. This supplies neither an engine nor a loader, and makes no
/// claim about startup, native activation or tutorial completion.
pub(crate) fn assert_capture_is_pure_for_test(app: &Viewer) -> anyhow::Result<()> {
    fn owners(app: &Viewer) -> anyhow::Result<serde_json::Value> {
        let session = app.session.as_ref().ok_or_else(|| anyhow::anyhow!("loaded fixture required"))?;
        Ok(serde_json::json!({
            "session": session.save(),
            "client": app.client.as_ref().map(|client| client.save(session)).transpose()?,
            "device": app.device.as_ref().map(|device| device.save(session)).transpose()?,
            "resident": app.resident, "floor": app.floor, "zoom": app.zoom,
            "selected": app.selected, "tutorial_highlight": app.tutorial_highlight,
            "move_inputs": app.move_inputs, "viewport_enter_requested": app.viewport_enter_requested
        }))
    }
    let before = owners(app)?;
    let session = app.session.as_ref().unwrap();
    let query = session.simulation.native_selected_person();
    for _ in 0..4 {
        let snapshot = Snapshot::capture(app);
        assert_eq!(snapshot.state, Selection::project(canonical(session), query.clone(), |id| {
            live_resident(session, id).is_some()
        }));
        let published = Published::capture(app);
        assert_eq!(published.resident, app.resident);
        assert_eq!(published.resident_selection.state, snapshot.state);
        assert_eq!(serde_json::to_value(&published.needs)?,
            serde_json::to_value(app.client.as_ref().and_then(|client| client.live_panel()))?);
        drop(Frontend { data: Arc::new(published), diagnostics: app.diagnostics,
            options_open: app.options_open, widget_epoch: app.widget_epoch }.view());
        drop(app.view());
        assert_eq!(owners(app)?, before, "capture/render changed a retained owner");
    }
    Ok(())
}

const SIZE: [u32; 2] = [800, 600];
fn helper(frontend: &Frontend) -> Element<'static, Message> {
    widget_lifetimes::retain(frontend.widget_epoch,
        overlay(frontend, true).unwrap_or_else(|| widget::Space::new().into()))
}
fn point(input: &mut Input, element: Element<'_, Message>, label: &str) -> iced::Point {
    let observation = input.element_observation(element, SIZE).unwrap();
    let bounds = observation.texts.iter().find(|item| item.text == label).unwrap().bounds;
    iced::Point::new(bounds.x + bounds.width / 2., bounds.y + bounds.height / 2.)
}
fn moved(point: iced::Point) -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::CursorMoved { position: point })
}
fn down() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
}
fn up() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left))
}
fn send(input: &mut Input, element: Element<'_, Message>, events: &[iced::Event]) -> Vec<i16> {
    input.element_batch(element, SIZE, events).messages.into_iter().map(|message| match message {
        Message::Resident(id) => id,
        _ => panic!("selection helper emitted a different command"),
    }).collect()
}

#[test]
#[ignore = "UNRUN; opt-in authored headless Iced GPU widgets; no private assets"]
fn resident_selection_widgets_emit_exact_existing_message_and_disabled_emits_none() -> anyhow::Result<()> {
    let mut input = Input::new()?;
    let mut frontend = frontend();
    let location = point(&mut input, helper(&frontend), "Select Betty");
    assert_eq!(send(&mut input, helper(&frontend), &[moved(location), down(), up(), up()]), [5]);
    assert_eq!(frontend.resident, 4, "click emission alone cannot select or mutate bookkeeping");
    data(&mut frontend).resident_selection.state = Selection::Selected(5);
    data(&mut frontend).residents[1].highlighted = true;
    input.reset();
    let location = point(&mut input, helper(&frontend), "Select Betty");
    assert_eq!(send(&mut input, helper(&frontend), &[moved(location), down(), up()]), [5],
        "already-selected tutorial portrait still invokes the actual activation handler");
    input.reset();
    let action = actions(&frontend, true).remove(0);
    let location = point(&mut input, control(&action, false), "Select Betty");
    assert!(send(&mut input, control(&action, false), &[moved(location), down(), up()]).is_empty());
    Ok(())
}

#[test]
#[ignore = "UNRUN; opt-in authored headless Iced GPU widgets; no private assets"]
fn resident_selection_widgets_end_replaced_reordered_disabled_and_blocked_gestures() -> anyhow::Result<()> {
    let mut input = Input::new()?;
    for change in 0..7 {
        input.reset();
        let mut frontend = frontend();
        let location = point(&mut input, helper(&frontend), "Select Bob");
        assert!(send(&mut input, helper(&frontend), &[moved(location), down()]).is_empty());
        let published = data(&mut frontend);
        match change {
            0 => published.residents.swap(0, 1),
            1 => { published.residents.remove(0); }
            2 => published.resident_selection.identities.get_mut(&4).unwrap().lifetime = 204,
            3 => published.resident_selection.identities.get_mut(&4).unwrap().guid += 1,
            4 => published.resident_selection.identities.get_mut(&4).unwrap().source.push_str(".replaced"),
            5 => published.residents[0].enabled = false,
            6 => published.pending = true,
            _ => unreachable!(),
        }
        assert!(send(&mut input, helper(&frontend), &[up()]).is_empty(), "replacement {change}");
    }
    for (name, block) in gates() {
        input.reset();
        let mut frontend = frontend();
        let location = point(&mut input, helper(&frontend), "Select Bob");
        assert!(send(&mut input, helper(&frontend), &[moved(location), down()]).is_empty());
        block(&mut frontend);
        assert!(send(&mut input, helper(&frontend), &[up()]).is_empty(), "{name}");
    }
    // A normal publication of the same control preserves an intentional hold.
    input.reset();
    let mut frontend = frontend();
    let location = point(&mut input, helper(&frontend), "Select Bob");
    assert!(send(&mut input, helper(&frontend), &[moved(location), down()]).is_empty());
    data(&mut frontend).status = "new ordinary publication".into();
    assert_eq!(send(&mut input, helper(&frontend), &[up()]), [4]);
    Ok(())
}

#[test]
#[ignore = "UNRUN; opt-in authored headless Iced GPU widgets; no private assets"]
fn resident_selection_play_shell_places_guidance_above_hud_and_keeps_blank_needs() -> anyhow::Result<()> {
    let mut input = Input::new()?;
    let frontend = frontend();
    let observation = input.element_observation(frontend.view(), SIZE)?;
    let guidance = observation.texts.iter().find(|item| item.text == "Select a Sim to give commands").unwrap();
    let action = observation.texts.iter().find(|item| item.text == "Select Bob").unwrap();
    assert!(guidance.bounds.y < action.bounds.y);
    assert!(action.bounds.y + action.bounds.height < SIZE[1] as f32 - 183.);
    input.reset();
    let needs = input.element_observation(frontend.selection_widgets_for_test(1), SIZE)?;
    assert!(!needs.texts.iter().any(|item| item.text == "Hunger" || item.text.contains("Select Bob")));
    for shell_enabled in [false, true] {
        input.reset();
        let confirmation = shell_enabled.then_some(neighborhood_usability::Confirmation {
            choice: neighborhood_usability::Choice::BuyPreview, revision: 1,
        });
        let observed = input.element_observation(frontend.view_with_selection_gate(
            confirmation, true, shell_enabled,
        ), SIZE)?;
        assert!(!observed.texts.iter().any(|item| item.text == "Select Bob"));
    }
    Ok(())
}

