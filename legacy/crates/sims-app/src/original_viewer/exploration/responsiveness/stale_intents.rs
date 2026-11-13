//! Find actual original-asset lifetime/position transitions before testing UI
//! races. No forged IDs, manual registry edits, replacement scripts or RNG seeds.
use super::*;
use std::collections::BTreeMap;

fn labels(app: &Application) -> Result<Vec<(usize, String)>> {
    let s = app.session()?;
    Ok(app
        .menu()
        .context("ready menu absent")?
        .actions()
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let label = a.label.clone().unwrap_or_else(|| {
                s.simulation
                    .vm
                    .action_menu(&s.program, a.target)
                    .ok()
                    .and_then(|entries| entries.into_iter().find(|e| e.id == a.menu_id))
                    .map(|e| e.label.clone())
                    .unwrap_or_default()
            });
            (i, label)
        })
        .collect())
}
fn lifetimes(app: &Application) -> Result<BTreeMap<i16, u32>> {
    let world = &app.session()?.simulation.vm.world;
    let storage = world
        .scheduler_storage
        .as_ref()
        .context("registry lifetimes absent")?;
    Ok(world
        .objects
        .keys()
        .filter_map(|&id| storage.handle(id).map(|handle| (id, handle)))
        .collect())
}
#[test]
#[ignore = "private Mashuga original-asset cleanup prefix; bounded actual playing-frame lifetime discovery, no GUI/guest"]
fn mashuga_original_asset_playing_frame_identity_transitions() -> Result<()> {
    run_worker(discover)
}
fn prepare(app: &mut Application, out: &Path) -> Result<(i16, String)> {
    select(app, 196)?;
    let limits = Limits::default();
    let mut offered_menus = vec![];
    let mut selected = None;
    let targets = std::env::var("SIMS_UI_IDENTITY_TARGET")
        .ok()
        .map(|s| s.parse::<i16>().map(|id| vec![id]))
        .transpose()?
        .unwrap_or_else(|| vec![198, 200, 197, 203, 204, 315]);
    for id in targets {
        app.execute(
            &Command::Object {
                identity: Identity::get(app.session()?, id)?,
            },
            &limits,
        )?;
        let offered = if app.menu().is_some() {
            labels(app)?
        } else {
            vec![]
        };
        offered_menus.push(json!({"target":id,"actions":offered,"status":app.viewer.status}));
        write_json(&out.join("offered-actions.json"), &offered_menus)?;
        if let Some((index, label)) = offered.iter().find(|(_, s)| {
            let s = s.to_ascii_lowercase();
            s.contains("clean") || s.contains("spray")
        }) {
            selected = Some((id, *index, label.clone()));
            break;
        }
        if app.menu().is_some() {
            app.execute(&Command::CloseMenu, &limits)?;
        }
    }
    let (target, index, label) = selected.context("no actual cleanup/spray action offered")?;
    let expected = app.menu().unwrap().actions()[index].clone();
    app.execute(&Command::Choice { index, expected }, &limits)?;
    app.execute(&Command::Playing { value: true }, &limits)?;
    ensure!(app.viewer.playing && !app.viewer.presentation_pending());
    Ok((target, label))
}
fn discover() -> Result<()> {
    let out = output()?;
    let mut app = load(&out)?;
    let (target, label) = prepare(&mut app, &out)?;
    let frames = std::env::var("SIMS_UI_IDENTITY_FRAMES")
        .ok()
        .map(|s| s.parse::<u32>())
        .transpose()?
        .unwrap_or(512);
    ensure!((1..=4096).contains(&frames), "unbounded identity discovery");
    let mut rows = vec![];
    let initial_tick = app.session()?.simulation.vm.tick;
    let checkpoint_frame = std::env::var("SIMS_UI_IDENTITY_CHECKPOINT_FRAME")
        .ok()
        .map(|s| s.parse::<u32>())
        .transpose()?;
    for frame in 1..=frames {
        if checkpoint_frame == Some(frame) {
            let bytes = app.checkpoint()?;
            let cold = app.restore(&bytes)?;
            ensure!(
                complete(&app)? == complete(&cold)?,
                "pre-stop managed checkpoint cold restore differs"
            );
            std::fs::write(out.join(format!("before-frame-{frame:04}.json")), &bytes)?;
            write_json(
                &out.join("pre-stop-checkpoint.json"),
                &json!({"before_frame":frame,
                "tick":app.session()?.simulation.vm.tick,"millis":app.millis,"bytes":bytes.len(),
                "sha256":engine::digest(&bytes),"managed_cold_restore_equal":true,
                "next_input":{"kind":"advance","millis":40}}),
            )?;
        }
        let before = lifetimes(&app)?;
        let positions = app.session()?.simulation.vm.world.positions.clone();
        app.millis += 40;
        let task = app.viewer.update(Message::Frame(
            app.epoch + Duration::from_millis(app.millis),
        ));
        ensure!(task.units() == 0, "unexpected native task effect");
        let after = lifetimes(&app)?;
        let world = &app.session()?.simulation.vm.world;
        let moved: Vec<_> = positions
            .iter()
            .filter_map(|(&id, p)| {
                world
                    .positions
                    .get(&id)
                    .filter(|q| *q != p)
                    .filter(|_| before.contains_key(&id) && before.get(&id) == after.get(&id))
                    .map(|q| json!({"id":id,"handle":after[&id],"before":p,"after":q}))
            })
            .collect();
        let deleted: Vec<_> = before
            .iter()
            .filter(|(id, _)| !after.contains_key(id))
            .map(|(&id, &handle)| json!({"id":id,"handle":handle}))
            .collect();
        let reused: Vec<_> = before
            .iter()
            .filter_map(|(&id, &handle)| {
                after
                    .get(&id)
                    .filter(|&&new| new != handle)
                    .map(|&new| json!({"id":id,"before":handle,"after":new}))
            })
            .collect();
        if !moved.is_empty()
            || !deleted.is_empty()
            || !reused.is_empty()
            || app.viewer.error.is_some()
        {
            rows.push(json!({"frame":frame,"millis":app.millis,"objects":world.objects.len(),
                "vm_tick":app.session()?.simulation.vm.tick,"moved":moved,"deleted":deleted,"reused":reused,
                "error":app.viewer.error,"pending":app.viewer.presentation_pending()}));
            write_json(&out.join("transitions.json"), &rows)?;
        }
        if app.viewer.error.is_some() {
            if app
                .viewer
                .error
                .as_ref()
                .is_some_and(|s| s.contains("family friend GUID owner"))
            {
                family_friend_diagnosis(&app, &out)?;
            }
            anyhow::bail!("actual playing prefix: {:?}", app.viewer.error);
        }
    }
    write_json(
        &out.join("discovery-result.json"),
        &json!({"passed":true,"initial_tick":initial_tick,
        "final_tick":app.session()?.simulation.vm.tick,"frames":frames,"transitions":rows,
        "target":target,"label":label,
        "scope":"Actual original cleanup asset through existing native handlers; discovery only, not yet stale-intent or native-engine parity proof."}),
    )
}

