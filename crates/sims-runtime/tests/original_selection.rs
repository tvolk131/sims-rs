use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Options, Session};
use sims_sim::pose::portrait::view::Input;

#[test]
#[ignore = "requires private original residential assets"]
fn original_indicator_refresh_owns_callbacks_and_restores() -> Result<()> {
    use std::collections::BTreeSet;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut initial = Session::load(&index, "UserData/Houses/House07.iff", Options::default())?;
    let people: Vec<_> = initial
        .house
        .household
        .as_ref()
        .unwrap()
        .residents
        .iter()
        .map(|r| r.object as i16)
        .collect();
    let camera = Input {
        viewport: [0, 0, 800, 600],
        center: [0, 0],
        zoom: 3,
        rotation: 0,
    };
    initial.begin_selection(Some(people[0]))?;
    while !initial.advance_selection(camera, 10000)? {}
    // Controlled post-notification inputs isolate the presenter's original
    // drain. Real expression/write-intent ordering has its own native oracle.
    initial
        .simulation
        .vm
        .world
        .objects
        .get_mut(&people[0])
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(77, -people[1]);
    initial
        .simulation
        .vm
        .world
        .objects
        .get_mut(&people[1])
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(81, 10);
    let initial = initial.save();
    for stars in [false, true] {
        let mut results = vec![];
        for (budget, resume) in [(1, true), (10000, false)] {
            let mut s = Session::restore(&index, initial.clone())?;
            s.simulation.vm.world.indicator_queues = sims_vm::indicator_unregister::Queues {
                dates: vec![None, Some(people[0]), Some(people[0])],
                stars: if stars {
                    vec![Some(people[1]), None, Some(people[1])]
                } else {
                    vec![]
                },
                dirty: true,
            };
            let clock_rng = (
                s.simulation.vm.tick,
                s.simulation.vm.rng_state,
                s.simulation.vm.lot_rng_state,
            );
            let mut expected_crt = s.simulation.vm.crt_rng_state.unwrap();
            if stars {
                for _ in 0..128 {
                    sims_vm::rng::next_crt(&mut expected_crt);
                }
            }
            let selected_flags: Vec<_> = people
                .iter()
                .map(|id| s.simulation.vm.world.objects[id].native_flags)
                .collect();
            let mut checkpoints = BTreeSet::new();
            let mut factories = BTreeSet::new();
            let mut graphic_calls = BTreeSet::new();
            let mut done = false;
            s.begin_indicator_refresh()?;
            for boundary in 0..3000 {
                if let Some(owner) = &s.simulation.native_selection {
                    ensure!(owner.is_refresh());
                    ensure!(s.simulation.finish_registry_pass().is_err());
                    let value = serde_json::to_value(&s.simulation.native_selection)?;
                    // Retain every distinct child service/phase, without
                    // repeatedly reloading for each instruction in a callback.
                    let key = format!(
                        "{:?}:{}:{}:{}:{}",
                        s.simulation.pending_native_selection(),
                        value["refresh"]["pending"],
                        value["indicators"]["stage"],
                        value["placement"]["pending"],
                        value["factory"]["stage"]
                    );
                    if let Some((request, started)) = s.simulation.pending_native_selection() {
                        match request {
                            sims_sim::lot_selection::Request::FactoryGraphic(id) if started => {
                                factories.insert(id);
                            }
                            sims_sim::lot_selection::Request::Graphic { call, .. } => {
                                graphic_calls.insert(format!("{call:?}"));
                            }
                            _ => (),
                        }
                    }
                    if resume && checkpoints.insert(key) {
                        let saved = s.save();
                        if !value["refresh"]["pending"].is_null() {
                            let mut orphan = serde_json::to_value(&saved.simulation)?;
                            orphan["state"]["native_selection"]["refresh"]["pending"] =
                                serde_json::Value::Null;
                            ensure!(
                                serde_json::from_value::<sims_sim::Snapshot>(orphan)?
                                    .restore(&s.program)
                                    .is_err()
                            );
                            let mut orphan = serde_json::to_value(&saved.simulation)?;
                            orphan["state"]["native_selection"]["refresh"] =
                                serde_json::Value::Null;
                            ensure!(
                                serde_json::from_value::<sims_sim::Snapshot>(orphan)?
                                    .restore(&s.program)
                                    .is_err()
                            );
                        }
                        s = Session::restore(
                            &index,
                            serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                        )
                        .with_context(|| {
                            format!("refresh restore stars={stars}, boundary={boundary}")
                        })?;
                        ensure!(
                            s.simulation == saved.simulation.state
                                && s.host.snapshot() == saved.assets
                        );
                    }
                }
                if s.advance_selection(camera, budget).with_context(|| {
                    format!("refresh stars={stars}, budget={budget}, boundary={boundary}")
                })? {
                    done = true;
                    break;
                }
            }
            ensure!(done && s.simulation.vm.world.indicator_queues.is_empty());
            ensure!(
                clock_rng
                    == (
                        s.simulation.vm.tick,
                        s.simulation.vm.rng_state,
                        s.simulation.vm.lot_rng_state
                    )
            );
            ensure!(s.simulation.vm.crt_rng_state == Some(expected_crt));
            ensure!(s.simulation.vm.world.globals[&3] == people[0]);
            ensure!(
                people
                    .iter()
                    .map(|id| s.simulation.vm.world.objects[id].native_flags)
                    .collect::<Vec<_>>()
                    == selected_flags
            );
            let secondary = s
                .simulation
                .native_lot
                .as_ref()
                .unwrap()
                .indicators
                .secondary
                .unwrap();
            if stars {
                ensure!(s.simulation.native_indicator_pools.spare.len() == 3);
                ensure!(s.simulation.native_indicator_pools.active.len() == 1);
                ensure!(
                    s.simulation.vm.world.objects[&secondary]
                        .container
                        .is_none()
                );
                if resume {
                    ensure!(factories.len() == 4);
                }
            } else {
                ensure!(
                    s.simulation.vm.world.objects[&secondary]
                        .container
                        .map(|c| c.0)
                        == Some(people[1])
                );
            }
            ensure!(!graphic_calls.is_empty());
            println!(
                "original refresh: stars={stars}, budget={budget}, {} full-session restores, {} graphic calls",
                checkpoints.len(),
                graphic_calls.len()
            );
            results.push(s.save());
        }
        ensure!(
            serde_json::to_value(&results[0])? == serde_json::to_value(&results[1])?,
            "refresh changed with callback budget"
        );
        std::fs::write(
            root.join(if stars {
                "cache/original-star-refresh-save.json"
            } else {
                "cache/original-date-refresh-save.json"
            }),
            serde_json::to_vec(&results[0])?,
        )?;
    }
    Ok(())
}

