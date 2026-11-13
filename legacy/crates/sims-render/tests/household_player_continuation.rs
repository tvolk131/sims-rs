//! Opt-in real-owner continuation, not original-engine parity.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::json;
use sims_content::Index;
use sims_runtime::{
    exploration::{self, Driver, Limits, Observation},
    original_session::{clock, move_tool},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
#[path = "support/player_action_driver.rs"]
mod support;
use support::{Boundary, Command, Household, Identity, classify};
#[path = "support/household_cases.rs"]
mod household_cases;

#[test]
#[ignore = "fresh source-only Newbie/Goth migration and small CPU warm/cold first boundary"]
fn fresh_userdata_household_opening_and_first_boundary() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(household_cases::fresh_probe)?
        .join()
        .expect("fresh UserData worker panicked")
}

#[test]
#[ignore = "finite pre-inspected original dialog inputs from a retained Newbie/Goth player checkpoint"]
fn retained_userdata_dialog_inputs_and_small_probe() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(household_cases::continue_probe)?
        .join()
        .expect("finite UserData worker panicked")
}

#[test]
#[ignore = "read-only pinned Newbie/Goth tutorial control request, source and continuation inspection"]
fn inspect_retained_userdata_control_effect() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(household_cases::inspect_effect_probe)?
        .join()
        .expect("effect inspection worker panicked")
}

#[test]
fn bound_control_host_rejects_missing_targets() -> Result<()> {
    support::require_bound_control(&[2011, 2013], 2013)?;
    ensure!(
        support::require_bound_control(&[], 2013).is_err(),
        "empty control host accepted a target"
    );
    ensure!(
        support::require_bound_control(&[2011, 2012], 2013).is_err(),
        "unbound control accepted a target"
    );
    Ok(())
}

#[test]
#[ignore = "separate conditional original polling input at a safe retained Goth boundary; no original parity"]
fn bind_observed_goth_input_without_clock_or_rng_changes() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(household_cases::supplied_goth_input)?
        .join()
        .expect("Goth input worker panicked")
}

#[test]
#[ignore = "discover resident/chair lifetimes and actually offered Sit; real queue and eight CPU warm/cold ticks"]
fn retained_userdata_offered_interaction_and_small_probe() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(household_cases::action_probe)?
        .join()
        .expect("household action worker panicked")
}

#[test]
#[ignore = "actual Client/Device UI-wait polling at retained Newbie/Goth frame boundary; no invented activation"]
fn retained_userdata_actual_ui_wait_host_pass() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(household_cases::ui_wait_probe)?
        .join()
        .expect("UI-wait host worker panicked")
}

#[test]
#[ignore = "pinned retained Dialog opcode constructor; actual Client begin/advance and independent cuts, no response or tick"]
fn retained_userdata_actual_dialog_construction_host_pass() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(household_cases::dialog_construction_probe)?
        .join()
        .expect("dialog-construction host worker panicked")
}

const SOURCE: &str = "cache/gameplay-gaps-reproducers-20261023/phone/initial.json";
const SOURCE_SHA: &str = "b10a6eebb1f395f4ec2786cd65276f23b476b6a03c208aa38a1150df7c23598f";
const INDEX: &str = "cache/managed-independent-20261022-third/immutable-assets";
const REQUIRED: u64 = 4096;
const PORTAL_STOP_SHA: &str = "e59d4f8d5585f387cb4aaf609be521719e6c1ec079289b8c27545257121a4b0c";
const FRESH_MODAL_SHA: &str = "510834f15e76708f8f6e41d0cc073323e6412763bf7861d08a52967bfb0f732e";

#[test]
#[ignore = "read-only exact retained fixture frontend ownership diagnostic"]
fn inspect_retained_player_frontend_boundary() -> Result<()> {
    std::thread::Builder::new().stack_size(64*1024*1024).spawn(|| -> Result<()> {
        let root=repo()?;
        let store=sims_fixtures::Store::open(root.join(std::env::var("SIMS_PLAYER_DIAGNOSTIC_STORE")?))?;
        let case=store.load_case(&std::env::var("SIMS_PLAYER_DIAGNOSTIC_CASE")?)?;
        ensure!(case.source.blob.sha256==std::env::var("SIMS_PLAYER_DIAGNOSTIC_SHA256")?,"diagnostic fixture changed");
        let a=Household::from_fixture(index(&root)?,&store,&case.source.blob)?;
        let w=a.device.panel_widgets().context("widgets absent")?;
        let catalog=w.catalog().context("catalog absent")?;
        let fields=serde_json::to_value(w)?;
        let out=root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
        ensure!(out.starts_with(root.join("cache")),"private output required");
        std::fs::create_dir(&out).context("refusing to overwrite frontend diagnosis")?;
        let object_ids:Vec<i16>=serde_json::from_str(&std::env::var("SIMS_PLAYER_DIAGNOSTIC_OBJECTS").unwrap_or_else(|_|"[]".into()))?;
        ensure!(object_ids.len()<=4,"object diagnosis must remain bounded");
        let object_details=object_ids.iter().map(|id|{
            let sim=&a.session.simulation;
            json!({"object":id,"lifetime":Identity::capture(&a.session,*id).ok(),"instance":sim.vm.world.objects.get(id),"position":sim.vm.world.positions.get(id),"body":sim.bodies.get(id),
                "accepted_presentation_routes":sim.routes.iter().filter(|(_,r)|r.caller==*id).collect::<Vec<_>>(),
                "native_path":sim.native_lot.as_ref().and_then(|n|n.route_paths.get(id)),
                "resident":sim.characters.get(id).map(|r|json!({"evaluated_tick":r.evaluated_tick,"controls":r.controls,"character_clock":r.character.clock,"placement":r.character.placement,"animation_yield":r.animation_yield,"ordinary_update":r.ordinary_update,"person_update":r.person_update,"native_walk":r.native_walk,"physical_movement":r.physical_movement,"displayed_animation":r.displayed_animation})),
                "threads":sim.vm.threads.iter().filter(|(_,t)|t.stack.iter().any(|f|f.caller==*id)).map(|(id,t)|json!({"thread":id,"stack":t.stack,"status":t.status,"service":t.service,"dispatch":t.dispatch,"autonomy":t.autonomy})).collect::<Vec<_>>()})
        }).collect::<Vec<_>>();
        write(&out,"source-denial.json",&owned_resolver_receipt(&root,&a)?)?;
        let body_profiles=serde_json::to_value(&a.session.simulation.vm.world.body_profiles)?;
        write(&out,"diagnostic.json",&json!({"source":case.source.blob,"state":a.observe()?,"mode":a.mode()?,
            "requested_actual_objects":object_details,
            "household_clock":a.session.simulation.household.as_ref().map(|h|&h.clock),
            "native_scheduler":a.session.simulation.native_scheduler,
            "route_control":a.session.simulation.route_control,
            "route_feedback":a.session.simulation.route_feedback,
            "body_profile_banks_sha256":exploration::digest_serialized(&body_profiles["banks"])? ,
            "body_profile_definitions_sha256":exploration::digest_serialized(&body_profiles["definitions"])? ,
            "resource_storage_sha256":exploration::digest_serialized(&a.session.simulation.vm.world.resource_storage)? ,
            "changed_resources_sha256":exploration::digest_serialized(&a.session.simulation.vm.world.changed_resources)? ,
            "native_pending":a.session.simulation.native_lot.as_ref().map(|n|json!({"routing_policy":n.routing_policy,"route":n.route.as_ref().map(|r|json!({"caller":r.caller,"thread":r.thread,"request":r.request,"target":r.target,"goal":r.goal,"failed":r.failed})),"movement":n.movement,"completion":n.completion,"shoo":n.shoo.is_some()})),
            "presentation":w.presentation(),"active_panel":w.presentation().and_then(|p|p.active()),
            "buy_visible":w.presentation().is_some_and(|p|p.visible(sims_render::camera_device::panels::Panel::Buy)),
            "catalog_revision":catalog.revision(),"catalog_view":catalog.view(),
            "category_accepts":(0..8).map(|i|catalog.accepts(catalog.revision(),sims_vm::interface::buy::input::Event::Category(i))).collect::<Vec<_>>(),
            "widget_fields":fields.get("fields"),"widget_call":fields.get("call"),"panel_fields":fields.get("panels"),"owners":fields.get("owners"),
            "transport":a.session.host.presentation.transport,"publication":a.session.host.presentation.publication,"tool_selection":a.session.host.presentation.tool_selection,
            "interface_pending":a.session.interface_update_pending(),"interface_update":a.session.host.presentation.interface_update,
            "native_parity_ticks":0,"inputs_submitted":0}))?;
        Ok(())
    })?.join().expect("frontend diagnostic worker panicked")
}

#[test]
#[ignore = "exact collapsed Buy stop; actual ExpandPanel services and managed recovery without gameplay ticks"]
fn collapsed_buy_stop_requires_explicit_expansion_before_category() -> Result<()> {
    std::thread::Builder::new().stack_size(64*1024*1024).spawn(|| -> Result<()> {
        let root=repo()?;
        let input=sims_fixtures::Store::open(root.join("fixtures/private/player-continuation-fresh-mashuga-20261026-first"))?;
        let source=input.load_case("first-stop")?.source.blob;
        ensure!(source.sha256=="d111fe07de3ee90c00c619d6e086a8e741a6328b9f198543ac06b3f789365093","collapsed stop changed");
        let out=root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
        ensure!(out.starts_with(root.join("cache")),"private output required");
        std::fs::create_dir(&out).context("refusing to overwrite collapsed recovery")?;
        let path=root.join(std::env::var("SIMS_PLAYER_NOTICE_STORE")?);
        ensure!(path.starts_with(root.join("fixtures/private")),"private new recovery store required");
        let store=sims_fixtures::Store::create(path)?;
        let mut a=Household::from_fixture(index(&root)?,&input,&source)?;
        let mut b=a.fork()?;
        let before=a.observe()?;
        let (revision,category)={
            let w=a.device.panel_widgets().context("widgets absent")?;
            let p=w.presentation().context("presentation absent")?;
            ensure!(p.collapsed()==Some(true) && !p.visible(sims_render::camera_device::panels::Panel::Buy),"not captured collapsed Buy parent");
            let c=w.catalog().context("catalog absent")?;
            let v=c.view().context("view absent")?;
            let category=v.labels.iter().position(|s|s=="Seating").context("Seating absent")? as u8;
            ensure!(c.accepts(c.revision(),sims_vm::interface::buy::input::Event::Category(category)),"native catalog rejects category independently");
            (c.revision(),category)
        };
        ensure!(!a.device.begin_buy_navigation(&mut a.session,revision,sims_vm::interface::buy::input::Event::Category(category))?,"collapsed parent accepted input");
        ensure!(a.observe()?==before,"legitimate rejected input changed state/RNG");
        let mut journal=vec![];
        let mut checkpoints=vec![];
        let interface_pending=a.session.interface_update_pending();
        let result=(||->Result<()> {
            checkpoints.push(fixture_cut(&root,&store,&a,&mut b,"collapsed-rejection-preserved")?);
            inputs(&out,&mut a,&mut b,vec![Command::ExpandPanel{value:0}],&mut journal)?;
            checkpoints.push(fixture_cut(&root,&store,&a,&mut b,"expansion-pending")?);
            inputs(&out,&mut a,&mut b,vec![Command::Drain{budget:256}],&mut journal)?;
            ensure!(a.device.panel_widgets().and_then(|w|w.presentation()).is_some_and(|p|p.visible(sims_render::camera_device::panels::Panel::Buy) && p.collapsed()==Some(false)),"actual expansion did not publish Buy visibility");
            checkpoints.push(fixture_cut(&root,&store,&a,&mut b,"expanded-buy-parent")?);
            inputs(&out,&mut a,&mut b,vec![Command::BuyCategory{label:"Seating".into()}],&mut journal)?;
            checkpoints.push(fixture_cut(&root,&store,&a,&mut b,"category-pending")?);
            inputs(&out,&mut a,&mut b,vec![Command::Drain{budget:256}],&mut journal)?;
            let view=a.device.panel_widgets().and_then(|w|w.catalog()).and_then(|c|c.view()).context("category unpublished")?;
            ensure!(view.subcatalog!=0 && !view.products.is_empty(),"actual category did not publish products");
            checkpoints.push(fixture_cut(&root,&store,&a,&mut b,"category-complete")?);
            let after=same(&a,&b)?;
            ensure!(after.tick==before.tick && after.rng==before.rng,"UI recovery changed gameplay time/RNG");
            Ok(())
        })();
        write(&out,"report.json",&json!({"source":source,"outcome":if result.is_ok(){"passed"}else{"stopped"},"initial_interface_pending":interface_pending,"state":a.observe()?,"gameplay_ticks":a.session.simulation.vm.tick-before.tick,"native_parity_ticks":0,"legitimate_collapsed_rejection_preserved":true,"checkpoints":checkpoints,"inputs":journal,"store_verification":store.verify_all()?,"stop":result.as_ref().err().map(|e|format!("{e:#}"))}))?;
        result
    })?.join().expect("collapsed recovery worker panicked")
}

#[test]
#[ignore = "separate conditional supplied original polling input; safe retained post-notice boundary only"]
fn matching_polling_profile_binds_only_order_and_provenance() -> Result<()> {
    std::thread::Builder::new().stack_size(64*1024*1024).spawn(|| -> Result<()> {
        use sims_fixtures::{Artifact,ArtifactFormat,CaseManifest,Compression,Coverage,HashVerification,Origin,ProvenanceSource,Reference,ReferenceKind};
        let root=repo()?;
        let input=sims_fixtures::Store::open(root.join("fixtures/private/player-first-run-notices-20261026-first"))?;
        let source=input.load_case("post-notices-eight-ticks")?.source.blob;
        ensure!(source.sha256=="9a14534d6c925960e54863ba8dce90297b6d359b25921a40df39a60c79e7fa24","safe source changed");
        let summary_path="cache/research/sims-exe/parity-investigation-20261020/results/gameplay-mashuga-neighborhood-summary-final.json";
        let log_path="cache/research/sims-exe/parity-investigation-20261020/mashuga-house9-live.jsonl";
        let summary=std::fs::read(root.join(summary_path))?;
        let log=std::fs::read(root.join(log_path))?;
        let summary_sha="f12dee14f2f7bf9de4555735e89f6ca0bc003081471ce79ed3fa072df4bd8205";
        let log_sha="5a381e3dd2cb77df13fab8b9f67d04062b7d9c37ea30092c74149fd8628b777d";
        ensure!(exploration::digest(&summary)==summary_sha && exploration::digest(&log)==log_sha,"polling source identity changed");
        let out=root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
        ensure!(out.starts_with(root.join("cache")),"private output required");
        std::fs::create_dir(&out).context("refusing to overwrite supplied-input declaration")?;
        let path=root.join(std::env::var("SIMS_PLAYER_NOTICE_STORE")?);
        ensure!(path.starts_with(root.join("fixtures/private")),"new private supplied-input store required");
        let store=sims_fixtures::Store::create(path)?;
        let mut a=Household::from_fixture(index(&root)?,&input,&source)?;
        let mut b=a.fork()?;
        let house_sha="14f5eec626457c8caa7ff3e58e4a86565ecd5a232721c0ddc0a06185a9fad201";
        let neighborhood_sha="64382c634cb7936a186fb9cf45ba1ec613ef4dfa52013a46697222bddd0f950c";
        ensure!(a.session.house.source=="UserData2/Houses/House09.iff"
            && a.session.house.sources.get("UserData2/Houses/House09.iff").map(String::as_str)==Some(house_sha)
            && a.session.house.sources.get("UserData2/Neighborhood.iff").map(String::as_str)==Some(neighborhood_sha),"fresh source/hash pair differs from prior explicit binding");
        ensure!(a.session.host.presentation.neighborhood_input.is_none() && !a.session.clock_pending()
            && a.session.simulation.vm.world.person.as_ref().is_some_and(|p|p.nearby_houses.is_none()),"not the safe unknown-input boundary");
        let captured=sims_runtime::original_session::neighborhood_input::Captured::from_polling(&a.session.house,&summary,&log,0)?;
        captured.validate_house(&a.session.house)?;
        let supplied=captured.nearby_houses()?;
        ensure!(supplied.is_empty(),"the exact original null-vector observation changed");
        let before=a.research_state()?;
        let before_state=a.observe()?;
        a.session.bind_neighborhood_capture(captured.clone())?;
        b.session.bind_neighborhood_capture(captured.clone())?;
        let after=a.research_state()?;
        let mut expected=before.clone();
        expected["session"]["simulation"]["state"]["vm"]["world"]["person"]["nearby_houses"]=serde_json::to_value(&supplied)?;
        expected["session"]["view"]["neighborhood_input"]=serde_json::to_value(&captured)?;
        ensure!(after==expected,"binding changed a field outside supplied order/provenance");
        let observed=same(&a,&b)?;
        ensure!(before_state.tick==observed.tick && before_state.rng==observed.rng,"binding changed gameplay time/RNG");
        let reject_before=a.observe()?;
        ensure!(a.session.bind_neighborhood_capture(captured.clone()).is_err(),"duplicate input binding accepted");
        ensure!(a.observe()?==reject_before,"rejected duplicate changed state/RNG");
        let blob=fixture_cut(&root,&store,&a,&mut b,"supplied-observed-neighborhood")?;
        let limitation="CONDITIONAL INTERNAL INPUT PROFILE: original polling proves null vectors only in sampled native context; prior imported-hash binding does not independently attest original guest source files or this full process; no tick/RNG/CRT copied; not original parity";
        let declaration=json!({"limitation":limitation,"starting_case":source,"summary_path":summary_path,"summary_sha256":summary_sha,"log_path":log_path,"log_sha256":log_sha,"sample_index":0,"captured":captured,"source_house_sha256":house_sha,"source_neighborhood_sha256":neighborhood_sha,"prior_managed_binding_sha256":"c343a2b726a4afccb8bcd8e6d5ac069d75a9c652e8bffd62be2fbb3b14425f65","before":before_state,"after":observed,"changed_fields":["session.simulation.state.vm.world.person.nearby_houses","session.view.neighborhood_input"],"all_other_fields_equal":true,"duplicate_binding_rejected_unchanged":true,"gameplay_ticks":0,"native_parity_ticks":0});
        let summary_blob=store.put_bytes(&summary,Compression::Gzip)?;
        let log_blob=store.put_bytes(&log,Compression::Gzip)?;
        let declaration_blob=store.put_value(&declaration)?;
        let artifact=Artifact::self_contained(blob.clone(),ArtifactFormat::MessagePack,1);
        let mut case=CaseManifest::minimal_import("conditional-observed-input","Authored Mashuga continuation with explicitly supplied observed neighborhood profile",artifact.clone(),vec![limitation.into()]);
        case.provenance.origin=Origin::Authored;
        case.provenance.source_locator=Some(format!("post-notices-eight-ticks:{}",source.sha256));
        case.provenance.sources=vec![ProvenanceSource{label:"original polling log".into(),sha256:log_sha.into(),verification:HashVerification::Provided},ProvenanceSource{label:"original scoped summary".into(),sha256:summary_sha.into(),verification:HashVerification::Provided},ProvenanceSource{label:"current imported house".into(),sha256:house_sha.into(),verification:HashVerification::Provided},ProvenanceSource{label:"current imported neighborhood".into(),sha256:neighborhood_sha.into(),verification:HashVerification::Provided}];
        case.coverage=Coverage::Component{scope:"conditional supplied-input binding and independent managed player restore".into(),limitations:vec![limitation.into()]};
        case.artifacts=vec![Artifact::self_contained(summary_blob,ArtifactFormat::Json,1),Artifact::self_contained(log_blob,ArtifactFormat::Other("original polling JSONL/schema5".into()),5),Artifact::self_contained(declaration_blob,ArtifactFormat::MessagePack,1)];
        case.references=vec![Reference{label:"conditional supplied input ready".into(),kind:ReferenceKind::Resumable,artifact,resume_contract:Some("sims-render test Checkpoint/msgpack-v1; exact matching CC assets and conditional supplied-input provenance; independent state/RNG restore proven".into()),boundary:sims_fixtures::Boundary::Phase{name:"safe initial input supplied".into(),tick:Some(observed.tick)}}];
        store.publish_case(&case)?;
        write(&out,"declaration.json",&declaration)?;
        write(&out,"report.json",&json!({"outcome":"passed","limitation":limitation,"fixture_store":store.root(),"case":case.id,"source_blob":blob,"state":observed,"store_verification":store.verify_all()?}))?;
        Ok(())
    })?.join().expect("supplied-input worker panicked")
}