fn family_friend_diagnosis(app: &Application, out: &Path) -> Result<()> {
    let s = app.session()?;
    let vm = &s.simulation.vm;
    let (id, thread) = vm
        .threads
        .iter()
        .find(|(_, t)| {
            matches!(&t.status,sims_vm::Status::Faulted(f)
        if f.message.contains("family friend GUID owner"))
        })
        .context("friend fault thread absent")?;
    let frame = thread.stack.last().context("friend fault frame absent")?;
    let directory = vm
        .world
        .person
        .as_ref()
        .context("neighbor directory absent")?;
    let slot = frame.stack_object;
    let neighbor = slot
        .checked_sub(1)
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| directory.neighbor_slots.as_ref()?.get(i))
        .copied()
        .flatten();
    let record = neighbor.and_then(|id| directory.neighbors.get(&id));
    let family = record.and_then(|n| n.person.get(&61)).copied();
    let metadata = family.and_then(|f| directory.family_metadata.get(&f));
    let instruction = s
        .program
        .modules
        .get(&frame.location.source)
        .and_then(|m| m.routines.get(&frame.location.bhav))
        .and_then(|r| r.body.instructions.get(frame.location.instruction as usize));
    write_json(
        &out.join("friend-owner-diagnosis.json"),
        &json!({"tick":vm.tick,"millis":app.millis,
        "thread":id,"status":thread.status,"top_frame":frame,"instruction":instruction,
        "memory_scope":24,"memory_index":9,"neighbor_slot":slot,"neighbor_id":neighbor,
        "neighbor_guid":record.map(|n|n.guid),"family":family,"family_metadata":metadata,
        "missing_owner_path":family.map(|f|format!("/session/simulation/state/vm/world/person/family_metadata/{f}/member_guids")),
        "converted_member_neighbor_ids":family.and_then(|f|directory.families.get(&f)),
        "qualification":"Historical checkpoint capability limit: missing original FAMI GUID vector remains unknown. Converted neighbor IDs do not supply raw original GUID history; no defaults or repairs applied."}),
    )
}

