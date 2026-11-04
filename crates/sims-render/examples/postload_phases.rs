//! Diagnostic checkpoints using the production resumable post-load owners.
//! No original values are injected and no simulation ticks are advanced.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Options, Session, completion};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};
#[path = "support/opening.rs"]
mod opening;

fn state(s: &Session) -> Value {
    let vm = &s.simulation.vm;
    json!({
        "tick":vm.tick,"global_rng":vm.rng_state,"lot_rng":vm.lot_rng_state,
        "crt_rng":vm.crt_rng_state,"globals":vm.world.globals,
        "actions":vm.world.actions,"positions":vm.world.positions,
        "budget":vm.world.budget,
        "objects":vm.world.objects.iter().map(|(&id,o)| {
            let thread=vm.object_threads.get(&id).map(|id| &vm.threads[id]);
            json!({"id":id,"guid":o.definition.as_ref().map(|d|d.guid),
                "data":o.data,"temps":o.temps,"attributes":o.attributes,
                "native_flags":o.native_flags,"registry_word":o.idle.as_ref().map(|idle| idle.registry_word()),
                "person":o.person,"motives":o.motives,"motive_changes":o.motive_changes,
                "motive_update":o.motive_update,
                "thread":thread})
        }).collect::<Vec<_>>()
    })
}
fn write(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let mut file = BufWriter::new(File::create_new(path)?);
    serde_json::to_writer(&mut file, value)?;
    file.flush()?;
    Ok(())
}
fn main() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("parity trace panicked"))?
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        matches!(args.len(), 4 | 5 | 6 | 7),
        "usage: postload_phases CONTENT_ROOT SOURCE NEW_OUTPUT_DIR CRT_SEED [RECORDED_INPUTS_JSON] [PROCESS_SKELETON_CACHE_JSON] [RETAINED_STARTUP_REPORT_JSON]"
    );
    let out = Path::new(&args[2]);
    ensure!(
        out.parent()
            .context("output parent")?
            .canonicalize()?
            .starts_with(Path::new("cache").canonicalize()?),
        "private output required"
    );
    std::fs::create_dir(out).context("refusing to replace evidence")?;
    let index = Index::scan(Path::new(&args[0]))?;
    let seed = args[3].parse()?;
    let application_context = args
        .get(4)
        .map(|path| -> Result<_> {
            let data: Value = serde_json::from_reader(File::open(path)?)?;
            data.get("application_context")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(Into::into)
        })
        .transpose()?
        .flatten();
    let resources = args
        .get(5)
        .map(|path| -> Result<_> { Ok(serde_json::from_reader(File::open(path)?)?) })
        .transpose()?
        .unwrap_or_default();
    let incoming = args
        .get(6)
        .map(|path| -> Result<_> {
            let report: Value = serde_json::from_reader(File::open(path)?)?;
            Ok(serde_json::from_value(report["incoming"].clone())?)
        })
        .transpose()?;
    let mut session = Session::load_with_startup(
        &index,
        &args[1],
        Options {
            application_context,

            native_postload: true,
            native_load_completion: true,
            crt_seed: seed,
            ..Default::default()
        },
        true,
        resources,
        incoming,
    )?;
    let house = sims_content::saves::House::load(&index, &args[1])?;
    write(
        &out.join("compatibility.json"),
        &house.compatibility(&session.program)?,
    )?;
    write(&out.join("loaded-state.json"), &state(&session))?;

    let mut device = Device::attach_for_load(&mut session, 800, 600, 0)?;
    for _ in 0..30000 {
        let input = session.retained_camera_input()?;
        if session.advance_postload(input, 256)? {
            break;
        }
    }
    ensure!(!session.postload_pending(), "postload budget");
    write(&out.join("postload-state.json"), &state(&session))?;
    session.begin_load_completion(completion::Config {
        controller_suppressed: 0,
        mode: 0,
    })?;
    for _ in 0..30000 {
        if device.advance_load_completion(&mut session, 256)? {
            break;
        }
    }
    ensure!(!session.load_completion_pending(), "completion budget");
    write(&out.join("completion-state.json"), &state(&session))?;
    let mut client = opening::bind(&mut session, &mut device)?;
    write(&out.join("application-state.json"), &state(&session))?;
    write(&out.join("application-session.json"), &session.save())?;
    session.begin_live_interface_update()?;
    for _ in 0..30000 {
        if device.advance_interface(&mut session, 256)? {
            break;
        }
    }
    ensure!(!session.interface_update_pending(), "interface budget");
    write(&out.join("interface-state.json"), &state(&session))?;
    session.bind_control_panel()?;
    device.bind_transport(&mut session)?;
    write(&out.join("bound-state.json"), &state(&session))?;
    write(&out.join("bound-session.json"), &session.save())?;
    client.bind_live_panel(&session)?;
    write(
        &out.join("application-client.json"),
        &client.save(&session)?,
    )?;
    //4d1901 calls4764fe before46d9d6. The interactive viewer already owns
    //this poll; expose it separately here so load and frame work are not
    //conflated. This headless client has no focused control. Any requested
    //widget observation must be supplied by its actual owner, not fabricated.
    let tick = session.simulation.vm.tick;
    opening::poll(&mut session, &client)?;
    ensure!(!session.ui_wait_pending(), "application poll budget");
    ensure!(
        session.simulation.vm.tick == tick,
        "UI poll advanced lot time"
    );
    write(&out.join("ui-poll-state.json"), &state(&session))?;
    write(&out.join("ui-poll-session.json"), &session.save())?;
    session.update_view_environment()?;
    write(&out.join("frame-state.json"), &state(&session))?;
    write(&out.join("frame-session.json"), &session.save())?;
    device.refresh(&session)?;
    software::render(device.scene()).save(out.join("bound.png"))?;
    println!("postload, completion, interface, binding and application poll states captured");
    Ok(())
}
