use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Options, Progress, Saved, Session};
use sims_sim::lot_route::path::Policy;

#[path = "support/registry_startup.rs"]
mod registry_startup;

#[test]
#[ignore = "archival pre-registry checkpoint; regenerate selected-famous-person deletion with the original scheduler before rerunning"]
fn pooled_graphic_deletion_unregisters_and_restores() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let saved: Saved = serde_json::from_slice(&std::fs::read(
        root.join("cache/selected-famous-person-deletion-save.json"),
    )?)?;
    let pools = saved.simulation.state.native_indicator_pools.clone();
    let id = pools
        .spare
        .iter()
        .flatten()
        .next()
        .copied()
        .context("spare star absent")?;
    let mut expected_pools = pools.clone();
    expected_pools.spare.remove(
        expected_pools
            .spare
            .iter()
            .position(|v| *v == Some(id))
            .unwrap(),
    );
    let camera = sims_sim::pose::portrait::view::Input {
        viewport: [0, 0, 800, 600],
        center: [0, 0],
        zoom: 3,
        rotation: 0,
    };
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
        ensure!(s.simulation.native_indicator_pools == expected_pools);
        // Save at the prelude boundary too: first-match erases must not replay.
        s = Session::restore(&index, s.save())?;
        let mut checkpoints = std::collections::BTreeSet::new();
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
                .with_context(|| format!("pooled deletion boundary {boundary}"))?;
            s.service_selection_assets(camera, budget)?;
            s.host.service(&mut s.simulation, &mut s.program)?;
            let checkpoint = format!(
                "{:?}:{:?}:{:?}",
                s.simulation.pending_removal_graphic(),
                s.simulation.retired_graphics,
                s.host.ordinary.retirements
            );
            if resume && checkpoints.insert(checkpoint) {
                let before = s.save();
                s = Session::restore(
                    &index,
                    serde_json::from_slice(&serde_json::to_vec(&before)?)?,
                )?;
                ensure!(
                    s.simulation == before.simulation.state && s.host.snapshot() == before.assets
                );
            }
            if finished {
                done = true;
                break;
            }
        }
        ensure!(done);
        s.simulation
            .take_synchronous_removal()
            .map_err(anyhow::Error::msg)?;
        ensure!(s.simulation.native_indicator_pools == expected_pools);
        ensure!(
            !s.simulation.vm.world.objects.contains_key(&id)
                && !s.host.ordinary.graphics.contains_key(&id)
        );
        ensure!(s.simulation.retired_graphics.is_empty() && s.host.ordinary.retirements.is_empty());
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
            "pooled star {id}: removed, budget={budget}, full session restores={}",
            checkpoints.len()
        );
        results.push(s.save());
    }
    ensure!(
        results[0].simulation == results[1].simulation && results[0].assets == results[1].assets
    );
    std::fs::write(
        root.join("cache/pooled-star-deletion-save.json"),
        serde_json::to_vec(&results[0])?,
    )?;
    Ok(())
}