fn complete(app: &Application) -> Result<String> {
    let s = app.session()?;
    engine::digest_serialized(&(
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
    ))
}
fn picked_message(scene: Arc<Scene>, id: i16) -> Result<(Message, [f32; 2])> {
    use widget::shader::Program as _;
    for draw in scene.draws.iter().filter(|d| d.object == Some(id)) {
        let vertices = &scene.vertices[draw.vertices.start as usize..draw.vertices.end as usize];
        let mut bounds = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for v in vertices {
            let p = [
                (v.position[0] + 1.) * scene.width as f32 / 2.,
                (1. - v.position[1]) * scene.height as f32 / 2.,
            ];
            bounds[0] = bounds[0].min(p[0]);
            bounds[1] = bounds[1].min(p[1]);
            bounds[2] = bounds[2].max(p[0]);
            bounds[3] = bounds[3].max(p[1]);
        }
        for y in bounds[1].floor() as i32..=bounds[3].ceil() as i32 {
            for x in bounds[0].floor() as i32..=bounds[2].ceil() as i32 {
                let pixel = [x as f32 + 0.5, y as f32 + 0.5];
                if scene.pick(pixel) != Some(id) {
                    continue;
                }
                let action = Viewport(scene.clone())
                    .update(
                        &mut Default::default(),
                        &iced::Event::Mouse(iced::mouse::Event::ButtonPressed(
                            iced::mouse::Button::Left,
                        )),
                        iced::Rectangle::with_size(Size::new(
                            scene.width as f32,
                            scene.height as f32,
                        )),
                        iced::mouse::Cursor::Available(iced::Point::new(pixel[0], pixel[1])),
                    )
                    .context("real viewport did not emit click")?;
                let (message, _, status) = action.into_inner();
                ensure!(status == iced::event::Status::Captured);
                return Ok((message.context("viewport click absent")?, pixel));
            }
        }
    }
    anyhow::bail!("target {id} has no visible pickable pixel")
}

