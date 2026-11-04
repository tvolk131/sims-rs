//! Source-only UserData migration and CPU household probes. No evolved VM,
//! client, device, old CRT/RNG or neighborhood-order state is imported.
use super::{
    Boundary, Frame, Household, Identity, index, owned_resolver_receipt, ready_frame, repo, same,
    write,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sims_fixtures::{
    Artifact, ArtifactFormat, BlobRef, CaseManifest, Coverage, Origin, Reference, ReferenceKind,
    Store,
};
use sims_render::{
    camera_device::Device,
    dialog::client::{Client, Inputs},
    original_house::OriginalHouse,
};
use sims_runtime::{
    Session,
    exploration::{self, Driver},
    original_session::{Options, completion},
};
use std::{path::Path, sync::Arc, time::Instant};

const BUNDLE: &str = "cache/sale-20261022-seventh/newbie/sold-returned-live.json";
const BUNDLE_SHA: &str = "ba27dc2513e2c87a63fd74bfd79588df5ae452dbce48a833cc7a03b6a2cc1d29";
const NEIGHBORHOOD_SHA: &str = "e585615fed1f9ddfb15ff8f5ebe7a09ea48a1369e9a3106f8331d3d9f5dde323";

struct Spec {
    name: &'static str,
    house: &'static str,
    house_sha: &'static str,
}
fn spec() -> Result<Spec> {
    Ok(match std::env::var("SIMS_PLAYER_HOUSEHOLD")?.as_str() {
        "newbie" => Spec {
            name: "newbie",
            house: "UserData/Houses/House07.iff",
            house_sha: "5b162c81135612acfa65628438df2246cd25ad8cb15e2646a28bb32e38de5133",
        },
        "goth" => Spec {
            name: "goth",
            house: "UserData/Houses/House05.iff",
            house_sha: "a6d18d81c5072c507b118b68598ee12ea57403634d6507c39b7a04ad4c6cdd99",
        },
        _ => anyhow::bail!("explicit newbie/goth source scenario required"),
    })
}
pub(super) fn source_house() -> Result<&'static str> {
    Ok(spec()?.house)
}

// Ignored JSON fields are skipped by Serde. In particular, the evolved
// simulation, view, client and device are not even decoded into engine types.
#[derive(Deserialize)]
struct SourceBundle {
    session: SourceSession,
}
#[derive(Deserialize)]
struct SourceSession {
    version: u32,
    managed: SourceContent,
}
#[derive(Deserialize)]
struct SourceContent {
    schema: u32,
    owned: sims_content::owned_resources::Neighborhood,
}

fn options() -> Options {
    Options{application_context:None,initial_process_profile:Some(sims_content::autonomy_resources::InitialProfile::CompleteCollection191ObservedInitial20261024),scheduler:sims_runtime::original_session::RegistryScheduler,native_load_completion:true,native_postload:true,routing:sims_sim::lot_route::path::Policy::InferredRectangles,crt_seed:0x51a5cafe,defer_indicators:false}
}

fn save_cut(
    root: &Path,
    store: &Store,
    a: &Household,
    b: &mut Household,
    label: &str,
) -> Result<BlobRef> {
    let before = same(a, b)?;
    let blob = a.checkpoint_fixture(store)?;
    let restored = Household::from_fixture(index(root)?, &Store::open(store.root())?, &blob)?;
    ensure!(
        restored.observe()? == before,
        "independent managed restore changed {label}"
    );
    *b = restored;
    let artifact = Artifact::self_contained(blob.clone(), ArtifactFormat::MessagePack, 1);
    let mut case=CaseManifest::minimal_import(label,format!("Authored {} {label}",a.session.house.source),artifact.clone(),vec!["Exact source-only migration and matching immutable CC assets; no observed original input schedule/parity".into()]);
    case.provenance.origin = Origin::Authored;
    case.provenance.source_locator = Some(format!(
        "source-only-owned-bundle:sha256:{BUNDLE_SHA}#{}",
        a.session.house.source
    ));
    case.coverage=Coverage::Component{scope:"CPU Session/Client/Device owned-state cold restore".into(),limitations:vec!["Declared new-process CRT/clock, InferredRectangles, no guessed neighborhood input; no original parity".into()]};
    case.references=vec![Reference{label:label.into(),kind:ReferenceKind::Resumable,artifact,resume_contract:Some("sims-render player Checkpoint/msgpack-v1; fresh immutable-only base; exact owned bytes and full retained-state/RNG restoration proven".into()),boundary:sims_fixtures::Boundary::Phase{name:label.into(),tick:Some(before.tick)}}];
    store.publish_case(&case)?;
    Ok(blob)
}

fn resident_discovery(a: &Household, require_present: bool) -> Result<serde_json::Value> {
    let s = &a.session;
    let household = s
        .house
        .household
        .as_ref()
        .context("fresh imported household absent")?;
    let mut residents = vec![];
    for r in &household.residents {
        let ids = s
            .simulation
            .vm
            .world
            .objects
            .iter()
            .filter(|(_, o)| {
                o.person.is_some() && o.definition.as_ref().is_some_and(|d| d.guid == r.guid)
            })
            .map(|(&id, _)| Identity::capture(s, id))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            ids.len() <= 1 && (!require_present || ids.len() == 1),
            "imported resident {} has ambiguous/missing runtime lifetime",
            r.name
        );
        residents.push(json!({"name":r.name,"guid":r.guid,"neighbor":r.neighbor,"character_source":r.character_source,"identity":ids.first(),"runtime_count":ids.len()}));
    }
    let objects=s.simulation.vm.world.objects.iter().filter(|(_,o)|o.person.is_none() && o.cursor.is_none()).filter_map(|(&id,o)|o.definition.as_ref().map(|d|(id,d))).map(|(id,d)|->Result<_>{Ok(json!({"identity":Identity::capture(s,id)?,"definition":d,"position":s.simulation.vm.world.positions.get(&id)}))}).collect::<Result<Vec<_>>>()?;
    Ok(
        json!({"household":household.name,"residents":residents,"objects":objects,"selected_person":s.simulation.native_selected_person().map_err(anyhow::Error::msg)?}),
    )
}

fn modal_discovery(a: &Household) -> Result<serde_json::Value> {
    let vm = &a.session.simulation.vm;
    let mut records = vec![];
    ensure!(
        vm.world.dialogs.records.len() <= 16,
        "unexpectedly large startup modal set"
    );
    for record in &vm.world.dialogs.records {
        if record.status > 1 {
            continue;
        }
        let thread = *vm
            .object_threads
            .get(&record.caller)
            .context("dialog has no suspended caller thread")?;
        let continuation = vm
            .threads
            .get(&thread)
            .context("dialog continuation absent")?;
        let resource = &a.session.index().resources[a.session.index().find(&record.source)?];
        let routine = a
            .session
            .program
            .modules
            .get(&record.origin.source)
            .and_then(|m| m.routines.get(&record.origin.bhav))
            .context("dialog origin routine absent")?;
        let mut active = record.clone();
        active.status = 1;
        records.push(json!({"record":record,"expected_active_record":active,"record_sha256":exploration::digest_serialized(&active)?,"caller":Identity::capture(&a.session,record.caller)?,"thread":thread,"continuation_sha256":exploration::digest_serialized(continuation)?,"continuation":continuation,"resource_sha256":exploration::digest(&resource.read()?),"bhav_body_sha256":exploration::digest_serialized(&routine.body)?,"bhav_body":routine.body}));
    }
    Ok(
        json!({"active":vm.world.dialogs.modal,"records":records,"responses_submitted":0,"scope":"read-only discovery; normalization of expected active status is a pin only, never VM mutation"}),
    )
}

