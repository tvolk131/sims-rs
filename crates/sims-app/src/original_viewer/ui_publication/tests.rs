use super::*;
use clap::Parser;

#[test]
fn frontend_publication_and_engine_owner_have_the_required_thread_contracts() {
    fn send<T: Send + 'static>() {}
    fn send_sync<T: Send + Sync + 'static>() {}
    send::<Viewer>();
    send::<Task<Message>>();
    send_sync::<Published>();
}

#[test]
fn loading_publication_is_owned_and_does_not_load_or_prepare() {
    let args = Args::parse_from(["sims-room", "--original", "UserData2/Houses/House09.iff"]);
    let (mut app, task) = Viewer::new(args);
    drop(task);
    app.prepare_thumbnail_catalog().unwrap();
    assert!(app.thumbnails.is_none());
    let before = Published::capture(&app);
    app.status = "another loading stage".into();
    app.error = Some("retained error".into());
    app.widget_epoch = 11;
    let after = Published::capture(&app);
    assert!(!before.session_present);
    assert!(before.scene.is_none() && before.art.is_none() && before.cluster.is_none());
    assert!(before.residents.is_empty() && before.catalog.is_none() && before.queue.is_empty());
    assert_eq!(before.status, "Loading original household…");
    assert!(before.error.is_none());
    assert_eq!(after.status, "another loading stage");
    assert_eq!(after.error.as_deref(), Some("retained error"));
    assert!(app.session.is_none() && app.client.is_none() && app.device.is_none());
    let observation = window_observation_value(&app);
    assert_eq!(observation["status"], "another loading stage");
    assert!(observation["application_mode"].is_null());
    assert!(observation["panel_state"].is_null());
    assert!(observation["needs_selected"].is_null());
    assert!(observation["walls"].is_null());
    assert_eq!(observation["people_visible"], true);
    assert_eq!(observation["error"], "retained error");
    let frontend = view::Frontend {
        data: Arc::new(before),
        diagnostics: false,
        options_open: false,
        widget_epoch: 0,
    };
    drop(frontend.view());
}

#[test]
#[ignore = "installed Goth household assets and offscreen renderer"]
fn window_observations_follow_actual_needs_selection_and_wall_owners() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(observe_person_panel_and_walls)?
        .join()
        .expect("window observation test panicked")
}

fn observe_person_panel_and_walls() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House05.iff",
        "--paused",
        "--root",
        root.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut app, _) = Viewer::new(args);
    app.install(ready)?;
    let mut responses = Vec::new();
    super::super::transport_tests::drain(&mut app, &mut responses)?;
    assert_eq!(app.device.as_ref().unwrap().panel_state(), Some(1));
    let _ = app.update(Message::Live);
    super::super::transport_tests::drain(&mut app, &mut responses)?;
    assert_eq!(app.device.as_ref().unwrap().panel_state(), Some(0));
    assert!(app.people_panel_visible());
    let person = app
        .client
        .as_ref()
        .and_then(|client| client.live_panel())
        .and_then(|panel| panel.person())
        .context("displayed person absent")?
        .identity();
    let tick = app.session.as_ref().unwrap().simulation.vm.tick;
    assert_eq!(window_observation_value(&app)["needs_selected"], true);
    let _ = app.update(Message::Job(person));
    super::super::transport_tests::drain(&mut app, &mut responses)?;
    let job = window_observation_value(&app);
    assert_eq!(job["needs_selected"], false);
    let _ = app.update(Message::Needs);
    super::super::transport_tests::drain(&mut app, &mut responses)?;
    assert_eq!(window_observation_value(&app)["needs_selected"], true);
    assert_eq!(job["needs_selected"], false);
    for walls in [1, 4, 2] {
        let _ = app.update(Message::Walls(walls));
        super::super::transport_tests::drain(&mut app, &mut responses)?;
        assert_eq!(
            window_observation_value(&app)["walls"],
            serde_json::json!({
                "all_walls":u8::from(walls == 2),
                "cutaway":u8::from(walls == 4)
            })
        );
    }
    assert_eq!(app.session.as_ref().unwrap().simulation.vm.tick, tick);
    assert!(app.error.is_none());
    Ok(())
}

#[test]
fn shared_catalog_keys_keep_complete_value_equality() {
    let original = CatalogIdentity {
        revision: 3,
        view: Some(sims_render::camera_device::panels::catalog::View {
            categories: [Default::default(); 8],
            labels: std::array::from_fn(|index| format!("category {index}")),
            heading: "Buy".into(),
            page: 0,
            previous: false,
            next: true,
            products: vec![12],
            subcatalog: 1,
        }),
        products: vec![(
            12,
            42,
            sims_vm::Definition {
                source: "Objects.far::Chair.iff".into(),
                object: 7,
                guid: 42,
                slots: 2,
            },
        )],
    };
    let shared = Arc::new(original.clone());
    assert!(shared == shared.clone());
    assert!(shared == Arc::new(original.clone()));
    let mut changed = original.clone();
    changed.revision += 1;
    assert!(shared != Arc::new(changed));
    for field in 0..6 {
        let mut changed = original.clone();
        match field {
            0 => changed.products[0].0 += 1,
            1 => changed.products[0].1 += 1,
            2 => changed.products[0].2.source.push_str(".replacement"),
            3 => changed.products[0].2.object += 1,
            4 => changed.products[0].2.guid += 1,
            5 => changed.products[0].2.slots += 1,
            _ => unreachable!(),
        }
        assert!(shared != Arc::new(changed));
    }
    for field in 0..9 {
        let mut changed = original.clone();
        let view = changed.view.as_mut().unwrap();
        match field {
            0 => view.categories[0].visible = true,
            1 => view.labels[0].push_str(" changed"),
            2 => view.heading.push_str(" changed"),
            3 => view.page += 1,
            4 => view.previous = true,
            5 => view.next = false,
            6 => view.products[0] += 1,
            7 => view.subcatalog += 1,
            8 => changed.view = None,
            _ => unreachable!(),
        }
        assert!(shared != Arc::new(changed));
    }
}