#[test]
#[ignore = "private Mashuga original cleanup prefix; prior-publication resident input after a playing Frame moves its still-live person"]
fn mashuga_worker_preserves_input_for_the_same_moved_live_instance() -> Result<()> {
    run_worker(moved_live)
}
fn moved_live() -> Result<()> {
    use super::super::super::worker_host::{Event, Host};
    let out = output()?;
    let mut reference = load(&out)?;
    prepare(&mut reference, &out)?;
    let id = std::env::var("SIMS_UI_MOVED_TARGET")
        .ok()
        .map(|s| s.parse::<i16>())
        .transpose()?
        .unwrap_or(196);
    let frame = std::env::var("SIMS_UI_MOVED_FRAME")
        .ok()
        .map(|s| s.parse::<u32>())
        .transpose()?
        .unwrap_or(71);
    ensure!(
        (1..=4096).contains(&frame),
        "unbounded moved-instance prefix"
    );
    for _ in 1..frame {
        reference.millis += 40;
        ensure!(
            reference
                .viewer
                .update(Message::Frame(
                    reference.epoch + Duration::from_millis(reference.millis)
                ))
                .units()
                == 0
        );
        reference.check_errors()?;
    }
    let identity = Identity::get(reference.session()?, id)?;
    let before = reference.session()?.simulation.vm.world.positions[&id];
    let checkpoint = reference.checkpoint()?;
    let mut cold = reference.restore(&checkpoint)?;
    let mut host = Host::from_viewer_for_test(cold.viewer);
    let publication = host.publication_for_test().clone();
    let (message, pixel) = if id == 196 {
        let control = publication
            .residents
            .iter()
            .find(|r| r.object == id)
            .context("published resident absent")?;
        ensure!(
            publication.people_visible && control.enabled,
            "resident control was not available"
        );
        (Message::Resident(control.object), None)
    } else {
        let (message, pixel) =
            picked_message(publication.scene.clone().context("scene absent")?, id)?;
        ensure!(
            matches!(message,Message::ViewportButton {down:true,picked:Some(target),..} if target==id)
        );
        (message, Some(pixel))
    };
    reference.millis += 40;
    cold.millis = reference.millis;
    let now = reference.epoch + Duration::from_millis(reference.millis);
    let (entered, resume) = host.gate_next_for_test();
    ensure!(
        host.update(Event::Widget {
            generation: 0,
            message: Message::Frame(now)
        })
        .units()
            == 1
    );
    entered.recv_timeout(Duration::from_secs(5))?;
    ensure!(
        host.update(host.ui_event_for_test(&publication, message.clone()))
            .units()
            == 0
    );
    ensure!(Arc::ptr_eq(&publication, host.publication_for_test()));
    ensure!(reference.viewer.update(Message::Frame(now)).units() == 0);
    identity.check(reference.session()?)?;
    let after = reference.session()?.simulation.vm.world.positions[&id];
    ensure!(
        before != after,
        "recorded playing Frame no longer moves the target"
    );
    ensure!(reference.viewer.update(message).units() == 0);
    resume.send(())?;
    ensure!(host.settle_for_test()?.is_empty());
    cold.viewer = host.recover_for_test();
    ensure!(
        complete(&reference)? == complete(&cold)?,
        "moved-instance ordered handlers diverge"
    );
    reference.check_errors()?;
    reference.drain(&Limits::default())?;
    cold.drain(&Limits::default())?;
    ensure!(
        complete(&reference)? == complete(&cold)?,
        "moved-instance menu outcome diverges"
    );
    write_json(
        &out.join("moved-live-result.json"),
        &json!({"passed":true,"identity":identity,
        "playing_frame":frame,"pixel_from_prior_publication":pixel,"position_before":before,"position_after":after,
        "complete_state_equal_after_ordered_calls":true,"complete_state_equal_after_menu_drains":true,
        "actions":if reference.menu().is_some(){labels(&reference)?}else{vec![]},
        "strict_original_parity_ticks":0,"scope":"Actual still-live native instance moves during the blocked playing Frame; input from its available prior-publication control reaches unchanged current native handlers in order. No lifetime rejection policy or original-engine parity claim."}),
    )
}

fn open_ground(app: &mut Application) -> Result<Identity> {
    let (pixel, _) = super::super::super::ground_tests::click_point(&app.viewer)?;
    app.execute(
        &Command::Viewport {
            pixel: pixel.map(f32::to_bits),
            expected: None,
        },
        &Limits::default(),
    )?;
    let s = app.session()?;
    let input = s
        .host
        .presentation
        .ground_input
        .as_ref()
        .context("ground owner absent")?;
    ensure!(
        input.ready() && s.running_menu(),
        "original ground menu did not publish"
    );
    Identity::get(s, input.target().context("ground helper absent")?)
}

