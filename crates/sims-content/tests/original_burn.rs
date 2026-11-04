//! Private integration of the Burn query adapter with a retained original-file
//! replay. Allocation is explicitly refused after selection: this checks the
//! real physical inputs, not native fire Init or a resumed household tick.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_sim::burn::{self, Execution, Input, Object, Request, Response, Wall, World};
use sims_vm::ObjectId;
use std::collections::{BTreeMap, BTreeSet};

struct Traced<'a> {
    world: burn::world::Live<'a>,
    calls: Vec<Value>,
}
impl World for Traced<'_> {
    fn size(&self) -> usize {
        self.world.size()
    }
    fn object(&self, id: ObjectId) -> Result<Option<Object>, String> {
        self.world.object(id)
    }
    fn on_lot(&mut self, id: ObjectId) -> Result<bool, String> {
        self.calls.push(json!(["on_lot", id]));
        self.world.on_lot(id)
    }
    fn tile(&mut self, tile: [i8; 3]) -> Result<Vec<ObjectId>, String> {
        let ids = self.world.tile(tile)?;
        self.calls.push(json!(["tile", tile, ids]));
        Ok(ids)
    }
    fn wall(&mut self, tile: [i8; 3]) -> Result<Wall, String> {
        self.calls.push(json!(["wall", tile]));
        self.world.wall(tile)
    }
}

