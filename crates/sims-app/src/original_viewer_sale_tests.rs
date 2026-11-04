//! Real furniture, actual Iced pickup/Sell input, retained engine children and
//! immutable-only managed restore. Authored input; not original tick parity.
use super::*;
use clap::Parser;
use serde_json::json;
use widget::shader::Program as _;

fn state(v: &Viewer) -> Result<Vec<u8>> {
    let s = v.session.as_ref().context("sale session absent")?;
    Ok(serde_json::to_vec(
        &json!({"session":s.save_managed()?,"client":v.client.as_ref().unwrap().save(s)?,"device":v.device.as_ref().unwrap().save(s)?,"inputs":v.move_inputs,"enter":v.viewport_enter_requested}),
    )?)
}
fn equal(a: &Viewer, b: &Viewer, out: &Path, label: &str) -> Result<()> {
    let x = state(a)?;
    let y = state(b)?;
    if x != y {
        std::fs::write(out.join(format!("{label}-expected.json")), x)?;
        std::fs::write(out.join(format!("{label}-actual.json")), y)?;
        anyhow::bail!("sale full retained state differs at{label}");
    }
    Ok(())
}
///48bce9 kind3 leaves the furniture detached. The initial snapshot's reference
///keeps its lifetime alive until4933dc clears Undo on the normal Live return.
fn retained_sale(v: &Viewer, target: i16, handle: u32, price: i32) -> Result<u32> {
    let s = v.session.as_ref().context("sold session absent")?;
    let w = &s.simulation.vm.world;
    ensure!(
        w.scheduler_storage
            .as_ref()
            .unwrap()
            .object_for_handle(handle)
            == Some(target),
        "sale lost the Undo-owned furniture lifetime"
    );
    ensure!(
        w.objects[&target]
            .native_flags
            .context("placed flags absent")?
            & 1
            == 0,
        "sold furniture remained on the lot"
    );
    ensure!(
        !v.scene
            .as_ref()
            .context("sale assertion needs a published scene")?
            .draws
            .iter()
            .any(|d| d.object == Some(target))
            && object_tests::object_pixel(v, target).is_none(),
        "off-lot sold furniture remained rendered or pickable"
    );
    let m = s.host.presentation.move_tool.as_ref().unwrap();
    let r = &m.history.references;
    let allocation = r
        .by_script_id(target)
        .map_err(anyhow::Error::msg)?
        .context("sale snapshot reference absent")?;
    let reference = &r.records[&allocation];
    ensure!(reference.object == Some(handle) && reference.count == 1);
    let mut commands = m.history.objects.0.iter().filter(|(_, o)| {
        o.kind == 3
            && o.cost == Some(price)
            && o.snapshots[0].active == 1
            && o.snapshots[0].key == reference.key
            && o.snapshots[1].active == 0
            && o.snapshots[1].key == 0
    });
    let (&command, _) = commands
        .next()
        .context("sale's initial/final snapshots differ")?;
    ensure!(commands.next().is_none(), "ambiguous sale snapshot owner");
    ensure!(m.history.store.commands[&command].references == 1);
    let undo = m
        .history
        .store
        .vector(sims_sim::edit_history::List::Undo(m.tool.clipboard))
        .map_err(anyhow::Error::msg)?;
    ensure!(
        m.history.store.allocations[&undo.allocation][..undo.len].contains(&Some(command)),
        "sale command is not owned by Undo"
    );
    Ok(command)
}
fn released_sale(v: &Viewer, target: i16, handle: u32, command: u32) -> Result<()> {
    let s = v.session.as_ref().unwrap();
    let w = &s.simulation.vm.world;
    ensure!(
        !w.objects.contains_key(&target)
            && w.scheduler_storage
                .as_ref()
                .unwrap()
                .object_for_handle(handle)
                .is_none(),
        "normal history close did not delete the off-lot furniture lifetime"
    );
    if let Some(m) = &s.host.presentation.move_tool {
        ensure!(
            m.history
                .references
                .by_script_id(target)
                .map_err(anyhow::Error::msg)?
                .is_none()
                && !m.history.objects.0.contains_key(&command)
                && !m.history.store.commands.contains_key(&command),
            "Live return retained the sale's history allocation"
        );
    }
    Ok(())
}
fn pickup(v: &mut Viewer, target: i16, pixel: [f32; 2]) -> Result<()> {
    let scene = v.scene.as_ref().unwrap().clone();
    let bounds = iced::Rectangle::with_size(Size::new(scene.width as f32, scene.height as f32));
    let cursor = iced::mouse::Cursor::Available(iced::Point::new(pixel[0], pixel[1]));
    let mut pressed = viewport_drag::Pointer::default();
    for down in [true, false] {
        let event = if down {
            iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)
        } else {
            iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)
        };
        let (message, _, captured) = Viewport(scene.clone())
            .update(&mut pressed, &iced::Event::Mouse(event), bounds, cursor)
            .context("sale pickup pointer absent")?
            .into_inner();
        ensure!(captured == iced::event::Status::Captured);
        let m = message.context("sale pickup message absent")?;
        if down {
            ensure!(matches!(m,Message::ViewportButton{picked:Some(id),..}if id==target));
        }
        let _ = v.update(m);
    }
    Ok(())
}
///Sample actual layout nodes, delivering no non-sale message to the game.
fn sell_click(v: &Viewer) -> Result<(Message, [f32; 2])> {
    use iced::advanced::{layout, renderer::Headless, widget::Tree};
    let mut r = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
        iced::Font::DEFAULT,
        iced::Pixels(16.),
        Some("wgpu"),
    ))
    .context("headless sale adapter absent")?;
    let bounds = iced::Rectangle::with_size(Size::new(
        v.display_size[0] as f32,
        v.display_size[1] as f32,
    ));
    let mut e = v.view();
    let mut t = Tree::new(e.as_widget());
    let n = e.as_widget_mut().layout(
        &mut t,
        &mut r,
        &layout::Limits::new(Size::ZERO, bounds.size()),
    );
    fn points(n: iced::advanced::Layout<'_>, out: &mut Vec<[f32; 2]>) {
        let b = n.bounds();
        if b.width > 0. && b.height > 0. {
            out.push([b.center_x(), b.center_y()]);
        }
        for c in n.children() {
            points(c, out);
        }
    }
    let mut candidates = vec![];
    points(iced::advanced::Layout::new(&n), &mut candidates);
    for point in candidates {
        let cursor = iced::mouse::Cursor::Available(iced::Point::new(point[0], point[1]));
        let mut messages = vec![];
        for event in [
            iced::Event::Window(window::Event::RedrawRequested(Instant::now())),
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                iced::mouse::Button::Left,
            )),
        ] {
            e.as_widget_mut().update(
                &mut t,
                &event,
                iced::advanced::Layout::new(&n),
                cursor,
                &r,
                &mut iced::advanced::clipboard::Null,
                &mut iced::advanced::Shell::new(&mut messages),
                &bounds,
            );
        }
        if let Some(m) = messages.into_iter().find(|m| matches!(m, Message::Sell(_))) {
            return Ok((m, point));
        }
    }
    anyhow::bail!("no visible enabled Sell button in actual Iced layout")
}
#[test]
#[ignore = "private original households, immutable-only assets, actual Iced input and offscreen Metal"]
fn furniture_sale_retains_native_children_and_cold_returns_in_three_homes() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("sale test panicked")
}
#[test]
#[ignore = "private preserved legacy held Goth checkpoint and immutable assets"]
fn legacy_safe_held_sale_reads_the_live_native_category() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(legacy_held)?
        .join()
        .expect("legacy sale panicked")
}
fn legacy_held() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    let output = repo.join(std::env::var("SIMS_SALE_LEGACY_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let input = repo.join("cache/sale-pre-category-20261022-first/goth/held-safe-buy.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House05.iff",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        input.to_str().unwrap(),
        "--save",
        output.join("sold.json").to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    let s = v.session.as_ref().unwrap();
    let m = s.host.presentation.move_tool.as_ref().unwrap();
    ensure!(
        m.tool.tool.cursor == 6 && m.history.budgets.0[&7000].category == 7,
        "legacy fixture does not contain the known duplicated field"
    );
    let before = s.simulation.vm.world.budget.clone().unwrap();
    let tick = s.simulation.vm.tick;
    let handle = v
        .device
        .as_ref()
        .unwrap()
        .furniture_selection()
        .unwrap()
        .object;
    let target = s
        .simulation
        .vm
        .world
        .scheduler_storage
        .as_ref()
        .unwrap()
        .object_for_handle(handle)
        .context("legacy held object lifetime ended")?;
    let event = v.sell_message().context("legacy held Sell disabled")?;
    let _ = v.update(event);
    ensure!(v.error.is_none(), "{:?}", v.error);
    for _ in 0..1000 {
        if !v.presentation_pending() {
            break;
        }
        v.step_with_clock(false)?;
    }
    ensure!(!v.presentation_pending(), "legacy sale did not return");
    v.refresh()?;
    let s = v.session.as_ref().unwrap();
    let after = s.simulation.vm.world.budget.clone().unwrap();
    let mut expected = before.clone();
    expected.balance = expected.balance.wrapping_add(150);
    expected.expenses[6] = expected.expenses[6].wrapping_sub(150);
    ensure!(
        after == expected && s.simulation.vm.tick == tick,
        "legacy category projection changed other budget fields or ran a tick"
    );
    let command = retained_sale(&v, target, handle, 150)?;
    v.save()?;
    std::fs::copy(output.join("sold.json"), output.join("sold-safe-buy.json"))?;
    let _ = v.update(Message::Live);
    transport_tests::drain(&mut v, &mut vec![])?;
    released_sale(&v, target, handle, command)?;
    v.save()?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"tick":tick,"input":input,"old_category_shadow":7,"native_tool_category":6,"refund":150,"previous_expense_totals_not_rewritten":true,"complete_expected_budget":true,"undo_owned_off_lot_before_live":true,"target_deleted_after_history_close":target,"strict_original_parity_ticks_added":0}),
        )?,
    )?;
    Ok(())
}
fn run() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    for entry in std::fs::read_dir(&root)? {
        ensure!(
            !entry?
                .file_name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with("userdata"),
            "original starting save namespace present"
        );
    }
    let output = repo.join(std::env::var("SIMS_SALE_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let mut reports = vec![];
    for (name, source, input, ids) in [
        (
            "newbie",
            "UserData/Houses/House07.iff",
            "cache/post-buy-newbie-4096-20261022-first/midpoint.json",
            vec![127, 128, 158, 159, 323],
        ),
        (
            "goth",
            "UserData/Houses/House05.iff",
            "cache/controller-live-goth-20261022-sixth/live-session.json",
            vec![198, 223, 224, 225, 387],
        ),
        (
            "mashuga",
            "UserData2/Houses/House09.iff",
            "cache/post-buy-mashuga-4096-observed-20261022-first/midpoint.json",
            vec![187, 188, 189, 190, 342],
        ),
    ] {
        if std::env::var("SIMS_SALE_HOUSE").is_ok_and(|s| s != name) {
            continue;
        }
        let out = output.join(name);
        std::fs::create_dir(&out)?;
        let save = out.join("session.json");
        let arguments = || {
            Args::parse_from([
                "sims-room",
                "--original",
                source,
                "--paused",
                "--root",
                root.to_str().unwrap(),
                "--load",
                repo.join(input).to_str().unwrap(),
                "--save",
                save.to_str().unwrap(),
            ])
        };
        let args = arguments();
        let ready = Arc::new(Ready::load(&args)?);
        let (mut a, _) = Viewer::new(args);
        a.install(ready)?;
        let mut inputs = vec![];
        transport_tests::drain(&mut a, &mut inputs)?;
        let _ = a.update(Message::Buy);
        transport_tests::drain(&mut a, &mut inputs)?;
        if a.device.as_ref().unwrap().panel_state() == Some(0) {
            let _ = a.update(Message::ExpandPanel);
            transport_tests::drain(&mut a, &mut inputs)?;
        }
        let _ = a.update(Message::Floor(1));
        transport_tests::drain(&mut a, &mut inputs)?;
        let _ = a.update(Message::ViewportEnter);
        transport_tests::drain(&mut a, &mut inputs)?;
        let mut chosen = None;
        for _ in 0..4 {
            chosen = ids.iter().copied().find_map(|id| {
                let o = a.session.as_ref()?.simulation.vm.world.objects.get(&id)?;
                if o.data.get(&62).copied().unwrap_or(1) != 0
                    || o.slots.iter().flatten().any(|id| *id > 0)
                {
                    return None;
                }
                object_tests::object_pixel(&a, id).map(|p| (id, p))
            });
            if chosen.is_some() {
                break;
            }
            let _ = a.update(Message::Rotate);
            transport_tests::drain(&mut a, &mut inputs)?;
        }
        let (target, pixel) = chosen.context("no free visible stock chair for sale")?;
        pickup(&mut a, target, pixel)?;
        transport_tests::drain(&mut a, &mut inputs)?;
        let selected = a
            .device
            .as_ref()
            .unwrap()
            .furniture_selection()
            .context("sale selection absent")?
            .clone();
        ensure!(selected.sell_allowed);
        let tick = a.session.as_ref().unwrap().simulation.vm.tick;
        let balance = a
            .session
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .world
            .budget
            .as_ref()
            .unwrap()
            .balance;
        let expense_before = a
            .session
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .world
            .budget
            .as_ref()
            .unwrap()
            .expenses;
        a.save()?;
        std::fs::copy(&save, out.join("held-safe-buy.json"))?;
        let before = state(&a)?;
        render_headless_element(&root, &out.join("held.png"), a.view())?;
        let (event, button) = sell_click(&a)?;
        ensure!(
            before == state(&a)?,
            "drawing or sampling Sell processed state"
        );
        let old_held = match event {
            Message::Sell(h) => h,
            _ => unreachable!(),
        };
        let _ = a.update(event);
        ensure!(a.error.is_none(), "sale begin: {:?}", a.error);
        a.save()?;
        //A real cold launch constructs immutable dialog/HUD artwork too.
        //Load this exact saved entry, never the initial checkpoint's state.
        let cold_args = Args::parse_from([
            "sims-room",
            "--original",
            source,
            "--paused",
            "--root",
            root.to_str().unwrap(),
            "--load",
            save.to_str().unwrap(),
            "--save",
            save.to_str().unwrap(),
        ]);
        let cold_ready = Arc::new(Ready::load(&cold_args)?);
        let (mut b, _) = Viewer::new(cold_args);
        b.install(cold_ready)?;
        equal(&a, &b, &out, "sale-entry-cold")?;
        let mut requests = vec![];
        let mut restores = 1;
        let mut slices = 0;
        let mut checked_removal_owner = false;
        while a.session.as_ref().unwrap().move_tool_pending() {
            slices += 1;
            ensure!(slices < 3000, "sale exceeded retained work budget");
            let request = a
                .session
                .as_ref()
                .unwrap()
                .host
                .presentation
                .move_tool
                .as_ref()
                .unwrap()
                .sale_request();
            if let Some(r) = request {
                let text = format!("{r:?}");
                if requests.last() != Some(&text) {
                    requests.push(text);
                }
            }
            for v in [&mut a, &mut b] {
                let s = v.session.as_mut().unwrap();
                if let Err(error) = v.device.as_mut().unwrap().advance_move_tool(s, 1) {
                    std::fs::write(
                        out.join(format!("sale-slice-{slices}-failed-session.json")),
                        serde_json::to_vec(&s.save())?,
                    )?;
                    std::fs::write(
                        out.join("error.json"),
                        serde_json::to_vec_pretty(
                            &json!({"slice":slices,"error":format!("{error:#}"),"requests":requests}),
                        )?,
                    )?;
                    return Err(error);
                }
            }
            if !checked_removal_owner
                && a.session
                    .as_ref()
                    .unwrap()
                    .move_tool_removal_target()
                    .is_some()
            {
                a.client
                    .as_ref()
                    .unwrap()
                    .validate(a.session.as_ref().unwrap())?;
                let s = a.session.as_mut().unwrap();
                let owner = s.host.presentation.move_tool.take();
                let unowned = a.client.as_ref().unwrap().validate(s);
                s.host.presentation.move_tool = owner;
                ensure!(
                    unowned.is_err(),
                    "portrait client accepted an unowned deletion"
                );
                checked_removal_owner = true;
            }
            equal(&a, &b, &out, &format!("sale-slice-{slices}"))?;
            ensure!(
                a.session.as_ref().unwrap().simulation.vm.tick == tick,
                "sale advanced the lot scheduler"
            );
            if slices % 8 == 0 && a.session.as_ref().unwrap().move_tool_pending() {
                a.save()?;
                b.load_with_index(&save, &Index::scan(&root)?)?;
                equal(&a, &b, &out, &format!("cold-slice-{slices}"))?;
                restores += 1;
            }
        }
        a.refresh()?;
        b.refresh()?;
        equal(&a, &b, &out, "sale-return")?;
        ensure!(checked_removal_owner, "test missed a suspended44123f child");
        let s = a.session.as_ref().unwrap();
        let delta = s.simulation.vm.world.budget.as_ref().unwrap().balance - balance;
        let mut expected_expenses = expense_before;
        expected_expenses[6] = expected_expenses[6].wrapping_sub(selected.price);
        ensure!(
            s.simulation.vm.world.budget.as_ref().unwrap().expenses == expected_expenses,
            "sale charged a different native expense category"
        );
        ensure!(
            delta == selected.price,
            "sale did not refund the original chair's value"
        );
        let command = retained_sale(&a, target, selected.object, selected.price)?;
        ensure!(
            s.host
                .presentation
                .move_tool
                .as_ref()
                .unwrap()
                .tool
                .held
                .is_none()
                && a.device.as_ref().unwrap().furniture_selection().is_none(),
            "sale retained cursor or selected furniture"
        );
        let before = state(&a)?;
        let _ = a.update(Message::Sell(old_held));
        ensure!(
            before == state(&a)?,
            "stale Sell input sold a replacement object"
        );
        a.save()?;
        std::fs::copy(&save, out.join("sold-safe-buy.json"))?;
        b.load_with_index(&save, &Index::scan(&root)?)?;
        //Both load with the same published generation. Rebuild both scenes,
        //rather than giving only the cold Viewer an extra device publication.
        a.refresh()?;
        b.refresh()?;
        equal(&a, &b, &out, "sold-safe-cold")?;
        ensure!(retained_sale(&b, target, selected.object, selected.price)? == command);
        let before = state(&a)?;
        render_headless_element(&root, &out.join("sold.png"), a.view())?;
        ensure!(before == state(&a)?);
        let _ = a.update(Message::Live);
        let _ = b.update(Message::Live);
        transport_tests::drain(&mut a, &mut inputs)?;
        transport_tests::drain(&mut b, &mut vec![])?;
        equal(&a, &b, &out, "return-live")?;
        released_sale(&a, target, selected.object, command)?;
        released_sale(&b, target, selected.object, command)?;
        a.save()?;
        std::fs::copy(&save, out.join("sold-returned-live.json"))?;
        reports.push(json!({"household":name,"source":source,"input":input,"tick":tick,"target":target,"refund":delta,"button_point":button,"engine_slices":slices,"cold_restores":restores+1,"requests":requests,"stale_sell_ignored":true,"undo_owned_off_lot_before_live":true,"deleted_after_native_history_close":true,"full_retained_state_equal":true,"immutable_only_installation":true,"safe_buy_and_live_saves":true,"headless_drawing_unchanged":true}));
        std::fs::write(
            output.join("report.json"),
            serde_json::to_vec_pretty(
                &json!({"households":reports,"strict_original_parity_ticks_added":0,"qualifying_4096_scenarios_added":0,"scope":"Rust retained sale/restore checks with original validated component semantics. No native household sale capture or continued gameplay ticks claimed."}),
            )?,
        )?;
    }
    Ok(())
}