#[test]
#[ignore = "private Mashuga original Go Here helper; actual playing cancellation Frame deletes the prior menu target before its queued leaf input"]
fn mashuga_worker_preserves_deleted_ground_menu_input_guards() -> Result<()> {
    run_worker(deleted_ground)
}
fn deleted_ground() -> Result<()> {
    use super::super::super::ui_publication::Published;
    use super::super::super::worker_host::{Event, Host};
    let out = output()?;
    let mut reference = load(&out)?;
    select(&mut reference, 196)?;
    reference.execute(&Command::Playing { value: true }, &Limits::default())?;
    let old = open_ground(&mut reference)?;
    let published = Published::capture(&reference.viewer);
    let choice = published
        .menu
        .as_ref()
        .context("published ground menu absent")?
        .popup
        .first()
        .and_then(|c| c.intent.as_ref())
        .context("published ground choice absent")?
        .message();
    ensure!(matches!(choice, Message::GroundChoice(_)));
    reference.dispatch(Message::GroundCancel)?;
    ensure!(reference.session()?.menu_command_pending());
    let checkpoint = reference.checkpoint()?;
    let mut probe = reference.restore(&checkpoint)?;
    probe.millis += 40;
    let now = probe.epoch + Duration::from_millis(probe.millis);
    let mut deletion_poll = None;
    let mut polls = vec![];
    for poll in 1..=64 {
        ensure!(probe.viewer.update(Message::Frame(now)).units() == 0);
        polls.push(json!({"poll":poll,"helper_alive":probe.session()?.simulation.vm.world.objects.contains_key(&old.object),
            "ground":probe.session()?.host.presentation.ground_input,"error":probe.viewer.error}));
        write_json(&out.join("deletion-service-polls.json"), &polls)?;
        probe.check_errors()?;
        if !probe
            .session()?
            .simulation
            .vm
            .world
            .objects
            .contains_key(&old.object)
        {
            deletion_poll = Some(poll);
            break;
        }
    }
    let deletion_poll = deletion_poll
        .context("original ground destructor did not finish its bounded native continuation")?;
    reference.millis = probe.millis;
    for _ in 1..deletion_poll {
        ensure!(reference.viewer.update(Message::Frame(now)).units() == 0);
        reference.check_errors()?;
    }
    ensure!(
        reference
            .session()?
            .simulation
            .vm
            .world
            .objects
            .contains_key(&old.object)
    );
    let checkpoint = reference.checkpoint()?;
    let mut cold = reference.restore(&checkpoint)?;
    let mut host = Host::from_viewer_for_test(cold.viewer);
    let (entered, resume) = host.gate_next_for_test();
    ensure!(
        host.update(Event::Widget {
            generation: 0,
            message: Message::Frame(now)
        })
        .units()
            == 1
    );
    entered.recv_timeout(Duration::from_secs(5))?;
    ensure!(
        host.update(host.ui_event_for_test(&published, choice.clone()))
            .units()
            == 0
    );
    cold.millis = reference.millis;
    ensure!(reference.viewer.playing);
    ensure!(reference.viewer.update(Message::Frame(now)).units() == 0);
    let removed = !reference
        .session()?
        .simulation
        .vm
        .world
        .objects
        .contains_key(&old.object);
    write_json(
        &out.join("deleted-frame-cut.json"),
        &json!({"identity":old,"removed":removed,
        "pending":reference.viewer.presentation_pending(),"error":reference.viewer.error,
        "ground":reference.session()?.host.presentation.ground_input,
        "live_menu":reference.session()?.host.presentation.live_menu}),
    )?;
    // The prelude found and replayed the actual final destructor service cut.
    if !removed {
        ensure!(reference.viewer.update(choice).units() == 0);
        resume.send(())?;
        host.settle_for_test()?;
        cold.viewer = host.recover_for_test();
        ensure!(
            complete(&reference)? == complete(&cold)?,
            "incomplete native cancellation differs"
        );
        anyhow::bail!(
            "recorded Frame did not yet delete the ground helper; use its actual retained continuation"
        );
    }
    ensure!(reference.viewer.update(choice).units() == 0);
    reference.check_errors()?;
    resume.send(())?;
    ensure!(host.settle_for_test()?.is_empty());
    cold.viewer = host.recover_for_test();
    ensure!(
        complete(&reference)? == complete(&cold)?,
        "deleted-owner queued input changed native outcome"
    );
    reference.drain(&Limits::default())?;
    cold.drain(&Limits::default())?;
    ensure!(
        complete(&reference)? == complete(&cold)?,
        "deleted-owner continuation differs"
    );
    let new = open_ground(&mut reference)?;
    write_json(
        &out.join("deleted-ground-result.json"),
        &json!({"passed":true,"old":old,"new":new,
        "deletion_service_polls":deletion_poll,
        "old_helper_deleted_by_original_cleanup":true,"reused_script_id":old.object==new.object,
        "distinct_native_lifetime":old.handle!=new.handle,"late_choice_ignored_by_current_handler":true,
        "complete_state_rng_and_continuation_equal":true,"strict_original_parity_ticks":0,
        "scope":"Actual original Go Here cancellation, blocked playing Frame and prior-publication leaf input; existing guard confirmation only. New-helper reuse is discovered, not yet a reused-menu race test."}),
    )
}

