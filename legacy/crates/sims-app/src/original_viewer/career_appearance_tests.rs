//! Independent Mashuga checkpoint; no player save or live GUI is accessed.
use super::*;
use clap::Parser;
use serde_json::json;

#[test]
#[ignore = "private Mashuga assets and explicit fresh SIMS_CAREER_APPEARANCE_OUTPUT; headless GPU"]
fn work_outfit_and_cold_draw_resume() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("career appearance test panicked")
}
fn run() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_CAREER_APPEARANCE_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    let source = repo.join("cache/no-selected-person-iced-20261023-third/no-selected-person.json");
    let destination = output.join("research-save.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData2/Houses/House09.iff",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        source.to_str().unwrap(),
        "--save",
        destination.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    let mut inputs = vec![];
    transport_tests::drain(&mut v, &mut inputs)?;
    let index = v.session.as_ref().unwrap().index().clone();
    let start = v.session.as_ref().unwrap().save_managed()?;
    let names = sims_content::clothing_catalog::installed_names(&index)?;
    let mut native_inputs = vec![];
    let mut rows = vec![];
    // Both actual residents, plus a valid uniform-bearing level and missing
    // career. Actor fields are explicitly supplied branch inputs; these are
    // not claimed to be a complete native carpool/workday capture.
    for (id, job, level) in [(195, 14, 0), (196, 10, 1), (195, 1, 3), (195, 32767, 0)] {
        let mut warm = Session::restore(&index, start.clone())?;
        let definition = warm.simulation.vm.world.objects[&id]
            .definition
            .clone()
            .unwrap();
        let profiles = warm
            .simulation
            .vm
            .world
            .body_profiles
            .get(&definition)
            .map_err(anyhow::Error::msg)?
            .to_vec();
        let catalog = warm.program.careers.as_ref().unwrap();
        let records=catalog.ids.iter().zip(catalog.records.as_ref().unwrap()).filter(|(key,_)| **key==job).map(|(key,record)| json!({"id":key,"levels":record.levels.iter().map(|l| l.strings[1..5].to_vec()).collect::<Vec<_>>()})).collect::<Vec<_>>();
        let o = warm.simulation.vm.world.objects.get_mut(&id).unwrap();
        let p = o.person.as_mut().unwrap();
        p.insert(8, 3);
        p.insert(56, job as i16);
        p.insert(57, level);
        o.career_appearance = None;
        let fields = p.clone();
        warm.host
            .characters
            .get_mut(&id)
            .unwrap()
            .appearance_owner
            .selection
            .cached[0] = -1;
        ensure!(warm.person_draw_preparation_ready());
        warm.begin_person_draw_preparation(vec![id])?;
        let pending = warm.save_managed()?;
        let mut cold = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&pending)?)?,
        )?;
        let before = warm.simulation.clone();
        ensure!(cold.step().is_err());
        while !warm.advance_person_draw_preparation(1)? {}
        while !cold.advance_person_draw_preparation(1)? {}
        ensure!(
            serde_json::to_vec(&warm.save())? == serde_json::to_vec(&cold.save())?,
            "cold partial draw changed the result"
        );
        let cache = warm.simulation.vm.world.objects[&id]
            .career_appearance
            .clone()
            .context("work cache absent")?;
        let mut expected = before.clone();
        expected
            .vm
            .world
            .objects
            .get_mut(&id)
            .unwrap()
            .career_appearance = Some(cache.clone());
        expected.vm.world.body_profiles = warm.simulation.vm.world.body_profiles.clone();
        // Exact native binding lifecycle invalidations, no animator sampling.
        for (actor, resident) in &mut expected.characters {
            let actual = &warm.simulation.characters[actor];
            resident.controls.geometry_generation = actual.controls.geometry_generation;
            resident.character.placement.dirty = actual.character.placement.dirty;
        }
        ensure!(
            warm.simulation == expected,
            "career draw changed scheduler, RNG, pose or other simulation state"
        );
        let strings = |values: &Vec<Vec<u8>>| {
            values
                .iter()
                .map(|b| sims_formats::strings::decode(b, 1))
                .collect::<Vec<_>>()
        };
        native_inputs.push(json!({"profile":profiles,"records":records,"bodies":strings(&names.bodies),"textures":strings(&names.textures.names),"career":job,"level":level,"age":fields[&58],"species":fields[&65],"expected":{"body":sims_formats::strings::decode(&cache.body,1),"texture":sims_formats::strings::decode(&cache.texture,1),"extra":sims_formats::strings::decode(&cache.extra,1),"profile":warm.simulation.vm.world.body_profiles.get(&definition).map_err(anyhow::Error::msg)?}}));
        let after = serde_json::to_vec(&warm.save())?;
        warm.begin_person_draw_preparation(vec![id])?;
        while !warm.advance_person_draw_preparation(1)? {}
        ensure!(
            after == serde_json::to_vec(&warm.save())?,
            "cached draw replayed work"
        );
        let prepared = warm.save_managed()?;
        let mut restored = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&prepared)?)?,
        )?;
        restored.begin_person_draw_preparation(vec![id])?;
        while !restored.advance_person_draw_preparation(1)? {}
        ensure!(after == serde_json::to_vec(&restored.save())?);
        let body = &warm.host.characters[&id].avatar.records.entries[4];
        rows.push(json!({"actor":id,"career":job,"level":level,"tick":warm.simulation.vm.tick,"cache":cache,"body":[body.name,body.texture],"full_state_cold_resume":true,"all_rng_threads_pose_unchanged":true}));
        if id == 195 && job == 14 {
            std::fs::write(output.join("pending.json"), serde_json::to_vec(&pending)?)?;
            std::fs::write(output.join("prepared.json"), serde_json::to_vec(&prepared)?)?;
            v.session = Some(warm);
            v.refresh()?;
            let state = serde_json::to_vec(&v.session.as_ref().unwrap().save())?;
            render_headless_element(&root, &output.join("sylvia-work-return-iced.png"), v.view())?;
            let scene = v.device.as_ref().unwrap().scene();
            let pixels = sims_render::software::render(scene);
            image::save_buffer(
                output.join("sylvia-work-return-world.png"),
                &pixels,
                scene.width,
                scene.height,
                image::ColorType::Rgba8,
            )?;
            ensure!(
                state == serde_json::to_vec(&v.session.as_ref().unwrap().save())?,
                "render progressed VM"
            );
            let cold_scene = OriginalHouse::default()
                .scene(&restored, OriginalHouse::native_camera(&restored)?)?;
            ensure!(
                pixels == sims_render::software::render(&cold_scene),
                "cold render differs"
            );
            let s = v.session.as_mut().unwrap();
            s.simulation
                .vm
                .world
                .objects
                .get_mut(&id)
                .unwrap()
                .person
                .as_mut()
                .unwrap()
                .insert(8, 0);
            s.begin_person_draw_preparation(vec![id])?;
            while !s.advance_person_draw_preparation(1)? {}
            ensure!(s.host.characters[&id].appearance_owner.selection.cached[0] == 0);
            ensure!(s.simulation.vm.world.objects[&id].career_appearance == Some(cache));
            std::fs::write(
                output.join("everyday.json"),
                serde_json::to_vec(&s.save_managed()?)?,
            )?;
        }
    }
    std::fs::write(
        output.join("native-inputs.json"),
        serde_json::to_vec(&native_inputs)?,
    )?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"source":source,"cases":rows,"player_save_accessed":false,"native_workday_capture":false,"new_strict_gameplay_ticks":0}),
        )?,
    )?;
    Ok(())
}