#[test]
#[ignore = "requires private original residential assets"]
fn primary_original_graphic_deletion_unregisters_retires_and_restores() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut initial = Session::load(&index, "UserData/Houses/House07.iff", Options::default())?;
    let camera = Input {
        viewport: [0, 0, 800, 600],
        center: [0, 0],
        zoom: 3,
        rotation: 0,
    };
    let person = initial.house.household.as_ref().unwrap().residents[0].object as i16;
    for target in [Some(person), None] {
        initial.begin_selection(target)?;
        while !initial.advance_selection(camera, 10000)? {}
    }
    // Explicit deletion of an off-lot original PlumbBob. The native prelude
    // must clear its primary registry reference before physical removal.
    let id = initial
        .simulation
        .native_lot
        .as_ref()
        .unwrap()
        .indicators
        .primary
        .context("original marker absent")?;
    ensure!(initial.simulation.vm.world.objects[&id].container.is_none());
    let saved = initial.save();
    let mut results = vec![];
    for (budget, resume) in [(1, true), (10000, false)] {
        let mut s = Session::restore(&index, saved.clone())?;
        let clock_rng = (
            s.simulation.vm.tick,
            s.simulation.vm.rng_state,
            s.simulation.vm.lot_rng_state,
            s.simulation.vm.crt_rng_state,
        );
        s.simulation
            .begin_synchronous_removal(&s.program, id)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            s.simulation
                .native_lot
                .as_ref()
                .unwrap()
                .indicators
                .primary
                .is_none()
        );
        let mut checkpoints = std::collections::BTreeSet::new();
        let mut refreshes = 0;
        let mut retired = false;
        let mut done = false;
        for boundary in 0..2000 {
            let mut step = sims_sim::Step {
                stop: sims_sim::StepStop::Budget,
                trace: vec![],
                events: vec![],
            };
            let mut remaining = budget;
            let finished = s
                .simulation
                .drive_synchronous_removal(&s.program, &mut remaining, &mut step)
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("ordinary deletion boundary {boundary}"))?;
            if s.simulation
                .pending_removal_graphic()
                .is_some_and(|(_, started)| !started)
            {
                refreshes += 1;
            }
            s.service_selection_assets(camera, budget)?;
            s.host.service(&mut s.simulation, &mut s.program)?;
            if s.simulation.retired_graphics.contains_key(&id) {
                retired = true;
                ensure!(!s.simulation.vm.world.objects.contains_key(&id));
                ensure!(s.simulation.finish_registry_pass().is_err());
                ensure!(s.host.ordinary.graphics.contains_key(&id));
            }
            let checkpoint = format!(
                "{:?}:{:?}:{:?}",
                s.simulation.pending_removal_graphic(),
                s.host.ordinary.retirements.get(&id),
                s.simulation.retired_graphics.get(&id)
            );
            if resume && checkpoints.insert(checkpoint) {
                if s.host.ordinary.retirements.contains_key(&id) {
                    let mut orphan = s.save();
                    orphan.assets.ordinary.retirements.remove(&id);
                    ensure!(
                        Session::restore(&index, orphan).is_err(),
                        "orphan ordinary destructor accepted"
                    );
                }
                let before = s.save();
                s = Session::restore(
                    &index,
                    serde_json::from_slice(&serde_json::to_vec(&before)?)?,
                )?;
                ensure!(
                    serde_json::to_value(s.save())? == serde_json::to_value(before)?,
                    "ordinary destructor save changed state"
                );
            }
            if finished {
                done = true;
                break;
            }
        }
        ensure!(
            done && retired && refreshes >= 1,
            "ordinary destruction skipped required phases"
        );
        ensure!(
            s.simulation
                .take_synchronous_removal()
                .map_err(anyhow::Error::msg)?
                == id
        );
        ensure!(
            !s.host.ordinary.graphics.contains_key(&id) && s.host.ordinary.retirements.is_empty()
        );
        ensure!(
            clock_rng
                == (
                    s.simulation.vm.tick,
                    s.simulation.vm.rng_state,
                    s.simulation.vm.lot_rng_state,
                    s.simulation.vm.crt_rng_state
                )
        );
        println!(
            "primary original graphic deletion: budget={budget}, refreshes={refreshes}, saved phases={}",
            checkpoints.len()
        );
        results.push(s.save());
    }
    ensure!(
        serde_json::to_value(&results[0])? == serde_json::to_value(&results[1])?,
        "ordinary deletion changed with service budget"
    );
    std::fs::write(
        root.join("cache/unowned-ordinary-deletion-save.json"),
        serde_json::to_vec(&results[0])?,
    )?;
    Ok(())
}