// Closed authored schedule. Hashes were inspected from the pinned source,
// including full dialog content and suspended continuations; no game strings
// or default responses are selected dynamically.
#[derive(Clone, Serialize)]
struct NoticePin {
    id: u64,
    caller: sims_vm::ObjectId,
    handle: u32,
    thread: sims_vm::ThreadId,
    source: String,
    instruction: usize,
    record_sha256: String,
    body_sha256: String,
    thread_sha256: String,
    resource_sha256: String,
    bhav_sha256: String,
}
fn notice_pins() -> [NoticePin; 5] {
    [
        NoticePin {
            id: 13,
            caller: 316,
            handle: 316,
            thread: 316,
            source: "ExpansionShared/ExpansionShared.far::HelpSystemMagic.iff".into(),
            instruction: 3,
            record_sha256: "e3d5a8996a4d925b27e2ef3a6925c3efdc89c03056307a03a571da7b2629c615"
                .into(),
            body_sha256: "8adc30cd1de26fbbbf34b0e9a2c619161c31903c1c2e2fe5c5b4bf750b798588".into(),
            thread_sha256: "14503b3e48ddc31714896c3175b50553026afd42d324a36ac990050fd9622d5c"
                .into(),
            resource_sha256: "30a6749cde478aaceb427aa9e80c185485b5620def9f2e8a254bce1091e8c5c3"
                .into(),
            bhav_sha256: "d61abfa49566ba6e3b903e65ea9b9181e19456fdc4618b5db5f483c3001110c5".into(),
        },
        NoticePin {
            id: 14,
            caller: 318,
            handle: 318,
            thread: 318,
            source: "ExpansionShared/ExpansionShared.far::HDHelpSystem.iff".into(),
            instruction: 34,
            record_sha256: "9f2691aeddb63a1232c4a30d1bf2e7e787a9a5cf54d4cafee4c8c9cceb1701a2"
                .into(),
            body_sha256: "74458af6ac718e72b3a2ab6598af7e3bea98e2d04da12a23a4c8794850231d2f".into(),
            thread_sha256: "d9903f0eab7ea3770bee064e8fe0f01147075b4ba6f297bc1f50152e95d77294"
                .into(),
            resource_sha256: "71366746ac857ded59c952fee02319000a7da31f280e9d2e71823f257c8e0bf6"
                .into(),
            bhav_sha256: "f04cb25b234043b35930e18b2eeac4dac3ecfa9efda3a6a92ca39dac4ffb1be5".into(),
        },
        NoticePin {
            id: 15,
            caller: 319,
            handle: 319,
            thread: 319,
            source: "ExpansionShared/ExpansionShared.far::NeighborhoodHelpSystem.iff".into(),
            instruction: 3,
            record_sha256: "025bdd2cc0ea7c55b2b47cc12268ec6f5f68194e8c414c20dbae1560838c5f4d"
                .into(),
            body_sha256: "018722c9a7b117b343d7815d4ce26980a01f01a1f544a691e0b1da621c0365cb".into(),
            thread_sha256: "a5a46ae58ac9d4b13becb4e4ed4dedf439d7ccf01b3e4f651dec8433a1d4bf0c"
                .into(),
            resource_sha256: "fd2878ef89bce4ebb176577dbb09c5ef7e34a1b641db67ebc38ae8fa65de4463"
                .into(),
            bhav_sha256: "7121f4945d8b84aeb02a7294cf30a2dee3324774079ae1853ee8f9d3ea237640".into(),
        },
        NoticePin {
            id: 16,
            caller: 320,
            handle: 320,
            thread: 320,
            source: "ExpansionShared/ExpansionShared.far::HelpSystemStudioTown.iff".into(),
            instruction: 3,
            record_sha256: "a6af5aa687e71a86a3e719cc1b103327742d04fa74e2f3cf0a7351c51cdd33bb"
                .into(),
            body_sha256: "2fbd7eaa7fe4848c76c02c21954e6f8c8df1f623401370a70d3daa0f2f1ce63a".into(),
            thread_sha256: "26df572fbce994f5c51d27102553f0df725437b99e7a79bfad0386cc4e88eb10"
                .into(),
            resource_sha256: "9cdb8fd6f04ce1179bb6104c218246c37bfe0afb71bcc5551be10513d236b552"
                .into(),
            bhav_sha256: "60a294a773d7c4934e3580f40bf6fc26f2dea760c1ad4a1a81220f07bc78bf5f".into(),
        },
        NoticePin {
            id: 17,
            caller: 321,
            handle: 321,
            thread: 321,
            source: "ExpansionShared/ExpansionShared.far::VacationHelpSystem.iff".into(),
            instruction: 3,
            record_sha256: "ff1f1cffdc5306ec3eb9f92783b334243a4770a681922c20606e3cd931fc4027"
                .into(),
            body_sha256: "5ef4d7566d4762c9e0f57717f8a20f650cdbbced96b82ebcf93899c863b62d4d".into(),
            thread_sha256: "86353978189b78a9036a8955317a9606d50b53595e9b7f47f9c44c78c4ed27b2"
                .into(),
            resource_sha256: "37957449e63690dd8d873cfb954ede33695d813cd49583dc6e81d49af04f72dc"
                .into(),
            bhav_sha256: "bd875d0aba2f6be4cc20f4acbc76b3e52cfdd3fd053ed1ef18b8b78a0366a755".into(),
        },
    ]
}
fn validate_notice_record(pin: &NoticePin, record: &sims_vm::dialog::Record) -> Result<()> {
    ensure!(
        record.id == pin.id
            && record.caller == pin.caller
            && record.source == pin.source
            && record.origin.source == pin.source
            && record.origin.bhav == 4119
            && record.origin.instruction == pin.instruction
            && record.status == 1
            && record.buttons.len() == 1
            && record.buttons[0].label == "OK"
            && record.buttons[0].result == 2
            && exploration::digest(record.body.as_bytes()) == pin.body_sha256
            && exploration::digest(&serde_json::to_vec(record)?) == pin.record_sha256,
        "planned notice identity/content differs: expected{}",
        pin.id
    );
    Ok(())
}
fn validate_notice(a: &Household, pin: &NoticePin) -> Result<sims_vm::dialog::Record> {
    let vm = &a.session.simulation.vm;
    ensure!(
        vm.world.dialogs.modal == Some(pin.id),
        "unexpected modal/order; expected{}",
        pin.id
    );
    let record = vm
        .world
        .dialogs
        .records
        .iter()
        .find(|r| r.id == pin.id)
        .context("planned record absent")?;
    validate_notice_record(pin, record)?;
    ensure!(
        Identity::capture(&a.session, pin.caller)?
            == Identity {
                object: pin.caller,
                handle: pin.handle
            }
            && vm.object_threads.get(&pin.caller) == Some(&pin.thread)
            && vm
                .threads
                .get(&pin.thread)
                .is_some_and(|t| serde_json::to_vec(t)
                    .is_ok_and(|v| exploration::digest(&v) == pin.thread_sha256)),
        "planned notice caller/continuation differs: {}",
        pin.id
    );
    let resource = a
        .session
        .index()
        .resources
        .iter()
        .find(|r| r.locator == pin.source)
        .context("notice original resource absent")?;
    ensure!(
        exploration::digest(&resource.read()?) == pin.resource_sha256,
        "notice original bytes changed"
    );
    let routine = &a
        .session
        .program
        .modules
        .get(&pin.source)
        .context("notice module absent")?
        .routines[&4119];
    ensure!(
        exploration::digest(&serde_json::to_vec(&routine.body)?) == pin.bhav_sha256,
        "notice BHAV body changed"
    );
    Ok(record.clone())
}

#[test]
fn finite_notice_record_gate_rejects_changed_content_identity_and_offers() -> Result<()> {
    let record: sims_vm::dialog::Record = serde_json::from_value(json!({
        "id":13,"caller":316,"source":"ExpansionShared/ExpansionShared.far::HelpSystemMagic.iff",
        "origin":{"source":"ExpansionShared/ExpansionShared.far::HelpSystemMagic.iff","bhav":4119,"instruction":3},
        "operands":[0,0,2,0,0,0,1,6],"stack_object":316,"title":"Authored tiny notice","body":"Authored protocol fixture",
        "buttons":[{"label":"OK","result":2}],"icon_object":null,"job_icon":null,"bitmap_icon":5000,"status":1
    }))?;
    let mut pin = notice_pins()[0].clone();
    pin.record_sha256 = exploration::digest(&serde_json::to_vec(&record)?);
    pin.body_sha256 = exploration::digest(record.body.as_bytes());
    validate_notice_record(&pin, &record)?;
    for case in 0..10 {
        let mut changed = record.clone();
        match case {
            0 => changed.id += 1,
            1 => changed.caller += 1,
            2 => changed.source.push_str("changed"),
            3 => changed.origin.bhav += 1,
            4 => changed.origin.instruction += 1,
            5 => changed.body.push('!'),
            6 => changed.title.push('!'),
            7 => changed.buttons[0].label = "Continue".into(),
            8 => changed.buttons[0].result = 3,
            _ => changed.status = 2,
        }
        ensure!(
            validate_notice_record(&pin, &changed).is_err(),
            "changed notice case{} accepted",
            case
        );
    }
    Ok(())
}

fn fixture_cut(
    root: &Path,
    store: &sims_fixtures::Store,
    a: &Household,
    b: &mut Household,
    label: &str,
) -> Result<sims_fixtures::BlobRef> {
    use sims_fixtures::{
        Artifact, ArtifactFormat, CaseManifest, Coverage, Origin, Reference, ReferenceKind,
    };
    let before = same(a, b)?;
    let blob = a.checkpoint_fixture(store)?;
    let independent = sims_fixtures::Store::open(store.root())?;
    let restored = Household::from_fixture(index(root)?, &independent, &blob)?;
    ensure!(
        restored.observe()? == before,
        "fixture cold cut changed state/RNG: {label}"
    );
    *b = restored;
    let artifact = Artifact::self_contained(blob.clone(), ArtifactFormat::MessagePack, 1);
    let mut case=CaseManifest::minimal_import(label,format!("Authored player checkpoint {label}"),artifact.clone(),vec!["Rust Session/Device/Client state; immutable matching CC assets still required; not original-engine parity".into()]);
    case.provenance.origin = Origin::Authored;
    case.provenance.source_locator = Some(
        if a.session.house.source == "UserData2/Houses/House09.iff" {
            format!("sha256:{FRESH_MODAL_SHA}")
        } else {
            format!(
                "imported-house:{}#sha256:{}",
                a.session.house.source,
                a.session
                    .house
                    .sources
                    .get(&a.session.house.source)
                    .context("checkpoint house source hash absent")?
            )
        },
    );
    case.coverage = Coverage::Component {
        scope: "independent managed player restore".into(),
        limitations: vec![
            "authored Rust continuation, not observed original inputs or strict parity".into(),
        ],
    };
    case.references.push(Reference{label:label.into(),kind:ReferenceKind::Resumable,artifact,resume_contract:Some("sims-render test Checkpoint/msgpack-v1; matching immutable CC assets; independent Session/Device/Client restore compared".into()),boundary:sims_fixtures::Boundary::Phase{name:label.into(),tick:Some(before.tick)}});
    store.publish_case(&case)?;
    Ok(blob)
}

#[test]
#[ignore = "exact five authored first-run responses, streamed managed cuts and eight real completed ticks"]
fn five_pinned_notices_then_eight_completed_ticks_keep_cold_state() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(authored_notice_probe)?
        .join()
        .expect("authored notice worker panicked")
}
fn authored_notice_probe() -> Result<()> {
    let root = repo()?;
    let source = root.join(std::env::var("SIMS_FRESH_MODAL_CHECKPOINT")?);
    let raw = std::fs::read(&source)?;
    ensure!(
        exploration::digest(&raw) == FRESH_MODAL_SHA,
        "notice source changed"
    );
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private output required"
    );
    std::fs::create_dir(&out).context("refusing to overwrite authored notice probe")?;
    let store_path = root.join(std::env::var("SIMS_PLAYER_NOTICE_STORE")?);
    ensure!(
        store_path.starts_with(root.join("fixtures/private")),
        "new private fixture store required"
    );
    let store = sims_fixtures::Store::create(&store_path)?;
    let mut a = Household::from_bytes(index(&root)?, &raw)?;
    let mut b = a.fork()?;
    let start = a.session.simulation.vm.tick;
    let started = Instant::now();
    let mut polls = 0;
    let mut receipts = vec![];
    let mut rows = vec![];
    let mut final_blob = None;
    let pins = notice_pins();
    let exe = std::env::current_exe()?;
    write(
        &out,
        "input.json",
        &json!({"source":source,"source_sha256":FRESH_MODAL_SHA,"pins":pins,"fixture_store":store.root(),"qualification":"finite authored Rust input schedule; not observed original input schedule or strict parity","test_executable_sha256":exploration::digest(&std::fs::read(exe)?),"test_source_sha256":exploration::digest(include_bytes!("household_player_continuation.rs")),"adapter_source_sha256":exploration::digest(include_bytes!("support/player_action_driver.rs"))}),
    )?;
    std::fs::write(
        out.join("test-source.rs"),
        include_bytes!("household_player_continuation.rs"),
    )?;
    std::fs::write(
        out.join("adapter-source.rs"),
        include_bytes!("support/player_action_driver.rs"),
    )?;
    let result = (|| -> Result<()> {
        for (n, pin) in pins.iter().enumerate() {
            let record = validate_notice(&a, pin)?;
            ensure!(
                validate_notice(&b, pin)? == record,
                "cold planned notice differs"
            );
            let before = fixture_cut(
                &root,
                &store,
                &a,
                &mut b,
                &format!("notice-{}-before", pin.id),
            )?;
            let state = a.observe()?;
            receipts.push(json!({"ordinal":n,"tick":state.tick,"record":record,"offered_result":2,"continuation_sha256":pin.thread_sha256,"before":before,"before_state":state,"complete":false}));
            write(&out, "ordered-inputs.json", &receipts)?;
            a.session
                .simulation
                .vm
                .answer_dialog(pin.id, record.buttons[0].result)
                .map_err(anyhow::Error::msg)?;
            b.session
                .simulation
                .vm
                .answer_dialog(pin.id, record.buttons[0].result)
                .map_err(anyhow::Error::msg)?;
            let after_state = same(&a, &b)?;
            ensure!(
                after_state.tick == start && after_state.rng == state.rng,
                "dialog response changed time/RNG"
            );
            ensure!(
                a.session.simulation.vm.world.dialogs.modal.is_none()
                    && a.session
                        .simulation
                        .vm
                        .world
                        .dialogs
                        .records
                        .iter()
                        .find(|r| r.id == pin.id)
                        .is_some_and(|r| r.status == 2),
                "offered response did not retain native result2"
            );
            let after = fixture_cut(
                &root,
                &store,
                &a,
                &mut b,
                &format!("notice-{}-after", pin.id),
            )?;
            let receipt = receipts.last_mut().unwrap();
            receipt["after"] = serde_json::to_value(after)?;
            receipt["after_state"] = serde_json::to_value(after_state)?;
            receipt["complete"] = json!(true);
            write(&out, "ordered-inputs.json", &receipts)?;
            if n + 1 < pins.len() {
                let x = ready_frame(&mut a, &mut polls, started, 900)?;
                let y = ready_frame(&mut b, &mut polls, started, 900)?;
                ensure!(
                    matches!(
                        (x, y),
                        (
                            Frame::Boundary(Boundary::Dialog),
                            Frame::Boundary(Boundary::Dialog)
                        )
                    ),
                    "another boundary before next planned notice"
                );
                same(&a, &b)?;
            }
        }
        while rows.len() < 8 || a.session.clock_pending() {
            let x = ready_frame(&mut a, &mut polls, started, 900)?;
            let y = ready_frame(&mut b, &mut polls, started, 900)?;
            match (x, y) {
                (Frame::Boundary(Boundary::Tick), Frame::Boundary(Boundary::Tick)) => {
                    let row = same(&a, &b)?;
                    ensure!(
                        row.tick == start + rows.len() as u64 + 1,
                        "small probe skipped completed tick"
                    );
                    rows.push(row);
                    write(&out, "ticks.json", &rows)?;
                }
                (
                    Frame::Boundary(Boundary::PartialPass),
                    Frame::Boundary(Boundary::PartialPass),
                )
                | (Frame::Complete, Frame::Complete) => {
                    same(&a, &b)?;
                }
                _ => anyhow::bail!("unplanned boundary after finite first-run notices"),
            }
            ensure!(
                rows.len() <= 16,
                "small probe cannot finish current frame within16 completed ticks"
            );
        }
        let blob = fixture_cut(&root, &store, &a, &mut b, "post-notices-eight-ticks")?;
        final_blob = Some(blob);
        Ok(())
    })();
    if result.is_err()
        && let Ok(blob) = a.checkpoint_fixture(&store)
    {
        let mut case = sims_fixtures::CaseManifest::minimal_import(
            "first-stop",
            "First authored-notice probe stop",
            sims_fixtures::Artifact::self_contained(
                blob,
                sims_fixtures::ArtifactFormat::MessagePack,
                1,
            ),
            vec!["unqualified stop, no parity claim".into()],
        );
        case.provenance.origin = sims_fixtures::Origin::Authored;
        store.publish_case(&case)?;
    }
    let verification = store.verify_all()?;
    write(
        &out,
        "report.json",
        &json!({"outcome":if result.is_ok(){"passed"}else{"stopped"},"responses_completed":receipts.iter().filter(|r|r["complete"]==true).count(),"compared_completed_ticks":rows.len(),"actual_tick_delta":a.session.simulation.vm.tick-start,"native_parity_ticks":0,"fixture_store":store.root(),"store_verification":verification,"final_source":final_blob,"polls":polls,"seconds":started.elapsed().as_secs_f64(),"stop":result.as_ref().err().map(|e|format!("{e:#}"))}),
    )?;
    result
}

