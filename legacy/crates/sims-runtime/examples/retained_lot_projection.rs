//! Source-bound lot reconstruction at the captured incoming resize boundary.
//! This is a component check, not publication of a complete original process.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sims_sim::{
    lot_geometry::{FloorSource, Lot},
    lot_rooms::{Metrics, Partition},
};
use std::{collections::BTreeMap, fs, path::PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FloorInput {
    source: FloorSource,
    expected_rooms: Partition,
    expected_metrics: BTreeMap<i16, Metrics>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpperFlags {
    contents: bool,
    support: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u32,
    evidence: serde_json::Value,
    size: usize,
    active_size: usize,
    heights: Vec<[u8; 4]>,
    pool: Vec<u8>,
    water: Vec<u8>,
    floors: Vec<FloorInput>,
    final_upper_flags: UpperFlags,
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 2,
        "usage: retained_lot_projection INPUT.json NEW_OUTPUT_DIRECTORY"
    );
    let raw = fs::read(&args[0])?;
    let input: Input = serde_json::from_slice(&raw)?;
    ensure!(input.schema == 1, "unsupported lot projection schema");
    ensure!(
        input.evidence["row"]["sha256"]
            == "e5002772820e16fc1ee15628ab08c598676d61b31a1d8e0df80a05d8938dde7a"
            && input.evidence["elf_sha256"]
                == "280e32b85aac6e4e4e9933e7fc5b6a12d58fd2918a7420b16e95bbd91aea3e65",
        "unsupported captured source; this helper does not authenticate arbitrary provenance"
    );
    ensure!(input.floors.len() == 2, "requires both captured floors");
    let lot = Lot::rebuild_in_active(
        input.size,
        input.active_size,
        input.heights,
        input.pool,
        input.water,
        input.floors.iter().map(|f| f.source.clone()).collect(),
    )
    .map_err(anyhow::Error::msg)?;
    for (number, (floor, expected)) in lot.floors.iter().zip(&input.floors).enumerate() {
        ensure!(
            floor.rooms == expected.expected_rooms,
            "floor {} room partition/order differs",
            number + 1
        );
        ensure!(
            floor.flags.flags == expected.source.flags,
            "floor {} native tile flags differ",
            number + 1
        );
        ensure!(
            floor.metrics == expected.expected_metrics,
            "floor {} native room metrics differ",
            number + 1
        );
    }
    // These native world bytes describe the last completed floor pass only.
    // Do not assign them to both floors or invent a lower-floor observation.
    let last = &lot.floors.last().context("final floor absent")?.flags;
    ensure!(
        last.upper_contents == input.final_upper_flags.contents
            && last.upper_support == input.final_upper_flags.support,
        "final native upper-floor flags differ"
    );
    lot.validate().map_err(anyhow::Error::msg)?;
    let output = PathBuf::from(&args[1]);
    fs::create_dir(&output)?;
    let store = sims_fixtures::Store::create(output.join("store"))?;
    let blob = store.put_value(&lot)?;
    let cold: Lot = store.get_value(&blob)?;
    ensure!(cold == lot, "cold lot differs");
    cold.validate().map_err(anyhow::Error::msg)?;
    store.pin(&sims_fixtures::Pin {
        schema: 1,
        id: "retained-incoming-lot".into(),
        reason:
            "Captured incoming lot component; full Incoming and original parity remain unqualified"
                .into(),
        blobs: vec![blob.clone()],
    })?;
    store.verify_all()?;
    let report = serde_json::json!({
        "schema": 1, "input_sha256": format!("{:x}", Sha256::digest(&raw)),
        "size": lot.size, "active_size": lot.active_extent(),
        "floors": lot.floors.iter().map(|f| serde_json::json!({
            "cells": f.source.flags.len(), "rooms": f.metrics,
            "ordered_room_registrations": f.rooms.room_tiles.values().map(Vec::len).sum::<usize>(),
            "split_cells": f.rooms.halves.len(),
        })).collect::<Vec<_>>(),
        "captured_primary_flags_metrics_and_registration_order_equal": true,
        "cold_equal": true, "lot": blob, "full_incoming_constructed": false,
        "original_parity_ticks_added": 0,
    });
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
