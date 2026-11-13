use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_sim::pose::{
    animator::{Animator, Settings, Slot},
    sampling::BoneInput,
};

fn exact(actual: &Value, expected: &Value, path: &str) -> Result<()> {
    match (actual, expected) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                exact(&a[k], v, &format!("{path}.{k}"))?;
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            ensure!(a.len() == b.len(), "length {path}");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                exact(a, b, &format!("{path}[{i}]"))?;
            }
        }
        (Value::Number(a), Value::Number(b)) if a.is_f64() || b.is_f64() => ensure!(
            (a.as_f64().unwrap() as f32).to_bits() == (b.as_f64().unwrap() as f32).to_bits(),
            "float {path}: {a} / {b}"
        ),
        _ => ensure!(actual == expected, "{path}: {actual} / {expected}"),
    }
    Ok(())
}
fn pose(b: &BoneInput, expected: &Value, context: &str) -> Result<()> {
    for (key, values, tolerance) in [
        ("translation", b.local.translation.as_slice(), 3e-5),
        ("rotation", b.local.rotation.as_slice(), 1e-6),
    ] {
        for (a, b) in values
            .iter()
            .zip(expected[key].as_array().context("channel")?)
        {
            ensure!(
                (*a - b.as_f64().unwrap() as f32).abs() < tolerance,
                "pose {context} {key}: {a} / {b}"
            );
        }
    }
    Ok(())
}
#[test]
#[ignore = "requires original BCF/CFP files and private native animator sequence report"]
fn original_layer_replacements_match_native_events_composition_and_retirement() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = sims_content::Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let house = sims_content::saves::House::load(&index, "UserData/Houses/House07.iff")?;
    let resident = &house.household.as_ref().context("household")?.residents[0];
    let person = house.objects.instances[&resident.object]
        .person
        .as_ref()
        .context("person")?;
    let avatar = sims_content::saved_avatar::load(&index, &resident.body_strings[0], person)?;
    let report: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-animator-sequences.json"),
    )?)?;
    let mut program = sims_vm::Program::default();
    let mut count = 0;
    let mut callbacks = 0;
    for (seq, row) in report["sequences"]
        .as_array()
        .context("sequences")?
        .iter()
        .enumerate()
    {
        let initial = &row["initial"];
        let settings: Settings = serde_json::from_value(initial["settings"].clone())?;
        let mut animator = Animator::new(0, 0, 0, true, -1);
        let mut bones: Vec<BoneInput> = serde_json::from_value(row["bones"].clone())?;
        for (step, row) in row["steps"].as_array().context("steps")?.iter().enumerate() {
            let a = &row["action"];
            let now = a["now"].as_i64().unwrap() as i32;
            let expected = &row["original"];
            let context = format!("{seq}/{step} {a}");
            let mut result = Value::Null;
            let mut seen = 0;
            match a["op"].as_str().unwrap() {
                "start" => {
                    let name = a["name"].as_str().unwrap();
                    if !program.animations.contains_key(name) {
                        sims_content::assets::load_named_animations(
                            &index,
                            &mut program,
                            &[name.to_owned()].into(),
                        )?;
                    }
                    result = json!(
                        animator
                            .start(
                                serde_json::from_value(a["slot"].clone())?,
                                &avatar.skeleton,
                                Some(&program.animations[name].clip),
                                a["reverse"].as_bool().unwrap(),
                                now,
                                settings
                            )
                            .map_err(anyhow::Error::msg)?
                    );
                }
                "clear" => animator
                    .clear(
                        serde_json::from_value::<Slot>(a["slot"].clone())?,
                        now,
                        settings,
                    )
                    .map_err(anyhow::Error::msg)?,
                "promote" => animator
                    .set_promote_next_foreground(a["value"].as_bool().unwrap())
                    .map_err(anyhow::Error::msg)?,
                "update" => {
                    animator
                        .begin(
                            now,
                            a["sample_pose"].as_bool().unwrap(),
                            true,
                            a["force"].as_bool().unwrap(),
                            &mut bones,
                        )
                        .map_err(anyhow::Error::msg)?;
                    while let Some(event) = animator
                        .next(&program, &mut bones)
                        .map_err(anyhow::Error::msg)?
                    {
                        let e = &expected["callbacks"][seen];
                        ensure!(
                            e["layer"] == event.layer.0
                                && e["binding"] == event.dispatch.binding
                                && e["bone"] == event.dispatch.bone
                                && e["event"] == event.dispatch.event,
                            "event {context}"
                        );
                        exact(&json!(event.dispatch.phase), &e["phase"], &context)?;
                        let state = serde_json::to_value(&animator)?;
                        for key in ["generation", "pose_generation"] {
                            exact(&state[key], &e[key], &format!("{context} callback {key}"))?;
                        }
                        pose(&bones[event.dispatch.bone], e, &context)?;
                        (animator, bones) =
                            serde_json::from_slice(&serde_json::to_vec(&(&animator, &bones))?)?;
                        ensure!(
                            animator
                                .next(&program, &mut bones)
                                .map_err(anyhow::Error::msg)?
                                == Some(event),
                            "pending callback {context}"
                        );
                        animator.acknowledge().map_err(anyhow::Error::msg)?;
                        (animator, bones) =
                            serde_json::from_slice(&serde_json::to_vec(&(&animator, &bones))?)?;
                        seen += 1;
                        callbacks += 1;
                    }
                }
                other => anyhow::bail!("unexpected action {other}"),
            }
            ensure!(
                seen == expected["callbacks"].as_array().unwrap().len(),
                "callback count {context}"
            );
            let mut projection = expected.clone();
            for key in ["callbacks", "bones", "released"] {
                projection.as_object_mut().unwrap().remove(key);
            }
            let mut actual = serde_json::to_value(&animator)?;
            actual["result"] = result;
            exact(&actual, &projection, &context)?;
            for (b, e) in bones.iter().zip(expected["bones"].as_array().unwrap()) {
                pose(b, e, &context)?;
                ensure!(e["priority"] == b.priority, "priority {context}");
            }
            (animator, bones) = serde_json::from_slice(&serde_json::to_vec(&(&animator, &bones))?)?;
            count += 1;
        }
    }
    ensure!(count >= 250 && callbacks > 0);
    println!(
        "Compared {count} original multi-layer steps, {callbacks} callbacks, and all bone poses"
    );
    Ok(())
}