pub(super) fn fresh_probe() -> Result<()> {
    let root = repo()?;
    let spec = spec()?;
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private output required"
    );
    std::fs::create_dir(&out).context("refusing to overwrite fresh-household evidence")?;
    let path = root.join(std::env::var("SIMS_PLAYER_NOTICE_STORE")?);
    ensure!(
        path.starts_with(root.join("fixtures/private")),
        "private new store required"
    );
    let store = Store::create(path)?;
    let raw = std::fs::read(root.join(BUNDLE))?;
    ensure!(
        exploration::digest(&raw) == BUNDLE_SHA,
        "raw source bundle changed"
    );
    let source: SourceBundle = serde_json::from_slice(&raw)?;
    ensure!(
        source.session.version == 2 && source.session.managed.schema == 1,
        "owned source schema differs"
    );
    let owned = source.session.managed.owned;
    owned.validate()?;
    ensure!(
        owned.entries.len() == 35 && owned.directory == "UserData",
        "owned source inventory differs"
    );
    let hashes = owned.hashes();
    ensure!(
        hashes.get(spec.house).is_some_and(|h| h == spec.house_sha)
            && hashes
                .get("UserData/Neighborhood.iff")
                .is_some_and(|h| h == NEIGHBORHOOD_SHA),
        "fresh house/neighborhood source pair differs"
    );
    let immutable = index(&root)?;
    let mounted = Arc::new(owned.mount(&immutable)?);
    for e in &owned.entries {
        ensure!(
            immutable.find(&e.locator).is_err(),
            "starting save visible on immutable base"
        );
        let r = &mounted.resources[mounted.find(&e.locator)?];
        ensure!(
            r.owned.is_some()
                && std::fs::File::open(&r.source).is_err()
                && exploration::digest(&r.read()?) == e.sha256,
            "source-only owned mount differs or falls back to original path"
        );
    }
    let owned_blob = store.put_value(&owned)?;
    let mut source_case=CaseManifest::minimal_import("owned-source-bundle",format!("Raw source-only UserData input for {}",spec.name),Artifact::self_contained(owned_blob.clone(),ArtifactFormat::MessagePack,1),vec!["Only raw35 hash-verified owned sources are extracted; evolved sold-returned-live simulation/client/device and its CRT/RNG/order are discarded, not restored".into()]);
    source_case.provenance.origin = Origin::Imported;
    source_case.provenance.source_locator = Some(format!("{BUNDLE}#sha256:{BUNDLE_SHA}"));
    store.publish_case(&source_case)?;
    write(
        &out,
        "input.json",
        &json!({"household":spec.name,"house":spec.house,"source_bundle":BUNDLE,"source_bundle_sha256":BUNDLE_SHA,"extraction":"only session.managed.owned; evolved simulation/client/device/catalog not decoded or restored","owned_source_hashes":hashes,"owned_source_blob":owned_blob,"options":options(),"frontend_inputs":{"width":1000,"height":740,"device_clock":0,"simulation_clock":1,"completion_mode":0,"controller_suppressed":0,"opening_selection":0},"qualification":"new authored Rust process, not original-process reconstruction or strict parity","helper_sha256":exploration::digest(include_bytes!("household_cases.rs")),"adapter_sha256":exploration::digest(include_bytes!("player_action_driver.rs")),"executable_sha256":exploration::digest(&std::fs::read(std::env::current_exe()?)?)}),
    )?;
    std::fs::write(
        out.join("helper-source.rs"),
        include_bytes!("household_cases.rs"),
    )?;
    let mut stage = "fresh Session load";
    let mut loaded = None;
    let mut player = None;
    let mut rows = vec![];
    let mut cuts = vec![];
    let mut polls = 0;
    let result = (|| -> Result<&str> {
        loaded = Some(Session::load(&mounted, spec.house, options())?);
        let s = loaded.as_mut().unwrap();
        write(
            &out,
            "load.json",
            &json!({"tick":s.simulation.vm.tick,"sources":s.house.sources,"options":s.options,"program_fingerprint":s.program.fingerprint(),"modules":s.program.modules.keys().collect::<Vec<_>>(),"family_metadata":s.simulation.vm.world.person.as_ref().map(|p|&p.family_metadata),"neighborhood_input":s.host.presentation.neighborhood_input}),
        )?;
        stage = "postload and CPU Device open";
        let mut device = Device::open_for_household(
            s,
            1000,
            740,
            0,
            completion::Config {
                controller_suppressed: 0,
                mode: 0,
            },
        )?;
        stage = "new Client and portrait lookup";
        let mut client = Client::new(s)?;
        let size = s
            .simulation
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("architecture absent")?
            .size as i32;
        client.bind_resident_buttons(s, Inputs::camera(OriginalHouse::native_camera(s)?, size)?)?;
        for _ in 0..10000 {
            if !client.ui_lookup_pending() {
                break;
            }
            client.advance_ui_lookup(s, 256)?;
        }
        ensure!(
            !client.ui_lookup_pending(),
            "portrait startup budget exceeded"
        );
        stage = "opening selection/Live and explicit clock";
        device.publish_opening_household(s, 0)?;
        device.publish_live_interface(s)?;
        client.bind_live_panel(s)?;
        client.follow_resident_portraits()?;
        s.bind_clock(1)?;
        let controls = &mut s
            .host
            .presentation
            .transport
            .as_mut()
            .context("transport absent")?
            .controls;
        controls
            .set_speed(controls.speed)
            .map_err(anyhow::Error::msg)?;
        client.collect(s)?;
        device.refresh(s)?;
        player = Some(Household::from_fresh_owners(
            mounted.clone(),
            loaded.take().unwrap(),
            device,
            client,
        ));
        let a = player.as_mut().unwrap();
        stage = "actual Move/tool/panel binding";
        a.bind_fresh_play_controls()?;
        write(&out, "discovery.json", &resident_discovery(a, true)?)?;
        write(
            &out,
            "owned-resolver.json",
            &owned_resolver_receipt(&root, a)?,
        )?;
        let mut b = a.fork()?;
        let start = a.observe()?.tick;
        cuts.push(save_cut(&root, &store, a, &mut b, "fresh-opening")?);
        stage = "small elapsed-clock warm/cold probe; no modal answers";
        let started = Instant::now();
        while rows.len() < 8 {
            let x = ready_frame(a, &mut polls, started, 600)?;
            let y = ready_frame(&mut b, &mut polls, started, 600)?;
            match (x, y) {
                (Frame::Boundary(Boundary::Tick), Frame::Boundary(Boundary::Tick)) => {
                    let state = same(a, &b)?;
                    ensure!(
                        state.tick == start + rows.len() as u64 + 1,
                        "completed tick skipped"
                    );
                    rows.push(state);
                }
                (Frame::Pending, Frame::Pending)
                | (Frame::Complete, Frame::Complete)
                | (
                    Frame::Boundary(Boundary::PartialPass),
                    Frame::Boundary(Boundary::PartialPass),
                ) => {}
                (Frame::Boundary(Boundary::Dialog), Frame::Boundary(Boundary::Dialog)) => {
                    same(a, &b)?;
                    write(&out, "modal-discovery.json", &modal_discovery(a)?)?;
                    cuts.push(save_cut(
                        &root,
                        &store,
                        a,
                        &mut b,
                        "first-unanswered-modal",
                    )?);
                    return Ok("stopped_at_unanswered_modal");
                }
                _ => anyhow::bail!("unhandled/different semantic household boundary"),
            }
        }
        cuts.push(save_cut(&root, &store, a, &mut b, "eight-completed-ticks")?);
        Ok("eight_completed_ticks")
    })();
    if result.is_err() {
        let blob = if let Some(a) = player.as_ref() {
            a.checkpoint_fixture(&store)
                .or_else(|_| store.put_value(&a.session.save()))?
        } else if let Some(s) = loaded.as_ref() {
            store.put_value(&s.save())?
        } else {
            owned_blob
        };
        let mut case = CaseManifest::minimal_import(
            "first-stop",
            "First exact fresh migration/service stop",
            Artifact::self_contained(blob.clone(), ArtifactFormat::MessagePack, 1),
            vec![
                "Stop artifact may be research Session only; no resume/parity qualification".into(),
            ],
        );
        case.provenance.origin = Origin::Authored;
        store.publish_case(&case)?;
        write(
            &out,
            "first-stop.json",
            &json!({"stage":stage,"blob":blob,"managed_player":player.as_ref().is_some_and(|a|a.checkpoint_fixture(&store).is_ok())}),
        )?;
    }
    write(&out, "ticks.json", &rows)?;
    write(
        &out,
        "report.json",
        &json!({"household":spec.name,"outcome":result.as_ref().ok(),"stage":stage,"completed_ticks":rows.len(),"native_parity_ticks":0,"cuts":cuts,"polls":polls,"stop":result.as_ref().err().map(|e|format!("{e:#}")),"verification":store.verify_all()?}),
    )?;
    result.map(|_| ())
}