#[test]
#[ignore = "requires private original residential assets"]
fn selected_original_person_deletion_releases_indicators_and_restores() -> Result<()> {
    for (fame, selected) in [(false, true), (true, true), (true, false)] {
        compare_selected_deletion(fame, selected)?;
    }
    Ok(())
}
fn compare_selected_deletion(fame: bool, selected: bool) -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut initial = Session::load(&index, "UserData/Houses/House07.iff", Options::default())?;
    let person = initial.house.household.as_ref().unwrap().residents[0].object as i16;
    let camera = Input {
        viewport: [0, 0, 800, 600],
        center: [0, 0],
        zoom: 3,
        rotation: 0,
    };
    initial.begin_selection(Some(person))?;
    while !initial.advance_selection(camera, 10000)? {}
    let marker = initial
        .simulation
        .native_lot
        .as_ref()
        .unwrap()
        .indicators
        .primary
        .context("primary marker absent")?;
    ensure!(
        initial.simulation.vm.world.objects[&marker]
            .container
            .map(|v| v.0)
            == Some(person)
    );
    if fame {
        // Explicit Superstar input: deleting a selected famous Sim calls the
        // native pool factory even though off-lot placement cannot attach it.
        initial
            .simulation
            .vm
            .world
            .objects
            .get_mut(&person)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .insert(81, 10);
    }
    let other = initial.house.household.as_ref().unwrap().residents[1].object as i16;
    let child_star = if !selected {
        initial.begin_selection(Some(other))?;
        while !initial.advance_selection(camera, 10000)? {}
        Some(
            initial.simulation.vm.world.objects[&person].slots[1]
                .context("contained star absent")?,
        )
    } else {
        None
    };
    // Native pending date/star refreshes are distinct from registry person order.
    initial.simulation.vm.world.indicator_queues.dates = vec![None, Some(person), Some(other)];
    initial.simulation.vm.world.indicator_queues.stars = vec![Some(other), Some(person), None];
    let saved = initial.save();
    let mut results = vec![];
    for (budget, resume) in [(1, true), (10000, false)] {
        let mut s = Session::restore(&index, saved.clone())?;
        let tick = s.simulation.vm.tick;
        let mut expected_crt = s.simulation.vm.crt_rng_state.unwrap();
        if fame && selected {
            for _ in 0..128 {
                sims_vm::rng::next_crt(&mut expected_crt);
            }
        }
        let rng = (
            s.simulation.vm.rng_state,
            s.simulation.vm.lot_rng_state,
            Some(expected_crt),
        );
        s.simulation
            .begin_synchronous_removal(&s.program, person)
            .map_err(anyhow::Error::msg)?;
        ensure!(s.simulation.vm.world.indicator_queues.dates == vec![None, Some(other)]);
        ensure!(s.simulation.vm.world.indicator_queues.stars == vec![Some(other), None]);
        let mut checkpoints = std::collections::BTreeSet::new();
        let mut restores = 0;
        let mut done = false;
        for boundary in 0..2000 {
            let mut step = sims_sim::Step {
                stop: sims_sim::StepStop::Budget,
                trace: vec![],
                events: vec![],
            };
            let mut remaining = budget;
            let finished = s
                .simulation
                .drive_synchronous_removal(&s.program, &mut remaining, &mut step)
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("deletion boundary {boundary}"))?;
            // Publish native selection to the presenter even between asset requests.
            s.service_selection_assets(camera, budget)?;
            s.host.service(&mut s.simulation, &mut s.program)?;
            let checkpoint = format!(
                "{:?}:{:?}:{:?}:{:?}:{:?}",
                s.simulation.pending_native_selection(),
                s.simulation.pending_reset_character_services(),
                s.simulation.pending_deletion_character_services(),
                s.simulation.pending_removal_graphic(),
                s.host.ordinary.retirements
            );
            if resume && checkpoints.insert(checkpoint) {
                if matches!(
                    s.simulation.pending_native_selection(),
                    Some((sims_sim::lot_selection::Request::FactoryGraphic(_), true))
                ) {
                    let mut orphan = sims_sim::Snapshot::capture(&s.program, &s.simulation);
                    orphan.state.deletion.last_mut().unwrap().entry = None;
                    ensure!(
                        orphan.restore(&s.program).is_err(),
                        "orphan nested factory was accepted"
                    );
                }
                let state = s.simulation.clone();
                let assets = s.host.snapshot();
                s = Session::restore(
                    &index,
                    serde_json::from_slice(&serde_json::to_vec(&s.save())?)?,
                )?;
                ensure!(s.simulation == state && s.host.snapshot() == assets);
                restores += 1;
            }
            if finished {
                done = true;
                break;
            }
        }
        ensure!(done, "selected deletion did not complete");
        ensure!(
            s.simulation
                .take_synchronous_removal()
                .map_err(anyhow::Error::msg)?
                == person
        );
        ensure!(!s.simulation.vm.world.objects.contains_key(&person));
        ensure!(s.simulation.vm.world.objects.contains_key(&marker));
        ensure!(
            s.simulation.vm.world.objects[&marker]
                .container
                .map(|v| v.0)
                == if selected { None } else { Some(other) }
        );
        if let Some(star) = child_star {
            ensure!(!s.simulation.vm.world.objects.contains_key(&star));
            ensure!(!s.host.ordinary.graphics.contains_key(&star));
        }
        ensure!(
            s.simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?
                == if selected { None } else { Some(other) }
        );
        ensure!(s.simulation.vm.tick == tick);
        ensure!(
            (
                s.simulation.vm.rng_state,
                s.simulation.vm.lot_rng_state,
                s.simulation.vm.crt_rng_state
            ) == rng
        );
        ensure!(s.simulation.vm.world.globals[&3] == if selected { 0 } else { other });
        ensure!(s.simulation.native_indicator_pools.active.is_empty());
        ensure!(
            s.simulation.native_indicator_pools.spare.len()
                == if !fame {
                    0
                } else if selected {
                    4
                } else {
                    3
                }
        );
        println!(
            "original person deletion: fame={fame}, selected={selected}, budget={budget}, full session restores={restores}"
        );
        results.push(s.save());
    }
    ensure!(results[0].simulation.state == results[1].simulation.state);
    ensure!(results[0].assets == results[1].assets);
    let name = if !selected {
        "cache/unselected-famous-person-deletion-save.json"
    } else if fame {
        "cache/selected-famous-person-deletion-save.json"
    } else {
        "cache/selected-person-deletion-save.json"
    };
    std::fs::write(root.join(name), serde_json::to_vec(&results[0])?)?;
    Ok(())
}

