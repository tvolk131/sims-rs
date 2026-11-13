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
    let frontend = view::Frontend {
        data: Arc::new(before),
        diagnostics: false,
        options_open: false,
        widget_epoch: 0,
    };
    drop(frontend.view());
}

#[test]
fn window_observation_does_not_invent_unsupported_completion_or_readiness() {
    let args = Args::parse_from(["sims-room", "--original", "UserData/Houses/House05.iff"]);
    let (app, task) = Viewer::new(args);
    drop(task);
    let observation = window_observation_value(&app);
    assert!(observation["terminals"].is_null());
    assert!(observation["hud_transport_ready"].is_null());
    assert!(observation["hud_normal_ready"].is_null());
    assert_eq!(
        observation["observation_capabilities"],
        serde_json::json!({
            "action_terminals": false,
            "route_terminals": false,
            "transport_readiness": false
        })
    );
    assert!(app.session.is_none() && app.client.is_none() && app.device.is_none());
    assert!(capture_window_observation(&app).is_none());
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

#[test]
fn captured_loaded_input_metadata_stays_immutable_across_later_work_and_snapshot_closure() {
    // Authored UNRUN helper coverage: copied observation JSON stays immutable
    // as accounting changes. This does not exercise Published::capture or draw;
    // the separate loaded-owner Host control covers retained publications.
    let account = window_session::loaded_input::Account::default();
    let args = Args::parse_from(["sims-room", "--paused"]);
    let (mut app, task) = Viewer::new(args); drop(task);
    app.loaded_input = Some(account.register());
    let before = window_observation_value(&app);
    let work = app.owner_work().unwrap();
    let during = window_observation_value(&app);
    assert_eq!(before["loaded_input"]["unresolved_in_flight"], false);
    assert_eq!(during["loaded_input"]["unresolved_in_flight"], true);
    let terminal = account.terminal(); work.complete(true);
    assert_eq!(before["loaded_input"]["unresolved_in_flight"], false);
    assert_eq!(before["loaded_input"]["work_ordinal"], 0);
    assert_eq!(during["loaded_input"]["work_ordinal"], 1);
    assert_eq!(terminal["unresolved_in_flight"], true);
}