#[test]
#[ignore = "publish compact authored input receipts after the completed five-notice/eight-tick probe"]
fn publish_authored_notice_schedule_and_checkpoint_closure() -> Result<()> {
    use sims_fixtures::{
        Artifact, ArtifactFormat, CaseManifest, Coverage, InputEvent, Origin, ReferenceKind,
        ScheduleStep,
    };
    let root = repo()?;
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    let store = sims_fixtures::Store::open(root.join(std::env::var("SIMS_PLAYER_NOTICE_STORE")?))?;
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("report.json"))?)?;
    let receipts: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(out.join("ordered-inputs.json"))?)?;
    let input: serde_json::Value = serde_json::from_slice(&std::fs::read(out.join("input.json"))?)?;
    ensure!(
        report["outcome"] == "passed"
            && report["responses_completed"] == 5
            && report["compared_completed_ticks"]
                .as_u64()
                .is_some_and(|n| n >= 8)
            && receipts.len() == 5
            && receipts.iter().all(|r| r["complete"] == true),
        "notice probe did not complete its explicit contract"
    );
    let labels = store.list_cases()?;
    let mut refs = vec![];
    for label in labels {
        let case = store.load_case(&label)?;
        if label.starts_with("notice-") || label == "post-notices-eight-ticks" {
            refs.extend(case.references);
        }
    }
    ensure!(
        refs.len() == 11,
        "expected ten notice cuts and final owned checkpoint"
    );
    let metadata=store.put_value(&json!({"schema":1,"qualification":"finite authored Rust schedule, never observed original inputs or strict parity","input":input,"receipts":receipts,"report":report}))?;
    let artifact = Artifact::declared(
        metadata,
        ArtifactFormat::MessagePack,
        1,
        refs.iter().map(|r| r.artifact.blob.clone()).collect(),
    );
    let mut case = CaseManifest::minimal_import(
        "authored-notice-schedule",
        "Exact five first-run notices and owned checkpoint closure",
        artifact,
        vec![
            "Original immutable assets are required; this is an authored Rust continuation fixture"
                .into(),
        ],
    );
    case.provenance.origin = Origin::Authored;
    case.provenance.source_locator = Some(format!("sha256:{FRESH_MODAL_SHA}"));
    case.coverage=Coverage::Component{scope:"finite offered-button inputs, per-notice independent owned restores, small completed-tick continuation".into(),limitations:vec!["InferredRectangles; not original input observation or tick parity".into()]};
    case.references = refs;
    for receipt in &receipts {
        case.schedule.push(ScheduleStep::WaitBoundary {
            boundary: sims_fixtures::Boundary::Event {
                name: format!("pinned modal{}", receipt["record"]["id"]),
                tick: receipt["tick"].as_u64(),
            },
        });
        case.schedule.push(ScheduleStep::Input{input:InputEvent{name:"vm.answer_dialog".into(),payload:json!({"id":receipt["record"]["id"],"result":receipt["offered_result"],"source":receipt["record"]["source"],"origin":receipt["record"]["origin"],"continuation_sha256":receipt["continuation_sha256"]})}});
    }
    case.schedule.push(ScheduleStep::WaitCompletedTicks {
        count: report["compared_completed_ticks"].as_u64().unwrap(),
    });
    store.publish_case(&case)?;
    let verified = store.verify_all()?;
    write(
        &out,
        "published-schedule.json",
        &json!({"case":case.id,"store":store.root(),"verification":verified}),
    )?;
    // Keep this bound to the typed checkpoint contract, not a generic payload
    // which merely happens to contain a state-shaped object.
    ensure!(
        case.references
            .iter()
            .all(|r| r.kind == ReferenceKind::Resumable && r.resume_contract.is_some()),
        "checkpoint restore qualification absent"
    );
    Ok(())
}

#[test]
#[ignore = "read-only inspection of the pinned first-run modal input and original assets"]
fn inspect_finite_first_run_notice_pins() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| -> Result<()> {
            let root = repo()?;
            let source = root.join(std::env::var("SIMS_FRESH_MODAL_CHECKPOINT")?);
            let raw = std::fs::read(&source)?;
            ensure!(exploration::digest(&raw) == FRESH_MODAL_SHA, "modal source changed");
            let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
            ensure!(out.starts_with(root.join("cache")), "private output required");
            std::fs::create_dir(&out).context("refusing to overwrite notice inspection")?;
            let a = Household::from_bytes(index(&root)?, &raw)?;
            let vm = &a.session.simulation.vm;
            ensure!(vm.world.dialogs.records.len()==5 && vm.world.dialogs.modal==Some(13), "not the exact five-notice boundary");
            let mut pins=vec![];
            for id in 13..=17 {
                let record=vm.world.dialogs.records.iter().find(|r|r.id==id).context("planned notice absent")?;
                let thread=vm.object_threads[&record.caller];
                let bytes=a.session.index().resources.iter().find(|r|r.locator==record.source).context("original notice resource absent")?.read()?;
                let routine=&a.session.program.modules[&record.source].routines[&record.origin.bhav];
                let mut active=record.clone();
                active.status=1;
                pins.push(json!({"id":id,"caller":Identity::capture(&a.session,record.caller)?,"thread":thread,
                    "active_record":active,"active_record_sha256":exploration::digest(&serde_json::to_vec(&active)?),
                    "body_sha256":exploration::digest(record.body.as_bytes()),
                    "thread_sha256":exploration::digest(&serde_json::to_vec(&vm.threads[&thread])?),
                    "suspended_thread":vm.threads[&thread],
                    "original_resource_sha256":exploration::digest(&bytes),
                    "bhav_body_sha256":exploration::digest(&serde_json::to_vec(&routine.body)?)}));
            }
            write(&out,"notice-pins.json",&json!({"input":source,"input_sha256":FRESH_MODAL_SHA,"qualification":"pins inspected from Rust-produced source and original immutable resources; no observed original input schedule","pins":pins,"responses_submitted":0,"gameplay_ticks":0}))?;
            Ok(())
        })?
        .join()
        .expect("notice inspection worker panicked")
}

#[test]
#[ignore = "private exact first-run modal checkpoint; preserves unanswered state"]
fn fresh_unanswered_modal_restores_independently_without_input() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| -> Result<()> {
            let root = repo()?;
            let source = root.join(std::env::var("SIMS_FRESH_MODAL_CHECKPOINT")?);
            let raw = std::fs::read(&source)?;
            ensure!(exploration::digest(&raw) == FRESH_MODAL_SHA, "modal source changed");
            let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
            ensure!(out.starts_with(root.join("cache")), "private output required");
            std::fs::create_dir(&out).context("refusing to overwrite modal restore evidence")?;
            let warm = Household::from_bytes(index(&root)?, &raw)?;
            let before = warm.observe()?;
            let protocol = serde_json::to_vec(&warm.session.simulation.vm.world.dialogs)?;
            ensure!(warm.session.simulation.vm.world.dialogs.modal == Some(13), "expected unanswered Magic Town modal absent");
            let saved = warm.checkpoint()?;
            let cold = Household::from_bytes(index(&root)?, &saved)?;
            ensure!(same(&warm, &cold)? == before, "modal restore changed state/RNG");
            ensure!(serde_json::to_vec(&cold.session.simulation.vm.world.dialogs)? == protocol, "modal restore changed exact dialog protocol");
            let exe=std::env::current_exe()?;
            write(&out, "report.json", &json!({"source":source,"source_sha256":FRESH_MODAL_SHA,"outcome":"passed","managed_cold_restores":2,"state":before,"response_submitted":false,"gameplay_ticks":0,"native_parity_ticks":0,"test_executable_sha256":exploration::digest(&std::fs::read(exe)?),"test_source_sha256":exploration::digest(include_bytes!("household_player_continuation.rs")),"adapter_source_sha256":exploration::digest(include_bytes!("support/player_action_driver.rs"))}))?;
            Ok(())
        })?
        .join()
        .expect("modal restore worker panicked")
}

#[test]
#[ignore = "private hash-bound source bundle; new original-house import, not a historical VM repair"]
fn fresh_mashuga_source_bundle_opening_boundary() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(fresh_source_probe)?
        .join()
        .expect("fresh-source worker panicked")
}
fn fresh_source_probe() -> Result<()> {
    use sims_render::{
        camera_device::Device,
        dialog::client::{Client, Inputs},
        original_house::OriginalHouse,
    };
    use sims_runtime::{
        Session,
        original_session::{Options, completion},
    };
    #[derive(serde::Deserialize)]
    struct Bundle {
        session: sims_runtime::original_session::Saved,
    }
    let root = repo()?;
    let raw = std::fs::read(root.join(SOURCE))?;
    ensure!(
        exploration::digest(&raw) == SOURCE_SHA,
        "source bundle changed"
    );
    let saved: Bundle = serde_json::from_slice(&raw)?;
    ensure!(saved.session.version == 2, "managed source bundle required");
    let content = saved
        .session
        .managed
        .context("owned source bundle absent")?;
    content.owned.validate()?;
    let options = Options {
        application_context: None,
        initial_process_profile: Some(sims_content::autonomy_resources::InitialProfile::CompleteCollection191ObservedInitial20261024),
        scheduler: sims_runtime::original_session::RegistryScheduler,
        native_load_completion: true,
        native_postload: true,
        routing: sims_sim::lot_route::path::Policy::InferredRectangles,
        crt_seed: 0x51a5cafe,
        defer_indicators: false,
    };
    let immutable = index(&root)?;
    let mounted = Arc::new(content.owned.mount(&immutable)?);
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private output required"
    );
    std::fs::create_dir(&out).context("refusing to overwrite fresh-source evidence")?;
    std::fs::write(
        out.join("test-source.rs"),
        include_bytes!("household_player_continuation.rs"),
    )?;
    std::fs::write(
        out.join("adapter-source.rs"),
        include_bytes!("support/player_action_driver.rs"),
    )?;
    let compiler = std::process::Command::new("rustc").arg("-Vv").output()?;
    let exe = std::env::current_exe()?;
    write(
        &out,
        "input.json",
        &json!({
            "scenario":"NEW source-file import; evolved VM/client/device are discarded",
            "source_bundle":SOURCE,"source_bundle_sha256":SOURCE_SHA,
            "house":saved.session.source,"owned_source_hashes":content.owned.hashes(),
            "immutable_index":INDEX,"options":options,
            "frontend_inputs":{"width":1000,"height":740,"device_clock":0,"simulation_clock":1,"completion_mode":0,"controller_suppressed":0,"selection_override":0,"fresh_play_controls":true},
            "qualification":"new authored-process inputs, not original-process reconstruction or strict original parity",
            "compiler":String::from_utf8(compiler.stdout)?,"test_executable":exe,
            "test_executable_sha256":exploration::digest(&std::fs::read(&exe)?),
            "test_source_sha256":exploration::digest(include_bytes!("household_player_continuation.rs")),
            "adapter_source_sha256":exploration::digest(include_bytes!("support/player_action_driver.rs"))
        }),
    )?;
    let mut stage = "new Session load";
    let mut loaded: Option<Session> = None;
    let result = (|| -> Result<()> {
        loaded = Some(Session::load(&mounted, &saved.session.source, options)?);
        let session = loaded.as_mut().unwrap();
        write(
            &out,
            "loaded.json",
            &json!({
                "tick":session.simulation.vm.tick,"program_fingerprint":session.program.fingerprint(),
                "module_sources":session.program.modules.keys().collect::<Vec<_>>(),
                "family_metadata":session.simulation.vm.world.person.as_ref().map(|p|&p.family_metadata),
                "import_source_hashes":session.house.sources
            }),
        )?;
        stage = "postload/load completion";
        let mut device = Device::open_for_household(
            session,
            1000,
            740,
            0,
            completion::Config {
                controller_suppressed: 0,
                mode: 0,
            },
        )?;
        stage = "new portrait Client";
        let mut client = Client::new(session)?;
        let size = session
            .simulation
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("fresh lot absent")?
            .size as i32;
        let inputs = Inputs::camera(OriginalHouse::native_camera(session)?, size)?;
        stage = "resident portrait publication";
        client.bind_resident_buttons(session, inputs)?;
        for _ in 0..10000 {
            if !client.ui_lookup_pending() {
                break;
            }
            client.advance_ui_lookup(session, 256)?;
        }
        ensure!(
            !client.ui_lookup_pending(),
            "portrait startup budget exceeded"
        );
        stage = "opening selection and Live publication";
        device.publish_opening_household(session, 0)?;
        device.publish_live_interface(session)?;
        client.bind_live_panel(session)?;
        client.follow_resident_portraits()?;
        session.bind_clock(1)?;
        let controls = &mut session
            .host
            .presentation
            .transport
            .as_mut()
            .context("fresh transport absent")?
            .controls;
        controls
            .set_speed(controls.speed)
            .map_err(anyhow::Error::msg)?;
        client.collect(session)?;
        device.refresh(session)?;
        let mut warm =
            Household::from_fresh_owners(mounted.clone(), loaded.take().unwrap(), device, client);
        stage = "fresh Move/tool/panel bindings";
        warm.bind_fresh_play_controls()?;
        stage = "independent managed play-ready restore";
        let bytes = warm.checkpoint()?;
        let cold = Household::from_bytes(index(&root)?, &bytes)?;
        same(&warm, &cold)?;
        std::fs::write(out.join("fresh-opening-managed.json"), &bytes)?;
        write(
            &out,
            "fresh-opening-state.json",
            &json!({"bytes":bytes.len(),"sha256":exploration::digest(&bytes),"state":warm.observe()?,"native_parity_ticks":0}),
        )?;
        stage =
            "opening and explicit play owners complete; gameplay and Buy actions not yet exercised";
        Ok(())
    })();
    write(
        &out,
        "report.json",
        &json!({"outcome":if result.is_ok(){"opening_complete"}else{"stopped"},"stage":stage,"stop":result.as_ref().err().map(|e|format!("{e:#}")),"completed_gameplay_ticks":0,"native_parity_ticks":0}),
    )?;
    if result.is_err()
        && let Some(session) = loaded
    {
        let bytes = serde_json::to_vec(&session.save())?;
        std::fs::write(out.join("load-stop-research.json"), &bytes)?;
        write(
            &out,
            "load-stop-state.json",
            &json!({"bytes":bytes.len(),"sha256":exploration::digest(&bytes),"resumable_player_save":false}),
        )?;
    }
    result
}

#[derive(Serialize)]
struct Cut {
    label: String,
    tick: u64,
    bytes: usize,
    sha256: String,
    state: Observation,
    format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fixture_blob: Option<sims_fixtures::BlobRef>,
}
#[derive(Serialize)]
struct Input {
    tick: u64,
    command: Command,
    complete: bool,
}
#[derive(Debug)]
enum Frame {
    Pending,
    Complete,
    Boundary(Boundary),
}