#[test]
#[ignore = "requires private original residential assets"]
fn original_selection_refills_stars_through_resumable_factories() -> Result<()> {
    use sims_sim::lot_selection::Request;
    use std::collections::BTreeSet;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut initial = Session::load(&index, "UserData/Houses/House07.iff", Options::default())?;
    let people: Vec<_> = initial
        .house
        .household
        .as_ref()
        .unwrap()
        .residents
        .iter()
        .map(|r| r.object as i16)
        .collect();
    let camera = Input {
        viewport: [0, 0, 800, 600],
        center: [0, 0],
        zoom: 3,
        rotation: 0,
    };
    initial.begin_selection(Some(people[0]))?;
    while !initial.advance_selection(camera, 10000)? {}
    // Explicit test input exercises the native Superstar branch; the shipped
    // Newbie household's person data is not changed by ordinary loading.
    initial
        .simulation
        .vm
        .world
        .objects
        .get_mut(&people[0])
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(81, 10);
    let initial = initial.save();
    let mut results = vec![];
    for resume in [false, true] {
        let mut s = Session::restore(&index, initial.clone())?;
        let existing: BTreeSet<_> = s.simulation.vm.world.objects.keys().copied().collect();
        let tick = s.simulation.vm.tick;
        let rng = (s.simulation.vm.rng_state, s.simulation.vm.lot_rng_state);
        let mut expected_crt = s.simulation.vm.crt_rng_state.unwrap();
        for _ in 0..128 {
            sims_vm::rng::next_crt(&mut expected_crt);
        }
        let g = &s.simulation.vm.world.globals;
        let expected_date = [g[&8], g[&7], g[&1], g[&0], g[&5]];
        let mut checkpoints = BTreeSet::new();
        let mut restores = 0;
        let mut done = false;
        s.begin_selection(Some(people[1]))?;
        for boundary in 0..2000 {
            let checkpoint = match s.simulation.pending_native_selection() {
                Some((Request::Refill, _)) => Some((0, 0)),
                Some((Request::FactoryGraphic(id), started)) => {
                    let o = &s.simulation.vm.world.objects[&id];
                    ensure!([45, 46, 47, 24, 21].map(|k| o.data[&k]) == [0; 5]);
                    ensure!(!s.simulation.vm.object_threads.contains_key(&id));
                    Some((id, if started { 2 } else { 1 }))
                }
                _ => s
                    .simulation
                    .vm
                    .object_threads
                    .iter()
                    .find_map(|(&id, &thread)| {
                        (!existing.contains(&id)
                            && s.simulation.vm.threads[&thread].callback.is_some())
                        .then_some((id, 3))
                    }),
            };
            if resume && checkpoint.is_some_and(|c| checkpoints.insert(c)) {
                let saved = s.save();
                if matches!(checkpoint, Some((_, 2))) {
                    let mut orphan = saved.simulation.clone();
                    orphan.state.native_selection = None;
                    ensure!(
                        orphan.restore(&s.program).is_err(),
                        "orphan pre-Init storage was accepted"
                    );
                    let mut altered = serde_json::to_value(&saved.simulation)?;
                    altered["state"]["native_selection"]["star_resource"]["guid"] = 1.into();
                    let altered: sims_sim::Snapshot = serde_json::from_value(altered)?;
                    ensure!(
                        altered.restore(&s.program).is_err(),
                        "changed factory resource was accepted"
                    );
                }
                let simulation = s.simulation.clone();
                let assets = saved.assets.clone();
                s = Session::restore(
                    &index,
                    serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                )
                .with_context(|| {
                    format!("star restore boundary {boundary}, checkpoint {checkpoint:?}")
                })?;
                ensure!(s.simulation == simulation && s.host.snapshot() == assets);
                restores += 1;
            }
            if s.advance_selection(camera, if resume { 1 } else { 10000 })
                .with_context(|| {
                    format!(
                        "star boundary {boundary}, pending {:?}",
                        s.simulation.pending_native_selection()
                    )
                })?
            {
                done = true;
                println!(
                    "star refill resumed={resume}: {boundary} boundaries, {restores} restores"
                );
                break;
            }
        }
        ensure!(done, "star refill did not complete");
        let stars: Vec<_> = s
            .simulation
            .vm
            .world
            .objects
            .keys()
            .filter(|id| !existing.contains(id))
            .copied()
            .collect();
        ensure!(
            stars.len() == 4,
            "initial star pool did not create four objects"
        );
        let pool = &s.simulation.native_indicator_pools;
        ensure!(pool.active == vec![Some(stars[3])]);
        ensure!(pool.spare == stars[..3].iter().copied().map(Some).collect::<Vec<_>>());
        ensure!(s.simulation.vm.world.objects[&people[0]].slots[1] == Some(stars[3]));
        for id in &stars {
            let object = &s.simulation.vm.world.objects[id];
            ensure!(
                object.definition.as_ref().unwrap().guid
                    == sims_vm::selection_indicators::STAR_GUID
            );
            ensure!(object.data[&8] as u16 & 0x4000 != 0);
            ensure!([45, 46, 47, 24, 21].map(|k| object.data[&k]) == expected_date);
            ensure!(s.host.ordinary.graphics.contains_key(id));
        }
        ensure!(s.simulation.vm.tick == tick);
        ensure!((s.simulation.vm.rng_state, s.simulation.vm.lot_rng_state) == rng);
        ensure!(s.simulation.vm.crt_rng_state == Some(expected_crt));
        if resume {
            ensure!(
                checkpoints.iter().filter(|(_, phase)| *phase == 2).count() == 4
                    && checkpoints.iter().filter(|(_, phase)| *phase == 3).count() == 4,
                "did not restore inside all four graphics and Init callbacks"
            );
        }
        // Selecting the famous person returns their star to the spare tail;
        // deselecting reuses it. No second factory or extra CRT draw may occur.
        for target in [Some(people[0]), None] {
            s.begin_selection(target)?;
            let mut complete = false;
            for _ in 0..2000 {
                if s.advance_selection(camera, if resume { 1 } else { 10000 })? {
                    complete = true;
                    break;
                }
            }
            ensure!(complete, "star reuse selection did not complete");
            ensure!(s.simulation.vm.world.objects.len() == existing.len() + 4);
            ensure!(s.simulation.vm.crt_rng_state == Some(expected_crt));
            if target.is_some() {
                ensure!(s.simulation.native_indicator_pools.active.is_empty());
                ensure!(s.simulation.native_indicator_pools.spare.last() == Some(&Some(stars[3])));
            }
        }
        ensure!(s.simulation.native_indicator_pools.active == vec![Some(stars[3])]);
        ensure!(
            s.simulation.native_indicator_pools.spare
                == stars[..3].iter().copied().map(Some).collect::<Vec<_>>()
        );
        let saved = s.save();
        s = Session::restore(&index, saved)?;
        results.push((s.simulation, s.host.snapshot()));
    }
    ensure!(
        results[0] == results[1],
        "star refill budget/resume changed state"
    );
    Ok(())
}