#[test]
#[ignore = "private Mashuga real Go Here delete/recreate; stale prior-popup leaf must not bind a new native lifetime during a blocked playing Frame"]
fn mashuga_worker_keeps_reused_ground_popup_lifetimes_distinct() -> Result<()> {
    run_worker(reused_ground)
}
fn reused_ground() -> Result<()> {
    use super::super::super::ui_publication::Published;
    use super::super::super::worker_host::{Event, Host};
    let out = output()?;
    let mut reference = load(&out)?;
    select(&mut reference, 196)?;
    reference.execute(&Command::Playing { value: true }, &Limits::default())?;
    let old = open_ground(&mut reference)?;
    let publication = Arc::new(Published::capture(&reference.viewer));
    let choice = publication
        .menu
        .as_ref()
        .context("old popup absent")?
        .popup
        .first()
        .and_then(|c| c.intent.as_ref())
        .context("old leaf absent")?
        .message();
    reference.execute(&Command::CloseMenu, &Limits::default())?;
    ensure!(
        !reference
            .session()?
            .simulation
            .vm
            .world
            .objects
            .contains_key(&old.object)
    );
    let new = open_ground(&mut reference)?;
    ensure!(
        old.object == new.object && old.handle != new.handle,
        "original helper did not actually reuse the script ID"
    );
    let current_publication = Published::capture(&reference.viewer);
    let checkpoint = reference.checkpoint()?;
    let mut cold = reference.restore(&checkpoint)?;
    let mut host = Host::from_viewer_for_test(cold.viewer);
    let (entered, resume) = host.gate_next_for_test();
    reference.millis += 40;
    cold.millis = reference.millis;
    let now = reference.epoch + Duration::from_millis(reference.millis);
    ensure!(
        host.update(Event::Widget {
            generation: 0,
            message: Message::Frame(now)
        })
        .units()
            == 1
    );
    entered.recv_timeout(Duration::from_secs(5))?;
    // This command is the real leaf extracted from the earlier popup, whose
    // original helper lifetime has ended. The current popup is a new owner.
    ensure!(
        host.update(host.ui_event_for_test(&publication, choice))
            .units()
            == 0
    );
    ensure!(reference.viewer.playing);
    ensure!(reference.viewer.update(Message::Frame(now)).units() == 0);
    new.check(reference.session()?)?;
    resume.send(())?;
    ensure!(host.settle_for_test()?.is_empty());
    cold.viewer = host.recover_for_test();
    let expected = complete(&reference)?;
    let actual = complete(&cold)?;
    write_json(
        &out.join("reused-ground-result.json"),
        &json!({"passed":expected==actual,"old":old,"new":new,
        "expected_state_sha256":expected,"actual_state_sha256":actual,
        "expected_live_menu":reference.session()?.host.presentation.live_menu,
        "actual_live_menu":cold.session()?.host.presentation.live_menu,
        "strict_original_parity_ticks":0,"scope":"Actual original helper cancellation/recreation at the same script ID; delayed leaf from its ended popup must not retarget the new popup. No original engine or injected world state."}),
    )?;
    if expected != actual {
        std::fs::write(out.join("reused-ground-before-frame.json"), &checkpoint)?;
    }
    ensure!(
        expected == actual,
        "a prior popup leaf retargeted the reused native helper lifetime"
    );
    // A leaf from this popup, observed before the same playing Frame, remains
    // valid. The fix must not turn a publication-age fence into UI semantics.
    let current_choice = current_publication.menu.as_ref().unwrap().popup[0]
        .intent
        .as_ref()
        .unwrap()
        .message();
    let mut host = Host::from_viewer_for_test(cold.viewer);
    ensure!(reference.viewer.update(current_choice.clone()).units() == 0);
    ensure!(
        host.update(host.ui_event_for_test(&current_publication, current_choice))
            .units()
            == 1
    );
    ensure!(host.settle_for_test()?.is_empty());
    cold.viewer = host.recover_for_test();
    ensure!(
        complete(&reference)? == complete(&cold)?,
        "current popup leaf changed its native handler outcome"
    );
    ensure!(
        reference.session()?.menu_command_pending(),
        "current popup leaf was rejected"
    );
    write_json(
        &out.join("valid-popup-leaf-result.json"),
        &json!({"passed":true,
        "same_publication_observed_before_playing_frame":true,"current_native_owner_accepted":true,
        "complete_state_rng_and_continuation_equal":true,"publication_age_rejection":false}),
    )?;
    Ok(())
}