fn repo() -> Result<PathBuf> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?)
}
fn index(root: &Path) -> Result<Arc<Index>> {
    let i = Index::scan(&root.join(INDEX))?;
    ensure!(
        i.resources
            .iter()
            .all(|r| !r.locator.to_ascii_lowercase().starts_with("userdata")),
        "original save namespace exposed"
    );
    Ok(Arc::new(i))
}
fn write(out: &Path, name: &str, value: &impl Serialize) -> Result<()> {
    std::fs::write(out.join(name), serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn same(a: &Household, b: &Household) -> Result<Observation> {
    let x = a.observe()?;
    let y = b.observe()?;
    ensure!(
        x == y,
        "warm/cold full state or RNG mismatch: warm={x:?}, cold={y:?}"
    );
    Ok(x)
}
fn cut(
    root: &Path,
    out: &Path,
    a: &Household,
    b: &mut Household,
    label: &str,
    cuts: &mut Vec<Cut>,
) -> Result<()> {
    let before = same(a, b)?;
    if let Ok(path) = std::env::var("SIMS_PLAYER_CONTINUATION_STORE") {
        let store = sims_fixtures::Store::open(root.join(path))?;
        let blob = fixture_cut(root, &store, a, b, label)?;
        cuts.push(Cut {
            label: label.into(),
            tick: before.tick,
            bytes: blob.bytes as usize,
            sha256: blob.sha256.clone(),
            state: before,
            format: "MessagePack checkpoint/msgpack-v1".into(),
            fixture_blob: Some(blob),
        });
        return write(out, "cuts.json", cuts);
    }
    let bytes = a.checkpoint()?;
    // A newly scanned immutable index cannot reuse a managed/UserData namespace.
    let restored = Household::from_bytes(index(root)?, &bytes)?;
    ensure!(
        restored.observe()? == before,
        "managed cold cut changed state at {label}"
    );
    *b = restored;
    cuts.push(Cut {
        label: label.into(),
        tick: before.tick,
        bytes: bytes.len(),
        sha256: exploration::digest(&bytes),
        state: before,
        format: "JSON checkpoint".into(),
        fixture_blob: None,
    });
    write(out, "cuts.json", cuts)
}
fn inputs(
    out: &Path,
    a: &mut Household,
    b: &mut Household,
    commands: Vec<Command>,
    journal: &mut Vec<Input>,
) -> Result<()> {
    for command in commands {
        journal.push(Input {
            tick: a.session.simulation.vm.tick,
            command: command.clone(),
            complete: false,
        });
        write(out, "inputs.json", journal)?;
        eprintln!(
            "continuation input at{}: {command:?}",
            a.session.simulation.vm.tick
        );
        a.execute(&command, &Limits::default())?;
        b.execute(&command, &Limits::default())?;
        same(a, b)?;
        journal.last_mut().unwrap().complete = true;
        write(out, "inputs.json", journal)?;
    }
    Ok(())
}
fn frame(s: &mut Household) -> Result<Frame> {
    if !s.session.clock_pending() {
        if matches!(
            s.session.house.source.as_str(),
            "UserData/Houses/House05.iff" | "UserData/Houses/House07.iff"
        ) && !s.poll_application_ui()?
        {
            return Ok(Frame::Pending);
        }
        let c = s
            .session
            .host
            .presentation
            .clock
            .as_ref()
            .context("captured elapsed clock absent")?;
        ensure!(c.fields.period > 0, "captured elapsed period invalid");
        // Explicit test event-loop time. No wall-clock acquisition or game RNG injection.
        let now = c.now.wrapping_add(c.fields.period as u32);
        s.session.set_clock_time(now)?;
        s.session.begin_clock_frame()?;
    }
    let before = s.session.simulation.vm.tick;
    match s.session.advance_clock(256)? {
        clock::Progress::Pending => {
            ensure!(
                s.session.simulation.vm.tick == before,
                "clock work advanced time without completion"
            );
            Ok(Frame::Pending)
        }
        clock::Progress::PollInput => {
            ensure!(
                s.session.simulation.vm.tick == before,
                "message poll advanced time"
            );
            // The deterministic fixture event loop has no queued input while its current frame runs.
            s.session.acknowledge_clock_input(false)?;
            Ok(Frame::Pending)
        }
        clock::Progress::Complete { .. } => {
            ensure!(
                s.session.simulation.vm.tick == before,
                "frame return advanced time"
            );
            Ok(Frame::Complete)
        }
        clock::Progress::Household(progress) => {
            let boundary = classify(before, s.session.simulation.vm.tick, progress)?;
            // Same app housekeeping at Tick/PartialPass/Dialog: sweep released
            // portrait handles without answering a modal or advancing the VM.
            if matches!(
                boundary,
                Boundary::Tick | Boundary::PartialPass | Boundary::Dialog
            ) {
                s.collect_dialog_images()?;
                if boundary == Boundary::Tick {
                    s.device.refresh(&s.session)?;
                }
            }
            Ok(Frame::Boundary(boundary))
        }
    }
}
fn active(s: &Household, receipt: (i16, u32)) -> bool {
    s.session
        .simulation
        .vm
        .world
        .actions
        .as_ref()
        .and_then(|w| w.actors.get(&receipt.0))
        .and_then(|a| a.active.as_ref())
        .is_some_and(|a| a.id == receipt.1)
}
fn ready_frame(
    s: &mut Household,
    polls: &mut usize,
    started: Instant,
    max_seconds: u64,
) -> Result<Frame> {
    loop {
        ensure!(
            started.elapsed().as_secs() < max_seconds,
            "continuation wall budget reached"
        );
        ensure!(*polls < 200_000, "continuation service poll limit reached");
        *polls += 1;
        match frame(s)? {
            Frame::Pending => (),
            other => return Ok(other),
        }
    }
}

#[test]
#[ignore = "private managed Mashuga;4096 completed native-registry ticks, real input owners and cold cuts"]
fn mashuga_4096_completed_ticks_keep_player_actions_edits_and_cold_saves() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("continuation worker panicked")
}
fn run() -> Result<()> {
    run_household("UserData2/Houses/House09.iff")
}

#[test]
#[ignore = "exact retained Newbie/Goth actual queued interaction;4096 CPU completed ticks, paid Buy edit and cold cuts"]
fn userdata_4096_completed_ticks_keep_player_actions_edits_and_cold_saves() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| run_household(household_cases::source_house()?))?
        .join()
        .expect("UserData continuation worker panicked")
}

fn run_household(expected_house: &str) -> Result<()> {
    let root = repo()?;
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private output required"
    );
    std::fs::create_dir(&out).context("refusing to overwrite continuation evidence")?;
    std::fs::write(
        out.join("test-source.rs"),
        include_bytes!("household_player_continuation.rs"),
    )?;
    std::fs::write(
        out.join("adapter-source.rs"),
        include_bytes!("support/player_action_driver.rs"),
    )?;
    std::fs::write(
        out.join("helper-source.rs"),
        include_bytes!("support/household_cases.rs"),
    )?;
    let max_ticks = std::env::var("SIMS_PLAYER_CONTINUATION_MAX_TICKS")
        .ok()
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(REQUIRED);
    ensure!(
        (1..=REQUIRED).contains(&max_ticks),
        "invalid bounded continuation target"
    );
    let max_seconds = std::env::var("SIMS_PLAYER_CONTINUATION_MAX_SECONDS")
        .ok()
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(900);
    ensure!(
        (1..=7200).contains(&max_seconds),
        "invalid continuation wall budget"
    );
    let (source_name, source_sha, mut warm) =
        if let Ok(path) = std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_STORE") {
            ensure!(
                std::env::var("SIMS_PLAYER_CONTINUATION_SOURCE").is_err(),
                "choose one starting-source protocol"
            );
            let case_id = std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_CASE")?;
            let expected = std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_SHA256")?;
            let store = sims_fixtures::Store::open(root.join(&path))?;
            let case = store.load_case(&case_id)?;
            ensure!(
                case.source.format == sims_fixtures::ArtifactFormat::MessagePack
                    && case.source.schema == 1
                    && case.source.blob.sha256 == expected
                    && case
                        .references
                        .iter()
                        .any(|r| r.kind == sims_fixtures::ReferenceKind::Resumable
                            && r.artifact.blob == case.source.blob),
                "starting fixture identity/restore contract differs"
            );
            let household = Household::from_fixture(index(&root)?, &store, &case.source.blob)?;
            (format!("fixture:{path}#{case_id}"), expected, household)
        } else {
            ensure!(
                std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_CASE").is_err()
                    && std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_SHA256").is_err(),
                "fixture input requires its store"
            );
            let (name, hash) = match (
                std::env::var("SIMS_PLAYER_CONTINUATION_SOURCE").ok(),
                std::env::var("SIMS_PLAYER_CONTINUATION_SOURCE_SHA256").ok(),
            ) {
                (Some(name), Some(hash)) => (name, hash),
                (None, None) => (SOURCE.into(), SOURCE_SHA.into()),
                _ => anyhow::bail!(
                    "an alternate managed source requires both path and expected SHA256"
                ),
            };
            let raw = std::fs::read(root.join(&name))?;
            ensure!(exploration::digest(&raw) == hash, "starting source changed");
            let household = Household::from_bytes(index(&root)?, &raw)?;
            (name, hash, household)
        };
    ensure!(
        warm.session.house.source == expected_house,
        "continuation source household differs"
    );
    let mut dialog_schedule =
        household_cases::RuntimeDialogSchedule::load(&root, expected_house, &source_sha)?;
    if let Some(schedule) = &dialog_schedule {
        schedule.retain_exact_input(&root, &out)?;
    }
    let scenario = warm
        .session
        .house
        .household
        .as_ref()
        .context("continuation household absent")?
        .name
        .clone();
    if expected_house != "UserData2/Houses/House09.iff" {
        ensure!(
            std::env::var("SIMS_PLAYER_CONTINUATION_KEEP_QUEUED_INPUT").as_deref() == Ok("1"),
            "UserData continuation requires an actual retained offered-action receipt"
        );
    }
    if let Ok(path) = std::env::var("SIMS_PLAYER_CONTINUATION_STORE") {
        let path = root.join(path);
        ensure!(
            path.starts_with(root.join("fixtures/private")),
            "private new continuation store required"
        );
        sims_fixtures::Store::create(path)?;
    }
    let mut cold = warm.fork()?;
    let compiler = std::process::Command::new("rustc").arg("-Vv").output()?;
    let exe = std::env::current_exe()?;
    write(
        &out,
        "input.json",
        &json!({"source":source_name,"source_sha256":source_sha,"house":expected_house,"program_fingerprint":warm.session.program.fingerprint(),"options":warm.session.options,"declared_neighborhood_input":warm.session.host.presentation.neighborhood_input,"immutable_index":INDEX,
        "routing_policy":"retained InferredRectangles","qualification":"internal continuation, not original parity","finite_runtime_dialogs":dialog_schedule.as_ref().map(|s|s.manifest()),
        "required_completed_ticks":REQUIRED,"requested_tick_limit":max_ticks,"max_seconds":max_seconds,
        "clock_policy":"advance explicit application time by current retained period only between frames; empty pending-input queue; no modal responses",
        "build_config":std::env::var("SIMS_PLAYER_CONTINUATION_BUILD_CONFIG").ok(),"debug_assertions":cfg!(debug_assertions),
        "compiler":String::from_utf8(compiler.stdout)?,"test_executable":exe,"test_executable_sha256":exploration::digest(&std::fs::read(&exe)?),
        "test_source_sha256":exploration::digest(include_bytes!("household_player_continuation.rs")),
        "adapter_source_sha256":exploration::digest(include_bytes!("support/player_action_driver.rs")),"helper_source_sha256":exploration::digest(include_bytes!("support/household_cases.rs"))}),
    )?;
    let start = warm.session.simulation.vm.tick;
    let started = Instant::now();
    let mut rows = vec![];
    let mut cuts = vec![];
    let mut journal = vec![];
    let mut dialog_journal = vec![];
    let mut active_cut = false;
    let mut buy_cut = false;
    let mut live_cut = false;
    let mut later_cut = false;
    let mut committed_edit = false;
    let mut completed = 0u64;
    let mut polls = 0usize;
    let result = (|| -> Result<()> {
        cut(
            &root,
            &out,
            &warm,
            &mut cold,
            "initial-independent",
            &mut cuts,
        )?;
        if std::env::var("SIMS_PLAYER_CONTINUATION_KEEP_QUEUED_INPUT").as_deref() != Ok("1") {
            let person = Identity::capture(&warm.session, 196)?;
            let roaches = Identity::capture(&warm.session, 140)?;
            inputs(
                &out,
                &mut warm,
                &mut cold,
                vec![
                    Command::Select { identity: person },
                    Command::Drain { budget: 256 },
                    Command::OpenObject { identity: roaches },
                    Command::Drain { budget: 256 },
                    Command::ChooseFirst,
                ],
                &mut journal,
            )?;
        }
        let receipt = warm
            .queued_receipt()
            .context("authored original roach interaction was refused or absent")?;
        if expected_house == "UserData2/Houses/House09.iff" {
            ensure!(
                receipt.0 == 196,
                "retained Mashuga player receipt belongs to another actor"
            );
        } else {
            let object = warm
                .session
                .simulation
                .vm
                .world
                .objects
                .get(&receipt.0)
                .context("retained actor absent")?;
            ensure!(
                object.person.is_some()
                    && object.definition.as_ref().is_some_and(|d| warm
                        .session
                        .house
                        .household
                        .as_ref()
                        .is_some_and(|h| h.residents.iter().any(|r| r.guid == d.guid))),
                "retained actor is not an actual imported resident"
            );
        }
        let actor = warm
            .session
            .simulation
            .vm
            .world
            .actions
            .as_ref()
            .and_then(|w| w.actors.get(&receipt.0))
            .context("queued actor absent")?;
        let retained_action = actor
            .active
            .iter()
            .chain(actor.queue.pending.iter())
            .find(|a| a.id == receipt.1)
            .context("retained player receipt is no longer active/pending")?;
        if expected_house == "UserData2/Houses/House09.iff" {
            ensure!(
                retained_action.target == 140,
                "retained Mashuga action target changed"
            );
        }
        while completed < max_ticks {
            // Diagnostic work-budget Pending yields may differ with caches.
            // Align semantic boundaries without advancing the completed side.
            let a = ready_frame(&mut warm, &mut polls, started, max_seconds)?;
            let b = ready_frame(&mut cold, &mut polls, started, max_seconds)?;
            match (a, b) {
                (Frame::Boundary(Boundary::Tick), Frame::Boundary(Boundary::Tick)) => {
                    let row = same(&warm, &cold)?;
                    ensure!(
                        row.tick == start + completed + 1,
                        "completed tick sequence skipped a boundary"
                    );
                    rows.push(row);
                    completed += 1;
                    if !active_cut && active(&warm, receipt) {
                        cut(
                            &root,
                            &out,
                            &warm,
                            &mut cold,
                            "active-player-interaction",
                            &mut cuts,
                        )?;
                        active_cut = true;
                    }
                    if completed.is_multiple_of(128) {
                        write(&out, "ticks.json", &rows)?;
                        eprintln!("{scenario} continuation:{completed}/{REQUIRED} completed ticks");
                    }
                    if completed >= 2048 && !later_cut {
                        cut(
                            &root,
                            &out,
                            &warm,
                            &mut cold,
                            "later-active-household",
                            &mut cuts,
                        )?;
                        later_cut = true;
                    }
                }
                (
                    Frame::Boundary(Boundary::PartialPass),
                    Frame::Boundary(Boundary::PartialPass),
                ) => {
                    if dialog_schedule.as_ref().is_some_and(|s| !s.complete()) {
                        same(&warm, &cold)?;
                    }
                }
                (Frame::Complete, Frame::Complete) => {
                    if completed >= 128 && !buy_cut {
                        use move_tool::{button, motion};
                        let pointer = motion::Input {
                            position: match expected_house {
                                "UserData/Houses/House05.iff" => [552, 456],
                                "UserData/Houses/House07.iff" => [552, 280],
                                _ => [376, 488],
                            },
                            screen: [230, 300],
                            picked: 0,
                        };
                        inputs(
                            &out,
                            &mut warm,
                            &mut cold,
                            vec![
                                Command::Pause { paused: true },
                                Command::Drain { budget: 256 },
                                Command::BeginMode { mode: 1 },
                                Command::Drain { budget: 256 },
                            ],
                            &mut journal,
                        )?;
                        let collapsed = warm
                            .device
                            .panel_widgets()
                            .and_then(|w| w.presentation())
                            .and_then(|p| p.collapsed())
                            .context("Buy parent expansion state unknown")?;
                        if collapsed {
                            inputs(
                                &out,
                                &mut warm,
                                &mut cold,
                                vec![
                                    Command::ExpandPanel { value: 0 },
                                    Command::Drain { budget: 256 },
                                ],
                                &mut journal,
                            )?;
                        }
                        inputs(
                            &out,
                            &mut warm,
                            &mut cold,
                            vec![
                                Command::BuyCategory {
                                    label: "Seating".into(),
                                },
                                Command::Drain { budget: 256 },
                                Command::BuyProduct {
                                    source_contains: "chair".into(),
                                },
                                Command::Drain { budget: 256 },
                                Command::EnterMove,
                                Command::Drain { budget: 256 },
                                Command::Move { pointer },
                                Command::Drain { budget: 256 },
                            ],
                            &mut journal,
                        )?;
                        cut(
                            &root,
                            &out,
                            &warm,
                            &mut cold,
                            "buy-unpaid-preview",
                            &mut cuts,
                        )?;
                        buy_cut = true;
                        let tool = warm
                            .session
                            .host
                            .presentation
                            .move_tool
                            .as_ref()
                            .context("Buy Move owner absent")?;
                        let price = tool.tool.value90 as i32;
                        let cursor_handle =
                            tool.tool.held.context("unpaid preview has no cursor")?;
                        let world = &warm.session.simulation.vm.world;
                        let cursor = world
                            .scheduler_storage
                            .as_ref()
                            .and_then(|r| r.object_for_handle(cursor_handle))
                            .context("preview cursor registry owner absent")?;
                        let handle = world
                            .objects
                            .get(&cursor)
                            .and_then(|o| o.cursor.as_ref())
                            .and_then(|c| c.underlying)
                            .context("preview furniture lifetime absent")?;
                        let furniture = world
                            .scheduler_storage
                            .as_ref()
                            .and_then(|r| r.object_for_handle(handle))
                            .context("preview furniture registry owner absent")?;
                        let identity = Identity::capture(&warm.session, furniture)?;
                        let before = world
                            .budget
                            .as_ref()
                            .context("household budget absent")?
                            .balance;
                        ensure!(
                            price >= 0 && before >= price,
                            "authored purchase exceeds actual budget"
                        );
                        inputs(
                            &out,
                            &mut warm,
                            &mut cold,
                            vec![
                                Command::Button {
                                    input: button::Input {
                                        kind: button::Kind::Down,
                                        pointer,
                                        shift: false,
                                    },
                                },
                                Command::Drain { budget: 256 },
                                Command::Button {
                                    input: button::Input {
                                        kind: button::Kind::Up,
                                        pointer,
                                        shift: false,
                                    },
                                },
                                Command::Drain { budget: 256 },
                            ],
                            &mut journal,
                        )?;
                        let after = warm
                            .session
                            .simulation
                            .vm
                            .world
                            .budget
                            .as_ref()
                            .context("post-purchase budget absent")?
                            .balance;
                        let hint = warm
                            .session
                            .host
                            .presentation
                            .move_tool
                            .as_ref()
                            .context("post-purchase Move owner absent")?
                            .button_hint();
                        ensure!(
                            hint == Some(button::Hint::Spent(price)) && after == before - price,
                            "authored placement did not produce a committed purchase: hint={hint:?}, funds_before={before}, funds_after={after}, quote={price}"
                        );
                        ensure!(
                            Identity::capture(&warm.session, furniture)? == identity
                                && !warm.session.simulation.unplaced.contains(&furniture),
                            "paid furniture is not live/on-lot"
                        );
                        committed_edit = true;
                        write(
                            &out,
                            "committed-edit.json",
                            &json!({"identity":identity,"quoted_price":price,"funds_before":before,"funds_after":after,"pointer":pointer}),
                        )?;
                        cut(
                            &root,
                            &out,
                            &warm,
                            &mut cold,
                            "buy-committed-furniture",
                            &mut cuts,
                        )?;
                        if warm
                            .session
                            .host
                            .presentation
                            .move_tool
                            .as_ref()
                            .is_some_and(|m| m.tool.held.is_some())
                        {
                            inputs(
                                &out,
                                &mut warm,
                                &mut cold,
                                vec![
                                    Command::Button {
                                        input: button::Input {
                                            kind: button::Kind::Cancel,
                                            pointer,
                                            shift: false,
                                        },
                                    },
                                    Command::Drain { budget: 256 },
                                ],
                                &mut journal,
                            )?;
                        }
                        inputs(
                            &out,
                            &mut warm,
                            &mut cold,
                            vec![
                                Command::CheckNoHeld,
                                Command::BeginMode { mode: 2 },
                                Command::Drain { budget: 256 },
                                Command::Pause { paused: false },
                                Command::Drain { budget: 256 },
                                Command::CheckMode { mode: 2 },
                            ],
                            &mut journal,
                        )?;
                        ensure!(
                            Identity::capture(&warm.session, furniture)? == identity
                                && warm
                                    .session
                                    .simulation
                                    .vm
                                    .world
                                    .budget
                                    .as_ref()
                                    .context("post-Live budget absent")?
                                    .balance
                                    == after,
                            "post-Live cleanup lost paid furniture or changed funds"
                        );
                        cut(
                            &root,
                            &out,
                            &warm,
                            &mut cold,
                            "post-live-resumed",
                            &mut cuts,
                        )?;
                        live_cut = true;
                    }
                }
                (
                    Frame::Boundary(Boundary::DialogConstruction(a)),
                    Frame::Boundary(Boundary::DialogConstruction(b)),
                ) if a == b && dialog_schedule.is_some() => {
                    let schedule = dialog_schedule.as_mut().unwrap();
                    let pin = schedule.constructor(&warm, a)?;
                    let ordinal = schedule.constructor_index();
                    ensure!(
                        schedule.constructor(&cold, b)? == pin,
                        "cold constructor pin differs"
                    );
                    let before = same(&warm, &cold)?;
                    cut(
                        &root,
                        &out,
                        &warm,
                        &mut cold,
                        &format!("finite-constructor-{ordinal}-before"),
                        &mut cuts,
                    )?;
                    dialog_journal.push(json!({"kind":"constructor_begin","pin":pin,"state":before,"complete":false}));
                    write(&out, "runtime-dialog-inputs.json", &dialog_journal)?;
                    warm.begin_dialog_construction(&pin)?;
                    cold.begin_dialog_construction(&pin)?;
                    same(&warm, &cold)?;
                    cut(
                        &root,
                        &out,
                        &warm,
                        &mut cold,
                        &format!("finite-constructor-{ordinal}-pending"),
                        &mut cuts,
                    )?;
                    let mut complete = false;
                    for poll in 0..1000 {
                        let x = warm.advance_dialog_construction(256)?;
                        let y = cold.advance_dialog_construction(256)?;
                        ensure!(x == y, "finite constructor host progress differs");
                        let state = same(&warm, &cold)?;
                        ensure!(
                            state.tick == before.tick && state.rng == before.rng,
                            "pinned presentation construction changed time/RNG"
                        );
                        dialog_journal.push(json!({"kind":"constructor_advance","poll":poll,"budget":256,"state":state,"complete":x}));
                        write(&out, "runtime-dialog-inputs.json", &dialog_journal)?;
                        if x {
                            complete = true;
                            break;
                        }
                    }
                    ensure!(
                        complete,
                        "finite constructor did not complete in its budget"
                    );
                    warm.collect_dialog_images()?;
                    cold.collect_dialog_images()?;
                    same(&warm, &cold)?;
                    cut(
                        &root,
                        &out,
                        &warm,
                        &mut cold,
                        &format!("finite-constructor-{ordinal}-completed-queued"),
                        &mut cuts,
                    )?;
                    schedule.constructed();
                }
                (Frame::Boundary(Boundary::Dialog), Frame::Boundary(Boundary::Dialog))
                    if dialog_schedule.is_some() =>
                {
                    let schedule = dialog_schedule.as_mut().unwrap();
                    schedule.validate_dialog(&warm)?;
                    schedule.validate_dialog(&cold)?;
                    let ordinal = schedule.response_index();
                    cut(
                        &root,
                        &out,
                        &warm,
                        &mut cold,
                        &format!("finite-modal-{ordinal}-active-unanswered"),
                        &mut cuts,
                    )?;
                    dialog_journal.push(schedule.answer(&mut warm, &mut cold)?);
                    write(&out, "runtime-dialog-inputs.json", &dialog_journal)?;
                    cut(
                        &root,
                        &out,
                        &warm,
                        &mut cold,
                        &format!("finite-modal-{ordinal}-answered"),
                        &mut cuts,
                    )?;
                }
                (Frame::Boundary(a), Frame::Boundary(b)) if a == b => {
                    let state = same(&warm, &cold)?;
                    write(
                        &out,
                        "unanswered-boundary.json",
                        &json!({
                            "boundary":a,"state":state,
                            "dialogs":warm.session.simulation.vm.world.dialogs,
                            "response_submitted":false
                        }),
                    )?;
                    anyhow::bail!("original unanswered/unsupported household boundary: {a:?}")
                }
                _ => {
                    anyhow::bail!("warm/cold clock progress differs before the next completed tick")
                }
            }
        }
        if completed == REQUIRED {
            ensure!(
                dialog_schedule.as_ref().is_none_or(|s| s.complete()),
                "finite runtime input schedule was not consumed"
            );
            cut(
                &root,
                &out,
                &warm,
                &mut cold,
                "final4096-independent",
                &mut cuts,
            )?;
        }
        Ok(())
    })();
    write(&out, "ticks.json", &rows)?;
    let qualified = result.is_ok()
        && completed == REQUIRED
        && active_cut
        && buy_cut
        && live_cut
        && later_cut
        && committed_edit;
    let stop = result.as_ref().err().map(|e| format!("{e:#}")).or_else(||(!qualified).then(||format!("bounded continuation incomplete at{completed}/{REQUIRED}; required checkpoints/edit must also complete")));
    let actual = warm.session.simulation.vm.tick.saturating_sub(start);
    write(
        &out,
        "report.json",
        &json!({"qualification":"internal warm/cold continuation, not strict-original parity",
        "outcome":if qualified{"completed"}else{"incomplete"},"required_completed_ticks":REQUIRED,"compared_completed_ticks":completed,
        "actual_warm_tick_delta":actual,"cold_tick_delta":cold.session.simulation.vm.tick.saturating_sub(start),"native_parity_ticks":0,
        "active_interaction_checkpoint":active_cut,"buy_checkpoint":buy_cut,"post_live_checkpoint":live_cut,"later_checkpoint":later_cut,
        "committed_furniture_edit":committed_edit,"finite_runtime_dialogs":dialog_schedule.as_ref().map(|s|s.manifest()),
        "all_cut_indices_exclude_original_saves":true,"polls":polls,"seconds":started.elapsed().as_secs_f64(),"stop":stop}),
    )?;
    if !qualified && std::env::var("SIMS_PLAYER_CONTINUATION_STORE").is_ok() {
        let store = sims_fixtures::Store::open(
            root.join(std::env::var("SIMS_PLAYER_CONTINUATION_STORE")?),
        )?;
        let (blob, player, error) = match warm.checkpoint_fixture(&store) {
            Ok(blob) => (blob, true, None),
            Err(error) => (
                store.put_value(&warm.session.save())?,
                false,
                Some(format!("{error:#}")),
            ),
        };
        let mut case = sims_fixtures::CaseManifest::minimal_import(
            "first-stop",
            "First retained continuation stop",
            sims_fixtures::Artifact::self_contained(
                blob.clone(),
                sims_fixtures::ArtifactFormat::MessagePack,
                if player { 1 } else { 2 },
            ),
            vec![
                "Rust-produced stop; no original-parity claim; schema2 is research Session only"
                    .into(),
            ],
        );
        case.provenance.origin = sims_fixtures::Origin::Authored;
        store.publish_case(&case)?;
        write(
            &out,
            "first-stop-state.json",
            &json!({"fixture_store":store.root(),"case":"first-stop","blob":blob,"managed_player_capture":player,"managed_capture_error":error}),
        )?;
    } else if !qualified {
        // One retained stop image, never a full snapshot per tick. The caller
        // imports this single large diagnostic into sims-fixtures after the run.
        let checkpoint = warm.checkpoint();
        match checkpoint {
            Ok(bytes) => {
                std::fs::write(out.join("first-stop-managed.json"), &bytes)?;
                write(
                    &out,
                    "first-stop-state.json",
                    &json!({"resumable":true,"bytes":bytes.len(),"sha256":exploration::digest(&bytes)}),
                )?;
            }
            Err(error) => {
                let bytes = match warm.research_state() {
                    Ok(state) => serde_json::to_vec(&state)?,
                    Err(_) => serde_json::to_vec(&warm.session.save())?,
                };
                std::fs::write(out.join("first-stop-session-research.json"), &bytes)?;
                write(
                    &out,
                    "first-stop-state.json",
                    &json!({"resumable":false,"managed_capture_error":format!("{error:#}"),"bytes":bytes.len(),"sha256":exploration::digest(&bytes)}),
                )?;
            }
        }
    }
    result?;
    ensure!(
        qualified,
        "bounded run did not meet the4096 actions/checkpoint criterion"
    );
    Ok(())
}