#[test]
#[ignore = "requires original neighborhoods/assets and native family reader report"]
fn original_family_state_matches_native_records_and_restores_after_removal() -> Result<()> {
    use sha2::{Digest, Sha256};
    use sims_vm::family::{self, Change, Host};
    struct Presenter;
    impl Host for Presenter {
        fn close_panel(&mut self, _world: &mut sims_vm::World) -> Result<(), String> {
            Ok(())
        }
        fn residential(&mut self, w: &sims_vm::World) -> Result<bool, String> {
            Ok(sims_vm::zoning::is_residential(
                &w.person.as_ref().unwrap().lot_zoning,
                w.globals[&10],
            ))
        }
        fn broadcast(&mut self, w: &mut sims_vm::World, c: Change) -> Result<(), String> {
            w.person.as_mut().unwrap().family_changes.push(c);
            Ok(())
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/native-original-family-metadata.json"),
    )?)?;
    let rows = report["families"]
        .as_array()
        .context("native family records")?;
    ensure!(rows.len() == 8);
    for row in rows {
        let source = row["source"].as_str().unwrap();
        let iff = sims_formats::iff::Iff::parse(index.resources[index.find(source)?].read()?)?;
        let data = iff.data(
            iff.get(b"FAMI", row["identity"].as_u64().unwrap() as u16)
                .context("original FAMI absent")?,
        );
        ensure!(format!("{:x}", Sha256::digest(data)) == row["sha256"].as_str().unwrap());
        let family = sims_formats::neighborhood::Family::parse(data)?;
        ensure!(
            serde_json::json!(family.flags) == row["original"]["flags"]
                && serde_json::json!(family.members) == row["original"]["members"]
        );
    }
    for source in [
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let mut session = Session::load(
            &index,
            source,
            Options {
                routing: Policy::InferredRectangles,
                ..Default::default()
            },
        )?;
        let people = session.simulation.vm.world.person.as_ref().unwrap();
        for (&id, f) in &session.house.families {
            ensure!(
                people.family_metadata[&(id as i16)]
                    == family::Metadata {
                        flags: f.flags,
                        friend_count: Some(f.friends),
                        member_guids: Some(f.members.clone()),
                        dirty: true
                    }
            );
        }
        let mut lost = session.save();
        lost.simulation
            .state
            .vm
            .world
            .person
            .as_mut()
            .unwrap()
            .family_metadata
            .clear();
        let error = Session::restore(&index, lost)
            .err()
            .context("lost original family metadata accepted")?;
        ensure!(error.to_string().contains("family metadata absent"));
        let resident = session
            .house
            .household
            .as_ref()
            .context("household absent")?
            .residents[0]
            .clone();
        let neighbor = resident.neighbor;
        let family = people.neighbors[&neighbor].person[&61];
        let dirty = people.neighbors[&neighbor].dirty;
        let mut before = Session::restore(&index, session.save())?;
        ensure!(
            family::remove(&mut session.simulation.vm.world, neighbor, &mut Presenter)
                .map_err(anyhow::Error::msg)?
        );
        ensure!(
            family::remove(&mut before.simulation.vm.world, neighbor, &mut Presenter)
                .map_err(anyhow::Error::msg)?
        );
        ensure!(
            session.simulation == before.simulation
                && session.host.snapshot() == before.host.snapshot()
        );
        let people = session.simulation.vm.world.person.as_ref().unwrap();
        ensure!(
            people.neighbors[&neighbor].person[&61] == 0
                && people.neighbors[&neighbor].dirty == dirty
        );
        ensure!(
            people.family_metadata[&family].flags
                == session.house.families[&(family as u16)].flags & !8
        );
        ensure!(
            people.family_changes
                == vec![Change {
                    family,
                    neighbor,
                    guid: resident.guid
                }]
        );
        let mut resumed = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&session.save())?)?,
        )?;
        ensure!(
            session.simulation == resumed.simulation
                && session.host.snapshot() == resumed.host.snapshot()
        );
        for _ in 0..3 {
            let actual = session.step().map_err(|e| format!("{e:#}"));
            ensure!(actual == resumed.step().map_err(|e| format!("{e:#}")));
            ensure!(
                session.simulation == resumed.simulation
                    && session.host.snapshot() == resumed.host.snapshot()
            );
            if actual.is_err() {
                break;
            }
        }
        println!(
            "{source}: native loaded family flags, explicit removal, notifications and complete restored continuation match"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires original Mashuga assets; controlled pajama input at the real skill-reload boundary"]
fn original_pajama_reload_retains_process_cache_and_profile_publications() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut session = Session::load(
        &index,
        "UserData2/Houses/House09.iff",
        Options {
            routing: Policy::InferredRectangles,
            ..Default::default()
        },
    )?;
    // Controlled headless Live/Play inputs after source loading.
    session.simulation.vm.world.globals.insert(17, 0);
    session.simulation.vm.world.globals.insert(19, 0);
    for _ in 0..100 {
        let result = session
            .simulation
            .run_current_tick(&session.program, 100_000);
        if matches!(&result, Ok(step) if step.stop == sims_sim::StepStop::Dialog) {
            registry_startup::answer_startup_dialog(&mut session)?;
            continue;
        }
        if !session.simulation.pending_skill_profiles().is_empty() {
            break;
        }
        if session
            .host
            .service(&mut session.simulation, &mut session.program)?
            == 0
        {
            anyhow::bail!(
                "profile boundary absent: {:?}",
                result.map_err(anyhow::Error::msg)?.stop
            );
        }
    }
    let id = session
        .simulation
        .pending_skill_profiles()
        .first()
        .context("skill reload absent")?
        .0;
    // Deliberately supplied selector input, not claimed as autonomously reached.
    // Use the real profile, age, body shape, cache ownership and continuation.
    let person = session.simulation.vm.world.objects.get_mut(&id).unwrap();
    let definition = person.definition.clone().unwrap();
    person.person.as_mut().unwrap().insert(8, 5);
    ensure!(!session.host.snapshot().clothing.loaded());
    let before = session.save();
    let mut control = Session::restore(
        &index,
        serde_json::from_slice(&serde_json::to_vec(&before)?)?,
    )?;
    let mut captured = false;
    for _ in 0..100 {
        let completed = session
            .host
            .service(&mut session.simulation, &mut session.program)?;
        ensure!(
            completed
                == control
                    .host
                    .service(&mut control.simulation, &mut control.program)?
        );
        ensure!(
            session.simulation == control.simulation
                && session.host.snapshot() == control.host.snapshot()
        );
        let assets = session.host.snapshot();
        if !captured && assets.clothing.loaded() {
            ensure!(assets.reloads[&id].service.clothing_used());
            let upgraded = session
                .simulation
                .vm
                .world
                .body_profiles
                .get(&definition)
                .map_err(anyhow::Error::msg)?;
            ensure!(!upgraded[32].is_empty());
            let report: serde_json::Value = serde_json::from_slice(&std::fs::read(
                root.join("cache/original-clothing-profile-native.json"),
            )?)?;
            let expected = report["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["definition"]["source"] == definition.source)
                .context("native resident comparison absent")?;
            ensure!(
                serde_json::json!(upgraded) == expected["original"]["profile"],
                "live migration differs from native profile"
            );
            let saved = session.save();
            session = Session::restore(
                &index,
                serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
            )?;
            ensure!(
                session.simulation == control.simulation
                    && session.host.snapshot() == control.host.snapshot()
            );
            let mut dropped = assets.clone();
            dropped.clothing = Default::default();
            ensure!(
                sims_content::character_loading::live::Host::restore(
                    &index,
                    session.host.presentation.clone(),
                    dropped
                )
                .is_err(),
                "reload accepted a lost clothing cache"
            );
            let mut damaged = serde_json::to_value(&assets)?;
            damaged["clothing"]["names"]["bodies"][0][0] = serde_json::json!(33);
            ensure!(
                sims_content::character_loading::live::Host::restore(
                    &index,
                    session.host.presentation.clone(),
                    serde_json::from_value(damaged)?
                )
                .is_err(),
                "reload accepted altered clothing name order/content"
            );
            captured = true;
        }
        if session.simulation.pending_skill_profiles().is_empty() {
            break;
        }
        ensure!(completed > 0, "pajama reload made no progress");
    }
    ensure!(captured && session.simulation.pending_skill_profiles().is_empty());
    let body = &session.host.characters[&id].avatar.records.entries[4];
    ensure!(
        body.name.starts_with("pajama"),
        "first reload reselected migrated sleepwear"
    );
    let saved = session.save();
    session = Session::restore(
        &index,
        serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
    )?;
    for _ in 0..3 {
        ensure!(session.step()? == control.step()?);
        ensure!(
            session.simulation == control.simulation
                && session.host.snapshot() == control.host.snapshot()
        );
    }
    println!(
        "{id}: live pajama reload publishes native profile, retains legacy first selection and restores process cache before/during/after service"
    );
    Ok(())
}

#[test]
#[ignore = "requires private original houses and assets"]
fn original_profile_reload_restores_before_and_during_its_asset_service() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut session = Session::load(
        &index,
        "UserData2/Houses/House09.iff",
        Options {
            routing: Policy::InferredRectangles,
            ..Default::default()
        },
    )?;
    // Controlled headless Live/Play inputs after source loading.
    session.simulation.vm.world.globals.insert(17, 0);
    session.simulation.vm.world.globals.insert(19, 0);
    for _ in 0..100 {
        let result = session
            .simulation
            .run_current_tick(&session.program, 100_000);
        if matches!(&result, Ok(step) if step.stop == sims_sim::StepStop::Dialog) {
            registry_startup::answer_startup_dialog(&mut session)?;
            continue;
        }
        if !session.simulation.pending_skill_profiles().is_empty() {
            break;
        }
        let completed = session
            .host
            .service(&mut session.simulation, &mut session.program)?;
        if completed == 0 {
            let step = result.map_err(anyhow::Error::msg)?;
            anyhow::bail!(
                "expected profile reload before first tick, got {:?}",
                step.stop
            );
        }
    }
    ensure!(
        !session.simulation.pending_skill_profiles().is_empty(),
        "first skill lookup did not consume Reset profile flag"
    );
    for started in [false, true] {
        if started {
            ensure!(
                session
                    .host
                    .service(&mut session.simulation, &mut session.program)?
                    > 0
            );
        }
        ensure!(
            session
                .simulation
                .pending_skill_profiles()
                .iter()
                .all(|(_, value)| *value == started)
        );
        let saved = session.save();
        let mut resumed = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
        )?;
        ensure!(
            resumed.simulation == session.simulation
                && resumed.host.snapshot() == session.host.snapshot()
        );
        let mut control = Session::restore(&index, saved.clone())?;
        ensure!(resumed.step()? == control.step()?);
        ensure!(
            resumed.simulation == control.simulation
                && resumed.host.snapshot() == control.host.snapshot()
        );
        if started {
            let mut missing = saved;
            missing.assets.reloads.clear();
            ensure!(
                Session::restore(&index, missing).is_err(),
                "lost profile asset job was accepted"
            );
        }
    }
    println!(
        "Mashuga profile reload restores before/inside asset service, re-links fresh resources, and continues identically"
    );
    Ok(())
}

