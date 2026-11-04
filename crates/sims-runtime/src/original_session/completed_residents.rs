//! Completed native character storage, composed without initialization/update.
//! Fresh Rust host continuations are not claims about source queues or World.
use super::{
    completed_geometry::{self, bytes, field, floats, i, rows, u, word},
    completed_skin::{Qualified, Retained},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_content::character_model;
use sims_sim::{
    character::Resident,
    pose::{movement::path, ordinary},
};
use sims_vm::ObjectId;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshAssets {
    pub geometry: sims_content::character_geometry::CharacterGeometry,
    pub native_appearances: BTreeMap<u32, u64>,
    pub native_bindings: Vec<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    pub characters: BTreeMap<ObjectId, MeshAssets>,
    pub meshes: sims_content::character_geometry::MeshPool,
    pub mesh_bmf: Vec<Vec<u8>>,
    pub native_meshes: Vec<u32>,
    pub adapter_epoch: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    pub residents: BTreeMap<ObjectId, Resident>,
    pub models: BTreeMap<ObjectId, character_model::State>,
    pub geometry: Geometry,
    pub retained: BTreeMap<ObjectId, Retained>,
    /// Explicit unresolved dependencies; this is not a playable World baseline.
    pub demands: Vec<String>,
}
fn text(v: &Value) -> Result<String> {
    let b = bytes(v, "used_raw")?;
    ensure!(
        b.is_ascii() && !b.contains(&0),
        "unsupported native resident narrow string"
    );
    Ok(String::from_utf8(b)?)
}
fn exact<T: serde::de::DeserializeOwned + Serialize>(fields: Value) -> Result<T> {
    let state: T = serde_json::from_value(fields.clone())?;
    ensure!(
        serde_json::to_value(&state)? == fields,
        "completed resident fields defaulted or discarded"
    );
    Ok(state)
}
fn settings(v: &Value, address: u32) -> Result<Vec<u8>> {
    super::completed_animations::bytes(
        field(field(v, "settings")?, &address.to_string())?
            .as_str()
            .context("resident settings bytes absent")?,
    )
}
pub fn import(
    input: &Value,
    skins: BTreeMap<ObjectId, Qualified>,
    geometry_input: &Value,
    epoch: &str,
    completed_tick: u64,
) -> Result<Imported> {
    ensure!(
        input["schema"] == 1 && input["executes_target_code"] == false && completed_tick == 88448,
        "resident source cut/profile differs"
    );
    import_fields(input, skins, geometry_input, epoch, completed_tick, None)
}

/// Retain current active movement using the independently imported native route
/// owners. The old public entry keeps its idle-only and exact-tick admission.
/// This is still a component join: native route execution and asset-host state
/// must be supplied before a Simulation can run.
pub fn import_captured(
    input: &Value,
    skins: BTreeMap<ObjectId, Qualified>,
    geometry_input: &Value,
    epoch: &str,
    completed_tick: u64,
    routes: &BTreeMap<ObjectId, super::completed_routes::Person>,
    memory: &sims_fixtures::readset::Readset<'_>,
) -> Result<Imported> {
    ensure!(input["schema"] == 1 && input["executes_target_code"] == false,
        "current resident profile differs");
    super::completed_skeletons::captured::reads(input, memory)?;
    super::completed_skeletons::captured::reads(geometry_input, memory)?;
    ensure!(skins.keys().eq(routes.keys()), "current resident/route coverage differs");
    for (&id, skin) in &skins {
        let route = &routes[&id];
        ensure!(route.readset_sha256 == memory.source_sha256()
            && route.animator_pointer == skin.retained.native.animator,
            "current resident/route source epoch differs");
    }
    import_fields(input, skins, geometry_input, epoch, completed_tick, Some(routes))
}

fn import_fields(
    input: &Value,
    skins: BTreeMap<ObjectId, Qualified>,
    geometry_input: &Value,
    epoch: &str,
    completed_tick: u64,
    routes: Option<&BTreeMap<ObjectId, super::completed_routes::Person>>,
) -> Result<Imported> {
    completed_geometry::check_code(input, CODE)?;
    let mut geometry = completed_geometry::import(geometry_input, &skins, epoch)?;
    let mut geometry_characters = std::mem::take(&mut geometry.characters);
    let mut mesh_assets = BTreeMap::new();
    let actors = rows(input, "actors")?;
    ensure!(
        actors.len() == skins.len(),
        "resident actor coverage differs"
    );
    let mut residents = BTreeMap::new();
    let mut models = BTreeMap::new();
    let mut retained = BTreeMap::new();
    let fade = settings(input, 0x6f6bd0)?;
    ensure!(fade.len() == 1, "native fade setting extent differs");
    let duration = settings(input, 0x6d0460)?;
    let sampling = settings(input, 0x6d0464)?;
    let tuning = settings(input, 0x6d07a8)?;
    let layer_settings = sims_sim::pose::animator::Settings {
        fade_enabled: fade[0] != 0,
        fade_duration: i(&duration, 0)?,
        sampling_config: u(&sampling, 0)?,
    };
    let path_settings = path::Settings {
        sample_step: floats::<1>(&tuning, 0)?[0],
        turn_threshold: floats::<1>(&tuning, 12)?[0],
        turn_step: floats::<1>(&tuning, 16)?[0],
        minimum_brake_time: i(&tuning, 44)?,
        endpoint_distance: floats(&tuning, 48)?,
    };
    for (id, s) in skins {
        let a = actors
            .iter()
            .find(|a| a["object"] == id)
            .context("resident actor absent")?;
        let ar = bytes(a, "animator_raw")?;
        let person = bytes(a, "person_raw")?;
        let n = &s.retained.native;
        ensure!(
            ar.len() == 0x2a0
                && person.len() == 0x188
                && word(a, "person")? == n.person
                && word(a, "animator")? == n.animator
                && u(&ar, 8)? == n.model
                && u(&ar, 0x138)? == n.person,
            "resident owner aliases differ"
        );
        ensure!(ar[0x150] == 0 && ar[0xd8] == 0,
            "resident profile requires separately qualified temporary head/profile continuation");
        if let Some(routes) = routes {
            ensure!(routes[&id].animator_raw == ar,
                "current resident movement header differs");
        } else {
            ensure!(i(&ar, 0x188)? == 0 && i(&ar, 0x190)? == 0,
                "resident profile requires separately qualified active movement/profile continuation");
        }
        let g = geometry_characters
            .remove(&id)
            .context("resident geometry absent")?;
        let clock = i(&ar, 0xb4)?;
        let evaluated_tick = completed_tick
            .checked_sub(1)
            .context("resident previous tick absent")?;
        ensure!(
            ordinary::clock(evaluated_tick as i32, ar[0xa8]) == clock
                && serde_json::to_value(&s.instance.skin.animator)?["last_clock"] == clock,
            "resident evaluation clock boundary differs"
        );
        let needs = field(a, "needs_resource")?;
        ensure!(
            word(needs, "pointer")? == u(&ar, 0x24c)?,
            "needs resource owner differs"
        );
        let nr = bytes(needs, "raw")?;
        let needs_name = field(needs, "name")?;
        ensure!(
            nr.len() == 0x74
                && word(needs_name, "begin")? == u(&nr, 4)?
                && word(needs_name, "end")? == u(&nr, 8)?
                && word(needs_name, "capacity")? == u(&nr, 12)?,
            "needs resource name owner differs"
        );
        let context: sims_sim::pose::properties::Context =
            exact(field(a, "property_context")?.clone())?;
        ensure!(
            context.actor == id
                && context.selected == (u(&person, 0x114)? & 2 != 0)
                && context.appearance_mode == i(&ar, 0x1b0)?
                && context.position_x.to_bits() == u(&ar, 0x80)?
                && context.lot_kind
                    == i16::from_le_bytes(
                        bytes(input, "lot_raw")?
                            .try_into()
                            .map_err(|_| anyhow::anyhow!("lot kind extent differs"))?
                    )
                && context.snow == (40..=42).contains(&context.lot_kind)
                && context.debug_sound == (word(input, "debug_raw")? != 0),
            "native resident property context differs"
        );
        let slot = i16::from_le_bytes(person[0x66..0x68].try_into()?);
        ensure!(slot <= 0, "contained native sound position unsupported");
        let size = word(input, "map_size")? as i32;
        let coordinate = |v: i32| (v >> 4) as i8 as i32;
        let inside = [i(&person, 0xf4)?, i(&person, 0xf8)?]
            .map(coordinate)
            .iter()
            .all(|&x| x > 0 && x < size - 1);
        ensure!(
            context.sound_visible == inside,
            "native sound position validity differs"
        );
        let path = if u(&ar, 0x184)? == 0 {
            ensure!(a["path"].is_null(), "unexpected path owner");
            None
        } else {
            let p = field(a, "path")?;
            let pr = bytes(p, "raw")?;
            let points = field(p, "points")?;
            let b = bytes(points, "used_raw")?;
            ensure!(
                word(p, "pointer")? == u(&ar, 0x184)?
                    && pr.len() == 12
                    && word(points, "begin")? == u(&pr, 0)?
                    && word(points, "end")? == u(&pr, 4)?
                    && word(points, "capacity")? == u(&pr, 8)?
                    && u(&pr, 4)?.checked_sub(u(&pr, 0)?) == Some(b.len() as u32)
                    && b.len() % 8 == 0,
                "resident path ownership differs"
            );
            Some(path::Path {
                points: b
                    .chunks_exact(8)
                    .map(|p| Ok([i(p, 0)?, i(p, 4)?]))
                    .collect::<Result<_>>()?,
                settings: path_settings,
                index: i(&ar, 0x180)?,
                progress: floats::<1>(&ar, 0x1ec)?[0],
                deadline: i(&ar, 0x1c0)?,
                phase_window: [floats::<1>(&ar, 0x174)?[0], floats::<1>(&ar, 0x170)?[0]],
                stop_distance: floats(&ar, 0x178)?,
                start_distance: [floats::<1>(&ar, 0x1f8)?[0], floats::<1>(&ar, 0x204)?[0]],
                lookahead: [floats::<1>(&ar, 0x200)?[0], floats::<1>(&ar, 0x1fc)?[0]],
                interrupt: i(&ar, 0x284)?,
                guard: i(&ar, 0x28c)?,
                delta: floats(&ar, 0x294)?,
                stop_mode: i(&ar, 0x18c)?,
            })
        };
        let mut movement_anchors = [None; 2];
        for (j, o) in [0x168, 0x16c].into_iter().enumerate() {
            let p = u(&ar, o)?;
            if p != 0 {
                movement_anchors[j] = Some(
                    n.bone_pointers
                        .iter()
                        .position(|&b| b == p)
                        .context("movement anchor owner absent")?,
                );
            }
        }
        let model = character_model::State {
            dirty: s.instance.geometry.dirty,
            evaluated_generation: s.instance.geometry.evaluated_generation,
            bounds_generation: s.instance.geometry.bounds_generation,
            bone_generations: s.instance.geometry.bone_generations.clone(),
            wiggle: s.instance.wiggle.clone(),
            flags: s.instance.flags.clone(),
            named_bone_flags: s.instance.named_bone_flags.clone(),
            local_bounds: g.local_bounds,
            render_bounds: g.render_bounds,
            bone_bounds: s.retained.bone_bounds.clone(),
            bone_bounds_generations: s.retained.bone_bounds_generations.clone(),
            previous_mode: i(&ar, 0x194)?,
            phase_window: [floats::<1>(&ar, 0x174)?[0], floats::<1>(&ar, 0x170)?[0]],
            stop_distance: floats(&ar, 0x178)?,
            clip_distances: floats(&ar, 0x1f8)?,
            needs_appearance: text(needs_name)?,
        };
        // These are synchronous C++ stack passes at the qualified completed
        // caller boundary. Rust request tokens start unpublished; queued VM
        // actions/events remain separately owned. This partial0x188-byte header
        // does not capture the native person+18c label string. Complete Session
        // import must bind it through completed_labels; [] here is unknown
        // component-only storage, not evidence of an empty native display cache.
        ensure!(
            u(&ar, 0x250)? == 0,
            "native censor appearance needs its binding owner"
        );
        let character: sims_sim::pose::refresh::Character = exact(
            json!({"skin":s.instance.skin,"hierarchy":s.instance.hierarchy,"head":{"last_clock":u(&ar,0x164)?,"basis_heading":floats::<1>(&ar,0x210)?[0],"target_heading":floats::<1>(&ar,0x214)?[0],"turn":floats::<1>(&ar,0x218)?[0],"target_point":floats::<3>(&ar,0x21c)?},"rig":s.rig,"matrix":g.matrix.values,"placement":{"base":floats::<3>(&ar,0x80)?,"previous":floats::<3>(&ar,0x1c4)?,"offset":floats::<3>(&ar,0x8c)?,"dirty":ar[0xd9]!=0},"model":g.model,"censor":{"binding":null,"mask":i(&ar,0x254)?},"clock":clock,"visible":ar[0x109]!=0,"pass":null}),
        )?;
        let movement = exact::<sims_sim::pose::movement::Movement>(
            json!({"state":{"status":i(&ar,0x188)?,"flags":[i(&ar,0x19c)?,i(&ar,0x290)?],"start":floats::<3>(&ar,0x1d0)?,"end":floats::<3>(&ar,0x1dc)?,"turn_delta":floats::<1>(&ar,0x1f0)?[0],"nominal_turn":floats::<1>(&ar,0x1f4)?[0],"reason":i(&ar,0x288)?,"apply_rate":ar[0x29c]},"path":path,"scalar_7c":floats::<1>(&ar,0x7c)?[0],"pass":null}),
        )?;
        if let Some(routes) = routes {
            ensure!(movement == routes[&id].movement,
                "current resident independently imported movement differs");
        }
        let controls = sims_sim::character::Controls {
            ordinary: ordinary::State {
                clock_mode: ar[0xa8],
                base_posture: i(&ar, 0x198)?,
                pickup: i(&ar, 0x1a4)?,
                carry: sims_vm::animation::CarrySelector {
                    mode: i(&ar, 0x1a8)?,
                    selection: i(&ar, 0x1ac)?,
                },
                mode: i(&ar, 0x190)?,
                rate: floats::<1>(&ar, 0x228)?[0],
                target_rate: floats::<1>(&ar, 0x22c)?[0],
            },
            movement,
            settings: layer_settings,
            movement_anchors,
            geometry_generation: s.instance.geometry.generation,
            animation_events: i(&ar, 0xa4)?,
            engaged: i(&ar, 0xa0)?,
        };
        let resident: Resident = exact(
            json!({"character":character,"controls":controls,"evaluated_tick":evaluated_tick,"context":context,"source":null,"animation_yield":null,"profile_dirty":ar[0xd8]!=0,"skill_profile":null,"ordinary_update":null,"person_update":null,"displayed_animation":[],"native_walk":null,"native_turn":null,"physical_movement":null,"refresh_tick":null}),
        )?;
        ensure!(
            !resident.character.busy() && !resident.controls.movement.busy(),
            "completed resident unexpectedly busy"
        );
        ensure!(
            residents.insert(id, resident).is_none(),
            "duplicate completed resident"
        );
        models.insert(id, model);
        retained.insert(id, s.retained);
        mesh_assets.insert(
            id,
            MeshAssets {
                geometry: g.geometry,
                native_appearances: g.native_appearances,
                native_bindings: g.native_bindings,
            },
        );
    }
    ensure!(
        geometry_characters.is_empty(),
        "unclaimed completed model owner"
    );
    let geometry = Geometry {
        characters: mesh_assets,
        meshes: geometry.meshes,
        mesh_bmf: geometry.mesh_bmf,
        native_meshes: geometry.native_meshes,
        adapter_epoch: geometry.adapter_epoch,
    };
    Ok(Imported{residents,models,geometry,retained,demands:vec!["Immutable source skeleton/catalog provenance must be imported before newly requested layer bindings; current animated local poses are not a rest skeleton".into(),"BoundAvatar records/catalog/texture/cache owners and appearance reload owner are not replaced by an empty avatar".into(),"World, pending VM host reentry, queued services/events, object inventory and exact next native tick remain separately unqualified".into()]})
}
const CODE: &[(&str, u32, usize, &str)] = &[
    (
        "FUN_00592726",
        0x592726,
        338,
        "2ef902dfa2aeeb4f9cb599c9a5c93b20d3891c3616bae1cae39bfc1f5e831921",
    ),
    (
        "FUN_00592878",
        0x592878,
        140,
        "451fbb2281274555cf99a7dbe5072c90c126e87b9c1ea8bb18223fc0a226da22",
    ),
    (
        "FUN_00590b8b",
        0x590b8b,
        122,
        "01648ca00a604bb1bfa9b83d2fe5c4b139ed9170227c127eff97bf4bad90e2df",
    ),
    (
        "FUN_004325b5",
        0x4325b5,
        161,
        "9b854bffb26f1104d43131fee268710950a412cd6ae284ccc7827d760f85de50",
    ),
    (
        "FUN_00422632",
        0x422632,
        51,
        "ec0b370dd4dfedc46161000af4d13861aed7e870c035835147b20ae4ee444883",
    ),
    (
        "FUN_004c471b",
        0x4c471b,
        23,
        "c1d2664c883eb6a7165b3ad303228e2d363f530630fe1774888172c518feabb1",
    ),
    (
        "FUN_005912fd",
        0x5912fd,
        1218,
        "82b324bb4fb6beb966fd3c0ddf5a6ecf143eb5dee7a9bba5d8223975d606af91",
    ),
    (
        "FUN_00587d2c",
        0x587d2c,
        817,
        "856c122ad47faeb6ae648d0b03e467a7b5ee700aa0a2f2237ca1e9b7d72cf341",
    ),
    (
        "FUN_0058831d",
        0x58831d,
        1247,
        "ec8a0f9457e81c6b37b6f26e4f5a7c04d0d963a59dda8e8df46089e615e9e1bb",
    ),
];
