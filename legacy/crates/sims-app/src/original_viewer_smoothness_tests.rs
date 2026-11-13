use super::*;
use clap::Parser;
use serde_json::json;

fn viewport_bounds(v: &Viewer, renderer: &mut iced::Renderer) -> Result<[f32; 4]> {
    use iced::advanced::{layout, widget::Tree};
    let shader: Element<'_, Message> =
        widget::shader::Shader::new(Viewport(v.scene.as_ref().unwrap().clone())).into();
    let tag = shader.as_widget().tag();
    let mut page = v.view();
    let mut tree = Tree::new(page.as_widget());
    let node = page.as_widget_mut().layout(
        &mut tree,
        renderer,
        &layout::Limits::new(
            Size::ZERO,
            Size::new(v.display_size[0] as f32, v.display_size[1] as f32),
        ),
    );
    fn find(
        t: &Tree,
        n: &layout::Node,
        tag: iced::advanced::widget::tree::Tag,
        offset: iced::Vector,
    ) -> Option<iced::Rectangle> {
        let b = n.bounds();
        let position = offset + iced::Vector::new(b.x, b.y);
        if t.tag == tag {
            return Some(iced::Rectangle {
                x: position.x,
                y: position.y,
                ..b
            });
        }
        t.children
            .iter()
            .zip(n.children())
            .find_map(|(t, n)| find(t, n, tag, position))
    }
    find(&tree, &node, tag, iced::Vector::ZERO)
        .map(|b| [b.x, b.y, b.width, b.height])
        .context("viewport layout node absent")
}
fn retained(v: &Viewer) -> Result<Vec<u8>> {
    let s = v.session.as_ref().unwrap();
    Ok(serde_json::to_vec(
        &json!({"session":s.save(),"client":v.client.as_ref().unwrap().save(s)?,
        "device":v.device.as_ref().unwrap().save(s)?}),
    )?)
}
fn timed_drain(v: &mut Viewer) -> Result<serde_json::Value> {
    let now = v.last.unwrap_or_else(Instant::now);
    let mut frames = vec![];
    for _ in 0..10000 {
        ensure!(v.error.is_none(), "{:?}", v.error);
        if !v.presentation_pending() {
            let start = Instant::now();
            v.refresh()?;
            return Ok(
                json!({"frames":frames,"final_refresh_ms":start.elapsed().as_secs_f64()*1000.}),
            );
        }
        let s = v.session.as_ref().unwrap();
        let flags = [
            s.publication_pending(),
            s.tool_selection_pending(),
            s.camera_command_pending(),
            s.house_notification_pending(),
        ];
        let start = Instant::now();
        let _ = v.update(Message::Frame(now));
        frames.push(json!({"ms":start.elapsed().as_secs_f64()*1000.,"pending":flags}));
    }
    anyhow::bail!("timed presentation did not finish")
}
#[test]
#[ignore = "private Mashuga checkpoint; explicit baseline/fixed profile and headless GPU"]
fn mashuga_popup_layout_and_buy_live_profile() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("smoothness test panicked")
}
fn run() -> Result<()> {
    use iced::advanced::renderer::Headless;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = repo.join(std::env::var("SIMS_SMOOTH_OUTPUT")?);
    ensure!(out.starts_with(repo.join("cache")));
    std::fs::create_dir(&out)?;
    let fixed = std::env::var_os("SIMS_SMOOTH_EXPECT_FIXED").is_some();
    let root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    let source = repo.join("cache/no-selected-person-iced-20261023-third/no-selected-person.json");
    let save = out.join("base.json");
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
        save.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    let mut inputs = vec![];
    transport_tests::drain(&mut v, &mut inputs)?;
    let _ = v.update(Message::Resident(v.resident));
    transport_tests::drain(&mut v, &mut inputs)?;
    v.save()?;
    let mut renderer = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
        iced::Font::DEFAULT,
        iced::Pixels(16.),
        Some("wgpu"),
    ))
    .context("headless adapter absent")?;
    let before = viewport_bounds(&v, &mut renderer)?;
    let _ = v.update(Message::Select(Some(187)));
    transport_tests::drain(&mut v, &mut inputs)?;
    let popup = viewport_bounds(&v, &mut renderer)?;
    let _ = v.update(Message::ObjectCancel);
    transport_tests::drain(&mut v, &mut inputs)?;
    let _ = v.update(Message::Select(Some(131)));
    let lamp_result = transport_tests::drain(&mut v, &mut inputs)
        .err()
        .map(|e| format!("{e:#}"));
    let lamp = viewport_bounds(&v, &mut renderer)?;
    std::fs::write(
        out.join("lamp-result.json"),
        serde_json::to_vec_pretty(
            &json!({"error":lamp_result,"app_error":v.error,"before":before,"popup":popup,"lamp":lamp}),
        )?,
    )?;
    if fixed {
        ensure!(lamp_result.is_none() && v.error.is_none());
        ensure!(
            !v.session.as_ref().unwrap().object_input_pending(),
            "empty lamp menu stayed open"
        );
        ensure!(
            v.session.as_ref().unwrap().simulation.vm.world.objects[&131]
                .native_flags
                .unwrap()
                & 8
                != 0
        );
        ensure!(
            before == popup && before == lamp,
            "popup changed world layout"
        );
    }
    let saved_error = v.error.take();
    v.error = Some("Diagnostic overlay must not resize the world".into());
    let error_overlay = viewport_bounds(&v, &mut renderer)?;
    v.error = saved_error;
    if fixed {
        ensure!(before == error_overlay, "error feedback resized world");
        v.load(&save)?;
        transport_tests::drain(&mut v, &mut inputs)?;
        v.select_object(Some(131))?;
        v.session.as_mut().unwrap().advance_object_input(1)?;
        v.session.as_mut().unwrap().advance_object_input(1)?;
        ensure!(v.session.as_ref().unwrap().object_input_pending());
        let checkpoint = out.join("pending-tile-fallback.json");
        v.args.save = checkpoint.clone();
        v.save()?;
        v.args.save = save.clone();
        timed_drain(&mut v)?;
        let expected = retained(&v)?;
        v.load(&checkpoint)?;
        timed_drain(&mut v)?;
        let actual = retained(&v)?;
        if expected != actual {
            std::fs::write(out.join("fallback-warm.json"), &expected)?;
            std::fs::write(out.join("fallback-cold.json"), &actual)?;
            anyhow::bail!("cold fallback continuation differed");
        }
    }
    v.load(&save)?;
    transport_tests::drain(&mut v, &mut inputs)?;
    let mut modes = vec![];
    for mode in [Message::Buy, Message::Live, Message::Buy, Message::Live] {
        let name = if matches!(mode, Message::Buy) {
            "Buy"
        } else {
            "Live"
        };
        let tick = v.session.as_ref().unwrap().simulation.vm.tick;
        let start = Instant::now();
        let _ = v.update(mode);
        let dispatch_ms = start.elapsed().as_secs_f64() * 1000.;
        let drain = timed_drain(&mut v)?;
        ensure!(
            v.session.as_ref().unwrap().simulation.vm.tick == tick,
            "mode input advanced gameplay tick"
        );
        modes.push(json!({"mode":name,"dispatch_ms":dispatch_ms,"drain":drain}));
    }
    let s = v.session.as_ref().unwrap();
    let camera = OriginalHouse::native_camera(s)?;
    let frozen = serde_json::to_vec(&s.simulation)?;
    let mut original = OriginalHouse::default();
    let _ = original.scene(s, camera)?;
    let reference = OriginalHouse::default().scene(s, camera)?;
    let reference_pixels = sims_render::software::render(&reference);
    let mut resources = vec![];
    for _ in 0..3 {
        let start = Instant::now();
        let scene = original.rebuild_resources(s, camera)?;
        let ms = start.elapsed().as_secs_f64() * 1000.;
        ensure!(
            sims_render::software::render(&scene) == reference_pixels,
            "reused resources changed pixels"
        );
        for x in (0..scene.width).step_by(47) {
            for y in (0..scene.height).step_by(43) {
                ensure!(
                    scene.pick([x as f32, y as f32]) == reference.pick([x as f32, y as f32]),
                    "reused resources changed picking"
                );
            }
        }
        resources
            .push(json!({"ms":ms,"vertices":scene.vertices.len(),"textures":scene.textures.len()}));
    }
    ensure!(frozen == serde_json::to_vec(&s.simulation)?);
    render_headless_element(&root, &out.join("final.png"), v.view())?;
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"fixed":fixed,"before":before,"popup":popup,"lamp":lamp,"error_overlay":error_overlay,"modes":modes,"resource_rebuilds":resources,"resource_pixels_and_picking_match_fresh":true,"fallback_cold_resume":fixed,"inputs":inputs,"strict_original_ticks_added":0}),
        )?,
    )?;
    Ok(())
}