#[test]
#[ignore = "seal compact continuation/edit receipts and the existing checkpoint dependency closure"]
fn publish_player_campaign_receipts_and_checkpoint_closure() -> Result<()> {
    use sims_fixtures::{
        Artifact, ArtifactFormat, CaseManifest, Coverage, InputEvent, Origin, ScheduleStep,
    };
    let root = repo()?;
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private receipts required"
    );
    let store = sims_fixtures::Store::open(root.join(std::env::var("SIMS_PLAYER_NOTICE_STORE")?))?;
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("report.json"))?)?;
    ensure!(
        report["native_parity_ticks"] == 0,
        "internal campaign cannot publish parity credit"
    );
    let mut files = std::collections::BTreeMap::new();
    let mut exact_plans = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(&out)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .context("receipt filename absent")?
            .to_string_lossy()
            .into_owned();
        if !path.is_file() || name == "published-campaign.json" {
            continue;
        }
        ensure!(
            std::fs::metadata(&path)?.len() <= 8 * 1024 * 1024,
            "unexpected bulky campaign artifact: {name}"
        );
        if name.ends_with("plan.json") {
            let raw = std::fs::read(&path)?;
            ensure!(raw.len() <= 32768, "unexpectedly large authored plan");
            exact_plans.insert(name.clone(),json!({"sha256":exploration::digest(&raw),"bytes":raw.len(),"utf8":String::from_utf8(raw)?}));
        }
        let value = match path.extension().and_then(|e| e.to_str()) {
            Some("json") => serde_json::from_slice(&std::fs::read(&path)?)?,
            Some("rs") => json!({"utf8":std::fs::read_to_string(&path)?}),
            _ => continue,
        };
        files.insert(name, value);
    }
    let mut refs = vec![];
    let mut dependencies = vec![];
    for id in store.list_cases()? {
        let case = store.load_case(&id)?;
        dependencies.extend(case.source.blob_refs());
        for artifact in &case.artifacts {
            dependencies.extend(artifact.blob_refs())
        }
        for reference in &case.references {
            dependencies.extend(reference.artifact.blob_refs())
        }
        refs.extend(case.references);
    }
    dependencies.sort_by(|a, b| a.sha256.cmp(&b.sha256));
    dependencies.dedup();
    let blob=store.put_value(&json!({"schema":1,"qualification":"authored internal runtime continuation; source applicability and coverage limitations remain in exact receipts; no strict-original parity","files":files,"exact_authored_input_files":exact_plans}))?;
    let mut case=CaseManifest::minimal_import("authored-player-campaign","Authored player campaign receipts and owned checkpoint closure",Artifact::declared(blob,ArtifactFormat::MessagePack,1,dependencies),vec!["Each recorded stop/completed count is retained without promotion to full game or original parity".into()]);
    case.provenance.origin = Origin::Authored;
    case.provenance.source_locator =
        files
            .get("input.json")
            .and_then(|i| i.get("source"))
            .map(|s| {
                s.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| s.to_string())
            });
    case.coverage=Coverage::Component{scope:"exact retained authored input/continuation receipts and verified checkpoint closure".into(),limitations:vec!["InferredRectangles and conditional supplied native input; no observed original action schedule or tick parity".into(),"Movement/rotation/Escape/sale only covered if the separately named edit cycle actually passes; Iced key delivery is separate".into()]};
    case.references = refs;
    if let Some(inputs) = files
        .get("inputs.json")
        .and_then(|v| v.as_array())
        .or_else(|| report.get("inputs").and_then(|v| v.as_array()))
    {
        for row in inputs {
            case.schedule.push(ScheduleStep::WaitBoundary {
                boundary: sims_fixtures::Boundary::Event {
                    name: "authored input at completed tick".into(),
                    tick: row["tick"].as_u64(),
                },
            });
            case.schedule.push(ScheduleStep::Input {
                input: InputEvent {
                    name: "player-action-driver".into(),
                    payload: row.clone(),
                },
            });
        }
    }
    store.publish_case(&case)?;
    write(
        &out,
        "published-campaign.json",
        &json!({"case":case.id,"store":store.root(),"verification":store.verify_all()?}),
    )?;
    Ok(())
}

#[test]
#[ignore = "add exact source-plan bytes/hash to an existing store without replacing any sealed case"]
fn retain_exact_authored_input_files_additively() -> Result<()> {
    use sims_fixtures::{Artifact, ArtifactFormat, CaseManifest, Coverage, Origin};
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct InputFile {
        path: String,
        sha256: String,
    }
    let root = repo()?;
    let manifest_path = std::env::var("SIMS_PLAYER_EXACT_INPUT_FILES")?;
    let raw = std::fs::read(root.join(&manifest_path))?;
    ensure!(
        raw.len() <= 32768
            && exploration::digest(&raw) == std::env::var("SIMS_PLAYER_EXACT_INPUT_FILES_SHA256")?,
        "exact-file declaration changed"
    );
    let declared: Vec<InputFile> = serde_json::from_slice(&raw)?;
    ensure!(
        (1..=32).contains(&declared.len()),
        "exact input file vector is unbounded/empty"
    );
    let store = sims_fixtures::Store::open(root.join(std::env::var("SIMS_PLAYER_NOTICE_STORE")?))?;
    let case_id = "exact-authored-input-files-20261026";
    let ids = store.list_cases()?;
    ensure!(
        !ids.iter().any(|id| id == case_id),
        "additive exact-input receipt already exists; no sealed case is replaced"
    );
    let mut seen = std::collections::BTreeSet::new();
    let mut files = vec![];
    for declared in declared {
        ensure!(
            seen.insert(declared.path.clone()),
            "exact input path repeated"
        );
        let path = root.join(&declared.path);
        ensure!(
            path.starts_with(root.join("cache")),
            "exact plans must be declared private research inputs"
        );
        let raw = std::fs::read(path)?;
        ensure!(
            raw.len() <= 32768 && exploration::digest(&raw) == declared.sha256,
            "original input bytes changed"
        );
        files.push(json!({"source_path":declared.path,"sha256":declared.sha256,"bytes":raw.len(),"utf8":String::from_utf8(raw)?}));
    }
    let blob=store.put_value(&json!({"schema":1,"qualification":"exact authored input bytes; no original input observation or simulation parity claim","declaration":{"path":manifest_path,"sha256":exploration::digest(&raw),"utf8":String::from_utf8(raw)?},"files":files}))?;
    let restored: serde_json::Value = store.get_value(&blob)?;
    for file in restored["files"]
        .as_array()
        .context("restored exact files absent")?
    {
        let bytes = file["utf8"]
            .as_str()
            .context("restored exact input absent")?
            .as_bytes();
        ensure!(
            file["bytes"].as_u64() == Some(bytes.len() as u64)
                && file["sha256"] == exploration::digest(bytes),
            "MessagePack exact-input round trip changed byte identity"
        );
    }
    let mut dependencies = vec![];
    for id in ids {
        let case = store.load_case(&id)?;
        dependencies.extend(case.source.blob_refs());
        for artifact in case.artifacts {
            dependencies.extend(artifact.blob_refs());
        }
        for reference in case.references {
            dependencies.extend(reference.artifact.blob_refs());
        }
    }
    dependencies.sort_by(|a, b| a.sha256.cmp(&b.sha256));
    dependencies.dedup();
    let mut case=CaseManifest::minimal_import(case_id,"Exact authored input files, additive byte receipt",Artifact::declared(blob.clone(),ArtifactFormat::MessagePack,1,dependencies),vec!["Original sealed cases remain immutable; this adds exact UTF8 and SHA256 of the explicitly declared plan files".into()]);
    case.provenance.origin = Origin::Authored;
    case.coverage = Coverage::Component {
        scope: "verified exact authored-input byte retention".into(),
        limitations: vec!["Does not add gameplay, original-input or original-parity credit".into()],
    };
    store.publish_case(&case)?;
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private exact receipt required"
    );
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out.join("exact-input-plan-receipt.json"))?;
    serde_json::to_writer_pretty(
        file,
        &json!({"case":case_id,"source":blob,"store":store.root(),"verification":store.verify_all()?}),
    )?;
    Ok(())
}

