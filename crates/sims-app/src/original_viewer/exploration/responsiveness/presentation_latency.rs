//! Bounded production-frontend benchmark on a frozen private household.
//! No window FPS claim, wall-clock threshold, native guest or checkpoint copies.
use super::super::super::ui_publication::{Published, view::Frontend};
use super::*;

const SAMPLES: usize = 17;
const VIEW_BATCH: usize = 128;

#[test]
#[ignore = "private original UIText STR#137; paired full parse versus validation-only timings"]
fn retained_tool_text_validation_latency() -> Result<()> {
    use sims_formats::{iff::Iff, strings::StringTable};
    let out = output()?;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let index =
        Index::scan(&repo.join("cache/managed-independent-20261022-third/immutable-assets"))?;
    let resource = &index.resources[index.find("UIText.iff")?];
    let iff = Iff::parse(resource.read()?)?;
    let data = iff.data(
        iff.get(b"STR#", 137)
            .context("Move UIText STR#137 absent")?,
    );
    let parsed = StringTable::parse(data)?;
    StringTable::validate(data)?;
    let mut parse = vec![];
    let mut validate = vec![];
    for sample in 0..SAMPLES {
        for retain in if sample % 2 == 0 {
            [true, false]
        } else {
            [false, true]
        } {
            let start = Instant::now();
            for _ in 0..VIEW_BATCH {
                if retain {
                    std::hint::black_box(StringTable::parse(std::hint::black_box(data))?);
                } else {
                    std::hint::black_box(StringTable::validate(std::hint::black_box(data))?);
                }
            }
            let us = start.elapsed().as_secs_f64() * 1e6 / VIEW_BATCH as f64;
            if retain {
                parse.push(us)
            } else {
                validate.push(us)
            }
        }
    }
    let result = json!({"passed":true,"source":resource.locator,"string_set":137,
        "bytes":data.len(),"version":parsed.version,"entries":parsed.entries.len(),
        "full_parse_and_drop":distribution(parse),"validation_only":distribution(validate),
        "profile":if cfg!(debug_assertions){"debug"}else{"release"},
        "scope":"Paired same-process original retained Move text. Every validation re-reads all structural fields; no cached result or omitted error check. Full parsing decodes and retains entries as the previous validation caller did."});
    eprintln!("TOOL_TEXT_VALIDATION {result}");
    write_json(&out.join("result.json"), &result)
}

fn frontend(v: &Viewer) -> Frontend {
    Frontend {
        data: Arc::new(Published::capture(v)),
        diagnostics: v.diagnostics,
        options_open: v.options_open,
        widget_epoch: v.widget_epoch,
    }
}

pub(super) fn distribution(mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    json!({"samples":samples.len(),"median_us":samples[samples.len()/2],
        "p95_us":samples[(samples.len()*95).div_ceil(100).saturating_sub(1)],
        "worst_us":samples.last().unwrap()})
}

pub(super) fn frozen_load(out: &Path) -> Result<Application> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let source = repo.join(
        std::env::var("SIMS_RESPONSIVENESS_SOURCE").unwrap_or_else(|_| {
            "cache/gameplay-gaps-reproducers-20261023/phone/initial.json".into()
        }),
    );
    let root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    let save = out.join("unused-player-save.json");
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
    let ready = Ready::load(&args)?;
    let bytes = std::fs::read(&source)?;
    write_json(
        &out.join("input-reference.json"),
        &json!({"path":source,"bytes":bytes.len()}),
    )?;
    let mut app = Application::restore_checkpoint(
        &args,
        ready
            .restore_index
            .as_ref()
            .context("restore index absent")?,
        &ready.icons,
        Instant::now(),
        &bytes,
    )?;
    app.execute(&Command::Playing { value: false }, &Limits::default())?;
    app.drain(&Limits::default())?;
    Ok(app)
}

fn update(app: &mut Application, message: Message, rows: &mut Vec<Value>) -> Result<()> {
    let label = format!("{message:?}");
    app.viewer.frame_profile = Some(FrameProfile::default());
    let started = Instant::now();
    let task = app.viewer.update(message);
    let us = started.elapsed().as_secs_f64() * 1e6;
    ensure!(
        task.units() == 0,
        "benchmark refuses unhandled application tasks"
    );
    let stages = app.viewer.frame_profile.take().context("profile absent")?;
    app.check_errors()?;
    rows.push(json!({"message":label,"us":us,"stages":stages}));
    Ok(())
}

