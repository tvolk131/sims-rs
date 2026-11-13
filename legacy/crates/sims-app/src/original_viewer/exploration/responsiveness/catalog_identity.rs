//! Actual Buy widgets, ordered production worker calls and native revision guards.
//! Indices refer to the full installed product vector, never a page-cell offset.
use super::super::super::ui_publication::{Published, view::Frontend};
use super::super::super::worker_host::{Event, Host};
use super::*;
use sims_vm::interface::buy::input::Event as Nav;

#[derive(Clone)]
struct Ui {
    source: Arc<Published>,
    message: Message,
    point: [f32; 2],
}
fn catalog(app: &Application) -> Result<&sims_render::camera_device::panels::catalog::Catalog> {
    app.viewer
        .device
        .as_ref()
        .and_then(|d| d.panel_widgets())
        .and_then(|w| w.catalog())
        .context("actual retained catalog absent")
}
fn full(app: &Application) -> Result<String> {
    let s = app.session()?;
    // Same JSON bytes as the streaming engine oracle. A temporary byte vector
    // avoids millions of tiny SHA updates in this debug-only private campaign.
    Ok(engine::digest(&serde_json::to_vec(&(
        s.save(),
        app.viewer
            .client
            .as_ref()
            .context("client absent")?
            .save(s)?,
        app.viewer
            .device
            .as_ref()
            .context("device absent")?
            .save(s)?,
        app.local(),
    ))?))
}
#[test]
fn catalog_audit_byte_hash_matches_the_streaming_full_state_oracle() -> Result<()> {
    let value = (
        vec![0u8, 1, 127, 255].repeat(1000),
        json!({"integer":-3,"float":0.125,"text":"é","none":null}),
    );
    ensure!(engine::digest(&serde_json::to_vec(&value)?) == engine::digest_serialized(&value)?);
    Ok(())
}
fn context(app: &Application) -> Result<Value> {
    let c = catalog(app)?;
    let visible = app
        .viewer
        .device
        .as_ref()
        .and_then(|d| d.panel_widgets())
        .and_then(|w| w.presentation())
        .is_some_and(|p| p.visible(sims_render::camera_device::panels::presentation::Panel::Buy));
    Ok(
        json!({"revision":c.revision(),"visible":visible,"view":c.view(),
        "products":c.view().into_iter().flat_map(|v|v.products.iter()).filter_map(|&i|
            c.product(i).map(|p|json!({"index":i,"definition":p.definition,"source":p.source,
                "name":p.display_name(),"price":p.fields.price}))).collect::<Vec<_>>(),
        "pending":app.viewer.presentation_pending()}),
    )
}
fn acceptance(app: &Application, message: &Message) -> Result<bool> {
    let c = catalog(app)?;
    let visible = context(app)?["visible"] == true;
    Ok(visible
        && !app.viewer.presentation_pending()
        && match message {
            Message::BuyProduct(revision, index) => c.accepts_product(*revision, *index),
            Message::BuyNavigate(revision, event) => c.accepts(*revision, *event),
            _ => anyhow::bail!("not an actual catalog command"),
        })
}
fn find(app: &Application, input: &mut Input, predicate: impl Fn(&Message) -> bool) -> Result<Ui> {
    let before = full(app)?;
    let source = Arc::new(Published::capture(&app.viewer));
    let frontend = Frontend {
        data: source.clone(),
        diagnostics: app.viewer.diagnostics,
        options_open: app.viewer.options_open,
        widget_epoch: app.viewer.widget_epoch,
    };
    input.reset();
    for _ in 0..5 {
        for point in input.batch_centers(&app.viewer) {
            let events = [
                iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                    position: iced::Point::new(point[0], point[1]),
                }),
                iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
                iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                    iced::mouse::Button::Left,
                )),
            ];
            for message in input
                .element_batch(frontend.view(), app.viewer.display_size, &events)
                .messages
            {
                if predicate(&message) {
                    ensure!(
                        full(app)? == before,
                        "passive actual Iced input changed saved state"
                    );
                    return Ok(Ui {
                        source,
                        message,
                        point,
                    });
                }
            }
        }
        let batch = input.element_batch(frontend.view(), app.viewer.display_size, &[]);
        let Some(scroll) = batch
            .owners
            .scrolls
            .iter()
            .find(|s| s.content[3] > s.bounds[3] + 1.)
        else {
            break;
        };
        let point = iced::Point::new(
            scroll.bounds[0] + scroll.bounds[2] * 0.75,
            scroll.bounds[1] + scroll.bounds[3] * 0.5,
        );
        input.element_batch(
            frontend.view(),
            app.viewer.display_size,
            &[
                iced::Event::Mouse(iced::mouse::Event::CursorMoved { position: point }),
                iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                    delta: iced::mouse::ScrollDelta::Lines { x: 0., y: -3. },
                }),
            ],
        );
    }
    anyhow::bail!("requested actual visible/enabled frontend control not found")
}
#[derive(Clone, Copy)]
enum Action {
    None,
    FirstCategory,
    OtherCategory,
    Next,
    Live,
    Buy,
}
fn action(app: &Application, input: &mut Input, action: Action) -> Result<Ui> {
    let chosen = match action {
        Action::FirstCategory => catalog(app)?
            .view()
            .context("catalog view absent")?
            .categories
            .iter()
            .position(|c| c.visible && c.enabled),
        Action::OtherCategory => catalog(app)?
            .view()
            .context("catalog view absent")?
            .categories
            .iter()
            .position(|c| c.visible && c.enabled && !c.selected),
        _ => None,
    };
    find(app, input, |m| match (action, m) {
        (
            Action::FirstCategory | Action::OtherCategory,
            Message::BuyNavigate(_, Nav::Category(i)),
        ) => Some(*i as usize) == chosen,
        (Action::Next, Message::BuyNavigate(_, Nav::Next)) => true,
        (Action::Live, Message::Live) | (Action::Buy, Message::Buy) => true,
        _ => false,
    })
}
fn call(app: &mut Application, host: &mut Host, input: &Ui) -> Result<()> {
    ensure!(
        app.viewer.update(input.message.clone()).units() == 0,
        "unhandled native task effect"
    );
    ensure!(
        host.update(host.ui_event_for_test(&input.source, input.message.clone()))
            .units()
            == 1
    );
    ensure!(host.settle_for_test()?.is_empty());
    app.check_errors()
}
fn frame(app: &mut Application, host: &mut Host) -> Result<()> {
    let now = app.epoch + Duration::from_millis(app.millis);
    ensure!(app.viewer.update(Message::Frame(now)).units() == 0);
    ensure!(host.update(Event::Frame(now)).units() == 1);
    ensure!(host.settle_for_test()?.is_empty());
    app.check_errors()
}
fn drain(app: &mut Application, host: &mut Host) -> Result<usize> {
    for n in 0..64 {
        if !app.viewer.presentation_pending() {
            return Ok(n);
        }
        frame(app, host)?;
    }
    anyhow::bail!("bounded native UI service did not return")
}
#[derive(Clone, Copy)]
enum Late {
    Product,
    Category,
}
#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    root: bool,
    pre_live: bool,
    change: Action,
    late: Late,
    expected: Option<bool>,
}

