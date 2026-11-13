//! Private-data differential of the shared driver and independent cold cuts.
//! This is not native Session/source admission.
use super::*;
use serde_json::{Value, json};
use sims_fixtures::{BlobRef, Store};
use std::{collections::BTreeSet, io::Write, path::Path, time::Instant};

fn cut(store: &Store, report: &Value, boundary: usize) -> Result<(BlobRef, Retained)> {
    let reference: BlobRef = serde_json::from_value(if boundary == 0 {
        report["baseline_retained_cut"].clone()
    } else {
        report["boundary_sequence"][boundary - 1]["retained_cut"].clone()
    })?;
    let cut: Retained = store.get_value(&reference)?;
    ensure!(
        cut.simulation.state.vm.tick == 89073 + boundary as u64,
        "cold boundary alignment differs"
    );
    Ok((reference, cut))
}

fn publish(
    runtime: &mut Runtime<'_>,
    program: &Program,
    report: &Value,
    boundary: usize,
) -> Result<()> {
    let events = report["character_cpu_publications"]["events"]
        .as_array()
        .context("publication inputs absent")?;
    let mut actors = Vec::new();
    let mut control = None;
    for e in events.iter().filter(|e| e["before_boundary"] == boundary) {
        ensure!(
            e["after_tick"] == runtime.tick() && e["notifications_acknowledged"] == true,
            "explicit publication input differs"
        );
        let c = e["control"]
            .as_u64()
            .context("publication precision absent")? as u16;
        ensure!(
            control.is_none_or(|old| old == c),
            "publication precision changed within phase"
        );
        control = Some(c);
        actors.push(
            e["actor"]
                .as_i64()
                .context("publication actor absent")?
                .try_into()?,
        );
    }
    if let Some(control) = control {
        runtime.publish_cpu(program, &actors, control)?;
    }
    Ok(())
}

fn reference_trace(report: &Value, boundary: usize) -> Result<Vec<Trace>> {
    let all: Vec<Trace> = serde_json::from_value(report["ordered_trace"].clone())?;
    Ok(all
        .into_iter()
        .filter(|t| t.tick == 89072 + boundary as u64)
        .collect())
}

fn same_cut(
    runtime: &Runtime<'_>,
    program: &Program,
    output: &Store,
    expected: &BlobRef,
) -> Result<BlobRef> {
    let actual = output.put_value(&runtime.snapshot(program))?;
    ensure!(
        &actual == expected,
        "retained cold owner difference at {}: expected {}, actual {}",
        runtime.tick(),
        expected.sha256,
        actual.sha256
    );
    Ok(actual)
}

