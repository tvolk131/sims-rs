//! Import observed incoming allocations without rebuilding rooms or reading IFF.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sims_fixtures::{Pin, Store};
use sims_sim::saved_matrix::{
    Allocated, Allocation, AllocationRequest, Backing, Host, Lookup, Matrix, Resize, Stop,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

struct NoServices;
impl Host for NoServices {
    fn lookup(&mut self, _: u32, _: u32, _: u32) -> Result<Lookup, Stop> {
        Err(Stop::Unavailable("unexpected lookup".into()))
    }
    fn source(&mut self, _: u64) -> Result<Vec<u8>, Stop> {
        Err(Stop::Unavailable("unexpected source read".into()))
    }
    fn release(&mut self, _: u64) -> Result<(), Stop> {
        Err(Stop::Unavailable("unexpected source release".into()))
    }
    fn free(&mut self, _: u64) -> Result<(), Stop> {
        Err(Stop::Unavailable("unexpected allocation free".into()))
    }
    fn allocate(&mut self, _: AllocationRequest) -> Result<Allocated, Stop> {
        Err(Stop::Unavailable("unexpected allocation".into()))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Lights {
    used: usize,
    capacity: usize,
    data: Vec<[u8; 4]>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Incoming {
    active_size: i32,
    matrices: BTreeMap<String, Matrix>,
    lights: Vec<Lights>,
    converter_scratch: u32,
    /// Newly declared Rust capability domain; not a recovered native counter.
    next_unpublished_allocation: u64,
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 2,
        "usage: retained_world_arrays INPUT.json NEW_OUTPUT"
    );
    let raw = fs::read(&args[0])?;
    let input: serde_json::Value = serde_json::from_slice(&raw)?;
    ensure!(
        input["schema"] == 1
            && input["epoch"] == "resize_entry"
            && input["evidence"]["row"]["sha256"]
                == "e5002772820e16fc1ee15628ab08c598676d61b31a1d8e0df80a05d8938dde7a",
        "foreign incoming array epoch"
    );
    let mut matrices = BTreeMap::new();
    let mut allocation_tokens = BTreeMap::new();
    let mut bindings = vec![];
    for (name, value) in input["matrices"]
        .as_object()
        .context("matrix observations absent")?
    {
        let base = value["native_backing_base"]
            .as_u64()
            .context("backing identity absent")?;
        ensure!(
            base != 0 && !allocation_tokens.contains_key(&base),
            "this incoming profile requires distinct observed allocations"
        );
        let token = allocation_tokens.len() as u64 + 1;
        allocation_tokens.insert(base, token);
        let matrix = Matrix {
            width: serde_json::from_value(value["width"].clone())?,
            height: serde_json::from_value(value["height"].clone())?,
            stride: serde_json::from_value(value["stride"].clone())?,
            allocation: Allocation::Known(Backing {
                token,
                column_offsets: serde_json::from_value(value["column_offsets"].clone())?,
                data: serde_json::from_value(value["data"].clone())?,
            }),
        };
        let Allocation::Known(backing) = &matrix.allocation else {
            unreachable!()
        };
        ensure!(
            matrix.width == 64 && matrix.height == 64 && backing.column_offsets.len() == 64,
            "unsupported incoming allocation dimensions"
        );
        let row_bytes = usize::try_from(matrix.height * matrix.stride)?;
        for &offset in &backing.column_offsets {
            ensure!(
                usize::try_from(offset)?
                    .checked_add(row_bytes)
                    .is_some_and(|end| end <= backing.data.len()),
                "incoming column outside observed backing"
            );
        }
        bindings.push(serde_json::json!({"matrix":name,"native_base":base,"new_token":token}));
        matrices.insert(name.clone(), matrix);
    }
    let expected: BTreeSet<_> = [
        "Altitude",
        "Ground",
        "Grass",
        "TargetGrass",
        "Floor0",
        "Floor1",
        "Primary0",
        "Primary1",
        "Flags0",
        "Flags1",
        "Head0",
        "Head1",
        "Wall0",
        "Wall1",
        "Pool",
        "Water",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    ensure!(
        matrices.keys().cloned().collect::<BTreeSet<_>>() == expected,
        "incoming matrix slots differ"
    );
    let mut lights = vec![];
    for value in input["lights"]
        .as_array()
        .context("light observations absent")?
    {
        let bytes: Vec<u8> = serde_json::from_value(value["data"].clone())?;
        let (entries, tail) = bytes.as_chunks::<4>();
        ensure!(tail.is_empty(), "partial light record");
        let light = Lights {
            used: serde_json::from_value(value["used"].clone())?,
            capacity: serde_json::from_value(value["capacity"].clone())?,
            data: entries.to_vec(),
        };
        ensure!(
            light.used <= light.capacity && light.data.len() == light.capacity,
            "incomplete light capacity"
        );
        lights.push(light);
    }
    ensure!(lights.len() == 2, "incoming light owner count differs");
    let incoming = Incoming {
        active_size: serde_json::from_value(input["active_size"].clone())?,
        matrices,
        lights,
        converter_scratch: serde_json::from_value(input["converter_scratch"].clone())?,
        next_unpublished_allocation: allocation_tokens.len() as u64 + 1,
    };
    let original = incoming.clone();
    let mut retained = incoming;
    // Execute the actual same-dimension branch on every observed allocation.
    // Any source/allocate/free service is an error; no fresh initialization is
    // allowed to make a superficially equal replacement.
    for matrix in retained.matrices.values_mut() {
        let mut resize = Resize::new(matrix.width, matrix.height);
        let mut complete = false;
        for _ in 0..4 {
            if let Some(status) = resize.step(matrix, &mut NoServices)? {
                ensure!(status, "same-size resize failed");
                complete = true;
                break;
            }
        }
        if !complete {
            bail!("same-size resize did not complete");
        }
    }
    ensure!(
        retained == original,
        "same-size branch reconstructed or changed incoming storage"
    );
    let output = PathBuf::from(&args[1]);
    fs::create_dir(&output).context("output must be new")?;
    let store = Store::create(output.join("store"))?;
    let blob = store.put_value(&retained)?;
    let cold: Incoming = store.get_value(&blob)?;
    ensure!(cold == retained, "incoming array cold roundtrip differs");
    store.pin(&Pin {
        schema: 1,
        id: "retained-world-array-owners".into(),
        reason: "Actual resize-entry matrix/light owners only; no saved array reads or new rooms"
            .into(),
        blobs: vec![blob.clone()],
    })?;
    let report = serde_json::json!({"schema":1,"input_sha256":sims_fixtures::digest(&raw),"snapshot":blob,"matrix_count":retained.matrices.len(),"light_count":retained.lights.len(),"new_identity_domain":bindings,"cold_equal":true,"same_size_native_branch_preserves_all_allocations":true,"source_allocate_free_services_used":0,"primary_rooms_rebuilt":false,"saved_array_payloads_consumed":false,"post_resize_values_claimed":false,"original_gameplay_ticks_added":0});
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
