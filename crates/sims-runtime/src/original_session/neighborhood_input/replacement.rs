use super::{Identity, Session, sha};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_content::{
    Index,
    neighborhood_input::{INSTALLED_HOUSES_ASCENDING_V1, InstalledHouses},
    saves::House,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replacement {
    schema: u32,
    policy: String,
    identity: Identity,
    manifest: InstalledHouses,
    manifest_sha256: String,
}
impl Replacement {
    pub fn from_index(index: &Index, house: &House) -> Result<Self> {
        let manifest = InstalledHouses::discover(
            index,
            &house.source,
            &house.neighborhood_source,
            house.number,
        )?;
        let result = Self {
            schema: 1,
            policy: INSTALLED_HOUSES_ASCENDING_V1.into(),
            identity: Identity::from_house(house)?,
            manifest_sha256: sha(&serde_json::to_vec(&manifest)?),
            manifest,
        };
        result.validate_house(house)?;
        Ok(result)
    }
    pub fn validate_house(&self, house: &House) -> Result<()> {
        ensure!(
            self.schema == 1 && self.policy == INSTALLED_HOUSES_ASCENDING_V1,
            "unsupported neighborhood replacement schema/policy"
        );
        self.manifest.validate(
            &self.identity.house,
            &self.identity.neighborhood,
            self.identity.lot,
        )?;
        self.identity.neighborhood_number()?;
        for source in [&self.identity.house, &self.identity.neighborhood] {
            ensure!(
                house
                    .sources
                    .keys()
                    .filter(|candidate| candidate.eq_ignore_ascii_case(source))
                    .count()
                    == 1,
                "ambiguous neighborhood replacement source identity"
            );
        }
        ensure!(
            self.manifest_sha256 == sha(&serde_json::to_vec(&self.manifest)?),
            "neighborhood replacement manifest digest differs"
        );
        ensure!(
            self.identity == Identity::from_house(house)?,
            "neighborhood replacement belongs to another import"
        );
        Ok(())
    }
    pub fn schema(&self) -> u32 {
        self.schema
    }
    pub fn policy_id(&self) -> &str {
        &self.policy
    }
    pub fn house_source(&self) -> &str {
        &self.identity.house
    }
    pub fn house_hash(&self) -> &str {
        &self.identity.house_hash
    }
    pub fn neighborhood_source(&self) -> &str {
        &self.identity.neighborhood
    }
    pub fn neighborhood_hash(&self) -> &str {
        &self.identity.neighborhood_hash
    }
    pub fn lot(&self) -> i16 {
        self.identity.lot
    }
    pub fn manifest(&self) -> &InstalledHouses {
        &self.manifest
    }
    pub fn nearby_houses(&self) -> &[i32] {
        self.manifest.nearby_houses()
    }
    pub fn candidate_count(&self) -> usize {
        self.nearby_houses().len()
    }
    pub fn manifest_hash(&self) -> &str {
        &self.manifest_sha256
    }
}

impl Session {
    pub fn bind_installed_houses_ascending_v1(&mut self) -> Result<()> {
        let replacement = Replacement::from_index(&self.index, &self.house)?;
        self.bind_neighborhood_replacement(replacement)
    }
    pub fn bind_neighborhood_replacement(&mut self, replacement: Replacement) -> Result<()> {
        replacement.validate_house(&self.house)?;
        ensure!(
            self.host.presentation.neighborhood_input.is_none()
                && self.host.presentation.neighborhood_replacement.is_none(),
            "neighborhood input provenance already bound"
        );
        self.require_neighborhood_input_boundary()?;
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending()
                && !self.view_refresh_pending()
                && !self.host.presentation.awaiting_house_open
                && !self
                    .host
                    .presentation
                    .property_refresh
                    .as_ref()
                    .is_some_and(|state| state.pending()),
            "neighborhood replacement requires idle presentation owners"
        );
        let people = self
            .simulation
            .vm
            .world
            .person
            .as_ref()
            .context("neighborhood person registry absent")?;
        ensure!(
            people.nearby_houses.is_none(),
            "replacement input cannot upgrade or overwrite a known nearby vector"
        );
        ensure!(
            replacement.manifest
                == InstalledHouses::discover(
                    &self.index,
                    &self.house.source,
                    &self.house.neighborhood_source,
                    self.house.number,
                )?,
            "neighborhood replacement differs from session discovery manifest"
        );
        let order = replacement.nearby_houses().to_vec();
        self.simulation
            .vm
            .world
            .person
            .as_mut()
            .unwrap()
            .nearby_houses = Some(order);
        self.host.presentation.neighborhood_replacement = Some(replacement);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::original_session::{Options, Saved, View, completed_epoch, managed};
    use serde_json::{Value, json};
    use sims_content::{Resource, character_loading::live};
    use sims_sim::{Room, Simulation};
    use sims_vm::{Program, Vm, World};
    use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

    const HOUSE: &str = "UserData2/Houses/House09.iff";
    const NEIGHBORHOOD: &str = "UserData2/Neighborhood.iff";

    fn session() -> Session {
        let index = Index {
            root: PathBuf::from("/not-read"),
            resources: [
                HOUSE,
                NEIGHBORHOOD,
                "UserData2/Houses/House01.iff",
                "UserData2/Houses/House11.iff",
            ]
            .into_iter()
            .map(|locator| Resource {
                locator: locator.into(),
                name: locator.rsplit('/').next().unwrap().into(),
                source: PathBuf::from("/not-read"),
                offset: 0,
                size: locator.len() as u64,
                unpacked_size: locator.len() as u64,
                archived: false,
                owned: Some(Arc::from(locator.as_bytes())),
            })
            .collect(),
            archives: 0,
            diagnostics: vec![],
        };
        let sources = BTreeMap::from([
            (HOUSE.to_owned(), sha(HOUSE.as_bytes())),
            (NEIGHBORHOOD.to_owned(), sha(NEIGHBORHOOD.as_bytes())),
        ]);
        let house: House = serde_json::from_value(json!({
            "source": HOUSE, "number": 9, "neighborhood_source": NEIGHBORHOOD,
            "sources": sources,
            "width": 1, "height": 1, "floors": [], "floor_map": [], "wall_map": [], "arrays": {},
            "house": {"version": 0x49, "name": "Synthetic", "pairs": ([[0,0]; 6]), "tail": []},
            "view": {"version": 0x49, "fields": ([0;4]), "camera_direction": 0, "unknown_shorts": ([0;2]), "unknown_word": 0, "roof": "", "tail": []},
            "simulator": {"version": 0x49, "globals": [], "ticks": 123, "scheduler_cursor": 0,
                "funds": 456, "lot_rng": 17, "global_rng": 19, "pending_funds": 0, "lot_value": 0,
                "objects_value": 0, "architecture_value": 0, "budget_days": [], "tail": []},
            "types": {"version": 3, "entries": []},
            "objects": {"version": 0x49, "order": [], "types_by_id": {}, "instances": {}, "tail": []},
            "definitions": {}, "neighbors": {"version": 1, "entries": [], "tail": []},
            "neighborhood": {"version": 20, "data": ([0;16]), "inventories": [], "options": ([false;11]), "option_bytes": [], "tail": []},
            "resource_storage": null, "families": {}, "household": null, "missing_tile_heads": []
        })).unwrap();
        let assets = live::Snapshot {
            floors: BTreeMap::new(),
            neighborhood_source: NEIGHBORHOOD.into(),
            initializer_settings: sims_sim::pose::animator::Settings {
                fade_enabled: false,
                fade_duration: 0,
                sampling_config: 0,
            },
            fresh: BTreeMap::new(),
            reloads: BTreeMap::new(),
            dependent_jobs: BTreeMap::new(),
            accessories: BTreeMap::new(),
            retirements: BTreeMap::new(),
            notification_jobs: BTreeMap::new(),
            notifications: None,
            characters: BTreeMap::new(),
            mesh_pool: Default::default(),
            clothing: Default::default(),
            skeletons: None,
            ordinary: Default::default(),
            autonomy_resources: None,
        };
        let host = live::Host::restore(&index, View::default(), assets).unwrap();
        let mut vm = Vm::new(
            World {
                person: Some(Default::default()),
                ..Default::default()
            },
            19,
        );
        vm.tick = 123;
        vm.lot_rng_state = Some(17);
        vm.crt_rng_state = Some(23);
        let simulation = Simulation::new(
            vm,
            Room {
                width: 16,
                height: 16,
                obstacles: vec![],
            },
            BTreeMap::new(),
        )
        .unwrap();
        Session {
            index: Arc::new(index),
            managed_policy: managed::Policy::HistoricalSchema1,
            house,
            program: Program::default(),
            simulation,
            host,
            options: Options::default(),
        }
    }

    fn state(session: &Session) -> Value {
        json!({
            "house": session.house, "program": session.program, "simulation": session.simulation,
            "assets": session.host.snapshot(), "view": session.host.presentation, "options": session.options,
            "index": {"resources": session.index.resources, "root": session.index.root,
                "archives": session.index.archives, "diagnostics": session.index.diagnostics},
            "managed_policy": format!("{:?}", session.managed_policy)
        })
    }

    fn capture(session: &Session) -> super::super::Captured {
        let log = serde_json::to_vec(&json!({
            "schema":5,"kind":"polling_not_atomic","owner":65536,"house":69632,
            "simulator":73728,"neighborhood":77824,"lot":9,"tick_before":123,"tick_after":123,
            "rng_before":19,"rng_after":19,"coordinates":{"header":[0,0,0],"words":[]},
            "nearby":{"header":[0,0,0],"words":[]}
        }))
        .unwrap();
        let summary =
            serde_json::to_vec(&json!({"neighborhood":2,"lot":9,"files":[{"sha256":sha(&log)}]}))
                .unwrap();
        super::super::Captured::from_identity(
            Identity::from_house(&session.house).unwrap(),
            &summary,
            &log,
            0,
        )
        .unwrap()
    }

    fn rejected_unchanged(session: &mut Session, replacement: Replacement) {
        let before = state(session);
        assert!(session.bind_neighborhood_replacement(replacement).is_err());
        assert_eq!(state(session), before);
    }

    #[test]
    fn bind_changes_only_input_and_provenance_not_tick_rng_or_other_state() {
        let mut session = session();
        assert_eq!(
            session.neighborhood_input_status().unwrap(),
            super::super::Status::Missing
        );
        let mut expected = state(&session);
        session.bind_installed_houses_ascending_v1().unwrap();
        let replacement = session
            .host
            .presentation
            .neighborhood_replacement
            .as_ref()
            .unwrap();
        expected["simulation"]["vm"]["world"]["person"]["nearby_houses"] = json!([1, 11]);
        expected["view"]["neighborhood_replacement"] = serde_json::to_value(replacement).unwrap();
        assert_eq!(state(&session), expected);
        assert_eq!(
            (
                session.simulation.vm.tick,
                session.simulation.vm.rng_state,
                session.simulation.vm.lot_rng_state,
                session.simulation.vm.crt_rng_state
            ),
            (123, 19, Some(17), Some(23))
        );
        assert_eq!(replacement.policy_id(), INSTALLED_HOUSES_ASCENDING_V1);
        assert_eq!((replacement.lot(), replacement.candidate_count()), (9, 2));
        assert_eq!(replacement.house_source(), HOUSE);
        assert_eq!(replacement.neighborhood_source(), NEIGHBORHOOD);
        assert_eq!(replacement.house_hash(), sha(HOUSE.as_bytes()));
        assert_eq!(
            replacement.neighborhood_hash(),
            sha(NEIGHBORHOOD.as_bytes())
        );
        assert!(matches!(
            session.neighborhood_input_status().unwrap(),
            super::super::Status::Replacement(_)
        ));
        session.validate_neighborhood_input().unwrap();
    }

    #[test]
    fn known_vectors_are_never_upgraded_even_empty_or_identical() {
        for order in [vec![], vec![1, 11], vec![11, 1]] {
            let mut session = session();
            let replacement = Replacement::from_index(&session.index, &session.house).unwrap();
            session
                .simulation
                .vm
                .world
                .person
                .as_mut()
                .unwrap()
                .nearby_houses = Some(order.clone());
            assert!(
                matches!(session.neighborhood_input_status().unwrap(), super::super::Status::PreexistingVector { count, .. } if count == order.len())
            );
            rejected_unchanged(&mut session, replacement);
            let saved = serde_json::to_value(session.save()).unwrap();
            assert!(saved["view"].get("neighborhood_replacement").is_none());
            let retained: Saved = serde_json::from_value(saved).unwrap();
            assert_eq!(
                retained
                    .simulation
                    .state
                    .vm
                    .world
                    .person
                    .unwrap()
                    .nearby_houses,
                Some(order)
            );
        }
    }

    #[test]
    fn existing_capture_can_still_bind_an_identical_known_vector() {
        let mut session = session();
        session
            .simulation
            .vm
            .world
            .person
            .as_mut()
            .unwrap()
            .nearby_houses = Some(vec![]);
        let captured = capture(&session);
        let order = captured.nearby_houses().unwrap();
        session.bind_neighborhood_capture(captured.clone()).unwrap();
        assert_eq!(
            session
                .simulation
                .vm
                .world
                .person
                .as_ref()
                .unwrap()
                .nearby_houses,
            Some(order)
        );
        assert!(session.host.presentation.neighborhood_replacement.is_none());
        let before = state(&session);
        assert_eq!(
            session
                .bind_neighborhood_capture(captured)
                .unwrap_err()
                .to_string(),
            "neighborhood capture already bound"
        );
        assert_eq!(state(&session), before);
    }

    #[test]
    fn captures_replacements_and_repeat_binds_conflict_without_mutation() {
        let mut captured = session();
        let replacement = Replacement::from_index(&captured.index, &captured.house).unwrap();
        let observation = capture(&captured);
        captured.bind_neighborhood_capture(observation).unwrap();
        assert!(matches!(
            captured.neighborhood_input_status().unwrap(),
            super::super::Status::ScopedCapture { count: 0, .. }
        ));
        rejected_unchanged(&mut captured, replacement);
        let mut replaced = session();
        let observation = capture(&replaced);
        replaced.bind_installed_houses_ascending_v1().unwrap();
        let before = state(&replaced);
        assert!(replaced.bind_neighborhood_capture(observation).is_err());
        assert_eq!(state(&replaced), before);
        let replacement = replaced
            .host
            .presentation
            .neighborhood_replacement
            .clone()
            .unwrap();
        rejected_unchanged(&mut replaced, replacement);
    }

    #[test]
    fn wrong_import_and_manifest_tampering_reject_before_any_mutation() {
        let mut session = session();
        let replacement = Replacement::from_index(&session.index, &session.house).unwrap();
        let original = serde_json::to_value(&replacement).unwrap();
        for (path, value) in [
            ("/schema", json!(2)),
            ("/policy", json!("native")),
            ("/identity/house", json!("UserData2/Houses/House08.iff")),
            ("/identity/lot", json!(8)),
            ("/identity/house_hash", json!("a".repeat(64))),
            ("/identity/neighborhood", json!("UserData/Neighborhood.iff")),
            ("/identity/neighborhood_hash", json!("b".repeat(64))),
            ("/manifest/order", json!([11, 1])),
            (
                "/manifest/candidates/0/locator",
                json!("UserData2/Houses/HOUSE01.IFF"),
            ),
            ("/manifest_sha256", json!("0".repeat(64))),
        ] {
            let mut damaged = original.clone();
            *damaged.pointer_mut(path).unwrap() = value;
            let damaged: Replacement = serde_json::from_value(damaged).unwrap();
            rejected_unchanged(&mut session, damaged);
        }
        let mut omitted = replacement.clone();
        let mut manifest = serde_json::to_value(&omitted.manifest).unwrap();
        manifest["order"] = json!([11]);
        manifest["candidates"].as_array_mut().unwrap().remove(0);
        omitted.manifest = serde_json::from_value(manifest).unwrap();
        omitted.manifest_sha256 = sha(&serde_json::to_vec(&omitted.manifest).unwrap());
        omitted.validate_house(&session.house).unwrap();
        rejected_unchanged(&mut session, omitted);
        session.house.sources.insert(HOUSE.into(), "c".repeat(64));
        rejected_unchanged(&mut session, replacement);
    }

    #[test]
    fn pending_input_presentation_visits_rng_scope_and_faults_reject_unchanged() {
        for boundary in 0..8 {
            let mut session = session();
            let replacement = Replacement::from_index(&session.index, &session.house).unwrap();
            match boundary {
                0 => session.simulation.visit = Some((1, None)),
                1 => {
                    session.simulation.vm.lot_rng_scope = Some(
                        serde_json::from_value(
                            json!({"tick":123,"saved_global":19,"lot_at_entry":17}),
                        )
                        .unwrap(),
                    )
                }
                2 => session.simulation.vm.scheduled_object_visit = Some(
                    serde_json::from_value(
                        json!({"parent":null,"object":1,"thread":1,"tick":123,"finished":false}),
                    )
                    .unwrap(),
                ),
                3 => session.host.presentation.ui_portrait_tick = Some(123),
                4 => session.host.presentation.awaiting_house_open = true,
                5 => session.host.presentation.object_input = Some(
                    serde_json::from_value(json!({
                        "tick":123,"actor":1,"clicked":1,"target":1,"phase":"Query","actions":[],
                        "definition":{"source":"synthetic.iff","object":1,"guid":1,"slots":0}
                    }))
                    .unwrap(),
                ),
                6 => {
                    session.simulation.vm.threads.insert(
                        1,
                        sims_vm::Thread {
                            service: None,
                            dispatch: None,
                            autonomy: None,
                            find_best: vec![],
                            completed_actions: vec![],
                            last_result: false,
                            callback: None,
                            callback_result: None,
                            stack: vec![],
                            frame_memory: vec![],
                            retained_frame_arena: None,
                            check: false,
                            interrupted: false,
                            status: sims_vm::Status::Faulted(sims_vm::Fault {
                                message: "native nearby-house order unavailable".into(),
                                stack: vec![],
                            }),
                        },
                    );
                }
                7 => session.simulation.vm.world.person = None,
                _ => unreachable!(),
            }
            rejected_unchanged(&mut session, replacement);
        }
    }

    #[test]
    fn pending_publication_notification_clock_loading_postload_and_selection_reject() {
        use sims_vm::commander::{
            Registry,
            house::{Fields, Interface, Receiver, dispatch::Dispatch},
        };
        let fields = || Fields {
            viewer_interface: false,
            include_visible: 0,
            renderer_dirty: 0,
            ui: Interface {
                mask_owner_present: false,
                panel_active: false,
                shared_present: false,
                gated: 0,
                creation_gated: 0,
                dirty: 0,
            },
        };
        for owner in 0..6 {
            let mut session = session();
            let replacement = Replacement::from_index(&session.index, &session.house).unwrap();
            match owner {
                0 => session.host.presentation.publication = Some(serde_json::from_value(json!({
                    "context":{"panel_owner":null,"closing":0,"walls":0,"tool_gate":0,"current_tool":null,"sound_object":0},
                    "serial":1,"tick":123,"frame":sims_vm::interface::publication::Frame::new(1),
                    "world":null,"camera":false,"drawing":null,"notice":false
                })).unwrap()),
                1 => session.host.presentation.notifications = Some(serde_json::from_value(json!({
                    "dispatch": Dispatch::begin(Registry::<Receiver>::default(), fields(), 0xf6, 0).unwrap(),
                    "viewer":null,"pending":null
                })).unwrap()),
                2 => session.host.presentation.clock = Some(crate::original_session::clock::State::from_completed_update(
                    sims_vm::scheduler::clock::Fields {
                        timing: Default::default(), period: 1, timer: Default::default(),
                        budget_d8: 0, budget_dc: 0, poll_messages: 0, publish: 0,
                    }, 0, 0, 123)),
                3 => session.host.presentation.loading = Some(serde_json::from_value(json!({
                    "call": sims_vm::load_completion::Completion::new(sims_vm::load_completion::Inputs {
                        lot_argument: 9, reset_requested: 0, imported: 0, residents_before_visits: vec![]
                    }).unwrap(), "family":0,"manager_lot":9,"classes":{},"config":null,
                    "registry":null,"fields":null,"child":null,"resource":null,"created":null
                })).unwrap()),
                4 => session.host.presentation.postload = Some(serde_json::from_value(json!({
                    "tail":sims_vm::load_finalize::Tail::default(),"enabled":1,
                    "registry":Registry::<Receiver>::default(),"fields":fields(),"started":false,"created":null
                })).unwrap()),
                5 => session.simulation.native_selection = Some(serde_json::from_value(json!({
                    "enabled":true,"selection":sims_vm::selection::Selection::new(None),
                    "flags":null,"indicators":null,"release":null,"placement":null,"external":null,"started":false
                })).unwrap()),
                _ => unreachable!(),
            }
            rejected_unchanged(&mut session, replacement);
        }
    }

    #[test]
    fn source_hash_case_duplicates_are_not_a_replacement_identity() {
        let mut session = session();
        let replacement = Replacement::from_index(&session.index, &session.house).unwrap();
        session
            .house
            .sources
            .insert(HOUSE.to_ascii_lowercase(), sha(HOUSE.as_bytes()));
        rejected_unchanged(&mut session, replacement);
    }

    #[test]
    fn default_legacy_and_captured_json_keep_replacement_absent_not_null() {
        let view = View::default();
        let encoded = serde_json::to_vec(&view).unwrap();
        let value: Value = serde_json::from_slice(&encoded).unwrap();
        assert!(value.get("neighborhood_replacement").is_none());
        assert!(value.get("neighborhood_input").is_none());
        let legacy: View = serde_json::from_value(value).unwrap();
        assert_eq!(serde_json::to_vec(&legacy).unwrap(), encoded);
        let mut session = session();
        let captured = capture(&session);
        let expected = serde_json::to_value(&captured).unwrap();
        session.bind_neighborhood_capture(captured).unwrap();
        let encoded = serde_json::to_vec(&session.host.presentation).unwrap();
        let view: Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(view["neighborhood_input"], expected);
        assert!(view.get("neighborhood_replacement").is_none());
        let restored: View = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(serde_json::to_vec(&restored).unwrap(), encoded);
        let mut missing = self::session();
        missing.host.presentation = legacy;
        assert_eq!(
            missing
                .simulation
                .vm
                .world
                .person
                .as_ref()
                .unwrap()
                .nearby_houses,
            None
        );
        assert_eq!(
            missing.neighborhood_input_status().unwrap(),
            super::super::Status::Missing
        );
    }

    #[test]
    fn retained_validation_never_discovers_again_and_rejects_conflict_or_live_tampering() {
        let mut session = session();
        session.bind_installed_houses_ascending_v1().unwrap();
        let frozen = state(&session);
        let index = Arc::make_mut(&mut session.index);
        index
            .resources
            .retain(|resource| resource.locator == NEIGHBORHOOD);
        assert!(Replacement::from_index(&session.index, &session.house).is_err());
        session.validate_neighborhood_input().unwrap();
        assert!(matches!(
            session.neighborhood_input_status().unwrap(),
            super::super::Status::Replacement(_)
        ));
        assert_eq!(state(&session)["simulation"], frozen["simulation"]);
        session
            .simulation
            .vm
            .world
            .person
            .as_mut()
            .unwrap()
            .nearby_houses = Some(vec![11, 1]);
        assert!(session.validate_neighborhood_input().is_err());
        assert!(session.save_managed().is_err());
        session
            .simulation
            .vm
            .world
            .person
            .as_mut()
            .unwrap()
            .nearby_houses = Some(vec![1, 11]);
        session.host.presentation.neighborhood_input = Some(capture(&session));
        assert!(session.validate_neighborhood_input().is_err());
        assert!(session.neighborhood_input_status().is_err());
        assert!(session.save_managed().is_err());
    }

    #[test]
    fn managed_payload_roundtrip_retains_input_without_starting_files_or_new_inventory() {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let mut original = session();
                original.bind_installed_houses_ascending_v1().unwrap();
                original.simulation.vm.world.globals.insert(7, 11);
                let saved = original.save_managed().unwrap();
                let saved: Saved =
                    serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
                let mut cold = session();
                let mut empty = (*cold.index).clone();
                empty.resources.clear();
                let content = saved.managed.as_ref().unwrap();
                cold.index = Arc::new(content.owned.mount(&empty).unwrap());
                cold.host = live::Host::restore(&cold.index, saved.view, saved.assets).unwrap();
                cold.simulation = saved.simulation.state;
                cold.validate_neighborhood_input().unwrap();
                let mut retained = state(&cold);
                let mut expected = state(&original);
                retained.as_object_mut().unwrap().remove("index");
                expected.as_object_mut().unwrap().remove("index");
                assert_eq!(retained, expected);
                assert_eq!(cold.simulation.vm.world.globals[&7], 11);
                assert_eq!(
                    cold.host.presentation.neighborhood_replacement,
                    original.host.presentation.neighborhood_replacement
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn replacement_is_disqualified_from_completed_native_equality_evidence() {
        let mut session = session();
        session.bind_installed_houses_ascending_v1().unwrap();
        let error = completed_epoch::view(&session.save(), &Value::Null, &Value::Null).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("disqualified from native-equality evidence")
        );
    }
}