#[test]
#[ignore = "read-only initial/final owner audit of the three sealed4096 cases; no simulation or input"]
fn audit_retained_household_acceptance_owners() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(audit_retained_household_owners_inner)?
        .join()
        .expect("read-only ownership audit worker panicked")
}

fn audit_retained_household_owners_inner() -> Result<()> {
    #[derive(serde::Deserialize)]
    struct Player {
        session: Saved,
    }
    #[derive(serde::Deserialize)]
    struct Saved {
        source: String,
        simulation: Snapshot,
        managed: Content,
    }
    #[derive(serde::Deserialize)]
    struct Content {
        house: Vec<u8>,
        owned: Owned,
    }
    #[derive(serde::Deserialize)]
    struct Owned {
        entries: Vec<Entry>,
    }
    #[derive(serde::Deserialize)]
    struct Entry {
        locator: String,
        sha256: String,
    }
    #[derive(serde::Deserialize)]
    struct Snapshot {
        state: State,
    }
    #[derive(serde::Deserialize)]
    struct State {
        vm: Vm,
    }
    #[derive(serde::Deserialize)]
    struct Vm {
        tick: u64,
        world: sims_vm::World,
    }
    fn summary(v: &serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::Object(v) => {
                json!({"kind":"object","members":v.len(),"sha256":exploration::digest_serialized(v).unwrap()})
            }
            serde_json::Value::Array(v) => {
                json!({"kind":"array","members":v.len(),"sha256":exploration::digest_serialized(v).unwrap()})
            }
            serde_json::Value::String(v) if v.len() > 160 => {
                json!({"kind":"string","bytes":v.len(),"sha256":exploration::digest(v.as_bytes())})
            }
            _ => v.clone(),
        }
    }
    fn diff(
        a: &serde_json::Value,
        b: &serde_json::Value,
        path: String,
        count: &mut usize,
        examples: &mut Vec<serde_json::Value>,
    ) {
        if a == b {
            return;
        }
        match (a, b) {
            (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
                let keys = a
                    .keys()
                    .chain(b.keys())
                    .collect::<std::collections::BTreeSet<_>>();
                for k in keys {
                    diff(
                        a.get(k).unwrap_or(&serde_json::Value::Null),
                        b.get(k).unwrap_or(&serde_json::Value::Null),
                        format!("{path}/{k}"),
                        count,
                        examples,
                    )
                }
            }
            (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
                for i in 0..a.len().max(b.len()) {
                    diff(
                        a.get(i).unwrap_or(&serde_json::Value::Null),
                        b.get(i).unwrap_or(&serde_json::Value::Null),
                        format!("{path}/{i}"),
                        count,
                        examples,
                    )
                }
            }
            _ => {
                *count += 1;
                if examples.len() < 24 {
                    examples.push(json!({"path":path,"before":summary(a),"after":summary(b)}));
                }
            }
        }
    }
    fn delta(a: impl Serialize, b: impl Serialize) -> Result<serde_json::Value> {
        let a = serde_json::to_value(a)?;
        let b = serde_json::to_value(b)?;
        let mut count = 0;
        let mut examples = vec![];
        diff(&a, &b, String::new(), &mut count, &mut examples);
        Ok(
            json!({"changed":a!=b,"before_sha256":exploration::digest_serialized(&a)?,"after_sha256":exploration::digest_serialized(&b)?,"changed_leaf_or_presence_paths":count,"examples":examples}),
        )
    }
    fn people(w: &sims_vm::World) -> serde_json::Value {
        let records=w.objects.iter().filter_map(|(id,o)|{
            let person=o.person.as_ref()?;let d=o.definition.as_ref()?;
            Some((format!("{}:{id}",d.guid),json!({"guid":d.guid,"object":id,"source":d.source,"person_fields":person,"position":w.positions.get(id)})))
        }).collect::<std::collections::BTreeMap<_,_>>();
        json!(records)
    }
    let root = repo()?;
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private read-only audit output required"
    );
    std::fs::create_dir(&out).context("refusing to overwrite owner audit")?;
    let mut ledger = vec![];
    for (name, dir) in [
        (
            "mashuga",
            "player-continuation-fresh-mashuga-20261026-third",
        ),
        ("newbie", "player-newbie-4096-20261026-first"),
        ("goth", "player-goth-4096-20261026-third"),
    ] {
        let store = sims_fixtures::Store::open(root.join("fixtures/private").join(dir))?;
        let receipt = store.load_case("authored-player-campaign")?;
        let packet: serde_json::Value = store.get_value(&receipt.source.blob)?;
        let report = &packet["files"]["report.json"];
        ensure!(
            report["outcome"] == "completed"
                && report["compared_completed_ticks"] == 4096
                && report["actual_warm_tick_delta"] == 4096
                && report["cold_tick_delta"] == 4096
                && report["native_parity_ticks"] == 0,
            "sealed campaign did not pass its exact internal criterion"
        );
        let initial = store.load_case("initial-independent")?.source.blob;
        let final_blob = store.load_case("final4096-independent")?.source.blob;
        let a: Player = store.get_value(&initial)?;
        let b: Player = store.get_value(&final_blob)?;
        ensure!(
            b.session.simulation.state.vm.tick == a.session.simulation.state.vm.tick + 4096,
            "retained audit tick interval differs"
        );
        let rows = packet["files"]["ticks.json"]
            .as_array()
            .context("durable tick rows absent")?;
        ensure!(
            rows.len() == 4096
                && rows.iter().enumerate().all(|(i, r)| r["tick"].as_u64()
                    == Some(a.session.simulation.state.vm.tick + i as u64 + 1)
                    && r["rng"].as_array().is_some_and(|rng| rng.len() == 3)),
            "durable completed tick rows are not contiguous"
        );
        let final_state: Observation = serde_json::from_value(rows.last().unwrap().clone())?;
        let restored = Household::from_fixture(index(&root)?, &store, &final_blob)?;
        ensure!(
            restored.observe()? == final_state,
            "fresh independent final restore differs from actual published warm state"
        );
        ensure!(
            exploration::digest_serialized(&restored.session.simulation.vm.world)?
                == exploration::digest_serialized(&b.session.simulation.state.vm.world)?,
            "partial reader differs from independently restored world"
        );
        let denial = owned_resolver_receipt(&root, &restored)?;
        write(&out, &format!("{name}-denied-source-paths.json"), &denial)?;
        let aw = &a.session.simulation.state.vm.world;
        let bw = &b.session.simulation.state.vm.world;
        let owned_a = a
            .session
            .managed
            .owned
            .entries
            .iter()
            .map(|e| (&e.locator, &e.sha256))
            .collect::<std::collections::BTreeMap<_, _>>();
        let owned_b = b
            .session
            .managed
            .owned
            .entries
            .iter()
            .map(|e| (&e.locator, &e.sha256))
            .collect::<std::collections::BTreeMap<_, _>>();
        let ap = aw
            .person
            .as_ref()
            .context("initial neighborhood owner absent")?;
        let bp = bw
            .person
            .as_ref()
            .context("final neighborhood owner absent")?;
        let live_final = bw
            .objects
            .values()
            .filter(|o| o.person.is_some())
            .filter_map(|o| o.definition.as_ref().map(|d| d.guid))
            .collect::<std::collections::BTreeSet<_>>();
        let changed_neighbors=ap.neighbors.keys().chain(bp.neighbors.keys()).copied().collect::<std::collections::BTreeSet<_>>().into_iter().filter(|id|ap.neighbors.get(id)!=bp.neighbors.get(id)).map(|id|{
            let guid=bp.neighbors.get(&id).or_else(||ap.neighbors.get(&id)).unwrap().guid;
            Ok(json!({"neighbor":id,"guid":guid,"no_live_person_object_at_final":!live_final.contains(&guid),"delta":delta(ap.neighbors.get(&id),bp.neighbors.get(&id))?}))
        }).collect::<Result<Vec<_>>>()?;
        ledger.push(json!({"household":name,"source":a.session.source,"store":store.root(),"metadata_receipt":receipt.source.blob,"initial":initial,"final":final_blob,"initial_tick":a.session.simulation.state.vm.tick,"final_tick":b.session.simulation.state.vm.tick,"completed_ticks":4096,"native_parity_ticks":0,
            "per_tick_source":"sealed authored-player-campaign files/ticks.json; same() checks full research Session/Device/Client and all3RNG at every completed boundary",
            "published_warm_final":final_state,"independent_final_restore_equal":true,"source_denial_entries":denial["entries"].as_array().unwrap().len(),"source_denial_scope":"resolver/installation, not OS sandbox; mounted resource.owned bytes are validated",
            "cuts":packet["files"]["cuts.json"],"runtime_input_profile":packet["files"]["input.json"],
            "backing_owned_resource_hashes":delta(owned_a,owned_b)?,"parsed_import_house_metadata":delta(exploration::digest(&a.session.managed.house),exploration::digest(&b.session.managed.house))?,
            "neighborhood_scalar_words":delta(&aw.neighborhood,&bw.neighborhood)?,"neighbor_record_changes":changed_neighbors,
            "families":delta(&ap.families,&bp.families)?,"family_metadata":delta(&ap.family_metadata,&bp.family_metadata)?,"family_change_notifications":delta(&ap.family_changes,&bp.family_changes)?,
            "person_resource_registry":delta(&ap.resource_instances,&bp.resource_instances)?,"neighbor_slot_owner":delta(&ap.neighbor_slots,&bp.neighbor_slots)?,
            "live_person_fields_and_position":delta(people(aw),people(bw))?,"body_profile_banks":delta(&aw.body_profiles,&bw.body_profiles)?,"resource_storage":delta(&aw.resource_storage,&bw.resource_storage)?,"changed_resource_flags":delta(&aw.changed_resources,&bw.changed_resources)?,"budget":delta(&aw.budget,&bw.budget)?,
            "limits":["endpoint owner audit cannot prove an absent-final neighbor never visited during intermediate ticks","unchanged starting IFF bytes are retained provenance, not proof of a mutable neighborhood/character update","InferredRectangles and supplied input applicability limits remain; this is not original engine parity"]}));
        write(
            &out,
            "ledger.json",
            &json!({"schema":1,"simulation_slices_or_inputs":0,"households":ledger}),
        )?;
    }
    Ok(())
}

