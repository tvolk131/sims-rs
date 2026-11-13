use super::*;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../tests/fixtures/hud-mood-gauge.json")).unwrap()
}

#[test]
fn gauge_crops_match_original_draw_instructions() {
    let report = fixture();
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 890);
    for (index, case) in cases.iter().enumerate() {
        let input = &case["input"];
        let mood = if input["selected"].as_bool().unwrap() {
            f32::from_bits(input["mood_bits"].as_u64().unwrap() as u32)
        } else {
            0.
        };
        let strips = if input["visible"].as_bool().unwrap() {
            mood_gauge_strips(mood).unwrap()
        } else {
            vec![]
        };
        let origin = [
            input["origin"][0].as_i64().unwrap(),
            input["origin"][1].as_i64().unwrap(),
        ];
        let actual: Vec<_> = strips.into_iter().map(|s| {
            let x = origin[0] + 193 + i64::from(s.position[0]);
            let y = origin[1] + 50 + i64::from(s.position[1]);
            serde_json::json!({
                "resource":s.resource,"source":s.source,
                "target":[x,y,x+i64::from(s.source[2]-s.source[0]),y+i64::from(s.source[3]-s.source[1])]
            })
        }).collect();
        assert_eq!(
            serde_json::json!(actual),
            case["original"]["blits"],
            "case{index}, mood{mood}"
        );
        assert!(
            case["original"]["object_storage_unchanged"]
                .as_bool()
                .unwrap()
        );
    }
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX] {
        assert!(mood_gauge_strips(invalid).is_none());
    }
}

#[test]
fn gauge_owner_matches_original_guid_cache_and_registry_chain() -> Result<()> {
    let report = fixture();
    let cases = report["owner_cases"].as_array().unwrap();
    ensure!(cases.len() == 144);
    for (index, case) in cases.iter().enumerate() {
        let input = &case["input"];
        let mut world = sims_vm::World::default();
        world.image_cache.entries = input["cache"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| sims_vm::image_cache::Entry::new(v.as_u64().unwrap() as u32))
            .collect();
        let mut order = vec![];
        let mut selected = None;
        for p in input["people"].as_array().unwrap() {
            let id = p["id"].as_i64().unwrap() as i16;
            let flags = p["flags"].as_u64().unwrap() as u32;
            let mut motives = [100f32.to_bits(); 16];
            motives[3] = p["mood_bits"].as_u64().unwrap() as u32;
            world.objects.insert(
                id,
                sims_vm::Object {
                    native_flags: Some(flags),
                    motives: Some(motives),
                    definition: Some(sims_vm::Definition {
                        source: "gauge fixture".into(),
                        object: id as u16,
                        guid: p["guid"].as_u64().unwrap() as u32,
                        slots: 0,
                    }),
                    ..Default::default()
                },
            );
            order.push(id);
            if selected.is_none() && flags & 2 != 0 {
                selected = Some(id);
            }
        }
        if input["active"].as_bool().unwrap() {
            world.person = Some(sims_vm::person::WorldState {
                person_order: order,
                ..Default::default()
            });
        } else {
            selected = None;
        }
        let before = serde_json::to_vec(&world)?;
        let actual = gauge_mood(&world, selected)?;
        ensure!(
            u64::from(actual.to_bits()) == case["original"]["mood_bits"].as_u64().unwrap(),
            "owner case{index}"
        );
        ensure!(
            before == serde_json::to_vec(&world)?,
            "gauge read changed world"
        );
        ensure!(
            case["original"]["object_storage_unchanged"]
                .as_bool()
                .unwrap()
        );
    }
    Ok(())
}

fn repo() -> Result<std::path::PathBuf> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?)
}

fn original_art(index: &Index, id: i32) -> Result<image::RgbaImage> {
    sims_content::dialog::ui_bitmaps::Bitmaps::load(index)?
        .image(
            index,
            sims_vm::dialog::GlobalBitmap {
                id,
                button_cell: false,
            },
            1,
            [255, 0, 255],
        )?
        .context("gauge artwork absent")
}

