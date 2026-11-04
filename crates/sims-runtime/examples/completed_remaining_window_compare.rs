//! Execute the retained window suffix. Observations only check computed results.
use anyhow::{ensure, Context, Result};
use sims_runtime::original_session::draw_parent::{
    GeometryInput, PaintRequest, State, WorldClassification, WorldComposition, WorldScene,
};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
};
type Saved = (
    State,
    WorldScene,
    WorldComposition,
    sims_sim::Simulation,
    Buffer,
    sims_vm::interface::state::State,
    WorldClassification,
);
struct Inputs {
    iterator: Vec<u8>,
    model: Vec<u8>,
    row: Vec<u8>,
    ordinary: Vec<u8>,
    objects: Vec<u8>,
}
fn resource(s: &mut Saved, i: &Inputs) -> Result<bool> {
    //At an ordinary entry only the absent source can be pending. The
    //directory/physical records are immutable B inputs, not callee replies.
    if s.0.world_paint().and_then(|p|p.row_prefix()).is_some_and(|r|
        r.request().kind == sims_sim::lot_presenter::tile_scan::Kind::Ordinary
            && r.at_native_cut(0x4ad8ac) && r.pending().is_some()) {
        s.0.bind_world_row_ordinary(&i.ordinary)?;
        s.0.bind_world_row_ordinary_objects(&i.objects)?;
        s.0.bind_world_row_ordinary_physical(&i.objects)?;
        return Ok(true);
    }
    let Some(request) = s.0.world_paint().and_then(|p| p.geometry_input_request()) else {
        return Ok(false);
    };
    match request {
        GeometryInput::Iterator => s.0.bind_world_geometry_body(&i.iterator)?,
        GeometryInput::First => s.0.bind_world_geometry_child(&i.model)?,
        GeometryInput::Corner => {
            s.0.bind_world_geometry_corner(&i.iterator, &i.model, &s.1)?
        }
        GeometryInput::Segments => {
            s.0.bind_world_geometry_segments_from_scene(&i.model, &s.1)?
        }
        GeometryInput::Visible => s.0.bind_world_geometry_visible(&i.model)?,
        // The existing live material directory gets the first opportunity.
        // Missing keys remain unsupported; no observed palette is installed.
        GeometryInput::Materials { .. } => return Ok(false),
    }
    Ok(true)
}
fn step(s: &mut Saved, i: &Inputs, before: &mut sims_sim::Simulation) -> Result<Option<(i16,i16,i16)>> {
    if resource(s, i)? {
        ensure!(&s.3 == before, "geometry input binding changed VM/RNG");
        return Ok(None);
    }
    let pending = s.0.world_paint().and_then(|p| p.pending());
    let visibility=s.0.world_paint().and_then(|p|p.row_prefix()).and_then(|r|r.visibility_write());
    let progress = match &pending {
        Some(PaintRequest::LevelGeometryChild { .. }) => {
            let p =
                s.0.advance_world_geometry_child_with_scene(&s.3, &s.5, &s.1)?;
            if !p {
                s.0.start_world_geometry_last_side_if_pending()?
            } else {
                true
            }
        }
        Some(PaintRequest::TileRow { request }) => {
            if s.0
                .world_paint()
                .and_then(|p| p.row_prefix())
                .is_none_or(|r| r.request() != request)
            {
                s.0.bind_world_row_prefix(&i.row)?;
                true
            } else if s
                .0
                .world_paint()
                .and_then(|p| p.row_prefix())
                .is_some_and(|r| r.complete() && !r.acknowledged())
            {
                s.0.return_world_row()?;
                true
            } else {
                s.0.advance_world_row_objects_mut(&mut s.3, &mut s.6)?
            }
        }
        _ => s.0.advance_world_paint(&mut s.1, &mut s.4)?,
    };
    ensure!(
        progress,
        "remaining window stopped at computed request {pending:?}"
    );
    if let Some((id,old,value))=visibility {
        let object=before.vm.world.objects.get_mut(&id).context("expected visibility owner absent")?;
        ensure!(object.data.get(&34)==Some(&old),"expected visibility source differs");
        object.data.insert(34,value);
    }
    ensure!(&s.3 == before, "window changed state outside its exact queued visibility store");
    Ok(visibility)
}
fn row_pc(pc: u32) -> bool {
    matches!(
        pc,
        0x4aee4c | 0x4ad8ac | 0x4ad680 | 0x4ad88f | 0x4ad6ab | 0x4ad885
            | 0x4aeb88 | 0x4aeb9a | 0x4aeb9e | 0x4aeba3 | 0x4aebc1 | 0x4aebc6 | 0x4aebcd
            | 0x4c8049 | 0x4c8388 | 0x4c845c | 0x4c8461 | 0x4b5c52 | 0x4a3813
            | 0x4c847b | 0x4b6b9f | 0x4b6dad | 0x4b71b5 | 0x4c848a
    )
}
fn selected(pc: u32) -> bool {
    row_pc(pc)
        || matches!(
            pc,
            0x4ad2a1
                | 0x4ad2af
                | 0x54045b
                | 0x53db67
                | 0x540480
                | 0x53c227
                | 0x5404a7
                | 0x4ad2d9
                | 0x4ad2df
                | 0x4ad304
                | 0x4ad309
                | 0x4ad30c
                | 0x4ad345
                | 0x4ad34f
                | 0x4ad363
                | 0x4ad368
                | 0x4ad4bd
                | 0x4ad29c
                | 0x4ad369
                | 0x4ad36e
                | 0x4ad372
                | 0x4ad374
                | 0x4ad378
                | 0x4ad1b8
                | 0x4ad1bb
                | 0x4ad1c1
                | 0x4ad1c3
                | 0x4ad1c7
                | 0x4ad1cd
                | 0x4ad1d1
                | 0x4ad1d3
                | 0x4ad1d6
                | 0x4ad1da
                | 0x4ad1de
                | 0x4ad1e2
                | 0x4ad1e6
                | 0x4ad1e9
                | 0x4ad1ed
                | 0x4ad1f1
                | 0x4ad1f5
                | 0x4ad1f8
                | 0x4ad1fd
                | 0x4ad200
                | 0x4ad205
                | 0x4ad239
                | 0x4ad23c
                | 0x4ad243
                | 0x4ad244
                | 0x4ad245
                | 0x4ad246
                | 0x4ad247
                | 0x4ad286
                | 0x4ad289
                | 0x4ad28c
                | 0x4ad28e
                | 0x4ad291
                | 0x4ad293
                | 0x4ad294
                | 0x4ad295
                | 0x4ad296
                | 0x4ad297
                | 0x4ad386
                | 0x4ad38a
                | 0x4ad38d
                | 0x4ad391
                | 0x4ad393
                | 0x4ad396
                | 0x4ad398
                | 0x4ad39d
                | 0x4ad3a1
                | 0x4ad47b
                | 0x4a92a2
        )
}
fn ready(s: &Saved, q: &serde_json::Value) -> Result<bool> {
    if q.get("surface_end").is_some(){return Ok(s.0.world_paint().is_some_and(|p|p.at_surface_end_cut(q)));}
    let pc = q["pc"].as_u64().context("remaining PC absent")? as u32;
    if row_pc(pc) {
        return match s.0.world_paint().and_then(|p|p.row_prefix()) {
            Some(r) => r.at_native_observation(q),
            None => Ok(false),
        };
    }
    Ok(s.0
        .world_paint()
        .context("remaining paint absent")?
        .at_tile_cut(q))
}
fn compare(s: &Saved, q: &serde_json::Value) -> Result<()> {
    let pc = q["pc"].as_u64().context("comparison PC absent")? as u32;
    if q.get("surface_end").is_some(){s.0.compare_world_surface_end(q)?;}
    else if row_pc(pc) { s.0.compare_world_row_prefix(q)?; }
    else { s.0.compare_world_tiles(q)?; }
    s.0.compare_world_shared_graphics(q)
        ?;
    if let Some(v)=q.get("visibility_word") {
        let id: i16=v["id"].as_i64().context("native visibility ID absent")?.try_into()?;
        let word: i16=v["word"].as_i64().context("native visibility word absent")?.try_into()?;
        ensure!(s.3.vm.world.objects.get(&id).and_then(|o|o.data.get(&34))==Some(&word),
            "current visibility word differs from original for object {id}");
    }
    Ok(())
}
fn cold(out: &sims_fixtures::Store, s: &Saved, inputs: &Inputs,
    target: &serde_json::Value,
    label: &str) -> Result<serde_json::Value> {
    let blob = out.put_value(s)?;
    let mut restored: Saved = out.get_value(&blob)?;
    let mut warm = s.clone();
    let mut warm_vm=s.3.clone();
    let mut restored_vm=s.3.clone();
    ensure!(warm == restored, "complete window cold state differs");
    let mut steps=0;
    while !ready(&warm,target)? && steps<262144 {
        step(&mut warm,inputs,&mut warm_vm)?;
        step(&mut restored,inputs,&mut restored_vm)?;
        ensure!(warm == restored,"complete window cold continuation differs at {label}, step {steps}");
        steps+=1;
    }
    ensure!(ready(&warm,target)? && ready(&restored,target)?,"cold window target not reached");
    compare(&warm,target)?; compare(&restored,target)?;
    Ok(serde_json::json!({"label":label,"blob":blob,"continuation_steps":steps,
        "target_pc":target["pc"],"target_stop":target["trace_stop"],
        "complete_state_equal_after_each_step":true,"native_target_compared":true}))
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let input:sims_fixtures::BlobRef=serde_json::from_str(&std::env::var("SIMS_DIAG_INPUT_REF")
        .unwrap_or_else(|_|"{\"sha256\":\"fcf045abec40a9832568a1459ce16121bfe3de384c8c9f7ab3d40e02268ece11\",\"bytes\":17319265}".into()))?;
    let store_in = std::env::var_os("SIMS_DIAG_INPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.join("fixtures/private/goth-single-gradient-20261102-first/full-base.store")
        });
    let mut s: Saved = sims_fixtures::Store::open(store_in)?.get_value(&input)?;
    let diagonal = std::fs::read(root.join("fixtures/private/goth-remaining-window-20261102-first/diagonal-source-second.json"))?;
    s.0.bind_world_geometry_diagonal_columns(&diagonal)?;
    s.0.bind_world_override_catalog(
        &std::fs::read(root.join("fixtures/private/goth-object-visibility-20261101-first/override-family-source.json"))?,
        &std::fs::read(root.join("fixtures/private/goth-ordinary-object-20261101-first/object-source.json"))?,
        &s.3)?;
    s.0.bind_world_surface_end(
        &std::fs::read(root.join("fixtures/private/goth-world-surfaces-20261031-first/surface-source-first.json"))?,
        &std::fs::read(root.join("fixtures/private/goth-window-end-witness-20261102-first/post-level-trace.jsonl.gz"))?)?;
    let first_stop: u64 = std::env::var("SIMS_DIAG_FIRST_STOP").unwrap_or_else(|_| "0".into()).parse()?;
    let inputs=Inputs{iterator:std::fs::read(root.join("fixtures/private/goth-level-geometry-20261101-first/geometry-source-first.json"))?,
        model:std::fs::read(root.join("fixtures/private/goth-geometry-first-vector-20261101-first/first-vector-source-first.json"))?,
        row:std::fs::read(root.join("fixtures/private/goth-row-context-20261031-first/row-source-first.json"))?,
        ordinary:std::fs::read(root.join("fixtures/private/goth-ordinary-row-20261101-first/ordinary-source.json"))?,
        objects:std::fs::read(root.join("fixtures/private/goth-ordinary-object-20261101-first/object-source.json"))?};
    let witness=std::env::var_os("SIMS_DIAG_WITNESS").map(PathBuf::from).unwrap_or_else(||root.join("fixtures/private/goth-remaining-window-witness-20261102-third/post-level-trace.jsonl.gz"));
    let encoded = std::fs::read(witness)?;
    let gradient_oracle = std::env::var_os("SIMS_DIAG_GRADIENT_ORACLE")
        .map(std::fs::read).transpose()?.map(|b| -> Result<_> {
            Ok((sims_fixtures::digest(&b),serde_json::from_slice::<serde_json::Value>(&b)?))
        }).transpose()?;
    let mut rows = vec![];
    let mut excluded = vec![];
    let mut entry: Option<serde_json::Value> = None;
    for l in BufReader::new(flate2::read::GzDecoder::new(encoded.as_slice())).lines() {
        let mut q: serde_json::Value = serde_json::from_str(&l?)?;
        if q["observer"] != "remaining_window" {
            continue;
        }
        let pc = q["pc"].as_u64().context("remaining PC absent")? as u32;
        if matches!(pc, 0x53db67 | 0x53c227) {
            ensure!(entry.is_none(), "nested geometry entry differs");
            entry = Some(q.clone());
        }
        if matches!(pc, 0x540480 | 0x5404a7) {
            let e = entry
                .take()
                .context("geometry return lacks captured entry")?;
            ensure!(
                e["level_geometry_child"]["arguments"][0] == pc
                    && e["tick"] == q["tick"]
                    && e["registers"]["ebp"] == q["registers"]["ebp"]
                    && e["registers"]["esp"].as_u64().context("entry ESP absent")? + 16
                        == q["registers"]["esp"]
                            .as_u64()
                            .context("return ESP absent")?,
                "native geometry natural return ABI differs"
            );
            // Comparison-only lifetime metadata derived from actual entry/return
            // registers. No deallocated child bytes or service reply is imported.
            q["geometry_body_event"] = serde_json::json!({"deallocated_child_locals":true,"source":"captured entry/return PC, ESP and EBP proof"});
        }
        let diagonal_cut = q.get("diagonal_geometry").is_some()
            && matches!(pc,0x4c79ca|0x4c7ae9|0x4c4907|0x4c7f7a);
        if diagonal_cut {
            q["visible_geometry"] = q["diagonal_geometry"].clone();
            if let Some(b) = q["visible_geometry"].get("buffers").cloned() { q["buffers"] = b; }
        }
        if q.get("visibility_word").is_some() {
            ensure!(q["registers"]["ebp"]==q["ordinary_frame"]["bp"],
                "native visibility cut escaped the live row frame");
            q["kind"]="window_tile_scan".into();
            q["ordinary_frame_active"]=true.into();
        }
        if q.get("gradient").is_some() { q["kind"]="window_tile_scan".into(); }
        if (selected(pc) || diagonal_cut || q.get("surface_end").is_some()) && q["trace_stop"].as_u64().context("trace stop absent")? >= first_stop {
            rows.push(q)
        } else {
            excluded.push(serde_json::json!({"pc":pc,"stop":q["trace_stop"]}));
        }
    }
    ensure!(!rows.is_empty(), "remaining window evidence absent");
    let out = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("remaining output store absent")?,
    )?;
    let before = s.3.clone();
    let mut expected_vm=before.clone();
    let (mut steps, mut current) = (0, 0);
    let mut checks = vec![];
    let mut cold_cuts=vec![];
    let mut visibility_writes=vec![];
    let mut visibility_checks=vec![];
    let mut ordered_gradient_stores=0;
    let do_cold=std::env::var_os("SIMS_DIAG_COLD").is_some();
    let result = (|| -> Result<()> {
        for (index,q) in rows.iter().enumerate() {
            current = q["pc"].as_u64().unwrap() as u32;
            for local_step in 0..262144 {
                if row_pc(current)
                    && matches!(current, 0x4aee4c | 0x4ad8ac)
                    && s.0.world_paint().context("paint absent")?.at_tile_cut(q)
                    && s.0.world_paint().and_then(|p|p.row_prefix()).is_none_or(|r|r.acknowledged())
                {
                    s.0.bind_world_row_prefix(&inputs.row)?;
                }
                if ready(&s, q)? {
                    break;
                }
                let visibility=step(&mut s, &inputs, &mut expected_vm).with_context(|| {
                    format!("advancing to {current:08x}, stop {}", q["trace_stop"])
                })?;
                if let Some(effect)=visibility {visibility_writes.push(serde_json::json!({"effect":effect,"before_native_stop":q["trace_stop"],"microstep":steps}));}
                steps += 1;
                if do_cold && matches!(current,0x4c7f7a|0x4c848a) && matches!(local_step,2|3|17) {
                    cold_cuts.push(cold(&out,&s,&inputs,q,
                        &format!("diagonal drawing before stop {}, microstep {local_step}",q["trace_stop"]))?);
                }
            }
            compare(&s,q)?;
            if current==0x4c848a {
                if let Some((_,oracle))=&gradient_oracle {
                    let cursor=serde_json::to_value(s.0.world_paint().and_then(|p|p.row_prefix()).context("gradient row absent")?)?;
                    let writes=&cursor["floor_cursor"]["diagonal"]["draw"]["gradient"]["writes"];
                    ensure!(writes==&oracle["events"],"ordered diagonal floor pixel stores differ from original instructions");
                    ordered_gradient_stores+=writes.as_array().context("diagonal gradient writes absent")?.len();
                }
            }
            if let Some(v)=q.get("visibility_word") {visibility_checks.push(serde_json::json!({"pc":current,"stop":q["trace_stop"],"object":v["id"],"word":v["word"]}));}
            if do_cold && (q.get("diagonal_geometry").is_some() || q.get("gradient").is_some() || q.get("surface_end").is_some()&&matches!(current,0x4b0298|0x4b02a0|0x5ded5a|0x604f7f|0x604f85|0x604fb6|0x604fbd|0x5ded61|0x5ded65|0x5ded6b|0x5ded6e|0x4b02e2|0x4b02e5|0x4b02ec|0x4b02f2|0x4b029d)) {
                if let Some(target)=rows.get(index+1) {
                    cold_cuts.push(cold(&out,&s,&inputs,target,
                        &format!("rendering native stop {}",q["trace_stop"]))?);
                }
            }
            checks.push(serde_json::json!({"pc":current,"stop":q["trace_stop"],"region":q["region_index"],"phase":q["phase"]}));
            println!(
                "{}",
                serde_json::json!({"checked":checks.len(),"pc":current,"stop":q["trace_stop"]})
            );
        }
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"first_stop":first_stop,"diagonal_B_input_sha256":sims_fixtures::digest(&diagonal),"frontier":out.put_value(&s)?,
        "steps":steps,"checks":checks,"cold_cuts":cold_cuts,"surface_end_inputs_consumed":s.0.world_paint().and_then(|p|p.surface_end_inputs_consumed()),"unrepresented_native_instruction_cuts":excluded,
        "first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"pc":current,"message":format!("{e:#}"),"computed_request":format!("{:?}",s.0.world_paint().and_then(|p|p.pending()))})),
        "VM_and_RNG_unchanged":s.3==before,"VM_matches_baseline_plus_exact_queued_visibility_stores":s.3==expected_vm,
        "ordered_visibility_writes":visibility_writes,"native_visibility_word_comparisons":visibility_checks,
        "native_trace_sha256":sims_fixtures::digest(&encoded),
        "ordered_diagonal_gradient_stores_compared":ordered_gradient_stores,
        "gradient_store_oracle_sha256":gradient_oracle.as_ref().map(|(h,_)|h),
        "resource_queries_are_read_only":true,"observed_values_used_as_replies":false,
        "source_admission":false,"additional_completed_fresh_ticks":0});
    std::fs::File::create(
        std::env::var_os("SIMS_DIAG_REPORT").context("remaining report absent")?,
    )?
    .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"checks":checks.len(),"first_unavailable":report["first_unavailable"]})
    );
    result
}