#[test]
#[ignore = "exactNewbie197028 or inspected197540; zero-input≤512tick carrier departure/writeback and actual safe managed cuts"]
fn newbie_existing_carrier_writeback_survives_managed_restore() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(newbie_carrier_writeback_inner)?
        .join()
        .expect("carrier writeback worker panicked")
}
fn newbie_carrier_writeback_inner() -> Result<()> {
    let root = repo()?;
    let outbound = std::env::var("SIMS_PLAYER_CARRIER_SOURCE").ok();
    ensure!(
        outbound
            .as_deref()
            .is_none_or(|v| v == "inspected-outbound197540"),
        "unknown carrier source declaration"
    );
    let (input_path, input_case, expected_sha, expected_tick, expected_warm) = if outbound.is_some()
    {
        (
            "fixtures/private/player-newbie-carrier-writeback-20261026-first",
            "first-stop",
            "aa0f65394dc3c29f3f741b90d29b5ca177ada52a6710e1ecb09227e46a1357ce",
            197540,
            "0c93e42e8c2a9628c6f1cb198d4ab2a78bf8c1a244d56b74469f491bf006b411",
        )
    } else {
        (
            "fixtures/private/player-newbie-4096-20261026-first",
            "final4096-independent",
            "46378debb0fe9281d54116941eefe5209c258ef0b727d99b3c78ba62bfe19cab",
            197028,
            "eba097c96eaa4cccbdd9a298f145f64752248261a80e6b4664d9992ad4f62cab",
        )
    };
    let input = sims_fixtures::Store::open(root.join(input_path))?;
    let source = input.load_case(input_case)?.source.blob;
    ensure!(source.sha256 == expected_sha, "Newbie final source differs");
    let mut a = Household::from_fixture(index(&root)?, &input, &source)?;
    let mut b = a.fork()?;
    let start = same(&a, &b)?;
    ensure!(
        start.tick == expected_tick && start.state_sha256 == expected_warm,
        "retained source does not match its published warm end"
    );
    let ids = a
        .session
        .simulation
        .vm
        .world
        .objects
        .iter()
        .filter(|(_, o)| {
            o.person.is_some() && o.definition.as_ref().is_some_and(|d| d.guid == 3708437381)
        })
        .map(|(&id, _)| Identity::capture(&a.session, id))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        ids.len() == 1,
        "existing source carrier lifetime is absent/ambiguous"
    );
    let carrier = ids[0].clone();
    let initial_person = a.session.simulation.vm.world.objects[&carrier.object]
        .person
        .as_ref()
        .unwrap()
        .clone();
    ensure!(
        carrier.object == 104 && carrier.handle == 327 && initial_person.get(&31) == Some(&25),
        "actual carrier/neighbor linkage differs"
    );
    let original = a
        .session
        .simulation
        .vm
        .world
        .person
        .as_ref()
        .context("neighborhood absent")?
        .neighbors
        .get(&25)
        .context("neighbor25 absent")?
        .clone();
    ensure!(
        original.guid == 3708437381 && original.person_revision < 9,
        "expected unpublished actual carrier record differs"
    );
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private mutation output required"
    );
    std::fs::create_dir(&out)?;
    let path = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_STORE")?);
    ensure!(
        path.starts_with(root.join("fixtures/private")),
        "private new mutation store required"
    );
    let store = sims_fixtures::Store::create(path)?;
    write(
        &out,
        "input.json",
        &json!({"schema":1,"source_store":input.root(),"source":source,"source_warm":start,"carrier":carrier,"neighbor":25,"guid":original.guid,"initial_neighbor":original,"initial_live_person":initial_person,"max_completed_ticks":512,"player_inputs":[],"qualification":"additional actual mutation/persistence coverage; zero chain credit toward the prior4096 and zero original parity","program_fingerprint":a.session.program.fingerprint(),"options":a.session.options,"neighborhood_input":a.session.host.presentation.neighborhood_input,"test_source_sha256":exploration::digest(include_bytes!("household_player_continuation.rs")),"adapter_source_sha256":exploration::digest(include_bytes!("support/player_action_driver.rs")),"executable_sha256":exploration::digest(&std::fs::read(std::env::current_exe()?)?)}),
    )?;
    write(
        &out,
        "carrier-lineage-plan.json",
        &json!({"schema":1,"origin_store":"fixtures/private/player-newbie-4096-20261026-first","origin_case":"final4096-independent","origin_tick":197028,"origin_checkpoint_sha256":"46378debb0fe9281d54116941eefe5209c258ef0b727d99b3c78ba62bfe19cab","origin_warm_sha256":"eba097c96eaa4cccbdd9a298f145f64752248261a80e6b4664d9992ad4f62cab","retained_source_store":input_path,"retained_source_case":input_case,"retained_source":source,"retained_warm":start,"inspected_outbound_follow_on":outbound.is_some(),"maximum_additional_completed_ticks":512,"inputs":[],"not_counted_toward_prior4096":true,"original_parity_ticks":0}),
    )?;
    std::fs::write(
        out.join("test-source.rs"),
        include_bytes!("household_player_continuation.rs"),
    )?;
    std::fs::write(
        out.join("adapter-source.rs"),
        include_bytes!("support/player_action_driver.rs"),
    )?;
    let mut cuts = vec![];
    let mut rows = vec![];
    let mut steps = vec![];
    let mut polls = 0;
    let started = Instant::now();
    let mut before_cut = false;
    let mut published = false;
    let mut removed = false;
    let result = (|| -> Result<()> {
        write(
            &out,
            "initial-source-denial.json",
            &owned_resolver_receipt(&root, &a)?,
        )?;
        cuts.push(fixture_cut(
            &root,
            &store,
            &a,
            &mut b,
            "initial-before-follow-on",
        )?);
        while rows.len() < 512 {
            let prior = same(&a, &b)?;
            let before_neighbor = b
                .session
                .simulation
                .vm
                .world
                .person
                .as_ref()
                .unwrap()
                .neighbors[&25]
                .clone();
            let before_person = b
                .session
                .simulation
                .vm
                .world
                .objects
                .get(&carrier.object)
                .and_then(|o| o.person.clone());
            let x = ready_frame(&mut a, &mut polls, started, 1800)?;
            let now = &a
                .session
                .simulation
                .vm
                .world
                .person
                .as_ref()
                .unwrap()
                .neighbors[&25];
            let now_alive =
                Identity::capture(&a.session, carrier.object).is_ok_and(|id| id == carrier);
            let transition =
                (now.person_revision == 9 && now.person != before_neighbor.person) || !now_alive;
            if transition && !before_cut {
                let mut verifier = b.fork()?;
                match fixture_cut(
                    &root,
                    &store,
                    &b,
                    &mut verifier,
                    "before-actual-publication-removal",
                ) {
                    Ok(blob) => {
                        cuts.push(blob);
                        b = verifier;
                        before_cut = true;
                    }
                    Err(error) => {
                        let blob = store.put_value(&b.session.save())?;
                        write(
                            &out,
                            "unsafe-prior-boundary.json",
                            &json!({"scope":"research Session only; not a safe player save","blob":blob,"error":format!("{error:#}"),"nearest_earlier_retained_safe":"initial-before-follow-on"}),
                        )?;
                        anyhow::bail!(
                            "immediate pre-publication boundary not player-safe; no boundary was reset or drained: {error:#}"
                        )
                    }
                }
            }
            let y = ready_frame(&mut b, &mut polls, started, 1800)?;
            let state = same(&a, &b)?;
            let boundary = format!("{x:?}");
            ensure!(
                format!("{x:?}") == format!("{y:?}"),
                "carrier warm/cold semantic boundaries differ"
            );
            steps.push(json!({"ordinal":steps.len(),"before":prior,"state":state,"boundary":format!("{x:?}"),"carrier_alive":now_alive,"neighbor_revision":a.session.simulation.vm.world.person.as_ref().unwrap().neighbors[&25].person_revision}));
            match (x, y) {
                (Frame::Boundary(Boundary::Tick), Frame::Boundary(Boundary::Tick)) => {
                    ensure!(
                        state.tick == start.tick + rows.len() as u64 + 1,
                        "follow-on skipped a completed tick"
                    );
                    rows.push(state);
                }
                (Frame::Complete, Frame::Complete)
                | (
                    Frame::Boundary(Boundary::PartialPass),
                    Frame::Boundary(Boundary::PartialPass),
                ) => ensure!(
                    state.tick == prior.tick,
                    "UI/partial pass received tick credit"
                ),
                _ => anyhow::bail!(
                    "unknown carrier follow-on input/service; no fabricated response: {boundary}"
                ),
            }
            let neighbor = &a
                .session
                .simulation
                .vm
                .world
                .person
                .as_ref()
                .unwrap()
                .neighbors[&25];
            if !published && neighbor.person_revision == 9 && neighbor.person != original.person {
                ensure!(
                    neighbor.person_revision == 9 && before_cut,
                    "neighbor changed without expected actual revision9 publication"
                );
                let live = before_person
                    .as_ref()
                    .context("publication lost the preceding live person owner")?;
                let mut checked = vec![];
                for field in [
                    13, 14, 15, 16, 20, 26, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 58, 60,
                ] {
                    ensure!(
                        neighbor.person.get(&field) == live.get(&field),
                        "published generated person field{field} differs from preceding actual owner"
                    );
                    checked.push(json!({"field":field,"before_neighbor":original.person.get(&field),"actual_live":live.get(&field),"after_neighbor":neighbor.person.get(&field)}));
                }
                write(
                    &out,
                    "actual-publication.json",
                    &json!({"before_neighbor":original,"after_neighbor":neighbor,"preceding_live_person":live,"checked_fields":checked,"source_semantics":"Vm::writeback_person_neighbor native425a69; no direct writeback/removal call was made by the test"}),
                )?;
                cuts.push(fixture_cut(
                    &root,
                    &store,
                    &a,
                    &mut b,
                    "after-actual-neighbor-publication",
                )?);
                published = true;
            }
            if !removed && !now_alive {
                ensure!(
                    published && before_cut,
                    "carrier removed without changed neighbor publication"
                );
                cuts.push(fixture_cut(
                    &root,
                    &store,
                    &a,
                    &mut b,
                    "after-actual-carrier-removal",
                )?);
                removed = true;
            }
            if transition || rows.len().is_multiple_of(32) {
                write(&out, "ticks.json", &rows)?;
                write(&out, "steps.json", &steps)?;
            }
            if published && removed {
                break;
            }
        }
        ensure!(
            published && removed,
            "actual carrier departure/publication did not occur inside512 completed ticks"
        );
        write(
            &out,
            "final-source-denial.json",
            &owned_resolver_receipt(&root, &a)?,
        )?;
        Ok(())
    })();
    write(&out, "ticks.json", &rows)?;
    write(&out, "steps.json", &steps)?;
    if result.is_err() {
        let blob = a
            .checkpoint_fixture(&store)
            .or_else(|_| store.put_value(&a.session.save()))?;
        write(
            &out,
            "first-stop.json",
            &json!({"blob":blob,"note":"unqualified first-stop evidence; no automatic input or state patch"}),
        )?;
    }
    let neighbor = &a
        .session
        .simulation
        .vm
        .world
        .person
        .as_ref()
        .unwrap()
        .neighbors[&25];
    write(
        &out,
        "report.json",
        &json!({"outcome":if result.is_ok(){"passed"}else{"incomplete"},"qualification":"additional no-input actual mutation/persistence; not chain credit or original parity","source":source,"starting_warm":start,"completed_ticks":rows.len(),"actual_tick_delta":a.observe()?.tick-start.tick,"player_inputs":0,"native_parity_ticks":0,"before_publication_safe_cut":before_cut,"actual_neighbor_changed":published,"actual_carrier_removed":removed,"neighbor":neighbor,"carrier_still_live":Identity::capture(&a.session,carrier.object).is_ok_and(|id|id==carrier),"cuts":cuts,"polls":polls,"stop":result.as_ref().err().map(|e|format!("{e:#}")),"verification":store.verify_all()?}),
    )?;
    result
}

/// Demonstrate resolver-level source denial, without claiming OS sandboxing:
/// original save locators cannot resolve on the installation-only base. The
/// same locators read validated checkpoint-owned bytes after mounting.
fn owned_resolver_receipt(root: &Path, a: &Household) -> Result<serde_json::Value> {
    let before = a.observe()?;
    let base = index(root)?;
    let saved = a.session.save_managed()?;
    let owned = &saved
        .managed
        .as_ref()
        .context("managed content absent")?
        .owned;
    owned.validate()?;
    let mounted = owned.mount(&base)?;
    let mut rows = vec![];
    for entry in &owned.entries {
        let denial = base
            .find(&entry.locator)
            .err()
            .context("original save resolved on base installation")?;
        let resource = &mounted.resources[mounted.find(&entry.locator)?];
        ensure!(
            resource.owned.is_some(),
            "managed lookup retained filesystem backend"
        );
        ensure!(
            std::fs::File::open(&resource.source).is_err(),
            "test installation exposes original starting-save path"
        );
        let hash = exploration::digest(&resource.read()?);
        ensure!(hash == entry.sha256, "owned resolver read differs");
        rows.push(json!({"locator":entry.locator,"base_lookup_denial":format!("{denial:#}"),"owned_sha256":hash}));
    }
    ensure!(
        rows.iter().any(|r| r["locator"] == a.session.house.source),
        "owned starting house absent"
    );
    ensure!(
        a.observe()? == before,
        "resolver evidence changed state/RNG"
    );
    Ok(
        json!({"scope":"resolver/test-installation denial, not OS-level enforcement","entries":rows,"state_neutral":true}),
    )
}

fn edit_physical(a: &Household, identity: &Identity) -> Result<serde_json::Value> {
    ensure!(
        Identity::capture(&a.session, identity.object)? == *identity,
        "furniture lifetime changed"
    );
    let world = &a.session.simulation.vm.world;
    let o = world
        .objects
        .get(&identity.object)
        .context("furniture absent")?;
    let p = world
        .positions
        .get(&identity.object)
        .context("furniture position absent")?;
    let physical = a
        .session
        .simulation
        .native_lot
        .as_ref()
        .context("physical lot absent")?
        .placement
        .links
        .capture_raw(world, identity.object)
        .map_err(anyhow::Error::msg)?;
    Ok(
        json!({"identity":identity,"position":[p.x,p.y],"level":p.level,"direction":o.data.get(&1),"room":o.data.get(&29),"container":o.container,"slots":o.slots,"on_lot":physical.on_lot,"unplaced":a.session.simulation.unplaced.contains(&identity.object)}),
    )
}

fn held_edit(a: &Household, identity: &Identity) -> Result<u32> {
    let world = &a.session.simulation.vm.world;
    let held = a
        .session
        .host
        .presentation
        .move_tool
        .as_ref()
        .and_then(|m| m.tool.held)
        .context("no actual held cursor")?;
    let cursor = world
        .scheduler_storage
        .as_ref()
        .and_then(|r| r.object_for_handle(held))
        .context("held registry entry absent")?;
    ensure!(
        world
            .objects
            .get(&cursor)
            .and_then(|o| o.cursor.as_ref())
            .and_then(|c| c.underlying)
            == Some(identity.handle),
        "cursor does not own exact furniture lifetime"
    );
    Ok(held)
}

/// The sale keeps the detached lifetime through the initial Undo snapshot;
/// normal Live teardown must later release this exact command and reference.
fn edit_sale_owner(a: &Household, identity: &Identity, price: i32) -> Result<u32> {
    let m = a
        .session
        .host
        .presentation
        .move_tool
        .as_ref()
        .context("sale Move absent")?;
    let refs = &m.history.references;
    let allocation = refs
        .by_script_id(identity.object)
        .map_err(anyhow::Error::msg)?
        .context("sale snapshot reference absent")?;
    let reference = &refs.records[&allocation];
    //48c296 increments for each captured snapshot of the same signed script
    //ID. This authored sequence owns both endpoints of an earlier move plus
    //the sale's initial state; an isolated-sale count of one would be wrong.
    let snapshot_owners = m
        .history
        .objects
        .0
        .values()
        .flat_map(|o| &o.snapshots)
        .filter(|s| s.key != 0 && s.key == reference.key)
        .count();
    ensure!(
        reference.object == Some(identity.handle)
            && snapshot_owners == 3
            && reference.count as usize == snapshot_owners,
        "sale snapshot lifetime/count differs from its three retained snapshot owners"
    );
    let mut commands = m.history.objects.0.iter().filter(|(_, o)| {
        o.kind == 3
            && o.cost == Some(price)
            && o.snapshots[0].active == 1
            && o.snapshots[0].key == reference.key
            && o.snapshots[1].active == 0
            && o.snapshots[1].key == 0
    });
    let (&command, _) = commands
        .next()
        .context("sale initial/final snapshots differ")?;
    ensure!(commands.next().is_none(), "ambiguous sale command owner");
    ensure!(
        m.history.store.commands[&command].references == 1,
        "sale command reference count differs"
    );
    let undo = m
        .history
        .store
        .vector(sims_sim::edit_history::List::Undo(m.tool.clipboard))
        .map_err(anyhow::Error::msg)?;
    ensure!(
        m.history.store.allocations[&undo.allocation][..undo.len].contains(&Some(command)),
        "sale command has no Undo owner"
    );
    Ok(command)
}

struct PaidEditSpec {
    store: String,
    source_sha: String,
    house: String,
    identity: Identity,
    funds: i32,
    price: i32,
    purchase: [i32; 2],
    destination: [i32; 2],
    displaced: [i32; 2],
    receipt: Option<(String, String)>,
}

#[test]
#[ignore = "same conditional campaign safe post-Live fixture; actual move/drag-rotate/Escape caller/sale and owned restore"]
fn paid_chair_edit_cycle_from_retained_post_live_keeps_cold_state() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            paid_edit_cycle(PaidEditSpec {
                store: "fixtures/private/player-continuation-fresh-mashuga-20261026-third".into(),
                source_sha: "1ebc4870e912939f0692a0a2c3d09812134366ffb1a3360aa0497c329b2b7190"
                    .into(),
                house: "UserData2/Houses/House09.iff".into(),
                identity: Identity {
                    object: 221,
                    handle: 346,
                },
                funds: 7618,
                price: 80,
                purchase: [376, 488],
                destination: [392, 488],
                displaced: [408, 488],
                receipt: None,
            })
        })?
        .join()
        .expect("paid edit cycle worker panicked")
}

#[test]
#[ignore = "actual Newbie/Goth paid receipt and exact post-Live fixture; separate full native edit caller cycle"]
fn userdata_paid_chair_edit_cycle_keeps_cold_state() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| -> Result<()> {
            let root = repo()?;
            let house = household_cases::source_house()?;
            let receipt_path = std::env::var("SIMS_PLAYER_EDIT_RECEIPT")?;
            let receipt_hash = std::env::var("SIMS_PLAYER_EDIT_RECEIPT_SHA256")?;
            let raw = std::fs::read(root.join(&receipt_path))?;
            ensure!(
                exploration::digest(&raw) == receipt_hash,
                "actual committed edit receipt changed"
            );
            let receipt: serde_json::Value = serde_json::from_slice(&raw)?;
            let identity: Identity = serde_json::from_value(receipt["identity"].clone())?;
            let purchase: [i32; 2] =
                serde_json::from_value(receipt["pointer"]["position"].clone())?;
            let funds = i32::try_from(
                receipt["funds_after"]
                    .as_i64()
                    .context("receipt funds absent")?,
            )?;
            let price = i32::try_from(
                receipt["quoted_price"]
                    .as_i64()
                    .context("receipt quote absent")?,
            )?;
            ensure!(
                price > 0
                    && receipt["funds_before"].as_i64()
                        == Some(i64::from(funds) + i64::from(price)),
                "receipt did not record paid placement"
            );
            let (destination, displaced) = match house {
                "UserData/Houses/House05.iff" => ([456, 392], [568, 456]),
                "UserData/Houses/House07.iff" => ([456, 408], [568, 280]),
                _ => anyhow::bail!("unsupported authored edit lot"),
            };
            paid_edit_cycle(PaidEditSpec {
                store: std::env::var("SIMS_PLAYER_EDIT_INPUT_STORE")?,
                source_sha: std::env::var("SIMS_PLAYER_EDIT_SOURCE_SHA256")?,
                house: house.into(),
                identity,
                funds,
                price,
                purchase,
                destination,
                displaced,
                receipt: Some((receipt_path, receipt_hash)),
            })
        })?
        .join()
        .expect("UserData paid edit worker panicked")
}