#[test]
#[ignore = "private Mashuga actual catalog widgets, production worker ordering, revisions and managed cold resume; headless Metal, no GUI/guest"]
fn mashuga_catalog_intents_keep_native_resource_and_panel_identity() -> Result<()> {
    run_worker(campaign)
}
fn campaign() -> Result<()> {
    let out = output()?;
    let mut base = load(&out)?;
    select(&mut base, 196)?;
    let mut input = Input::new()?;
    let buy = find(&base, &mut input, |m| matches!(m, Message::Buy))?;
    base.dispatch(buy.message)?;
    base.drain(&Limits::default())?;
    let root = base.checkpoint()?;
    let category = action(&base, &mut input, Action::FirstCategory)?;
    base.dispatch(category.message)?;
    base.drain(&Limits::default())?;
    ensure!(
        catalog(&base)?.view().context("products absent")?.next,
        "actual first category has no next page"
    );
    let products = base.checkpoint()?;
    let cases = [
        Case {
            name: "root-category-index-after-category",
            root: true,
            pre_live: false,
            change: Action::FirstCategory,
            late: Late::Category,
            expected: Some(false),
        },
        Case {
            name: "product-after-next-page",
            root: false,
            pre_live: false,
            change: Action::Next,
            late: Late::Product,
            expected: Some(false),
        },
        Case {
            name: "category-after-next-page",
            root: false,
            pre_live: false,
            change: Action::Next,
            late: Late::Category,
            expected: Some(false),
        },
        Case {
            name: "product-after-category",
            root: false,
            pre_live: false,
            change: Action::OtherCategory,
            late: Late::Product,
            expected: Some(false),
        },
        Case {
            name: "product-after-live",
            root: false,
            pre_live: false,
            change: Action::Live,
            late: Late::Product,
            expected: Some(false),
        },
        Case {
            name: "category-after-live",
            root: false,
            pre_live: false,
            change: Action::Live,
            late: Late::Category,
            expected: Some(false),
        },
        Case {
            name: "product-after-live-buy",
            root: false,
            pre_live: true,
            change: Action::Buy,
            late: Late::Product,
            expected: None,
        },
        Case {
            name: "same-owner-product-after-frame",
            root: false,
            pre_live: false,
            change: Action::None,
            late: Late::Product,
            expected: Some(true),
        },
        Case {
            name: "same-owner-category-after-frame",
            root: false,
            pre_live: false,
            change: Action::None,
            late: Late::Category,
            expected: Some(true),
        },
    ];
    let filter = std::env::var("SIMS_CATALOG_IDENTITY_CASE").ok();
    let mut rows = vec![];
    for case in cases {
        if filter.as_ref().is_some_and(|f| f != case.name) {
            continue;
        }
        write_json(
            &out.join("catalog-identity-progress.json"),
            &json!({"case":case.name,"completed":rows.len()}),
        )?;
        let bytes = if case.root { &root } else { &products };
        let mut reference = base.restore(bytes)?;
        let late = match case.late {
            Late::Product => find(&reference, &mut input, |m| {
                matches!(m, Message::BuyProduct(..))
            })?,
            Late::Category => action(&reference, &mut input, Action::FirstCategory)?,
        };
        let original = context(&reference)?;
        let mut cold = reference.restore(bytes)?;
        let mut host = Host::from_viewer_for_test(cold.viewer);
        if case.pre_live {
            let live = action(&reference, &mut input, Action::Live)?;
            call(&mut reference, &mut host, &live)?;
            drain(&mut reference, &mut host)?;
        }
        let mut final_polls: usize = 0;
        let mut earlier = None;
        if !matches!(case.change, Action::None) {
            let change = action(&reference, &mut input, case.change)?;
            let mut probe = reference.restore(&reference.checkpoint()?)?;
            probe.dispatch(change.message.clone())?;
            for n in 0..64 {
                if !probe.viewer.presentation_pending() {
                    final_polls = n;
                    break;
                }
                let now = probe.epoch + Duration::from_millis(probe.millis);
                probe.dispatch(Message::Frame(now))?;
                ensure!(n != 63, "native transition probe did not return");
            }
            earlier = Some(
                json!({"message":format!("{:?}",change.message),"point":change.point,"completion_polls":final_polls}),
            );
            call(&mut reference, &mut host, &change)?;
            for _ in 0..final_polls.saturating_sub(1) {
                frame(&mut reference, &mut host)?;
            }
        }
        cold.viewer = host.recover_for_test();
        ensure!(
            full(&reference)? == full(&cold)?,
            "{}: ordered transition differs before final Frame",
            case.name
        );
        let cut = reference.checkpoint()?;
        cold = reference.restore(&cut)?;
        ensure!(
            full(&reference)? == full(&cold)?,
            "{}: cold pre-Frame cut differs",
            case.name
        );
        host = Host::from_viewer_for_test(cold.viewer);
        let now = reference.epoch + Duration::from_millis(reference.millis);
        let (entered, resume) = host.gate_next_for_test();
        ensure!(host.update(Event::Frame(now)).units() == 1);
        entered.recv_timeout(Duration::from_secs(5))?;
        ensure!(
            host.update(host.ui_event_for_test(&late.source, late.message.clone()))
                .units()
                == 0
        );
        ensure!(reference.viewer.update(Message::Frame(now)).units() == 0);
        reference.check_errors()?;
        ensure!(
            !reference.viewer.presentation_pending(),
            "{}: final Frame did not complete the actual transition",
            case.name
        );
        let current = context(&reference)?;
        let accepted = acceptance(&reference, &late.message)?;
        let before = full(&reference)?;
        ensure!(reference.viewer.update(late.message.clone()).units() == 0);
        reference.check_errors()?;
        let after = full(&reference)?;
        resume.send(())?;
        ensure!(host.settle_for_test()?.is_empty());
        cold.viewer = host.recover_for_test();
        let same = full(&cold)? == after;
        let expected = case.expected.is_none_or(|expected| accepted == expected);
        let row = json!({"name":case.name,"passed":same&&expected,
            "original":original,"after_earlier_ordered_input":current,"earlier":earlier,
            "late":{"message":format!("{:?}",late.message),"point":late.point},
            "native_guard_accepts":accepted,"complete_state_rng_continuations_equal":same,
            "complete_state_unchanged_by_late_handler":before==after,"post_input_state_sha256":after,
            "pre_frame_managed_cold_equal":true,"final_frame_completes_transition":final_polls>0});
        rows.push(row);
        write_json(&out.join("catalog-identity-cases.json"), &rows)?;
        if !same || !expected {
            std::fs::write(out.join(format!("{}-before-frame.json", case.name)), &cut)?;
            anyhow::bail!("{}: preserved actual catalog identity failure", case.name);
        }
        // Resume the actual accepted/rejected native caller from an independent
        // managed save, then compare with the warm unchanged handler's return.
        let post = reference.checkpoint()?;
        cold = reference.restore(&post)?;
        ensure!(
            full(&reference)? == full(&cold)?,
            "post-input cold state differs"
        );
        host = Host::from_viewer_for_test(cold.viewer);
        let polls = drain(&mut reference, &mut host)?;
        cold.viewer = host.recover_for_test();
        ensure!(
            full(&reference)? == full(&cold)?,
            "{}: cold native return differs",
            case.name
        );
        if accepted && matches!(case.late, Late::Product) {
            let Message::BuyProduct(_, index) = late.message else {
                unreachable!()
            };
            ensure!(
                catalog(&reference)?.selected(index),
                "actual valid resource did not become selected"
            );
        }
        rows.last_mut().unwrap()["post_input_managed_cold_equal"] = json!(true);
        rows.last_mut().unwrap()["cold_return_equal"] = json!(true);
        rows.last_mut().unwrap()["cold_return_polls"] = json!(polls);
        write_json(&out.join("catalog-identity-cases.json"), &rows)?;
    }
    ensure!(!rows.is_empty(), "no bounded catalog case selected");
    write_json(
        &out.join("catalog-identity-result.json"),
        &json!({"passed":true,"cases":rows.len(),
        "production_transport_changed":false,"original_engine_parity_ticks":0,
        "scope":"Actual visible Iced controls, existing full-vector product IDs and native revision/visibility guards, ordered production worker Frame and managed cold resume; no synthetic registry, GUI, native guest or player-save mutation."}),
    )
}