fn transition(app: &mut Application, message: Message, operations: &mut Vec<Value>) -> Result<()> {
    let before = app.observe()?;
    let mut rows = vec![];
    update(app, message, &mut rows)?;
    let now = app.epoch + Duration::from_millis(app.millis);
    for _ in 0..Limits::default().service_polls {
        if !app.viewer.presentation_pending() {
            break;
        }
        update(app, Message::Frame(now), &mut rows)?;
    }
    ensure!(
        !app.viewer.presentation_pending(),
        "pending presentation service budget exceeded"
    );
    let after = app.observe()?;
    ensure!(
        before.tick == after.tick && before.rng == after.rng,
        "paused presentation command advanced simulation tick/RNG"
    );
    operations.push(json!({"updates":rows,"state":after}));
    Ok(())
}

pub(super) fn measure(app: &Application, out: &Path, label: &str, rows: &mut Vec<Value>) -> Result<()> {
    let before = app.observe()?;
    let state = frontend(&app.viewer);
    let size = app.viewer.display_size;
    let mut legacy = Input::new()?;
    let mut production = Input::new()?;
    let icons = app.viewer.icons.as_ref().context("icons absent")?;
    let started = Instant::now();
    let expected = icons.with_view_caches_held(|| legacy.batch_pixels(&app.viewer))?;
    let reference_cold_us = started.elapsed().as_secs_f64() * 1e6;
    let started = Instant::now();
    let actual = icons
        .with_view_caches_held(|| production.element_pixels(state.view(), size, 1.))?
        .0;
    let production_cold_us = started.elapsed().as_secs_f64() * 1e6;
    ensure!(
        actual == expected,
        "{label}: production frontend changed rendered pixels"
    );
    let eager = production
        .element_pixels(state.eager_view_for_test(), size, 1.)?
        .0;
    ensure!(
        actual == eager,
        "{label}: lazy construction differs from same-binary eager control"
    );

    // Warm renderer/text/image resources and its persistent real Iced cache.
    for _ in 0..3 {
        let _ = production.element_pixels(state.view(), size, 1.)?;
    }
    let mut samples = vec![];
    let mut views = vec![];
    let mut eager_views = vec![];
    let mut constructors: std::collections::BTreeMap<&str, Vec<f64>> = Default::default();
    for sample in 0..SAMPLES {
        // Alternate order so the paired constructor control shares this
        // exact build, backend, process and immutable publication.
        for eager in if sample % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let started = Instant::now();
            for _ in 0..VIEW_BATCH {
                std::hint::black_box(if eager {
                    state.eager_view_for_test()
                } else {
                    state.view()
                });
            }
            let us = started.elapsed().as_secs_f64() * 1e6 / VIEW_BATCH as f64;
            if eager {
                eager_views.push(us)
            } else {
                views.push(us)
            }
        }
        for (name, us) in state.construction_costs_for_test(VIEW_BATCH, sample % 2 != 0) {
            constructors.entry(name).or_default().push(us);
        }
        let started = Instant::now();
        let element = state.view();
        let view_us = started.elapsed().as_secs_f64() * 1e6;
        let (pixels, costs) = production.element_pixels(element, size, 1.)?;
        ensure!(
            pixels == expected,
            "{label}: repeated render changed pixels"
        );
        samples.push(json!({"view_us":view_us,"layout_us":costs.build_layout_us,
            "widget_update_us":costs.widget_update_us,"draw_us":costs.draw_us,
            "gpu_submission_and_readback_us":costs.gpu_readback_us}));
    }
    let stages: serde_json::Map<String, Value> = [
        "view_us",
        "layout_us",
        "widget_update_us",
        "draw_us",
        "gpu_submission_and_readback_us",
    ]
    .into_iter()
    .map(|key| {
        (
            key.into(),
            distribution(samples.iter().map(|s| s[key].as_f64().unwrap()).collect()),
        )
    })
    .collect();

    // Compare actual pointer batches against the unchanged inline frontend.
    let points = legacy.batch_centers(&app.viewer);
    let mut batches = 0;
    for point in points {
        legacy.reset();
        production.reset();
        for event in [
            iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                position: iced::Point::new(point[0], point[1]),
            }),
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                iced::mouse::Button::Left,
            )),
        ] {
            let events = std::slice::from_ref(&event);
            let (old, new) = icons.with_view_caches_held(|| {
                (
                    legacy.application_batch(&app.viewer, events),
                    production.element_batch(state.view(), size, events),
                )
            });
            ensure!(
                format!("{:?}", old.messages) == format!("{:?}", new.messages),
                "{label}: pointer message mismatch at {point:?}"
            );
            ensure!(
                old.statuses == new.statuses && old.owners == new.owners,
                "{label}: pointer ownership mismatch at {point:?}"
            );
            batches += 1;
        }
    }
    ensure!(
        before == app.observe()?,
        "{label}: rendering/input discovery changed complete state/RNG"
    );
    image::save_buffer(
        out.join(format!("{label}.png")),
        &actual,
        size[0],
        size[1],
        image::ColorType::Rgba8,
    )?;
    let constructors: serde_json::Map<String, Value> = constructors
        .into_iter()
        .map(|(name, samples)| (name.into(), distribution(samples)))
        .collect();
    let row = json!({"label":label,"size":size,"reference_cold_us":reference_cold_us,
        "production_cold_us":production_cold_us,"view_construct_and_drop":distribution(views),
        "eager_control_construct_and_drop":distribution(eager_views),
        "stages":stages,"constructors":constructors,"timing_samples":samples,"complete_state_equal":true,
        "exact_rgba_equal":true,"pointer_batches_equal":batches,"state":before});
    eprintln!(
        "PRESENTATION_LATENCY {label}: view={} us, eager={} us, GPU submission/readback={} us",
        row["view_construct_and_drop"]["median_us"],
        row["eager_control_construct_and_drop"]["median_us"],
        row["stages"]["gpu_submission_and_readback_us"]["median_us"]
    );
    rows.push(row);
    write_json(&out.join("measurements.json"), rows)
}