#[test]
#[ignore = "requires private original residential assets"]
fn original_household_selection_attaches_switches_clears_and_resumes() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let initial = Session::load(&index, "UserData/Houses/House07.iff", Options::default())?.save();
    let mut results = vec![];
    for resume in [false, true] {
        let mut s = Session::restore(&index, initial.clone())?;
        let people = s
            .house
            .household
            .as_ref()
            .unwrap()
            .residents
            .iter()
            .map(|r| r.object as i16)
            .collect::<Vec<_>>();
        ensure!(people.len() == 2);
        let primary = s
            .simulation
            .native_lot
            .as_ref()
            .unwrap()
            .indicators
            .primary
            .context("primary absent")?;
        let tick = s.simulation.vm.tick;
        let rng = (
            s.simulation.vm.rng_state,
            s.simulation.vm.lot_rng_state,
            s.simulation.vm.crt_rng_state,
        );
        s.simulation
            .characters
            .get_mut(&people[0])
            .unwrap()
            .character
            .visible = false;
        let hidden_pose = s.simulation.characters[&people[0]]
            .character
            .skin
            .animator
            .pose_generation();
        let mut boundaries = 0;
        let mut restores = 0;
        for (ordinal, target) in [
            Some(people[0]),
            Some(people[1]),
            Some(people[1]),
            None,
            Some(people[0]),
        ]
        .into_iter()
        .enumerate()
        {
            let unchanged = s.simulation.clone();
            s.begin_selection(target)?;
            let before = s.simulation.clone();
            ensure!(s.simulation.run_current_tick(&s.program, 1000).is_err());
            ensure!(s.simulation.finish_registry_pass().is_err());
            ensure!(s.simulation == before);
            let mut done = false;
            for _ in 0..2000 {
                if resume
                    && restores < 12
                    && (boundaries % 7 == 0
                        || s.simulation
                            .vm
                            .threads
                            .values()
                            .any(|t| t.callback.is_some()))
                {
                    let saved = s.save();
                    let assets = saved.assets.clone();
                    let simulation = s.simulation.clone();
                    s = Session::restore(
                        &index,
                        serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                    )?;
                    ensure!(s.simulation == simulation && s.host.snapshot() == assets);
                    restores += 1;
                }
                boundaries += 1;
                if s.advance_selection(
                    Input {
                        viewport: [0, 0, 800, 600],
                        center: [0, 0],
                        zoom: 3,
                        rotation: 0,
                    },
                    if resume { 1 } else { 10000 },
                )
                .with_context(|| {
                    format!(
                        "selection target {target:?}, boundary {boundaries}, request {:?}",
                        s.simulation.pending_native_selection()
                    )
                })? {
                    done = true;
                    break;
                }
            }
            ensure!(done, "selection did not complete");
            if ordinal == 0 {
                ensure!(
                    s.simulation.characters[&people[0]]
                        .character
                        .skin
                        .animator
                        .pose_generation()
                        != hidden_pose,
                    "selection skipped hidden person refresh"
                );
            }
            if ordinal == 2 {
                ensure!(
                    s.simulation == unchanged,
                    "reselecting the same person changed native state"
                );
            }
            ensure!(
                s.simulation
                    .native_selected_person()
                    .map_err(anyhow::Error::msg)?
                    == target
            );
            ensure!(s.simulation.native_lot.as_ref().unwrap().selected_person == target);
            ensure!(s.host.presentation.selected == target);
            ensure!(
                s.simulation.vm.tick == tick
                    && (
                        s.simulation.vm.rng_state,
                        s.simulation.vm.lot_rng_state,
                        s.simulation.vm.crt_rng_state
                    ) == rng
            );
            if let Some(id) = target {
                ensure!(s.simulation.vm.world.objects[&id].slots[1] == Some(primary));
                ensure!(s.simulation.vm.world.objects[&primary].container == Some((id, 1)));
                ensure!(
                    s.simulation.vm.world.positions[&primary]
                        == s.simulation.vm.world.positions[&id]
                );
                ensure!(
                    !s.host.ordinary.graphics[&primary]
                        .ready
                        .avatar
                        .avatar
                        .parts
                        .is_empty()
                );
            } else {
                ensure!(s.simulation.vm.world.objects[&primary].container.is_none());
                ensure!(s.simulation.vm.world.positions[&primary].x == -16);
            }
        }
        println!("selection resumed={resume}; services={boundaries}; restores={restores}");
        let mut ticks = 0;
        let mut boundary = None;
        for _ in 0..100 {
            match s.step() {
                Ok(_) => ticks += 1,
                Err(error)
                    if error.to_string().contains(
                        "full routing wrapper configuration/publication remains UNVERIFIED",
                    ) =>
                {
                    boundary = Some(error.to_string());
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        println!(
            "strict routing after selection: +{ticks}; protected boundary={}",
            boundary.is_some()
        );
        results.push((s.simulation, s.host.snapshot(), ticks, boundary));
    }
    ensure!(
        results[0] == results[1],
        "selection budget/resume changed state"
    );
    Ok(())
}