#[derive(Clone, Deserialize, Serialize)]
struct ModalPin {
    id: u64,
    caller: Identity,
    thread: u64,
    record_sha256: String,
    continuation_sha256: String,
    resource_sha256: String,
    bhav_body_sha256: String,
    offered_result: u32,
}
#[derive(Deserialize, Serialize)]
struct AuthoredInputs {
    schema: u32,
    house: String,
    starting_checkpoint_sha256: String,
    /// A finite pre-inspected input vector, never filled from running state.
    dialogs: Vec<ModalPin>,
    #[serde(default)]
    control_effects: Vec<super::support::ControlEffectPin>,
}

/// One inspected constructor and a bounded ordered vector of offered inputs. This is an
/// authored input schedule, never an automatic policy for future dialogs.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RuntimeDialogPlan {
    schema: u32,
    house: String,
    starting_checkpoint_sha256: String,
    constructor: super::support::DialogConstructionPin,
    #[serde(default)]
    additional_constructors: Vec<super::support::DialogConstructionPin>,
    dialogs: Vec<ModalPin>,
}
pub(super) struct RuntimeDialogSchedule {
    plan: RuntimeDialogPlan,
    path: String,
    sha256: String,
    constructors: usize,
    responses: usize,
}
impl RuntimeDialogSchedule {
    pub(super) fn load(root: &Path, house: &str, source: &str) -> Result<Option<Self>> {
        let Ok(path) = std::env::var("SIMS_PLAYER_RUNTIME_DIALOG_INPUT") else {
            ensure!(
                std::env::var("SIMS_PLAYER_RUNTIME_DIALOG_INPUT_SHA256").is_err(),
                "runtime dialog hash requires its input file"
            );
            return Ok(None);
        };
        let sha256 = std::env::var("SIMS_PLAYER_RUNTIME_DIALOG_INPUT_SHA256")?;
        let raw = std::fs::read(root.join(&path))?;
        ensure!(
            raw.len() <= 32768 && exploration::digest(&raw) == sha256,
            "finite runtime dialog bytes changed"
        );
        let plan: RuntimeDialogPlan = serde_json::from_slice(&raw)?;
        ensure!(
            plan.schema == 1 && plan.house == house && plan.starting_checkpoint_sha256 == source,
            "runtime dialog starting-source contract differs"
        );
        ensure!(
            (1..=16).contains(&plan.dialogs.len())
                && plan
                    .dialogs
                    .iter()
                    .map(|p| p.id)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == plan.dialogs.len(),
            "finite runtime response identities are absent, repeated or unbounded"
        );
        ensure!(
            plan.additional_constructors.len() < 16
                && std::iter::once(&plan.constructor)
                    .chain(plan.additional_constructors.iter())
                    .map(|p| p.request_id)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == 1 + plan.additional_constructors.len(),
            "finite runtime constructors are repeated or unbounded"
        );
        ensure!(
            plan.constructor.request_id == plan.dialogs[0].id
                && plan.constructor.thread == plan.dialogs[0].thread
                && plan.constructor.caller == plan.dialogs[0].caller,
            "constructor and offered input owners differ"
        );
        Ok(Some(Self {
            plan,
            path,
            sha256,
            constructors: 0,
            responses: 0,
        }))
    }
    pub(super) fn manifest(&self) -> serde_json::Value {
        json!({"path":self.path,"sha256":self.sha256,"plan":self.plan,"constructors_consumed":self.constructors,"responses_consumed":self.responses,"qualification":"finite exact authored source-pinned responses; never an observed original input schedule or automatic modal acknowledgement"})
    }
    pub(super) fn retain_exact_input(&self, root: &Path, out: &Path) -> Result<()> {
        let raw = std::fs::read(root.join(&self.path))?;
        ensure!(
            exploration::digest(&raw) == self.sha256,
            "runtime input changed before receipt retention"
        );
        std::fs::write(out.join("runtime-dialog-plan.json"), raw)?;
        Ok(())
    }
    pub(super) fn constructor(
        &self,
        a: &Household,
        thread: u64,
    ) -> Result<super::support::DialogConstructionPin> {
        let pin = if self.constructors == 0 {
            Some(&self.plan.constructor)
        } else {
            self.plan.additional_constructors.get(self.constructors - 1)
        }
        .context("unexpected or repeated runtime constructor")?;
        ensure!(
            pin.thread == thread && a.inspect_dialog_construction(thread)? == *pin,
            "uninterrupted replay did not reach the exact inspected constructor"
        );
        Ok(pin.clone())
    }
    pub(super) fn constructed(&mut self) {
        self.constructors += 1;
    }
    pub(super) fn validate_dialog(&self, a: &Household) -> Result<sims_vm::dialog::Record> {
        ensure!(
            self.constructors > 0 && self.responses < self.plan.dialogs.len(),
            "unexpected or repeated runtime modal"
        );
        validate_modal(a, &self.plan.dialogs[self.responses])
    }
    pub(super) fn answer(
        &mut self,
        a: &mut Household,
        b: &mut Household,
    ) -> Result<serde_json::Value> {
        let record = self.validate_dialog(a)?;
        ensure!(
            self.validate_dialog(b)? == record,
            "cold runtime offered modal differs"
        );
        let before = same(a, b)?;
        let pin = self.plan.dialogs[self.responses].clone();
        a.session
            .simulation
            .vm
            .answer_dialog(pin.id, pin.offered_result)
            .map_err(anyhow::Error::msg)?;
        b.session
            .simulation
            .vm
            .answer_dialog(pin.id, pin.offered_result)
            .map_err(anyhow::Error::msg)?;
        let after = same(a, b)?;
        ensure!(
            after.tick == before.tick && after.rng == before.rng,
            "offered answer advanced time/RNG"
        );
        let ordinal = self.responses;
        self.responses += 1;
        Ok(
            json!({"kind":"exact_offered_dialog_response","ordinal":ordinal,"pin":pin,"record":record,"before":before,"after":after,"complete":true}),
        )
    }
    pub(super) fn response_index(&self) -> usize {
        self.responses
    }
    pub(super) fn constructor_index(&self) -> usize {
        self.constructors
    }
    pub(super) fn complete(&self) -> bool {
        self.constructors == 1 + self.plan.additional_constructors.len()
            && self.responses == self.plan.dialogs.len()
    }
}
fn validate_modal(a: &Household, pin: &ModalPin) -> Result<sims_vm::dialog::Record> {
    let vm = &a.session.simulation.vm;
    ensure!(
        vm.world.dialogs.modal == Some(pin.id),
        "unexpected modal/order"
    );
    validate_modal_record(a, pin, 1)
}
fn validate_modal_record(
    a: &Household,
    pin: &ModalPin,
    expected_status: u32,
) -> Result<sims_vm::dialog::Record> {
    let vm = &a.session.simulation.vm;
    let r = vm
        .world
        .dialogs
        .records
        .iter()
        .find(|r| r.id == pin.id)
        .context("pinned modal absent")?;
    // A queued constructor result is normalized only in this small borrowed
    // record comparison. VM dialog status is never edited by the harness.
    let mut active = r.clone();
    active.status = 1;
    ensure!(
        r.status == expected_status
            && r.caller == pin.caller.object
            && exploration::digest_serialized(&active)? == pin.record_sha256,
        "pinned dialog content/origin/offers changed"
    );
    ensure!(
        Identity::capture(&a.session, r.caller)? == pin.caller
            && vm.object_threads.get(&r.caller) == Some(&pin.thread),
        "pinned caller lifetime/thread changed"
    );
    ensure!(
        vm.threads.get(&pin.thread).is_some_and(
            |t| exploration::digest_serialized(t).is_ok_and(|h| h == pin.continuation_sha256)
        ),
        "pinned suspended continuation changed"
    );
    ensure!(
        r.buttons.iter().any(|b| b.result == pin.offered_result),
        "authored result is not offered"
    );
    let resource = &a.session.index().resources[a.session.index().find(&r.source)?];
    let routine = a
        .session
        .program
        .modules
        .get(&r.origin.source)
        .and_then(|m| m.routines.get(&r.origin.bhav))
        .context("pinned source BHAV absent")?;
    ensure!(
        exploration::digest(&resource.read()?) == pin.resource_sha256
            && exploration::digest_serialized(&routine.body)? == pin.bhav_body_sha256,
        "pinned original resource/BHAV changed"
    );
    Ok(r.clone())
}