#[test]
#[ignore = "installed original UI artwork and captured original Live frame"]
fn gauge_artwork_and_native_reference_pixels_match() -> Result<()> {
    let repo = repo()?;
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let icons = dialog_view::Icons::load(&index)?;
    for id in [4506, 4510] {
        let art = original_art(&index, id)?;
        let (width, height, frames) = icons.hud(id).context("gauge image not loaded")?;
        ensure!((*width, *height, frames.len()) == (27, 25, 1));
        for height in [2, 5, 10, 15, 20, 25] {
            ensure!(icons.mood_strip(id, height).is_some());
        }
        // These are opaque cropped strips, including the tapered gauge's
        // shaded surround. Unlike mode trim, they contain no magenta key.
        ensure!(art.pixels().all(|p| p[3] == 255));
    }
    let reference =
        image::open(repo.join("cache/visual-parity-20261021/original-live-expanded.png"))?
            .into_rgba8();
    let strip = mood_gauge_strips(71.49).unwrap().remove(0);
    ensure!(strip.resource == 4506 && strip.source == [0, 5, 27, 25]);
    let art = original_art(&index, strip.resource)?;
    let mut count = 0;
    for y in strip.source[1]..strip.source[3] {
        for x in 0..27 {
            let p = art.get_pixel(x, y);
            if p[3] == 0 {
                continue;
            }
            let actual = reference.get_pixel(425 + x, 503 + y);
            ensure!(
                (0..3).all(|c| p[c].abs_diff(actual[c]) <= 7),
                "original gauge pixel({x},{y}) differs"
            );
            count += 1;
        }
    }
    ensure!(count > 100);
    Ok(())
}

fn retained(v: &Viewer) -> Result<Vec<u8>> {
    let s = v.session.as_ref().context("session absent")?;
    Ok(serde_json::to_vec(&(
        s.save(),
        v.client.as_ref().context("client absent")?.save(s)?,
        v.device.as_ref().context("device absent")?.save(s)?,
    ))?)
}

#[test]
#[ignore = "private Live checkpoint and offscreen Metal adapter"]
fn headless_gauge_draw_preserves_complete_retained_state() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(render_gauge)?
        .join()
        .expect("mood gauge check panicked")
}

fn render_gauge() -> Result<()> {
    use clap::Parser;
    let repo = repo()?;
    let root = repo.join("game-data/cabinet-extracted");
    let checkpoint = repo.join(std::env::var("SIMS_MOOD_GAUGE_CHECKPOINT")?);
    let output = repo.join(std::env::var("SIMS_MOOD_GAUGE_OUTPUT")?);
    let house = std::env::var("SIMS_MOOD_GAUGE_HOUSE")
        .unwrap_or_else(|_| "UserData/Houses/House07.iff".into());
    ensure!(checkpoint.starts_with(repo.join("cache")) && output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("gauge output must be fresh")?;
    let args = Args::parse_from([
        "sims-room",
        "--original",
        &house,
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        checkpoint.to_str().unwrap(),
        "--save",
        output.join("unused.json").to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut viewer, _) = Viewer::new(args);
    viewer.install(ready)?;
    let tick = viewer.session.as_ref().unwrap().simulation.vm.tick;
    let live_mood = live_gauge_mood(viewer.session.as_ref().unwrap())?;
    let before = retained(&viewer)?;
    for size in [[800, 600], [1000, 740], [1320, 900]] {
        super::super::super::support::render_headless_element_at_size(
            &root,
            &output.join(format!("live-{}x{}.png", size[0], size[1])),
            viewer.view(),
            size,
        )?;
        ensure!(
            before == retained(&viewer)?,
            "HUD drawing changed retained state"
        );
    }
    let icons = viewer.icons.as_ref().context("icons absent")?;
    let index = viewer.session.as_ref().unwrap().index();
    for mood in [-100., -80., -60., -40., -20., 0., 10., 30., 50., 70., 90.] {
        let segments = mood_segments(mood).unwrap();
        let (_, _, base) = icons.hud(4911).unwrap();
        let component = widget::stack![widget::image(base[0].clone()).width(108).height(100)]
            .push(mood_gauge_layer(icons, mood).unwrap())
            .width(108)
            .height(100);
        let path = output.join(format!("gauge-{segments:+}.png"));
        super::super::super::support::render_headless_element_at_size(
            &root,
            &path,
            component.into(),
            [108, 100],
        )?;
        let frame = image::open(path)?.into_rgba8();
        for strip in mood_gauge_strips(mood).unwrap() {
            let art = original_art(index, strip.resource)?;
            for y in strip.source[1]..strip.source[3] {
                for x in 0..27 {
                    let pixel = art.get_pixel(x, y);
                    if pixel[3] == 0 {
                        continue;
                    }
                    let actual = frame.get_pixel(
                        strip.position[0] as u32 + x,
                        strip.position[1] as u32 + y - strip.source[1],
                    );
                    ensure!(
                        (0..4).all(|c| pixel[c].abs_diff(actual[c]) <= 1),
                        "gauge{segments} crop differs"
                    );
                }
            }
        }
        ensure!(
            before == retained(&viewer)?,
            "gauge component changed retained state"
        );
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "checkpoint":checkpoint,"tick":tick,"live_mood":live_mood,"component_levels":11,"frames":14,
            "complete_retained_state_unchanged":true,"component_pixels_match_original_artwork":true,
            "original_parity_ticks_added":0,"continuation_ticks_added":0
        }))?,
    )?;
    Ok(())
}