#[test]
#[ignore = "private Mashuga actual held catalog cell through an ordered Next transition; headless Iced and managed native return, no GUI/guest"]
fn mashuga_catalog_cell_press_cannot_retarget_a_new_resource() -> Result<()> {
    run_worker(held_cell)
}
fn held_cell() -> Result<()> {
    let out = output()?;
    let mut reference = load(&out)?;
    select(&mut reference, 196)?;
    let mut input = Input::new()?;
    let buy = find(&reference, &mut input, |m| matches!(m, Message::Buy))?;
    reference.dispatch(buy.message)?;
    reference.drain(&Limits::default())?;
    let category = action(&reference, &mut input, Action::FirstCategory)?;
    reference.dispatch(category.message)?;
    reference.drain(&Limits::default())?;
    let product = find(&reference, &mut input, |m| {
        matches!(m, Message::BuyProduct(..))
    })
    .context("held-cell visible product")?;
    let Message::BuyProduct(old_revision, old_index) = product.message else {
        unreachable!()
    };
    let old_definition = catalog(&reference)?
        .product(old_index)
        .context("old product absent")?
        .definition;
    let original = context(&reference)?;
    let mut cold = reference.restore(&reference.checkpoint()?)?;
    let mut host = Host::from_viewer_for_test(cold.viewer);
    let frontend = |host: &Host| Frontend {
        data: host.publication_for_test().clone(),
        diagnostics: false,
        options_open: false,
        widget_epoch: host.widget_epoch_for_test(),
    };
    let movement = iced::Event::Mouse(iced::mouse::Event::CursorMoved {
        position: iced::Point::new(product.point[0], product.point[1]),
    });
    let press = iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left));
    let release = iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
        iced::mouse::Button::Left,
    ));
    input.reset();
    let f = frontend(&host);
    ensure!(
        !input
            .element_batch(
                f.view(),
                reference.viewer.display_size,
                &[movement.clone(), press.clone()]
            )
            .messages
            .iter()
            .any(|m| matches!(m, Message::BuyProduct(..))),
        "product activated on press"
    );
    frame(&mut reference, &mut host)?;
    ensure!(
        catalog(&reference)?.revision() == old_revision,
        "positive ordinary Frame changed the catalog owner"
    );
    let f = frontend(&host);
    ensure!(
        input
            .element_batch(
                f.view(),
                reference.viewer.display_size,
                std::slice::from_ref(&release)
            )
            .messages
            .iter()
            .any(|m| matches!(m,Message::BuyProduct(r,i) if *r==old_revision&&*i==old_index)),
        "same-owner ordinary Frame discarded a valid held product press"
    );
    let next = action(&reference, &mut input, Action::Next).context("held-cell visible Next")?;
    // A real earlier focus event must not consume the independent catalog key
    // reset. Preserve this retained Tree and use the normal delayed handler.
    let f = frontend(&host);
    input.element_batch(
        f.view(),
        reference.viewer.display_size,
        &[
            iced::Event::Window(iced::window::Event::Unfocused),
            iced::Event::Window(iced::window::Event::Focused),
        ],
    );
    ensure!(reference.viewer.update(Message::FocusLost).units() == 0);
    ensure!(
        host.update(Event::Widget {
            generation: 0,
            message: Message::FocusLost
        })
        .units()
            == 1
    );
    ensure!(host.settle_for_test()?.is_empty());
    let mut hud_input = Input::new()?;
    let options = find(&reference, &mut hud_input, |m| {
        matches!(m, Message::Options)
    })?;
    // Next was generated by the real prior visible control. While its worker
    // call is blocked and A is still published, press A's actual product cell.
    let (entered, resume) = host.gate_next_for_test();
    ensure!(
        host.update(host.ui_event_for_test(&next.source, next.message.clone()))
            .units()
            == 1
    );
    entered.recv_timeout(Duration::from_secs(5))?;
    // The real Next control is below the product rows. Scroll its retained
    // actual Iced viewport back to the product before pressing; do not forge
    // private scroll state or assume the navigation row was initially visible.
    let f = frontend(&host);
    let owners = input
        .element_batch(f.view(), reference.viewer.display_size, &[])
        .owners;
    let scroll = owners
        .scrolls
        .iter()
        .find(|s| s.content[3] > s.bounds[3] + 1.)
        .context("catalog scroll owner absent")?;
    let scroll_point = iced::Point::new(
        scroll.bounds[0] + scroll.bounds[2] * 0.75,
        scroll.bounds[1] + scroll.bounds[3] * 0.5,
    );
    let f = frontend(&host);
    input.element_batch(
        f.view(),
        reference.viewer.display_size,
        &[
            iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                position: scroll_point,
            }),
            iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { x: 0., y: 20. },
            }),
        ],
    );
    let f = frontend(&host);
    input.element_batch(f.view(), reference.viewer.display_size, &[movement, press]);
    let f = frontend(&host);
    hud_input.element_batch(
        f.view(),
        reference.viewer.display_size,
        &[
            iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                position: iced::Point::new(options.point[0], options.point[1]),
            }),
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
        ],
    );
    ensure!(reference.viewer.update(next.message).units() == 0);
    reference.check_errors()?;
    resume.send(())?;
    ensure!(host.settle_for_test()?.is_empty());
    let mut polls = 0;
    while reference.viewer.presentation_pending() {
        ensure!(
            polls < 64,
            "held-cell native page transition did not return"
        );
        let f = frontend(&host);
        input.element_batch(f.view(), reference.viewer.display_size, &[]);
        let f = frontend(&host);
        hud_input.element_batch(f.view(), reference.viewer.display_size, &[]);
        frame(&mut reference, &mut host)?;
        polls += 1;
    }
    let current = context(&reference)?;
    let before_release = reference.checkpoint()?;
    let publication = host.publication_for_test().clone();
    let f = frontend(&host);
    let released = input
        .element_batch(
            f.view(),
            reference.viewer.display_size,
            std::slice::from_ref(&release),
        )
        .messages;
    let f = frontend(&host);
    ensure!(
        hud_input
            .element_batch(f.view(), reference.viewer.display_size, &[release])
            .messages
            .iter()
            .any(|m| matches!(m, Message::Options)),
        "catalog replacement ended an unrelated HUD press"
    );
    let picked = released.iter().find_map(|m| match m {
        Message::BuyProduct(r, i) => Some((*r, *i)),
        _ => None,
    });
    let actual = picked.and_then(|(r, i)| {
        catalog(&reference)
            .ok()?
            .product(i)
            .map(|p| (r, i, p.definition))
    });
    let retargeted = actual.is_some_and(|(_, _, definition)| definition != old_definition);
    let mut accepted_wrong = false;
    let cold_equal;
    if retargeted {
        let (revision, index, definition) = actual.unwrap();
        let message = Message::BuyProduct(revision, index);
        accepted_wrong = acceptance(&reference, &message)?;
        ensure!(
            accepted_wrong,
            "different emitted resource was already rejected by native guards"
        );
        call(
            &mut reference,
            &mut host,
            &Ui {
                source: publication,
                message,
                point: product.point,
            },
        )?;
        cold.viewer = host.recover_for_test();
        ensure!(
            full(&reference)? == full(&cold)?,
            "actual retargeted native handler differs through worker"
        );
        cold = reference.restore(&reference.checkpoint()?)?;
        ensure!(
            full(&reference)? == full(&cold)?,
            "retargeted pending caller cold restore differs"
        );
        host = Host::from_viewer_for_test(cold.viewer);
        drain(&mut reference, &mut host)?;
        cold.viewer = host.recover_for_test();
        ensure!(
            full(&reference)? == full(&cold)?,
            "retargeted caller cold return differs"
        );
        ensure!(
            catalog(&reference)?.selected(index),
            "retargeted product was not actually selected"
        );
        ensure!(catalog(&reference)?.product(index).unwrap().definition == definition);
        cold_equal = true;
        std::fs::write(out.join("held-cell-before-release.json"), &before_release)?;
    } else {
        cold.viewer = host.recover_for_test();
        ensure!(
            full(&reference)? == full(&cold)?,
            "passive held press changed complete state"
        );
        cold = reference.restore(&reference.checkpoint()?)?;
        ensure!(
            full(&reference)? == full(&cold)?,
            "safe held press cold restore differs"
        );
        host = Host::from_viewer_for_test(cold.viewer);
        drain(&mut reference, &mut host)?;
        cold.viewer = host.recover_for_test();
        ensure!(
            full(&reference)? == full(&cold)?,
            "safe held press cold return differs"
        );
        cold_equal = true;
    }
    write_json(
        &out.join("held-cell-result.json"),
        &json!({"passed":!retargeted,
        "original":original,"current":current,"point":product.point,
        "old":{"revision":old_revision,"index":old_index,"definition":old_definition},
        "release_messages":format!("{released:?}"),"emitted_product":actual,
        "different_resource_emitted":retargeted,"native_guard_accepts_wrong_resource":accepted_wrong,
        "wrong_resource_actually_selected":retargeted&&accepted_wrong,
        "retargeted_native_state_rng_and_cold_equal":cold_equal,
        "same_owner_held_press_survives_frame":true,"page_transition_polls":polls,
        "prior_focus_ack_does_not_consume_catalog_reset":true,"unrelated_hud_press_preserved":true,
        "complete_state_rng_and_cold_equal":cold_equal,
        "original_engine_parity_ticks":0,"scope":"Actual Iced pressed cell while the production owner is blocked in an earlier ordered Next callback, pending and completed native publications, native product callback and managed cold return. No synthetic registry, GUI, native guest or player save."}),
    )?;
    ensure!(
        !retargeted,
        "ended catalog cell press selected a different installed resource"
    );
    Ok(())
}
