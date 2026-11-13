//! Original-asset regressions for the exact retained rejected-person owners.
//! No teardown replay, caller reset, manual tick advance or player-save writes.
use super::*;

fn summary(app: &Application) -> Result<Value> {
    let s = app.session()?;
    let vm = &s.simulation.vm;
    Ok(
        json!({"tick":vm.tick,"rng":[Some(vm.rng_state),vm.lot_rng_state,vm.crt_rng_state],
        "lot_rng_scope":vm.lot_rng_scope,"native_scheduler":s.simulation.native_scheduler,
        "person134":vm.world.objects.get(&134),"factory134":s.simulation.person_factories.get(&134),
        "parent387":vm.threads.get(&387),"deletion":s.simulation.deletion,
        "retired134":s.simulation.retired_characters.contains_key(&134),
        "asset134":s.host.characters.contains_key(&134),"household134":s.simulation.household.as_ref().is_some_and(|h|h.people.contains_key(&134)),
        "neighbors":vm.world.person.as_ref().map(|p| [32,34].map(|n|(n,p.neighbors.get(&n))))}),
    )
}
fn equal(warm: &Application, cold_app: &Application) -> Result<()> {
    ensure!(
        full(warm)? == full(cold_app)? && identities(warm)? == identities(cold_app)?,
        "full rejected-person diagnostic state/RNG/content owners differ"
    );
    Ok(())
}
fn recheckpoint(
    warm: &Application,
    cold_app: &mut Application,
    out: &Path,
    cut: usize,
    label: &str,
    persist: bool,
) -> Result<Vec<u8>> {
    equal(warm, cold_app)?;
    let before = full(warm)?;
    let bytes = warm.checkpoint()?;
    ensure!(
        full(warm)? == before,
        "capture mutated rejected-person state"
    );
    let restored = cold(warm, &bytes)?;
    equal(warm, &restored)?;
    *cold_app = restored;
    if persist {
        space(out, bytes.len() as u64)?;
        std::fs::write(out.join(format!("{cut:03}-{label}.json")), &bytes)?;
    }
    write_json(
        &out.join(format!("{cut:03}-{label}-proof.json")),
        &json!({"cut":cut,"label":label,
        "checkpoint_sha256":engine::digest(&bytes),"bytes":bytes.len(),"full":before,
        "safe_player_boundary":false,"actual_teardown_yield":true,"independent_cold_restore":true,"source_owners":identities(warm)?}),
    )?;
    Ok(bytes)
}
fn forged_retired_owners(app: &Application, out: &Path) -> Result<()> {
    let bytes = app.checkpoint()?;
    let mut value: Value = serde_json::from_slice(&bytes)?;
    let prefix = "/session/simulation/state/person_factories/134";
    let mut changes = vec![
        ("stale-parent", format!("{prefix}/parent"), json!(283)),
        ("wrong-parent", format!("{prefix}/parent"), json!(999999)),
        ("wrong-request", format!("{prefix}/request"), json!(290)),
        (
            "wrong-target",
            format!("{prefix}/rejection_cleanup/target"),
            json!(104),
        ),
        (
            "missing-committed-resource",
            format!("{prefix}/rejected_definition"),
            Value::Null,
        ),
        (
            "wrong-committed-guid",
            format!("{prefix}/rejected_definition/guid"),
            json!(1),
        ),
        (
            "wrong-committed-source",
            format!("{prefix}/rejected_definition/source"),
            json!("missing-resource.iff"),
        ),
        (
            "wrong-committed-object",
            format!("{prefix}/rejected_definition/object"),
            json!(0),
        ),
        (
            "wrong-committed-kind",
            format!("{prefix}/rejected_definition"),
            json!({"source":"GameData_ALL_Recursive/Objects/Objects.far::Cursors.iff","object":128,"guid":1,"slots":0}),
        ),
        (
            "wrong-destructor-resource",
            "/session/simulation/state/deletion/0/entry_definition/guid".into(),
            json!(1),
        ),
        (
            "wrong-retired-character",
            "/session/simulation/state/retired_characters/134/resident/context/actor".into(),
            json!(104),
        ),
        (
            "missing-cleanup",
            format!("{prefix}/rejection_cleanup"),
            Value::Null,
        ),
        (
            "wrong-phase",
            format!("{prefix}/rejection_cleanup/deleting"),
            json!(false),
        ),
        (
            "missing-deletion",
            "/session/simulation/state/deletion".into(),
            json!([]),
        ),
        (
            "missing-retired-asset",
            "/session/simulation/state/retired_characters".into(),
            json!({}),
        ),
    ];
    let mut duplicated = value
        .pointer("/session/simulation/state/person_factories")
        .context("factory owner table absent")?
        .clone();
    let duplicate = duplicated["134"].clone();
    duplicated
        .as_object_mut()
        .unwrap()
        .insert("135".into(), duplicate);
    changes.push((
        "duplicate-retired-factory",
        "/session/simulation/state/person_factories".into(),
        duplicated,
    ));
    let mut rows = vec![];
    for (name, path, replacement) in changes {
        let place = value.pointer_mut(&path).context("forgery path absent")?;
        let previous = std::mem::replace(place, replacement.clone());
        let forged = serde_json::to_vec(&value)?;
        let result = cold(app, &forged);
        let error = result
            .err()
            .with_context(|| format!("forged{name} owner was accepted"))?;
        *value
            .pointer_mut(&path)
            .context("forgery restoration path absent")? = previous;
        rows.push(
            json!({"name":name,"path":path,"replacement":replacement,"error":format!("{error:#}")}),
        );
    }
    write_json(
        &out.join("retired-owner-negatives.json"),
        &json!({"checkpoint_sha256":engine::digest(&bytes),"cases":rows}),
    )
}
#[test]
#[ignore = "exact completed rejected-cleanup state; actual retained Clock UpdateLot continuation"]
fn rejected_cleanup_returns_through_actual_retained_clock() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run_actual_clock)?
        .join()
        .expect("retained clock proof panicked")
}
fn run_actual_clock() -> Result<()> {
    let repo = PathBuf::from(std::env::var("SIMS_NATIVE_CONTINUATION_REPO")?).canonicalize()?;
    let out = repo.join(std::env::var("SIMS_NATIVE_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(repo.join("cache")),
        "private output required"
    );
    std::fs::create_dir(&out)?;
    space(&out, 0)?;
    let source=repo.join("cache/native-owned-continuation-20261024-first/person-rejection-private/actual-yields-tail-second/028-vm-yield.json");
    let bytes = std::fs::read(&source)?;
    let source_sha = engine::digest(&bytes);
    ensure!(
        source_sha == "756ffbd8871816decb9e37aab3c9d81f9dedb0f714afd9dc2d70f50352b0596b",
        "not exact completed person134 cleanup payload"
    );
    let mut warm = restore(&repo, &source, &out)?;
    ensure!(
        warm.checkpoint()? == bytes,
        "clock proof changed initial retained bytes"
    );
    let mut cold_app = cold(&warm, &bytes)?;
    equal(&warm, &cold_app)?;
    let initial = summary(&warm)?;
    ensure!(
        initial["tick"] == 197015
            && initial["factory134"].is_null()
            && initial["person134"].is_null()
            && initial["retired134"] == false
            && initial["asset134"] == false
            && initial["deletion"] == json!([]),
        "cleanup not actually complete"
    );
    let initial_error = warm.viewer.error.clone();
    let initial_menu_error = warm.viewer.menu_error.clone();
    let clock_before = serde_json::to_value(&warm.session()?.host.presentation.clock)?;
    ensure!(
        clock_before["call"]["frame"]["pending"] == "UpdateLot"
            && clock_before["call"]["frame"]["stage"] == "AfterUpdate",
        "actual UpdateLot caller absent"
    );
    write_json(
        &out.join("initial.json"),
        &json!({"source":source,"source_sha256":source_sha,"state":initial,"clock":clock_before,"full":full(&warm)?,"source_owners":identities(&warm)?}),
    )?;
    let mut outcomes = vec![];
    for app in [&mut warm, &mut cold_app] {
        outcomes.push(
            app.viewer
                .session
                .as_mut()
                .unwrap()
                .advance_clock(1)
                .map_err(|e| format!("{e:#}")),
        );
    }
    ensure!(
        outcomes[0] == outcomes[1],
        "actual retained clock outcomes differ"
    );
    let session_before = [
        serde_json::to_vec(&warm.session()?.save())?,
        serde_json::to_vec(&cold_app.session()?.save())?,
    ];
    let program_before = [
        serde_json::to_vec(&warm.session()?.program)?,
        serde_json::to_vec(&cold_app.session()?.program)?,
    ];
    let owners_before = [identities(&warm)?, identities(&cold_app)?];
    ensure!(
        session_before[0] == session_before[1]
            && program_before[0] == program_before[1]
            && owners_before[0] == owners_before[1],
        "raw post-clock Session/Program/owned contents differ before surface publication"
    );
    write_json(
        &out.join("actual-call-before-publication.json"),
        &json!({"method":"Session.advance_clock(1)","outcome":format!("{:?}",outcomes[0]),
        "state":summary(&warm)?,"clock":serde_json::to_value(&warm.session()?.host.presentation.clock)?,
        "full_session_sha256":engine::digest(&session_before[0]),"full_program_sha256":engine::digest(&program_before[0]),"source_owners":owners_before[0],
        "safe_player_boundary":false,"camera_surface_publication_pending":true}),
    )?;
    if let Err(e) = &outcomes[0] {
        anyhow::bail!("actual retained clock error before surface publication: {e}");
    }
    // Same normal publisher called by Viewer.refresh. It has only immutable
    // access to Session and cannot advance VM, clock, animation or RNG.
    for app in [&mut warm, &mut cold_app] {
        let viewer = &mut app.viewer;
        viewer
            .device
            .as_mut()
            .context("camera Device absent")?
            .refresh(viewer.session.as_ref().context("Session absent")?)?;
    }
    ensure!(
        session_before
            == [
                serde_json::to_vec(&warm.session()?.save())?,
                serde_json::to_vec(&cold_app.session()?.save())?
            ]
            && program_before
                == [
                    serde_json::to_vec(&warm.session()?.program)?,
                    serde_json::to_vec(&cold_app.session()?.program)?
                ]
            && owners_before == [identities(&warm)?, identities(&cold_app)?],
        "ordinary headless camera publication changed full Session/Program/owned content/RNG"
    );
    write_json(
        &out.join("surface-publication-neutrality.json"),
        &json!({"actual_api":"camera_device::Device.refresh(&Session)","full_session_bytes_unchanged_both_branches":true,
        "full_program_bytes_unchanged_both_branches":true,"owned_content_unchanged_both_branches":true,
        "session_sha256":engine::digest(&session_before[0]),"program_sha256":engine::digest(&program_before[0]),"state":summary(&warm)?}),
    )?;
    equal(&warm, &cold_app)?;
    write_json(
        &out.join("actual-call.json"),
        &json!({"method":"Session.advance_clock(1)","calls_per_branch":1,"outcome":format!("{:?}",outcomes[0]),"before":initial,"after":summary(&warm)?,"clock_before":clock_before,"clock_after":serde_json::to_value(&warm.session()?.host.presentation.clock)?,"full":full(&warm)?}),
    )?;
    let result = (|| -> Result<()> {
        if let Err(e) = &outcomes[0] {
            anyhow::bail!("actual retained clock error: {e}");
        }
        ensure!(
            outcomes[0]
                == Ok(sims_runtime::original_session::clock::Progress::Household(
                    sims_runtime::original_session::Progress::Tick
                )),
            "retained Clock did not complete household tick"
        );
        ensure!(
            warm.session()?.simulation.vm.tick == 197016,
            "actual Clock did not complete exactly one tick"
        );
        ensure!(
            warm.viewer.error == initial_error && warm.viewer.menu_error == initial_menu_error,
            "clock proof cleared original frontend error"
        );
        recheckpoint(
            &warm,
            &mut cold_app,
            &out,
            0,
            "actual-clock-completed",
            true,
        )?;
        write_json(
            &out.join("complete.json"),
            &json!({"actual_api":"Session.advance_clock(1)","calls_per_branch":1,"completed_ticks":1,"start_tick":197015,"end_tick":197016,
            "source":source,"source_sha256":source_sha,"full":full(&warm)?,"state":summary(&warm)?,"source_owners":identities(&warm)?,
            "prior_timed_out_runs_remain_timed_out":true,"safe_player_boundary":false,"strict_original_parity_ticks":0}),
        )?;
        Ok(())
    })();
    if let Err(e) = &result {
        write_json(
            &out.join("first-error.json"),
            &json!({"error":format!("{e:#}"),"state":summary(&warm)?}),
        )?;
        for (name, app) in [("warm", &warm), ("cold", &cold_app)] {
            let bytes = app.checkpoint()?;
            space(&out, bytes.len() as u64)?;
            std::fs::write(out.join(format!("failure-{name}.json")), bytes)?;
        }
    }
    ensure!(
        engine::digest(&std::fs::read(&source)?) == source_sha,
        "cleanup input changed"
    );
    result
}

#[test]
#[ignore = "exact private real-character retirement checkpoint;16invalid-owner negatives"]
fn actual_retired_rejection_rejects_forged_owners() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run_retired_negatives)?
        .join()
        .expect("retired owner proof panicked")
}
fn run_retired_negatives() -> Result<()> {
    let repo = PathBuf::from(std::env::var("SIMS_NATIVE_CONTINUATION_REPO")?).canonicalize()?;
    let out = repo.join(std::env::var("SIMS_NATIVE_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(repo.join("cache")),
        "private output required"
    );
    std::fs::create_dir(&out)?;
    space(&out, 0)?;
    let source=repo.join("cache/native-owned-continuation-20261024-first/person-rejection-private/actual-yields-third/023-vm-yield.json");
    let bytes = std::fs::read(&source)?;
    let source_sha = engine::digest(&bytes);
    ensure!(
        source_sha == "5494a613a240e763910153663942a2ade22a81eba91cfc67cb6ea3edbaf0597f",
        "not exact real retired person134 checkpoint"
    );
    let warm = restore(&repo, &source, &out)?;
    ensure!(
        warm.checkpoint()? == bytes,
        "current source changed original retired checkpoint bytes"
    );
    let cold_app = cold(&warm, &bytes)?;
    equal(&warm, &cold_app)?;
    let state = summary(&warm)?;
    ensure!(
        state["person134"].is_null()
            && state["retired134"] == true
            && state["asset134"] == true
            && state["factory134"]["parent"] == 387
            && state["factory134"]["request"] == 289
            && state["factory134"]["rejected_definition"]["guid"] == 3229573508u32,
        "actual retired ownership differs"
    );
    forged_retired_owners(&warm, &out)?;
    let cases: Value =
        serde_json::from_slice(&std::fs::read(out.join("retired-owner-negatives.json"))?)?;
    ensure!(
        cases["cases"].as_array().is_some_and(|v| v.len() == 16),
        "not all16negative cases reached"
    );
    equal(&warm, &cold_app)?;
    ensure!(
        warm.checkpoint()? == bytes,
        "negative restore changed original retained state"
    );
    write_json(
        &out.join("complete.json"),
        &json!({"source":source,"source_sha256":source_sha,"rejected_owner_cases":16,
        "full":full(&warm)?,"source_owners":identities(&warm)?,"state":state,
        "no_vm_steps":true,"completed_ticks":0,"strict_original_parity_ticks":0,"safe_player_boundary":false}),
    )
}