pub(super) fn continue_probe() -> Result<()> {
    let root = repo()?;
    let spec = spec()?;
    let input_path = std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_STORE")?;
    let input_case = std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_CASE")?;
    let expected = std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_SHA256")?;
    let input = Store::open(root.join(&input_path))?;
    let source = input.load_case(&input_case)?;
    ensure!(
        source.source.blob.sha256 == expected
            && source.source.format == ArtifactFormat::MessagePack
            && source.references.iter().any(
                |r| r.kind == ReferenceKind::Resumable && r.artifact.blob == source.source.blob
            ),
        "retained player source/contract differs"
    );
    let pin_path = std::env::var("SIMS_PLAYER_AUTHORED_DIALOG_INPUTS")?;
    let pin_sha = std::env::var("SIMS_PLAYER_AUTHORED_DIALOG_INPUTS_SHA256")?;
    let raw = std::fs::read(root.join(&pin_path))?;
    ensure!(
        exploration::digest(&raw) == pin_sha,
        "finite authored dialog input file changed"
    );
    let planned: AuthoredInputs = serde_json::from_slice(&raw)?;
    ensure!(
        planned.schema == 1
            && planned.house == spec.house
            && planned.starting_checkpoint_sha256 == expected
            && (1..=16).contains(&(planned.dialogs.len() + planned.control_effects.len())),
        "finite authored input scope differs"
    );
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private output required"
    );
    std::fs::create_dir(&out).context("refusing to overwrite retained continuation")?;
    let path = root.join(std::env::var("SIMS_PLAYER_NOTICE_STORE")?);
    ensure!(
        path.starts_with(root.join("fixtures/private")),
        "private new store required"
    );
    let store = Store::create(path)?;
    let mut a = Household::from_fixture(index(&root)?, &input, &source.source.blob)?;
    ensure!(
        a.session.house.source == spec.house,
        "retained household differs"
    );
    let mut b = a.fork()?;
    let mut rows = vec![];
    let mut cuts = vec![];
    let mut receipts = vec![];
    let mut polls = 0;
    let start = a.observe()?.tick;
    write(
        &out,
        "input.json",
        &json!({"household":spec.name,"source_store":input_path,"source_case":input_case,"source":source.source.blob,"authored_dialog_inputs":planned,"authored_input_file":pin_path,"authored_input_sha256":pin_sha,"qualification":"finite authored input, not observed original schedule/parity","helper_sha256":exploration::digest(include_bytes!("household_cases.rs")),"adapter_sha256":exploration::digest(include_bytes!("player_action_driver.rs")),"executable_sha256":exploration::digest(&std::fs::read(std::env::current_exe()?)?)}),
    )?;
    std::fs::write(
        out.join("helper-source.rs"),
        include_bytes!("household_cases.rs"),
    )?;
    std::fs::write(out.join("authored-dialog-plan.json"), &raw)?;
    let result = (|| -> Result<&str> {
        write(
            &out,
            "owned-resolver.json",
            &owned_resolver_receipt(&root, &a)?,
        )?;
        if let Some(pin) = planned.dialogs.first()
            && a.session.simulation.vm.world.dialogs.modal.is_none()
        {
            validate_modal_record(&a, pin, 0)?;
            validate_modal_record(&b, pin, 0)?;
            cuts.push(save_cut(
                &root,
                &store,
                &a,
                &mut b,
                "queued-dialog-before-activation",
            )?);
            let before = same(&a, &b)?;
            let started = Instant::now();
            let mut steps = vec![];
            let mut active = false;
            for pass in 0..1000 {
                let prior = same(&a, &b)?;
                let scope_before = serde_json::to_value(&a.session.simulation.vm.lot_rng_scope)?;
                let x = ready_frame(&mut a, &mut polls, started, 600)?;
                let y = ready_frame(&mut b, &mut polls, started, 600)?;
                let after = same(&a, &b)?;
                let scope_after = serde_json::to_value(&a.session.simulation.vm.lot_rng_scope)?;
                steps.push(json!({"pass":pass,"before":prior,"state":after,"lot_scope_before":scope_before,"lot_scope_after":scope_after,"warm":format!("{x:?}"),"cold":format!("{y:?}"),"note":"raw global RNG may restore its saved owner at a partial pass; no value is injected"}));
                write(&out, "dialog-activation-steps.json", &steps)?;
                match (x, y) {
                    (Frame::Boundary(Boundary::Dialog), Frame::Boundary(Boundary::Dialog)) => {
                        ensure!(
                            after.tick == prior.tick,
                            "modal activation advanced gameplay time"
                        );
                        if after.rng != prior.rng {
                            ensure!(
                                scope_after.is_null()
                                    && scope_before["saved_global"].as_u64().is_some()
                                    && scope_before["saved_global"].as_u64()
                                        == after.rng[0].map(u64::from)
                                    && after.rng[1] == prior.rng[0]
                                    && after.rng[2] == prior.rng[2],
                                "modal prefix changed RNG beyond its retained native scope return"
                            );
                        }
                        validate_modal(&a, pin)?;
                        validate_modal(&b, pin)?;
                        active = true;
                        break;
                    }
                    (Frame::Complete, Frame::Complete)
                    | (
                        Frame::Boundary(Boundary::PartialPass),
                        Frame::Boundary(Boundary::PartialPass),
                    ) => ensure!(
                        after.tick == prior.tick,
                        "partial/UI return advanced gameplay time"
                    ),
                    (Frame::Boundary(Boundary::Tick), Frame::Boundary(Boundary::Tick)) => {
                        ensure!(
                            after.tick == start + rows.len() as u64 + 1 && rows.len() < 8,
                            "pre-activation completed tick escaped bounded sequence"
                        );
                        rows.push(after);
                        write(&out, "ticks.json", &rows)?;
                    }
                    _ => {
                        anyhow::bail!("unexpected boundary before pinned queued dialog activation")
                    }
                }
            }
            ensure!(
                same(&a, &b)?.tick >= before.tick,
                "queued dialog moved backwards"
            );
            ensure!(active, "bounded queued dialog activation did not complete");
            cuts.push(save_cut(
                &root,
                &store,
                &a,
                &mut b,
                "queued-dialog-activated-unanswered",
            )?);
        }
        for (ordinal, pin) in planned.dialogs.iter().enumerate() {
            let record = validate_modal(&a, pin)?;
            ensure!(
                validate_modal(&b, pin)? == record,
                "cold pinned modal differs"
            );
            let before = save_cut(
                &root,
                &store,
                &a,
                &mut b,
                &format!("dialog-{ordinal}-before"),
            )?;
            let state = same(&a, &b)?;
            receipts.push(json!({"ordinal":ordinal,"tick":state.tick,"pin":pin,"record":record,"before":before,"complete":false}));
            write(&out, "ordered-inputs.json", &receipts)?;
            a.session
                .simulation
                .vm
                .answer_dialog(pin.id, pin.offered_result)
                .map_err(anyhow::Error::msg)?;
            b.session
                .simulation
                .vm
                .answer_dialog(pin.id, pin.offered_result)
                .map_err(anyhow::Error::msg)?;
            let after = same(&a, &b)?;
            ensure!(
                after.tick == state.tick && after.rng == state.rng,
                "dialog answer advanced time/RNG"
            );
            cuts.push(save_cut(
                &root,
                &store,
                &a,
                &mut b,
                &format!("dialog-{ordinal}-answered"),
            )?);
            receipts.last_mut().unwrap()["complete"] = json!(true);
            write(&out, "ordered-inputs.json", &receipts)?;
            // The next planned record may already be active from the original
            // dialog queue. Otherwise inspect only actual semantic boundaries.
            if ordinal + 1 < planned.dialogs.len()
                && a.session.simulation.vm.world.dialogs.modal.is_none()
            {
                let started = Instant::now();
                loop {
                    let x = ready_frame(&mut a, &mut polls, started, 600)?;
                    let y = ready_frame(&mut b, &mut polls, started, 600)?;
                    match (x, y) {
                        (Frame::Boundary(Boundary::Dialog), Frame::Boundary(Boundary::Dialog)) => {
                            same(&a, &b)?;
                            break;
                        }
                        (Frame::Complete, Frame::Complete)
                        | (
                            Frame::Boundary(Boundary::PartialPass),
                            Frame::Boundary(Boundary::PartialPass),
                        ) => {}
                        _ => anyhow::bail!("new clock/boundary before next finite authored dialog"),
                    }
                }
            }
        }
        for (ordinal, pin) in planned.control_effects.iter().enumerate() {
            let before = save_cut(
                &root,
                &store,
                &a,
                &mut b,
                &format!("control-{ordinal}-before"),
            )?;
            let state = same(&a, &b)?;
            let mut invalid = pin.clone();
            invalid.request.value = invalid.request.value.wrapping_add(1);
            ensure!(
                a.complete_control_highlight(&invalid).is_err(),
                "changed control request accepted"
            );
            invalid = pin.clone();
            invalid.caller.handle = invalid.caller.handle.wrapping_add(1);
            ensure!(
                a.complete_control_highlight(&invalid).is_err(),
                "changed control lifetime accepted"
            );
            invalid = pin.clone();
            invalid.program_sha256 = "0".repeat(64);
            ensure!(
                a.complete_control_highlight(&invalid).is_err(),
                "changed control program accepted"
            );
            ensure!(
                super::support::require_bound_control(&[], i32::from(pin.request.value)).is_err(),
                "missing bound control accepted"
            );
            ensure!(
                same(&a, &b)? == state,
                "rejected control inputs changed retained state/RNG"
            );
            receipts.push(json!({"kind":"explicit_control_highlight","ordinal":ordinal,"pin":pin,"before":before,"complete":false}));
            write(&out, "ordered-inputs.json", &receipts)?;
            match pin.request.kind {
                sims_vm::ui_effect::Kind::Control => {
                    a.complete_control_highlight(pin)?;
                    b.complete_control_highlight(pin)?;
                }
                sims_vm::ui_effect::Kind::Portrait => {
                    a.complete_absent_portrait_highlight(pin)?;
                    b.complete_absent_portrait_highlight(pin)?;
                }
                _ => anyhow::bail!("unplanned UI effect kind"),
            }
            let after = same(&a, &b)?;
            ensure!(
                after.tick == state.tick && after.rng == state.rng,
                "control completion advanced time/RNG"
            );
            cuts.push(save_cut(
                &root,
                &store,
                &a,
                &mut b,
                &format!("control-{ordinal}-completed"),
            )?);
            receipts.last_mut().unwrap()["complete"] = json!(true);
            write(&out, "ordered-inputs.json", &receipts)?;
        }
        let started = Instant::now();
        while rows.len() < 8 {
            let x = ready_frame(&mut a, &mut polls, started, 600)?;
            let y = ready_frame(&mut b, &mut polls, started, 600)?;
            match (x, y) {
                (Frame::Boundary(Boundary::Tick), Frame::Boundary(Boundary::Tick)) => {
                    let state = same(&a, &b)?;
                    ensure!(
                        state.tick == start + rows.len() as u64 + 1,
                        "tick boundary skipped"
                    );
                    rows.push(state)
                }
                (Frame::Complete, Frame::Complete)
                | (
                    Frame::Boundary(Boundary::PartialPass),
                    Frame::Boundary(Boundary::PartialPass),
                ) => {}
                (Frame::Boundary(Boundary::Dialog), Frame::Boundary(Boundary::Dialog)) => {
                    same(&a, &b)?;
                    write(&out, "modal-discovery.json", &modal_discovery(&a)?)?;
                    cuts.push(save_cut(
                        &root,
                        &store,
                        &a,
                        &mut b,
                        "next-unanswered-modal",
                    )?);
                    return Ok("stopped_at_unanswered_modal");
                }
                (Frame::Boundary(x), Frame::Boundary(y)) if x == y => {
                    same(&a, &b)?;
                    write(
                        &out,
                        "service-discovery.json",
                        &json!({"boundary":x,"ui_owner":a.session.simulation.vm.world.ui_owner,"threads":a.session.simulation.vm.threads.iter().filter(|(_,t)|matches!(t.status,sims_vm::Status::AwaitingUiEffect(_))).collect::<Vec<_>>()}),
                    )?;
                    cuts.push(save_cut(
                        &root,
                        &store,
                        &a,
                        &mut b,
                        "next-unhandled-service",
                    )?);
                    return Ok("stopped_at_unhandled_service");
                }
                _ => anyhow::bail!("different warm/cold semantic boundary"),
            }
        }
        cuts.push(save_cut(
            &root,
            &store,
            &a,
            &mut b,
            "eight-completed-ticks",
        )?);
        Ok("eight_completed_ticks")
    })();
    if result.is_err() {
        let blob = a
            .checkpoint_fixture(&store)
            .or_else(|_| store.put_value(&a.session.save()))?;
        let mut case = CaseManifest::minimal_import(
            "first-stop",
            "Exact finite-input continuation stop",
            Artifact::self_contained(blob.clone(), ArtifactFormat::MessagePack, 1),
            vec!["May be a research Session; no implied resume qualification".into()],
        );
        case.provenance.origin = Origin::Authored;
        store.publish_case(&case)?;
        write(&out, "first-stop.json", &json!({"blob":blob}))?;
    }
    write(&out, "ticks.json", &rows)?;
    write(
        &out,
        "report.json",
        &json!({"household":spec.name,"outcome":result.as_ref().ok(),"completed_ticks":rows.len(),"authored_responses":receipts.iter().filter(|r|r["complete"]==true).count(),"native_parity_ticks":0,"stop":result.as_ref().err().map(|e|format!("{e:#}")),"cuts":cuts,"polls":polls,"verification":store.verify_all()?}),
    )?;
    result.map(|_| ())
}