#[test]
#[ignore = "private original Go Here popup and headless Metal; actual Iced held press survives ordinary playing Frame but ends when popup commits cancellation"]
fn mashuga_popup_press_uses_native_owner_lifetime_not_publication_age() -> Result<()> {
    run_worker(popup_press)
}
fn popup_press() -> Result<()> {
    use super::super::super::ui_publication::view::Frontend;
    use super::super::super::worker_host::{Event, Host};
    let out = output()?;
    let mut reference = load(&out)?;
    select(&mut reference, 196)?;
    reference.execute(&Command::Playing { value: true }, &Limits::default())?;
    open_ground(&mut reference)?;
    let checkpoint = reference.checkpoint()?;
    let mut cold = reference.restore(&checkpoint)?;
    let mut host = Host::from_viewer_for_test(cold.viewer);
    let frontend = |host: &Host| Frontend {
        data: host.publication_for_test().clone(),
        diagnostics: false,
        options_open: false,
        widget_epoch: host.widget_epoch_for_test(),
    };
    let size = reference.viewer.display_size;
    let mut ui = Input::new()?;
    let press = iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left));
    let release = iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
        iced::mouse::Button::Left,
    ));
    let mut point = None;
    let mut options_point = None;
    for p in ui.batch_centers(&reference.viewer) {
        ui.reset();
        let events = [
            iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                position: iced::Point::new(p[0], p[1]),
            }),
            press.clone(),
            release.clone(),
        ];
        let f = frontend(&host);
        let batch = ui.element_batch(f.view(), size, &events);
        if batch
            .messages
            .iter()
            .any(|m| matches!(m, Message::GroundChoice(0)))
        {
            point = Some(p);
        }
        if batch.messages.iter().any(|m| matches!(m, Message::Options)) {
            options_point = Some(p);
        }
        if point.is_some() && options_point.is_some() {
            break;
        }
    }
    let point = point.context("real popup leaf button not found")?;
    let options_point = options_point.context("real HUD Options button not found")?;
    let movement = iced::Event::Mouse(iced::mouse::Event::CursorMoved {
        position: iced::Point::new(point[0], point[1]),
    });
    ui.reset();
    let f = frontend(&host);
    ensure!(
        !ui.element_batch(f.view(), size, &[movement.clone(), press.clone()])
            .messages
            .iter()
            .any(|m| matches!(m, Message::GroundChoice(_))),
        "leaf activated on press"
    );
    let epoch = host.publication_for_test().native_popup_epoch;
    reference.millis += 40;
    cold.millis = reference.millis;
    let now = reference.epoch + Duration::from_millis(reference.millis);
    ensure!(reference.viewer.update(Message::Frame(now)).units() == 0);
    ensure!(
        host.update(Event::Widget {
            generation: 0,
            message: Message::Frame(now)
        })
        .units()
            == 1
    );
    ensure!(host.settle_for_test()?.is_empty());
    ensure!(
        host.publication_for_test().native_popup_epoch == epoch,
        "ordinary playing Frame ended a live popup press"
    );
    let f = frontend(&host);
    ensure!(
        ui.element_batch(f.view(), size, std::slice::from_ref(&release))
            .messages
            .iter()
            .any(|m| matches!(m, Message::GroundChoice(0))),
        "same-owner held press was rejected by age"
    );
    // Do not dispatch the sampled valid leaf. An earlier actual Unfocused
    // event must not consume the independent future native-popup epoch.
    let f = frontend(&host);
    ui.element_batch(
        f.view(),
        size,
        &[iced::Event::Window(iced::window::Event::Unfocused)],
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
    // Start another real held press,
    // then use the actual Escape subscription message to cancel its owner.
    let f = frontend(&host);
    ui.element_batch(f.view(), size, &[movement, press]);
    let mut hud_ui = Input::new()?;
    let f = frontend(&host);
    hud_ui.element_batch(
        f.view(),
        size,
        &[
            iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                position: iced::Point::new(options_point[0], options_point[1]),
            }),
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
        ],
    );
    ensure!(reference.viewer.update(Message::DialogKey(27)).units() == 0);
    ensure!(
        host.update(Event::Widget {
            generation: 0,
            message: Message::DialogKey(27)
        })
        .units()
            == 1
    );
    ensure!(host.settle_for_test()?.is_empty());
    ensure!(
        host.publication_for_test().native_popup_epoch != epoch,
        "popup return did not end old button presses"
    );
    let f = frontend(&host);
    let batch = ui.element_batch(f.view(), size, std::slice::from_ref(&release));
    ensure!(
        !batch
            .messages
            .iter()
            .any(|m| matches!(m, Message::GroundChoice(_))),
        "ended popup reused a held leaf press"
    );
    let f = frontend(&host);
    ensure!(
        hud_ui
            .element_batch(f.view(), size, &[release])
            .messages
            .iter()
            .any(|m| matches!(m, Message::Options)),
        "native popup return ended an unrelated HUD press"
    );
    cold.viewer = host.recover_for_test();
    ensure!(
        complete(&reference)? == complete(&cold)?,
        "passive Iced input changed state/RNG/continuations"
    );
    write_json(
        &out.join("popup-press-result.json"),
        &json!({"passed":true,"point":point,
        "same_owner_playing_frame_keeps_press":true,"popup_return_ends_press":true,"unrelated_hud_press_preserved":true,
        "earlier_focus_event_does_not_consume_owner_epoch":true,
        "complete_state_rng_and_continuation_equal":true,"publication_age_rejection":false,
        "strict_original_parity_ticks":0,"scope":"Actual production frontend and native popup epoch through Iced events; no GUI, synthetic native state or original-engine parity claim."}),
    )
}
