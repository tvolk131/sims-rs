use super::*;

fn validate_managed_input(bytes: &[u8], expected_sha256: &str, source: &str) -> Result<()> {
    ensure!(
        engine::digest(bytes) == expected_sha256,
        "frozen discovery managed input changed"
    );
    let value: Value = serde_json::from_slice(bytes)?;
    ensure!(
        value["session"]["version"].as_u64() == Some(2)
            && value["session"]["source"].as_str() == Some(source),
        "discovery input must be a managed save for the requested household"
    );
    Ok(())
}

#[test]
#[ignore = "private installed game data; new SIMS_ALPHA_OUTPUT beneath cache required"]
fn discover_alpha_household_actions() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("alpha discovery worker panicked")
}

fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let relative = PathBuf::from(std::env::var("SIMS_ALPHA_OUTPUT")?);
    ensure!(
        relative.starts_with("cache")
            && relative
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_))),
        "alpha output must stay beneath cache"
    );
    let output = root.join(relative);
    validate_output(
        &root.join("game-data/cabinet-extracted"),
        &output.join("player-save.json"),
    )?;
    std::fs::create_dir(&output).context("refusing to overwrite alpha discovery evidence")?;
    let household = std::env::var("SIMS_ALPHA_HOUSEHOLD").unwrap_or_else(|_| "goth".into());
    let source = super::alpha_activities::household_source(&household)?;
    let managed_input = std::env::var_os("SIMS_ALPHA_MANAGED_INPUT")
        .map(|relative| -> Result<(PathBuf, String)> {
            let path = root.join(relative).canonicalize()?;
            ensure!(
                path.starts_with(root.join("cache/playable-alpha").canonicalize()?),
                "discovery managed input must stay beneath cache/playable-alpha"
            );
            let bytes = std::fs::read(&path)?;
            let sha256 = std::env::var("SIMS_ALPHA_MANAGED_INPUT_SHA256")?;
            validate_managed_input(&bytes, &sha256, source)?;
            Ok((path, sha256))
        })
        .transpose()?;
    let mut args = Args::parse_from([
        "sims-room",
        "--original",
        source,
        "--paused",
        "--inferred-routing",
        "--buy-preview",
        "--neighborhood-policy",
        "installed-houses-ascending-v1",
        "--root",
        root.join("game-data/cabinet-extracted").to_str().unwrap(),
        "--save",
        output.join("player-save.json").to_str().unwrap(),
    ]);
    if let Some((path, _)) = &managed_input {
        args.load = Some(path.clone());
    }
    let limits = Limits {
        commands: 20_000,
        completed_ticks: 10_000,
        service_polls: 20_000,
        wall_seconds: 600.,
    };
    let wall = Instant::now();
    let mut app = None;
    let mut evidence = Evidence {
        commands: vec![],
        events: vec![],
        samples: vec![],
        frames: 0,
        dialogs: 0,
    };
    let result = (|| -> Result<()> {
        let ready = Arc::new(Ready::load(&args)?);
        app = Some(Application::install_ready(
            &args,
            ready,
            Instant::now(),
            &limits,
        )?);
        let app = app.as_mut().unwrap();
        if managed_input.is_none() && household != "newbie" {
            play_for(
                app,
                60,
                "fresh_discovery_startup",
                wall,
                &limits,
                &mut evidence,
            )?;
            app.execute(&Command::Playing { value: false }, &limits)?;
        }
        let session = app.session()?;
        let inventory: Vec<_> = session
            .simulation
            .vm
            .world
            .objects
            .iter()
            .filter_map(|(&id, object)| {
                let definition = object.definition.as_ref()?;
                let module = session.program.modules.get(&definition.source)?;
                let entries = module.object_interactions.get(&definition.object);
                Some(json!({"object":id,"person":object.person.is_some(),
                    "identity":Identity::get(session,id).ok(),"definition":definition,
                    "position":session.simulation.vm.world.positions.get(&id),
                    "menu_table":entries}))
            })
            .collect();
        write_json(
            &output.join("inventory.json"),
            &json!({"household":household,
            "source":source,"tick":session.simulation.vm.tick,"calendar":calendar(app)?,
            "objects":inventory,"residents":app.residents()?,"acceptance_complete":false}),
        )?;
        std::fs::write(
            output.join("application-checkpoint.json"),
            app.checkpoint()?,
        )?;
        app.viewer.save()?;
        if let Ok(targets) = std::env::var("SIMS_ALPHA_MENU_TARGETS") {
            let targets: Vec<i16> = serde_json::from_str(&targets)?;
            ensure!(
                !targets.is_empty() && targets.len() <= 32,
                "invalid discovery target count"
            );
            let actor = app
                .residents()?
                .into_iter()
                .next()
                .context("no selectable discovery resident")?;
            let mut menus = Vec::new();
            for target in targets {
                let mut branch = app.fork()?;
                let identity = Identity::get(branch.session()?, target)?;
                let result = (|| -> Result<()> {
                    branch.execute(&Command::Live, &limits)?;
                    branch.execute(
                        &Command::Resident {
                            identity: actor.clone(),
                        },
                        &limits,
                    )?;
                    branch.execute(
                        &Command::Object {
                            identity: identity.clone(),
                        },
                        &limits,
                    )?;
                    Ok(())
                })();
                menus.push(json!({"actor":actor,"target":identity,
                    "error":result.as_ref().err().map(|event|format!("{event:#}")),
                    "actions":branch.menu().map(|menu|menu.actions())}));
            }
            write_json(
                &output.join("offered-menus.json"),
                &json!({"schema":1,"menus":menus}),
            )?;
        }
        Ok(())
    })();
    write_json(
        &output.join("discovery-report.json"),
        &json!({"schema":1,
        "discovery_passed":result.is_ok(),"acceptance_complete":false,"household":household,
        "startup":if managed_input.is_some() {"production_managed_load"} else {"fresh_original_import"},
        "managed_input":managed_input.as_ref().map(|(path, sha256)|json!({"path":path,"sha256":sha256})),
        "error":result.as_ref().err().map(|event|format!("{event:#}")),
        "wall_seconds":wall.elapsed().as_secs_f64(),"commands":evidence.commands,
        "events":evidence.events,"samples":evidence.samples}),
    )?;
    result
}

#[test]
fn discovery_managed_input_requires_exact_bytes_version_and_household() -> Result<()> {
    let source = "UserData/Houses/House07.iff";
    let value = json!({"session":{"version":2,"source":source}});
    let bytes = serde_json::to_vec(&value)?;
    let sha256 = engine::digest(&bytes);
    validate_managed_input(&bytes, &sha256, source)?;
    assert!(validate_managed_input(&bytes, &"0".repeat(64), source).is_err());
    assert!(validate_managed_input(&bytes, &sha256, "UserData/Houses/House05.iff").is_err());
    for replacement in [
        json!({"version":1,"source":source}),
        json!({"source":source}),
        json!({"version":2}),
        Value::Null,
    ] {
        let rejected = serde_json::to_vec(&json!({"session":replacement}))?;
        assert!(validate_managed_input(&rejected, &engine::digest(&rejected), source).is_err());
    }
    let mut changed = bytes;
    changed.push(b' ');
    assert!(validate_managed_input(&changed, &sha256, source).is_err());
    Ok(())
}