#[test]
#[ignore = "private Mashuga checkpoint; bounded production frontend profiling, exact pixels/input and state invariants"]
fn mashuga_production_presentation_latency_and_equivalence() -> Result<()> {
    run_worker(campaign)
}

fn campaign() -> Result<()> {
    let out = output()?;
    let started = Instant::now();
    let mut app = frozen_load(&out)?;
    select(&mut app, 196)?;
    let setup_us = started.elapsed().as_secs_f64() * 1e6;
    let mut rows = vec![];
    let mut operations = vec![];
    measure(&app, &out, "live", &mut rows)?;
    transition(&mut app, Message::Needs, &mut operations)?;
    measure(&app, &out, "needs", &mut rows)?;
    transition(&mut app, Message::Options, &mut operations)?;
    measure(&app, &out, "options", &mut rows)?;
    transition(&mut app, Message::Options, &mut operations)?;
    transition(&mut app, Message::Buy, &mut operations)?;
    measure(&app, &out, "buy-root", &mut rows)?;
    let catalog = app
        .viewer
        .device
        .as_ref()
        .and_then(|d| d.panel_widgets())
        .and_then(|w| w.catalog())
        .context("catalog absent")?;
    let index = catalog
        .view()
        .context("catalog view absent")?
        .categories
        .iter()
        .position(|c| c.visible && c.enabled)
        .context("enabled category absent")?;
    let revision = catalog.revision();
    transition(
        &mut app,
        Message::BuyNavigate(
            revision,
            sims_vm::interface::buy::input::Event::Category(index as u8),
        ),
        &mut operations,
    )?;
    measure(&app, &out, "buy-products", &mut rows)?;
    transition(&mut app, Message::Live, &mut operations)?;
    measure(&app, &out, "live-return", &mut rows)?;
    app.execute(
        &Command::Resize {
            width: 800,
            height: 600,
        },
        &Limits::default(),
    )?;
    measure(&app, &out, "live-800x600", &mut rows)?;
    transition(&mut app, Message::Diagnostics, &mut operations)?;
    measure(&app, &out, "diagnostics", &mut rows)?;
    let state = app.observe()?;
    let started = Instant::now();
    let cold = app.restore(&app.checkpoint()?)?;
    let checkpoint_and_restore_us = started.elapsed().as_secs_f64() * 1e6;
    ensure!(
        state == cold.observe()?,
        "cold save/resume changed complete state/RNG"
    );
    write_json(
        &out.join("result.json"),
        &json!({"passed":true,"setup_us":setup_us,
        "checkpoint_and_restore_us":checkpoint_and_restore_us,"operations":operations,"cases":rows,
        "samples_per_case":SAMPLES,"view_batch":VIEW_BATCH,"profile":if cfg!(debug_assertions){"debug"}else{"release"},
        "os":std::env::consts::OS,"arch":std::env::consts::ARCH,
        "scope":"Frozen independent Mashuga; paused original presentation services, pure production Frontend and pinned Iced offscreen GPU submission/readback. No compositor/window FPS or additional native gameplay parity claim. Hashing, validation, IO, cold restore, and widget discovery are outside timing spans; no CI wall-clock thresholds."}),
    )
}