#[test]
#[ignore = "requires private original houses and assets"]
fn original_sessions_restore_in_a_fresh_program_and_continue_identically() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let mut session = Session::load(
            &index,
            source,
            Options {
                routing: Policy::InferredRectangles,
                ..Default::default()
            },
        )?;
        // Controlled headless Live/Play inputs after source loading.
        session.simulation.vm.world.globals.insert(17, 0);
        session.simulation.vm.world.globals.insert(19, 0);
        ensure!(
            session.save().headline_catalog,
            "fresh household omitted shared headline resources"
        );
        let start = session.simulation.vm.tick;
        let indicators = &session.simulation.native_lot.as_ref().unwrap().indicators;
        ensure!(session.host.ordinary.graphics.len() == 2);
        for id in [indicators.primary, indicators.secondary] {
            let id = id.context("load-time indicator absent")?;
            let graphic = &session.host.ordinary.graphics[&id];
            ensure!(graphic.owner.ready == 1 && graphic.ready.avatar.avatar.parts.is_empty());
            ensure!(session.simulation.vm.world.objects[&id].data[&8] & 0x4000 != 0);
        }
        for _ in 0..80 {
            ensure!(registry_startup::step_startup_inputs(&mut session)? == Progress::Tick);
        }
        let saved: Saved = serde_json::from_slice(&serde_json::to_vec(&session.save())?)?;
        if source == "UserData/Houses/House07.iff" {
            let mut missing_graphics = saved.clone();
            missing_graphics.assets.ordinary = Default::default();
            let legacy = Session::restore(&index, missing_graphics)?;
            ensure!(
                legacy.host.ordinary.graphics.is_empty() && legacy.simulation == session.simulation,
                "legacy diagnostic restore silently rebuilt missing graphics or consumed CRT draws"
            );
            let mut inconsistent = saved.clone();
            inconsistent.options.routing = Policy::Strict;
            let error = Session::restore(&index, inconsistent)
                .err()
                .expect("mismatched routing policy accepted");
            ensure!(error.to_string().contains("saved routing policy differs"));
        }
        let mut resumed = Session::restore(&index, saved)?;
        ensure!(
            session.simulation == resumed.simulation
                && session.host.snapshot() == resumed.host.snapshot()
        );
        for _ in 0..20 {
            ensure!(
                registry_startup::step_startup_inputs(&mut session)?
                    == registry_startup::step_startup_inputs(&mut resumed)?
            );
        }
        ensure!(
            session.simulation == resumed.simulation
                && session.host.snapshot() == resumed.host.snapshot()
        );
        ensure!(session.simulation.vm.tick == start + 100);
        println!("{source}: +100; fresh-program restoration and 20 subsequent ticks identical");
    }
    Ok(())
}