pub(crate) fn run(
    index: &Index,
    program: &Program,
    report_path: &Path,
    destination: &Path,
) -> Result<()> {
    ensure!(
        !destination.exists() && !destination.with_extension("store").exists(),
        "cold output already exists"
    );
    let encoded = std::fs::read(report_path)?;
    let report_sha = sims_fixtures::digest(&encoded);
    let report: Value = serde_json::from_slice(&encoded)?;
    ensure!(
        report["requested_boundaries"] == 32
            && report["boundary_sequence"]
                .as_array()
                .is_some_and(|rows| rows.len() == 32)
            && report["faults"] == json!([]),
        "full retained reference absent"
    );
    for (i, row) in report["boundary_sequence"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        ensure!(
            row["boundary"] == i + 1
                && row["fields"]["differences"] == 0
                && row["dispatches"].get("error").is_none(),
            "retained reference has a failed boundary"
        );
    }
    let source = Store::open(report_path.with_extension("store"))?;
    let output = Store::create(destination.with_extension("store"))?;
    let services = Services {
        materials: graphic_materials::Backend {
            mode: 0,
            flags: 0,
            references: 1,
            ramp_size: 32,
        },
        normal_material_mode: 0,
    };
    let started = Instant::now();
    let (baseline, initial) = cut(&source, &report, 0)?;
    let mut runtime = Runtime::restore(index, program, initial)?;
    same_cut(&runtime, program, &output, &baseline)?;
    let mut boundaries = Vec::new();
    let mut suspended = Vec::new();
    for boundary in 1..=32 {
        publish(&mut runtime, program, &report, boundary)?;
        let mut selected = BTreeSet::new();
        let trace = runtime.finish_update(program, services, |r, prefix| {
            if [1, 9, 19, 32].contains(&boundary) && selected.len() < 2 {
                let descriptor = format!("{:?}", r.pending_services());
                if !r.pending_services().is_empty() && selected.insert(descriptor.clone()) {
                    let reference = output.put_value(&r.snapshot(program))?;
                    suspended.push((boundary, prefix, reference, descriptor));
                }
            }
            Ok(())
        })?;
        ensure!(
            trace == reference_trace(&report, boundary)?,
            "shared driver changed ordered trace at {boundary}"
        );
        let (expected, _) = cut(&source, &report, boundary)?;
        let actual = same_cut(&runtime, program, &output, &expected)?;
        boundaries.push(json!({"boundary":boundary,"tick":runtime.tick(),"state":actual,"instructions":trace.len(),"matched":true}));
        eprintln!("shared retained driver boundary {boundary}: exact owned state and trace match");
    }
    let replay_ms = started.elapsed().as_secs_f64() * 1000.;
    drop(runtime);
    let cold_start = Instant::now();
    let mut resumed = Vec::new();
    for boundary in [0, 8, 18, 31] {
        let (reference, initial) = cut(&source, &report, boundary)?;
        let mut runtime = Runtime::restore(index, program, initial)?;
        same_cut(&runtime, program, &output, &reference)?;
        let mut suffix = Vec::new();
        for next in boundary + 1..=(boundary + 2).min(32) {
            publish(&mut runtime, program, &report, next)?;
            let trace = runtime.finish_update(program, services, |_, _| Ok(()))?;
            ensure!(
                trace == reference_trace(&report, next)?,
                "cold completed-cut trace differs"
            );
            let (expected, _) = cut(&source, &report, next)?;
            suffix.push(same_cut(&runtime, program, &output, &expected)?);
        }
        resumed.push(
            json!({"boundary":boundary,"checkpoint":reference,"suffix":suffix,"matched":true}),
        );
    }
    let mut service_resumes = Vec::new();
    for (boundary, prefix, reference, descriptor) in suspended {
        let initial: Retained = output.get_value(&reference)?;
        let mut runtime = Runtime::restore(index, program, initial)?;
        same_cut(&runtime, program, &output, &reference)?;
        let trace = runtime.finish_update(program, services, |_, _| Ok(()))?;
        let full = reference_trace(&report, boundary)?;
        ensure!(
            trace == full[prefix..],
            "cold suspended trace suffix differs at {boundary}/{prefix}"
        );
        let (expected, _) = cut(&source, &report, boundary)?;
        same_cut(&runtime, program, &output, &expected)?;
        service_resumes.push(
            json!({"boundary":boundary,"trace_prefix":prefix,"checkpoint":reference,
            "requests":descriptor,"suffix_instructions":trace.len(),"matched":true}),
        );
    }
    let result = json!({"schema":1,"kind":"retained_component_cold_reexecution","native_parity_credit":0,
        "reference_report_sha256":report_sha,"program_fingerprint":program.fingerprint(),"baseline":baseline,
        "boundaries":boundaries,"completed_cold_resumes":resumed,"suspended_cold_resumes":service_resumes,
        "replay_ms":replay_ms,"cold_resume_ms":cold_start.elapsed().as_secs_f64()*1000.,
        "qualification":"Exact retained component/trace differential against the current original-matched producer. Independent runtime and immutable asset restoration; explicit witnessed character publications. Material inputs retain the earlier unqualified backend profile. No enclosing clock/application caller or complete Session/native parity admission."});
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    file.write_all(&serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}

/// Rebuild the real pre-view C8 prefix from B, without inserting a view update
/// between native lot returns. Original scalar/stack and instruction evidence
/// remain comparisons; no expected endpoint state is installed.
pub(crate) fn scene_prefix(index:&Index,program:&Program,destination:&Path)->Result<()> {
    let root=Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let destination=if destination.is_absolute(){destination.to_path_buf()}else{root.join(destination)};
    ensure!(!destination.exists()&&!destination.with_extension("store").exists(),"scene prefix output exists");
    let old_bytes=std::fs::read(root.join("fixtures/private/goth-retained-cold-20261031-first/reexecution-first.json"))?;
    ensure!(sims_fixtures::digest(&old_bytes)=="80a9d9ade1d61a513cffd06e444428d376514fa533ddb096905c32ffee3c3849","scene baseline receipt differs");
    let old:Value=serde_json::from_slice(&old_bytes)?;ensure!(old["program_fingerprint"]==program.fingerprint(),"scene prefix Program differs");
    let source=Store::open(root.join("fixtures/private/goth-retained-cold-20261031-first/reexecution-first.store"))?;
    let baseline:BlobRef=serde_json::from_value(old["baseline"].clone())?;let cut:Retained=source.get_value(&baseline)?;
    let encoded=std::fs::read(root.join("fixtures/private/goth-draw-replay-20261031-third/sequence-projection-first.json"))?;
    ensure!(sims_fixtures::digest(&encoded)==old["reference_report_sha256"].as_str().unwrap(),"scene trace source differs");let report:Value=serde_json::from_slice(&encoded)?;
    let store=Store::create(destination.with_extension("store"))?;let mut runtime=Runtime::restore(index,program,cut)?;
    ensure!(store.put_value(&runtime.snapshot(program))?==baseline,"scene baseline cold restore differs");
    let services=Services{materials:sims_content::graphic_materials::Backend{mode:0,flags:0,references:1,ramp_size:32},normal_material_mode:0};
    let mut boundaries=vec![];
    for boundary in 1..=8 {
        publish(&mut runtime,program,&report,boundary)?;
        let trace=runtime.finish_update(program,services,|_,_|Ok(()))?;
        ensure!(trace==reference_trace(&report,boundary)?,"scene prefix native instruction/yield order differs at {boundary}");
        let native_root=root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence");
        let encoded=std::fs::read(native_root.join(format!("completed-{boundary:02}/observation.json.gz")))?;
        let native:Value=serde_json::from_reader(flate2::read::GzDecoder::new(encoded.as_slice()))?;
        let fields=crate::original_session::completed_fields::compare(&native,&runtime.simulation)?;
        ensure!(fields["differences"]==0,"scene prefix scalar/stack difference at {boundary}: {}",fields["first_differences"]);
        let state=store.put_value(&runtime.snapshot(program))?;
        boundaries.push(json!({"boundary":boundary,"tick":runtime.tick(),"state":state,"instructions":trace.len(),"native_observation_sha256":sims_fixtures::digest(&encoded),"observed_fields":fields}));
    }
    let result=json!({"schema":1,"kind":"explicit_pre_view_retained_prefix","native_parity_credit":0,"baseline":baseline,"program_fingerprint":program.fingerprint(),"boundaries":boundaries,"environment_updates":0,"qualification":"Original-matched scalar/stack and instruction/yield prefix only. The next4a8895 caller owns the environment rebuild. No renderer cadence is inserted at individual lot returns; complete source/Session and32-tick parity remain unqualified."});
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(&destination)?;file.write_all(&serde_json::to_vec_pretty(&result)?)?;file.write_all(b"\n")?;Ok(())
}