fn paid_edit_cycle(spec: PaidEditSpec) -> Result<()> {
    use move_tool::{button, motion};
    let root = repo()?;
    let input = sims_fixtures::Store::open(root.join(&spec.store))?;
    let source = input.load_case("post-live-resumed")?.source.blob;
    ensure!(source.sha256 == spec.source_sha, "post-Live input changed");
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private output required"
    );
    std::fs::create_dir(&out).context("refusing to overwrite edit evidence")?;
    let path = root.join(std::env::var("SIMS_PLAYER_NOTICE_STORE")?);
    ensure!(
        path.starts_with(root.join("fixtures/private")),
        "private new edit store required"
    );
    let store = sims_fixtures::Store::create(path)?;
    let mut a = Household::from_fixture(index(&root)?, &input, &source)?;
    let mut b = a.fork()?;
    let identity = spec.identity.clone();
    let start = a.observe()?;
    ensure!(
        a.session.house.source == spec.house
            && a.mode()? == 2
            && a.session
                .simulation
                .vm
                .world
                .budget
                .as_ref()
                .context("funds absent")?
                .balance
                == spec.funds,
        "retained source contract differs"
    );
    let initial = edit_physical(&a, &identity)?;
    ensure!(
        initial["position"] == json!(spec.purchase) && initial["on_lot"] == true,
        "paid source furniture differs"
    );
    let mut journal = vec![];
    let mut cuts = vec![];
    let mut stage = "initial owned restore";
    write(
        &out,
        "input.json",
        &json!({"source":source,"source_store":spec.store,"identity":identity,"initial":initial,"receipt":spec.receipt,"parameters":{"funds":spec.funds,"actual_purchase_price":spec.price,"purchase":spec.purchase,"destination":spec.destination,"displaced":spec.displaced},"qualification":"separate authored internal edit cycle, not4096 schedule or original parity","keyboard_scope":"actual original Cancel caller dispatched by app Escape; Iced keyboard event delivery not exercised","test_source_sha256":exploration::digest(include_bytes!("household_player_continuation.rs")),"adapter_source_sha256":exploration::digest(include_bytes!("support/player_action_driver.rs")),"binary_sha256":exploration::digest(&std::fs::read(std::env::current_exe()?)?)}),
    )?;
    let result = (|| -> Result<()> {
        write(
            &out,
            "initial-source-denial.json",
            &owned_resolver_receipt(&root, &a)?,
        )?;
        cuts.push(fixture_cut(&root, &store, &a, &mut b, "initial-post-live")?);
        stage = "pause and reenter Buy";
        inputs(
            &out,
            &mut a,
            &mut b,
            vec![
                Command::Pause { paused: true },
                Command::Drain { budget: 256 },
                Command::BeginMode { mode: 1 },
                Command::Drain { budget: 256 },
                Command::EnterMove,
                Command::Drain { budget: 256 },
            ],
            &mut journal,
        )?;
        let pickup = motion::Input {
            position: spec.purchase,
            screen: [230, 300],
            picked: identity.object,
        };
        stage = "pick up paid furniture";
        for kind in [button::Kind::Down, button::Kind::Up] {
            inputs(
                &out,
                &mut a,
                &mut b,
                vec![
                    Command::Button {
                        input: button::Input {
                            kind,
                            pointer: pickup,
                            shift: false,
                        },
                    },
                    Command::Drain { budget: 256 },
                ],
                &mut journal,
            )?;
        }
        held_edit(&a, &identity)?;
        cuts.push(fixture_cut(
            &root,
            &store,
            &a,
            &mut b,
            "held-paid-furniture",
        )?);
        let destination = motion::Input {
            position: spec.destination,
            screen: [246, 316],
            picked: 0,
        };
        stage = "move existing paid furniture";
        inputs(
            &out,
            &mut a,
            &mut b,
            vec![
                Command::Move {
                    pointer: destination,
                },
                Command::Drain { budget: 256 },
            ],
            &mut journal,
        )?;
        let facing = a.session.simulation.vm.world.objects[&identity.object].data[&1];
        let desired = (facing + 2) & 7;
        let rotation = a
            .session
            .host
            .presentation
            .presenter
            .as_ref()
            .context("presenter absent")?
            .view
            .display_rotation as i16;
        let delta = match (desired + rotation * 2) & 7 {
            0 => [14, -14],
            2 => [14, 14],
            4 => [-14, 14],
            6 => [-14, -14],
            _ => anyhow::bail!("noncardinal chair facing"),
        };
        let rotated = motion::Input {
            screen: [
                destination.screen[0] + delta[0],
                destination.screen[1] + delta[1],
            ],
            ..destination
        };
        stage = "drag rotation";
        inputs(
            &out,
            &mut a,
            &mut b,
            vec![
                Command::Button {
                    input: button::Input {
                        kind: button::Kind::Down,
                        pointer: destination,
                        shift: false,
                    },
                },
                Command::Drain { budget: 256 },
                Command::Move { pointer: rotated },
                Command::Drain { budget: 256 },
            ],
            &mut journal,
        )?;
        ensure!(
            a.session.simulation.vm.world.objects[&identity.object].data[&1] == desired,
            "actual drag failed to rotate furniture"
        );
        inputs(
            &out,
            &mut a,
            &mut b,
            vec![
                Command::Button {
                    input: button::Input {
                        kind: button::Kind::Up,
                        pointer: rotated,
                        shift: false,
                    },
                },
                Command::Drain { budget: 256 },
                Command::EscapeMove { pointer: rotated },
                Command::Drain { budget: 256 },
                Command::CheckNoHeld,
            ],
            &mut journal,
        )?;
        let moved = edit_physical(&a, &identity)?;
        ensure!(
            moved["position"] == json!(destination.position)
                && moved["direction"] == desired
                && moved["on_lot"] == true
                && moved["unplaced"] == false,
            "committed move/rotation has incorrect physical state: {moved}"
        );
        ensure!(
            a.session
                .simulation
                .vm
                .world
                .budget
                .as_ref()
                .unwrap()
                .balance
                == spec.funds,
            "moving paid furniture charged funds"
        );
        cuts.push(fixture_cut(
            &root,
            &store,
            &a,
            &mut b,
            "moved-rotated-safe-buy",
        )?);
        stage = "pick up and cancel an uncommitted displacement";
        let pickup = motion::Input {
            picked: identity.object,
            ..destination
        };
        for kind in [button::Kind::Down, button::Kind::Up] {
            inputs(
                &out,
                &mut a,
                &mut b,
                vec![
                    Command::Button {
                        input: button::Input {
                            kind,
                            pointer: pickup,
                            shift: false,
                        },
                    },
                    Command::Drain { budget: 256 },
                ],
                &mut journal,
            )?;
        }
        held_edit(&a, &identity)?;
        let displaced = motion::Input {
            position: spec.displaced,
            screen: [262, 332],
            picked: 0,
        };
        inputs(
            &out,
            &mut a,
            &mut b,
            vec![
                Command::Move { pointer: displaced },
                Command::Drain { budget: 256 },
            ],
            &mut journal,
        )?;
        cuts.push(fixture_cut(
            &root,
            &store,
            &a,
            &mut b,
            "uncommitted-existing-move",
        )?);
        inputs(
            &out,
            &mut a,
            &mut b,
            vec![
                Command::EscapeMove { pointer: displaced },
                Command::Drain { budget: 256 },
                Command::CheckNoHeld,
            ],
            &mut journal,
        )?;
        ensure!(
            edit_physical(&a, &identity)? == moved,
            "Escape caller failed exact physical rollback"
        );
        cuts.push(fixture_cut(
            &root,
            &store,
            &a,
            &mut b,
            "escape-rollback-safe-buy",
        )?);
        stage = "pick up to sell";
        for kind in [button::Kind::Down, button::Kind::Up] {
            inputs(
                &out,
                &mut a,
                &mut b,
                vec![
                    Command::Button {
                        input: button::Input {
                            kind,
                            pointer: pickup,
                            shift: false,
                        },
                    },
                    Command::Drain { budget: 256 },
                ],
                &mut journal,
            )?;
        }
        let held = held_edit(&a, &identity)?;
        let selection = a
            .device
            .furniture_selection()
            .context("sale controls absent")?;
        ensure!(
            selection.sell_allowed && selection.price > 0 && selection.price <= spec.price,
            "actual sale eligibility/value differs"
        );
        let credit = selection.price;
        stage = "actual sale and Undo ownership";
        inputs(
            &out,
            &mut a,
            &mut b,
            vec![
                Command::SellFurniture { held },
                Command::Drain { budget: 256 },
                Command::CheckNoHeld,
            ],
            &mut journal,
        )?;
        let sold = edit_physical(&a, &identity)?;
        ensure!(
            sold["on_lot"] == false
                && sold["position"][0].as_i64().is_some_and(|v| v < 0)
                && sold["position"][1].as_i64().is_some_and(|v| v < 0),
            "sale did not retain off-lot furniture for Undo"
        );
        ensure!(
            a.session
                .simulation
                .vm
                .world
                .budget
                .as_ref()
                .unwrap()
                .balance
                == spec.funds + credit,
            "actual sale credit differs"
        );
        ensure!(
            !a.device
                .scene()
                .draws
                .iter()
                .any(|d| d.object == Some(identity.object)),
            "sold furniture remains rendered"
        );
        let sale_command = edit_sale_owner(&a, &identity, credit)?;
        cuts.push(fixture_cut(
            &root,
            &store,
            &a,
            &mut b,
            "sale-undo-owned-safe-buy",
        )?);
        let before = same(&a, &b)?;
        ensure!(
            a.execute(&Command::SellFurniture { held }, &Limits::default())
                .is_err(),
            "stale sold cursor accepted another sale"
        );
        ensure!(
            same(&a, &b)? == before,
            "rejected stale sale changed state/RNG"
        );
        stage = "post-Live history teardown";
        inputs(
            &out,
            &mut a,
            &mut b,
            vec![
                Command::BeginMode { mode: 2 },
                Command::Drain { budget: 256 },
                Command::CheckMode { mode: 2 },
            ],
            &mut journal,
        )?;
        ensure!(
            a.session
                .simulation
                .vm
                .world
                .scheduler_storage
                .as_ref()
                .and_then(|r| r.object_for_handle(identity.handle))
                .is_none()
                && !a
                    .session
                    .simulation
                    .vm
                    .world
                    .objects
                    .contains_key(&identity.object),
            "Live history close retained sold lifetime"
        );
        if let Some(m) = &a.session.host.presentation.move_tool {
            ensure!(
                m.history
                    .references
                    .by_script_id(identity.object)
                    .map_err(anyhow::Error::msg)?
                    .is_none()
                    && !m.history.objects.0.contains_key(&sale_command)
                    && !m.history.store.commands.contains_key(&sale_command),
                "Live close retained sale history allocations"
            );
        }
        ensure!(
            a.session
                .simulation
                .vm
                .world
                .budget
                .as_ref()
                .unwrap()
                .balance
                == spec.funds + credit
                && !a.session.house_notification_pending(),
            "post-Live sale funds/notification completion differs"
        );
        cuts.push(fixture_cut(
            &root,
            &store,
            &a,
            &mut b,
            "sale-released-post-live",
        )?);
        write(
            &out,
            "final-source-denial.json",
            &owned_resolver_receipt(&root, &a)?,
        )?;
        write(
            &out,
            "sale.json",
            &json!({"identity":identity,"moved":moved,"sold":sold,"credit":credit,"final_funds":spec.funds+credit,"stale_sale_rejected_without_mutation":true}),
        )?;
        ensure!(
            a.observe()?.tick == start.tick,
            "UI edit cycle advanced gameplay time"
        );
        Ok(())
    })();
    if result.is_err()
        && let Ok(blob) = a.checkpoint_fixture(&store)
    {
        let mut case = sims_fixtures::CaseManifest::minimal_import(
            "first-stop",
            "First authored paid edit stop",
            sims_fixtures::Artifact::self_contained(
                blob.clone(),
                sims_fixtures::ArtifactFormat::MessagePack,
                1,
            ),
            vec!["internal exact retained scenario, not original parity".into()],
        );
        case.provenance.origin = sims_fixtures::Origin::Authored;
        store.publish_case(&case)?;
        write(&out, "first-stop.json", &json!({"stage":stage,"blob":blob}))?;
    }
    write(
        &out,
        "report.json",
        &json!({"outcome":if result.is_ok(){"passed"}else{"stopped"},"stage":stage,"source":source,"tick":a.session.simulation.vm.tick,"gameplay_ticks":a.session.simulation.vm.tick-start.tick,"native_parity_ticks":0,"cuts":cuts,"inputs":journal,"store_verification":store.verify_all()?,"stop":result.as_ref().err().map(|e|format!("{e:#}"))}),
    )?;
    result
}

#[test]
#[ignore = "private exact128-tick portal checkpoint; actual Device cursor children and independent managed cuts"]
fn captured_portal_service_survives_real_buy_cursor_lifecycle() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(portal_reproducer)?
        .join()
        .expect("portal regression worker panicked")
}
fn portal_reproducer() -> Result<()> {
    use move_tool::{button, motion};
    let root = repo()?;
    let source=std::env::var("SIMS_CURSOR_PORTAL_CHECKPOINT").context("materialize the exact mashuga-cursor-portal-stop128-20261026 archive and set SIMS_CURSOR_PORTAL_CHECKPOINT")?;
    let source = root.join(source);
    let raw = std::fs::read(&source)
        .with_context(|| format!("read exact portal checkpoint {}", source.display()))?;
    ensure!(
        exploration::digest(&raw) == PORTAL_STOP_SHA,
        "portal stop checkpoint changed"
    );
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private output required"
    );
    std::fs::create_dir(&out).context("refusing to overwrite portal regression evidence")?;
    std::fs::write(
        out.join("test-source.rs"),
        include_bytes!("household_player_continuation.rs"),
    )?;
    std::fs::write(
        out.join("adapter-source.rs"),
        include_bytes!("support/player_action_driver.rs"),
    )?;
    let mut a = Household::from_bytes(index(&root)?, &raw)?;
    let mut b = a.fork()?;
    ensure!(
        a.session.simulation.vm.tick == 180474 && !a.session.clock_pending(),
        "not the captured completed-frame boundary"
    );
    let parked = serde_json::to_vec(&a.session.simulation.vm.threads[&11])?;
    let service = a.session.simulation.vm.threads[&11]
        .service
        .as_ref()
        .context("captured portal service absent")?;
    ensure!(
        service.caller == 196 && service.callee == 45 && service.bhav == 4105,
        "captured portal identity changed"
    );
    let mut cuts = vec![];
    let mut journal = vec![];
    write(
        &out,
        "input.json",
        &json!({"source":source,"source_sha256":PORTAL_STOP_SHA,"fixture_case":"mashuga-cursor-portal-stop128-20261026",
        "tick":180474,"qualification":"real retained Rust portal/cursor regression; not fresh original active-portal parity",
        "test_source_sha256":exploration::digest(include_bytes!("household_player_continuation.rs")),"adapter_source_sha256":exploration::digest(include_bytes!("support/player_action_driver.rs"))}),
    )?;
    let result = (|| -> Result<()> {
        let before = a.observe()?;
        let clock = &a
            .session
            .simulation
            .household
            .as_ref()
            .context("calendar absent")?
            .clock;
        let date = sims_vm::construction::ObjectDate {
            year: clock.year,
            month: clock.month,
            day: clock.day,
        };
        ensure!(
            !a.session
                .simulation
                .begin_cursor_factory(&a.session.program, None, 0, 1, false, date)
                .map_err(anyhow::Error::msg)?,
            "resource-free guard created a cursor"
        );
        ensure!(
            a.observe()? == before,
            "mere cursor guard changed state/RNG"
        );
        same(&a, &b)?;
        inputs(
            &out,
            &mut a,
            &mut b,
            vec![Command::Drain { budget: 256 }],
            &mut journal,
        )?;
        ensure!(
            serde_json::to_vec(&a.session.simulation.vm.threads[&11])? == parked,
            "Enter changed parked portal service/frames"
        );
        cut(&root, &out, &a, &mut b, "entered-unpaid-preview", &mut cuts)?;
        let mut point = None;
        for position in [
            [376, 488],
            [424, 520],
            [344, 520],
            [312, 552],
            [280, 552],
            [456, 552],
        ] {
            let pointer = motion::Input {
                position,
                screen: [230, 300],
                picked: 0,
            };
            inputs(
                &out,
                &mut a,
                &mut b,
                vec![Command::Move { pointer }, Command::Drain { budget: 256 }],
                &mut journal,
            )?;
            let h = a
                .session
                .host
                .presentation
                .move_tool
                .as_ref()
                .context("Move absent")?
                .tool
                .held
                .context("cursor absent")?;
            let world = &a.session.simulation.vm.world;
            let cursor = world
                .scheduler_storage
                .as_ref()
                .and_then(|r| r.object_for_handle(h))
                .context("cursor lifetime absent")?;
            if world.objects[&cursor]
                .cursor
                .as_ref()
                .is_some_and(|c| c.valid != 0)
            {
                point = Some((pointer, cursor));
                break;
            }
        }
        let (pointer, cursor) =
            point.context("authored preview positions all refused; no committed-edit claim")?;
        let world = &a.session.simulation.vm.world;
        let handle = world.objects[&cursor]
            .cursor
            .as_ref()
            .and_then(|c| c.underlying)
            .context("underlying furniture absent")?;
        let furniture = world
            .scheduler_storage
            .as_ref()
            .and_then(|r| r.object_for_handle(handle))
            .context("furniture lifetime absent")?;
        let identity = Identity::capture(&a.session, furniture)?;
        let funds = world.budget.as_ref().context("budget absent")?.balance;
        let price = a
            .session
            .host
            .presentation
            .move_tool
            .as_ref()
            .unwrap()
            .tool
            .value90 as i32;
        cut(&root, &out, &a, &mut b, "hover-valid", &mut cuts)?;
        inputs(
            &out,
            &mut a,
            &mut b,
            vec![
                Command::Button {
                    input: button::Input {
                        kind: button::Kind::Down,
                        pointer,
                        shift: false,
                    },
                },
                Command::Drain { budget: 256 },
                Command::Button {
                    input: button::Input {
                        kind: button::Kind::Up,
                        pointer,
                        shift: false,
                    },
                },
                Command::Drain { budget: 256 },
            ],
            &mut journal,
        )?;
        ensure!(
            a.session
                .host
                .presentation
                .move_tool
                .as_ref()
                .unwrap()
                .button_hint()
                == Some(button::Hint::Spent(price)),
            "real drop did not publish Spent"
        );
        ensure!(
            a.session
                .simulation
                .vm
                .world
                .budget
                .as_ref()
                .unwrap()
                .balance
                == funds - price,
            "real drop budget differs from actual quote"
        );
        ensure!(
            Identity::capture(&a.session, furniture)? == identity
                && !a.session.simulation.unplaced.contains(&furniture),
            "paid furniture lifetime/placement absent"
        );
        cut(&root, &out, &a, &mut b, "paid-cursor-release", &mut cuts)?;
        if a.session
            .host
            .presentation
            .move_tool
            .as_ref()
            .is_some_and(|m| m.tool.held.is_some())
        {
            inputs(
                &out,
                &mut a,
                &mut b,
                vec![
                    Command::Button {
                        input: button::Input {
                            kind: button::Kind::Cancel,
                            pointer,
                            shift: false,
                        },
                    },
                    Command::Drain { budget: 256 },
                ],
                &mut journal,
            )?;
        }
        inputs(
            &out,
            &mut a,
            &mut b,
            vec![
                Command::CheckNoHeld,
                Command::BeginMode { mode: 2 },
                Command::Drain { budget: 256 },
                Command::CheckMode { mode: 2 },
            ],
            &mut journal,
        )?;
        ensure!(
            serde_json::to_vec(&a.session.simulation.vm.threads[&11])? == parked,
            "cursor lifecycle changed parked portal service/frames"
        );
        ensure!(
            Identity::capture(&a.session, furniture)? == identity,
            "post-Live cleanup deleted paid furniture"
        );
        ensure!(
            a.session.simulation.vm.tick == 180474,
            "UI continuation advanced gameplay"
        );
        cut(
            &root,
            &out,
            &a,
            &mut b,
            "post-live-portal-preserved",
            &mut cuts,
        )?;
        Ok(())
    })();
    write(
        &out,
        "report.json",
        &json!({"outcome":if result.is_ok(){"passed"}else{"failed"},"tick":a.session.simulation.vm.tick,
        "inputs":journal.len(),"cold_restores":cuts.len(),"stop":result.as_ref().err().map(|e|format!("{e:#}")),"native_parity_ticks":0,
        "parked_thread_unchanged":serde_json::to_vec(&a.session.simulation.vm.threads[&11])?==parked}),
    )?;
    if result.is_err()
        && let Ok(bytes) = a.checkpoint()
    {
        std::fs::write(out.join("first-stop-managed.json"), bytes)?;
    }
    result
}