#[test]
#[ignore = "archival pre-registry v82 checkpoint; regenerate the Car Pool boundary with the original scheduler before rerunning"]
fn original_headline_dependency_preserves_legacy_state_and_reconstructs_exactly() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let capture: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/mashuga-dynamic-followup/final.json"),
    )?)?;
    let old: Saved = serde_json::from_value(capture["session"].clone())?;
    ensure!(!old.headline_catalog);
    let mut s = Session::restore(&index, old.clone())?;
    ensure!(
        serde_json::to_value(s.save())? == serde_json::to_value(&old)?,
        "legacy restore changed its saved dependency or state"
    );
    ensure!(s.step()? == Progress::Dialog);
    ensure!(
        serde_json::to_value(s.save())? == serde_json::to_value(&old)?,
        "unanswered legacy modal hydrated resources"
    );
    let old_fingerprint = s.program.fingerprint();
    s.prepare_headline_catalog()?;
    let ready = s.save();
    ensure!(ready.headline_catalog);
    ensure!(
        ready.simulation.state == old.simulation.state
            && ready.assets == old.assets
            && ready.view == old.view,
        "catalog hydration changed live state"
    );
    ensure!(
        ready.source_hashes == old.source_hashes
            && ready.modules == old.modules
            && ready.animations == old.animations
    );
    let catalog = s.program.modules[&s.program.globals]
        .headline_catalog
        .as_ref()
        .context("catalog absent")?
        .clone();
    ensure!(
        !catalog.frames.is_empty() && catalog.source.to_ascii_lowercase().ends_with("sprites.iff")
    );
    ensure!(
        s.program.fingerprint() != old_fingerprint,
        "resource catalog missing from fingerprint"
    );
    let restored = Session::restore(
        &index,
        serde_json::from_slice(&serde_json::to_vec(&ready)?)?,
    )?;
    ensure!(serde_json::to_value(restored.save())? == serde_json::to_value(&ready)?);
    ensure!(
        restored.program.modules[&restored.program.globals]
            .headline_catalog
            .as_ref()
            == Some(&catalog)
    );
    let mut missing = ready.clone();
    missing.headline_catalog = false;
    let error = Session::restore(&index, missing)
        .err()
        .context("missing catalog dependency accepted")?;
    ensure!(error.to_string() == "room snapshot version/content mismatch");
    s.prepare_headline_catalog()?;
    ensure!(
        serde_json::to_value(s.save())? == serde_json::to_value(&ready)?,
        "repeated hydration changed saved state"
    );
    std::fs::write(
        root.join("cache/headline-dependency-check.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "legacy_restored_identically":true,"unanswered_modal_unchanged":true,"simulation_and_assets_unchanged_on_hydration":true,
            "source":catalog.source,"content_hash":catalog.content_hash,"sprites":catalog.frames.len(),"clown_sprite_617_frames":catalog.frames.get(&617),
            "saved_catalog_reconstructed_identically":true,"missing_dependency_rejected":error.to_string(),"idempotent":true
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private Complete Collection native skeleton registry"]
fn original_named_skeletons_use_ndx_catalogs_and_retain_sources() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut rows = vec![];
    for (name, filename) in [
        ("adult", "adult-skeleton.cmx.bcf"),
        ("child", "child-skeleton.cmx.bcf"),
        ("dog", "dog_skeleton.cmx.bcf"),
        ("kat", "kat_skeleton.cmx.bcf"),
    ] {
        let start = std::time::Instant::now();
        let (skeleton, id) = sims_content::avatar::load_skeleton(&index, name)?;
        let seconds = start.elapsed().as_secs_f64();
        let resource = &index.resources[id];
        ensure!(resource.name.rsplit(['\\', '/']).next() == Some(filename));
        let raw = sims_formats::avatar::read_bcf(&resource.read()?)?;
        ensure!(
            raw.skeletons.len() == 1 && skeleton == raw.skeletons[0],
            "registered skeleton differs from its original BCF"
        );
        let bound = sims_content::saved_avatar::BoundAvatar::empty(&index, name, 42)?;
        ensure!(
            bound.avatar.skeleton == skeleton
                && bound.avatar.sources.contains_key(&resource.locator)
        );
        rows.push(serde_json::json!({"name":name,"source":resource.locator,"bones":skeleton.bones.len(),"lookup_seconds":seconds}));
    }
    std::fs::write(
        root.join("cache/native-named-skeletons.json"),
        serde_json::to_vec_pretty(&rows)?,
    )?;
    Ok(())
}
#[test]
#[ignore = "private original pet animation and skeleton resources"]
fn original_pet_empty_channels_load_and_preserve_toe_pose() -> anyhow::Result<()> {
    use sims_sim::pose::sampling;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = sims_content::Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut program = sims_vm::Program::default();
    let names = ["d2o-turn-90-cw".to_owned()].into();
    let legacy = sims_content::assets::load_named_animations(&index, &mut program, &names)?;
    assert_eq!(legacy.unresolved, vec!["d2o-turn-90-cw"]);
    assert!(program.animations.is_empty());
    let native = sims_content::assets::load_named_native_animations(&index, &mut program, &names)?;
    assert!(native.unresolved.is_empty());
    assert_eq!(native.loaded_clips, 1);
    let asset = &program.animations["d2o-turn-90-cw"];
    let (skeleton, source) = sims_content::avatar::load_skeleton(&index, "dog")?;
    let empty: Vec<_> = asset
        .clip
        .motions
        .iter()
        .filter(|m| m.frames == 0)
        .collect();
    assert_eq!(
        empty.iter().map(|m| m.bone.as_str()).collect::<Vec<_>>(),
        vec!["L_TOE0", "R_TOE0"]
    );
    assert!(
        empty
            .iter()
            .all(|m| m.translation.is_none() && m.rotation.is_none())
    );
    for frame in [0., 0.5, 1., 15.25, 29.] {
        let locals = sampling::at_frame(&skeleton, &asset.clip, &asset.samples, frame, false)
            .map_err(anyhow::Error::msg)?;
        for m in &empty {
            let i = skeleton
                .bones
                .iter()
                .position(|b| b.name == m.bone)
                .unwrap();
            assert_eq!(locals[i].translation, skeleton.bones[i].translation);
            assert_eq!(locals[i].rotation, skeleton.bones[i].rotation);
        }
    }
    std::fs::write(
        root.join("cache/original-pet-pose-inputs.json"),
        serde_json::to_vec(&serde_json::json!([
            {"source":asset.source,"object":310,"animation":asset,"bones":skeleton.bones}
        ]))?,
    )?;
    std::fs::write(
        root.join("cache/original-pet-empty-motion-report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "animation":asset.source,"samples":asset.samples_source,"skeleton":index.resources[source].locator,
            "empty_bones":empty.iter().map(|m|&m.bone).collect::<Vec<_>>(),"frames":asset.clip.frames,
            "empty_channels_preserve_pose":true,"legacy_preload_unchanged":true
        }))?,
    )?;
    Ok(())
}