#[test]
#[ignore = "requires private original assets and the retained Mashuga Burn boundary"]
fn original_mashuga_burn_selection_matches_native_queries() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = std::env::var("SIMS_BURN_SNAPSHOT")
        .context("set SIMS_BURN_SNAPSHOT to the retained original-live-character-final.json")?;
    let retained: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    let snapshot: sims_sim::Snapshot = serde_json::from_value(retained["simulation"].clone())?;
    let mut simulation = snapshot.state;
    ensure!(retained["source"] == "UserData2/Houses/House09.iff");
    ensure!(simulation.vm.tick == 211698 && retained["start"] == 178126);
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    // Rebuild the actual definition/function catalog required by these queries.
    // This is not a claim of full suspended VM/animator restoration.
    let sources: BTreeSet<_> = simulation
        .vm
        .world
        .objects
        .values()
        .filter_map(|o| o.definition.as_ref().map(|d| d.source.clone()))
        .collect();
    let mut program = sims_content::vm::load_program(&index, "Global.iff", "Global.iff")?;
    for source in sources {
        if !program.modules.contains_key(&source) {
            for (name, module) in
                sims_content::vm::load_program(&index, &source, "Global.iff")?.modules
            {
                if let Some(old) = program.modules.get(&name) {
                    ensure!(old.content_hash == module.content_hash);
                } else {
                    program.modules.insert(name, module);
                }
            }
        }
    }
    sims_content::saves::link_neighborhood_creation_resources(
        &index,
        &mut program,
        retained["assets"]["neighborhood_source"]
            .as_str()
            .context("neighborhood source absent")?,
        &[burn::FIRE_GUID].into(),
    )?;
    let fire = burn::world::Resource::loaded_fire(&program)
        .map_err(anyhow::Error::msg)?
        .context("original fire resource absent")?;
    let frame = simulation.vm.threads[&11]
        .stack
        .last()
        .context("Burn frame absent")?;
    ensure!(
        frame.stack_object == 178
            && frame.location.bhav == 4124
            && frame.location.instruction == 15
    );
    let instruction = &program.modules[&frame.location.source].routines[&4124]
        .body
        .instructions[15];
    ensure!(instruction.opcode == 9 && instruction.operands[..2] == [0, 1]);
    let input = Input {
        target: frame.stack_object,
        mode: instruction.operands[0],
        flags: instruction.operands[1],
    };
    let links = &simulation
        .native_lot
        .as_ref()
        .context("native lot absent")?
        .placement
        .links;
    let order: Vec<_> = links
        .heads
        .iter()
        .flatten()
        .filter(|id| **id > 0)
        .map(|id| links.walk(&simulation.vm.world, *id))
        .collect::<Result<Vec<_>, _>>()
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .flatten()
        .collect();
    ensure!(order.iter().collect::<BTreeSet<_>>().len() == order.len());
    let ids: Vec<_> = simulation.vm.world.objects.keys().copied().collect();
    let mut world = burn::world::Live::new(&program, &mut simulation, Some(&fire))
        .map_err(anyhow::Error::msg)?;
    let mut objects = BTreeMap::new();
    let mut records = BTreeMap::new();
    for id in ids {
        let o = world
            .object(id)
            .map_err(anyhow::Error::msg)?
            .context("record absent")?;
        objects.insert(
            id.to_string(),
            json!({
                "position":o.position,"level":o.level,"direction":o.direction,
                "flags":o.flags,"flags2":o.flags2,"uses":o.uses,"type":o.kind,
                "multipart":o.master,"exclude_group":o.group_excluded,
                "head":o.head.unwrap_or(0),"next":o.next.unwrap_or(0),
                "fire":o.fire_resource,"bounds":o.bounds,
            }),
        );
        records.insert(id, o);
    }
    // The supplied native iterator in this probe filters the recorded physical
    // order by tile. Prove that representation matches this retained world;
    // never silently replace the actual containment tree with registry order.
    for tile in records.values().map(|o| o.tile).collect::<BTreeSet<_>>() {
        let expected: Vec<_> = order
            .iter()
            .copied()
            .filter(|id| records[id].tile == tile)
            .collect();
        ensure!(
            world.tile(tile).map_err(anyhow::Error::msg)? == expected,
            "physical tile model differs at {tile:?}"
        );
    }
    let case = json!({"target":input.target,"mode":input.mode,"flags":input.flags,
        "size":world.size(),"on_lot":world.on_lot(input.target).map_err(anyhow::Error::msg)?,
        "resource":true,"allocate":false,"objects":objects,"order":order});
    std::fs::write(
        root.join("cache/native-burn-mashuga-input.json"),
        serde_json::to_vec(&case)?,
    )?;
    let mut control = None;
    for restore in [false, true] {
        let mut s: sims_sim::Simulation =
            serde_json::from_slice(&serde_json::to_vec(&simulation)?)?;
        let mut world = Traced {
            world: burn::world::Live::new(&program, &mut s, Some(&fire))
                .map_err(anyhow::Error::msg)?,
            calls: vec![],
        };
        let mut e = Execution::new(input);
        while let Some(request) = e.next(&mut world).map_err(anyhow::Error::msg)? {
            if restore {
                e = serde_json::from_slice(&serde_json::to_vec(&e)?)?;
                e.validate().map_err(anyhow::Error::msg)?;
                ensure!(e.next(&mut world).map_err(anyhow::Error::msg)? == Some(request.clone()));
            }
            let response = match request {
                Request::Resource => {
                    world.calls.push(json!(["resource", burn::FIRE_GUID]));
                    Response::Resource(true)
                }
                Request::Factory => {
                    world.calls.push(json!(["factory", 0]));
                    Response::Created(None)
                }
                other => anyhow::bail!("unexpected service before refused allocation: {other:?}"),
            };
            e.respond(response).map_err(anyhow::Error::msg)?;
        }
        let result = json!({"error":null,"returned":e.result().map(u32::from),
            "selected":e.affected(),"calls":world.calls});
        if let Some(expected) = &control {
            ensure!(*expected == result);
        }
        control = Some(result);
    }
    let result = control.unwrap();
    if let Ok(path) = std::env::var("SIMS_BURN_NATIVE_RESULT") {
        let native: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(
            native["cases"][0]["input"] == case,
            "native input does not match retained lot"
        );
        ensure!(
            native["cases"][0]["original"] == result,
            "native Burn selection/query sequence differs"
        );
    }
    std::fs::write(
        root.join("cache/native-burn-mashuga-rust.json"),
        serde_json::to_vec(&result)?,
    )?;
    eprintln!(
        "Mashuga Burn: {} real object records, selected {}, {} physical/service calls, restored identically",
        records.len(),
        result["selected"],
        result["calls"].as_array().unwrap().len()
    );
    Ok(())
}