pub(super) fn inspect_effect_probe() -> Result<()> {
    let root = repo()?;
    let spec = spec()?;
    let input = Store::open(root.join(std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_STORE")?))?;
    let source = input.load_case(&std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_CASE")?)?;
    ensure!(
        source.source.blob.sha256 == std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_SHA256")?,
        "effect inspection source changed"
    );
    let a = Household::from_fixture(index(&root)?, &input, &source.source.blob)?;
    ensure!(
        a.session.house.source == spec.house,
        "effect inspection household changed"
    );
    let before = a.observe()?;
    let mut pins = vec![];
    let mut constructors = vec![];
    for (&thread, t) in &a.session.simulation.vm.threads {
        if matches!(t.status, sims_vm::Status::AwaitingUiEffect(_)) {
            pins.push(json!({"pin":a.inspect_control_effect(thread)?,"continuation":t}));
        }
        if matches!(t.status, sims_vm::Status::AwaitingDialog(_)) {
            constructors
                .push(json!({"pin":a.inspect_dialog_construction(thread)?,"continuation":t}));
        }
    }
    ensure!(
        a.observe()? == before,
        "effect inspection mutated state/RNG"
    );
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private inspection output required"
    );
    std::fs::create_dir(&out).context("refusing to overwrite effect inspection")?;
    write(
        &out,
        "effect-pins.json",
        &json!({"source":source.source.blob,"house":spec.house,"state":before,"effects":pins,"constructors":constructors,"responses_submitted":0,"native_parity_ticks":0}),
    )
}

/// Complete an explicitly inspected presentation constructor, never its choice.
pub(super) fn dialog_construction_probe() -> Result<()> {
    #[derive(Deserialize)]
    struct Plan {
        schema: u32,
        house: String,
        starting_checkpoint_sha256: String,
        pin: super::support::DialogConstructionPin,
    }
    let root = repo()?;
    let spec = spec()?;
    let input = Store::open(root.join(std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_STORE")?))?;
    let source = input
        .load_case(&std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_CASE")?)?
        .source
        .blob;
    ensure!(
        source.sha256 == std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_SHA256")?,
        "constructor source changed"
    );
    let plan_path = std::env::var("SIMS_PLAYER_DIALOG_CONSTRUCTION_INPUT")?;
    let raw = std::fs::read(root.join(&plan_path))?;
    ensure!(
        exploration::digest(&raw) == std::env::var("SIMS_PLAYER_DIALOG_CONSTRUCTION_INPUT_SHA256")?,
        "constructor plan bytes changed"
    );
    let plan: Plan = serde_json::from_slice(&raw)?;
    ensure!(
        plan.schema == 1
            && plan.house == spec.house
            && plan.starting_checkpoint_sha256 == source.sha256,
        "constructor plan source contract differs"
    );
    let mut a = Household::from_fixture(index(&root)?, &input, &source)?;
    ensure!(
        a.session.house.source == spec.house,
        "constructor source house differs"
    );
    ensure!(
        a.inspect_dialog_construction(plan.pin.thread)? == plan.pin,
        "constructor request changed before any host work"
    );
    let mut b = a.fork()?;
    let start = same(&a, &b)?;
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private constructor output required"
    );
    std::fs::create_dir(&out).context("refusing to overwrite constructor evidence")?;
    let path = root.join(std::env::var("SIMS_PLAYER_NOTICE_STORE")?);
    ensure!(
        path.starts_with(root.join("fixtures/private")),
        "new private constructor store required"
    );
    let store = Store::create(path)?;
    let mut cuts = vec![];
    let mut journal = vec![];
    let mut stage = "constructor negative controls";
    write(
        &out,
        "input.json",
        &json!({"source":source,"plan_path":plan_path,"plan_sha256":exploration::digest(&raw),"program_fingerprint":a.session.program.fingerprint(),"options":a.session.options,"neighborhood_input":a.session.host.presentation.neighborhood_input,"helper_sha256":exploration::digest(include_bytes!("household_cases.rs")),"adapter_sha256":exploration::digest(include_bytes!("player_action_driver.rs")),"executable_sha256":exploration::digest(&std::fs::read(std::env::current_exe()?)?)}),
    )?;
    std::fs::write(
        out.join("helper-source.rs"),
        include_bytes!("household_cases.rs"),
    )?;
    std::fs::write(
        out.join("adapter-source.rs"),
        include_bytes!("player_action_driver.rs"),
    )?;
    std::fs::write(out.join("constructor-plan.json"), &raw)?;
    let result = (|| -> Result<()> {
        let mut negatives = vec![];
        let mut bad = plan.pin.clone();
        bad.request_id += 1;
        negatives.push(bad);
        let mut bad = plan.pin.clone();
        bad.caller.handle += 1;
        negatives.push(bad);
        let mut bad = plan.pin.clone();
        bad.program_sha256 = "0".repeat(64);
        negatives.push(bad);
        let mut bad = plan.pin.clone();
        bad.continuation_sha256 = "0".repeat(64);
        negatives.push(bad);
        let mut bad = plan.pin.clone();
        bad.thread = u64::MAX;
        negatives.push(bad);
        for (i, bad) in negatives.iter().enumerate() {
            ensure!(
                a.begin_dialog_construction(bad).is_err(),
                "mismatched constructor accepted"
            );
            ensure!(
                same(&a, &b)? == start,
                "rejected constructor mutated state/RNG"
            );
            journal.push(json!({"kind":"rejected_constructor","index":i,"pin":bad,"state":start}));
        }
        cuts.push(save_cut(
            &root,
            &store,
            &a,
            &mut b,
            "constructor-before-host",
        )?);
        stage = "actual Client begin";
        a.begin_dialog_construction(&plan.pin)?;
        b.begin_dialog_construction(&plan.pin)?;
        same(&a, &b)?;
        journal.push(json!({"kind":"client_begin","pin":plan.pin,"state":a.observe()?}));
        cuts.push(save_cut(
            &root,
            &store,
            &a,
            &mut b,
            "constructor-host-pending",
        )?);
        stage = "actual pending Client advance";
        let mut complete = false;
        for poll in 0..1000 {
            let x = a.advance_dialog_construction(256)?;
            let y = b.advance_dialog_construction(256)?;
            ensure!(x == y, "constructor host progress differs");
            let state = same(&a, &b)?;
            ensure!(
                state.tick == start.tick,
                "constructor host advanced gameplay tick"
            );
            journal.push(json!({"kind":"client_advance","poll":poll,"budget":256,"complete":x,"state":state}));
            if x {
                complete = true;
                break;
            }
        }
        ensure!(complete, "bounded dialog constructor not complete");
        a.collect_dialog_images()?;
        b.collect_dialog_images()?;
        same(&a, &b)?;
        stage = "inspect final unanswered modal";
        write(&out, "dialog-discovery.json", &modal_discovery(&a)?)?;
        cuts.push(save_cut(
            &root,
            &store,
            &a,
            &mut b,
            "constructor-completed-unanswered",
        )?);
        ensure!(
            a.observe()?.tick == start.tick,
            "constructor counted gameplay ticks"
        );
        Ok(())
    })();
    write(&out, "host-steps.json", &journal)?;
    if result.is_err() {
        let blob = save_cut(&root, &store, &a, &mut b, "constructor-first-stop")?;
        write(&out, "first-stop.json", &json!({"blob":blob}))?;
    }
    write(
        &out,
        "report.json",
        &json!({"outcome":if result.is_ok(){"complete unanswered dialog"}else{"incomplete"},"stage":stage,"source":source,"source_tick":start.tick,"completed_ticks":0,"responses_submitted":0,"native_parity_ticks":0,"qualification":"test-only actual Client presentation-host completion from the exact retained branch; no new gameplay credit","cuts":cuts,"stop":result.as_ref().err().map(|e|format!("{e:#}")),"verification":store.verify_all()?}),
    )?;
    result
}

pub(super) fn supplied_goth_input() -> Result<()> {
    use sims_fixtures::Compression;
    let root = repo()?;
    let spec = spec()?;
    ensure!(
        spec.name == "goth",
        "Goth observation must never supply Newbie input"
    );
    let input = Store::open(root.join("fixtures/private/player-goth-help-no-20261026-first"))?;
    let source = input.load_case("eight-completed-ticks")?.source.blob;
    ensure!(
        source.sha256 == "420a1d28a1befb4345e6e9b3f735591dbf95b6c69f9704e4c8f5023b1c925cf6",
        "safe small Goth source changed"
    );
    let summary_path = "cache/research/sims-exe/parity-investigation-20261020/results/gameplay-goth-neighborhood-summary.json";
    let log_path = "cache/research/sims-exe/parity-investigation-20261020/goth-house5-start.jsonl";
    let summary = std::fs::read(root.join(summary_path))?;
    let log = std::fs::read(root.join(log_path))?;
    ensure!(
        exploration::digest(&summary)
            == "0040aa8f5ad93eb482e88cff5e38088bac1730c57c3c4ed69e1c550ada883d38"
            && exploration::digest(&log)
                == "509f476046ce76ebd5dad38f031f1a1094f3f51a20eeb77ee81b533029a59a8a",
        "scoped original observation changed"
    );
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private supplied-input output required"
    );
    std::fs::create_dir(&out).context("refusing to overwrite supplied-input evidence")?;
    let path = root.join(std::env::var("SIMS_PLAYER_NOTICE_STORE")?);
    ensure!(
        path.starts_with(root.join("fixtures/private")),
        "private new input store required"
    );
    let store = Store::create(path)?;
    let mut a = Household::from_fixture(index(&root)?, &input, &source)?;
    let mut b = a.fork()?;
    let initial = same(&a, &b)?;
    let mut polls = 0;
    let started = Instant::now();
    // End only the already-active elapsed frame. Do not start a new frame or
    // clear its ownership to manufacture a bindable boundary.
    while a.session.clock_pending() || b.session.clock_pending() {
        ensure!(
            a.session.clock_pending() == b.session.clock_pending(),
            "retained frame owners differ"
        );
        match (
            ready_frame(&mut a, &mut polls, started, 600)?,
            ready_frame(&mut b, &mut polls, started, 600)?,
        ) {
            (Frame::Complete, Frame::Complete)
            | (Frame::Boundary(Boundary::PartialPass), Frame::Boundary(Boundary::PartialPass)) => {
                same(&a, &b)?;
            }
            _ => anyhow::bail!("unexpected gameplay/modal boundary while finishing retained frame"),
        }
    }
    let safe = same(&a, &b)?;
    ensure!(
        initial.tick == safe.tick && initial.rng == safe.rng,
        "safe frame completion changed tick/RNG"
    );
    ensure!(
        a.session.house.source == spec.house
            && a.session
                .house
                .sources
                .get(spec.house)
                .is_some_and(|h| h == spec.house_sha)
            && a.session
                .house
                .sources
                .get("UserData/Neighborhood.iff")
                .is_some_and(|h| h == NEIGHBORHOOD_SHA),
        "Goth source pair changed"
    );
    let captured = sims_runtime::original_session::neighborhood_input::Captured::from_polling(
        &a.session.house,
        &summary,
        &log,
        0,
    )?;
    captured.validate_house(&a.session.house)?;
    let supplied = captured.nearby_houses()?;
    ensure!(
        supplied.is_empty(),
        "exact sampled null-vector observation changed"
    );
    let before = a.research_state()?;
    let before_state = a.observe()?;
    a.session.bind_neighborhood_capture(captured.clone())?;
    b.session.bind_neighborhood_capture(captured.clone())?;
    let mut expected = before;
    expected["session"]["simulation"]["state"]["vm"]["world"]["person"]["nearby_houses"] =
        serde_json::to_value(supplied)?;
    expected["session"]["view"]["neighborhood_input"] = serde_json::to_value(&captured)?;
    ensure!(
        a.research_state()? == expected,
        "binding changed fields beyond supplied order/provenance"
    );
    let after = same(&a, &b)?;
    ensure!(
        before_state.tick == after.tick && before_state.rng == after.rng,
        "binding advanced time/RNG"
    );
    ensure!(
        a.session
            .bind_neighborhood_capture(captured.clone())
            .is_err(),
        "duplicate binding accepted"
    );
    ensure!(
        same(&a, &b)? == after,
        "duplicate rejection changed state/RNG"
    );
    let blob = save_cut(&root, &store, &a, &mut b, "conditional-observed-input")?;
    let limitation = "CONDITIONAL INTERNAL INPUT ONLY: sampled null vectors in original Goth context; no original source-hash/full-process attestation;69 samples end88951, no later vector-timeline parity or original tick/RNG/CRT copying; Newbie remains unknown";
    let declaration = json!({"limitation":limitation,"initial_source":source,"sample_index":0,"captured":captured,"safe_frame_completion":{"polls":polls,"initial":initial,"safe":safe},"before":before_state,"after":after,"changed_fields":["session.simulation.state.vm.world.person.nearby_houses","session.view.neighborhood_input"],"all_other_fields_equal":true,"duplicate_rejected_unchanged":true,"native_parity_ticks":0});
    let summary_blob = store.put_bytes(&summary, Compression::Gzip)?;
    let log_blob = store.put_bytes(&log, Compression::Gzip)?;
    let declaration_blob = store.put_value(&declaration)?;
    // Extend the new immutable case before publication under a different ID;
    // the resumable source itself remains the same owned checkpoint blob.
    let mut case = store.load_case("conditional-observed-input")?;
    case.id = "conditional-goth-start".into();
    case.notes.push(limitation.into());
    case.coverage = Coverage::Component {
        scope: "conditional observed-input binding and independent managed restore".into(),
        limitations: vec![limitation.into()],
    };
    case.artifacts = vec![
        Artifact::self_contained(summary_blob, ArtifactFormat::Json, 1),
        Artifact::self_contained(
            log_blob,
            ArtifactFormat::Other("original polling JSONL/schema3".into()),
            3,
        ),
        Artifact::self_contained(declaration_blob, ArtifactFormat::MessagePack, 1),
    ];
    store.publish_case(&case)?;
    write(&out, "declaration.json", &declaration)?;
    write(
        &out,
        "report.json",
        &json!({"outcome":"passed","source":blob,"native_parity_ticks":0,"verification":store.verify_all()?}),
    )
}

pub(super) fn action_probe() -> Result<()> {
    use super::Command;
    let root = repo()?;
    let spec = spec()?;
    let input = Store::open(root.join(std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_STORE")?))?;
    let source = input.load_case(&std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_CASE")?)?;
    ensure!(
        source.source.blob.sha256 == std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_SHA256")?,
        "action probe source changed"
    );
    let mut a = Household::from_fixture(index(&root)?, &input, &source.source.blob)?;
    ensure!(
        a.session.house.source == spec.house,
        "action household differs"
    );
    let mut b = a.fork()?;
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private action output required"
    );
    std::fs::create_dir(&out).context("refusing to overwrite action probe")?;
    let path = root.join(std::env::var("SIMS_PLAYER_NOTICE_STORE")?);
    ensure!(
        path.starts_with(root.join("fixtures/private")),
        "new private action store required"
    );
    let store = Store::create(path)?;
    let mut journal = vec![];
    let mut cuts = vec![];
    let mut rows = vec![];
    let mut polls = 0;
    let mut active = false;
    let mut stage = "safe frame completion";
    write(
        &out,
        "input.json",
        &json!({"source":source.source.blob,"house":spec.house,"program_fingerprint":a.session.program.fingerprint(),"options":a.session.options,"neighborhood_input":a.session.host.presentation.neighborhood_input,"helper_sha256":exploration::digest(include_bytes!("household_cases.rs")),"adapter_sha256":exploration::digest(include_bytes!("player_action_driver.rs")),"executable_sha256":exploration::digest(&std::fs::read(std::env::current_exe()?)?)}),
    )?;
    std::fs::write(
        out.join("helper-source.rs"),
        include_bytes!("household_cases.rs"),
    )?;
    let result = (|| -> Result<&str> {
        let started = Instant::now();
        while a.session.clock_pending() || b.session.clock_pending() {
            ensure!(
                a.session.clock_pending() == b.session.clock_pending(),
                "frame owners differ"
            );
            match (
                ready_frame(&mut a, &mut polls, started, 600)?,
                ready_frame(&mut b, &mut polls, started, 600)?,
            ) {
                (Frame::Complete, Frame::Complete)
                | (
                    Frame::Boundary(Boundary::PartialPass),
                    Frame::Boundary(Boundary::PartialPass),
                ) => {
                    same(&a, &b)?;
                }
                _ => anyhow::bail!("unexpected semantic boundary before authored action"),
            }
        }
        stage = "discover actual resident and visible chair lifetimes";
        let actor = a
            .session
            .simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?
            .context("source has no selected original person")?;
        let identity = Identity::capture(&a.session, actor)?;
        let guid = a
            .session
            .simulation
            .vm
            .world
            .objects
            .get(&actor)
            .and_then(|o| o.definition.as_ref())
            .context("resident definition absent")?
            .guid;
        ensure!(
            a.session
                .house
                .household
                .as_ref()
                .is_some_and(|h| h.residents.iter().any(|r| r.guid == guid)),
            "selected actor is not an imported resident"
        );
        let target = a
            .session
            .simulation
            .vm
            .world
            .objects
            .iter()
            .find(|(id, o)| {
                o.person.is_none()
                    && o.cursor.is_none()
                    && o.definition
                        .as_ref()
                        .is_some_and(|d| d.source.ends_with("DiningChairs.iff"))
                    && a.device
                        .scene()
                        .draws
                        .iter()
                        .any(|d| d.object == Some(**id))
            })
            .map(|(&id, _)| Identity::capture(&a.session, id))
            .transpose()?
            .context("no actual visible original DiningChairs target")?;
        write(
            &out,
            "discovery.json",
            &json!({"residents":resident_discovery(&a, false)?,"actor":identity,"target":target}),
        )?;
        super::inputs(
            &out,
            &mut a,
            &mut b,
            vec![
                Command::Pause { paused: true },
                Command::Drain { budget: 256 },
                Command::Select { identity },
                Command::Drain { budget: 256 },
                Command::OpenObject {
                    identity: target.clone(),
                },
                Command::Drain { budget: 256 },
            ],
            &mut journal,
        )?;
        stage = "inspect actual offered interaction";
        let actions = a
            .session
            .host
            .presentation
            .object_input
            .as_ref()
            .context("offered object menu absent")?
            .actions()
            .to_vec();
        // The real HUD retains nonempty cached text and otherwise resolves
        // the offered menu_id in the actual target's TTAB. It does not require
        // every action to have an eagerly populated cached label.
        let labels = actions
            .iter()
            .map(|action| -> Result<String> {
                if let Some(label) = action.label.as_ref().filter(|s| !s.is_empty()) {
                    return Ok(label.clone());
                }
                a.session
                    .simulation
                    .vm
                    .action_menu(&a.session.program, action.target)
                    .map_err(anyhow::Error::msg)?
                    .iter()
                    .find(|entry| entry.id == action.menu_id)
                    .map(|entry| entry.label.clone())
                    .context("offered action TTAB label absent")
            })
            .collect::<Result<Vec<_>>>()?;
        write(
            &out,
            "offered-actions.json",
            &json!({"actions":actions,"actual_hud_labels":labels}),
        )?;
        let index = labels
            .iter()
            .position(|label| label.eq_ignore_ascii_case("Sit"))
            .context("actual original chair did not offer Sit")?;
        let action = &actions[index];
        super::inputs(
            &out,
            &mut a,
            &mut b,
            vec![
                Command::Choose {
                    index,
                    expected: action.clone(),
                },
                Command::Drain { budget: 256 },
                Command::Pause { paused: false },
                Command::Drain { budget: 256 },
            ],
            &mut journal,
        )?;
        let receipt = a
            .queued_receipt()
            .context("actual offered interaction was refused")?;
        let blob = save_cut(&root, &store, &a, &mut b, "queued-player-source")?;
        cuts.push(blob.clone());
        write(
            &out,
            "ready-action.json",
            &json!({"source":source.source.blob,"checkpoint":blob,"actor":receipt.0,"serial":receipt.1,"target":target,"offered_index":index,"offered_action":action,"qualification":"authored real offered input; not original action observation/parity"}),
        )?;
        stage = "eight completed action ticks";
        let start = a.observe()?.tick;
        let started = Instant::now();
        while rows.len() < 8 {
            match (
                ready_frame(&mut a, &mut polls, started, 600)?,
                ready_frame(&mut b, &mut polls, started, 600)?,
            ) {
                (Frame::Boundary(Boundary::Tick), Frame::Boundary(Boundary::Tick)) => {
                    let state = same(&a, &b)?;
                    ensure!(
                        state.tick == start + rows.len() as u64 + 1,
                        "action tick boundary skipped"
                    );
                    rows.push(state);
                    if !active && super::active(&a, receipt) {
                        cuts.push(save_cut(
                            &root,
                            &store,
                            &a,
                            &mut b,
                            "first-active-player-interaction",
                        )?);
                        active = true;
                    }
                }
                (Frame::Complete, Frame::Complete)
                | (
                    Frame::Boundary(Boundary::PartialPass),
                    Frame::Boundary(Boundary::PartialPass),
                ) => {}
                (Frame::Boundary(Boundary::Dialog), Frame::Boundary(Boundary::Dialog)) => {
                    same(&a, &b)?;
                    write(&out, "modal-discovery.json", &modal_discovery(&a)?)?;
                    cuts.push(save_cut(
                        &root,
                        &store,
                        &a,
                        &mut b,
                        "first-unanswered-modal",
                    )?);
                    return Ok("stopped_at_modal");
                }
                _ => anyhow::bail!("unexpected/different action boundary"),
            }
        }
        cuts.push(save_cut(&root, &store, &a, &mut b, "eight-action-ticks")?);
        Ok("eight_action_ticks")
    })();
    if result.is_err() {
        let blob = a
            .checkpoint_fixture(&store)
            .or_else(|_| store.put_value(&a.session.save()))?;
        let mut case = CaseManifest::minimal_import(
            "first-stop",
            "Exact authored player action stop",
            Artifact::self_contained(blob, ArtifactFormat::MessagePack, 1),
            vec!["May be research Session only; no implied resume/parity".into()],
        );
        case.provenance.origin = Origin::Authored;
        store.publish_case(&case)?;
    }
    write(
        &out,
        "report.json",
        &json!({"outcome":result.as_ref().ok(),"stage":stage,"source":source.source.blob,"completed_ticks":rows.len(),"active_player_cut":active,"native_parity_ticks":0,"cuts":cuts,"stop":result.as_ref().err().map(|e|format!("{e:#}")),"verification":store.verify_all()?}),
    )?;
    write(&out, "ticks.json", &rows)?;
    result.map(|_| ())
}

pub(super) fn ui_wait_probe() -> Result<()> {
    let root = repo()?;
    let spec = spec()?;
    let input = Store::open(root.join(std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_STORE")?))?;
    let source = input.load_case(&std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_CASE")?)?;
    ensure!(
        source.source.blob.sha256 == std::env::var("SIMS_PLAYER_CONTINUATION_INPUT_SHA256")?,
        "UI-wait source changed"
    );
    let mut a = Household::from_fixture(index(&root)?, &input, &source.source.blob)?;
    ensure!(
        a.session.house.source == spec.house,
        "UI-wait household differs"
    );
    let mut b = a.fork()?;
    let out = root.join(std::env::var("SIMS_PLAYER_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private UI-wait output required"
    );
    std::fs::create_dir(&out).context("refusing to overwrite UI-wait probe")?;
    let path = root.join(std::env::var("SIMS_PLAYER_NOTICE_STORE")?);
    ensure!(
        path.starts_with(root.join("fixtures/private")),
        "new private UI-wait store required"
    );
    let store = Store::create(path)?;
    let start = same(&a, &b)?;
    let mut polls = 0;
    let started = Instant::now();
    while a.session.clock_pending() || b.session.clock_pending() {
        ensure!(
            a.session.clock_pending() == b.session.clock_pending(),
            "UI-wait prelude frame owners differ"
        );
        match (
            ready_frame(&mut a, &mut polls, started, 600)?,
            ready_frame(&mut b, &mut polls, started, 600)?,
        ) {
            (Frame::Complete, Frame::Complete)
            | (Frame::Boundary(Boundary::PartialPass), Frame::Boundary(Boundary::PartialPass)) => {
                same(&a, &b)?;
            }
            _ => anyhow::bail!("UI-wait prelude encountered gameplay/modal boundary"),
        }
    }
    let owner = a.session.simulation.vm.world.ui_owner.owner;
    let identity = owner
        .map(|id| Identity::capture(&a.session, id))
        .transpose()?;
    write(
        &out,
        "before.json",
        &json!({"source":source.source.blob,"state":a.observe()?,"owner":identity,"watch":a.session.host.presentation.ui_wait,"program_fingerprint":a.session.program.fingerprint(),"options":a.session.options,"helper_sha256":exploration::digest(include_bytes!("household_cases.rs")),"adapter_sha256":exploration::digest(include_bytes!("player_action_driver.rs"))}),
    )?;
    let mut complete = false;
    let mut stop = None;
    for _ in 0..1000 {
        let x = a.poll_application_ui();
        let y = b.poll_application_ui();
        match (x, y) {
            (Ok(true), Ok(true)) => {
                same(&a, &b)?;
                complete = true;
                break;
            }
            (Ok(false), Ok(false)) => {}
            (Err(x), Err(y)) => {
                ensure!(
                    format!("{x:#}") == format!("{y:#}"),
                    "UI-wait branch errors differ"
                );
                same(&a, &b)?;
                stop = Some(format!("{x:#}"));
                break;
            }
            _ => anyhow::bail!("UI-wait host progress differs"),
        }
    }
    ensure!(
        a.observe()?.tick == start.tick,
        "UI-wait polling counted gameplay time"
    );
    if let Some(condition) = a.session.pending_ui_observation() {
        let before = same(&a, &b)?;
        let bad = sims_vm::ui_owner::wait::Condition::Control(-32768);
        ensure!(
            condition != bad
                && a.session
                    .acknowledge_ui_wait(
                        &bad,
                        sims_vm::ui_owner::wait::Observation::Control {
                            image: None,
                            state: 0
                        }
                    )
                    .is_err(),
            "mismatched UI wait observation accepted"
        );
        ensure!(
            same(&a, &b)? == before,
            "rejected UI-wait observation changed state/RNG"
        );
    }
    let blob = save_cut(
        &root,
        &store,
        &a,
        &mut b,
        if complete {
            "completed-ui-wait-pass"
        } else {
            "retained-ui-wait-boundary"
        },
    )?;
    write(
        &out,
        "report.json",
        &json!({"outcome":if complete{"completed_ui_pass"}else{"stopped_at_actual_observation"},"source":source.source.blob,"checkpoint":blob,"owner_before":identity,"owner_after":a.session.simulation.vm.world.ui_owner.owner,"watch":a.session.host.presentation.ui_wait,"pending_observation":a.session.pending_ui_observation(),"state":a.observe()?,"clock_tick_delta":a.session.simulation.vm.tick-start.tick,"native_parity_ticks":0,"stop":stop,"verification":store.verify_all()?}),
    )
}