#[test]
#[ignore = "requires original clips and native complete callback sequence report"]
fn live_skin_executes_original_properties_between_native_pose_samples() -> Result<()> {
    use sims_sim::pose::{
        live::Live,
        properties::{Context as PropertyContext, Request},
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = sims_content::Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let house = sims_content::saves::House::load(&index, "UserData/Houses/House07.iff")?;
    let resident = &house.household.as_ref().context("household")?.residents[0];
    let person = house.objects.instances[&resident.object]
        .person
        .as_ref()
        .context("person")?;
    let avatar = sims_content::saved_avatar::load(&index, &resident.body_strings[0], person)?;
    let report: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-animator-callbacks.json"),
    )?)?;
    let mut program = sims_vm::Program::default();
    let mut count = 0;
    let mut service_count = 0;
    let context = PropertyContext {
        actor: 137,
        sound_visible: true,
        selected: true,
        debug_sound: false,
        position_x: 32.,
        appearance_mode: 0,
        snow: false,
        lot_kind: 1,
    };
    for (seq, row) in report["sequences"]
        .as_array()
        .context("sequences")?
        .iter()
        .enumerate()
    {
        let settings: Settings = serde_json::from_value(row["initial"]["settings"].clone())?;
        let mut live = Live::new(
            Animator::new(0, 0, 0, true, -1),
            serde_json::from_value(row["bones"].clone())?,
        );
        let mut person = [
            (22, 0),
            (23, 0),
            (30, 0),
            (65, 0),
            (68, 0),
            (74, 0),
            (84, 0),
        ]
        .into();
        for (step, row) in row["steps"].as_array().context("steps")?.iter().enumerate() {
            let a = &row["action"];
            let now = a["now"].as_i64().unwrap() as i32;
            let expected = &row["original"];
            let label = format!("callbacks {seq}/{step} {a}");
            let mut result = Value::Null;
            let mut actions = vec![];
            match a["op"].as_str().unwrap() {
                "start" => {
                    let name = a["name"].as_str().unwrap();
                    if !program.animations.contains_key(name) {
                        sims_content::assets::load_named_animations(
                            &index,
                            &mut program,
                            &[name.to_owned()].into(),
                        )?;
                    }
                    result = json!(
                        live.animator
                            .start(
                                serde_json::from_value(a["slot"].clone())?,
                                &avatar.skeleton,
                                Some(&program.animations[name].clip),
                                a["reverse"].as_bool().unwrap(),
                                now,
                                settings
                            )
                            .map_err(anyhow::Error::msg)?
                    );
                }
                "clear" => live
                    .animator
                    .clear(serde_json::from_value(a["slot"].clone())?, now, settings)
                    .map_err(anyhow::Error::msg)?,
                "promote" => live
                    .animator
                    .set_promote_next_foreground(a["value"].as_bool().unwrap())
                    .map_err(anyhow::Error::msg)?,
                "update" => {
                    live.begin(
                        now,
                        a["sample_pose"].as_bool().unwrap(),
                        true,
                        a["force"].as_bool().unwrap(),
                    )
                    .map_err(anyhow::Error::msg)?;
                    while let Some(request) = live
                        .next(&program, &mut person, context)
                        .map_err(anyhow::Error::msg)?
                    {
                        (live, person) =
                            serde_json::from_slice(&serde_json::to_vec(&(&live, &person))?)?;
                        ensure!(
                            live.next(&program, &mut person, context)
                                .map_err(anyhow::Error::msg)?
                                == Some(request.clone()),
                            "pending native property {label}"
                        );
                        let surface = match &request {
                            Request::FootstepSurface { .. } => Some(1),
                            Request::Sound { .. } => None,
                            other => anyhow::bail!("unexpected service {other:?}"),
                        };
                        actions.push(request);
                        live.acknowledge(surface).map_err(anyhow::Error::msg)?;
                        (live, person) =
                            serde_json::from_slice(&serde_json::to_vec(&(&live, &person))?)?;
                        service_count += 1;
                    }
                    ensure!(!live.busy(), "unfinished {label}");
                }
                other => anyhow::bail!("unexpected action {other}"),
            }
            ensure!(
                json!(actions) == expected["actions"],
                "service order {label}: {actions:?} / {}",
                expected["actions"]
            );
            let mut projection = expected.clone();
            for key in ["actions", "callback_count", "bones", "released"] {
                projection.as_object_mut().unwrap().remove(key);
            }
            let mut actual = serde_json::to_value(&live.animator)?;
            actual["result"] = result;
            exact(&actual, &projection, &label)?;
            for (b, e) in live.bones.iter().zip(expected["bones"].as_array().unwrap()) {
                pose(b, e, &label)?;
                ensure!(e["priority"] == b.priority, "bone priority {label}");
            }
            (live, person) = serde_json::from_slice(&serde_json::to_vec(&(&live, &person))?)?;
            count += 1;
        }
    }
    ensure!(count == 252 && service_count > 0);
    println!(
        "Compared {count} complete native callback/pose steps and {service_count} surface/sound requests"
    );
    Ok(())
}
